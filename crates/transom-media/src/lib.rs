//! GSMTC sluzba (`Windows.Media.Control`), overena v S5 bez package identity.
//!
//! Navrh (PLAN §6):
//! - Vsetky WinRT/WIC volania bezia na **vlastnom MTA vlakne**, nikdy na UI vlakne (STA volania by pumpovali spravy
//!   a znovu vstupovali do wndproc hostitela).
//! - Handlery udalosti (`SessionsChanged`, `MediaPropertiesChanged`, `PlaybackInfoChanged`) iba poslu poziadavku do
//!   kanala; vlakno davku zluci, nacita stav a ak sa zmenil, ulozi novu snimku a zavola `notify`.
//!   UI vlakno si potom precita **poslednu** snimku (stratena notifikacia teda nemoze nechat stary stav navzdy).
//! - `TimelinePropertiesChanged` sa neodoberá (Spotify ju posiela ~15x/min, lista poziciu nezobrazuje).
//! - Tokeny handlerov sa ukladaju a pri `SessionsChanged` a ukonceni sa odhlasia (chyba z S5 v1: nasobne handlery).
//! - Session tej istej aplikacie (rovnake AUMID) dostanu kluc s priponou `#2`, `#3` podla poradia v `GetSessions`.
//!
//! Obal / nahlad (`MediaProperties.Thumbnail`):
//! - cita sa **len** pri `MediaPropertiesChanged` danej session (a pri prihlaseni k session), nie pri zmene stavu;
//! - po zmene skladby (udalost aj prikaz dalsia / predosla) sa session docita este [`FOLLOW_UPS_MS`] po nej: Spotify
//!   posiela udalost skor, nez ma novy nazov a hlavne obal pripraveny, a dalsia udalost uz nemusi prist (na pomalsom PC
//!   by ostal stary obal / nazov az do dalsej zmeny);
//! - bajty (max. 8 MB) sa zahashuju; rovnaky obsah sa dekoduje len raz (cache podla hashu, max. 24 obrazkov);
//! - dekodovanie cez WIC: stredovy stvorcovy vyrez, zmensenie na 96 x 96 (Fant), `32bppPBGRA`;
//! - obal sa viaze na identitu media (`SessionInfo::media_key`, `resolve_art`): zastaraly obal inej skladby sa
//!   nezobrazi, chybajuci nahlad = zastupny symbol.
#![cfg(windows)]

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use transom_core::image::{content_hash, Image};

/// Docitania session po zmene skladby (ms po udalosti / prikaze).
const FOLLOW_UPS_MS: [u64; 4] = [250, 800, 2000, 4000];

/// Naplanuje docitania session `key` (nahradi predosle planovane docitania tej istej session).
fn plan_follow_ups(f: &mut Vec<(Instant, String)>, key: &str, now: Instant) {
    f.retain(|(_, k)| k != key);
    f.extend(
        FOLLOW_UPS_MS
            .iter()
            .map(|ms| (now + Duration::from_millis(*ms), key.to_string())),
    );
}

/// Docitania, ktore uz nastali: odstrani ich a vrati kluce session (bez opakovania).
fn due_follow_ups(f: &mut Vec<(Instant, String)>, now: Instant) -> Vec<String> {
    let mut due = Vec::new();
    f.retain(|(t, k)| {
        if *t <= now {
            if !due.contains(k) {
                due.push(k.clone());
            }
            false
        } else {
            true
        }
    });
    due
}
use transom_core::media::{
    resolve_art, stable_state, Controls, Fetched, PlayState, SessionInfo, Timeline,
};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Props,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand, WICRect,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

/// Strana dekodovaneho obalu v px (staci na 22 lp pri 200 % = 44 px, s rezervou pre popover).
pub const ART_SIDE: u32 = 96;
const MAX_THUMB_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IMAGES: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
    /// Presun na poziciu (jednotky 100 ns ako casova os relacie, absolutne - vratane `start`).
    Seek(i64),
}

