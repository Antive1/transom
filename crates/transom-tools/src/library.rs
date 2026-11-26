//! Kniznica snimok obrazovky: **iba cita** hotove obrazky z priecinkov na disku a zobrazuje ich od najnovsich. Nic nekopiruje,
//! nepremenuje, nemaze ani neuklada na disk (zoznam aj miniatury zijú len v pamati); otvorit sa smie snimka alebo jej priecinok.
//!
//! * Priecinky: predvoleny (Windows `FOLDERID_Screenshots`, zisti ho volajuci pri kazdom otvoreni) a priecinky pouzivatela;
//!   bez podpriecinkov, nikdy cely disk. Cesty prechadzaju `clean_dir` (absolutne, nie zariadenie, nie koren disku).
//! * Skenovanie: kazdy priecinok na vlastnom vlakne s casovym limitom, takze nedostupny sietovy priecinok nezdrzi ani ostatne
//!   priecinky, ani iné nastroje. Cas, velkost a atributy sa berú z vymenovania (subor sa neotvara); zoznam je ohraniceny
//!   (`max_items` najnovsich, najviac `MAX_ENUM` prejdenych poloziek na priecinok).
//! * Obnovenie: len kym je karta otvorena. Zmeny hlasi sledovanie priecinkov (`WatchFactory`); preplnenie bufra, nedostupny
//!   priecinok aj ruchle obnovenie vedu k bezpecnemu uplnemu znovunacitaniu. Prudke serie zmien sa zluuju (debounce).
//! * Sukromie: cesty ani nazvy suborov sa nelogu; modul nic nezapisuje.
use crate::lane::is_remote_path;
use crate::names::check_path;
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use transom_config::ShotsCfg;

/// Pripony, ktore sa povazuju za obrazky (porovnanie bez ohladu na velkost pismen).
pub const IMAGE_EXTS: [&str; 8] = ["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"];
/// Najviac prejdenych poloziek v jednom priecinku (obrana proti obrovskym priecinkom).
pub const MAX_ENUM: usize = 100_000;
/// Casovy limit skenovania priecinka; po jeho uplynuti sa priecinok oznaci ako pomaly a vlakno sa opusti.
pub const SCAN_DEADLINE: Duration = Duration::from_secs(4);
/// Ticho po poslednej zmene, po ktorom sa zoznam znovu nacita, a najdlhsie oneskorenie pri neustalych zmenach.
pub const DEBOUNCE: Duration = Duration::from_millis(350);
pub const MAX_WAIT: Duration = Duration::from_millis(1500);

// Dokumentovane atributy (GetFileAttributes): subor nie je plne lokalne, jeho citanie by ho mohlo stiahnut z cloudu.
const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL_ON_OPEN: u32 = 0x4_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

/// Subor nie je plne lokalne (OneDrive "len online" a pod.): nedekoduje sa, aby sa nestiahol.
pub fn is_cloud(attrs: u32) -> bool {
    attrs & (ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS) != 0
}

