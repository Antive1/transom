use super::*;
use std::sync::atomic::AtomicUsize;

fn raw(name: &str, modified: i64) -> Raw {
    Raw {
        name: name.into(),
        modified,
        bytes: 1000,
        attrs: 0,
        file: true,
    }
}

type Listing = Result<Vec<Raw>, DirState>;

/// Falosne vymenovanie: priecinok -> polozky alebo chyba; pocita volania a vie jeden priecinok zablokovat.
#[derive(Default)]
struct FakeFs {
    dirs: Mutex<HashMap<String, Listing>>,
    calls: Mutex<HashMap<String, usize>>,
    gate: Mutex<HashMap<String, mpsc::Receiver<()>>>,
}

impl FakeFs {
    fn set(&self, dir: &str, l: Listing) {
        self.dirs.lock().unwrap().insert(dir.to_string(), l);
    }

    fn calls(&self, dir: &str) -> usize {
        self.calls.lock().unwrap().get(dir).copied().unwrap_or(0)
    }

    fn total_calls(&self) -> usize {
        self.calls.lock().unwrap().values().sum()
    }
}

impl Lister for FakeFs {
    fn list(&self, dir: &str, f: &mut dyn FnMut(Raw) -> bool) -> Result<(), DirState> {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(dir.to_string())
            .or_insert(0) += 1;
        let gate = self.gate.lock().unwrap().remove(dir);
        if let Some(rx) = gate {
            let _ = rx.recv_timeout(Duration::from_secs(10));
        }
        let l = self.dirs.lock().unwrap().get(dir).cloned();
        match l {
            Some(Ok(v)) => {
                for r in v {
                    if !f(r) {
                        break;
                    }
                }
                Ok(())
            }
            Some(Err(e)) => Err(e),
            None => Err(DirState::Missing),
        }
    }
}

type Notify = Arc<dyn Fn(Change) + Send + Sync>;

struct Watching {
    notify: Mutex<Option<Notify>>,
    live: Arc<AtomicUsize>,
    starts: AtomicUsize,
}