/// Posledny znamy stav vsetkych session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// GSMTC manazer je dostupny (inak `error`).
    pub available: bool,
    pub error: Option<String>,
    pub sessions: Vec<SessionInfo>,
    /// Zvysi sa pri kazdej zverejnenej zmene.
    pub generation: u64,
    /// Pocitadla pre diagnostiku/meranie.
    pub refreshes: u64,
    pub commands_failed: u64,
    pub thumb_reads: u64,
    pub decodes: u64,
    pub cache_hits: u64,
}

enum Msg {
    Refresh,
    /// `MediaPropertiesChanged` session s klucom (nahlad sa ma precitat znova).
    Props(String),
    SessionsChanged,
    Command(String, Command),
    Quit,
}

type Images = Arc<Mutex<HashMap<u64, Arc<Image>>>>;
type LogFn = Box<dyn Fn(&str) + Send>;

pub struct MediaService {
    tx: Sender<Msg>,
    shared: Arc<Mutex<Snapshot>>,
    images: Images,
    thread: Option<JoinHandle<()>>,
}

impl MediaService {
    /// Spusti vlakno. `notify` sa vola z media vlakna po kazdej zverejnenej zmene (napr. `PostMessage` do UI);
    /// `log` dostava diagnosticke riadky (citanie nahladov, cache).
    pub fn start(notify: impl Fn() + Send + 'static, log: impl Fn(&str) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let images: Images = Arc::new(Mutex::new(HashMap::new()));
        let (tx2, sh2, im2) = (tx.clone(), shared.clone(), images.clone());
        let thread = std::thread::Builder::new()
            .name("transom-media".into())
            .spawn(move || worker(rx, tx2, sh2, im2, Box::new(notify), Box::new(log)))
            .ok();
        MediaService {
            tx,
            shared,
            images,
            thread,
        }
    }

    /// Kopia poslednej snimky (lacne: niekolko kratkych retazcov).
    pub fn snapshot(&self) -> Snapshot {
        lock(&self.shared).clone()
    }

    /// Dekodovany obal podla `SessionInfo::art_id`.
    pub fn image(&self, id: u64) -> Option<Arc<Image>> {
        self.images
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
    }

    /// Znova precita stav relacii (aj casovu os; snimka sa obnovi aj bez notifikacie). Na zobrazenu poziciu skladby.
    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    /// Posle prikaz session s klucom `key`; vykona sa asynchronne na media vlakne.
    pub fn command(&self, key: &str, cmd: Command) {
        let _ = self.tx.send(Msg::Command(key.to_string(), cmd));
    }
}

impl Drop for MediaService {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn lock(m: &Mutex<Snapshot>) -> std::sync::MutexGuard<'_, Snapshot> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Sub {
    key: String,
    session: Session,
    t_props: Option<i64>,
    t_play: Option<i64>,
}

fn state(s: Status) -> PlayState {
    match s {
        Status::Opened => PlayState::Opened,
        Status::Changing => PlayState::Changing,
        Status::Stopped => PlayState::Stopped,
        Status::Playing => PlayState::Playing,
        Status::Paused => PlayState::Paused,
        _ => PlayState::Closed,
    }
}

fn unsubscribe(subs: &mut Vec<Sub>) {
    for s in subs.drain(..) {
        if let Some(t) = s.t_props {
            let _ = s.session.RemoveMediaPropertiesChanged(t);
        }
        if let Some(t) = s.t_play {
            let _ = s.session.RemovePlaybackInfoChanged(t);
        }
    }
}

