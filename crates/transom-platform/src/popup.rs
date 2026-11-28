//! Popover pripojeny k liste (kalendar, panel vyuzitia AI), kontextove menu, aktivacia aplikacie a spustenie
//! programu - vsetko cez **frontu poziadaviek** spracovanu v riadiacom okne.
//!
//! Preco fronta: `ShowWindow`, `SetForegroundWindow`, `DestroyWindow` a `TrackPopupMenu` (modalna slucka) posielaju
//! synchronne spravy do nasich wndproc. Keby sa volali z `BarHandler::on_event`/`paint`, prisli by v case, ked je
//! handler vypozicany, a udalosti by sa ticho stratili. Handler preto len zaradi poziadavku ([`request`]) a riadiace
//! okno ju vykona po navrate do slucky sprav.
//!
//! Popover je **aktivovatelne** okno (kalendar je zamerna akcia, rovnako ako systemovy kalendar): zatvori sa pri
//! strate aktivacie, Esc, opakovanom kliknuti na tu istu polozku, fullscreene, zmene DPI a odstraneni listy.
//! Ovladanie medii a ostatne casti listy ostavaju neaktivujuce.
use crate::activate::activate_app;
use crate::appbar::{bar_geometry, ctl_hwnd, emit, with_handler, BarEvent, PointerKind};
use std::cell::{Cell, RefCell};
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateRoundRectRgn, DeleteDC,
    DeleteObject, EndPaint, GetDC, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, PatBlt,
    ReleaseDC, SelectObject, SetWindowRgn, BLACKNESS, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT, SRCCOPY, WHITENESS,
};
use windows::Win32::UI::Accessibility::{IAccessible, LresultFromObject, NotifyWinEvent};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetClientRect, GetForegroundWindow, IsWindow, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassW, SetForegroundWindow, ShowWindow, TrackPopupMenu, IDC_ARROW, MF_SEPARATOR,
    MF_STRING, OBJID_CLIENT, SW_SHOW, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_TOPALIGN,
    TRACK_POPUP_MENU_FLAGS, WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_CHAR, WM_DESTROY,
    WM_ERASEBKGND, WM_GETOBJECT, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NULL, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Sprava riadiacemu oknu: spracovat frontu poziadaviek.
pub(crate) const WM_TRANSOM_REQUEST: u32 = WM_APP + 0x33;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WA_INACTIVE: usize = 0;

/// Kam sa otvori vlastne kontextove menu.
#[derive(Clone, Debug)]
pub enum MenuAnchor {
    /// Pod listou monitora `monitor_id`, lavy okraj na `x` (px v klientskych suradniciach listy).
    Bar { monitor_id: String, x: i32 },
    /// V bode obrazovky (ikona v oblasti oznameni): menu rastie smerom k stredu monitora.
    Screen { x: i32, y: i32 },
}

/// Poziadavka pre riadiace okno.
#[derive(Clone, Debug)]
pub enum Request {
    /// Otvorit vlastne (kreslene aplikaciou) kontextove menu `kind`: velkost urci `BarHandler::popover_size`,
    /// zachyti mys (klik mimo zatvori), po zatvoreni vrati popredie predchadzajucemu oknu.
    ShowMenu {
        kind: u32,
        anchor: MenuAnchor,
    },
    /// Otvorit popover `kind` pod listou monitora `monitor_id`, vycentrovany na `anchor_x` (px v klientskych
    /// suradniciach listy), velkost v lp. Ak je ten isty druh uz otvoreny, zavrie sa (prepinac).
    ShowPopover {
        kind: u32,
        monitor_id: String,
        anchor_x: i32,
        w_lp: i32,
        h_lp: i32,
    },
    ClosePopover,
    /// Tahanie suborov von z popoveru (OLE `SHDoDragDrop`, len COPY/LINK); blokuje do konca tahania, popover sa pocas neho nezatvara.
    DragOut(Vec<String>),
    /// Aktivovat aplikaciu media session (vysledok: `BarEvent::Activated`).
    Activate {
        app: String,
        title: String,
    },
    /// Kontextove menu pod bodom `x` listy; polozka s id 0 = oddelovac. Vysledok: `BarEvent::MenuChosen`.
    Menu {
        monitor_id: String,
        x: i32,
        items: Vec<(u32, String)>,
    },
    /// Kontextove menu v bode obrazovky (ikona v oblasti oznameni); vysledok: `BarEvent::MenuChosen`.
    MenuAt {
        x: i32,
        y: i32,
        items: Vec<(u32, String)>,
    },
    /// Skryt (`true`) alebo znova zobrazit vsetky listy.
    SetHidden(bool),
    /// Zmenit vyber monitorov (uplatni sa hned).
    SetMonitors(crate::monitor::MonitorSelection),
    /// Zapnut / vypnut automaticke skryvanie listy (listy sa vytvoria nanovo).
    SetAutoHide(bool),
    /// Spustit program (napr. okno nastaveni).
    Launch {
        path: String,
        args: String,
    },
}

#[derive(Clone, Copy)]
struct Pop {
    hwnd: isize,
    kind: u32,
    dpi: u32,
    /// Vlastne kontextove menu: zachytava mys, vracia popredie, vystavuje `MenuAccessible`.
    menu: bool,
    /// Okno v popredi pred otvorenim (vrati sa pri zatvoreni menu, ak bol popover stale v popredi).
    prev: isize,
    /// Sklenene pozadie (akrylove rozmazanie, pozri [`crate::glass`]).
    glass: Option<crate::glass::GlassStyle>,
}

/// Animacia otvarania skleneneho panelu: okno rastie z 92 % (ukotvene hore pri liste) po pruzine a obsah sa
/// roztmieva; kresli sa z jednej snimky v plnej velkosti (`FRAME`), takze sa rozlozenie pocas nej neprepocitava.
struct Anim {
    t0: std::time::Instant,
    to: (i32, i32, i32, i32),
    /// Pevny bod rastu ako podiel sirky a vysky (panel: hore v strede, menu: roh pri kurzore).
    anchor: (f32, f32),
}

