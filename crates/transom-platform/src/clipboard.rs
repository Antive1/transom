//! Schranka pre nastroje: odber zmien (len `AddClipboardFormatListener`, ziadne dotazovanie) na vlastnom vlakne s oknom len na
//! spravy, citanie snimky schranky (nazvy formatov, priznaky sukromia, vlastnik, text / obrazok / zoznam suborov s limitmi) a zapis
//! polozky spat. Listener sa spusta **len** po vyslovnom zapnuti historie a pri pozastaveni alebo vypnuti sa zastavi (vtedy sa
//! obsah schranky vobec necita). Obsah sa nikdy nelogue.
use crate::dragdrop::parse_hdrop;
use crate::imaging::{dib_dims, dib_to_bgra, encode_png, within};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, EnumClipboardFormats,
    GetClipboardData, GetClipboardFormatNameW, GetClipboardOwner, OpenClipboard,
    RegisterClipboardFormatW, RemoveClipboardFormatListener, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowThreadProcessId, PostMessageW, RegisterClassW, TranslateMessage, HWND_MESSAGE, MSG,
    WM_CLIPBOARDUPDATE, WM_CLOSE, WNDCLASSW, WS_EX_NOACTIVATE,
};

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIB: u32 = 8;
/// Najvacsie citane objemy (bajty); vacsi obsah sa nacita ako "neuloziteľný" (len nazov formatu).
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_DIB_BYTES: usize = 64 * 1024 * 1024;
const OWN_FORMAT: PCWSTR = w!("transom.OwnWrite");

/// Co sa zo schranky vobec smie nacitat (nastavuje aplikacia podla konfiguracie histori). Vypnuty druh obsahu sa **necita ani
/// nekopiruje ani nekoduje**: obrazok sa najprv zamietne podla hlavicky a az potom sa kopiruje pamat, a to este pred
/// dekodovanim a kodovanim do PNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadPolicy {
    pub text: bool,
    pub images: bool,
    pub files: bool,
    /// Najvacsia strana obrazka (body).
    pub max_side: u32,
}

impl Default for ReadPolicy {
    fn default() -> Self {
        ReadPolicy {
            text: true,
            images: true,
            files: true,
            max_side: 8192,
        }
    }
}

static POLICY_BITS: AtomicU32 = AtomicU32::new(0b111);
static POLICY_SIDE: AtomicU32 = AtomicU32::new(8192);

/// Nastavi politiku citania (volat pri kazdej zmene konfiguracie; prejavi sa pri dalsej zmene schranky).
pub fn set_read_policy(p: ReadPolicy) {
    let bits = u32::from(p.text) | u32::from(p.images) << 1 | u32::from(p.files) << 2;
    POLICY_BITS.store(bits, Ordering::Relaxed);
    POLICY_SIDE.store(p.max_side, Ordering::Relaxed);
}

pub fn read_policy() -> ReadPolicy {
    let b = POLICY_BITS.load(Ordering::Relaxed);
    ReadPolicy {
        text: b & 1 != 0,
        images: b & 2 != 0,
        files: b & 4 != 0,
        max_side: POLICY_SIDE.load(Ordering::Relaxed),
    }
}

/// Rozhodnutie podla hlavicky DIB (prvych 12 B): nacitat obrazok? Cista funkcia (bez schranky), aby sa dala testovat.
pub fn dib_wanted(policy: &ReadPolicy, head: &[u8]) -> bool {
    policy.images && dib_dims(head).is_some_and(|(w, h)| within(w, h, policy.max_side))
}

/// Snimka schranky (kopia poloziek; mapuje sa na `transom_tools::clipboard::Snapshot`).
#[derive(Clone, Debug, Default)]
pub struct ClipSnapshot {
    pub formats: Vec<String>,
    pub can_include_history: Option<u32>,
    pub can_upload: Option<u32>,
    pub owner_exe: Option<String>,
    pub own_write: bool,
    pub text: Option<String>,
    pub image_png: Option<Vec<u8>>,
    pub image_dims: (u32, u32),
    pub files: Option<Vec<String>>,
}

/// Co sa zapisuje do schranky.
pub enum Put {
    Text(String),
    Files(Vec<String>),
    Png(Vec<u8>),
}

