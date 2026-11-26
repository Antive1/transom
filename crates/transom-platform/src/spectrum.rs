//! Spektrum prehravaneho zvuku pre ekvalizer v liste: zachytenie vystupu (WASAPI loopback na predvolenom vystupe,
//! zdielany rezim) na samostatnom vlakne a 5 pasiem (FFT 1024 vzoriek). Zvuk sa nikam neuklada ani neposiela -
//! z kazdeho bloku ostane len 5 cisel (0..=255). Bezi len kym existuje `Spectrum` (ekvalizer je viditelny).
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Pocet pasiem a ich hranice (Hz).
pub const BANDS: usize = 5;
const EDGES: [f32; BANDS + 1] = [50.0, 160.0, 420.0, 1100.0, 3000.0, 9000.0];
const N: usize = 1024;

pub struct Spectrum {
    levels: Arc<[AtomicU8; BANDS]>,
    /// Na vystupe nieco hra (prisli data za poslednych ~50 ms).
    active: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Spectrum {
    /// Spusti zachytavanie; `on_frame` sa zavola po kazdom novom vypocte (~ kazdych 10-20 ms, len ked hra zvuk).
    pub fn start(on_frame: impl Fn() + Send + 'static) -> Spectrum {
        let levels: Arc<[AtomicU8; BANDS]> = Arc::new(Default::default());
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(false));
        let (l, s, a) = (levels.clone(), stop.clone(), active.clone());
        let thread = std::thread::Builder::new()
            .name("transom-spectrum".into())
            .spawn(move || {
                // zariadenie sa moze zmenit / zlyhat: znova o chvilu, kym sa nezastavi
                while !s.load(Ordering::Relaxed) {
                    let _ = capture(&l, &s, &a, &on_frame);
                    for v in l.iter() {
                        v.store(0, Ordering::Relaxed);
                    }
                    a.store(false, Ordering::Relaxed);
                    on_frame();
                    std::thread::sleep(Duration::from_millis(800));
                }
            })
            .ok();
        Spectrum {
            levels,
            active,
            stop,
            thread,
        }
    }

    /// Na predvolenom vystupe prave hra zvuk (inak hudba ide inam, napr. iny vystup alebo vzdialene zariadenie).
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Aktualne urovne pasiem (0..=255).
    pub fn levels(&self) -> [u8; BANDS] {
        std::array::from_fn(|i| self.levels[i].load(Ordering::Relaxed))
    }
}

impl Drop for Spectrum {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // vlakno skonci do ~20 ms (cakanie na data je kratke); UI necaka
        if let Some(t) = self.thread.take() {
            std::thread::spawn(move || {
                let _ = t.join();
            });
        }
    }
}

/// Pasma z blokov vzoriek (mono, f32): uroven 0..=1 na pasmo (logaritmicky, -66..-6 dB).
pub fn bands(samples: &[f32], rate: f32) -> [f32; BANDS] {
    let n = samples.len().next_power_of_two().clamp(2, N);
    // buffre v zasobniku (vola sa ~100x za sekundu, kym hra hudba)
    let (mut re_buf, mut im_buf) = ([0.0f32; N], [0.0f32; N]);
    let (re, im) = (&mut re_buf[..n], &mut im_buf[..n]);
    let full = (n == N).then(hann);
    for (i, v) in samples.iter().take(n).enumerate() {
        // Hannovo okno (pre plny blok z tabulky; rovnake hodnoty)
        let w = match full {
            Some(t) => t[i],
            None => hann_at(i, n),
        };
        re[i] = v * w;
    }
    fft(re, im);
    let bin = rate / n as f32;
    std::array::from_fn(|b| {
        let (lo, hi) = (
            (EDGES[b] / bin).floor().max(1.0) as usize,
            ((EDGES[b + 1] / bin).ceil() as usize).min(n / 2),
        );
        if hi <= lo {
            return 0.0;
        }
        let e: f32 = (lo..hi)
            .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
            .fold(0.0, f32::max);
        // amplituda vzhladom na plny sinus v okne (n/4)
        let db = 20.0 * (e / (n as f32 / 4.0)).max(1e-6).log10();
        ((db + 66.0) / 60.0).clamp(0.0, 1.0)
    })
}

