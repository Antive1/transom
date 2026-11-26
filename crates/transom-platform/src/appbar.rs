//! AppBar hostitel: jedna plnosirkova lista (AppBar na hornej hrane) na vybrany monitor.
//!
//! Zlucene poznatky zo spikov:
//! - S1: poradie `ABM_NEW` -> `ABM_QUERYPOS` -> `ABM_SETPOS` -> `MoveWindow`; rezervacia sa pri pade procesu uvolni
//!   okamzite; `ABN_POSCHANGED` chodi aj ako spatna vazba na vlastny `SETPOS` -> `SETPOS` sa neposiela, ak sa
//!   obdlznik nezmenil.
//! - S2: obdlznik sa pocita z `rcMonitor` (nie z navrhu `WM_DPICHANGED`); monitor sa identifikuje cez interface path;
//!   `WM_DISPLAYCHANGE` (aj pri zmene DPI) spusta `reconcile` (zrusi/vytvori listy); broadcast `WM_DISPLAYCHANGE`
//!   dostavaju len top-level okna, preto je riadiace okno skryty top-level, nie message-only.
//! - S3: `TaskbarCreated` -> znova `ABM_NEW` + `SETPOS`; `ABN_FULLSCREENAPP` -> skryt/zobrazit len listu dotknuteho
//!   monitora (hide); po navrate sa nastavi znova `HWND_TOPMOST`.
//!
//! Vlastnictvo: vsetok stav patri vlaknu, na ktorom bol `BarHost` vytvoreny (thread-local), nie globalnym zamkom.
//! Win32 volania, ktore synchronne posielaju spravy do vlastnych wndprocs, sa NIKDY nevolaju pocas vypozicky stavu.
use crate::layered::{Area, BarAppearance, LayeredBuf};
use crate::monitor::{self, MonitorInfo, MonitorSelection, Rect};
use crate::power::PowerNotifications;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use windows::core::Interface;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetDC, IntersectClipRect, InvalidateRect, ReleaseDC, RestoreDC, SaveDC,
    HDC, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{IAccessible, LresultFromObject};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABN_FULLSCREENAPP,
    ABN_POSCHANGED, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, KillTimer, LoadCursorW, MoveWindow, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, GWL_EXSTYLE, HWND_BOTTOM, HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE, MSG,
    OBJID_CLIENT, PBT_APMPOWERSTATUSCHANGE, PBT_POWERSETTINGCHANGE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_APP,
    WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND,
    WM_GETOBJECT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT,
    WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMECHANGE, WM_TIMER, WM_USER, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Spatne volanie AppBaru (callback sprava) - len pre okna listy.
const WM_APPBAR: u32 = WM_USER + 0x51;
const TIMER_QUIT: usize = 1;
const TIMER_WAKE: usize = 2;
/// Opakovanie neuspesneho pridania ikony v oblasti oznameni.
const TIMER_TRAY_RETRY: usize = 3;
/// Opakovana kontrola listy skrytej vlastnou detekciou plnoobrazovkovej aplikacie (viz `crate::fullscreen`).
const TIMER_FS_RECHECK: usize = 4;
const FS_RECHECK_MS: u32 = 2000;
/// Kontrola polohy mysi pri automatickom skryvani.
const TIMER_AUTOHIDE: usize = 5;
/// Interval kontroly polohy mysi pri automatickom skryvani (ms).
pub const AUTOHIDE_POLL_MS: u32 = 120;
/// Po odchode mysi z listy sa skryje az po tejto dobe (ms) - kratky prechod mimo ju nezhasne.
const AUTOHIDE_DELAY_MS: u64 = 600;
const TRAY_RETRY_MS: u32 = 2000;
/// Pomenovane jednorazove casovace z [`schedule_timer`] maju ID `TIMER_USER_BASE + id`.
const TIMER_USER_BASE: usize = 0x100;
/// Poziadavka na prekreslenie vrstveneho okna (`request_redraw`).
const WM_TRANSOM_REDRAW: u32 = WM_APP + 0x34;
/// Casovac opakovania vrstveneho prekreslenia, ked handler prave bezi (ID v ramci casovacov okna listy).
const TIMER_LAYERED_RETRY: usize = 0x201;
/// Za kolko ms sa vrstvene prekreslenie zopakuje, ak handler prave bezi.
const LAYERED_RETRY_MS: u32 = 15;
/// `PBT_APMRESUMEAUTOMATIC` (wParam `WM_POWERBROADCAST`): system sa prebudil zo spanku.
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
/// Notifikacia z ineho vlakna ([`Notifier`]) -> `BarEvent::Notify(wParam)`.
const WM_TRANSOM_NOTIFY: u32 = WM_APP + 0x31;
/// `WM_MOUSELEAVE` (winuser.h 0x02A3; v `windows` 0.62 ho exportuje len modul Controls).
const WM_MOUSELEAVE: u32 = 0x02A3;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Debug)]
pub struct BarConfig {
    /// Vyska listy v logickych pixeloch (lp); px = lp * dpi / 96.
    pub height_lp: u32,
    pub monitors: MonitorSelection,
    /// Automaticke skryvanie (ochrana OLED): lista nerezervuje miesto (len `ABM_NEW` kvoli oznameniam, bez `ABM_SETPOS`),
    /// je skryta a vysunie sa, ked mys dosiahne horny okraj monitora; skryje sa po odchode mysi (ak nie je otvoreny popover
    /// ani tahanie). Poloha mysi sa kontroluje casovacom [`AUTOHIDE_POLL_MS`] len pocas zapnuteho skryvania.
    pub auto_hide: bool,
}

impl Default for BarConfig {
    fn default() -> Self {
        BarConfig {
            height_lp: 28,
            monitors: MonitorSelection::All,
            auto_hide: false,
        }
    }
}

/// Kontext pre kreslenie jednej listy (v `WM_PAINT`, pri priehladnom pozadi dvakrat - pozri `layered`).
pub struct PaintCtx {
    pub hwnd: HWND,
    pub hdc: HDC,
    pub client: Rect,
    pub dpi: u32,
    pub monitor_id: String,
    /// Priehladny rezim: namiesto farby pozadia temy kreslit na tuto farbu (cierna/biela, pozri `layered`).
    pub backdrop: Option<(u8, u8, u8)>,
    /// Priehladny rezim: text kreslit sivym vyhladzovanim (nie ClearType).
    pub grayscale: bool,
    /// `false` pri druhom prechode priehladneho rezimu: len kreslit, nezapisovat rozlozenie, oblasti ani log.
    pub bookkeeping: bool,
}

