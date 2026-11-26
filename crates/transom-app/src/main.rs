//! transom - binar listy: AppBar hostitel + renderer R0 (GDI) + stavove polozky (Search, media, Tray, bateria, hodiny).
//!
//! Prekresluje sa len pri udalostiach: start, zmena polohy/DPI, zmena temy, zmena minuty, zmena napajania, zmena
//! zobrazenych medii, interakcia mysou (zmena hover casti, klik) a vyprsanie docasneho oznamu. Ziadna slucka
//! prekreslovania ani dotazovania: media chodia ako udalosti z GSMTC vlakna, bateria cez power notifikacie.
#![windows_subsystem = "windows"]

mod ai;
mod ai_cards;
mod ai_month;
mod audio_pop;
#[cfg(test)]
mod audio_tests;
mod bar_edit;
#[cfg(test)]
mod bar_stress_tests;
mod caldata;
mod clip_notch;
mod extras;
#[cfg(test)]
mod extras_tests;
#[cfg(test)]
mod gallery;
#[cfg(test)]
mod glass_tests;
mod icon_rt;
mod info_notch;
mod island;
mod island_menu;
mod island_rt;
mod launcher_pop;
mod launcher_rt;
mod library_rt;
mod live_tools;
mod media_notch;
mod menu;
#[cfg(test)]
mod mixer_tests;
mod periph_pop;
#[cfg(test)]
mod periph_tests;
mod pop_kit;
mod smooth;
mod stash_notch;
mod tiles;
mod timer;
mod tools_bridge;
mod tools_pop;
#[cfg(test)]
mod tools_tests;
mod weather;

/// Testy citaju globalny jazyk (`transom_i18n::set_lang`): jazykovo zavisle testy bezia po jednom a vzdy od anglictiny.
#[cfg(test)]
pub(crate) mod test_lang {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub fn guard() -> std::sync::MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        transom_i18n::set_lang(transom_i18n::Lang::En);
        g
    }
}
mod cal;
mod calsync;
mod ticker;
mod update;
mod update_pop;
mod view;

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use transom_audio as audio;
use transom_core::battery::{Battery, PowerRaw};
use transom_core::clock::ClockOptions;
use transom_core::item::{Action, ArtImage, ItemId, Part, StatusItem};
use transom_core::layout::{hit_test, Placed};
use transom_core::marquee::Policy;
use transom_core::media::{
    select, spotify_hint_needed, toggle_command, Controls, PlayState, Selection, SessionInfo,
    ToggleCmd,
};
use transom_i18n::{t, tf};
use transom_media::{Command, MediaService, Snapshot};
use transom_peripherals as periph;
use transom_platform::clock::{self as pclock, format_clock, next_wake_delay_for};
use transom_platform::input::{self, Blocked, Foreground, Shortcut};
use transom_platform::popup::{self, PopoverPaint, Request};
use transom_platform::regions::{self, Region};
use transom_platform::system::system_uses_dark;
use transom_platform::{
    invalidate_all, notifier, power, process, register_broadcast, schedule_timer, schedule_wake,
    BarAppearance, BarConfig, BarEvent, BarHandler, BarHost, MonitorSelection, PaintCtx,
    PointerKind,
};
use transom_render::{
    render_bar_with, Color, GdiPainter, Interaction, MarqueeDraw, Size, Theme, ThemeMode,
};
use transom_sound as snd;

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

static LOG: Mutex<Option<(std::fs::File, Instant)>> = Mutex::new(None);
/// Pocet dokoncenych kreslení listy (prvy prechod; len na diagnostiku/meranie, zapisuje sa do logu pri ukonceni).
static PAINTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn log(msg: &str) {
    if let Ok(mut g) = LOG.lock() {
        if let Some((f, t0)) = g.as_mut() {
            let _ = writeln!(f, "[{:8.2}s] {msg}", t0.elapsed().as_secs_f32());
        }
    }
}

/// ID casovacov hostitela (`schedule_timer`).
const T_VERIFY: u32 = 1;
const T_NOTICE: u32 = 2;
const T_ACTIVATE: u32 = 3;
const T_USAGE: u32 = 4;
/// Posuvny text: dalsi snimok (planuje sa len ak sa nieco hybe).
const T_MARQUEE: u32 = 5;
/// Test `--test-menu`: kroky scenara (otvorenie menu, klavesy, vyber).
const T_TESTMENU: u32 = 9;
/// Test `--test-ai`: otvori panel AI, prepne Zostava/Spotrebovane a mesacny pohlad (spravy priamo oknu popoveru).
const T_TESTAI: u32 = 10;
/// Test `--test-tools`: kroky zivého scenara a "tep" na meranie blokovania UI vlakna.
const T_TESTTOOLS: u32 = 11;
const T_TESTBEAT: u32 = 12;
/// Citanie rozlozenia klavesnice a Caps Lock (len kym je polozka Klavesnica viditelna).
const T_KEYS: u32 = 13;
const KEYS_POLL_MS: u32 = 250;
/// Tik casovaca a stopiek (kym nieco bezi).
const T_TIMER: u32 = 14;
/// Kratka odlozka pred rozbalenim vybezku odlozenych suborov (prejdenie mysou cez listu ho neotvori).
const T_STASH: u32 = 15;
/// Koniec kratkeho oznamenia vo vybezku (hlasitost, casovac, udalost).
const T_INFO: u32 = 16;
/// Cakanie na snimku z vyrezu obrazovky (schranka sa kontroluje kazdych 400 ms, najviac 90 s).
const T_SNIP: u32 = 17;
/// Obnova aktivneho okna a virtualnych ploch (kazdu sekundu, len ked su polozky v liste).
const T_WATCH: u32 = 18;
/// Ostrov v strede listy: koniec oznamenia / nazvu skladby a snimky ekvalizera.
const T_ISLAND: u32 = 19;
/// Snimky plynuleho posuvania panelov (s obnovou obrazovky).
const T_GLIDE: u32 = 20;
/// Snimky ekvalizera v ostrove (s obnovou obrazovky, len kym hra hudba).
const T_EQ: u32 = 21;
/// Cas skladby v otvorenom prehravaci (0,5 s).
const T_MEDIA: u32 = 22;
/// Test `--test-update`: kroky scenara simulovanej aktualizacie.
const T_TESTUPD: u32 = 23;
/// Kod notifikacie z media vlakna.
const N_MEDIA: u32 = 1;
/// Prebudenie presneho casovaca snimok posuvneho textu (`transom_platform::FrameTimer`).
const N_FRAME: u32 = 3;
/// Zmena stavu systemoveho vystupu zvuku (notifikacia Core Audio -> prekreslit ikonu a panel).
const N_AUDIO: u32 = 4;
/// Nova snimka sluzby nastrojov (Polica, schranka, poznamky).
const N_TOOLS: u32 = 5;
/// Nove vysledky vyhladavaca z pracovneho vlakna.
const N_LAUNCHER: u32 = 6;
/// Zoznam aplikacii vyhladavaca sa dokoncil nacitat (obnovi sa aktualny dotaz).
const N_LAUNCHER_APPS: u32 = 7;
/// Kniznica snimok: novy zoznam alebo hotova miniatura (prekreslit panel).
const N_LIB: u32 = 8;
/// Dialog "Pridat subory do Police" skoncil (vybrane cesty alebo zrusenie).
const N_PICKED: u32 = 9;
/// Nacitala sa ikona vysledku vyhladavaca (prekreslit okno vyhladavaca).
const N_ICONS: u32 = 10;
/// Nova snimka baterii periferii (prekreslit listu a panel).
const N_PERIPH: u32 = 11;
/// Nova snimka mixera aplikacii (prekreslit panel Zvuk).
const N_MIXER: u32 = 12;
/// Nova snimka jasu monitorov (prekreslit listu a panel).
const N_BRIGHT: u32 = 13;
/// Nova snimka Bluetooth zvukovych zariadeni (panel Zvuk).
const N_BT: u32 = 16;
/// Nacital sa pripojeny kalendar (ICS).
const N_ICS: u32 = 17;
/// Nove meranie vytazenia systemu.
const N_SYSMON: u32 = 14;
/// Nova snimka mikrofonu a kamery.
const N_PRIV: u32 = 15;
/// Nova predpoved pocasia.
const N_WEATHER: u32 = 18;
/// Zmena v priecinku snimok obrazovky (ostrov).
const N_SHOTS: u32 = 19;
/// Zmena v priecinku mostu statusline (ostrov: pracuje AI).
const N_BRIDGE: u32 = 20;
/// Zachytavanie zvuku ma nove urovne (ekvalizer v ostrove).
const N_EQ: u32 = 21;
/// Vysledok kontroly alebo instalacie aktualizacie.
const N_UPDATE: u32 = 22;
/// Druhy popoveru.
const POP_CALENDAR: u32 = 1;
const POP_AI: u32 = 2;
/// Vlastne kontextove menu lisaty (`menu.rs`).
const POP_MENU: u32 = 3;
const POP_AUDIO: u32 = 4;
const POP_TOOLS: u32 = 5;
const POP_LAUNCHER: u32 = 6;
/// Id globalnej skratky vyhladavaca (`RegisterHotKey`).
const HOTKEY_LAUNCHER: i32 = 1;
/// Id globalnej skratky stlmenia mikrofonu.
const HOTKEY_MIC: i32 = 2;
/// Posun mysi (px) so stlacenym tlacidlom na subore Police, od ktoreho zacne tahanie von.
const DRAG_PX: i32 = 6;
const POP_PERIPH: u32 = 7;
/// Panely zo sady `pop_kit` (`extras.rs`).
const POP_TIMER: u32 = 8;
const POP_BRIGHT: u32 = 9;
const POP_PRIVACY: u32 = 10;
/// Panel upravy lisaty (zoznam poloziek, oddelovace, Hotovo); kym je otvoreny, polozky sa tahaju po liste.
const POP_EDIT: u32 = 11;
/// Predpoved pocasia (klik na pocasie v liste alebo dlazdicu v Nastrojoch).
const POP_WEATHER: u32 = 12;
/// Panel dostupnej aktualizacie (ikona uplne vpravo, oznamenie, polozka menu).
const POP_UPDATE: u32 = 13;
/// Polozky kontextoveho menu.
const MENU_SETTINGS: u32 = 1;
const MENU_CUSTOMIZE: u32 = 3;
const MENU_AI: u32 = 6;
const MENU_TOOLS: u32 = 7;
const MENU_HIDE: u32 = 4;
const MENU_SHOW: u32 = 5;
const MENU_QUIT: u32 = 9;
const MENU_EDIT: u32 = 8;
const MENU_UPDATE: u32 = 10;

/// Dostupna novsia verzia (polozka menu "Aktualizovat na ..."); nastavuje ju lista po kontrole.
static UPDATE_MENU: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Volby hodin z konfiguracie.
fn clock_options(c: &transom_config::Clock) -> ClockOptions {
    use transom_config::{DateMode as D, HourMode as H, Tri};
    use transom_core::clock::{DateMode, HourMode, LeadingZero};
    ClockOptions {
        hour: match c.hour {
            H::Locale => HourMode::Locale,
            H::H12 => HourMode::H12,
            H::H24 => HourMode::H24,
        },
        leading_zero: match c.leading_zero {
            Tri::Locale => LeadingZero::Locale,
            Tri::Yes => LeadingZero::Yes,
            Tri::No => LeadingZero::No,
        },
        seconds: c.seconds,
        weekday: c.weekday,
        date: match c.date {
            D::None => DateMode::None,
            D::Short => DateMode::Short,
            D::Long => DateMode::Long,
            // vlastne formaty sklada `clock_text`
            D::Dmy | D::DmySlash | D::Mdy | D::Iso => DateMode::None,
        },
    }
}

/// Text hodin: cas podla Windows, den v tyzdni v jazyku transom (skratka alebo cely nazov) a datum v zvolenom formate.
fn clock_text(c: &transom_config::Clock) -> String {
    use transom_config::DateMode as D;
    let mut o = clock_options(c);
    o.weekday = false;
    o.date = transom_core::clock::DateMode::None;
    let time = format_clock(&o);
    let today = pclock::today();
    let wd = c.weekday.then(|| {
        let i = transom_core::clock::weekday(today);
        t(&format!("wd.{}{i}", if c.weekday_long { "l" } else { "s" }))
    });
    let (y, m, d) = (today.y, today.m, today.d);
    let date = match c.date {
        D::None => None,
        D::Short => Some(pclock::short_date(today)),
        D::Long => Some(pclock::long_date(today)),
        D::Dmy => Some(format!("{d:02}.{m:02}.{y}")),
        D::DmySlash => Some(format!("{d:02}/{m:02}/{y}")),
        D::Mdy => Some(format!("{m:02}/{d:02}/{y}")),
        D::Iso => Some(format!("{y}-{m:02}-{d:02}")),
    };
    // dlhy datum Windows moze uz obsahovat den - neopakovat
    let wd = wd.filter(|w| date.as_ref().is_none_or(|x| !x.contains(w.as_str())));
    transom_core::clock::compose(wd.as_deref(), date.as_deref(), &time)
}

fn first_day(c: &transom_config::Clock) -> u8 {
    use transom_config::FirstDay as F;
    use transom_core::clock::FirstDay;
    let f = match c.first_day {
        F::Locale => FirstDay::Locale,
        F::Monday => FirstDay::Monday,
        F::Sunday => FirstDay::Sunday,
        F::Saturday => FirstDay::Saturday,
    };
    f.resolve(pclock::locale_first_day())
}
/// Ako dlho po skratke overit jej ucinok. S4: Search sa otvoril za ~42 ms, fokus po Win+B za ~23 ms.
const VERIFY_SEARCH_MS: u32 = 600;
const VERIFY_TRAY_MS: u32 = 400;
const NOTICE_MS: u32 = 7000;

/// Skratka, ktorej ucinok sa este overuje.
#[derive(Debug)]
enum Pending {
    Search { before: Foreground },
    TrayFocus,
    TrayFlyout,
}

/// Trieda okna flyoutu skrytych ikon (S4, Windows 11 26300).
const OVERFLOW_CLASS: &str = "TopLevelWindowForOverflowXamlIsland";

fn is_taskbar(fg: &Foreground) -> bool {
    fg.class == "Shell_TrayWnd" || fg.class == "Shell_SecondaryTrayWnd"
}

/// Je prvok s fokusom tlacidlo "Show Hidden Icons"? Overene UI Automation na tomto stroji (Windows 11 26300, anglicke UI):
/// AutomationId `SystemTrayIcon` a trieda `SystemTray.NormalButton` su spolocne aj pre ine ikony (napr. indikator
/// vstupu), jediny rozlisujuci udaj je **lokalizovany** nazov. Enter sa preto posle len pri presnej zhode nazvu;
/// v inych jazykoch Tray tlacidlo skonci pri fokuse v oblasti oznameni (bez Enteru) - bezpecne, nic sa neaktivuje.
const OVERFLOW_NAMES: &[&str] = &["Show Hidden Icons"];

fn is_overflow_button(f: &input::Focused) -> bool {
    f.automation_id == "SystemTrayIcon" && OVERFLOW_NAMES.contains(&f.name.trim())
}

/// Panel zo sady pre akciu polozky.
fn kit_kind_for(a: Action) -> Option<u32> {
    match a {
        Action::TimerPanel => Some(POP_TIMER),
        Action::BrightnessPanel => Some(POP_BRIGHT),
        Action::PrivacyPanel => Some(POP_PRIVACY),
        Action::UpdatePanel => Some(POP_UPDATE),
        _ => None,
    }
}

fn blocked_text(b: Blocked, what: &str, keys: &str) -> String {
    match b {
        Blocked::ElevatedForeground => tf("blocked.elevated", &[what, keys]),
        Blocked::ModifiersHeld => tf("blocked.modifiers", &[what]),
        Blocked::Rejected => tf("blocked.rejected", &[what, keys]),
    }
}

struct App {
    mode: ThemeMode,
    theme: Theme,
    clock: String,
    battery: Option<Battery>,
    /// Simulovana bateria (len test vzhladu, `--sim-battery`); ma prednost pred skutocnym stavom.
    sim_battery: Option<Battery>,
    /// Simulovana media session (len test vzhladu posuvneho textu, `--sim-media`); ma prednost pred skutocnou.
    sim_media: Option<SessionInfo>,
    media_enabled: bool,
    media: Option<MediaService>,
    snapshot: Snapshot,
    selection: Selection,
    prefer_app: Option<String>,
    user_pick: Option<String>,
    spotify_running: bool,
    spotify_hint_dismissed: bool,
    notice: Option<String>,
    pending: Option<Pending>,
    /// Posledne rozlozenie kazdej listy (podla ID monitora) pre hit-testing.
    layouts: HashMap<String, Vec<Placed>>,
    /// Skratka vyhladavaca, o ktoru sa transom naposledy pokusil (`Some("")` = vypnuta); zlyhanie sa oznami len raz.
    hotkey_tried: Option<String>,
    /// Skratka je naozaj zaregistrovana.
    hotkey_ok: bool,
    /// Sirka kazdej listy v px (z posledneho kreslenia) - ukotvenie vyhladavaca otvoreneho skratkou.
    bar_widths: HashMap<String, i32>,
    /// Faza posunu obsahu (ochrana OLED), `None` = bez posunu.
    shift_phase: Option<u64>,
    /// Id suborov Police v liste z posledneho kreslenia (cast `i` polozky Polica = `painted_shelf[i]`).
    painted_shelf: Vec<String>,
    /// x stlacenia mysi na liste (tahanie suboru Police zacne po posune o `DRAG_PX`).
    press_x: i32,
    hovered: Option<(String, ItemId, usize)>,
    /// Cast, nad ktorou bolo stlacene lave tlacidlo (akcia az pri uvolneni nad tou istou castou).
    pressed: Option<(String, ItemId, usize)>,
    /// Obal zvolenej session, vyrieseny raz pri novej snimke (nie pri kazdom pohybe mysi).
    art: Option<ArtImage>,
    /// Akcie oblasti (v poradi `regions::set`) pre MSAA `Invoke`, podla monitora.
    region_actions: HashMap<String, Vec<Option<Action>>>,
    config: transom_config::Config,
    config_path: std::path::PathBuf,
    /// Index registrovanej spravy "konfiguracia sa zmenila".
    bc_config: u32,
    /// Index spravy "znova zobraz listu" (druhe spustenie transom).
    bc_restore: u32,
    /// Lista je skryta uzivatelom (menu "Skryt listu").
    hidden: bool,
    /// Vybrany monitor nie je pripojeny: lista je na primarnom a zobrazuje trvaly oznam.
    display_fallback: bool,
    cal: cal::CalState,
    /// Monitor, na ktorom bol otvoreny posledny popover.
    popover_monitor: Option<String>,
    /// Okno, ktore sa ma po aktivacii ocitnut v poprede (overi `T_ACTIVATE`).
    activate_target: Option<isize>,
    ai: ai::AiState,
    ai_started: bool,
    /// Monitor a x posledneho praveho kliku (ukotvenie panelu otvoreneho z ponuky).
    menu_at: Option<(String, i32)>,
    /// Otvorene vlastne kontextove menu (model + rozlozenie).
    menu: Option<menu::Menu>,
    /// Krok testovacieho scenara `--test-menu` (0 = nespusteny).
    test_menu: u32,
    test_ai: u32,
    /// Index spravy "lokalne data o vyuziti zmazane".
    bc_usage_deleted: u32,
    bc_update_check: u32,
    /// Aktualizacie (vlakno sa nespusti pri testoch a izolovanych behoch).
    updater: Option<update::Updater>,
    /// Verzia, o ktorej uz lista dala vediet (oznamenie sa neopakuje).
    update_announced: Option<String>,
    /// Dostupna novsia verzia (z vlakna aktualizacii alebo simulacie): ikona na liste, polozka menu, panel.
    update_avail: Option<update::Release>,
    /// Stahuje a overuje sa instalator (hlavne tlacidlo panela je zakazane).
    update_busy: bool,
    /// Posledna chyba instalacie (ukaze sa v paneli); vymaze sa pri dalsom pokuse.
    update_error: Option<String>,
    /// `--test-update`: simulovana aktualizacia (ziadna siet, nic sa nestahuje ani neotvara); krok scenara.
    test_update: u32,
    paints: u64,
    /// Posuvny nazov skladby (marquee): spolocny cas, geometria a posun podla listy.
    ticker: ticker::Ticker,
    /// Presny casovac snimok posuvu (vytvara sa pri prvej potrebe; `None` = nedostupny, pouzije sa `SetTimer`).
    frame_timer: Option<transom_platform::FrameTimer>,
    frame_timer_tried: bool,
    /// Posledne zalogovane 5 s okno diagnostiky plynulosti.
    stats_logged: u64,
    /// Windows: animacie su vypnute -> text sa neposuva (staticky s "...").
    reduced_motion: bool,
    /// Listy skryte plnou obrazovkou (animacia sa tam nekresli).
    hidden_bars: HashSet<String>,
    /// Posun posuvneho textu pouzity v prvom prechode vrstveneho kreslenia (druhy prechod musi byt rovnaky).
    pass_anim: Option<(usize, i32)>,
    /// `--no-tray`: ikona v oblasti oznameni sa nepridava (testy, aby po sebe nenechavali "duchov").
    no_tray: bool,
    /// Ikona v oblasti oznameni sa synchronizovala (az ked existuje hostitel).
    tray_synced: bool,
    /// Zvuky rozhrania (po umyselnom a uspesnom kliknuti). Mlcky prehravac, kym ho `main` nenahradi skutocnym;
    /// testy a automatizacia nikdy nemaju zariadenie.
    sound: snd::Player,
    /// Sluzba systemoveho vystupu zvuku (Core Audio); bezi len ked je polozka `audio` v liste viditelna a nejde
    /// o automatizovany beh.
    audio: Option<audio::Service>,
    /// Mixer aplikacii: bezi len pocas otvoreneho panela Zvuk.
    mixer: Option<transom_mixer::Service>,
    /// Bluetooth zvukove zariadenia: bezi len pocas otvoreneho panela Zvuk.
    bt: Option<transom_mixer::bt::BtService>,
    /// Pripojene kalendare (ICS): bezi, kym je nejaky odkaz v nastaveniach.
    ics: Option<calsync::IcsSync>,
    /// Rozlozenie klavesnice a Caps Lock z posledneho citania.
    keys: transom_platform::keyboard::KeyState,
    /// Casovac citania klavesnice je naplanovany.
    keys_polling: bool,
    /// Casovac, pomodoro a stopky.
    timers: timer::Timers,
    /// Stav otvoreneho panela zo sady (Casovac, Jas, Mikrofon a kamera).
    kit: pop_kit::Kit,
    /// Jas monitorov (DDC/CI): bezi, kym je polozka viditelna alebo panel otvoreny.
    bright: Option<transom_platform::brightness::Service>,
    /// Merac vytazenia (kym je polozka viditelna) a co meria.
    sysmon: Option<(
        transom_platform::sysmon::Monitor,
        transom_platform::sysmon::Want,
    )>,
    /// Mikrofon a kamera (kym je polozka viditelna alebo panel otvoreny).
    privacy: Option<transom_mixer::capture::PrivService>,
    /// Skratka stlmenia mikrofonu: posledny pokus o registraciu a ci sa podarila.
    mic_hotkey_tried: Option<String>,
    mic_hotkey_ok: bool,
    /// Po skratke ukazat oznam so stavom z dalsej snimky.
    mic_notice_pending: bool,
    /// Sluzby novych poloziek sa uz raz zosynchronizovali (po vytvoreni riadiaceho okna).
    extras_started: bool,
    /// Povolene spustenie sluzby (`false` v testoch a pri automatizacii).
    audio_allowed: bool,
    audio_panel: audio_pop::AudioPanel,
    /// Sluzba baterii periferii (citanie HID a vlastnosti Bluetooth na vlastnom vlakne); bezi len ked je polozka
    /// `peripherals` viditelna, je zapnuty aspon jeden zdroj a nejde o automatizovany beh. Druhy prvok = konfiguracia,
    /// s ktorou bezi (zmena zdrojov alebo intervalu ju spusti znova).
    periph: Option<(periph::Service, transom_config::Peripherals)>,
    /// Povolene spustenie sluzby periferii (`false` v testoch a pri automatizacii).
    periph_allowed: bool,
    periph_panel: periph_pop::PeriphPanel,
    tools_panel: tools_pop::ToolsPanel,
    tools_data: tools_pop::ToolsData,
    launcher_panel: launcher_pop::LauncherPanel,
    live_tools: Option<live_tools::Live>,
    /// Poznamka, ktoru sa pri zatvoreni okna nepodarilo ulozit (limit, nepouzitelne ulozisko): ostava len v pamati a obnovi sa pri dalsom otvoreni.
    note_draft: Option<tools_pop::Editor>,
    /// Caka sa na vysledok pustenia suborov: ukaze sa ako oznam na liste (panel mozno nie je otvoreny).
    drop_pending: bool,
    /// Ikony vysledkov vyhladavaca (asynchronne nacitavanie systemovych ikon; vznika pri prvom otvoreni vyhladavaca).
    icons: Option<std::sync::Arc<icon_rt::IconRt>>,
    /// Kniznica snimok obrazovky (len citanie priecinkov; vznika pri prvom otvoreni karty Snimky) a nastavenie, s ktorym bezi.
    library: Option<library_rt::LibraryRt>,
    lib_cfg: Option<transom_config::ShotsCfg>,
    /// Panel Nastroje je otvoreny (kniznica sa sleduje len kym je viditelna karta Snimky).
    tools_open: bool,
    /// Uprava lisaty priamo na liste (`None` = vypnuta).
    bar_edit: Option<bar_edit::BarEdit>,
    /// Vybezok odlozenych suborov (prejdenie mysou nad ikonou v strede listy).
    stash: stash_notch::StashNotch,
    /// Vybezok s prehravanou hudbou (prejdenie mysou nad mediami).
    media_notch: media_notch::MediaNotch,
    /// Nahlad schranky (prejdenie mysou nad tlacidlom Schranka).
    clip_notch: clip_notch::ClipNotch,
    /// Co vybezok prave ukazuje (pri prezerani).
    notch_view: NotchView,
    /// Kratke oznamenie vo vybezku (hlasitost, koniec casovaca, udalost).
    notch_info: Option<info_notch::Info>,
    /// Udalosti, ktore uz vybezok ohlasil (zaciatok; nazov sa nikde neuklada ani nelogguje).
    announced: Vec<i64>,
    /// Predpoved otvorena z Nastrojov: sipka spat.
    weather_back: bool,
    /// Kotva posledneho otvoreneho panela (monitor, x v liste): dlazdice z Nastrojov otvaraju panely na tom istom mieste.
    popover_anchor: Option<(String, i32)>,
    /// Ostrov v strede listy (hudba, AI, kratke oznamenia) a jeho menu.
    island: island_rt::IslandRt,
    island_menu: island_menu::IslandMenu,
    /// Rozbehnuty vyrez do odlozenych suborov: (poradie obsahu schranky pred vyrezom, zaciatok).
    snip: Option<(u32, std::time::Instant)>,
    /// Aktivne okno (aplikacia, titulok) z poslednej obnovy; titulok sa nikdy nelogue.
    window_info: Option<(String, String)>,
    /// Virtualne plochy (pocet, aktualna).
    desktop_info: Option<(usize, usize)>,
    /// Obnova bezi (casovac je naplanovany).
    watching: bool,
    /// Pocasie (bezi len s nastavenym mestom a polozkou v liste).
    weather: Option<weather::WeatherSync>,
    /// Uchyt okna lisy (vlastnik dialogu vyberu suborov).
    bar_hwnd: isize,
    /// Dialog vyberu suborov pre Policu bezi na vlastnom vlakne; vysledok (cesty) sa odovzda cez `N_PICKED`.
    picking: bool,
    picked: std::sync::Arc<Mutex<Option<Vec<String>>>>,
    launcher: Option<launcher_rt::Rt>,
    launcher_box: launcher_rt::Mailbox,
    launcher_recents: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    tools: Option<transom_tools::service::Service>,
    tools_snap: std::sync::Arc<transom_tools::service::Snapshot>,
    clip_listener: Option<transom_platform::clipboard::Listener>,
}

