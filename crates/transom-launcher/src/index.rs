//! Vlastny index nazvov suborov (ako Everything): cesty k priecinkom a nazvy poloziek v pamati, hladanie podretazcov za
//! milisekundy. Zostavuje sa na pozadi prechodom priecinkov pouzivatela (profil bez AppData a priecinky na jednotkach mimo
//! systemu), uklada sa na disk (rychly start) a obnovuje sa, ked zastara. Cita sa len zoznam priecinkov (nazvy), nikdy obsah
//! suborov. Odkazy (reparse points), skryte a systemove priecinky a bezne "tazke" priecinky (`node_modules`, `.git`, ...)
//! sa vynechavaju.
use crate::engine::Cancel;
use crate::rank::{fold, score, tokens};
use crate::sources::{is_excluded, Source};
use crate::{Item, Kind, Target};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

/// Najviac poloziek v indexe (ochrana pamate pri obrovskych diskoch).
pub const MAX_ENTRIES: usize = 1_500_000;
/// Index starsi nez toto sa pri otvoreni vyhladavaca obnovi na pozadi.
pub const STALE_AFTER: Duration = Duration::from_secs(10 * 60);
/// Najviac kandidatov (zhoda podretazca), ktore sa este presne ohodnotia.
const MAX_CANDIDATES: usize = 4000;

/// Nazvy priecinkov, ktore sa neprechadzaju (vyvoj, vyrovnavacie pamate, kos).
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "target",
    "__pycache__",
    ".cache",
    ".vs",
    ".gradle",
    ".venv",
    "venv",
    "$recycle.bin",
    "system volume information",
];

/// Priecinky v koreni jednotky, ktore patria systemu (prechadzaju sa len pouzivatelske).
const SKIP_ROOT_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "users",
    "recovery",
    "perflogs",
    "$windows.~bt",
    "$windows.~ws",
    "windows.old",
    "$winreagent",
    "msocache",
    "intel",
    "amd",
    "nvidia",
    "drivers",
    "onedrivetemp",
];

#[derive(Clone, Copy)]
struct Entry {
    name: (u32, u16),
    key: (u32, u16),
    dir: u32,
    is_dir: bool,
}

/// Jeden hotovy index (nemeni sa; novy sa vymeni naraz).
#[derive(Default)]
pub struct Snapshot {
    dirs: Vec<Box<str>>,
    names: String,
    keys: String,
    entries: Vec<Entry>,
    pub built: Option<SystemTime>,
}

impl Snapshot {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn str_of(buf: &str, (o, l): (u32, u16)) -> &str {
        let (o, l) = (o as usize, l as usize);
        buf.get(o..o + l).unwrap_or("")
    }

    fn push(&mut self, dir: u32, name: &str, is_dir: bool) {
        let name: String = name.chars().take(400).collect();
        let key: String = name.chars().map(fold).collect();
        if name.len() > u16::MAX as usize || key.len() > u16::MAX as usize {
            return;
        }
        let n = (self.names.len() as u32, name.len() as u16);
        let k = (self.keys.len() as u32, key.len() as u16);
        self.names.push_str(&name);
        self.keys.push_str(&key);
        self.entries.push(Entry {
            name: n,
            key: k,
            dir,
            is_dir,
        });
    }

    /// Cesta polozky `i`.
    fn path(&self, e: &Entry) -> String {
        let dir = self.dirs.get(e.dir as usize).map_or("", |d| d);
        let name = Self::str_of(&self.names, e.name);
        if dir.ends_with('\\') {
            format!("{dir}{name}")
        } else {
            format!("{dir}\\{name}")
        }
    }

