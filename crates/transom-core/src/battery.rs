//! Bateria: prevod surovych hodnot `SYSTEM_POWER_STATUS` (Win32 `GetSystemPowerStatus`) na stav pre lištu.
//! Polozka sa zobrazi len vtedy, ked system hlasi systemovu bateriu.

/// Surove hodnoty zo `SYSTEM_POWER_STATUS` (rovnake vyznamy ako v dokumentacii Win32).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerRaw {
    /// 0 = baterie, 1 = siet, 255 = neznamy.
    pub ac_line: u8,
    /// Bitove priznaky: 1 vysoka, 2 nizka, 4 kriticka, 8 nabija sa, 128 = **ziadna systemova bateria**, 255 = neznamy.
    pub battery_flag: u8,
    /// 0..=100, 255 = neznamy.
    pub percent: u8,
    /// `SystemStatusFlag`: 1 = sporic baterie zapnuty.
    pub saver: u8,
}

const FLAG_CHARGING: u8 = 8;
const FLAG_NO_BATTERY: u8 = 128;
const UNKNOWN: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// `None`, ak system percento nepozna.
    pub percent: Option<u8>,
    pub charging: bool,
    pub on_ac: bool,
    pub saver: bool,
}

impl Battery {
    /// Stav z hodnot `GetSystemPowerStatus`; `None` = polozku baterie skryt (ziadna bateria alebo uplne neznamy stav).
    pub fn from_raw(raw: PowerRaw) -> Option<Battery> {
        if raw.battery_flag != UNKNOWN && raw.battery_flag & FLAG_NO_BATTERY != 0 {
            return None;
        }
        let percent = (raw.percent <= 100).then_some(raw.percent);
        if raw.battery_flag == UNKNOWN && percent.is_none() {
            return None;
        }
        Some(Battery {
            percent,
            charging: raw.battery_flag != UNKNOWN && raw.battery_flag & FLAG_CHARGING != 0,
            on_ac: raw.ac_line == 1,
            saver: raw.saver & 1 != 0,
        })
    }

    /// Text vedla symbolu, napr. `57 %`; pri neznamom percente `?`.
    pub fn label(&self) -> String {
        match self.percent {
            Some(p) => format!("{p} %"),
            None => "?".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(ac_line: u8, battery_flag: u8, percent: u8) -> PowerRaw {
        PowerRaw {
            ac_line,
            battery_flag,
            percent,
            saver: 0,
        }
    }

    #[test]
    fn desktop_without_battery_is_hidden() {
        // typicka hodnota stolneho PC: na sieti, flag 128, percento 255
        assert_eq!(Battery::from_raw(raw(1, 128, 255)), None);
        // aj keby percento bolo "platne", flag 128 rozhoduje
        assert_eq!(Battery::from_raw(raw(1, 128, 100)), None);
    }

    #[test]
    fn fully_unknown_is_hidden() {
        assert_eq!(Battery::from_raw(raw(255, 255, 255)), None);
    }

    #[test]
    fn laptop_on_battery_is_visible() {
        let b = Battery::from_raw(raw(0, 1, 76)).unwrap();
        assert_eq!(b.percent, Some(76));
        assert!(!b.charging && !b.on_ac && !b.saver);
        assert_eq!(b.label(), "76 %");
    }

    #[test]
    fn charging_flag_and_saver() {
        let b = Battery::from_raw(PowerRaw {
            ac_line: 1,
            battery_flag: 8 | 2,
            percent: 15,
            saver: 1,
        })
        .unwrap();
        assert!(b.charging && b.on_ac && b.saver);
    }

    #[test]
    fn unknown_percent_with_known_battery_still_visible() {
        let b = Battery::from_raw(raw(1, 0, 255)).unwrap();
        assert_eq!(b.percent, None);
        assert_eq!(b.label(), "?");
        // neznamy flag, ale znamy percent -> zobrazit, bez "nabija"
        let b = Battery::from_raw(raw(255, 255, 40)).unwrap();
        assert_eq!(b.percent, Some(40));
        assert!(!b.charging);
    }
}
