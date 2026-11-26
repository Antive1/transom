//! Prehravanie zvukov rozhrania mimo UI vlakna: obmedzena fronta, jedno pracovne vlakno, zvukove zariadenie za
//! rozhranim `Backend` (skutocne WASAPI v `wasapi.rs`, v testoch falosne). UI vlakno nikdy necaka na zvuk:
//! `cue` iba skontroluje pravidla a skusi vlozit spravu do fronty (`try_send`); pri plnej fronte alebo prilis
//! rychlych klikoch sa zvuk zahodi.
use crate::policy::{self, Origin, Outcome};
use crate::synth;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use transom_config::{UiSounds, MAX_SOUND_VOLUME};

/// Chyba zvukoveho zariadenia. Ziadna z nich nesmie zhodit ani zablokovat listu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// Nie je dostupne vystupne zariadenie ani zvukova sluzba.
    Unavailable(String),
    /// Zariadenie existuje, ale prehratie zlyhalo (format, zariadenie sa zmenilo, casovy limit).
    Failed(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Unavailable(m) => write!(f, "zariadenie nie je dostupne: {m}"),
            BackendError::Failed(m) => write!(f, "prehratie zlyhalo: {m}"),
        }
    }
}

/// Zvukove zariadenie. Vytvara sa a pouziva len na pracovnom vlakne (nemusi byt `Send`).
pub trait Backend {
    /// Prehra `clip` (16-bitove PCM, stereo, `synth::RATE` Hz) s hlasitostou relacie `volume` (0.0..=1.0). Vrati sa
    /// po dohrati (zvuk je strucny). Hlasitost sa uplatni len na relaciu transom, nikdy na system.
    fn play(&mut self, clip: &[i16], volume: f32) -> Result<(), BackendError>;
    /// Uvolni zariadenie a relaciu (po necinnosti, po chybe a pri ukonceni).
    fn release(&mut self);
}

type Factory = Box<dyn FnOnce() -> Box<dyn Backend> + Send>;

/// Casovanie enginu (v testoch skratene).
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Najmensi odstup medzi dvoma prijatymi zvukmi (rychle opakovane kliky sa zlucuju).
    pub min_interval: Duration,
    /// Po zlyhani zariadenia sa dalsi pokus (okrem nahladu) urobi najskor po tomto case.
    pub backoff: Duration,
    /// Po tomto case bez zvuku sa relacia uvolni (v mixeri zmizne).
    pub idle: Duration,
    /// Zapis do logu (len zmena stavu: nedostupne / obnovene).
    pub log: Option<fn(&str)>,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            min_interval: Duration::from_millis(60),
            backoff: Duration::from_secs(5),
            idle: Duration::from_secs(4),
            log: None,
        }
    }
}

/// Pocitadla na diagnostiku a testy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub played: u64,
    pub failed: u64,
    /// Zahodene, lebo fronta je plna (zariadenie este hra predosly zvuk).
    pub dropped_busy: u64,
    /// Zahodene, lebo prisli prilis rychlo po sebe.
    pub dropped_rate: u64,
    /// Preskocene po zlyhani zariadenia (pocas odstupu `backoff`).
    pub skipped_unavailable: u64,
    /// Zahodene vo fronte, lebo pouzivatel medzitym stlmil zvuky.
    pub skipped_muted: u64,
}

#[derive(Default)]
struct Counters {
    played: AtomicU64,
    failed: AtomicU64,
    dropped_busy: AtomicU64,
    dropped_rate: AtomicU64,
    skipped_unavailable: AtomicU64,
    skipped_muted: AtomicU64,
}