    /// Najlepsie zhody nazvu: vsetky slova dotazu ako podretazce nazvu (bez diakritiky), presne skore z `rank::score`.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        excluded: &[String],
        cancel: &Cancel,
    ) -> Vec<Item> {
        let toks = tokens(query);
        if toks.is_empty() || toks.iter().all(|t| t.chars().count() < 2) {
            return Vec::new();
        }
        let mut cands: Vec<(i32, usize)> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if i % 65536 == 0 && cancel.is_cancelled() {
                return Vec::new();
            }
            let key = Self::str_of(&self.keys, e.key);
            if toks.iter().all(|t| key.contains(t.as_str())) {
                let name = Self::str_of(&self.names, e.name);
                if let Some(s) = score(query, name, "") {
                    cands.push((s, i));
                    if cands.len() >= MAX_CANDIDATES {
                        break;
                    }
                }
            }
        }
        cands.sort_by_key(|c| std::cmp::Reverse(c.0));
        let mut out = Vec::new();
        for (_, i) in cands {
            if out.len() >= limit {
                break;
            }
            let e = &self.entries[i];
            let path = self.path(e);
            if is_excluded(&path, excluded) {
                continue;
            }
            let name = Self::str_of(&self.names, e.name).to_string();
            let parent = self.dirs.get(e.dir as usize).map_or("", |d| d).to_string();
            out.push(Item::new(
                name,
                parent,
                if e.is_dir { Kind::Folder } else { Kind::File },
                Target::Path(path),
            ));
        }
        out
    }

    /// Ulozi index (riadky: `D<tab>cesta`, potom `f<tab>nazov` / `d<tab>nazov` jeho poloziek).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("tmp");
        {
            let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            writeln!(w, "transom-file-index 1")?;
            let mut cur = u32::MAX;
            for e in &self.entries {
                if e.dir != cur {
                    cur = e.dir;
                    writeln!(w, "D\t{}", self.dirs.get(cur as usize).map_or("", |d| d))?;
                }
                let n = Self::str_of(&self.names, e.name);
                writeln!(w, "{}\t{n}", if e.is_dir { 'd' } else { 'f' })?;
            }
            w.flush()?;
        }
        std::fs::rename(&tmp, path)
    }

    /// Nacita ulozeny index (`None` = chyba alebo iny format).
    pub fn load(path: &Path) -> Option<Snapshot> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        if lines.next()? != "transom-file-index 1" {
            return None;
        }
        let mut s = Snapshot {
            built: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
            ..Snapshot::default()
        };
        let mut cur: Option<u32> = None;
        for l in lines {
            let (tag, rest) = l.split_once('\t')?;
            match tag {
                "D" => {
                    s.dirs.push(rest.into());
                    cur = Some(s.dirs.len() as u32 - 1);
                }
                "f" | "d" => s.push(cur?, rest, tag == "d"),
                _ => return None,
            }
            if s.entries.len() >= MAX_ENTRIES {
                break;
            }
        }
        Some(s)
    }
}

fn skip_dir(name: &str, root_level: bool) -> bool {
    let l = name.to_lowercase();
    SKIP_DIRS.contains(&l.as_str()) || (root_level && SKIP_ROOT_DIRS.contains(&l.as_str()))
}

/// Atributy, pre ktore sa polozka vynecha: skryta, systemova, odkaz (reparse point - aj prepojenia a OneDrive len v cloude
/// su normalne, odkazy na ine miesta by mohli tvorit slucky).
#[cfg(windows)]
fn skipped_attrs(md: &std::fs::Metadata) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;
    let a = md.file_attributes();
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const REPARSE: u32 = 0x400;
    const RECALL_ON_DATA: u32 = 0x0040_0000;
    let hidden = a & (HIDDEN | SYSTEM) != 0;
    let link = a & REPARSE != 0 && a & RECALL_ON_DATA == 0;
    (hidden, link)
}