fn hann_at(i: usize, n: usize) -> f32 {
    0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos()
}

/// Hannovo okno pre blok `N` vzoriek (raz za proces).
fn hann() -> &'static [f32; N] {
    static T: std::sync::OnceLock<[f32; N]> = std::sync::OnceLock::new();
    T.get_or_init(|| std::array::from_fn(|i| hann_at(i, N)))
}

/// Otocne faktory FFT `(sin, cos)` uhla `-2 pi k / len` pre kazdy stupen `len = 2, 4, .. N` a `k < len / 2`, za sebou
/// (stupen `len` zacina na indexe `len / 2 - 1`). Pocitane presne tak ako predtym v slucke FFT (vysledok sa nemeni),
/// len raz za proces namiesto ~5 000 `sin_cos` pri kazdom bloku.
fn twiddles() -> &'static [(f32, f32)] {
    static T: std::sync::OnceLock<Vec<(f32, f32)>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Vec::with_capacity(N);
        let mut len = 2;
        while len <= N {
            let a = -2.0 * std::f32::consts::PI / len as f32;
            t.extend((0..len / 2).map(|k| (a * k as f32).sin_cos()));
            len <<= 1;
        }
        t
    })
}

/// Radix-2 FFT na mieste (dlzka mocnina 2, najviac `N`).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let tw = twiddles();
    let mut len = 2;
    while len <= n {
        let stage = &tw[len / 2 - 1..len - 1];
        for start in (0..n).step_by(len) {
            for (k, &(s, c)) in stage.iter().enumerate() {
                let (i0, i1) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[i1] * c - im[i1] * s, re[i1] * s + im[i1] * c);
                re[i1] = re[i0] - tr;
                im[i1] = im[i0] - ti;
                re[i0] += tr;
                im[i0] += ti;
            }
        }
        len <<= 1;
    }
}

