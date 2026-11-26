//! transom-settings: okno nastaveni (samostatny proces, spusta sa z kontextoveho menu lisaty; jedna instancia).
//! Kresli sa tym istym rendererom ako lista (GDI, `transom-render`). Kazda zmena sa hned atomicky ulozi
//! (`transom-config`) a lista dostane broadcast `transom-config-changed-v1`, takze sa prejavi nazivo.
//! Ak je konfiguracia novsej verzie alebo poskodena, okno ju len zobrazi a neuklada.
//! Vsetky texty su z `transom-i18n`; zmena jazyka sa prejavi hned (aj v nazve okna).
#![windows_subsystem = "windows"]

#[cfg(test)]
mod a11y_tests;
mod appearance_page;
#[cfg(test)]
mod archive_tests;
mod audio_page;
mod cc;
mod chrome;
mod displays_page;
mod dnd;
mod edit_names;
mod extras_page;
mod focus;
mod kbd_actions;
#[cfg(test)]
mod keyboard_tests;
mod media_page;
#[cfg(test)]
mod nav_tests;
mod notch_page;
mod page_a11y;
mod periph_page;
mod shot;
mod sound_page;
#[cfg(test)]
mod sound_tests;
mod toolgroup_page;
mod tools_page;
mod transfer_dialog;
mod transfer_io;
mod transfer_page;
#[cfg(test)]
mod transfer_tests;
mod updates_page;
mod weather_page;
mod welcome_page;

