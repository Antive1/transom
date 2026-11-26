//! Presny casovac snimok pre plynuly pohyb (posuvny text): `WM_TIMER` (`SetTimer`) chodi v nasobkoch ~15,6 ms aj po
//! `timeBeginPeriod(1)` (namerane: pozadovanych 16 ms prislo ako 31 ms, 33 ms ako 47 ms; odchylka 7-18 ms), takze pohyb
//! "drhol". Preto samostatne vlakno caka na **vysokorozlisenom waitable casovaci** (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`,
//! Windows 10 1803+; inak obycajny waitable casovac) a po uplynuti posle hostitelovi `Notify(code)`.
//! Vlakno nepouziva CPU, kym nie je casovac nastaveny (blokuje sa v `WaitForMultipleObjects`); bez posuvu sa nenastavi nikdy.
use crate::Notifier;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CancelWaitableTimer, CreateEventW, CreateWaitableTimerExW, SetEvent, SetWaitableTimer,
    WaitForMultipleObjects, INFINITE, TIMER_ALL_ACCESS,
};

const CREATE_WAITABLE_TIMER_HIGH_RESOLUTION: u32 = 0x0000_0002;

pub struct FrameTimer {
    timer: isize,
    stop: isize,
    thread: Option<std::thread::JoinHandle<()>>,
    high_res: bool,
}

fn h(v: isize) -> HANDLE {
    HANDLE(v as *mut _)
}

impl FrameTimer {
    /// Vytvori casovac a vlakno; po uplynuti kazdeho nastavenia posle `notifier.notify(code)`. `None`, ak sa nepodarilo
    /// vytvorit objekty (volajuci potom pouzije `SetTimer`).
    pub fn new(notifier: Notifier, code: u32) -> Option<FrameTimer> {
        // SAFETY: vytvorenie vlastnych objektov; uchytky sa zatvaraju v `Drop`.
        unsafe {
            let (timer, high_res) = match CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            ) {
                Ok(t) => (t, true),
                Err(_) => (
                    CreateWaitableTimerExW(None, PCWSTR::null(), 0, TIMER_ALL_ACCESS.0).ok()?,
                    false,
                ),
            };
            let Ok(stop) = CreateEventW(None, true, false, PCWSTR::null()) else {
                let _ = CloseHandle(timer);
                return None;
            };
            let (t, s) = (timer.0 as isize, stop.0 as isize);
            let thread = std::thread::Builder::new()
                .name("transom-frame".into())
                .spawn(move || loop {
                    let r = WaitForMultipleObjects(&[h(t), h(s)], false, INFINITE);
                    if r == WAIT_OBJECT_0 {
                        notifier.notify(code);
                    } else {
                        break;
                    }
                })
                .ok()?;
            Some(FrameTimer {
                timer: t,
                stop: s,
                thread: Some(thread),
                high_res,
            })
        }
    }

    /// Vysoke rozlisenie (inak ~15,6 ms).
    pub fn is_high_res(&self) -> bool {
        self.high_res
    }

    /// Nastavi jednorazove prebudenie za `ms` (prepise predchadzajuce).
    pub fn arm(&self, ms: u32) {
        // relativny cas: zaporne cislo v jednotkach 100 ns
        let due = -(ms.max(1) as i64) * 10_000;
        // SAFETY: platny uchytok casovaca, `due` zije pocas volania.
        unsafe {
            let _ = SetWaitableTimer(h(self.timer), &due, 0, None, None, false);
        }
    }

    /// Zrusi cakajuce prebudenie.
    pub fn disarm(&self) {
        // SAFETY: platny uchytok casovaca.
        unsafe {
            let _ = CancelWaitableTimer(h(self.timer));
        }
    }
}

impl Drop for FrameTimer {
    fn drop(&mut self) {
        // SAFETY: zastavenie a zatvorenie vlastnych objektov; vlakno sa pred zatvorenim uchytkov pripoji.
        unsafe {
            let _ = SetEvent(h(self.stop));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // SAFETY: handly casovaca a udalosti vytvoril `new`, vlakno uz skoncilo (join), zatvaraju sa raz.
        unsafe {
            let _ = CloseHandle(h(self.timer));
            let _ = CloseHandle(h(self.stop));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Overuje len **vlakno a casovac** (bez okna hostitela): do `Notifier` sa neda posielat, tak sa meria cez priamy
    /// waitable casovac s rovnakymi parametrmi ako `FrameTimer`.
    #[test]
    fn high_resolution_waitable_timer_keeps_16_ms_cadence_better_than_wm_timer() {
        // SAFETY: lokalny casovac, zatvoreny na konci.
        unsafe {
            let Ok(t) = CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            ) else {
                return; // starsi Windows: nic na overenie
            };
            let mut worst = Duration::ZERO;
            let mut total = Duration::ZERO;
            let n = 30;
            for _ in 0..n {
                let start = Instant::now();
                let due = -(16i64 * 10_000);
                let _ = SetWaitableTimer(t, &due, 0, None, None, false);
                WaitForMultipleObjects(&[t], false, INFINITE);
                let dt = start.elapsed();
                total += dt;
                worst = worst.max(dt);
            }
            let _ = CloseHandle(t);
            let mean = total / n;
            // ~16 ms +- par ms (obycajny casovac by dal ~31 ms)
            assert!(
                mean < Duration::from_millis(22) && worst < Duration::from_millis(40),
                "priemer {mean:?} najhorsie {worst:?}"
            );
        }
    }
}
