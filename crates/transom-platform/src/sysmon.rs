//! Vytazenie systemu pre polozku lisaty: CPU (`GetSystemTimes`), pamat (`GlobalMemoryStatusEx`), GPU (PDH,
//! anglicke nazvy pocitadiel, ako Spravca uloh: sucet procesov na kazdom jadre GPU, zobrazi sa najvytazenejsie) a siet
//! (`GetIfTable2`, len hardverove rozhrania bez filtrov, aby sa prenos nepocital dvakrat).
//!
//! Vsetko su rozdiely medzi dvoma meraniami, preto prve meranie CPU, GPU a siete este nema hodnotu. Teploty sa necitaju:
//! Windows na ne nema dokumentovane rozhranie bez ovladaca alebo administratorskych prav.
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

/// Jedno meranie.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    pub cpu: Option<u32>,
    pub ram: Option<u32>,
    pub gpu: Option<u32>,
    /// Bajty za sekundu (prijate, odoslane).
    pub down: Option<u64>,
    pub up: Option<u64>,
}

fn ft(f: FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

/// Percento CPU z rozdielov casov (kernel obsahuje necinnost).
pub fn cpu_percent(prev: (u64, u64, u64), now: (u64, u64, u64)) -> Option<u32> {
    let idle = now.0.checked_sub(prev.0)?;
    let busy_total = (now.1.checked_sub(prev.1)?) + (now.2.checked_sub(prev.2)?);
    if busy_total == 0 {
        return None;
    }
    let used = busy_total.saturating_sub(idle);
    Some(((used * 100 + busy_total / 2) / busy_total).min(100) as u32)
}

/// Vytazenie GPU z instancii pocitadla "GPU Engine": sucet procesov pre kazde jadro (cast nazvu od "luid_"), maximum.
pub fn gpu_percent(items: &[(String, f64)]) -> Option<u32> {
    if items.is_empty() {
        return None;
    }
    let mut engines: Vec<(String, f64)> = Vec::new();
    for (name, v) in items {
        let key = match name.find("luid_") {
            Some(i) => &name[i..],
            None => name.as_str(),
        };
        if !v.is_finite() || *v < 0.0 {
            continue;
        }
        match engines.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 += v,
            None => engines.push((key.to_string(), *v)),
        }
    }
    let max = engines.iter().map(|e| e.1).fold(0.0, f64::max);
    Some(max.round().clamp(0.0, 100.0) as u32)
}

struct Gpu {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

impl Gpu {
    fn open() -> Option<Gpu> {
        let mut query = PDH_HQUERY::default();
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: PDH dotaz a pocitadlo s anglickym nazvom (nezavisle od jazyka Windows); zatvori sa v Drop.
        unsafe {
            if PdhOpenQueryW(None, 0, &mut query) != 0 {
                return None;
            }
            if PdhAddEnglishCounterW(
                query,
                w!("\\GPU Engine(*)\\Utilization Percentage"),
                0,
                &mut counter,
            ) != 0
            {
                let _ = PdhCloseQuery(query);
                return None;
            }
            let _ = PdhCollectQueryData(query);
        }
        Some(Gpu { query, counter })
    }

    fn read(&mut self) -> Option<u32> {
        // SAFETY: dvojfazove citanie pola (velkost, potom data) do bufferu zarovnaneho na polozky.
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let (mut size, mut count) = (0u32, 0u32);
            let _ = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                None,
            );
            if size == 0 || size > 16 * 1024 * 1024 {
                return None;
            }
            let item = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let mut buf: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> =
                vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); (size as usize).div_ceil(item) + 1];
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(buf.as_mut_ptr()),
            ) != 0
            {
                return None;
            }
            let items: Vec<(String, f64)> = buf[..count as usize]
                .iter()
                .filter(|i| i.FmtValue.CStatus == 0)
                .map(|i| {
                    (
                        i.szName.to_string().unwrap_or_default(),
                        i.FmtValue.Anonymous.doubleValue,
                    )
                })
                .collect();
            gpu_percent(&items)
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: dotaz z PdhOpenQueryW, zatvori sa raz.
        let _ = unsafe { PdhCloseQuery(self.query) };
    }
}

fn net_totals() -> Option<(u64, u64)> {
    let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: tabulku alokuje system, uvolni sa cez FreeMibTable; riadky sa citaju v ramci NumEntries.
    unsafe {
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return None;
        }
        let n = (*t).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), n);
        let (mut i, mut o) = (0u64, 0u64);
        for r in rows {
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            let hardware = flags & 0x1 != 0;
            let filter = flags & 0x2 != 0;
            // 24 = softverova slucka; 1 = rozhranie je zapnute
            if hardware && !filter && r.Type != 24 && r.OperStatus.0 == 1 {
                i += r.InOctets;
                o += r.OutOctets;
            }
        }
        FreeMibTable(t as _);
        Some((i, o))
    }
}

