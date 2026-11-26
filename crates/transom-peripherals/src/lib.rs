//! Batérie periférií pre transom: mysi, klavesnice, sluchadla a ovladace, ktore hlasia svoju uroven.
//!
//! * `model`: cisty model (citania, posledne hodnoty, spiace zariadenia, vyber na listu) bez Win32.
//! * `service`: vlastne vlakno, kola citania, zlucovanie poziadaviek na obnovu.
//! * zdroje: `bluetooth` (uroven, ktoru hlasi Windows), `razer`, `logitech` (HID++ 2.0), `nuphy`; vrstva `hid`.
//!
//! Vsetko je **len na citanie**: posielaju sa iba citacie prikazy (uroven, nabijanie, nazov). Ziadne nastavenia
//! zariadeni, osvetlenie, firmver ani parovanie. Nic sa neodhaduje: ked zariadenie uroven nehlasi, ukaze sa bez nej.
pub mod bluetooth;
pub mod generic;
#[cfg(windows)]
pub mod hid;
pub mod logitech;
pub mod model;
pub mod nuphy;
pub mod razer;
pub mod service;

pub use model::{
    for_bar, kind_from_name, short_label, Availability, Kind, Peripheral, Presence, Reading,
    Snapshot, Source, Tracker,
};
pub use service::{Provider, Service, Tuning};
#[cfg(windows)]
pub use system::providers as system_providers;

#[cfg(windows)]
mod system {
    use super::*;

    macro_rules! provider {
        ($ty:ty, $src:expr) => {
            impl Provider for $ty {
                fn source(&self) -> Source {
                    $src
                }
                fn poll(&mut self) -> Vec<Reading> {
                    <$ty>::poll(self)
                }
            }
        };
    }
    provider!(bluetooth::Provider, Source::Bluetooth);
    provider!(razer::Provider, Source::Razer);
    provider!(logitech::Provider, Source::Logitech);
    provider!(nuphy::Provider, Source::NuPhy);
    provider!(generic::Provider, Source::Hid);

    pub fn providers(sources: &[Source]) -> Vec<Box<dyn Provider>> {
        sources
            .iter()
            .map(|s| -> Box<dyn Provider> {
                match s {
                    Source::Bluetooth => Box::new(bluetooth::Provider),
                    Source::Razer => Box::<razer::Provider>::default(),
                    Source::Logitech => Box::<logitech::Provider>::default(),
                    Source::NuPhy => Box::<nuphy::Provider>::default(),
                    Source::Hid => Box::<generic::Provider>::default(),
                }
            })
            .collect()
    }
}

/// Spusti sluzbu so skutocnymi zdrojmi (`sources`), kolo kazdych `interval`. `notify` sa vola z vlakna sluzby.
#[cfg(windows)]
pub fn start_system(
    sources: Vec<Source>,
    interval: std::time::Duration,
    notify: std::sync::Arc<dyn Fn() + Send + Sync>,
) -> Result<Service, String> {
    Service::start(
        move || system::providers(&sources),
        notify,
        Tuning {
            interval,
            ..Tuning::default()
        },
    )
}

#[cfg(test)]
mod tests {
    /// Crate posiela len citacie prikazy: ziadne zapisove prikazy znamych protokolov.
    #[test]
    fn only_read_commands_are_ever_sent() {
        for (name, src) in [
            ("razer.rs", include_str!("razer.rs")),
            ("nuphy.rs", include_str!("nuphy.rs")),
            ("logitech.rs", include_str!("logitech.rs")),
        ] {
            let code = src.split("#[cfg(test)]").next().unwrap_or("");
            // Razer: jedine triedy/prikazy su 0x07 0x80 / 0x84
            if name == "razer.rs" {
                assert!(
                    code.contains("CMD_BATTERY: u8 = 0x80")
                        && code.contains("CMD_CHARGING: u8 = 0x84")
                );
            }
            // NuPhy: SetFunc (0x06) ani ine zapisove prikazy sa nikdy nepouziju
            if name == "nuphy.rs" {
                assert!(!code.contains("0x06"), "{name}");
            }
            for bad in ["SetFunc", "set_dpi", "firmware_write", "SetFeatureValue"] {
                assert!(!code.contains(bad), "{name} obsahuje {bad}");
            }
        }
    }
}
