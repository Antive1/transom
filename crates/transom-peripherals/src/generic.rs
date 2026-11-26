//! Standardna uroven baterie HID (bez protokolu vyrobcu): zariadenia, ktore v popise reportov hlasia
//! **Battery Strength** (Generic Device Controls 0x06 / 0x20) alebo **Absolute State Of Charge** (Battery System
//! 0x85 / 0x65). Hodnota sa cita z feature alebo input reportu a prepocita podla logickeho rozsahu z popisu.
//!
//! Len citanie: ziadne zapisy do zariadenia. Vynechane su zariadenia, ktore citaju presnejsie zdroje (Razer, Logitech,
//! NuPhy cez vlastny protokol; Bluetooth cez uroven, ktoru hlasi Windows), aby sa neukazovali dvakrat.
use crate::model::{kind_from_name, Kind};

/// Generic Device Controls / Battery Strength.
pub const PAGE_GENERIC_DEVICE: u16 = 0x06;
pub const USAGE_BATTERY_STRENGTH: u16 = 0x20;
/// Battery System / Absolute State Of Charge (percenta).
pub const PAGE_BATTERY_SYSTEM: u16 = 0x85;
pub const USAGE_ABSOLUTE_SOC: u16 = 0x65;

/// Vyrobcovia, ktorych zariadenia cita vlastny protokol (presnejsi, vratane nabijania).
pub const OWN_PROTOCOL_VIDS: [u16; 3] =
    [crate::razer::VID, crate::logitech::VID, crate::nuphy::VID];

/// Je to batériova hodnota, ktoru vieme citat ako percenta?
pub fn is_battery_usage(page: u16, usage: u16) -> bool {
    (page == PAGE_GENERIC_DEVICE && usage == USAGE_BATTERY_STRENGTH)
        || (page == PAGE_BATTERY_SYSTEM && usage == USAGE_ABSOLUTE_SOC)
}

/// Surova hodnota v logickom rozsahu -> 0..=100. Neplatny rozsah alebo hodnota mimo neho = `None` (nic sa neodhaduje).
pub fn percent(raw: i64, lmin: i32, lmax: i32) -> Option<u8> {
    let (lo, hi) = (i64::from(lmin), i64::from(lmax));
    if hi <= lo || raw < lo || raw > hi {
        return None;
    }
    Some((((raw - lo) * 100 + (hi - lo) / 2) / (hi - lo)).clamp(0, 100) as u8)
}

/// Cesta patri Bluetooth zariadeniu (uroven hlasi Windows - zdroj Bluetooth).
pub fn is_bluetooth_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    // HID over GATT (BLE) a klasicky Bluetooth HID
    p.contains("{00001812-0000-1000-8000-00805f9b34fb}")
        || p.contains("{00001124-0000-1000-8000-00805f9b34fb}")
        || p.contains("bthledevice")
        || p.contains("bthenum")
}

/// Druh zariadenia podla kolekcie (Generic Desktop: mys, klavesnica, ovladac), inak podla nazvu.
pub fn kind_of(usage_page: u16, usage: u16, name: &str) -> Kind {
    match (usage_page, usage) {
        (0x01, 0x01 | 0x02) => Kind::Mouse,
        (0x01, 0x06 | 0x07) => Kind::Keyboard,
        (0x01, 0x04 | 0x05) => Kind::Controller,
        _ => kind_from_name(name),
    }
}

#[cfg(windows)]
pub use system::Provider;

#[cfg(windows)]
mod system {
    use super::*;
    use crate::hid::{self, BatteryCap};
    use crate::model::{Presence, Reading, Source};
    use std::time::{Duration, Instant};

    /// Zoznam kandidatov sa obnovuje najviac raz za 5 minut (vymenovanie vsetkych HID rozhrani nie je zadarmo);
    /// skor, ked sa nic neprecitalo.
    const RESCAN: Duration = Duration::from_secs(300);

    #[derive(Default)]
    pub struct Provider {
        found: Vec<(hid::HidInfo, BatteryCap)>,
        scanned: Option<Instant>,
    }

