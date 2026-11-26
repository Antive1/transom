//! Beh vyhladavaca: zostavenie zdrojov podla nastaveni, zoznamy (akcie transom, stranky Nastaveni Windows, aplikacie, poznamky,
//! Polica), napojenie na index Windows Search a nahrievanie na pozadi. Vsetko mimo UI vlakna; UI dostane hotove vysledky.
use crate::tools_pop::Section;
use std::sync::{Arc, Mutex};
use transom_i18n::t;
use transom_launcher::engine::Update;
use transom_launcher::sources::{
    app_item, FileSearch, FilesSource, IndexUnavailable, ListSource, Source,
};
use transom_launcher::{Cancel, Engine, Item, Kind, Target};
use transom_tools::service::Snapshot;

/// Najviac poloziek z jedneho zdroja zoznamu (obrana proti obrovskym zoznamom).
const MAX_LIST: usize = 5000;

pub struct Lists {
    pub apps: Arc<ListSource>,
    pub notes: Arc<ListSource>,
    pub shelf: Arc<ListSource>,
}

pub struct Rt {
    pub engine: Engine,
    pub lists: Lists,
    pub files: Option<Arc<FilesSource>>,
    pub cfg: transom_config::LauncherCfg,
    apps_loaded: Arc<dyn Fn() + Send + Sync>,
}

impl Rt {
    /// Znovu nacita zoznam aplikacii na pozadi (nainstalovane / odinstalovane aplikacie sa prejavia bez restartu).
    pub fn refresh_apps(&self) {
        if !self.cfg.apps {
            return;
        }
        let (list, done) = (self.lists.apps.clone(), self.apps_loaded.clone());
        let _ = std::thread::Builder::new()
            .name("transom-launcher-apps".into())
            .spawn(move || {
                use windows::Win32::System::Com::{
                    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
                };
                // SAFETY: COM pre toto vlakno na dobu vymenovania.
                let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
                let items: Vec<Item> = transom_platform::shellops::list_apps()
                    .into_iter()
                    .take(MAX_LIST)
                    .map(|(n, target)| app_item(&n, &target))
                    .collect();
                list.set(items);
                done();
                if init {
                    // SAFETY: parter k uspesnemu `CoInitializeEx`.
                    unsafe { CoUninitialize() };
                }
            });
    }
}

/// Stabilne id akcii transom -> (kluc nazvu, id).
pub const ACTIONS: [(&str, &str); 9] = [
    ("lau.a_settings", "settings"),
    ("lau.a_layout", "layout"),
    ("lau.a_ai", "ai"),
    ("lau.a_audio", "audio"),
    ("lau.a_calendar", "calendar"),
    ("lau.a_shelf", "shelf"),
    ("lau.a_clipboard", "clipboard"),
    ("lau.a_notes", "notes"),
    ("lau.a_shots", "shots"),
];

/// Pevne stranky Nastaveni Windows (dokumentovane `ms-settings:` URI).
pub const SETTINGS: [(&str, &str); 12] = [
    ("lau.s_display", "ms-settings:display"),
    ("lau.s_sound", "ms-settings:sound"),
    ("lau.s_network", "ms-settings:network"),
    ("lau.s_bluetooth", "ms-settings:bluetooth"),
    ("lau.s_apps", "ms-settings:appsfeatures"),
    ("lau.s_personalization", "ms-settings:personalization"),
    ("lau.s_privacy", "ms-settings:privacy"),
    ("lau.s_update", "ms-settings:windowsupdate"),
    ("lau.s_time", "ms-settings:dateandtime"),
    ("lau.s_access", "ms-settings:easeofaccess"),
    ("lau.s_storage", "ms-settings:storagesense"),
    ("lau.s_power", "ms-settings:batterysaver"),
];

pub fn action_items() -> Vec<Item> {
    ACTIONS
        .iter()
        .map(|(k, id)| Item::new(t(k), "transom", Kind::Action, Target::Action(id)))
        .collect()
}

pub fn setting_items() -> Vec<Item> {
    SETTINGS
        .iter()
        .map(|(k, uri)| Item::new(t(k), "Windows", Kind::Setting, Target::Uri(uri)))
        .collect()
}

/// Sekcia panela Nastroje pre akciu (`None` = nie je to akcia nastrojov).
pub fn tools_section(id: &str) -> Option<Section> {
    match id {
        "shelf" => Some(Section::Shelf),
        "clipboard" => Some(Section::Clipboard),
        "notes" => Some(Section::Notes),
        "shots" => Some(Section::Shots),
        _ => None,
    }
}

