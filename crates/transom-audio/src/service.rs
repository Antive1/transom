//! Sluzba systemoveho vystupu zvuku: vlastne vlakno (COM MTA riesi backend), udalosti z Core Audio, bez pollingu.
//!
//! * UI vlakno nikdy nevola COM: cita len snimku (`Arc<Snapshot>`) a posiela prikazy do fronty.
//! * Notifikacie Windows (callbacky na cudzich vlaknach) iba vlozia malu udalost do fronty; vsetko ostatne (citanie
//!   zoznamu, nova registracia na ine zariadenie) robi toto vlakno. Rychle serie udalosti (zasuvanie zariadenia
//!   posiela desiatky sprav, zmena predvoleneho vystupu 3 za sebou) sa zlucuju: po prvej sa pocka kratky tichy usek
//!   (`coalesce`) a zoznam sa precita **raz**. Je to cakanie na dalsiu udalost, nie pravidelne dotazovanie.
//! * Zapis hlasitosti ide len z prikazu UI; udalost z Windows (aj ozvena nasej zmeny) len aktualizuje model, nikdy
//!   nespusti dalsi zapis, takze nevznikne spatna vazba.
use crate::model::{
    display_devices, AudioError, Availability, Controls, RawDevice, Snapshot, Volume,
};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Co backend vrati po obnoveni: zoznam, hlasitost predvoleneho vystupu a to, co vie ovladat.
#[derive(Clone, Debug, PartialEq)]
pub struct Refresh {
    pub devices: Vec<RawDevice>,
    pub volume: Option<Volume>,
    pub controls: Controls,
}

/// Zvukovy system. Vytvara sa a pouziva len na vlakne sluzby (nemusi byt `Send`).
pub trait Backend {
    /// Znova nacita zoznam, naviaze sa na aktualne predvolene zariadenie (nova registracia hlasitosti, ak sa
    /// zmenilo) a precita hlasitost.
    fn refresh(&mut self) -> Result<Refresh, AudioError>;
    fn set_volume(&mut self, percent: u8) -> Result<(), AudioError>;
    fn set_mute(&mut self, muted: bool) -> Result<(), AudioError>;
    /// Nastavi predvoleny vystup (zariadenie `id`). Predvolene nepodporovane.
    fn set_default(&mut self, _id: &str) -> Result<(), AudioError> {
        Err(AudioError::Unsupported)
    }
    /// Zrusi registracie a uvolni rozhrania (pred koncom vlakna; nikdy z callbacku).
    fn shutdown(&mut self);
}

/// Udalost z notifikacii Core Audio (posiela ju backend z callbackov).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Zmena zoznamu, stavu, nazvu alebo predvoleneho vystupu: treba znova nacitat.
    Devices,
    /// Zmena hlasitosti alebo stlmenia predvoleneho vystupu (`own` = zmenu spravili sme my).
    Volume { percent: u8, muted: bool, own: bool },
}

enum Msg {
    Event(Event),
    SetVolume(u8),
    SetMute(bool),
    SetDefault(u64),
    Refresh,
    Quit,
}

/// Casovanie sluzby (v testoch skratene).
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Tichy usek po prvej udalosti zoznamu, nez sa zoznam precita (zlucenie serie udalosti).
    pub coalesce: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            coalesce: Duration::from_millis(80),
        }
    }
}

/// Pocitadla na diagnostiku a testy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Kolkokrat sa zoznam skutocne nacital.
    pub refreshes: u64,
    pub writes: u64,
}

type Factory = Box<dyn FnOnce(Sender<Event>) -> Box<dyn Backend> + Send>;

struct Shared {
    snap: Mutex<Arc<Snapshot>>,
    counters: Mutex<Counters>,
}

/// Uchyt na sluzbu. Pri zruseni (`Drop`) sa vlakno ukonci a backend uvolni registracie (bez cakania na UI vlakne).
pub struct Service {
    tx: Sender<Msg>,
    shared: Arc<Shared>,
}