/// Meria v case: kazde volanie `sample` porovna s predchadzajucim.
pub struct Sampler {
    cpu: Option<(u64, u64, u64)>,
    net: Option<(u64, u64, Instant)>,
    gpu: Option<Gpu>,
    gpu_tried: bool,
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler {
            cpu: None,
            net: None,
            gpu: None,
            gpu_tried: false,
        }
    }

    pub fn sample(&mut self, want_gpu: bool, want_net: bool) -> Sample {
        let mut s = Sample::default();
        let (mut idle, mut kernel, mut user) = Default::default();
        // SAFETY: vystupne struktury na zasobniku.
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_ok() {
            let now = (ft(idle), ft(kernel), ft(user));
            if let Some(p) = self.cpu {
                s.cpu = cpu_percent(p, now);
            }
            self.cpu = Some(now);
        }
        let mut m = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: struktura s nastavenou dlzkou.
        if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
            s.ram = Some(m.dwMemoryLoad.min(100));
        }
        if want_gpu {
            if self.gpu.is_none() && !self.gpu_tried {
                self.gpu_tried = true;
                self.gpu = Gpu::open();
            }
            if let Some(g) = &mut self.gpu {
                s.gpu = g.read();
            }
        } else {
            self.gpu = None;
            self.gpu_tried = false;
        }
        if want_net {
            if let Some((i, o)) = net_totals() {
                let now = Instant::now();
                if let Some((pi, po, pt)) = self.net {
                    let dt = now.duration_since(pt).as_secs_f64();
                    if dt > 0.05 {
                        s.down = Some((i.saturating_sub(pi) as f64 / dt) as u64);
                        s.up = Some((o.saturating_sub(po) as f64 / dt) as u64);
                    }
                }
                self.net = Some((i, o, now));
            }
        } else {
            self.net = None;
        }
        s
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler::new()
    }
}

/// Co sa ma merat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub gpu: bool,
    pub net: bool,
    pub interval: Duration,
}

/// Merac na vlastnom vlakne (PDH moze trvat desiatky ms): meria v intervale, kym zije.
pub struct Monitor {
    tx: Sender<Want>,
    last: Arc<Mutex<Sample>>,
}

impl Monitor {
    pub fn start(want: Want, notify: Arc<dyn Fn() + Send + Sync>) -> Result<Monitor, String> {
        let (tx, rx) = channel::<Want>();
        let last = Arc::new(Mutex::new(Sample::default()));
        let l2 = Arc::clone(&last);
        std::thread::Builder::new()
            .name("transom-sysmon".into())
            .spawn(move || {
                let mut sm = Sampler::new();
                let mut want = want;
                loop {
                    let s = sm.sample(want.gpu, want.net);
                    if let Ok(mut g) = l2.lock() {
                        if *g != s {
                            *g = s;
                            drop(g);
                            notify();
                        }
                    }
                    match rx.recv_timeout(want.interval) {
                        Ok(w) => want = w,
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Monitor { tx, last })
    }

    pub fn set(&self, want: Want) {
        let _ = self.tx.send(want);
    }

    pub fn last(&self) -> Sample {
        self.last.lock().map(|g| *g).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_and_gpu_percentages_come_from_differences_and_engines() {
        // necinnost 600 z 1000 (kernel obsahuje necinnost) = 40 %
        assert_eq!(cpu_percent((0, 0, 0), (600, 800, 200)), Some(40));
        assert_eq!(cpu_percent((5, 5, 5), (5, 5, 5)), None);
        assert_eq!(
            cpu_percent((10, 0, 0), (5, 1, 1)),
            None,
            "pretecenie pocitadla"
        );
        let items = vec![
            ("pid_1_luid_0x1_phys_0_eng_0_engtype_3D".to_string(), 30.0),
            ("pid_2_luid_0x1_phys_0_eng_0_engtype_3D".to_string(), 25.5),
            (
                "pid_2_luid_0x1_phys_0_eng_4_engtype_VideoDecode".to_string(),
                70.0,
            ),
            (
                "pid_3_luid_0x1_phys_0_eng_0_engtype_3D".to_string(),
                f64::NAN,
            ),
        ];
        // jadro 3D: 30 + 25,5; dekodovanie videa 70 -> maximum 70
        assert_eq!(gpu_percent(&items), Some(70));
        assert_eq!(gpu_percent(&items[..2]), Some(56));
        assert_eq!(gpu_percent(&[]), None);
        assert_eq!(gpu_percent(&[("x".into(), 250.0)]), Some(100));
    }

    #[test]
    fn a_real_sample_has_memory_and_after_a_moment_cpu() {
        let mut s = Sampler::new();
        let a = s.sample(false, true);
        assert!(a.ram.is_some_and(|r| r <= 100));
        assert_eq!(a.cpu, None, "prve meranie nema rozdiel");
        std::thread::sleep(Duration::from_millis(120));
        let b = s.sample(false, true);
        assert!(b.cpu.is_some_and(|c| c <= 100), "{b:?}");
        assert!(b.down.is_some() && b.up.is_some(), "{b:?}");
    }
}