impl App {
    fn new(
        mode: ThemeMode,
        prefer_app: Option<String>,
        sim_battery: Option<Battery>,
        media: bool,
    ) -> Self {
        let mut a = App {
            mode,
            theme: mode.resolve(system_uses_dark()),
            clock: String::new(),
            battery: None,
            sim_battery,
            sim_media: None,
            media_enabled: media,
            media: None,
            snapshot: Snapshot::default(),
            selection: Selection::default(),
            prefer_app,
            user_pick: None,
            spotify_running: false,
            spotify_hint_dismissed: false,
            notice: None,
            pending: None,
            layouts: HashMap::new(),
            hotkey_tried: None,
            hotkey_ok: false,
            bar_widths: HashMap::new(),
            shift_phase: None,
            painted_shelf: Vec::new(),
            press_x: 0,
            hovered: None,
            pressed: None,
            art: None,
            region_actions: HashMap::new(),
            config_path: transom_config::default_path(),
            config: transom_config::Config::default(),
            bc_config: register_broadcast(transom_config::CHANGED_MESSAGE),
            bc_restore: register_broadcast(transom_config::RESTORE_MESSAGE),
            hidden: false,
            display_fallback: false,
            cal: cal::CalState::new(pclock::today()),
            popover_monitor: None,
            activate_target: None,
            ai: ai::AiState::new(),
            ai_started: false,
            menu_at: None,
            menu: None,
            test_menu: 0,
            test_ai: 0,
            bc_usage_deleted: register_broadcast(transom_config::USAGE_DELETED_MESSAGE),
            bc_update_check: register_broadcast(transom_config::UPDATE_CHECK_MESSAGE),
            updater: None,
            update_announced: None,
            update_avail: None,
            update_busy: false,
            update_error: None,
            test_update: 0,
            paints: 0,
            ticker: ticker::Ticker::default(),
            stats_logged: 0,
            frame_timer: None,
            frame_timer_tried: false,
            reduced_motion: !transom_platform::system::animations_enabled(),
            hidden_bars: HashSet::new(),
            pass_anim: None,
            no_tray: false,
            tray_synced: false,
            sound: snd::Player::silent(),
            audio: None,
            mixer: None,
            bt: None,
            ics: None,
            keys: Default::default(),
            keys_polling: false,
            timers: timer::Timers::new(),
            kit: pop_kit::Kit::default(),
            bright: None,
            sysmon: None,
            privacy: None,
            mic_hotkey_tried: None,
            mic_hotkey_ok: false,
            mic_notice_pending: false,
            extras_started: false,
            audio_allowed: false,
            audio_panel: audio_pop::AudioPanel::default(),
            periph: None,
            periph_allowed: false,
            periph_panel: periph_pop::PeriphPanel::default(),
            tools_panel: tools_pop::ToolsPanel::default(),
            tools_data: tools_pop::ToolsData::default(),
            launcher_panel: launcher_pop::LauncherPanel::default(),
            live_tools: None,
            note_draft: None,
            drop_pending: false,
            icons: None,
            library: None,
            lib_cfg: None,
            tools_open: false,
            bar_edit: None,
            stash: stash_notch::StashNotch::default(),
            media_notch: media_notch::MediaNotch::default(),
            clip_notch: clip_notch::ClipNotch::default(),
            notch_view: NotchView::Stash,
            notch_info: None,
            announced: Vec::new(),
            popover_anchor: None,
            weather_back: false,
            island: island_rt::IslandRt::default(),
            island_menu: island_menu::IslandMenu::default(),
            snip: None,
            window_info: None,
            desktop_info: None,
            watching: false,
            weather: None,
            bar_hwnd: 0,
            picking: false,
            picked: std::sync::Arc::default(),
            launcher: None,
            launcher_box: std::sync::Arc::default(),
            launcher_recents: std::sync::Arc::default(),
            tools: None,
            tools_snap: std::sync::Arc::default(),
            clip_listener: None,
        };
        a.refresh_battery();
        a.reload_config();
        a.ai.load_cache(&a.config.usage);
        a
    }

    /// Nacita konfiguraciu (pri starte a po sprave z okna nastaveni) a prekresli, ak sa nieco zmenilo.
    fn reload_config(&mut self) {
        let loaded = transom_config::load(&self.config_path);
        if let Some(n) = &loaded.note {
            log(&format!("konfiguracia: {n}"));
        }
        log(&format!(
            "konfiguracia nacitana z {} (len na citanie: {})",
            self.config_path.display(),
            loaded.read_only
        ));
        let before = self.config.clone();
        if matches!(
            loaded.issue,
            Some(transom_config::Issue::Corrupt | transom_config::Issue::Unreadable)
        ) {
            log("konfiguracia je poskodena alebo necitatelna: ostava platna konfiguracia z pamate");
        }
        // poskodeny / rozpisany subor nesmie vratit listu na predvolene hodnoty
        let effective = loaded.effective(&before);
        let lang_before = transom_i18n::lang();
        let windows_lang = pclock::ui_language_tag();
        let lang = transom_i18n::resolve(&effective.language, &windows_lang);
        log(&format!(
            "jazyk aplikacie: {} (nastavenie '{}', Windows '{windows_lang}')",
            lang.code(),
            effective.language
        ));
        self.update("konfiguracia", |a| {
            // jazyk sa prepne hned: nove texty listy, tooltipov a MSAA vzniknu pri prekresleni nizsie
            transom_i18n::set_lang(lang);
            transom_platform::access::set_default_action_name(&t("acc.press"));
            a.config = effective;
            // zvuky: nove nastavenie (stlmenie a hlasitost) plati hned, aj pre zvuk cakajuci vo fronte
            a.sound.set_settings(&a.config.ui_sounds);
            a.clock = clock_text(&a.config.clock);
            a.reselect();
        });
        if before.displays != self.config.displays {
            // monitory zmenene v nastaveniach: uplatni sa hned (pridaju/odoberu sa listy; ulozene ID sa nemeni)
            popup::request(Request::SetMonitors(selection_of(&self.config.displays)));
        }
        let (bu, au) = (before.usage.clone(), self.config.usage.clone());
        // zdroje a nastavenie archivu (zapnutie / pozastavenie / retencia) = novy zber (aj prvy zapis archivu hned)
        let sources_changed = (
            bu.claude_local,
            bu.claude_statusline,
            bu.codex_local,
            bu.archive.clone(),
        ) != (
            au.claude_local,
            au.claude_statusline,
            au.codex_local,
            au.archive.clone(),
        );
        if sources_changed {
            // zdroje AI zapnute/vypnute: nacitat cache alebo zahodit data
            self.update("zdroje AI", |a| a.ai.load_cache(&a.config.usage));
            // zapnuty zdroj: nacitat hned (na pozadi; je to cin pouzivatela, nie dotazovanie)
            self.ai
                .refresh(&self.config.usage, notifier(), ai::RefreshKind::Manual);
        }
        // ina zmena (zdroje, interval): preplanovat; zmena kruhu (poskytovatel, rezim) obnovu nespusta
        if sources_changed || bu.refresh_minutes != au.refresh_minutes {
            self.schedule_usage();
        }
        if before.usage != self.config.usage {
            popup::invalidate_popover();
        }
        if before.media != self.config.media {
            // posuvny text: nove nastavenie (smer, rychlost, zap/vyp) = znova od zaciatku
            self.reduced_motion = !transom_platform::system::animations_enabled();
            self.ticker.reset();
            invalidate_all();
            self.kick_ticker();
        }
        if before.appearance.tray_icon != self.config.appearance.tray_icon {
            self.sync_tray();
        }
        self.sync_tools();
        if before.tools.launcher != self.config.tools.launcher {
            self.launcher = None;
            if !self.config.tools.launcher.recent_searches {
                transom_tools::recents::Recents::open(transom_tools::tools_dir().join("launcher"))
                    .clear();
            }
        }
        if before.tools.launcher != self.config.tools.launcher {
            self.sync_hotkey();
        }
        if before.layout != self.config.layout {
            self.sync_audio();
        }
        if before.layout != self.config.layout || before.peripherals != self.config.peripherals {
            self.sync_periph();
            invalidate_all();
            popup::invalidate_popover();
        }
        if before.audio_panel != self.config.audio_panel {
            popup::invalidate_popover();
        }
        // novy kalendar sa pridal zo schranky: jeho tajny odkaz nema ostat v historii schranky transom
        let added: Vec<String> = self
            .config
            .ics_calendars
            .iter()
            .filter(|u| !before.ics_calendars.contains(u))
            .cloned()
            .collect();
        if !added.is_empty() {
            let ids = extras::clip_ids_with_links(&self.tools_snap.clipboard, &added);
            for id in ids {
                self.tools_send(transom_tools::service::Cmd::ClipDelete(id));
            }
            log(&format!(
                "kalendare ICS: pridane {} (z historie schranky odstranene)",
                added.len()
            ));
        }
        if before.oled != self.config.oled {
            log(&format!("ochrana OLED: {:?}", self.config.oled));
            if before.oled.auto_hide() != self.config.oled.auto_hide() {
                // listy sa vytvoria nanovo (s rezervaciou miesta alebo bez nej)
                popup::request(Request::SetAutoHide(self.config.oled.auto_hide()));
            }
            self.sync_shift();
            invalidate_all();
        }
        if before.appearance != self.config.appearance {
            // alfa < 255 prepne okna na vrstveny rezim, 255 spat na ClearType (rozhoduje platforma pri kresleni)
            log(&format!(
                "vzhlad pozadia: farba={:?} alfa={}",
                self.config.appearance.color(),
                self.config.appearance.alpha()
            ));
            invalidate_all();
        }
        if lang != lang_before {
            invalidate_all();
            popup::invalidate_popover();
        }
        if before.clock != self.config.clock {
            // sekundy zapnute/vypnute: preplanovat prebudenie hned
            schedule_wake(next_wake_delay_for(self.config.clock.seconds));
            popup::invalidate_popover();
        }
    }

    /// Periodicka obnova vyuzitia AI v intervale z konfiguracie (0 = len na poziadanie); bez zapnutych zdrojov nic.
    fn schedule_usage(&self) {
        let u = &self.config.usage;
        match u.refresh_interval_minutes() {
            // interval je vzdy najmenej 5 min; ani casovac nebezi castejsie
            Some(m) if u.any_enabled() => schedule_timer(T_USAGE, m.saturating_mul(60_000)),
            _ => transom_platform::cancel_timer(T_USAGE),
        }
    }

    /// Ikona transom v oblasti oznameni: zapnuta v nastaveniach (a nie `--no-tray`); znacka podla temy **panela uloh**
    /// (`SystemUsesLightTheme`), nie podla temy aplikacie.
    fn sync_tray(&mut self) {
        self.tray_synced = true;
        let want = (self.config.appearance.tray_icon || self.hidden) && !self.no_tray;
        let light_taskbar = !system_uses_dark();
        let added = transom_platform::set_tray(want.then(|| transom_platform::TraySpec {
            tip: t(if self.hidden {
                "tray.tip_hidden"
            } else {
                "tray.tip"
            }),
            light_taskbar,
        }));
        log(&format!(
            "ikona v oblasti oznameni: zelana={want} pridana={added} panel_uloh={}",
            if light_taskbar { "svetly" } else { "tmavy" }
        ));
    }

    /// Prehrava sa zvolena session?
    fn media_playing(&self) -> bool {
        self.chosen_session()
            .is_some_and(|s| s.state == PlayState::Playing)
    }

    /// Text + stav prehravania zvolenej session (zmena restartuje posuvny text).
    fn ticker_key(&self) -> Option<(String, bool)> {
        self.chosen_session()
            .map(|s| (s.display_text(), s.state == PlayState::Playing))
    }

    fn marquee_policy(&self) -> Policy {
        Policy {
            enabled: self.config.media.marquee,
            reduced_motion: self.reduced_motion,
            playing: self.media_playing(),
            scroll_when_paused: self.config.media.marquee_paused,
        }
    }

    /// (rychlost v lp/s, opacny smer)
    fn marquee_motion(&self) -> (f32, bool) {
        (
            self.config.media.speed().lp_per_s(),
            self.config.media.direction() == transom_config::MarqueeDirection::Rtl,
        )
    }

    /// Zosuladi posuvny text so stavom (zmena textu/prehravania ho restartuje od zaciatku).
    fn sync_ticker(&mut self) {
        let key = self.ticker_key();
        if self.ticker.set_key(key) {
            self.kick_ticker();
        }
    }

    /// Vyziada vyhodnotenie animacie (jednorazovy casovac za 1 ms; dalsie planuje `marquee_tick` podla potreby).
    fn kick_ticker(&mut self) {
        self.arm_frame(1);
    }

    /// Naplanuje dalsi snimok posuvu: presny casovac (vlakno + waitable timer), inak `SetTimer`.
    fn arm_frame(&mut self, ms: u32) {
        // presny casovac sa da vytvorit az ked existuje hostitel (notifier); dovtedy sa pouzije `SetTimer`
        if !self.frame_timer_tried && notifier().is_some() {
            self.frame_timer_tried = true;
            self.frame_timer =
                notifier().and_then(|n| transom_platform::FrameTimer::new(n, N_FRAME));
            log(&format!(
                "casovac snimok posuvu: {}",
                match &self.frame_timer {
                    Some(t) if t.is_high_res() => "vysoke rozlisenie (waitable)",
                    Some(_) => "waitable (bez vysokeho rozlisenia)",
                    None => "SetTimer (zaloha)",
                }
            ));
        }
        match &self.frame_timer {
            Some(t) => t.arm(ms),
            None => schedule_timer(T_MARQUEE, ms),
        }
    }

    /// Jeden snimok posuvneho textu: prekresli len usek textu na lištach, kde sa posun zmenil, a naplanuje dalsi
    /// snimok (alebo nic, ak sa nehybe nic - pauza na konci textu, staticky text, pozastavene prehravanie).
    fn marquee_tick(&mut self) {
        let policy = self.marquee_policy();
        let (speed, rtl) = self.marquee_motion();
        let elapsed = self.ticker.elapsed_ms();
        let step = self
            .ticker
            .step(&policy, speed, rtl, elapsed, &self.hidden_bars);
        for (m, x, w) in &step.repaint {
            transom_platform::invalidate_span(m, *x, *w);
        }
        match step.next_ms {
            Some(ms) => self.arm_frame(ms),
            None => {
                // nic sa nehybe: zrusit cakajuce prebudenie (zadne dalsie odbery)
                if let Some(t) = &self.frame_timer {
                    t.disarm();
                }
            }
        }
        // diagnostika plynulosti (--ticker-stats): suhrn do logu najviac raz za 5 s
        if elapsed / 5000 != self.stats_logged {
            self.stats_logged = elapsed / 5000;
            let sm = self.ticker.stats_summary();
            if !sm.is_empty() {
                log(&format!("plynulost posuvu: {sm}"));
            }
        }
    }

    /// Tema listy: tema (svetla/tmava/system) s vlastnou farbou **len pozadia**; text ostava podla temy, kym je na farbe
    /// citatelny, inak sa pouziju farby textu opacnej temy (`Theme::on_background`). Odvodzuje sa vzdy znova z
    /// `self.theme`, takze ju zmena temy systemu neprepise; popovery pouzivaju `self.theme` (nepriehladne).
    /// Cierne sklo pod panelmi a menu (`bar = false`) alebo pod listou (`bar = true`) podla nastaveni vzhladu; nie pri
    /// ochrane OLED s ciernym pozadim, vysokom kontraste ani vypnutych efektoch priehladnosti.
    fn glass_on(&self, bar: bool) -> bool {
        let a = &self.config.appearance;
        (if bar { a.glass_bar } else { a.glass_panels })
            && !self.config.oled.black_bg()
            && transom_platform::glass::system_allows_cached()
    }

    /// Farba, ktorou zacinaju panely pod listou: farba lisaty; pri priehladnej liste (pod nou je hlavne plocha) farba
    /// temy, aby panel nezacinal napr. bielou z "uplne priehladnej bielej".
    fn bar_lead_color(&self) -> (u8, u8, u8) {
        let alpha = self.config.appearance.effective(&self.config.oled).1;
        let bar = self.bar_theme().background;
        // farba lisaty len ak je na nej text panela (tema aplikacie) citatelny
        let c = if alpha >= 128 && transom_render::audit::ratio(bar, self.theme.foreground) >= 4.5 {
            bar
        } else {
            self.theme.background
        };
        (c.r, c.g, c.b)
    }

    fn bar_theme(&self) -> Theme {
        let (bg, alpha) = self.config.appearance.effective(&self.config.oled);
        // sklo pod listou ma nadych temy (nie vlastnu farbu): text sa riadi temou, inak by svetla vlastna farba dala
        // tmavy text na ciernom skle
        let bg = if self.glass_on(true) { None } else { bg };
        let text = match self.config.appearance.text() {
            transom_config::TextChoice::Auto => transom_render::BarText::Auto,
            transom_config::TextChoice::Light => transom_render::BarText::Light,
            transom_config::TextChoice::Dark => transom_render::BarText::Dark,
            transom_config::TextChoice::Custom((r, g, b)) => {
                transom_render::BarText::Custom(Color::rgb(r, g, b))
            }
        };
        let th = self
            .theme
            .for_bar(bg.map(|(r, g, b)| Color::rgb(r, g, b)), alpha)
            .with_text(text);
        if self.config.oled.dim_text() {
            th.dimmed()
        } else {
            th
        }
    }

    /// Faza posunu obsahu (ochrana OLED): meni sa raz za minutu; pri zmene sa prekreslia vsetky listy. Vypnuty posun = `None`.
    fn sync_shift(&mut self) {
        let phase = self
            .config
            .oled
            .pixel_shift()
            .then(|| (ai::now_unix().max(0) / 60) as u64);
        if phase != self.shift_phase {
            self.shift_phase = phase;
            invalidate_all();
        }
    }

    fn prefer_app(&self) -> Option<&str> {
        let c = self.config.media.prefer_app.trim();
        if c.is_empty() {
            self.prefer_app.as_deref()
        } else {
            Some(c)
        }
    }

    /// Zvolena session: simulovana (`--sim-media`), inak skutocna z GSMTC.
    fn chosen_session(&self) -> Option<&SessionInfo> {
        self.sim_media.as_ref().or_else(|| {
            self.selection
                .chosen
                .as_ref()
                .and_then(|k| self.snapshot.sessions.iter().find(|s| &s.key == k))
        })
    }

    fn items(&self) -> Vec<StatusItem> {
        let media = self.chosen_session();
        // kruh sa pocita z casu: po resete okna (minutovy tik) sa hodnota sama zmeni na "nedostupne"
        let ring = ai::ring_view(self.ai.data.as_ref(), &self.config.usage, ai::now_unix());
        let fallback_text = self.display_fallback.then(|| t("notice.display_missing"));
        let audio_view = self.audio_wanted_in_layout().then(|| {
            let (glyph, name) = audio_pop::bar_glyph_and_name(&self.audio_snapshot());
            view::AudioView { glyph, name }
        });
        let periph_snap = self.periph_snapshot();
        let periph_view = self
            .periph_wanted()
            .then(|| view::periph_view(&periph_snap, self.config.peripherals.on_bar));
        let shelf_view = self.shelf_bar();
        let keys = self.item_visible("keyboard").then_some(&self.keys);
        let timer_view = self
            .item_visible("timer")
            .then(|| extras::timer_view(&self.timers, extras::now_ms()));
        let bright_view = self
            .item_visible("brightness")
            .then(|| extras::bright_view(&self.bright_snapshot()));
        let priv_snap = self.privacy_snapshot();
        let priv_parts = if self.item_visible("privacy") {
            extras::priv_parts(&priv_snap, self.config.privacy.always_show)
        } else {
            Vec::new()
        };
        let sys_view = self
            .sysmon
            .as_ref()
            .filter(|_| self.item_visible("sysmon"))
            .map(|(m, _)| extras::sys_view(&m.last(), &self.config.sysmon));
        let weather_snap = self.weather.as_ref().and_then(|w| w.snapshot());
        let mut items = view::build_items(&view::View {
            clock: &self.clock,
            battery: self.sim_battery.or(self.battery),
            media,
            art: self.art.clone(),
            media_pick: self.selection.indicator(),
            spotify_hint: !self.spotify_hint_dismissed
                && self.media_enabled
                && spotify_hint_needed(self.spotify_running, &self.snapshot.sessions),
            notice: self.notice.as_deref().or(fallback_text.as_deref()),
            ai: Some(&ring),
            audio: audio_view.as_ref(),
            peripherals: periph_view.as_ref(),
            shelf: shelf_view.as_ref(),
            keyboard: keys,
            timer: timer_view.as_ref(),
            brightness: bright_view.as_ref(),
            sysmon: sys_view.as_ref(),
            privacy: &priv_parts,
            window: self.window_info.as_ref(),
            desktops: self.desktop_info,
            weather: weather_snap.as_ref(),
            update: self.update_icon(),
        });
        // stred listy: ostrov (logo transom, pocas aktivity hudba / AI / oznamenie)
        if let Some(it) = items.iter_mut().find(|i| i.id == ItemId::Shelf) {
            it.parts = vec![self.island_part()];
        }
        view::arrange(items, &self.config.layout)
    }

    /// Je polozka `id` viditelna v rozlozeni?
    fn item_visible(&self, id: &str) -> bool {
        self.config
            .layout
            .items
            .iter()
            .any(|s| s.id == id && s.visible)
    }

    /// Klavesnica: precita stav a naplanuje dalsie citanie, kym je polozka viditelna (lacne volania, bez haku).
    fn sync_keys(&mut self) {
        if !self.item_visible("keyboard") {
            if std::mem::take(&mut self.keys_polling) {
                transom_platform::appbar::cancel_timer(T_KEYS);
            }
            return;
        }
        let now = transom_platform::keyboard::state();
        if now != self.keys {
            self.update("klavesnica", |a| a.keys = now);
        }
        self.keys_polling = true;
        schedule_timer(T_KEYS, KEYS_POLL_MS);
    }

    /// Subory Police v liste: len ked je Polica zapnuta a polozka viditelna v rozlozeni.
    fn shelf_bar(&self) -> Option<view::ShelfView> {
        let visible = self
            .config
            .layout
            .items
            .iter()
            .any(|s| s.id == "shelf" && s.visible);
        // ostrov v strede: zobrazeny podla rozlozenia (odlozene subory su len jedna z jeho kariet)
        visible.then(|| view::shelf_view(&self.tools_snap.shelf))
    }

    /// Odlozene subory pre vybezok: prvych `MAX_ROWS` v poradi panela a celkovy pocet.
    fn stash_files(&self) -> (Vec<stash_notch::File>, usize) {
        let rows = &self.tools_snap.shelf;
        let files = rows
            .iter()
            .take(stash_notch::MAX_ROWS)
            .map(|r| stash_notch::File {
                id: r.id.clone(),
                name: r.name.clone(),
                dir: r.dir,
                available: r.state == transom_tools::shelf::State::Available && !r.remote,
            })
            .collect();
        (files, rows.len())
    }

    /// Mys stale nad ikonou odlozenych suborov po odlozke: rozbalit vybezok (nie pri uprave lisaty ani pri otvorenom
    /// paneli, aby sa neprekryvali).
    /// Vyrez do odlozenych suborov: spusti vyrez Windows (Win+Shift+S) a caka na novu snimku v schranke.
    fn snip_start(&mut self) -> snd::Outcome {
        if !self.config.tools.shelf.enabled {
            self.show_notice(t("snip.stash_off"));
            return snd::Outcome::Failed;
        }
        if !transom_platform::shellops::open_target("ms-screenclip:") {
            self.show_notice(t("snip.failed"));
            return snd::Outcome::Failed;
        }
        log("vyrez: spusteny");
        self.snip = Some((
            transom_platform::clipboard::sequence(),
            std::time::Instant::now(),
        ));
        schedule_timer(T_SNIP, 400);
        snd::Outcome::Done
    }

    /// Pocasie bezi len so zvolenym miestom (alebo polohou zariadenia) a polozkou Pocasie v liste (inak sa nic neodosiela).
    fn sync_weather(&mut self) {
        let target = self.config.weather_target();
        let Some(target) = target.filter(|_| self.item_visible("weather")) else {
            if self.weather.take().is_some() {
                log("pocasie: vypnute");
            }
            return;
        };
        match &mut self.weather {
            Some(w) => w.set_target(target),
            None => {
                if let Some(n) = notifier() {
                    self.weather = weather::WeatherSync::start(
                        target,
                        std::sync::Arc::new(move || n.notify(N_WEATHER)),
                    );
                }
            }
        }
    }

    /// Obnova aktivneho okna a ploch (kazdu sekundu, kym je aspon jedna z poloziek v liste).
    fn watch_tick(&mut self) {
        let want_window = self.item_visible("window");
        let want_desk = self.item_visible("desktops");
        let w = if want_window {
            transom_platform::desktops::active_window().or_else(|| self.window_info.clone())
        } else {
            None
        };
        let d = if want_desk {
            transom_platform::desktops::desktops()
        } else {
            None
        };
        if w != self.window_info || d != self.desktop_info {
            self.window_info = w;
            self.desktop_info = d;
            invalidate_all();
        }
        if (want_window || want_desk) && !self.watching {
            self.watching = true;
            schedule_timer(T_WATCH, 1000);
        }
    }

    /// Kapatko: dalsie kliknutie na obrazovke precita farbu (nepostupi do aplikacie pod mysou).
    fn pick_start(&mut self) -> snd::Outcome {
        if !transom_platform::picker::start() {
            self.show_notice(t("pick.failed"));
            return snd::Outcome::Failed;
        }
        log("kapatko: spustene");
        self.show_info(
            info_notch::Info {
                icon: transom_render::scene::glyph_char(transom_core::item::Glyph::Picker),
                title: t("pick.name"),
                sub: t("pick.hint"),
                value: None,
                muted: false,
                swatch: None,
            },
            20_000,
        );
        snd::Outcome::Done
    }

    /// Vysledok kapatka: HEX do schranky a vzorka vo vybezku.
    fn pick_done(&mut self, c: Option<(u8, u8, u8)>) {
        let Some(c) = c else {
            log("kapatko: zrusene");
            self.info_end();
            return;
        };
        let hex = transom_platform::picker::hex(c);
        log("kapatko: farba skopirovana");
        self.tools_send(transom_tools::service::Cmd::Put(
            transom_tools::clipboard::Payload::Text(hex.clone()),
        ));
        self.show_info(
            info_notch::Info {
                icon: ' ',
                title: hex,
                sub: format!("RGB {}, {}, {} · {}", c.0, c.1, c.2, t("pick.copied")),
                value: None,
                muted: false,
                swatch: Some(c),
            },
            4000,
        );
    }

