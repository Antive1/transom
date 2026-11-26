//! Sluzba nastrojov: jedno pracovne vlakno vlastni vsetky ulozista (Polica, schranka, poznamky) a vykonava prikazy z UI,
//! takze UI vlakno nikdy necaka na disk, siet ani kopirovanie. Po kazdej zmene sa publikuje nemenna snimka (`Arc<Snapshot>`)
//! s ocistenymi riadkami na zobrazenie a vlakno upozorni UI (`notify`). Obsah sa nikdy nelogue.
use crate::clipboard::{self, ClipKind, History};
use crate::lane::LaneProbe;
use crate::names::clean_display;
use crate::notes::{NoteError, Notes};
use crate::shelf::{AddReport, FileProbe, FsProbe, Kind, RejectReason, Shelf, State};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use transom_config::Tools;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfRow {
    pub id: String,
    pub name: String,
    pub location: String,
    pub size: u64,
    pub copy: bool,
    pub dir: bool,
    pub pinned: bool,
    pub state: State,
    /// Sietova cesta (UNC, WebDAV, namapovany sietovy disk): nesonduje sa, netaha sa von a nekopiruje sa z nej.
    pub remote: bool,
    /// Cesta na tahanie von / kopirovanie cesty (odkaz = povodny subor, kopia = ulozeny subor).
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipRow {
    pub id: String,
    pub kind: ClipKind,
    pub preview: String,
    pub app: String,
    pub size: u64,
    pub pinned: bool,
    pub added: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRow {
    pub id: String,
    pub title: String,
    pub pinned: bool,
    pub modified: i64,
}

/// Vysledok poslednej operacie pre hlasku v UI (bez obsahu a ciest).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    Added {
        added: usize,
        duplicates: usize,
        rejected: usize,
        truncated: bool,
    },
    Rejected(RejectReason),
    /// Polozka Police sa presunula na miesto `pos` z `total` v zobrazenom zozname (pripnute su hore).
    Moved {
        pos: usize,
        total: usize,
    },
    /// Polozka sa v svojej skupine (pripnute / nepripnute) neda posunut dalej; nic sa nezmenilo.
    MoveEdge {
        first: bool,
    },
    Cleared(usize),
    NoteError(NoteError),
    /// Ulozisko sa nepouziva (poskodene alebo z novsej verzie): nic sa nezapisuje.
    StorageUnusable,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub generation: u64,
    pub shelf: Vec<ShelfRow>,
    pub clipboard: Vec<ClipRow>,
    pub notes: Vec<NoteRow>,
    pub clip_enabled: bool,
    pub clip_paused: bool,
    pub msg: Option<Msg>,
    /// Poznamka otvorena na upravu (id, text): nesie ju len snimka po `Cmd::NoteOpen`, aby sa pri bezne zmene neprenasal obsah.
    pub opened: Option<(String, String)>,
    /// Aspon jedno ulozisko je nepouzitelne (poskodene / novsia verzia).
    pub unusable: bool,
}

pub enum Cmd {
    Refresh,
    SetCfg(Box<Tools>),
    AddPaths {
        paths: Vec<String>,
        copy: bool,
    },
    ShelfPin(String),
    /// Posunie polozku Police o jedno miesto (`delta` < 0 hore, > 0 dole) v ramci pripnutych / nepripnutych.
    ShelfMove {
        id: String,
        delta: i8,
    },
    ShelfRemove(String),
    ShelfClear {
        keep_pinned: bool,
    },
    Clip(Box<clipboard::Snapshot>),
    ClipPin(String),
    ClipDelete(String),
    ClipClear {
        keep_pinned: bool,
    },
    /// Obnovi polozku do schranky (zapis vykona `ClipboardWriter`).
    ClipRestore(String),
    /// Zapise do schranky cestu k suboru (Polica / snimok) ako zoznam suborov; vyslovna akcia pouzivatela.
    Put(clipboard::Payload),
    /// Skopiruje text poznamky do schranky.
    NoteCopy(String),
    NoteOpen(String),
    /// UI prevzalo otvorenu poznamku (az potom sa prestane nosit v snimkach).
    NoteOpenDone,
    NoteCreate(String),
    NoteUpdate {
        id: String,
        text: String,
    },
    NotePin(String),
    NoteDelete(String),
    /// Zabudne poslednu hlasku (UI pred novou akciou, aby stara hlaska nezmatla novy vysledok).
    ClearMsg,
    Quit,
}