use dnd::{apply_drop, column_items, drop_target, ColGeom};
use std::cell::RefCell;
use transom_config::{Config, DateMode, FirstDay, Group, HourMode, Layout, Tri};
use transom_core::item::{Action, Flex, Glyph, ItemId, Part, Side, StatusItem, Tone};
use transom_i18n::{t, tf, Lang};
use transom_platform::clock as pclock;
use transom_render::brand::{self, Palette, Rgba};
use transom_render::design::{self, Kind, Ty};
use transom_render::navicons;
use transom_render::{
    render_bar, Color, Font, GdiPainter, HAlign, Interaction, Painter, Rect, Size, TextStyle,
    Theme, ThemeMode,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, InvalidateRect, SelectObject, SetBkColor, SetTextColor,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, HDC,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::Dialogs::{ChooseColorW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetClientRect, GetMessageW,
    GetWindowTextLengthW, GetWindowTextW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SendMessageW, SetForegroundWindow, SetTimer,
    SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, CW_USEDEFAULT, EN_CHANGE, HMENU,
    IDC_ARROW, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_SETFONT, WM_SETICON,
    WM_SIZE, WM_TIMER, WNDCLASSW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_TABSTOP,
};

const WIN_W_LP: i32 = 840;
const WIN_H_LP: i32 = 720;
const NAV_W_LP: i32 = 200;
// hit-test kody a spravy vlastneho ramu (hodnoty z winuser.h)
const HTCLIENT: u32 = 1;
const HTCAPTION: u32 = 2;
const HTMINBUTTON: u32 = 8;
const HTMAXBUTTON: u32 = 9;
const HTTOP: u32 = 12;
const HTTOPLEFT: u32 = 13;
const HTTOPRIGHT: u32 = 14;
const HTCLOSE: u32 = 20;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_ACTIVATE: u32 = 0x0006;
const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_GETOBJECT: u32 = 0x003D;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCHITTEST: u32 = 0x0084;
const WM_NCMOUSEMOVE: u32 = 0x00A0;
const WM_NCLBUTTONDOWN: u32 = 0x00A1;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_SYSCOMMAND: u32 = 0x0112;
const TME_LEAVE_NC: u32 = 0x0002 | 0x0010;
const TIMER_PREVIEW: usize = 1;
/// Odlozene hladanie miesta pocas pisania.
const TIMER_WX: usize = 41;
/// Vysledok hladania miesta (z vlakna).
const WM_APP_WX: u32 = 0x8000 + 41;
/// Vysledok overenia polohy (wParam: 0 povolene, 1 zakazane, 2 nedostupne).
const WM_APP_GEO: u32 = 0x8000 + 42;

/// Posledny vysledok hladania miesta (cislo dotazu, vysledok); okno ho prevezme pri `WM_APP_WX`.
type WxResult = (u64, Result<Vec<transom_config::WeatherPlace>, ()>);
static WX_RESULT: std::sync::Mutex<Option<WxResult>> = std::sync::Mutex::new(None);

/// Spusti hladanie miesta pre aktualny text (na vlakne; vysledok pride ako `WM_APP_WX`).
fn start_wx_search(hwnd: HWND) {
    let Some((gen, q)) = with_ui(|ui| (ui.wx.gen, ui.wx.query.clone())) else {
        return;
    };
    if q.chars().count() < weather_page::MIN_QUERY {
        return;
    }
    let lang = transom_i18n::lang()
        .code()
        .split('-')
        .next()
        .unwrap_or("en")
        .to_string();
    let target = hwnd.0 as isize;
    let _ = std::thread::Builder::new()
        .name("transom-wx-search".into())
        .spawn(move || {
            let country = transom_platform::geoloc::region_country().unwrap_or_default();
            let r = weather_page::search(&q, &lang, &country);
            if let Ok(mut g) = WX_RESULT.lock() {
                // starsie hladanie, ktore dobehlo neskor, neprepise novsiu odpoved
                if g.as_ref().is_none_or(|(old, _)| *old < gen) {
                    *g = Some((gen, r));
                }
            }
            // SAFETY: okno moze byt medzitym zatvorene - PostMessage na neplatne okno len zlyha
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(target as *mut _)),
                    WM_APP_WX,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
}
/// Zivy nahlad posuvneho textu (~30 snimok/s, len na stranke Media pri zapnutom posuve).
const TIMER_MEDIA: usize = 2;
/// Odlozene ulozenie hodnoty zo vstupneho pola: ticho po poslednom znaku (ms).
const TIMER_COMMIT: usize = 3;
/// Snimky plynuleho posuvania (s obnovou obrazovky).
const TIMER_GLIDE: usize = 0x6C;
const COMMIT_MS: u32 = 450;
/// Oznamenia vstupneho pola (`EN_SETFOCUS` / `EN_KILLFOCUS` nie su v `windows` ako konstanty typu u32).
const EN_KILLFOCUS_CODE: u32 = 0x0200;
const EN_SETFOCUS_CODE: u32 = 0x0100;
/// ID natívnych vstupnych poli (HEX, R, G, B) = `EDIT_ID0 + i`.
const EDIT_ID0: usize = 100;
/// `EM_SETLIMITTEXT`
const EM_SETLIMITTEXT: u32 = 0xC5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Layout,
    /// Vybezok pod listou (co sa v nom ukazuje).
    Notch,
    /// Skupina dlazdic v paneli Nastroje.
    ToolGroup,
    /// Podstranky nastrojov (`Tools` = Polica, ako doteraz).
    Clipboard,
    Notes,
    Shots,
    Launcher,
    Displays,
    Media,
    Appearance,
    Clock,
    Ai,
    Sound,
    Audio,
    Peripherals,
    Tools,
    Extras,
    Language,
    Transfer,
    /// Aktualizacie (kontrola novych vydani).
    Updates,
    /// Vitajte: zakladne nastavenie (pri prvom spusteni).
    Welcome,
}

/// Velke kategorie nastaveni (bocny panel) a ich podkategorie (zalozky nad obsahom): (kluc nazvu, index ikony v
/// `navicons::ALL`, stranky so svojimi nazvami).
/// Kategoria: (kluc nazvu, index ikony, podstranky s nazvami).
type Cat = (&'static str, usize, &'static [(Page, &'static str)]);

const CATS: [Cat; 4] = [
    (
        "cat.bar",
        0,
        &[
            (Page::Layout, "nav.layout"),
            (Page::Displays, "nav.displays"),
            (Page::Appearance, "nav.appearance"),
            (Page::Notch, "notch.title"),
        ],
    ),
    (
        "cat.items",
        7,
        &[
            (Page::Clock, "nav.clock"),
            (Page::Media, "nav.media"),
            (Page::Ai, "nav.ai"),
            (Page::Audio, "nav.audio"),
            (Page::Peripherals, "nav.peripherals"),
            (Page::Extras, "nav.extras"),
        ],
    ),
    (
        "cat.tools",
        9,
        &[
            (Page::ToolGroup, "tls.group"),
            (Page::Tools, "tools.sec_shelf"),
            (Page::Clipboard, "tools.sec_clipboard"),
            (Page::Notes, "tools.sec_notes"),
            (Page::Shots, "tools.sec_shots"),
            (Page::Launcher, "lau.title"),
        ],
    ),
    (
        "cat.system",
        11,
        &[
            (Page::Welcome, "nav.welcome"),
            (Page::Sound, "nav.sound"),
            (Page::Language, "nav.language"),
            (Page::Transfer, "nav.transfer"),
            (Page::Updates, "nav.updates"),
        ],
    ),
];

/// Kategoria stranky (index v `CATS`).
fn cat_of(pg: Page) -> usize {
    CATS.iter()
        .position(|(_, _, pages)| pages.iter().any(|(p, _)| *p == pg))
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Nav(Page),
    /// Velka kategoria v bocnom paneli (otvori naposledy pouzitu podkategoriu).
    Cat(u8),
    Chip(String),
    /// Zaskrtavacie policko "Zobrazit v liste" - jedine miesto, kde sa meni viditelnost polozky.
    Show(String),
    ResetLayout,
    Preset(usize),
    Hour(HourMode),
    Zero(Tri),
    Seconds(bool),
    /// Den v tyzdni: 0 = nie, 1 = skratka, 2 = cely nazov.
    Weekday(u8),
    Date(DateMode),
    First(FirstDay),
    Src(u8),
    Interval(u32),
    Provider(&'static str),
    Mode(&'static str),
    DisableAll,
    DeleteData,
    CancelDelete,
    CopyStatusline,
    /// Pripojenie Claude Code: pripojit, nahradit cudzi statusLine (dvojkrokovo), ponechat, vratit, skusit.
    CcConnect,
    CcReplace,
    CcKeep,
    CcDisconnect,
    CcTest,
    /// Archiv dennych suctov: zapnut / pozastavit, retencia (mesiace; 0 = bez obmedzenia), zmazat (dvojkrokovo).
    ArchiveEnable,
    ArchivePause,
    ArchiveRetention(u32),
    ArchiveDelete,
    ArchiveCancel,
    Lang(&'static str),
    MarqueeOn(bool),
    /// Pravidelna kontrola aktualizacii.
    UpdAuto(bool),
    /// Ikona dostupnej aktualizacie na liste.
    UpdIcon(bool),
    /// Poziadat listu o kontrolu aktualizacii hned.
    UpdCheck,
    UpdInstall,
    /// Volba stranky Vitajte (id, hodnota).
    Wel(&'static str, bool),
    /// Hotovo na stranke Vitajte: zatvori okno nastaveni.
    WelDone,
    MarqueeDir(&'static str),
    MarqueeSpeed(&'static str),
    MarqueePaused(bool),
    /// Farba pozadia podla temy (zrusi vlastnu farbu).
    ThemeColor,
    Swatch(usize),
    /// Systemovy dialog vyberu farby.
    Pick,
    Transparent,
    /// Draha posuvnika nepriehladnosti (tahanie sa riesi pri mysi).
    OpacityTrack,
    OpacityStep(i32),
    /// Zvuky rozhrania: zapnutie (vypnutie = stlmenie), posuvnik hlasitosti, krok -/+ a ukazka.
    SoundToggle,
    SoundTrack,
    SoundStep(i32),
    SoundPreview,
    /// Rychla akcia panela Zvuk (zapnut / vypnut v paneli).
    AudioQuick(&'static str),
    /// Baterie periferii: zdroj (zapnut / vypnut), pocet zariadeni v liste (0 = vsetky), interval v sekundach.
    PeriphCount(u32),
    PeriphInterval(u32),
    ToolsToggle(&'static str),
    ToolsCycle(&'static str),
    ToolsClear(&'static str),
    ToolsAddFolder(&'static str),
    ToolsRemoveFolder(&'static str, usize),
    /// Pridanie / odstranenie aplikacie zo zoznamu, z ktorej sa schranka nezachytava (index v `clipboard.excluded_apps`).
    ToolsAddApp,
    ToolsRemoveApp(usize),
    /// Skratka vyhladavaca: zachytit novu (prepinac rezimu zachytavania), vypnut, obnovit predvolenu.
    ToolsHotkey,
    ToolsHotkeyOff,
    ToolsHotkeyDefault,
    /// Farba textu lisaty: rezim (`auto`, `light`, `dark`) a vlastna farba (systemovy vyber farby).
    TextMode(&'static str),
    TextPick,
    /// Ochrana OLED: hlavny prepinac (`enabled`) a podvolby.
    OledToggle(&'static str),
    /// Doplnky: casovac (minuty), zvuk, vytazenie, skratka mikrofonu, ikona mikrofonu, kalendare ICS.
    ExWork(u32),
    ExBreak(u32),
    ExLong(u32),
    ExRounds(u32),
    ExSound,
    ExSys(&'static str),
    ExSysInterval(u32),
    ExMicKey(&'static str),
    ExMicShow,
    IcsAdd,
    IcsRemove(usize),
    ResetAppearance,
    /// Stranka Displays: rezim (all / primary / selected), zapnutie monitora (index v `Ui.monitors`), odstranenie ulozeneho
    /// nepripojeneho monitora, identifikacia.
    DispMode(&'static str),
    DispToggle(usize),
    DispRemove(String),
    DispIdentify,
    /// Formular rucnych datumov (kalendar): poskytovatel, druh, opakovanie, interval, pridat/ulozit, zrusit, upravit/odstranit
    /// zaznam `i`.
    DateProvider(u8),
    DateKind(u8),
    DateRepeat(u8),
    DateEvery(i32),
    DateAdd,
    DateCancel,
    DateEdit(usize),
    DateRemove(usize),
    /// Kalendar pri poli datumu: otvorit / zavriet, posun mesiaca, vyber dna.
    DateCal,
    DateCalMonth(i32),
    DateCalDay(u32),
    /// Ikona transom v oblasti oznameni (zapnut / vypnut).
    TrayIcon,
    /// Pocasie: mesto zo schranky / zrusit.
    /// Pridat vysledok hladania (index) a zvolit ho pre listu.
    WeatherPick(usize),
    /// Zvolit ulozene miesto pre listu (`None` = pocasie vypnute).
    WeatherUse(Option<usize>),
    /// Zvolit polohu zariadenia.
    WeatherDevice,
    WeatherRemove(usize),
    /// Otvorit nastavenia polohy Windows.
    WeatherGeoSettings,
    /// Volba vybezku (id z `notch_page::NOTCH_OPTS`).
    NotchOpt(&'static str),
    /// Dlazdica skupiny Nastroje (zapnut / vypnut).
    ToolTile(&'static str),
    /// Rozlozenie: pridat oddelovac na koniec skupiny / odstranit oddelovac.
    AddSep(Group),
    RemoveSep(String),
    /// Cierne sklo pod panelmi / pod listou (zapnut / vypnut).
    GlassPanels,
    GlassBar,
    /// Nepriehladnost skla panelov (%).
    GlassOpacity(u32),
    /// Export / import nastaveni: otvorit dialog, sekcia nahladu (index v `Section::ALL`), importovat vybrane (dvojkrokovo), zrusit nahlad.
    TransferExport,
    TransferImport,
    TransferSection(u8),
    TransferApply,
    TransferCancel,
}

/// Volba v riadku nastaveni: (text, akcia, aktivna).
type Choice = (String, Hit, bool);

/// Stav casti "Archiv" na stranke AI.
#[derive(Clone, Debug, Default)]
struct ArchiveUi {
    info: transom_usage::archive::Info,
    /// Prve kliknutie na "Zmazat" len pyta potvrdenie.
    confirm_delete: bool,
}

/// Stav tlacidiel vlastnej hlavicky.
#[derive(Clone, Copy, Debug)]
struct ChromeState {
    hover: Option<chrome::Btn>,
    pressed: Option<chrome::Btn>,
    /// Tlacidlo s fokusom klavesnice (zobrazuje sa len pri ovladani klavesnicou).
    focus: Option<chrome::Btn>,
    active: bool,
    maximized: bool,
    /// `TrackMouseEvent(TME_LEAVE | TME_NONCLIENT)` je aktivny.
    tracking: bool,
}

impl Default for ChromeState {
    fn default() -> Self {
        ChromeState {
            hover: None,
            pressed: None,
            focus: None,
            active: true,
            maximized: false,
            tracking: false,
        }
    }
}

struct Ui {
    cfg: Config,
    /// Ovladace stranky vratane tych mimo viditelneho okna (poloha pri poslednom kresleni, bez orezania): kruh fokusu klavesnice.
    all_hits: Vec<(Rect, Hit)>,
    /// Ovladac kreslenej stranky s fokusom klavesnice (identita, nie poloha).
    kfocus: Option<Hit>,
    /// Nazvy ovladacov zachytene pri poslednom kresleni (pre citacky obrazovky) a nazov skupiny volieb prave kreslenej.
    acc: Vec<page_a11y::AccMeta>,
    acc_group: Option<String>,
    /// Cielove akcie poloziek posledneho zverejneneho MSAA snimku (index = child ID - 1).
    acc_targets: Vec<page_a11y::Target>,
    /// Posledne oznamene tlacidlo hlavicky s fokusom (udalost fokusu sa posiela len pri zmene).
    last_title_focus: Option<usize>,
    /// Export / import nastaveni: stav nahladu, cakajuca operacia dialogu a priznak, ze dialog prave bezi.
    transfer: transfer_page::Transfer,
    pending_xfer: Option<transfer_page::XferOp>,
    transfer_busy: bool,
    /// Staticke texty stranky pre citacku obrazovky (nadpis nahladu, zmeny, ignorovane), zachytene pri kresleni.
    acc_statics: Vec<String>,
    /// Relacia upravy pola farby (hodnota spred editacie pre Esc).
    edit_session: Option<appearance_page::EditSession>,
    /// Rozpracovany klavesnicovy presun polozky rozlozenia.
    moving: Option<dnd::MoveSession>,
    /// Po nasledujucom kresleni zobrazit celu tuto polozku (po presune).
    reveal: Option<Hit>,
    /// Stav suboru, z ktoreho `cfg` vychadza (po poslednom nacitani alebo ulozeni): ukladanie zluci len zmeny oproti nemu,
    /// takze zmena listy alebo inej instancie nezanikne (`transom_config::save_merged`).
    base: Config,
    read_only: bool,
    note: Option<String>,
    page: Page,
    /// Rozbalene kategorie v strome bocneho panela.
    expanded: [bool; 4],
    /// Stranka, pre ktoru sa strom naposledy kreslil (nova stranka rozbali svoju kategoriu).
    nav_seen: Option<Page>,
    /// Naposledy rozbalena kategoria (pri nedostatku miesta ostane rozbalena spolu s kategoriou otvorenej stranky).
    nav_last: Option<usize>,
    hover: Option<Hit>,
    hits: Vec<(Rect, Hit)>,
    cols: Vec<ColGeom>,
    /// Tahana polozka a aktualna poloha mysi.
    drag: Option<(String, i32, i32)>,
    /// Kliknutie na chip bez posunu = ziadna akcia; posun > prah = tahanie.
    press: Option<(String, i32, i32)>,
    status: String,
    theme: Theme,
    /// Prve kliknutie na "Zmazat" len pyta potvrdenie; zrusi sa pri prechode na inu stranku.
    confirm_delete: bool,
    /// Pripojenie Claude Code (stranka AI).
    cc: cc::Cc,
    /// Archiv dennych suctov (stranka AI): zhrnutie suboru a druhy krok mazania.
    arc: ArchiveUi,
    /// Stav vlastnej hlavicky okna (hover / stlacenie / fokus klavesnice / aktivne / maximalizovane).
    chrome: ChromeState,
    /// Jazyk zobrazenia Windows (BCP 47) pre volbu "Podla Windows" - len sa cita.
    windows_lang: String,
    /// Natívne vstupne polia HEX, R, G, B (HWND; 0 = este nevytvorene).
    edits: [isize; 6],
    /// Karta pocasia (hladanie, poloha).
    wx: weather_page::WxUi,
    /// Pole hladania miesta: kde ho nakreslila stranka Doplnky (a pri akom posune), kde lezi nativne pole.
    wx_rect: Rect,
    wx_scroll: i32,
    wx_placed: Rect,
    /// Pismo a stetec vstupnych poli (a DPI, pre ktore su vytvorene).
    edit_font: isize,
    edit_brush: isize,
    edit_dpi: u32,
    /// Prave sa prepisuju texty poli programovo (zmena sa nepocita ako pouzivatelova).
    syncing: bool,
    /// Pole `i` obsahuje neplatnu hodnotu.
    bad: [bool; 4],
    /// Tahanie posuvnika nepriehladnosti (nahlad hned, ulozenie az pri uvolneni mysi).
    drag_opacity: bool,
    /// Draha posuvnika z posledneho kreslenia.
    track: Rect,
    /// Tahanie posuvnika hlasitosti zvukov (nahlad hned, ulozenie a ukazka az pri uvolneni mysi).
    drag_sound: bool,
    /// Draha posuvnika hlasitosti zvukov z posledneho kreslenia.
    sound_track: Rect,
    /// Prehravac ukazky zvuku (v tomto procese; mlcky v testoch a pri snimkach).
    sound: transom_sound::Player,
    /// Po ulozeni hodnoty z klavesnice prehrat ukazku.
    sound_preview_pending: bool,
    /// Vymazanie ulozista Nastrojov caka na druhy klik.
    tools_confirm: Option<&'static str>,
    /// Caka vyber priecinka pre vyhladavac ("excluded" / "fallback").
    pending_folder: Option<&'static str>,
    /// Caka vyber aplikacie (.exe) pre vylucenie zo zachytavania schranky.
    pending_app: bool,
    /// Kluc textu poslednej chyby pri pridani aplikacie (zobrazi sa pod zoznamom, kym sa nezmeni dalsia volba).
    apps_msg: Option<&'static str>,
    /// Zachytava sa nova skratka vyhladavaca (dalsia klavesa s modifikatormi sa ulozi; Esc zrusi).
    hotkey_capture: bool,
    /// Kluc textu chyby poslednej zachytenej skratky.
    hotkey_msg: Option<&'static str>,
    /// Zachytavanie skoncilo klavesou: kym su drzane modifikatory, ich uvolnenie (WM_SYSKEYUP, WM_SYSCHAR) sa prehltne,
    /// aby okno neprelo do rezimu menu. Inak sa systemove klavesy (Alt+F4, Alt+medzernik) spracuju normalne.
    hotkey_swallow: bool,
    /// Po uvolneni mysi otvorit dialog vyberu farby (nesmie bezat vo vypozicke stavu).
    pending_pick: bool,
    /// Spustenie s Windows (hodnota v registri, nie konfiguracia; precitana pri otvoreni okna).
    autostart: bool,
    /// Zatvorit okno (Hotovo na stranke Vitajte) - po uvolneni vypozicky stavu.
    pending_close: bool,
    /// Po uvolneni mysi otvorit vyber vlastnej farby textu.
    pending_text_pick: bool,
    /// Hlaska stranky Doplnky (pridanie kalendara).
    extras_msg: Option<String>,
    /// Platna hodnota z pola caka na ulozenie (odlozene o `COMMIT_MS`, alebo hned pri strate fokusu).
    commit_pending: bool,
    /// Zaciatok animacie nahladu posuvneho textu.
    anim_start: std::time::Instant,
    /// Posun obsahu stranky (px, >= 0) a jeho najvacsi povoleny posun z posledneho kreslenia.
    scroll: i32,
    max_scroll: i32,
    /// Plynule posuvanie kolieskom (snimky s obnovou obrazovky).
    glide: transom_render::design::Glide,
    /// Prave prebieha plynule posuvanie: natívne polia su skryte a stranka kresli ich obraz (bez blikania a smuh).
    gliding: bool,
    /// Okno obsahu (posuvna oblast) z posledneho kreslenia - vstupne polia mimo neho sa skryvaju.
    viewport: Rect,
    /// Po kresleni sa zmenil rozsah posuvania: treba dalsi prekreslenie.
    repaint: bool,
    /// Rastrovane znacky (logo, wordmark, navigacne ikony) pre aktualne DPI a temu.
    art: Option<Art>,
    /// Pripojene monitory (pre stranku Displays; obnovuje sa pri otvoreni stranky a pri `WM_DISPLAYCHANGE`).
    monitors: Vec<transom_platform::monitor::MonitorInfo>,
    /// Formular rucnych datumov kalendara (stranka AI) a obdlznik jeho natívneho pola s datumom.
    form: DateForm,
    date_rect: Rect,
    /// Posun obsahu v case posledneho kreslenia (pole sa pri posuvani posuva s obsahom, kym sa nedokresli) a naposledy
    /// umiestnena poloha natívneho pola.
    date_scroll: i32,
    date_placed: Rect,
    /// Karty sekcii prave kreslenej stranky (px obsahu) a zaciatok otvorenej karty: kreslia sa pod obsah (pozri `paint`).
    cards: Vec<Rect>,
    card_top: Option<i32>,
    /// Prave sa kresli riadok volieb (`choice_row`): tlacidla su volby (vyber = nadych + okraj + polotucne).
    in_choice: bool,
    /// Prave sa kreslia riadky zoznamu (odstranit polozku): text tlacidla vlavo.
    btn_left: bool,
}

/// Okraj obsahu karty: karta presahuje stlpec obsahu o `CARD_PAD_X_LP` vlavo aj vpravo (poloha obsahu sa nemeni),
/// vnutorny okraj hore a dole `CARD_PAD_Y_LP`, medzera medzi kartami `CARD_GAP_LP`.
const CARD_PAD_X_LP: i32 = 16;
/// Sirka stlpca popisku riadku volieb (rovnaka na vsetkych strankach - volby zacinaju na rovnakom x).
const CHOICE_LABEL_LP: i32 = 180;
const CARD_PAD_Y_LP: i32 = 14;
const CARD_GAP_LP: i32 = 12;

/// Druh tlacidla nastaveni podla ovladaca (vyznam, nie vzhlad volajuceho): mazanie dat = nebezpecne (druhy krok =
/// `active`), volby a prepinace = vyber, pridanie datumu a import = hlavna akcia; ikona len kde pomaha pri prehliadani.
fn button_kind(h: &Hit) -> (Kind, Option<char>) {
    match h {
        Hit::DeleteData | Hit::ArchiveDelete | Hit::ToolsClear(_) => {
            (Kind::Destructive, Some('\u{E74D}'))
        }
        Hit::DateAdd | Hit::TransferApply | Hit::CcConnect | Hit::WelDone => (Kind::Primary, None),
        Hit::Preset(_)
        | Hit::Lang(_)
        | Hit::ThemeColor
        | Hit::DateEdit(_)
        | Hit::TextMode(_)
        | Hit::GlassOpacity(_) => (Kind::Choice, None),
        Hit::TextPick => (Kind::Choice, Some('\u{E790}')),
        Hit::DateCal => (Kind::Choice, Some('\u{E787}')),
        Hit::ToolsAddFolder(_) | Hit::ToolsAddApp => (Kind::Secondary, Some('\u{E710}')),
        Hit::ToolsRemoveFolder(..)
        | Hit::ToolsRemoveApp(_)
        | Hit::DispRemove(_)
        | Hit::DateRemove(_) => (Kind::Secondary, Some('\u{E711}')),
        Hit::ResetLayout | Hit::ResetAppearance => (Kind::Secondary, Some('\u{E72C}')),
        Hit::SoundPreview => (Kind::Secondary, Some('\u{E768}')),
        Hit::DispIdentify => (Kind::Secondary, Some('\u{E7F4}')),
        Hit::Pick => (Kind::Secondary, Some('\u{E790}')),
        Hit::CcReplace => (Kind::Destructive, None),
        _ => (Kind::Secondary, None),
    }
}

/// Rozpracovany rucny datum: hodnoty volieb, upravovany zaznam a priznak neplatneho datumu.
#[derive(Clone, Debug, PartialEq)]
struct DateForm {
    provider: u8,
    kind: u8,
    repeat: u8,
    every: u32,
    editing: Option<usize>,
    error: bool,
    /// Otvoreny kalendar: zobrazeny mesiac (rok, mesiac).
    cal: Option<(i32, u32)>,
}

impl Default for DateForm {
    fn default() -> Self {
        DateForm {
            provider: 0,
            kind: 0,
            repeat: 0,
            every: 6,
            editing: None,
            error: false,
            cal: None,
        }
    }
}

/// Rastrovane obrazky pre navigaciu (kluc = DPI a tmavost temy).
struct Art {
    key: (u32, bool),
    logo_px: u32,
    logo: Vec<Rgba>,
    word: (Vec<Rgba>, u32, u32),
    icon_px: u32,
    icons: Vec<Vec<Rgba>>,
    icons_sel: Vec<Vec<Rgba>>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    /// Snimky plynuleho posuvania (len kym sa posuva).
    static GLIDE: RefCell<Option<transom_platform::vsync::Pacer>> = const { RefCell::new(None) };
}

fn label(id: &str) -> String {
    if transom_config::separator_no(id).is_some() {
        return t("item.separator");
    }
    // kazda znama polozka ma nazov `item.<id>` (novu polozku tak nemozno zabudnut doplnit)
    if transom_config::KNOWN_ITEMS.contains(&id) {
        t(&format!("item.{id}"))
    } else {
        "?".into()
    }
}

fn clock_opts(c: &transom_config::Clock) -> transom_core::clock::ClockOptions {
    use transom_core::clock::{ClockOptions, DateMode as D, HourMode as H, LeadingZero as Z};
    ClockOptions {
        hour: match c.hour {
            HourMode::Locale => H::Locale,
            HourMode::H12 => H::H12,
            HourMode::H24 => H::H24,
        },
        leading_zero: match c.leading_zero {
            Tri::Locale => Z::Locale,
            Tri::Yes => Z::Yes,
            Tri::No => Z::No,
        },
        seconds: c.seconds,
        weekday: c.weekday,
        date: match c.date {
            DateMode::None => D::None,
            DateMode::Short => D::Short,
            DateMode::Long => D::Long,
            DateMode::Dmy | DateMode::DmySlash | DateMode::Mdy | DateMode::Iso => D::None,
        },
    }
}

/// Text hodin ako v liste: cas podla Windows, den v jazyku transom, datum v zvolenom formate (zhodne s `clock_text`
/// aplikacie).
fn clock_preview(c: &transom_config::Clock) -> String {
    let mut o = clock_opts(c);
    o.weekday = false;
    o.date = transom_core::clock::DateMode::None;
    let time = pclock::format_clock(&o);
    let today = pclock::today();
    let wd = c.weekday.then(|| {
        let i = transom_core::clock::weekday(today);
        t(&format!("wd.{}{i}", if c.weekday_long { "l" } else { "s" }))
    });
    let date = match c.date {
        DateMode::None => None,
        DateMode::Short => Some(pclock::short_date(today)),
        DateMode::Long => Some(pclock::long_date(today)),
        m => Some(date_sample(m)),
    };
    let wd = wd.filter(|w| date.as_ref().is_none_or(|x| !x.contains(w.as_str())));
    transom_core::clock::compose(wd.as_deref(), date.as_deref(), &time)
}

/// Ukazkove polozky pre nahlad lisaty (bez skutocnych dat; kruh AI je oznaceny ako ukazka hodnoty).
fn sample_items(cfg: &Config) -> Vec<StatusItem> {
    let clock = clock_preview(&cfg.clock);
    let remaining = cfg.usage.ring_mode == "remaining";
    let make = |id: ItemId| -> StatusItem {
        let parts = match id {
            ItemId::Search => vec![Part::button(Glyph::Search, Action::Search, true, "")],
            ItemId::Media => vec![
                Part::Art {
                    art: None,
                    action: None,
                },
                Part::button(Glyph::Prev, Action::MediaPrev, true, ""),
                Part::button(Glyph::Pause, Action::MediaToggle, true, ""),
                Part::button(Glyph::Next, Action::MediaNext, true, ""),
                Part::Text {
                    text: "♪ …".into(),
                    flex: Some(Flex {
                        min_lp: 40,
                        max_lp: 120,
                    }),
                    tone: Tone::Normal,
                    action: None,
                },
            ],
            ItemId::Ai => {
                let (fill, key) = if remaining {
                    (60, "ring.left")
                } else {
                    (40, "ring.used")
                };
                vec![Part::Ring {
                    fill: Some(fill),
                    ccw: remaining,
                    muted: false,
                    alert: false,
                    label: tf(key, &[&fill.to_string()]),
                    name: String::new(),
                    action: None,
                }]
            }
            ItemId::Tools => vec![Part::button(Glyph::Tools, Action::ToolsPanel, true, "")],
            ItemId::Audio => vec![Part::button(Glyph::Volume2, Action::AudioPanel, true, "")],
            // ukazka: jedna mys (skutocne zariadenia sa v nastaveniach necitaju)
            ItemId::Peripherals => vec![Part::Device {
                glyph: Glyph::Mouse,
                percent: Some(70),
                charging: false,
                muted: false,
                low: false,
                label: "70 %".into(),
                name: String::new(),
                action: None,
            }],
            ItemId::Battery => vec![
                Part::Battery {
                    percent: 80,
                    charging: false,
                },
                Part::text("80 %"),
            ],
            // ukazka: jeden subor (skutocna Polica sa v nastaveniach necita)
            ItemId::Shelf => vec![Part::File {
                glyph: Glyph::File,
                muted: false,
                label: t("shelf.sample"),
                name: String::new(),
                action: None,
            }],
            ItemId::Keyboard => vec![Part::File {
                glyph: Glyph::Keyboard,
                muted: false,
                label: "SK".into(),
                name: String::new(),
                action: None,
            }],
            ItemId::Timer => vec![Part::File {
                glyph: Glyph::Timer,
                muted: false,
                label: "24:59".into(),
                name: String::new(),
                action: None,
            }],
            ItemId::Brightness => vec![Part::File {
                glyph: Glyph::Brightness,
                muted: false,
                label: "60 %".into(),
                name: String::new(),
                action: None,
            }],
            ItemId::SysMon => vec![Part::text("CPU 12 %  RAM 48 %")],
            ItemId::Privacy => vec![Part::button(Glyph::Mic, Action::PrivacyPanel, true, "")],
            ItemId::ToolShelf => vec![Part::button(Glyph::Shelf, Action::ToolsShelf, true, "")],
            ItemId::ToolClipboard => {
                vec![Part::button(
                    Glyph::Clipboard,
                    Action::ToolsClipboard,
                    true,
                    "",
                )]
            }
            ItemId::ToolNotes => vec![Part::button(Glyph::Note, Action::ToolsNotes, true, "")],
            ItemId::ToolShots => {
                vec![Part::button(
                    Glyph::Screenshot,
                    Action::ToolsShots,
                    true,
                    "",
                )]
            }
            ItemId::ToolSnip => vec![Part::button(Glyph::Snip, Action::SnipToStash, true, "")],
            ItemId::ToolPicker => {
                vec![Part::button(Glyph::Picker, Action::ColorPick, true, "")]
            }
            ItemId::Window => vec![Part::Text {
                text: "Firefox \u{2014} transom".into(),
                flex: Some(transom_core::item::Flex {
                    min_lp: 80,
                    max_lp: 360,
                }),
                tone: transom_core::item::Tone::Muted,
                action: None,
            }],
            ItemId::Focus => vec![Part::button(Glyph::Focus, Action::FocusPanel, true, "")],
            ItemId::Weather => vec![Part::File {
                glyph: Glyph::Weather,
                muted: false,
                label: "12°".into(),
                name: String::new(),
                action: None,
            }],
            ItemId::Desktops => vec![Part::File {
                glyph: Glyph::Desktops,
                muted: false,
                label: "1 / 2".into(),
                name: String::new(),
                action: None,
            }],
            ItemId::Separator(_) => vec![Part::Divider],
            ItemId::Clock | ItemId::Notice => vec![Part::text(clock.clone())],
            ItemId::Update => vec![Part::button(Glyph::Download, Action::UpdatePanel, true, "")],
        };
        StatusItem::new(id, Side::Left, parts)
    };
    let mut out = Vec::new();
    for s in cfg.layout.items.iter().filter(|s| s.visible) {
        let id = match s.id.as_str() {
            x if transom_config::separator_no(x).is_some() => {
                ItemId::Separator(transom_config::separator_no(x).unwrap_or(1))
            }
            "tool_shelf" => ItemId::ToolShelf,
            "tool_clipboard" => ItemId::ToolClipboard,
            "tool_notes" => ItemId::ToolNotes,
            "tool_shots" => ItemId::ToolShots,
            "tool_snip" => ItemId::ToolSnip,
            "tool_picker" => ItemId::ToolPicker,
            "window" => ItemId::Window,
            "desktops" => ItemId::Desktops,
            "focus" => ItemId::Focus,
            "weather" => ItemId::Weather,
            "search" => ItemId::Search,
            "media" => ItemId::Media,
            "ai" => ItemId::Ai,
            "audio" => ItemId::Audio,
            "tools" => ItemId::Tools,
            "peripherals" => ItemId::Peripherals,
            "shelf" => ItemId::Shelf,
            "keyboard" => ItemId::Keyboard,
            "timer" => ItemId::Timer,
            "brightness" => ItemId::Brightness,
            "sysmon" => ItemId::SysMon,
            "privacy" => ItemId::Privacy,
            "battery" => ItemId::Battery,
            "clock" => ItemId::Clock,
            _ => continue,
        };
        let mut it = make(id);
        it.side = match s.group {
            Group::Left => Side::Left,
            Group::Center => Side::Center,
            Group::Right => Side::Right,
        };
        out.push(it);
    }
    out
}

fn style(size: f32, bold: bool, color: Color, align: HAlign) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    }
}

fn icon_style(color: Color) -> TextStyle {
    TextStyle {
        font: Font::Icons,
        size_lp: 12.0,
        bold: false,
        color,
        align: HAlign::Center,
    }
}

/// Y suradnica zaciatku obsahu stranky (bez posunu) pod oznamovacim pasom `top`.
fn content_origin(scale: f32, top: i32) -> i32 {
    top + (22.0 * scale).round() as i32
}

/// Jazyk aplikacie podla konfiguracie (pri "system" podla Windows; nepodporovany = anglictina).
fn apply_lang(cfg: &Config, windows_lang: &str) -> bool {
    let before = transom_i18n::lang();
    let l = transom_i18n::resolve(&cfg.language, windows_lang);
    transom_i18n::set_lang(l);
    l != before
}

impl Ui {
    /// Smie polozka `id` byt v skupine `g`? So zapnutymi odlozenymi subormi patri stred len im (a oni len do stredu).
    pub(crate) fn center_ok(&self, id: &str, g: Group) -> bool {
        !self.cfg.center_locked() || (g == Group::Center) == (id == "shelf")
    }

    fn save(&mut self) {
        // pravidlo stredu plati pre kazdu zmenu (aj zapnutie / vypnutie odlozenych suborov)
        self.cfg.enforce_center();
        if self.read_only {
            self.status = t("status.readonly");
            return;
        }
        match transom_config::save_merged(&transom_config::default_path(), &self.base, &self.cfg) {
            Ok(merged) => {
                // prevezmi aj zmeny inych procesov (napr. rezim kruhu z listy), ktore sa zlucili do suboru
                self.cfg = merged.clone();
                self.base = merged;
                notify_bar();
                self.status = t("status.saved");
            }
            Err(e) => self.status = tf("status.save_failed", &[&e.to_string()]),
        }
    }

    /// Znova zisti pripojene monitory (otvorenie stranky Displays, zmena zobrazenia).
    fn refresh_monitors(&mut self) {
        self.monitors = transom_platform::monitor::enumerate();
    }

    /// Vynuluje formular rucneho datumu (aj text pola).
    fn reset_date_form(&mut self) {
        self.form = DateForm::default();
        if self.edits[4] != 0 {
            set_edit_text(self.edits[4], "");
        }
    }

    fn ai_visible(&self) -> bool {
        self.cfg
            .layout
            .items
            .iter()
            .any(|s| s.id == "ai" && s.visible)
    }

    fn art(&mut self, s: f32) -> &Art {
        let key = ((s * 100.0).round() as u32, self.theme.dark);
        if self.art.as_ref().map(|a| a.key) != Some(key) {
            let px = |v: f32| (v * s).round() as u32;
            let pal = if self.theme.dark {
                Palette::ON_DARK
            } else {
                Palette::ON_LIGHT
            };
            let icon_px = px(18.0);
            let th = self.theme;
            self.art = Some(Art {
                key,
                logo_px: px(30.0),
                logo: brand::render_app_icon(px(30.0)),
                word: brand::render_wordmark(px(15.0), &pal),
                icon_px,
                icons: navicons::ALL
                    .iter()
                    .map(|i| navicons::render(*i, icon_px, th.muted))
                    .collect(),
                icons_sel: navicons::ALL
                    .iter()
                    .map(|i| navicons::render(*i, icon_px, th.accent_text))
                    .collect(),
            });
        }
        self.art.as_ref().unwrap()
    }

    /// Vlastna hlavicka okna: znacka vlavo, minimalizovat / maximalizovat (obnovit) / zavriet vpravo. Bez deliacej
    /// ciary a bez textu nazvu - hlavicka splyva s bocnym panelom a obsahom (nazov okna ostava pre panel uloh).
    fn paint_caption(&mut self, p: &mut dyn Painter) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        let cap = px(chrome::CAPTION_LP);
        let g = chrome::layout(sz.w, (s * 96.0).round() as u32);
        let ch = self.chrome;
        p.fill_rect(g.caption, th.background);
        // znacka + wordmark
        {
            let a = self.art(s);
            let (lp, logo) = (a.logo_px as i32, a.logo.clone());
            let (word, ww, wh) = (a.word.0.clone(), a.word.1 as i32, a.word.2 as i32);
            let ly = (cap - lp) / 2;
            p.blend_rgba(Rect::new(px(14), ly, lp, lp), &logo);
            p.blend_rgba(
                Rect::new(px(14) + lp + px(9), ly + (lp - wh) / 2, ww, wh),
                &word,
            );
        }
        for b in chrome::BTNS {
            let r = g.button(b);
            let hot = ch.hover == Some(b);
            let down = hot && ch.pressed == Some(b);
            let close = b == chrome::Btn::Close;
            let (bg, fg) = match (close, hot, down) {
                (true, _, true) => (
                    Some(Color::rgb(0x9E, 0x24, 0x1A)),
                    Color::rgb(255, 255, 255),
                ),
                (true, true, _) => (
                    Some(Color::rgb(0xC4, 0x2B, 0x1C)),
                    Color::rgb(255, 255, 255),
                ),
                (_, _, true) => (Some(th.pressed), th.foreground),
                (_, true, _) => (Some(th.hover), th.foreground),
                // neaktivne okno: tlmene, ale citatelne znaky (stale su to klikatelne tlacidla)
                _ => (None, if ch.active { th.foreground } else { th.muted }),
            };
            if let Some(bg) = bg {
                p.fill_rect(r, bg);
            }
            p.draw_text(
                chrome::glyph(b, ch.maximized),
                r,
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 10.0,
                    bold: false,
                    color: fg,
                    align: HAlign::Center,
                },
            );
            // fokus klavesnice: dvojfarebny vnutorny ramik (viditelny aj na cervenej vyplni zatvarania)
            if ch.focus == Some(b) {
                design::focus_ring_inset(
                    p,
                    &th,
                    Rect::new(r.x + px(3), r.y + px(3), r.w - px(6), r.h - px(6)),
                    design::radius::CONTROL,
                    if close && bg.is_some() {
                        Color::rgb(255, 255, 255)
                    } else {
                        bg.unwrap_or(th.background)
                    },
                );
            }
        }
    }

    /// Prekreslenie hlavicky pre pristupnost: nazvy a obdlzniky tlacidiel (po kazdom kresleni).
    fn titlebar_entries(
        &self,
        client_w: i32,
        dpi: u32,
    ) -> Vec<transom_platform::menu_access::MenuEntry> {
        let g = chrome::layout(client_w, dpi);
        chrome::BTNS
            .iter()
            .map(|&b| {
                let r = g.button(b);
                transom_platform::menu_access::MenuEntry {
                    name: self.chrome_name(b),
                    separator: false,
                    enabled: true,
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                }
            })
            .collect()
    }

    /// Lokalizovany nazov tlacidla hlavicky (pristupnost).
    fn chrome_name(&self, b: chrome::Btn) -> String {
        t(match (b, self.chrome.maximized) {
            (chrome::Btn::Min, _) => "chrome.min",
            (chrome::Btn::Max, false) => "chrome.max",
            (chrome::Btn::Max, true) => "chrome.restore",
            (chrome::Btn::Close, _) => "chrome.close",
        })
    }

    fn paint_nav(&mut self, p: &mut dyn Painter) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        let nav_w = px(NAV_W_LP);
        let cap = px(chrome::CAPTION_LP);
        // navigacia na ploche okna (zdvihnute su len karty obsahu), oddelena vlasom
        p.fill_rect(Rect::new(0, cap, nav_w, (sz.h - cap).max(0)), th.background);
        p.fill_rect(Rect::new(nav_w - 1, cap, 1, (sz.h - cap).max(0)), th.edge);
        // strom: kategoria (ikona, nazov, sipka rozbalenia) a pod nou jej podkategorie, ak je rozbalena
        let cur = cat_of(self.page);
        if self.nav_seen != Some(self.page) {
            // nova stranka (aj z odkazu zvonku): jej kategoria sa rozbali
            self.nav_seen = Some(self.page);
            self.expanded[cur] = true;
        }
        let mut y = cap + px(12);
        for (i, (key, icon, pages)) in CATS.iter().enumerate() {
            let open = self.expanded[i];
            let r = Rect::new(px(10), y, nav_w - px(20), px(36));
            let hit = Hit::Cat(i as u8);
            if self.hover.as_ref() == Some(&hit) {
                p.fill_round_rect(
                    r,
                    px(design::radius::CONTROL + 2),
                    transom_render::scene::mix(th.hover, th.background, 0.35),
                );
            }
            let has_sel = cur == i;
            let a = self.art(s);
            let ip = a.icon_px as i32;
            let img = if has_sel {
                a.icons_sel[*icon].clone()
            } else {
                a.icons[*icon].clone()
            };
            p.blend_rgba(Rect::new(r.x + px(12), r.y + (r.h - ip) / 2, ip, ip), &img);
            let st = Ty::BodyStrong.style(th.foreground, HAlign::Left);
            let tw = r.w - px(40) - px(22);
            let lines = Self::wrap_lines(p, &t(key), &st, tw);
            let lh = p.measure_text("Ag", &st).h.max(px(17));
            let rh = r.h.max(lh * lines.len() as i32 + px(8));
            let r = Rect::new(r.x, r.y, r.w, rh);
            let top = r.y + (r.h - lh * lines.len() as i32) / 2;
            for (k, l) in lines.iter().enumerate() {
                p.draw_text(l, Rect::new(r.x + px(38), top + k as i32 * lh, tw, lh), &st);
            }
            // sipka: dole = rozbalene, vpravo = zbalene (stav nesie aj pristupny nazov)
            p.draw_text(
                if open { "\u{E70D}" } else { "\u{E76C}" },
                Rect::new(r.x + r.w - px(24), r.y, px(20), r.h),
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 9.0,
                    bold: false,
                    color: th.muted,
                    align: HAlign::Center,
                },
            );
            self.tag(&hit, t(key), Some(open));
            self.hits.push((r, hit));
            y += r.h + px(2);
            if !open {
                y += px(4);
                continue;
            }
            for &(pg, pkey) in pages.iter() {
                let sel = self.page == pg;
                let hit = Hit::Nav(pg);
                let st = if sel {
                    Ty::BodyStrong.style(th.foreground, HAlign::Left)
                } else {
                    Ty::Body.style(th.foreground, HAlign::Left)
                };
                let tw = nav_w - px(20) - px(46);
                let lines = Self::wrap_lines(p, &t(pkey), &st, tw);
                let lh = p.measure_text("Ag", &st).h.max(px(17));
                let rh = px(30).max(lh * lines.len() as i32 + px(8));
                let r = Rect::new(px(10), y, nav_w - px(20), rh);
                // vybrana: zdvihnuta plocha + akcentova znacka + polotucne (nie len farba); pod mysou: polovicny odtien
                if sel {
                    p.fill_round_rect(r, px(design::radius::CONTROL + 2), th.hover);
                    p.fill_round_rect(
                        Rect::new(r.x + px(26), r.y + px(8), px(3), r.h - px(16)),
                        px(2),
                        th.accent,
                    );
                } else if self.hover.as_ref() == Some(&hit) {
                    p.fill_round_rect(
                        r,
                        px(design::radius::CONTROL + 2),
                        transom_render::scene::mix(th.hover, th.background, 0.35),
                    );
                }
                let top = r.y + (r.h - lh * lines.len() as i32) / 2;
                for (k, l) in lines.iter().enumerate() {
                    p.draw_text(l, Rect::new(r.x + px(38), top + k as i32 * lh, tw, lh), &st);
                }
                self.tag(&hit, t(pkey), Some(sel));
                self.hits.push((r, hit));
                y += rh;
            }
            y += px(6);
        }
        // strom sa nezmesti do okna (dlhe preklady, nizke okno): ostatne kategorie sa zbalia a strom sa prekresli
        let keep = |i: usize| i == cur || Some(i) == self.nav_last;
        if y > sz.h - px(8) && (0..CATS.len()).any(|i| !keep(i) && self.expanded[i]) {
            for (i, e) in self.expanded.iter_mut().enumerate() {
                *e = *e && keep(i);
            }
            self.repaint = true;
        }
    }

    fn paint(&mut self, p: &mut dyn Painter) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        self.hits.clear();
        self.acc.clear();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.background);
        self.paint_nav(p);
        self.paint_caption(p);
        let nav_w = px(NAV_W_LP);
        let cap = px(chrome::CAPTION_LP);
        let x0 = nav_w + px(28);
        let w = (sz.w - x0 - px(36)).max(px(200));
        // oznam (novsia / poskodena konfiguracia) - pevny pas nad obsahom
        let mut top = cap;
        if let Some(n) = &self.note {
            top = cap + px(38);
            p.draw_text(
                n,
                Rect::new(x0, cap, w, px(38)),
                &style(12.0, true, th.low, HAlign::Left),
            );
            p.fill_rect(Rect::new(nav_w, top - 1, sz.w - nav_w, 1), th.edge);
        }
        let status_h = px(36);
        let vp = Rect::new(
            nav_w,
            top,
            sz.w - nav_w,
            (sz.h - top - status_h).max(px(100)),
        );
        self.viewport = vp;
        let origin = content_origin(s, top) - self.scroll;
        // obsah v posuvnom okne; najnizsi nakresleny bod urcuje rozsah posuvania
        let hits_start = self.hits.len();
        p.push_clip(vp);
        self.cards.clear();
        self.card_top = None;
        // obsah sa najprv zaznamena (meria sa skutocnym painterom), potom sa pod neho nakreslia karty sekcii a obsah sa
        // prehra: rozmer karty je znamy az po rozlozeni jej obsahu
        let mut rec = transom_render::record::Recorder::new(p);
        let bottom = {
            let mut tp = transom_render::audit::Track::new(&mut rec);
            match self.page {
                Page::Layout => self.paint_layout(&mut tp, x0, origin, w),
                Page::Displays => self.paint_displays(&mut tp, x0, origin, w),
                Page::Media => self.paint_media(&mut tp, x0, origin, w),
                Page::Appearance => {
                    self.track = appearance_page::layout(s, x0, origin, w).track;
                    self.paint_appearance(&mut tp, x0, origin, w)
                }
                Page::Clock => self.paint_clock(&mut tp, x0, origin, w),
                Page::Ai => self.paint_ai(&mut tp, x0, origin, w),
                Page::Sound => self.paint_sound(&mut tp, x0, origin, w),
                Page::Audio => self.paint_audio_page(&mut tp, x0, origin, w),
                Page::Peripherals => self.paint_periph_page(&mut tp, x0, origin, w),
                Page::Tools | Page::Clipboard | Page::Notes | Page::Shots | Page::Launcher => {
                    self.paint_tools_page(&mut tp, x0, origin, w)
                }
                Page::Extras => self.paint_extras_page(&mut tp, x0, origin, w),
                Page::Notch => self.paint_notch_page(&mut tp, x0, origin, w),
                Page::ToolGroup => self.paint_toolgroup_page(&mut tp, x0, origin, w),
                Page::Language => self.paint_language(&mut tp, x0, origin, w),
                Page::Transfer => self.paint_transfer(&mut tp, x0, origin, w),
                Page::Updates => self.paint_updates(&mut tp, x0, origin, w),
                Page::Welcome => self.paint_welcome(&mut tp, x0, origin, w),
            }
            tp.max_bottom
        };
        let ops = rec.into_ops();
        // neuzavreta karta (stranka skoncila vo vnutri sekcie) sa uzavrie pod poslednym obsahom
        if self.card_top.is_some() && bottom != i32::MIN {
            self.card_end(s, x0, w, bottom);
        }
        for c in self.cards.clone() {
            design::card(p, &th, c);
        }
        transom_render::record::replay(p, ops);
        self.paint_field_images(p, x0, origin, w);
        let bottom = self.cards.iter().map(|c| c.y + c.h).fold(bottom, i32::max);
        p.pop_clip();
        // ovladace aj mimo viditelneho okna (pre kruh fokusu klavesnice), este pred orezom
        self.all_hits = self.hits.clone();
        // po klavesnicovom presune: polohy su uz aktualne, zobrazit celu polozku (a dalsie kreslenie po posune)
        if let Some(h) = self.reveal.take() {
            if self.ensure_visible(&h) {
                self.repaint = true;
            }
        }
        // klik v oblasti mimo okna obsahu (orezanej) sa nepocita
        let mut k = hits_start;
        while k < self.hits.len() {
            let r = self.hits[k].0;
            let (x1, y1) = (r.x.max(vp.x), r.y.max(vp.y));
            let (x2, y2) = ((r.x + r.w).min(vp.x + vp.w), (r.y + r.h).min(vp.y + vp.h));
            if x2 > x1 && y2 > y1 {
                self.hits[k].0 = Rect::new(x1, y1, x2 - x1, y2 - y1);
                k += 1;
            } else {
                self.hits.remove(k);
            }
        }
        // fokus klavesnice na ovladaci stranky: dvojfarebny ramik okolo (viditelny aj na akcentovej / cervenej vyplni
        // a odlisny od vyberu, ktory ma vlastnu vypln a okraj)
        if let Some(f) = &self.kfocus {
            if let Some((r, _)) = self.hits.iter().find(|(_, h)| h == f).cloned() {
                let under = self.under(r);
                design::focus_ring(p, &th, r, design::radius::CONTROL, under);
            }
        }
        // rozsah posuvania z celkovej vysky obsahu (vratane okrajov hore a dole)
        let content_h = if bottom == i32::MIN {
            0
        } else {
            bottom + self.scroll - top + px(24)
        };
        let max_scroll = (content_h - vp.h).max(0);
        if max_scroll != self.max_scroll || self.scroll > max_scroll {
            self.max_scroll = max_scroll;
            if self.scroll > max_scroll {
                self.scroll = max_scroll;
            }
            self.repaint = true;
        }
        // jemny posuvnik (indikator polohy; ovlada sa kolieskom a klavesnicou)
        if self.max_scroll > 0 {
            let th_h = (vp.h * vp.h / (vp.h + self.max_scroll)).max(px(28));
            let ty = vp.y + (vp.h - th_h) * self.scroll / self.max_scroll;
            p.fill_round_rect(Rect::new(sz.w - px(10), ty, px(4), th_h), px(2), th.muted);
        }
        // stavovy riadok (pevny)
        p.fill_rect(
            Rect::new(nav_w, sz.h - status_h, sz.w - nav_w, status_h),
            th.background,
        );
        p.fill_rect(Rect::new(nav_w, sz.h - status_h, sz.w - nav_w, 1), th.edge);
        let status_rect = Rect::new(x0, sz.h - status_h, w, status_h);
        let one = Ty::Secondary.style(th.muted, HAlign::Left);
        if p.measure_text(&self.status, &one).w <= w {
            p.draw_text(&self.status, status_rect, &one);
        } else {
            // dlhsie hlasenie (napr. chyba s textom systemu) sa zalomi na dva mensie riadky; nic sa neorezava
            let small = Ty::Caption.style(th.muted, HAlign::Left);
            let lines = Self::wrap_lines(p, &self.status, &small, w);
            let lh = status_h / 2;
            for (i, l) in lines.iter().take(2).enumerate() {
                p.draw_text(
                    l,
                    Rect::new(x0, status_rect.y + i as i32 * lh, w, lh),
                    &small,
                );
            }
        }
    }

    /// Posunie obsah tak, aby bol ovladac `h` cely vo viditelnom okne (navigacia je pevna). Vrati, ci sa posunulo.
    fn ensure_visible(&mut self, h: &Hit) -> bool {
        let vp = self.viewport;
        if matches!(h, Hit::Nav(_) | Hit::Cat(_)) || vp.h <= 0 {
            return false;
        }
        let Some((r, _)) = self.all_hits.iter().find(|(_, x)| x == h).cloned() else {
            return false;
        };
        let pad = 12;
        let delta = if r.y < vp.y {
            r.y - vp.y - pad
        } else if r.y + r.h > vp.y + vp.h {
            r.y + r.h - (vp.y + vp.h) + pad
        } else {
            0
        };
        delta != 0 && self.scroll_by(delta)
    }

    /// Posun obsahu o `delta` px (zaporne = nahor), orezany na povoleny rozsah.
    fn scroll_by(&mut self, delta: i32) -> bool {
        let v = (self.scroll + delta).clamp(0, self.max_scroll);
        let changed = v != self.scroll;
        self.scroll = v;
        changed
    }

    /// Tlacidlo stranky. Vzhlad (druh) urcuje vyznam ovladaca (`button_kind`); `active` = vybrana volba, alebo druhy krok
    /// nebezpecnej akcie. Pristupny nazov je vzdy `text` (ikona sa do neho nepridava).
    fn button(&mut self, p: &mut dyn Painter, r: Rect, text: &str, hit: Hit, active: bool) {
        let th = self.theme;
        let (kind, icon) = if self.in_choice {
            (Kind::Choice, None)
        } else {
            button_kind(&hit)
        };
        let st = design::State {
            hover: self.hover.as_ref() == Some(&hit),
            selected: active && kind == Kind::Choice,
            armed: active && kind == Kind::Destructive,
            ..Default::default()
        };
        let mut b = design::Button::new(text, kind).state(st);
        b.icon = icon;
        b.left = self.btn_left;
        let under = self.under(r);
        design::button(p, &th, r, &b, under);
        let name = page_a11y::acc_label(&hit, text, self.acc_group.as_deref());
        self.tag(&hit, name, Some(active));
        self.hits.push((r, hit));
    }

    /// Zaskrtavacie policko (zaoblene; zapnute = znacka aj farba, nie len farba).
    fn checkbox(&self, p: &mut dyn Painter, r: Rect, on: bool) {
        design::checkbox(p, &self.theme, r, on, false, false);
    }

    /// Plocha pod ovladacom: karta (ak je prave otvorena alebo ho obsahuje), inak pozadie okna.
    fn under(&self, r: Rect) -> Color {
        let inside =
            |c: &Rect| r.x >= c.x && r.y >= c.y && r.x + r.w <= c.x + c.w && r.y + r.h <= c.y + c.h;
        if self.card_top.is_some() || self.cards.iter().any(inside) {
            self.theme.surface
        } else {
            self.theme.background
        }
    }

    /// Zaciatok karty sekcie na `y`; vrati y, kde zacina jej obsah.
    fn card_begin(&mut self, s: f32, y: i32) -> i32 {
        self.card_top = Some(y);
        y + (CARD_PAD_Y_LP as f32 * s).round() as i32
    }

    /// Koniec karty: `y` je spodok jej obsahu. Vrati y pre nasledujuci obsah (pod kartou a medzerou).
    fn card_end(&mut self, s: f32, x0: i32, w: i32, y: i32) -> i32 {
        let px = |v: i32| (v as f32 * s).round() as i32;
        let Some(top) = self.card_top.take() else {
            return y;
        };
        let bottom = y + px(CARD_PAD_Y_LP);
        self.cards.push(Rect::new(
            x0 - px(CARD_PAD_X_LP),
            top,
            w + 2 * px(CARD_PAD_X_LP),
            bottom - top,
        ));
        bottom + px(CARD_GAP_LP)
    }

    /// Nadpis sekcie (v karte); vrati y pod nim.
    fn section(&self, p: &mut dyn Painter, x0: i32, y: i32, w: i32, text: &str) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let st = Ty::Section.style(self.theme.foreground, HAlign::Left);
        let lh = px(Ty::Section.line_lp());
        let mut y = y;
        for l in Self::wrap_lines(p, text, &st, w) {
            p.draw_text(&l, Rect::new(x0, y, w, lh), &st);
            y += lh;
        }
        y + px(6)
    }

    /// Podnadpis (zoznam, skupina volieb v sekcii); vrati y pod nim.
    fn subsection(&self, p: &mut dyn Painter, x0: i32, y: i32, w: i32, text: &str) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let st = Ty::Subsection.style(self.theme.foreground, HAlign::Left);
        let lh = px(Ty::Subsection.line_lp());
        let mut y = y;
        for l in Self::wrap_lines(p, text, &st, w) {
            p.draw_text(&l, Rect::new(x0, y, w, lh), &st);
            y += lh;
        }
        y + px(4)
    }

    /// Riadok volieb: nazov vlavo, tlacidla vpravo. Ak sa niektory text (dlhsi preklad) nezmesti, tlacidla sa
    /// zalomia po dvoch. Vrati pouzitu vysku vratane medzery.
    fn choice_row(
        &mut self,
        p: &mut dyn Painter,
        area: Rect,
        lw: i32,
        name: &str,
        opts: Vec<Choice>,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let th = self.theme;
        let (x0, y, w) = (area.x, area.y, area.w);
        let ch = px(design::CONTROL_H_LP);
        let step = ch + px(6);
        // texty sa meraju vybranym (polotucnym) stylom tlacidla: vyber nikdy nezmeni rozlozenie
        let bst = Ty::Control.style(th.foreground, HAlign::Center);
        let bst = TextStyle { bold: true, ..bst };
        let widest = opts
            .iter()
            .map(|(text, _, _)| p.measure_text(text, &bst).w)
            .max()
            .unwrap_or(0);
        // popisok: Body, zalomeny v stlpci popisku; ak by mal viac ako dva riadky alebo sa vedla neho nezmesti ani
        // jedno tlacidlo, ide nad tlacidla (cela sirka)
        let lst = Ty::Body.style(th.foreground, HAlign::Left);
        let lh = px(Ty::Body.line_lp());
        let side_lines = Self::wrap_lines(p, name, &lst, lw - px(12));
        let stacked = widest + px(16) > w - lw || side_lines.len() > 2;
        let lines = if stacked {
            Self::wrap_lines(p, name, &lst, w)
        } else {
            side_lines
        };
        let (lw, by) = if stacked {
            (0, lines.len() as i32 * lh + px(4))
        } else {
            (lw, 0)
        };
        // popisok zvislo na stred prveho riadku tlacidiel (jednoriadkovy), viacriadkovy od vrchu
        let ly = if stacked || lines.len() > 1 {
            y
        } else {
            y + (ch - lh) / 2
        };
        for (k, l) in lines.iter().enumerate() {
            p.draw_text(
                l,
                Rect::new(
                    x0,
                    ly + k as i32 * lh,
                    if stacked { w } else { lw - px(12) },
                    lh,
                ),
                &lst,
            );
        }
        let n = opts.len().max(1) as i32;
        let width = |k: i32| (w - lw - px(6) * (k - 1)) / k;
        let fits = |p: &mut dyn Painter, k: i32| {
            opts.iter()
                .all(|(text, _, _)| p.measure_text(text, &bst).w <= width(k) - px(16))
        };
        // dlhsie preklady: tlacidla sa zalamuju (po n, n-1, ..., 1 v riadku), kym sa texty nezmestia; potom sa riadky
        // vyvazia (5 volieb = 3 + 2, nie 4 + 1)
        let mut per_row = n;
        while per_row > 1 && !fits(p, per_row) {
            per_row -= 1;
        }
        let rows = (n + per_row - 1) / per_row;
        let per_row = (n + rows - 1) / rows;
        let ow = width(per_row);
        self.acc_group = Some(name.to_string());
        self.in_choice = true;
        for (i, (text, hit, active)) in opts.into_iter().enumerate() {
            let (col, row) = (i as i32 % per_row, i as i32 / per_row);
            self.button(
                p,
                Rect::new(x0 + lw + col * (ow + px(6)), y + by + row * step, ow, ch),
                &text,
                hit,
                active,
            );
        }
        self.acc_group = None;
        self.in_choice = false;
        let label_h = if stacked { 0 } else { lines.len() as i32 * lh };
        (by + rows * step).max(label_h + px(6)) + px(4)
    }

    /// Zalomi `text` na riadky sirky najviac `w` px (po slovach; slovo dlhsie nez riadok a text bez medzier - japoncina,
    /// cinstina - sa lomi po znakoch). Nic sa neorezava ani neskracuje.
    fn wrap_lines(p: &mut dyn Painter, text: &str, st: &TextStyle, w: i32) -> Vec<String> {
        transom_render::painter::wrap_text(p, text, st, w)
    }

    /// Zalomeny odsek textu; vrati y pod poslednym riadkom (bez medzery).
    #[allow(clippy::too_many_arguments)]
    fn paragraph(
        &self,
        p: &mut dyn Painter,
        x0: i32,
        y: i32,
        w: i32,
        text: &str,
        size: f32,
        color: Color,
    ) -> i32 {
        self.paragraph_b(p, x0, y, w, text, size, false, color)
    }

    #[allow(clippy::too_many_arguments)]
    fn paragraph_b(
        &self,
        p: &mut dyn Painter,
        x0: i32,
        mut y: i32,
        w: i32,
        text: &str,
        size: f32,
        bold: bool,
        color: Color,
    ) -> i32 {
        let s = p.scale();
        // vyska riadku zo stupnice (11/16, 12/18, 13/20 lp), inak ~1,5 x velkost
        let line_lp = match size as i32 {
            11 => Ty::Caption.line_lp() as f32,
            12 => Ty::Secondary.line_lp() as f32,
            13 => Ty::Body.line_lp() as f32,
            _ => (size * 1.5).round(),
        };
        let line_h = (line_lp * s).round() as i32;
        let st = style(size, bold, color, HAlign::Left);
        for line in Self::wrap_lines(p, text, &st, w) {
            p.draw_text(&line, Rect::new(x0, y, w, line_h), &st);
            y += line_h;
        }
        y
    }

    /// Vysvetlivky (Secondary, tlmene); medzi odsekmi medzera `PARAGRAPH_GAP_LP`.
    fn notes(&self, p: &mut dyn Painter, x0: i32, mut y: i32, w: i32, keys: &[&str]) -> i32 {
        let gap = (design::PARAGRAPH_GAP_LP as f32 * p.scale()).round() as i32;
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                y += gap;
            }
            y = self.paragraph(p, x0, y, w, &t(k), 12.0, self.theme.muted);
        }
        y
    }

    /// Nadpis stranky (PageTitle, zalomeny - nic sa neskracuje); vrati y pod nim.
    fn title(&self, p: &mut dyn Painter, x0: i32, y: i32, w: i32, key: &str) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let st = Ty::PageTitle.style(self.theme.foreground, HAlign::Left);
        let lh = px(Ty::PageTitle.line_lp() + 4);
        let mut y = y;
        for l in Self::wrap_lines(p, &t(key), &st, w) {
            p.draw_text(&l, Rect::new(x0, y, w, lh), &st);
            y += lh;
        }
        y + px(8)
    }

    /// Informacny pruh (varovanie): tlmeny cerveny nadych, znak varovania a text (zalomeny). Vrati y pod nim.
    fn infobar(&self, p: &mut dyn Painter, x0: i32, y: i32, w: i32, text: &str) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let st = Ty::Secondary.style(th.foreground, HAlign::Left);
        let lh = px(Ty::Secondary.line_lp());
        let tx = x0 + px(36);
        let lines = Self::wrap_lines(p, text, &st, w - px(48));
        let h = lines.len() as i32 * lh + px(20);
        let under = if self.card_top.is_some() {
            th.surface
        } else {
            th.background
        };
        design::fill_with_stroke(
            p,
            Rect::new(x0, y, w, h),
            px(design::radius::CONTROL),
            Some(transom_render::scene::mix(th.danger, under, 0.10)),
            Some(transom_render::scene::mix(th.danger, under, 0.45)),
        );
        p.draw_text(
            &design::WARNING_GLYPH.to_string(),
            Rect::new(x0 + px(10), y + px(10), px(18), lh),
            &TextStyle {
                font: Font::Icons,
                size_lp: 14.0,
                bold: false,
                color: th.danger,
                align: HAlign::Center,
            },
        );
        for (k, l) in lines.iter().enumerate() {
            p.draw_text(
                l,
                Rect::new(tx, y + px(10) + k as i32 * lh, w - px(48), lh),
                &st,
            );
        }
        y + h + px(8)
    }

    /// Rad tlacidiel sirokych podla textu (aspon 120 lp), zalamany na sirku `w` - nic nepretecie okno. Vrati y pod nim.
    fn flow_buttons(
        &mut self,
        p: &mut dyn Painter,
        x0: i32,
        y: i32,
        w: i32,
        items: Vec<(String, Hit, bool)>,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let ch = px(design::CONTROL_H_LP);
        let (mut x, mut y) = (x0, y);
        for (text, hit, active) in items {
            let (kind, icon) = button_kind(&hit);
            let mut b = design::Button::new(&text, kind).state(design::State {
                selected: active && kind == Kind::Choice,
                armed: active && kind == Kind::Destructive,
                ..Default::default()
            });
            b.icon =
                icon.or((kind == Kind::Destructive && active).then_some(design::WARNING_GLYPH));
            let bw = design::button_width(p, &b).max(px(120)).min(w);
            if x > x0 && x + bw > x0 + w {
                x = x0;
                y += ch + px(8);
            }
            self.button(p, Rect::new(x, y, bw, ch), &text, hit, active);
            x += bw + px(8);
        }
        y + ch + px(10)
    }

    fn paint_layout(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "layout.title");
        y = self.paragraph(p, x0, y, w, &t("layout.help"), 12.0, th.muted) + px(4);
        y = self.paragraph(p, x0, y, w, &t("layout.kbd_help"), 12.0, th.muted) + px(12);
        // nahlad: skutocny renderer, zmensena sirka (v karte)
        y = self.card_begin(s, y);
        let bar_h = px(28);
        p.fill_rect(Rect::new(x0 - 1, y - 1, w + 2, bar_h + 2), th.hover);
        let items = sample_items(&self.cfg);
        {
            let mut sub = Offset {
                inner: p,
                dx: x0,
                dy: y,
                w,
                h: bar_h,
            };
            render_bar(&mut sub, &th, &items, Interaction::default());
        }
        y += bar_h + px(8);
        y = self.paragraph(p, x0, y, w, &t("layout.preview_note"), 12.0, th.muted);
        y = self.card_end(s, x0, w, y) + px(4);
        // ostrov v strede: zapnuty = stred patri jemu a stlpec Stred sa neukazuje (len vlavo a vpravo)
        {
            let on = self.cfg.center_locked();
            let row = Rect::new(x0, y, w, px(28));
            let hit = Hit::NotchOpt("island");
            if self.hover.as_ref() == Some(&hit) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            self.checkbox(
                p,
                Rect::new(row.x + px(4), row.y + px(4), px(20), px(20)),
                on,
            );
            p.draw_text(
                &t("notch.island_on"),
                Rect::new(row.x + px(34), row.y, row.w - px(34), row.h),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            self.tag(&hit, t("notch.island_on"), Some(on));
            self.hits.push((row, hit));
            y = self.paragraph(
                p,
                x0 + px(34),
                y + px(28),
                w - px(34),
                &t("notch.island_on_note"),
                12.0,
                th.muted,
            ) + px(10);
        }
        // skupiny; kazda ukazuje vsetky svoje polozky s policom "Zobrazit v liste"
        let cols_def: Vec<(Group, &str)> = if self.cfg.center_locked() {
            vec![(Group::Left, "group.left"), (Group::Right, "group.right")]
        } else {
            vec![
                (Group::Left, "group.left"),
                (Group::Center, "group.center"),
                (Group::Right, "group.right"),
            ]
        };
        let gap = px(10);
        let n = cols_def.len() as i32;
        let cw = (w - (n - 1) * gap) / n;
        let chip_h = px(40);
        // stlpce su vysoke podla najplnsej skupiny (aspon 300 lp), aby sa ziadna polozka neprekryla s textom pod nimi
        let most = [Group::Left, Group::Center, Group::Right]
            .iter()
            .map(|g| {
                self.cfg
                    .layout
                    .in_group(*g)
                    .filter(|s| transom_config::is_layout_id(&s.id))
                    .count() as i32
            })
            .max()
            .unwrap_or(0);
        // pod polozkami tlacidlo "+ Oddelovac"
        let add_h = px(30);
        let ch = px(300).max(px(32) + most * (chip_h + px(6)) + px(6) + add_h + px(10));
        let show_text = t("layout.show");
        self.cols.clear();
        let target = self.drag.as_ref().and_then(|(id, mx, my)| {
            // geometria z predchadzajuceho vykreslenia (rovnaka velkost okna)
            drop_target(&self.prev_cols(), id, *mx, *my).filter(|t| self.center_ok(id, t.0))
        });
        for (i, (g, key)) in cols_def.iter().enumerate() {
            let cx = x0 + i as i32 * (cw + gap);
            let is_target = target.map(|t| t.0) == Some(*g);
            design::fill_with_stroke(
                p,
                Rect::new(cx, y, cw, ch),
                px(design::radius::CARD),
                Some(if is_target { th.hover } else { th.surface }),
                Some(if is_target { th.accent } else { th.edge }),
            );
            p.draw_text(
                &t(key),
                Rect::new(cx + px(10), y + px(4), cw - px(12), px(24)),
                &Ty::Subsection.style(th.muted, HAlign::Left),
            );
            let mut chips = Vec::new();
            for (j, id) in column_items(&self.cfg.layout, *g).into_iter().enumerate() {
                let r = Rect::new(
                    cx + px(8),
                    y + px(32) + j as i32 * (chip_h + px(6)),
                    cw - px(16),
                    chip_h,
                );
                let visible = self
                    .cfg
                    .layout
                    .items
                    .iter()
                    .any(|s| s.id == id && s.visible);
                let moving = self.moving.as_ref().is_some_and(|m| m.id == id);
                let dragging = moving || self.drag.as_ref().is_some_and(|(d, _, _)| d == &id);
                design::fill_with_stroke(
                    p,
                    r,
                    px(design::radius::CONTROL),
                    Some(if dragging {
                        th.control_hover
                    } else {
                        th.control
                    }),
                    Some(th.control_edge),
                );
                if moving {
                    // klavesnicovy presun: akcentovy okraj (nie len farba)
                    p.stroke_round_rect(r, px(4), px(2).max(2), th.accent);
                }
                // uchytenie (tahanie) a nazov
                p.draw_text(
                    "\u{E76F}",
                    Rect::new(r.x, r.y, px(22), px(22)),
                    &icon_style(th.muted),
                );
                // dlhy nazov (napr. "Baterie urządzeń peryferyjnych" v uzkom stlpci): najprv mensie pismo, potom "..."
                // (cely nazov ostava v pristupnom nazve)
                let name_w = r.w - px(26);
                let color = if dragging || !visible {
                    th.muted
                } else {
                    th.foreground
                };
                let full = label(&id);
                let mut st = style(12.0, true, color, HAlign::Left);
                if p.measure_text(&full, &st).w > name_w {
                    st = style(11.0, true, color, HAlign::Left);
                }
                let shown = fit_text(p, &full, &st, name_w);
                p.draw_text(
                    &shown,
                    Rect::new(r.x + px(22), r.y + px(2), name_w, px(20)),
                    &st,
                );
                // oddelovac: namiesto "Zobrazit v liste" tlacidlo Odstranit
                if transom_config::separator_no(&id).is_some() {
                    let row = Rect::new(r.x + px(18), r.y + px(21), r.w - px(22), px(18));
                    let hit = Hit::RemoveSep(id.clone());
                    if self.hover.as_ref() == Some(&hit) {
                        p.fill_round_rect(row, px(design::radius::CONTROL), th.control_hover);
                    }
                    p.draw_text(
                        "\u{E711}",
                        Rect::new(row.x, row.y, px(20), row.h),
                        &TextStyle {
                            size_lp: 9.0,
                            ..icon_style(th.danger)
                        },
                    );
                    let rm = t("layout.sep_remove");
                    p.draw_text(
                        &rm,
                        Rect::new(row.x + px(24), row.y, row.w - px(24), row.h),
                        &style(11.0, false, th.danger, HAlign::Left),
                    );
                    self.tag(&hit, format!("{}: {}", label(&id), rm), None);
                    self.tag(&Hit::Chip(id.clone()), label(&id), None);
                    self.hits.push((row, hit));
                    self.hits.push((r, Hit::Chip(id.clone())));
                    chips.push((id, r.y + r.h / 2));
                    continue;
                }
                // "Zobrazit v liste": policko + text (cely riadok je cielom kliknutia)
                let row = Rect::new(r.x + px(18), r.y + px(21), r.w - px(22), px(18));
                if self.hover.as_ref() == Some(&Hit::Show(id.clone())) {
                    p.fill_round_rect(row, px(design::radius::CONTROL), th.control_hover);
                }
                self.checkbox(
                    p,
                    Rect::new(row.x + px(4), row.y + px(2), px(14), px(14)),
                    visible,
                );
                p.draw_text(
                    &show_text,
                    Rect::new(row.x + px(24), row.y, row.w - px(24), row.h),
                    &style(11.0, false, th.foreground, HAlign::Left),
                );
                self.tag(
                    &Hit::Show(id.clone()),
                    format!("{}: {}", label(&id), show_text),
                    Some(visible),
                );
                self.tag(&Hit::Chip(id.clone()), label(&id), None);
                self.hits.push((row, Hit::Show(id.clone())));
                self.hits.push((r, Hit::Chip(id.clone())));
                chips.push((id, r.y + r.h / 2));
            }
            // ciarka vlozenia
            if let Some((tg, ti)) = target {
                if tg == *g {
                    let iy = y + px(32) + ti as i32 * (chip_h + px(6)) - px(4);
                    p.fill_rect(
                        Rect::new(cx + px(8), iy, cw - px(16), px(2).max(1)),
                        th.accent,
                    );
                }
            }
            // "+ Oddelovac" na spodku stlpca (pri plnom pocte zakazane)
            let full = self
                .cfg
                .layout
                .items
                .iter()
                .filter(|s| transom_config::separator_no(&s.id).is_some())
                .count()
                >= transom_config::MAX_SEPARATORS as usize;
            let add = t("layout.sep_add");
            let ar = Rect::new(cx + px(8), y + ch - add_h - px(8), cw - px(16), add_h);
            if *g == Group::Center && self.cfg.center_locked() {
                // stred patri odlozenym suborom: namiesto tlacidla vysvetlenie
                let note = Ty::Caption.style(th.muted, HAlign::Left);
                let lines = Self::wrap_lines(p, &t("layout.center_locked"), &note, cw - px(20));
                let lh = px(16);
                let top = y + ch - px(10) - lines.len() as i32 * lh;
                for (k, l) in lines.iter().enumerate() {
                    p.draw_text(
                        l,
                        Rect::new(cx + px(10), top + k as i32 * lh, cw - px(20), lh),
                        &note,
                    );
                }
                self.cols.push(ColGeom {
                    group: *g,
                    x: cx,
                    y,
                    w: cw,
                    h: ch,
                    chips,
                });
                continue;
            }
            let hit = Hit::AddSep(*g);
            let b = design::Button::new(&add, Kind::Quiet)
                .icon('\u{E710}')
                .state(design::State {
                    hover: self.hover.as_ref() == Some(&hit),
                    focus: self.kfocus.as_ref() == Some(&hit),
                    disabled: full,
                    ..Default::default()
                });
            design::button(p, &th, ar, &b, th.surface);
            self.tag(&hit, format!("{}: {}", t(key), add), None);
            if !full {
                self.hits.push((ar, hit));
            }
            self.cols.push(ColGeom {
                group: *g,
                x: cx,
                y,
                w: cw,
                h: ch,
                chips,
            });
        }
        y += ch + px(12);
        let reset = t("layout.reset");
        let bw = design::button_width(
            p,
            &design::Button::new(&reset, Kind::Secondary).icon('\u{E72C}'),
        );
        self.button(
            p,
            Rect::new(x0, y, bw.max(px(160)).min(w), px(design::CONTROL_H_LP)),
            &reset,
            Hit::ResetLayout,
            false,
        );
        y += px(design::CONTROL_H_LP + 12);
        // tri kluce tvoria jeden odsek (veta pokracuje), preto jeden zalomeny text
        let narrow = ["layout.narrow1", "layout.narrow2", "layout.narrow3"]
            .iter()
            .map(|k| t(k))
            .collect::<Vec<_>>()
            .join(" ");
        self.paragraph(p, x0, y, w, &narrow, 12.0, th.muted);
        // tahana polozka pod mysou
        if let Some((id, mx, my)) = &self.drag {
            let r = Rect::new(mx - px(70), my - px(16), px(140), px(32));
            p.fill_round_rect(r, px(design::radius::CONTROL), th.accent);
            p.draw_text(
                &label(id),
                r,
                &style(12.0, true, th.on_accent, HAlign::Center),
            );
        }
    }

    /// Geometria stlpcov z posledneho vykreslenia (pre ciel tahania pocas kreslenia).
    fn prev_cols(&self) -> Vec<ColGeom> {
        self.cols.clone()
    }

    fn paint_clock(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "clock.title");
        y = self.card_begin(s, y);
        // zivy nahlad (zapustena plocha v karte; pismo hodin ako na liste)
        let preview = clock_preview(&self.cfg.clock);
        p.fill_round_rect(
            Rect::new(x0, y, w, px(48)),
            px(design::radius::CONTROL),
            th.background,
        );
        p.draw_text(
            &preview,
            Rect::new(x0 + px(16), y, w - px(16), px(48)),
            &TextStyle {
                font: Font::Tabular,
                ..style(18.0, true, th.foreground, HAlign::Left)
            },
        );
        p.draw_text(
            &t("clock.preview"),
            Rect::new(x0, y, w - px(14), px(48)),
            &Ty::Caption.style(th.muted, HAlign::Right),
        );
        y += px(60);
        // predvolby po troch v riadku (dlhsie preklady sa zmestia)
        let presets = CLOCK_PRESETS;
        const PER_ROW: i32 = 3;
        let bw = (w - px(8) * (PER_ROW - 1)) / PER_ROW;
        let c = self.cfg.clock;
        for (i, (key, wd, wd_long, secs, date)) in presets.iter().enumerate() {
            let active = c.weekday == *wd
                && (!*wd || c.weekday_long == *wd_long)
                && c.seconds == *secs
                && c.date == *date
                && c.hour == HourMode::Locale
                && c.leading_zero == Tri::Locale;
            let (col, row) = (i as i32 % PER_ROW, i as i32 / PER_ROW);
            self.button(
                p,
                Rect::new(
                    x0 + col * (bw + px(8)),
                    y + row * px(design::CONTROL_H_LP + 6),
                    bw,
                    px(design::CONTROL_H_LP),
                ),
                &t(key),
                Hit::Preset(i),
                active,
            );
        }
        let rows = (presets.len() as i32 + PER_ROW - 1) / PER_ROW;
        y += rows * px(design::CONTROL_H_LP + 6) - px(6);
        y = self.card_end(s, x0, w, y);
        y = self.card_begin(s, y);
        let c = self.cfg.clock;
        let locale_first = t(&format!("day.{}", pclock::locale_first_day().min(6)));
        let rows: Vec<(String, Vec<Choice>)> = vec![
            (
                t("clock.hour"),
                vec![
                    (
                        t("opt.system"),
                        Hit::Hour(HourMode::Locale),
                        c.hour == HourMode::Locale,
                    ),
                    (
                        t("opt.12h"),
                        Hit::Hour(HourMode::H12),
                        c.hour == HourMode::H12,
                    ),
                    (
                        t("opt.24h"),
                        Hit::Hour(HourMode::H24),
                        c.hour == HourMode::H24,
                    ),
                ],
            ),
            (
                t("clock.zero"),
                vec![
                    (
                        t("opt.system"),
                        Hit::Zero(Tri::Locale),
                        c.leading_zero == Tri::Locale,
                    ),
                    (
                        t("opt.zero_yes"),
                        Hit::Zero(Tri::Yes),
                        c.leading_zero == Tri::Yes,
                    ),
                    (
                        t("opt.zero_no"),
                        Hit::Zero(Tri::No),
                        c.leading_zero == Tri::No,
                    ),
                ],
            ),
            (
                t("clock.seconds"),
                vec![
                    (t("opt.no"), Hit::Seconds(false), !c.seconds),
                    (t("opt.yes"), Hit::Seconds(true), c.seconds),
                ],
            ),
            (
                t("clock.weekday"),
                vec![
                    (t("opt.no"), Hit::Weekday(0), !c.weekday),
                    (
                        t("opt.wd_short"),
                        Hit::Weekday(1),
                        c.weekday && !c.weekday_long,
                    ),
                    (
                        t("opt.wd_long"),
                        Hit::Weekday(2),
                        c.weekday && c.weekday_long,
                    ),
                ],
            ),
            (
                t("clock.date"),
                vec![
                    (
                        t("opt.date_none"),
                        Hit::Date(DateMode::None),
                        c.date == DateMode::None,
                    ),
                    (
                        t("opt.date_short"),
                        Hit::Date(DateMode::Short),
                        c.date == DateMode::Short,
                    ),
                    (
                        t("opt.date_long"),
                        Hit::Date(DateMode::Long),
                        c.date == DateMode::Long,
                    ),
                    // formaty pouzivane vo svete, ukazane na dnesnom datume
                    (
                        date_sample(DateMode::Dmy),
                        Hit::Date(DateMode::Dmy),
                        c.date == DateMode::Dmy,
                    ),
                    (
                        date_sample(DateMode::DmySlash),
                        Hit::Date(DateMode::DmySlash),
                        c.date == DateMode::DmySlash,
                    ),
                    (
                        date_sample(DateMode::Mdy),
                        Hit::Date(DateMode::Mdy),
                        c.date == DateMode::Mdy,
                    ),
                    (
                        date_sample(DateMode::Iso),
                        Hit::Date(DateMode::Iso),
                        c.date == DateMode::Iso,
                    ),
                ],
            ),
            (
                t("clock.first"),
                vec![
                    (
                        tf("opt.system_day", &[&locale_first]),
                        Hit::First(FirstDay::Locale),
                        c.first_day == FirstDay::Locale,
                    ),
                    (
                        t("day.0"),
                        Hit::First(FirstDay::Monday),
                        c.first_day == FirstDay::Monday,
                    ),
                    (
                        t("day.6"),
                        Hit::First(FirstDay::Sunday),
                        c.first_day == FirstDay::Sunday,
                    ),
                    (
                        t("day.5"),
                        Hit::First(FirstDay::Saturday),
                        c.first_day == FirstDay::Saturday,
                    ),
                ],
            ),
        ];
        let lw = px(CHOICE_LABEL_LP);
        for (name, opts) in rows {
            y += self.choice_row(p, Rect::new(x0, y, w, 0), lw, &name, opts);
        }
        y += px(4);
        y = self.notes(p, x0, y, w, &["clock.note1", "clock.note2"]);
        self.card_end(s, x0, w, y);
    }

    /// Oddiel "Pripojit Claude Code": co sa pridava a kam, stav, akcie (pripojit / vratit / skusit), riesenie problemov.
    /// Vrati y pod oddielom.
    fn paint_cc(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        use transom_usage::claude_setup::Connection as C;
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("cc.title"));
        let path = cc::Cc::settings_path().display().to_string();
        y = self.paragraph(p, x0, y, w, &tf("cc.what", &[&path]), 12.0, th.muted);
        y = self.paragraph(p, x0, y, w, &t("cc.desktop"), 12.0, th.muted) + px(6);
        let conn = self.cc.conn();
        let (state, color) = match &conn {
            C::NotConfigured => (t("cc.state.none"), th.muted),
            C::Foreign(c) => (tf("cc.state.foreign", &[c]), th.low),
            C::ConfiguredNoSnapshot => (t("cc.state.nosnap"), th.muted),
            C::Outdated(c) => (tf("cc.state.outdated", &[c]), th.low),
            C::Connected { limits: true } => (t("cc.state.ok"), th.accent_text),
            C::Connected { limits: false } => (t("cc.state.nolimits"), th.muted),
            C::Error(_) => (t("cc.state.error"), th.low),
        };
        y = self.paragraph_b(p, x0, y, w, &state, 13.0, true, color) + px(8);
        // tlacidla: pripojit / (dvojkrokove) nahradit / vratit / skusit (sirka podla textu, zalamuju sa)
        let mut items: Vec<(String, Hit, bool)> = Vec::new();
        match &conn {
            C::NotConfigured | C::ConfiguredNoSnapshot | C::Connected { .. } => {
                if matches!(conn, C::NotConfigured) {
                    items.push((t("cc.connect"), Hit::CcConnect, true));
                }
            }
            // most zo starej kopie (premenovanie, presun): nase nastavenie sa prepise bez dalsej otazky
            C::Outdated(_) => items.push((t("cc.reconnect"), Hit::CcConnect, true)),
            C::Foreign(_) => {
                if self.cc.confirm_replace {
                    items.push((t("cc.replace_confirm"), Hit::CcReplace, true));
                    items.push((t("cc.keep"), Hit::CcKeep, false));
                } else {
                    items.push((t("cc.replace"), Hit::CcReplace, false));
                }
            }
            C::Error(_) => {}
        }
        if self.cc.can_rollback {
            items.push((t("cc.rollback"), Hit::CcDisconnect, false));
        }
        items.push((t("cc.test"), Hit::CcTest, false));
        items.push((t("aip.copy"), Hit::CopyStatusline, false));
        y = self.flow_buttons(p, x0, y, w, items);
        if matches!(conn, C::Foreign(_)) {
            y = self.paragraph(p, x0, y, w, &t("cc.chain"), 12.0, th.muted) + px(4);
        }
        if let Some(m) = self.cc.test.clone() {
            y = self.paragraph_b(p, x0, y, w, &m, 12.0, false, th.foreground) + px(4);
        }
        if matches!(conn, C::ConfiguredNoSnapshot) {
            y = self.paragraph(p, x0, y, w, &t("cc.trouble"), 12.0, th.muted) + px(4);
        }
        self.card_end(s, x0, w, y - px(6))
    }

    /// Oddiel "Archiv dennych suctov (volitelny)": co sa uklada a kam, zapnutie (opt-in), pozastavenie, retencia, stav suboru,
    /// zmazanie (dvojkrokove). Vrati y pod oddielom.
    fn paint_archive(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let a = self.cfg.usage.archive.clone();
        let mut y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("arc.title"));
        // zapnutie: riadok s polickom, nazvom a dvoma popismi (co sa uklada / co sa neuklada a kde)
        let path = transom_usage::archive::path().display().to_string();
        let dst = Ty::Secondary.style(th.muted, HAlign::Left);
        let dw = w - px(34);
        let l1 = Self::wrap_lines(p, &t("arc.d1"), &dst, dw);
        let l2 = Self::wrap_lines(p, &tf("arc.d2", &[&path]), &dst, dw);
        let lh = px(Ty::Secondary.line_lp());
        let h = px(26) + (l1.len() + l2.len()) as i32 * lh + px(6);
        let row = Rect::new(x0, y, w, h);
        if self.hover == Some(Hit::ArchiveEnable) {
            p.fill_round_rect(
                Rect::new(x0 + px(30), y, w - px(30), h),
                px(design::radius::CONTROL),
                th.hover,
            );
        }
        self.checkbox(p, Rect::new(x0, y + px(3), px(20), px(20)), a.enabled);
        p.draw_text(
            &t("arc.enable"),
            Rect::new(x0 + px(34), y, w - px(34), px(26)),
            &Ty::Body.style(th.foreground, HAlign::Left),
        );
        let mut ly = y + px(26);
        for l in l1.iter().chain(l2.iter()) {
            p.draw_text(l, Rect::new(x0 + px(34), ly, dw, lh), &dst);
            ly += lh;
        }
        self.tag(&Hit::ArchiveEnable, t("arc.enable"), Some(a.enabled));
        self.hits.push((row, Hit::ArchiveEnable));
        y += h + px(6);
        // pozastavenie (len ked je archiv zapnuty)
        let prow = Rect::new(x0, y, w, px(28));
        if a.enabled && self.hover == Some(Hit::ArchivePause) {
            p.fill_round_rect(
                Rect::new(x0 + px(30), y, w - px(30), px(28)),
                px(design::radius::CONTROL),
                th.hover,
            );
        }
        self.checkbox(p, Rect::new(x0, y + px(4), px(20), px(20)), a.paused);
        p.draw_text(
            &t("arc.pause"),
            Rect::new(x0 + px(34), y, w - px(34), px(28)),
            &Ty::Body.style(
                if a.enabled { th.foreground } else { th.muted },
                HAlign::Left,
            ),
        );
        if a.enabled {
            self.tag(&Hit::ArchivePause, t("arc.pause"), Some(a.paused));
            self.hits.push((prow, Hit::ArchivePause));
        }
        y += px(32);
        y = self.paragraph(
            p,
            x0 + px(34),
            y,
            w - px(34),
            &t("arc.pause_d"),
            12.0,
            th.muted,
        ) + px(8);
        // retencia
        let lw = px(crate::CHOICE_LABEL_LP);
        let opts: Vec<Choice> = transom_config::ARCHIVE_RETENTIONS
            .iter()
            .map(|&m| {
                (
                    if m == 0 {
                        t("arc.forever")
                    } else {
                        tf("arc.months", &[&m.to_string()])
                    },
                    Hit::ArchiveRetention(m),
                    a.retention_months == m,
                )
            })
            .collect();
        y += self.choice_row(p, Rect::new(x0, y, w, 0), lw, &t("arc.retention"), opts);
        // stav: zdroj zapnuty? / pozastavene / zhrnutie suboru
        let u = &self.cfg.usage;
        if a.enabled && !(u.claude_local || u.codex_local) {
            y = self.infobar(p, x0, y, w, &t("arc.needsrc"));
        }
        if a.enabled && a.paused {
            y = self.paragraph_b(
                p,
                x0,
                y,
                w,
                &t("arc.paused_state"),
                12.0,
                true,
                th.foreground,
            ) + px(2);
        }
        let info = &self.arc.info;
        let (text, col) = if info.unreadable {
            (t("arc.unreadable"), th.low)
        } else if info.rows == 0 {
            (t("arc.empty"), th.muted)
        } else {
            let date = |d: Option<(i32, u32, u32)>| {
                d.map(|(yy, mm, dd)| pclock::long_date(transom_core::clock::Ymd::new(yy, mm, dd)))
                    .unwrap_or_default()
            };
            (
                tf(
                    "arc.info",
                    &[
                        &info.days.to_string(),
                        &info.rows.to_string(),
                        &date(info.from),
                        &date(info.to),
                        &format!("{} KB", (info.bytes / 1024).max(1)),
                    ],
                ),
                th.muted,
            )
        };
        y = self.paragraph(p, x0, y, w, &text, 12.0, col) + px(8);
        // zmazanie (dvojkrokove): len ked subor existuje
        if info.rows > 0 || info.unreadable {
            let items = if self.arc.confirm_delete {
                vec![
                    (t("arc.delete_confirm"), Hit::ArchiveDelete, true),
                    (t("aip.cancel"), Hit::ArchiveCancel, false),
                ]
            } else {
                vec![(t("arc.delete"), Hit::ArchiveDelete, false)]
            };
            y = self.flow_buttons(p, x0, y, w, items);
        }
        y = self.paragraph(p, x0, y, w, &t("arc.note"), 12.0, th.muted);
        self.card_end(s, x0, w, y)
    }

    fn paint_ai(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "aip.title");
        y = self.notes(p, x0, y, w, &["aip.intro", "aip.separate"]) + px(8);
        if !self.ai_visible() {
            y = self.infobar(p, x0, y, w, &t("aip.hidden_note"));
        }
        y += px(4);
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("aip.sources_h"));
        let u = self.cfg.usage.clone();
        let sources: [(u8, bool, &str, String, String); 3] = [
            (
                0,
                u.claude_local,
                "aip.claude_local",
                t("aip.claude_local.d1"),
                t("aip.claude_local.d2"),
            ),
            (
                1,
                u.claude_statusline,
                "aip.statusline",
                t("aip.statusline.d1"),
                tf("aip.statusline.d2", &[&cc::Cc::command()]),
            ),
            (
                2,
                u.codex_local,
                "aip.codex",
                t("aip.codex.d1"),
                t("aip.codex.d2"),
            ),
        ];
        for (i, on, title, d1, d2) in sources {
            // popisy sa zalamuju (cesta k suboru aj dlhe preklady); vyska riadku sa odvodi od poctu riadkov
            let dst = Ty::Secondary.style(th.muted, HAlign::Left);
            let dw = w - px(34);
            let l1 = Self::wrap_lines(p, &d1, &dst, dw);
            let l2 = Self::wrap_lines(p, &d2, &dst, dw);
            let lh = px(Ty::Secondary.line_lp());
            let h = px(26) + (l1.len() + l2.len()) as i32 * lh + px(6);
            let row = Rect::new(x0, y, w, h);
            if self.hover == Some(Hit::Src(i)) {
                p.fill_round_rect(
                    Rect::new(x0 + px(30), y, w - px(30), h),
                    px(design::radius::CONTROL),
                    th.hover,
                );
            }
            self.checkbox(p, Rect::new(x0, y + px(3), px(20), px(20)), on);
            p.draw_text(
                &t(title),
                Rect::new(x0 + px(34), y, w - px(34), px(26)),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            let mut ly = y + px(26);
            for l in l1.iter().chain(l2.iter()) {
                p.draw_text(l, Rect::new(x0 + px(34), ly, dw, lh), &dst);
                ly += lh;
            }
            self.tag(&Hit::Src(i), t(title), Some(on));
            self.hits.push((row, Hit::Src(i)));
            y += h + px(6);
        }
        y = self.card_end(s, x0, w, y - px(6));
        y = self.paint_cc(p, x0, y, w);
        y = self.paint_archive(p, x0, y, w);
        // ukazovatel v liste: obnova udajov, poskytovatel a co kruh ukazuje (jedna karta)
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("aip.bar"));
        let lw = px(CHOICE_LABEL_LP);
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("aip.interval"),
            [
                (0u32, t("opt.on_demand")),
                (5, tf("opt.minutes", &["5"])),
                (15, tf("opt.minutes", &["15"])),
                (30, tf("opt.minutes", &["30"])),
                (60, tf("opt.minutes", &["60"])),
                (240, tf("opt.hours", &["4"])),
            ]
            .into_iter()
            .map(|(m, txt)| (txt, Hit::Interval(m), u.refresh_minutes == m))
            .collect(),
        );
        y = self.paragraph(p, x0, y, w, &t("aip.interval_note"), 12.0, th.muted) + px(10);
        let prov = transom_usage::limits::ProviderChoice::parse(&u.bar_provider);
        use transom_usage::limits::ProviderChoice as PC;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("aip.provider"),
            vec![
                (t("opt.auto"), Hit::Provider("auto"), prov == PC::Auto),
                (
                    "Claude Code".into(),
                    Hit::Provider("claude"),
                    prov == PC::Claude,
                ),
                ("Codex".into(), Hit::Provider("codex"), prov == PC::Codex),
            ],
        );
        let remaining = transom_usage::limits::RingMode::parse(&u.ring_mode)
            == transom_usage::limits::RingMode::Remaining;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("aip.mode"),
            vec![
                (t("opt.used"), Hit::Mode("used"), !remaining),
                (t("opt.left"), Hit::Mode("remaining"), remaining),
            ],
        );
        y = self.card_end(s, x0, w, y - px(4));
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("aip.data_h"));
        // vypnut vsetko / zmazat (druhy krok: cervene potvrdenie + Zrusit); rad sa zalamuje, nic nepretecie okno
        let mut items = vec![(t("aip.disable"), Hit::DisableAll, false)];
        if self.confirm_delete {
            items.push((t("aip.delete_confirm"), Hit::DeleteData, true));
            items.push((t("aip.cancel"), Hit::CancelDelete, false));
        } else {
            items.push((t("aip.delete"), Hit::DeleteData, false));
        }
        y = self.flow_buttons(p, x0, y, w, items);
        y = self.notes(
            p,
            x0,
            y,
            w,
            &["aip.note1", "aip.note2", "aip.note3", "aip.note4"],
        );
        y = self.card_end(s, x0, w, y);
        self.paint_dates(p, x0, y, w);
    }

    /// Kalendar: rucne datumy resetu a fakturacie (zoznam + formular). Fakturacny datum sa zo zdrojov zistit neda, preto
    /// ho zadava pouzivatel; nic sa neodvodzuje z limitov.
    fn paint_dates(&mut self, p: &mut dyn Painter, x0: i32, mut y: i32, w: i32) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("dat.title"));
        y = self.notes(p, x0, y, w, &["dat.note1", "dat.note2"]) + px(10);
        let today = pclock::today();
        let dates = self.cfg.usage.dates.clone();
        if dates.is_empty() {
            y = self.paragraph(p, x0, y, w, &t("dat.none"), 12.0, th.muted) + px(6);
        }
        for (i, m) in dates.iter().enumerate() {
            let provider = if m.provider == "codex" {
                "OpenAI Codex"
            } else {
                "Claude Code"
            };
            let kind = t(if m.kind == "billing" {
                "cal.kind.billing"
            } else {
                "cal.kind.reset"
            });
            let rep = match m.repeat.as_str() {
                "days" => tf("cal.repeat.days", &[&m.every_days.to_string()]),
                "monthly" => t("cal.repeat.monthly"),
                _ => t("cal.repeat.once"),
            };
            let next =
                match transom_core::schedule::rule_from_parts(&m.date, &m.repeat, m.every_days)
                    .and_then(|r| r.next_on_or_after(today))
                {
                    Some(d) => tf("dat.next", &[&pclock::long_date(d)]),
                    None => t("dat.noupcoming"),
                };
            let text = format!("{provider} · {kind} · {rep} · {} · {next}", m.date);
            let (l_edit, l_rm) = (t("dat.edit"), t("dat.remove"));
            let w_edit = design::button_width(
                p,
                &design::Button::new(&l_edit, Kind::Choice).state(design::State {
                    selected: true,
                    ..Default::default()
                }),
            );
            let w_rm = design::button_width(
                p,
                &design::Button::new(&l_rm, Kind::Secondary).icon('\u{E711}'),
            );
            let tw = (w - w_edit - w_rm - px(20)).max(px(120));
            let tst = Ty::Body.style(th.foreground, HAlign::Left);
            let lines = Self::wrap_lines(p, &text, &tst, tw);
            let lh = px(Ty::Body.line_lp());
            let ch = px(design::CONTROL_H_LP);
            let row_h = (lines.len() as i32 * lh).max(ch);
            let ty = if lines.len() == 1 {
                y + (ch - lh) / 2
            } else {
                y
            };
            for (k, l) in lines.iter().enumerate() {
                p.draw_text(l, Rect::new(x0, ty + k as i32 * lh, tw, lh), &tst);
            }
            let bx = x0 + w - w_edit - w_rm - px(8);
            self.button(
                p,
                Rect::new(bx, y, w_edit, ch),
                &l_edit,
                Hit::DateEdit(i),
                self.form.editing == Some(i),
            );
            self.button(
                p,
                Rect::new(bx + w_edit + px(8), y, w_rm, ch),
                &l_rm,
                Hit::DateRemove(i),
                false,
            );
            y += row_h + px(8);
        }
        // formular
        y += px(6);
        let lw = px(crate::CHOICE_LABEL_LP);
        let f = self.form.clone();
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("dat.provider"),
            vec![
                ("Claude Code".into(), Hit::DateProvider(0), f.provider == 0),
                ("OpenAI Codex".into(), Hit::DateProvider(1), f.provider == 1),
            ],
        );
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            "",
            vec![
                (t("cal.kind.reset"), Hit::DateKind(0), f.kind == 0),
                (t("cal.kind.billing"), Hit::DateKind(1), f.kind == 1),
            ],
        );
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("dat.repeat"),
            vec![
                (t("cal.repeat.once"), Hit::DateRepeat(0), f.repeat == 0),
                (
                    tf("cal.repeat.days", &[&f.every.to_string()]),
                    Hit::DateRepeat(1),
                    f.repeat == 1,
                ),
                (t("cal.repeat.monthly"), Hit::DateRepeat(2), f.repeat == 2),
            ],
        );
        if f.repeat == 1 {
            // interval: - / + (1-366 dni)
            let ch = px(design::CONTROL_H_LP);
            self.button(
                p,
                Rect::new(x0 + lw, y, ch, ch),
                "−",
                Hit::DateEvery(-1),
                false,
            );
            self.button(
                p,
                Rect::new(x0 + lw + ch + px(6), y, ch, ch),
                "+",
                Hit::DateEvery(1),
                false,
            );
            p.draw_text(
                &tf("cal.repeat.days", &[&f.every.to_string()]),
                Rect::new(x0 + lw + 2 * ch + px(18), y, w - lw - 2 * ch - px(18), ch),
                &Ty::BodyStrong.style(th.foreground, HAlign::Left),
            );
            y += ch + px(10);
        }
        // datum (natívne pole) + Pridat / Ulozit + Zrusit; dlhsi popisok (preklad) ide nad pole, nikdy sa neoreze
        let dl = t("dat.date");
        let dst = Ty::Body.style(th.foreground, HAlign::Left);
        if p.measure_text(&dl, &dst).w > lw - px(12) {
            for l in Self::wrap_lines(p, &dl, &dst, w) {
                p.draw_text(&l, Rect::new(x0, y, w, px(Ty::Body.line_lp())), &dst);
                y += px(Ty::Body.line_lp());
            }
            y += px(4);
        } else {
            p.draw_text(
                &dl,
                Rect::new(x0, y, lw - px(12), px(design::CONTROL_H_LP)),
                &dst,
            );
        }
        self.date_rect = Rect::new(x0 + lw, y + px(3), px(130), px(26));
        self.date_scroll = self.scroll;
        let full = dates.len() >= transom_config::MAX_MANUAL_DATES && f.editing.is_none();
        let add_label = if f.editing.is_some() {
            t("dat.save")
        } else {
            t("dat.add")
        };
        let aw = design::button_width(p, &design::Button::new(&add_label, Kind::Primary));
        let ch = px(design::CONTROL_H_LP);
        // tlacidlo kalendara hned za polom
        let cal_btn = Rect::new(self.date_rect.x + self.date_rect.w + px(6), y, ch, ch);
        self.button(p, cal_btn, "", Hit::DateCal, f.cal.is_some());
        let mut ax = cal_btn.x + cal_btn.w + px(10);
        // Pridat / Ulozit a Zrusit sa nezmestia vedla pola (dlhe preklady): idu na dalsi riadok pod pole
        let cw_need = if f.editing.is_some() {
            design::button_width(p, &design::Button::new(&t("dat.cancel"), Kind::Secondary)) + px(8)
        } else {
            0
        };
        if ax + aw.max(px(90)) + cw_need > x0 + w {
            y += ch + px(8);
            ax = x0 + lw;
        }
        if !full {
            self.button(
                p,
                Rect::new(ax, y, aw.max(px(90)), ch),
                &add_label,
                Hit::DateAdd,
                true,
            );
        }
        if f.editing.is_some() {
            let cl = t("dat.cancel");
            let cw = design::button_width(p, &design::Button::new(&cl, Kind::Secondary));
            self.button(
                p,
                Rect::new(ax + aw.max(px(90)) + px(8), y, cw, ch),
                &cl,
                Hit::DateCancel,
                false,
            );
        }
        y += ch + px(8);
        if let Some((cy, cm)) = f.cal {
            y = self.paint_date_cal(p, x0 + lw, y, cy, cm) + px(8);
        }
        if f.error {
            y = self.paragraph_b(
                p,
                x0 + lw,
                y,
                w - lw,
                &t("dat.invalid"),
                12.0,
                true,
                th.danger,
            ) + px(4);
        } else if full {
            y = self.paragraph(p, x0 + lw, y, w - lw, &t("dat.full"), 12.0, th.muted) + px(4);
        }
        self.card_end(s, x0, w, y)
    }

    /// Vnutro pola pre natívny EDIT: odsadene od ramika a vertikalne v strede (EDIT kresli text hore).
    fn field_inner(r: Rect) -> Rect {
        let lh = (r.h * 18 / 26).max(1);
        Rect::new(
            r.x + r.h * 4 / 26,
            r.y + (r.h - lh) / 2,
            r.w - r.h * 6 / 26,
            lh,
        )
    }

    /// Obraz natívnych poli stranky (ramik, podklad, text) pod nimi: pocas plynuleho posuvania su polia skryte a vidno
    /// tento obraz; inak ho prekryva samotne pole.
    fn paint_field_images(&self, p: &mut dyn Painter, x0: i32, origin: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut fields: Vec<(Rect, usize)> = Vec::new();
        match self.page {
            Page::Appearance => {
                let lay = appearance_page::layout(s, x0, origin, w);
                fields.push((lay.hex, 0));
                for (k, r) in lay.rgb.iter().enumerate() {
                    fields.push((*r, k + 1));
                }
            }
            Page::Ai if self.date_rect.w > 0 => fields.push((self.date_rect, 4)),
            Page::Extras if self.wx_rect.w > 0 => fields.push((self.wx_rect, 5)),
            _ => {}
        }
        for (r, i) in fields {
            if self.edits[i] == 0 {
                continue;
            }
            // pole pocasia ma ramik zo stranky (zaobleny); ostatne jemny ramik a zaoblenie ako ovladace
            if i == 5 {
                let rad = (px(design::radius::CONTROL) - px(1).max(1)).max(0);
                p.fill_round_rect(r, rad, th.placeholder);
            } else {
                let rad = px(design::radius::CONTROL);
                p.fill_round_rect(r, rad, th.control_edge);
                let b = px(1).max(1);
                p.fill_round_rect(
                    Rect::new(r.x + b, r.y + b, r.w - 2 * b, r.h - 2 * b),
                    (rad - b).max(0),
                    th.placeholder,
                );
            }
            let text = get_edit_text(self.edits[i]);
            let (shown, color) = if text.is_empty() && i == 5 {
                (t("ex.wx_search"), th.muted)
            } else {
                (text, th.foreground)
            };
            // presne ako natívny EDIT: text od laveho okraja vnutra, horna hrana riadku na hornej hrane vnutra
            let inner = Self::field_inner(r);
            let st = Ty::Body.style(color, HAlign::Left);
            let lh = p.measure_text("Ag", &st).h.max(1);
            p.draw_text(&shown, Rect::new(inner.x, inner.y, inner.w, lh), &st);
        }
    }

    /// Mesacny kalendar na vyber datumu (pod polom): hlavicka s mesiacom a sipkami, dni v tyzdni, mriezka dni.
    fn paint_date_cal(&mut self, p: &mut dyn Painter, x: i32, y: i32, cy: i32, cm: u32) -> i32 {
        use transom_core::clock::{days_in_month, weekday, Ymd};
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let cell = px(32);
        let cw = 7 * cell;
        let pad = px(8);
        let rows = {
            let lead = weekday(Ymd::new(cy, cm, 1)) as u32;
            (lead + days_in_month(cy, cm)).div_ceil(7) as i32
        };
        let h = pad + cell + px(22) + rows * cell + pad;
        let card = Rect::new(x, y, cw + 2 * pad, h);
        p.fill_round_rect(card, px(design::radius::CONTROL), th.control_edge);
        let b = px(1).max(1);
        p.fill_round_rect(
            Rect::new(card.x + b, card.y + b, card.w - 2 * b, card.h - 2 * b),
            px(design::radius::CONTROL),
            th.background,
        );
        let (x, mut y) = (x + pad, y + pad);
        // hlavicka: < Mesiac rok >
        self.button(
            p,
            Rect::new(x, y, cell, cell),
            "\u{2039}",
            Hit::DateCalMonth(-1),
            false,
        );
        self.button(
            p,
            Rect::new(x + cw - cell, y, cell, cell),
            "\u{203A}",
            Hit::DateCalMonth(1),
            false,
        );
        p.draw_text(
            &pclock::month_title(cy, cm),
            Rect::new(x + cell, y, cw - 2 * cell, cell),
            &Ty::BodyStrong.style(th.foreground, HAlign::Center),
        );
        y += cell;
        let ws = Ty::Caption.style(th.muted, HAlign::Center);
        for k in 0..7 {
            p.draw_text(
                &t(&format!("wd.s{k}")),
                Rect::new(x + k * cell, y, cell, px(22)),
                &ws,
            );
        }
        y += px(22);
        let today = pclock::today();
        let typed = if self.edits[4] != 0 {
            transom_core::schedule::parse_date(&get_edit_text(self.edits[4]))
        } else {
            None
        };
        let lead = weekday(Ymd::new(cy, cm, 1)) as i32;
        for d in 1..=days_in_month(cy, cm) {
            let i = lead + d as i32 - 1;
            let r = Rect::new(x + (i % 7) * cell, y + (i / 7) * cell, cell, cell);
            let day = Ymd::new(cy, cm, d);
            let picked = typed == Some(day);
            let inner = Rect::new(r.x + px(2), r.y + px(2), r.w - px(4), r.h - px(4));
            let hov = self.hover == Some(Hit::DateCalDay(d));
            if picked {
                p.fill_round_rect(inner, inner.h / 2, th.accent);
            } else if hov {
                p.fill_round_rect(inner, inner.h / 2, th.control_hover);
            } else if day == today {
                p.stroke_round_rect(inner, inner.h / 2, px(1).max(1), th.accent);
            }
            let fg = if picked { th.on_accent } else { th.foreground };
            p.draw_text(&d.to_string(), r, &Ty::Body.style(fg, HAlign::Center));
            self.hits.push((r, Hit::DateCalDay(d)));
        }
        card.y + card.h
    }

    fn paint_language(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "lang.title");
        y = self.notes(p, x0, y, w, &["lang.help1", "lang.help2"]);
        y += px(12);
        y = self.card_begin(s, y);
        let cur = self.cfg.language.trim().to_string();
        let is_system = cur.is_empty() || cur.eq_ignore_ascii_case("system");
        let detected = transom_i18n::resolve("system", &self.windows_lang);
        let sys_label = tf(
            "lang.system",
            &[&format!(
                "{} → {}",
                if self.windows_lang.is_empty() {
                    "?"
                } else {
                    &self.windows_lang
                },
                detected.native_name()
            )],
        );
        self.button(
            p,
            Rect::new(x0, y, w, px(design::CONTROL_H_LP)),
            &sys_label,
            Hit::Lang("system"),
            is_system,
        );
        y += px(design::CONTROL_H_LP + 14);
        // jazyky v dvoch stlpcoch, nazov v danom jazyku (neprekladaju sa)
        let cw = (w - px(10)) / 2;
        for (i, l) in Lang::ALL.iter().enumerate() {
            let r = Rect::new(
                x0 + (i as i32 % 2) * (cw + px(10)),
                y + (i as i32 / 2) * px(38),
                cw,
                px(32),
            );
            let active = !is_system && Lang::from_tag(&cur) == Some(*l);
            self.button(p, r, l.native_name(), Hit::Lang(l.code()), active);
        }
        let rows = (Lang::ALL.len() as i32 + 1) / 2;
        self.card_end(s, x0, w, y + rows * px(38) - px(6));
    }

    /// Znova nacita zhrnutie archivu (otvorenie stranky AI, po zmene / zmazani).
    fn refresh_archive(&mut self) {
        self.arc.info = transom_usage::archive::info(&transom_usage::archive::path());
    }

    /// Pripoji Claude Code (zaloha + zlucenie jedneho kluca). Uspech je vyslovny suhlas so zdrojom statusline, preto ho zapne.
    fn cc_connect(&mut self, replace: bool) -> bool {
        let (msg, ok) = self.cc.connect(replace);
        self.status = msg;
        if ok && !self.cfg.usage.claude_statusline {
            self.cfg.usage.claude_statusline = true;
            return true;
        }
        false
    }

    fn click(&mut self, h: Hit) -> bool {
        let mut changed = true;
        if !matches!(h, Hit::ToolsHotkey) {
            // ina volba ukonci zachytavanie skratky
            self.hotkey_capture = false;
        }
        if !matches!(h, Hit::Chip(_)) {
            // aktivacia ineho ovladaca potvrdi rozpracovany presun
            self.finish_move(true);
        }
        if matches!(
            h,
            Hit::TransferExport
                | Hit::TransferImport
                | Hit::TransferSection(_)
                | Hit::TransferApply
                | Hit::TransferCancel
        ) && (self.transfer_busy || self.read_only)
        {
            return false;
        }
        if !matches!(h, Hit::TransferApply) {
            // ine kliknutie ako na "Importovat vybrane" zrusi rozpracovane potvrdenie importu
            if let transfer_page::Transfer::Preview(p) = &mut self.transfer {
                p.confirm = None;
            }
        }
        if !matches!(h, Hit::DeleteData | Hit::CancelDelete) {
            self.confirm_delete = false;
        }
        if !matches!(h, Hit::CcReplace) {
            self.cc.confirm_replace = false;
        }
        if !matches!(h, Hit::ArchiveDelete | Hit::ArchiveCancel) {
            self.arc.confirm_delete = false;
        }
        match h {
            Hit::Cat(i) => {
                // rozbalit / zbalit; nic sa nenaviguje
                let i = (i as usize).min(CATS.len() - 1);
                self.expanded[i] = !self.expanded[i];
                self.nav_last = self.expanded[i].then_some(i);
                return false;
            }
            Hit::Nav(pg) => {
                self.page = pg;
                self.expanded[cat_of(pg)] = true;
                self.scroll = 0;
                // odchod zo stranky zahodi nahlad importu (subor mohol medzitym zastarat)
                self.transfer = transfer_page::Transfer::Idle;
                if pg == Page::Displays {
                    self.refresh_monitors();
                }
                if pg == Page::Ai {
                    self.cc.refresh();
                    self.refresh_archive();
                }
                self.anim_start = std::time::Instant::now();
                return true;
            }
            Hit::UpdAuto(v) => self.cfg.updates.auto_check = v,
            Hit::UpdIcon(v) => self.cfg.updates.show_icon = v,
            Hit::Wel(id, v) => changed = self.welcome_set(id, v),
            Hit::WelDone => {
                self.pending_close = true;
                changed = false;
            }
            Hit::UpdInstall => {
                notify_message(transom_config::UPDATE_INSTALL_MESSAGE);
                self.status = t("upd.downloading");
                changed = false;
            }
            Hit::UpdCheck => {
                notify_message(transom_config::UPDATE_CHECK_MESSAGE);
                self.status = t("upd.check_sent");
                changed = false;
            }
            Hit::MarqueeOn(v) => {
                self.cfg.media.marquee = v;
                self.anim_start = std::time::Instant::now();
            }
            Hit::MarqueeDir(d) => {
                self.cfg.media.marquee_direction = d.into();
                self.anim_start = std::time::Instant::now();
            }
            Hit::MarqueeSpeed(sp) => {
                self.cfg.media.marquee_speed = sp.into();
                self.anim_start = std::time::Instant::now();
            }
            Hit::MarqueePaused(v) => self.cfg.media.marquee_paused = v,
            Hit::ThemeColor => {
                self.set_bg(None);
                self.fill_edits(None);
            }
            Hit::Swatch(i) => {
                self.set_bg(Some(appearance_page::presets()[i]));
                self.fill_edits(None);
            }
            Hit::Pick => {
                self.pending_pick = true;
                changed = false;
            }
            Hit::Transparent => {
                self.cfg.appearance.bg_transparent = !self.cfg.appearance.bg_transparent;
            }
            Hit::TrayIcon => {
                self.cfg.appearance.tray_icon = !self.cfg.appearance.tray_icon;
            }
            Hit::GlassPanels => {
                self.cfg.appearance.glass_panels = !self.cfg.appearance.glass_panels;
                // vzhlad okna nastaveni sa riadi tou istou volbou
                self.theme = window_theme(self.cfg.appearance.glass_panels);
                self.art = None;
            }
            Hit::GlassOpacity(v) => self.cfg.appearance.glass_opacity = v,
            Hit::GlassBar => {
                self.cfg.appearance.glass_bar = !self.cfg.appearance.glass_bar;
            }
            Hit::TextMode(m) => self.cfg.appearance.text_mode = m.to_string(),
            Hit::TextPick => {
                self.pending_text_pick = true;
                changed = false;
            }
            Hit::OledToggle(id) => appearance_page::oled_toggle(&mut self.cfg.oled, id),
            Hit::OpacityTrack => changed = false,
            Hit::OpacityStep(d) => {
                let v = self.cfg.appearance.opacity_percent() as i32 + d;
                self.cfg.appearance.bg_opacity = appearance_page::clamp_opacity(v);
            }
            Hit::SoundToggle => {
                self.cfg.ui_sounds.enabled = !self.cfg.ui_sounds.enabled;
            }
            Hit::SoundTrack => changed = false,
            Hit::SoundStep(d) => {
                let v = self.cfg.ui_sounds.volume as i32 + d;
                self.cfg.ui_sounds.volume = sound_page::clamp_volume(v);
                // krok -/+ je umysel pouzivatela: ukazka po ulozeni (viz nizsie)
                self.sound_preview_pending = true;
            }
            Hit::SoundPreview => {
                self.preview_button();
                changed = false;
            }
            Hit::AudioQuick(id) => {
                let on = self.cfg.audio_panel.has(id);
                self.cfg.audio_panel.set(id, !on);
            }
            Hit::PeriphCount(n) => self.cfg.peripherals.on_bar = n,
            Hit::ExWork(v) => self.cfg.timer.work_min = v,
            Hit::ExBreak(v) => self.cfg.timer.short_break_min = v,
            Hit::ExLong(v) => self.cfg.timer.long_break_min = v,
            Hit::ExRounds(v) => self.cfg.timer.rounds = v,
            Hit::ExSound => self.cfg.timer.sound = !self.cfg.timer.sound,
            Hit::ExSys(id) => extras_page::sys_toggle(&mut self.cfg.sysmon, id),
            Hit::ExSysInterval(v) => self.cfg.sysmon.interval_secs = v,
            Hit::ExMicKey(k) => self.cfg.privacy.mic_hotkey = k.to_string(),
            Hit::ExMicShow => self.cfg.privacy.always_show = !self.cfg.privacy.always_show,
            Hit::IcsAdd => changed = self.ics_add_from_clipboard(),
            Hit::WeatherPick(i) => changed = self.wx_pick(i),
            Hit::WeatherUse(i) => {
                changed = self.wx_select(i.map_or(
                    transom_config::WeatherSource::None,
                    transom_config::WeatherSource::Place,
                ))
            }
            Hit::WeatherDevice => changed = self.wx_select(transom_config::WeatherSource::Device),
            Hit::WeatherRemove(i) => changed = self.wx_remove(i),
            Hit::WeatherGeoSettings => {
                transom_platform::geoloc::open_settings();
                changed = false;
            }
            Hit::IcsRemove(i) => {
                if i < self.cfg.ics_calendars.len() {
                    self.cfg.ics_calendars.remove(i);
                    self.extras_msg = None;
                }
            }
            Hit::PeriphInterval(v) => self.cfg.peripherals.interval_secs = v,
            Hit::ToolsToggle(_)
            | Hit::ToolsCycle(_)
            | Hit::ToolsClear(_)
            | Hit::ToolsAddFolder(_)
            | Hit::ToolsRemoveFolder(..)
            | Hit::ToolsAddApp
            | Hit::ToolsRemoveApp(_)
            | Hit::ToolsHotkey
            | Hit::ToolsHotkeyOff
            | Hit::ToolsHotkeyDefault => {
                changed = self.tools_click(&h);
            }
            Hit::ResetAppearance => {
                self.cfg.appearance = transom_config::Appearance::default();
                self.bad = [false; 4];
                self.fill_edits(None);
            }
            Hit::Chip(_) => return false,
            Hit::TransferExport => {
                self.pending_xfer = Some(transfer_page::XferOp::Export);
                changed = false;
            }
            Hit::TransferImport => {
                self.pending_xfer = Some(transfer_page::XferOp::Import);
                changed = false;
            }
            Hit::TransferSection(i) => {
                self.transfer_toggle(i);
                changed = false;
            }
            Hit::TransferApply => {
                self.transfer_apply();
                changed = false;
            }
            Hit::TransferCancel => {
                self.cancel_transfer();
                changed = false;
            }
            Hit::Show(id) => {
                let vis = self
                    .cfg
                    .layout
                    .items
                    .iter()
                    .any(|s| s.id == id && s.visible);
                self.cfg.layout.set_visible(&id, !vis);
            }
            Hit::ResetLayout => self.cfg.layout = Layout::default(),
            Hit::NotchOpt(id) => notch_page::toggle(&mut self.cfg, id),
            Hit::ToolTile(id) => toolgroup_page::toggle(&mut self.cfg, id),
            Hit::AddSep(g) => {
                if self.cfg.layout.add_separator(g).is_none() {
                    changed = false;
                }
            }
            Hit::RemoveSep(id) => {
                if !self.cfg.layout.remove_separator(&id) {
                    changed = false;
                }
            }
            Hit::Preset(i) => {
                if let Some(&(_, wd, wd_long, secs, date)) = CLOCK_PRESETS.get(i) {
                    let c = &mut self.cfg.clock;
                    c.seconds = secs;
                    c.weekday = wd;
                    c.weekday_long = wd_long;
                    c.date = date;
                    c.hour = HourMode::Locale;
                    c.leading_zero = Tri::Locale;
                }
            }
            Hit::Hour(v) => self.cfg.clock.hour = v,
            Hit::Zero(v) => self.cfg.clock.leading_zero = v,
            Hit::Seconds(v) => self.cfg.clock.seconds = v,
            Hit::Weekday(v) => {
                self.cfg.clock.weekday = v > 0;
                self.cfg.clock.weekday_long = v == 2;
            }
            Hit::Date(v) => self.cfg.clock.date = v,
            Hit::First(v) => self.cfg.clock.first_day = v,
            Hit::Src(i) => {
                // zdroj riadi len citanie dat; viditelnost polozky sa tu nemeni (poznamka na stranke)
                let u = &mut self.cfg.usage;
                match i {
                    0 => u.claude_local = !u.claude_local,
                    1 => u.claude_statusline = !u.claude_statusline,
                    _ => u.codex_local = !u.codex_local,
                }
            }
            Hit::Interval(m) => self.cfg.usage.refresh_minutes = m,
            Hit::Provider(v) => self.cfg.usage.bar_provider = v.into(),
            Hit::Mode(v) => self.cfg.usage.ring_mode = v.into(),
            Hit::DisableAll => {
                let u = &mut self.cfg.usage;
                u.claude_local = false;
                u.claude_statusline = false;
                u.codex_local = false;
            }
            Hit::DateProvider(v) => {
                self.form.provider = v;
                changed = false;
            }
            Hit::DateKind(v) => {
                self.form.kind = v;
                changed = false;
            }
            Hit::DateRepeat(v) => {
                self.form.repeat = v;
                changed = false;
            }
            Hit::DateEvery(d) => {
                self.form.every = (self.form.every as i32 + d).clamp(1, 366) as u32;
                changed = false;
            }
            Hit::DateCal => {
                changed = false;
                self.form.cal = match self.form.cal {
                    Some(_) => None,
                    None => {
                        // mesiac podla zadaneho datumu, inak dnesny
                        let typed = if self.edits[4] != 0 {
                            transom_core::schedule::parse_date(&get_edit_text(self.edits[4]))
                        } else {
                            None
                        };
                        let d = typed.unwrap_or_else(pclock::today);
                        Some((d.y, d.m))
                    }
                };
            }
            Hit::DateCalMonth(dm) => {
                changed = false;
                if let Some((y, m)) = self.form.cal {
                    self.form.cal = Some(transom_core::clock::add_months(y, m, dm));
                }
            }
            Hit::DateCalDay(d) => {
                changed = false;
                if let Some((y, m)) = self.form.cal.take() {
                    let iso = transom_core::schedule::iso(transom_core::clock::Ymd::new(y, m, d));
                    if self.edits[4] != 0 {
                        set_edit_text(self.edits[4], &iso);
                    }
                    self.form.error = false;
                }
            }
            Hit::DateCancel => {
                self.reset_date_form();
                changed = false;
            }
            Hit::DateEdit(i) => {
                changed = false;
                if let Some(m) = self.cfg.usage.dates.get(i).cloned() {
                    self.form = DateForm {
                        provider: u8::from(m.provider == "codex"),
                        kind: u8::from(m.kind == "billing"),
                        repeat: match m.repeat.as_str() {
                            "days" => 1,
                            "monthly" => 2,
                            _ => 0,
                        },
                        every: m.every_days.clamp(1, 366),
                        editing: Some(i),
                        error: false,
                        cal: None,
                    };
                    if self.edits[4] != 0 {
                        set_edit_text(self.edits[4], &m.date);
                    }
                }
            }
            Hit::DateRemove(i) => {
                if i < self.cfg.usage.dates.len() {
                    self.cfg.usage.dates.remove(i);
                    // upravovany zaznam sa odstranil alebo posunul: formular sa vynuluje
                    self.reset_date_form();
                } else {
                    changed = false;
                }
            }
            Hit::DateAdd => {
                let text = if self.edits[4] != 0 {
                    get_edit_text(self.edits[4])
                } else {
                    String::new()
                };
                match transom_core::schedule::parse_date(&text) {
                    None => {
                        self.form.error = true;
                        changed = false;
                    }
                    Some(d) => {
                        let f = self.form.clone();
                        let m = transom_config::ManualDate {
                            provider: if f.provider == 1 { "codex" } else { "claude" }.into(),
                            kind: if f.kind == 1 { "billing" } else { "reset" }.into(),
                            date: transom_core::schedule::iso(d),
                            repeat: ["none", "days", "monthly"][f.repeat.min(2) as usize].into(),
                            every_days: f.every.clamp(1, 366),
                        };
                        let list = &mut self.cfg.usage.dates;
                        match f.editing {
                            Some(i) if i < list.len() => list[i] = m,
                            _ if list.len() < transom_config::MAX_MANUAL_DATES => list.push(m),
                            _ => changed = false,
                        }
                        if changed {
                            self.reset_date_form();
                        }
                    }
                }
            }
            Hit::DispMode(m) => {
                let mons = self.monitors.clone();
                displays_page::set_mode(&mut self.cfg.displays, &mons, m);
            }
            Hit::DispToggle(i) => {
                let mons = self.monitors.clone();
                if !displays_page::toggle(&mut self.cfg.displays, &mons, i) {
                    changed = false;
                    self.status = t("disp.last");
                }
            }
            Hit::DispRemove(id) => {
                if !displays_page::remove_saved(&mut self.cfg.displays, &id) {
                    changed = false;
                    self.status = t("disp.last");
                }
            }
            Hit::DispIdentify => {
                changed = false;
                identify_displays(&self.monitors);
            }
            Hit::CancelDelete => {
                self.confirm_delete = false;
                changed = false;
            }
            Hit::DeleteData => {
                changed = false;
                if !self.confirm_delete {
                    self.confirm_delete = true;
                } else {
                    self.confirm_delete = false;
                    let d = transom_usage::cache::delete_local_data();
                    notify_message(transom_config::USAGE_DELETED_MESSAGE);
                    let n = d.len().to_string();
                    self.status = if self.cfg.usage.any_enabled() {
                        tf("status.deleted_enabled", &[&n])
                    } else {
                        tf("status.deleted", &[&n])
                    };
                }
            }
            Hit::CopyStatusline => {
                // iba prikaz s lomkami (Claude Code ho spusta cez Git Bash, ktory zjedava spatne lomky)
                let json = format!(
                    "\"statusLine\": {{ \"type\": \"command\", \"command\": {} }}",
                    serde_json_string(&cc::Cc::command())
                );
                self.status = if copy_text(&json) {
                    t("status.copied")
                } else {
                    t("status.copy_failed")
                };
                changed = false;
            }
            Hit::CcConnect | Hit::CcKeep => {
                changed = false;
                if h == Hit::CcConnect {
                    changed = self.cc_connect(false);
                }
            }
            Hit::CcReplace => {
                changed = false;
                if self.cc.confirm_replace {
                    changed = self.cc_connect(true);
                } else {
                    self.cc.confirm_replace = true;
                }
            }
            Hit::CcDisconnect => {
                changed = false;
                self.status = self.cc.disconnect();
            }
            Hit::CcTest => {
                changed = false;
                self.cc.run_test();
            }
            // archiv: zapnutie je **vedomy suhlas** (nikdy sa nezapne samo); zapnutie zrusi pozastavenie
            Hit::ArchiveEnable => {
                let a = &mut self.cfg.usage.archive;
                a.enabled = !a.enabled;
                if a.enabled {
                    a.paused = false;
                }
                self.status = t(if a.enabled { "arc.on" } else { "arc.off" });
            }
            Hit::ArchivePause => {
                let a = &mut self.cfg.usage.archive;
                if !a.enabled {
                    return false;
                }
                a.paused = !a.paused;
                self.status = t(if a.paused {
                    "arc.paused_now"
                } else {
                    "arc.resumed"
                });
            }
            Hit::ArchiveRetention(m) => {
                let old = self.cfg.usage.archive.retention_months;
                self.cfg.usage.archive.retention_months = m;
                // kratsia retencia sa uplatni hned (aj pri pozastaveni); dlhsia nic nemaze
                if m != 0 && (old == 0 || m < old) {
                    let today = pclock::today();
                    let n = transom_usage::archive::prune_file(
                        &transom_usage::archive::path(),
                        today,
                        m,
                    )
                    .unwrap_or(0);
                    if n > 0 {
                        self.status = tf("arc.pruned", &[&n.to_string()]);
                    }
                    self.refresh_archive();
                }
            }
            Hit::ArchiveCancel => {
                self.arc.confirm_delete = false;
                changed = false;
            }
            Hit::ArchiveDelete => {
                changed = false;
                if !self.arc.confirm_delete {
                    self.arc.confirm_delete = true;
                } else {
                    self.arc.confirm_delete = false;
                    self.status =
                        match transom_usage::archive::delete(&transom_usage::archive::path()) {
                            Ok(true) => t("arc.deleted"),
                            Ok(false) => t("arc.nothing"),
                            Err(e) => tf("arc.delete_failed", &[&e.to_string()]),
                        };
                    notify_message(transom_config::USAGE_DELETED_MESSAGE);
                    self.refresh_archive();
                }
            }
            Hit::Lang(code) => {
                self.cfg.language = code.into();
                apply_lang(&self.cfg, &self.windows_lang);
            }
        }
        if changed {
            self.save();
        }
        // ukazka hlasitosti po kroku -/+ (az po ulozeni; stlmene zvuky nehraju)
        if std::mem::take(&mut self.sound_preview_pending) {
            self.play_preview();
        }
        true
    }
}