const TIMER_ANIM: usize = 0x51;
/// Odlozene prekreslenie, ked bola obsluha aplikacie pri `WM_PAINT` zaneprazdnena.
const TIMER_REPAINT: usize = 0x52;
/// Vysunutie panelu z listy (ms) a roztmievanie obsahu (ms).
const REVEAL_MS: f32 = 230.0;
const FADE_MS: f32 = 150.0;
/// Snimky animacie prichadzaju s obnovou obrazovky (`vsync::Pacer`); predpoved polohy o snimku dopredu (ms).
const FRAME_MS: u32 = 5;

/// Viditelne riadky panelu `h` px v case `ms`: zhora (panel pod listou) alebo zdola (menu nad kotvou).
fn reveal_rows(h: i32, ms: f32, from_bottom: bool) -> (i32, i32) {
    let v = ((h as f32) * crate::system::ease_out(ms / REVEAL_MS)).round() as i32;
    let v = v.clamp(1, h.max(1));
    if from_bottom {
        (h - v, h)
    } else {
        (0, v)
    }
}

/// Zastavi animaciu otvarania (a presny casovac, ak bezal).
fn anim_clear() {
    ANIM.with(|a| a.borrow_mut().take());
    PACE.with(|p| p.borrow_mut().take());
}

thread_local! {
    /// Cakajuci horny surrogat zo `WM_CHAR` (znaky mimo BMP prichadzaju ako dve spravy).
    static HIGH: std::cell::Cell<u16> = const { std::cell::Cell::new(0) };
    static QUEUE: RefCell<Vec<Request>> = const { RefCell::new(Vec::new()) };
    static POP: Cell<Option<Pop>> = const { Cell::new(None) };
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ANIM: RefCell<Option<Anim>> = const { RefCell::new(None) };
    /// Sklenene okno je skryte pred DWM, kym sa prvy raz nevykresli (pozri `glass::cloak`).
    static CLOAKED: Cell<bool> = const { Cell::new(false) };
    /// Snimky animacie otvarania zladene s obrazovkou.
    static PACE: RefCell<Option<crate::vsync::Pacer>> = const { RefCell::new(None) };
    /// Dva prechody a vystup skleneneho panelu (znovu pouzite medzi prekresleniami rovnakej velkosti).
    static FRAME: RefCell<Option<crate::layered::LayeredBuf>> = const { RefCell::new(None) };
}

/// Kontext kreslenia popoveru (dvojity buffer: `hdc` je pamatovy DC).
pub struct PopoverPaint {
    pub hdc: HDC,
    pub w: i32,
    pub h: i32,
    pub dpi: u32,
    pub kind: u32,
    /// Sklo: kreslit pozadie (aj okraj) touto farbou namiesto farby temy - prechod na cierne / biele pozadie,
    /// z ktoreho sa odvodi alfa (pozri [`crate::layered`]).
    pub backdrop: Option<(u8, u8, u8)>,
}

/// Zaradi poziadavku; vykona sa v riadiacom okne po navrate do slucky sprav. Volat na vlakne hostitela.
pub fn request(r: Request) {
    QUEUE.with(|q| q.borrow_mut().push(r));
    if let Some(ctl) = ctl_hwnd() {
        // SAFETY: PostMessageW len s cislami na nase riadiace okno.
        let _ = unsafe { PostMessageW(Some(ctl), WM_TRANSOM_REQUEST, WPARAM(0), LPARAM(0)) };
    }
}

/// Ak su vo fronte poziadavky (zaradene pred vznikom riadiaceho okna), posle riadiacemu oknu spravu, nech ich spracuje.
pub(crate) fn kick_requests() {
    if QUEUE.with(|q| q.borrow().is_empty()) {
        return;
    }
    if let Some(ctl) = ctl_hwnd() {
        // SAFETY: PostMessageW len s cislami na nase riadiace okno.
        let _ = unsafe { PostMessageW(Some(ctl), WM_TRANSOM_REQUEST, WPARAM(0), LPARAM(0)) };
    }
}

/// Druh otvoreneho popoveru.
pub fn popover_open() -> Option<u32> {
    POP.with(|p| p.get()).map(|p| p.kind)
}

/// DPI otvoreneho panela (96 = 100 %); bez panela 96.
pub fn popover_dpi() -> u32 {
    POP.with(|p| p.get()).map_or(96, |p| p.dpi)
}

/// Obdlznik otvoreneho panelu (nie kontextove menu) na obrazovke: (vlavo, hore, vpravo); `None` = ziadny.
pub fn popover_extent() -> Option<(i32, i32, i32)> {
    let p = POP.with(|p| p.get()).filter(|p| !p.menu)?;
    let mut r = RECT::default();
    // SAFETY: citanie obdlznika vlastneho okna.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(HWND(p.hwnd as *mut _), &mut r)
    }
    .ok()?;
    Some((r.left, r.top, r.right))
}

/// Vyziada prekreslenie popoveru (lacne; `InvalidateRect` nic synchronne neposiela).
pub fn invalidate_popover() {
    if let Some(p) = POP.with(|p| p.get()) {
        // SAFETY: handle nasho popoveru (moze uz neexistovat: chyba bez nasledkov).
        let _ = unsafe { InvalidateRect(Some(HWND(p.hwnd as *mut _)), None, false) };
    }
}

pub(crate) fn register_class(hinst: HINSTANCE) {
    // SAFETY: registracia triedy s konstantnymi retazcami a nasou wndproc.
    unsafe {
        let cur = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(popover_proc),
            hInstance: hinst,
            lpszClassName: w!("TransomPopover"),
            hCursor: cur,
            ..Default::default()
        });
    }
}

fn close_now() {
    close_ex(true);
}

/// Zatvori popover. `restore`: vratit popredie predchadzajucemu oknu (len menu, a len ak popover este bol v popredi -
/// ak pouzivatel klikol do inej aplikacie, popredie sa nemeni).
fn close_ex(restore: bool) {
    anim_clear();
    FRAME.with(|f| f.borrow_mut().take());
    if let Some(p) = POP.with(|c| c.take()) {
        // SAFETY: popover patri tomuto vlaknu; znici sa raz (hodnota sa vybrala z POP).
        unsafe {
            let hw = HWND(p.hwnd as *mut _);
            let was_fg = GetForegroundWindow() == hw;
            if p.menu {
                let _ = ReleaseCapture();
                crate::menu_access::set_snapshot(Vec::new(), None);
                NotifyWinEvent(EVENT_SYSTEM_MENUPOPUPEND, hw, OBJID_CLIENT.0, 0);
            }
            crate::menu_access::set_snapshot_of(crate::menu_access::Kind::Panel, Vec::new(), None);
            let _ = DestroyWindow(hw);
            if restore && p.menu && was_fg && p.prev != 0 {
                let prev = HWND(p.prev as *mut _);
                if IsWindow(Some(prev)).as_bool() {
                    let _ = SetForegroundWindow(prev);
                }
            }
        }
        emit(BarEvent::PopoverClosed { kind: p.kind });
        // spodna ciara listy sa pod panelom vrati
        crate::appbar::invalidate_all();
    }
}

