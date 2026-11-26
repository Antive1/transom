//! Jas monitorov cez DDC/CI (dokumentovane Monitor Configuration API v `dxva2`: `GetPhysicalMonitorsFromHMONITOR`,
//! `GetVCPFeatureAndVCPFeatureReply` / `SetVCPFeature`, kod VCP 0x10 = jas).
//!
//! * Volania DDC su pomale (desiatky az stovky ms) a niektore monitory si kazdu zmenu zapisu do pamate: vsetko bezi na
//!   vlastnom vlakne, pocas tahania sa zapisuje najviac raz za `WRITE_GAP` a vzdy len posledna hodnota.
//! * Zobrazuje sa len to, co monitor po zapise naozaj hlasi (spatne citanie), nie pozadovana hodnota.
//! * Monitor bez DDC/CI (alebo s vypnutym DDC/CI v jeho menu, vstavany displej notebooku) sa oznaci ako nepodporovany.
use crate::monitor::{self, MonitorInfo};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, SetVCPFeature,
    PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};

const VCP_BRIGHTNESS: u8 = 0x10;
/// Najmensi odstup zapisov na jeden monitor.
const WRITE_GAP: Duration = Duration::from_millis(120);

/// Jeden monitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// Stabilne ID monitora (device interface path).
    pub key: String,
    pub name: String,
    /// Jas 0-100 % podla monitora; `None` = monitor DDC/CI nepodporuje alebo neodpoveda.
    pub level: Option<u32>,
    pub primary: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub ready: bool,
    pub displays: Vec<Display>,
    /// Posledny zapis zlyhal (kluc monitora).
    pub failed: Option<String>,
    pub generation: u64,
}

enum Msg {
    Refresh,
    Set(String, u32),
    Quit,
}

pub struct Service {
    tx: Sender<Msg>,
    snap: Arc<Mutex<Arc<Snapshot>>>,
}

impl Service {
    /// Spusti vlakno a prve citanie. `notify` sa vola z vlakna po kazdej zmene snimky.
    pub fn start(notify: Arc<dyn Fn() + Send + Sync>) -> Result<Service, String> {
        let (tx, rx) = channel();
        let snap = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let s2 = Arc::clone(&snap);
        std::thread::Builder::new()
            .name("transom-brightness".into())
            .spawn(move || worker(rx, s2, notify))
            .map_err(|e| e.to_string())?;
        let _ = tx.send(Msg::Refresh);
        Ok(Service { tx, snap })
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snap.lock().map(|g| Arc::clone(&g)).unwrap_or_default()
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    /// Nastavi jas monitora (0-100 %). Neblokuje; viac poziadaviek za sebou sa zluci na poslednu.
    pub fn set(&self, key: &str, percent: u32) {
        let _ = self.tx.send(Msg::Set(key.to_string(), percent.min(100)));
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

struct Phys {
    key: String,
    name: String,
    primary: bool,
    handles: Vec<PHYSICAL_MONITOR>,
    /// Maximum jasu monitora (pre prepocet na percenta).
    max: u32,
    last_write: Option<Instant>,
}

impl Phys {
    /// Precita jas; DDC obcas neodpovie (monitor je zaneprazdneny), preto az 3 pokusy s odstupom.
    fn read(&mut self) -> Option<u32> {
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(60));
            }
            if let Some(v) = self.read_once() {
                return Some(v);
            }
        }
        None
    }

    fn read_once(&mut self) -> Option<u32> {
        for h in &self.handles {
            let (mut cur, mut max) = (0u32, 0u32);
            // SAFETY: platny handle fyzickeho monitora; vystupne ukazovatele na lokalne premenne.
            let ok = unsafe {
                GetVCPFeatureAndVCPFeatureReply(
                    h.hPhysicalMonitor,
                    VCP_BRIGHTNESS,
                    None,
                    &mut cur,
                    Some(&mut max),
                )
            } != 0;
            if ok && max > 0 {
                self.max = max;
                return Some(to_percent(cur, max));
            }
        }
        None
    }

    fn write(&mut self, percent: u32) -> bool {
        if self.max == 0 {
            return false;
        }
        if let Some(t) = self.last_write {
            let since = t.elapsed();
            if since < WRITE_GAP {
                std::thread::sleep(WRITE_GAP - since);
            }
        }
        let raw = from_percent(percent, self.max);
        let mut ok = false;
        for h in &self.handles {
            // SAFETY: platny handle fyzickeho monitora.
            ok |= unsafe { SetVCPFeature(h.hPhysicalMonitor, VCP_BRIGHTNESS, raw) } != 0;
        }
        self.last_write = Some(Instant::now());
        ok
    }
}