/// Udalosti hostitela (diagnostika, neskor invalidacia scény).
#[derive(Clone, Debug)]
pub enum BarEvent {
    /// Lista bola umiestnena (alebo znova umiestnena) na obdlznik; `dpi` je DPI monitora.
    Placed {
        monitor_id: String,
        rect: Rect,
        dpi: u32,
    },
    /// Lista bola vytvorena / zrusena kvoli zmene monitorov.
    Created {
        monitor_id: String,
    },
    Removed {
        monitor_id: String,
    },
    /// `ABN_FULLSCREENAPP`: plnoobrazovkova aplikacia sa otvorila (true) / zatvorila (false) na monitore listy.
    Fullscreen {
        monitor_id: String,
        opened: bool,
    },
    /// Explorer sa restartoval a AppBary sa registrovali znova.
    TaskbarCreated,
    /// Casovac z [`schedule_wake`] vyprsal (jednorazovo).
    Wake,
    /// Zmenil sa systemovy cas / casova zona / regionalny format casu / system sa prebudil zo spanku.
    TimeChanged,
    /// Zmenila sa systemova farebna schema (svetly/tmavy rezim).
    ThemeChanged,
    /// Zmenilo sa systemove nastavenie animacii (`SPI_SETCLIENTAREAANIMATION`) - pozri `system::animations_enabled`.
    AnimationsChanged,
    /// Zmenil sa stav "vybrany monitor nie je pripojeny" (`true` = lista je na primarnom ako zaloha; aplikacia to
    /// viditelne oznami). Vyber v konfiguracii sa nemeni.
    MonitorsFallback(bool),
    /// Ikona v oblasti oznameni: klik / klavesnica (`Select`) alebo kontextove menu (`Menu`); `x`, `y` = bod na obrazovke.
    Tray {
        kind: TrayKind,
        x: i32,
        y: i32,
    },
    /// Zmena napajania (siet/bateria, percento, sporic) - stav si treba precitat znova (`power::read`).
    PowerChanged,
    /// Mys nad listou (suradnice v px klientskej oblasti listy). Kliknutie listu neaktivuje (`WS_EX_NOACTIVATE`).
    Pointer {
        monitor_id: String,
        kind: PointerKind,
        x: i32,
        y: i32,
    },
    /// Pomenovany jednorazovy casovac z [`schedule_timer`] vyprsal.
    Timer(u32),
    /// Notifikacia z ineho vlakna cez [`Notifier`].
    Notify(u32),
    /// Pristupnostny klient (MSAA `accDoDefaultAction`) vyvolal oblast `index` z `regions::set`.
    Invoke {
        monitor_id: String,
        index: usize,
    },
    PopoverOpened {
        kind: u32,
    },
    PopoverClosed {
        kind: u32,
    },
    PopoverPointer {
        kind: u32,
        pointer: PointerKind,
        x: i32,
        y: i32,
    },
    /// Klavesa v popoveri: `shift` = drzany Shift, `repeat` = automaticke opakovanie (drzana klavesa).
    PopoverKey {
        kind: u32,
        vk: u32,
        shift: bool,
        repeat: bool,
        /// Drzany Ctrl (napr. Ctrl+V vloz, Ctrl+Enter).
        ctrl: bool,
    },
    /// Znak napisany v popoveri (`WM_CHAR`, zlozene surrogaty; bez riadiacich znakov okrem `\n` pre Enter v poznamke).
    PopoverChar {
        kind: u32,
        ch: char,
    },
    /// Pustenie suborov na listu alebo vybezok (po spracovani mimo OLE; len absolutne cesty bez kontroly existencie, najviac 500).
    FilesDropped(Vec<String>),
    /// Pustene virtualne subory (nazov uz vycisteny, obsah): aplikacia ich ulozi a prida do odlozenych suborov.
    FileDataDropped(Vec<(String, Vec<u8>)>),
    /// Koliesko mysi nad listou (`delta` v jednotkach `WHEEL_DELTA`, kladne = nahor; x v klientskej ploche listy).
    BarWheel {
        monitor_id: String,
        x: i32,
        delta: i32,
    },
    /// Vysledok kapatka na farbu (`picker::start`): farba bodu pod kliknutim, `None` = zrusene.
    ColorPicked(Option<(u8, u8, u8)>),
    /// Mys vo vybezku odlozenych suborov (rezim prezerania, pozri `dragdrop::notch_browse`).
    NotchPointer {
        kind: PointerKind,
        x: i32,
        y: i32,
    },
    /// Koliesko mysi nad popoverom: `delta` v jednotkach `WHEEL_DELTA` (120 = jeden zub; kladne = nahor).
    PopoverWheel {
        kind: u32,
        delta: i32,
    },
    /// Vysledok `Request::Activate`.
    Activated(crate::activate::Activation),
    /// Zvolena polozka kontextoveho menu.
    MenuChosen(u32),
    /// Registrovana broadcast sprava ([`register_broadcast`]) - napr. zmena konfigurácie z okna nastaveni.
    Broadcast(u32),
    /// Stlacena globalna skratka `id` z [`crate::hotkey::register`].
    Hotkey(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayKind {
    /// Lavy klik alebo Enter/medzernik na ikone.
    Select,
    /// Pravy klik alebo kontextova klavesa.
    Menu,
    /// Klik na oznamenie (bublinu) ikony.
    Balloon,
}

/// Prida/zmeni/odstrani ikonu transom v oblasti oznameni (volat na vlakne hostitela; bez hostitela nic). Vrati, ci je
/// ikona po volani pridana.
pub fn set_tray(spec: Option<crate::tray::TraySpec>) -> bool {
    let ctl = CTL.with(|c| c.get());
    if ctl == 0 {
        return false;
    }
    let added = crate::tray::set(hw(ctl), spec);
    arm_tray_retry(hw(ctl));
    added
}

/// Oznamenie (bublina) pri ikone transom v oblasti oznameni (volat na vlakne hostitela). Vrati, ci sa ukazalo.
pub fn tray_balloon(title: &str, text: &str) -> bool {
    let ctl = CTL.with(|c| c.get());
    ctl != 0 && crate::tray::balloon(hw(ctl), title, text)
}

/// Ak je ikona ziadana a nepodarilo sa ju pridat (napr. start pred panelom uloh), zopakuje sa casovacom (najviac niekolko pokusov).
fn arm_tray_retry(ctl: HWND) {
    if crate::tray::wants_retry() {
        // SAFETY: casovac riadiaceho okna tohto vlakna.
        unsafe { SetTimer(Some(ctl), TIMER_TRAY_RETRY, TRAY_RETRY_MS, None) };
    } else {
        // SAFETY: zrusenie vlastneho casovaca (ak nebezi, nic sa nestane).
        unsafe {
            let _ = KillTimer(Some(ctl), TIMER_TRAY_RETRY);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Move,
    /// Mys opustila listu (`WM_MOUSELEAVE`).
    Leave,
    /// Stlacenie laveho tlacidla (stav "stlacene"; akcia az pri `Click` nad tou istou castou).
    Down,
    /// Uvolnenie laveho tlacidla.
    Click,
    /// Uvolnenie praveho tlacidla (kontextove menu transom).
    RightClick,
}

/// Odovzdava sa hostitelovi; vola sa vzdy na vlakne hostitela. Nesmie z metod volat API hostitela.
pub trait BarHandler {
    fn paint(&mut self, ctx: &PaintCtx);
    /// Vzhlad pozadia listy; alfa < 255 prepne okno na vrstveny rezim (priehladne pozadie), 255 = nepriehladne
    /// s ClearType (povodna cesta). Cita sa pri kazdom prekresleni, takze zmena nastaveni sa prejavi hned.
    fn appearance(&self) -> BarAppearance {
        BarAppearance::OPAQUE
    }
    fn on_event(&mut self, _event: &BarEvent) {}
    /// Kreslenie otvoreneho popoveru (`crate::popup`).
    fn paint_popover(&mut self, _ctx: &crate::popup::PopoverPaint) {}
    /// Sklenene pozadie pre popover `kind` (`None` = nepriehladny). Pri skle sa `paint_popover` vola dvakrat
    /// s `backdrop` (cierne a biele pozadie).
    fn popover_glass(&self, _kind: u32) -> Option<crate::glass::GlassStyle> {
        None
    }
    /// Velkost popoveru (px) pre dpi ciloveho monitora, ak ju urcuje obsah (vlastne menu: z nameranych textov); `hdc` je
    /// obrazovkovy DC na meranie. `None` = popover sa neotvori.
    fn popover_size(
        &mut self,
        _kind: u32,
        _dpi: u32,
        _hdc: windows::Win32::Graphics::Gdi::HDC,
    ) -> Option<(i32, i32)> {
        None
    }
}

struct Bar {
    hwnd: isize,
    id: String,
    last: Option<Rect>,
    dpi: u32,
    /// `TrackMouseEvent(TME_LEAVE)` je aktivny.
    tracking: bool,
    /// Obdlznik monitora listy (pre detekciu plnoobrazovkovej aplikacie).
    mon: Rect,
    /// Lista vznikla, kym uz plnoobrazovkova aplikacia bezala (`ABN_FULLSCREENAPP` pride len pri zmene stavu): je skryta a
    /// kontroluje sa opakovane, kym aplikacia neskonci. Prijaty `ABN_FULLSCREENAPP` ju preberie (prizvuk systemu ma prednost).
    fs_check: bool,
    /// Na monitore listy bezi aplikacia na celu obrazovku (podla `ABN_FULLSCREENAPP` / vlastnej kontroly).
    fs: bool,
    /// Automaticke skryvanie: lista je prave vysunuta.
    revealed: bool,
    /// Automaticke skryvanie: kedy sa vysunuta lista skryje (mys je mimo nej); `None` = netreba.
    hide_at: Option<std::time::Instant>,
}

struct State {
    config: BarConfig,
    bars: Vec<Bar>,
    /// Uzivatel skryl listu (Skryt listu): vsetky listy zanikli (ABM_REMOVE, pracovna plocha sa uvolnila).
    hidden: bool,
    /// Posledny stav "vybrany monitor nie je pripojeny" (lista je na primarnom) - hlasi sa len pri zmene.
    fallback: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static HANDLER: RefCell<Option<Box<dyn BarHandler>>> = const { RefCell::new(None) };
    /// Riadiace okno (pre `schedule_wake`); 0 = neexistuje.
    static CTL: Cell<isize> = const { Cell::new(0) };
    static POWER: RefCell<Option<PowerNotifications>> = const { RefCell::new(None) };
    /// Registrovane broadcast spravy (id spravy), index = hodnota v `BarEvent::Broadcast`.
    static BROADCASTS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// Listy vo vrstvenom rezime (priehladne pozadie): povrchy a cakajuca neplatna oblast.
    static LAYERED: RefCell<HashMap<isize, LayeredState>> = RefCell::new(HashMap::new());
}

/// Stav vrstveneho okna: povrchy (vytvoria sa pri prvom kresleni) a zlucena neplatna oblast; `pending` = uz je
/// naplanovana sprava `WM_TRANSOM_REDRAW` (viac poziadaviek sa zluci do jedneho prekreslenia).
#[derive(Default)]
struct LayeredState {
    buf: Option<LayeredBuf>,
    dirty: Option<Area>,
    pending: bool,
    /// Sklo: okno **nie je** vrstvene (to by vyplo rozmazanie), obsah sa posiela `BitBlt` do okna a pod nim je
    /// akrylove rozmazanie; inak vrstvene okno (`UpdateLayeredWindow`).
    glass: bool,
}

/// Poziadavka na prekreslenie listy (`rect` = klientske px, `None` = cela). Nepriehladna lista: `InvalidateRect`
/// (-> `WM_PAINT`, nezmenene). Vrstvene okno `WM_PAINT` nedostava, preto sa poziadavka zluci a odosle ako vlastna
/// sprava (nikdy sa nekresli priamo: volajuci moze drzat vypozicku handlera).
fn request_redraw(raw: isize, rect: Option<RECT>) {
    let area = rect
        .map(|r| Area {
            x0: r.left,
            y0: r.top,
            x1: r.right,
            y1: r.bottom,
        })
        .unwrap_or(Area::full(i32::MAX, i32::MAX));
    let post = LAYERED.with(|m| {
        m.borrow_mut().get_mut(&raw).map(|s| {
            s.dirty = Some(s.dirty.map_or(area, |d| d.union(area)));
            !std::mem::replace(&mut s.pending, true)
        })
    });
    match post {
        Some(true) => {
            // SAFETY: PostMessageW len s cislami na nase okno (moze uz neexistovat: chyba bez nasledkov).
            let _ = unsafe { PostMessageW(Some(hw(raw)), WM_TRANSOM_REDRAW, WPARAM(0), LPARAM(0)) };
        }
        Some(false) => {}
        None => {
            // SAFETY: obdlznik (ak je) je lokalny a zije pocas volania.
            let _ = unsafe {
                InvalidateRect(Some(hw(raw)), rect.as_ref().map(|r| r as *const _), false)
            };
        }
    }
}

fn handler_appearance() -> Option<BarAppearance> {
    HANDLER.with(|c| {
        c.try_borrow()
            .ok()
            .and_then(|g| g.as_ref().map(|h| h.appearance()))
    })
}

/// Prepne okno na priehladny rezim: vrstvene okno (`WS_EX_LAYERED`; obsah sa posle az prvym `UpdateLayeredWindow`,
/// bez neho by okno ostalo neviditelne), alebo sklo (`glass`: alfa pixelov cez DWM, okno ostava nevrstvene).
/// Pri zmene rezimu sa najprv vrati do nepriehladneho stavu.
fn enter_layered(raw: isize, glass: bool) {
    match LAYERED.with(|m| m.borrow().get(&raw).map(|s| s.glass)) {
        Some(g) if g == glass => return,
        Some(_) => leave_layered(raw),
        None => {}
    }
    let hwnd = hw(raw);
    if glass {
        crate::glass::set_glass(hwnd, true);
    } else {
        // SAFETY: zmena stylu nasho okna na jeho vlakne.
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        }
    }
    LAYERED.with(|m| {
        m.borrow_mut().insert(
            raw,
            LayeredState {
                glass,
                ..Default::default()
            },
        )
    });
}

/// Spat na nepriehladne okno (ClearType): odobrat `WS_EX_LAYERED` / vypnut rozmazanie a vyziadat `WM_PAINT`.
fn leave_layered(raw: isize) {
    let Some(st) = LAYERED.with(|m| m.borrow_mut().remove(&raw)) else {
        return;
    };
    let hwnd = hw(raw);
    if st.glass {
        crate::glass::set_glass(hwnd, false);
    }
    // SAFETY: zmena stylu nasho okna na jeho vlakne.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED.0 as isize));
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Zopakuje vrstvene prekreslenie o `LAYERED_RETRY_MS` (zachova `pending`, aby sa ine poziadavky len zlucili).
fn retry_layered(hwnd: HWND) {
    let raw = hwnd.0 as isize;
    LAYERED.with(|m| {
        if let Some(s) = m.borrow_mut().get_mut(&raw) {
            s.pending = true;
            if s.dirty.is_none() {
                s.dirty = Some(Area::full(i32::MAX, i32::MAX));
            }
        }
    });
    // SAFETY: casovac na nase okno; parametre su cisla.
    unsafe {
        SetTimer(Some(hwnd), TIMER_LAYERED_RETRY, LAYERED_RETRY_MS, None);
    }
}

/// Jedno prekreslenie vrstveneho okna: dva prechody sceny (cierne a biele pozadie), zlozenie alfy a
/// `UpdateLayeredWindow` len pre neplatnu oblast.
fn layered_redraw(hwnd: HWND) {
    let raw = hwnd.0 as isize;
    let Some(app) = handler_appearance() else {
        // handler prave bezi (re-entrancia): neplatna oblast ostava, skusit o chvilu znova cez kratky casovac
        // (okamzity opakovany `PostMessage` by sa v vnorenej slucke sprav mohol tocit dokola)
        retry_layered(hwnd);
        return;
    };
    if app.is_opaque() {
        leave_layered(raw);
        return;
    }
    if LAYERED.with(|m| m.borrow().get(&raw).map(|s| s.glass)) != Some(app.glass.is_some()) {
        // zmena rezimu (sklo <-> vrstvene) za behu: prepnut a prekreslit cele
        enter_layered(raw, app.glass.is_some());
        request_redraw(raw, None);
        return;
    }
    let Some((id, dpi)) = bar_info(raw) else {
        return;
    };
    let mut rc = RECT::default();
    // SAFETY: vystupny RECT je lokalny.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let (w, h) = (rc.right, rc.bottom);
    let Some((mut buf, dirty)) = LAYERED.with(|m| {
        m.borrow_mut().get_mut(&raw).map(|s| {
            s.pending = false;
            (s.buf.take(), s.dirty.take())
        })
    }) else {
        return;
    };
    let mut dirty = dirty.unwrap_or(Area::full(i32::MAX, i32::MAX));
    if buf.as_ref().is_none_or(|b| (b.w, b.h) != (w, h)) {
        buf = LayeredBuf::new(w, h);
        dirty = Area::full(i32::MAX, i32::MAX);
    }
    if let (Some(b), Some(area)) = (buf.as_mut(), dirty.clip(w, h)) {
        let client: Rect = rc.into();
        let mut painted = true;
        for (dc, backdrop, first) in [
            (b.black_dc(), (0u8, 0u8, 0u8), true),
            (b.white_dc(), (255u8, 255u8, 255u8), false),
        ] {
            // SAFETY: stav DC sa ulozi a obnovi (SaveDC/RestoreDC).
            unsafe {
                let saved = SaveDC(dc);
                IntersectClipRect(dc, area.x0, area.y0, area.x1, area.y1);
                let ctx = PaintCtx {
                    hwnd,
                    hdc: dc,
                    client,
                    dpi,
                    monitor_id: id.clone(),
                    backdrop: Some(backdrop),
                    grayscale: true,
                    bookkeeping: first,
                };
                let ran = HANDLER.with(|c| {
                    c.try_borrow_mut()
                        .ok()
                        .and_then(|mut g| g.as_mut().map(|h| h.paint(&ctx)))
                        .is_some()
                });
                let _ = RestoreDC(dc, saved);
                painted &= ran;
            }
        }
        if painted {
            if let Some(st) = app.glass {
                b.compose_glass(area, &st, 255);
                // SAFETY: DC okna sa uvolni (ReleaseDC) v tom istom bloku.
                unsafe {
                    let dc = GetDC(Some(hwnd));
                    b.blit(dc, area);
                    ReleaseDC(Some(hwnd), dc);
                }
            } else {
                b.compose(area, app.bg, app.alpha);
                b.present(hwnd, area);
            }
        } else {
            // handler bol vypozicany: oblast sa nestrati, zopakuje sa
            LAYERED.with(|m| {
                if let Some(s) = m.borrow_mut().get_mut(&raw) {
                    s.dirty = Some(s.dirty.map_or(area, |d| d.union(area)));
                }
            });
            retry_layered(hwnd);
        }
    }
    LAYERED.with(|m| {
        if let Some(s) = m.borrow_mut().get_mut(&raw) {
            s.buf = buf;
        }
    });
}

/// Zaregistruje spravu `RegisterWindowMessage(name)`; ked ju riadiace okno dostane (napr. broadcast z okna
/// nastaveni), emituje `BarEvent::Broadcast(index)`. Vrati index.
pub fn register_broadcast(name: &str) -> u32 {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: retazec ukonceny nulou zije pocas volania.
    let id = unsafe { RegisterWindowMessageW(PCWSTR(wide.as_ptr())) };
    BROADCASTS.with(|b| {
        let mut b = b.borrow_mut();
        b.push(id);
        (b.len() - 1) as u32
    })
}

/// Prekresli len vodorovny usek `x .. x + w` listy na monitore `monitor_id` (napr. hodiny pri sekundovom tiku).
pub fn invalidate_span(monitor_id: &str, x: i32, w: i32) {
    if let Some(h) = bar_hwnd(monitor_id) {
        let mut rc = RECT::default();
        // SAFETY: vystupny RECT je lokalny; volania s handle nasho okna.
        unsafe {
            let _ = GetClientRect(h, &mut rc);
            let r = RECT {
                left: x.max(0),
                top: 0,
                right: (x + w).min(rc.right),
                bottom: rc.bottom,
            };
            request_redraw(h.0 as isize, Some(r));
        }
    }
}

/// Ciste ukoncenie z vlakna hostitela (napr. z kontextoveho menu): zrusi listy a ukonci slucku sprav.
pub fn request_quit() {
    if let Some(c) = ctl_hwnd() {
        // SAFETY: PostMessageW len s cislami na nase riadiace okno.
        let _ = unsafe { PostMessageW(Some(c), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

pub(crate) fn ctl_hwnd() -> Option<HWND> {
    let c = CTL.with(|c| c.get());
    (c != 0).then(|| hw(c))
}

/// Monitor (id listy), na ktorom je kurzor mysi; `None`, ak tam lista nie je (alebo ziadna lista neexistuje).
pub fn bar_under_cursor() -> Option<String> {
    let mut pt = windows::Win32::Foundation::POINT::default();
    // SAFETY: vystupny POINT je lokalny.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) }.ok()?;
    st(|s| {
        s.bars
            .iter()
            .find(|b| {
                pt.x >= b.mon.left && pt.x < b.mon.right && pt.y >= b.mon.top && pt.y < b.mon.bottom
            })
            .map(|b| b.id.clone())
    })
    .flatten()
}

pub(crate) fn bar_hwnd(monitor_id: &str) -> Option<HWND> {
    st(|s| {
        s.bars
            .iter()
            .find(|b| b.id == monitor_id)
            .map(|b| hw(b.hwnd))
    })
    .flatten()
}

/// Obdlznik listy na obrazovke a DPI jej monitora.
/// x v klientskej ploche listy monitora `monitor_id` -> x na obrazovke (napr. kotva vybezku pod polozkou).
/// Spodok listy na obrazovke (y).
pub fn bar_bottom(monitor_id: &str) -> Option<i32> {
    bar_geometry(monitor_id).map(|(r, _)| r.bottom)
}

pub fn bar_screen_x(monitor_id: &str, x: i32) -> Option<i32> {
    bar_geometry(monitor_id).map(|(r, _)| r.left + x)
}

/// Spodok viditelnej listy, ktora lezi vo vodorovnom rozsahu `left..right` (vybezok a ostrov visia tesne pod nou, aj
/// ked pracovna plocha este nie je upravena - napr. hned po starte).
pub(crate) fn bar_bottom_in(left: i32, right: i32) -> Option<i32> {
    let bars = st(|s| s.bars.iter().map(|b| b.hwnd).collect::<Vec<_>>())?;
    bars.into_iter().find_map(|h| {
        let mut r = RECT::default();
        // SAFETY: citanie stavu a obdlznika vlastneho okna.
        unsafe {
            if !windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hw(h)).as_bool() {
                return None;
            }
            windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hw(h), &mut r).ok()?;
        }
        (r.left < right && r.right > left).then_some(r.bottom)
    })
}

/// Lista pod kurzorom (inak prva): id monitora a stred listy v jej klientskych suradniciach (kotva panelu v strede).
pub fn bar_center_under_cursor() -> Option<(String, i32)> {
    let bars = st(|s| {
        s.bars
            .iter()
            .map(|b| (b.id.clone(), b.hwnd))
            .collect::<Vec<_>>()
    })?;
    let mut pt = windows::Win32::Foundation::POINT::default();
    // SAFETY: citanie polohy kurzora a obdlznikov vlastnych okien.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
    }
    let rects: Vec<(String, RECT)> = bars
        .into_iter()
        .filter_map(|(id, h)| {
            let mut r = RECT::default();
            // SAFETY: vystupny RECT je lokalny.
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hw(h), &mut r) }
                .ok()
                .map(|()| (id, r))
        })
        .collect();
    let (id, r) = rects
        .iter()
        .find(|(_, r)| pt.x >= r.left && pt.x < r.right)
        .or(rects.first())?;
    Some((id.clone(), (r.right - r.left) / 2))
}

pub(crate) fn bar_geometry(monitor_id: &str) -> Option<(RECT, u32)> {
    let (h, dpi) = st(|s| {
        s.bars
            .iter()
            .find(|b| b.id == monitor_id)
            .map(|b| (b.hwnd, b.dpi))
    })
    .flatten()?;
    let mut r = RECT::default();
    // SAFETY: vystupny RECT je lokalny.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hw(h), &mut r) }.ok()?;
    Some((r, dpi))
}

/// Volanie handlera mimo `emit` (kreslenie popoveru); pri re-entrancii sa preskoci.
pub(crate) fn with_handler(f: impl FnOnce(&mut dyn BarHandler)) {
    HANDLER.with(|c| {
        if let Ok(mut g) = c.try_borrow_mut() {
            if let Some(h) = g.as_mut() {
                f(h.as_mut());
            }
        }
    });
}

/// Naplanuje jednorazovy `BarEvent::Timer(id)` za `delay_ms` (opakovane volanie s rovnakym `id` nahradi plan).
pub fn schedule_timer(id: u32, delay_ms: u32) {
    let ctl = CTL.with(|c| c.get());
    if ctl != 0 {
        // SAFETY: casovac na nase riadiace okno; parametre su cisla.
        unsafe {
            SetTimer(
                Some(hw(ctl)),
                TIMER_USER_BASE + id as usize,
                delay_ms.max(1),
                None,
            )
        };
    }
}

/// Zrusi naplanovany `BarEvent::Timer(id)`.
pub fn cancel_timer(id: u32) {
    let ctl = CTL.with(|c| c.get());
    if ctl != 0 {
        // SAFETY: zrusenie casovaca nasho okna; neexistujuci casovac = chyba bez nasledkov.
        let _ = unsafe { KillTimer(Some(hw(ctl)), TIMER_USER_BASE + id as usize) };
    }
}

thread_local! {
    /// Opakovane `BarEvent::Timer(id)` s obnovou obrazovky (`pace_timer`).
    static PACERS: RefCell<Vec<(u32, crate::vsync::Pacer)>> = const { RefCell::new(Vec::new()) };
}

/// `BarEvent::Timer(id)` pri kazdom obnoveni obrazovky (60 - 240 Hz), kym `on`; dalsia snimka pride az po
/// spracovani predchadzajucej. Na plynule animacie (ekvalizer, posuvanie), len kym bezia.
pub fn pace_timer(id: u32, on: bool) {
    let ctl = CTL.with(|c| c.get());
    PACERS.with(|p| {
        let mut p = p.borrow_mut();
        let has = p.iter().any(|(i, _)| *i == id);
        if on && !has && ctl != 0 {
            p.push((
                id,
                crate::vsync::Pacer::start(hw(ctl), TIMER_USER_BASE + id as usize),
            ));
        } else if !on && has {
            p.retain(|(i, _)| *i != id);
        }
    });
}

/// Posiela `BarEvent::Notify` na vlakno hostitela z lubovolneho vlakna (`PostMessage`). Ak hostitel uz neexistuje,
/// sprava sa ticho zahodi - prijemca ma preto vzdy citat najnovsi zdielany stav, nie obsah notifikacie.
#[derive(Clone, Copy, Debug)]
pub struct Notifier(isize);

impl Notifier {
    pub fn notify(&self, code: u32) {
        // SAFETY: PostMessageW len s cislami.
        let _ = unsafe {
            PostMessageW(
                Some(hw(self.0)),
                WM_TRANSOM_NOTIFY,
                WPARAM(code as usize),
                LPARAM(0),
            )
        };
    }
}

/// Notifier pre aktualneho hostitela (volat na vlakne hostitela, po `BarHost::new`).
pub fn notifier() -> Option<Notifier> {
    let ctl = CTL.with(|c| c.get());
    (ctl != 0).then_some(Notifier(ctl))
}

/// Naplanuje jednorazovy `BarEvent::Wake` za `delay_ms` (opakovane volanie nahradi predchadzajuci plan).
/// Volat na vlakne hostitela; je povolene aj z `BarHandler::on_event`. Bez planu proces nespotrebuva ziadny CPU.
pub fn schedule_wake(delay_ms: u32) {
    let ctl = CTL.with(|c| c.get());
    if ctl != 0 {
        // SAFETY: casovac na nase riadiace okno; parametre su cisla.
        unsafe { SetTimer(Some(hw(ctl)), TIMER_WAKE, delay_ms.max(1), None) };
    }
}

/// Kratka vypozicka stavu; None, ak je stav prave vypozicany (re-entrancia) alebo neexistuje.
fn st<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

const WM_MOUSEWHEEL_MSG: u32 = 0x020A;

pub(crate) fn emit(ev: BarEvent) {
    HANDLER.with(|c| {
        if let Ok(mut g) = c.try_borrow_mut() {
            if let Some(h) = g.as_mut() {
                h.on_event(&ev);
            }
        }
    });
}

fn hw(h: isize) -> HWND {
    HWND(h as *mut _)
}

fn abd(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: WM_APPBAR,
        ..Default::default()
    }
}

pub struct BarHost {
    ctl: HWND,
}

impl BarHost {
    /// Vytvori riadiace okno a listy pre vybrane monitory. Vola sa na vlakne, ktore bude spracuvat spravy (`run`).
    pub fn new(config: BarConfig, handler: Box<dyn BarHandler>) -> windows::core::Result<Self> {
        let already = STATE.with(|c| c.borrow().is_some());
        if already {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_UNEXPECTED,
                "BarHost uz existuje na tomto vlakne",
            ));
        }
        HANDLER.with(|c| *c.borrow_mut() = Some(handler));
        // OLE (cieľ pustenia suborov, tahanie von) bezi na tomto vlakne; zlyhanie len vypne pustenie
        crate::dragdrop::init();
        // SAFETY: registracia triedy a vytvorenie riadiaceho okna na vlakne UI; retazce ziju pocas volani.
        let ctl = unsafe {
            let wide: Vec<u16> = "TaskbarCreated\0".encode_utf16().collect();
            TASKBAR_CREATED.store(
                RegisterWindowMessageW(PCWSTR(wide.as_ptr())),
                Ordering::Relaxed,
            );
            let hinst = GetModuleHandleW(None)?;
            let cur = LoadCursorW(None, IDC_ARROW)?;
            // Opakovana registracia triedy zlyha (uz existuje) - nevadi.
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(bar_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("TransomBar"),
                hCursor: cur,
                ..Default::default()
            });
            crate::popup::register_class(hinst.into());
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(ctl_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("TransomCtl"),
                ..Default::default()
            });
            // Skryty top-level (nie message-only): len top-level okna dostavaju broadcast WM_DISPLAYCHANGE (S2).
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("TransomCtl"),
                w!("transom-ctl"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinst.into()),
                None,
            )?
        };
        STATE.with(|c| {
            *c.borrow_mut() = Some(State {
                config,
                bars: Vec::new(),
                hidden: false,
                fallback: false,
            })
        });
        CTL.with(|c| c.set(ctl.0 as isize));
        // poziadavky zaradene este pred vznikom riadiaceho okna (napr. otvorenie nastaveni pri prvom spusteni) by inak
        // cakali na dalsiu poziadavku: sprava ich spracuje hned po spusteni slucky
        crate::popup::kick_requests();
        let pn = PowerNotifications::register(ctl);
        POWER.with(|c| *c.borrow_mut() = Some(pn));
        reconcile("startup");
        Ok(BarHost { ctl })
    }

    /// Slucka sprav; vrati sa po `quit`/`quit_after`. Bez sprav proces spi (ziadne budenie).
    pub fn run(&self) -> i32 {
        let mut msg = MSG::default();
        // SAFETY: slucka sprav vlakna UI nad lokalnou strukturou MSG.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        msg.wParam.0 as i32
    }

    /// Ciste ukoncenie z lubovolneho vlakna: zrusi rezervacie a ukonci slucku.
    pub fn quit(&self) {
        // SAFETY: PostMessageW len s cislami na nase riadiace okno.
        let _ = unsafe { PostMessageW(Some(self.ctl), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }

    /// Ukonci sa po uplynuti `d` (pre testy/smoke).
    pub fn quit_after(&self, d: Duration) {
        // SAFETY: casovac na nase riadiace okno; parametre su cisla.
        unsafe {
            SetTimer(
                Some(self.ctl),
                TIMER_QUIT,
                d.as_millis().min(u32::MAX as u128) as u32,
                None,
            )
        };
    }

    /// Vyziada prekreslenie vsetkych listov (po zmene scény).
    pub fn invalidate_all(&self) {
        invalidate_all();
    }
}

impl Drop for BarHost {
    fn drop(&mut self) {
        POWER.with(|c| *c.borrow_mut() = None);
        // ikona v oblasti oznameni sa odstrani skor, nez zanikne jej okno (inak by zostal "duch")
        crate::tray::remove(self.ctl);
        remove_all_bars();
        crate::dragdrop::uninit();
        // SAFETY: casovace a okna patria tomuto vlaknu; uvolnuju sa raz pri ukonceni.
        unsafe {
            let _ = KillTimer(Some(self.ctl), TIMER_QUIT);
            let _ = KillTimer(Some(self.ctl), TIMER_WAKE);
            let _ = KillTimer(Some(self.ctl), TIMER_TRAY_RETRY);
            let _ = KillTimer(Some(self.ctl), TIMER_FS_RECHECK);
            let _ = DestroyWindow(self.ctl);
        }
        CTL.with(|c| c.set(0));
        STATE.with(|c| *c.borrow_mut() = None);
        HANDLER.with(|c| *c.borrow_mut() = None);
    }
}

pub fn invalidate_all() {
    let hs: Vec<isize> = st(|s| s.bars.iter().map(|b| b.hwnd).collect()).unwrap_or_default();
    for h in hs {
        request_redraw(h, None);
    }
}

/// QUERYPOS -> SETPOS -> MoveWindow pre jednu listu. `reset` zrusi kontrolu "obdlznik sa nezmenil".
fn set_pos(hwnd_raw: isize, reset: bool) {
    // 1) kratka vypozicka: ID monitora a vyska
    let Some((id, height_lp)) = st(|s| {
        let h = s.config.height_lp;
        s.bars.iter_mut().find(|b| b.hwnd == hwnd_raw).map(|b| {
            if reset {
                b.last = None;
            }
            (b.id.clone(), h)
        })
    })
    .flatten() else {
        return;
    };
    // 2) enumeracia a vypocet bez vypozicky
    let Some(m) = monitor::enumerate().into_iter().find(|m| m.id == id) else {
        return; // monitor zmizol; zosuladi ho `reconcile` pri WM_DISPLAYCHANGE
    };
    let h = monitor::bar_height_px(height_lp, m.dpi);
    let want = monitor::bar_rect(m.rect, h);
    let hwnd = hw(hwnd_raw);
    if st(|s| s.config.auto_hide).unwrap_or(false) {
        // skryvana lista nerezervuje miesto: len poloha nad hornym okrajom monitora (prekryva okna, ked je vysunuta)
        let same = st(|s| {
            s.bars.iter_mut().find(|b| b.hwnd == hwnd_raw).map(|b| {
                b.dpi = m.dpi;
                b.mon = m.rect;
                b.last.replace(want) == Some(want)
            })
        })
        .flatten();
        if same == Some(false) {
            // SAFETY: presun nasho okna na jeho vlakne.
            unsafe {
                let _ = MoveWindow(hwnd, want.left, want.top, want.width(), want.height(), true);
            }
            request_redraw(hwnd_raw, None);
            emit(BarEvent::Placed {
                monitor_id: id,
                rect: want,
                dpi: m.dpi,
            });
        }
        return;
    }
    let mut d = abd(hwnd);
    d.uEdge = ABE_TOP;
    d.rc = want.into();
    // SAFETY: APPBARDATA je lokalna s nastavenym cbSize.
    unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut d) };
    d.rc.bottom = d.rc.top + h;
    let fin: Rect = d.rc.into();
    // 3) kontrola zmeny
    let unchanged = st(|s| {
        s.bars.iter_mut().find(|b| b.hwnd == hwnd_raw).map(|b| {
            b.dpi = m.dpi;
            let same = b.last == Some(fin);
            b.last = Some(fin);
            same
        })
    })
    .flatten();
    match unchanged {
        None | Some(true) => return,
        Some(false) => {}
    }
    // 4) system volania bez vypozicky (synchronne posielaju spravy do nasich wndprocs)
    // SAFETY: APPBARDATA je lokalna s nastavenym cbSize; ziadna vypozicka stavu nezije (system posiela spravy do nasich wndproc).
    unsafe {
        SHAppBarMessage(ABM_SETPOS, &mut d);
        let f: Rect = d.rc.into();
        let _ = MoveWindow(hwnd, f.left, f.top, f.width(), f.height(), true);
        request_redraw(hwnd_raw, None);
        emit(BarEvent::Placed {
            monitor_id: id,
            rect: f,
            dpi: m.dpi,
        });
    }
}

