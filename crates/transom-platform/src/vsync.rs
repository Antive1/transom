//! Snimky animacii zladene s obrazovkou: vlakno caka na dalsie zlozenie DWM (`DwmFlush`, teda obnovovaciu frekvenciu
//! monitora - 60, 144 aj 240 Hz) a posle oknu `WM_TIMER` s danym id. Novu spravu posle az ked okno predchadzajucu
//! spracovalo (`ack`), takze pomale kreslenie nezahlti frontu. Bezi len pocas animacie (zahodenim sa zastavi).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_TIMER};

pub struct Pacer {
    stop: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Pacer {
    /// Zacne posielat `WM_TIMER(id)` oknu `hwnd` pri kazdom obnoveni obrazovky.
    pub fn start(hwnd: HWND, id: usize) -> Pacer {
        let stop = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(AtomicBool::new(false));
        let (s, p, h) = (stop.clone(), pending.clone(), hwnd.0 as isize);
        let thread = std::thread::Builder::new()
            .name("transom-vsync".into())
            .spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    // SAFETY: bez parametrov; blokuje do dalsieho zlozenia obrazovky.
                    let ok = unsafe { windows::Win32::Graphics::Dwm::DwmFlush() }.is_ok();
                    if !ok {
                        // bez DWM (napr. vzdialena plocha bez zlozenia): ~120 snimok za sekundu
                        std::thread::sleep(std::time::Duration::from_millis(8));
                    }
                    if s.load(Ordering::Relaxed) {
                        break;
                    }
                    if !p.swap(true, Ordering::AcqRel) {
                        // SAFETY: odoslanie spravy oknu (neplatne okno = chyba, ziadny pad).
                        unsafe {
                            let _ =
                                PostMessageW(Some(HWND(h as _)), WM_TIMER, WPARAM(id), LPARAM(0));
                        }
                    }
                }
            })
            .ok();
        Pacer {
            stop,
            pending,
            thread,
        }
    }

    /// Okno spracovalo snimku: dalsia moze prist.
    pub fn ack(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

impl Drop for Pacer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // vlakno skonci po najblizsom zlozeni (do jednej snimky); UI vlakno necaka
        if let Some(t) = self.thread.take() {
            std::thread::spawn(move || {
                let _ = t.join();
            });
        }
    }
}