impl Drop for Phys {
    fn drop(&mut self) {
        if !self.handles.is_empty() {
            // SAFETY: handly z GetPhysicalMonitorsFromHMONITOR, uvolnia sa raz.
            let _ = unsafe { DestroyPhysicalMonitors(&self.handles) };
        }
    }
}

pub fn to_percent(cur: u32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    ((u64::from(cur.min(max)) * 100 + u64::from(max) / 2) / u64::from(max)) as u32
}

pub fn from_percent(p: u32, max: u32) -> u32 {
    ((u64::from(p.min(100)) * u64::from(max) + 50) / 100) as u32
}

fn open(m: &MonitorInfo) -> Phys {
    let r = RECT {
        left: m.rect.left,
        top: m.rect.top,
        right: m.rect.right,
        bottom: m.rect.bottom,
    };
    let mut handles = Vec::new();
    // SAFETY: HMONITOR z obdlznika monitora; pole ma presnu velkost podla poctu.
    unsafe {
        let hm = MonitorFromRect(&r, MONITOR_DEFAULTTONULL);
        if !hm.is_invalid() {
            let mut n = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hm, &mut n).is_ok() && n > 0 && n < 16 {
                let mut arr = vec![PHYSICAL_MONITOR::default(); n as usize];
                if GetPhysicalMonitorsFromHMONITOR(hm, &mut arr).is_ok() {
                    handles = arr;
                }
            }
        }
    }
    let desc = handles
        .first()
        .map(|h| {
            let d = { h.szPhysicalMonitorDescription };
            let end = d.iter().position(|c| *c == 0).unwrap_or(d.len());
            String::from_utf16_lossy(&d[..end])
        })
        .unwrap_or_default();
    Phys {
        key: m.id.clone(),
        name: if m.name.is_empty() {
            desc
        } else {
            m.name.clone()
        },
        primary: m.primary,
        handles,
        max: 0,
        last_write: None,
    }
}

fn worker(rx: Receiver<Msg>, snap: Arc<Mutex<Arc<Snapshot>>>, notify: Arc<dyn Fn() + Send + Sync>) {
    let mut mons: Vec<Phys> = Vec::new();
    let mut levels: Vec<Option<u32>> = Vec::new();
    let mut gen = 0u64;
    let mut failed: Option<String>;
    let mut publish = |mons: &[Phys], levels: &[Option<u32>], failed: &Option<String>| {
        gen += 1;
        let s = Snapshot {
            ready: true,
            displays: mons
                .iter()
                .zip(levels)
                .map(|(m, l)| Display {
                    key: m.key.clone(),
                    name: m.name.clone(),
                    level: *l,
                    primary: m.primary,
                })
                .collect(),
            failed: failed.clone(),
            generation: gen,
        };
        if let Ok(mut g) = snap.lock() {
            *g = Arc::new(s);
        }
        notify();
    };
    while let Ok(m) = rx.recv() {
        // zlucit cakajuce spravy: posledna hodnota pre kazdy monitor, obnovenie raz
        let mut refresh = false;
        let mut sets: Vec<(String, u32)> = Vec::new();
        let mut quit = false;
        let mut take = |m: Msg, refresh: &mut bool, quit: &mut bool| match m {
            Msg::Refresh => *refresh = true,
            Msg::Quit => *quit = true,
            Msg::Set(k, v) => {
                sets.retain(|(kk, _)| *kk != k);
                sets.push((k, v));
            }
        };
        take(m, &mut refresh, &mut quit);
        loop {
            match rx.recv_timeout(Duration::from_millis(15)) {
                Ok(m) => take(m, &mut refresh, &mut quit),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            break;
        }
        if refresh {
            mons = monitor::enumerate().iter().map(open).collect();
            levels = mons.iter_mut().map(Phys::read).collect();
            failed = None;
            publish(&mons, &levels, &failed);
        }
        for (k, v) in sets {
            let Some(i) = mons.iter().position(|m| m.key == k) else {
                continue;
            };
            let ok = mons[i].write(v);
            // spatne citanie: ukazat, co monitor naozaj hlasi
            let back = mons[i].read();
            levels[i] = back.or(levels[i]);
            failed = (!ok || back.is_none()).then(|| k.clone());
            publish(&mons, &levels, &failed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_convert_both_ways_without_drift() {
        assert_eq!(to_percent(50, 100), 50);
        assert_eq!(to_percent(255, 255), 100);
        assert_eq!(to_percent(0, 0), 0);
        assert_eq!(to_percent(300, 100), 100);
        for max in [100, 255, 75] {
            for p in 0..=100 {
                let raw = from_percent(p, max);
                assert!(raw <= max);
                // pri max 100 je prevod presny; inak sa nestrati viac nez 1 %
                assert!(to_percent(raw, max).abs_diff(p) <= 1, "{p} {max}");
            }
        }
    }
}