const EVENT_SYSTEM_MENUPOPUPSTART: u32 = 0x0006;
const EVENT_SYSTEM_MENUPOPUPEND: u32 = 0x0007;
const EVENT_OBJECT_FOCUS: u32 = 0x8005;
const DWMWA_WINDOW_CORNER_PREFERENCE: windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE =
    windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(33);

/// Polozky otvoreneho menu pre citacky obrazovky + aktivna polozka (volat po otvoreni a po kazdej zmene aktivnej
/// polozky). Zmena aktivnej polozky posle `EVENT_OBJECT_FOCUS`, otvorenie `EVENT_SYSTEM_MENUPOPUPSTART`.
pub fn set_menu_accessibility(entries: Vec<MenuEntry>, active: Option<usize>) {
    let Some(p) = POP.with(|p| p.get()).filter(|p| p.menu) else {
        return;
    };
    let hw = HWND(p.hwnd as *mut _);
    let (old, old_active) = crate::menu_access::snapshot();
    let first = old.is_empty();
    let n = entries.len();
    crate::menu_access::set_snapshot(entries, active);
    // SAFETY: NotifyWinEvent s handle nasho okna; parametre su cisla.
    unsafe {
        if first && n > 0 {
            NotifyWinEvent(EVENT_SYSTEM_MENUPOPUPSTART, hw, OBJID_CLIENT.0, 0);
        }
        if active != old_active || first {
            if let Some(a) = active {
                NotifyWinEvent(EVENT_OBJECT_FOCUS, hw, OBJID_CLIENT.0, a as i32 + 1);
            }
        }
    }
}

pub use crate::menu_access::MenuEntry;

/// Prvky otvoreneho popoveru (nie menu), napr. panel AI: tlacidla s nazvami a obdlznikmi pre citacky obrazovky a
/// aktivna polozka (pod mysou / zvolena). Zmena aktivnej polozky posle `EVENT_OBJECT_FOCUS`.
pub fn set_panel_accessibility(entries: Vec<MenuEntry>, active: Option<usize>) {
    use crate::menu_access::{set_snapshot_of, snapshot_of, Kind};
    let Some(p) = POP.with(|p| p.get()).filter(|p| !p.menu) else {
        return;
    };
    let old_active = snapshot_of(Kind::Panel).1;
    // panel bez vlastnych roli (AI): predchadzajuce role sa zahodia
    crate::menu_access::clear_panel_extras();
    set_snapshot_of(Kind::Panel, entries, active);
    if active != old_active {
        if let Some(a) = active {
            // SAFETY: NotifyWinEvent s handle nasho okna; parametre su cisla.
            unsafe {
                NotifyWinEvent(
                    EVENT_OBJECT_FOCUS,
                    HWND(p.hwnd as *mut _),
                    OBJID_CLIENT.0,
                    a as i32 + 1,
                );
            }
        }
    }
}

/// Vytvori okno popoveru (zdielane medzi `ShowPopover` a `ShowMenu`).
fn create_popover(
    kind: u32,
    dpi: u32,
    pos: (i32, i32),
    size: (i32, i32),
    menu: bool,
    glass: Option<crate::glass::GlassStyle>,
    anchor: (f32, f32),
) -> Option<HWND> {
    // SAFETY: vytvorenie popoveru na vlakne UI; retazce a struktury ziju pocas volani.
    unsafe {
        let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?;
        let prev = GetForegroundWindow();
        // sklo s animaciami: okno ma hned konecnu velkost, vysuva sa len jeho viditelna cast (oblast okna) - ziadna
        // zmena velkosti ani zmensovanie obrazu pocas animacie
        let animate = glass.is_some() && crate::system::animations_enabled();
        let (x0, y0, w0, h0) = (pos.0, pos.1, size.0, size.1);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("TransomPopover"),
            w!("transom-popover"),
            WS_POPUP,
            x0,
            y0,
            w0,
            h0,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        POP.with(|c| {
            c.set(Some(Pop {
                hwnd: hwnd.0 as isize,
                kind,
                dpi,
                menu,
                prev: prev.0 as isize,
                glass,
            }))
        });
        FRAME.with(|f| f.borrow_mut().take());
        // nabeh pripojeneho panela zacina presne farbou, akou lista na obrazovke naozaj vyzera (tesne nad panelom)
        let glass = glass.map(|mut st| {
            if st.rim == crate::glass::Rim::Attached {
                if let Some(l) = st.lead.as_mut() {
                    let gap = (2 * dpi as i32 / 96).max(2);
                    l.adopt(crate::glass::screen_color(
                        pos.0,
                        pos.0 + size.0,
                        pos.1 - gap,
                    ));
                }
            }
            st
        });
        POP.with(|c| {
            if let Some(mut p) = c.get() {
                p.glass = glass;
                c.set(Some(p));
            }
        });
        if glass.is_some() {
            // rohy (pripojeny panel len spodne) aj okraj v nich kresli sklo samo, vyhladene: DWM nezaobluje
            let round: i32 = 1;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &round as *const i32 as *const _,
                4,
            );
            // pred zobrazenim skryte pred DWM: odkryje ho az prve vykreslenie (ziadny zablesk celeho okna). Pocas
            // vysuvania sa okno neorezava oblastou: neodkryta cast je priehladna (`LayeredBuf::clear_outside`).
            crate::glass::cloak(hwnd, true);
            CLOAKED.with(|c| c.set(true));
            crate::glass::set_glass(hwnd, true);
        }
        anim_clear();
        if animate {
            ANIM.with(|a| {
                *a.borrow_mut() = Some(Anim {
                    t0: std::time::Instant::now(),
                    to: (pos.0, pos.1, size.0, size.1),
                    anchor,
                })
            });
            PACE.with(|p| *p.borrow_mut() = Some(crate::vsync::Pacer::start(hwnd, TIMER_ANIM)));
        }
        if menu && glass.is_none() {
            // zaoblene rohy (Windows 11; inde sa volanie tichu ignoruje)
            let round: i32 = 2;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &round as *const i32 as *const _,
                4,
            );
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        // povolene: proces listy prijal posledny vstup (klik pouzivatela)
        let _ = SetForegroundWindow(hwnd);
        if menu {
            // klik mimo menu (aj na liste alebo v inej aplikacii) ho zatvori ako pri nativnom menu
            SetCapture(hwnd);
        }
        Some(hwnd)
    }
}

