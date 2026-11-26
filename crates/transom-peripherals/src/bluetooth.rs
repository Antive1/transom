//! Bluetooth: uroven, ktoru Windows hlasi pre sparovane zariadenia (ta ista ako v Nastaveniach > Bluetooth).
//!
//! Windows ju uklada ako vlastnost uzla zariadenia `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2` (bajt 0..=100).
//! **Kluc nie je v hlavickach SDK** (pouziva ho Get-PnpDeviceProperty a komunita); preto sa vsetko cita len na citanie
//! a pri akejkolvek nezhode sa zariadenie radsej nezobrazi.
//!
//! Windows si poslednu uroven pamata aj pre vypnute zariadenie (overene 2026-11-23: Xbox ovladac vypnuty od septembra
//! stale hlasi 60 %). Zariadenie sa preto ukaze len ked je pripojene: vlastnost `{83DA6326-…} 15` (bool; tiez mimo
//! SDK) = `true`, alebo - ak ju uzol nema - ked je cas poslednej urovne (`{104EA319-…} 7`) mladsi nez 30 minut.
use crate::model::{kind_from_name, Presence, Reading, Source};

/// Cas FILETIME (100 ns od 1601) -> sekundy.
pub const FILETIME_PER_SEC: u64 = 10_000_000;
/// Bez priznaku pripojenia: uroven starsia nez toto sa neukaze.
pub const FRESH_SECS: u64 = 30 * 60;

/// Ukazat zariadenie? `connected` = priznak pripojenia, `updated`/`now` = FILETIME.
pub fn should_show(connected: Option<bool>, updated: Option<u64>, now: u64) -> bool {
    match connected {
        Some(c) => c,
        None => updated.is_some_and(|u| now.saturating_sub(u) / FILETIME_PER_SEC < FRESH_SECS),
    }
}

pub fn reading(container: &str, name: &str, percent: u8) -> Reading {
    Reading {
        key: format!("bt:{}", container.to_ascii_lowercase()),
        name: name.to_string(),
        kind: kind_from_name(name),
        source: Source::Bluetooth,
        percent: Some(percent.min(100)),
        // Windows stav nabijania Bluetooth zariadeni nehlasi
        charging: false,
        presence: Presence::Live,
    }
}

