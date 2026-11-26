//! Win32 vrstva transom: monitory + DPI (`monitor`), AppBar hostitel (`appbar`), napajanie (`power`),
//! simulacia skratiek (`input`), procesy (`process`), hodiny a systemove nastavenia.
//! Poznatky zo spikov S1-S3 su zapisane pri jednotlivych funkciach.
#![cfg(windows)]

pub mod access;
pub mod activate;
pub mod appbar;
pub mod autostart;
pub mod brightness;
pub mod clipboard;
pub mod clock;
pub mod desktops;
pub mod dirwatch;
pub mod dragdrop;
pub mod filepick;
pub mod frametimer;
pub mod fullscreen;
pub mod geoloc;
pub mod glass;
pub mod hotkey;
pub mod http;
pub mod icons;
pub mod imaging;
pub mod input;
pub mod keyboard;
pub mod layered;
pub mod menu_access;
pub mod monitor;
pub mod picker;
pub mod popup;
pub mod power;
pub mod process;
pub mod regions;
pub mod selfupdate;
pub mod shellicon;
pub mod shellops;
pub mod spectrum;
pub mod sysmon;
pub mod system;
pub mod thumb;
pub mod tray;
mod tray_model;
pub mod vfiles;
pub mod vsync;

pub use frametimer::FrameTimer;

/// Posle vsetkym oknam registrovanu spravu `name` (`wparam` ako cislo). Pouziva sa na "znova zobraz listu" a "prepni stranku
/// nastaveni" (druhe spustenie programu najde uz bezaciu instanciu).
pub fn post_broadcast(name: &str, wparam: usize) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW};
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: registracia spravy a odoslanie broadcastu (HWND_BROADCAST = 0xFFFF).
    unsafe {
        let msg = RegisterWindowMessageW(PCWSTR(wide.as_ptr()));
        let _ = PostMessageW(Some(HWND(0xffff as *mut _)), msg, WPARAM(wparam), LPARAM(0));
    }
}

pub use appbar::{
    cancel_timer, invalidate_all, invalidate_span, is_hidden, notifier, register_broadcast,
    schedule_timer, schedule_wake, set_tray, BarConfig, BarEvent, BarHandler, BarHost, Notifier,
    PaintCtx, PointerKind, TrayKind,
};
pub use layered::BarAppearance;
pub use monitor::{MonitorInfo, MonitorSelection, Rect};
pub use tray::TraySpec;

/// Nastavi Per-Monitor V2 DPI awareness procesu. Zavolat pred vytvorenim akehokolvek okna.
pub fn init_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    // Zlyha, ak je awareness uz nastavena (napr. manifestom) - to je v poriadku.
    // SAFETY: nastavenie DPI procesu pred vytvorenim okien; chyba (uz nastavene) je neskodna.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}