fn resubscribe(mgr: &Manager, subs: &mut Vec<Sub>, tx: &Sender<Msg>) {
    unsubscribe(subs);
    let Ok(list) = mgr.GetSessions() else { return };
    let mut seen: HashMap<String, u32> = HashMap::new();
    for session in list {
        let app = session
            .SourceAppUserModelId()
            .map(|x| x.to_string())
            .unwrap_or_default();
        let n = seen.entry(app.clone()).or_insert(0);
        *n += 1;
        let key = if *n == 1 { app } else { format!("{app}#{n}") };
        let (a, b, k) = (tx.clone(), tx.clone(), key.clone());
        let t_props = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = a.send(Msg::Props(k.clone()));
                Ok(())
            }))
            .ok();
        let t_play = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = b.send(Msg::Refresh);
                Ok(())
            }))
            .ok();
        subs.push(Sub {
            key,
            session,
            t_props,
            t_play,
        });
    }
}

fn read(sub: &Sub) -> (SessionInfo, Option<Props>) {
    let s = &sub.session;
    let mut info = SessionInfo {
        key: sub.key.clone(),
        app_id: s
            .SourceAppUserModelId()
            .map(|x| x.to_string())
            .unwrap_or_default(),
        ..Default::default()
    };
    let props = s.TryGetMediaPropertiesAsync().and_then(|a| a.join()).ok();
    if let Some(p) = &props {
        info.title = p.Title().map(|x| x.to_string()).unwrap_or_default();
        info.artist = p.Artist().map(|x| x.to_string()).unwrap_or_default();
        info.album = p.AlbumTitle().map(|x| x.to_string()).unwrap_or_default();
    }
    if let Ok(pi) = s.GetPlaybackInfo() {
        info.state = pi.PlaybackStatus().map(state).unwrap_or_default();
        if let Ok(c) = pi.Controls() {
            info.controls = Controls {
                play: c.IsPlayEnabled().unwrap_or(false),
                pause: c.IsPauseEnabled().unwrap_or(false),
                toggle: c.IsPlayPauseToggleEnabled().unwrap_or(false),
                next: c.IsNextEnabled().unwrap_or(false),
                prev: c.IsPreviousEnabled().unwrap_or(false),
                seek: c.IsPlaybackPositionEnabled().unwrap_or(false),
            };
        }
    }
    if let Ok(t) = s.GetTimelineProperties() {
        info.timeline = Timeline {
            start: t.StartTime().map(|d| d.Duration).unwrap_or(0),
            end: t.EndTime().map(|d| d.Duration).unwrap_or(0),
            position: t.Position().map(|d| d.Duration).unwrap_or(0),
            last_updated: t.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0),
        };
    }
    (info, props)
}

/// Bajty nahladu (`None` = session nahlad nema alebo sa neda precitat).
fn thumbnail_bytes(p: &Props) -> Option<Vec<u8>> {
    let r = p.Thumbnail().ok()?;
    let st = r.OpenReadAsync().ok()?.join().ok()?;
    let size = st.Size().ok()?;
    if size == 0 || size > MAX_THUMB_BYTES {
        return None;
    }
    let dr = DataReader::CreateDataReader(&st).ok()?;
    dr.LoadAsync(size as u32).ok()?.join().ok()?;
    let mut buf = vec![0u8; size as usize];
    dr.ReadBytes(&mut buf).ok()?;
    Some(buf)
}

/// Najvacsi povoleny rozmer obalu (strana a plocha). Komprimovany limit (`MAX_THUMB_BYTES`) sam dekompresnu bombu
/// (maly subor, obrovsky obraz) nezastavi.
const MAX_ART_SIDE: u32 = 16_384;
const MAX_ART_PIXELS: u64 = 64_000_000;

/// Platne rozmery obalu: nenulove a v povolenych medziach.
fn dimensions_ok(w: u32, h: u32) -> bool {
    w > 0
        && h > 0
        && w <= MAX_ART_SIDE
        && h <= MAX_ART_SIDE
        && u64::from(w) * u64::from(h) <= MAX_ART_PIXELS
}