/// Painter posunuty do podobdlznika (nahlad lisaty v okne nastaveni).
struct Offset<'a> {
    inner: &'a mut dyn Painter,
    dx: i32,
    dy: i32,
    w: i32,
    h: i32,
}

impl Painter for Offset<'_> {
    fn size(&self) -> Size {
        Size {
            w: self.w,
            h: self.h,
        }
    }
    fn scale(&self) -> f32 {
        self.inner.scale()
    }
    fn fill_rect(&mut self, r: Rect, c: Color) {
        self.inner
            .fill_rect(Rect::new(r.x + self.dx, r.y + self.dy, r.w, r.h), c);
    }
    fn measure_text(&mut self, t: &str, s: &TextStyle) -> Size {
        self.inner.measure_text(t, s)
    }
    fn draw_text(&mut self, t: &str, r: Rect, s: &TextStyle) {
        self.inner
            .draw_text(t, Rect::new(r.x + self.dx, r.y + self.dy, r.w, r.h), s);
    }
    fn blit(&mut self, r: Rect, px: &[Color]) {
        self.inner
            .blit(Rect::new(r.x + self.dx, r.y + self.dy, r.w, r.h), px);
    }
    fn blend_rgba(&mut self, r: Rect, px: &[Rgba]) {
        self.inner
            .blend_rgba(Rect::new(r.x + self.dx, r.y + self.dy, r.w, r.h), px);
    }
    fn push_clip(&mut self, r: Rect) {
        self.inner
            .push_clip(Rect::new(r.x + self.dx, r.y + self.dy, r.w, r.h));
    }
    fn pop_clip(&mut self) {
        self.inner.pop_clip();
    }
}