/// Sirka vyduteho prechodu do listy (px) pre panel pripojeny k liste.
fn popover_flare(st: &crate::glass::GlassStyle, dpi: u32) -> i32 {
    if st.rim == crate::glass::Rim::Attached {
        10 * dpi as i32 / 96
    } else {
        0
    }
}

/// Pripojeny panel sa vysuva vzdy zhora (z listy).
fn menu_attached(st: &crate::glass::GlassStyle) -> bool {
    st.rim == crate::glass::Rim::Attached
}

/// Viditelna cast okna pocas vysuvania (`v` riadkov zhora alebo zdola), so zaoblenim ako ma panel.
#[allow(clippy::too_many_arguments)]
fn reveal_region(hwnd: HWND, w: i32, h: i32, v: i32, from_bottom: bool, dpi: u32, glass: bool) {
    let r = (8 * dpi as i32 / 96).max(4);
    let (y0, y1) = if from_bottom { (h - v, h) } else { (0, v) };
    // SAFETY: oblast prevezme okno (netreba ju uvolnit).
    unsafe {
        if glass {
            // sklo: rovna oblast (zaoblena oblast okna ma tvrdy okraj a DWM na jej oblucku nad sklom ukazal biele
            // pixely); zaoblene rohy dokresli sklo na konci animacie
            let rgn = windows::Win32::Graphics::Gdi::CreateRectRgn(0, y0, w, y1);
            let _ = SetWindowRgn(hwnd, Some(rgn), true);
            return;
        }
        // vysuva sa z listy: hore rovno (tvar hornych rohov a vyduty prechod kresli sklo), zaoblene len spodne rohy;
        // inak by sa horne rohy na konci animacie zmenili a bliklo by to
        let top = if from_bottom { y0 } else { y0 - 2 * r };
        let rgn = CreateRoundRectRgn(0, top, w + 1, y1 + 1, 2 * r, 2 * r);
        let _ = SetWindowRgn(hwnd, Some(rgn), true);
    }
}