pub fn is_image_name(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > 255 || name.chars().any(|c| c.is_control()) {
        return false;
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => IMAGE_EXTS
            .iter()
            .any(|e| e.eq_ignore_ascii_case(ext.trim_end())),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub name: String,
    /// Cas poslednej zmeny (sekundy od 1970).
    pub modified: i64,
    pub bytes: u64,
    /// Subor nie je plne lokalne (cloud): miniatura sa nedekoduje.
    pub cloud: bool,
    /// Subor je na sietovom umiestneni: miniatura sa nedekoduje (nic sa cez siet neznasa automaticky).
    pub remote: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirState {
    Ok,
    Missing,
    Denied,
    /// Priecinok neodpovedal vcas (siet, odpojeny disk).
    Slow,
    /// Prejdenych `MAX_ENUM` poloziek; zoznam je neuplny.
    Capped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirInfo {
    pub default: bool,
    pub remote: bool,
    pub state: DirState,
}

/// Priecinok na skenovanie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir {
    pub path: String,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub generation: u64,
    /// Karta je otvorena (sledovanie zmien bezi).
    pub open: bool,
    /// Prave sa nacitava (zoznam nizsie je predchadzajuci).
    pub scanning: bool,
    pub items: Vec<Entry>,
    /// Kolko obrazkov sa nasli spolu (viac nez `items.len()`, ak sa zoznam orezal).
    pub total: usize,
    pub dirs: Vec<DirInfo>,
}

impl Snapshot {
    pub fn truncated(&self) -> bool {
        self.total > self.items.len()
    }

    /// Pocet priecinkov, ktore sa nepodarilo (uplne) nacitat. Predvoleny priecinok Windows, ktory este neexistuje, nie je
    /// problem (Windows ho vytvori pri prvej snimke): zobrazi sa ako prazdny zoznam.
    pub fn problems(&self) -> usize {
        self.dirs
            .iter()
            .filter(|d| !(d.state == DirState::Ok || (d.default && d.state == DirState::Missing)))
            .count()
    }
}

/// Hlavny priecinok je koren disku (`C:\`, `\\?\C:\`), co sa ako zdroj snimok neprijima.
fn is_drive_root(p: &str) -> bool {
    let t = p.strip_prefix(r"\\?\").unwrap_or(p);
    let b = t.as_bytes();
    b.len() >= 2
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && t[2..].trim_matches('\\').is_empty()
}

/// Ocisti a over cestu priecinka: oddelovace `\`, bez koncoveho `\`, absolutna lokalna alebo UNC (nie zariadenie, nie
/// koren disku). `None` = neprijatelne. Nic sa neotvara.
pub fn clean_dir(raw: &str) -> Option<String> {
    let mut p = raw.trim().replace('/', "\\");
    while p.len() > 3 && p.ends_with('\\') {
        p.pop();
    }
    check_path(&p).ok()?;
    if is_drive_root(&p) {
        return None;
    }
    Some(p)
}

/// Priecinky na skenovanie: predvoleny (ak je zapnuty a zisteny) a priecinky pouzivatela; neplatne sa preskocia, duplicity
/// (aj rozdiel vo velkosti pismen alebo koncovom `\`) sa zlucia. Najviac `TOOLS_MAX_SHOT_FOLDERS + 1`.
pub fn plan_dirs(default: Option<&str>, cfg: &ShotsCfg) -> Vec<Dir> {
    let mut out: Vec<Dir> = Vec::new();
    let mut add = |raw: &str, default: bool| {
        if let Some(p) = clean_dir(raw) {
            let key = p.to_lowercase();
            if !out.iter().any(|d| d.path.to_lowercase() == key) {
                out.push(Dir { path: p, default });
            }
        }
    };
    if cfg.default_folder {
        if let Some(d) = default {
            add(d, true);
        }
    }
    for f in cfg
        .folders
        .iter()
        .take(transom_config::TOOLS_MAX_SHOT_FOLDERS)
    {
        add(f, false);
    }
    out
}

/// Jedna polozka vymenovania priecinka.
#[derive(Clone, Debug)]
pub struct Raw {
    pub name: String,
    pub modified: i64,
    pub bytes: u64,
    pub attrs: u32,
    /// Skutocny subor (nie priecinok, symbolicky odkaz ani spojenie).
    pub file: bool,
}

/// Vymenovanie priecinka (v testoch falosne). Vracia chybu priecinka; `f` vracia `false`, ak sa ma vymenovanie skoncit.
pub trait Lister: Send + Sync {
    fn list(&self, dir: &str, f: &mut dyn FnMut(Raw) -> bool) -> Result<(), DirState>;
}

/// Skutocne vymenovanie cez `std::fs::read_dir` (cas, velkost a atributy z vymenovania, subory sa neotvaraju).
pub struct FsLister;

impl Lister for FsLister {
    fn list(&self, dir: &str, f: &mut dyn FnMut(Raw) -> bool) -> Result<(), DirState> {
        let rd = std::fs::read_dir(dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DirState::Missing,
            std::io::ErrorKind::PermissionDenied => DirState::Denied,
            _ => DirState::Failed,
        })?;
        for e in rd {
            let Ok(e) = e else { continue };
            let (Ok(ft), Ok(md)) = (e.file_type(), e.metadata()) else {
                continue;
            };
            let modified = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            #[cfg(windows)]
            let attrs = std::os::windows::fs::MetadataExt::file_attributes(&md);
            #[cfg(not(windows))]
            let attrs = 0;
            let raw = Raw {
                name: e.file_name().to_string_lossy().into_owned(),
                modified,
                bytes: md.len(),
                attrs,
                file: ft.is_file(),
            };
            if !f(raw) {
                break;
            }
        }
        Ok(())
    }
}

/// Najnovsie prve; pri rovnakom case podla nazvu (bez ohladu na velkost pismen), aby poradie ostalo stabilne.
fn sort_newest(v: &mut [Entry]) {
    v.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
}

pub struct DirScan {
    pub items: Vec<Entry>,
    /// Kolko obrazkov sa v priecinku nasli (aj tych, ktore sa nezmestili do `keep`).
    pub seen: usize,
    pub state: DirState,
}

/// Prehlada jeden priecinok (plytko): obrazky podla pripony, najviac `keep` najnovsich; pamat je ohranicena (`2 * keep`).
pub fn scan_dir(dir: &str, remote: bool, lister: &dyn Lister, keep: usize) -> DirScan {
    let keep = keep.max(1);
    let mut buf: Vec<Entry> = Vec::new();
    let (mut seen, mut visited) = (0usize, 0usize);
    let mut capped = false;
    let base = dir.trim_end_matches('\\').to_string();
    let res = lister.list(dir, &mut |r| {
        visited += 1;
        if visited > MAX_ENUM {
            capped = true;
            return false;
        }
        if r.file && is_image_name(&r.name) {
            seen += 1;
            buf.push(Entry {
                path: format!("{base}\\{}", r.name),
                name: r.name,
                modified: r.modified,
                bytes: r.bytes,
                cloud: is_cloud(r.attrs),
                remote,
            });
            if buf.len() >= keep * 2 {
                sort_newest(&mut buf);
                buf.truncate(keep);
            }
        }
        true
    });
    sort_newest(&mut buf);
    buf.truncate(keep);
    DirScan {
        items: buf,
        seen,
        state: match res {
            Ok(()) if capped => DirState::Capped,
            Ok(()) => DirState::Ok,
            Err(s) => s,
        },
    }
}

/// Zluci vysledky priecinkov: duplicity podla cesty (bez ohladu na velkost pismen) sa zlucia, najnovsie prve, najviac `keep`.
pub fn merge(scans: Vec<DirScan>, keep: usize) -> (Vec<Entry>, usize) {
    let total: usize = scans.iter().map(|s| s.seen).sum();
    let mut all: Vec<Entry> = scans.into_iter().flat_map(|s| s.items).collect();
    sort_newest(&mut all);
    let mut seen = std::collections::HashSet::new();
    all.retain(|e| seen.insert(e.path.to_lowercase()));
    all.truncate(keep);
    (all, total)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Pribudol, zmizol alebo sa zmenil subor.
    Changed,
    /// Buffer sledovania pretiekol alebo system nezaznamenal vsetky zmeny: treba znovu vymenovat.
    Overflow,
    /// Sledovany priecinok zmizol alebo je nedostupny.
    Unavailable,
}

/// Spusti sledovanie priecinkov a vrati predmet, ktoreho zahodenie sledovanie ukonci (a uvolni uchytky priecinkov).
pub trait WatchFactory: Send + Sync {
    fn watch(
        &self,
        dirs: &[String],
        notify: Arc<dyn Fn(Change) + Send + Sync>,
    ) -> Box<dyn Any + Send>;
}

enum Cmd {
    Open { dirs: Vec<Dir>, max: usize },
    Close,
    Refresh,
    Event,
    Stop,
}

pub struct Deps {
    pub lister: Arc<dyn Lister>,
    pub watcher: Arc<dyn WatchFactory>,
    /// Zavola sa po kazdom publikovani (z vlakna kniznice); volajuci ho zluci (jedna sprava na prekreslenie).
    pub wake: Arc<dyn Fn() + Send + Sync>,
    pub debounce: Duration,
    pub max_wait: Duration,
    pub deadline: Duration,
}

impl Deps {
    pub fn new(
        lister: Arc<dyn Lister>,
        watcher: Arc<dyn WatchFactory>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Deps {
        Deps {
            lister,
            watcher,
            wake,
            debounce: DEBOUNCE,
            max_wait: MAX_WAIT,
            deadline: SCAN_DEADLINE,
        }
    }
}

pub struct Library {
    tx: Sender<Cmd>,
    snap: Arc<Mutex<Arc<Snapshot>>>,
}

impl Library {
    pub fn start(deps: Deps) -> Library {
        let (tx, rx) = mpsc::channel();
        let snap: Arc<Mutex<Arc<Snapshot>>> = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let (s2, tx2) = (snap.clone(), tx.clone());
        let _ = std::thread::Builder::new()
            .name("transom-library".into())
            .spawn(move || run(rx, tx2, deps, s2));
        Library { tx, snap }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snap.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Karta sa otvorila: sleduj priecinky a nacitaj zoznam.
    pub fn open(&self, dirs: Vec<Dir>, max_items: usize) {
        let _ = self.tx.send(Cmd::Open {
            dirs,
            max: max_items.clamp(1, 5000),
        });
    }

    /// Karta sa zatvorila: skonci sledovanie (uvolni uchytky priecinkov); posledny zoznam ostane v pamati.
    pub fn close(&self) {
        let _ = self.tx.send(Cmd::Close);
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Cmd::Refresh);
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

struct Worker {
    deps: Deps,
    snap: Arc<Mutex<Arc<Snapshot>>>,
    dirs: Vec<Dir>,
    max: usize,
    cur: Snapshot,
    /// Priecinky, ktorych opusteny sken este bezi (nespusta sa dalsi).
    running: HashMap<String, Arc<AtomicBool>>,
}

impl Worker {
    fn publish(&mut self) {
        self.cur.generation += 1;
        *self.snap.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(self.cur.clone());
        (self.deps.wake)();
    }

    fn scan(&mut self) {
        self.cur.scanning = true;
        self.publish();
        let (tx, rx) = mpsc::channel::<(usize, DirScan)>();
        let mut states: Vec<Option<DirScan>> = Vec::new();
        let mut waiting = 0usize;
        let remote: Vec<bool> = self.dirs.iter().map(|d| is_remote_path(&d.path)).collect();
        for (i, d) in self.dirs.iter().enumerate() {
            states.push(None);
            let busy = self
                .running
                .get(&d.path.to_lowercase())
                .is_some_and(|f| !f.load(Ordering::SeqCst));
            if busy {
                // predosly sken tohto priecinka este nedobehol (zaseknuty): nespusta sa dalsi vlakno
                states[i] = Some(DirScan {
                    items: Vec::new(),
                    seen: 0,
                    state: DirState::Slow,
                });
                continue;
            }
            let done = Arc::new(AtomicBool::new(false));
            self.running.insert(d.path.to_lowercase(), done.clone());
            let (lister, path, keep, txi, rem) = (
                self.deps.lister.clone(),
                d.path.clone(),
                self.max,
                tx.clone(),
                remote[i],
            );
            let spawned = std::thread::Builder::new()
                .name("transom-library-scan".into())
                .spawn(move || {
                    let r = scan_dir(&path, rem, lister.as_ref(), keep);
                    done.store(true, Ordering::SeqCst);
                    let _ = txi.send((i, r));
                });
            if spawned.is_ok() {
                waiting += 1;
            } else {
                states[i] = Some(DirScan {
                    items: Vec::new(),
                    seen: 0,
                    state: DirState::Failed,
                });
            }
        }
        drop(tx);
        let start = Instant::now();
        while waiting > 0 {
            let left = self.deps.deadline.saturating_sub(start.elapsed());
            match rx.recv_timeout(left) {
                Ok((i, r)) => {
                    states[i] = Some(r);
                    waiting -= 1;
                }
                Err(_) => break,
            }
        }
        let mut infos = Vec::new();
        let mut scans = Vec::new();
        for (i, d) in self.dirs.iter().enumerate() {
            let s = states[i].take().unwrap_or(DirScan {
                // nedobehol vcas: priecinok sa oznaci ako pomaly a jeho vlakno sa opusti
                items: Vec::new(),
                seen: 0,
                state: DirState::Slow,
            });
            infos.push(DirInfo {
                default: d.default,
                remote: remote[i],
                state: s.state,
            });
            scans.push(s);
        }
        // dokoncene vlakna uz nie su "bezace": uvolnia sa zaznamy, aby sa priecinok mohol znovu skenovat
        self.running.retain(|_, f| !f.load(Ordering::SeqCst));
        let (items, total) = merge(scans, self.max);
        self.cur.items = items;
        self.cur.total = total;
        self.cur.dirs = infos;
        self.cur.scanning = false;
        self.publish();
    }
}

fn run(rx: mpsc::Receiver<Cmd>, tx: Sender<Cmd>, deps: Deps, snap: Arc<Mutex<Arc<Snapshot>>>) {
    let mut w = Worker {
        deps,
        snap,
        dirs: Vec::new(),
        max: 300,
        cur: Snapshot::default(),
        running: HashMap::new(),
    };
    // drzi sledovanie zive; zahodenie (`= None`) uvolni uchytky priecinkov
    let mut _watch: Option<Box<dyn Any + Send>> = None;
    let mut due: Option<Instant> = None;
    let mut first: Option<Instant> = None;
    loop {
        let msg = match due {
            Some(t) => match rx.recv_timeout(t.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
        };
        match msg {
            None => {
                due = None;
                first = None;
                if w.cur.open {
                    w.scan();
                }
            }
            Some(Cmd::Open { dirs, max }) => {
                _watch = None; // stare uchytky sa uvolnia pred novym sledovanim
                w.dirs = dirs;
                w.max = max;
                w.cur.open = true;
                if !w.dirs.is_empty() {
                    let paths: Vec<String> = w.dirs.iter().map(|d| d.path.clone()).collect();
                    let t2 = tx.clone();
                    _watch = Some(w.deps.watcher.watch(
                        &paths,
                        Arc::new(move |_| {
                            let _ = t2.send(Cmd::Event);
                        }),
                    ));
                }
                due = Some(Instant::now());
                first = None;
            }
            Some(Cmd::Close) => {
                _watch = None;
                due = None;
                first = None;
                w.cur.open = false;
                w.cur.scanning = false;
                w.publish();
            }
            Some(Cmd::Refresh) => {
                if w.cur.open {
                    due = Some(Instant::now());
                }
            }
            Some(Cmd::Event) => {
                // vsetky druhy zmien vedu k tomu istemu: znovu vymenovat (zmeny sa neciataju, nazvy sa nespracuvaju)
                if w.cur.open {
                    let now = Instant::now();
                    let f = *first.get_or_insert(now);
                    due = Some((now + w.deps.debounce).min(f + w.deps.max_wait));
                }
            }
            Some(Cmd::Stop) => return,
        }
    }
}

#[cfg(test)]
mod tests;