fn set_edit_text(h: isize, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: retazec ukonceny nulou zije pocas volania; handle je nase edit pole.
    unsafe {
        let _ = SetWindowTextW(HWND(h as *mut _), PCWSTR(wide.as_ptr()));
    }
}

fn get_edit_text(h: isize) -> String {
    // SAFETY: dlzka obmedzena na 64 znakov, buffer ma `n + 1` prvkov (nula) a citanie sa obmedzi na `n`.
    unsafe {
        let hw = HWND(h as *mut _);
        let n = GetWindowTextLengthW(hw).clamp(0, 64) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(hw, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..got.min(n)])
    }
}

/// Zobrazi/skryje a umiestni vstupne polia podla stranky a DPI (rovnaka geometria ako pri kresleni).
fn place_edits(ui: &mut Ui, hwnd: HWND) {
    if ui.edits.contains(&0) {
        return;
    }
    // SAFETY: volania s handle nasho okna na jeho vlakne.
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = dpi as f32 / 96.0;
        if ui.edit_dpi != dpi {
            let old = ui.edit_font;
            let px = (13.0 * scale).round() as i32;
            // rovnake pismo ako kreslene popisky (Segoe UI Variable Text, zaloha Segoe UI)
            let (name, weight) =
                transom_render::fonts::face(transom_render::fonts::family(), Font::Ui, 13.0, false);
            let face: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let f = CreateFontW(
                -px,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH.0 as u32,
                PCWSTR(face.as_ptr()),
            );
            ui.edit_font = f.0 as isize;
            ui.edit_dpi = dpi;
            for e in ui.edits {
                SendMessageW(
                    HWND(e as *mut _),
                    WM_SETFONT,
                    Some(WPARAM(f.0 as usize)),
                    Some(LPARAM(1)),
                );
                // bez vnutornych okrajov: text zacina presne tam, kde ho kresli obraz pola (ziadny posun pri posuvani)
                // EM_SETMARGINS, EC_LEFTMARGIN | EC_RIGHTMARGIN, 0 / 0
                SendMessageW(HWND(e as *mut _), 0x00D3, Some(WPARAM(3)), Some(LPARAM(0)));
            }
            if old != 0 {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(old as *mut _));
            }
        }
        let show = ui.page == Page::Appearance;
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let px = |v: i32| (v as f32 * scale).round() as i32;
        let x0 = px(NAV_W_LP) + px(28);
        let w = (rc.right - x0 - px(36)).max(px(200));
        let top = px(chrome::CAPTION_LP) + if ui.note.is_some() { px(38) } else { 0 };
        let y0 = content_origin(scale, top) - ui.scroll;
        let lay = appearance_page::layout(scale, x0, y0, w);
        let vp = ui.viewport;
        let rects = [lay.hex, lay.rgb[0], lay.rgb[1], lay.rgb[2]];
        // datum rucneho zaznamu (stranka AI): pole lezi na mieste, ktore nakreslila posledna stranka, a len cele v okne obsahu
        {
            let h = HWND(ui.edits[4] as *mut _);
            let mut r = ui.date_rect;
            r.y += ui.date_scroll - ui.scroll;
            ui.date_placed = r;
            place_native(h, r, vp, ui.page == Page::Ai, ui.gliding);
        }
        // hladanie miesta (stranka Doplnky): rovnako ako datum; napoveda priamo v nativnom poli (inak by sa pri
        // posuvani striedala nakreslena napoveda s prazdnym polom)
        {
            let h = HWND(ui.edits[5] as *mut _);
            if ui.page == Page::Extras {
                let cue: Vec<u16> = t("ex.wx_search")
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                // EM_SETCUEBANNER; wParam 1 = napoveda aj s fokusom (kym je pole prazdne)
                SendMessageW(
                    h,
                    0x1501,
                    Some(WPARAM(1)),
                    Some(LPARAM(cue.as_ptr() as isize)),
                );
            }
            let mut r = ui.wx_rect;
            r.y += ui.wx_scroll - ui.scroll;
            ui.wx_placed = r;
            place_native(h, r, vp, ui.page == Page::Extras, ui.gliding);
        }
        for (i, r) in rects.iter().enumerate() {
            let h = HWND(ui.edits[i] as *mut _);
            place_native(h, *r, vp, show, ui.gliding);
        }
    }
}