/// Jeden krok animacie otvarania: viditelna cast okna podla casu (ease-out) a roztmievanie obsahu; okno sa
/// nezvacsuje ani neprekresluje, ked sa meni len oblast. Na konci cele okno (rohy kresli sklo).
fn anim_step(hwnd: HWND) {
    let Some((t, to, anchor)) = ANIM.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| (a.t0.elapsed(), a.to, a.anchor))
    }) else {
        // SAFETY: zrusenie casovaca nasho okna.
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_ANIM);
        }
        return;
    };
    let ms = t.as_secs_f32() * 1000.0;
    let Some(p) = POP.with(|p| p.get()) else {
        return;
    };
    let from_bottom = anchor.1 >= 0.5 && !p.glass.is_some_and(|g| menu_attached(&g));
    // SAFETY: casovace, poloha a priehladnost nasho okna na jeho vlakne.
    unsafe {
        if ms >= REVEAL_MS.max(FADE_MS) {
            let _ = KillTimer(Some(hwnd), TIMER_ANIM);
            anim_clear();
            let _ = SetWindowRgn(hwnd, None, true);
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        if p.glass.is_some() {
            // sklo: kazda snimka dokresli odkryte riadky (bez oblasti okna)
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        let (y0, y1) = reveal_rows(to.3, ms, from_bottom);
        reveal_region(hwnd, to.2, to.3, y1 - y0, from_bottom, p.dpi, false);
        // obsah sa prekresluje len pocas roztmievania (potom sa meni uz len oblast okna)
        if ms <= FADE_MS + FRAME_MS as f32 {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

/// Kreslenie skleneneho panelu: dva prechody obsahu (cierne a biele pozadie), zlozenie nad sklo a `BitBlt`
/// (pocas animacie s roztmievanym obsahom zo snimky z jej zaciatku, len viditelne riadky).
fn paint_glass(hdc: HDC, kind: u32, dpi: u32, rc: RECT, st: &crate::glass::GlassStyle) {
    let attached = st.rim == crate::glass::Rim::Attached;
    let mut st = *st;
    // nabeh z farby listy je v lp
    if let Some(l) = st.lead.as_mut() {
        l.px = l.px * dpi as i32 / 96;
    }
    st.corner = 8 * dpi as i32 / 96;
    let flare = popover_flare(&st, dpi);
    st.flare = flare;
    let st = &st;
    let anim = ANIM.with(|a| {
        a.borrow()
            .as_ref()
            .map(|a| (a.t0.elapsed().as_secs_f32() * 1000.0, a.anchor))
    });
    let (fw, fh) = (rc.right, rc.bottom);
    let content = anim.map_or(255, |(ms, _)| {
        (crate::system::ease_out(ms / FADE_MS) * 255.0) as u8
    });
    let mut buf = FRAME.with(|f| f.borrow_mut().take());
    let reuse = anim.is_some() && buf.as_ref().is_some_and(|b| (b.w, b.h) == (fw, fh));
    if buf.as_ref().is_none_or(|b| (b.w, b.h) != (fw, fh)) {
        buf = crate::layered::LayeredBuf::new(fw, fh);
    }
    let Some(mut b) = buf else {
        return;
    };
    let painting = std::time::Instant::now();
    if !reuse {
        for (dc, bd) in [
            (b.black_dc(), (0u8, 0u8, 0u8)),
            (b.white_dc(), (255, 255, 255)),
        ] {
            let ctx = PopoverPaint {
                hdc: dc,
                w: fw - 2 * flare,
                h: fh,
                dpi,
                kind,
                backdrop: Some(bd),
            };
            // obsah v tele panela (pasy vyduteho prechodu su ciste sklo: podklad oboch prechodov, inak by zostali
            // nevyplnene a vyzerali ako nepriehladna cierna)
            // SAFETY: vlastne DC bufra; posun sa po kresleni vrati.
            unsafe {
                let rop = if bd.0 == 0 { BLACKNESS } else { WHITENESS };
                let _ = PatBlt(dc, 0, 0, fw, fh, rop);
                let mut old = POINT::default();
                let _ =
                    windows::Win32::Graphics::Gdi::SetViewportOrgEx(dc, flare, 0, Some(&mut old));
                with_handler(|hd| hd.paint_popover(&ctx));
                let _ = windows::Win32::Graphics::Gdi::SetViewportOrgEx(dc, old.x, old.y, None);
            }
        }
        if attached {
            // okraj obsahu (lem panela) by pri liste vyzeral ako ciara a na bokoch by presiel cez cierny nabeh az
            // k liste: vsetky jeho riadky a stlpce su ciste sklo, obrys da len okraj skla (pri liste sa strati)
            let n = (dpi as i32 / 96).max(1);
            let cw = fw - 2 * flare;
            // SAFETY: vlastne DC bufra.
            unsafe {
                for (dc, rop) in [(b.black_dc(), BLACKNESS), (b.white_dc(), WHITENESS)] {
                    let _ = PatBlt(dc, 0, 0, fw, n, rop);
                    let _ = PatBlt(dc, flare, 0, n, fh, rop);
                    let _ = PatBlt(dc, flare + cw - n, 0, n, fh, rop);
                    let _ = PatBlt(dc, flare, fh - n, cw, n, rop);
                }
            }
        }
    }
    // pocas vysuvania len viditelne riadky (zlozenie celeho panelu pri vysokom DPI stoji ~7 ms)
    let area = match anim {
        Some((ms, anchor)) => {
            let from_bottom = anchor.1 >= 0.5 && !menu_attached(st);
            let (y0, y1) = reveal_rows(fh, ms, from_bottom);
            crate::layered::Area {
                x0: 0,
                y0,
                x1: fw,
                y1,
            }
        }
        None => crate::layered::Area::full(fw, fh),
    };
    b.compose_glass(area, st, content);
    if anim.is_some() {
        // neodkryta cast okna je priehladna (okno sa neorezava oblastou); kresli sa cele okno
        b.clear_outside(area);
        b.blit(hdc, crate::layered::Area::full(fw, fh));
    } else {
        b.blit(hdc, area);
    }
    // prve (tazke) vykreslenie obsahu sa do animacie nepocita: inak by bola prva viditelna snimka uz skoro na konci
    // a panel by sa "zjavil"
    if !reuse && anim.is_some() {
        let spent = painting.elapsed();
        ANIM.with(|a| {
            if let Some(a) = a.borrow_mut().as_mut() {
                a.t0 += spent;
            }
        });
    }
    if CLOAKED.with(|c| c.replace(false)) {
        crate::glass::cloak(
            unsafe { windows::Win32::Graphics::Gdi::WindowFromDC(hdc) },
            false,
        );
    }
    FRAME.with(|f| *f.borrow_mut() = Some(b));
}

/// Monitor v bode obrazovky: (pracovna plocha, dpi).
fn monitor_at(x: i32, y: i32) -> Option<(RECT, u32)> {
    // SAFETY: MONITORINFO s nastavenym cbSize je lokalna.
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(m, &mut mi).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        Some((mi.rcWork, dx))
    }
}

/// Lavy horny roh menu `w` x `h` px: pod bodom (alebo nad nim, ak `above`), v hraniciach `bounds`.
fn menu_origin(anchor: (i32, i32), size: (i32, i32), bounds: RECT, above: bool) -> (i32, i32) {
    let (w, h) = size;
    let x = anchor
        .0
        .clamp(bounds.left, (bounds.right - w).max(bounds.left));
    let y = if above { anchor.1 - h } else { anchor.1 };
    let y = y.clamp(bounds.top, (bounds.bottom - h).max(bounds.top));
    (x, y)
}

/// Spracuje frontu (vola riadiace okno pri `WM_TRANSOM_REQUEST`). Ziadna vypozicka stavu pocas Win32 volani.
pub(crate) fn run_requests() {
    let reqs: Vec<Request> = QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for r in reqs {
        match r {
            Request::ClosePopover => close_now(),
            Request::DragOut(paths) => {
                crate::dragdrop::drag_out(&paths);
                // po tahani okno pravdepodobne nie je aktivne (cielova aplikacia ma popredie): zatvori sa, nezostane visiet
                close_now();
            }
            Request::ShowPopover {
                kind,
                monitor_id,
                anchor_x,
                w_lp,
                h_lp,
            } => {
                // vysunuty vybezok pod stredom listy by sa s panelom prekryval
                crate::dragdrop::notch_collapse();
                let same = popover_open() == Some(kind);
                // pri prepnuti z menu na iny popover sa popredie nevracia (novy popover ho hned prevezme)
                close_ex(false);
                if same {
                    continue; // prepinac: druhe kliknutie zavrie
                }
                let Some((bar, dpi)) = bar_geometry(&monitor_id) else {
                    continue;
                };
                // skryvana lista sa pri otvoreni popoveru (aj zo skratky) vysunie a ostane, kym je otvoreny
                crate::appbar::reveal(&monitor_id);
                let w = w_lp * dpi as i32 / 96;
                let mut h = h_lp * dpi as i32 / 96;
                // popover nikdy nepresiahne pracovnu plochu monitora (pri vysokom DPI / nizkom rozliseni; obsah sa posuva)
                if let Some((work, _)) = monitor_at(bar.left, bar.top) {
                    h = h.min(
                        (work.bottom - bar.bottom - dpi as i32 * 8 / 96).max(dpi as i32 * 200 / 96),
                    );
                }
                let x =
                    (bar.left + anchor_x - w / 2).clamp(bar.left, (bar.right - w).max(bar.left));
                let y = bar.bottom;
                let mut glass = None;
                with_handler(|hd| glass = hd.popover_glass(kind));
                if create_popover(kind, dpi, (x, y), (w, h), false, glass, (0.5, 0.0)).is_none() {
                    continue;
                }
                emit(BarEvent::PopoverOpened { kind });
                crate::appbar::invalidate_all();
            }
            Request::ShowMenu { kind, anchor } => {
                crate::dragdrop::notch_collapse();
                close_ex(false);
                let (dpi, bounds, at, above) = match &anchor {
                    MenuAnchor::Bar { monitor_id, x } => {
                        let Some((bar, dpi)) = bar_geometry(monitor_id) else {
                            continue;
                        };
                        (dpi, bar, (bar.left + x, bar.bottom), false)
                    }
                    MenuAnchor::Screen { x, y } => {
                        let Some((work, dpi)) = monitor_at(*x, *y) else {
                            continue;
                        };
                        (dpi, work, (*x, *y), *y > (work.top + work.bottom) / 2)
                    }
                };
                // velkost zmeria aplikacia (texty v aktualnom jazyku) pre dpi ciloveho monitora
                let mut size = None;
                // SAFETY: DC obrazovky sa uvolni (ReleaseDC) v tom istom bloku.
                unsafe {
                    let dc = GetDC(None);
                    with_handler(|hd| size = hd.popover_size(kind, dpi, dc));
                    ReleaseDC(None, dc);
                }
                let Some(size) = size else { continue };
                let pos = menu_origin(at, size, bounds, above);
                let mut glass = None;
                with_handler(|hd| glass = hd.popover_glass(kind));
                // menu rastie z rohu pri kurzore (nad kurzorom zdola)
                let anchor = (
                    if pos.0 < at.0 { 1.0 } else { 0.0 },
                    // zdola len ked je menu cele nad bodom kliknutia (lista hore: menu pod nou sa vysuva zhora)
                    if pos.1 + size.1 <= at.1 + 2 { 1.0 } else { 0.0 },
                );
                if create_popover(kind, dpi, pos, size, true, glass, anchor).is_none() {
                    continue;
                }
                emit(BarEvent::PopoverOpened { kind });
                crate::appbar::invalidate_all();
            }
            Request::Activate { app, title } => {
                let r = activate_app(&app, &title);
                emit(BarEvent::Activated(r));
            }
            Request::Menu {
                monitor_id,
                x,
                items,
            } => {
                let Some((bar, _)) = bar_geometry(&monitor_id) else {
                    continue;
                };
                let Some(owner) = crate::appbar::bar_hwnd(&monitor_id) else {
                    continue;
                };
                let chosen =
                    // SAFETY: `track_menu` na vlakne UI s platnym vlastnikom; menu sa znici vo vnutri.
                    unsafe { track_menu(owner, bar.left + x, bar.bottom, TPM_TOPALIGN, &items) };
                if chosen != 0 {
                    emit(BarEvent::MenuChosen(chosen));
                }
            }
            Request::MenuAt { x, y, items } => {
                let Some(owner) = crate::appbar::ctl_hwnd() else {
                    continue;
                };
                // ikona v oblasti oznameni byva dole: menu rastie nahor (Windows ho v pripade potreby posunie)
                let chosen =
                    // SAFETY: `track_menu` na vlakne UI s platnym vlastnikom; menu sa znici vo vnutri.
                    unsafe { track_menu(owner, x, y, TRACK_POPUP_MENU_FLAGS(0x0020), &items) };
                if chosen != 0 {
                    emit(BarEvent::MenuChosen(chosen));
                }
            }
            Request::SetHidden(h) => {
                if h {
                    close_now();
                }
                crate::appbar::set_hidden(h);
            }
            Request::SetMonitors(sel) => crate::appbar::set_monitors(sel),
            Request::SetAutoHide(on) => crate::appbar::set_auto_hide(on),
            // SAFETY: retazce ukoncene nulou ziju pocas ShellExecuteW; cesta a parametre pochadzaju z nasho kodu (nie z cudzich dat).
            Request::Launch { path, args } => unsafe {
                // spustene okno (nastavenia) smie prevziat popredie (inak by len zablikalo na paneli uloh)
                let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(u32::MAX);
                let p: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                let a: Vec<u16> = args.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = ShellExecuteW(
                    None,
                    w!("open"),
                    PCWSTR(p.as_ptr()),
                    PCWSTR(a.as_ptr()),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                );
            },
        }
    }
}

/// Zobrazi kontextove menu v bode obrazovky a vrati zvolene id (0 = nic). Dokumentovana poziadavka `TrackPopupMenu`:
/// vlastnik v poprede, po zatvoreni `WM_NULL`; potom sa vrati popredie povodnemu oknu (lista je neaktivujuca a
/// fokus si nema nechat).
///
/// # Safety
/// `owner` musi byt platne okno tohto vlakna.
unsafe fn track_menu(
    owner: HWND,
    x: i32,
    y: i32,
    align: TRACK_POPUP_MENU_FLAGS,
    items: &[(u32, String)],
) -> u32 {
    // SAFETY: menu sa vytvori a znici v tomto bloku; texty su retazce ukoncene nulou zijuce pocas volani.
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return 0 };
        for (id, text) in items {
            if *id == 0 {
                let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            } else {
                let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(m, MF_STRING, *id as usize, PCWSTR(t.as_ptr()));
            }
        }
        let previous = GetForegroundWindow();
        let _ = SetForegroundWindow(owner);
        let r = TrackPopupMenu(m, TPM_RETURNCMD | align, x, y, None, owner, None);
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        if !previous.0.is_null() && previous != owner {
            let _ = SetForegroundWindow(previous);
        }
        r.0 as u32
    }
}

/// Zavriet popover pri zmene geometrie/fullscreene (bez cakania na frontu nie - cez frontu, mimo vypozicky).
pub(crate) fn close_soon() {
    if popover_open().is_some() {
        request(Request::ClosePopover);
    }
}

/// Test (`--test-menu`): posle klavesu priamo oknu otvoreneho popoveru (v procese; nie je to vstup na ploche).
pub fn post_key_to_popover(vk: u32) -> bool {
    match POP.with(|p| p.get()) {
        // SAFETY: PostMessageW len s cislami na nas popover.
        Some(p) => unsafe {
            PostMessageW(
                Some(HWND(p.hwnd as *mut _)),
                WM_KEYDOWN,
                WPARAM(vk as usize),
                LPARAM(0),
            )
            .is_ok()
        },
        None => false,
    }
}

/// Test (`--test-tools`): posle oknu otvoreneho popoveru napisany znak (`WM_CHAR`), ako keby ho napisal pouzivatel.
pub fn post_char_to_popover(c: char) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::WM_CHAR as CHAR;
    let Some(p) = POP.with(|p| p.get()) else {
        return false;
    };
    let mut buf = [0u16; 2];
    // SAFETY: sprava platnemu oknu tohto procesu.
    unsafe {
        c.encode_utf16(&mut buf).iter().all(|u| {
            PostMessageW(
                Some(HWND(p.hwnd as *mut _)),
                CHAR,
                WPARAM(*u as usize),
                LPARAM(0),
            )
            .is_ok()
        })
    }
}

