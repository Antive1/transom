//! Systemovy vystup zvuku Windows pre transom: zoznam vystupov, predvoleny vystup, hlasitost a stlmenie.
//!
//! * `model`: cisty model (zariadenia, hlasitost, dostupnost, ciste nazvy) bez Win32.
//! * `service`: vlastne vlakno, fronta udalosti a prikazov, zlucovanie serii udalosti (bez pollingu).
//! * `coreaudio` (Windows): dokumentovane Core Audio (MMDevice + EndpointVolume) s notifikaciami.
//!
//! Toto je hlasitost **systemoveho vystupu** (hlavna hlasitost predvoleneho zariadenia). S crate `transom-sound`
//! (zvuky rozhrania, relacia transom) nema nic spolocne a nikdy ho nevola. Predvoleny vystup sa nemeni: Windows 11
//! na to nema podporovane verejne API, preto panel len otvara natívne povrchy Windows a zmenu potvrdi az notifikacia.
//! Mixer aplikacii (relacie, vystup pre aplikaciu cez nedokumentovane rozhranie) je zamerne v crate `transom-mixer`.
#[cfg(windows)]
pub mod coreaudio;
pub mod model;
pub mod service;

pub use model::{
    clean_name, display_devices, percent_of, scalar_of, AudioError, Availability, Controls, Device,
    DeviceState, RawDevice, Snapshot, Volume,
};
pub use service::{Backend, Counters, Event, Refresh, Service, Tuning};

/// Spusti sluzbu so skutocnym Core Audio. `notify` sa vola z vlakna sluzby po kazdej zmene snimky.
#[cfg(windows)]
pub fn start_system(notify: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<Service, String> {
    Service::start(
        |tx| Box::new(coreaudio::CoreAudioBackend::new(tx)),
        notify,
        Tuning::default(),
    )
}

#[cfg(test)]
mod tests {
    /// Tento crate nesmie siahat na zvukove relacie transom ani na streamy. Nedokumentovana zmena predvoleneho vystupu
    /// (`IPolicyConfig`) je povolena **len** v `coreaudio.rs` (vyslovna volba pouzivatela v paneli Zvuk).
    #[test]
    fn the_crate_never_uses_sessions_streams_or_undocumented_default_device_apis() {
        for (name, src) in [
            ("coreaudio.rs", include_str!("coreaudio.rs")),
            ("service.rs", include_str!("service.rs")),
            ("model.rs", include_str!("model.rs")),
        ] {
            let code: String = src
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            for bad in [
                ["ISimple", "AudioVolume"].concat(),
                ["IAudio", "Client"].concat(),
                ["IAudioSession", "Control"].concat(),
                ["IPolicy", "Config"].concat(),
                ["SetDefault", "Endpoint"].concat(),
                ["transom", "_sound"].concat(),
            ] {
                // testy v tomto subore vyssie nie su v skenovanych retazcoch, preto staci hladat vsade okrem testov
                let non_test = code.split("#[cfg(test)]").next().unwrap_or("");
                let policy = bad.starts_with("IPolicy") || bad.starts_with("SetDefault");
                if policy && name == "coreaudio.rs" {
                    continue;
                }
                assert!(!non_test.contains(&bad), "{name} pouziva {bad}");
            }
        }
    }
}
