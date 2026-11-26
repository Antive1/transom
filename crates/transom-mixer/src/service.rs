//! Sluzba mixera: vlastne vlakno (COM MTA riesi backend), notifikacie Windows len vlozia udalost do fronty, serie sa
//! zlucia a relacie sa precitaju raz. UI vlakno nikdy nevola COM: cita snimku a posiela prikazy.
//!
//! Hlasitost a stlmenie sa zapisuju len z prikazu UI (nikdy ako reakcia na udalost), takze nevznikne spatna vazba.
//! Snimka sa po zapise hned upravi (posuvnik neskace) a potvrdi ju az dalsie citanie.
use crate::model::{OutDevice, RawSession, Route, Snapshot};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Co backend precital.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub sessions: Vec<RawSession>,
    pub devices: Vec<OutDevice>,
    /// Trasa pre PID (0 = systemove zvuky); chybajuci PID = predvoleny vystup.
    pub routes: HashMap<u32, Route>,
    pub routing: bool,
}

/// Zvukovy system (vytvara sa a pouziva len na vlakne sluzby).
pub trait Backend {
    fn read(&mut self) -> Reading;
    /// Hlasitost (0.0..=1.0) vsetkych relacii procesov `pids` (pri systemovych zvukoch `system`).
    fn set_volume(&mut self, pids: &[u32], system: bool, v: f32);
    fn set_mute(&mut self, pids: &[u32], system: bool, m: bool);
    /// Vystup pre procesy (`None` = predvoleny). Vrati, ci to Windows prijal.
    fn set_route(&mut self, pids: &[u32], device: Option<&str>) -> bool;
    fn shutdown(&mut self);
}

enum Msg {
    Dirty,
    Volume(u64, u8),
    Mute(u64, bool),
    Route(u64, Option<String>),
    Quit,
}

type Factory = Box<dyn FnOnce(Sender<()>) -> Box<dyn Backend> + Send>;

pub struct Service {
    tx: Sender<Msg>,
    snap: Arc<Mutex<Arc<Snapshot>>>,
}