fn capture(
    levels: &[AtomicU8; BANDS],
    stop: &AtomicBool,
    active: &AtomicBool,
    on_frame: &dyn Fn(),
) -> windows::core::Result<()> {
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    };
    // SAFETY: COM na tomto vlakne (parovane s CoUninitialize); format a buffre zije cez ich pouzitie; GetBuffer /
    // ReleaseBuffer v paroch.
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let r = (|| -> windows::core::Result<()> {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
            let fmt = client.GetMixFormat()?;
            let f: WAVEFORMATEX = *fmt;
            let (ch, rate, bits, tag) = (
                f.nChannels.max(1) as usize,
                f.nSamplesPerSec as f32,
                f.wBitsPerSample,
                f.wFormatTag,
            );
            let init = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                1_000_000,
                0,
                fmt,
                None,
            );
            CoTaskMemFree(Some(fmt as *const _));
            init?;
            // 32 bit = float (zdielany format zmiesavaca), 16 bit = cele cisla; ine formaty sa nepouzivaju
            let float = bits == 32 && (tag == 3 || tag == 0xFFFE);
            if !(float || bits == 16) {
                return Ok(());
            }
            let cap: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            let mut ring: Vec<f32> = Vec::with_capacity(N * 2);
            let mut smooth = [0.0f32; BANDS];
            let mut quiet = 0u32;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                let mut got = false;
                while cap.GetNextPacketSize()? > 0 {
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0u32;
                    let mut flags = 0u32;
                    cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                    let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                    for i in 0..frames as usize {
                        let mut v = 0.0f32;
                        if !silent && !data.is_null() {
                            for c in 0..ch {
                                v += if float {
                                    *(data as *const f32).add(i * ch + c)
                                } else {
                                    *(data as *const i16).add(i * ch + c) as f32 / 32768.0
                                };
                            }
                        }
                        ring.push(v / ch as f32);
                    }
                    cap.ReleaseBuffer(frames)?;
                    got = true;
                }
                if ring.len() > N {
                    ring.drain(..ring.len() - N);
                }
                // nic nehra (loopback neposiela data): stlpce plynulo klesnu
                quiet = if got { 0 } else { quiet + 1 };
                if active.swap(quiet < 5, Ordering::Relaxed) != (quiet < 5) {
                    on_frame();
                }
                let target = if ring.len() == N && quiet < 5 {
                    bands(&ring, rate)
                } else {
                    [0.0; BANDS]
                };
                let mut changed = false;
                for b in 0..BANDS {
                    // rychly nabeh, pomaly pokles (ako skutocny ekvalizer)
                    let k = if target[b] > smooth[b] { 0.6 } else { 0.18 };
                    smooth[b] += (target[b] - smooth[b]) * k;
                    let v = (smooth[b] * 255.0).round() as u8;
                    changed |= levels[b].swap(v, Ordering::Relaxed) != v;
                }
                if changed {
                    on_frame();
                }
            }
            let _ = client.Stop();
            Ok(())
        })();
        if com {
            CoUninitialize();
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32) -> Vec<f32> {
        (0..N)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / 48_000.0).sin())
            .collect()
    }

    /// Povodna FFT (uhly v slucke): tabulkova musi davat bit po bite to iste.
    fn fft_reference(re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let a = -2.0 * std::f32::consts::PI / len as f32;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (s, c) = (a * k as f32).sin_cos();
                    let (i0, i1) = (start + k, start + k + len / 2);
                    let (tr, ti) = (re[i1] * c - im[i1] * s, re[i1] * s + im[i1] * c);
                    re[i1] = re[i0] - tr;
                    im[i1] = im[i0] - ti;
                    re[i0] += tr;
                    im[i0] += ti;
                }
            }
            len <<= 1;
        }
    }

    #[test]
    fn the_table_driven_fft_and_window_give_exactly_the_old_results() {
        for n in [2usize, 8, 64, 256, N] {
            let sig: Vec<f32> = (0..n)
                .map(|i| ((i * 7919 % 1000) as f32 / 500.0 - 1.0) * 0.8)
                .collect();
            let (mut a_re, mut a_im) = (sig.clone(), vec![0.0f32; n]);
            let (mut b_re, mut b_im) = (sig.clone(), vec![0.0f32; n]);
            fft(&mut a_re, &mut a_im);
            fft_reference(&mut b_re, &mut b_im);
            assert!(
                a_re.iter()
                    .zip(&b_re)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
                    && a_im
                        .iter()
                        .zip(&b_im)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                "n={n}"
            );
        }
        for (i, w) in hann().iter().enumerate() {
            let old = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N as f32 - 1.0)).cos();
            assert_eq!(w.to_bits(), old.to_bits());
        }
    }

    #[test]
    fn a_tone_lights_its_band_and_silence_none() {
        let b = bands(&sine(100.0, 0.5), 48_000.0);
        assert!(b[0] > 0.7, "{b:?}");
        assert!(b[3] < b[0] && b[4] < b[0], "{b:?}");
        let b = bands(&sine(5000.0, 0.5), 48_000.0);
        assert!(b[4] > 0.7 && b[0] < b[4], "{b:?}");
        assert_eq!(bands(&[0.0; N], 48_000.0), [0.0; BANDS]);
    }

    #[test]
    #[ignore = "zivy test: zachytava predvoleny vystup (ticho alebo hudba)"]
    fn live_capture_runs_and_stops() {
        let s = Spectrum::start(|| {});
        let mut mx = [0u8; BANDS];
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(20));
            for (m, v) in mx.iter_mut().zip(s.levels()) {
                *m = (*m).max(v);
            }
        }
        eprintln!("max levels {mx:?} active={}", s.active());
        drop(s);
    }
}