    impl Provider {
        pub fn poll(&mut self) -> Vec<Reading> {
            if self.scanned.is_none_or(|t| t.elapsed() >= RESCAN) || self.found.is_empty() {
                self.found = hid::enumerate_all()
                    .into_iter()
                    .filter(|i| !OWN_PROTOCOL_VIDS.contains(&i.vid) && !is_bluetooth_path(&i.path))
                    .filter_map(|i| hid::battery_cap(&i.path).map(|c| (i, c)))
                    .collect();
                self.scanned = Some(Instant::now());
            }
            let mut out: Vec<Reading> = Vec::new();
            for (info, cap) in &self.found {
                let key = format!("hid:{:04x}:{:04x}", info.vid, info.pid);
                if out.iter().any(|r| r.key == key) {
                    continue;
                }
                let Some(raw) = hid::read_battery(&info.path, cap) else {
                    continue;
                };
                let Some(pct) = percent(raw, cap.logical_min, cap.logical_max) else {
                    continue;
                };
                let name = if info.product.is_empty() {
                    format!("HID {:04X}:{:04X}", info.vid, info.pid)
                } else {
                    info.product.clone()
                };
                out.push(Reading {
                    key,
                    kind: kind_of(info.usage_page, info.usage, &name),
                    name,
                    source: Source::Hid,
                    percent: Some(pct),
                    charging: false,
                    presence: Presence::Live,
                });
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn percent_table_covers_signed_and_degenerate_ranges() {
        use super::percent;
        // (surova, min, max, vysledok)
        for (raw, lo, hi, want) in [
            (0i64, 0, 100, Some(0)),
            (100, 0, 100, Some(100)),
            (50, 0, 255, Some(20)),
            (255, 0, 255, Some(100)),
            (0, -128, 127, Some(50)),
            (-128, -128, 127, Some(0)),
            (i64::from(i32::MAX), i32::MIN, i32::MAX, Some(100)),
            (101, 0, 100, None),
            (-1, 0, 100, None),
            (5, 5, 5, None),
            (5, 10, 0, None),
        ] {
            assert_eq!(percent(raw, lo, hi), want, "{raw} v {lo}..={hi}");
        }
    }

    use super::*;

    #[test]
    fn battery_values_scale_by_the_logical_range_and_reject_garbage() {
        assert_eq!(percent(50, 0, 100), Some(50));
        assert_eq!(percent(255, 0, 255), Some(100));
        assert_eq!(percent(128, 0, 255), Some(50));
        assert_eq!(percent(3, 1, 5), Some(50));
        assert_eq!(percent(300, 0, 255), None);
        assert_eq!(percent(-1, 0, 100), None);
        assert_eq!(percent(5, 5, 5), None);
    }

    #[test]
    fn only_standard_battery_usages_count() {
        assert!(is_battery_usage(0x06, 0x20));
        assert!(is_battery_usage(0x85, 0x65));
        // zostavajuca kapacita v mAh nie su percenta
        assert!(!is_battery_usage(0x85, 0x66));
        assert!(!is_battery_usage(0x01, 0x20));
    }

    #[test]
    fn bluetooth_paths_and_own_protocol_vendors_are_left_to_their_sources() {
        assert!(is_bluetooth_path(
            r"\\?\hid#{00001812-0000-1000-8000-00805f9b34fb}_dev_vid&02045e_pid&0b13#a&1"
        ));
        assert!(!is_bluetooth_path(r"\\?\hid#vid_1532&pid_00bf&mi_00#7&1"));
        assert!(OWN_PROTOCOL_VIDS.contains(&0x1532));
    }

    #[test]
    fn the_collection_decides_the_kind() {
        assert_eq!(kind_of(0x01, 0x02, "x"), Kind::Mouse);
        assert_eq!(kind_of(0x01, 0x06, "x"), Kind::Keyboard);
        assert_eq!(kind_of(0x01, 0x05, "x"), Kind::Controller);
        assert_eq!(kind_of(0xFF00, 0x01, "Wireless Gamepad"), Kind::Controller);
    }
}

#[cfg(all(test, windows))]
mod live {
    /// Rucne: `cargo test -p transom-peripherals generic::live -- --ignored --nocapture` (len citanie).
    #[test]
    #[ignore]
    fn print_standard_hid_batteries() {
        let all = crate::hid::enumerate_all();
        let caps: Vec<_> = all
            .iter()
            .filter_map(|i| crate::hid::battery_cap(&i.path).map(|c| (i, c)))
            .collect();
        println!(
            "HID rozhrani: {}, so standardnou bateriou: {}",
            all.len(),
            caps.len()
        );
        for (i, c) in &caps {
            println!(
                "  {:04x}:{:04x} kolekcia {:04x}/{:04x} {:?} -> {:?}",
                i.vid,
                i.pid,
                i.usage_page,
                i.usage,
                c,
                crate::hid::read_battery(&i.path, c)
            );
        }
        let mut p = super::Provider::default();
        for r in p.poll() {
            println!("  citanie: {} {:?} {:?}", r.key, r.kind, r.percent);
        }
    }
}
