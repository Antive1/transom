//! Mixer aplikacii pre transom: hlasitost, stlmenie a vystup kazdej aplikacie (aj systemovych zvukov).
//!
//! * `model`: cisty model (zoskupenie relacii do aplikacii, mena, trasy) bez Win32.
//! * `service`: vlastne vlakno, fronta udalosti a prikazov, zlucovanie serii udalosti (bez pollingu).
//! * `core` (Windows): Core Audio relacie na vsetkych vystupoch a nedokumentovane rozhranie politiky vystupu
//!   (to iste ako EarTrumpet). Toto je zamerne oddelene od `transom-audio`, ktore taketo rozhrania nepouziva.
//! * `capture` (Windows): kto pouziva mikrofon a kameru, stlmenie predvoleneho mikrofonu.
//!
//! Hlasitost aplikacie je v rozsahu 0-100 % (strop relacie Windows); predvolena hodnota sa nikdy nezapisuje.
#[cfg(windows)]
pub mod bt;
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod core;
pub mod model;
pub mod service;

pub use model::{App, OutDevice, Route, Snapshot};
pub use service::Service;

/// Spusti sluzbu so skutocnym Core Audio. Sluzba bezi len kym zije `Service` (panel Zvuk je otvoreny).
#[cfg(windows)]
pub fn start_system(
    notify: std::sync::Arc<dyn Fn() + Send + Sync>,
    system_name: String,
) -> Result<Service, String> {
    Service::start(
        |tx| Box::new(core::CoreBackend::new(tx)),
        notify,
        system_name,
        std::time::Duration::from_millis(60),
    )
}

#[cfg(test)]
mod tests {
    /// Mixer meni len relacie aplikacii a ich vystup: nikdy hlavnu hlasitost ani predvoleny vystup systemu.
    #[test]
    fn the_mixer_never_touches_the_master_volume_or_the_system_default_output() {
        let code: String = include_str!("core.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for bad in [
            ["IAudioEndpoint", "Volume"].concat(),
            ["IPolicy", "Config"].concat(),
            ["SetDefault", "Endpoint"].concat(),
            ["ClearAll", "Persisted"].concat(),
            ["clear", "_all("].concat(),
        ] {
            assert!(!code.contains(&bad), "core.rs pouziva {bad}");
        }
    }
}