/// Test (`--test-tools`): obdlznik otvoreneho popoveru v suradniciach obrazovky (x, y, sirka, vyska).
pub fn popover_screen_rect() -> Option<(i32, i32, i32, i32)> {
    let p = POP.with(|p| p.get())?;
    let mut r = RECT::default();
    // SAFETY: citanie rozmerov vlastneho okna.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(HWND(p.hwnd as *mut _), &mut r)
            .ok()?;
    }
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// Test (`--test-ai`): posle oknu otvoreneho popoveru stlacenie a uvolnenie laveho tlacidla v bode `(x, y)` (px klienta).
pub fn post_click_to_popover(x: i32, y: i32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN as DOWN;
    match POP.with(|p| p.get()) {
        // SAFETY: PostMessageW len s cislami (suradnice zbalene do lParam).
        Some(p) => unsafe {
            let h = HWND(p.hwnd as *mut _);
            let lp = LPARAM(((y as isize & 0xFFFF) << 16) | (x as isize & 0xFFFF));
            let a = PostMessageW(Some(h), DOWN, WPARAM(1), lp).is_ok();
            let b = PostMessageW(Some(h), WM_LBUTTONUP, WPARAM(0), lp).is_ok();
            a && b
        },
        None => false,
    }
}

/// Test (`--test-ai-real`): bod `(x, y)` klientskej oblasti otvoreneho popoveru v suradniciach obrazovky.
pub fn popover_client_to_screen(x: i32, y: i32) -> Option<(i32, i32)> {
    let p = POP.with(|p| p.get())?;
    let mut pt = POINT { x, y };
    // SAFETY: vystupny POINT je lokalny.
    unsafe {
        windows::Win32::Graphics::Gdi::ClientToScreen(HWND(p.hwnd as *mut _), &mut pt)
            .as_bool()
            .then_some((pt.x, pt.y))
    }
}