struct Guard(Arc<AtomicUsize>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl WatchFactory for Watching {
    fn watch(
        &self,
        _: &[String],
        notify: Arc<dyn Fn(Change) + Send + Sync>,
    ) -> Box<dyn Any + Send> {
        *self.notify.lock().unwrap() = Some(notify);
        self.live.fetch_add(1, Ordering::SeqCst);
        self.starts.fetch_add(1, Ordering::SeqCst);
        Box::new(Guard(self.live.clone()))
    }
}

impl Watching {
    fn new() -> Arc<Watching> {
        Arc::new(Watching {
            notify: Mutex::new(None),
            live: Arc::new(AtomicUsize::new(0)),
            starts: AtomicUsize::new(0),
        })
    }

    fn fire(&self, c: Change) {
        if let Some(n) = self.notify.lock().unwrap().as_ref() {
            n(c);
        }
    }
}

fn wait(mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn dir(p: &str, default: bool) -> Dir {
    Dir {
        path: p.into(),
        default,
    }
}

fn lib(fs: &Arc<FakeFs>, w: &Arc<Watching>) -> (Library, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let w2 = wakes.clone();
    let mut deps = Deps::new(
        fs.clone(),
        w.clone(),
        Arc::new(move || {
            w2.fetch_add(1, Ordering::SeqCst);
        }),
    );
    deps.debounce = Duration::from_millis(60);
    deps.max_wait = Duration::from_millis(400);
    deps.deadline = Duration::from_millis(400);
    (Library::start(deps), wakes)
}

#[test]
fn only_images_are_listed_newest_first_with_a_stable_order_and_a_bounded_top_k() {
    let fs = FakeFs::default();
    let mut v = vec![
        raw("old.png", 100),
        raw("new.PNG", 300),
        raw("mid.jpg", 200),
        raw("notes.txt", 400),
        raw("archive.png.zip", 500),
        raw(".png", 600),
        raw("noext", 700),
        raw("B.png", 250),
        raw("a.png", 250),
    ];
    // priecinok a symbolicky odkaz nie su subory
    let mut folder = raw("folder.png", 800);
    folder.file = false;
    v.push(folder);
    fs.set(r"D:\Shots", Ok(v));
    let same = fs.dirs.lock().unwrap().get(r"D:\Shots").cloned().unwrap();
    fs.set(r"D:\Shots\", same);
    let s = scan_dir(r"D:\Shots", false, &fs, 100);
    let names: Vec<&str> = s.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["new.PNG", "a.png", "B.png", "mid.jpg", "old.png"],
        "najnovsie prve, pri rovnakom case podla nazvu bez ohladu na velkost pismen"
    );
    assert_eq!((s.seen, s.state), (5, DirState::Ok));
    assert_eq!(s.items[0].path, r"D:\Shots\new.PNG");
    // orezanie na K najnovsich: pocet "nasli sa" ostane pravdivy a pamat je ohranicena
    let s2 = scan_dir(r"D:\Shots", false, &fs, 2);
    assert_eq!(
        s2.items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["new.PNG", "a.png"]
    );
    assert_eq!(s2.seen, 5);
    // priecinok s koncovym lomitkom da tie iste cesty
    assert_eq!(
        scan_dir(r"D:\Shots\", false, &fs, 1).items[0].path,
        r"D:\Shots\new.PNG"
    );
}

#[test]
fn a_huge_folder_is_capped_and_cloud_and_network_files_are_flagged() {
    struct Many;
    impl Lister for Many {
        fn list(&self, _: &str, f: &mut dyn FnMut(Raw) -> bool) -> Result<(), DirState> {
            for i in 0..(MAX_ENUM + 50) {
                if !f(raw(&format!("{i}.png"), i as i64)) {
                    break;
                }
            }
            Ok(())
        }
    }
    let s = scan_dir(r"D:\Big", false, &Many, 50);
    assert_eq!(s.state, DirState::Capped);
    assert_eq!(s.seen, MAX_ENUM);
    assert_eq!(s.items.len(), 50);
    assert_eq!(
        s.items[0].modified,
        MAX_ENUM as i64 - 1,
        "z prejdenych su najnovsie prve"
    );
    // cloud atributy: OFFLINE, RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS (hodnoty z dokumentacie)
    assert!(is_cloud(0x1000) && is_cloud(0x4_0000) && is_cloud(0x40_0000));
    assert!(
        !is_cloud(0x20) && !is_cloud(0x400),
        "ARCHIVE a REPARSE_POINT samotne nie su cloud"
    );
    let fs = FakeFs::default();
    let mut c = raw("cloud.png", 5);
    c.attrs = 0x40_0000;
    fs.set(r"\\nas\shots", Ok(vec![c, raw("local.png", 4)]));
    let s = scan_dir(r"\\nas\shots", true, &fs, 10);
    assert!(s.items[0].cloud && s.items.iter().all(|e| e.remote));
    assert!(!s.items[1].cloud);
}

#[test]
fn folders_are_validated_normalised_deduplicated_and_never_a_whole_drive() {
    assert_eq!(clean_dir(r" D:/Shots/ ").as_deref(), Some(r"D:\Shots"));
    assert_eq!(
        clean_dir(r"\\nas\share\x\").as_deref(),
        Some(r"\\nas\share\x")
    );
    for bad in [
        "",
        "  ",
        "Shots",
        r"..\Shots",
        r"C:\",
        "C:",
        r"c:\\",
        r"\\?\C:\",
        r"\\.\PhysicalDrive0",
        r"\\?\GLOBALROOT\Device\x",
        r"\\",
        r"\\server",
        r"\\server\pipe\x",
        "D:\\a\u{7}b",
        r"D:\CON",
    ] {
        assert_eq!(clean_dir(bad), None, "{bad:?}");
    }
    let mut cfg = transom_config::Config::default().tools.screenshots;
    cfg.folders = vec![
        r"d:\pictures\shots".into(),
        r"D:\Pictures\Shots\".into(),
        r"E:\Other".into(),
        r"C:\".into(),
        "relative".into(),
    ];
    // predvoleny (zisteny pri otvoreni) je prvy; rovnaky priecinok pridany pouzivatelom sa zluci
    let d = plan_dirs(Some(r"D:\Pictures\Shots"), &cfg);
    assert_eq!(
        d,
        vec![dir(r"D:\Pictures\Shots", true), dir(r"E:\Other", false)]
    );
    // vypnuty predvoleny priecinok sa nepouzije, ani ked je zisteny; nezisteny predvoleny nic nepokazi
    cfg.default_folder = false;
    assert_eq!(plan_dirs(Some(r"X:\Shots"), &cfg).len(), 2);
    cfg.default_folder = true;
    assert_eq!(plan_dirs(None, &cfg).len(), 2);
}

#[test]
fn merging_folders_removes_duplicate_paths_and_keeps_the_truthful_total() {
    let fs = FakeFs::default();
    fs.set(r"D:\A", Ok(vec![raw("x.png", 10), raw("y.png", 30)]));
    fs.set(r"d:\a", Ok(vec![raw("x.png", 10), raw("z.png", 20)]));
    let a = scan_dir(r"D:\A", false, &fs, 10);
    let b = scan_dir(r"d:\a", false, &fs, 10);
    let (items, total) = merge(vec![a, b], 3);
    assert_eq!(
        items.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["y.png", "z.png", "x.png"]
    );
    assert_eq!(
        total, 4,
        "celkovy pocet pocita to, co sa nasli (duplicitu zluci az zoznam)"
    );
}

#[test]
fn the_real_lister_reads_only_listing_metadata_skips_folders_and_never_modifies_anything() {
    let root = crate::store::temp_dir("library-fs");
    let shots = root.join("Shots");
    std::fs::create_dir_all(shots.join("sub.png")).unwrap();
    let set = |name: &str, secs: u64, body: &[u8]| {
        let p = shots.join(name);
        std::fs::write(&p, body).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    };
    set("one.png", 1_700_000_000, b"AAAA");
    set("two.JPG", 1_800_000_000, b"BBBBBB");
    set("three.txt", 1_900_000_000, b"CC");
    let before: Vec<(String, Vec<u8>)> = ["one.png", "two.JPG", "three.txt"]
        .iter()
        .map(|n| (n.to_string(), std::fs::read(shots.join(n)).unwrap()))
        .collect();
    let s = scan_dir(shots.to_str().unwrap(), false, &FsLister, 100);
    let names: Vec<&str> = s.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["two.JPG", "one.png"]);
    assert_eq!(s.items[0].modified, 1_800_000_000);
    assert_eq!(s.items[0].bytes, 6);
    assert_eq!(s.state, DirState::Ok);
    // nic sa nezmenilo: obsah aj nazvy
    for (n, body) in before {
        assert_eq!(std::fs::read(shots.join(&n)).unwrap(), body);
    }
    assert!(shots.join("sub.png").is_dir());
    // chybajuci priecinok je stav, nie panika ani vytvorenie priecinka
    let gone = root.join("nope");
    let m = scan_dir(gone.to_str().unwrap(), false, &FsLister, 10);
    assert_eq!((m.state, m.items.len()), (DirState::Missing, 0));
    assert!(!gone.exists(), "kniznica priecinok nikdy nevytvara");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn opening_lists_watches_and_closing_releases_the_watch_while_keeping_the_last_list() {
    let fs = Arc::new(FakeFs::default());
    fs.set(r"D:\A", Ok(vec![raw("a.png", 1), raw("b.png", 2)]));
    fs.set(r"D:\Gone", Err(DirState::Missing));
    let w = Watching::new();
    let (l, wakes) = lib(&fs, &w);
    assert!(
        !l.snapshot().open,
        "zatvorena kniznica nic nesleduje ani nenacitava"
    );
    l.open(vec![dir(r"D:\A", true), dir(r"D:\Gone", false)], 100);
    assert!(wait(|| {
        let s = l.snapshot();
        s.open && !s.scanning && s.items.len() == 2
    }));
    let s = l.snapshot();
    assert_eq!(s.items[0].name, "b.png");
    assert_eq!(s.dirs.len(), 2);
    assert_eq!(
        s.dirs[0],
        DirInfo {
            default: true,
            remote: false,
            state: DirState::Ok
        }
    );
    assert_eq!(s.dirs[1].state, DirState::Missing);
    assert_eq!(s.problems(), 1);
    assert!(!s.truncated());
    assert_eq!(w.live.load(Ordering::SeqCst), 1);
    assert!(
        wakes.load(Ordering::SeqCst) >= 2,
        "UI sa zobudi pri zaciatku aj konci nacitania"
    );
    // zatvorenie uvolni sledovanie (uchytky priecinkov) a neskore udalosti uz nic nenacitaju
    l.close();
    assert!(wait(|| w.live.load(Ordering::SeqCst) == 0));
    let n = fs.total_calls();
    w.fire(Change::Changed);
    w.fire(Change::Overflow);
    l.refresh();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fs.total_calls(), n, "zatvorena kniznica sa nenacitava");
    assert_eq!(l.snapshot().items.len(), 2, "posledny zoznam ostal");
    assert!(!l.snapshot().open);
}

#[test]
fn bursts_of_changes_become_one_rescan_and_overflow_or_unavailability_also_rescan() {
    let fs = Arc::new(FakeFs::default());
    fs.set(r"D:\A", Ok(vec![raw("a.png", 1)]));
    let w = Watching::new();
    let (l, _) = lib(&fs, &w);
    l.open(vec![dir(r"D:\A", false)], 100);
    assert!(wait(
        || l.snapshot().items.len() == 1 && !l.snapshot().scanning
    ));
    assert_eq!(fs.calls(r"D:\A"), 1);
    // novy subor + serie udalosti (jedno ulozenie vyvola viac udalosti): jedno znovunacitanie
    fs.set(r"D:\A", Ok(vec![raw("a.png", 1), raw("b.png", 5)]));
    for _ in 0..20 {
        w.fire(Change::Changed);
    }
    assert!(wait(|| l.snapshot().items.len() == 2));
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(fs.calls(r"D:\A"), 2, "20 udalosti = 1 znovunacitanie");
    // odstranenie suboru
    fs.set(r"D:\A", Ok(vec![raw("b.png", 5)]));
    w.fire(Change::Changed);
    assert!(wait(|| l.snapshot().items.len() == 1));
    // preplnenie bufra aj nedostupny priecinok = bezpecne uplne znovunacitanie (nikdy sa nespolieha na neuplne udalosti)
    fs.set(r"D:\A", Ok(vec![raw("b.png", 5), raw("c.png", 9)]));
    w.fire(Change::Overflow);
    assert!(wait(|| l.snapshot().items.len() == 2));
    fs.set(r"D:\A", Err(DirState::Missing));
    w.fire(Change::Unavailable);
    assert!(wait(
        || l.snapshot().dirs.first().map(|d| d.state) == Some(DirState::Missing)
    ));
    assert!(l.snapshot().items.is_empty());
    // priecinok sa vratil: dalsia udalost ho znovu zobrazi
    fs.set(r"D:\A", Ok(vec![raw("z.png", 10)]));
    w.fire(Change::Changed);
    assert!(wait(|| l.snapshot().items.len() == 1));
    assert_eq!(l.snapshot().dirs[0].state, DirState::Ok);
    // trvale prudenie udalosti nezablokuje obnovu (najdlhsie oneskorenie)
    let gen = l.snapshot().generation;
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(900) {
        w.fire(Change::Changed);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        l.snapshot().generation > gen,
        "obnova prebehla aj pocas nepretrzitych zmien"
    );
}

#[test]
fn an_unreachable_folder_times_out_without_stalling_others_and_is_never_scanned_twice_at_once() {
    let fs = Arc::new(FakeFs::default());
    fs.set(r"D:\Fast", Ok(vec![raw("f.png", 7)]));
    fs.set(r"\\dead\share", Ok(vec![raw("x.png", 9)]));
    let (release, rx) = mpsc::channel();
    fs.gate.lock().unwrap().insert(r"\\dead\share".into(), rx);
    let w = Watching::new();
    let (l, _) = lib(&fs, &w);
    let t = Instant::now();
    l.open(
        vec![dir(r"D:\Fast", true), dir(r"\\dead\share", false)],
        100,
    );
    assert!(wait(|| {
        let s = l.snapshot();
        !s.scanning && s.open && s.dirs.len() == 2
    }));
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "limit 400 ms, nie cakanie na server"
    );
    let s = l.snapshot();
    assert_eq!(s.items.len(), 1, "rychly priecinok sa zobrazil");
    assert_eq!(s.dirs[1].state, DirState::Slow);
    assert!(s.dirs[1].remote, "UNC je sietovy priecinok");
    // dalsie obnovenie nespusti druhe vlakno pre ten isty zaseknuty priecinok
    // generacia sa cita pred obnovou: obnova ju moze zvysit skor, nez by ju test precital (pretek na pomalom stroji)
    let gen = l.snapshot().generation;
    l.refresh();
    assert!(wait(|| l.snapshot().generation >= gen + 2));
    assert_eq!(fs.calls(r"\\dead\share"), 1);
    // po uvolneni sa vlakno dokonci a nasledujuce obnovenie priecinok nacita normalne
    release.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    l.refresh();
    assert!(wait(|| {
        let s = l.snapshot();
        !s.scanning && s.dirs.len() == 2 && s.dirs[1].state == DirState::Ok
    }));
    assert_eq!(l.snapshot().items.len(), 2);
}

#[test]
fn a_default_folder_that_does_not_exist_yet_is_not_a_problem_but_a_missing_added_folder_is() {
    let fs = Arc::new(FakeFs::default());
    fs.set(r"D:\Default", Err(DirState::Missing));
    fs.set(r"E:\Mine", Ok(vec![raw("a.png", 1)]));
    fs.set(r"E:\Gone", Err(DirState::Missing));
    let w = Watching::new();
    let (l, _) = lib(&fs, &w);
    l.open(vec![dir(r"D:\Default", true), dir(r"E:\Mine", false)], 100);
    assert!(wait(
        || l.snapshot().items.len() == 1 && !l.snapshot().scanning
    ));
    assert_eq!(l.snapshot().dirs[0].state, DirState::Missing);
    assert_eq!(
        l.snapshot().problems(),
        0,
        "Windows vytvori priecinok pri prvej snimke"
    );
    l.open(vec![dir(r"D:\Default", true), dir(r"E:\Gone", false)], 100);
    assert!(wait(|| l.snapshot().items.is_empty()
        && !l.snapshot().scanning
        && l.snapshot().dirs.len() == 2));
    assert_eq!(
        l.snapshot().problems(),
        1,
        "chybajuci priecinok pridany pouzivatelom je problem"
    );
}

#[test]
fn no_folders_gives_an_empty_snapshot_without_a_watch_and_a_long_list_reports_truncation() {
    let fs = Arc::new(FakeFs::default());
    let w = Watching::new();
    let (l, _) = lib(&fs, &w);
    l.open(Vec::new(), 100);
    assert!(wait(|| l.snapshot().open && !l.snapshot().scanning));
    assert!(l.snapshot().items.is_empty() && l.snapshot().dirs.is_empty());
    assert_eq!(
        w.starts.load(Ordering::SeqCst),
        0,
        "bez priecinkov nie je co sledovat"
    );
    let many: Vec<Raw> = (0..50).map(|i| raw(&format!("{i}.png"), i)).collect();
    fs.set(r"D:\M", Ok(many));
    l.open(vec![dir(r"D:\M", false)], 20);
    assert!(wait(|| l.snapshot().items.len() == 20));
    let s = l.snapshot();
    assert_eq!((s.total, s.truncated()), (50, true));
    assert_eq!(s.items[0].name, "49.png");
}