/// Zapis do systemovej schranky (vykona platforma; vracia, ci sa zapis podaril). Zapis musi oznacit vlastnu zmenu, aby sa nezachytila znova.
pub type ClipboardWriter = Arc<dyn Fn(clipboard::Payload) -> bool + Send + Sync>;

pub struct Service {
    tx: Sender<Cmd>,
    snap: Arc<Mutex<Arc<Snapshot>>>,
}

impl Service {
    pub fn start(
        dir: PathBuf,
        cfg: Tools,
        notify: Arc<dyn Fn() + Send + Sync>,
        writer: ClipboardWriter,
    ) -> Result<Service, String> {
        Self::start_with_lane(dir, cfg, notify, writer, LaneProbe::new(Arc::new(FsProbe)))
    }

    /// Ako `start`, ale s vlastnou sondou (testy: visiaca sonda). Pracovne vlakno nikdy nevola suborovy system na ceste od
    /// pouzivatela priamo, len cez tuto sondu s casovym limitom.
    pub fn start_with_lane(
        dir: PathBuf,
        cfg: Tools,
        notify: Arc<dyn Fn() + Send + Sync>,
        writer: ClipboardWriter,
        lane: LaneProbe,
    ) -> Result<Service, String> {
        let (tx, rx) = channel::<Cmd>();
        let snap = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let s2 = Arc::clone(&snap);
        std::thread::Builder::new()
            .name("transom-tools".into())
            .spawn(move || worker(rx, dir, cfg, s2, notify, writer, lane))
            .map_err(|e| e.to_string())?;
        let _ = tx.send(Cmd::Refresh);
        Ok(Service { tx, snap })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snap.lock().map(|g| Arc::clone(&g)).unwrap_or_default()
    }

    /// Odosielatel prikazov pre ine vlakna (napr. odber schranky).
    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    /// Posle prikaz (neblokuje; po zaniknuti vlakna sa ticho zahodi).
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

struct World {
    dir: PathBuf,
    cfg: Tools,
    shelf: Shelf,
    clip: History,
    notes: Notes,
    gen: u64,
    opened: Option<(String, String)>,
}

impl World {
    fn open(dir: PathBuf, cfg: Tools) -> World {
        let t = dir.clone();
        World {
            shelf: Shelf::open(t.join("shelf"), cfg.shelf.clone()),
            clip: History::open(t.join("clipboard"), cfg.clipboard.clone()),
            notes: Notes::open(t.join("notes"), cfg.notes.clone()),
            dir,
            cfg,
            gen: 0,
            opened: None,
        }
    }

