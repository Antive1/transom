//! Poloha zariadenia (volitelne, len ked ju pouzivatel zvoli ako zdroj pocasia): sluzba polohy Windows
//! (`Windows.Devices.Geolocation`). Suradnice sa zaokruhlia na 2 desatinne miesta (~1 km) a nikde sa nelogguju.
//! Volat len na pracovnom vlakne (blokuje az do odpovede alebo casoveho limitu). Region Windows pre hladanie PSC.
use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator, PositionAccuracy};
use windows::Foundation::TimeSpan;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// Preco sa poloha nezistila.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeoError {
    /// Poloha je vo Windows vypnuta alebo nie je povolena pre aplikacie.
    Denied,
    /// Sluzba polohy neodpovedala (bez signalu, casovy limit, chyba).
    Unavailable,
}

fn init() {
    // SAFETY: pracovne vlakno; opakovana inicializacia vrati len S_FALSE / RPC_E_CHANGED_MODE.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Smie transom pouzit polohu? (Windows moze pri prvom volani ukazat vlastnu otazku.)
pub fn access() -> Result<(), GeoError> {
    init();
    let s = Geolocator::RequestAccessAsync()
        .and_then(|op| op.join())
        .map_err(|_| GeoError::Unavailable)?;
    if s == GeolocationAccessStatus::Allowed {
        Ok(())
    } else if s == GeolocationAccessStatus::Denied {
        Err(GeoError::Denied)
    } else {
        Err(GeoError::Unavailable)
    }
}

/// Zaokruhlenie suradnice na ~1 km.
pub fn coarse(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Aktualna poloha (sirka, dlzka) zaokruhlena na ~1 km. Pouzije aj polohu staru do 10 minut; limit 15 s.
pub fn position() -> Result<(f64, f64), GeoError> {
    access()?;
    let g = Geolocator::new().map_err(|_| GeoError::Unavailable)?;
    let _ = g.SetDesiredAccuracy(PositionAccuracy::Default);
    // TimeSpan je v jednotkach 100 ns
    let age = TimeSpan {
        Duration: 10 * 60 * 10_000_000,
    };
    let limit = TimeSpan {
        Duration: 15 * 10_000_000,
    };
    let pos = g
        .GetGeopositionAsyncWithAgeAndTimeout(age, limit)
        .and_then(|op| op.join())
        .map_err(|e| {
            // E_ACCESSDENIED: poloha vypnuta medzi otazkou a citanim
            if e.code().0 as u32 == 0x8007_0005 {
                GeoError::Denied
            } else {
                GeoError::Unavailable
            }
        })?;
    let p = pos
        .Coordinate()
        .and_then(|c| c.Point())
        .and_then(|p| p.Position())
        .map_err(|_| GeoError::Unavailable)?;
    if !p.Latitude.is_finite() || !p.Longitude.is_finite() {
        return Err(GeoError::Unavailable);
    }
    Ok((coarse(p.Latitude), coarse(p.Longitude)))
}

/// Otvori nastavenia polohy Windows (Sukromie a zabezpecenie > Poloha).
pub fn open_settings() {
    use windows::core::w;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    // SAFETY: konstantne retazce.
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:privacy-location"),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// Krajina regionu Windows (ISO 3166 dvojpismenovy kod, napr. "SK"); `None`, ak nie je zistitelna.
pub fn region_country() -> Option<String> {
    let mut buf = [0u16; 16];
    // SAFETY: buffer na zasobniku s dlzkou.
    let n = unsafe { windows::Win32::Globalization::GetUserDefaultGeoName(&mut buf) };
    if n <= 1 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(n as usize - 1)]);
    (s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic())).then(|| s.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_are_rounded_to_about_a_kilometre() {
        assert_eq!(coarse(48.14816), 48.15);
        assert_eq!(coarse(-17.10674), -17.11);
    }

    #[test]
    fn region_country_is_a_two_letter_code_when_known() {
        if let Some(c) = region_country() {
            assert_eq!(c.len(), 2);
            assert!(c.chars().all(|c| c.is_ascii_uppercase()));
        }
    }
}
