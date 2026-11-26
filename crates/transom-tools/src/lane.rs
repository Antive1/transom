//! Izolacia pristupu k cestam pouzivatela. Cesta z Police moze viest na sietovy disk, UNC zdielanie alebo WebDAV a nedostupny
//! server dokaze zablokovat volanie suboroveho systemu na desiatky sekund. Spolocne pracovne vlakno nastrojov (schranka,
//! poznamky, snimky...) preto **nikdy** nevola `symlink_metadata` ani `File::open` na ceste od pouzivatela priamo:
//!
//! * kazda sonda ide na male pevne vlakna ("lane") a pracovne vlakno caka najviac `DEADLINE`; po uplynuti sa cesta ukaze ako
//!   neskontrolovana (`Probe::unchecked`), nie ako chybajuca, a neskory vysledok sa len ulozi na dalsie pouzitie;
//! * ak su vsetky vlakna uviaznute, sondy sa vobec nezadavaju (nikto nic necaka) a front je ohraniceny;
//! * cesty, ktore vyzeraju sietove (UNC, `\\?\UNC`, namapovany sietovy disk podla `set_remote_classifier`), sa **automaticky
//!   nesonduju vobec** a kopia z nich sa neurobi (`RejectReason::Network`); referencia sa prijme;
//! * otvorenie suboru na kopirovanie ma vlastny limit.
//!
//! Klasifikacia je len optimalizacia (`subst`, spojenia a pod. ju oklamu): bezpecnost zaklada casovy limit, nie rozpoznanie.
use crate::names::path_key;
use crate::shelf::{FileProbe, Probe};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Najdlhsie cakanie pracovneho vlakna na jednu sondu.
pub const DEADLINE: Duration = Duration::from_millis(250);
/// Najdlhsie cakanie na otvorenie suboru pre kopiu.
pub const OPEN_DEADLINE: Duration = Duration::from_secs(3);
const WORKERS: usize = 2;
const QUEUE: usize = 64;
const LATE_MAX: usize = 512;

static CLASSIFIER: OnceLock<fn(&str) -> bool> = OnceLock::new();

/// Nastavi rozpoznavanie namapovanych sietovych diskov (`C:\` styl; UNC sa rozpoznava textovo vzdy). Zavola aplikacia
/// (`GetDriveTypeW`); tento crate nema zavislost na Windows API.
pub fn set_remote_classifier(f: fn(&str) -> bool) {
    let _ = CLASSIFIER.set(f);
}

/// Vyzera cesta ako sietova (UNC, `\\?\UNC\`, namapovany sietovy disk)? Nic sa neotvara.
pub fn is_remote_path(p: &str) -> bool {
    let n = p.replace('/', "\\");
    let l = n.to_ascii_lowercase();
    if l.starts_with(r"\\?\unc\") {
        return true;
    }
    if let Some(rest) = n.strip_prefix(r"\\?\") {
        return CLASSIFIER.get().is_some_and(|f| f(rest));
    }
    if l.starts_with(r"\\") {
        return true;
    }
    CLASSIFIER.get().is_some_and(|f| f(p))
}

type Job = (PathBuf, SyncSender<Probe>);

/// Sondy a otvaranie suborov cez fixne vlakna s casovym limitom (viz modul).
pub struct LaneProbe {
    inner: Arc<dyn FileProbe + Send + Sync>,
    tx: SyncSender<Job>,
    /// Pre kazde vlakno cas (ms od `base` + 1), odkedy bezi aktualna sonda; 0 = necinne.
    busy: Arc<Vec<AtomicU64>>,
    base: Instant,
    /// Neskore vysledky sond, ktore uz nikto nevyckava (podla `path_key`).
    late: Arc<Mutex<HashMap<String, Probe>>>,
    deadline: Duration,
    open_deadline: Duration,
}

impl LaneProbe {
    pub fn new(inner: Arc<dyn FileProbe + Send + Sync>) -> LaneProbe {
        Self::with_deadlines(inner, DEADLINE, OPEN_DEADLINE)
    }