#[cfg(not(windows))]
fn skipped_attrs(_md: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

/// Prejde `roots` a zostavi index. `root_level_skip`: korene, v ktorych sa vynechavaju systemove priecinky jednotky.
pub fn build(roots: &[(PathBuf, bool)], excluded: &[String], stop: &AtomicBool) -> Snapshot {
    let mut s = Snapshot::default();
    let mut stack: Vec<(PathBuf, bool)> = roots.to_vec();
    while let Some((dir, root_level)) = stack.pop() {
        if stop.load(Ordering::Relaxed) || s.entries.len() >= MAX_ENTRIES {
            break;
        }
        let ds = dir.to_string_lossy().to_string();
        if is_excluded(&ds, excluded) {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        s.dirs.push(ds.into());
        let di = s.dirs.len() as u32 - 1;
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            let (hidden, link) = skipped_attrs(&md);
            if hidden {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = md.is_dir() && !link;
            if md.is_dir() && skip_dir(&name, root_level) {
                continue;
            }
            s.push(di, &name, is_dir || md.is_dir());
            if is_dir {
                stack.push((e.path(), false));
            }
        }
    }
    s.built = Some(SystemTime::now());
    s
}

/// Predvolene korene: profil pouzivatela (bez AppData) a priecinky v korenoch pevnych jednotiek (bez systemovych).
pub fn default_roots() -> Vec<(PathBuf, bool)> {
    let mut roots: Vec<(PathBuf, bool)> = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        if let Ok(rd) = std::fs::read_dir(&home) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if n == "appdata" || n.starts_with('.') || n == "ntuser.dat" {
                    continue;
                }
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    roots.push((e.path(), false));
                }
            }
        }
    }
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if root.exists() && drive_is_fixed(&root) {
            roots.push((root, true));
        }
    }
    roots
}

#[cfg(windows)]
fn drive_is_fixed(root: &Path) -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    use std::os::windows::ffi::OsStrExt;
    let w: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: retazec ukonceny nulou zije pocas volania.
    let t = unsafe { GetDriveTypeW(w.as_ptr()) };
    t == 3 // DRIVE_FIXED
}

#[cfg(not(windows))]
fn drive_is_fixed(_root: &Path) -> bool {
    false
}

/// Index ako zdroj vyhladavaca: hlada v aktualnom snimku; obnova bezi na pozadi a snimok sa vymeni naraz.
pub struct IndexSource {
    snap: RwLock<Arc<Snapshot>>,
    building: AtomicBool,
    stop: Arc<AtomicBool>,
    excluded: Vec<String>,
    limit: usize,
    store: Option<PathBuf>,
}

impl IndexSource {
    /// Novy zdroj: nacita ulozeny index (ak je) a hned spusti obnovu na pozadi.
    pub fn start(excluded: Vec<String>, limit: usize, store: Option<PathBuf>) -> Arc<IndexSource> {
        let snap = store
            .as_deref()
            .and_then(Snapshot::load)
            .unwrap_or_default();
        let me = Arc::new(IndexSource {
            snap: RwLock::new(Arc::new(snap)),
            building: AtomicBool::new(false),
            stop: Arc::new(AtomicBool::new(false)),
            excluded,
            limit,
            store,
        });
        me.refresh();
        me
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snap.read().map(|g| Arc::clone(&g)).unwrap_or_default()
    }