fn create_bar(m: &MonitorInfo) {
    let index = st(|s| s.bars.len()).unwrap_or(0);
    // SAFETY: vytvorenie okna listy na vlakne UI; retazce ziju pocas volani.
    let hwnd = unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return;
        };
        let title: Vec<u16> = format!("transom-bar-{index}\0").encode_utf16().collect();
        match CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            w!("TransomBar"),
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        ) {
            Ok(h) => h,
            Err(_) => return,
        }
    };
    crate::dragdrop::register(hwnd);
    // lista musi byt v stave skor, nez moze prist prva sprava od AppBaru
    st(|s| {
        s.bars.push(Bar {
            hwnd: hwnd.0 as isize,
            id: m.id.clone(),
            last: None,
            dpi: m.dpi,
            tracking: false,
            mon: m.rect,
            fs_check: false,
            fs: false,
            revealed: false,
            hide_at: None,
        })
    });
    let auto_hide = st(|s| s.config.auto_hide).unwrap_or(false);
    let mut d = abd(hwnd);
    // SAFETY: APPBARDATA je lokalna s nastavenym cbSize.
    unsafe { SHAppBarMessage(ABM_NEW, &mut d) };
    set_pos(hwnd.0 as isize, true);
    // Plnoobrazovkova aplikacia, ktora uz bezi: `ABN_FULLSCREENAPP` pride az pri jej zmene, takze lista by ostala viditelna nad hrou.
    let fullscreen = crate::fullscreen::app_on(m.rect);
    // SAFETY: volania s handle nasho okna na jeho vlakne.
    unsafe {
        if auto_hide {
            // skryta, kym mys nedosiahne horny okraj (a nie je tam aplikacia na celu obrazovku)
            let raw = hwnd.0 as isize;
            st(|s| {
                if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
                    b.fs = fullscreen;
                }
            });
            let _ = ShowWindow(hwnd, SW_HIDE);
            arm_autohide();
        } else if fullscreen {
            let raw = hwnd.0 as isize;
            st(|s| {
                if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
                    b.fs_check = true;
                }
            });
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_BOTTOM),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            arm_fs_recheck();
        } else {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
    emit(BarEvent::Created {
        monitor_id: m.id.clone(),
    });
    if fullscreen {
        emit(BarEvent::Fullscreen {
            monitor_id: m.id.clone(),
            opened: true,
        });
    }
}

