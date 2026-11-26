//! OLE drag-and-drop pre lištu: prijem suborov (cieľ pustenia na liste a na vyrastajucom "vybezku"), vybezok (animovane okno pod
//! stredom listy) a tahanie von (`SHDoDragDrop`). Vsetko je pasivne a bezpecne:
//! * Prijima sa len `CF_HDROP` (zoznam suborov); efekt je vzdy **COPY** (nikdy MOVE, takze zdroj nic nezmaze).
//! * Pocas `DragEnter`/`DragOver` sa len pyta `QueryGetData` (ziadne citanie dat); data sa citaju az pri `Drop`, s limitmi a
//!   vlastnym parserom (`parse_hdrop`); nic sa nestatuje ani neotvara na UI vlakne.
//! * Pustenie sa zaradi do fronty a spracuje sa po navrate z OLE (`BarEvent::FilesDropped`); lista ani vybezok nikdy
//!   nekradnu popredie (`WS_EX_NOACTIVATE`) a nic sa nespusta ani neotvara.
use crate::appbar::{ctl_hwnd, with_handler, BarEvent};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::Instant;
use windows::core::{implement, Ref};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, POINTL, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateRoundRectRgn, DeleteDC,
    DeleteObject, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, SelectObject,
    SetWindowRgn, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, OleInitialize, OleUninitialize, RegisterDragDrop,
    ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::GetDpiForMonitor;
use windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, KillTimer,
    PostMessageW, RegisterClassW, SetTimer, SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SW_HIDE, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Druh popoveru, ktorym sa kresli vybezok (`BarHandler::paint_popover`).
pub const NOTCH_KIND: u32 = 100;
/// Velkost vybezku v lp.
pub const NOTCH_W_LP: i32 = 420;
pub const NOTCH_H_LP: i32 = 124;
/// Najviac suborov z jedneho pustenia, najdlhsia cesta (znaky UTF-16) a najvacsi objem metadat (bajty).
pub const MAX_DROP_FILES: usize = 500;
pub const MAX_PATH_UNITS: usize = 32_767;
pub const MAX_DROP_BYTES: usize = 4 * 1024 * 1024;

const T_ANIM: usize = 1;
const T_GRACE: usize = 2;
const GRACE_MS: u32 = 650;
/// `WM_MOUSELEAVE` (nie je v exportoch windows-rs pod tymto menom vo vsetkych verziach).
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const EXPAND_MS: f32 = 180.0;
const COLLAPSE_MS: f32 = 140.0;
/// Sprava riadiacemu oknu: spracovat frontu pusteni (`pump`).
pub(crate) const WM_TRANSOM_DRAG: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x38;

/// Efekt pustenia: len kopirovanie (nikdy presun, ktory by zdroju zmazal originaly) a len ak ide o zoznam suborov.
pub fn effect_for(allowed: DROPEFFECT, accepting: bool) -> DROPEFFECT {
    if accepting && (allowed.0 & DROPEFFECT_COPY.0) != 0 {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

/// Cesty z bloku `DROPFILES` (CF_HDROP): hlavicka 20 bajtov (`pFiles`, bod, `fNC`, `fWide`), potom retazce UTF-16 oddelene
/// NUL a ukoncene dvojitym NUL. Chybny alebo neuplny blok, nie-Unicode zoznam, nadmerny pocet alebo dlzka = prazdny vysledok
/// alebo orezanie na limity. Nic sa nehodnoti ani neotvara.
pub fn parse_hdrop(bytes: &[u8]) -> Vec<String> {
    if bytes.len() < 20 || bytes.len() > MAX_DROP_BYTES {
        return Vec::new();
    }
    let p_files = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let wide = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) != 0;
    if !wide || p_files < 20 || p_files >= bytes.len() {
        return Vec::new();
    }
    let data = &bytes[p_files..];
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let mut out = Vec::new();
    let mut cur: Vec<u16> = Vec::new();
    for u in units {
        if u == 0 {
            if cur.is_empty() {
                break; // dvojity NUL = koniec zoznamu
            }
            if let Ok(s) = String::from_utf16(&cur) {
                out.push(s);
            }
            cur.clear();
            if out.len() >= MAX_DROP_FILES {
                break;
            }
        } else {
            cur.push(u);
            if cur.len() > MAX_PATH_UNITS {
                // nezmyselna cesta: zvysok bloku sa nepouzije
                return out;
            }
        }
    }
    out
}

/// Zostavi blok `DROPFILES` (na testy a na tahanie von s vlastnym objektom): cesty UTF-16 oddelene NUL, dvojity NUL na konci.
pub fn build_hdrop(paths: &[String]) -> Vec<u8> {
    let mut v = vec![0u8; 20];
    v[0] = 20;
    v[16] = 1; // fWide
    for p in paths {
        for u in p.encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v.extend_from_slice(&[0, 0]);
    }
    v.extend_from_slice(&[0, 0]);
    v
}

enum Q {
    Enter,
    Leave,
    Drop(Vec<String>),
    /// Virtualne subory (nazov, obsah) - napr. obrazok zo stranky v prehliadaci.
    DropData(Vec<(String, Vec<u8>)>),
    /// Mys vo vybezku pri prezerani odlozenych suborov (px v klientskej ploche vybezku).
    Pointer(crate::appbar::PointerKind, i32, i32),
    /// Vysledok kapatka na farbu (`crate::picker`); fronta tu sluzi ako odlozene doručenie mimo hacika mysi.
    Color(Option<(u8, u8, u8)>),
}

/// Odlozene oznamenie vysledku kapatka (vola hacik mysi; aplikacia ho dostane mimo hacika).
pub(crate) fn post_color(c: Option<(u8, u8, u8)>) {
    post(Q::Color(c));
}

thread_local! {
    static OLE: Cell<bool> = const { Cell::new(false) };
    static QUEUE: RefCell<VecDeque<Q>> = const { RefCell::new(VecDeque::new()) };
    static NOTCH: RefCell<Option<NotchState>> = const { RefCell::new(None) };
    static DRAGGING: Cell<bool> = const { Cell::new(false) };
}

/// Test (`--test-tools`): vlozi pustenie zoznamu ciest do tej istej fronty ako skutocne OLE pustenie. Neoveruje OLE vrstvu OS
/// (ta sa testuje priamym volanim `IDropTarget`), ale cely zvysok cesty (udalost -> aplikacia -> sluzba -> ulozisko -> panel).
pub fn simulate_drop(paths: Vec<String>) {
    post(Q::Drop(paths));
}

/// Prebieha tahanie von z lišty / popoveru (popover sa pocas neho nezatvara pri deaktivacii).
pub fn dragging() -> bool {
    DRAGGING.with(|d| d.get())
}

/// Inicializuje OLE na vlakne listy (STA); vola sa pred prvym oknom. Vracia, ci sa podarilo.
pub fn init() -> bool {
    // SAFETY: inicializacia OLE pre toto vlakno; parter `uninit` pri zaniku hostitela.
    let ok = unsafe { OleInitialize(None) }.is_ok();
    OLE.with(|o| o.set(ok));
    watch_drags();
    ok
}

// ---- tahanie v systeme: vybezok sa vysunie uz pri priblizeni k stredu listy ----

const EVENT_SYSTEM_DRAGDROPSTART: u32 = 0x000E;
const EVENT_SYSTEM_DRAGDROPEND: u32 = 0x000F;
const T_NEAR: usize = 3;

thread_local! {
    /// Bezi tahanie (OLE DoDragDrop v lubovolnej aplikacii) a vybezok sa vysunul pri priblizeni (nie nad listou).
    static NEAR: Cell<(bool, bool)> = const { Cell::new((false, false)) };
}

/// Odber systemovych udalosti zaciatku a konca tahania (OLE ich posiela pre citacky obrazovky). Ziadne cudzie data sa
/// necitaju - len poloha kurzora pocas tahania.
fn watch_drags() {
    use windows::Win32::UI::Accessibility::SetWinEventHook;
    use windows::Win32::UI::WindowsAndMessaging::{WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS};
    // SAFETY: hacik bez DLL (mimo kontextu) na tomto vlakne; zanikne s procesom.
    unsafe {
        let _ = SetWinEventHook(
            EVENT_SYSTEM_DRAGDROPSTART,
            EVENT_SYSTEM_DRAGDROPEND,
            None,
            Some(drag_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
    }
}

unsafe extern "system" fn drag_event(
    _h: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    _obj: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let Some(hwnd) = ensure_notch() else { return };
    if event == EVENT_SYSTEM_DRAGDROPSTART {
        NEAR.with(|n| n.set((true, false)));
        // SAFETY: casovac vlastneho okna.
        unsafe {
            SetTimer(Some(hwnd), T_NEAR, 30, None);
        }
    } else {
        let (_, opened) = NEAR.with(|n| n.replace((false, false)));
        // SAFETY: casovac vlastneho okna.
        unsafe {
            let _ = KillTimer(Some(hwnd), T_NEAR);
        }
        // vysunuty len priblizenim a nic sa nepustilo: zbalit (ak nad nim nie je mys)
        if opened && NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| !s.browse)) {
            notch_leave();
        }
    }
}

/// Pocas tahania: kurzor blizko stredu pod listou (do ~150 lp pod nou, +-320 lp od stredu) vysunie vybezok.
fn near_tick() {
    if !NEAR.with(|n| n.get().0) || dragging() {
        return;
    }
    let mut pt = POINT::default();
    // SAFETY: citanie polohy kurzora.
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let Some((x, y, w, _, dpi)) = place_for_cursor() else {
        return;
    };
    let k = |v: i32| v * dpi as i32 / 96;
    let cx = x + w / 2;
    let near = (pt.x - cx).abs() < k(320) && pt.y >= y - k(40) && pt.y < y + k(150);
    let open = NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| s.to > 0.0));
    if near && !open {
        NEAR.with(|n| n.set((true, true)));
        notch_enter();
    }
}