    /// Nova snimka v schranke: ulozit ako PNG do priecinka nastrojov a pridat do odlozenych suborov (odkaz).
    fn snip_poll(&mut self) {
        let Some((seq, t0)) = self.snip else {
            return;
        };
        if transom_platform::clipboard::sequence() == seq {
            if t0.elapsed().as_secs() < 90 {
                schedule_timer(T_SNIP, 400);
            } else {
                log("vyrez: bez snimky (cas vyprsal)");
                self.snip = None;
            }
            return;
        }
        self.snip = None;
        let Some((_, _, png)) = transom_platform::clipboard::read_image_png() else {
            log("vyrez: v schranke nie je obrazok");
            return;
        };
        let dir = transom_tools::store::tools_dir().join("snips");
        let path = dir.join(format!("snip-{}.png", ai::now_unix()));
        if std::fs::create_dir_all(&dir).is_err() || std::fs::write(&path, &png).is_err() {
            self.show_notice(t("snip.failed"));
            return;
        }
        log("vyrez: snimka pridana do odlozenych suborov");
        self.drop_pending = true;
        self.tools_send(transom_tools::service::Cmd::AddPaths {
            paths: vec![path.to_string_lossy().into_owned()],
            copy: false,
        });
        self.island_notice(island::Notice {
            icon: transom_render::scene::glyph_char(transom_core::item::Glyph::Snip),
            title: t("snip.done"),
            sub: t("island.stash_sub"),
            act: Some(island::NoticeAct::Stash),
        });
    }

    /// Kratke oznamenie vo vybezku na `ms` (nie pocas upravy lisaty ani pri otvorenom paneli).
    fn show_info(&mut self, info: info_notch::Info, ms: u32) {
        if self.bar_edit.is_some() || popup::popover_open().is_some() {
            return;
        }
        let h = info_notch::height_lp(&info);
        self.notch_info = Some(info);
        self.notch_view = NotchView::Info;
        transom_platform::dragdrop::notch_browse_at(None, Some(h));
        transom_platform::dragdrop::invalidate_notch();
        schedule_timer(T_INFO, ms);
    }

    /// Koniec oznamenia: vybezok sa zbali (ak nad nim nie je mys).
    fn info_end(&mut self) {
        if self.notch_view == NotchView::Info {
            self.notch_info = None;
            self.notch_view = NotchView::Stash;
            transom_platform::dragdrop::notch_browse(None);
        }
    }