thread_local! {
    /// Posledne farby poli (text, podklad, stetec) pre WM_CTLCOLOREDIT, ked stav okna nie je dostupny.
    static EDIT_COLORS: std::cell::Cell<(u32, u32, isize)> = const { std::cell::Cell::new((0, 0, 0)) };
}

/// Nativne pole na mieste `r`: v okne obsahu orezane (pri posuvani ciastocne viditelne), pocas plynuleho posuvania
/// skryte (stranka kresli jeho obraz). Mimo okna obsahu alebo inej stranky skryte.
fn place_native(h: HWND, r: Rect, vp: Rect, page: bool, gliding: bool) {
    // natívne pole lezi vo vnutri nakresleneho ramika: text zarovnany s obrazom pola, ktory kresli stranka
    let r = Ui::field_inner(r);
    // pocas plynuleho posuvania je pole skryte (stranka kresli jeho obraz); pole s fokusom ostava
    // SAFETY: GetFocus nema vedlajsie ucinky
    let focused = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == h;
    let page = page && (!gliding || focused);
    use windows::Win32::Graphics::Gdi::{
        CreateRectRgn, RedrawWindow, SetWindowRgn, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE,
    };
    // SAFETY: vlastne detske okno; oblast prevezme okno.
    unsafe {
        let top = r.y.max(vp.y);
        let bottom = (r.y + r.h).min(vp.y + vp.h);
        if !page || vp.h <= 0 || r.h <= 0 || bottom <= top {
            let _ = ShowWindow(h, SW_HIDE);
            return;
        }
        // najprv orezanie (v suradniciach pola), potom presun a hned prekreslenie: pole sa nikdy nenakresli cez okraj
        // obsahu ani so starym obrazom
        let full = top == r.y && bottom == r.y + r.h;
        let rgn = if full {
            None
        } else {
            Some(CreateRectRgn(0, top - r.y, r.w, bottom - r.y))
        };
        let _ = SetWindowRgn(h, rgn, false);
        let _ = SetWindowPos(
            h,
            None,
            r.x,
            r.y,
            r.w,
            r.h,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE
                | windows::Win32::UI::WindowsAndMessaging::SWP_SHOWWINDOW,
        );
        let _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME);
    }
}