fn arm_autohide() {
    let ctl = CTL.with(|c| c.get());
    if ctl != 0 {
        // SAFETY: casovac riadiaceho okna; opakovane volanie len prepise ten isty casovac.
        unsafe { SetTimer(Some(hw(ctl)), TIMER_AUTOHIDE, AUTOHIDE_POLL_MS, None) };
    }
}

/// Rozhodnutie automatickeho skryvania pre jednu listu (cista funkcia - testovatelna).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HideStep {
    Nothing,
    Reveal,
    /// Zacat odpocet do skrytia.
    Arm,
    Disarm,
    Hide,
}

/// `at_edge` = mys na hornom okraji monitora listy, `over` = mys nad listou, `keep` = popover / tahanie (lista ostava),
/// `fs` = aplikacia na celu obrazovku, `due` = odpocet vyprsal (None = nebezi).
pub(crate) fn hide_step(
    revealed: bool,
    at_edge: bool,
    over: bool,
    keep: bool,
    fs: bool,
    due: Option<bool>,
) -> HideStep {
    if !revealed {
        return if at_edge && !fs {
            HideStep::Reveal
        } else {
            HideStep::Nothing
        };
    }
    if over || keep {
        return if due.is_some() {
            HideStep::Disarm
        } else {
            HideStep::Nothing
        };
    }
    match due {
        None => HideStep::Arm,
        Some(true) => HideStep::Hide,
        Some(false) => HideStep::Nothing,
    }
}