#[cfg(windows)]
mod live {
    use super::*;
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
        SetupDiGetDevicePropertyW, DIGCF_ALLCLASSES, DIGCF_PRESENT, HDEVINFO, SP_DEVINFO_DATA,
    };
    use windows::Win32::Devices::Properties::{
        DEVPKEY_Device_ContainerId, DEVPKEY_Device_DeviceDesc, DEVPKEY_Device_FriendlyName,
        DEVPROPTYPE,
    };
    use windows::Win32::Foundation::{DEVPROPKEY, FILETIME};
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

    const BATTERY_FMT: GUID = GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5);
    const KEY_LEVEL: DEVPROPKEY = DEVPROPKEY {
        fmtid: BATTERY_FMT,
        pid: 2,
    };
    const KEY_UPDATED: DEVPROPKEY = DEVPROPKEY {
        fmtid: BATTERY_FMT,
        pid: 7,
    };
    const KEY_CONNECTED: DEVPROPKEY = DEVPROPKEY {
        fmtid: GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29),
        pid: 15,
    };
    const TYPE_BYTE: u32 = 0x03;
    const TYPE_BOOLEAN: u32 = 0x11;
    const TYPE_FILETIME: u32 = 0x10;
    const TYPE_GUID: u32 = 0x0D;
    const TYPE_STRING: u32 = 0x12;

    fn prop(set: HDEVINFO, dev: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<(u32, Vec<u8>)> {
        let mut ty = DEVPROPTYPE(0);
        let mut buf = vec![0u8; 512];
        let mut need = 0u32;
        // SAFETY: platny zoznam a polozka, buffer s danou velkostou
        unsafe {
            SetupDiGetDevicePropertyW(set, dev, key, &mut ty, Some(&mut buf), Some(&mut need), 0)
        }
        .ok()?;
        buf.truncate(need as usize);
        Some((ty.0, buf))
    }

    fn string(set: HDEVINFO, dev: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<String> {
        let (ty, b) = prop(set, dev, key)?;
        if ty != TYPE_STRING {
            return None;
        }
        let w: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&c| c != 0)
            .collect();
        let s = String::from_utf16_lossy(&w).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    #[derive(Default)]
    pub struct Provider;

    impl Provider {
        pub fn poll(&mut self) -> Vec<Reading> {
            let mut out: Vec<Reading> = Vec::new();
            // SAFETY: zoznam vsetkych pritomnych uzlov; uvolni sa na konci
            let Ok(set) = (unsafe {
                SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT)
            }) else {
                return out;
            };
            // SAFETY: cas systemu
            let now = unsafe { GetSystemTimeAsFileTime() };
            let now = (u64::from(now.dwHighDateTime) << 32) | u64::from(now.dwLowDateTime);
            for i in 0..8192u32 {
                let mut dev = SP_DEVINFO_DATA {
                    cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                    ..Default::default()
                };
                // SAFETY: platny zoznam, struktura so spravnou velkostou
                if unsafe { SetupDiEnumDeviceInfo(set, i, &mut dev) }.is_err() {
                    break;
                }
                let Some((TYPE_BYTE, level)) = prop(set, &dev, &KEY_LEVEL) else {
                    continue;
                };
                let Some(&percent) = level.first() else {
                    continue;
                };
                if percent > 100 {
                    continue;
                }
                let connected = match prop(set, &dev, &KEY_CONNECTED) {
                    Some((TYPE_BOOLEAN, b)) => b.first().map(|&v| v != 0),
                    _ => None,
                };
                let updated = match prop(set, &dev, &KEY_UPDATED) {
                    Some((TYPE_FILETIME, b)) if b.len() >= 8 => {
                        let ft = FILETIME {
                            dwLowDateTime: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                            dwHighDateTime: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                        };
                        Some((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime))
                    }
                    _ => None,
                };
                if !should_show(connected, updated, now) {
                    continue;
                }
                let name = string(set, &dev, &DEVPKEY_Device_FriendlyName)
                    .or_else(|| string(set, &dev, &DEVPKEY_Device_DeviceDesc))
                    .unwrap_or_else(|| "Bluetooth".into());
                // viac uzlov toho isteho zariadenia ma spolocny kontajner
                let container = match prop(set, &dev, &DEVPKEY_Device_ContainerId) {
                    Some((TYPE_GUID, b)) if b.len() >= 16 => b[..16]
                        .iter()
                        .map(|x| format!("{x:02x}"))
                        .collect::<String>(),
                    _ => format!("{name}#{i}"),
                };
                let r = reading(&container, &name, percent);
                if !out.iter().any(|o| o.key == r.key) {
                    out.push(r);
                }
            }
            // SAFETY: zoznam z SetupDiGetClassDevsW
            unsafe {
                let _ = SetupDiDestroyDeviceInfoList(set);
            }
            out
        }
    }
}

#[cfg(windows)]
pub use live::Provider;

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: u64 = FILETIME_PER_SEC;

    #[test]
    fn connected_flag_decides_and_stale_value_is_hidden() {
        let now = 1_000_000_000 * SEC;
        // Xbox ovladac z 2026-11-23: priznak false, uroven z pred 2,5 mesiaca -> skryt
        assert!(!should_show(
            Some(false),
            Some(now - 80 * 24 * 3600 * SEC),
            now
        ));
        // pripojene zariadenie sa ukaze aj so starou urovnou (uroven sa meni zriedka)
        assert!(should_show(Some(true), Some(now - 5 * 3600 * SEC), now));
        // bez priznaku rozhoduje vek urovne
        assert!(should_show(None, Some(now - 10 * 60 * SEC), now));
        assert!(!should_show(None, Some(now - 31 * 60 * SEC), now));
        assert!(!should_show(None, None, now));
    }

    #[test]
    fn readings_are_keyed_by_container() {
        let a = reading("ABCD", "Xbox Wireless Controller", 60);
        assert_eq!(a.key, "bt:abcd");
        assert_eq!(a.kind, crate::model::Kind::Controller);
        assert_eq!(a.percent, Some(60));
        assert!(!a.charging);
        assert_eq!(reading("x", "y", 250).percent, Some(100));
    }
}