impl Service {
    /// Spusti sluzbu. `notify` sa zavola (z vlakna sluzby) po kazdej zmene snimky.
    pub fn start<F>(
        factory: F,
        notify: Arc<dyn Fn() + Send + Sync>,
        tuning: Tuning,
    ) -> Result<Service, String>
    where
        F: FnOnce(Sender<Event>) -> Box<dyn Backend> + Send + 'static,
    {
        let (tx, rx) = channel::<Msg>();
        let (etx, erx) = channel::<Event>();
        let shared = Arc::new(Shared {
            snap: Mutex::new(Arc::new(Snapshot::default())),
            counters: Mutex::new(Counters::default()),
        });
        // udalosti z backendu sa prelievaju do hlavnej fronty (jedna fronta = zachovane poradie s prikazmi)
        let fwd_tx = tx.clone();
        std::thread::Builder::new()
            .name("transom-audio-fwd".into())
            .spawn(move || {
                for e in erx {
                    if fwd_tx.send(Msg::Event(e)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let s2 = Arc::clone(&shared);
        let tx2 = tx.clone();
        std::thread::Builder::new()
            .name("transom-audio".into())
            .spawn(move || worker(rx, tx2, etx, s2, notify, tuning, Box::new(factory)))
            .map_err(|e| e.to_string())?;
        // prve nacitanie
        let _ = tx.send(Msg::Refresh);
        Ok(Service { tx, shared })
    }

    /// Aktualna snimka (lacna kopia).
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.shared
            .snap
            .lock()
            .map(|g| Arc::clone(&g))
            .unwrap_or_default()
    }

    /// Nastavi hlasitost predvoleneho vystupu (percento 0..=100, vyssie sa orezie). Neblokuje.
    pub fn set_volume(&self, percent: u32) {
        let _ = self.tx.send(Msg::SetVolume(percent.min(100) as u8));
    }

    /// Stlmi / odstlmi predvoleny vystup. Neblokuje.
    pub fn set_mute(&self, muted: bool) {
        let _ = self.tx.send(Msg::SetMute(muted));
    }

    /// Nastavi predvoleny vystup na zariadenie s klucom `key` (`Device::key`). Neblokuje; potvrdenie pride ako
    /// zmena snimky (notifikacia Windows).
    pub fn set_default(&self, key: u64) {
        let _ = self.tx.send(Msg::SetDefault(key));
    }

    /// Znova nacita stav (napr. pri otvoreni panela: jedina cesta obnovy po strate sluzby bez pollingu).
    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    pub fn counters(&self) -> Counters {
        self.shared.counters.lock().map(|c| *c).unwrap_or_default()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

struct State {
    snap: Snapshot,
    /// ID predvoleneho zariadenia z posledneho nacitania (len na detekciu zmeny; nikdy sa nezobrazuje).
    default_id: Option<String>,
    /// ID zariadeni z posledneho nacitania (len na volbu predvoleneho vystupu podla kluca).
    ids: Vec<String>,
}

fn publish(shared: &Shared, notify: &Arc<dyn Fn() + Send + Sync>, st: &mut State, new: Snapshot) {
    if new == st.snap {
        return;
    }
    st.snap = Snapshot {
        generation: st.snap.generation + 1,
        ..new
    };
    if let Ok(mut g) = shared.snap.lock() {
        *g = Arc::new(st.snap.clone());
    }
    notify();
}

fn bump(shared: &Shared, f: impl FnOnce(&mut Counters)) {
    if let Ok(mut c) = shared.counters.lock() {
        f(&mut c);
    }
}

fn apply_refresh(
    shared: &Shared,
    notify: &Arc<dyn Fn() + Send + Sync>,
    st: &mut State,
    r: Result<Refresh, AudioError>,
) {
    let mut new = st.snap.clone();
    match r {
        Ok(r) => {
            let new_default = r
                .devices
                .iter()
                .find(|d| d.is_default)
                .map(|d| d.id.clone());
            // zmena predvoleneho vystupu na INY: len ak sme uz jeden poznali (prve nacitanie nie je zmena)
            if st.default_id.is_some() && new_default.is_some() && st.default_id != new_default {
                new.default_changes += 1;
            }
            st.default_id = new_default;
            st.ids = r.devices.iter().map(|d| d.id.clone()).collect();
            new.devices = display_devices(&r.devices);
            let has_active = new
                .devices
                .iter()
                .any(|d| d.is_default || d.state == crate::model::DeviceState::Active);
            new.availability = if has_active {
                Availability::Ok
            } else {
                Availability::NoDevice
            };
            new.volume = if has_active { r.volume } else { None };
            new.controls = r.controls;
        }
        Err(AudioError::NoDevice) => {
            st.default_id = None;
            new.availability = Availability::NoDevice;
            new.devices.clear();
            new.volume = None;
        }
        Err(_) => {
            // sluzba nedostupna: zahodit stav; dalsi pokus az pri dalsom podnete (udalost / otvorenie panela)
            st.default_id = None;
            new.availability = Availability::ServiceUnavailable;
            new.volume = None;
        }
    }
    publish(shared, notify, st, new);
}

#[allow(clippy::too_many_arguments)]
fn worker(
    rx: Receiver<Msg>,
    self_tx: Sender<Msg>,
    event_tx: Sender<Event>,
    shared: Arc<Shared>,
    notify: Arc<dyn Fn() + Send + Sync>,
    tuning: Tuning,
    factory: Factory,
) {
    let mut backend = factory(event_tx);
    let mut st = State {
        snap: Snapshot::default(),
        default_id: None,
        ids: Vec::new(),
    };
    let mut quit = false;
    // `self_tx` drzi frontu nazive; koniec riadi `Quit`
    let _keep = self_tx;
    while !quit {
        let Ok(msg) = rx.recv() else { break };
        let mut pending = vec![msg];
        while let Some(m) = pending.pop() {
            match m {
                Msg::Quit => quit = true,
                Msg::Event(Event::Volume { percent, muted, .. }) => {
                    let mut new = st.snap.clone();
                    if new.availability == Availability::Ok {
                        new.volume = Some(Volume { percent, muted });
                        publish(&shared, &notify, &mut st, new);
                    }
                }
                Msg::SetVolume(p) => {
                    bump(&shared, |c| c.writes += 1);
                    match backend.set_volume(p) {
                        Ok(()) => {}
                        Err(AudioError::Unsupported) => {
                            let mut new = st.snap.clone();
                            new.controls.volume = false;
                            publish(&shared, &notify, &mut st, new);
                        }
                        Err(_) => pending.push(Msg::Refresh),
                    }
                }
                Msg::SetMute(m) => {
                    bump(&shared, |c| c.writes += 1);
                    match backend.set_mute(m) {
                        Ok(()) => {}
                        Err(AudioError::Unsupported) => {
                            let mut new = st.snap.clone();
                            new.controls.mute = false;
                            publish(&shared, &notify, &mut st, new);
                        }
                        Err(_) => pending.push(Msg::Refresh),
                    }
                }
                Msg::SetDefault(key) => {
                    let id = st
                        .ids
                        .iter()
                        .find(|id| crate::model::key_of(id) == key)
                        .cloned();
                    if let Some(id) = id {
                        bump(&shared, |c| c.writes += 1);
                        let _ = backend.set_default(&id);
                        // potvrdenie (aj zlyhanie) ukaze nove nacitanie zoznamu
                        pending.push(Msg::Refresh);
                    }
                }
                Msg::Event(Event::Devices) | Msg::Refresh => {
                    // zlucit: pockat na tichy usek; ostatne spravy sa vybavia hned (poradie sa zachova)
                    let mut later: Vec<Msg> = Vec::new();
                    loop {
                        match rx.recv_timeout(tuning.coalesce) {
                            Ok(Msg::Event(Event::Devices)) | Ok(Msg::Refresh) => continue,
                            Ok(Msg::Quit) => {
                                quit = true;
                                break;
                            }
                            Ok(other) => later.push(other),
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => {
                                quit = true;
                                break;
                            }
                        }
                    }
                    if !quit {
                        bump(&shared, |c| c.refreshes += 1);
                        let r = backend.refresh();
                        apply_refresh(&shared, &notify, &mut st, r);
                    }
                    // spravy z useku sa spracuju po obnoveni (pending je zasobnik: obratene poradie)
                    later.reverse();
                    pending.extend(later);
                }
            }
        }
    }
    backend.shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceState, RawDevice};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::Sender;

    #[derive(Default)]
    struct FakeState {
        devices: Vec<RawDevice>,
        volume: u8,
        muted: bool,
        controls_volume: bool,
        controls_mute: bool,
        fail: Option<AudioError>,
        sets: Vec<u8>,
        mutes: Vec<bool>,
        shut: bool,
        refresh_thread: Option<std::thread::ThreadId>,
    }

    #[derive(Clone)]
    struct Fake {
        st: Arc<Mutex<FakeState>>,
        tx: Arc<Mutex<Option<Sender<Event>>>>,
        refreshes: Arc<AtomicU32>,
    }

    struct FakeBackend(Fake);

    impl Backend for FakeBackend {
        fn refresh(&mut self) -> Result<Refresh, AudioError> {
            self.0.refreshes.fetch_add(1, Ordering::SeqCst);
            let mut s = self.0.st.lock().unwrap();
            s.refresh_thread = Some(std::thread::current().id());
            if let Some(e) = s.fail.clone() {
                return Err(e);
            }
            if s.devices.is_empty() {
                return Err(AudioError::NoDevice);
            }
            Ok(Refresh {
                devices: s.devices.clone(),
                volume: Some(Volume {
                    percent: s.volume,
                    muted: s.muted,
                }),
                controls: Controls {
                    volume: s.controls_volume,
                    mute: s.controls_mute,
                },
            })
        }
        fn set_volume(&mut self, p: u8) -> Result<(), AudioError> {
            let mut s = self.0.st.lock().unwrap();
            if !s.controls_volume {
                return Err(AudioError::Unsupported);
            }
            if let Some(e) = s.fail.clone() {
                return Err(e);
            }
            s.sets.push(p);
            s.volume = p;
            // ozvena: Windows ohlasi nasu vlastnu zmenu
            if let Some(tx) = self.0.tx.lock().unwrap().as_ref() {
                let _ = tx.send(Event::Volume {
                    percent: p,
                    muted: s.muted,
                    own: true,
                });
            }
            Ok(())
        }
        fn set_mute(&mut self, m: bool) -> Result<(), AudioError> {
            let mut s = self.0.st.lock().unwrap();
            if !s.controls_mute {
                return Err(AudioError::Unsupported);
            }
            s.mutes.push(m);
            s.muted = m;
            Ok(())
        }
        fn shutdown(&mut self) {
            self.0.st.lock().unwrap().shut = true;
        }
    }

    fn dev(id: &str, name: &str, def: bool) -> RawDevice {
        RawDevice {
            id: id.into(),
            name: name.into(),
            state: DeviceState::Active,
            is_default: def,
        }
    }

    struct Rig {
        fake: Fake,
        svc: Service,
        notified: Arc<AtomicU32>,
    }

    fn rig(devices: Vec<RawDevice>) -> Rig {
        let fake = Fake {
            st: Arc::new(Mutex::new(FakeState {
                devices,
                volume: 40,
                controls_volume: true,
                controls_mute: true,
                ..Default::default()
            })),
            tx: Arc::new(Mutex::new(None)),
            refreshes: Arc::new(AtomicU32::new(0)),
        };
        let notified = Arc::new(AtomicU32::new(0));
        let n2 = Arc::clone(&notified);
        let f2 = fake.clone();
        let svc = Service::start(
            move |tx| {
                *f2.tx.lock().unwrap() = Some(tx);
                Box::new(FakeBackend(f2))
            },
            Arc::new(move || {
                n2.fetch_add(1, Ordering::SeqCst);
            }),
            Tuning {
                coalesce: Duration::from_millis(40),
            },
        )
        .unwrap();
        Rig {
            fake,
            svc,
            notified,
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

    fn inject(r: &Rig, e: Event) {
        let g = r.fake.tx.lock().unwrap();
        g.as_ref().unwrap().send(e).unwrap();
    }

    #[test]
    fn the_first_load_publishes_devices_default_and_volume_and_notifies() {
        let r = rig(vec![dev("a", "Speakers", true), dev("b", "Headset", false)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        let s = r.svc.snapshot();
        assert_eq!(s.devices.len(), 2);
        assert_eq!(s.default_device().unwrap().name, "Speakers");
        assert_eq!(
            s.volume,
            Some(Volume {
                percent: 40,
                muted: false
            })
        );
        assert_eq!(s.default_changes, 0, "prve nacitanie nie je zmena");
        assert!(r.notified.load(Ordering::SeqCst) >= 1);
        // COM / backend bezi na vlakne sluzby, nie na tomto
        let t = r.fake.st.lock().unwrap().refresh_thread.unwrap();
        assert_ne!(t, std::thread::current().id());
    }

    #[test]
    fn no_endpoints_gives_no_device_and_controls_have_nothing_to_show() {
        let r = rig(vec![]);
        assert!(wait(
            || r.svc.snapshot().availability == Availability::NoDevice
        ));
        let s = r.svc.snapshot();
        assert!(s.devices.is_empty() && s.volume.is_none());
    }

    #[test]
    fn a_service_failure_is_a_visible_state_and_the_next_trigger_recovers_without_polling() {
        let r = rig(vec![dev("a", "Speakers", true)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        r.fake.st.lock().unwrap().fail = Some(AudioError::Unavailable("RPC".into()));
        inject(&r, Event::Devices);
        assert!(wait(
            || r.svc.snapshot().availability == Availability::ServiceUnavailable
        ));
        assert!(r.svc.snapshot().volume.is_none());
        // ziadne dalsie nacitanie samo od seba (ziadny polling)
        let n = r.fake.refreshes.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(r.fake.refreshes.load(Ordering::SeqCst), n);
        // sluzba sa vratila; podnet (otvorenie panela) stav obnovi
        r.fake.st.lock().unwrap().fail = None;
        r.svc.refresh();
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        assert_eq!(r.svc.snapshot().volume.unwrap().percent, 40);
    }

    #[test]
    fn a_burst_of_device_events_is_one_read_and_the_end_state_is_the_last_one() {
        let r = rig(vec![dev("a", "Speakers", true)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        let before = r.fake.refreshes.load(Ordering::SeqCst);
        r.fake
            .st
            .lock()
            .unwrap()
            .devices
            .push(dev("b", "USB DAC", false));
        for _ in 0..1000 {
            inject(&r, Event::Devices);
        }
        assert!(wait(|| r.svc.snapshot().devices.len() == 2));
        std::thread::sleep(Duration::from_millis(150));
        let reads = r.fake.refreshes.load(Ordering::SeqCst) - before;
        assert!((1..=5).contains(&reads), "1000 udalosti = {reads} citani");
    }

    #[test]
    fn the_default_change_is_counted_only_from_a_reported_change_to_another_device() {
        let r = rig(vec![dev("a", "Speakers", true), dev("b", "Headset", false)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        // roly: tri udalosti za jednu zmenu = jedna zmena
        {
            let mut s = r.fake.st.lock().unwrap();
            s.devices = vec![dev("a", "Speakers", false), dev("b", "Headset", true)];
        }
        for _ in 0..3 {
            inject(&r, Event::Devices);
        }
        assert!(wait(|| r.svc.snapshot().default_changes == 1));
        std::thread::sleep(Duration::from_millis(120));
        let s = r.svc.snapshot();
        assert_eq!(s.default_changes, 1);
        assert_eq!(s.default_device().unwrap().name, "Headset");
        // obnovenie bez zmeny predvoleneho nic nepripocita
        r.svc.refresh();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(r.svc.snapshot().default_changes, 1);
    }

    #[test]
    fn removing_the_default_device_moves_the_state_and_never_writes_to_another_device() {
        let r = rig(vec![dev("a", "Speakers", true)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        r.fake.st.lock().unwrap().devices.clear();
        inject(&r, Event::Devices);
        assert!(wait(
            || r.svc.snapshot().availability == Availability::NoDevice
        ));
        // zapis pri odpojenom zariadeni: chyba je hodnota, nie panika; spusti sa len nove nacitanie
        r.fake.st.lock().unwrap().fail = Some(AudioError::NoDevice);
        r.svc.set_volume(55);
        std::thread::sleep(Duration::from_millis(150));
        assert!(r.fake.st.lock().unwrap().sets.is_empty());
        assert_eq!(r.svc.snapshot().availability, Availability::NoDevice);
    }

    #[test]
    fn our_own_change_is_echoed_into_the_model_but_causes_no_further_write() {
        let r = rig(vec![dev("a", "Speakers", true)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        r.svc.set_volume(63);
        assert!(wait(
            || r.svc.snapshot().volume.map(|v| v.percent) == Some(63)
        ));
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            r.fake.st.lock().unwrap().sets,
            vec![63],
            "jeden zapis, ziadna spatna vazba"
        );
        // cudzia zmena zo systemu (napr. klavesa hlasitosti) len zmeni model
        inject(
            &r,
            Event::Volume {
                percent: 20,
                muted: true,
                own: false,
            },
        );
        assert!(wait(|| r.svc.snapshot().volume
            == Some(Volume {
                percent: 20,
                muted: true
            })));
        assert_eq!(r.fake.st.lock().unwrap().sets, vec![63]);
    }

    #[test]
    fn unsupported_volume_or_mute_disables_the_control_and_never_pretends() {
        let r = rig(vec![dev("a", "HDMI", true)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        {
            let mut s = r.fake.st.lock().unwrap();
            s.controls_volume = false;
            s.controls_mute = false;
        }
        r.svc.set_volume(10);
        r.svc.set_mute(true);
        assert!(wait(|| {
            let c = r.svc.snapshot().controls;
            !c.volume && !c.mute
        }));
        let s = r.fake.st.lock().unwrap();
        assert!(s.sets.is_empty() && s.mutes.is_empty());
    }

    #[test]
    fn volume_commands_are_bounded_and_mute_reaches_the_backend() {
        let r = rig(vec![dev("a", "Speakers", true)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        r.svc.set_volume(900);
        r.svc.set_volume(0);
        r.svc.set_mute(true);
        assert!(wait(|| r.fake.st.lock().unwrap().mutes == vec![true]));
        let s = r.fake.st.lock().unwrap();
        assert_eq!(s.sets, vec![100, 0]);
    }

    #[test]
    fn dropping_the_service_shuts_the_backend_down() {
        let r = rig(vec![dev("a", "Speakers", true)]);
        assert!(wait(|| r.svc.snapshot().availability == Availability::Ok));
        let st = Arc::clone(&r.fake.st);
        let flag = Arc::new(AtomicBool::new(false));
        let _ = &flag;
        drop(r);
        assert!(wait(|| st.lock().unwrap().shut));
    }
}