/// Systemovy dialog vyberu farby (modalny; vola sa mimo vypozicky stavu okna).
fn pick_color(owner: HWND, current: (u8, u8, u8)) -> Option<(u8, u8, u8)> {
    use windows::Win32::Foundation::COLORREF;
    let rgb = |c: (u8, u8, u8)| COLORREF(c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16);
    let mut custom = [COLORREF(0x00FF_FFFF); 16];
    for (i, c) in appearance_page::presets().iter().enumerate() {
        custom[i] = rgb(*c);
    }
    let mut cc = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner,
        rgbResult: rgb(current),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    // SAFETY: `cc` je platna struktura a `custom` ma 16 prvkov, ako dialog vyzaduje.
    let ok = unsafe { ChooseColorW(&mut cc) }.as_bool();
    ok.then(|| {
        let v = cc.rgbResult.0;
        (
            (v & 0xFF) as u8,
            ((v >> 8) & 0xFF) as u8,
            ((v >> 16) & 0xFF) as u8,
        )
    })
}

/// Retazec ako JSON literal (uvodzovky a spatne lomky escapovane).
fn serde_json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn copy_text(s: &str) -> bool {
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: schranka sa zatvori vo vsetkych vetvach; do pamate GlobalAlloc sa kopiruje presne `wide.len()` znakov; po uspesnom SetClipboardData ju vlastni system, inak sa uvolni.
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = (|| -> Option<()> {
            let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let ptr = GlobalLock(h) as *mut u16;
            if ptr.is_null() {
                let _ = windows::Win32::Foundation::GlobalFree(Some(h));
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(h);
            // CF_UNICODETEXT = 13; schranka prevezme vlastnictvo pamate len pri uspechu, inak ju treba uvolnit
            if SetClipboardData(13, Some(HANDLE(h.0))).is_err() {
                let _ = windows::Win32::Foundation::GlobalFree(Some(h));
                return None;
            }
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

/// Oznami liste zmenu konfiguracie (broadcast registrovanej spravy top-level oknam).
fn notify_bar() {
    notify_message(transom_config::CHANGED_MESSAGE);
}

fn notify_message(name: &str) {
    // v testoch sa nic nerozosiela (netreba rusit bezace procesy pouzivatela)
    #[cfg(test)]
    let _ = name;
    #[cfg(not(test))]
    notify_message_impl(name);
}

#[cfg(not(test))]
fn notify_message_impl(name: &str) {
    let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: registrovana sprava bez ukazovatelov (wParam/lParam = 0) pre vsetky okna.
    unsafe {
        let msg = RegisterWindowMessageW(PCWSTR(name.as_ptr()));
        let _ = PostMessageW(Some(HWND(0xffff as *mut _)), msg, WPARAM(0), LPARAM(0));
    }
}

/// Nazov okna v aktualnom jazyku.
/// Znacka transom ako ikona okna (titulok, Alt+Tab, panel uloh): male aj velke rozmery podla DPI. Ikony ostavaju
/// zive do konca procesu (okno ich pouziva).
fn set_window_icons(hwnd: HWND) {
    use transom_platform::icons;
    // SAFETY: ikony sa priradia nasmu oknu cez WM_SETICON (synchronne, len handle ako cislo).
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        for (small, which) in [(true, 0usize), (false, 1usize)] {
            let px = icons::system_icon_px(dpi, small);
            if let Some(h) = icons::hicon(icons::APP_ICO, px) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(which)),
                    Some(LPARAM(h.0 as isize)),
                );
            }
        }
    }
}

fn set_title(hwnd: HWND) {
    let title: Vec<u16> = t("set.title")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: retazec ukonceny nulou zije pocas volania.
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
    }
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn xy(lp: LPARAM) -> (i32, i32) {
    (
        (lp.0 & 0xFFFF) as u16 as i16 as i32,
        ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

fn hit_at(ui: &Ui, x: i32, y: i32) -> Option<Hit> {
    let inside = |r: &Rect| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;
    // policko "Zobrazit v liste" lezi v chipe a ma prednost pred nim
    ui.hits
        .iter()
        .find(|(r, h)| matches!(h, Hit::Show(_)) && inside(r))
        .or_else(|| ui.hits.iter().find(|(r, _)| inside(r)))
        .map(|(_, h)| h.clone())
}

fn colorref(c: Color) -> u32 {
    c.r as u32 | (c.g as u32) << 8 | (c.b as u32) << 16
}

/// DWM: tmave systemove prvky (menu na Alt+Medzernik), jemny okraj v farbe okna a zaoblene rohy (Windows 11; inde sa
/// volanie ignoruje).
/// 32-bitova DIB sekcia `w` x `h` pre pamatovy DC `dc` (`None` = nepodarilo sa, pouzije sa bitmapa zariadenia).
fn dib_buffer(
    dc: windows::Win32::Graphics::Gdi::HDC,
    w: i32,
    h: i32,
) -> Option<(windows::Win32::Graphics::Gdi::HBITMAP, *mut u32)> {
    use windows::Win32::Graphics::Gdi::{
        CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: DIB pre nas pamatovy DC; bity vlastni bitmapa (uvolni ju volajuci cez DeleteObject).
    unsafe { CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }
        .ok()
        .filter(|_| !bits.is_null())
        .map(|b| (b, bits as *mut u32))
}

fn apply_dwm(hwnd: HWND, th: &Theme) {
    // SAFETY: atributy DWM s ukazovatelmi na lokalne cisla a ich velkostou.
    unsafe {
        let dark = th.dark as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(20),
            &dark as *const i32 as *const _,
            4,
        );
        let border = colorref(th.edge);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(34),
            &border as *const u32 as *const _,
            4,
        );
        let round = 2i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(33),
            &round as *const i32 as *const _,
            4,
        );
    }
}

/// Tlacidlo hlavicky z hit-test kodu.
fn btn_of(code: u32) -> Option<chrome::Btn> {
    match code {
        HTMINBUTTON => Some(chrome::Btn::Min),
        HTMAXBUTTON => Some(chrome::Btn::Max),
        HTCLOSE => Some(chrome::Btn::Close),
        _ => None,
    }
}

/// Vykona tlacidlo hlavicky systemovym prikazom (rovnako ako systemove tlacidla; Alt+F4 a Alt+Medzernik ostavaju systemove).
fn press_button(hwnd: HWND, b: chrome::Btn) {
    // SAFETY: PostMessageW len s cislami na nase okno.
    unsafe {
        let max = windows::Win32::UI::WindowsAndMessaging::IsZoomed(hwnd).as_bool();
        let _ = PostMessageW(
            Some(hwnd),
            WM_SYSCOMMAND,
            WPARAM(chrome::sys_command(b, max)),
            LPARAM(0),
        );
    }
}