    /// Koliesko nad listou: nad Zvukom hlasitost, nad Jasom jas monitorov (hodnotu ukazu ich panely a polozky).
    fn bar_wheel(&mut self, monitor_id: &str, x: i32, delta: i32) {
        let Some((id, _)) = self.layouts.get(monitor_id).and_then(|l| hit_test(l, x)) else {
            return;
        };
        let steps = (delta / 120).clamp(-5, 5);
        if steps == 0 {
            return;
        }
        match id {
            ItemId::Audio => {
                if let Some(v) = self.audio_snapshot().volume {
                    let p = (v.percent as i32 + steps * 2).clamp(0, 100) as u32;
                    self.run_audio_cmd(audio_pop::AudioCmd::SetVolume(p));
                    invalidate_all();
                }
            }
            ItemId::Brightness => {
                let snap = self.bright_snapshot();
                if let Some(b) = &self.bright {
                    for d in &snap.displays {
                        if let Some(l) = d.level {
                            b.set(&d.key, (l as i32 + steps * 5).clamp(0, 100) as u32);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Udalost z kalendara zacne do 10 minut: raz ju ohlasi vybezok (nazov sa nelogguje).
    fn event_check(&mut self) {
        if !self.config.notch.event {
            return;
        }
        let Some(cals) = self.ics.as_ref().map(|s| s.calendars()) else {
            return;
        };
        let today = pclock::today();
        let tomorrow =
            transom_core::clock::civil_from_days(transom_core::clock::days_from_civil(today) + 1);
        let occ = cal::ics_occurrences(&cals, today, tomorrow);
        let now = ai::now_unix();
        let Some((title, start)) = info_notch::next_event(&occ, now, 600) else {
            return;
        };
        if self.announced.contains(&start) {
            return;
        }
        self.announced.push(start);
        self.announced.retain(|s| *s > now - 86_400);
        log("kalendar: vybezok s najblizsou udalostou");
        let mins = ((start - now) as f64 / 60.0).ceil().max(1.0) as i64;
        self.island_notice(island::Notice {
            icon: '\u{E787}',
            title,
            sub: tf(
                "island.cal_sub",
                &[&tf("notch.event_in", &[&mins.to_string()])],
            ),
            act: Some(island::NoticeAct::Calendar),
        });
    }

    fn stash_open(&mut self) {
        let Some((monitor, id, _)) = self.hovered.clone() else {
            return;
        };
        let Some(view) = notch_view_for(id) else {
            return;
        };
        // bezi kratke oznamenie (hlasitost, casovac, udalost): ma prednost pred prezeranim
        if self.bar_edit.is_some()
            || popup::popover_open().is_some()
            || self.notch_view == NotchView::Info
        {
            return;
        }
        match view {
            NotchView::Island => {}
            NotchView::Info => {}
            NotchView::Weather => {
                if self.weather.as_ref().and_then(|w| w.snapshot()).is_none() {
                    return;
                }
                let anchor = self.layouts.get(&monitor).and_then(|l| {
                    l.iter().find(|p| p.item == ItemId::Weather).and_then(|p| {
                        transom_platform::appbar::bar_screen_x(&monitor, p.x + p.w / 2)
                    })
                });
                self.notch_view = view;
                transom_platform::dragdrop::notch_browse_at(anchor, Some(weather::NOTCH_H_LP));
            }
            NotchView::Clip => {
                // len so zapnutou historiou (nahlad ju nikdy nezapina)
                if !self.config.notch.clipboard || !self.tools_snap.clip_enabled {
                    return;
                }
                let anchor = self.layouts.get(&monitor).and_then(|l| {
                    l.iter()
                        .find(|p| p.item == ItemId::ToolClipboard)
                        .and_then(|p| {
                            transom_platform::appbar::bar_screen_x(&monitor, p.x + p.w / 2)
                        })
                });
                self.clip_notch.hover = None;
                self.notch_view = view;
                let n = self.tools_snap.clipboard.len();
                transom_platform::dragdrop::notch_browse_at(anchor, Some(clip_notch::height_lp(n)));
            }
            NotchView::Stash => {
                if !self.config.notch.stash {
                    return;
                }
                let (_, total) = self.stash_files();
                self.stash.hover = None;
                self.notch_view = view;
                log(&format!("odlozene subory: vybezok ({total})"));
                transom_platform::dragdrop::notch_browse(Some(stash_notch::height_lp(total)));
            }
            NotchView::Media => {
                if !self.config.notch.media || self.chosen_session().is_none() {
                    return;
                }
                // pod stredom polozky medii (nie v strede listy)
                let anchor = self.layouts.get(&monitor).and_then(|l| {
                    let xs: Vec<(i32, i32)> = l
                        .iter()
                        .filter(|p| p.item == ItemId::Media)
                        .map(|p| (p.x, p.x + p.w))
                        .collect();
                    let x0 = xs.iter().map(|v| v.0).min()?;
                    let x1 = xs.iter().map(|v| v.1).max()?;
                    transom_platform::appbar::bar_screen_x(&monitor, (x0 + x1) / 2)
                });
                self.media_notch.hover = None;
                self.notch_view = view;
                log("hudba: vybezok");
                transom_platform::dragdrop::notch_browse_at(anchor, Some(media_notch::H_LP));
            }
        }
    }

    /// Mys vo vybezku odlozenych suborov.
    fn stash_pointer(&mut self, kind: PointerKind, x: i32, y: i32) {
        // menu ostrova: zalozky a vlastne karty (snimky, AI) riesi menu, ostatne karty su vybezky pod zalozkami
        let (view, y, kind) = if self.notch_view == NotchView::Island {
            let (switched, cmd, redraw, own) = self.island_menu.pointer(kind, x, y);
            if let Some(tab) = switched {
                self.island_menu_data();
                // vyska podla karty (hudba nizsia), menu ostava na mieste
                transom_platform::dragdrop::notch_browse_at(
                    self.island_menu.anchor,
                    Some(island_menu::height_lp(Some(tab))),
                );
            }
            if redraw {
                transom_platform::dragdrop::invalidate_notch();
            }
            if let Some(c) = cmd {
                self.island_menu_cmd(c);
            }
            let view = match self.island_menu.tab {
                Some(island_menu::Tab::Music) => NotchView::Media,
                Some(island_menu::Tab::Clipboard) => NotchView::Clip,
                Some(island_menu::Tab::Files) => NotchView::Stash,
                _ => return,
            };
            // v riadku zaloziek: obsah karty dostane len odchod mysi (zrusi zvyraznenie)
            let kind = if own { PointerKind::Leave } else { kind };
            (view, y - self.island_menu.tab_h, kind)
        } else {
            (self.notch_view, y, kind)
        };
        if matches!(
            view,
            NotchView::Info | NotchView::Weather | NotchView::Island
        ) {
            return;
        }
        if view == NotchView::Clip {
            let (restore, redraw) = self.clip_notch.pointer(kind, x, y);
            if redraw {
                transom_platform::dragdrop::invalidate_notch();
            }
            if let Some(id) = restore {
                log("schranka: polozka z nahladu obnovena");
                self.tools_send(transom_tools::service::Cmd::ClipRestore(id));
                self.show_notice(t("clip.peek_restored"));
            }
            return;
        }
        if view == NotchView::Media {
            let (act, redraw) = self.media_notch.pointer(kind, x, y);
            if redraw {
                transom_platform::dragdrop::invalidate_notch();
            }
            if let Some(f) = self.media_notch.seek.take() {
                self.media_seek(f);
            }
            if let Some(a) = act {
                let out = self.act(a, None);
                self.sound.cue(snd::Origin::Pointer, out);
                transom_platform::dragdrop::invalidate_notch();
            }
            return;
        }
        let (cmd, redraw) = self.stash.pointer(kind, x, y);
        if redraw {
            transom_platform::dragdrop::invalidate_notch();
        }
        match cmd {
            Some(stash_notch::Cmd::Copy(id)) => {
                let out = self.shelf_copy(&id);
                self.sound.cue(snd::Origin::Pointer, out);
            }
            Some(stash_notch::Cmd::Drag(id)) => self.shelf_drag_id(&id),
            Some(stash_notch::Cmd::Open) => {
                let at = self.layouts.iter().find_map(|(m, l)| {
                    l.iter()
                        .find(|p| p.item == ItemId::Shelf)
                        .map(|p| (m.clone(), p.x + p.w / 2))
                });
                let out = self.act(Action::ToolsShelf, at);
                self.sound.cue(snd::Origin::Pointer, out);
            }
            None => {}
        }
    }

    /// Presun prehravania na cast `f` (0..=1) skladby zvolenej relacie.
    fn media_seek(&mut self, f: f32) {
        let Some(s) = self.chosen_session().cloned() else {
            return;
        };
        let tl = s.timeline;
        if !s.controls.seek || tl.end <= tl.start {
            return;
        }
        let t = tl.start + ((tl.end - tl.start) as f64 * f as f64) as i64;
        if let Some(m) = &self.media {
            log(&format!("media prikaz presun na {:.0} %", f * 100.0));
            m.command(&s.key, Command::Seek(t));
            // ukazat hned novu poziciu (prehravac ju potvrdi pri dalsom citani)
            if let Some(x) = self.snapshot.sessions.iter_mut().find(|x| x.key == s.key) {
                x.timeline.position = t;
                x.timeline.last_updated = transom_core::media::now_filetime();
            }
            transom_platform::dragdrop::invalidate_notch();
        }
    }

    /// Klik na subor Police v liste (`part` = poradie suboru z posledneho kreslenia): skopiruje ho do schranky (ako Enter
    /// v paneli). Nedostupny subor sa nekopiruje, len sa vysvetli.
    fn shelf_click(&mut self, part: usize) -> snd::Outcome {
        let Some(id) = self.painted_shelf.get(part).cloned() else {
            return snd::Outcome::Failed;
        };
        self.shelf_copy(&id)
    }

    /// Skopiruje odlozeny subor `id` do schranky; nedostupny sa nekopiruje, len sa vysvetli.
    fn shelf_copy(&mut self, id: &str) -> snd::Outcome {
        let Some(row) = self.tools_snap.shelf.iter().find(|r| r.id == id).cloned() else {
            return snd::Outcome::Failed;
        };
        let chip = view::shelf_view(std::slice::from_ref(&row));
        if !chip.files.first().is_some_and(|f| f.available) {
            self.show_notice(tf("shelf.unavailable", &[&row.name]));
            return snd::Outcome::Failed;
        }
        self.tools_send(transom_tools::service::Cmd::Put(
            transom_tools::clipboard::Payload::Files(vec![row.path.clone()]),
        ));
        self.show_notice(tf("shelf.copied", &[&row.name]));
        snd::Outcome::Done
    }

    /// Tahanie suboru Police z listy von (kopia alebo odkaz, nikdy presun). Bezi cez frontu poziadaviek, nie v obsluhe mysi.
    #[allow(dead_code)]
    fn shelf_drag(&mut self, part: usize) {
        if let Some(id) = self.painted_shelf.get(part).cloned() {
            self.shelf_drag_id(&id);
        }
    }

    /// Tahanie odlozeneho suboru `id` von (kopia alebo odkaz, nikdy presun).
    fn shelf_drag_id(&mut self, id: &str) {
        let Some(row) = self.tools_snap.shelf.iter().find(|r| r.id == id) else {
            return;
        };
        if row.remote {
            self.show_notice(t("tools.net_drag"));
        } else if row.state == transom_tools::shelf::State::Available {
            log("polica: tahanie suboru z listy");
            popup::request(Request::DragOut(vec![row.path.clone()]));
        } else {
            let name = row.name.clone();
            self.show_notice(tf("shelf.unavailable", &[&name]));
        }
    }

    /// Zaregistruje / zrusi globalnu skratku vyhladavaca podla nastaveni. Vypnuty vyhladavac skratku nema (inak by ju
    /// `Action::Launcher` presmeroval na Win+S s este drzanym Alt). Zlyhanie (skratku drzi ina aplikacia) sa oznami.
    fn sync_hotkey(&mut self) {
        let cfg = &self.config.tools.launcher;
        // aplikacia na celu obrazovku (hra, video): skratka sa uvolni, aby ju dostala ona (ak to pouzivatel nezmenil)
        let paused = !cfg.hotkey_in_fullscreen && !self.hidden_bars.is_empty();
        let wanted = (cfg.enabled && !paused).then(|| cfg.hotkey()).flatten();
        let label = wanted.map(|h| h.label()).unwrap_or_default();
        if self.hotkey_tried.as_deref() == Some(label.as_str()) {
            return;
        }
        self.hotkey_tried = Some(label.clone());
        let Some(h) = wanted else {
            if std::mem::take(&mut self.hotkey_ok) {
                transom_platform::hotkey::unregister(HOTKEY_LAUNCHER);
                log(if paused {
                    "skratka vyhladavaca: pozastavena (aplikacia na celu obrazovku)"
                } else {
                    "skratka vyhladavaca: vypnuta"
                });
            }
            return;
        };
        let keys = transom_platform::hotkey::Keys {
            ctrl: h.ctrl,
            alt: h.alt,
            shift: h.shift,
            win: h.win,
            vk: h.vk,
        };
        match transom_platform::hotkey::register(HOTKEY_LAUNCHER, keys) {
            Ok(()) => {
                self.hotkey_ok = true;
                log(&format!("skratka vyhladavaca: {label}"));
            }
            Err(code) => {
                self.hotkey_ok = false;
                log(&format!(
                    "skratka vyhladavaca {label}: registracia zlyhala (chyba {code})"
                ));
                self.show_notice(tf("notice.hotkey_taken", &[&label]));
            }
        }
    }

    /// Skratka vyhladavaca: otvori ho na monitore s kurzorom (pod polozkou Search, inak v strede listy); druhe stlacenie
    /// ho zavrie.
    fn launcher_hotkey(&mut self) {
        if popup::popover_open() == Some(POP_LAUNCHER) {
            popup::request(Request::ClosePopover);
            return;
        }
        let monitor = transom_platform::appbar::bar_under_cursor()
            .filter(|m| self.layouts.contains_key(m))
            .or_else(|| self.popover_monitor.clone())
            .filter(|m| self.layouts.contains_key(m))
            .or_else(|| {
                let mut keys: Vec<&String> = self.layouts.keys().collect();
                keys.sort();
                keys.first().map(|m| (*m).clone())
            });
        let Some(monitor) = monitor else {
            log("skratka vyhladavaca: ziadna lista (skryta), vyhladavac sa neotvori");
            return;
        };
        let placed = &self.layouts[&monitor];
        let x = placed
            .iter()
            .find(|p| p.item == ItemId::Search)
            .map(|p| p.x + p.w / 2)
            .unwrap_or_else(|| self.bar_widths.get(&monitor).copied().unwrap_or(0) / 2);
        let o = self.act(Action::Launcher, Some((monitor, x)));
        self.sound.cue(snd::Origin::Assistive, o);
    }

    /// Vykona zmenu stavu a prekresli len vtedy, ak sa zmenil zobrazeny obsah.
    fn update(&mut self, why: &str, f: impl FnOnce(&mut Self)) {
        let before = self.items();
        f(self);
        if self.items() != before {
            log(&format!("zmena obsahu ({why}) -> prekreslit"));
            invalidate_all();
        }
        self.sync_ticker();
    }

    /// Je polozka Zvuk v liste viditelna?
    fn audio_wanted_in_layout(&self) -> bool {
        self.config
            .layout
            .items
            .iter()
            .any(|s| s.id == "audio" && s.visible)
    }

    /// Aktualna snimka systemoveho vystupu (bez sluzby: "este sa nacitalo").
    fn audio_snapshot(&self) -> std::sync::Arc<audio::Snapshot> {
        self.audio
            .as_ref()
            .map(|s| s.snapshot())
            .unwrap_or_default()
    }

    /// Spusti / zastavi sluzbu podla toho, ci je polozka Zvuk viditelna (notifikacie Core Audio bez pollingu; ked
    /// polozka nie je viditelna, nebeží ziadne vlakno ani registracia).
    fn sync_audio(&mut self) {
        let wanted = self.audio_allowed && self.audio_wanted_in_layout();
        if wanted && self.audio.is_none() {
            let Some(n) = notifier() else { return };
            match audio::start_system(std::sync::Arc::new(move || n.notify(N_AUDIO))) {
                Ok(s) => {
                    self.audio = Some(s);
                    log("zvuk: sluzba systemoveho vystupu spustena");
                }
                Err(e) => log(&format!("zvuk: sluzbu sa nepodarilo spustit: {e}")),
            }
        } else if !wanted && self.audio.take().is_some() {
            log("zvuk: sluzba systemoveho vystupu zastavena");
            if popup::popover_open() == Some(POP_AUDIO) {
                popup::request(Request::ClosePopover);
            }
        }
    }

    /// Je polozka Baterie periferii v liste viditelna a je zapnuty aspon jeden zdroj?
    fn periph_wanted(&self) -> bool {
        self.config
            .layout
            .items
            .iter()
            .any(|s| s.id == "peripherals" && s.visible)
    }

    fn periph_snapshot(&self) -> std::sync::Arc<periph::Snapshot> {
        self.periph
            .as_ref()
            .map(|(s, _)| s.snapshot())
            .unwrap_or_default()
    }

    /// Spusti / zastavi / znova spusti sluzbu periferii podla rozlozenia a nastaveni (zdroje, interval).
    fn sync_periph(&mut self) {
        let wanted = self.periph_allowed && self.periph_wanted();
        let cfg = self.config.peripherals.clone();
        // znova spustit len pri zmene zdrojov alebo intervalu (novy start zahodi posledne hodnoty a stav nabijania);
        // pocet zariadeni v liste sa uplatni pri kresleni
        let same = |c: &transom_config::Peripherals| {
            c.sources == cfg.sources && c.interval_secs == cfg.interval_secs
        };
        if wanted && self.periph.as_ref().is_none_or(|(_, c)| !same(c)) {
            let Some(n) = notifier() else { return };
            // vsetky zdroje vzdy: pouzivatel nic nevybera, transom zariadenia rozpozna sam
            let sources: Vec<periph::Source> = periph::Source::ALL.to_vec();
            let interval = std::time::Duration::from_secs(u64::from(cfg.interval_secs.max(10)));
            // stara sluzba skonci sama po zruseni (dokonci rozbehnute kolo)
            self.periph = None;
            match periph::start_system(
                sources,
                interval,
                std::sync::Arc::new(move || n.notify(N_PERIPH)),
            ) {
                Ok(s) => {
                    self.periph = Some((s, cfg));
                    log("periferie: sluzba spustena");
                }
                Err(e) => log(&format!("periferie: sluzbu sa nepodarilo spustit: {e}")),
            }
        } else if !wanted && self.periph.take().is_some() {
            log("periferie: sluzba zastavena");
            if popup::popover_open() == Some(POP_PERIPH) {
                popup::request(Request::ClosePopover);
            }
        }
    }

    /// Otvorenie panela periferii: cisty stav a nove kolo citania.
    fn periph_open(&mut self) {
        self.sync_periph();
        self.periph_panel.reset();
        if let Some((s, _)) = &self.periph {
            s.refresh();
        }
    }

    fn paint_periph(&mut self, p: &mut dyn transom_render::Painter) {
        let snap = self.periph_snapshot();
        self.periph_panel.paint(p, &self.theme, &snap);
        let (entries, extras, focus, name) = self.periph_panel.a11y();
        popup::set_panel_accessibility(entries, focus);
        transom_platform::menu_access::set_panel_extras(name, extras);
        if std::mem::take(&mut self.periph_panel.repaint) {
            popup::invalidate_popover();
        }
    }

    fn run_periph_cmd(&mut self, cmd: periph_pop::PeriphCmd) {
        match cmd {
            periph_pop::PeriphCmd::Refresh => {
                if let Some((s, _)) = &self.periph {
                    s.refresh();
                }
            }
            periph_pop::PeriphCmd::Settings => {
                popup::request(Request::ClosePopover);
                self.launch_settings(Some("peripherals"));
            }
        }
        popup::invalidate_popover();
    }

    /// Otvorenie panela Zvuk: cisty stav a znova nacitanie (jedina cesta obnovy po strate sluzby bez pollingu).
    fn audio_open(&mut self) {
        self.sync_audio();
        let snap = self.audio_snapshot();
        self.audio_panel.reset(&snap);
        if let Some(s) = &self.audio {
            s.refresh();
        }
        // mixer aplikacii a ikony aplikacii (len kym je panel otvoreny)
        self.ensure_icons();
        if self.mixer.is_none() {
            if let Some(n) = notifier() {
                match transom_mixer::start_system(
                    std::sync::Arc::new(move || n.notify(N_MIXER)),
                    t("mix.system"),
                ) {
                    Ok(m) => self.mixer = Some(m),
                    Err(e) => log(&format!("mixer: sluzbu sa nepodarilo spustit: {e}")),
                }
            }
        }
        if self.bt.is_none() {
            if let Some(n) = notifier() {
                match transom_mixer::bt::BtService::start(std::sync::Arc::new(move || {
                    n.notify(N_BT)
                })) {
                    Ok(b) => self.bt = Some(b),
                    Err(e) => log(&format!("bluetooth: sluzbu sa nepodarilo spustit: {e}")),
                }
            }
        }
    }

    /// Ikony aplikacii (vyhladavac, mixer): vytvoria sa pri prvom pouziti; zlyhane nacitania sa skusia znova.
    fn ensure_icons(&mut self) {
        match &self.icons {
            Some(i) => i.on_open(),
            None => {
                if let Some(n) = notifier() {
                    self.icons = Some(std::sync::Arc::new(icon_rt::IconRt::start(
                        std::sync::Arc::new(move || n.notify(N_ICONS)),
                    )));
                }
            }
        }
    }

    fn paint_tools(&mut self, p: &mut dyn transom_render::Painter) {
        // sekcia Snimky sa sklada pri kazdom kresleni z kniznice (zoznam aj hotove miniatury prichadzaju asynchronne)
        if self.tools_panel.section == tools_pop::Section::Shots {
            let cfg = &self.config.tools.screenshots;
            let box_px = self.tools_panel.thumb_box();
            self.tools_data.sections[3] = match self.library.as_mut() {
                Some(rt) => rt.section(cfg, box_px),
                None => tools_pop::SectionView {
                    enabled: cfg.enabled,
                    empty: Some(t(if cfg.enabled {
                        "tools.lib_loading"
                    } else {
                        "tools.lib_off"
                    })),
                    ..Default::default()
                },
            };
        }
        self.tools_data.group = self.tool_tiles();
        self.tools_panel.paint(p, &self.theme, &self.tools_data);
        // viditelne riadky bez miniatury: kniznica ich nacita na pozadi (zastarane poziadavky zahodi)
        let wanted = self.tools_panel.take_wanted();
        if let Some(rt) = &self.library {
            rt.request(&wanted, self.tools_panel.thumb_box());
        }
        let (entries, extras, focus, name) = self.tools_panel.a11y();
        popup::set_panel_accessibility(entries, focus);
        transom_platform::menu_access::set_panel_extras(name, extras);
        if std::mem::take(&mut self.tools_panel.repaint) {
            popup::invalidate_popover();
        }
    }

    /// Spusti sluzbu nastrojov (ulozisko a pracovne vlakno) a odber schranky podla nastaveni. Volane po kazdom nacitani
    /// konfiguracie. Schranka sa nesleduje, kym ju pouzivatel vyslovne nezapne (a pocas pozastavenia sa nesleduje vobec).
    fn sync_tools(&mut self) {
        use transom_tools::service::{Cmd, Service};
        if self.tools.is_none() {
            let Some(n) = notifier() else { return };
            let writer: transom_tools::service::ClipboardWriter = std::sync::Arc::new(|p| {
                use transom_platform::clipboard::{write, Put};
                use transom_tools::clipboard::Payload;
                write(&match p {
                    Payload::Text(t) => Put::Text(t),
                    Payload::Files(f) => Put::Files(f),
                    Payload::Image(b) => Put::Png(b),
                })
            });
            match Service::start(
                transom_tools::tools_dir(),
                self.config.tools.clone(),
                std::sync::Arc::new(move || n.notify(N_TOOLS)),
                writer,
            ) {
                Ok(s) => {
                    self.tools = Some(s);
                    log("nastroje: sluzba spustena");
                }
                Err(e) => log(&format!("nastroje: sluzbu sa nepodarilo spustit: {e}")),
            }
        }
        let Some(svc) = &self.tools else { return };
        // namapovane sietove disky sa rozpoznaju systemovym API (UNC textovo); sietova cesta sa nesonduje ani nekopiruje
        transom_tools::lane::set_remote_classifier(transom_platform::shellops::is_remote_drive);
        svc.send(Cmd::SetCfg(Box::new(self.config.tools.clone())));
        // vypnuty druh obsahu sa zo schranky nenacita vobec (obrazok sa zamietne uz podla hlavicky, pred kopiou a kodovanim)
        transom_platform::clipboard::set_read_policy(tools_bridge::read_policy(
            &self.config.tools.clipboard,
        ));
        let want = self.config.tools.clipboard.enabled && !self.config.tools.clipboard.paused;
        if want && self.clip_listener.is_none() {
            let tx = svc.sender();
            self.clip_listener =
                transom_platform::clipboard::Listener::start(std::sync::Arc::new(move |s| {
                    let _ = tx.send(Cmd::Clip(Box::new(tools_bridge::to_tools_snapshot(s))));
                }));
            log(&format!(
                "nastroje: odber schranky {}",
                if self.clip_listener.is_some() {
                    "zapnuty"
                } else {
                    "sa nepodarilo zapnut"
                }
            ));
        } else if !want && self.clip_listener.take().is_some() {
            log("nastroje: odber schranky zastaveny");
        }
        self.library_sync();
    }

    /// Kniznica snimok sleduje priecinky a nacitava zoznam **len kym je otvorena karta Snimky** v paneli Nastroje a kniznica je
    /// zapnuta; inak sa uvolnia uchytky priecinkov aj miniatury. Zmena nastaveni (priecinky, zapnutie) ju znovu otvori.
    fn library_sync(&mut self) {
        let cfg = self.config.tools.screenshots.clone();
        let want = ((self.tools_open
            && self.tools_panel.section == tools_pop::Section::Shots
            && self.tools_panel.editor.is_none())
            || self.island_shots_open())
            && cfg.enabled;
        if want && self.library.is_none() {
            let Some(n) = notifier() else { return };
            self.library = Some(library_rt::LibraryRt::start(std::sync::Arc::new(
                move || n.notify(N_LIB),
            )));
        }
        let Some(rt) = &mut self.library else { return };
        if want {
            if !rt.is_open() || self.lib_cfg.as_ref() != Some(&cfg) {
                rt.open(&cfg);
                self.lib_cfg = Some(cfg);
            }
        } else if rt.is_open() {
            rt.close();
            self.lib_cfg = None;
        }
    }

    /// Otvori snimku (predvolenou aplikaciou) alebo jej priecinok; nic sa nemeni. Cesta pochadza len zo zoznamu kniznice.
    fn open_shot(&mut self, i: usize, folder: bool) {
        let Some(e) = self.library.as_ref().and_then(|rt| rt.entry(i)) else {
            return;
        };
        let ok = if folder {
            transom_platform::shellops::open_containing(&e.path)
        } else {
            transom_platform::shellops::open_target(&e.path)
        };
        self.tools_panel.msg = (!ok).then(|| t("tools.lib_open_err"));
        self.tools_panel.repaint = true;
    }

    /// Zavrie editor poznamky; `save` ulozi zmeny (nova prazdna poznamka sa nevytvori).
    fn editor_finish(&mut self, save: bool) {
        use transom_tools::service::Cmd;
        let Some(e) = self.tools_panel.editor.take() else {
            return;
        };
        if save && e.dirty() && self.note_blocked(&e) {
            // ulozenie by zlyhalo (limit, vypnute / nepouzitelne ulozisko): text sa nezahodi, obnovi sa pri dalsom otvoreni
            self.note_draft = Some(e);
        } else if save && e.dirty() {
            match e.id {
                Some(id) => self.tools_send(Cmd::NoteUpdate { id, text: e.text }),
                None if !e.text.trim().is_empty() => self.tools_send(Cmd::NoteCreate(e.text)),
                None => {}
            }
        }
        self.tools_panel.repaint = true;
    }

    /// Zlyha ulozenie tejto poznamky podla stavu, ktory aplikacia pozna? (limit poctu, velkost, vypnute alebo poskodene ulozisko)
    fn note_blocked(&self, e: &tools_pop::Editor) -> bool {
        if e.text.trim().is_empty() {
            return false;
        }
        let cfg = &self.config.tools.notes;
        !cfg.enabled
            || self.tools_snap.unusable
            || e.text.len() > transom_tools::notes::MAX_NOTE_BYTES
            || (e.id.is_none() && self.tools_snap.notes.len() >= cfg.max_notes as usize)
    }

    fn on_tools_changed(&mut self) {
        if let Some(s) = &self.tools {
            self.tools_snap = s.snapshot();
            self.clip_changed();
            // otvoreny vybezok odlozenych suborov sleduje zoznam (novy subor, odstranenie)
            if transom_platform::dragdrop::notch_browsing() && self.notch_view == NotchView::Island
            {
                transom_platform::dragdrop::invalidate_notch();
            }
            if transom_platform::dragdrop::notch_browsing() && self.notch_view == NotchView::Stash {
                let (_, total) = self.stash_files();
                transom_platform::dragdrop::notch_browse(Some(stash_notch::height_lp(total)));
                transom_platform::dragdrop::invalidate_notch();
            }
            if self.drop_pending {
                use transom_tools::service::Msg;
                if let Some(m) =
                    self.tools_snap.msg.clone().filter(|m| {
                        matches!(m, Msg::Added { .. } | Msg::Rejected(_) | Msg::Cleared(_))
                    })
                {
                    self.drop_pending = false;
                    if let Some(text) = tools_bridge::msg_text(&m) {
                        self.show_notice(text);
                    }
                }
            }
            self.tools_data = tools_bridge::view(&self.tools_snap);
            // otvorena poznamka na upravu (snimka po `NoteOpen`)
            if let Some((id, text)) = self.tools_snap.opened.clone() {
                if self.tools_panel.editor.is_none() {
                    self.tools_panel.open_editor(Some(id), text);
                }
                // otvorenu poznamku sluzba nesie, kym ju nepotvrdime (snimka sa nemoze stratit)
                if let Some(svc) = &self.tools {
                    svc.send(transom_tools::service::Cmd::NoteOpenDone);
                }
            }
        }
        popup::invalidate_popover();
    }

    /// Pridanie suborov do Police: rovnaka cesta pre pustenie na listu aj pre dialog (kontrola ciest, duplicity, limity; originaly
    /// sa nikdy nemenia). Vysledok sa ukaze ako oznam na liste.
    fn add_to_shelf(&mut self, paths: Vec<String>) {
        self.drop_pending = true;
        self.tools_send(transom_tools::service::Cmd::AddPaths { paths, copy: false });
    }

    /// Dialog "Pridat subory do Police" na vlastnom vlakne (modalny, caka na pouzivatela; okno Nastroje sa pritom zavrie ako pri
    /// kazdom inom okne). Najviac jeden dialog naraz.
    fn pick_shelf_files(&mut self) {
        if self.picking {
            return;
        }
        let Some(n) = notifier() else { return };
        let (owner, title) = (self.bar_hwnd, t("tools.shelf_pick_title"));
        let slot = std::sync::Arc::clone(&self.picked);
        let spawned = std::thread::Builder::new()
            .name("transom-pick".into())
            .spawn(move || {
                let paths =
                    transom_platform::filepick::pick_files(owner, &title).unwrap_or_default();
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(paths);
                n.notify(N_PICKED);
            });
        self.picking = spawned.is_ok();
    }

    fn tools_send(&self, c: transom_tools::service::Cmd) {
        if let Some(s) = &self.tools {
            s.send(c);
        }
    }

    /// Pripravi vyhladavac: prazdny panel a beh zdrojov (vytvara sa pri prvom otvoreni a po zmene nastaveni vyhladavaca).
    fn launcher_open(&mut self) {
        self.launcher_panel.reset();
        // ikony vysledkov: vytvoria sa pri prvom otvoreni; zlyhane nacitania sa pri kazdom otvoreni skusia znova
        self.ensure_icons();
        let cfg = self.config.tools.launcher.clone();
        if self.launcher.as_ref().is_some_and(|r| r.cfg != cfg) {
            self.launcher = None;
        } else if let Some(rt) = &self.launcher {
            rt.refresh_apps();
        }
        if self.launcher.is_none() && cfg.enabled {
            let Some(n) = notifier() else { return };
            let n3 = n;
            // vlastna schranka pre kazdy beh: oneskoreny vysledok stareho behu nepomyli generacie noveho
            self.launcher_box = std::sync::Arc::default();
            let mailbox = self.launcher_box.clone();
            let recents = self.launcher_recents.clone();
            if cfg.recent_searches {
                let r = transom_tools::recents::Recents::open(
                    transom_tools::tools_dir().join("launcher"),
                );
                if let Ok(mut g) = recents.lock() {
                    *g = r.list();
                }
            } else if let Ok(mut g) = recents.lock() {
                g.clear();
            }
            let rt = launcher_rt::build(
                &cfg,
                std::sync::Arc::new(move |u| {
                    if let Ok(mut g) = mailbox.lock() {
                        *g = Some(u);
                    }
                    n.notify(N_LAUNCHER);
                }),
                std::sync::Arc::new(move || recents.lock().map(|g| g.clone()).unwrap_or_default()),
                std::sync::Arc::new(move || n3.notify(N_LAUNCHER_APPS)),
            );
            self.launcher = Some(rt);
            self.sync_launcher_lists();
        }
        if let Some(rt) = &self.launcher {
            self.launcher_panel.files_pending = rt.files.is_some();
            if let Some(f) = &rt.files {
                self.launcher_panel.index_down =
                    f.index_state() == transom_launcher::sources::IndexState::Unavailable;
            }
            // prazdny dotaz ukaze akcie a nastavenia (ziadne subory)
            let g = rt.engine.query("");
            self.launcher_panel.generation = g;
        }
    }

    fn sync_launcher_lists(&mut self) {
        if let Some(rt) = &self.launcher {
            let (notes, shelf) = launcher_rt::tools_items(&self.tools_snap);
            rt.lists.notes.set(notes);
            rt.lists.shelf.set(shelf);
        }
    }

    fn launcher_flash(&mut self, text: String) {
        self.launcher_panel.flash = Some(text);
        popup::invalidate_popover();
    }

    fn on_launcher_results(&mut self) {
        let u = self.launcher_box.lock().ok().and_then(|mut g| g.take());
        if let Some(u) = u {
            let rt_down = self
                .launcher
                .as_ref()
                .and_then(|r| r.files.as_ref())
                .is_some_and(|f| {
                    f.index_state() == transom_launcher::sources::IndexState::Unavailable
                });
            self.launcher_panel.index_down = rt_down;
            if self
                .launcher_panel
                .set_results(u.generation, u.hits, u.done)
            {
                popup::invalidate_popover();
            }
        }
        // prazdna schranka = vysledok uz prebrala skorsia notifikacia (jedna notifikacia na aktualizaciu, schranka drzi len poslednu);
        // **nikdy** sa tu nespusta novy dotaz: obnovu po nacitani aplikacii riesi `N_LAUNCHER_APPS`
    }

    fn run_launcher_cmd(&mut self, cmd: launcher_pop::LauncherCmd) {
        use launcher_pop::LauncherCmd as C;
        match cmd {
            C::Query(q) => {
                if let Some(rt) = &self.launcher {
                    let g = rt.engine.query(&q);
                    self.launcher_panel.generation = g;
                }
                popup::invalidate_popover();
            }
            C::Paste => {
                if let Some(text) = transom_platform::clipboard::read_text() {
                    if let Some(c) = self.launcher_panel.paste(&text) {
                        self.run_launcher_cmd(c);
                    }
                }
            }
            C::Do(i, act) => self.launcher_do(i, act),
        }
    }

    /// Vykona zvolenu akciu nad vysledkom. Cielom je vzdy hotova polozka zo zdroja (nikdy text dotazu).
    fn launcher_do(&mut self, i: usize, act: transom_launcher::Act) {
        use transom_launcher::{Act, Target};
        let Some(hit) = self.launcher_panel.results.get(i).cloned() else {
            return;
        };
        let item = hit.item;
        let ok = match (act, &item.target) {
            (Act::Open, Target::Path(p)) => transom_platform::shellops::open_target(p),
            (Act::Open, Target::Uri(u)) => {
                u.starts_with("ms-settings:") && transom_platform::shellops::open_target(u)
            }
            (Act::Open, Target::Action(id)) => self.launcher_action(id),
            (Act::Open, Target::Note(_)) => self.launcher_tools_section(tools_pop::Section::Notes),
            (Act::Open, Target::Shelf(_)) => self.launcher_tools_section(tools_pop::Section::Shelf),
            (Act::OpenContaining, Target::Path(p)) => {
                transom_platform::shellops::open_containing(p)
            }
            (Act::CopyPath, Target::Path(p)) => {
                let done = transom_platform::clipboard::write(
                    &transom_platform::clipboard::Put::Text(p.clone()),
                );
                if done {
                    self.launcher_flash(t("lau.copied"));
                    return;
                }
                false
            }
            _ => false,
        };
        if !ok {
            self.launcher_flash(t("lau.open_failed"));
            self.sound.cue(snd::Origin::Popover, snd::Outcome::Failed);
            return;
        }
        // poznamky a polozky Police sa do nedavnych nezapisuju (citlive nazvy by prezili ich vymazanie)
        if self.config.tools.launcher.recent_searches
            && !matches!(item.target, Target::Note(_) | Target::Shelf(_))
        {
            let mut r =
                transom_tools::recents::Recents::open(transom_tools::tools_dir().join("launcher"));
            r.push(&item.title);
            if let Ok(mut g) = self.launcher_recents.lock() {
                *g = r.list();
            }
        }
        self.sound.cue(snd::Origin::Popover, snd::Outcome::Done);
        // okno sa zavrie, ak akcia neotvorila iny popover (ten ho nahradil)
        if matches!(item.target, Target::Path(_) | Target::Uri(_)) || act != Act::Open {
            popup::request(Request::ClosePopover);
        }
    }

    fn launcher_tools_section(&mut self, s: tools_pop::Section) -> bool {
        let at = self
            .popover_monitor
            .clone()
            .or_else(|| self.layouts.keys().next().cloned())
            .map(|m| (m, 0));
        // `act` ulozi rozpisanu poznamku a prip. obnovi neulozeny koncept; pozadovana sekcia sa nastavi len ak editor nie je otvoreny
        let _ = self.act(Action::ToolsPanel, at);
        if self.tools_panel.editor.is_none() {
            self.tools_panel.reset(s);
        }
        self.library_sync();
        true
    }

    fn launcher_action(&mut self, id: &str) -> bool {
        if let Some(sec) = launcher_rt::tools_section(id) {
            return self.launcher_tools_section(sec);
        }
        match id {
            "settings" => {
                popup::request(Request::ClosePopover);
                self.on_menu(MENU_SETTINGS);
            }
            "layout" => {
                popup::request(Request::ClosePopover);
                self.on_menu(MENU_CUSTOMIZE);
            }
            "ai" => {
                let _ = self.act(Action::AiPanel, None);
            }
            "audio" => {
                let _ = self.act(Action::AudioPanel, None);
            }
            "peripherals" => {
                let _ = self.act(Action::PeripheralsPanel, None);
            }
            "calendar" => {
                let _ = self.act(Action::Calendar, None);
            }
            _ => return false,
        }
        true
    }

    fn paint_launcher(&mut self, p: &mut dyn transom_render::Painter) {
        let t0 = Instant::now();
        let icons: Option<&dyn icon_rt::IconSource> =
            self.icons.as_deref().map(|i| i as &dyn icon_rt::IconSource);
        self.launcher_panel.paint_with(p, &self.theme, icons);
        let t1 = Instant::now();
        let (entries, extras, focus, name) = self.launcher_panel.a11y();
        popup::set_panel_accessibility(entries, focus);
        transom_platform::menu_access::set_panel_extras(name, extras);
        let (draw, acc) = (t1 - t0, t1.elapsed());
        if draw.as_millis() + acc.as_millis() >= 50 {
            log(&format!(
                "POMALE KRESLENIE vyhladavaca kreslenie={} ms pristupnost={} ms",
                draw.as_millis(),
                acc.as_millis()
            ));
        }
        if std::mem::take(&mut self.launcher_panel.repaint) {
            popup::invalidate_popover();
        }
    }

    /// Prikaz panela Nastroje. Hladanie otvara vstavany vyhladavac transom (Windows Search ostava len ako volitelna rychla akcia).
    fn run_tools_cmd(&mut self, cmd: tools_pop::ToolsCmd) {
        use tools_pop::ToolsCmd as C;
        match cmd {
            C::Search => {
                let o = self.act(Action::Launcher, None);
                self.sound.cue(snd::Origin::Popover, o);
            }
            C::ShelfAdd => self.pick_shelf_files(),
            C::Tile(id) => self.tool_tile(id),
            // spat na dlazdice: panel sa otvori znova v nizkej podobe
            C::Home => self.tools_home(),
            // snimky (iba citanie): otvorit, ukazat v priecinku, obnovit; nic sa nemeni ani nemaze
            C::Activate(tools_pop::Section::Shots, i) => self.open_shot(i, false),
            C::ShowInFolder(_, i) => self.open_shot(i, true),
            C::Refresh => {
                if let Some(rt) = &self.library {
                    rt.refresh();
                }
            }
            C::DragOut(tools_pop::Section::Shots, i) => {
                // tahanie von je len kopia (nikdy presun); cloud a sietove subory sa netahaju
                match self.library.as_ref().and_then(|rt| rt.entry(i)) {
                    Some(e) if !e.remote && !e.cloud => {
                        popup::request(Request::DragOut(vec![e.path]));
                    }
                    Some(_) => self.tools_panel.msg = Some(t("tools.net_drag")),
                    None => {}
                }
            }
            C::NoteNew => self.tools_panel.open_editor(None, String::new()),
            C::EditorSave => {
                // limit poznamok: editor ostava otvoreny a text sa nestrati
                let blocked = self
                    .tools_panel
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.dirty() && self.note_blocked(e));
                if blocked {
                    self.tools_panel.msg = Some(t("tools.note_err"));
                    self.tools_panel.repaint = true;
                } else {
                    self.tools_panel.msg = None;
                    self.editor_finish(true);
                }
            }
            C::EditorDiscard => self.editor_finish(false),
            C::EditorCopy => {
                if let Some(e) = &self.tools_panel.editor {
                    let text = e.text.clone();
                    self.tools_send(transom_tools::service::Cmd::Put(
                        transom_tools::clipboard::Payload::Text(text),
                    ));
                }
            }
            C::DragOut(sec, i) => {
                let paths = tools_bridge::drag_paths(sec, i, &self.tools_snap);
                if !paths.is_empty() {
                    popup::request(Request::DragOut(paths));
                } else if tools_bridge::is_remote_row(sec, i, &self.tools_snap) {
                    // sietova polozka sa netaha (shell by ju pri nedostupnom serveri zablokoval UI): vysvetlenie namiesto ticha
                    self.tools_panel.msg = Some(t("tools.net_drag"));
                }
            }
            other => {
                if let Some(c) = tools_bridge::plan(other, &self.tools_snap) {
                    self.tools_send(c);
                }
            }
        }
        popup::invalidate_popover();
    }

    fn paint_audio(&mut self, p: &mut dyn transom_render::Painter) {
        let snap = self.audio_snapshot();
        let quick = self.config.audio_panel.quick_actions.clone();
        let mix = self.mixer.as_ref().map(|m| m.snapshot());
        self.audio_panel.bt = self.bt.as_ref().map(|b| b.snapshot());
        self.audio_panel.profiles = self
            .config
            .audio_profiles
            .iter()
            .filter_map(|p| {
                transom_config::AUDIO_PROFILE_SLOTS
                    .iter()
                    .position(|s| *s == p.slot)
                    .map(|i| i as u8)
            })
            .collect();
        self.audio_panel.profiles.sort_unstable();
        let icons: Option<&dyn icon_rt::IconSource> =
            self.icons.as_deref().map(|i| i as &dyn icon_rt::IconSource);
        self.audio_panel
            .paint_with(p, &self.theme, &snap, &quick, mix, icons);
        let (entries, extras, focus, name) = self.audio_panel.a11y();
        popup::set_panel_accessibility(entries, focus);
        transom_platform::menu_access::set_panel_extras(name, extras);
        if std::mem::take(&mut self.audio_panel.repaint) {
            popup::invalidate_popover();
        }
    }

    /// Vykona prikaz panela Zvuk. Zapis hlasitosti a stlmenia ide do sluzby (Core Audio); tlacidla len **otvaraju**
    /// povrchy Windows a hlasia "poziadane": ani vystup, ani nastavenie sa netvrdi ako zmenene.
    fn run_audio_cmd(&mut self, cmd: audio_pop::AudioCmd) {
        use audio_pop::AudioCmd as C;
        match cmd {
            C::SetVolume(v) => {
                if let Some(s) = &self.audio {
                    s.set_volume(v);
                }
            }
            C::SetMute(m) => {
                if let Some(s) = &self.audio {
                    s.set_mute(m);
                }
            }
            C::Picker => {
                self.audio_shortcut(Shortcut::OutputPicker, t("audio.picker"), "Win+Ctrl+V")
            }
            C::SoundSettings => {
                popup::request(Request::Launch {
                    path: "ms-settings:sound".into(),
                    args: String::new(),
                });
                self.audio_requested(t("audio.sound_settings"), snd::Outcome::Done);
            }
            C::AppVolume(k, v) => {
                if let Some(m) = &self.mixer {
                    m.set_volume(k, v);
                }
            }
            C::AppMute(k, on) => {
                if let Some(m) = &self.mixer {
                    m.set_mute(k, on);
                }
            }
            C::AppRoute(k, i) => {
                if let Some(m) = &self.mixer {
                    let snap = m.snapshot();
                    let dev = match i {
                        None => Some(None),
                        Some(i) => snap.devices.get(usize::from(i)).map(|d| Some(d.id.clone())),
                    };
                    if let Some(dev) = dev {
                        log(&format!(
                            "mixer: vystup aplikacie -> {}",
                            if dev.is_some() {
                                "zariadenie"
                            } else {
                                "predvoleny"
                            }
                        ));
                        m.set_route(k, dev);
                    }
                }
            }
            C::ProfApply(i) => self.profile_apply(i),
            C::BtSet(i, connect) => {
                if let Some(b) = &self.bt {
                    if let Some(d) = b.snapshot().devices.get(usize::from(i)) {
                        log(&format!(
                            "bluetooth: {} zariadenia",
                            if connect { "pripojenie" } else { "odpojenie" }
                        ));
                        b.set(&d.key, connect);
                    }
                }
            }
            C::ProfSave(i) => self.profile_save(i),
            C::ProfDelete(i) => self.profile_delete(i),
            C::AppsSettings => {
                popup::request(Request::Launch {
                    path: "ms-settings:apps-volume".into(),
                    args: String::new(),
                });
                self.audio_requested(t("mix.apps_settings"), snd::Outcome::Done);
            }
            C::Quick(id) => match id {
                "quick_settings" => {
                    self.audio_shortcut(Shortcut::QuickSettings, t("qa.quick_settings"), "Win+A")
                }
                "notifications" => {
                    self.audio_shortcut(Shortcut::Notifications, t("qa.notifications"), "Win+N")
                }
                "search" => {
                    let o = self.act(Action::Search, None);
                    self.audio_requested(t("qa.search"), o);
                }
                "tray" => {
                    let o = self.act(Action::Tray, None);
                    self.audio_requested(t("qa.tray"), o);
                }
                "settings" => {
                    self.launch_settings(None);
                    self.audio_requested(t("qa.settings"), snd::Outcome::Done);
                }
                _ => {}
            },
        }
        popup::invalidate_popover();
    }

    /// Odoslanie klavesovej skratky Windows: pri blokovani (zvyseny proces, drzane modifikatory) sa oznami chyba,
    /// inak len "poziadane" (otvorenie Windows nevieme potvrdit).
    fn audio_shortcut(&mut self, s: Shortcut, what: String, keys: &str) {
        match input::send_shortcut(s) {
            Ok(before) => {
                log(&format!(
                    "zvuk: skratka {keys} odoslana, popredie pred: {before:?}"
                ));
                self.audio_requested(what, snd::Outcome::Done);
            }
            Err(b) => {
                let text = blocked_text(b, &what, keys);
                self.show_notice(text.clone());
                self.audio_panel.msg = Some(text);
            }
        }
    }

    fn audio_requested(&mut self, what: String, outcome: snd::Outcome) {
        if outcome == snd::Outcome::Done {
            self.audio_panel.msg = Some(tf("audio.requested", &[&what]));
        }
        // umysel pouzivatela, uspesne odoslane: tichy zvuk rozhrania (mute a posuvnik sa nikdy neozvu)
        self.sound.cue(snd::Origin::Popover, outcome);
    }

    fn refresh_battery(&mut self) {
        self.battery = power::read().and_then(|p| {
            Battery::from_raw(PowerRaw {
                ac_line: p.ac_line,
                battery_flag: p.battery_flag,
                percent: p.percent,
                saver: p.status_flag,
            })
        });
    }

    /// Znova zisti temu (pri `System`); `true`, ak sa zmenila.
    fn refresh_theme(&mut self) -> bool {
        let t = self.mode.resolve(system_uses_dark());
        let changed = t != self.theme;
        self.theme = t;
        changed
    }

    /// Prvy zber vyuzitia AI az ked existuje hostitel (notifier), potom periodicky.
    fn ensure_usage(&mut self) {
        if self.ai_started || notifier().is_none() {
            return;
        }
        self.ai_started = true;
        if self.config.usage.any_enabled() && self.ai.is_old(300) {
            self.ai
                .refresh(&self.config.usage, notifier(), ai::RefreshKind::Auto);
        }
        self.schedule_usage();
    }

    fn ensure_media(&mut self) {
        if self.media.is_some() || !self.media_enabled {
            return;
        }
        let Some(n) = notifier() else { return };
        self.media = Some(MediaService::start(move || n.notify(N_MEDIA), log));
        log("media vlakno spustene");
    }

    fn reselect(&mut self) {
        let previous = self.selection.chosen.clone();
        self.selection = select(
            &self.snapshot.sessions,
            self.prefer_app(),
            self.user_pick.as_deref(),
            previous.as_deref(),
        );
        // zvolena pouzivatelom zmizla -> zabudnut
        if let Some(p) = &self.user_pick {
            if !self
                .snapshot
                .sessions
                .iter()
                .any(|s| &s.key == p && s.has_metadata())
            {
                self.user_pick = None;
            }
        }
        // obal patri zvolenej session a jej aktualnej skladbe (art_id riesi media vlakno cez resolve_art)
        let id = self
            .selection
            .chosen
            .as_ref()
            .and_then(|k| self.snapshot.sessions.iter().find(|s| &s.key == k))
            .and_then(|s| s.art_id);
        if self.art.as_ref().map(|a| a.id) != id {
            self.art = id.and_then(|id| {
                self.media
                    .as_ref()
                    .and_then(|m| m.image(id))
                    .map(|image| ArtImage { id, image })
            });
        }
    }

    /// Napoveda pre Spotify: proces sa kontroluje len ked GSMTC nema Spotify session (pri media udalosti a pri
    /// minutovom ticku hodin - ziadne samostatne prebudzanie).
    fn check_spotify(&mut self) {
        if !self.media_enabled {
            return;
        }
        let has_session = self
            .snapshot
            .sessions
            .iter()
            .any(|s| s.app_id.to_lowercase().contains("spotify"));
        let running = if has_session {
            true
        } else {
            process::is_running("Spotify.exe")
        };
        if !running {
            self.spotify_hint_dismissed = false;
        }
        self.spotify_running = running;
    }

    fn on_media(&mut self) {
        // otvoreny vybezok s hudbou sleduje skladbu (nazov, stav, priebeh)
        if matches!(self.notch_view, NotchView::Media | NotchView::Island)
            && transom_platform::dragdrop::notch_browsing()
        {
            transom_platform::dragdrop::invalidate_notch();
        }
        let Some(m) = &self.media else { return };
        let snap = m.snapshot();
        self.update("media", |a| {
            a.snapshot = snap;
            a.reselect();
            a.check_spotify();
        });
        let s = &self.snapshot;
        log(&format!(
            "media gen={} obnoveni={} chyby_prikazov={} nahlady: citania={} dekodovania={} cache={} sessions=[{}] vyber={:?} obal={:?} kandidati={:?}{}",
            s.generation,
            s.refreshes,
            s.commands_failed,
            s.thumb_reads,
            s.decodes,
            s.cache_hits,
            s.sessions
                .iter()
                .map(|x| format!(
                    "{}:{:?}:{:?}:art={:?}:tl={}",
                    x.key,
                    x.state,
                    x.title,
                    x.art_id.map(|a| format!("{a:016x}")),
                    if x.timeline.valid().is_some() {
                        "ok"
                    } else {
                        "neplatna"
                    }
                ))
                .collect::<Vec<_>>()
                .join(", "),
            self.selection.chosen,
            self.art.as_ref().map(|a| format!("{:016x}", a.id)),
            self.selection.candidates,
            s.error
                .as_deref()
                .map(|e| format!(" chyba={e}"))
                .unwrap_or_default()
        ));
    }

    fn tick(&mut self, why: &str) {
        self.event_check();
        self.watch_tick();
        let secs = self.config.clock.seconds;
        if secs && why == "wake" {
            // sekundovy tik: ak sa zmenil len text hodin a ma rovnaku dlzku (cislice Segoe UI su rovnako siroke),
            // prekreslit len usek hodin; zmenu minuty a dlzky riesi plna cesta nizsie
            let new = clock_text(&self.config.clock);
            // nova minuta (aj pri 12 h s AM/PM) podla skutocnej sekundy, nie podla textu
            let minute = pclock::current_second() == 0;
            if !minute && new.chars().count() == self.clock.chars().count() {
                if new != self.clock {
                    self.clock = new;
                    for (m, placed) in &self.layouts {
                        if let Some(p) = placed.iter().find(|p| p.item == ItemId::Clock) {
                            transom_platform::invalidate_span(m, p.x, p.w);
                        }
                    }
                }
                schedule_wake(next_wake_delay_for(true));
                return;
            }
        }
        self.sync_shift();
        if !self.keys_polling {
            self.sync_keys();
        }
        self.sync_bright(false);
        self.sync_sysmon();
        self.sync_privacy(false);
        self.update(why, |a| {
            a.clock = clock_text(&a.config.clock);
            // proces Spotify sa kontroluje len raz za minutu (aj pri sekundovych hodinach)
            if !secs || pclock::current_second() == 0 || why != "wake" {
                a.check_spotify();
            }
        });
        // obmedzeny pohyb vo Windows: znova precitat aj bez udalosti (nezavisle od toho, co Windows rozosle)
        if (!secs || pclock::current_second() == 0 || why != "wake")
            && self.reduced_motion == transom_platform::system::animations_enabled()
        {
            self.reduced_motion = !self.reduced_motion;
            log(&format!("animacie v systeme: {}", !self.reduced_motion));
            invalidate_all();
            self.kick_ticker();
        }
        // novy zaznam mostu statusline (lacne metadata, raz za minutu) -> obnovit na pozadi
        // (brana 5 min v `AiState::refresh`: ak este nemoze, skusi sa pri dalsom tiku)
        if self.ai.bridge_changed(&self.config.usage)
            && self
                .ai
                .refresh(&self.config.usage, notifier(), ai::RefreshKind::Auto)
        {
            log("vyuzitie AI: novy zaznam statusline");
        }
        // polnoc / zmena casu: dnesok v otvorenom kalendari
        popup::invalidate_popover();
        let delay = next_wake_delay_for(secs);
        if !secs || why != "wake" {
            log(&format!(
                "{why}: clock='{}' dalsie prebudenie za {delay} ms",
                self.clock
            ));
        }
        schedule_wake(delay);
    }

    fn show_notice(&mut self, text: String) {
        log(&format!("oznam: {text}"));
        self.update("oznam", |a| a.notice = Some(text));
        schedule_timer(T_NOTICE, NOTICE_MS);
    }

    /// `at` = (monitor, stred kliknutej casti v px listy) - pre ukotvenie popoveru.
    /// Vrati vysledok pre zvuk rozhrania: `Done` = akcia sa vykonala alebo uspesne odoslala, `Failed` = nebolo co
    /// vykonat alebo ju system zablokoval (vtedy sa nehra).
    fn act(&mut self, action: Action, at: Option<(String, i32)>) -> snd::Outcome {
        log(&format!("akcia {action:?}"));
        let mut outcome = snd::Outcome::Done;
        match action {
            // vypnuty vstavany vyhladavac: tlacidlo Search nesmie otvorit nefunkcne okno, pouzije sa Windows Search
            Action::Launcher if !self.config.tools.launcher.enabled => {
                return self.act(Action::Search, at);
            }
            // bez indexu suboru (klik a MSAA ho riesia samy cez `shelf_click`): prvy subor
            Action::ShelfFile => return self.shelf_click(0),
            Action::Island => {
                self.island_click(at);
                return snd::Outcome::Done;
            }
            Action::SnipToStash => return self.snip_start(),
            Action::ColorPick => return self.pick_start(),
            Action::FocusPanel => {
                // Windows nema podporovane API na prepnutie Nerusit / Sustredenia: otvori ich vlastny panel
                return match transom_platform::input::send_shortcut(
                    transom_platform::input::Shortcut::Notifications,
                ) {
                    Ok(_) => snd::Outcome::Done,
                    Err(_) => snd::Outcome::Failed,
                };
            }
            Action::DesktopNext => {
                return match self.desktop_info {
                    Some((n, cur)) if n > 1 => {
                        let ok = transom_platform::desktops::switch_to(cur, (cur + 1) % n);
                        schedule_timer(T_WATCH, 300);
                        self.watching = true;
                        if ok {
                            snd::Outcome::Done
                        } else {
                            snd::Outcome::Failed
                        }
                    }
                    _ => snd::Outcome::Failed,
                };
            }
            Action::Calendar
            | Action::AiPanel
            | Action::AudioPanel
            | Action::PeripheralsPanel
            | Action::ToolsPanel
            | Action::ToolsShelf
            | Action::ToolsClipboard
            | Action::ToolsNotes
            | Action::ToolsShots
            | Action::TimerPanel
            | Action::BrightnessPanel
            | Action::PrivacyPanel
            | Action::WeatherOpen
            | Action::UpdatePanel
            | Action::Launcher => {
                let Some((monitor_id, x)) = at
                    .or_else(|| self.popover_monitor.clone().map(|m| (m, 0)))
                    .or_else(|| self.layouts.keys().next().map(|m| (m.clone(), 0)))
                else {
                    return snd::Outcome::Failed;
                };
                if action == Action::WeatherOpen
                    && self.weather.as_ref().and_then(|w| w.snapshot()).is_none()
                {
                    return snd::Outcome::Failed;
                }
                if action == Action::UpdatePanel && self.update_avail.is_none() {
                    return snd::Outcome::Failed;
                }
                self.popover_anchor = Some((monitor_id.clone(), x));
                self.weather_back = false;
                let (kind, w, h) = if action == Action::WeatherOpen {
                    (
                        POP_WEATHER,
                        transom_platform::dragdrop::NOTCH_W_LP,
                        weather::NOTCH_H_LP,
                    )
                } else if action == Action::Calendar {
                    self.cal.reset(pclock::today());
                    if let Some(s) = &self.ics {
                        s.on_open();
                    }
                    (
                        POP_CALENDAR,
                        transom_render::calendar::CAL_W_LP,
                        transom_render::calendar::CAL_H_LP,
                    )
                } else if action == Action::AudioPanel {
                    self.audio_open();
                    (POP_AUDIO, audio_pop::AUDIO_W_LP, audio_pop::AUDIO_H_LP)
                } else if let Some(k) = kit_kind_for(action) {
                    self.kit_open(k);
                    let h = match self.update_view().filter(|_| k == POP_UPDATE) {
                        Some(v) => update_pop::height_lp(&v, extras::KIT_W_LP),
                        None => extras::kit_h(k),
                    };
                    (k, extras::KIT_W_LP, h)
                } else if action == Action::PeripheralsPanel {
                    self.periph_open();
                    (POP_PERIPH, periph_pop::PERIPH_W_LP, periph_pop::PERIPH_H_LP)
                } else if action == Action::Launcher {
                    self.launcher_open();
                    (
                        POP_LAUNCHER,
                        launcher_pop::LAUNCHER_W_LP,
                        launcher_pop::LAUNCHER_H_LP,
                    )
                } else if let Some(sec) = tools_section(action) {
                    // samostatne tlacidlo nastroja pri otvorenom paneli: len prepne cast (nezatvori panel)
                    if self.tools_open
                        && action != Action::ToolsPanel
                        && !self.tools_panel.home
                        && self.tools_panel.section != sec
                        && popup::popover_open() == Some(POP_TOOLS)
                    {
                        self.editor_finish(true);
                        self.tools_panel.reset(sec);
                        popup::invalidate_popover();
                        return snd::Outcome::Done;
                    }
                    // rozpisana poznamka sa pred vynulovanim panela ulozi (klik na polozku lisety nedeaktivuje okno)
                    self.editor_finish(true);
                    self.tools_panel.reset(sec);
                    // tlacidlo Nastroje: uvod s dlazdicami; tlacidla jednotlivych nastrojov rovno ich sekciu
                    self.tools_panel.home = action == Action::ToolsPanel;
                    self.tools_open = true;
                    if let Some(d) = self.note_draft.take() {
                        // neulozena poznamka z minuleho zatvorenia: znovu sa otvori v editore s vysvetlenim
                        self.tools_panel.reset(tools_pop::Section::Notes);
                        self.tools_panel.editor = Some(d);
                        self.tools_panel.msg = Some(t("tools.note_err"));
                    }
                    // uvod s dlazdicami je nizky (podla poctu riadkov), sekcia ma plnu vysku
                    let h = if self.tools_panel.home {
                        let n = self.tool_tiles().len() as i32;
                        tools_pop::home_h_lp(n).min(tools_pop::TOOLS_H_LP)
                    } else {
                        tools_pop::TOOLS_H_LP
                    };
                    (POP_TOOLS, tools_pop::TOOLS_W_LP, h)
                } else {
                    // pri otvoreni obnovit, ak su data starsie nez 5 minut (zber bezi mimo UI vlakna)
                    if self.ai.is_old(300) {
                        self.ai
                            .refresh(&self.config.usage, notifier(), ai::RefreshKind::Auto);
                    }
                    self.ai.on_open();
                    (POP_AI, ai::AI_W_LP, ai::AI_H_LP)
                };
                self.popover_monitor = Some(monitor_id.clone());
                popup::request(Request::ShowPopover {
                    kind,
                    monitor_id,
                    anchor_x: x,
                    w_lp: w,
                    h_lp: h,
                });
            }
            Action::KeyboardSwitch => match input::send_shortcut(Shortcut::InputSwitch) {
                Ok(_) => {}
                Err(b) => {
                    self.show_notice(blocked_text(b, &t("item.keyboard"), "Win+Space"));
                    outcome = snd::Outcome::Failed;
                }
            },
            Action::SysMonOpen => {
                popup::request(Request::Launch {
                    path: "taskmgr.exe".into(),
                    args: String::new(),
                });
            }

            Action::MediaOpenSource => {
                let Some(s) = self
                    .selection
                    .chosen
                    .as_ref()
                    .and_then(|k| self.snapshot.sessions.iter().find(|s| &s.key == k))
                else {
                    return snd::Outcome::Failed;
                };
                log(&format!("aktivacia zdroja: app='{}'", s.app_id));
                popup::request(Request::Activate {
                    app: s.app_id.clone(),
                    title: s.title.clone(),
                });
            }
            Action::Search => match input::send_shortcut(Shortcut::Search) {
                Ok(before) => {
                    log(&format!("Win+S odoslane, popredie pred: {before:?}"));
                    self.pending = Some(Pending::Search { before });
                    schedule_timer(T_VERIFY, VERIFY_SEARCH_MS);
                }
                Err(b) => {
                    self.show_notice(blocked_text(b, "Search", "Win+S"));
                    outcome = snd::Outcome::Failed;
                }
            },
            Action::Tray => match input::send_shortcut(Shortcut::Tray) {
                Ok(before) => {
                    log(&format!("Win+B odoslane, popredie pred: {before:?}"));
                    self.pending = Some(Pending::TrayFocus);
                    schedule_timer(T_VERIFY, VERIFY_TRAY_MS);
                }
                Err(b) => {
                    self.show_notice(blocked_text(b, "Tray", "Win+B"));
                    outcome = snd::Outcome::Failed;
                }
            },
            Action::MediaPrev | Action::MediaNext | Action::MediaToggle => {
                let Some(key) = self.selection.chosen.clone() else {
                    return snd::Outcome::Failed;
                };
                let Some(s) = self.snapshot.sessions.iter().find(|s| s.key == key) else {
                    return snd::Outcome::Failed;
                };
                let cmd = match action {
                    Action::MediaPrev => Some(Command::Prev),
                    Action::MediaNext => Some(Command::Next),
                    _ => toggle_command(s).map(|t| match t {
                        ToggleCmd::Toggle => Command::Toggle,
                        ToggleCmd::Play => Command::Play,
                        ToggleCmd::Pause => Command::Pause,
                    }),
                };
                if let (Some(c), Some(m)) = (cmd, &self.media) {
                    log(&format!("media prikaz {c:?} -> {key}"));
                    m.command(&key, c);
                } else {
                    outcome = snd::Outcome::Failed;
                }
            }
            Action::MediaCycle => {
                self.update("prepnutie session", |a| {
                    a.user_pick = a.selection.next_candidate();
                    a.reselect();
                });
                log(&format!("pouzivatel zvolil {:?}", self.user_pick));
            }
            Action::DismissHint => {
                self.update("skryt napovedu", |a| a.spotify_hint_dismissed = true)
            }
        }
        outcome
    }

    /// Jednorazove overenie ucinku skratky (ziadna slucka). Uspech sa nehlasi; pri zlyhani sa zobrazi oznam.
    fn verify(&mut self) {
        let Some(p) = self.pending.take() else { return };
        let fg = input::foreground();
        log(&format!("overenie {p:?}: popredie teraz {fg:?}"));
        match p {
            Pending::Search { before } => {
                if fg.hwnd == before.hwnd || fg.hwnd == 0 {
                    let why = if fg.elevated.is_none() && before.elevated.is_none() {
                        t("notice.maybe_admin")
                    } else {
                        String::new()
                    };
                    self.show_notice(tf("notice.search_failed", &[&why]));
                }
            }
            Pending::TrayFocus => {
                if !is_taskbar(&fg) {
                    self.show_notice(t("notice.tray_no_focus"));
                    return;
                }
                let focused = input::focused_element(Duration::from_millis(800));
                log(&format!("Win+B: fokus na {focused:?}"));
                match focused {
                    Some(f) if is_overflow_button(&f) => match input::send_enter() {
                        Ok(()) => {
                            self.pending = Some(Pending::TrayFlyout);
                            schedule_timer(T_VERIFY, VERIFY_TRAY_MS);
                        }
                        Err(b) => self.show_notice(blocked_text(b, "Tray", "Enter")),
                    },
                    _ => self.show_notice(t("notice.tray_focus")),
                }
            }
            Pending::TrayFlyout => {
                if fg.class != OVERFLOW_CLASS {
                    self.show_notice(t("notice.tray_flyout"));
                }
            }
        }
    }

    /// Zapne upravu lisaty: panel so zoznamom poloziek pod listou (na monitore, kde sa otvorilo menu).
    fn start_bar_edit(&mut self) {
        let Some(monitor_id) = self
            .menu_at
            .clone()
            .map(|(m, _)| m)
            .or_else(|| self.layouts.keys().next().cloned())
        else {
            return;
        };
        log("uprava lisaty: zapnuta");
        self.bar_edit = Some(bar_edit::BarEdit::default());
        self.hovered = None;
        self.pressed = None;
        self.kit_open(POP_EDIT);
        let center = self.bar_width(&monitor_id) / 2;
        self.popover_monitor = Some(monitor_id.clone());
        popup::request(Request::ShowPopover {
            kind: POP_EDIT,
            monitor_id,
            anchor_x: center,
            w_lp: extras::KIT_W_LP,
            h_lp: extras::kit_h(POP_EDIT),
        });
        invalidate_all();
    }

    /// Sirka listy monitora v px (z posledneho rozlozenia; 0 = nezname).
    fn bar_width(&self, monitor_id: &str) -> i32 {
        self.bar_widths.get(monitor_id).copied().unwrap_or(0)
    }

    /// Mys nad listou pocas upravy: tahanie poloziek (klik nic neotvara).
    fn edit_pointer(&mut self, monitor_id: &str, kind: PointerKind, x: i32) {
        let hit = self
            .layouts
            .get(monitor_id)
            .and_then(|l| hit_test(l, x))
            .map(|(id, _)| id)
            .filter(|id| bar_edit::config_id(*id).is_some());
        let Some(ed) = self.bar_edit.as_mut() else {
            return;
        };
        match kind {
            PointerKind::Down => {
                ed.drag = hit.map(|id| bar_edit::Drag {
                    monitor: monitor_id.to_string(),
                    id,
                    start_x: x,
                    x,
                    moved: false,
                });
            }
            PointerKind::Move => {
                ed.hover = hit.map(|id| (monitor_id.to_string(), id));
                if let Some(d) = ed.drag.as_mut().filter(|d| d.monitor == monitor_id) {
                    d.x = x;
                    d.moved |= (x - d.start_x).abs() > DRAG_PX;
                }
            }
            PointerKind::Click => {
                if let Some(d) = ed
                    .drag
                    .take()
                    .filter(|d| d.moved && d.monitor == monitor_id)
                {
                    let items = self.items();
                    let placed = self.layouts.get(monitor_id).cloned().unwrap_or_default();
                    let w = self.bar_width(monitor_id);
                    let locked = self.config.center_locked();
                    let target = bar_edit::drop_target(&placed, &items, w, d.id, x, locked);
                    if let (Some((g, before, _)), Some(id)) = (target, bar_edit::config_id(d.id)) {
                        let before = before.and_then(bar_edit::config_id);
                        log(&format!("uprava lisaty: {id} -> {g:?} pred {before:?}"));
                        self.save_config_with(|c| c.layout.move_before(&id, g, before.as_deref()));
                        popup::invalidate_popover();
                    }
                }
            }
            PointerKind::Leave => {
                ed.hover = None;
                if !transom_platform::input::left_button_down() {
                    ed.drag = None;
                }
            }
            PointerKind::RightClick => {
                ed.drag = None;
            }
        }
        invalidate_all();
    }

    fn pointer(&mut self, monitor_id: &str, kind: PointerKind, x: i32) {
        // uprava lisaty: mys tahá polozky, nic sa neotvara (prave tlacidlo stale otvori menu)
        if self.bar_edit.is_some() && kind != PointerKind::RightClick {
            self.edit_pointer(monitor_id, kind, x);
            return;
        }
        if kind == PointerKind::Down {
            self.press_x = x;
        }
        let hit = match kind {
            PointerKind::Leave => None,
            _ => self.layouts.get(monitor_id).and_then(|l| hit_test(l, x)),
        };
        let items = self.items();
        let action = hit.and_then(|(id, part)| {
            items
                .iter()
                .find(|i| i.id == id)
                .and_then(|i| i.parts.get(part))
                .and_then(|p| p.action())
        });
        // hover/stlacenie len nad klikatelnymi, povolenymi castami; prekreslit len pri zmene
        let here = match (action, hit) {
            (Some(_), Some((id, part))) => Some((monitor_id.to_string(), id, part)),
            _ => None,
        };
        let new_hover = if kind == PointerKind::Leave {
            None
        } else {
            here.clone()
        };
        let new_pressed = match kind {
            PointerKind::Down => here.clone(),
            PointerKind::Move => self.pressed.clone().filter(|p| Some(p) == here.as_ref()),
            PointerKind::Click | PointerKind::Leave | PointerKind::RightClick => None,
        };
        // akcia: uvolnenie nad tou istou castou, nad ktorou bolo stlacenie
        let fire = kind == PointerKind::Click && here.is_some() && self.pressed == here;
        if new_hover != self.hovered || new_pressed != self.pressed {
            log(&format!(
                "hover {:?} stlacene {:?}",
                new_hover.as_ref().map(|(_, i, p)| (i, p)),
                new_pressed.as_ref().map(|(_, i, p)| (i, p))
            ));
            self.hovered = new_hover;
            self.pressed = new_pressed;
            invalidate_all();
        }
        if fire && action == Some(Action::ShelfFile) {
            if let Some((_, part)) = hit {
                let out = self.shelf_click(part);
                self.sound.cue(snd::Origin::Pointer, out);
            }
        } else if fire {
            if let Some(a) = action {
                let center = hit.and_then(|(id, part)| {
                    self.layouts
                        .get(monitor_id)
                        .and_then(|l| l.iter().find(|p| p.item == id && p.part == part))
                        .map(|p| p.x + p.w / 2)
                });
                let out = self.act(a, center.map(|c| (monitor_id.to_string(), c)));
                self.sound.cue(snd::Origin::Pointer, out);
            }
        }
        // vstup do nastaveni, ktory sa neda skryt: prave tlacidlo na prazdnom mieste listy (aj nad textom bez
        // akcie, napr. nazov skladby alebo bateria); nad ovladacim prvkom (aj vypnutym) sa menu neotvori a prvok
        // sa nespusti
        // ovladaci prvok = tlacidlo (aj vypnute), obal, kruh alebo text s akciou
        let on_control = hit.is_some_and(|(id, part)| {
            items
                .iter()
                .find(|i| i.id == id)
                .and_then(|i| i.parts.get(part))
                .is_some_and(|p| {
                    p.action().is_some()
                        || matches!(p, Part::Icon { .. } | Part::Art { .. } | Part::Ring { .. })
                })
        });
        if kind == PointerKind::RightClick && !on_control {
            self.menu_at = Some((monitor_id.to_string(), x));
            log(&format!("kontextove menu (prazdne miesto x={x})"));
            self.open_menu(
                popup::MenuAnchor::Bar {
                    monitor_id: monitor_id.to_string(),
                    x,
                },
                menu_items(false),
            );
        } else if kind == PointerKind::RightClick {
            log(&format!(
                "prave tlacidlo nad prvkom {hit:?} - bez menu a bez akcie"
            ));
        }
    }

    /// Panel vyuzitia AI.
    fn paint_ai(&mut self, p: &mut dyn transom_render::Painter) {
        // vysledok mohol prist pred notifikaciou: prevziat a prekreslit aj listu (kruh)
        if self.ai.take_ready() {
            invalidate_all();
        }
        self.ai.paint(
            p,
            &self.theme,
            &self.config.usage,
            pclock::today(),
            ai::now_unix(),
        );
        // pristupnost: tlacidla a dni panela pre citacky obrazovky; zmenil sa rozsah posuvania -> dokreslit
        let (entries, active) = self.ai.a11y();
        popup::set_panel_accessibility(entries, active);
        if std::mem::take(&mut self.ai.repaint) {
            popup::invalidate_popover();
        }
    }

    /// Spusti okno nastaveni (volitelne rovno na stranke `page`, napr. "layout"). Ak uz bezi, druha instancia ho len prepne.
    fn launch_settings(&self, page: Option<&str>) {
        let exe = std::env::current_exe()
            .map(|p| p.with_file_name("transom-settings.exe"))
            .unwrap_or_default();
        log(&format!("spustam nastavenia: {} {page:?}", exe.display()));
        popup::request(Request::Launch {
            path: exe.display().to_string(),
            args: page.map(|p| format!("--page {p}")).unwrap_or_default(),
        });
    }

    /// Skryje / zobrazi listu. Skryta lista ma **vzdy** cestu spat: ikona v oblasti oznameni sa vynuti aj ked ju pouzivatel
    /// vypol (klik / menu "Zobrazit listu") a opatovne spustenie transom listu tiez vrati. Skrytie nie je trvale (po
    /// restarte je lista zobrazena).
    fn set_hidden(&mut self, hidden: bool) {
        if self.hidden == hidden {
            return;
        }
        self.hidden = hidden;
        log(&format!(
            "lista {}",
            if hidden { "skryta" } else { "zobrazena" }
        ));
        popup::request(Request::SetHidden(hidden));
        self.sync_tray();
    }

    /// Volba "Zostava / Spotrebovane" v paneli AI: zapise sa do konfiguracie (znova nacitanej z disku, meni sa **len**
    /// `usage.ring_mode`, ak je subor zapisatelny), hned sa prejavi v paneli aj v kruhu v liste a ostatne procesy
    /// (okno nastaveni) sa o zmene dozvedia broadcastom.
    fn set_ring_mode(&mut self, m: transom_usage::limits::RingMode) {
        let v = match m {
            transom_usage::limits::RingMode::Remaining => "remaining",
            transom_usage::limits::RingMode::Used => "used",
        };
        if self.config.usage.ring_mode == v {
            return;
        }
        // pod zamkom znova nacita subor a zmeni len toto pole (zmeny nastaveni z druheho procesu nezanikne); poskodeny alebo
        // novsi subor sa nezapise
        match transom_config::update(&self.config_path, |c| c.usage.ring_mode = v.into()) {
            Ok(_) => transom_platform::post_broadcast(transom_config::CHANGED_MESSAGE, 0),
            Err(e) => log(&format!("rezim kruhu: konfiguracia sa neulozila: {e}")),
        }
        log(&format!("rezim kruhu a kariet: {v}"));
        self.update("rezim kruhu", |a| a.config.usage.ring_mode = v.into());
    }

    /// Otvori vlastne kontextove menu (okno, velkost a zachytenie mysi riesi platforma, texty a vzhlad `menu.rs`).
    fn open_menu(&mut self, anchor: popup::MenuAnchor, items: Vec<(u32, String)>) {
        self.menu = Some(menu::Menu::new(items));
        popup::request(Request::ShowMenu {
            kind: POP_MENU,
            anchor,
        });
    }

    /// Polozky a aktivna polozka menu pre citacky obrazovky (po otvoreni a po kazdej zmene aktivnej polozky).
    fn publish_menu(&self) {
        if let Some(m) = &self.menu {
            popup::set_menu_accessibility(m.entries(), m.active);
        }
    }

    /// Vybrana polozka menu: zatvori menu a vykona ju (pre "AI usage" sa menu zatvori otvorenim panela).
    fn choose_menu(&mut self, id: u32) {
        if id != MENU_AI && id != MENU_TOOLS {
            popup::request(Request::ClosePopover);
        }
        self.on_menu(id);
        // ukoncenie zvuk neprehra (proces konci); ostatne polozky su umyselna a odoslana akcia
        if id != MENU_QUIT {
            self.sound.cue(snd::Origin::Menu, snd::Outcome::Done);
        }
    }

    fn on_menu_pointer(&mut self, pointer: PointerKind, x: i32, y: i32) {
        let Some(m) = self.menu.as_mut() else {
            return;
        };
        let h = if pointer == PointerKind::Leave {
            None
        } else {
            m.hit(x, y)
        };
        let mut redraw = m.hover(h);
        match pointer {
            PointerKind::Down => {
                m.pressed = h.is_some();
                redraw = true;
            }
            PointerKind::Click => {
                m.pressed = false;
                if let Some(id) = h.and_then(|i| m.choose(i)) {
                    self.choose_menu(id);
                    return;
                }
                redraw = true;
            }
            _ => {}
        }
        if redraw {
            popup::invalidate_popover();
            self.publish_menu();
        }
    }

    fn on_menu_key(&mut self, vk: u32) {
        let Some(m) = self.menu.as_mut() else {
            return;
        };
        match m.key(vk) {
            menu::Nav::None => {}
            menu::Nav::Redraw => {
                popup::invalidate_popover();
                self.publish_menu();
            }
            menu::Nav::Choose(id) => self.choose_menu(id),
        }
    }

    /// Scenar `--test-menu [esc]` (len test): otvori menu na liste, posle klavesy priamo jeho oknu a zapise vysledok do logu
    /// (popredie pred/po, vybrana polozka). Nepouziva ziadny vstup na ploche.
    fn test_menu_step(&mut self) {
        let esc = std::env::args().any(|a| a == "esc");
        let step = self.test_menu;
        self.test_menu += 1;
        log(&format!(
            "test-menu krok {step}: popredie = {}",
            popup::foreground_description()
        ));
        match step {
            1 => {
                let Some(m) = self.layouts.keys().next().cloned() else {
                    return;
                };
                self.menu_at = Some((m.clone(), 300));
                self.open_menu(
                    popup::MenuAnchor::Bar {
                        monitor_id: m,
                        x: 300,
                    },
                    menu_items(false),
                );
            }
            2..=5 => {
                log(&format!(
                    "test-menu: Down -> {}",
                    popup::post_key_to_popover(0x28)
                ));
            }
            6 => {
                let ok = popup::post_key_to_popover(if esc { 0x1B } else { 0x0D });
                log(&format!(
                    "test-menu: {} -> {ok}",
                    if esc { "Esc" } else { "Enter" }
                ));
            }
            8 => {
                log(&format!("test-menu hotovo; skryta={}", self.hidden));
                if self.hidden {
                    self.set_hidden(false);
                }
                return;
            }
            _ => {}
        }
        schedule_timer(T_TESTMENU, 700);
    }

    /// Scenar `--test-ai` (len test): otvori panel AI, po nacitani dat prepne Zostava/Spotrebovane (klik na tlacidlo v
    /// paneli), potom Mesiac a den; zapisuje do logu rezim z konfiguracie a text kruhu v liste. Screenshoty robi skript.
    fn test_ai_step(&mut self) {
        let step = self.test_ai;
        self.test_ai += 1;
        // `--test-ai-real`: panel sa len otvori a kazdu sekundu sa do logu zapisu suradnice prvkov na OBRAZOVKE; klika skript
        // skutocnou mysou (po overeni, ze pod kurzorom je okno popoveru tejto instancie)
        if flag("--test-ai-real") {
            if step == 1 {
                self.act(Action::AiPanel, None);
            } else if step > 1 {
                let pt = |me: &Self, hit: ai::AiHit| {
                    me.ai
                        .hits
                        .iter()
                        .find(|(_, h)| *h == hit)
                        .and_then(|(r, _)| {
                            popup::popover_client_to_screen(r.x + r.w / 2, r.y + r.h / 2)
                        })
                        .map_or("-".to_string(), |(x, y)| format!("{x},{y}"))
                };
                use transom_usage::limits::RingMode;
                let ring =
                    ai::ring_view(self.ai.data.as_ref(), &self.config.usage, ai::now_unix()).label;
                log(&format!(
                    "real-targets mode_left={} mode_used={} tab_month={} tab_today={} day={} unit_cost={} scroll={}/{} month_ui={:?}/{:?} mode={} ring='{ring}' popover={:?} panel_scroll_area={}",
                    pt(self, ai::AiHit::Mode(RingMode::Remaining)),
                    pt(self, ai::AiHit::Mode(RingMode::Used)),
                    pt(self, ai::AiHit::Tab(transom_usage::Period::Month)),
                    pt(self, ai::AiHit::Tab(transom_usage::Period::Today)),
                    pt(self, ai::AiHit::Day(pclock::today().d)),
                    pt(self, ai::AiHit::Unit(transom_usage::history::Metric::Cost)),
                    self.ai.scroll,
                    self.ai.max_scroll,
                    self.ai.tab,
                    self.ai.month.day,
                    self.config.usage.ring_mode,
                    popup::popover_open(),
                    pt(self, ai::AiHit::Refresh),
                ));
            }
            schedule_timer(T_TESTAI, 700);
            return;
        }
        let ring = {
            let d = self.ai.data.as_ref();
            ai::ring_view(d, &self.config.usage, ai::now_unix()).label
        };
        log(&format!(
            "test-ai krok {step}: rezim={} kruh='{ring}' scroll={}/{}",
            self.config.usage.ring_mode, self.ai.scroll, self.ai.max_scroll
        ));
        let click = |hit: ai::AiHit, me: &mut Self| {
            let r = me.ai.hits.iter().find(|(_, h)| *h == hit).map(|(r, _)| *r);
            match r {
                Some(r) => {
                    let ok = popup::post_click_to_popover(r.x + r.w / 2, r.y + r.h / 2);
                    log(&format!("test-ai: klik {hit:?} -> {ok}"));
                }
                None => log(&format!("test-ai: prvok {hit:?} nie je viditelny")),
            }
        };
        match step {
            1 => {
                self.act(Action::AiPanel, None);
            }
            // krok 2-3: pockat na data
            4 => click(
                ai::AiHit::Mode(transom_usage::limits::RingMode::Remaining),
                self,
            ),
            6 => click(ai::AiHit::Mode(transom_usage::limits::RingMode::Used), self),
            8 => click(
                ai::AiHit::Mode(transom_usage::limits::RingMode::Remaining),
                self,
            ),
            10 => click(ai::AiHit::Tab(transom_usage::Period::Month), self),
            12 => click(ai::AiHit::Day(pclock::today().d), self),
            14 => click(ai::AiHit::Unit(transom_usage::history::Metric::Cost), self),
            16 => {
                log("test-ai hotovo");
                return;
            }
            _ => {}
        }
        schedule_timer(T_TESTAI, 1500);
    }

    fn on_menu(&mut self, id: u32) {
        match id {
            MENU_SETTINGS => self.launch_settings(None),
            MENU_CUSTOMIZE => self.launch_settings(Some("layout")),
            MENU_EDIT => self.start_bar_edit(),
            MENU_AI => {
                // popover AI sa otvori pod listou na mieste, kde sa kliklo (z ikony v oblasti oznameni: prva lista)
                let at = self.menu_at.clone();
                self.act(Action::AiPanel, at);
            }
            MENU_TOOLS => {
                let at = self.menu_at.clone();
                self.act(Action::ToolsPanel, at);
            }
            MENU_HIDE => self.set_hidden(true),
            MENU_SHOW => self.set_hidden(false),
            MENU_QUIT => transom_platform::appbar::request_quit(),
            MENU_UPDATE => self.open_update_panel(),
            _ => {}
        }
    }

    /// Spusti vlakno aktualizacii (raz) a nastavi pravidelnu kontrolu podla nastavenia.
    fn sync_updates(&mut self) {
        let auto = self.config.updates.auto_check;
        match &self.updater {
            Some(u) => u.set_auto(auto),
            None => {
                if !update::allowed(automation_run()) {
                    return;
                }
                if let Some(n) = notifier() {
                    self.updater = update::Updater::start(
                        auto,
                        std::sync::Arc::new(move || n.notify(N_UPDATE)),
                    );
                    log(&format!(
                        "aktualizacie: pravidelna kontrola {}",
                        if auto { "zapnuta" } else { "vypnuta" }
                    ));
                }
            }
        }
    }

    /// Novy stav aktualizacii: polozka menu, ikona na liste, oznamenie o novej verzii, vysledok akcie pouzivatela.
    fn on_update(&mut self) {
        let Some(u) = &self.updater else { return };
        let st = u.snapshot();
        let panel = popup::popover_open() == Some(POP_UPDATE);
        // chyba po stahovani je chyba instalacie (do panela); chyba kontroly na poziadanie len oznamenie
        let installing = self.update_busy;
        self.update_busy = st.phase == update::Phase::Downloading;
        match &st.phase {
            update::Phase::Launched => {
                // instalator transom zatvori a po instalacii znova spusti
                transom_platform::appbar::request_quit();
                return;
            }
            update::Phase::UpToDate => {
                u.clear_phase();
                self.update_notice(t("upd.latest"));
            }
            update::Phase::Failed { msg, manual } => {
                let (msg, manual) = (msg.clone(), *manual);
                u.clear_phase();
                if manual {
                    // chyba instalacie: v otvorenom paneli, inak oznamenie
                    if installing {
                        self.update_error = Some(msg.clone());
                    }
                    if !(panel && installing) {
                        self.update_notice(tf("upd.failed", &[&msg]));
                    }
                }
            }
            update::Phase::Downloading => {
                if !panel {
                    self.show_notice(t("upd.downloading"));
                }
            }
            update::Phase::Idle => {}
        }
        self.set_update_avail(st.available);
    }

    /// Dostupna verzia sa zmenila (alebo potvrdila): menu, ikona, panel a jednorazove oznamenie o novej verzii.
    fn set_update_avail(&mut self, avail: Option<update::Release>) {
        if let Ok(mut m) = UPDATE_MENU.lock() {
            *m = avail.as_ref().map(|r| r.version.clone());
        }
        if let Some(r) = &avail {
            if self.update_announced.as_deref() != Some(r.version.as_str()) {
                self.update_announced = Some(r.version.clone());
                self.update_notice(update_pop::notice_text(r));
            }
        }
        self.update_avail = avail;
        invalidate_all();
        if popup::popover_open() == Some(POP_UPDATE) {
            if self.update_avail.is_none() {
                popup::request(Request::ClosePopover);
            } else {
                popup::invalidate_popover();
            }
        }
    }

    /// Oznamenie o aktualizacii: pri ikone v oblasti oznameni (klik = panel aktualizacie), inak v liste.
    fn update_notice(&mut self, text: String) {
        if !transom_platform::appbar::tray_balloon(&t("upd.title"), &text) {
            self.show_notice(text);
        }
    }

    /// Ikona dostupnej aktualizacie na liste: je novsia verzia, kontrola je zapnuta a ikona nie je vypnuta v nastaveniach.
    fn update_icon(&self) -> Option<&str> {
        update_pop::icon_version(self.update_avail.as_ref(), &self.config.updates)
    }

    /// Obsah panela aktualizacie (`None` = ziadna dostupna verzia).
    fn update_view(&self) -> Option<update_pop::UpdView<'_>> {
        let release = self.update_avail.as_ref()?;
        Some(update_pop::UpdView {
            release,
            installed: env!("CARGO_PKG_VERSION"),
            // simulacia: ako nainstalovana kopia (tlacidlo Aktualizovat teraz), nic sa nestahuje
            self_update: self.test_update > 0 || update::can_self_update(),
            busy: self.update_busy,
            error: self.update_error.as_deref(),
        })
    }

    /// Otvorit panel aktualizacie (oznamenie, polozka menu): pod ikonou na liste, inak pri pravom okraji listy.
    /// Uz otvoreny panel sa nezatvori (otvorenie je prepinac), len prekresli.
    fn open_update_panel(&mut self) {
        if self.update_avail.is_none() {
            if let Some(u) = &self.updater {
                u.check_now();
            }
            return;
        }
        if popup::popover_open() == Some(POP_UPDATE) {
            popup::invalidate_popover();
            return;
        }
        // skryta lista (oznamenie pri ikone v oblasti oznameni): panel nema pod cim byt - otvori sa stranka vydania
        // s poznamkami, nic sa neinstaluje
        if self.hidden {
            log("aktualizacie: lista je skryta, otvaram stranku vydania");
            self.open_release_page();
            return;
        }
        // monitor: kde sa otvorilo menu, inak posledny popover, inak lista s ikonou, inak prva lista
        let with_icon = self
            .layouts
            .iter()
            .find(|(_, l)| l.iter().any(|p| p.item == ItemId::Update))
            .map(|(m, _)| m.clone());
        let Some(m) = self
            .menu_at
            .clone()
            .map(|(m, _)| m)
            .or_else(|| self.popover_monitor.clone())
            .filter(|m| self.layouts.contains_key(m))
            .or(with_icon)
            .or_else(|| self.layouts.keys().next().cloned())
        else {
            return;
        };
        // pod ikonou, ak je na tej liste, inak pri pravom okraji
        let x = self
            .layouts
            .get(&m)
            .and_then(|l| l.iter().find(|p| p.item == ItemId::Update))
            .map(|p| p.x + p.w / 2)
            .unwrap_or_else(|| self.bar_width(&m));
        let _ = self.act(Action::UpdatePanel, Some((m, x)));
    }

    /// Aktualizovat na dostupnu verziu: nainstalovana kopia sa aktualizuje sama, prenosna otvori stranku vydania.
    fn start_update(&mut self) {
        if self.test_update > 0 {
            // simulacia: nic sa nestahuje ani nespusta
            log("test-update: Aktualizovat teraz (simulovane stahovanie)");
            self.update_error = None;
            self.update_busy = true;
            popup::invalidate_popover();
            return;
        }
        let Some(u) = &self.updater else { return };
        match u.snapshot().available {
            Some(r) if update::can_self_update() => {
                if self.update_busy {
                    return;
                }
                log(&format!("aktualizacie: instalujem {}", r.version));
                self.update_error = None;
                self.update_busy = true;
                u.install();
                popup::invalidate_popover();
            }
            Some(r) => {
                log("aktualizacie: prenosna kopia, otvaram stranku vydania");
                update::open_page(&r.page);
            }
            None => u.check_now(),
        }
    }

    /// Stranka vydania v prehliadaci (v simulacii len zapis do logu).
    fn open_release_page(&mut self) {
        let Some(r) = &self.update_avail else { return };
        if self.test_update > 0 {
            log(&format!(
                "test-update: stranka vydania {} (neotvara sa)",
                r.page
            ));
            return;
        }
        update::open_page(&r.page);
    }

    /// Prikaz z panela aktualizacie.
    pub(crate) fn update_cmd(&mut self, id: u32) {
        match id {
            update_pop::UP_NOW => {
                if self.update_view().is_some_and(|v| v.self_update) {
                    self.start_update();
                } else {
                    self.open_release_page();
                }
            }
            update_pop::UP_LATER => popup::request(Request::ClosePopover),
            update_pop::UP_NOTES => self.open_release_page(),
            _ => {}
        }
    }

    /// Scenar `--test-update` (len test, bez siete): simulovana dostupna verzia s ukazkovymi poznamkami, ikona na liste,
    /// otvorenie panela, Aktualizovat teraz (simulovane stahovanie), chyba, Neskor. Zapisuje do logu, co vidi citacka.
    fn test_update_step(&mut self) {
        let step = self.test_update;
        self.test_update += 1;
        let log_panel = |me: &Self, what: &str| {
            let (entries, _, focus, name) = me.kit.a11y();
            let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
            log(&format!(
                "test-update {what}: panel={:?} nazov='{name}' fokus={focus:?} prvky={names:?}",
                popup::popover_open()
            ));
        };
        match step {
            1 => {
                log("test-update: SIMULOVANA aktualizacia 0.3.0 (ziadna siet)");
                self.set_update_avail(Some(update_pop::sample()));
            }
            2 => {
                let icon = self.layouts.values().find_map(|l| {
                    l.iter()
                        .find(|p| p.item == ItemId::Update)
                        .map(|p| format!("{}+{}", p.x, p.w))
                });
                let last = self
                    .layouts
                    .values()
                    .next()
                    .and_then(|l| l.last())
                    .map(|p| p.item);
                log(&format!(
                    "test-update: ikona={icon:?} posledna_polozka={last:?} menu={:?}",
                    UPDATE_MENU.lock().ok().and_then(|m| m.clone())
                ));
                self.open_update_panel();
            }
            3 => log_panel(self, "otvoreny"),
            4 => self.update_cmd(update_pop::UP_NOW),
            5 => log_panel(self, "stahovanie"),
            6 => {
                self.update_busy = false;
                self.update_error = Some("simulated: checksum mismatch".into());
                popup::invalidate_popover();
            }
            7 => log_panel(self, "chyba"),
            8 => self.update_cmd(update_pop::UP_LATER),
            9 => {
                log(&format!(
                    "test-update hotovo: panel={:?} ikona_ostava={}",
                    popup::popover_open(),
                    self.update_icon().is_some()
                ));
                return;
            }
            _ => return,
        }
        schedule_timer(T_TESTUPD, 700);
    }

    fn on_popover_pointer(&mut self, kind: u32, pointer: PointerKind, x: i32, y: i32) {
        // predpoved otvorena z Nastrojov: sipka spat
        if kind == POP_WEATHER && self.weather_back && pointer == PointerKind::Click {
            let r = weather::back_rect(popup::popover_dpi() as f32 / 96.0);
            if x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h {
                self.tools_home();
            }
            return;
        }
        if kind == POP_MENU {
            self.on_menu_pointer(pointer, x, y);
            return;
        }
        if kind == POP_LAUNCHER {
            let (cmd, redraw) = self.launcher_panel.pointer(pointer, x, y);
            if let Some(c) = cmd {
                self.run_launcher_cmd(c);
            }
            if redraw {
                popup::invalidate_popover();
            }
            return;
        }
        if kind == POP_TOOLS {
            let (cmd, redraw) = self.tools_panel.pointer(pointer, x, y);
            if let Some(c) = cmd {
                self.run_tools_cmd(c);
            }
            self.library_sync();
            if redraw {
                popup::invalidate_popover();
            }
            return;
        }
        if kind == POP_AUDIO {
            let snap = self.audio_snapshot();
            let (cmd, redraw) = self.audio_panel.pointer(pointer, x, y, &snap);
            if let Some(c) = cmd {
                self.run_audio_cmd(c);
            }
            if redraw {
                popup::invalidate_popover();
            }
            return;
        }
        if extras::is_kit(kind) {
            let (cmd, redraw) = self.kit.pointer(pointer, x, y);
            if let Some(c) = cmd {
                self.kit_cmd(kind, c);
            }
            if redraw {
                popup::invalidate_popover();
            }
            return;
        }
        if kind == POP_PERIPH {
            let (cmd, redraw) = self.periph_panel.pointer(pointer, x, y);
            if let Some(c) = cmd {
                self.run_periph_cmd(c);
            }
            if redraw {
                popup::invalidate_popover();
            }
            return;
        }
        if kind == POP_AI {
            let h = if pointer == PointerKind::Leave {
                None
            } else {
                self.ai.hit(x, y)
            };
            let mut redraw = h != self.ai.hover;
            self.ai.hover = h;
            if pointer == PointerKind::Click {
                if h.is_some() {
                    self.sound.cue(snd::Origin::Popover, snd::Outcome::Done);
                }
                match h {
                    Some(ai::AiHit::Mode(m)) => {
                        self.set_ring_mode(m);
                        redraw = true;
                    }
                    Some(
                        hit @ (ai::AiHit::Tab(_)
                        | ai::AiHit::Details
                        | ai::AiHit::MonthPrev
                        | ai::AiHit::MonthNext
                        | ai::AiHit::MonthThis
                        | ai::AiHit::Unit(_)
                        | ai::AiHit::Prov(_)
                        | ai::AiHit::Day(_)),
                    ) => {
                        redraw |= self.ai.click(hit, pclock::today());
                    }
                    Some(ai::AiHit::Refresh) => {
                        // tlacidlo Obnovit je vzdy dostupne (vedoma akcia pouzivatela)
                        self.ai
                            .refresh(&self.config.usage, notifier(), ai::RefreshKind::Manual);
                        redraw = true;
                    }
                    Some(ai::AiHit::Settings) => {
                        popup::request(Request::ClosePopover);
                        self.on_menu(MENU_SETTINGS);
                    }
                    None => {}
                }
            }
            if redraw {
                popup::invalidate_popover();
            }
            return;
        }
        if kind != POP_CALENDAR {
            return;
        }
        let h = if pointer == PointerKind::Leave {
            None
        } else {
            self.cal.hit(x, y)
        };
        let mut redraw = h != self.cal.hover;
        self.cal.hover = h;
        if pointer == PointerKind::Click {
            if let Some(h) = h {
                redraw |= self.cal.click(h, pclock::today());
                self.sound.cue(snd::Origin::Popover, snd::Outcome::Done);
            }
        }
        if redraw {
            popup::invalidate_popover();
        }
    }
}

/// Polozky kontextoveho menu v aktualnom jazyku (nazvy citaju aj citacky obrazovky).
/// Polozky kontextoveho menu: bezna lista = Nastavenia, Prispôsobit listu, Vyuzitie AI, Skryt listu, Ukoncit; pri skrytej liste (menu ikony
/// v oblasti oznameni) = Zobrazit listu, Nastavenia, Ukoncit.
fn menu_items(hidden: bool) -> Vec<(u32, String)> {
    if hidden {
        vec![
            (MENU_SHOW, t("menu.show")),
            (MENU_SETTINGS, t("menu.settings")),
            (0, String::new()),
            (MENU_QUIT, t("menu.quit")),
        ]
    } else {
        let mut v = vec![
            (MENU_SETTINGS, t("menu.settings")),
            (MENU_EDIT, t("menu.edit_bar")),
            (MENU_CUSTOMIZE, t("menu.customize")),
            (MENU_AI, t("menu.ai")),
            (MENU_TOOLS, t("menu.tools")),
            (MENU_HIDE, t("menu.hide")),
            (0, String::new()),
            (MENU_QUIT, t("menu.quit")),
        ];
        if let Some(ver) = UPDATE_MENU.lock().ok().and_then(|m| m.clone()) {
            v.insert(0, (MENU_UPDATE, tf("upd.menu", &[&ver])));
            v.insert(1, (0, String::new()));
        }
        v
    }
}

/// Vyber monitorov z konfiguracie.
fn selection_of(d: &transom_config::Displays) -> MonitorSelection {
    match d.mode.as_str() {
        "primary" => MonitorSelection::Primary,
        "selected" => MonitorSelection::Selected(d.selected.iter().map(|r| r.id.clone()).collect()),
        _ => MonitorSelection::All,
    }
}

/// Interaktivne oblasti (tooltip + MSAA) z toho isteho rozlozenia, nad ktorym bezi hit-testing;
/// ku kazdej aj akcia (len povolene), v tom istom poradi.
fn regions_of(items: &[StatusItem], placed: &[Placed]) -> Vec<(Region, Option<Action>)> {
    placed
        .iter()
        .filter_map(|pl| {
            let part = items
                .iter()
                .find(|i| i.id == pl.item)
                .and_then(|i| i.parts.get(pl.part))?;
            // (nazov, tlacidlo, povolene, hodnota pre citacku obrazovky)
            let (name, button, enabled) = match part {
                Part::Icon { name, enabled, .. } => (name.clone(), true, *enabled),
                Part::Text {
                    text,
                    action: Some(Action::MediaCycle),
                    ..
                } => (tf("media.cycle", &[text]), true, true),
                Part::Text {
                    text,
                    action: Some(Action::DismissHint),
                    ..
                } => (tf("hint.dismiss", &[text]), true, true),
                Part::Art {
                    action: Some(_), ..
                } => (t("media.open_source"), true, true),
                Part::Text {
                    text,
                    action: Some(Action::Calendar),
                    ..
                } => (tf("clock.open_calendar", &[text]), true, true),
                // kruh a oznacenie poskytovatela: ten isty nazov (stav, hodnota, cas zaznamu)
                Part::Ring { name, .. } => (name.clone(), true, true),
                // bateria periferie: nazov zariadenia, uroven a stav slovami
                Part::Device { name, .. } => (name.clone(), true, true),
                // subor Police: nazov a stav (dostupny / nedostupny) slovami
                Part::File { name, .. } | Part::Island { name, .. } => (name.clone(), true, true),
                Part::Text {
                    text,
                    action: Some(Action::ToolsPanel),
                    ..
                } if pl.item == ItemId::Shelf => (
                    tf("shelf.more_name", &[text.trim_start_matches('+')]),
                    true,
                    true,
                ),
                Part::Text {
                    text,
                    action: Some(Action::PeripheralsPanel),
                    ..
                } => (
                    tf("per.more_name", &[text.trim_start_matches('+')]),
                    true,
                    true,
                ),
                Part::Text {
                    action: Some(Action::AiPanel),
                    ..
                } => (ai_name(items), true, true),
                Part::Text { text, .. } if pl.item == ItemId::Media => (text.clone(), false, true),
                // stav baterie (percento, nabijanie): predtym nebol pre citacku obrazovky vobec vystaveny
                Part::Text { text, .. } if pl.item == ItemId::Battery => {
                    (format!("{}: {text}", t("item.battery")), false, true)
                }
                _ => return None,
            };
            // hodnota (accValue): cas hodin, percento kruhu AI, text baterie / skladby
            let value = match part {
                Part::Ring { label, .. } | Part::Device { label, .. } => label.clone(),
                Part::Text {
                    text,
                    action: Some(Action::Calendar),
                    ..
                } => text.clone(),
                Part::Text { text, .. } if matches!(pl.item, ItemId::Battery | ItemId::Media) => {
                    text.clone()
                }
                _ => String::new(),
            };
            Some((
                Region {
                    x: pl.x,
                    w: pl.w,
                    name,
                    button,
                    enabled,
                    value,
                    // odlozene subory ukazu vybezok, bublina by ho prekryla
                    no_tip: pl.item == ItemId::Shelf,
                },
                part.action(),
            ))
        })
        .collect()
}

/// Pristupny nazov polozky AI (z kruhu), aby aj oznacenie poskytovatela hovorilo cely stav.
fn ai_name(items: &[StatusItem]) -> String {
    items
        .iter()
        .find(|i| i.id == ItemId::Ai)
        .and_then(|i| {
            i.parts.iter().find_map(|p| match p {
                Part::Ring { name, .. } => Some(name.clone()),
                _ => None,
            })
        })
        .unwrap_or_else(|| t("item.ai"))
}

/// Obsah vybezku pri prezerani (prejdenie mysou nad polozkou listy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotchView {
    /// Menu ostrova (zalozky).
    Island,
    Stash,
    Media,
    Info,
    Clip,
    Weather,
}

/// Obsah vybezku pre polozku listy pod mysou (`None` = ziadny).
fn notch_view_for(item: ItemId) -> Option<NotchView> {
    match item {
        // stred listy je ostrov: odlozene subory su v jeho menu (klik)
        ItemId::Media => Some(NotchView::Media),
        ItemId::ToolClipboard => Some(NotchView::Clip),
        ItemId::Weather => Some(NotchView::Weather),
        _ => None,
    }
}

/// Cast panela Nastroje, na ktorej sa otvori po akcii (`ToolsPanel` = Polica, ako doteraz).
fn tools_section(a: Action) -> Option<tools_pop::Section> {
    use tools_pop::Section;
    Some(match a {
        Action::ToolsPanel | Action::ToolsShelf => Section::Shelf,
        Action::ToolsClipboard => Section::Clipboard,
        Action::ToolsNotes => Section::Notes,
        Action::ToolsShots => Section::Shots,
        _ => return None,
    })
}

impl BarHandler for App {
    fn paint_popover(&mut self, ctx: &PopoverPaint) {
        // sklo: sive vyhladzovanie textu (ClearType by na priesvitnom podklade dal farebne lemy)
        let mut p = GdiPainter::new(ctx.hdc, Size { w: ctx.w, h: ctx.h }, ctx.dpi)
            .grayscale(ctx.backdrop.is_some());
        // sklo: pozadie aj okraj v prechode na cierne / biele pozadie (alfa sa odvodi z rozdielu); tema sa vrati
        let saved = self.theme;
        if let Some((r, g, b)) = ctx.backdrop {
            self.theme = self.theme.glass_pass(Color::rgb(r, g, b));
        }
        match ctx.kind {
            POP_CALENDAR => {
                let first = first_day(&self.config.clock);
                let plans: &[transom_usage::PlanSnapshot] =
                    self.ai.data.as_ref().map_or(&[], |d| d.plans.as_slice());
                let providers = ai::enabled_providers(&self.config.usage);
                let ics = self.ics.as_ref().map(|s| s.calendars()).unwrap_or_default();
                let ctx = cal::CalCtx {
                    plans,
                    providers: &providers,
                    dates: &self.config.usage.dates,
                    now: ai::now_unix(),
                    max_age_s: ai::LIMIT_FRESH_S,
                    ics: &ics,
                    has_ics: !self.config.ics_calendars.is_empty(),
                };
                self.cal
                    .paint(&mut p, &self.theme, first, pclock::today(), &ctx);
            }
            POP_AI => self.paint_ai(&mut p),
            POP_WEATHER => {
                if let Some(w) = self.weather.as_ref().and_then(|w| w.snapshot()) {
                    weather::paint_with(&mut p, &self.theme, &w, self.weather_back);
                }
            }
            POP_AUDIO => self.paint_audio(&mut p),
            POP_PERIPH => self.paint_periph(&mut p),
            k if extras::is_kit(k) => self.paint_kit(k, &mut p),
            POP_TOOLS => self.paint_tools(&mut p),
            POP_LAUNCHER => self.paint_launcher(&mut p),
            transom_platform::dragdrop::NOTCH_KIND => {
                if transom_platform::dragdrop::notch_browsing()
                    && self.notch_view == NotchView::Island
                {
                    self.paint_island_menu(&mut p);
                } else if transom_platform::dragdrop::notch_browsing()
                    && self.notch_view == NotchView::Weather
                {
                    if let Some(w) = self.weather.as_ref().and_then(|w| w.snapshot()) {
                        weather::paint(&mut p, &self.theme, &w);
                    }
                } else if transom_platform::dragdrop::notch_browsing()
                    && self.notch_view == NotchView::Clip
                {
                    let rows = self.tools_snap.clipboard.clone();
                    self.clip_notch.paint(&mut p, &self.theme, &rows);
                } else if transom_platform::dragdrop::notch_browsing()
                    && self.notch_view == NotchView::Info
                {
                    if let Some(i) = self.notch_info.clone() {
                        info_notch::paint(&mut p, &self.theme, &i);
                    }
                } else if transom_platform::dragdrop::notch_browsing()
                    && self.notch_view == NotchView::Media
                {
                    if let Some(info) = self.chosen_session().cloned() {
                        let art = self.art.clone();
                        self.media_notch
                            .paint(&mut p, &self.theme, &info, art.as_ref());
                    }
                } else if transom_platform::dragdrop::notch_browsing() {
                    let (files, total) = self.stash_files();
                    self.stash.paint(&mut p, &self.theme, &files, total);
                } else {
                    tools_pop::paint_notch(&mut p, &self.theme)
                }
            }
            POP_MENU => {
                if let Some(m) = &self.menu {
                    m.paint(&mut p, &self.theme);
                }
            }
            _ => {}
        }
        // sklo: okraj kresli sklo (aj po oblucku vyduteho prechodu do listy); vlastny 1 px ramik obsahu by tam bol rovnou
        // ciarou - prekryje sa pozadim. Oddelovace a obrysy vo vnutri (farba `edge`) ostanu.
        if ctx.backdrop.is_some() {
            let (w, h, bg) = (ctx.w, ctx.h, self.theme.background);
            // ramik ma hrubku px(1) (pri 150 % su to 2 px)
            let b = ((ctx.dpi as f32 / 96.0).round() as i32).max(1);
            for r in [
                transom_render::Rect::new(0, 0, w, b),
                transom_render::Rect::new(0, h - b, w, b),
                transom_render::Rect::new(0, 0, b, h),
                transom_render::Rect::new(w - b, 0, b, h),
            ] {
                transom_render::Painter::fill_rect(&mut p, r, bg);
            }
        }
        self.theme = saved;
    }

    fn popover_glass(&self, kind: u32) -> Option<transom_platform::glass::GlassStyle> {
        // vsetky panely, menu aj zona pustenia suborov (vybezok pri liste)
        use transom_platform::glass::{GlassStyle, Lead, Rim};
        // vsetko pod listou (panely, vybezok) je k nej pripojene; menu na prave tlacidlo ostava samostatne
        let attached = kind != POP_MENU;
        self.glass_on(false).then(|| {
            let bg = self.theme.background;
            let mut st = GlassStyle::for_theme(
                (bg.r, bg.g, bg.b),
                self.theme.dark,
                if attached { Rim::Attached } else { Rim::All },
            );
            // nepriehladnost skla podla nastavenia (Pozadie -> Priehladnost panelov)
            st.tint_alpha = (self.config.appearance.glass_opacity.clamp(40, 100) * 255 / 100) as u8;
            // pripojene vyrastaju z listy: hore zacinaju jej plnou farbou a plynulo prejdu do skla (sklenena lista
            // uz je tym istym sklom)
            if attached && !self.glass_on(true) {
                // farbu spresni okno podla skutocnej farby listy na obrazovke; dlhy plynuly nabeh = "vysunute z listy"
                let fg = self.theme.foreground;
                st.lead = Some(Lead {
                    color: self.bar_lead_color(),
                    alpha: 255,
                    px: 120,
                    text: (fg.r, fg.g, fg.b),
                });
            }
            st
        })
    }

    fn popover_size(
        &mut self,
        kind: u32,
        dpi: u32,
        hdc: windows::Win32::Graphics::Gdi::HDC,
    ) -> Option<(i32, i32)> {
        if kind != POP_MENU {
            return None;
        }
        let m = self.menu.as_mut()?;
        let mut p = GdiPainter::new(hdc, Size { w: 1, h: 1 }, dpi);
        m.measure(&mut p);
        Some(m.size)
    }

    fn appearance(&self) -> BarAppearance {
        let th = self.bar_theme();
        let bg = th.background;
        BarAppearance {
            alpha: self.config.appearance.effective(&self.config.oled).1,
            bg: (bg.r, bg.g, bg.b),
            // sklo berie nadych z temy, nie z vlastnej farby listy (napr. "uplne priehladna biela" by dala biele sklo
            // pod svetlym textom)
            glass: self.glass_on(true).then(|| {
                let t = self.theme.background;
                transom_platform::glass::GlassStyle::for_theme(
                    (t.r, t.g, t.b),
                    self.theme.dark,
                    transom_platform::glass::Rim::Bottom,
                )
            }),
        }
    }

    fn paint(&mut self, ctx: &PaintCtx) {
        self.bar_hwnd = ctx.hwnd.0 as isize;
        let mut p = GdiPainter::new(
            ctx.hdc,
            Size {
                w: ctx.client.width(),
                h: ctx.client.height(),
            },
            ctx.dpi,
        )
        .grayscale(ctx.grayscale);
        let items = self.items();
        let on_this = |v: &Option<(String, ItemId, usize)>| {
            v.as_ref()
                .filter(|(m, _, _)| m == &ctx.monitor_id)
                .map(|(_, id, part)| (*id, *part))
        };
        let ui = Interaction {
            hovered: on_this(&self.hovered),
            pressed: on_this(&self.pressed),
            shift: self.shift_phase.map_or((0, 0), |ph| {
                let (x, y) = transom_config::pixel_shift_lp(ph);
                let s = |v: i32| (v as f32 * ctx.dpi as f32 / 96.0).round() as i32;
                (s(x), s(y))
            }),
            no_lines: self.config.oled.no_lines(),
            // pod otvorenym vybezkom bez spodnej ciary (vysunie sa z listy)
            // aj pod otvorenym panelom, ktory visi z tejto listy (vyzera vysunuty ako vybezok)
            gap: {
                let left = transom_platform::appbar::bar_screen_x(&ctx.monitor_id, 0);
                let bottom = transom_platform::appbar::bar_bottom(&ctx.monitor_id);
                let pop = transom_platform::popup::popover_extent()
                    .filter(|(_, top, _)| bottom.is_some_and(|b| (top - b).abs() <= 3))
                    .map(|(a, _, b)| (a, b));
                // ciara listy pokracuje az po zaciatok vyduteho obluka panela (par px dnu od okraja okna); o pixel
                // hlbsie nez horny bod obluka, aby sa ciara s oblukom spojila aj bez jeho najvyssieho riadku
                let inset = 5 * ctx.dpi as i32 / 96;
                pop.or_else(transom_platform::dragdrop::notch_extent)
                    .and_then(|(a, b)| Some((a - left? + inset, b - left? - inset)))
            },
        };
        // vlastna farba pozadia; pri priehladnom rezime sa kresli na cierne/biele pozadie (pozri `layered`)
        let mut theme = self.bar_theme();
        if let Some((r, g, b)) = ctx.backdrop {
            theme.background = Color::rgb(r, g, b);
        }
        // posun posuvneho textu: v prvom prechode z casu, v druhom ten isty (inak by sa alfa odvodila z roznych polohy)
        let anim = if ctx.bookkeeping {
            let (speed, rtl) = self.marquee_motion();
            let a = self.ticker.offset(
                &ctx.monitor_id,
                &self.marquee_policy(),
                speed,
                rtl,
                self.ticker.elapsed_ms(),
            );
            self.pass_anim = a;
            a
        } else {
            self.pass_anim
        };
        let draw = anim.map(|(part, offset_px)| MarqueeDraw {
            item: ItemId::Media,
            part,
            offset_px,
        });
        let info = render_bar_with(&mut p, &theme, &items, ui, draw);
        // uprava lisaty: obrys kazdej polozky, zvyraznena polozka pod mysou / tahana a ciarka vlozenia (v oboch prechodoch)
        if let Some(ed) = &self.bar_edit {
            use transom_render::Painter as _;
            let w = ctx.client.width();
            let h = ctx.client.height();
            let s = ctx.dpi as f32 / 96.0;
            let px = |v: f32| (v * s).round() as i32;
            let dragging = ed
                .drag
                .as_ref()
                .filter(|d| d.moved && d.monitor == ctx.monitor_id);
            for (id, x0, x1) in bar_edit::spans(&info.placed) {
                let r =
                    transom_render::Rect::new(x0 + 1, px(3.0), (x1 - x0 - 2).max(1), h - px(6.0));
                let hot = dragging.is_some_and(|d| d.id == id)
                    || ed
                        .hover
                        .as_ref()
                        .is_some_and(|(m, i)| m == &ctx.monitor_id && *i == id);
                p.stroke_round_rect(
                    r,
                    px(5.0),
                    px(1.0).max(1),
                    if hot { theme.accent } else { theme.muted },
                );
            }
            if let Some(d) = dragging {
                let locked = self.config.center_locked();
                if let Some((_, _, lx)) =
                    bar_edit::drop_target(&info.placed, &items, w, d.id, d.x, locked)
                {
                    let lw = px(2.0).max(2);
                    p.fill_rect(
                        transom_render::Rect::new((lx - lw / 2).clamp(0, w - lw), 0, lw, h),
                        theme.accent,
                    );
                }
            }
            // pas dole: lista je v rezime upravy
            p.fill_rect(
                transom_render::Rect::new(0, h - px(2.0).max(2), w, px(2.0).max(2)),
                theme.accent,
            );
        }
        if !ctx.bookkeeping {
            return;
        }
        let placed = info.placed;
        let geom = info
            .overflow
            .iter()
            .find(|o| o.item == ItemId::Media)
            .map(|o| ticker::Geom {
                part: o.part,
                x: o.x,
                w: o.w,
                overflow_px: o.overflow_px,
                scale: ctx.dpi as f32 / 96.0,
            });
        self.ticker.painted(&ctx.monitor_id, anim.map(|a| a.1));
        if self.ticker.set_geom(&ctx.monitor_id, geom) {
            // text zacal/prestal pretekat alebo sa zmenila sirka: vyhodnotit, ci treba animaciu
            log(&format!(
                "posuvny text {}: {}",
                ctx.monitor_id,
                geom.map_or("nepretieka".to_string(), |g| format!(
                    "pretecenie {} px v mieste x={} sirka={}",
                    g.overflow_px, g.x, g.w
                ))
            ));
            self.kick_ticker();
        }
        let layout_same = self.layouts.get(&ctx.monitor_id) == Some(&placed);
        let (regs, actions): (Vec<Region>, Vec<Option<Action>>) =
            regions_of(&items, &placed).into_iter().unzip();
        self.region_actions.insert(ctx.monitor_id.clone(), actions);
        if regions::set(ctx.hwnd, regs) {
            log(&format!(
                "oblasti (tooltip/MSAA) aktualizovane: {}",
                ctx.monitor_id
            ));
        }
        self.paints += 1;
        PAINTS.store(self.paints, std::sync::atomic::Ordering::Relaxed);
        // snimok posuvneho textu s nezmenenym rozlozenim sa nelogguje (30 riadkov/s by len zahltilo log)
        if !(layout_same && anim.is_some()) {
            log(&format!(
                "paint #{} {} dpi={} {}x{} clock='{}' alfa={} casti=[{}]",
                self.paints,
                ctx.monitor_id,
                ctx.dpi,
                ctx.client.width(),
                ctx.client.height(),
                self.clock,
                self.config.appearance.alpha(),
                placed
                    .iter()
                    .map(|p| format!("{:?}.{}@{}+{}", p.item, p.part, p.x, p.w))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        self.layouts.insert(ctx.monitor_id.clone(), placed);
        self.bar_widths
            .insert(ctx.monitor_id.clone(), ctx.client.width());
        self.painted_shelf = self
            .shelf_bar()
            .map(|v| v.files.into_iter().map(|f| f.id).collect())
            .unwrap_or_default();
        // test `--test-menu`: scenar sa spusti po prvom vykresleni (rozlozenie uz existuje)
        if self.test_menu == 0 && flag("--test-menu") {
            self.test_menu = 1;
            schedule_timer(T_TESTMENU, 1500);
        }
        if self.live_tools.is_none() && flag("--test-tools") {
            self.live_tools = Some(live_tools::Live::default());
            schedule_timer(T_TESTTOOLS, 2500);
        }
        if self.test_ai == 0 && (flag("--test-ai") || flag("--test-ai-real")) {
            self.test_ai = 1;
            schedule_timer(T_TESTAI, 2000);
        }
        if self.test_update == 0 && flag("--test-update") {
            self.test_update = 1;
            schedule_timer(T_TESTUPD, 1500);
        }
    }

    fn on_event(&mut self, ev: &BarEvent) {
        let _slow = SlowEvent::start(ev);
        self.ensure_media();
        self.sync_audio();
        if self.periph.is_none() {
            self.sync_periph();
        }
        // sluzba nastrojov sa spusta, az ked je k dispozicii notifikator (po vytvoreni riadiaceho okna), nie v `App::new`
        if self.tools.is_none() {
            self.sync_tools();
        }
        if self.hotkey_tried.is_none() && notifier().is_some() {
            self.sync_hotkey();
        }
        // nove polozky (klavesnica, jas, vytazenie, mikrofon a kamera): hned po vytvoreni riadiaceho okna
        if !self.extras_started && notifier().is_some() {
            self.extras_started = true;
            self.sync_keys();
            self.sync_bright(false);
            self.sync_sysmon();
            self.sync_privacy(false);
            self.sync_ics();
            self.sync_updates();
            self.sync_weather();
            self.sync_island_watches();
            self.island_test_start();
        }
        self.ensure_usage();
        if !self.tray_synced && notifier().is_some() {
            // `--start-hidden` (test): zacat so skrytou listou, rovnako ako po "Skryt listu" (ikona sa vynuti)
            if flag("--start-hidden") {
                self.hidden = false;
                self.set_hidden(true);
            }
            self.sync_tray();
        }
        match ev {
            BarEvent::Pointer {
                kind: PointerKind::Move,
                ..
            } => {}
            // obsah sa nikdy nelogue: napisane znaky a cesty pustenych suborov len ako druh udalosti (a pocet)
            BarEvent::PopoverChar { .. } => log("PopoverChar"),
            // ostrov: zmeny v priecinku mostu (log moze byt v tom istom priecinku - zapis by sa tocil) a snimky ekvalizera
            BarEvent::Notify(N_BRIDGE)
            | BarEvent::Timer(T_ISLAND)
            | BarEvent::Timer(T_GLIDE)
            | BarEvent::Timer(T_EQ)
            | BarEvent::Timer(T_MEDIA)
            | BarEvent::Notify(N_EQ) => {}
            BarEvent::FilesDropped(p) => log(&format!("FilesDropped(n={})", p.len())),
            _ => log(&format!("{ev:?}")),
        }
        match ev {
            BarEvent::Wake => self.tick("wake"),
            BarEvent::TimeChanged => self.tick("zmena casu"),
            BarEvent::ThemeChanged => {
                if self.refresh_theme() {
                    invalidate_all();
                }
                // tema panela uloh sa moze zmenit nezavisle od temy aplikacie: znacka v oblasti oznameni ju nasleduje
                self.sync_tray();
            }
            BarEvent::Tray { kind, x, y } => {
                log(&format!("ikona v oblasti oznameni: {kind:?} @ {x},{y}"));
                match kind {
                    transom_platform::TrayKind::Select => {
                        // skryta lista: klik na ikonu ju vrati; inak otvori nastavenia
                        self.on_menu(if self.hidden {
                            MENU_SHOW
                        } else {
                            MENU_SETTINGS
                        })
                    }
                    transom_platform::TrayKind::Menu => {
                        self.menu_at = None;
                        self.open_menu(
                            popup::MenuAnchor::Screen { x: *x, y: *y },
                            menu_items(self.hidden),
                        );
                    }
                    // jedine oznamenie pri ikone je o novej verzii
                    transom_platform::TrayKind::Balloon => self.open_update_panel(),
                }
            }
            BarEvent::MonitorsFallback(fb) => {
                // vybrany monitor nie je pripojeny: lista je na primarnom a **viditelne to hlasi** (nic sa nepresuva ticho)
                log(&format!(
                    "vybrane monitory nepripojene, zaloha na primarnom: {fb}"
                ));
                self.update("monitory", |a| a.display_fallback = *fb);
            }
            BarEvent::Broadcast(i) if *i == self.bc_restore => self.set_hidden(false),
            BarEvent::PowerChanged => self.update("napajanie", |a| a.refresh_battery()),
            BarEvent::Notify(N_MEDIA) => self.on_media(),
            BarEvent::Notify(N_FRAME) => self.marquee_tick(),
            BarEvent::Timer(T_VERIFY) => self.verify(),
            BarEvent::Timer(T_STASH) => self.stash_open(),
            BarEvent::Timer(T_INFO) => self.info_end(),
            BarEvent::Timer(T_ISLAND) => self.island_tick(),
            BarEvent::Timer(T_GLIDE) => self.glide_tick(),
            BarEvent::Timer(T_EQ) => self.eq_frame(),
            BarEvent::Timer(T_MEDIA) => self.media_tick(),
            BarEvent::Notify(N_EQ) => self.eq_wake(),
            BarEvent::Notify(N_SHOTS) => self.shots_changed(),
            BarEvent::Notify(N_BRIDGE) => self.bridge_changed(),
            BarEvent::Timer(T_SNIP) => self.snip_poll(),
            BarEvent::Timer(T_WATCH) => {
                self.watching = false;
                self.watch_tick();
            }
            BarEvent::ColorPicked(c) => self.pick_done(*c),
            BarEvent::BarWheel {
                monitor_id,
                x,
                delta,
            } => {
                let m = monitor_id.clone();
                self.bar_wheel(&m, *x, *delta);
            }
            BarEvent::NotchPointer { kind, x, y } => self.stash_pointer(*kind, *x, *y),
            BarEvent::Timer(T_TESTMENU) => self.test_menu_step(),
            BarEvent::Timer(T_TESTAI) => self.test_ai_step(),
            BarEvent::Timer(T_TESTUPD) => self.test_update_step(),
            BarEvent::Timer(T_TESTTOOLS) => self.test_tools_step(),
            BarEvent::Timer(T_TESTBEAT) => self.test_tools_beat(),
            BarEvent::Notify(ai::N_USAGE) => {
                // prevziat vysledok vnutri `update`, aby sa porovnal obsah pred a po (inak by sa lista
                // neprekreslila, napr. pri zmene kruhu po novom zazname statusline)
                let mut got = false;
                self.update("vyuzitie AI", |a| got = a.ai.take_result());
                log(&format!(
                    "vyuzitie AI: zber dokonceny (novy vysledok: {got})"
                ));
                // data sa zmazali pocas zberu: zastaraly vysledok sa zahodil, nacitat znova
                if self.ai.take_rerun() {
                    self.ai
                        .refresh(&self.config.usage, notifier(), ai::RefreshKind::Manual);
                }
                popup::invalidate_popover();
                self.schedule_usage();
            }
            BarEvent::Timer(T_USAGE) => {
                self.ai
                    .refresh(&self.config.usage, notifier(), ai::RefreshKind::Auto);
                // casovac je jednorazovy: znova (ak brana zber nepustila, Notify neprijde)
                self.schedule_usage();
            }
            BarEvent::Timer(T_MARQUEE) => self.marquee_tick(),
            BarEvent::Timer(T_KEYS) => self.sync_keys(),
            BarEvent::Timer(T_TIMER) => self.timer_tick(),
            BarEvent::AnimationsChanged => {
                self.reduced_motion = !transom_platform::system::animations_enabled();
                log(&format!("animacie v systeme: {}", !self.reduced_motion));
                invalidate_all();
                self.kick_ticker();
            }
            BarEvent::Fullscreen { monitor_id, opened } => {
                if *opened {
                    self.hidden_bars.insert(monitor_id.clone());
                } else {
                    self.hidden_bars.remove(monitor_id);
                    self.kick_ticker();
                }
                self.sync_hotkey();
            }
            BarEvent::Timer(T_ACTIVATE) => {
                // jednorazove overenie, ze sa popredie naozaj zmenilo (rovnako ako pri Search)
                if let Some(h) = self.activate_target.take() {
                    let fg = input::foreground();
                    log(&format!(
                        "overenie aktivacie: ciel 0x{h:X}, popredie {fg:?}"
                    ));
                    if fg.hwnd != h {
                        self.show_notice(t("notice.activate_late"));
                    }
                }
            }
            BarEvent::Timer(T_NOTICE) => self.update("koniec oznamu", |a| a.notice = None),
            // Explorer sa restartoval: ikonu v oblasti oznameni prida riadiace okno platformy (raz); tu len diagnostika
            BarEvent::TaskbarCreated => log(&format!(
                "TaskbarCreated: ikona v oblasti oznameni pridana={}",
                transom_platform::tray::is_added()
            )),
            BarEvent::Pointer {
                monitor_id,
                kind,
                x,
                ..
            } => self.pointer(monitor_id, *kind, *x),
            BarEvent::Broadcast(i) if *i == self.bc_config => {
                self.reload_config();
                self.sync_weather();
                self.sync_island_watches();
                self.sync_keys();
                self.sync_bright(false);
                self.sync_sysmon();
                self.sync_privacy(false);
                self.sync_ics();
                self.sync_updates();
            }
            BarEvent::Broadcast(i) if *i == self.bc_update_check => {
                log("aktualizacie: kontrola z nastaveni");
                if let Some(u) = &self.updater {
                    u.check_now();
                }
            }
            BarEvent::Notify(N_UPDATE) => self.on_update(),
            BarEvent::Broadcast(i) if *i == self.bc_usage_deleted => {
                // nastavenia zmazali lokalne data: zahodit aj data v pamati (nove sa nacitaju len pri zapnutych zdrojoch)
                log("vyuzitie AI: lokalne data zmazane z nastaveni");
                // aj rozbehnuty zber sa stane zastaranym (nevzkriesi cache ani data v pamati)
                self.update("zmazanie dat AI", |a| a.ai.usage_deleted());
                // zapnute zdroje sa nacitaju znova hned (zmazany archiv uz nesmie ostat v pamati ani v grafe)
                self.ai
                    .refresh(&self.config.usage, notifier(), ai::RefreshKind::Manual);
                popup::invalidate_popover();
            }
            BarEvent::PopoverPointer {
                kind,
                pointer,
                x,
                y,
            } => self.on_popover_pointer(*kind, *pointer, *x, *y),
            BarEvent::PopoverKey {
                kind,
                vk,
                shift,
                repeat,
                ctrl,
            } if *kind == POP_TOOLS => {
                let (cmd, redraw) = if self.tools_panel.editor.is_some() {
                    if *ctrl && *vk == 0x56 {
                        let pasted = transom_platform::clipboard::read_text()
                            .is_some_and(|t| self.tools_panel.editor_paste(&t));
                        (None, pasted)
                    } else {
                        self.tools_panel.editor_key(*vk, *ctrl)
                    }
                } else {
                    self.tools_panel
                        .key_ctrl(*vk, *shift, *ctrl, *repeat, &self.tools_data)
                };
                if let Some(c) = cmd {
                    self.run_tools_cmd(c);
                }
                self.library_sync();
                if redraw {
                    popup::invalidate_popover();
                }
            }
            BarEvent::PopoverChar { kind, ch } if *kind == POP_TOOLS => {
                if self.tools_panel.editor_char(*ch) {
                    popup::invalidate_popover();
                }
            }
            BarEvent::PopoverClosed { kind } if *kind == POP_LAUNCHER => {
                // zatvorenie okna zrusi rozbehnute hladanie
                if let Some(rt) = &self.launcher {
                    rt.engine.cancel();
                }
            }
            BarEvent::PopoverClosed { kind } if *kind == POP_EDIT => {
                log("uprava lisaty: vypnuta");
                self.bar_edit = None;
                invalidate_all();
            }
            // znovu otvorene (dlazdica / spat zatvorili a otvorili panel): kniznica snimok ide znova
            BarEvent::PopoverOpened { kind } if *kind == POP_TOOLS => {
                self.tools_open = true;
                self.library_sync();
            }
            BarEvent::PopoverClosed { kind } if *kind == POP_TOOLS => {
                // rozpisana poznamka sa pri zatvoreni okna nestrati (ulozi sa); explicitne zahodenie zahodi poznamku uz skor
                self.editor_finish(true);
                // kniznica snimok prestane sledovat priecinky (uvolnia sa uchytky) a zahodi miniatury
                self.tools_open = false;
                self.library_sync();
            }
            BarEvent::PopoverWheel { kind, delta } if *kind == POP_TOOLS => {
                if self.tools_panel.wheel(*delta) {
                    self.glide_start();
                }
            }
            BarEvent::PopoverKey {
                kind,
                vk,
                shift,
                repeat,
                ..
            } if *kind == POP_AUDIO => {
                let snap = self.audio_snapshot();
                let (cmd, redraw) = self.audio_panel.key(*vk, *shift, *repeat, &snap);
                if let Some(c) = cmd {
                    self.run_audio_cmd(c);
                }
                if redraw {
                    popup::invalidate_popover();
                }
            }
            BarEvent::PopoverWheel { kind, delta } if *kind == POP_AUDIO => {
                let snap = self.audio_snapshot();
                let (cmd, redraw) = self.audio_panel.wheel(*delta, &snap);
                let slider = cmd.is_some();
                if let Some(c) = cmd {
                    self.run_audio_cmd(c);
                }
                if redraw && slider {
                    popup::invalidate_popover();
                } else if redraw {
                    self.glide_start();
                }
            }
            BarEvent::PopoverKey {
                kind,
                vk,
                shift,
                repeat,
                ..
            } if *kind == POP_PERIPH => {
                let (cmd, redraw) = self.periph_panel.key(*vk, *shift, *repeat);
                if let Some(c) = cmd {
                    self.run_periph_cmd(c);
                }
                if redraw {
                    popup::invalidate_popover();
                }
            }
            BarEvent::PopoverKey {
                kind,
                vk,
                shift,
                repeat,
                ..
            } if extras::is_kit(*kind) => {
                let (cmd, redraw) = self.kit.key(*vk, *shift, *repeat);
                if let Some(c) = cmd {
                    self.kit_cmd(*kind, c);
                }
                if redraw {
                    popup::invalidate_popover();
                }
            }
            BarEvent::PopoverWheel { kind, delta } if extras::is_kit(*kind) => {
                let (cmd, redraw) = self.kit.wheel(*delta);
                let slider = cmd.is_some();
                if let Some(c) = cmd {
                    self.kit_cmd(*kind, c);
                }
                if redraw && slider {
                    popup::invalidate_popover();
                } else if redraw {
                    self.glide_start();
                }
            }
            BarEvent::PopoverClosed { kind } if extras::is_kit(*kind) => self.kit_closed(*kind),
            BarEvent::PopoverWheel { kind, delta } if *kind == POP_PERIPH => {
                if self.periph_panel.wheel(*delta) {
                    self.glide_start();
                }
            }
            BarEvent::Notify(N_PERIPH) => {
                // snimka sa cita pri kresleni: prekreslit listu a panel (ak je otvoreny)
                invalidate_all();
                popup::invalidate_popover();
            }
            BarEvent::Notify(N_TOOLS) => {
                self.on_tools_changed();
                self.sync_launcher_lists();
            }
            BarEvent::Notify(N_LAUNCHER) => self.on_launcher_results(),
            BarEvent::Notify(N_ICONS) => {
                // dalsia hotova ikona: prekreslit len okno vyhladavaca alebo panel Zvuk (mixer)
                if matches!(popup::popover_open(), Some(POP_LAUNCHER | POP_AUDIO)) {
                    popup::invalidate_popover();
                }
            }
            // nove meranie: snimka sa cita pri kresleni, prekreslit listu
            BarEvent::Notify(N_SYSMON) => invalidate_all(),
            BarEvent::Notify(N_PRIV) => self.on_privacy(),
            BarEvent::Hotkey(id) if *id == HOTKEY_MIC => self.mic_hotkey(),
            BarEvent::Notify(N_BRIGHT) => {
                invalidate_all();
                if popup::popover_open() == Some(POP_BRIGHT) {
                    popup::invalidate_popover();
                }
            }
            BarEvent::Notify(N_WEATHER) => {
                invalidate_all();
                if self.notch_view == NotchView::Weather {
                    transom_platform::dragdrop::invalidate_notch();
                }
            }
            BarEvent::Notify(N_ICS) => {
                if popup::popover_open() == Some(POP_CALENDAR) {
                    popup::invalidate_popover();
                }
            }
            BarEvent::Notify(N_BT) => {
                if popup::popover_open() == Some(POP_AUDIO) {
                    popup::invalidate_popover();
                }
            }
            BarEvent::Notify(N_MIXER) => {
                if popup::popover_open() == Some(POP_AUDIO) {
                    popup::invalidate_popover();
                }
            }
            BarEvent::PopoverClosed { kind } if *kind == POP_AUDIO => {
                // mixer aplikacii nebezi, ked panel nie je otvoreny (zrusia sa registracie relacii)
                self.mixer = None;
                self.bt = None;
            }
            BarEvent::Notify(N_LIB) => {
                // novy zoznam alebo miniatura: prekreslit panel (kreslenie si zoberie hotove data); neotvoreny panel nic nekresli
                if self.tools_open {
                    popup::invalidate_popover();
                }
                if self.island_shots_open() {
                    transom_platform::dragdrop::invalidate_notch();
                }
            }
            BarEvent::Notify(N_LAUNCHER_APPS) => {
                if let Some(rt) = &self.launcher {
                    let g = rt.engine.query(&self.launcher_panel.query);
                    self.launcher_panel.generation = g;
                }
            }
            BarEvent::PopoverChar { kind, ch } if *kind == POP_LAUNCHER => {
                if let Some(c) = self.launcher_panel.ch(*ch) {
                    self.run_launcher_cmd(c);
                }
                popup::invalidate_popover();
            }
            BarEvent::PopoverKey { kind, vk, ctrl, .. } if *kind == POP_LAUNCHER => {
                let (cmd, redraw) = self.launcher_panel.key(*vk, *ctrl);
                if let Some(c) = cmd {
                    self.run_launcher_cmd(c);
                }
                if redraw {
                    popup::invalidate_popover();
                }
            }
            BarEvent::FilesDropped(paths) => {
                log(&format!("nastroje: pustenych poloziek: {}", paths.len()));
                self.add_to_shelf(paths.clone());
            }
            BarEvent::FileDataDropped(files) => {
                // obsah bez cesty (obrazok / subor zo stranky): ulozit k odlozenym suborom pod povodnym nazvom
                let dir = transom_tools::store::tools_dir()
                    .join("dropped")
                    .join(format!("{}-{}", ai::now_unix(), std::process::id()));
                let mut paths = Vec::new();
                if std::fs::create_dir_all(&dir).is_ok() {
                    for (k, (name, bytes)) in files.iter().enumerate() {
                        let mut p = dir.join(name);
                        if paths.iter().any(|q: &String| q == &p.to_string_lossy()) {
                            p = dir.join(format!("{k}-{name}"));
                        }
                        if std::fs::write(&p, bytes).is_ok() {
                            paths.push(p.to_string_lossy().into_owned());
                        }
                    }
                }
                log(&format!("nastroje: pustenych obsahov: {}", paths.len()));
                if !paths.is_empty() {
                    self.add_to_shelf(paths);
                }
            }
            BarEvent::Notify(N_PICKED) => {
                self.picking = false;
                let paths = self
                    .picked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .unwrap_or_default();
                // zrusenie dialogu = prazdny zoznam: ticho sa nic nestane
                log(&format!("nastroje: vybranych suborov: {}", paths.len()));
                if !paths.is_empty() {
                    self.add_to_shelf(paths);
                }
            }
            BarEvent::Notify(N_AUDIO) => {
                // snimka sa cita zo sluzby pri kreslení: staci prekreslit listu (ikona) a panel (ak je otvoreny)
                invalidate_all();
                popup::invalidate_popover();
            }
            BarEvent::PopoverKey { kind, vk, .. } if *kind == POP_AI => {
                if self.ai.key(*vk, pclock::today()) {
                    popup::invalidate_popover();
                }
            }
            BarEvent::PopoverWheel { kind, delta } if *kind == POP_AI => {
                // 120 = jeden zub kolieska = 1,5 riadka posunu
                let step = -*delta * self.ai.unit * 3 / 2 / 120;
                if self.ai.wheel_by(step) {
                    self.glide_start();
                }
            }
            BarEvent::PopoverOpened { kind } if *kind == POP_MENU => self.publish_menu(),
            BarEvent::PopoverClosed { kind } if *kind == POP_MENU => self.menu = None,
            BarEvent::PopoverKey { kind, vk, .. } if *kind == POP_MENU => self.on_menu_key(*vk),
            BarEvent::PopoverKey { kind, vk, .. } if *kind == POP_CALENDAR => {
                // sipka vlavo/vpravo = mesiac, Home = dnes
                match *vk {
                    0x25 => self.cal.shift(-1),
                    0x27 => self.cal.shift(1),
                    0x24 => {
                        self.cal
                            .click(transom_render::calendar::CalHit::Today, pclock::today());
                    }
                    _ => return,
                }
                popup::invalidate_popover();
            }
            BarEvent::Activated(r) => {
                use transom_platform::activate::Activation;
                log(&format!("aktivacia zdroja: {r:?}"));
                match r {
                    Activation::Refused { exe } => {
                        self.show_notice(tf("notice.activate_refused", &[exe]))
                    }
                    Activation::Requested { hwnd, .. } => {
                        self.activate_target = Some(*hwnd);
                        schedule_timer(T_ACTIVATE, 400);
                    }
                    Activation::NotFound => self.show_notice(t("notice.activate_notfound")),
                    _ => {}
                }
            }
            BarEvent::MenuChosen(id) => self.on_menu(*id),
            BarEvent::Hotkey(id) if *id == HOTKEY_LAUNCHER => self.launcher_hotkey(),
            BarEvent::Removed { monitor_id } => {
                self.ticker.forget(monitor_id);
                if self.hidden_bars.remove(monitor_id) {
                    self.sync_hotkey();
                }
                self.layouts.remove(monitor_id);
                self.bar_widths.remove(monitor_id);
                self.region_actions.remove(monitor_id);
            }
            BarEvent::Invoke { monitor_id, index } => {
                let a = self
                    .region_actions
                    .get(monitor_id)
                    .and_then(|v| v.get(*index))
                    .copied()
                    .flatten();
                log(&format!("MSAA invoke #{index} -> {a:?}"));
                if a == Some(Action::ShelfFile) {
                    // k-ta oblast so suborom Police = k-ty subor (subory su prve casti polozky, nikdy sa neskryvaju jednotlivo)
                    let k = self
                        .region_actions
                        .get(monitor_id)
                        .map(|v| {
                            v[..*index]
                                .iter()
                                .filter(|x| **x == Some(Action::ShelfFile))
                                .count()
                        })
                        .unwrap_or(0);
                    let out = self.shelf_click(k);
                    self.sound.cue(snd::Origin::Assistive, out);
                } else if let Some(a) = a {
                    let out = self.act(a, None);
                    self.sound.cue(snd::Origin::Assistive, out);
                }
            }
            _ => {}
        }
        // ostrov v strede: co ukazat (lacne; okno sa meni len pri zmene)
        if notifier().is_some() {
            self.island_sync();
        }
    }
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1))
        .cloned()
}

/// Diagnostika: obsluha udalosti, ktora trvala viac ako 100 ms (zablokovane UI vlakno), sa zapise do logu (len nazov druhu udalosti a trvanie,
/// nikdy obsah).
struct SlowEvent<'a> {
    t0: Instant,
    ev: &'a BarEvent,
}

impl<'a> SlowEvent<'a> {
    fn start(ev: &'a BarEvent) -> SlowEvent<'a> {
        SlowEvent {
            t0: Instant::now(),
            ev,
        }
    }
}

impl Drop for SlowEvent<'_> {
    fn drop(&mut self) {
        let ms = self.t0.elapsed().as_millis();
        if ms >= 100 {
            // len nazov druhu udalosti (text pred prvou zatvorkou), nikdy jej obsah
            let d = format!("{:?}", self.ev);
            let kind: String = d
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            log(&format!("POMALA UDALOST {kind} {ms} ms"));
        }
    }
}

fn flag(name: &str) -> bool {
    std::env::args().any(|x| x == name)
}

/// Je to automaticky test alebo simulacia (`--test-*`, `--sim-*`, `--secs`, `--no-tray`, `--ticker-stats`,
/// `--no-sound`)? Vtedy sa nikdy nevytvori zvukove zariadenie, aby skripty a testy neprehravali zvuky.
fn automation_run() -> bool {
    automation_args(std::env::args().skip(1))
}

fn automation_args(args: impl Iterator<Item = String>) -> bool {
    args.into_iter().any(|a| {
        a.starts_with("--test-")
            || a.starts_with("--sim-")
            || matches!(
                a.as_str(),
                "--secs" | "--no-tray" | "--ticker-stats" | "--no-sound"
            )
    })
}

/// `--sim-media "Nazov skladby"` alebo `"Nazov|paused"` - LEN na test vzhladu posuvneho textu (oznacene ako simulovane).
fn parse_sim_media(v: &str) -> Option<SessionInfo> {
    let (title, paused) = match v.rsplit_once('|') {
        Some((t, "paused")) => (t, true),
        _ => (v, false),
    };
    if title.trim().is_empty() {
        return None;
    }
    // "Nazov~Interpret"
    let (title, artist) = title.split_once('~').unwrap_or((title, ""));
    Some(SessionInfo {
        key: "simulated".into(),
        title: title.trim().to_string(),
        artist: artist.trim().to_string(),
        state: if paused {
            PlayState::Paused
        } else {
            PlayState::Playing
        },
        controls: Controls {
            play: true,
            pause: true,
            prev: true,
            next: true,
            ..Default::default()
        },
        ..Default::default()
    })
}

/// `--sim-battery 57` alebo `57,charging` - LEN na test vzhladu na stroji bez baterie (oznacene ako simulovane).
fn parse_sim_battery(v: &str) -> Option<Battery> {
    let mut it = v.split(',');
    let percent = it.next()?.trim().parse::<u32>().ok()?.min(100) as u8;
    let charging = it.any(|x| x.trim() == "charging");
    Some(Battery {
        percent: Some(percent),
        charging,
        on_ac: charging,
        saver: false,
    })
}

/// Jeden riadok o panike pre `--log`: vlakno a miesto v kode (subor:riadok), **bez textu paniky** (ten moze obsahovat cesty alebo
/// obsah). Release zostavenie ma `panic = "abort"`: po tomto riadku proces skonci, lista zmizne a Windows vrati pracovnu plochu
/// (overene meraniami v docs/PLAN.md); bez `--log` sa nic nezapisuje.
fn crash_note(thread: &str, location: &str) -> String {
    format!(
        "PANIKA vo vlakne '{thread}' na {location} (verzia {}); proces sa ukonci",
        env!("CARGO_PKG_VERSION")
    )
}

/// Nainstaluje hak paniky, ktory do logu (ak je zapnuty `--log`) zapise `crash_note` a potom zavola povodny hak.
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map_or_else(|| "?".to_string(), |l| format!("{}:{}", l.file(), l.line()));
        let note = crash_note(std::thread::current().name().unwrap_or("?"), &loc);
        // `try_lock` s kratkym opakovanim: ine vlakno moze prave zapisovat do logu, ale ak by zamok drzalo panikujuce
        // vlakno samo, hak nesmie uviaznut (po ~50 ms to vzda)
        for _ in 0..50 {
            match LOG.try_lock() {
                Ok(mut g) => {
                    if let Some((f, t0)) = g.as_mut() {
                        let _ = writeln!(f, "[{:8.2}s] {note}", t0.elapsed().as_secs_f32());
                    }
                    break;
                }
                Err(std::sync::TryLockError::Poisoned(_)) => break,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
            }
        }
        prev(info);
    }));
}