pub fn uninit() {
    destroy_notch();
    if OLE.with(|o| o.replace(false)) {
        // SAFETY: parter k uspesnemu `OleInitialize` na tomto vlakne.
        unsafe { OleUninitialize() };
    }
}

fn post(q: Q) {
    QUEUE.with(|c| c.borrow_mut().push_back(q));
    if let Some(ctl) = ctl_hwnd() {
        // SAFETY: sprava platnemu oknu tohto procesu.
        unsafe {
            let _ = PostMessageW(Some(ctl), WM_TRANSOM_DRAG, WPARAM(0), LPARAM(0));
        }
    }
}

/// Spracuje frontu pusteni (vola riadiace okno pri `WM_TRANSOM_DRAG`, mimo OLE slucky).
pub(crate) fn pump() {
    let items: Vec<Q> = QUEUE.with(|c| c.borrow_mut().drain(..).collect());
    for q in items {
        match q {
            Q::Enter | Q::Leave => {}
            Q::Drop(paths) => crate::appbar::emit(BarEvent::FilesDropped(paths)),
            Q::DropData(files) => crate::appbar::emit(BarEvent::FileDataDropped(files)),
            Q::Pointer(kind, x, y) => crate::appbar::emit(BarEvent::NotchPointer { kind, x, y }),
            Q::Color(c) => crate::appbar::emit(BarEvent::ColorPicked(c)),
        }
    }
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// Pyta sa len, ci objekt obsahuje zoznam suborov (nic nenacitava).
fn has_files(data: Option<&IDataObject>) -> bool {
    let Some(d) = data else { return false };
    let f = hdrop_format();
    // SAFETY: `QueryGetData` len zisti dostupnost formatu.
    unsafe { d.QueryGetData(&f) }.is_ok() || crate::vfiles::available(d)
}

/// Je medium z `IDataObject::GetData` globalna pamat (`TYMED_HGLOBAL`)? Len v tom pripade je `u.hGlobal` platne.
fn is_hglobal_medium(tymed: u32) -> bool {
    tymed == TYMED_HGLOBAL.0 as u32
}

/// Nacita zoznam suborov z objektu (az pri pusteni): bajty bloku s limitom a vlastny parser.
fn read_files(d: &IDataObject) -> Vec<String> {
    let f = hdrop_format();
    // SAFETY: `GetData` vrati vlastnictvo mediumu, ktore uvolnime `ReleaseStgMedium`; globalna pamat sa cita pod zamkom s
    // velkostou z `GlobalSize` (nikdy za jej hranicu) a hned sa kopiruje.
    unsafe {
        let Ok(mut m) = d.GetData(&f) else {
            return Vec::new();
        };
        // zdroj smie vratit aj iny druh mediumu (stream, storage...): `u.hGlobal` by bol vtedy nezmyselny ukazovatel
        if !is_hglobal_medium(m.tymed) {
            ReleaseStgMedium(&mut m);
            return Vec::new();
        }
        let h = m.u.hGlobal;
        let size = GlobalSize(h);
        let paths = if (20..=MAX_DROP_BYTES).contains(&size) {
            let p = GlobalLock(h) as *const u8;
            if p.is_null() {
                Vec::new()
            } else {
                let bytes = std::slice::from_raw_parts(p, size).to_vec();
                let _ = GlobalUnlock(h);
                parse_hdrop(&bytes)
            }
        } else {
            Vec::new()
        };
        ReleaseStgMedium(&mut m);
        paths
    }
}

#[implement(IDropTarget)]
struct DropTarget {
    accepting: Cell<bool>,
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Ref<'_, IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        // vlastne tahanie von (z Police v liste / panela) sa na listu znova neprida
        let ok = !dragging() && has_files(pdataobj.as_ref());
        self.accepting.set(ok);
        if !pdweffect.is_null() {
            // SAFETY: OLE dodava platny ukazovatel na efekt.
            unsafe { *pdweffect = effect_for(*pdweffect, ok) };
        }
        if ok {
            notch_enter();
            post(Q::Enter);
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if !pdweffect.is_null() {
            // SAFETY: OLE dodava platny ukazovatel na efekt.
            unsafe { *pdweffect = effect_for(*pdweffect, self.accepting.get()) };
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.accepting.replace(false) {
            notch_leave();
            post(Q::Leave);
        }
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Ref<'_, IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let accepted = self.accepting.replace(false);
        let mut effect = DROPEFFECT_NONE;
        if accepted {
            if let Some(d) = pdataobj.as_ref() {
                let paths = read_files(d);
                if !paths.is_empty() {
                    effect = DROPEFFECT_COPY;
                    post(Q::Drop(paths));
                } else if crate::vfiles::available(d) {
                    // obsah bez cesty na disku (prehliadac, posta): aplikacia ho ulozi do odlozenych suborov
                    let files = crate::vfiles::read(d);
                    if !files.is_empty() {
                        effect = DROPEFFECT_COPY;
                        post(Q::DropData(files));
                    }
                }
            }
        }
        if !pdweffect.is_null() {
            // SAFETY: OLE dodava platny ukazovatel na efekt.
            unsafe { *pdweffect = effect };
        }
        notch_collapse_now();
        Ok(())
    }
}

/// Zaregistruje okno ako cieľ pustenia suborov (po `init`). Chyba sa ticho ignoruje (lista funguje aj bez pustenia).
pub fn register(hwnd: HWND) -> bool {
    if !OLE.with(|o| o.get()) {
        return false;
    }
    let t: IDropTarget = DropTarget {
        accepting: Cell::new(false),
    }
    .into();
    // SAFETY: platne okno tohto vlakna po `OleInitialize`.
    unsafe { RegisterDragDrop(hwnd, &t) }.is_ok()
}

pub fn revoke(hwnd: HWND) {
    if OLE.with(|o| o.get()) {
        // SAFETY: odregistrovanie pred zanikom okna.
        unsafe {
            let _ = RevokeDragDrop(hwnd);
        }
    }
}

// ---- vybezok (animovane okno pod stredom listy) ----

struct NotchState {
    hwnd: HWND,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    dpi: u32,
    /// Aktualna vyska (px) a ciel animacie.
    cur: f32,
    from: f32,
    to: f32,
    t0: Instant,
    dur_ms: f32,
    /// Okno ma zapnutu priehladnost (cierne sklo ako panely).
    glass: bool,
    /// Rezim prezerania odlozenych suborov (otvoreny prejdenim mysou nad polozkou v liste), nie len napoveda pri tahani.
    browse: bool,
    /// Mys je nad vybezkom (zbalenie sa odklada).
    hover: bool,
    /// Farba listy nad vybezkom (precitana z obrazovky pri otvoreni) - zaciatok nabehu skla.
    lead_color: Option<(u8, u8, u8)>,
    /// Kde okno naozaj lezi (x, y, w, h); animuje sa len jeho viditelna cast, nie velkost.
    placed: (i32, i32, i32, i32),
    /// Bezi animacia (drzi presny casovac).
    animating: bool,
    /// Viditelna cast teraz (x na obrazovke, sirka) a na zaciatku animacie: vybezok meni aj sirku (ostrov -> panel).
    cx: f32,
    cw: f32,
    fx: f32,
    fw: f32,
}

/// Okno vybezku: zjednotenie zaciatku a ciela animacie (x, sirka, vyska); obsah lezi v cieli.
fn notch_span(fx: f32, fw: f32, x: i32, w: i32, h: i32, from_h: f32) -> (i32, i32, i32) {
    let l = (fx.round() as i32).min(x);
    let r = ((fx + fw).round() as i32).max(x + w);
    (l, r - l, h.max(from_h.round() as i32))
}

thread_local! {
    /// Snimky animacie vybezku zladene s obrazovkou.
    static NPACE: std::cell::RefCell<Option<crate::vsync::Pacer>> = const { std::cell::RefCell::new(None) };
}

/// Vyduty prechod vybezku do listy (lp).
const NOTCH_FLARE_LP: i32 = 10;

/// Polomer spodnych rohov vybezku (lp).
const NOTCH_CORNER_LP: i32 = 14;

/// Zapne / vypne sklo okna vybezku podla nastavenia (pred zobrazenim, aby okno nezablikalo nekreslenym pozadim).
fn sync_notch_glass(hwnd: HWND) -> bool {
    let mut glass = None;
    with_handler(|hd| glass = hd.popover_glass(NOTCH_KIND));
    let was = NOTCH.with(|n| n.borrow().as_ref().map(|s| s.glass));
    if was != Some(glass.is_some()) {
        crate::glass::set_glass(hwnd, glass.is_some());
        NOTCH.with(|n| {
            if let Some(s) = n.borrow_mut().as_mut() {
                s.glass = glass.is_some();
            }
        });
    }
    glass.is_some()
}

/// Viditelna cast vybezku: `v` riadkov zhora; zaoblene spodne rohy. Na konci animacie pri skle cele okno (rohy
/// kresli sklo vyhladene), bez skla zaoblena oblast.
fn notch_region(hwnd: HWND, rx: i32, w: i32, v: i32, dpi: u32, whole_glass: bool) {
    // SAFETY: oblast prevezme okno.
    unsafe {
        if whole_glass {
            let _ = SetWindowRgn(hwnd, None, true);
            return;
        }
        let r = (NOTCH_CORNER_LP * dpi as i32 / 96).min(v.max(2) / 2);
        let rgn = CreateRoundRectRgn(rx, -2 * r, rx + w + 1, v.max(1) + 1, 2 * r, 2 * r);
        let _ = SetWindowRgn(hwnd, Some(rgn), true);
    }
}

fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Vyska vybezku v case `elapsed_ms` od zaciatku animacie (ease-out cubic): cista funkcia na testy.
pub fn notch_height(from: f32, to: f32, elapsed_ms: f32, dur_ms: f32) -> f32 {
    if dur_ms <= 0.0 {
        return to;
    }
    from + (to - from) * ease_out(elapsed_ms / dur_ms)
}

unsafe extern "system" fn notch_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    // SAFETY: wndproc okna tahania na vlakne UI; wParam/lParam sa pouzivaju len ako cisla; BeginPaint/EndPaint v tej istej vetve, GDI
    // objekty vlastnia pomocne typy (LayeredBuf, malovanie); stav NOTCH sa pozicia len kratko, bez volani posielajucich spravy.
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                // obsah ma velkost ciela (w, h) a lezi v okne posunuty o `ox` (okno je pocas zmeny sirky sirsie)
                let st = NOTCH.with(|n| {
                    n.borrow()
                        .as_ref()
                        .map(|s| (s.w, s.h, s.dpi, s.x - s.placed.0))
                });
                let (ww, wh) = (rc.right.max(1), rc.bottom.max(1));
                // sklo ako pri paneloch (ta ista volba v nastaveniach); prepnutie za behu zapne / vypne priehladnost
                sync_notch_glass(hwnd);
                let mut glass = None;
                with_handler(|hd| glass = hd.popover_glass(NOTCH_KIND));
                let glass_buf = match (st, glass) {
                    (Some(_), Some(_)) => crate::layered::LayeredBuf::new(ww, wh),
                    _ => None,
                };
                if let (Some((w, h, dpi, ox)), Some(mut gs), Some(mut b)) = (st, glass, glass_buf) {
                    // nabeh z farby listy je v lp: prepocet na DPI vybezku
                    if let Some(l) = gs.lead.as_mut() {
                        l.px = l.px * dpi as i32 / 96;
                        l.adopt(NOTCH.with(|n| n.borrow().as_ref().and_then(|s| s.lead_color)));
                    }
                    gs.corner = NOTCH_CORNER_LP * dpi as i32 / 96;
                    // vyduty prechod do listy (ako panely)
                    gs.flare = NOTCH_FLARE_LP * dpi as i32 / 96;
                    let fl = gs.flare;
                    // dva prechody (cierne a biele pozadie) -> alfa po pixeloch, zlozene nad sklo
                    for (dc, bd) in [
                        (b.black_dc(), (0u8, 0u8, 0u8)),
                        (b.white_dc(), (255, 255, 255)),
                    ] {
                        let ctx = crate::popup::PopoverPaint {
                            hdc: dc,
                            w: w - 2 * fl,
                            h,
                            dpi,
                            kind: NOTCH_KIND,
                            backdrop: Some(bd),
                        };
                        // cely podklad (aj pasy vyduteho prechodu a okraj okna pri zmene sirky) je ciste sklo
                        let _ = windows::Win32::Graphics::Gdi::PatBlt(
                            dc,
                            0,
                            0,
                            ww,
                            wh,
                            if bd.0 == 0 {
                                windows::Win32::Graphics::Gdi::BLACKNESS
                            } else {
                                windows::Win32::Graphics::Gdi::WHITENESS
                            },
                        );
                        let mut old = POINT::default();
                        let _ = windows::Win32::Graphics::Gdi::SetViewportOrgEx(
                            dc,
                            ox + fl,
                            0,
                            Some(&mut old),
                        );
                        with_handler(|hd| hd.paint_popover(&ctx));
                        let _ =
                            windows::Win32::Graphics::Gdi::SetViewportOrgEx(dc, old.x, old.y, None);
                    }
                    let area = crate::layered::Area::full(ww, wh);
                    b.compose_glass(area, &gs, 255);
                    b.blit(hdc, area);
                } else if let Some((w, h, dpi, ox)) = st {
                    // kresli sa cely vybezok (plna velkost); viditelna je len cast podla aktualnej vysky okna
                    if ox != 0 || rc.right != w || rc.bottom != h {
                        // okno je pocas zmeny sirky vacsie nez obsah: zvysok ciernou (bez skla nic nekresli)
                        let _ = windows::Win32::Graphics::Gdi::FillRect(
                            hdc,
                            &rc,
                            windows::Win32::Graphics::Gdi::HBRUSH(
                                windows::Win32::Graphics::Gdi::GetStockObject(
                                    windows::Win32::Graphics::Gdi::BLACK_BRUSH,
                                )
                                .0,
                            ),
                        );
                    }
                    let mem = CreateCompatibleDC(Some(hdc));
                    let bmp = CreateCompatibleBitmap(hdc, w.max(1), h.max(1));
                    let old = SelectObject(mem, bmp.into());
                    let ctx = crate::popup::PopoverPaint {
                        hdc: mem,
                        w,
                        h,
                        dpi,
                        kind: NOTCH_KIND,
                        backdrop: None,
                    };
                    with_handler(|hd| hd.paint_popover(&ctx));
                    let _ = BitBlt(
                        hdc,
                        ox,
                        0,
                        rc.right.min(w),
                        rc.bottom.min(h),
                        Some(mem),
                        0,
                        0,
                        SRCCOPY,
                    );
                    SelectObject(mem, old);
                    let _ = DeleteObject(bmp.into());
                    let _ = DeleteDC(mem);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == T_ANIM => {
                NPACE.with(|p| {
                    if let Some(p) = p.borrow().as_ref() {
                        p.ack();
                    }
                });
                notch_tick();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == T_NEAR => {
                near_tick();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == T_GRACE => {
                let _ = KillTimer(Some(hwnd), T_GRACE);
                // mys sa medzitym presunula na vybezok: ostava otvoreny
                if !NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| s.hover)) {
                    notch_collapse_now();
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                use crate::appbar::PointerKind;
                // obsah lezi v tele vybezku (posun okna a pas vyduteho prechodu pri skle)
                let ox = NOTCH.with(|n| {
                    n.borrow().as_ref().map_or(0, |s| {
                        s.x - s.placed.0
                            + if s.glass {
                                NOTCH_FLARE_LP * s.dpi as i32 / 96
                            } else {
                                0
                            }
                    })
                });
                let (x, y) = (
                    (lp.0 & 0xFFFF) as i16 as i32 - ox,
                    ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
                );
                let entered = NOTCH.with(|n| {
                    n.borrow_mut()
                        .as_mut()
                        .map(|s| !std::mem::replace(&mut s.hover, true))
                        .unwrap_or(false)
                });
                if entered {
                    let _ = KillTimer(Some(hwnd), T_GRACE);
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut t);
                }
                let kind = match msg {
                    WM_LBUTTONDOWN => {
                        SetCapture(hwnd);
                        PointerKind::Down
                    }
                    WM_LBUTTONUP => {
                        let _ = ReleaseCapture();
                        PointerKind::Click
                    }
                    _ => PointerKind::Move,
                };
                post(Q::Pointer(kind, x, y));
                LRESULT(0)
            }
            WM_MOUSELEAVE_MSG => {
                let browse = NOTCH.with(|n| {
                    n.borrow_mut()
                        .as_mut()
                        .map(|s| {
                            s.hover = false;
                            s.browse
                        })
                        .unwrap_or(false)
                });
                post(Q::Pointer(crate::appbar::PointerKind::Leave, 0, 0));
                if browse {
                    notch_leave();
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn ensure_notch() -> Option<HWND> {
    if let Some(h) = NOTCH.with(|n| n.borrow().as_ref().map(|s| s.hwnd)) {
        return Some(h);
    }
    // SAFETY: vytvorenie neaktivujuceho okna vybezku; trieda sa registruje opakovane bez chyby (uz existuje).
    unsafe {
        let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?;
        let cls = windows::core::w!("TransomNotch");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(notch_proc),
            hInstance: hinst.into(),
            lpszClassName: cls,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            cls,
            windows::core::w!("transom-notch"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        register(hwnd);
        NOTCH.with(|n| {
            *n.borrow_mut() = Some(NotchState {
                hwnd,
                x: 0,
                y: 0,
                w: 1,
                h: 1,
                dpi: 96,
                cur: 0.0,
                from: 0.0,
                to: 0.0,
                t0: Instant::now(),
                dur_ms: 0.0,
                glass: false,
                browse: false,
                hover: false,
                lead_color: None,
                placed: (0, 0, 0, 0),
                animating: false,
                cx: 0.0,
                cw: 0.0,
                fx: 0.0,
                fw: 0.0,
            })
        });
        // DWM nezaobluje (rohy kresli sklo alebo oblast okna)
        let round: i32 = 1;
        let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE,
            &round as *const i32 as *const _,
            4,
        );
        Some(hwnd)
    }
}

fn place_for_cursor() -> Option<(i32, i32, i32, i32, u32)> {
    let mut pt = POINT::default();
    // SAFETY: citanie polohy kurzora.
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    place_at(pt)
}

/// Vybezok pod listou monitora s bodom `pt` (stred monitora, predvolena velkost, DPI monitora).
fn place_at(pt: POINT) -> Option<(i32, i32, i32, i32, u32)> {
    // SAFETY: citanie informacii o monitore.
    unsafe {
        let m = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(m, &mut mi).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let w = NOTCH_W_LP * dx as i32 / 96;
        let h = NOTCH_H_LP * dx as i32 / 96;
        let work = mi.rcWork;
        let x = work.left + ((work.right - work.left) - w) / 2;
        let y =
            crate::appbar::bar_bottom_in(mi.rcMonitor.left, mi.rcMonitor.right).unwrap_or(work.top);
        Some((x, y, w, h, dx))
    }
}

/// Vodorovny rozsah pracovnej plochy monitora pod bodom (`x`, `y`).
fn monitor_span(x: i32, y: i32) -> Option<(i32, i32)> {
    // SAFETY: citanie informacii o monitore.
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(m, &mut mi)
            .as_bool()
            .then_some((mi.rcWork.left, mi.rcWork.right))
    }
}

fn animate_to(target: f32, dur: f32) {
    let Some((hwnd, was)) = NOTCH.with(|n| {
        n.borrow_mut().as_mut().map(|s| {
            // mensi obsah (menej suborov): okno sa hned zmensi, animuje sa len rast a zbalenie
            if target > 0.0 && s.cur > target && s.cw.round() as i32 == s.w {
                s.cur = target;
            }
            // na inom monitore (alebo inej vyske listy) sa nepresuva cez obrazovku: zacne znova zhora
            if s.cur > 0.0 && target > 0.0 {
                let (l, r) = monitor_span(s.x + s.w / 2, s.y).unwrap_or((i32::MIN, i32::MAX));
                let (cl, cr) = (s.cx.round() as i32, (s.cx + s.cw).round() as i32);
                if cl < l || cr > r || s.placed.1 != s.y {
                    s.cur = 0.0;
                }
            }
            if s.cur <= 0.0 {
                // z ukrytia: vysuva sa zhora v plnej sirke ciela
                (s.cx, s.cw) = (s.x as f32, s.w as f32);
            }
            (s.fx, s.fw) = (s.cx, s.cw);
            s.from = s.cur;
            s.to = target;
            s.t0 = Instant::now();
            s.dur_ms = dur;
            (s.hwnd, std::mem::replace(&mut s.animating, true))
        })
    }) else {
        return;
    };
    if !was {
        NPACE.with(|p| *p.borrow_mut() = Some(crate::vsync::Pacer::start(hwnd, T_ANIM)));
    }
    if !crate::system::animations_enabled() {
        // obmedzeny pohyb: bez animacie rovno ciel
        NOTCH.with(|n| {
            if let Some(s) = n.borrow_mut().as_mut() {
                s.dur_ms = 0.0;
            }
        });
    }
    notch_tick();
    // spodna ciara listy pod vybezkom (zmizne / vrati sa)
    crate::appbar::invalidate_all();
}

fn notch_tick() {
    let Some((hwnd, x, y, w, h, dpi, cur, to, done, placed, cx, cw, span)) = NOTCH.with(|n| {
        let mut b = n.borrow_mut();
        let s = b.as_mut()?;
        let el = s.t0.elapsed().as_secs_f32() * 1000.0;
        s.cur = notch_height(s.from, s.to, el, s.dur_ms);
        let k = if s.dur_ms <= 0.0 {
            1.0
        } else {
            ease_out(el / s.dur_ms)
        };
        let done = s.dur_ms <= 0.0 || el >= s.dur_ms;
        // pri skryti sa sirka nemeni (zasuva sa len vyska)
        let (tx, tw) = if s.to > 0.0 {
            (s.x as f32, s.w as f32)
        } else {
            (s.fx, s.fw)
        };
        s.cx = s.fx + (tx - s.fx) * k;
        s.cw = s.fw + (tw - s.fw) * k;
        if done {
            s.cur = s.to;
            (s.cx, s.cw) = (tx, tw);
        }
        let span = notch_span(s.fx, s.fw, s.x, s.w, s.h, s.from);
        Some((
            s.hwnd, s.x, s.y, s.w, s.h, s.dpi, s.cur, s.to, done, s.placed, s.cx, s.cw, span,
        ))
    }) else {
        return;
    };
    let finish = |hwnd: HWND| {
        // SAFETY: casovac vlastneho okna.
        unsafe {
            let _ = KillTimer(Some(hwnd), T_ANIM);
        }
        if NOTCH.with(|n| {
            n.borrow_mut()
                .as_mut()
                .is_some_and(|s| std::mem::replace(&mut s.animating, false))
        }) {
            NPACE.with(|p| p.borrow_mut().take());
        }
    };
    // SAFETY: zobrazenie / oblast vlastneho okna bez aktivacie.
    unsafe {
        if cur.round() as i32 <= 0 && to <= 0.0 {
            finish(hwnd);
            let _ = ShowWindow(hwnd, SW_HIDE);
            NOTCH.with(|n| {
                if let Some(s) = n.borrow_mut().as_mut() {
                    s.browse = false;
                    s.hover = false;
                    s.placed = (0, 0, 0, 0);
                    (s.cx, s.cw) = (0.0, 0.0);
                }
            });
            crate::appbar::invalidate_all();
            return;
        }
        let glass = sync_notch_glass(hwnd);
        let end = done && to > 0.0;
        // na konci presne ciel (rohy kresli sklo), pocas animacie zjednotenie zaciatku a ciela
        let want = if end {
            (x, y, w, h)
        } else {
            (span.0, y, span.1, span.2)
        };
        let vis = (cur.round() as i32).min(want.3);
        let rx = cx.round() as i32 - want.0;
        let rw = (cw.round() as i32).min(want.2);
        if placed != want {
            // najprv oblast (viditelna len cast), potom poloha a okamzite vykreslenie - ziadny zablesk
            notch_region(hwnd, rx, rw, vis, dpi, end && glass);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                want.0,
                want.1,
                want.2,
                want.3,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            NOTCH.with(|n| {
                if let Some(s) = n.borrow_mut().as_mut() {
                    s.placed = want;
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
        } else {
            notch_region(hwnd, rx, rw, vis, dpi, end && glass);
        }
        if done {
            finish(hwnd);
        }
    }
}

/// Prezeranie odlozenych suborov: `Some(h_lp)` rozbali vybezok pod stredom listy (vyska podla obsahu, bez aktivacie
/// okna), `None` ho po kratkej odlozke zbali - ak sa mys medzitym nepresunie na vybezok.
pub fn notch_browse(h_lp: Option<i32>) {
    notch_browse_at(None, h_lp);
}

/// Ako [`notch_browse`], ale vybezok je pod bodom `anchor_x` (x na obrazovke, napr. stred polozky listy; `None` = stred
/// monitora), vzdy cely na monitore.
pub fn notch_browse_at(anchor_x: Option<i32>, h_lp: Option<i32>) {
    match h_lp {
        Some(h_lp) => {
            let Some(hwnd) = ensure_notch() else { return };
            // SAFETY: zrusenie odlozeneho zbalenia vlastneho okna.
            unsafe {
                let _ = KillTimer(Some(hwnd), T_GRACE);
            }
            // pod kotvou (bod v liste, na ktory sa kliklo / ukazalo), inak pod kurzorom
            let at = anchor_x.and_then(|ax| {
                crate::appbar::bar_bottom_in(ax, ax + 1).map(|b| POINT { x: ax, y: b - 1 })
            });
            let Some((mut x, y, w, _, dpi)) = (match at {
                Some(pt) => place_at(pt),
                None => place_for_cursor(),
            }) else {
                return;
            };
            if let Some(ax) = anchor_x {
                // monitor: stred z place_for_cursor = (lavy okraj + sirka/2); posun na kotvu, ale nie mimo monitora
                let (left, right) = monitor_span(ax, y).unwrap_or((x, x + w));
                x = (ax - w / 2).clamp(left, (right - w).max(left));
            }
            let h = h_lp.max(40) * dpi as i32 / 96;
            let (open, same) = NOTCH.with(|n| {
                let mut b = n.borrow_mut();
                let Some(s) = b.as_mut() else {
                    return (false, false);
                };
                let open = s.to > 0.0;
                let same = open && s.h == h && s.x == x;
                if !open {
                    // farba listy tesne nad vybezkom, kym este nie je vysunuty
                    let gap = (2 * dpi as i32 / 96).max(2);
                    s.lead_color = crate::glass::screen_color(x, x + w, y - gap);
                }
                s.browse = true;
                (s.x, s.y, s.w, s.h, s.dpi) = (x, y, w, h, dpi);
                (open, same)
            });
            if same {
                // SAFETY: prekreslenie vlastneho okna.
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            } else {
                // rozbalenie (alebo zmena vysky pri zmene poctu suborov)
                let _ = open;
                animate_to(h as f32, EXPAND_MS);
            }
        }
        None => {
            if NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| s.browse && !s.hover)) {
                notch_leave();
            }
        }
    }
}

/// Zbali vybezok hned (otvara sa panel alebo menu, ktore by sa s nim prekryvalo).
pub fn notch_collapse() {
    if NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| s.hover)) {
        NOTCH.with(|n| {
            if let Some(s) = n.borrow_mut().as_mut() {
                s.hover = false;
            }
        });
    }
    notch_collapse_now();
}

/// Je vybezok otvoreny v rezime prezerania odlozenych suborov?
pub fn notch_browsing() -> bool {
    NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| s.browse && s.to > 0.0))
}

/// Prekresli vybezok (zmena zvyraznenia alebo zoznamu).
pub fn invalidate_notch() {
    if let Some(h) = NOTCH.with(|n| n.borrow().as_ref().map(|s| s.hwnd)) {
        // SAFETY: prekreslenie vlastneho okna.
        unsafe {
            let _ = InvalidateRect(Some(h), None, false);
        }
    }
}

fn notch_enter() {
    let Some(hwnd) = ensure_notch() else { return };
    // SAFETY: zrusenie odlozeneho zbalenia (grace) - kurzor sa len presunul medzi listou a vybezkom.
    unsafe {
        let _ = KillTimer(Some(hwnd), T_GRACE);
    }
    let Some((x, y, w, h, dpi)) = place_for_cursor() else {
        return;
    };
    let already = NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| s.to > 0.0));
    NOTCH.with(|n| {
        if let Some(s) = n.borrow_mut().as_mut() {
            s.browse = false;
            s.x = x;
            s.y = y;
            s.w = w;
            s.h = h;
            s.dpi = dpi;
        }
    });
    if already {
        // kurzor presiel na lištu na inom monitore: vybezok sa presunie hned
        notch_tick();
    } else {
        animate_to(h as f32, EXPAND_MS);
    }
}