/// Vysunie listu monitora `monitor_id` (napr. pri otvoreni vyhladavaca skratkou), ak je skryvanie zapnute.
pub(crate) fn reveal(monitor_id: &str) {
    let raw = st(|s| {
        if !s.config.auto_hide {
            return None;
        }
        s.bars.iter_mut().find(|b| b.id == monitor_id).map(|b| {
            let was = b.revealed;
            b.revealed = true;
            b.hide_at = None;
            (!was).then_some(b.hwnd)
        })
    })
    .flatten()
    .flatten();
    if let Some(raw) = raw {
        show_on_top(hw(raw));
    }
}

fn show_on_top(h: HWND) {
    // SAFETY: okno listy je nase; zobrazenie bez aktivacie navrchu.
    unsafe {
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Stav listy pre jeden krok skryvania: okno, monitor, obdlznik listy, vysunuta, cela obrazovka, odpocet.
type HideSnap = (
    isize,
    Rect,
    Option<Rect>,
    bool,
    bool,
    Option<std::time::Instant>,
);

/// Casovac automatickeho skryvania: vysunie listu pri mysi na hornom okraji, skryje ju po odchode mysi.
fn autohide_tick(ctl: HWND) {
    let on = st(|s| s.config.auto_hide && !s.bars.is_empty()).unwrap_or(false);
    if !on {
        // SAFETY: zrusenie vlastneho casovaca.
        unsafe {
            let _ = KillTimer(Some(ctl), TIMER_AUTOHIDE);
        }
        return;
    }
    let mut pt = windows::Win32::Foundation::POINT::default();
    // SAFETY: vystupny bod.
    if unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) }.is_err() {
        return;
    }
    let keep = crate::popup::popover_open().is_some()
        || crate::dragdrop::dragging()
        || crate::dragdrop::notch_visible();
    let now = std::time::Instant::now();
    let bars: Vec<HideSnap> = st(|s| {
        s.bars
            .iter()
            .map(|b| (b.hwnd, b.mon, b.last, b.revealed, b.fs, b.hide_at))
            .collect()
    })
    .unwrap_or_default();
    for (raw, mon, rect, revealed, fs, hide_at) in bars {
        let in_x = pt.x >= mon.left && pt.x < mon.right;
        let at_edge = in_x && pt.y >= mon.top && pt.y <= mon.top + 1;
        let over = rect
            .is_some_and(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom);
        // pred vysunutim skutocna kontrola celej obrazovky (oznamenie systemu mohlo neprist alebo byt zastarane); robi sa len
        // pri mysi na hornom okraji skrytej listy
        let fs = if !revealed && at_edge {
            let now_fs = crate::fullscreen::app_on(mon);
            if now_fs != fs {
                st(|s| {
                    if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
                        b.fs = now_fs;
                    }
                });
            }
            now_fs
        } else {
            fs
        };
        let step = hide_step(revealed, at_edge, over, keep, fs, hide_at.map(|t| now >= t));
        let set = |rev: bool, at: Option<std::time::Instant>| {
            st(|s| {
                if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
                    b.revealed = rev;
                    b.hide_at = at;
                }
            });
        };
        match step {
            HideStep::Nothing => {}
            HideStep::Reveal => {
                set(true, None);
                show_on_top(hw(raw));
            }
            HideStep::Arm => set(
                true,
                Some(now + std::time::Duration::from_millis(AUTOHIDE_DELAY_MS)),
            ),
            HideStep::Disarm => set(true, None),
            HideStep::Hide => {
                set(false, None);
                // SAFETY: okno listy je nase.
                unsafe {
                    let _ = ShowWindow(hw(raw), SW_HIDE);
                }
                emit_leave(raw);
            }
        }
    }
}

