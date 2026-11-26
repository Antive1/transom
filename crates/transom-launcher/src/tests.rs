use crate::engine::{Cancel, Engine, Update};
use crate::rank::{fold, norm, rank, score, tokens};
use crate::sources::IndexUnavailable;
use crate::sources::{
    is_excluded, sql_like_term, FileSearch, FilesSource, IndexState, ListSource, Source,
};
use crate::{allowed_acts, Act, Item, Kind, Target};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn app(name: &str, path: &str) -> Item {
    Item::new(name, path, Kind::App, Target::Path(path.into()))
}

fn file(path: &str) -> Item {
    let name = path.rsplit('\\').next().unwrap();
    Item::new(name, path, Kind::File, Target::Path(path.into()))
}

#[test]
fn diacritics_case_and_tokens_are_folded() {
    assert_eq!(fold('Č'), 'c');
    assert_eq!(fold('ľ'), 'l');
    assert_eq!(norm("Žiadosť o ČAS"), "ziadost o cas");
    assert_eq!(tokens("  Word   Exce  "), vec!["word", "exce"]);
    assert!(tokens(&"x ".repeat(50)).len() <= 8, "najviac 8 tokenov");
    assert!(tokens(&"y".repeat(500))[0].chars().count() <= 64);
    assert!(score("ziadost", "Žiadosť", "").is_some());
}

#[test]
fn exact_beats_prefix_beats_word_beats_substring_beats_fuzzy_beats_detail() {
    let s = |t: &str, d: &str| score("note", t, d);
    let exact = s("Note", "").unwrap();
    let prefix = s("Notepad", "").unwrap();
    let word = s("My Notes", "").unwrap();
    let sub = s("Footnote", "").unwrap();
    let fuzzy = s("Nice Old Text Editor", "").unwrap();
    let detail = s("Editor", r"C:\x\note.lnk").unwrap();
    assert!(
        exact > prefix && prefix > word && word > sub && sub > fuzzy && fuzzy > detail,
        "{exact} {prefix} {word} {sub} {fuzzy} {detail}"
    );
    assert_eq!(score("zzz", "Notepad", ""), None);
    assert_eq!(score("", "cokolvek", ""), Some(0));
}

#[test]
fn every_token_must_match_and_order_of_words_does_not_matter() {
    assert!(score("studio visual", "Visual Studio Code", "").is_some());
    assert_eq!(score("visual zzz", "Visual Studio Code", ""), None);
}

#[test]
fn ranking_is_stable_deduplicated_and_limited_with_category_bonus() {
    let items = vec![
        file(r"C:\docs\calc.txt"),
        app("Calculator", r"C:\ProgramData\Start\Calculator.lnk"),
        app("Calculator", r"C:\ProgramData\Start\Calculator.lnk"), // duplicita
        Item::new("Calendar", "", Kind::Action, Target::Action("calendar")),
        file(r"C:\docs\notes.txt"),
    ];
    let r = rank("cal", &items, 10, &[]);
    assert_eq!(r.len(), 3, "duplicita sa zlucila, nezhoda vypadla: {r:?}");
    assert_eq!(
        r[0].item.title, "Calculator",
        "aplikacia ma bonus pred akciou a suborom pri rovnakej predpone"
    );
    let limited = rank("cal", &items, 2, &[]);
    assert_eq!(limited.len(), 2);
    // opakovane radenie vrati rovnake poradie
    assert_eq!(rank("cal", &items, 10, &[]), r);
    // nedavne dostanu bonus len ak su zadane
    let rec = rank("cal", &items, 10, &["Calendar".to_string()]);
    assert_eq!(rec[0].item.title, "Calendar");
}

#[test]
fn actions_depend_on_the_kind_and_never_on_the_query_text() {
    let a = app("X", r"C:\a.lnk");
    assert_eq!(
        allowed_acts(&a),
        vec![Act::Open, Act::OpenContaining, Act::CopyPath]
    );
    let s = Item::new("Sound", "", Kind::Setting, Target::Uri("ms-settings:sound"));
    assert_eq!(allowed_acts(&s), vec![Act::Open]);
    let n = Item::new("Note", "", Kind::Note, Target::Note("id".into()));
    assert_eq!(allowed_acts(&n), vec![Act::Open]);
}

#[test]
fn sql_terms_cannot_change_the_query_structure() {
    assert_eq!(sql_like_term("it's"), "it''s");
    assert_eq!(sql_like_term("100%_[x]"), "100_x");
    assert_eq!(sql_like_term("a;DROP TABLE x--"), "aDROP TABLE x--");
    assert_eq!(sql_like_term("a\u{0}b\nc\"d"), "abcd");
    assert!(sql_like_term(&"a".repeat(1000)).len() <= 80);
    assert_eq!(sql_like_term("Žiadosť č.1"), "Žiadosť č.1");
    // ziadna medzera/znak nemoze skoncit retazec: apostrofy su vzdy v parach
    let t = sql_like_term("''''x'");
    assert_eq!(t.matches('\'').count() % 2, 0);
}

