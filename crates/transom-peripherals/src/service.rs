//! Sluzba batérii periférií: vlastne vlakno, zdroje sa citaju po kolach (pravidelne + na poziadanie, napr. po
//! zasunuti prijimaca). UI vlakno nikdy necaka na HID: cita len snimku (`Arc<Snapshot>`).
use crate::model::{Availability, Reading, Snapshot, Source, Tracker};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Jeden zdroj urovni (Razer, Logitech, ...). Vytvara a pouziva sa len na vlakne sluzby.
pub trait Provider {
    fn source(&self) -> Source;
    /// Jedno kolo citania. Nesmie blokovat dlhsie nez par sekund.
    fn poll(&mut self) -> Vec<Reading>;
}

/// Casovanie sluzby.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Pravidelne kolo.
    pub interval: Duration,
    /// Po poziadavke na obnovu (zasunutie zariadenia) pockat, kym sa ovladac usadi; dalsie poziadavky v tomto
    /// useku sa zlucia do jedneho kola.
    pub settle: Duration,
    /// Zariadenie, ktore nehlasi ziadny zdroj, sa po tomto zahodi.
    pub forget_after: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            interval: Duration::from_secs(60),
            settle: Duration::from_millis(1500),
            forget_after: Duration::from_secs(180),
        }
    }
}

enum Msg {
    Refresh,
    Quit,
}

struct Shared {
    snap: Mutex<Arc<Snapshot>>,
    rounds: Mutex<u64>,
}

/// Uchyt na sluzbu; pri zruseni sa vlakno ukonci (po dokonceni prave beziaceho kola).
pub struct Service {
    tx: Sender<Msg>,
    shared: Arc<Shared>,
}

type Factory = Box<dyn FnOnce() -> Vec<Box<dyn Provider>> + Send>;

impl Service {
    /// `factory` vytvori zdroje na vlakne sluzby; `notify` sa vola z vlakna sluzby po kazdej zmene snimky.
    pub fn start<F>(
        factory: F,
        notify: Arc<dyn Fn() + Send + Sync>,
        tuning: Tuning,
    ) -> Result<Service, String>
    where
        F: FnOnce() -> Vec<Box<dyn Provider>> + Send + 'static,
    {
        let (tx, rx) = channel::<Msg>();
        let shared = Arc::new(Shared {
            snap: Mutex::new(Arc::new(Snapshot::default())),
            rounds: Mutex::new(0),
        });
        let s2 = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("transom-peripherals".into())
            .spawn(move || worker(rx, s2, notify, tuning, Box::new(factory)))
            .map_err(|e| e.to_string())?;
        Ok(Service { tx, shared })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.shared
            .snap
            .lock()
            .map(|g| Arc::clone(&g))
            .unwrap_or_default()
    }

    /// Poziada o nove kolo (po kratkom usadeni; viac poziadaviek sa zlucuje). Neblokuje.
    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    /// Kolko kol uz prebehlo (diagnostika, testy).
    pub fn rounds(&self) -> u64 {
        self.shared.rounds.lock().map(|r| *r).unwrap_or(0)
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

fn worker(
    rx: Receiver<Msg>,
    shared: Arc<Shared>,
    notify: Arc<dyn Fn() + Send + Sync>,
    tuning: Tuning,
    factory: Factory,
) {
    let mut providers = factory();
    let mut tracker = Tracker::new(tuning.forget_after);
    let mut generation = 0u64;
    let mut next = Instant::now();
    loop {
        // cakat na dalsie kolo alebo poziadavku
        let wait = next.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Msg::Refresh) => {
                // usadenie a zlucenie dalsich poziadaviek
                let until = Instant::now() + tuning.settle;
                loop {
                    match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                        Ok(Msg::Refresh) => continue,
                        Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let mut readings = Vec::new();
        for p in providers.iter_mut() {
            readings.extend(p.poll());
        }
        let now = Instant::now();
        tracker.update(readings, now);
        let devices = tracker.devices(now);
        if let Ok(mut r) = shared.rounds.lock() {
            *r += 1;
        }
        let changed = {
            let Ok(mut g) = shared.snap.lock() else {
                return;
            };
            let changed = g.availability != Availability::Ready || g.devices != devices;
            if changed {
                generation += 1;
                *g = Arc::new(Snapshot {
                    generation,
                    availability: Availability::Ready,
                    devices,
                });
            }
            changed
        };
        if changed {
            notify();
        }
        next = Instant::now() + tuning.interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Presence};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        script: Arc<Mutex<Vec<Vec<Reading>>>>,
    }

    impl Provider for Fake {
        fn source(&self) -> Source {
            Source::Razer
        }
        fn poll(&mut self) -> Vec<Reading> {
            let mut s = self.script.lock().unwrap();
            if s.len() > 1 {
                s.remove(0)
            } else {
                s.first().cloned().unwrap_or_default()
            }
        }
    }

    fn mouse(p: u8) -> Reading {
        Reading {
            key: "razer:m".into(),
            name: "Razer M".into(),
            kind: Kind::Mouse,
            source: Source::Razer,
            percent: Some(p),
            charging: false,
            presence: Presence::Live,
        }
    }

    fn wait_for(cond: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(Instant::now() < end, "cas vyprsal");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn first_round_runs_at_start_and_notifies_only_on_change() {
        let script = Arc::new(Mutex::new(vec![
            vec![mouse(50)],
            vec![mouse(50)],
            vec![mouse(49)],
        ]));
        let notified = Arc::new(AtomicUsize::new(0));
        let n2 = Arc::clone(&notified);
        let s2 = Arc::clone(&script);
        let svc = Service::start(
            move || vec![Box::new(Fake { script: s2 }) as Box<dyn Provider>],
            Arc::new(move || {
                n2.fetch_add(1, Ordering::SeqCst);
            }),
            Tuning {
                interval: Duration::from_millis(20),
                settle: Duration::from_millis(1),
                forget_after: Duration::from_secs(60),
            },
        )
        .unwrap();
        wait_for(|| svc.snapshot().devices.first().and_then(|d| d.percent) == Some(49));
        let s = svc.snapshot();
        assert_eq!(s.availability, Availability::Ready);
        // 50 (zmena), 50 (bez zmeny), 49 (zmena)
        assert_eq!(notified.load(Ordering::SeqCst), 2);
        assert!(svc.rounds() >= 3);
    }

    #[test]
    fn refresh_requests_are_coalesced_into_one_round() {
        let script = Arc::new(Mutex::new(vec![vec![mouse(10)]]));
        let s2 = Arc::clone(&script);
        let svc = Service::start(
            move || vec![Box::new(Fake { script: s2 }) as Box<dyn Provider>],
            Arc::new(|| {}),
            Tuning {
                interval: Duration::from_secs(3600),
                settle: Duration::from_millis(150),
                forget_after: Duration::from_secs(60),
            },
        )
        .unwrap();
        wait_for(|| svc.rounds() == 1);
        for _ in 0..5 {
            svc.refresh();
        }
        wait_for(|| svc.rounds() == 2);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(svc.rounds(), 2, "5 poziadaviek = 1 kolo");
    }
}