fn main() {
    transom_platform::init_dpi_awareness();

    if let Some(p) = arg("--log") {
        if let Ok(f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
        {
            if let Ok(mut g) = LOG.lock() {
                *g = Some((f, Instant::now()));
            }
        }
    }
    install_panic_hook();

    // Jedna instancia: dve listy naraz by sa skladali (zistene pri meraniach).
    // SAFETY: konstantny nazov mutexu; handle sa zatvori na konci `main` (pri skorom navrate ho zatvori koniec procesu).
    let mutex = unsafe { CreateMutexW(None, true, w!("Local\\transom-app-single-instance")) };
    let Ok(mutex) = mutex else { return };
    // SAFETY: GetLastError hned po CreateMutexW na tom istom vlakne.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        log("uz bezi, koncim");
        // ak je lista skryta (menu "Skryt listu"), opatovne spustenie ju vrati
        transom_platform::post_broadcast(transom_config::RESTORE_MESSAGE, 0);
        return;
    }

    // vyber monitorov: z konfiguracie (`displays`); `--monitors primary` ho len prepise (testy)
    let start_cfg = transom_config::load(&transom_config::default_path()).config;
    let monitors = if arg("--monitors").as_deref() == Some("primary") {
        MonitorSelection::Primary
    } else {
        selection_of(&start_cfg.displays)
    };
    let config = BarConfig {
        height_lp: arg("--height").and_then(|v| v.parse().ok()).unwrap_or(28),
        monitors,
        auto_hide: start_cfg.oled.auto_hide(),
    };
    let mode = arg("--theme")
        .and_then(|v| ThemeMode::parse(&v))
        .unwrap_or_default();
    let prefer_app = arg("--media-app").filter(|s| !s.trim().is_empty());
    let sim_battery = arg("--sim-battery").and_then(|v| parse_sim_battery(&v));
    let media = !flag("--no-media");
    log(&format!(
        "start {config:?} theme={mode:?} media={media} media_app={prefer_app:?} sim_battery={sim_battery:?} (SIMULOVANE)"
    ));
    let mut app = App::new(mode, prefer_app, sim_battery, media);
    app.no_tray = flag("--no-tray");
    // zvuky rozhrania: skutocne zariadenie sa otvori az pri prvom zvuku; pri testoch a simulaciach nikdy
    let automation = automation_run();
    app.sound = snd::start_system(snd::StartOptions {
        automation,
        log: |m| log(m),
    });
    app.sound.set_settings(&app.config.ui_sounds);
    // sluzba systemoveho vystupu zvuku: nikdy pri automatizacii (spusti sa po vytvoreni hostitela, ked ju lista chce)
    app.audio_allowed = !automation;
    // HID a Bluetooth sa pri testoch nikdy necitaju (vysledok by zavisel od zariadeni na stroji)
    app.periph_allowed = !automation;
    log(&format!(
        "zvuky rozhrania: zapnute={} hlasitost={}% automatizacia={automation} mlcky_prehravac={}",
        app.config.ui_sounds.enabled,
        app.config.ui_sounds.volume,
        app.sound.is_silent()
    ));
    if flag("--ticker-stats") {
        app.ticker.enable_stats();
    }
    app.sim_media = arg("--sim-media").and_then(|v| parse_sim_media(&v));
    if app.sim_media.is_some() {
        log("media: SIMULOVANA session (--sim-media), len test vzhladu");
    }
    log(&format!("bateria (skutocna): {:?}", app.battery));
    let secs = app.config.clock.seconds;
    let host = match BarHost::new(config, Box::new(app)) {
        Ok(h) => h,
        Err(e) => {
            log(&format!("BarHost::new zlyhal: {e}"));
            return;
        }
    };
    // prve prebudenie podla konfiguracie (pri sekundach na najblizsiu sekundu)
    schedule_wake(next_wake_delay_for(secs));
    if let Some(s) = arg("--secs").and_then(|v| v.parse::<u64>().ok()) {
        host.quit_after(Duration::from_secs(s));
    }
    let code = host.run();
    log(&format!(
        "koniec kod={code} kreslenia_celkom={}",
        PAINTS.load(std::sync::atomic::Ordering::Relaxed)
    ));
    let t = Instant::now();
    drop(host); // zrusi listy (ABM_REMOVE), potom App -> MediaService (Quit + join media vlakna)
    log(&format!("upratane za {} ms", t.elapsed().as_millis()));
    // SAFETY: handle z CreateMutexW vyssie, zatvara sa raz.
    unsafe {
        let _ = CloseHandle(mutex);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rozhodnutie o panike (docs/PLAN.md 8.24): release ma `panic = "abort"`. Ak by sa zmenilo, treba znova posudit zotavenie
    /// z paniky v zbere AI a v zdrojoch vyhladavaca (a dokumentaciu), preto tento test zlyha, kym sa to nespravi vedome.
    #[test]
    fn the_release_profile_keeps_panic_abort_as_documented() {
        let toml = include_str!("../../../Cargo.toml");
        let release = toml
            .split("[profile.release]")
            .nth(1)
            .expect("profil release")
            .split(
                "
[",
            )
            .next()
            .unwrap();
        assert!(release.contains("panic = \"abort\""), "{release}");
        assert!(
            include_str!("../../../CONTRIBUTING.md").contains("panic = \"abort\""),
            "CONTRIBUTING.md vysvetluje pravidlo pre vlakna a Win32 callbacky"
        );
    }

    #[test]
    fn a_panic_note_names_the_thread_and_place_but_never_the_panic_text() {
        let n = crash_note("transom-usage", "crates/x/src/y.rs:42");
        assert!(n.contains("transom-usage") && n.contains("crates/x/src/y.rs:42"));
        assert!(n.contains(env!("CARGO_PKG_VERSION")));
        // hak zapise riadok do logu a text paniky (moze niest cesty alebo obsah) vynecha
        let dir = std::env::temp_dir().join(format!("transom-hook-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("hook.log");
        let f = std::fs::File::create(&file).unwrap();
        *LOG.lock().unwrap() = Some((f, Instant::now()));
        install_panic_hook();
        let _ = std::thread::Builder::new()
            .name("probe-panic".into())
            .spawn(|| panic!("tajny obsah suboru heslo.txt"))
            .unwrap()
            .join();
        *LOG.lock().unwrap() = None;
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("PANIKA") && text.contains("probe-panic"),
            "{text}"
        );
        assert!(
            !text.contains("tajny obsah") && !text.contains("heslo"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_and_simulation_runs_never_get_a_sound_device() {
        let v = |a: &[&str]| automation_args(a.iter().map(|s| s.to_string()));
        for a in [
            "--test-menu",
            "--test-ai",
            "--test-ai-real",
            "--test-update",
            "--sim-battery",
            "--sim-media",
            "--secs",
            "--no-tray",
            "--ticker-stats",
            "--no-sound",
        ] {
            assert!(v(&[a]), "{a}");
        }
        assert!(!v(&[]));
        assert!(!v(&["--theme", "dark", "--height", "30", "--log", "x.log"]));
    }

    /// Izolovana konfiguracia pre test (skutocny `config.json` pouzivatela sa necita ani nemeni).
    fn isolated_config(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("transom-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.json");
        std::env::set_var("TRANSOM_CONFIG", &file);
        (dir, file)
    }

    #[test]
    fn reloading_the_config_applies_the_sound_setting_at_once_including_mute() {
        let _g = crate::test_lang::guard();
        let (dir, file) = isolated_config("sound-reload");
        let write = |enabled: bool, volume: u32| {
            let c = transom_config::Config {
                ui_sounds: transom_config::UiSounds { enabled, volume },
                ..Default::default()
            };
            transom_config::save(&file, &c).unwrap();
        };
        write(true, 40);
        let mut a = App::new(ThemeMode::default(), None, None, false);
        assert_eq!(
            a.sound.settings(),
            transom_config::UiSounds {
                enabled: true,
                volume: 40
            }
        );
        // stlmenie v nastaveniach plati hned po sprave o zmene konfiguracie
        write(false, 40);
        a.reload_config();
        assert!(!a.sound.settings().enabled);
        assert_eq!(a.sound.settings().effective_volume(), 0);
        // poskodeny subor nezapne zvuky ani nezmeni hlasitost
        std::fs::write(&file, "{ poskodene").unwrap();
        a.reload_config();
        assert!(!a.sound.settings().enabled);
        std::env::remove_var("TRANSOM_CONFIG");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_app_has_a_silent_player_so_no_test_can_open_the_audio_device() {
        let _g = crate::test_lang::guard();
        let (dir, _file) = isolated_config("sound-silent");
        let a = App::new(ThemeMode::default(), None, None, false);
        std::env::remove_var("TRANSOM_CONFIG");
        let _ = std::fs::remove_dir_all(dir);
        assert!(a.sound.is_silent());
        // zvuky v konfiguracii su predvolene vypnute
        assert!(!transom_config::Config::default().ui_sounds.enabled);
        assert!(!a.sound.cue(snd::Origin::Pointer, snd::Outcome::Done));
    }

    #[test]
    fn context_menu_has_the_localized_items_and_the_hidden_menu_can_restore() {
        // prepina globalny jazyk cez vsetky jazyky: bez zamku by rusil testy, ktore ocakavaju anglictinu
        let _lang = crate::test_lang::guard();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            let m = menu_items(false);
            let ids: Vec<u32> = m.iter().map(|(i, _)| *i).collect();
            assert_eq!(
                ids,
                [
                    MENU_SETTINGS,
                    MENU_EDIT,
                    MENU_CUSTOMIZE,
                    MENU_AI,
                    MENU_TOOLS,
                    MENU_HIDE,
                    0,
                    MENU_QUIT
                ],
                "{lang:?}"
            );
            // kazda polozka ma neprazdny, navzajom odlisny preklad
            let labels: Vec<&String> = m.iter().filter(|(i, _)| *i != 0).map(|(_, l)| l).collect();
            assert!(labels.iter().all(|l| !l.trim().is_empty()), "{lang:?}");
            for (i, a) in labels.iter().enumerate() {
                for b in &labels[i + 1..] {
                    assert_ne!(a, b, "{lang:?}");
                }
            }
            // skryta lista (menu ikony v oblasti oznameni): "Zobrazit listu" je prva polozka a "Skryt" tam nie je
            let h = menu_items(true);
            assert_eq!(h[0].0, MENU_SHOW);
            assert!(!h
                .iter()
                .any(|(i, _)| *i == MENU_HIDE || *i == MENU_CUSTOMIZE));
            assert!(
                h.iter().any(|(i, _)| *i == MENU_QUIT)
                    && h.iter().any(|(i, _)| *i == MENU_SETTINGS)
            );
        }
        // anglicke nazvy podla zadania
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let en: Vec<String> = menu_items(false).into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            en,
            [
                "Settings",
                "Edit bar",
                "Customize bar",
                "AI usage…",
                "Tools…",
                "Hide bar",
                "",
                "Quit transom"
            ]
        );
    }

    #[test]
    fn display_selection_maps_config_modes() {
        use transom_config::{DisplayRef, Displays};
        assert_eq!(selection_of(&Displays::default()), MonitorSelection::All);
        let p = Displays {
            mode: "primary".into(),
            selected: vec![],
        };
        assert_eq!(selection_of(&p), MonitorSelection::Primary);
        let s = Displays {
            mode: "selected".into(),
            selected: vec![DisplayRef {
                id: "X".into(),
                label: String::new(),
            }],
        };
        assert_eq!(
            selection_of(&s),
            MonitorSelection::Selected(vec!["X".into()])
        );
    }

    #[test]
    fn sim_media_parsing() {
        let _lang = crate::test_lang::guard();
        let m = parse_sim_media("A very long title|paused").unwrap();
        assert_eq!(
            (m.title.as_str(), m.state),
            ("A very long title", PlayState::Paused)
        );
        assert_eq!(
            parse_sim_media("Playing one").unwrap().state,
            PlayState::Playing
        );
        assert_eq!(parse_sim_media("  |paused"), None);
    }

    /// Pristupne oblasti lisaty: batéria (predtym nevystavena) a hodnoty (hodiny, kruh AI) pre `accValue`.
    #[test]
    fn bar_regions_expose_the_battery_and_values_for_screen_readers() {
        let _lang = crate::test_lang::guard();
        let ring = ai::RingView {
            fill: Some(8),
            ccw: false,
            muted: false,
            alert: false,
            label: "8 % used".into(),
            name: "AI usage: Claude 8 % used".into(),
            detail: Some("Claude".into()),
        };
        let items = view::build_items(&view::View {
            clock: "17:25",
            battery: Some(Battery {
                percent: Some(42),
                charging: true,
                on_ac: true,
                saver: false,
            }),
            ai: Some(&ring),
            ..Default::default()
        });
        // kazda cast kazdej polozky dostane miesto (poloha nie je predmetom testu)
        let mut placed = Vec::new();
        let mut x = 0;
        for it in &items {
            for part in 0..it.parts.len() {
                placed.push(Placed {
                    item: it.id,
                    part,
                    x,
                    w: 40,
                });
                x += 40;
            }
        }
        let regs: Vec<Region> = regions_of(&items, &placed)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        let battery = regs
            .iter()
            .find(|r| r.name.starts_with(&t("item.battery")))
            .expect("baterii vidi citacka obrazovky");
        assert!(
            battery.name.contains("42") && !battery.button,
            "{battery:?}"
        );
        assert!(battery.value.contains("42"), "{battery:?}");
        let ai_region = regs
            .iter()
            .find(|r| r.value == "8 % used")
            .expect("kruh AI nesie hodnotu");
        assert!(ai_region.button && ai_region.name.contains("Claude"));
        let clock = regs
            .iter()
            .find(|r| r.value == "17:25")
            .expect("hodiny nesu cas ako hodnotu");
        assert!(clock.button, "hodiny otvaraju kalendar");
        // kazdy region ma neprazdny nazov
        assert!(regs.iter().all(|r| !r.name.trim().is_empty()), "{regs:?}");
    }

    #[test]
    fn sim_battery_parsing() {
        let _lang = crate::test_lang::guard();
        assert_eq!(parse_sim_battery("57").unwrap().percent, Some(57));
        assert!(parse_sim_battery("12,charging").unwrap().charging);
        assert_eq!(parse_sim_battery("300").unwrap().percent, Some(100));
        assert_eq!(parse_sim_battery("x"), None);
    }

    #[test]
    fn overflow_button_detection() {
        let _lang = crate::test_lang::guard();
        let f = |id: &str, name: &str| input::Focused {
            automation_id: id.into(),
            name: name.into(),
            class_name: "SystemTray.NormalButton".into(),
        };
        assert!(is_overflow_button(&f(
            "SystemTrayIcon",
            "Show Hidden Icons"
        )));
        // ina ikona s rovnakym AutomationId a triedou (indikator vstupu) -> ziadny Enter
        assert!(!is_overflow_button(&f(
            "SystemTrayIcon",
            "Tray Input Indicator English (United States)"
        )));
        assert!(!is_overflow_button(&f("", "Show Hidden Icons")));
    }
}