fn open_clipboard(owner: Option<HWND>) -> bool {
    // schranku moze kratko drzat ina aplikacia: par kratkych pokusov (nikdy nekonecna slucka)
    for _ in 0..5 {
        // SAFETY: bezne otvorenie schranky; zatvara sa v `Guard`.
        if unsafe { OpenClipboard(owner) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    false
}

struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: parter k uspesnemu `OpenClipboard`.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn format_name(f: u32) -> String {
    match f {
        1 => "CF_TEXT".into(),
        2 => "CF_BITMAP".into(),
        8 => "CF_DIB".into(),
        13 => "CF_UNICODETEXT".into(),
        15 => "CF_HDROP".into(),
        17 => "CF_DIBV5".into(),
        _ => {
            let mut b = [0u16; 128];
            // SAFETY: buffer s vlastnou dlzkou.
            let n = unsafe { GetClipboardFormatNameW(f, &mut b) };
            if n > 0 {
                String::from_utf16_lossy(&b[..n as usize])
            } else {
                format!("#{f}")
            }
        }
    }
}

/// Bajty globalnej pamate formatu (s limitom); `None` ak chyba, je prilis velka alebo sa nedala zamknut.
fn global_bytes(fmt: u32, max: usize) -> Option<Vec<u8>> {
    // SAFETY: schranka je otvorena; dlzka zo `GlobalSize`, citanie pod zamkom a hned kopia.
    unsafe {
        let h = GetClipboardData(fmt).ok()?;
        let hg = HGLOBAL(h.0);
        let size = GlobalSize(hg);
        if size == 0 || size > max {
            return None;
        }
        let p = GlobalLock(hg) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(hg);
        Some(v)
    }
}

/// Ako `global_bytes`, ale pamat sa skopiruje **az ked** `accept` schvali jej prvych `head_len` bajtov (hlavicka). Zamietnuty
/// obsah sa nekopiruje vobec.
fn global_bytes_if(
    fmt: u32,
    max: usize,
    head_len: usize,
    accept: impl FnOnce(&[u8]) -> bool,
) -> Result<Vec<u8>, Gate> {
    // SAFETY: schranka je otvorena; dlzka zo `GlobalSize`, citanie pod zamkom (hlavicka najviac po koniec bloku), kopia az po schvaleni.
    unsafe {
        let h = GetClipboardData(fmt).map_err(|_| Gate::Missing)?;
        let hg = HGLOBAL(h.0);
        let size = GlobalSize(hg);
        if size == 0 {
            return Err(Gate::Missing);
        }
        if size > max {
            return Err(Gate::TooBig);
        }
        let p = GlobalLock(hg) as *const u8;
        if p.is_null() {
            return Err(Gate::Missing);
        }
        let v = if accept(std::slice::from_raw_parts(p, size.min(head_len))) {
            Ok(std::slice::from_raw_parts(p, size).to_vec())
        } else {
            Err(Gate::Rejected)
        };
        let _ = GlobalUnlock(hg);
        v
    }
}

/// Preco sa obsah nenacital (bez kopirovania pamate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    /// Format v schranke nie je alebo je prazdny.
    Missing,
    /// Blok je vacsi nez limit.
    TooBig,
    /// Hlavicka neprejde kontrolou (nadmerne alebo nezmyselne rozmery).
    Rejected,
}

/// Priznak sukromia aplikacie (DWORD). `None` = priznak v schranke nie je. Ak **je** pritomny, ale hodnotu sa nepodarilo precitat
/// (oneskorene vykreslenie, zly tvar), berie sa to ako `Some(0)` (= nezaradit): zlyhanie citania nikdy neznamena povolenie.
fn dword_format(name: PCWSTR, present: &[u32]) -> Option<u32> {
    // SAFETY: `RegisterClipboardFormatW` s platnym retazcom.
    let id = unsafe { RegisterClipboardFormatW(name) };
    if id == 0 || !present.contains(&id) {
        return None;
    }
    let Some(b) = global_bytes(id, 16) else {
        return Some(0);
    };
    Some(if b.len() >= 4 {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        0
    })
}

fn owner_exe() -> Option<String> {
    // SAFETY: dotaz na vlastnika schranky a nazov jeho obrazu (obmedzene opravnenie, ziadne citanie pamate procesu).
    unsafe {
        let o = GetClipboardOwner().ok()?;
        let mut pid = 0u32;
        GetWindowThreadProcessId(o, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut b = [0u16; 520];
        let mut n = b.len() as u32;
        let ok = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(b.as_mut_ptr()),
            &mut n,
        )
        .is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let full = String::from_utf16_lossy(&b[..n as usize]);
        full.rsplit('\\').next().map(str::to_string)
    }
}

/// Precita aktualny obsah schranky. Najprv sa zisti zoznam formatov a priznaky; **obsah sa nacita len ak nie je oznaceny ako
/// citlivy** (rozhodnutie robi aj `transom_tools::clipboard::capture`, tu je to len uspora a minimalizacia).
pub fn read_snapshot(own_marker: bool) -> Option<ClipSnapshot> {
    if !open_clipboard(None) {
        return None;
    }
    let _g = Guard;
    let mut s = ClipSnapshot::default();
    let mut f = 0u32;
    let mut ids = Vec::new();
    // SAFETY: schranka je otvorena; pocet formatov je ohraniceny.
    unsafe {
        loop {
            f = EnumClipboardFormats(f);
            if f == 0 || ids.len() >= 200 {
                break;
            }
            ids.push(f);
        }
    }
    s.formats = ids.iter().map(|f| format_name(*f)).collect();
    s.own_write = own_marker || s.formats.iter().any(|n| n == "transom.OwnWrite");
    s.can_include_history = dword_format(w!("CanIncludeInClipboardHistory"), &ids);
    s.can_upload = dword_format(w!("CanUploadToCloudClipboard"), &ids);
    s.owner_exe = owner_exe();
    let sensitive = s.can_include_history == Some(0)
        || s.formats.iter().any(|n| {
            let l = n.to_ascii_lowercase();
            l == "excludeclipboardcontentfrommonitorprocessing" || l == "clipboard viewer ignore"
        });
    if sensitive || s.own_write {
        return Some(s);
    }
    let pol = read_policy();
    if pol.text && ids.contains(&CF_UNICODETEXT) {
        if let Some(b) = global_bytes(CF_UNICODETEXT, MAX_TEXT_BYTES) {
            let units: Vec<u16> = b
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
            s.text = String::from_utf16(&units[..end]).ok();
        }
    }
    if pol.files && ids.contains(&CF_HDROP) {
        if let Some(b) = global_bytes(CF_HDROP, 4 * 1024 * 1024) {
            let v = parse_hdrop(&b);
            if !v.is_empty() {
                s.files = Some(v);
            }
        }
    }
    if pol.images && s.files.is_none() && ids.contains(&CF_DIB) {
        // hlavicka rozhodne pred kopiou; DIB a BGRA sa uvolnia hned po dekodovani resp. kodovani (nezostavaju ziv pocas WIC)
        let img = global_bytes_if(CF_DIB, MAX_DIB_BYTES, 12, |head| dib_wanted(&pol, head))
            .ok()
            .and_then(|b| dib_to_bgra(&b))
            .and_then(|(w, h, bgra)| encode_png(w, h, &bgra).ok().map(|png| (w, h, png)));
        if let Some((w, h, png)) = img {
            s.image_dims = (w, h);
            s.image_png = Some(png);
        }
    }
    Some(s)
}

/// Poradove cislo obsahu schranky (meni sa pri kazdom zapise; `GetClipboardSequenceNumber`).
pub fn sequence() -> u32 {
    // SAFETY: bez parametrov.
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

/// Obrazok zo schranky ako PNG na vyslovnu poziadavku pouzivatela (rychla snimka do odlozenych suborov). Nezavisle od
/// historie schranky; najviac `MAX_DIB_BYTES`. Nic sa neuklada ani nelogue (uklada volajuci, kam pouzivatel chcel).
/// Volat na vlakne s COM (WIC).
pub fn read_image_png() -> Option<(u32, u32, Vec<u8>)> {
    if !open_clipboard(None) {
        return None;
    }
    let b = {
        let _g = Guard;
        global_bytes(CF_DIB, MAX_DIB_BYTES)?
    };
    let (w, h, bgra) = crate::imaging::dib_to_bgra(&b)?;
    crate::imaging::encode_png(w, h, &bgra)
        .ok()
        .map(|png| (w, h, png))
}

/// Text zo schranky na vyslovnu poziadavku pouzivatela (Ctrl+V vo vstupnom poli): najviac 1 MiB, bez riadiacich znakov okrem
/// novych riadkov a tabulatorov. Nic sa neuklada ani nelogue.
pub fn read_text() -> Option<String> {
    if !open_clipboard(None) {
        return None;
    }
    let _g = Guard;
    let b = global_bytes(CF_UNICODETEXT, MAX_TEXT_BYTES)?;
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
    let s = String::from_utf16(&units[..end]).ok()?;
    Some(
        s.chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect(),
    )
}

/// Spusti odber zmien na vlastnom vlakne; `on_change` dostane snimku (volane z toho vlakna). Zastavenie = `drop`.
pub struct Listener {
    hwnd: isize,
    join: Option<std::thread::JoinHandle<()>>,
}

type Callback = Arc<dyn Fn(ClipSnapshot) + Send + Sync>;

thread_local! {
    static CB: std::cell::RefCell<Option<Callback>> = const { std::cell::RefCell::new(None) };
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        if let Some(s) = read_snapshot(false) {
            CB.with(|c| {
                if let Some(cb) = c.borrow().as_ref() {
                    cb(s);
                }
            });
        }
        return LRESULT(0);
    }
    // SAFETY: standardne spracovanie ostatnych sprav.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

impl Listener {
    pub fn start(on_change: Callback) -> Option<Listener> {
        let (tx, rx): (Sender<Option<isize>>, _) = channel();
        let join = std::thread::Builder::new()
            .name("transom-clipboard".into())
            .spawn(move || {
                // SAFETY: okno len na spravy a jeho slucka patria tomuto vlaknu; COM je potrebny pre WIC (PNG obrazkov).
                unsafe {
                    let com = windows::Win32::System::Com::CoInitializeEx(
                        None,
                        windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                    )
                    .is_ok();
                    let Ok(hinst) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    else {
                        let _ = tx.send(None);
                        return;
                    };
                    let cls = w!("TransomClipboard");
                    RegisterClassW(&WNDCLASSW {
                        lpfnWndProc: Some(proc),
                        hInstance: hinst.into(),
                        lpszClassName: cls,
                        ..Default::default()
                    });
                    let Ok(hwnd) = CreateWindowExW(
                        WS_EX_NOACTIVATE,
                        cls,
                        w!("transom-clipboard"),
                        Default::default(),
                        0,
                        0,
                        0,
                        0,
                        Some(HWND_MESSAGE),
                        None,
                        Some(hinst.into()),
                        None,
                    ) else {
                        let _ = tx.send(None);
                        return;
                    };
                    if AddClipboardFormatListener(hwnd).is_err() {
                        let _ = DestroyWindow(hwnd);
                        let _ = tx.send(None);
                        return;
                    }
                    CB.with(|c| *c.borrow_mut() = Some(on_change));
                    let _ = tx.send(Some(hwnd.0 as isize));
                    let mut m = MSG::default();
                    while GetMessageW(&mut m, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&m);
                        DispatchMessageW(&m);
                    }
                    let _ = RemoveClipboardFormatListener(hwnd);
                    let _ = DestroyWindow(hwnd);
                    CB.with(|c| *c.borrow_mut() = None);
                    if com {
                        windows::Win32::System::Com::CoUninitialize();
                    }
                }
            })
            .ok()?;
        match rx.recv().ok()? {
            Some(h) => Some(Listener {
                hwnd: h,
                join: Some(join),
            }),
            None => {
                let _ = join.join();
                None
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // SAFETY: sprava oknu patriacemu nasmu vlaknu; `WM_CLOSE` -> `DefWindowProc` zrusi okno, ale slucku ukoncime `WM_QUIT`.
        unsafe {
            let h = HWND(self.hwnd as *mut _);
            let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(
                Some(h),
                windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                WPARAM(0),
                LPARAM(0),
            );
        }
        // ukoncenie slucky nesmie blokovat volajuce (UI) vlakno: vlakno listenera moze prave citat velky obrazok. Spozdene volanie
        // spätnej funkcie nic nezachyti, lebo sluzba kontroluje zapnutie historie pri kazdom zachyteni.
        if let Some(j) = self.join.take() {
            let _ = std::thread::Builder::new()
                .name("transom-clipboard-join".into())
                .spawn(move || {
                    let _ = j.join();
                });
        }
    }
}

fn alloc_global(bytes: &[u8]) -> Option<HGLOBAL> {
    // SAFETY: nova globalna pamat pre schranku; po `SetClipboardData` jej vlastnictvo prechadza na system.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Some(h)
    }
}

/// Zapise polozku do schranky a oznaci ju ako nas zapis (znacka `transom.OwnWrite`), aby sa nezachytila znova. Vracia uspech.
pub fn write(p: &Put) -> bool {
    if !open_clipboard(None) {
        return false;
    }
    let _g = Guard;
    // SAFETY: schranka je otvorena; kazde `SetClipboardData` dostane novu globalnu pamat.
    unsafe {
        if EmptyClipboard().is_err() {
            return false;
        }
        let ok = match p {
            Put::Text(t) => {
                let mut v: Vec<u8> = t.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
                v.extend_from_slice(&[0, 0]);
                set(CF_UNICODETEXT, &v)
            }
            Put::Files(f) => set(CF_HDROP, &crate::dragdrop::build_hdrop(f)),
            Put::Png(b) => {
                let id = RegisterClipboardFormatW(w!("PNG"));
                id != 0 && set(id, b)
            }
        };
        let m = RegisterClipboardFormatW(OWN_FORMAT);
        if m != 0 {
            let _ = set(m, &[1, 0, 0, 0]);
        }
        // obnovena polozka sa nema dostat do historie Windows (Win+V) ani do cloudovej schranky, ak to pouzivatel nechcel:
        // dokumentovane priznaky sukromia (DWORD 0)
        for name in [
            w!("CanIncludeInClipboardHistory"),
            w!("CanUploadToCloudClipboard"),
        ] {
            let f = RegisterClipboardFormatW(name);
            if f != 0 {
                let _ = set(f, &[0, 0, 0, 0]);
            }
        }
        ok
    }
}

unsafe fn set(fmt: u32, bytes: &[u8]) -> bool {
    let Some(h) = alloc_global(bytes) else {
        return false;
    };
    // SAFETY: vlastnictvo pamate prechadza na system pri uspechu; pri zlyhani ju uvolnime my (inak by unikla).
    unsafe {
        if SetClipboardData(fmt, Some(HANDLE(h.0))).is_ok() {
            true
        } else {
            let _ = windows::Win32::Foundation::GlobalFree(Some(h));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skutocna schranka sa v testoch NEMENI (pouzivatel ju moze pouzivat): overuje sa len vypis nazvov a zivotny cyklus
    /// listenera s okamzitym zastavenim; ziadny zapis a ziadne citanie obsahu.
    #[test]
    fn the_listener_starts_and_stops_cleanly_without_touching_the_clipboard_content() {
        let l = Listener::start(Arc::new(|_| {})).expect("listener");
        drop(l);
        let l2 = Listener::start(Arc::new(|_| {})).expect("znova");
        drop(l2);
    }

    #[test]
    fn disabled_oversized_or_malformed_images_are_refused_by_the_header_before_any_copy() {
        let hdr = |w: i32, h: i32| {
            let mut v = Vec::new();
            v.extend_from_slice(&40u32.to_le_bytes());
            v.extend_from_slice(&w.to_le_bytes());
            v.extend_from_slice(&h.to_le_bytes());
            v
        };
        let on = ReadPolicy::default();
        assert!(dib_wanted(&on, &hdr(1920, 1080)));
        assert!(dib_wanted(&on, &hdr(1920, -1080)), "zhora nadol");
        let off = ReadPolicy {
            images: false,
            ..on
        };
        assert!(
            !dib_wanted(&off, &hdr(10, 10)),
            "vypnute obrazky sa necitaju"
        );
        assert!(!dib_wanted(&on, &hdr(8193, 10)), "strana nad limitom");
        assert!(!dib_wanted(&on, &hdr(16384, 16384)), "nad 64 Mpx");
        let small = ReadPolicy {
            max_side: 100,
            ..on
        };
        assert!(dib_wanted(&small, &hdr(100, 100)) && !dib_wanted(&small, &hdr(101, 1)));
        for bad in [hdr(0, 10), hdr(10, 0), hdr(-4, 10)] {
            assert!(!dib_wanted(&on, &bad));
        }
        assert!(!dib_wanted(&on, &[]) && !dib_wanted(&on, &hdr(10, 10)[..11]));
    }

    #[test]
    fn the_read_policy_round_trips_and_defaults_to_everything_allowed() {
        let before = read_policy();
        let p = ReadPolicy {
            text: false,
            images: true,
            files: false,
            max_side: 640,
        };
        set_read_policy(p);
        assert_eq!(read_policy(), p);
        set_read_policy(before);
        assert_eq!(ReadPolicy::default().max_side, 8192);
    }

    #[test]
    fn well_known_formats_get_stable_names() {
        assert_eq!(format_name(CF_UNICODETEXT), "CF_UNICODETEXT");
        assert_eq!(format_name(CF_HDROP), "CF_HDROP");
        assert_eq!(format_name(8), "CF_DIB");
    }
}