fn bump(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

struct Shared {
    /// Bit 8 = zapnute, nizsich 8 bitov = hlasitost 0..=100.
    settings: AtomicU32,
    stop: AtomicBool,
    counters: Counters,
}

fn pack(s: &UiSounds) -> u32 {
    s.volume.min(MAX_SOUND_VOLUME) | if s.enabled { 0x100 } else { 0 }
}

fn unpack(v: u32) -> UiSounds {
    UiSounds {
        enabled: v & 0x100 != 0,
        volume: (v & 0xFF).min(MAX_SOUND_VOLUME),
    }
}

enum Msg {
    /// Zvuk po akcii: pred prehratim sa znova skontroluje stlmenie.
    Cue,
    /// Nahlad hlasitosti z nastaveni (percenta); hra aj pri stlmenych zvukoch, rozhoduje o tom UI.
    Preview(u32),
}

/// Uchyt na zvukovy engine. `Player::silent()` nema vlakno ani zariadenie a nikdy nehra.
pub struct Player {
    tx: Option<SyncSender<Msg>>,
    shared: Arc<Shared>,
    last: Mutex<Option<Instant>>,
    min_interval: Duration,
}

impl Player {
    /// Mlcky prehravac: ziadne vlakno, zariadenie ani zvuk (testy, `--shot`, automatizacia).
    pub fn silent() -> Player {
        Player {
            tx: None,
            shared: Arc::new(Shared {
                settings: AtomicU32::new(0),
                stop: AtomicBool::new(false),
                counters: Counters::default(),
            }),
            last: Mutex::new(None),
            min_interval: Duration::ZERO,
        }
    }

    /// Prehravac s vlastnym zariadenim (pracovne vlakno `transom-sound`). Zariadenie sa vytvori az na vlakne
    /// (COM). Ak sa vlakno nepodari spustit, vrati mlcky prehravac.
    pub fn with_backend<F>(factory: F, tuning: Tuning) -> Player
    where
        F: FnOnce() -> Box<dyn Backend> + Send + 'static,
    {
        let shared = Arc::new(Shared {
            settings: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            counters: Counters::default(),
        });
        // fronta dlzky 1: jeden zvuk hra, jeden caka, dalsie sa zahodia
        let (tx, rx) = sync_channel::<Msg>(1);
        let worker_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("transom-sound".into())
            .spawn(move || worker(rx, worker_shared, tuning, Box::new(factory)));
        match spawned {
            Ok(_) => Player {
                tx: Some(tx),
                shared,
                last: Mutex::new(None),
                min_interval: tuning.min_interval,
            },
            Err(_) => Player::silent(),
        }
    }

    /// Je to mlcky prehravac (bez zariadenia)?
    pub fn is_silent(&self) -> bool {
        self.tx.is_none()
    }

    /// Nastavi zvuky z konfiguracie (volat pri nacitani aj pri kazdej zmene konfiguracie). Stlmenie plati hned,
    /// aj pre zvuk, ktory uz caka vo fronte.
    pub fn set_settings(&self, s: &UiSounds) {
        self.shared.settings.store(pack(s), Ordering::Relaxed);
    }

    /// Aktualne nastavenie.
    pub fn settings(&self) -> UiSounds {
        unpack(self.shared.settings.load(Ordering::Relaxed))
    }

    /// Podnet z rozhrania: prehra zvuk, ak to pravidla dovolia (`policy::should_play`) a nie je prilis skoro po
    /// predchadzajucom. Nikdy neblokuje. Vrati, ci sa zvuk zaradil.
    pub fn cue(&self, origin: Origin, outcome: Outcome) -> bool {
        let Some(tx) = &self.tx else {
            return false;
        };
        if !policy::should_play(&self.settings(), origin, outcome) {
            return false;
        }
        self.send(tx, Msg::Cue)
    }

    /// Nahlad zvuku pri hlasitosti `percent` (tlacidlo v nastaveniach). Hra aj pri stlmenych zvukoch; tlacidlo
    /// je vtedy v UI vypnute. Nulova hlasitost nehra. Nikdy neblokuje.
    pub fn preview(&self, percent: u32) -> bool {
        let Some(tx) = &self.tx else {
            return false;
        };
        let p = percent.min(MAX_SOUND_VOLUME);
        if p == 0 {
            return false;
        }
        self.send(tx, Msg::Preview(p))
    }

    fn send(&self, tx: &SyncSender<Msg>, msg: Msg) -> bool {
        let now = Instant::now();
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|t| now.duration_since(t) < self.min_interval) {
                bump(&self.shared.counters.dropped_rate);
                return false;
            }
            *last = Some(now);
        }
        match tx.try_send(msg) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                bump(&self.shared.counters.dropped_busy);
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn stats(&self) -> Stats {
        let c = &self.shared.counters;
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        Stats {
            played: get(&c.played),
            failed: get(&c.failed),
            dropped_busy: get(&c.dropped_busy),
            dropped_rate: get(&c.dropped_rate),
            skipped_unavailable: get(&c.skipped_unavailable),
            skipped_muted: get(&c.skipped_muted),
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // pracovne vlakno sa po odpojeni fronty samo skonci (bez cakania: UI sa nezdrzuje)
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn worker(rx: Receiver<Msg>, shared: Arc<Shared>, tuning: Tuning, factory: Factory) {
    let mut backend = factory();
    let clip = synth::click();
    let mut retry_at: Option<Instant> = None;
    let mut down = false;
    let say = |m: &str| {
        if let Some(l) = tuning.log {
            l(m);
        }
    };
    loop {
        match rx.recv_timeout(tuning.idle) {
            Ok(msg) => {
                if shared.stop.load(Ordering::Relaxed) {
                    break;
                }
                let (percent, preview) = match msg {
                    Msg::Cue => {
                        let s = unpack(shared.settings.load(Ordering::Relaxed));
                        (s.effective_volume(), false)
                    }
                    Msg::Preview(p) => (p, true),
                };
                if percent == 0 {
                    bump(&shared.counters.skipped_muted);
                    continue;
                }
                if !preview && retry_at.is_some_and(|t| Instant::now() < t) {
                    bump(&shared.counters.skipped_unavailable);
                    continue;
                }
                match backend.play(clip, policy::session_volume(percent)) {
                    Ok(()) => {
                        bump(&shared.counters.played);
                        retry_at = None;
                        if down {
                            down = false;
                            say("zvuky rozhrania: zariadenie je znova dostupne");
                        }
                    }
                    Err(e) => {
                        bump(&shared.counters.failed);
                        backend.release();
                        retry_at = Some(Instant::now() + tuning.backoff);
                        if !down {
                            down = true;
                            say(&format!("zvuky rozhrania: {e}"));
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => backend.release(),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    backend.release();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Default)]
    struct Fake {
        /// (hlasitost, pocet vzoriek) kazdeho prehrania
        plays: Arc<Mutex<Vec<(f32, usize)>>>,
        fail: Arc<AtomicBool>,
        delay_ms: Arc<AtomicU64>,
        calls: Arc<AtomicUsize>,
        released: Arc<AtomicUsize>,
    }

    struct FakeBackend(Fake);

    impl Backend for FakeBackend {
        fn play(&mut self, clip: &[i16], volume: f32) -> Result<(), BackendError> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            let d = self.0.delay_ms.load(Ordering::SeqCst);
            if d > 0 {
                std::thread::sleep(Duration::from_millis(d));
            }
            if self.0.fail.load(Ordering::SeqCst) {
                return Err(BackendError::Unavailable("ziadne zariadenie".into()));
            }
            self.0.plays.lock().unwrap().push((volume, clip.len()));
            Ok(())
        }
        fn release(&mut self) {
            self.0.released.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn fast() -> Tuning {
        Tuning {
            min_interval: Duration::from_millis(30),
            backoff: Duration::from_millis(150),
            idle: Duration::from_millis(100),
            log: None,
        }
    }

    fn player(f: &Fake, t: Tuning) -> Player {
        let f2 = f.clone();
        Player::with_backend(move || Box::new(FakeBackend(f2)), t)
    }

    fn on(v: u32) -> UiSounds {
        UiSounds {
            enabled: true,
            volume: v,
        }
    }

    fn wait(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..400 {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn a_muted_player_never_starts_a_stream() {
        let f = Fake::default();
        let p = player(&f, fast());
        p.set_settings(&UiSounds {
            enabled: false,
            volume: 80,
        });
        for o in [Origin::Pointer, Origin::Menu, Origin::Popover] {
            assert!(!p.cue(o, Outcome::Done));
        }
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        // nulova hlasitost je to iste
        p.set_settings(&on(0));
        assert!(!p.cue(Origin::Pointer, Outcome::Done));
        assert!(!p.preview(0));
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn only_deliberate_successful_cues_reach_the_device() {
        let f = Fake::default();
        let p = player(&f, fast());
        p.set_settings(&on(40));
        for (o, out) in [
            (Origin::Hover, Outcome::Done),
            (Origin::Press, Outcome::Done),
            (Origin::Background, Outcome::Done),
            (Origin::Animation, Outcome::Done),
            (Origin::Automation, Outcome::Done),
            (Origin::Pointer, Outcome::Disabled),
            (Origin::Menu, Outcome::Failed),
        ] {
            assert!(!p.cue(o, out), "{o:?} {out:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(wait(|| f.plays.lock().unwrap().len() == 1));
        let (vol, len) = f.plays.lock().unwrap()[0];
        assert!((vol - 0.4).abs() < 1e-6, "{vol}");
        assert_eq!(len, synth::click().len());
    }

    #[test]
    fn the_volume_sent_to_the_device_is_always_within_zero_and_one() {
        let f = Fake::default();
        let p = player(&f, fast());
        for stored in [1, 50, 100, 5000, u32::MAX] {
            p.set_settings(&on(stored));
            let before = f.plays.lock().unwrap().len();
            assert!(p.cue(Origin::Pointer, Outcome::Done), "{stored}");
            assert!(wait(|| f.plays.lock().unwrap().len() == before + 1));
            let v = f.plays.lock().unwrap()[before].0;
            assert!((0.0..=1.0).contains(&v), "{stored} -> {v}");
            std::thread::sleep(Duration::from_millis(35));
        }
        for pv in [1, 100, 101, u32::MAX] {
            let before = f.plays.lock().unwrap().len();
            assert!(p.preview(pv));
            assert!(wait(|| f.plays.lock().unwrap().len() == before + 1));
            let v = f.plays.lock().unwrap()[before].0;
            assert!((0.0..=1.0).contains(&v), "{pv} -> {v}");
            std::thread::sleep(Duration::from_millis(35));
        }
    }

    #[test]
    fn muting_applies_immediately_even_to_a_sound_already_waiting_in_the_queue() {
        let f = Fake::default();
        f.delay_ms.store(120, Ordering::SeqCst);
        let p = player(&f, fast());
        p.set_settings(&on(50));
        assert!(p.cue(Origin::Pointer, Outcome::Done)); // hra 120 ms
        std::thread::sleep(Duration::from_millis(40));
        assert!(p.cue(Origin::Pointer, Outcome::Done)); // caka vo fronte
        p.set_settings(&UiSounds {
            enabled: false,
            volume: 50,
        });
        assert!(wait(|| p.stats().skipped_muted == 1), "{:?}", p.stats());
        assert_eq!(f.plays.lock().unwrap().len(), 1, "druhy zvuk uz nezaznel");
    }

    #[test]
    fn a_missing_device_never_panics_or_blocks_and_is_retried_after_the_backoff() {
        let f = Fake::default();
        f.fail.store(true, Ordering::SeqCst);
        let p = player(&f, fast());
        p.set_settings(&on(50));
        let t = Instant::now();
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(t.elapsed() < Duration::from_millis(50), "cue neblokuje");
        assert!(wait(|| p.stats().failed == 1));
        assert!(
            f.released.load(Ordering::SeqCst) >= 1,
            "zariadenie sa uvolnilo"
        );
        // pocas odstupu sa zariadenie nevola znova
        std::thread::sleep(Duration::from_millis(40));
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(
            wait(|| p.stats().skipped_unavailable == 1),
            "{:?}",
            p.stats()
        );
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        // zariadenie sa vratilo: po odstupe sa hra znova
        f.fail.store(false, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(160));
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(wait(|| p.stats().played == 1), "{:?}", p.stats());
        // nahlad zo Nastaveni skusi zariadenie aj pocas odstupu
        f.fail.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(40));
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(wait(|| p.stats().failed == 2));
        f.fail.store(false, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(40));
        assert!(p.preview(30));
        assert!(wait(|| p.stats().played == 2), "{:?}", p.stats());
    }

    #[test]
    fn a_slow_device_never_blocks_the_caller_and_extra_cues_are_dropped() {
        let f = Fake::default();
        f.delay_ms.store(250, Ordering::SeqCst);
        let p = player(
            &f,
            Tuning {
                min_interval: Duration::ZERO,
                ..fast()
            },
        );
        p.set_settings(&on(50));
        let t = Instant::now();
        let accepted = (0..50)
            .filter(|_| p.cue(Origin::Pointer, Outcome::Done))
            .count();
        assert!(
            t.elapsed() < Duration::from_millis(100),
            "{:?}",
            t.elapsed()
        );
        assert!((1..=3).contains(&accepted), "{accepted}");
        assert!(p.stats().dropped_busy >= 40, "{:?}", p.stats());
    }

    #[test]
    fn rapid_repeated_clicks_are_merged_by_the_rate_limit() {
        let f = Fake::default();
        let p = player(
            &f,
            Tuning {
                min_interval: Duration::from_millis(200),
                ..fast()
            },
        );
        p.set_settings(&on(50));
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(!p.cue(Origin::Pointer, Outcome::Done));
        assert!(!p.cue(Origin::Menu, Outcome::Done));
        assert_eq!(p.stats().dropped_rate, 2);
        std::thread::sleep(Duration::from_millis(210));
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(wait(|| f.plays.lock().unwrap().len() == 2));
    }

    #[test]
    fn the_device_is_released_after_idle_so_the_mixer_session_disappears() {
        let f = Fake::default();
        let p = player(&f, fast());
        p.set_settings(&on(50));
        assert!(p.cue(Origin::Pointer, Outcome::Done));
        assert!(wait(|| f.plays.lock().unwrap().len() == 1));
        let r = f.released.load(Ordering::SeqCst);
        assert!(
            wait(|| f.released.load(Ordering::SeqCst) > r),
            "po necinnosti"
        );
    }

    #[test]
    fn a_silent_player_has_no_thread_and_never_plays() {
        let p = Player::silent();
        assert!(p.is_silent());
        p.set_settings(&on(100));
        assert!(!p.cue(Origin::Pointer, Outcome::Done));
        assert!(!p.preview(100));
        assert_eq!(p.stats(), Stats::default());
    }

    #[test]
    fn settings_round_trip_through_the_packed_atomic() {
        let p = Player::silent();
        for s in [
            on(0),
            on(25),
            on(100),
            UiSounds {
                enabled: false,
                volume: 70,
            },
        ] {
            p.set_settings(&s);
            assert_eq!(p.settings(), s);
        }
        p.set_settings(&on(9999));
        assert_eq!(p.settings().volume, MAX_SOUND_VOLUME);
    }
}