/// Dekoduje obrazok cez WIC: stredovy stvorec, zmenseny na `ART_SIDE`, premultiplikovane BGRA.
fn decode(f: &IWICImagingFactory, bytes: &[u8]) -> windows::core::Result<Image> {
    // SAFETY: WIC objekty ziju pocas bloku; pixely sa kopiruju do buffra s velkostou podla sirky, vysky a kroku.
    unsafe {
        let stream = f.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let dec =
            f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = dec.GetFrame(0)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        if !dimensions_ok(w, h) {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        let side = w.min(h);
        let clip = f.CreateBitmapClipper()?;
        let rect = WICRect {
            X: ((w - side) / 2) as i32,
            Y: ((h - side) / 2) as i32,
            Width: side as i32,
            Height: side as i32,
        };
        clip.Initialize(&frame, &rect)?;
        let scaler = f.CreateBitmapScaler()?;
        scaler.Initialize(&clip, ART_SIDE, ART_SIDE, WICBitmapInterpolationModeFant)?;
        let conv = f.CreateFormatConverter()?;
        conv.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut buf = vec![0u8; (ART_SIDE * ART_SIDE * 4) as usize];
        conv.CopyPixels(std::ptr::null(), ART_SIDE * 4, &mut buf)?;
        Image::from_pbgra(ART_SIDE, ART_SIDE, &buf)
            .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
    }
}

fn run_command(subs: &[Sub], key: &str, cmd: Command) -> bool {
    let Some(sub) = subs.iter().find(|s| s.key == key) else {
        return false;
    };
    let s = &sub.session;
    let r = match cmd {
        Command::Toggle => s.TryTogglePlayPauseAsync().and_then(|a| a.join()),
        Command::Play => s.TryPlayAsync().and_then(|a| a.join()),
        Command::Pause => s.TryPauseAsync().and_then(|a| a.join()),
        Command::Next => s.TrySkipNextAsync().and_then(|a| a.join()),
        Command::Prev => s.TrySkipPreviousAsync().and_then(|a| a.join()),
        Command::Seek(t) => s.TryChangePlaybackPositionAsync(t).and_then(|a| a.join()),
    };
    r.unwrap_or(false)
}

/// Porovnanie bez casovej osi: jej zmena sama o sebe lisu neprekresluje (pozicia sa nezobrazuje).
fn same_for_display(a: &[SessionInfo], b: &[SessionInfo]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            SessionInfo {
                timeline: Timeline::default(),
                ..x.clone()
            } == SessionInfo {
                timeline: Timeline::default(),
                ..y.clone()
            }
        })
}

/// Stav obalov na media vlakne.
struct Art {
    factory: Option<IWICImagingFactory>,
    images: Images,
    /// kluc session -> (identita media, obal) z poslednej snimky
    last: HashMap<String, (String, Option<u64>)>,
}