impl Service {
    /// Spusti sluzbu. `notify` sa zavola z vlakna sluzby po kazdej zmene snimky; `system_name` je meno systemovych zvukov.
    pub fn start<F>(
        factory: F,
        notify: Arc<dyn Fn() + Send + Sync>,
        system_name: String,
        coalesce: Duration,
    ) -> Result<Service, String>
    where
        F: FnOnce(Sender<()>) -> Box<dyn Backend> + Send + 'static,
    {
        let (tx, rx) = channel::<Msg>();
        let (etx, erx) = channel::<()>();
        let fwd = tx.clone();
        std::thread::Builder::new()
            .name("transom-mixer-fwd".into())
            .spawn(move || {
                for _ in erx {
                    if fwd.send(Msg::Dirty).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let snap = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let s2 = Arc::clone(&snap);
        let f: Factory = Box::new(factory);
        std::thread::Builder::new()
            .name("transom-mixer".into())
            .spawn(move || worker(rx, etx, s2, notify, f, system_name, coalesce))
            .map_err(|e| e.to_string())?;
        let _ = tx.send(Msg::Dirty);
        Ok(Service { tx, snap })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snap.lock().map(|g| Arc::clone(&g)).unwrap_or_default()
    }

    pub fn set_volume(&self, key: u64, percent: u32) {
        let _ = self.tx.send(Msg::Volume(key, percent.min(100) as u8));
    }

    pub fn set_mute(&self, key: u64, muted: bool) {
        let _ = self.tx.send(Msg::Mute(key, muted));
    }

    /// Vystup aplikacie: `None` = predvoleny.
    pub fn set_route(&self, key: u64, device: Option<String>) {
        let _ = self.tx.send(Msg::Route(key, device));
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

fn worker(
    rx: Receiver<Msg>,
    etx: Sender<()>,
    snap: Arc<Mutex<Arc<Snapshot>>>,
    notify: Arc<dyn Fn() + Send + Sync>,
    factory: Factory,
    system_name: String,
    coalesce: Duration,
) {
    let mut be = factory(etx);
    let mut gen = 0u64;
    let mut failed: Option<u64> = None;
    let publish = |s: Snapshot| {
        if let Ok(mut g) = snap.lock() {
            *g = Arc::new(s);
        }
        notify();
    };
    let current = || snap.lock().map(|g| (**g).clone()).unwrap_or_default();
    let mut refresh = |be: &mut Box<dyn Backend>, failed: Option<u64>| {
        let r = be.read();
        gen += 1;
        publish(Snapshot {
            ready: true,
            apps: crate::model::group(&r.sessions, &r.routes, &system_name),
            devices: r.devices,
            routing: r.routing,
            route_failed: failed,
            generation: gen,
        });
    };
    while let Ok(m) = rx.recv() {
        let mut dirty = false;
        let mut quit = false;
        let mut handle =
            |m: Msg, be: &mut Box<dyn Backend>, dirty: &mut bool, quit: &mut bool| match m {
                Msg::Dirty => *dirty = true,
                Msg::Quit => *quit = true,
                Msg::Volume(k, v) => {
                    let mut s = current();
                    if let Some(a) = s.apps.iter_mut().find(|a| a.key == k) {
                        be.set_volume(&a.pids, a.system, f32::from(v) / 100.0);
                        a.volume = v;
                        publish(s);
                    }
                }
                Msg::Mute(k, m) => {
                    let mut s = current();
                    if let Some(a) = s.apps.iter_mut().find(|a| a.key == k) {
                        be.set_mute(&a.pids, a.system, m);
                        a.muted = m;
                        publish(s);
                    }
                }
                Msg::Route(k, d) => {
                    let s = current();
                    if let Some(a) = s.app(k) {
                        let pids = if a.system { vec![0] } else { a.pids.clone() };
                        failed = (!be.set_route(&pids, d.as_deref())).then_some(k);
                        *dirty = true;
                    }
                }
            };
        handle(m, &mut be, &mut dirty, &mut quit);
        // zlucenie serie: cakat na tichy usek a potom citat raz
        if dirty && !quit {
            loop {
                match rx.recv_timeout(coalesce) {
                    Ok(m) => handle(m, &mut be, &mut dirty, &mut quit),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        quit = true;
                        break;
                    }
                }
                if quit {
                    break;
                }
            }
        }
        if quit {
            break;
        }
        if dirty {
            refresh(&mut be, failed);
        }
    }
    be.shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    #[derive(Default)]
    struct World {
        sessions: Vec<RawSession>,
        routes: HashMap<u32, Route>,
        writes: Vec<String>,
        reject_route: bool,
    }

    struct Fake(Arc<Mutex<World>>);

    impl Backend for Fake {
        fn read(&mut self) -> Reading {
            let w = self.0.lock().unwrap();
            Reading {
                sessions: w.sessions.clone(),
                devices: vec![OutDevice {
                    id: "a".into(),
                    name: "A".into(),
                    is_default: true,
                }],
                routes: w.routes.clone(),
                routing: true,
            }
        }
        fn set_volume(&mut self, pids: &[u32], _s: bool, v: f32) {
            let mut w = self.0.lock().unwrap();
            w.writes.push(format!("vol {pids:?} {v:.2}"));
            for s in w.sessions.iter_mut().filter(|s| pids.contains(&s.pid)) {
                s.volume = v;
            }
        }
        fn set_mute(&mut self, pids: &[u32], _s: bool, m: bool) {
            self.0
                .lock()
                .unwrap()
                .writes
                .push(format!("mute {pids:?} {m}"));
        }
        fn set_route(&mut self, pids: &[u32], d: Option<&str>) -> bool {
            let mut w = self.0.lock().unwrap();
            w.writes.push(format!("route {pids:?} {d:?}"));
            if w.reject_route {
                return false;
            }
            for p in pids {
                match d {
                    Some(d) => w.routes.insert(*p, Route::Device(d.into())),
                    None => w.routes.remove(p),
                };
            }
            true
        }
        fn shutdown(&mut self) {
            self.0.lock().unwrap().writes.push("shutdown".into());
        }
    }

    fn wait(cond: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(Instant::now() < end, "timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn commands_reach_every_session_of_the_app_and_routing_is_confirmed_by_a_new_reading() {
        let world = Arc::new(Mutex::new(World::default()));
        {
            let mut w = world.lock().unwrap();
            for pid in [7, 8] {
                w.sessions.push(RawSession {
                    pid,
                    system: false,
                    exe: Some(r"C:\a\b.exe".into()),
                    description: None,
                    display: String::new(),
                    volume: 1.0,
                    muted: false,
                    active: true,
                });
            }
        }
        let hits = Arc::new(AtomicUsize::new(0));
        let h2 = Arc::clone(&hits);
        let w2 = Arc::clone(&world);
        let svc = Service::start(
            move |_| Box::new(Fake(w2)),
            Arc::new(move || {
                h2.fetch_add(1, Ordering::SeqCst);
            }),
            "Sys".into(),
            Duration::from_millis(5),
        )
        .unwrap();
        wait(|| svc.snapshot().ready);
        let s = svc.snapshot();
        assert_eq!(s.apps.len(), 1);
        // predvolena hlasitost aplikacie sa nikdy nezapisuje sama
        assert!(world.lock().unwrap().writes.is_empty());
        let key = s.apps[0].key;
        svc.set_volume(key, 150);
        wait(|| svc.snapshot().apps[0].volume == 100 && !world.lock().unwrap().writes.is_empty());
        svc.set_volume(key, 35);
        wait(|| svc.snapshot().apps[0].volume == 35);
        svc.set_route(key, Some("b".into()));
        wait(|| svc.snapshot().apps[0].route == Route::Device("b".into()));
        assert_eq!(svc.snapshot().route_failed, None);
        world.lock().unwrap().reject_route = true;
        svc.set_route(key, None);
        wait(|| svc.snapshot().route_failed == Some(key));
        // odmietnuta zmena: trasa ostava, ako ju hlasi Windows
        assert_eq!(svc.snapshot().apps[0].route, Route::Device("b".into()));
        drop(svc);
        wait(|| world.lock().unwrap().writes.last().map(String::as_str) == Some("shutdown"));
        let writes = world.lock().unwrap().writes.clone();
        assert_eq!(writes[0], "vol [7, 8] 1.00");
        assert!(writes.contains(&"vol [7, 8] 0.35".to_string()));
        assert!(writes.contains(&"route [7, 8] Some(\"b\")".to_string()));
        assert!(hits.load(Ordering::SeqCst) >= 3);
    }
}