/// Poznamky a Polica z posledneho snimku sluzby nastrojov (len nazvy; obsah sa nehlada).
pub fn tools_items(s: &Snapshot) -> (Vec<Item>, Vec<Item>) {
    let notes = s
        .notes
        .iter()
        .filter(|n| !n.title.is_empty())
        .take(MAX_LIST)
        .map(|n| Item::new(n.title.clone(), "", Kind::Note, Target::Note(n.id.clone())))
        .collect();
    let shelf = s
        .shelf
        .iter()
        .take(MAX_LIST)
        .map(|r| {
            Item::new(
                r.name.clone(),
                r.location.clone(),
                Kind::Shelf,
                Target::Shelf(r.id.clone()),
            )
        })
        .collect();
    (notes, shelf)
}

/// Pocet dotazov na index od spustenia procesu (diagnostika a test `--test-tools`: dotazy sa nesmu opakovat sami od seba).
pub static INDEX_QUERIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

static TEST_INDEX_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Test: nastavi simulovane oneskorenie dotazu do indexu (0 = vypnute). Pouziva ho len scenar `--test-tools`.
pub fn set_test_index_delay_ms(ms: u64) {
    TEST_INDEX_DELAY_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

fn test_index_delay_ms() -> Option<u64> {
    let v = TEST_INDEX_DELAY_MS.load(std::sync::atomic::Ordering::Relaxed);
    (v > 0).then_some(v)
}

/// Index Windows Search: kazde volanie inicializuje COM na svojom vlakne (zdroje bezia na pracovnych vlaknach).
pub struct WinSearch;

impl FileSearch for WinSearch {
    fn query(
        &self,
        term: &str,
        limit: usize,
        cancel: &Cancel,
    ) -> Result<Vec<String>, IndexUnavailable> {
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
        };
        // jeden dotaz do indexu naraz: zastarane dotazy cakaju na zamok a po jeho ziskani uz neplatia (studeny `Execute` trva sekundy)
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        INDEX_QUERIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t_wait = std::time::Instant::now();
        let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let waited = t_wait.elapsed().as_millis();
        if cancel.is_cancelled() {
            crate::log(&format!(
                "vyhladavac: dotaz do indexu zastaral pocas cakania na zamok ({waited} ms), preskoceny"
            ));
            return Ok(Vec::new());
        }
        let safe = transom_launcher::sources::sql_like_term(term);
        if safe.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        // SAFETY: COM pre toto vlakno na dobu volania.
        let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        // test `--test-tools`: simulovany studeny start indexu (necancelovatelne cakanie ako skutocny `Execute`)
        if let Some(ms) = test_index_delay_ms() {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        let t_exec = std::time::Instant::now();
        let r = transom_platform::shellops::index_query(&safe, limit, &|| cancel.is_cancelled());
        crate::log(&format!(
            "vyhladavac: dotaz do indexu cakanie_na_zamok={waited} ms trvanie={} ms vysledkov={:?} zrusene={}",
            t_exec.elapsed().as_millis(),
            r.as_ref().map(|v| v.len()),
            cancel.is_cancelled()
        ));
        if init {
            // SAFETY: parter k uspesnemu `CoInitializeEx`.
            unsafe { CoUninitialize() };
        }
        r.map_err(|_| IndexUnavailable)
    }
}

/// Zostavi beh vyhladavaca podla nastaveni. `publish` dostava postupne vysledky; `recents` vrati nedavne volby (prazdne, ak
/// su vypnute). Aplikacie a index sa nahrievaju na pozadi (zoznam aplikacii sa nacita raz).
pub fn build(
    cfg: &transom_config::LauncherCfg,
    publish: Arc<dyn Fn(Update) + Send + Sync>,
    recents: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    on_apps_loaded: Arc<dyn Fn() + Send + Sync>,
) -> Rt {
    let lists = Lists {
        apps: ListSource::new("apps", Vec::new()),
        notes: ListSource::new("notes", Vec::new()),
        shelf: ListSource::new("shelf", Vec::new()),
    };
    let mut sources: Vec<Arc<dyn Source>> = Vec::new();
    if cfg.actions {
        sources.push(ListSource::new("actions", action_items()));
    }
    if cfg.settings {
        sources.push(ListSource::new("settings", setting_items()));
    }
    if cfg.apps {
        sources.push(lists.apps.clone());
    }
    if cfg.notes {
        sources.push(lists.notes.clone());
    }
    if cfg.shelf {
        sources.push(lists.shelf.clone());
    }
    let files = cfg.files.then(|| {
        Arc::new(FilesSource::new(
            Arc::new(WinSearch),
            cfg.fallback_folders.clone(),
            cfg.excluded_folders.clone(),
            cfg.max_results as usize * 2,
        ))
    });
    if let Some(f) = &files {
        sources.push(f.clone());
    }
    let engine = Engine::start(sources, cfg.max_results as usize, recents, publish);
    let apps_cb = on_apps_loaded.clone();
    // nahriatie: zoznam aplikacii a studeny start indexu (merane: prvy dotaz ~9 s, dalsie 25-60 ms) bez blokovania UI
    let apps = cfg.apps.then(|| lists.apps.clone());
    let warm = cfg.files;
    let _ = std::thread::Builder::new()
        .name("transom-launcher-warm".into())
        .spawn(move || {
            use windows::Win32::System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
            };
            // SAFETY: COM pre toto vlakno do konca nahrievania.
            let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            if let Some(a) = apps {
                let items: Vec<Item> = transom_platform::shellops::list_apps()
                    .into_iter()
                    .take(MAX_LIST)
                    .map(|(n, target)| app_item(&n, &target))
                    .collect();
                a.set(items);
                on_apps_loaded();
            }
            if warm {
                let _ = transom_platform::shellops::index_query("zz", 1, &|| false);
            }
            if init {
                // SAFETY: parter k uspesnemu `CoInitializeEx`.
                unsafe { CoUninitialize() };
            }
        });
    Rt {
        engine,
        lists,
        files,
        cfg: cfg.clone(),
        apps_loaded: apps_cb,
    }
}