/// Test: trieda okna v popredi a ci je to otvoreny popover (do logu).
pub fn foreground_description() -> String {
    // SAFETY: buffery s danou dlzkou; okno moze medzitym zaniknut (chyba bez dereferencie).
    unsafe {
        let fg = GetForegroundWindow();
        let mut buf = [0u16; 64];
        let n = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(fg, &mut buf);
        let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let popover = POP
            .with(|p| p.get())
            .is_some_and(|p| p.hwnd == fg.0 as isize);
        format!("{class}{}", if popover { " (popover)" } else { "" })
    }
}

/// Bod z `lParam` mysacej spravy je mimo klientskej oblasti okna.
fn outside(hwnd: HWND, lp: LPARAM) -> bool {
    let x = (lp.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    let mut rc = RECT::default();
    // SAFETY: vystupny RECT je lokalny.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    x < rc.left || x >= rc.right || y < rc.top || y >= rc.bottom
}

unsafe extern "system" fn popover_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: wndproc popoveru na vlakne UI: wParam/lParam sa pouzivaju len ako cisla; obsluha sa vola mimo vypozicky stavu POP.
    unsafe {
        let kind = POP.with(|p| p.get()).filter(|p| p.hwnd == hwnd.0 as isize);
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT if crate::appbar::handler_busy() => {
                // obsah kresli aplikacia, ktora je prave zaneprazdnena (synchronne prekreslenie z jej volania): nekreslit
                // prazdny / cierny panel, prekreslit o chvilu
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                windows::Win32::UI::WindowsAndMessaging::SetTimer(
                    Some(hwnd),
                    TIMER_REPAINT,
                    15,
                    None,
                );
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_REPAINT => {
                let _ = KillTimer(Some(hwnd), TIMER_REPAINT);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                if let Some(st) = kind.and_then(|p| p.glass) {
                    paint_glass(
                        hdc,
                        kind.map_or(0, |p| p.kind),
                        kind.map_or(96, |p| p.dpi),
                        rc,
                        &st,
                    );
                } else if let Some(p) = kind {
                    let (w, h) = (rc.right, rc.bottom);
                    let mem = CreateCompatibleDC(Some(hdc));
                    let bmp = CreateCompatibleBitmap(hdc, w.max(1), h.max(1));
                    let old = SelectObject(mem, bmp.into());
                    let ctx = PopoverPaint {
                        hdc: mem,
                        w,
                        h,
                        dpi: p.dpi,
                        kind: p.kind,
                        backdrop: None,
                    };
                    with_handler(|hd| hd.paint_popover(&ctx));
                    let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
                    SelectObject(mem, old);
                    let _ = DeleteObject(bmp.into());
                    let _ = DeleteDC(mem);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            // Alt+medzernik / samotny Alt (napr. este drzany Alt zo skratky, ktora popover otvorila): ziadna systemova ponuka
            // ani rezim menu (prehltol by dalsiu klavesu); SC_KEYMENU = 0xF100
            0x0112 if kind.is_some() && (wp.0 & 0xFFF0) == 0xF100 => LRESULT(0),
            // WM_ACTIVATE sa posiela synchronne -> len zaradit poziadavku, nie emitovat
            WM_ACTIVATE => {
                if (wp.0 & 0xFFFF) == WA_INACTIVE && kind.is_some() && !crate::dragdrop::dragging()
                {
                    request(Request::ClosePopover);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if let Some(p) = kind {
                    if wp.0 == 0x1B {
                        request(Request::ClosePopover);
                    } else {
                        // stav klavesu vlakna, ktore spracuva tuto spravu; nic sa nemeni
                        let shift =
                            windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x10) < 0;
                        let ctrl =
                            windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) < 0;
                        emit(BarEvent::PopoverKey {
                            kind: p.kind,
                            vk: wp.0 as u32,
                            shift,
                            repeat: (lp.0 >> 30) & 1 != 0,
                            ctrl,
                        });
                    }
                }
                LRESULT(0)
            }
            // napisany znak: surrogaty sa zlozia; riadiace znaky (Backspace, Enter, Tab, Esc, Ctrl+pismeno) sa neposielaju
            // (ide o `PopoverKey`), takze sem prichadza len pisatelny text
            WM_CHAR => {
                if let Some(p) = kind {
                    let u = wp.0 as u32;
                    let ch = if (0xD800..0xDC00).contains(&u) {
                        HIGH.with(|h| h.set(u as u16));
                        None
                    } else if (0xDC00..0xE000).contains(&u) {
                        let hi = HIGH.with(|h| h.replace(0)) as u32;
                        (hi != 0)
                            .then(|| char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (u - 0xDC00)))
                            .flatten()
                    } else {
                        HIGH.with(|h| h.set(0));
                        char::from_u32(u)
                    };
                    if let Some(c) = ch.filter(|c| !c.is_control()) {
                        emit(BarEvent::PopoverChar {
                            kind: p.kind,
                            ch: c,
                        });
                    }
                }
                LRESULT(0)
            }
            // zachytenie mysi pri menu: klik (lave aj prave tlacidlo) mimo okna ho zatvori a nic dalej nedostane
            WM_RBUTTONDOWN | WM_RBUTTONUP | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE
                if kind.is_some_and(|p| p.menu) && outside(hwnd, lp) =>
            {
                if msg != WM_MOUSEMOVE {
                    request(Request::ClosePopover);
                } else if let Some(p) = kind {
                    // hover mimo menu: zrusit aktivnu polozku
                    emit(BarEvent::PopoverPointer {
                        kind: p.kind,
                        pointer: PointerKind::Leave,
                        x: -1,
                        y: -1,
                    });
                }
                LRESULT(0)
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP if kind.is_some_and(|p| p.menu) => LRESULT(0),
            // pristupnost: menu ako `MENUPOPUP` s polozkami
            WM_GETOBJECT if kind.is_some_and(|p| p.menu) && lp.0 as i32 == OBJID_CLIENT.0 => {
                let acc = crate::menu_access::MenuAccessible::create(hwnd);
                LresultFromObject(&IAccessible::IID, wp, &acc)
            }
            // panel (napr. AI): tlacidla zo snimky, ak ju aplikacia publikovala
            WM_GETOBJECT
                if kind.is_some()
                    && lp.0 as i32 == OBJID_CLIENT.0
                    && !crate::menu_access::snapshot_of(crate::menu_access::Kind::Panel)
                        .0
                        .is_empty() =>
            {
                let acc = crate::menu_access::MenuAccessible::create_kind(
                    hwnd,
                    crate::menu_access::Kind::Panel,
                );
                LresultFromObject(&IAccessible::IID, wp, &acc)
            }
            // `accDoDefaultAction`: klik do stredu polozky `wp`
            crate::access::WM_TRANSOM_INVOKE => {
                if let Some(p) = kind {
                    let snap = if p.menu {
                        crate::menu_access::snapshot()
                    } else {
                        crate::menu_access::snapshot_of(crate::menu_access::Kind::Panel)
                    };
                    if let Some(e) = snap.0.get(wp.0).cloned() {
                        emit(BarEvent::PopoverPointer {
                            kind: p.kind,
                            pointer: PointerKind::Click,
                            x: e.x + e.w / 2,
                            y: e.y + e.h / 2,
                        });
                    }
                }
                LRESULT(0)
            }
            // koliesko: posun obsahu popoveru (panel AI); `wParam` high word = delta so znamienkom
            WM_MOUSEWHEEL => {
                if let Some(p) = kind {
                    emit(BarEvent::PopoverWheel {
                        kind: p.kind,
                        delta: ((wp.0 >> 16) as u16 as i16) as i32,
                    });
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                // mys zachytilo ine okno (nie nasa vlastna zmena): menu sa zatvori
                if kind.is_some_and(|p| p.menu) && lp.0 != hwnd.0 as isize {
                    request(Request::ClosePopover);
                }
                // panel: mys pocas tahania vzalo ine okno (Alt+Tab, systemove okno) - tahanie sa ukonci
                if let Some(p) = kind.filter(|p| !p.menu) {
                    if lp.0 != 0 && lp.0 != hwnd.0 as isize {
                        emit(BarEvent::PopoverPointer {
                            kind: p.kind,
                            pointer: PointerKind::Leave,
                            x: -1,
                            y: -1,
                        });
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                if let Some(p) = kind {
                    if msg == WM_MOUSEMOVE && !TRACKING.with(|t| t.replace(true)) {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut t);
                    }
                    // suradnice v tele panela (bez pasov vyduteho prechodu)
                    let fl = p.glass.map_or(0, |g| popover_flare(&g, p.dpi));
                    let x = (lp.0 & 0xFFFF) as u16 as i16 as i32 - fl;
                    let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                    // panel: pocas stlacenia drzi mys (tahanie posuvnika pokracuje aj mimo okna, napr. hlasitost
                    // rychlo az na 0); menu ma mys zachytenu stale
                    if msg == WM_LBUTTONDOWN && !p.menu {
                        SetCapture(hwnd);
                    }
                    emit(BarEvent::PopoverPointer {
                        kind: p.kind,
                        pointer: match msg {
                            WM_MOUSEMOVE => PointerKind::Move,
                            WM_LBUTTONDOWN => PointerKind::Down,
                            _ => PointerKind::Click,
                        },
                        x,
                        y,
                    });
                    if msg == WM_LBUTTONUP && !p.menu && GetCapture() == hwnd {
                        let _ = ReleaseCapture();
                        // mys uz moze byt mimo okna: nove sledovanie posle odchod hned
                        TRACKING.with(|t| t.set(true));
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut t);
                    }
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                TRACKING.with(|t| t.set(false));
                // pocas tahania (mys zachytena panelom) odchod nie je koniec: dalsi pohyb sledovanie obnovi
                if kind.is_some_and(|p| !p.menu) && GetCapture() == hwnd {
                    return LRESULT(0);
                }
                if let Some(p) = kind {
                    emit(BarEvent::PopoverPointer {
                        kind: p.kind,
                        pointer: PointerKind::Leave,
                        x: -1,
                        y: -1,
                    });
                }
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_ANIM => {
                PACE.with(|p| {
                    if let Some(p) = p.borrow().as_ref() {
                        p.ack();
                    }
                });
                anim_step(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                TRACKING.with(|t| t.set(false));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