#[test]
fn excluded_folders_match_on_a_path_boundary_ignoring_case_and_slashes() {
    let ex = vec![r"C:\Users\Me\Private".to_string(), "D:/Secret/".to_string()];
    assert!(is_excluded(r"c:\users\me\private\a.txt", &ex));
    assert!(is_excluded(r"C:\Users\Me\Private", &ex));
    assert!(is_excluded(r"D:\Secret\x\y.txt", &ex));
    assert!(
        !is_excluded(r"C:\Users\Me\PrivateStuff\a.txt", &ex),
        "len cela hranica priecinka"
    );
    assert!(!is_excluded(r"C:\Users\Me\Public\a.txt", &ex));
    assert!(!is_excluded(r"C:\x", &[String::new()]));
}

struct FakeIndex(Result<Vec<String>, IndexUnavailable>, AtomicUsize);

impl FileSearch for FakeIndex {
    fn query(&self, _t: &str, _l: usize, _c: &Cancel) -> Result<Vec<String>, IndexUnavailable> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.clone()
    }
}

#[test]
fn the_files_source_filters_excluded_paths_and_falls_back_to_shallow_configured_folders() {
    let idx = Arc::new(FakeIndex(
        Ok(vec![
            r"C:\a\report.docx".into(),
            r"C:\Private\report2.docx".into(),
        ]),
        AtomicUsize::new(0),
    ));
    let src = FilesSource::new(idx.clone(), vec![], vec![r"C:\Private".into()], 20);
    let r = src.search("report", &Cancel::never());
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].detail, r"C:\a");
    assert_eq!(src.index_state(), IndexState::Available);
    assert!(
        src.search("a", &Cancel::never()).is_empty(),
        "jednoznakovy dotaz sa nehlada"
    );
    assert_eq!(idx.1.load(Ordering::SeqCst), 1);

    // index nedostupny: plytke prehladanie priecinka (bez rekurzie, bez citania obsahu)
    let d = std::env::temp_dir().join(format!("transom-launcher-fb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("budget-2026.xlsx"), b"x").unwrap();
    std::fs::write(d.join("other.txt"), b"budget inside content").unwrap();
    std::fs::write(d.join("sub").join("budget-deep.xlsx"), b"x").unwrap();
    let down = Arc::new(FakeIndex(Err(IndexUnavailable), AtomicUsize::new(0)));
    let src = FilesSource::new(down, vec![d.to_string_lossy().to_string()], vec![], 20);
    let r = src.search("budget", &Cancel::never());
    assert_eq!(
        src.index_state(),
        IndexState::Unavailable,
        "UI vysvetli obmedzenie"
    );
    assert_eq!(r.len(), 1, "iba priamy obsah a iba podla nazvu: {r:?}");
    assert_eq!(r[0].title, "budget-2026.xlsx");
    // vynechany priecinok sa nepouzije ani ako zaloha
    let src = FilesSource::new(
        Arc::new(FakeIndex(Err(IndexUnavailable), AtomicUsize::new(0))),
        vec![d.to_string_lossy().to_string()],
        vec![d.to_string_lossy().to_string()],
        20,
    );
    assert!(src.search("budget", &Cancel::never()).is_empty());
    // neexistujuci priecinok nespadne
    let src = FilesSource::new(
        Arc::new(FakeIndex(Err(IndexUnavailable), AtomicUsize::new(0))),
        vec![r"Z:\nonexistent".into()],
        vec![],
        20,
    );
    assert!(src.search("budget", &Cancel::never()).is_empty());
    let _ = std::fs::remove_dir_all(d);
}

struct Slow {
    delay: u64,
    seen: Arc<Mutex<Vec<String>>>,
    items: Vec<Item>,
    slow: bool,
    boom: bool,
}

impl Source for Slow {
    fn name(&self) -> &'static str {
        "slow"
    }
    fn slow(&self) -> bool {
        self.slow
    }
    fn search(&self, q: &str, c: &Cancel) -> Vec<Item> {
        self.seen.lock().unwrap().push(q.to_string());
        if self.boom {
            panic!("zdroj zlyhal");
        }
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(self.delay) {
            if c.is_cancelled() {
                return Vec::new();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.items.clone()
    }
}

fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    for _ in 0..1000 {
        if let Some(v) = f() {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    None
}

#[test]
fn rapid_typing_is_debounced_to_the_last_query_and_results_arrive_incrementally_fast_first() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fast = Arc::new(Slow {
        delay: 0,
        seen: seen.clone(),
        items: vec![app("Notepad", r"C:\n.lnk")],
        slow: false,
        boom: false,
    });
    let slowsrc = Arc::new(Slow {
        delay: 80,
        seen: Arc::new(Mutex::new(Vec::new())),
        items: vec![file(r"C:\notes.txt")],
        slow: true,
        boom: false,
    });
    let out: Arc<Mutex<Vec<(u64, usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let o2 = out.clone();
    let e = Engine::start(
        vec![slowsrc, fast],
        8,
        Arc::new(Vec::new),
        Arc::new(move |u: Update| {
            o2.lock()
                .unwrap()
                .push((u.generation, u.hits.len(), u.done))
        }),
    );
    e.query("n");
    e.query("no");
    e.query("not");
    let last = e.query("note");
    let done = wait_for(|| {
        out.lock()
            .unwrap()
            .iter()
            .find(|(g, _, d)| *g == last && *d)
            .copied()
    });
    let got = out.lock().unwrap().clone();
    assert!(done.is_some(), "{got:?}");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["note"],
        "debounce: hladal sa len posledny dotaz"
    );
    // najprv rychly zdroj (1 vysledok), potom aj pomaly (2 vysledky)
    let mine: Vec<_> = got.iter().filter(|(g, _, _)| *g == last).collect();
    assert_eq!(mine.len(), 2, "{got:?}");
    assert_eq!((mine[0].1, mine[0].2), (1, false));
    assert_eq!((mine[1].1, mine[1].2), (2, true));
    assert!(
        got.iter().all(|(g, _, _)| *g == last),
        "ziadne vysledky zastaranych dotazov"
    );
}

#[test]
fn a_slow_source_never_delays_the_fast_results_of_a_newer_query_and_stale_results_are_never_published(
) {
    // zdroj, ktory sa zrusit neda (ako studeny OLE DB `Execute`): spi 1500 ms bez kontroly zrusenia
    struct Stubborn;
    impl Source for Stubborn {
        fn name(&self) -> &'static str {
            "stubborn"
        }
        fn slow(&self) -> bool {
            true
        }
        fn search(&self, _q: &str, _c: &Cancel) -> Vec<Item> {
            std::thread::sleep(Duration::from_millis(1500));
            vec![file(r"C:\slow.txt")]
        }
    }
    let fast = ListSource::new(
        "actions",
        vec![Item::new(
            "Calendar",
            "",
            Kind::Action,
            Target::Action("calendar"),
        )],
    );
    let out: Arc<Mutex<Vec<(u64, usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let o2 = out.clone();
    let e = Engine::start(
        vec![Arc::new(Stubborn), fast],
        8,
        Arc::new(Vec::new),
        Arc::new(move |u: Update| {
            o2.lock()
                .unwrap()
                .push((u.generation, u.hits.len(), u.done))
        }),
    );
    let first = e.query("cal");
    std::thread::sleep(Duration::from_millis(150)); // po debounce uz hlada; pomaly zdroj spi
    let t0 = Instant::now();
    let second = e.query("cale");
    let got = wait_for(|| {
        out.lock()
            .unwrap()
            .iter()
            .find(|(g, h, _)| *g == second && *h == 1)
            .copied()
    });
    assert!(
        got.is_some(),
        "rychle vysledky noveho dotazu neprisli: {:?}",
        out.lock().unwrap()
    );
    assert!(
        t0.elapsed() < Duration::from_millis(600),
        "novy dotaz cakal na pomaly zdroj: {:?}",
        t0.elapsed()
    );
    // po dobehnuti pomaleho zdroja sa jeho zastarane vysledky (dotaz `first`/`second` uz nie je aktualny) nepublikuju
    e.cancel();
    std::thread::sleep(Duration::from_millis(1700));
    assert!(out
        .lock()
        .unwrap()
        .iter()
        .all(|(g, h, _)| *g != first || *h <= 1));
    assert!(
        !out.lock()
            .unwrap()
            .iter()
            .any(|(g, h, d)| *g == second && *d && *h == 2),
        "zrusene vysledky sa nesmu publikovat"
    );
}

/// Platí len v zostaveniach s odvijanim (testy, dev). Release ma `panic = "abort"`: tam panika zdroja ukonci proces a zdroje preto
/// nesmu panikovat; tento test dokumentuje len, ze izolacia vlakna sama o sebe funguje, kde je odvijanie k dispozicii.
#[cfg(panic = "unwind")]
#[test]
fn a_panicking_source_is_contained_where_unwinding_is_available() {
    let bad = Arc::new(Slow {
        delay: 0,
        seen: Arc::new(Mutex::new(Vec::new())),
        items: vec![],
        slow: false,
        boom: true,
    });
    let good = ListSource::new(
        "actions",
        vec![Item::new(
            "Calendar",
            "",
            Kind::Action,
            Target::Action("calendar"),
        )],
    );
    let out: Arc<Mutex<Vec<(usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let o2 = out.clone();
    let e = Engine::start(
        vec![bad, good],
        8,
        Arc::new(Vec::new),
        Arc::new(move |u: Update| o2.lock().unwrap().push((u.hits.len(), u.done))),
    );
    e.query("cal");
    let r = wait_for(|| out.lock().unwrap().iter().find(|(_, d)| *d).copied());
    assert_eq!(r, Some((1, true)), "{:?}", out.lock().unwrap());
}

#[test]
fn no_sources_and_no_matches_finish_with_an_empty_result() {
    let out: Arc<Mutex<Vec<(usize, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let o2 = out.clone();
    let e = Engine::start(
        vec![],
        8,
        Arc::new(Vec::new),
        Arc::new(move |u: Update| o2.lock().unwrap().push((u.hits.len(), u.done))),
    );
    e.query("x");
    assert_eq!(
        wait_for(|| out.lock().unwrap().first().copied()),
        Some((0, true))
    );
}

#[test]
fn enabled_sources_follow_the_configuration() {
    let mut c = transom_config::Config::default().tools.launcher;
    assert_eq!(
        crate::enabled_sources(&c),
        vec!["actions", "apps", "settings", "notes", "shelf", "files"]
    );
    c.files = false;
    c.apps = false;
    assert_eq!(
        crate::enabled_sources(&c),
        vec!["actions", "settings", "notes", "shelf"]
    );
}

/// Meranie pre rozhodnutie o navrhu (vypis do `--nocapture`) a hruba horna hranica, aby sa radenie nikdy nestalo pomalym: 5000 poloziek
/// (viac nez realne zoznamy aplikacii, nastaveni, poznamok a Police dokopy) sa zoradi v debug zostave pod 250 ms.
#[test]
fn ranking_five_thousand_items_stays_far_below_a_frame_budget_in_release_and_a_loose_bound_in_debug(
) {
    let items: Vec<Item> = (0..5000)
        .map(|i| {
            app(
                &format!("Application number {i} Studio Notes"),
                &format!(r"C:\ProgramData\Start\App{i}.lnk"),
            )
        })
        .collect();
    let t0 = Instant::now();
    let mut n = 0;
    for q in ["a", "app", "studio n", "notes 49", "zzzz"] {
        n += rank(q, &items, 8, &[]).len();
    }
    let per = t0.elapsed() / 5;
    eprintln!("MERANIE radenie 5000 poloziek: {per:?} na dotaz ({n} vysledkov)");
    assert!(per < Duration::from_millis(250), "{per:?}");
}

#[test]
fn score_edge_cases_table() {
    // (dotaz, nazov, podrobnost, ocakavane: Some = zhoda, None = nie)
    let cases: [(&str, &str, &str, bool); 12] = [
        ("   ", "Notepad", "", true),
        ("notepadx", "Notepad", "", false),
        ("n", "Notepad", "", true),
        ("x", "Notepad", "", false),
        ("x", "Notepad", r"C:\x.lnk", true),
        ("メモ", "メモ帳", "", true),
        ("帳", "メモ帳", "", true),
        ("np", "Notepad", "", true),
        ("pn", "Notepad", "", false),
        ("ÉCRAN", "écran de veille", "", true),
        ("łodz", "Łódź", "", true),
        ("", "", "", true),
    ];
    for (q, t, d, hit) in cases {
        assert_eq!(score(q, t, d).is_some(), hit, "{q:?} v {t:?}");
    }
    // jediny znak sa neberie ako postupnost (fuzzy), dlhy dotaz sa nikdy nekraja mimo nazvu
    assert!(score("n", "Notepad", "").unwrap() > 0);
    assert!(score(&"a".repeat(500), "a", "").is_none());
}

#[test]
fn ties_are_ordered_shorter_then_alphabetically_and_max_zero_is_empty() {
    let items = vec![
        app("Notes B", r"C:\b.lnk"),
        app("Notes A", r"C:\a.lnk"),
        app("Notes", r"C:\n.lnk"),
    ];
    let titles: Vec<String> = rank("notes", &items, 10, &[])
        .into_iter()
        .map(|h| h.item.title)
        .collect();
    assert_eq!(titles, ["Notes", "Notes A", "Notes B"]);
    assert!(rank("notes", &items, 0, &[]).is_empty(), "max 0 = nic");
    assert_eq!(rank("notes", &items, 1, &[]).len(), 1);
    // nedavna volba predbehne rovnako dobru zhodu, nie presnu zhodu
    let recent = rank("notes", &items, 10, &["Notes B".to_string()]);
    let titles: Vec<&str> = recent.iter().map(|h| h.item.title.as_str()).collect();
    assert_eq!(titles, ["Notes", "Notes B", "Notes A"]);
}