    fn view(&mut self, msg: Option<Msg>, probe: &dyn FileProbe) -> Snapshot {
        self.gen += 1;
        let shelf = self
            .shelf
            .rows(probe)
            .into_iter()
            .map(|r| {
                let path = self
                    .shelf
                    .path_of(&r.id)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                ShelfRow {
                    id: r.id,
                    name: r.name,
                    location: clean_display(&r.location, 120),
                    size: r.size,
                    copy: r.kind == Kind::Copy,
                    dir: r.dir,
                    pinned: r.pinned,
                    state: r.state,
                    remote: r.kind == Kind::Ref && crate::lane::is_remote_path(&path),
                    path,
                }
            })
            .collect();
        Snapshot {
            generation: self.gen,
            shelf,
            clipboard: self
                .clip
                .entries()
                .iter()
                .map(|e| ClipRow {
                    id: e.id.clone(),
                    kind: e.kind,
                    preview: e.preview.clone(),
                    app: e.app.clone(),
                    size: e.size,
                    pinned: e.pinned,
                    added: e.added,
                })
                .collect(),
            notes: self
                .notes
                .list()
                .into_iter()
                .map(|n| NoteRow {
                    id: n.id.clone(),
                    title: Notes::title(n),
                    pinned: n.pinned,
                    modified: n.modified,
                })
                .collect(),
            clip_enabled: self.cfg.clipboard.enabled,
            clip_paused: self.cfg.clipboard.paused,
            msg,
            opened: self.opened.clone(),
            unusable: matches!(self.shelf.status, crate::shelf::Status::Unusable(_))
                || matches!(self.clip.status, clipboard::Status::Unusable(_))
                || matches!(self.notes.status, crate::notes::Status::Unusable(_)),
        }
    }
}

/// Vykona cakajuce ziadosti o vymazanie z nastaveni (znacky); vrati, ci sa nieco zmenilo.
fn apply_clear_requests(w: &mut World) -> Option<Msg> {
    let mut total = 0usize;
    let mut any = false;
    for store in crate::store::take_clear_requests(&w.dir) {
        any = true;
        total += match store {
            "shelf" => {
                let n = w.shelf.clear(false);
                let _ = w.shelf.save();
                n
            }
            "clipboard" => w.clip.clear(false),
            "notes" => w.notes.clear(false).unwrap_or(0),
            _ => 0,
        };
    }
    any.then_some(Msg::Cleared(total))
}

fn report_msg(r: &AddReport) -> Msg {
    if r.added == 0 && r.duplicates == 0 && r.rejected.len() == 1 && !r.truncated {
        return Msg::Rejected(r.rejected[0]);
    }
    Msg::Added {
        added: r.added,
        duplicates: r.duplicates,
        rejected: r.rejected.len(),
        truncated: r.truncated,
    }
}

fn worker(
    rx: std::sync::mpsc::Receiver<Cmd>,
    dir: PathBuf,
    cfg: Tools,
    out: Arc<Mutex<Arc<Snapshot>>>,
    notify: Arc<dyn Fn() + Send + Sync>,
    writer: ClipboardWriter,
    probe: LaneProbe,
) {
    let mut w = World::open(dir, cfg);
    let mut first = true;
    let mut last_msg: Option<Msg> = None;
    while let Ok(cmd) = rx.recv() {
        // hlaska o poslednej akcii pouzivatela prezije aj necinnostne zmeny (nastavenia, schranka, kopirovanie)
        let keep_msg = matches!(
            cmd,
            Cmd::SetCfg(_)
                | Cmd::Refresh
                | Cmd::Clip(_)
                | Cmd::Put(_)
                | Cmd::NoteCopy(_)
                | Cmd::ClipRestore(_)
                | Cmd::NoteOpen(_)
                | Cmd::NoteOpenDone
        );
        let mut msg: Option<Msg> = None;
        let mut changed = first;
        first = false;
        match cmd {
            Cmd::Quit => break,
            Cmd::Refresh => {
                msg = apply_clear_requests(&mut w);
                let t = now();
                if w.shelf.enforce(t) > 0 {
                    let _ = w.shelf.save();
                }
                w.clip.enforce(t);
                changed = true;
            }
            Cmd::SetCfg(c) => {
                w.shelf.set_cfg(c.shelf.clone());
                w.clip.set_cfg(c.clipboard.clone());
                w.notes.set_cfg(c.notes.clone());
                w.cfg = *c;
                msg = apply_clear_requests(&mut w);
                let t = now();
                if w.shelf.enforce(t) > 0 {
                    let _ = w.shelf.save();
                }
                w.clip.enforce(t);
                changed = true;
            }
            Cmd::AddPaths { paths, copy } => {
                let copy = copy || w.cfg.shelf.copy_by_default;
                let r = w.shelf.add_paths(&paths, copy, &probe, now());
                if r.added > 0 {
                    let _ = w.shelf.save();
                }
                msg = Some(report_msg(&r));
                changed = true;
            }
            Cmd::ShelfPin(id) => {
                if w.shelf.toggle_pin(&id) {
                    let _ = w.shelf.save();
                    changed = true;
                }
            }
            Cmd::ShelfMove { id, delta } => {
                use crate::shelf::MoveResult;
                match w.shelf.move_item(&id, i32::from(delta)) {
                    MoveResult::Moved { pos, total } => {
                        let _ = w.shelf.save();
                        msg = Some(Msg::Moved { pos, total });
                        changed = true;
                    }
                    MoveResult::Edge { first } => {
                        msg = Some(Msg::MoveEdge { first });
                        changed = true;
                    }
                    MoveResult::Unknown | MoveResult::ReadOnly => {}
                }
            }
            Cmd::ShelfRemove(id) => {
                if w.shelf.remove(&id) {
                    let _ = w.shelf.save();
                    crate::store::purge_backup(&w.dir.join("shelf").join("index.json"));
                    changed = true;
                }
            }
            Cmd::ShelfClear { keep_pinned } => {
                let n = w.shelf.clear(keep_pinned);
                let _ = w.shelf.save();
                crate::store::purge_backup(&w.dir.join("shelf").join("index.json"));
                msg = Some(Msg::Cleared(n));
                changed = true;
            }
            Cmd::Clip(snap) => {
                // zmena sa publikuje len ked sa nieco ulozilo (iinak by kazda schranka zbytocne budila UI)
                changed = matches!(
                    w.clip.capture(&snap, now()),
                    clipboard::Outcome::Stored(_) | clipboard::Outcome::Duplicate(_)
                );
            }
            Cmd::ClipPin(id) => changed = w.clip.toggle_pin(&id),
            Cmd::ClipDelete(id) => changed = w.clip.delete(&id),
            Cmd::ClipClear { keep_pinned } => {
                msg = Some(Msg::Cleared(w.clip.clear(keep_pinned)));
                changed = true;
            }
            Cmd::ClipRestore(id) => {
                if let Some(p) = w.clip.payload(&id) {
                    let _ = writer(p);
                }
            }
            Cmd::Put(p) => {
                let _ = writer(p);
            }
            Cmd::NoteCopy(id) => {
                if let Some(n) = w.notes.get(&id) {
                    let _ = writer(clipboard::Payload::Text(n.text.clone()));
                }
            }
            Cmd::NoteOpenDone => {
                changed = w.opened.take().is_some();
            }
            Cmd::NoteOpen(id) => {
                w.opened = w.notes.get(&id).map(|n| (n.id.clone(), n.text.clone()));
                changed = w.opened.is_some();
            }
            Cmd::NoteCreate(t) => {
                msg = w.notes.create(&t, now()).err().map(Msg::NoteError);
                changed = true;
            }
            Cmd::NoteUpdate { id, text } => {
                msg = w.notes.update(&id, &text, now()).err().map(Msg::NoteError);
                changed = true;
            }
            Cmd::NotePin(id) => {
                msg = w.notes.toggle_pin(&id).err().map(Msg::NoteError);
                changed = true;
            }
            Cmd::NoteDelete(id) => {
                msg = w.notes.delete(&id).err().map(Msg::NoteError);
                changed = true;
            }
            Cmd::ClearMsg => {
                last_msg = None;
                changed = true;
            }
        }
        if keep_msg && msg.is_none() {
            msg = last_msg.clone();
        } else {
            last_msg = msg.clone();
        }
        if changed {
            let s = Arc::new(w.view(msg, &probe));
            if let Ok(mut g) = out.lock() {
                *g = s;
            }
            notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::temp_dir;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    struct Rig {
        svc: Service,
        dir: PathBuf,
        notified: Arc<AtomicU32>,
        written: Arc<Mutex<Vec<clipboard::Payload>>>,
    }

    fn rig(name: &str, tweak: impl FnOnce(&mut Tools)) -> Rig {
        let dir = temp_dir(name);
        let mut cfg = transom_config::Config::default().tools;
        tweak(&mut cfg);
        let notified = Arc::new(AtomicU32::new(0));
        let n2 = Arc::clone(&notified);
        let written = Arc::new(Mutex::new(Vec::new()));
        let w2 = Arc::clone(&written);
        let svc = Service::start(
            dir.clone(),
            cfg,
            Arc::new(move || {
                n2.fetch_add(1, Ordering::SeqCst);
            }),
            Arc::new(move |p| {
                w2.lock().unwrap().push(p);
                true
            }),
        )
        .unwrap();
        Rig {
            svc,
            dir,
            notified,
            written,
        }
    }

    fn wait(mut c: impl FnMut() -> bool) -> bool {
        for _ in 0..600 {
            if c() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// Sonda, ktora na cestach s "dead" visi 5 s (mrtvy server); ostatne cesty existuju.
    struct Dead;

    impl FileProbe for Dead {
        fn probe(&self, p: &std::path::Path) -> crate::shelf::Probe {
            if p.to_string_lossy().contains("dead") {
                std::thread::sleep(Duration::from_secs(5));
            }
            crate::shelf::Probe {
                exists: true,
                size: 3,
                ..Default::default()
            }
        }
        fn open(&self, p: &std::path::Path) -> std::io::Result<std::fs::File> {
            if p.to_string_lossy().contains("dead") {
                std::thread::sleep(Duration::from_secs(5));
            }
            Err(std::io::Error::other("test"))
        }
    }

    /// Mrtvy server (cesta s pismenom disku aj UNC) nesmie zdrzat ani prvu snimku, ani nesuvisiace prikazy.
    #[test]
    fn an_unreachable_shelf_path_never_delays_other_tools_and_is_listed_as_unchecked() {
        let dir = temp_dir("svc-dead");
        let lane = LaneProbe::with_deadlines(
            Arc::new(Dead),
            Duration::from_millis(80),
            Duration::from_millis(200),
        );
        let svc = Service::start_with_lane(
            dir.clone(),
            transom_config::Config::default().tools,
            Arc::new(|| {}),
            Arc::new(|_| true),
            lane,
        )
        .unwrap();
        svc.send(Cmd::AddPaths {
            paths: vec![
                r"Z:\dead\a.txt".into(),
                r"\\dead\share\b.txt".into(),
                r"Y:\dead\c.txt".into(),
            ],
            copy: false,
        });
        let t = std::time::Instant::now();
        svc.send(Cmd::NoteCreate("poznamka".into()));
        assert!(
            wait(|| svc.snapshot().notes.len() == 1),
            "poznamka sa nevytvorila"
        );
        assert!(
            t.elapsed() < Duration::from_millis(1500),
            "nesuvisiaci prikaz cakal {:?} na mrtvy server",
            t.elapsed()
        );
        let s = svc.snapshot();
        assert_eq!(s.shelf.len(), 3, "odkazy sa prijali bez overenia");
        assert!(s.shelf.iter().all(|r| r.state == State::Unchecked));
        assert!(
            s.shelf.iter().any(|r| r.remote) && s.shelf.iter().any(|r| !r.remote),
            "UNC je sietova, cesta s pismenom disku bez klasifikatora nie"
        );
        // kopia z mrtveho servera: odmietnuta, nic sa nezapisalo
        svc.send(Cmd::AddPaths {
            paths: vec![r"\\dead\share\copy.bin".into(), r"Q:\dead\copy2.bin".into()],
            copy: true,
        });
        assert!(wait(|| matches!(
            svc.snapshot().msg,
            Some(Msg::Added { rejected: 2, .. })
        )));
        assert_eq!(svc.snapshot().shelf.len(), 3);
        drop(svc);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn moving_a_shelf_item_reports_its_position_stops_at_the_edge_and_survives_a_restart() {
        let r = rig("svc-move", |_| {});
        let src = temp_dir("svc-move-src");
        let mut ps = Vec::new();
        for n in ["a.txt", "b.txt", "c.txt"] {
            let p = src.join(n);
            std::fs::write(&p, b"1").unwrap();
            ps.push(p.to_string_lossy().to_string());
            r.svc.send(Cmd::AddPaths {
                paths: vec![p.to_string_lossy().to_string()],
                copy: false,
            });
            let want = ps.len();
            assert!(wait(|| r.svc.snapshot().shelf.len() == want));
            std::thread::sleep(Duration::from_millis(1100)); // rozne casy pridania
        }
        let names = |s: &Snapshot| s.shelf.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&r.svc.snapshot()), ["c.txt", "b.txt", "a.txt"]);
        let a = r.svc.snapshot().shelf[2].id.clone();
        r.svc.send(Cmd::ShelfMove {
            id: a.clone(),
            delta: -1,
        });
        assert!(wait(
            || r.svc.snapshot().msg == Some(Msg::Moved { pos: 2, total: 3 })
        ));
        assert_eq!(names(&r.svc.snapshot()), ["c.txt", "a.txt", "b.txt"]);
        r.svc.send(Cmd::ShelfMove {
            id: a.clone(),
            delta: -1,
        });
        assert!(wait(
            || r.svc.snapshot().msg == Some(Msg::Moved { pos: 1, total: 3 })
        ));
        // na okraji sa nic nezmeni, ale pouzivatel dostane hlasku (citacka obrazovky)
        let gen = r.svc.snapshot().generation;
        r.svc.send(Cmd::ShelfMove {
            id: a.clone(),
            delta: -1,
        });
        assert!(wait(|| r.svc.snapshot().generation > gen));
        assert_eq!(r.svc.snapshot().msg, Some(Msg::MoveEdge { first: true }));
        assert_eq!(names(&r.svc.snapshot()), ["a.txt", "c.txt", "b.txt"]);
        // neznama polozka nic nezmeni
        r.svc.send(Cmd::ShelfMove {
            id: "x".into(),
            delta: 1,
        });
        // restart: poradie ostalo; povodne subory nedotknute
        let dir = r.dir.clone();
        drop(r);
        std::thread::sleep(Duration::from_millis(50));
        let again = Service::start(
            dir.clone(),
            transom_config::Config::default().tools,
            Arc::new(|| {}),
            Arc::new(|_| true),
        )
        .unwrap();
        assert!(wait(|| again.snapshot().shelf.len() == 3));
        assert_eq!(names(&again.snapshot()), ["a.txt", "c.txt", "b.txt"]);
        for p in &ps {
            assert_eq!(std::fs::read(p).unwrap(), b"1");
        }
        drop(again);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(src);
    }

    #[test]
    fn a_multi_file_drop_is_added_off_the_ui_thread_and_survives_a_restart() {
        let r = rig("svc-drop", |_| {});
        let src = temp_dir("svc-src");
        let a = src.join("a.txt");
        let b = src.join("b.txt");
        std::fs::write(&a, b"1").unwrap();
        std::fs::write(&b, b"22").unwrap();
        let paths = vec![
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
            a.to_string_lossy().to_string(),
        ];
        r.svc.send(Cmd::AddPaths { paths, copy: false });
        assert!(wait(|| r.svc.snapshot().shelf.len() == 2));
        let s = r.svc.snapshot();
        assert_eq!(
            s.msg,
            Some(Msg::Added {
                added: 2,
                duplicates: 1,
                rejected: 0,
                truncated: false
            })
        );
        assert!(s
            .shelf
            .iter()
            .all(|x| !x.copy && x.state == State::Available));
        assert!(r.notified.load(Ordering::SeqCst) >= 1);
        // restart: polozky ostali
        let dir = r.dir.clone();
        drop(r);
        std::thread::sleep(Duration::from_millis(50));
        let again = Service::start(
            dir.clone(),
            transom_config::Config::default().tools,
            Arc::new(|| {}),
            Arc::new(|_| true),
        )
        .unwrap();
        assert!(wait(|| again.snapshot().shelf.len() == 2));
        // povodne subory nedotknute
        assert_eq!(std::fs::read(&a).unwrap(), b"1");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(src);
    }

    #[test]
    fn a_stored_copy_survives_the_original_being_deleted_and_removal_keeps_originals() {
        let r = rig("svc-copy", |_| {});
        let src = temp_dir("svc-copy-src");
        let a = src.join("doc.txt");
        std::fs::write(&a, b"data").unwrap();
        r.svc.send(Cmd::AddPaths {
            paths: vec![a.to_string_lossy().to_string()],
            copy: true,
        });
        assert!(wait(|| r.svc.snapshot().shelf.len() == 1));
        let row = r.svc.snapshot().shelf[0].clone();
        assert!(row.copy);
        std::fs::remove_file(&a).unwrap();
        r.svc.send(Cmd::Refresh);
        assert!(wait(|| r.svc.snapshot().generation >= 3));
        assert_eq!(
            r.svc.snapshot().shelf[0].state,
            State::Available,
            "kopia je dostupna aj bez originalu"
        );
        assert_eq!(std::fs::read(&row.path).unwrap(), b"data");
        r.svc.send(Cmd::ShelfRemove(row.id.clone()));
        assert!(wait(|| r.svc.snapshot().shelf.is_empty()));
        assert!(!std::path::Path::new(&row.path).exists());
        let _ = std::fs::remove_dir_all(r.dir.clone());
        let _ = std::fs::remove_dir_all(src);
    }

    #[test]
    fn clipboard_history_is_opt_in_and_restore_goes_through_the_writer() {
        let r = rig("svc-clip", |_| {});
        let snap = |t: &str| clipboard::Snapshot {
            text: Some(t.into()),
            ..Default::default()
        };
        r.svc.send(Cmd::Clip(Box::new(snap("tajne"))));
        r.svc.send(Cmd::Refresh);
        assert!(wait(|| r.svc.snapshot().generation >= 2));
        assert!(
            r.svc.snapshot().clipboard.is_empty(),
            "vypnuta schranka nezachytila nic"
        );
        assert!(!r.dir.join("clipboard").join("blobs").exists());
        // zapnutie vyslovne cez nastavenia
        let mut cfg = transom_config::Config::default().tools;
        cfg.clipboard.enabled = true;
        r.svc.send(Cmd::SetCfg(Box::new(cfg)));
        r.svc.send(Cmd::Clip(Box::new(snap("ahoj"))));
        assert!(wait(|| r.svc.snapshot().clipboard.len() == 1));
        let id = r.svc.snapshot().clipboard[0].id.clone();
        r.svc.send(Cmd::ClipRestore(id.clone()));
        assert!(wait(|| !r.written.lock().unwrap().is_empty()));
        assert_eq!(
            r.written.lock().unwrap()[0],
            clipboard::Payload::Text("ahoj".into())
        );
        r.svc.send(Cmd::ClipClear { keep_pinned: false });
        assert!(wait(|| r.svc.snapshot().clipboard.is_empty()));
        assert_eq!(r.svc.snapshot().msg, Some(Msg::Cleared(1)));
        let _ = std::fs::remove_dir_all(r.dir.clone());
    }

    #[test]
    fn notes_commands_report_errors_as_values() {
        let r = rig("svc-notes", |c| {
            c.notes.max_notes = 1;
        });
        r.svc.send(Cmd::NoteCreate("prva".into()));
        assert!(wait(|| r.svc.snapshot().notes.len() == 1));
        r.svc.send(Cmd::NoteCreate("druha".into()));
        assert!(wait(
            || r.svc.snapshot().msg == Some(Msg::NoteError(NoteError::Full))
        ));
        assert_eq!(r.svc.snapshot().notes.len(), 1, "nic sa nezmazalo");
        let id = r.svc.snapshot().notes[0].id.clone();
        r.svc.send(Cmd::NoteUpdate {
            id: id.clone(),
            text: "prva (upravena)".into(),
        });
        assert!(wait(|| r.svc.snapshot().notes[0].title == "prva (upravena)"));
        r.svc.send(Cmd::NoteDelete(id));
        assert!(wait(|| r.svc.snapshot().notes.is_empty()));
        let _ = std::fs::remove_dir_all(r.dir.clone());
    }

    #[test]
    fn opening_a_note_for_editing_delivers_its_text_once_and_only_in_that_snapshot() {
        let r = rig("svc-open", |_| {});
        r.svc.send(Cmd::NoteCreate(
            "Nakup
mlieko"
                .into(),
        ));
        assert!(wait(|| r.svc.snapshot().notes.len() == 1));
        assert!(
            r.svc.snapshot().opened.is_none(),
            "bezna snimka nenesie obsah poznamok"
        );
        let id = r.svc.snapshot().notes[0].id.clone();
        r.svc.send(Cmd::NoteOpen(id.clone()));
        assert!(wait(|| r.svc.snapshot().opened.is_some()));
        assert_eq!(
            r.svc.snapshot().opened,
            Some((
                id,
                "Nakup
mlieko"
                    .to_string()
            ))
        );
        r.svc.send(Cmd::NoteOpenDone);
        r.svc.send(Cmd::Refresh);
        assert!(
            wait(|| r.svc.snapshot().opened.is_none()),
            "dalsia snimka uz obsah nenesie"
        );
        let _ = std::fs::remove_dir_all(r.dir.clone());
    }

    #[test]
    fn a_clear_marker_from_settings_empties_one_store_including_pinned_and_is_consumed() {
        let r = rig("svc-marker", |_| {});
        r.svc.send(Cmd::NoteCreate("a".into()));
        r.svc.send(Cmd::NoteCreate("b".into()));
        assert!(wait(|| r.svc.snapshot().notes.len() == 2));
        let id = r.svc.snapshot().notes[0].id.clone();
        r.svc.send(Cmd::NotePin(id));
        crate::store::request_clear(&r.dir, "notes").unwrap();
        assert!(
            crate::store::request_clear(&r.dir, "../x").is_err(),
            "cesta v nazve sa odmietne"
        );
        r.svc.send(Cmd::SetCfg(Box::new(
            transom_config::Config::default().tools,
        )));
        assert!(wait(|| r.svc.snapshot().notes.is_empty()));
        assert!(
            !r.dir.join("clear-notes.req").exists(),
            "znacka sa spotrebovala"
        );
        let _ = std::fs::remove_dir_all(r.dir.clone());
    }

    #[test]
    fn a_newer_storage_is_reported_and_never_overwritten() {
        let dir = temp_dir("svc-newer");
        std::fs::create_dir_all(dir.join("shelf")).unwrap();
        std::fs::write(
            dir.join("shelf").join("index.json"),
            br#"{"format":"transom-tools-shelf","version":42,"items":[]}"#,
        )
        .unwrap();
        let svc = Service::start(
            dir.clone(),
            transom_config::Config::default().tools,
            Arc::new(|| {}),
            Arc::new(|_| true),
        )
        .unwrap();
        assert!(wait(|| svc.snapshot().generation >= 1));
        assert!(svc.snapshot().unusable);
        svc.send(Cmd::AddPaths {
            paths: vec![r"C:\Windows\notepad.exe".into()],
            copy: false,
        });
        assert!(wait(
            || svc.snapshot().msg == Some(Msg::Rejected(RejectReason::ReadOnly))
        ));
        assert!(
            std::fs::read_to_string(dir.join("shelf").join("index.json"))
                .unwrap()
                .contains("42")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