/// Kurzor opustil listu / vybezok: zbalit az po kratkej odlozke (prechod lista -> vybezok neblika).
fn notch_leave() {
    if let Some(hwnd) = NOTCH.with(|n| n.borrow().as_ref().map(|s| s.hwnd)) {
        // SAFETY: casovac pre vlastne okno.
        unsafe {
            SetTimer(Some(hwnd), T_GRACE, GRACE_MS, None);
        }
    }
}

fn notch_collapse_now() {
    if NOTCH.with(|n| {
        n.borrow()
            .as_ref()
            .is_some_and(|s| s.to > 0.0 || s.cur > 0.0)
    }) {
        if let Some(hwnd) = NOTCH.with(|n| n.borrow().as_ref().map(|s| s.hwnd)) {
            // SAFETY: zrusenie odlozeneho zbalenia.
            unsafe {
                let _ = KillTimer(Some(hwnd), T_GRACE);
            }
        }
        animate_to(0.0, COLLAPSE_MS);
    }
}

fn destroy_notch() {
    if let Some(s) = NOTCH.with(|n| n.borrow_mut().take()) {
        if s.animating {
            NPACE.with(|p| p.borrow_mut().take());
        }
        revoke(s.hwnd);
        // SAFETY: zrusenie vlastneho okna.
        unsafe {
            let _ = DestroyWindow(s.hwnd);
        }
    }
}