    pub fn with_deadlines(
        inner: Arc<dyn FileProbe + Send + Sync>,
        deadline: Duration,
        open_deadline: Duration,
    ) -> LaneProbe {
        let (tx, rx): (SyncSender<Job>, Receiver<Job>) = sync_channel(QUEUE);
        let rx = Arc::new(Mutex::new(rx));
        let base = Instant::now();
        let busy: Arc<Vec<AtomicU64>> = Arc::new((0..WORKERS).map(|_| AtomicU64::new(0)).collect());
        let late: Arc<Mutex<HashMap<String, Probe>>> = Arc::new(Mutex::new(HashMap::new()));
        for i in 0..WORKERS {
            let (rx, inner, busy, late) = (
                Arc::clone(&rx),
                Arc::clone(&inner),
                Arc::clone(&busy),
                Arc::clone(&late),
            );
            let _ = std::thread::Builder::new()
                .name(format!("transom-probe-{i}"))
                .spawn(move || loop {
                    let job = {
                        let g = rx.lock().unwrap_or_else(|e| e.into_inner());
                        g.recv()
                    };
                    let Ok((path, reply)) = job else { break };
                    busy[i].store(base.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
                    let p = inner.probe(&path);
                    busy[i].store(0, Ordering::Relaxed);
                    if reply.send(p).is_err() {
                        // pracovne vlakno uz necaka: vysledok sa ulozi pre dalsie pouzitie (ohranicene)
                        let mut m = late.lock().unwrap_or_else(|e| e.into_inner());
                        if m.len() >= LATE_MAX {
                            m.clear();
                        }
                        m.insert(path_key(&path.to_string_lossy()), p);
                    }
                });
        }
        LaneProbe {
            inner,
            tx,
            busy,
            base,
            late,
            deadline,
            open_deadline,
        }
    }

    /// Su vsetky vlakna dlhsie nez limit v jednom volani (uviaznute)? Vtedy sa nova sonda ani nezada.
    fn degraded(&self) -> bool {
        let now = self.base.elapsed().as_millis() as u64 + 1;
        let limit = self.deadline.as_millis() as u64;
        self.busy.iter().all(|b| {
            let s = b.load(Ordering::Relaxed);
            s != 0 && now.saturating_sub(s) > limit
        })
    }
}

fn unchecked() -> Probe {
    Probe {
        exists: true,
        unchecked: true,
        ..Probe::default()
    }
}

impl FileProbe for LaneProbe {
    fn probe(&self, path: &Path) -> Probe {
        let text = path.to_string_lossy();
        if is_remote_path(&text) {
            return unchecked();
        }
        let key = path_key(&text);
        if let Some(p) = self
            .late
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key)
        {
            return p;
        }
        if self.degraded() {
            return unchecked();
        }
        let (rtx, rrx) = sync_channel(1);
        if self.tx.try_send((path.to_path_buf(), rtx)).is_err() {
            return unchecked();
        }
        rrx.recv_timeout(self.deadline)
            .unwrap_or_else(|_| unchecked())
    }

    fn remote(&self, path: &Path) -> bool {
        is_remote_path(&path.to_string_lossy())
    }