/// Klavesy hlavicky (zavolane zo slucky sprav pred `TranslateMessage`, aby ich neodchytilo vstupne pole): Tab / Shift+Tab
/// prechadza tlacidla, sipky vlavo/vpravo medzi nimi, Enter / medzernik vykona, Esc zrusi fokus. Vrati, ci kluc spracovala.
/// Sprava pre zachytavanie skratky; `true` = spracovana (neposiela sa dalej).
fn hotkey_capture_msg(hwnd: HWND, message: u32, vk: u32) -> bool {
    // WM_KEYDOWN, WM_SYSKEYDOWN / WM_KEYUP, WM_SYSKEYUP, WM_CHAR, WM_SYSCHAR
    let down = matches!(message, 0x0100 | 0x0104);
    let other = matches!(message, 0x0101 | 0x0105 | 0x0102 | 0x0106);
    if !down && !other {
        return false;
    }
    // SAFETY: GetKeyState nema predpoklady.
    let key = |k: i32| unsafe { GetKeyState(k) } < 0;
    let mut handled = false;
    let mut changed = false;
    let any_mod = key(0x11) || key(0x12) || key(0x10) || key(0x5B) || key(0x5C);
    with_ui(|ui| {
        if !ui.hotkey_capture {
            if ui.hotkey_swallow {
                // po zachyteni este prehltnut uvolnenie Alt (inak by okno preslo do rezimu menu); potom uz nie
                handled = matches!(message, 0x0105 | 0x0106);
                ui.hotkey_swallow = any_mod;
            }
            return;
        }
        handled = true;
        if down {
            changed =
                ui.hotkey_capture_key(vk, key(0x11), key(0x12), key(0x10), key(0x5B) || key(0x5C));
            if !ui.hotkey_capture {
                ui.hotkey_swallow = true;
            }
        }
    });
    if changed {
        with_ui(|ui| ui.save());
    }
    if handled && down {
        // SAFETY: prekreslenie nasho okna.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    handled
}

fn chrome_key(hwnd: HWND, edit: Option<HWND>, vk: u32, repeat: bool) -> bool {
    use focus::{KeyAction, PageFocus, Slot};
    // SAFETY: GetKeyState nema predpoklady.
    let shift = unsafe { GetKeyState(0x10) } < 0;
    // nasledky (SetFocus, aktivacia, tlacidlo hlavicky) sa robia az mimo vypozicky stavu: vyvolavaju dalsie spravy okna
    let mut to_focus: Option<HWND> = None;
    let mut activate: Option<Hit> = None;
    let mut press: Option<chrome::Btn> = None;
    let mut arm_commit = false;
    let handled = with_ui(|ui| {
        let edit_idx = edit.and_then(|e| ui.edits.iter().position(|&x| x == e.0 as isize));
        let chrome_focused = ui.chrome.focus.is_some();
        let focused = chrome_focused || ui.kfocus.is_some();
        let page = match &ui.kfocus {
            Some(Hit::Chip(_)) => PageFocus::Chip {
                moving: ui.moving.is_some(),
            },
            Some(Hit::OpacityTrack | Hit::SoundTrack) => PageFocus::Slider,
            _ if ui.transfer_open() => PageFocus::Preview,
            _ => PageFocus::Other,
        };
        match focus::route_key_ctx(
            vk,
            shift,
            edit.is_some(),
            focused,
            chrome_focused,
            page,
            repeat,
        ) {
            KeyAction::Pass => false,
            KeyAction::ClosePreview => {
                ui.cancel_transfer();
                // Esc je klavesnica: fokus sa vrati na tlacidlo Importovat
                ui.kfocus = Some(Hit::TransferImport);
                ui.reveal = ui.kfocus.clone();
                true
            }
            KeyAction::Grab => {
                if let Some(Hit::Chip(id)) = ui.kfocus.clone() {
                    ui.begin_move(&id);
                }
                true
            }
            KeyAction::Drop => {
                ui.finish_move(true);
                true
            }
            KeyAction::Nudge(d) => {
                ui.nudge_move(d);
                true
            }
            KeyAction::Slider { key, fine } => {
                arm_commit = ui.slider_key(key, fine);
                true
            }
            KeyAction::Cancel => {
                if let Some(i) = edit_idx {
                    // Esc v poli: vrati hodnotu spred editacie a neulozi zamietnutu; az potom sa pole opusti
                    ui.edit_cancel(i);
                    to_focus = Some(hwnd);
                } else {
                    ui.finish_move(false);
                }
                true
            }
            KeyAction::Move { back } => {
                // Tab pocas presunu ho potvrdi
                ui.finish_move(true);
                let page: Vec<Hit> = ui.all_hits.iter().map(|(_, h)| h.clone()).collect();
                let ring = focus::ring(&page, &visible_edits(ui));
                let cur = match (edit_idx, ui.kfocus.clone(), ui.chrome.focus) {
                    (Some(i), _, _) => Some(Slot::Edit(i)),
                    (None, Some(h), _) => Some(Slot::Page(h)),
                    (None, None, Some(b)) => Some(Slot::Chrome(b)),
                    _ => None,
                };
                ui.kfocus = None;
                ui.chrome.focus = None;
                to_focus = Some(hwnd);
                match focus::step(&ring, cur.as_ref(), back) {
                    Some(Slot::Page(h)) => {
                        ui.ensure_visible(&h);
                        place_edits(ui, hwnd);
                        ui.kfocus = Some(h);
                    }
                    Some(Slot::Edit(i)) => to_focus = Some(HWND(ui.edits[i] as *mut _)),
                    Some(Slot::Chrome(b)) => ui.chrome.focus = Some(b),
                    None => {}
                }
                true
            }
            KeyAction::Activate => {
                if edit.is_some() {
                    // potvrdenie hodnoty: pole opusti fokus (EN_KILLFOCUS ulozi); neplatnu hodnotu nepotvrdi (Esc ju vrati)
                    if edit_idx.is_some_and(|i| i < 4) && ui.bad.iter().any(|b| *b) {
                        ui.status = t("app.invalid");
                    } else if edit_idx == Some(5) && !ui.wx.results.is_empty() {
                        // Enter v hladani miesta: prvy vysledok
                        activate = Some(Hit::WeatherPick(0));
                    } else {
                        to_focus = Some(hwnd);
                    }
                } else if let Some(h) = ui.kfocus.clone() {
                    activate = Some(h);
                } else {
                    press = ui.chrome.focus;
                }
                true
            }
            KeyAction::Clear => {
                ui.kfocus = None;
                ui.chrome.focus = None;
                if edit.is_some() {
                    to_focus = Some(hwnd);
                }
                true
            }
            KeyAction::ChromeSide { right } => {
                // sipky sa nezacykluju do "ziadne": ostanu na okrajoch
                ui.chrome.focus = match ui.chrome.focus {
                    Some(b) if !right => chrome::BTNS.get(b.index().saturating_sub(1)).copied(),
                    Some(b) => chrome::BTNS.get((b.index() + 1).min(2)).copied(),
                    None => None,
                };
                true
            }
        }
    })
    .unwrap_or(false);
    if arm_commit {
        // posuvnik: ulozi sa po chvili ticha (auto-opakovanie klavesu by inak ukladalo desiatky krat za sekundu)
        // SAFETY: casovac na nase okno; parametre su cisla.
        unsafe { SetTimer(Some(hwnd), TIMER_COMMIT, COMMIT_MS, None) };
    }
    if let Some(h) = to_focus {
        // SAFETY: fokus na nase edit pole, mimo vypozicky stavu UI.
        unsafe {
            let _ = SetFocus(Some(h));
        }
    }
    if let Some(h) = activate {
        // rovnako ako kliknutie mysou na ovladac
        activate_hit(hwnd, h);
    }
    if let Some(b) = press {
        press_button(hwnd, b);
    }
    if handled {
        // SAFETY: prekreslenie nasho okna.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    handled
}

/// Aktivuje ovladac stranky rovnako ako kliknutie mysou (klavesnica, citacka obrazovky).
fn activate_hit(hwnd: HWND, h: Hit) {
    let lang_before = transom_i18n::lang();
    with_ui(|ui| ui.click(h));
    after_click(hwnd, lang_before);
}

/// Zverejni snimku ovladacov stranky pre citacky obrazovky (MSAA `OBJID_CLIENT`): zostavi sa pod vypozickou stavu, odovzda
/// sa az po jej uvolneni (oznamenie udalosti moze vyvolat dalsie spravy okna).
fn publish_page_access(hwnd: HWND) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let built = with_ui(|ui| {
        // SAFETY: GetFocus nema predpoklady.
        let focus = unsafe { GetFocus() };
        let mut edits = Vec::new();
        let mut focused_edit = None;
        for i in visible_edits(ui) {
            let e = HWND(ui.edits[i] as *mut _);
            let mut rc = RECT::default();
            // SAFETY: vystupny RECT je lokalny; handle je nase edit pole.
            let (a, b) = unsafe {
                if GetWindowRect(e, &mut rc).is_err() {
                    continue;
                }
                let mut a = POINT {
                    x: rc.left,
                    y: rc.top,
                };
                let mut b = POINT {
                    x: rc.right,
                    y: rc.bottom,
                };
                let _ = ScreenToClient(hwnd, &mut a);
                let _ = ScreenToClient(hwnd, &mut b);
                (a, b)
            };
            if e == focus {
                focused_edit = Some(i);
            }
            edits.push((
                i,
                (a.x, a.y, b.x - a.x, b.y - a.y),
                get_edit_text(ui.edits[i]),
            ));
        }
        let (entries, extras, targets, focus_idx) = ui.a11y_snapshot(&edits, focused_edit);
        ui.acc_targets = targets;
        (ui.a11y_page_name(), entries, extras, focus_idx)
    });
    if let Some((name, entries, extras, focus_idx)) = built {
        transom_platform::menu_access::set_page(hwnd.0 as isize, name, entries, extras, focus_idx);
    }
}

/// Indexy vstupnych poli, ktore su prave zobrazene (kandidati kruhu fokusu klavesnice).
fn visible_edits(ui: &Ui) -> Vec<usize> {
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    ui.edits
        .iter()
        .enumerate()
        // SAFETY: IsWindowVisible nad nasim polom (zaniknute okno = false).
        .filter(|(_, e)| **e != 0 && unsafe { IsWindowVisible(HWND(**e as *mut _)) }.as_bool())
        .map(|(i, _)| i)
        .collect()
}

/// Po kliknuti alebo aktivacii klavesnicou: casovace nahladov, vstupne polia, dialog vyberu farby a titulok pri zmene jazyka.
fn after_click(hwnd: HWND, lang_before: transom_i18n::Lang) {
    // pocasie: overenie polohy zariadenia (vlakno) a vymazanie pola hladania po pridani miesta
    if with_ui(|ui| std::mem::take(&mut ui.wx.geo_check)).unwrap_or(false) {
        let target = hwnd.0 as isize;
        let _ = std::thread::Builder::new()
            .name("transom-geo".into())
            .spawn(move || {
                let code = match transom_platform::geoloc::access() {
                    Ok(()) => 0,
                    Err(transom_platform::geoloc::GeoError::Denied) => 1,
                    Err(_) => 2,
                };
                // SAFETY: okno moze byt medzitym zatvorene - PostMessage na neplatne okno len zlyha
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(target as *mut _)),
                        WM_APP_GEO,
                        WPARAM(code),
                        LPARAM(0),
                    );
                }
            });
    }
    with_ui(|ui| {
        if std::mem::take(&mut ui.wx.clear_field) && ui.edits[5] != 0 {
            ui.syncing = true;
            set_edit_text(ui.edits[5], "");
            ui.syncing = false;
        }
    });
    // SAFETY: casovace a volania s handle nasho okna na jeho vlakne.
    unsafe {
        with_ui(|ui| {
            // nahlad hodin so sekundami: casovac len na stranke hodin
            if ui.page == Page::Clock {
                SetTimer(Some(hwnd), TIMER_PREVIEW, 1000, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_PREVIEW);
            }
            // nahlad posuvneho textu: ~30 snimok/s len na stranke Media pri zapnutom posuve
            if ui.preview_animates() {
                SetTimer(Some(hwnd), TIMER_MEDIA, 33, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_MEDIA);
            }
            place_edits(ui, hwnd);
        });
        if with_ui(|ui| std::mem::take(&mut ui.pending_close)).unwrap_or(false) {
            let _ = PostMessageW(
                Some(hwnd),
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
        // dialog vyberu farby je modalny: spusta sa az po uvolneni vypozicky stavu
        if with_ui(|ui| std::mem::take(&mut ui.pending_pick)).unwrap_or(false) {
            if let Some(cur) = with_ui(|ui| ui.effective_bg()) {
                if let Some(c) = pick_color(hwnd, cur) {
                    with_ui(|ui| {
                        ui.set_bg(Some(c));
                        ui.fill_edits(None);
                        ui.save();
                    });
                }
            }
        }
        if with_ui(|ui| std::mem::take(&mut ui.pending_text_pick)).unwrap_or(false) {
            let cur = with_ui(|ui| ui.text_color_now()).unwrap_or((255, 255, 255));
            if let Some(c) = pick_color(hwnd, cur) {
                with_ui(|ui| {
                    ui.cfg.appearance.text_mode = "custom".into();
                    ui.cfg.appearance.text_color = transom_config::format_hex(c);
                    ui.save();
                });
            }
        }
        // export / import: dialog suboru (modalny, mimo vypozicky stavu) a spracovanie vybranej cesty
        transfer_page::run_pending(hwnd);
        tools_page::run_pending_folder(hwnd);
        tools_page::run_pending_app(hwnd);
        if transom_i18n::lang() != lang_before {
            set_title(hwnd);
            with_ui(|ui| name_edits(ui));
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Pristupne nazvy vstupnych poli v aktualnom jazyku.
fn name_edits(ui: &Ui) {
    for (e, name) in ui.edits.iter().zip(edit_names::names()) {
        if *e != 0 {
            edit_names::set(HWND(*e as *mut _), &name);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: wndproc nastaveni na vlakne UI: lParam sa dereferencuje len pri systemovych spravach s dokumentovanou strukturou (WM_NCCALCSIZE, WM_GETMINMAXINFO, WM_DPICHANGED) a text WM_SETTINGCHANGE cez overeny `setting_name`; stav cez `with_ui` bez vypozicky cez volania posielajuce spravy.
    unsafe {
        match msg {
            // vlastna hlavicka: klientska oblast zacina na hornom okraji okna (lavy/pravy/dolny rám zostava systemovy)
            WM_NCCALCSIZE if wp.0 != 0 => {
                let params =
                    &mut *(lp.0 as *mut windows::Win32::UI::WindowsAndMessaging::NCCALCSIZE_PARAMS);
                let saved_top = params.rgrc[0].top;
                let r = DefWindowProcW(hwnd, msg, wp, lp);
                params.rgrc[0].top = saved_top;
                if windows::Win32::UI::WindowsAndMessaging::IsZoomed(hwnd).as_bool() {
                    // maximalizovane okno presahuje pracovnu plochu o sirku ramu na vsetkych stranach
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    use windows::Win32::UI::WindowsAndMessaging::{SM_CXPADDEDBORDER, SM_CYFRAME};
                    params.rgrc[0].top += GetSystemMetricsForDpi(SM_CYFRAME, dpi)
                        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                }
                r
            }
            WM_NCHITTEST => {
                let r = DefWindowProcW(hwnd, msg, wp, lp);
                let code = r.0 as u32;
                // systemovy rám (lavy/pravy/dolny okraj a spodne rohy) ostava systemovy
                if code != HTCLIENT && code != HTCAPTION && !(HTTOP..=HTTOPRIGHT).contains(&code) {
                    return r;
                }
                let (sx, sy) = xy(lp);
                let mut pt = windows::Win32::Foundation::POINT { x: sx, y: sy };
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let dpi = GetDpiForWindow(hwnd).max(96);
                let max = windows::Win32::UI::WindowsAndMessaging::IsZoomed(hwnd).as_bool();
                use chrome::{Btn, Zone};
                LRESULT(match chrome::zone(pt.x, pt.y, rc.right, dpi, max) {
                    Zone::Client => HTCLIENT,
                    Zone::Caption => HTCAPTION,
                    Zone::Btn(Btn::Min) => HTMINBUTTON,
                    Zone::Btn(Btn::Max) => HTMAXBUTTON,
                    Zone::Btn(Btn::Close) => HTCLOSE,
                    Zone::Top => HTTOP,
                    Zone::TopLeft => HTTOPLEFT,
                    Zone::TopRight => HTTOPRIGHT,
                } as isize)
            }
            // hover tlacidiel hlavicky (nad nimi mys nepise klientske spravy)
            WM_NCMOUSEMOVE => {
                let hot = btn_of(wp.0 as u32);
                let redraw = with_ui(|ui| {
                    let mut ch = hot != ui.chrome.hover || ui.hover.is_some();
                    ui.chrome.hover = hot;
                    ui.hover = None;
                    if !ui.chrome.tracking {
                        ui.chrome.tracking = true;
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags:
                                windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT_FLAGS(
                                    TME_LEAVE_NC,
                                ),
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut t);
                        ch = true;
                    }
                    ch
                })
                .unwrap_or(false);
                if redraw {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCMOUSELEAVE => {
                with_ui(|ui| {
                    ui.chrome.tracking = false;
                    if ui.chrome.pressed.is_none() {
                        ui.chrome.hover = None;
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            // stlacenie tlacidla hlavicky: vlastne sledovanie (mys sa zachyti, vykona sa pri uvolneni nad nim)
            WM_NCLBUTTONDOWN if btn_of(wp.0 as u32).is_some() => {
                let b = btn_of(wp.0 as u32);
                with_ui(|ui| {
                    ui.chrome.pressed = b;
                    ui.chrome.hover = b;
                    ui.chrome.focus = None;
                });
                SetCapture(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                if lp.0 != hwnd.0 as isize {
                    with_ui(|ui| ui.chrome.pressed = None);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            // minimalna velkost (klientska oblast + rám), najviac pracovna plocha monitora
            WM_GETMINMAXINFO => {
                use windows::Win32::Graphics::Gdi::{
                    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
                };
                use windows::Win32::UI::WindowsAndMessaging::{
                    MINMAXINFO, SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYSIZEFRAME,
                };
                let mmi = &mut *(lp.0 as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(hwnd);
                let dpi = if dpi == 0 { 96 } else { dpi };
                let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                let fx = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + pad;
                let fy = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + pad;
                let mut mi = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let (mut ww, mut wh) = (i32::MAX, i32::MAX);
                if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi)
                    .as_bool()
                {
                    ww = mi.rcWork.right - mi.rcWork.left;
                    wh = mi.rcWork.bottom - mi.rcWork.top;
                }
                let (w, h) = chrome::min_track(dpi, fx, fy, ww, wh);
                mmi.ptMinTrackSize = windows::Win32::Foundation::POINT { x: w, y: h };
                LRESULT(0)
            }
            WM_ACTIVATE => {
                with_ui(|ui| ui.chrome.active = (wp.0 & 0xFFFF) != 0);
                let _ = InvalidateRect(Some(hwnd), None, false);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            // zmena temy Windows (svetla / tmava): prekreslit, znova nacitat farby poli a DWM atributy
            WM_SETTINGCHANGE => {
                // text cita len overenu pamat a s obmedzenou dlzkou (ukazovatel posiela odosielatel spravy)
                let is_theme = transom_platform::appbar::setting_name(lp)
                    .is_some_and(|s| s == "ImmersiveColorSet");
                if is_theme {
                    let glass = with_ui(|ui| ui.cfg.appearance.glass_panels).unwrap_or(true);
                    let th = window_theme(glass);
                    with_ui(|ui| {
                        ui.theme = th;
                        ui.art = None;
                        if ui.edit_brush != 0 {
                            EDIT_COLORS.with(|e| e.set((0, 0, 0)));
                            let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(
                                ui.edit_brush as *mut _,
                            ));
                            ui.edit_brush = 0;
                        }
                        for e in ui.edits {
                            let _ = InvalidateRect(Some(HWND(e as *mut _)), None, true);
                        }
                    });
                    apply_dwm(hwnd, &th);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            // pristupnost: tlacidla hlavicky ako `OBJID_TITLEBAR`
            WM_GETOBJECT => {
                let h = hwnd.0 as isize;
                // hlavicka (`OBJID_TITLEBAR`) a klientska plocha s ovladacmi stranky (`OBJID_CLIENT`)
                match transom_platform::menu_access::titlebar_object(h, wp.0, lp.0)
                    .or_else(|| transom_platform::menu_access::page_object(h, wp.0, lp.0))
                {
                    Some(r) => LRESULT(r),
                    None => DefWindowProcW(hwnd, msg, wp, lp),
                }
            }
            // `accDoDefaultAction` ovladaca stranky: rovnako ako klik / Enter
            transom_platform::menu_access::WM_TRANSOM_INVOKE_PAGE => {
                match with_ui(|ui| ui.acc_targets.get(wp.0).cloned()).flatten() {
                    Some(page_a11y::Target::Hit(h)) => activate_hit(hwnd, h),
                    Some(page_a11y::Target::Edit(i)) => {
                        if let Some(e) = with_ui(|ui| ui.edits.get(i).copied()).flatten() {
                            let _ = SetFocus(Some(HWND(e as *mut _)));
                        }
                    }
                    Some(page_a11y::Target::Static) | None => {}
                }
                LRESULT(0)
            }
            // `accDoDefaultAction` tlacidla hlavicky
            transom_platform::access::WM_TRANSOM_INVOKE => {
                if let Some(b) = chrome::BTNS.get(wp.0) {
                    press_button(hwnd, *b);
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let (w, h) = (rc.right.max(1), rc.bottom.max(1));
                let mem = CreateCompatibleDC(Some(hdc));
                // buffer v pamati (DIB, ako panely): kreslenie a miesanie robi GDI v softveri, nie ovladac grafiky - na
                // niektorych ovladacoch `AlphaBlend` do bitmapy zariadenia ignoroval alfu (cierne rohy kariet)
                let dib = dib_buffer(mem, w, h);
                let bmp = dib.map_or_else(|| CreateCompatibleBitmap(hdc, w, h), |d| d.0);
                let old = SelectObject(mem, bmp.into());
                let dpi = GetDpiForWindow(hwnd).max(96);
                with_ui(|ui| {
                    let mut p = GdiPainter::new(mem, Size { w, h }, dpi);
                    ui.paint(&mut p);
                });
                // okno je cele nepriehladne: GDI necha alfu bodov 0, miesanie ju nastavi na 255 - nastroje, ktore oknam
                // pridavaju priehladnost podla alfy (napr. Mica pre vsetky okna), by inak ukazali pozadie len cez cast
                // okna a rohy kariet by boli cierne
                if let Some((_, bits)) = dib {
                    let _ = windows::Win32::Graphics::Gdi::GdiFlush();
                    let px = std::slice::from_raw_parts_mut(bits, (w * h) as usize);
                    for v in px {
                        *v |= 0xFF00_0000;
                    }
                }
                let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                let _ = EndPaint(hwnd, &ps);
                // citacky obrazovky: tlacidla hlavicky (nazvy, obdlzniky, fokus)
                let title_focus = with_ui(|ui| {
                    transom_platform::menu_access::set_titlebar(
                        ui.titlebar_entries(w, dpi),
                        ui.chrome.focus.map(|b| b.index()),
                    );
                    let f = ui.chrome.focus.map(|b| b.index());
                    (f != ui.last_title_focus).then(|| {
                        ui.last_title_focus = f;
                        f
                    })
                })
                .flatten()
                .flatten();
                if let Some(f) = title_focus {
                    transom_platform::menu_access::notify_titlebar_focus(hwnd.0 as isize, f);
                }
                // ovladace stranky (nazvy, roly, stavy, hodnoty, fokus)
                publish_page_access(hwnd);
                // rozsah posuvania sa po kresleni zmenil (napr. iny jazyk / DPI): dokresli sa so spravnym posunom
                // natívne pole datumu musi sediet na mieste, ktore prave nakreslila stranka AI
                with_ui(|ui| {
                    let mut r = ui.date_rect;
                    r.y += ui.date_scroll - ui.scroll;
                    let mut q = ui.wx_rect;
                    q.y += ui.wx_scroll - ui.scroll;
                    if (ui.page == Page::Ai && r != ui.date_placed)
                        || (ui.page == Page::Extras && q != ui.wx_placed)
                    {
                        place_edits(ui, hwnd);
                    }
                });
                if with_ui(|ui| std::mem::take(&mut ui.repaint)).unwrap_or(false) {
                    with_ui(|ui| place_edits(ui, hwnd));
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            // koliesko mysi a klavesnica posuvaju obsah stranky
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) as u16 as i16) as i32;
                let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                let step = -delta * (56 * dpi / 96) / 120;
                // plynulo: koliesko posunie ciel, snimky s obnovou obrazovky dokĺzu (bez animacii vo Windows hned)
                let moved =
                    with_ui(|ui| ui.glide.wheel(ui.scroll, step, ui.max_scroll)).unwrap_or(false);
                if moved {
                    if media_page::animations_enabled() {
                        GLIDE.with(|g| {
                            if g.borrow().is_none() {
                                *g.borrow_mut() =
                                    Some(transom_platform::vsync::Pacer::start(hwnd, TIMER_GLIDE));
                            }
                        });
                    } else {
                        with_ui(|ui| {
                            let mut s = ui.scroll;
                            ui.glide.finish(&mut s);
                            ui.scroll = s.clamp(0, ui.max_scroll);
                            place_edits(ui, hwnd);
                        });
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                let line = 40 * dpi / 96;
                let handled = with_ui(|ui| {
                    let page = (ui.viewport.h - line).max(line);
                    let d = match wp.0 as u32 {
                        0x26 => -line,
                        0x28 => line,
                        0x21 => -page,
                        0x22 => page,
                        0x24 => -ui.max_scroll,
                        0x23 => ui.max_scroll,
                        _ => return false,
                    };
                    ui.scroll_by(d);
                    place_edits(ui, hwnd);
                    true
                })
                .unwrap_or(false);
                if handled {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                // stlacene tlacidlo hlavicky: hover podla toho, ci je mys stale nad nim (mys je zachytena)
                let pressing = with_ui(|ui| {
                    ui.chrome.pressed.map(|b| {
                        let mut rc = RECT::default();
                        let _ = GetClientRect(hwnd, &mut rc);
                        let dpi = GetDpiForWindow(hwnd).max(96);
                        let over = chrome::zone(x, y, rc.right, dpi, ui.chrome.maximized)
                            == chrome::Zone::Btn(b);
                        ui.chrome.hover = over.then_some(b);
                    })
                })
                .flatten()
                .is_some();
                if pressing {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                }
                // mys nad klientskou oblastou: tlacidla hlavicky uz nie su pod mysou
                if with_ui(|ui| ui.chrome.hover.take().is_some()).unwrap_or(false) {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                let redraw = with_ui(|ui| {
                    if let Some((id, sx, sy)) = ui.press.clone() {
                        if ui.drag.is_none() && ((x - sx).abs() > 4 || (y - sy).abs() > 4) {
                            ui.drag = Some((id, x, y));
                        }
                    }
                    if ui.drag_opacity {
                        ui.cfg.appearance.bg_opacity = appearance_page::opacity_at(ui.track, x);
                        return true;
                    }
                    if ui.drag_sound {
                        ui.cfg.ui_sounds.volume = sound_page::volume_at(ui.sound_track, x);
                        return true;
                    }
                    if let Some(d) = ui.drag.as_mut() {
                        d.1 = x;
                        d.2 = y;
                        return true;
                    }
                    let h = hit_at(ui, x, y);
                    let changed = h != ui.hover;
                    ui.hover = h;
                    changed
                })
                .unwrap_or(false);
                if redraw {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                with_ui(|ui| {
                    ui.finish_move(true);
                    ui.kfocus = None;
                });
                let (x, y) = xy(lp);
                with_ui(|ui| {
                    ui.chrome.focus = None;
                    match hit_at(ui, x, y) {
                        Some(Hit::Chip(id)) => ui.press = Some((id, x, y)),
                        Some(Hit::OpacityTrack) => {
                            // posuvnik: nahlad hned, ulozenie az pri uvolneni mysi
                            ui.drag_opacity = true;
                            ui.cfg.appearance.bg_opacity = appearance_page::opacity_at(ui.track, x);
                        }
                        Some(Hit::SoundTrack) => {
                            ui.drag_sound = true;
                            ui.cfg.ui_sounds.volume = sound_page::volume_at(ui.sound_track, x);
                        }
                        _ => {}
                    }
                });
                // klik mimo vstupneho pola odober fokus (kurzor v poli zmizne)
                let _ = SetFocus(Some(hwnd));
                let _ = SetCapture(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                // stlacene tlacidlo hlavicky sa odoberie pred `ReleaseCapture` (ten posle `WM_CAPTURECHANGED`)
                let pressed_btn = with_ui(|ui| ui.chrome.pressed.take()).flatten();
                let _ = ReleaseCapture();
                // uvolnenie tlacidla hlavicky: vykona sa, ak je mys stale nad nim
                if let Some(b) = pressed_btn {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    let max = windows::Win32::UI::WindowsAndMessaging::IsZoomed(hwnd).as_bool();
                    if chrome::zone(x, y, rc.right, dpi, max) == chrome::Zone::Btn(b) {
                        press_button(hwnd, b);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    return LRESULT(0);
                }
                let lang_before = transom_i18n::lang();
                with_ui(|ui| {
                    let drag = ui.drag.take();
                    ui.press = None;
                    if std::mem::take(&mut ui.drag_opacity) {
                        // koniec tahania posuvnika: az teraz ulozit a rozoslat
                        ui.save();
                    } else if std::mem::take(&mut ui.drag_sound) {
                        // koniec tahania hlasitosti zvukov: ulozit raz a prehrat jednu ukazku (nie pri kazdom pohybe)
                        ui.save();
                        ui.play_preview();
                    } else if let Some((id, _, _)) = drag {
                        if let Some(tg) =
                            drop_target(&ui.cols, &id, x, y).filter(|t| ui.center_ok(&id, t.0))
                        {
                            apply_drop(&mut ui.cfg.layout, &id, tg);
                            ui.save();
                        }
                    } else if let Some(h) = hit_at(ui, x, y) {
                        ui.click(h);
                    }
                });
                after_click(hwnd, lang_before);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_GLIDE => {
                GLIDE.with(|g| {
                    if let Some(p) = g.borrow().as_ref() {
                        p.ack();
                    }
                });
                let moving = with_ui(|ui| {
                    let mut s = ui.scroll;
                    let m = ui.glide.step(&mut s);
                    ui.scroll = s.clamp(0, ui.max_scroll);
                    ui.gliding = m;
                    place_edits(ui, hwnd);
                    m
                })
                .unwrap_or(false);
                let _ = InvalidateRect(Some(hwnd), None, false);
                if !moving {
                    GLIDE.with(|g| g.borrow_mut().take());
                }
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_COMMIT => {
                let _ = KillTimer(Some(hwnd), TIMER_COMMIT);
                with_ui(|ui| ui.flush_pending());
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == TIMER_WX {
                    let _ = KillTimer(Some(hwnd), TIMER_WX);
                    start_wx_search(hwnd);
                }
                // nahlad hodin (1 s) aj nahlad posuvneho textu (~30 snimok/s) len prekresluju okno
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_APP_WX => {
                let r = WX_RESULT.lock().ok().and_then(|mut g| g.take());
                if let Some((gen, res)) = r {
                    with_ui(|ui| ui.wx_results_arrived(gen, res));
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_APP_GEO => {
                with_ui(|ui| {
                    ui.wx.geo = match wp.0 {
                        0 => weather_page::GeoState::Allowed,
                        1 => weather_page::GeoState::Denied,
                        _ => weather_page::GeoState::Unavailable,
                    };
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_SIZE => {
                // SIZE_MAXIMIZED = 2, SIZE_RESTORED = 0 (zbalene okno stav nemeni)
                with_ui(|ui| {
                    match wp.0 {
                        2 => ui.chrome.maximized = true,
                        0 => ui.chrome.maximized = false,
                        _ => {}
                    }
                    place_edits(ui, hwnd)
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            // zmena textu vo vstupnom poli (HEX / R / G / B)
            WM_COMMAND => {
                let code = ((wp.0 >> 16) & 0xFFFF) as u32;
                let id = wp.0 & 0xFFFF;
                if (EDIT_ID0..EDIT_ID0 + 6).contains(&id) {
                    let i = id - EDIT_ID0;
                    if code == EN_CHANGE {
                        // nahlad hned; ulozenie az pri Enter / Tab / strate fokusu (Esc hodnotu vrati a neulozi nic)
                        with_ui(|ui| ui.on_edit_change(i));
                        // pole hladania miesta: hladat az po kratkej pauze v pisani
                        if with_ui(|ui| std::mem::take(&mut ui.wx.restart)).unwrap_or(false) {
                            SetTimer(Some(hwnd), TIMER_WX, 350, None);
                        }
                    } else if code == EN_SETFOCUS_CODE {
                        with_ui(|ui| ui.edit_focus(i));
                    } else if code == EN_KILLFOCUS_CODE {
                        with_ui(|ui| {
                            if ui.edit_blur(i) {
                                ui.save();
                            }
                        });
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            // tmave pozadie a svetly text vo vstupnych poliach (podla temy okna)
            WM_CTLCOLOREDIT => {
                let hdc = HDC(wp.0 as *mut _);
                // farby pola: pole sa moze kreslit aj vo chvili, ked je stav okna pozicany (presun pola) - vtedy
                // posledne zname farby (inak by malo predvolene biele pozadie)
                let brush = with_ui(|ui| {
                    let th = ui.theme;
                    SetTextColor(
                        hdc,
                        windows::Win32::Foundation::COLORREF(
                            th.foreground.r as u32
                                | (th.foreground.g as u32) << 8
                                | (th.foreground.b as u32) << 16,
                        ),
                    );
                    SetBkColor(
                        hdc,
                        windows::Win32::Foundation::COLORREF(
                            th.placeholder.r as u32
                                | (th.placeholder.g as u32) << 8
                                | (th.placeholder.b as u32) << 16,
                        ),
                    );
                    if ui.edit_brush == 0 {
                        ui.edit_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(
                            th.placeholder.r as u32
                                | (th.placeholder.g as u32) << 8
                                | (th.placeholder.b as u32) << 16,
                        ))
                        .0 as isize;
                    }
                    let rgb = |c: Color| c.r as u32 | (c.g as u32) << 8 | (c.b as u32) << 16;
                    EDIT_COLORS
                        .with(|e| e.set((rgb(th.foreground), rgb(th.placeholder), ui.edit_brush)));
                    ui.edit_brush
                });
                let brush = brush.or_else(|| {
                    let (fg, bg, b) = EDIT_COLORS.with(|e| e.get());
                    (b != 0).then(|| {
                        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(fg));
                        SetBkColor(hdc, windows::Win32::Foundation::COLORREF(bg));
                        b
                    })
                });
                match brush {
                    Some(b) => LRESULT(b),
                    None => DefWindowProcW(hwnd, msg, wp, lp),
                }
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                with_ui(|ui| ui.edit_dpi = 0);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            // zmena zobrazeni (pripojenie / odpojenie, rozlisenie): obnovit zoznam monitorov na stranke Displays
            0x007E => {
                with_ui(|ui| ui.refresh_monitors());
                let _ = InvalidateRect(Some(hwnd), None, false);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            // konfiguraciu zmenil iny proces (volba Zostava / Spotrebovane v paneli AI v lište): prevezme sa **len** tato
            // hodnota, aby ju nasledne ulozenie z tohto okna ticho nevratilo
            // lista nasla (alebo uz nema) aktualizaciu: stranka Aktualizacie ukaze / skryje instalaciu
            m if m == update_state_message() => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            m if m == config_message() => {
                let loaded = transom_config::load(&transom_config::default_path());
                let changed = with_ui(|ui| {
                    let differs = !loaded.read_only
                        && ui.cfg.usage.ring_mode != loaded.config.usage.ring_mode;
                    if differs {
                        ui.cfg.usage.ring_mode = loaded.config.usage.ring_mode.clone();
                        ui.base.usage.ring_mode = loaded.config.usage.ring_mode.clone();
                    }
                    differs
                })
                .unwrap_or(false);
                if changed {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            // druha instancia (`--page`): prepni stranku (wParam = poradie v `PAGE_ORDER`)
            m if m == page_message() => {
                with_ui(|ui| {
                    if let Some((pg, _)) = PAGE_ORDER.get(wp.0) {
                        ui.page = *pg;
                        ui.scroll = 0;
                        ui.transfer = transfer_page::Transfer::Idle;
                        if *pg == Page::Displays {
                            ui.refresh_monitors();
                        }
                        if *pg == Page::Ai {
                            ui.cc.refresh();
                            ui.refresh_archive();
                        }
                        place_edits(ui, hwnd);
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_DESTROY => {
                // rozpracovany presun sa zrusi (polovicaty sa neuklada); hodnota rozpisaneho pola a cakajuca zmena
                // posuvnika sa pri zatvoreni okna neztratia
                with_ui(|ui| {
                    ui.finish_move(false);
                    if ui.edit_session.take().is_some_and(|s| s.blur()) {
                        ui.save();
                    }
                    ui.flush_pending();
                });
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Zakladny stav okna (z nacitanej konfiguracie); pouziva aj offscreen rezim `--shot`.
fn make_ui(loaded: transom_config::Loaded, theme: Theme, windows_lang: String) -> Ui {
    Ui {
        all_hits: Vec::new(),
        kfocus: None,
        acc: Vec::new(),
        acc_group: None,
        acc_targets: Vec::new(),
        last_title_focus: None,
        transfer: transfer_page::Transfer::Idle,
        pending_xfer: None,
        transfer_busy: false,
        acc_statics: Vec::new(),
        edit_session: None,
        moving: None,
        reveal: None,
        base: loaded.config.clone(),
        cfg: loaded.config,
        read_only: loaded.read_only,
        note: loaded.issue.map(|i| match i {
            transom_config::Issue::Corrupt => t("cfg.corrupt"),
            transom_config::Issue::Newer(v) => tf("cfg.newer", &[&v.to_string()]),
            transom_config::Issue::Unreadable => t("cfg.unreadable"),
        }),
        page: Page::Layout,
        expanded: [true, false, false, false],
        nav_seen: None,
        nav_last: None,
        hover: None,
        hits: Vec::new(),
        cols: Vec::new(),
        drag: None,
        press: None,
        status: tf(
            "status.file",
            &[&transom_config::default_path().display().to_string()],
        ),
        theme,
        confirm_delete: false,
        cc: cc::Cc::default(),
        arc: ArchiveUi::default(),
        chrome: ChromeState::default(),
        windows_lang,
        edits: [0; 6],
        wx: weather_page::WxUi::default(),
        wx_rect: Rect::new(0, 0, 0, 0),
        wx_scroll: 0,
        wx_placed: Rect::new(0, 0, 0, 0),
        edit_font: 0,
        edit_brush: 0,
        edit_dpi: 0,
        syncing: false,
        bad: [false; 4],
        drag_opacity: false,
        track: Rect::default(),
        drag_sound: false,
        sound_track: Rect::default(),
        sound: transom_sound::Player::silent(),
        sound_preview_pending: false,
        tools_confirm: None,
        pending_folder: None,
        pending_app: false,
        apps_msg: None,
        hotkey_capture: false,
        hotkey_msg: None,
        hotkey_swallow: false,
        pending_pick: false,
        autostart: transom_platform::autostart::enabled(),
        pending_close: false,
        pending_text_pick: false,
        extras_msg: None,
        commit_pending: false,
        anim_start: std::time::Instant::now(),
        scroll: 0,
        max_scroll: 0,
        glide: Default::default(),
        gliding: false,
        viewport: Rect::new(0, 0, 0, 0),
        repaint: false,
        art: None,
        monitors: Vec::new(),
        form: DateForm::default(),
        date_rect: Rect::new(0, 0, 0, 0),
        date_scroll: 0,
        date_placed: Rect::new(0, 0, 0, 0),
        cards: Vec::new(),
        card_top: None,
        in_choice: false,
        btn_left: false,
    }
}

/// Velkost okna (px) zmensena tak, aby sa zmestila do pracovnej plochy monitora, na ktorom okno je
/// (pri vysokom DPI / nizkom rozliseni by pevne rozmery presahovali obrazovku; obsah sa potom posuva).
fn fit_to_work_area(hwnd: HWND, w: i32, h: i32) -> (i32, i32) {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    // SAFETY: MONITORINFO s nastavenym cbSize je lokalna.
    unsafe {
        let m = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(m, &mut mi).as_bool() {
            let aw = mi.rcWork.right - mi.rcWork.left;
            let ah = mi.rcWork.bottom - mi.rcWork.top;
            return (w.min(aw).max(400), h.min(ah).max(300));
        }
    }
    (w, h)
}

/// Id registrovanej spravy "prepni stranku" (`transom_config::SETTINGS_PAGE_MESSAGE`).
fn page_message() -> u32 {
    thread_local! { static MSG: u32 = {
        let name: Vec<u16> = transom_config::SETTINGS_PAGE_MESSAGE.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: retazec ukonceny nulou zije pocas volania.
        unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
    }; }
    MSG.with(|m| *m)
}

/// Id registrovanej spravy "zmenila sa dostupna aktualizacia" (`transom_config::UPDATE_STATE_MESSAGE`).
fn update_state_message() -> u32 {
    thread_local! { static MSG: u32 = {
        let name: Vec<u16> = transom_config::UPDATE_STATE_MESSAGE.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: retazec ukonceny nulou zije pocas volania.
        unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
    }; }
    MSG.with(|m| *m)
}

/// Id registrovanej spravy "konfiguracia sa zmenila" (`transom_config::CHANGED_MESSAGE`; posiela ju aj panel AI v lište).
fn config_message() -> u32 {
    thread_local! { static MSG: u32 = {
        let name: Vec<u16> = transom_config::CHANGED_MESSAGE.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: retazec ukonceny nulou zije pocas volania.
        unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) }
    }; }
    MSG.with(|m| *m)
}

/// Poradie stranok a ich nazvy pre `--page` (rovnake nazvy ako `--shot --page`); index sa posiela druhej instancii.
const PAGE_ORDER: [(Page, &str); 21] = [
    (Page::Layout, "layout"),
    (Page::Displays, "displays"),
    (Page::Media, "media"),
    (Page::Appearance, "appearance"),
    (Page::Clock, "clock"),
    (Page::Ai, "ai"),
    (Page::Sound, "sound"),
    (Page::Audio, "audio"),
    (Page::Peripherals, "peripherals"),
    (Page::Tools, "tools"),
    (Page::Extras, "extras"),
    (Page::Language, "language"),
    (Page::Transfer, "transfer"),
    // podstranky nastrojov (na konci: poradie je index pre druhu instanciu)
    (Page::Clipboard, "clipboard"),
    (Page::Notes, "notes"),
    (Page::Shots, "shots"),
    (Page::Launcher, "launcher"),
    (Page::Notch, "notch"),
    (Page::ToolGroup, "toolgroup"),
    (Page::Updates, "updates"),
    (Page::Welcome, "welcome"),
];

/// Predvolby hodin: (kluc, den v tyzdni, cely nazov dna, sekundy, datum).
const CLOCK_PRESETS: [(&str, bool, bool, bool, DateMode); 6] = [
    ("preset.time", false, false, false, DateMode::None),
    ("preset.seconds", false, false, true, DateMode::None),
    ("preset.weekday", true, false, false, DateMode::None),
    ("preset.date", true, false, false, DateMode::Short),
    ("preset.long_date", false, false, false, DateMode::Long),
    // "streda 25.11.2026 22:14"
    ("preset.day_date", true, true, false, DateMode::Dmy),
];

/// Ukazka vlastneho formatu datumu na dnesnom dni (napr. "25.11.2026").
fn date_sample(m: DateMode) -> String {
    let d = transom_platform::clock::today();
    let (y, mo, dd) = (d.y, d.m, d.d);
    match m {
        DateMode::Dmy => format!("{dd:02}.{mo:02}.{y}"),
        DateMode::DmySlash => format!("{dd:02}/{mo:02}/{y}"),
        DateMode::Mdy => format!("{mo:02}/{dd:02}/{y}"),
        DateMode::Iso => format!("{y}-{mo:02}-{dd:02}"),
        _ => String::new(),
    }
}

/// Stranka z `--page <nazov>`.
fn page_arg(args: &[String]) -> Option<Page> {
    let i = args.iter().position(|a| a == "--page")?;
    let name = args.get(i + 1)?;
    PAGE_ORDER.iter().find(|(_, n)| n == name).map(|(p, _)| *p)
}

/// Na chvilu (2 s) ukaze na kazdom monitore cislo a nazov (rovnake cislo ako v zozname stranky Displays). Okna su
/// bez fokusu, nad vsetkym a nechytaju klik; samy zaniknu.
fn identify_displays(monitors: &[transom_platform::monitor::MonitorInfo]) {
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
        WS_POPUP,
    };
    // SAFETY: registracia triedy a vytvorenie okna na vlakne UI; retazce ziju pocas volani.
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return;
        };
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        REGISTERED.call_once(|| {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(identify_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("TransomIdentify"),
                ..Default::default()
            });
        });
        for (n, (_, m)) in displays_page::ordered(monitors).into_iter().enumerate() {
            let scale = m.dpi as f32 / 96.0;
            let (w, h) = ((300.0 * scale) as i32, (150.0 * scale) as i32);
            let x = m.rect.left + (m.rect.width() - w) / 2;
            let y = m.rect.top + (m.rect.height() - h) / 2;
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                w!("TransomIdentify"),
                w!(""),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(hinst.into()),
                None,
            ) else {
                continue;
            };
            IDENT.with(|c| {
                c.borrow_mut().insert(
                    hwnd.0 as isize,
                    ((n + 1).to_string(), displays_page::describe(m), m.dpi),
                )
            });
            let _ = ShowWindow(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
            );
            SetTimer(Some(hwnd), 1, 2200, None);
        }
    }
}

thread_local! {
    /// Texty identifikacnych okien: hwnd -> (cislo, popis, dpi).
    static IDENT: RefCell<std::collections::HashMap<isize, (String, String, u32)>> =
        RefCell::new(std::collections::HashMap::new());
}

unsafe extern "system" fn identify_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: wndproc okna identifikacie monitora: parametre su cisla; BeginPaint/EndPaint v tej istej vetve, kreslenie cez GdiPainter.
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let (w, h) = (rc.right.max(1), rc.bottom.max(1));
                let info = IDENT.with(|c| c.borrow().get(&(hwnd.0 as isize)).cloned());
                if let Some((num, text, dpi)) = info {
                    let mut p = GdiPainter::new(hdc, Size { w, h }, dpi);
                    let th = Theme::dark();
                    let s = p.scale();
                    p.fill_rect(Rect::new(0, 0, w, h), th.accent);
                    let b = (3.0 * s).round() as i32;
                    p.fill_rect(Rect::new(b, b, w - 2 * b, h - 2 * b), th.background);
                    p.draw_text(
                        &num,
                        Rect::new(0, (10.0 * s) as i32, w, (80.0 * s) as i32),
                        &style(56.0, true, th.accent_text, HAlign::Center),
                    );
                    let lines = transom_render::painter::wrap_text(
                        &mut p,
                        &text,
                        &style(13.0, false, th.foreground, HAlign::Center),
                        w - (24.0 * s) as i32,
                    );
                    for (k, l) in lines.iter().take(2).enumerate() {
                        p.draw_text(
                            l,
                            Rect::new(
                                (12.0 * s) as i32,
                                ((92.0 + 20.0 * k as f32) * s) as i32,
                                w - (24.0 * s) as i32,
                                (20.0 * s) as i32,
                            ),
                            &style(13.0, false, th.foreground, HAlign::Center),
                        );
                    }
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER => {
                IDENT.with(|c| c.borrow_mut().remove(&(hwnd.0 as isize)));
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Text skrateny na sirku `w` (px) so znakom "…" na konci; zmesti sa bez zmeny, vrati sa cely.
fn fit_text(p: &mut dyn Painter, text: &str, st: &TextStyle, w: i32) -> String {
    if p.measure_text(text, st).w <= w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cand: String = chars[..mid]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
            + "…";
        if p.measure_text(&cand, st).w <= w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_string()
        + "…"
}

/// Tema okna nastaveni podla systemu. So sklom (`glass`: volba *Cierne sklo pod panelmi*, tmava tema, povolene
/// efekty priehladnosti) plne cierne pozadie a plochy v style skla (jemne svetle karty a ovladace so svetlym
/// okrajom) - okno samo priehladne nie je, aby sa nastavenia dobre citali nad akymkolvek obsahom.
pub(crate) fn window_theme(glass: bool) -> Theme {
    themed(
        transom_platform::system::system_uses_dark(),
        glass && transom_platform::glass::system_allows(),
    )
}

/// Tema okna pre `dark` a volbu skla (bez dotazu na system; pouziva aj offscreen `--shot`).
pub(crate) fn themed(dark: bool, glass: bool) -> Theme {
    let th = ThemeMode::System.resolve(dark);
    if dark && glass {
        th.glass_pass(transom_render::Color::hex(0x0E0E10))
    } else {
        th
    }
}

fn main() {
    transom_platform::init_dpi_awareness();
    // offscreen kontrola vzhladu (bez okna, bez jednej instancie): pozri `shot`
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--shot") {
        // deterministicky: vzdy predvolena konfiguracia (nikdy skutocny subor pouzivatela)
        let loaded = transom_config::Loaded {
            config: transom_config::Config::default(),
            read_only: false,
            note: None,
            issue: None,
        };
        let theme = ThemeMode::System.resolve(transom_platform::system::system_uses_dark());
        std::process::exit(shot::run(&args, make_ui(loaded, theme, String::new())));
    }
    // jedna instancia: druhe spustenie len prinesie existujuce okno do popredia
    // `TRANSOM_TEST_INSTANCE` (len test): oddeli jedinu instanciu, aby test nerusil okno nastaveni pouzivatela
    let mutex_name: Vec<u16> = format!(
        "Local\\transom-settings-single-instance{}",
        std::env::var("TRANSOM_TEST_INSTANCE").unwrap_or_default()
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect();
    // SAFETY: nazov mutexu je retazec ukonceny nulou; handle sa zatvori na konci `main` (pri skorom navrate ho zatvori koniec procesu).
    let mutex = unsafe { CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr())) };
    let Ok(mutex) = mutex else { return };
    // SAFETY: GetLastError hned po CreateMutexW na tom istom vlakne.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // uz bezi: najst okno (okno moze vznikat - kratke opakovanie), obnovit ho ak je zbalene, prepnut stranku (`--page`)
        // a dat do popredia
        let page_idx = page_arg(&args).and_then(|p| PAGE_ORDER.iter().position(|(pg, _)| *pg == p));
        // SAFETY: hladanie a aktivacia nasho okna; sprava nesie len cisla.
        unsafe {
            for _ in 0..10 {
                if let Ok(h) = FindWindowW(w!("TransomSettings"), PCWSTR::null()) {
                    if windows::Win32::UI::WindowsAndMessaging::IsIconic(h).as_bool() {
                        let _ = ShowWindow(h, windows::Win32::UI::WindowsAndMessaging::SW_RESTORE);
                    }
                    if let Some(i) = page_idx {
                        transom_platform::post_broadcast(transom_config::SETTINGS_PAGE_MESSAGE, i);
                    }
                    let _ = SetForegroundWindow(h);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        return;
    }
    let loaded = transom_config::load(&transom_config::default_path());
    let theme = window_theme(loaded.config.appearance.glass_panels);
    let windows_lang = pclock::ui_language_tag();
    apply_lang(&loaded.config, &windows_lang);
    UI.with(|c| {
        let mut ui = make_ui(loaded, theme, windows_lang);
        // ukazka zvuku: skutocne zariadenie sa otvori az pri prvej ukazke (a nikdy v testovacej instancii)
        ui.sound = transom_sound::start_system(transom_sound::StartOptions {
            automation: std::env::var_os("TRANSOM_TEST_INSTANCE").is_some(),
            log: |_| {},
        });
        if let Some(p) = page_arg(&args) {
            ui.page = p;
        }
        ui.refresh_monitors();
        if ui.page == Page::Ai {
            ui.cc.refresh();
            ui.refresh_archive();
        }
        *c.borrow_mut() = Some(ui);
    });
    // SAFETY: registracia triedy a vytvorenie hlavneho okna na vlakne UI; retazce ziju pocas volani.
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return;
        };
        let cur = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("TransomSettings"),
            hCursor: cur,
            ..Default::default()
        });
        let Ok(hwnd) = CreateWindowExW(
            Default::default(),
            w!("TransomSettings"),
            w!("transom"),
            WS_OVERLAPPEDWINDOW | windows::Win32::UI::WindowsAndMessaging::WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIN_W_LP,
            WIN_H_LP,
            None,
            None,
            Some(hinst.into()),
            None,
        ) else {
            return;
        };
        set_title(hwnd);
        set_window_icons(hwnd);
        if let Some(th) = with_ui(|ui| ui.theme) {
            apply_dwm(hwnd, &th);
        }
        // natívne vstupne polia pre HEX a RGB (zobrazuju sa len na stranke Pozadie)
        for i in 0..6usize {
            // bez vlastneho ramika: ramik a podklad kresli stranka (rovnaky vzhlad pocas posuvania aj po nom)
            let e = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_TABSTOP | WINDOW_STYLE(0x80), // ES_AUTOHSCROLL
                0,
                0,
                10,
                10,
                Some(hwnd),
                Some(HMENU((EDIT_ID0 + i) as *mut _)),
                Some(hinst.into()),
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                e,
                EM_SETLIMITTEXT,
                Some(WPARAM(match i {
                    0 => 7,
                    4 => 10,
                    5 => 80,
                    _ => 3,
                })),
                Some(LPARAM(0)),
            );
            with_ui(|ui| ui.edits[i] = e.0 as isize);
        }
        with_ui(|ui| {
            ui.fill_edits(None);
            place_edits(ui, hwnd);
            name_edits(ui);
        });
        // velkost podla DPI monitora okna
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        let (ww, wh) = fit_to_work_area(hwnd, WIN_W_LP * dpi / 96, WIN_H_LP * dpi / 96);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            ww,
            wh,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | SWP_NOZORDER
                | windows::Win32::UI::WindowsAndMessaging::SWP_FRAMECHANGED,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // klavesy hlavicky (Tab, sipky, Enter, Esc) sa spracuju skor nez ich odchyti vstupne pole
            let in_edit = msg.hwnd != hwnd
                && windows::Win32::UI::WindowsAndMessaging::GetParent(msg.hwnd).ok() == Some(hwnd);
            let ours = msg.hwnd == hwnd || in_edit;
            // zachytavanie skratky vyhladavaca: klavesy (aj s Alt = WM_SYSKEYDOWN) idu do zachytavania, nie do okna; systemove
            // spravy Alt (WM_SYSKEYUP, WM_SYSCHAR) sa prehltnu, aby Alt+medzernik neotvoril systemovu ponuku okna
            if msg.hwnd == hwnd && hotkey_capture_msg(hwnd, msg.message, msg.wParam.0 as u32) {
                continue;
            }
            if msg.message == WM_KEYDOWN
                && ours
                && chrome_key(
                    hwnd,
                    in_edit.then_some(msg.hwnd),
                    msg.wParam.0 as u32,
                    (msg.lParam.0 >> 30) & 1 == 1,
                )
            {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = CloseHandle(mutex);
    }
}