impl Art {
    /// Precita a (ak treba) dekoduje nahlad; vrati vysledok pre `resolve_art`.
    fn fetch(
        &mut self,
        info: &SessionInfo,
        props: Option<&Props>,
        shared: &Mutex<Snapshot>,
        log: &LogFn,
    ) -> Fetched {
        let t0 = Instant::now();
        lock(shared).thumb_reads += 1;
        let Some(bytes) = props.and_then(thumbnail_bytes) else {
            log(&format!(
                "nahlad [{}] '{}' album='{}': ziadny",
                info.key, info.title, info.album
            ));
            return Fetched::Missing;
        };
        let hash = content_hash(&bytes);
        let cached = self
            .images
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&hash);
        if cached {
            lock(shared).cache_hits += 1;
            log(&format!(
                "nahlad [{}] '{}' album='{}': {} B hash={hash:016x} cache",
                info.key,
                info.title,
                info.album,
                bytes.len()
            ));
            return Fetched::Image(hash);
        }
        if self.factory.is_none() {
            self.factory =
                // SAFETY: COM je inicializovany na tomto vlakne (MTA, `run`).
                unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                    .ok();
        }
        let Some(f) = &self.factory else {
            return Fetched::Missing;
        };
        match decode(f, &bytes) {
            Ok(img) => {
                lock(shared).decodes += 1;
                log(&format!(
                    "nahlad [{}] '{}' album='{}': {} B hash={hash:016x} dekodovane za {} ms",
                    info.key,
                    info.title,
                    info.album,
                    bytes.len(),
                    t0.elapsed().as_millis()
                ));
                self.images
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(hash, Arc::new(img));
                Fetched::Image(hash)
            }
            Err(e) => {
                log(&format!("nahlad [{}]: dekodovanie zlyhalo: {e}", info.key));
                Fetched::Missing
            }
        }
    }

    /// Udrzi cache v limite: zahodi obrazky, na ktore neodkazuje ziadna aktualna session.
    fn trim(&self, sessions: &[SessionInfo]) {
        let mut g = self.images.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() <= MAX_IMAGES {
            return;
        }
        let used: HashSet<u64> = sessions.iter().filter_map(|s| s.art_id).collect();
        g.retain(|k, _| used.contains(k));
    }
}