/// Skryta lista uz nedostane `WM_MOUSELEAVE`: hover a stlacenie sa zrusia rovnako, ako keby mys odisla.
fn emit_leave(raw: isize) {
    st(|s| {
        if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
            b.tracking = false;
        }
    });
    if let Some((id, _)) = bar_info(raw) {
        emit(BarEvent::Pointer {
            monitor_id: id,
            kind: PointerKind::Leave,
            x: -1,
            y: -1,
        });
    }
}

/// Zapne / vypne automaticke skryvanie: listy sa vytvoria nanovo (s rezervaciou miesta alebo bez nej). Volat cez
/// `popup::Request::SetAutoHide` (rovnako ako `set_hidden`).
pub(crate) fn set_auto_hide(on: bool) {
    if st(|s| std::mem::replace(&mut s.config.auto_hide, on) != on).unwrap_or(false) {
        remove_all_bars();
        reconcile(if on { "autohide on" } else { "autohide off" });
    }
}

fn arm_fs_recheck() {
    let ctl = CTL.with(|c| c.get());
    if ctl != 0 {
        // SAFETY: casovac riadiaceho okna; opakovane volanie len prepise ten isty casovac.
        unsafe { SetTimer(Some(hw(ctl)), TIMER_FS_RECHECK, FS_RECHECK_MS, None) };
    }
}

/// Casovac: listy skryte vlastnou detekciou sa zobrazia, hned ako plnoobrazovkova aplikacia na ich monitore skonci. Casovac
/// bezi len kym je nejaka takato lista (inak sa zrusi), takze v bezne pouzivani nestoji nic.
fn fs_recheck(ctl: HWND) {
    let pending: Vec<(isize, String, Rect)> = st(|s| {
        s.bars
            .iter()
            .filter(|b| b.fs_check)
            .map(|b| (b.hwnd, b.id.clone(), b.mon))
            .collect()
    })
    .unwrap_or_default();
    let mut waiting = false;
    for (raw, id, mon) in pending {
        if crate::fullscreen::app_on(mon) {
            waiting = true;
            continue;
        }
        st(|s| {
            if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
                b.fs_check = false;
            }
        });
        // SAFETY: okno listy je nase; zobrazenie bez aktivacie a navrat na vrch (rovnako ako po `ABN_FULLSCREENAPP` = false).
        unsafe {
            let h = hw(raw);
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                h,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        emit(BarEvent::Fullscreen {
            monitor_id: id,
            opened: false,
        });
    }
    if !waiting {
        // SAFETY: zrusenie vlastneho casovaca.
        unsafe {
            let _ = KillTimer(Some(ctl), TIMER_FS_RECHECK);
        }
    }
}

fn remove_bar(hwnd_raw: isize) {
    crate::popup::close_soon();
    let removed = st(|s| {
        s.bars
            .iter()
            .position(|b| b.hwnd == hwnd_raw)
            .map(|i| s.bars.remove(i))
    })
    .flatten();
    if let Some(b) = removed {
        LAYERED.with(|m| m.borrow_mut().remove(&b.hwnd));
        let hwnd = hw(b.hwnd);
        crate::dragdrop::revoke(hwnd);
        let mut d = abd(hwnd);
        // SAFETY: odhlasenie a znicenie nasho okna na jeho vlakne, raz.
        unsafe {
            SHAppBarMessage(ABM_REMOVE, &mut d);
            let _ = DestroyWindow(hwnd);
        }
        emit(BarEvent::Removed { monitor_id: b.id });
    }
}

fn remove_all_bars() {
    let hs: Vec<isize> = st(|s| s.bars.iter().map(|b| b.hwnd).collect()).unwrap_or_default();
    for h in hs {
        remove_bar(h);
    }
}

/// Zosuladi listy s aktualnymi monitormi: zrusi listy zmiznutych monitorov, vytvori chybajuce, ostatnym obnovi polohu.
fn reconcile(_why: &str) {
    let Some(sel) = st(|s| s.config.monitors.clone()) else {
        return;
    };
    let (mons, fallback) = monitor::select_with_fallback(monitor::enumerate(), &sel);
    let mons = if st(|s| s.hidden).unwrap_or(false) {
        Vec::new()
    } else {
        mons
    };
    let existing: Vec<(isize, String)> =
        st(|s| s.bars.iter().map(|b| (b.hwnd, b.id.clone())).collect()).unwrap_or_default();
    let ids: Vec<String> = existing.iter().map(|(_, id)| id.clone()).collect();
    let wanted: Vec<String> = mons.iter().map(|m| m.id.clone()).collect();
    let plan = monitor::plan_reconcile(&ids, &wanted);
    for id in &plan.remove {
        for (h, _) in existing.iter().filter(|(_, i)| i == id) {
            remove_bar(*h);
        }
    }
    for id in &plan.keep {
        for (h, _) in existing.iter().filter(|(_, i)| i == id) {
            set_pos(*h, true); // geometria monitora sa mohla zmenit
        }
    }
    for m in mons.iter().filter(|m| plan.add.contains(&m.id)) {
        create_bar(m);
    }
    let fallback = fallback && !st(|s| s.hidden).unwrap_or(false);
    if st(|s| std::mem::replace(&mut s.fallback, fallback) != fallback).unwrap_or(false) {
        emit(BarEvent::MonitorsFallback(fallback));
    }
}

/// Skryje / zobrazi vsetky listy (po zobrazeni sa vytvoria nanovo s rezervaciou pracovnej plochy). Volat cez
/// `popup::Request::SetHidden` (nie priamo z `on_event`: `emit` by pocas spracovania udalosti zahodil `Created`/`Removed`).
pub(crate) fn set_hidden(hidden: bool) {
    if st(|s| std::mem::replace(&mut s.hidden, hidden) != hidden).unwrap_or(false) {
        reconcile(if hidden { "hide" } else { "show" });
    }
}

/// Zmeni vyber monitorov a okamzite uplatni (listy sa pridaju/odoberu); pozri `set_hidden` k volaniu cez `Request`.
pub(crate) fn set_monitors(sel: MonitorSelection) {
    if st(|s| {
        let changed = s.config.monitors != sel;
        s.config.monitors = sel;
        changed
    })
    .unwrap_or(false)
    {
        reconcile("monitors");
    }
}

/// Je lista skryta uzivatelom?
pub fn is_hidden() -> bool {
    st(|s| s.hidden).unwrap_or(false)
}