    fn open(&self, path: &Path) -> std::io::Result<std::fs::File> {
        if is_remote_path(&path.to_string_lossy()) {
            return Err(std::io::Error::other("sietova cesta"));
        }
        let (rtx, rrx) = sync_channel(1);
        let (inner, p) = (Arc::clone(&self.inner), path.to_path_buf());
        std::thread::Builder::new()
            .name("transom-open".into())
            .spawn(move || {
                // pri neskorom vysledku sa subor po zlyhani odoslania zatvori (uvolni sa pri zahodeni)
                let _ = rtx.send(inner.open(&p));
            })
            .map_err(|_| std::io::Error::other("vlakno"))?;
        match rrx.recv_timeout(self.open_deadline) {
            Ok(r) => r,
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "otvorenie suboru trva prilis dlho",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Sonda, ktora na cestach s "hang" visi dlho (simulacia mrtveho servera); ostatne vrati hned.
    struct Hanging {
        calls: AtomicUsize,
        hang: Duration,
    }

    impl FileProbe for Hanging {
        fn probe(&self, p: &Path) -> Probe {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if p.to_string_lossy().contains("hang") {
                std::thread::sleep(self.hang);
            }
            Probe {
                exists: true,
                size: 7,
                ..Probe::default()
            }
        }
        fn open(&self, p: &Path) -> std::io::Result<std::fs::File> {
            if p.to_string_lossy().contains("hang") {
                std::thread::sleep(self.hang);
            }
            Err(std::io::Error::other("test"))
        }
    }

    fn lane(hang_ms: u64) -> (LaneProbe, Arc<Hanging>) {
        let h = Arc::new(Hanging {
            calls: AtomicUsize::new(0),
            hang: Duration::from_millis(hang_ms),
        });
        (
            LaneProbe::with_deadlines(
                h.clone(),
                Duration::from_millis(60),
                Duration::from_millis(120),
            ),
            h,
        )
    }

    #[test]
    fn network_looking_paths_are_never_probed_and_are_reported_as_unchecked() {
        let (l, h) = lane(0);
        for p in [
            r"\\srv\share\x.txt",
            r"//srv/share/x.txt",
            r"\\?\UNC\srv\share\x.txt",
            r"\\srv@SSL\DavWWWRoot\x",
        ] {
            assert!(l.remote(Path::new(p)), "{p}");
            let pr = l.probe(Path::new(p));
            assert!(pr.unchecked && pr.exists, "{p}");
        }
        assert_eq!(
            h.calls.load(Ordering::SeqCst),
            0,
            "ziadna sonda sa nezadala"
        );
        assert!(l.open(Path::new(r"\\srv\share\x.txt")).is_err());
        assert_eq!(h.calls.load(Ordering::SeqCst), 0);
        // lokalna cesta sa sonduje normalne
        let pr = l.probe(Path::new(r"C:\a\b.txt"));
        assert!(!pr.unchecked && pr.size == 7);
        assert!(!l.remote(Path::new(r"C:\a\b.txt")));
    }

    #[test]
    fn a_hung_probe_returns_unchecked_at_the_deadline_and_the_late_answer_is_used_next_time() {
        let (l, _h) = lane(400);
        let t = Instant::now();
        let pr = l.probe(Path::new(r"Z:\hang\one.txt"));
        assert!(
            pr.unchecked,
            "po limite je cesta neskontrolovana, nie chybajuca"
        );
        assert!(
            t.elapsed() < Duration::from_millis(300),
            "caka sa najviac limit, nie cely visiaci pokus: {:?}",
            t.elapsed()
        );
        std::thread::sleep(Duration::from_millis(500));
        let pr = l.probe(Path::new(r"Z:\hang\one.txt"));
        assert!(!pr.unchecked && pr.size == 7, "neskory vysledok sa pouzil");
    }

    #[test]
    fn when_every_lane_thread_is_stuck_new_probes_return_at_once_and_local_paths_do_not_pile_up() {
        let (l, h) = lane(1500);
        // zablokuje obe vlakna
        for i in 0..WORKERS {
            let _ = l.probe(Path::new(&format!(r"Z:\hang\{i}")));
        }
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        for i in 0..2000 {
            let pr = l.probe(Path::new(&format!(r"C:\ok\{i}")));
            assert!(pr.unchecked);
        }
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "2000 sond pri uviaznutych vlaknach trvalo {:?}",
            t.elapsed()
        );
        assert_eq!(
            h.calls.load(Ordering::SeqCst),
            WORKERS,
            "nic sa nezaradilo za uviaznute"
        );
    }

    #[test]
    fn opening_a_file_has_its_own_deadline_and_a_failed_open_is_an_error_not_a_hang() {
        let (l, _h) = lane(600);
        let t = Instant::now();
        let r = l.open(Path::new(r"Z:\hang\big.bin"));
        assert_eq!(r.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_millis(400));
        assert!(l.open(Path::new(r"C:\ok.bin")).is_err());
    }
}