/// Posledne vysledky z pracovneho vlakna cakajuce na prevzatie UI vlaknom.
pub type Mailbox = Arc<Mutex<Option<Update>>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_and_setting_has_a_text_in_the_current_language_and_a_fixed_target() {
        let _g = crate::test_lang::guard();
        for it in action_items().into_iter().chain(setting_items()) {
            assert!(
                !it.title.is_empty() && !it.title.starts_with("lau."),
                "{it:?}"
            );
            match it.target {
                Target::Uri(u) => {
                    assert!(u.starts_with("ms-settings:") && u.is_ascii() && !u.contains(' '))
                }
                Target::Action(id) => assert!(ACTIONS.iter().any(|(_, a)| *a == id)),
                other => panic!("neocakavany cil {other:?}"),
            }
        }
        assert_eq!(tools_section("notes"), Some(Section::Notes));
        assert_eq!(tools_section("settings"), None);
    }

    #[test]
    fn notes_and_shelf_items_carry_only_titles_and_names() {
        use transom_tools::service::{NoteRow, ShelfRow};
        let s = Snapshot {
            notes: vec![
                NoteRow {
                    id: "n1".into(),
                    title: "Nakup".into(),
                    pinned: false,
                    modified: 1,
                },
                NoteRow {
                    id: "n2".into(),
                    title: String::new(),
                    pinned: false,
                    modified: 1,
                },
            ],
            shelf: vec![ShelfRow {
                id: "s1".into(),
                name: "a.txt".into(),
                location: r"C:\x".into(),
                size: 1,
                copy: false,
                dir: false,
                pinned: false,
                state: transom_tools::shelf::State::Available,
                remote: false,
                path: r"C:\x\a.txt".into(),
            }],
            ..Default::default()
        };
        let (n, sh) = tools_items(&s);
        assert_eq!(n.len(), 1, "poznamka bez nazvu sa preskoci");
        assert_eq!(n[0].detail, "", "obsah poznamky sa nehlada ani nezobrazuje");
        assert_eq!(sh[0].target, Target::Shelf("s1".into()));
    }

    #[test]
    fn the_runtime_builds_with_every_source_off_and_answers_with_an_empty_result() {
        let mut cfg = transom_config::Config::default().tools.launcher;
        cfg.apps = false;
        cfg.files = false;
        cfg.settings = false;
        cfg.actions = false;
        cfg.notes = false;
        cfg.shelf = false;
        let got: Arc<Mutex<Vec<(usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let g2 = got.clone();
        let rt = build(
            &cfg,
            Arc::new(move |u| g2.lock().unwrap().push((u.hits.len(), u.done))),
            Arc::new(Vec::new),
            Arc::new(|| {}),
        );
        rt.engine.query("x");
        for _ in 0..200 {
            if !got.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got.lock().unwrap().first().copied(), Some((0, true)));
    }
}