fn bar_info(hwnd_raw: isize) -> Option<(String, u32)> {
    st(|s| {
        s.bars
            .iter()
            .find(|b| b.hwnd == hwnd_raw)
            .map(|b| (b.id.clone(), b.dpi))
    })
    .flatten()
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let raw = hwnd.0 as isize;
    // SAFETY: wndproc listy na vlakne UI: wParam/lParam sa pouzivaju len ako cisla (ziadna dereferencia); stav sa pozicia cez `st`, ktore nedrzi vypozicku cez volania posielajuce spravy.
    unsafe {
        match msg {
            WM_APPBAR => {
                match wp.0 as u32 {
                    ABN_POSCHANGED => set_pos(raw, false),
                    ABN_FULLSCREENAPP => {
                        let opened = lp.0 != 0;
                        // oznamenie systemu preberie stav (nepotrebna dalsia vlastna kontrola tejto listy)
                        let auto_hide = st(|s| {
                            if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
                                b.fs_check = false;
                                b.fs = opened;
                                if opened {
                                    b.revealed = false;
                                    b.hide_at = None;
                                }
                            }
                            s.config.auto_hide
                        })
                        .unwrap_or(false);
                        if opened {
                            crate::popup::close_soon();
                        }
                        if auto_hide {
                            // skryvana lista: pri celej obrazovke sa skryje, po jej skonceni ostava skryta (vysunie ju mys)
                            if opened {
                                let _ = ShowWindow(hwnd, SW_HIDE);
                            }
                        } else if opened {
                            let _ = SetWindowPos(
                                hwnd,
                                Some(HWND_BOTTOM),
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                            let _ = ShowWindow(hwnd, SW_HIDE);
                        } else {
                            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                            let _ = SetWindowPos(
                                hwnd,
                                Some(HWND_TOPMOST),
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                        }
                        if let Some((id, _)) = bar_info(raw) {
                            emit(BarEvent::Fullscreen {
                                monitor_id: id,
                                opened,
                            });
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            // S2: rozmer sa berie z rcMonitor, nie z navrhu systemu.
            WM_DPICHANGED => {
                crate::popup::close_soon();
                set_pos(raw, true);
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            // MSAA: klientska oblast listy ako IAccessible (nazvy, role, stav tlacidiel)
            WM_GETOBJECT if lp.0 as i32 == OBJID_CLIENT.0 => {
                let acc = crate::access::BarAccessible::create(hwnd);
                LresultFromObject(&IAccessible::IID, wp, &acc)
            }
            crate::access::WM_TRANSOM_INVOKE => {
                if let Some((id, _)) = bar_info(raw) {
                    emit(BarEvent::Invoke {
                        monitor_id: id,
                        index: wp.0,
                    });
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL_MSG => {
                // suradnice su na obrazovke: prepocet do klientskej plochy listy
                let mut pt = windows::Win32::Foundation::POINT {
                    x: (lp.0 & 0xFFFF) as u16 as i16 as i32,
                    y: ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
                };
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                if let Some((id, _)) = bar_info(raw) {
                    emit(BarEvent::BarWheel {
                        monitor_id: id,
                        x: pt.x,
                        delta,
                    });
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP => {
                let x = (lp.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                if msg == WM_MOUSEMOVE {
                    let start = st(|s| {
                        s.bars.iter_mut().find(|b| b.hwnd == raw).map(|b| {
                            let was = b.tracking;
                            b.tracking = true;
                            !was
                        })
                    })
                    .flatten()
                    .unwrap_or(false);
                    if start {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut t);
                    }
                }
                if let Some((id, _)) = bar_info(raw) {
                    emit(BarEvent::Pointer {
                        monitor_id: id,
                        kind: match msg {
                            WM_MOUSEMOVE => PointerKind::Move,
                            WM_LBUTTONDOWN => PointerKind::Down,
                            WM_RBUTTONUP => PointerKind::RightClick,
                            _ => PointerKind::Click,
                        },
                        x,
                        y,
                    });
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                st(|s| {
                    if let Some(b) = s.bars.iter_mut().find(|b| b.hwnd == raw) {
                        b.tracking = false;
                    }
                });
                if let Some((id, _)) = bar_info(raw) {
                    emit(BarEvent::Pointer {
                        monitor_id: id,
                        kind: PointerKind::Leave,
                        x: -1,
                        y: -1,
                    });
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            // vrstvene okno: obsah posiela len `layered_redraw` (UpdateLayeredWindow), nie WM_PAINT
            WM_TRANSOM_REDRAW => {
                layered_redraw(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_LAYERED_RETRY => {
                let _ = KillTimer(Some(hwnd), TIMER_LAYERED_RETRY);
                layered_redraw(hwnd);
                LRESULT(0)
            }
            WM_PAINT => {
                // priehladne pozadie: prepnut na vrstveny rezim / sklo; nepriehladne: povodna cesta (ClearType) nizsie.
                // Sklo (nevrstvene okno) dostava `WM_PAINT` aj pri odkryti a zmene velkosti: cele sa prekresli.
                if let Some(a) = handler_appearance().filter(|a| !a.is_opaque()) {
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(hwnd, &mut ps);
                    let _ = EndPaint(hwnd, &ps);
                    enter_layered(raw, a.glass.is_some());
                    request_redraw(raw, None);
                    return LRESULT(0);
                }
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                if let Some((id, dpi)) = bar_info(raw) {
                    let ctx = PaintCtx {
                        hwnd,
                        hdc,
                        client: rc.into(),
                        dpi,
                        monitor_id: id,
                        backdrop: None,
                        grayscale: false,
                        bookkeeping: true,
                    };
                    HANDLER.with(|c| {
                        if let Ok(mut g) = c.try_borrow_mut() {
                            if let Some(h) = g.as_mut() {
                                h.paint(&ctx);
                            }
                        }
                    });
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_DESTROY => {
                crate::regions::forget(raw);
                // zrusene zvonka (nie cez remove_bar): uvolnit rezervaciu a zabudnut listu
                let known = st(|s| {
                    s.bars
                        .iter()
                        .position(|b| b.hwnd == raw)
                        .map(|i| s.bars.remove(i))
                })
                .flatten();
                if known.is_some() {
                    let mut d = abd(hwnd);
                    SHAppBarMessage(ABM_REMOVE, &mut d);
                }
                LRESULT(0)
            }
            m => {
                let tc = TASKBAR_CREATED.load(Ordering::Relaxed);
                if tc != 0 && m == tc {
                    // S3: explorer sa restartoval -> registrovat znova a prepocitat polohu
                    let mut d = abd(hwnd);
                    SHAppBarMessage(ABM_NEW, &mut d);
                    set_pos(raw, true);
                    // ikonu v oblasti oznameni a udalost `TaskbarCreated` spracuje RAZ riadiace okno (`ctl_proc`): pri viacerych
                    // listach (alebo ziadnej) by sa inak pridavala N-krat (duplicita) alebo vobec
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, m, wp, lp)
            }
        }
    }
}

/// Najviac tolko znakov sa cita z textu `WM_SETTINGCHANGE` (zname nazvy oblasti su kratke).
const SETTING_NAME_MAX: usize = 128;

/// Pocet bajtov od `p` (najviac `max`), ktore lezia v pridelenych a citatelnych strankach tohto procesu (`VirtualQuery`).
/// Nulovy alebo neplatny ukazovatel = 0. Nic sa necita.
fn readable_bytes(p: *const u8, max: usize) -> usize {
    use windows::Win32::System::Memory::{
        VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
    };
    // PAGE_READONLY | PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
    const READABLE: u32 = 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80;
    let mut done = 0usize;
    while done < max {
        let addr = (p as usize).saturating_add(done);
        let mut mbi = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: VirtualQuery len opisuje adresny priestor (necita ciel); `mbi` je platny vystupny buffer zadanej velkosti
        let n = unsafe {
            VirtualQuery(
                Some(addr as *const core::ffi::c_void),
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        let prot = mbi.Protect.0;
        if n == 0
            || mbi.State != MEM_COMMIT
            || prot & READABLE == 0
            || prot & (PAGE_GUARD.0 | PAGE_NOACCESS.0) != 0
        {
            break;
        }
        let end = (mbi.BaseAddress as usize).saturating_add(mbi.RegionSize);
        let avail = end.saturating_sub(addr);
        if avail == 0 {
            break;
        }
        done = done.saturating_add(avail);
    }
    done.min(max)
}

/// Text z `lParam` spravy `WM_SETTINGCHANGE` (UTF-16, najviac [`SETTING_NAME_MAX`] znakov), ak je pritomny.
///
/// Ukazovatel posiela odosielatel spravy: medzi procesmi ho Windows skopiruje, ale v ramci procesu (alebo pri
/// systemovych parametroch, kde `lParam` nie je text) moze ukazovat kamkolvek. Cita sa preto len pamat overena cez
/// `VirtualQuery` a nezarovnany ukazovatel sa cita po znakoch bez predpokladu zarovnania. Poskodeny ukazovatel tak
/// listu nezhodi (predtym ju mohol kazdy proces toho isteho pouzivatela ukoncit).
pub fn setting_name(lp: LPARAM) -> Option<String> {
    let p = lp.0 as *const u16;
    if p.is_null() {
        return None;
    }
    let chars = readable_bytes(p.cast(), SETTING_NAME_MAX * 2) / 2;
    if chars == 0 {
        return None;
    }
    let mut out: Vec<u16> = Vec::new();
    for i in 0..chars {
        // SAFETY: `i < chars` a cely rozsah `p .. p + 2*chars` lezi v pridelenych citatelnych strankach (readable_bytes);
        // `read_unaligned` nepredpoklada zarovnanie
        let c = unsafe { p.add(i).read_unaligned() };
        if c == 0 {
            break;
        }
        out.push(c);
    }
    Some(String::from_utf16_lossy(&out))
}

unsafe extern "system" fn ctl_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: wndproc riadiaceho okna na vlakne UI: lParam sa dereferencuje len cez `setting_name` (overena pamat), inak su parametre cisla.
    unsafe {
        match msg {
            WM_DISPLAYCHANGE => {
                reconcile("WM_DISPLAYCHANGE");
                LRESULT(0)
            }
            WM_ENDSESSION if wp.0 != 0 => {
                crate::tray::remove(hwnd);
                remove_all_bars();
                LRESULT(0)
            }
            WM_CLOSE => {
                // ikona sa odstrani, kym vlastnik (toto okno) este existuje; inak by zostal "duch"
                crate::tray::remove(hwnd);
                remove_all_bars();
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_QUIT => {
                let _ = KillTimer(Some(hwnd), TIMER_QUIT);
                crate::tray::remove(hwnd);
                remove_all_bars();
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_FS_RECHECK => {
                fs_recheck(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_AUTOHIDE => {
                autohide_tick(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_TRAY_RETRY => {
                crate::tray::retry(hwnd);
                arm_tray_retry(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_WAKE => {
                // jednorazovy: najprv zrusit, az potom oznamit (handler moze naplanovat dalsi)
                let _ = KillTimer(Some(hwnd), TIMER_WAKE);
                emit(BarEvent::Wake);
                LRESULT(0)
            }
            WM_TIMER if wp.0 >= TIMER_USER_BASE => {
                let _ = KillTimer(Some(hwnd), wp.0);
                let id = (wp.0 - TIMER_USER_BASE) as u32;
                // snimka s obnovou obrazovky: dalsia moze prist az po tejto
                PACERS.with(|p| {
                    if let Some((_, pc)) = p.borrow().iter().find(|(i, _)| *i == id) {
                        pc.ack();
                    }
                });
                emit(BarEvent::Timer(id));
                LRESULT(0)
            }
            WM_TRANSOM_NOTIFY => {
                emit(BarEvent::Notify(wp.0 as u32));
                LRESULT(0)
            }
            crate::dragdrop::WM_TRANSOM_DRAG => {
                crate::dragdrop::pump();
                LRESULT(0)
            }
            // ikona v oblasti oznameni (NOTIFYICON_VERSION_4): udalost v nizkom slove lParam, bod ukotvenia vo wParam
            crate::tray::WM_TRANSOM_TRAY => {
                let ev = (lp.0 & 0xFFFF) as u32;
                let x = (wp.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                let kind = match ev {
                    crate::tray::NIN_SELECT | crate::tray::NIN_KEYSELECT => Some(TrayKind::Select),
                    crate::tray::WM_CONTEXTMENU => Some(TrayKind::Menu),
                    crate::tray::NIN_BALLOONUSERCLICK => Some(TrayKind::Balloon),
                    _ => None,
                };
                if let Some(kind) = kind {
                    emit(BarEvent::Tray { kind, x, y });
                }
                LRESULT(0)
            }
            crate::popup::WM_TRANSOM_REQUEST => {
                crate::popup::run_requests();
                LRESULT(0)
            }
            // WM_HOTKEY: globalna skratka (wParam = id z `RegisterHotKey`)
            0x0312 => {
                emit(BarEvent::Hotkey(wp.0 as i32));
                LRESULT(0)
            }
            // zmena systemoveho casu alebo casovej zony
            WM_TIMECHANGE => {
                emit(BarEvent::TimeChanged);
                LRESULT(0)
            }
            // lParam = text oblasti zmeny: "ImmersiveColorSet" (farebna schema), "intl" (regionalne nastavenia, 12/24 h)
            WM_SETTINGCHANGE => {
                // priehladnost / vysoky kontrast sa mohli zmenit: zapamatana odpoved pre sklo neplati
                crate::glass::forget_system_allows();
                // SPI_SETCLIENTAREAANIMATION = 0x1043 (lParam nie je text)
                if wp.0 == 0x1043 {
                    emit(BarEvent::AnimationsChanged);
                }
                match setting_name(lp) {
                    Some(s) if s == "ImmersiveColorSet" => emit(BarEvent::ThemeChanged),
                    Some(s) if s == "intl" || s == "TimeZoneInformation" => {
                        emit(BarEvent::TimeChanged)
                    }
                    _ => {}
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_POWERBROADCAST => {
                match wp.0 {
                    PBT_APMRESUMEAUTOMATIC => {
                        emit(BarEvent::TimeChanged);
                        emit(BarEvent::PowerChanged);
                    }
                    w if w == PBT_APMPOWERSTATUSCHANGE as usize
                        || w == PBT_POWERSETTINGCHANGE as usize =>
                    {
                        emit(BarEvent::PowerChanged)
                    }
                    _ => {}
                }
                LRESULT(1) // TRUE
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                // Explorer sa restartoval: jeho ikony zanikli. Prijima to riadiace okno (aj ked je lista skryta alebo ziadna)
                // a ikona sa pridava presne raz, bez ohladu na pocet listov
                crate::tray::taskbar_created(hwnd);
                arm_tray_retry(hwnd);
                emit(BarEvent::TaskbarCreated);
                LRESULT(0)
            }
            m => {
                let idx = BROADCASTS.with(|b| b.borrow().iter().position(|id| *id == m));
                if let Some(i) = idx {
                    emit(BarEvent::Broadcast(i as u32));
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, m, wp, lp)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_hide_reveals_at_the_edge_waits_before_hiding_and_stays_while_needed() {
        use HideStep::*;
        // skryta: len mys na okraji a bez celej obrazovky
        assert_eq!(hide_step(false, true, false, false, false, None), Reveal);
        assert_eq!(hide_step(false, true, false, false, true, None), Nothing);
        assert_eq!(hide_step(false, false, true, true, false, None), Nothing);
        // vysunuta: mys odisla -> odpocet, po nom skryt; navrat mysi alebo popover / tahanie odpocet zrusi
        assert_eq!(hide_step(true, false, false, false, false, None), Arm);
        assert_eq!(
            hide_step(true, false, false, false, false, Some(false)),
            Nothing
        );
        assert_eq!(
            hide_step(true, false, false, false, false, Some(true)),
            Hide
        );
        assert_eq!(
            hide_step(true, false, true, false, false, Some(true)),
            Disarm
        );
        assert_eq!(
            hide_step(true, false, false, true, false, Some(true)),
            Disarm
        );
        assert_eq!(hide_step(true, true, true, false, false, None), Nothing);
    }
}

#[cfg(test)]
mod setting_name_tests {
    use super::*;
    use windows::Win32::System::Memory::{
        VirtualAlloc, VirtualFree, VirtualProtect, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
        PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, PAGE_READWRITE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[test]
    fn valid_null_and_garbage_pointers_never_crash() {
        let w = wide("ImmersiveColorSet");
        assert_eq!(
            setting_name(LPARAM(w.as_ptr() as isize)).as_deref(),
            Some("ImmersiveColorSet")
        );
        assert_eq!(setting_name(LPARAM(0)), None);
        // prvych 64 KiB adresneho priestoru nie je nikdy pridelenych; rovnako jadrove adresy a "-1"
        for bad in [1isize, 0x10, 0x1043, 0xFFFF, -1, isize::MIN] {
            assert_eq!(setting_name(LPARAM(bad)), None, "{bad:#x}");
        }
        // dlhy text bez ukoncenia sa skrati
        let long: Vec<u16> = std::iter::repeat_n(u16::from(b'a'), 1000).collect();
        assert_eq!(
            setting_name(LPARAM(long.as_ptr() as isize)).map(|s| s.len()),
            Some(SETTING_NAME_MAX)
        );
        // nezarovnany ukazovatel
        let mut bytes = [0u8; 16];
        bytes[1] = b'i';
        bytes[3] = b'n';
        bytes[5] = b't';
        bytes[7] = b'l';
        assert_eq!(
            setting_name(LPARAM(bytes.as_ptr() as isize + 1)).as_deref(),
            Some("intl")
        );
    }

    /// Text bez ukoncenia tesne pred neprístupnou strankou: cita sa len po jej hranicu (predtym pristup do nepridelenej
    /// pamate = pad listy).
    #[test]
    fn an_unterminated_string_before_an_inaccessible_page_stops_at_the_page() {
        // SAFETY: rezervacia a pridelenie dvoch stranok; druha sa nastavi na PAGE_NOACCESS; pamat sa na konci uvolni
        unsafe {
            let base =
                VirtualAlloc(None, 0x2000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as *mut u8;
            assert!(!base.is_null());
            let mut old = PAGE_PROTECTION_FLAGS(0);
            VirtualProtect(base.add(0x1000).cast(), 0x1000, PAGE_NOACCESS, &mut old).unwrap();
            // posledne 3 znaky prvej stranky, bez nuly
            let start = base.add(0x1000 - 6) as *mut u16;
            for i in 0..3 {
                start.add(i).write(u16::from(b'x'));
            }
            assert_eq!(setting_name(LPARAM(start as isize)).as_deref(), Some("xxx"));
            // ukazovatel priamo do neprístupnej stranky
            assert_eq!(setting_name(LPARAM(base.add(0x1000) as isize)), None);
            VirtualFree(base.cast(), 0, MEM_RELEASE).unwrap();
            // uvolnena pamat
            assert_eq!(setting_name(LPARAM(base as isize)), None);
        }
    }
}