    /// Obnovi index na pozadi, ak uz neprebieha.
    pub fn refresh(self: &Arc<Self>) {
        if self.building.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("transom-file-index".into())
            .spawn(move || {
                let s = build(&default_roots(), &me.excluded, &me.stop);
                if !me.stop.load(Ordering::Relaxed) {
                    if let Some(p) = &me.store {
                        let _ = s.save(p);
                    }
                    if let Ok(mut g) = me.snap.write() {
                        *g = Arc::new(s);
                    }
                }
                me.building.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            self.building.store(false, Ordering::Release);
        }
    }

    /// Obnova, ak je index starsi nez [`STALE_AFTER`] (napr. pri otvoreni vyhladavaca).
    pub fn refresh_if_stale(self: &Arc<Self>) {
        let old = self
            .snapshot()
            .built
            .and_then(|b| b.elapsed().ok())
            .is_none_or(|age| age > STALE_AFTER);
        if old {
            self.refresh();
        }
    }
}

impl Drop for IndexSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Source for IndexSource {
    fn name(&self) -> &'static str {
        "file-index"
    }
    fn search(&self, query: &str, cancel: &Cancel) -> Vec<Item> {
        self.snapshot()
            .search(query, self.limit, &self.excluded, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("transom-index-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn builds_searches_by_words_without_diacritics_and_skips_heavy_folders() {
        let d = tmp("build");
        std::fs::create_dir_all(d.join("Projekty").join("node_modules").join("x")).unwrap();
        std::fs::write(d.join("Projekty").join("Žiadosť o dovolenku.docx"), b"").unwrap();
        std::fs::write(
            d.join("Projekty")
                .join("node_modules")
                .join("x")
                .join("dovolenka.js"),
            b"",
        )
        .unwrap();
        std::fs::write(d.join("faktura-2026.pdf"), b"").unwrap();
        let s = build(&[(d.clone(), false)], &[], &AtomicBool::new(false));
        let c = Cancel::never();
        let hits = s.search("ziadost dovol", 10, &[], &c);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].title, "Žiadosť o dovolenku.docx");
        assert_eq!(hits[0].kind, Kind::File);
        assert!(
            s.search("dovolenka", 10, &[], &c).is_empty(),
            "node_modules sa neindexuje"
        );
        let f = s.search("Projekty", 10, &[], &c);
        assert_eq!(f[0].kind, Kind::Folder);
        // vynechany priecinok sa nevrati
        let ex = vec![d.join("Projekty").to_string_lossy().to_string()];
        assert!(s.search("ziadost", 10, &ex, &c).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn saved_index_loads_back_identically() {
        let d = tmp("save");
        std::fs::create_dir_all(d.join("a b")).unwrap();
        std::fs::write(d.join("a b").join("report final.txt"), b"").unwrap();
        let s = build(&[(d.clone(), false)], &[], &AtomicBool::new(false));
        let file = d.join("idx").join("files.idx");
        s.save(&file).unwrap();
        let l = Snapshot::load(&file).expect("nacitane");
        assert_eq!(l.len(), s.len());
        let c = Cancel::never();
        let a = s.search("report", 5, &[], &c);
        let b = l.search("report", 5, &[], &c);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].target, b[0].target);
        assert!(Snapshot::load(&d.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Meranie na skutocnom pocitaci: `cargo test -p transom-launcher real_index -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_index_build_and_search_times() {
        let t = std::time::Instant::now();
        let s = build(&default_roots(), &[], &AtomicBool::new(false));
        eprintln!(
            "zostavenie: {} poloziek, {} priecinkov, {:?}",
            s.len(),
            s.dirs.len(),
            t.elapsed()
        );
        eprintln!(
            "pamat: nazvy {} kB, kluce {} kB",
            s.names.len() / 1024,
            s.keys.len() / 1024
        );
        let c = Cancel::never();
        for q in [
            "transom",
            "readme",
            "img",
            "faktura",
            "razer battery",
            "xyzqwv",
        ] {
            let t = std::time::Instant::now();
            let h = s.search(q, 12, &[], &c);
            eprintln!("hladanie {q:?}: {} vysledkov, {:?}", h.len(), t.elapsed());
        }
        let f = std::env::temp_dir().join("transom-index-bench.idx");
        let t = std::time::Instant::now();
        s.save(&f).unwrap();
        eprintln!(
            "ulozenie: {:?}, {} kB",
            t.elapsed(),
            std::fs::metadata(&f).unwrap().len() / 1024
        );
        let t = std::time::Instant::now();
        let l = Snapshot::load(&f).unwrap();
        eprintln!("nacitanie: {:?} ({} poloziek)", t.elapsed(), l.len());
        let _ = std::fs::remove_file(&f);
    }
}