/// Vodorovny rozsah viditelneho vybezku na obrazovke (x od, x do); `None` = skryty. Lista pod nim nekresli spodnu ciaru.
pub fn notch_extent() -> Option<(i32, i32)> {
    NOTCH.with(|n| {
        n.borrow()
            .as_ref()
            .filter(|s| s.cur > 0.0 || s.to > 0.0)
            .map(|s| (s.cx.round() as i32, (s.cx + s.cw).round() as i32))
    })
}

/// Je vybezok prave viditelny? (diagnostika a testy)
pub fn notch_visible() -> bool {
    NOTCH.with(|n| n.borrow().as_ref().is_some_and(|s| s.cur > 0.0))
}

// ---- tahanie von ----

/// Spusti OLE tahanie suborov z Police (volat z fronty poziadaviek, nie z obsluhy udalosti; blokuje do konca tahania).
/// Povolene efekty len COPY a LINK: ak by cielova aplikacia vratila MOVE, nic sa nemaze (Polica ani povodne subory).
pub fn drag_out(paths: &[String]) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        BHID_DataObject, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHParseDisplayName,
    };
    if paths.is_empty() || !OLE.with(|o| o.get()) {
        return false;
    }
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    for p in paths.iter().take(MAX_DROP_FILES) {
        let w: Vec<u16> = p.encode_utf16().chain(std::iter::once(0)).collect();
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        // SAFETY: retazec je ukonceny nulou; PIDL sa uvolni nizsie.
        if unsafe { SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None) }.is_ok()
            && !pidl.is_null()
        {
            pidls.push(pidl);
        }
    }
    let mut ok = false;
    if !pidls.is_empty() {
        let consts: Vec<*const ITEMIDLIST> =
            pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
        // SAFETY: PIDL-y su platne az do uvolnenia; objekt tahania vznika zo shell poloziek (len cesty, ziadna zmena suborov).
        unsafe {
            if let Ok(arr) = SHCreateShellItemArrayFromIDLists(&consts) {
                if let Ok(data) = arr.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) {
                    DRAGGING.with(|d| d.set(true));
                    let r = SHDoDragDrop(
                        None,
                        &data,
                        None,
                        DROPEFFECT_COPY | windows::Win32::System::Ole::DROPEFFECT_LINK,
                    );
                    DRAGGING.with(|d| d.set(false));
                    ok = r.is_ok();
                }
            }
        }
    }
    for p in pidls {
        // SAFETY: PIDL pridelene shellom (CoTaskMem).
        unsafe { CoTaskMemFree(Some(p as *const _)) };
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wellformed_hdrop_is_parsed_and_a_roundtrip_through_build_hdrop_is_exact() {
        let paths = vec![
            r"C:\a\b.txt".to_string(),
            r"\\srv\share\x y.pdf".to_string(),
            "C:\\Ľúbo\\č.docx".to_string(),
        ];
        assert_eq!(parse_hdrop(&build_hdrop(&paths)), paths);
        assert!(parse_hdrop(&build_hdrop(&[])).is_empty());
    }

    #[test]
    fn hostile_or_malformed_blocks_never_panic_and_never_exceed_the_limits() {
        // prilis kratke, zly offset, ANSI zoznam, bez ukoncenia
        assert!(parse_hdrop(&[]).is_empty());
        assert!(parse_hdrop(&[0; 19]).is_empty());
        let mut b = build_hdrop(&["C:\\a".into()]);
        b[0] = 200; // pFiles za koncom
        assert!(parse_hdrop(&b).is_empty());
        let mut b = build_hdrop(&["C:\\a".into()]);
        b[16] = 0; // nie Unicode
        assert!(parse_hdrop(&b).is_empty());
        let mut b = build_hdrop(&["C:\\a".into()]);
        b[0] = 5; // pFiles do hlavicky
        assert!(parse_hdrop(&b).is_empty());
        // bez koncoveho dvojiteho NUL: posledna necela cesta sa nezapocita
        let mut b = build_hdrop(&["C:\\a".into(), "C:\\b".into()]);
        b.truncate(b.len() - 4);
        assert_eq!(parse_hdrop(&b), vec!["C:\\a".to_string()]);
        // nepary pocet bajtov a neplatne UTF-16 (osamely surogat)
        let mut b = build_hdrop(&["C:\\a".into()]);
        b.push(1);
        assert_eq!(parse_hdrop(&b), vec!["C:\\a".to_string()]);
        let mut bad = vec![0u8; 20];
        bad[0] = 20;
        bad[16] = 1;
        bad.extend_from_slice(&[0x00, 0xD8, 0x00, 0x00, 0x00, 0x00]);
        assert!(parse_hdrop(&bad).is_empty());
        // miliony poloziek: orezane na limit
        let many: Vec<String> = (0..2000).map(|i| format!("C:\\f\\{i}")).collect();
        assert_eq!(parse_hdrop(&build_hdrop(&many)).len(), MAX_DROP_FILES);
        // blok nad limit objemu sa zahodi cely
        let huge = vec![1u8; MAX_DROP_BYTES + 1];
        assert!(parse_hdrop(&huge).is_empty());
        // nezmyselne dlha cesta
        let long = vec!["x".repeat(MAX_PATH_UNITS + 5), "C:\\ok".into()];
        assert!(
            parse_hdrop(&build_hdrop(&long)).is_empty(),
            "po nezmyselnej ceste sa zvysok nepouzije"
        );
    }

    #[test]
    fn the_drop_effect_is_copy_or_none_and_never_move() {
        let all = DROPEFFECT(1 | 2 | 4);
        assert_eq!(effect_for(all, true), DROPEFFECT_COPY);
        assert_eq!(effect_for(all, false), DROPEFFECT_NONE);
        assert_eq!(
            effect_for(DROPEFFECT(2), true),
            DROPEFFECT_NONE,
            "zdroj dovoluje len presun: odmietnut"
        );
        assert_eq!(effect_for(DROPEFFECT(4), true), DROPEFFECT_NONE);
        for allowed in 0..8u32 {
            for acc in [true, false] {
                assert_ne!(effect_for(DROPEFFECT(allowed), acc).0 & 2, 2, "nikdy MOVE");
            }
        }
    }

    #[test]
    fn the_notch_animation_is_monotonic_bounded_and_ends_exactly_at_the_target() {
        assert_eq!(notch_height(0.0, 100.0, 0.0, 180.0), 0.0);
        assert_eq!(notch_height(0.0, 100.0, 180.0, 180.0), 100.0);
        assert_eq!(notch_height(0.0, 100.0, 9999.0, 180.0), 100.0);
        assert_eq!(
            notch_height(40.0, 0.0, 140.0, 140.0),
            0.0,
            "zbalenie z polovice"
        );
        assert_eq!(
            notch_height(0.0, 100.0, 50.0, 0.0),
            100.0,
            "bez animacie rovno ciel"
        );
        let mut last = -1.0;
        for ms in 0..=180 {
            let h = notch_height(0.0, 100.0, ms as f32, 180.0);
            assert!((0.0..=100.0).contains(&h) && h >= last, "{ms}: {h}");
            last = h;
        }
    }

    /// Registracia ciela pustenia na skrytom okne s rovnakymi stylmi ako lista (bez aktivacie, vrstvene, topmost); druha
    /// registracia tej istej HWND zlyha, po `revoke` sa da registrovat znova. Nic sa nezobrazi.
    #[test]
    fn a_drop_target_can_be_registered_on_a_non_activating_layered_topmost_window_and_revoked() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::WS_EX_LAYERED;
        std::thread::spawn(|| {
            assert!(init(), "OleInitialize na STA vlakne");
            // SAFETY: skryte testovacie okno na tomto vlakne.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                    w!("STATIC"),
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    10,
                    10,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("skryte okno")
            };
            assert!(register(hwnd));
            assert!(
                !register(hwnd),
                "druha registracia tej istej HWND zlyha (uz registrovane)"
            );
            revoke(hwnd);
            assert!(register(hwnd), "po revoke sa da znova");
            revoke(hwnd);
            // SAFETY: zrusenie testovacieho okna.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            uninit();
        })
        .join()
        .unwrap();
    }

    /// Zdroj pustenia vrati namiesto globalnej pamate stream: `u.hGlobal` by bol nezmyselny ukazovatel. Musi sa ignorovat bez
    /// pada a medium sa musi uvolnit.
    #[test]
    fn a_dropped_object_with_a_non_global_memory_medium_is_ignored_safely() {
        use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
        use windows::Win32::System::Com::{
            IDataObject_Impl, STGMEDIUM, STGMEDIUM_0, TYMED_ISTREAM,
        };
        #[implement(IDataObject)]
        struct Hostile;
        #[allow(non_snake_case)]
        impl IDataObject_Impl for Hostile_Impl {
            fn GetData(&self, _: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
                // SAFETY: stream nad novou globalnou pamatou; vlastnictvo prechadza na prijemcu mediumu.
                let stream = unsafe {
                    CreateStreamOnHGlobal(windows::Win32::Foundation::HGLOBAL::default(), true)?
                };
                Ok(STGMEDIUM {
                    tymed: TYMED_ISTREAM.0 as u32,
                    u: STGMEDIUM_0 {
                        pstm: std::mem::ManuallyDrop::new(Some(stream)),
                    },
                    pUnkForRelease: std::mem::ManuallyDrop::new(None),
                })
            }
            fn GetDataHere(
                &self,
                _: *const FORMATETC,
                _: *mut STGMEDIUM,
            ) -> windows::core::Result<()> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn QueryGetData(&self, _: *const FORMATETC) -> windows::core::HRESULT {
                windows::Win32::Foundation::S_OK
            }
            fn GetCanonicalFormatEtc(
                &self,
                _: *const FORMATETC,
                _: *mut FORMATETC,
            ) -> windows::core::HRESULT {
                windows::Win32::Foundation::E_NOTIMPL
            }
            fn SetData(
                &self,
                _: *const FORMATETC,
                _: *const STGMEDIUM,
                _: windows::core::BOOL,
            ) -> windows::core::Result<()> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn EnumFormatEtc(
                &self,
                _: u32,
            ) -> windows::core::Result<windows::Win32::System::Com::IEnumFORMATETC> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn DAdvise(
                &self,
                _: *const FORMATETC,
                _: u32,
                _: windows::core::Ref<'_, windows::Win32::System::Com::IAdviseSink>,
            ) -> windows::core::Result<u32> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn EnumDAdvise(
                &self,
            ) -> windows::core::Result<windows::Win32::System::Com::IEnumSTATDATA> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
        }
        let d: IDataObject = Hostile.into();
        assert!(read_files(&d).is_empty());
        assert!(is_hglobal_medium(TYMED_HGLOBAL.0 as u32));
        assert!(!is_hglobal_medium(TYMED_ISTREAM.0 as u32));
        assert!(!is_hglobal_medium(0));
    }

    /// Priame volanie `IDropTarget` so syntetickym `IDataObject` (CF_HDROP v pamati): ziadna mys, ziadna plocha. Over efekt a ze
    /// ne-HDROP objekt sa odmietne.
    #[test]
    fn the_drop_target_accepts_only_file_lists_answers_copy_and_queues_the_dropped_paths() {
        use windows::Win32::System::Com::{IDataObject_Impl, STGMEDIUM, STGMEDIUM_0};
        use windows::Win32::System::Memory::{GlobalAlloc, GMEM_MOVEABLE};
        use windows::Win32::System::Ole::{OleInitialize as Ole, OleUninitialize as Unole};
        #[implement(IDataObject)]
        struct Fake {
            files: Option<Vec<u8>>,
        }
        #[allow(non_snake_case)]
        impl IDataObject_Impl for Fake_Impl {
            fn GetData(&self, f: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
                // SAFETY: platny FORMATETC od volajuceho; alokacia globalnej pamate s kopiou bloku.
                unsafe {
                    let fe = &*f;
                    let Some(bytes) = self.files.as_ref().filter(|_| fe.cfFormat == CF_HDROP.0)
                    else {
                        return Err(windows::Win32::Foundation::DV_E_FORMATETC.into());
                    };
                    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
                    let p = GlobalLock(h) as *mut u8;
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                    let _ = GlobalUnlock(h);
                    Ok(STGMEDIUM {
                        tymed: TYMED_HGLOBAL.0 as u32,
                        u: STGMEDIUM_0 { hGlobal: h },
                        pUnkForRelease: std::mem::ManuallyDrop::new(None),
                    })
                }
            }
            fn GetDataHere(
                &self,
                _: *const FORMATETC,
                _: *mut STGMEDIUM,
            ) -> windows::core::Result<()> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn QueryGetData(&self, f: *const FORMATETC) -> windows::core::HRESULT {
                // SAFETY: platny FORMATETC od volajuceho.
                let ok = self.files.is_some() && unsafe { (*f).cfFormat } == CF_HDROP.0;
                if ok {
                    windows::Win32::Foundation::S_OK
                } else {
                    windows::Win32::Foundation::DV_E_FORMATETC
                }
            }
            fn GetCanonicalFormatEtc(
                &self,
                _: *const FORMATETC,
                _: *mut FORMATETC,
            ) -> windows::core::HRESULT {
                windows::Win32::Foundation::E_NOTIMPL
            }
            fn SetData(
                &self,
                _: *const FORMATETC,
                _: *const STGMEDIUM,
                _: windows::core::BOOL,
            ) -> windows::core::Result<()> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn EnumFormatEtc(
                &self,
                _: u32,
            ) -> windows::core::Result<windows::Win32::System::Com::IEnumFORMATETC> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn DAdvise(
                &self,
                _: *const FORMATETC,
                _: u32,
                _: windows::core::Ref<'_, windows::Win32::System::Com::IAdviseSink>,
            ) -> windows::core::Result<u32> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
            fn EnumDAdvise(
                &self,
            ) -> windows::core::Result<windows::Win32::System::Com::IEnumSTATDATA> {
                Err(windows::Win32::Foundation::E_NOTIMPL.into())
            }
        }
        std::thread::spawn(|| {
            // SAFETY: OLE na testovacom vlakne.
            unsafe { Ole(None).unwrap() };
            let target: IDropTarget = DropTarget {
                accepting: Cell::new(false),
            }
            .into();
            let files: IDataObject = Fake {
                files: Some(build_hdrop(&[
                    r"C:\a\b.txt".to_string(),
                    r"C:\c\d.pdf".to_string(),
                ])),
            }
            .into();
            let text_only: IDataObject = Fake { files: None }.into();
            let pt = POINTL { x: 0, y: 0 };
            // SAFETY: priame volania rozhrania s platnymi ukazovatelmi.
            unsafe {
                // objekt bez zoznamu suborov: efekt NONE a pustenie nic nezaradi
                let mut eff = DROPEFFECT(1 | 2 | 4);
                target
                    .DragEnter(&text_only, MODIFIERKEYS_FLAGS(0), pt, &mut eff)
                    .unwrap();
                assert_eq!(eff, DROPEFFECT_NONE);
                target.DragLeave().unwrap();
                // zoznam suborov: COPY aj ked zdroj dovoluje presun
                let mut eff = DROPEFFECT(1 | 2 | 4);
                target
                    .DragEnter(&files, MODIFIERKEYS_FLAGS(0), pt, &mut eff)
                    .unwrap();
                assert_eq!(eff, DROPEFFECT_COPY);
                let mut eff = DROPEFFECT(1 | 2);
                target
                    .DragOver(MODIFIERKEYS_FLAGS(0), pt, &mut eff)
                    .unwrap();
                assert_eq!(eff, DROPEFFECT_COPY);
                let mut eff = DROPEFFECT(1 | 2);
                target
                    .Drop(&files, MODIFIERKEYS_FLAGS(0), pt, &mut eff)
                    .unwrap();
                assert_eq!(eff, DROPEFFECT_COPY, "nikdy MOVE");
            }
            let queued: Vec<Vec<String>> = QUEUE.with(|c| {
                c.borrow()
                    .iter()
                    .filter_map(|q| {
                        if let Q::Drop(p) = q {
                            Some(p.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            assert_eq!(
                queued,
                vec![vec![r"C:\a\b.txt".to_string(), r"C:\c\d.pdf".to_string()]]
            );
            // pustenie bez predchadzajuceho DragEnter (alebo odmietnuty objekt) nezaradi nic
            QUEUE.with(|c| c.borrow_mut().clear());
            let mut eff = DROPEFFECT(1);
            // SAFETY: platne ukazovatele.
            unsafe {
                target
                    .Drop(&files, MODIFIERKEYS_FLAGS(0), pt, &mut eff)
                    .unwrap()
            };
            assert_eq!(eff, DROPEFFECT_NONE);
            assert!(QUEUE.with(|c| c.borrow().iter().all(|q| !matches!(q, Q::Drop(_)))));
            // SAFETY: parter k `OleInitialize`.
            unsafe { Unole() };
        })
        .join()
        .unwrap();
    }
}