fn worker(
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    shared: Arc<Mutex<Snapshot>>,
    images: Images,
    notify: Box<dyn Fn() + Send>,
    log: LogFn,
) {
    // SAFETY: COM pre toto vlakno; ak sa inicializoval, na konci sa CoUninitialize.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let mgr = match Manager::RequestAsync().and_then(|a| a.join()) {
        Ok(m) => m,
        Err(e) => {
            {
                let mut g = lock(&shared);
                g.available = false;
                g.error = Some(format!("GSMTC nedostupne: {e}"));
                g.generation += 1;
            }
            notify();
            // cakat na ukoncenie; prikazy nie je kam poslat
            while let Ok(m) = rx.recv() {
                if matches!(m, Msg::Quit) {
                    break;
                }
            }
            if com {
                // SAFETY: par k uspesnemu CoInitializeEx na tom istom vlakne.
                unsafe { CoUninitialize() };
            }
            return;
        }
    };
    let t_sessions = {
        let a = tx.clone();
        mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = a.send(Msg::SessionsChanged);
            Ok(())
        }))
        .ok()
    };
    let mut subs = Vec::new();
    resubscribe(&mgr, &mut subs, &tx);
    let mut art = Art {
        factory: None,
        images,
        last: HashMap::new(),
    };
    // nahlady na precitanie (pri prihlaseni vsetky session)
    let mut art_pending: HashSet<String> = subs.iter().map(|s| s.key.clone()).collect();
    // posledny stav a poradove cislo prechodu do Playing (pre "naposledy zacala hrat")
    let mut seq: HashMap<String, (PlayState, u64)> = HashMap::new();
    let mut counter = 0u64;
    let mut published: Option<Vec<SessionInfo>> = None;
    let mut pending_refresh = true;
    // planovane docitania po zmene skladby (cas, kluc session)
    let mut follow_ups: Vec<(Instant, String)> = Vec::new();

    loop {
        if pending_refresh {
            pending_refresh = false;
            let mut sessions = Vec::with_capacity(subs.len());
            for sub in &subs {
                let (mut s, props) = read(sub);
                let e = seq.entry(s.key.clone()).or_insert((PlayState::Closed, 0));
                let prev_state = (e.0 != PlayState::Closed).then_some(e.0);
                s.state = stable_state(prev_state, s.state);
                if s.state == PlayState::Playing && e.0 != PlayState::Playing {
                    counter += 1;
                    e.1 = counter;
                }
                e.0 = s.state;
                s.play_seq = e.1;
                let key = s.media_key();
                let fetched = if art_pending.remove(&s.key) {
                    art.fetch(&s, props.as_ref(), &shared, &log)
                } else {
                    Fetched::NotRead
                };
                let prev = art.last.get(&s.key).map(|(k, a)| (k.as_str(), *a));
                s.art_id = resolve_art(prev, &key, fetched);
                art.last.insert(s.key.clone(), (key, s.art_id));
                sessions.push(s);
            }
            seq.retain(|k, _| sessions.iter().any(|s| &s.key == k));
            art.last.retain(|k, _| sessions.iter().any(|s| &s.key == k));
            art.trim(&sessions);
            let changed = published
                .as_deref()
                .is_none_or(|p| !same_for_display(p, &sessions));
            {
                let mut g = lock(&shared);
                g.available = true;
                g.error = None;
                g.refreshes += 1;
                g.sessions = sessions.clone();
                if changed {
                    g.generation += 1;
                }
            }
            if changed {
                published = Some(sessions);
                notify();
            }
        }
        // cakat na udalost, najdlhsie do najblizsieho docitania
        let first = match follow_ups.iter().map(|(t, _)| *t).min() {
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            },
            Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        };
        for k in due_follow_ups(&mut follow_ups, Instant::now()) {
            art_pending.insert(k);
            pending_refresh = true;
        }
        let Some(first) = first else { continue };
        // udalosti chodia v davkach (props + playback): kratko pockat a zlucit
        std::thread::sleep(Duration::from_millis(30));
        let mut msgs = vec![first];
        msgs.extend(rx.try_iter());
        let mut quit = false;
        let mut sessions_changed = false;
        let mut cmds = Vec::new();
        for m in msgs {
            match m {
                Msg::Quit => quit = true,
                Msg::Refresh => pending_refresh = true,
                Msg::Props(k) => {
                    plan_follow_ups(&mut follow_ups, &k, Instant::now());
                    art_pending.insert(k);
                    pending_refresh = true;
                }
                Msg::SessionsChanged => sessions_changed = true,
                Msg::Command(k, c) => cmds.push((k, c)),
            }
        }
        if quit {
            break;
        }
        if sessions_changed {
            resubscribe(&mgr, &mut subs, &tx);
            // zoznam sa zmenil: kluce `AUMID#n` sa mozu precislovat, preto precitat nahlady vsetkych session
            // (rovnaky obsah sa vdaka cache podla hashu nededekoduje znova)
            for s in &subs {
                art_pending.insert(s.key.clone());
            }
            pending_refresh = true;
        }
        for (k, c) in cmds {
            // dalsia / predosla skladba: docitat aj bez udalosti (niektore prehravace ju poslu neskoro alebo vobec)
            plan_follow_ups(&mut follow_ups, &k, Instant::now());
            if !run_command(&subs, &k, c) {
                lock(&shared).commands_failed += 1;
            }
            // vysledok prikazu pride ako udalost; obnovit aj tak (niektore prehravace udalost neposlu)
            pending_refresh = true;
        }
    }
    unsubscribe(&mut subs);
    if let Some(t) = t_sessions {
        let _ = mgr.RemoveSessionsChanged(t);
    }
    drop(mgr);
    // WIC objekty uvolnit pred CoUninitialize
    drop(art);
    if com {
        // SAFETY: par k uspesnemu CoInitializeEx na tom istom vlakne.
        unsafe { CoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_ups_are_planned_per_session_and_fire_once() {
        let t0 = Instant::now();
        let mut f = Vec::new();
        plan_follow_ups(&mut f, "Spotify.exe", t0);
        plan_follow_ups(&mut f, "Spotify.exe", t0);
        assert_eq!(f.len(), FOLLOW_UPS_MS.len(), "opakovana zmena nahradi plan");
        plan_follow_ups(&mut f, "Brave", t0);
        assert!(due_follow_ups(&mut f, t0).is_empty());
        let due = due_follow_ups(&mut f, t0 + Duration::from_millis(300));
        assert_eq!(due.len(), 2, "{due:?}");
        assert_eq!(f.len(), 2 * (FOLLOW_UPS_MS.len() - 1));
        let due = due_follow_ups(&mut f, t0 + Duration::from_secs(5));
        assert_eq!(due.len(), 2);
        assert!(f.is_empty());
    }

    #[test]
    fn art_dimensions_are_limited_against_decompression_bombs() {
        assert!(dimensions_ok(1, 1) && dimensions_ok(640, 640) && dimensions_ok(8000, 8000));
        assert!(
            !dimensions_ok(0, 100) && !dimensions_ok(100, 0),
            "nulovy rozmer"
        );
        assert!(!dimensions_ok(16_385, 10), "prilis siroky");
        assert!(!dimensions_ok(10, 16_385), "prilis vysoky");
        assert!(!dimensions_ok(u32::MAX, u32::MAX), "nesmie pretiect");
        assert!(!dimensions_ok(16_384, 16_384), "268 Mpx > 64 Mpx");
        assert!(
            dimensions_ok(8000, 8000) && !dimensions_ok(8001, 8000),
            "hranica 64 Mpx"
        );
    }

    #[test]
    fn timeline_only_change_is_not_a_display_change() {
        let a = SessionInfo {
            key: "A".into(),
            title: "t".into(),
            ..Default::default()
        };
        let mut b = a.clone();
        b.timeline.position = 123;
        assert!(same_for_display(
            std::slice::from_ref(&a),
            std::slice::from_ref(&b)
        ));
        b.title = "u".into();
        assert!(!same_for_display(
            std::slice::from_ref(&a),
            std::slice::from_ref(&b)
        ));
        // zmena obalu je zmena zobrazenia
        let mut c = a.clone();
        c.art_id = Some(5);
        assert!(!same_for_display(&[a], &[c]));
    }

    #[test]
    fn wic_decodes_bmp_to_center_square() {
        // 4x2 BMP (24 bpp, zdola nahor): lava polovica cervena, prava modra -> stredovy stvorec 2x2 obsahuje oboje
        let mut bmp = Vec::new();
        let row_bytes = 4 * 3; // 12, uz zarovnane na 4
        let data_len = row_bytes * 2;
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&((54 + data_len) as u32).to_le_bytes());
        bmp.extend_from_slice(&[0, 0, 0, 0]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&4i32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 24]);
        for _ in 0..2 {
            for x in 0..4 {
                if x < 2 {
                    bmp.extend_from_slice(&[0, 0, 255]); // BGR cervena
                } else {
                    bmp.extend_from_slice(&[255, 0, 0]); // modra
                }
            }
        }
        // SAFETY: COM pre testovacie vlakno; na konci sa CoUninitialize.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        {
            let f: IWICImagingFactory =
                // SAFETY: COM je inicializovany na tomto vlakne.
                unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                    .unwrap();
            let img = decode(&f, &bmp).unwrap();
            assert_eq!((img.w, img.h), (ART_SIDE, ART_SIDE));
            let left = img.px[(ART_SIDE * ART_SIDE / 2 + 2) as usize];
            let right = img.px[(ART_SIDE * ART_SIDE / 2 + ART_SIDE - 3) as usize];
            // interpolacia (Fant) pri zvacseni 2 -> 96 miesa okraje; rozhoduje dominantny kanal
            let ch = |p: u32| ((p >> 16) & 0xFF, p & 0xFF);
            let (lr, lb) = ch(left);
            let (rr, rb) = ch(right);
            assert!(lr > 0xC0 && lb < 0x40, "lava strana cervena: {left:08x}");
            assert!(rb > 0xC0 && rr < 0x40, "prava strana modra: {right:08x}");
            assert!(decode(&f, b"nie je obrazok").is_err());
        }
        if com {
            // SAFETY: par k uspesnemu CoInitializeEx na tom istom vlakne.
            unsafe { CoUninitialize() };
        }
    }
}
