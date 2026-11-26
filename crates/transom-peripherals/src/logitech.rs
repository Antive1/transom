//! Logitech HID++ 2.0 (prijimace Unifying / Bolt / Lightspeed a zariadenia na kabli). Napisane nanovo podla
//! verejne popisaneho protokolu (spravy, funkcia ROOT, vlastnosti 0x0005, 0x1000, 0x1001, 0x1004); ziadny prevzaty
//! kod. **Neoverene na skutocnom zariadeni** (vyvojovy stroj nema Logitech hardver) - pokryte len testami formatu.
//!
//! Sprava: `[report, index zariadenia, index vlastnosti, (funkcia << 4) | sw id, parametre...]`.
//! Kratky report 0x10 ma 7 bajtov, dlhy 0x11 ma 20. Odpoved ma rovnake prve 4 bajty; chyba HID++ 2.0 ma na
//! mieste vlastnosti 0xFF, chyba HID++ 1.0 (napr. zariadenie za prijimacom nie je pripojene) 0x8F.
use crate::model::{kind_from_name, Kind, Presence, Reading, Source};

pub const VID: u16 = 0x046D;
pub const SHORT: u8 = 0x10;
pub const LONG: u8 = 0x11;
pub const SHORT_LEN: usize = 7;
pub const LONG_LEN: usize = 20;
/// Nase "software id" (nenulove, aby sa odpovede nemiesali s notifikaciami zariadenia).
pub const SW_ID: u8 = 0x0B;

pub const FEAT_ROOT: u16 = 0x0000;
pub const FEAT_DEVICE_NAME: u16 = 0x0005;
pub const FEAT_BATTERY_STATUS: u16 = 0x1000;
pub const FEAT_BATTERY_VOLTAGE: u16 = 0x1001;
pub const FEAT_UNIFIED_BATTERY: u16 = 0x1004;

/// Index zariadenia priamo pripojeneho (kabel, Bluetooth); za prijimacom su indexy 1..=6.
pub const DIRECT: u8 = 0xFF;

pub fn message(report: u8, device: u8, feature: u8, function: u8, params: &[u8]) -> Vec<u8> {
    let len = if report == SHORT { SHORT_LEN } else { LONG_LEN };
    let mut b = vec![0u8; len];
    b[0] = report;
    b[1] = device;
    b[2] = feature;
    b[3] = (function << 4) | SW_ID;
    for (i, p) in params.iter().take(len - 4).enumerate() {
        b[4 + i] = *p;
    }
    b
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Odpoved: parametre (bajty od indexu 4).
    Ok(Vec<u8>),
    /// Chyba (kod). HID++ 1.0 chyba za prijimacom = zariadenie nie je pripojene.
    Error { code: u8, hidpp10: bool },
    /// Iny report (notifikacia, odpoved inemu programu).
    Foreign,
}

pub fn parse(buf: &[u8], device: u8, feature: u8, function: u8) -> Answer {
    if buf.len() < SHORT_LEN || (buf[0] != SHORT && buf[0] != LONG) || buf[1] != device {
        return Answer::Foreign;
    }
    let fsw = (function << 4) | SW_ID;
    if buf[2] == 0x8F && buf[3] == feature && buf[4] == fsw {
        return Answer::Error {
            code: buf[5],
            hidpp10: true,
        };
    }
    if buf[2] == 0xFF && buf[3] == feature && buf[4] == fsw {
        return Answer::Error {
            code: buf[5],
            hidpp10: false,
        };
    }
    if buf[2] == feature && buf[3] == fsw {
        return Answer::Ok(buf[4..].to_vec());
    }
    Answer::Foreign
}

/// Vysledok citania baterie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: Option<u8>,
    pub charging: bool,
}

/// 0x1004 UNIFIED_BATTERY, funkcia 1 (getStatus): [percento, uroven (bity), stav nabijania, externe napajanie].
pub fn unified_battery(p: &[u8]) -> Option<Battery> {
    if p.len() < 3 {
        return None;
    }
    let percent = if p[0] > 0 {
        Some(p[0].min(100))
    } else {
        // zariadenie hlasi len uroven: plna / dobra / nizka / kriticka
        match p[1] {
            l if l & 0x08 != 0 => Some(100),
            l if l & 0x04 != 0 => Some(50),
            l if l & 0x02 != 0 => Some(20),
            l if l & 0x01 != 0 => Some(5),
            _ => None,
        }
    };
    let charging = matches!(p[2], 1 | 2);
    Some(Battery { percent, charging })
}

/// 0x1000 BATTERY_STATUS, funkcia 0: [percento, dalsia uroven, stav].
pub fn battery_status(p: &[u8]) -> Option<Battery> {
    if p.len() < 3 {
        return None;
    }
    match p[2] {
        5 | 6 => None, // neplatna bateria / tepelna chyba
        3 => Some(Battery {
            percent: Some(if p[0] == 0 { 100 } else { p[0].min(100) }),
            charging: false,
        }),
        s => Some(Battery {
            percent: (p[0] > 0).then_some(p[0].min(100)),
            charging: matches!(s, 1 | 2 | 4),
        }),
    }
}

/// 0x1001 BATTERY_VOLTAGE, funkcia 0: [napatie mV (big endian), priznaky]. Percento je **priblizne** z typickej
/// vybijacej krivky Li-Pol clanku.
pub fn battery_voltage(p: &[u8]) -> Option<Battery> {
    if p.len() < 3 {
        return None;
    }
    let mv = u16::from_be_bytes([p[0], p[1]]);
    if mv == 0 {
        return None;
    }
    Some(Battery {
        percent: Some(voltage_percent(mv)),
        charging: p[2] & 0x80 != 0,
    })
}

pub fn voltage_percent(mv: u16) -> u8 {
    const CURVE: [(u16, u8); 11] = [
        (3500, 0),
        (3671, 10),
        (3717, 20),
        (3751, 30),
        (3778, 40),
        (3811, 50),
        (3859, 60),
        (3922, 70),
        (3989, 80),
        (4067, 90),
        (4186, 100),
    ];
    if mv <= CURVE[0].0 {
        return 0;
    }
    for w in CURVE.windows(2) {
        let ((v0, p0), (v1, p1)) = (w[0], w[1]);
        if mv <= v1 {
            let f = f32::from(mv - v0) / f32::from(v1 - v0);
            return (f32::from(p0) + f * f32::from(p1 - p0)).round() as u8;
        }
    }
    100
}

/// 0x0005 DEVICE_NAME, funkcia 2 (typ) -> druh.
pub fn kind_of_type(t: u8) -> Kind {
    match t {
        0 | 2 => Kind::Keyboard,
        3..=5 => Kind::Mouse,
        8 => Kind::Headset,
        10..=12 => Kind::Controller,
        _ => Kind::Other,
    }
}

pub fn display_name(raw: &str) -> String {
    let n = raw.trim_matches(char::from(0)).trim();
    if n.is_empty() {
        "Logitech".into()
    } else if n.to_ascii_lowercase().starts_with("logitech") {
        n.into()
    } else {
        format!("Logitech {n}")
    }
}

pub fn key(name: &str) -> String {
    format!("logitech:{}", name.to_ascii_lowercase())
}

/// Odstrani cislo kolekcie z cesty, aby kratka a dlha kolekcia toho isteho rozhrania mali spolocny kluc.
pub fn interface_key(path: &str) -> String {
    // \\?\hid#vid_046d&pid_c548&mi_02&col01#8&2d3f9b1&0&0000#{guid}: bez "&colNN" a bez posledneho "&NNNN"
    let p = path.to_ascii_lowercase();
    let parts: Vec<&str> = p.split('#').collect();
    if parts.len() < 3 {
        return p;
    }
    let hw = match parts[1].find("&col") {
        Some(i) => &parts[1][..i],
        None => parts[1],
    };
    let inst = match parts[2].rfind('&') {
        Some(i) if parts[1].contains("&col") => &parts[2][..i],
        _ => parts[2],
    };
    format!("{}#{hw}#{inst}", parts[0])
}

#[cfg(windows)]
mod live {
    use super::*;
    use crate::hid::{enumerate, IoDevice};
    use std::time::{Duration, Instant};

    /// Jedno HID++ rozhranie (kratka a/alebo dlha kolekcia).
    struct Channel {
        key: String,
        short: Option<IoDevice>,
        long: Option<IoDevice>,
    }

    enum Outcome {
        Ok(Vec<u8>),
        Error { hidpp10: bool },
        Timeout,
        Io,
    }

    impl Channel {
        fn send(&self, device: u8, feature: u8, function: u8, params: &[u8]) -> Outcome {
            let (dev, report) = match (&self.short, &self.long) {
                (Some(s), _) if params.len() <= 3 => (s, SHORT),
                (_, Some(l)) => (l, LONG),
                (Some(s), None) => (s, SHORT),
                (None, None) => return Outcome::Io,
            };
            // stare reporty prec
            let mut buf = [0u8; LONG_LEN];
            for d in [&self.short, &self.long].into_iter().flatten() {
                while d.read(&mut buf, Duration::from_millis(1)).is_some() {}
            }
            if !dev.write(
                &message(report, device, feature, function, params),
                Duration::from_millis(300),
            ) {
                return Outcome::Io;
            }
            let deadline = Instant::now() + Duration::from_millis(400);
            while Instant::now() < deadline {
                for d in [&self.long, &self.short].into_iter().flatten() {
                    let mut b = [0u8; LONG_LEN];
                    if let Some(n) = d.read(&mut b, Duration::from_millis(15)) {
                        match parse(&b[..n.max(SHORT_LEN)], device, feature, function) {
                            Answer::Ok(p) => return Outcome::Ok(p),
                            Answer::Error { hidpp10, .. } => return Outcome::Error { hidpp10 },
                            Answer::Foreign => {}
                        }
                    }
                }
            }
            Outcome::Timeout
        }

        /// Index vlastnosti (ROOT.getFeature); `Ok(None)` = zariadenie ju nema.
        fn feature(&self, device: u8, id: u16) -> Result<Option<u8>, Outcome> {
            let [hi, lo] = id.to_be_bytes();
            match self.send(device, 0, 0, &[hi, lo]) {
                Outcome::Ok(p) => Ok((p.first().copied().unwrap_or(0) != 0).then(|| p[0])),
                o => Err(o),
            }
        }

        fn name(&self, device: u8) -> (String, Kind) {
            let Ok(Some(f)) = self.feature(device, FEAT_DEVICE_NAME) else {
                return (display_name(""), Kind::Other);
            };
            let len = match self.send(device, f, 0, &[]) {
                Outcome::Ok(p) => usize::from(p.first().copied().unwrap_or(0)).min(64),
                _ => 0,
            };
            let mut bytes = Vec::new();
            while bytes.len() < len {
                match self.send(device, f, 1, &[bytes.len() as u8]) {
                    Outcome::Ok(p) if !p.is_empty() => {
                        let take = (len - bytes.len()).min(p.len());
                        bytes.extend_from_slice(&p[..take]);
                    }
                    _ => break,
                }
            }
            let name = display_name(&String::from_utf8_lossy(&bytes));
            let kind = match self.send(device, f, 2, &[]) {
                Outcome::Ok(p) if !p.is_empty() => kind_of_type(p[0]),
                _ => kind_from_name(&name),
            };
            (name, kind)
        }

        fn battery(&self, device: u8) -> Result<Option<Battery>, Outcome> {
            for (id, function, parse_fn) in [
                (
                    FEAT_UNIFIED_BATTERY,
                    1u8,
                    unified_battery as fn(&[u8]) -> Option<Battery>,
                ),
                (FEAT_BATTERY_STATUS, 0, battery_status),
                (FEAT_BATTERY_VOLTAGE, 0, battery_voltage),
            ] {
                if let Some(f) = self.feature(device, id)? {
                    return match self.send(device, f, function, &[]) {
                        Outcome::Ok(p) => Ok(parse_fn(&p)),
                        o => Err(o),
                    };
                }
            }
            Ok(None)
        }
    }

    #[derive(Default)]
    pub struct Provider {
        channels: Vec<Channel>,
        /// Nazov a druh podla (rozhranie, index zariadenia) - nacita sa raz.
        names: Vec<(String, u8, String, Kind)>,
    }

    impl Provider {
        fn refresh_channels(&mut self) {
            let infos: Vec<_> = enumerate(&[VID])
                .into_iter()
                .filter(|i| matches!(i.usage_page, 0xFF00 | 0xFF43))
                .collect();
            let mut keys: Vec<String> = infos.iter().map(|i| interface_key(&i.path)).collect();
            keys.sort();
            keys.dedup();
            self.channels.retain(|c| keys.contains(&c.key));
            for k in keys {
                if self.channels.iter().any(|c| c.key == k) {
                    continue;
                }
                let mut ch = Channel {
                    key: k.clone(),
                    short: None,
                    long: None,
                };
                for i in infos.iter().filter(|i| interface_key(&i.path) == k) {
                    if usize::from(i.output_len) == SHORT_LEN && ch.short.is_none() {
                        ch.short = IoDevice::open(&i.path, true);
                    } else if usize::from(i.output_len) == LONG_LEN && ch.long.is_none() {
                        ch.long = IoDevice::open(&i.path, true);
                    }
                }
                if ch.short.is_some() || ch.long.is_some() {
                    self.channels.push(ch);
                }
            }
            self.names
                .retain(|(k, ..)| self.channels.iter().any(|c| &c.key == k));
        }

        fn name_of(&mut self, ch: usize, device: u8) -> (String, Kind) {
            let key = self.channels[ch].key.clone();
            if let Some((.., n, k)) = self
                .names
                .iter()
                .find(|(c, d, ..)| *c == key && *d == device)
            {
                return (n.clone(), *k);
            }
            let (n, k) = self.channels[ch].name(device);
            self.names.push((key, device, n.clone(), k));
            (n, k)
        }

        pub fn poll(&mut self) -> Vec<Reading> {
            self.refresh_channels();
            let mut out = Vec::new();
            for ch in 0..self.channels.len() {
                // priamo pripojene zariadenie odpovie na indexe 0xFF; prijimac (HID++ 1.0) vrati chybu
                let direct = self.channels[ch].battery(DIRECT);
                let targets: Vec<u8> = match direct {
                    Ok(Some(_)) => vec![DIRECT],
                    _ => (1..=6).collect(),
                };
                for device in targets {
                    let result = if device == DIRECT {
                        direct.as_ref().ok().cloned().flatten().ok_or(Outcome::Io)
                    } else {
                        self.channels[ch]
                            .battery(device)
                            .and_then(|b| b.ok_or(Outcome::Io))
                    };
                    match result {
                        Ok(b) => {
                            let (name, kind) = self.name_of(ch, device);
                            out.push(Reading {
                                key: key(&name),
                                name,
                                kind,
                                source: Source::Logitech,
                                percent: b.percent,
                                charging: b.charging,
                                presence: Presence::Live,
                            });
                        }
                        Err(Outcome::Error { hidpp10: true }) | Err(Outcome::Timeout) => {
                            // sparovane, ale nepripojene: ak uz pozname nazov, ukazat poslednu hodnotu
                            let key_ch = self.channels[ch].key.clone();
                            if let Some((.., n, k)) = self
                                .names
                                .iter()
                                .find(|(c, d, ..)| *c == key_ch && *d == device)
                            {
                                out.push(Reading {
                                    key: key(n),
                                    name: n.clone(),
                                    kind: *k,
                                    source: Source::Logitech,
                                    percent: None,
                                    charging: false,
                                    presence: Presence::Asleep,
                                });
                            }
                        }
                        Err(_) => {}
                    }
                }
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

    #[test]
    fn messages_have_the_hidpp_layout() {
        assert_eq!(
            message(SHORT, 1, 0, 0, &[0x10, 0x04]),
            vec![0x10, 0x01, 0x00, 0x0B, 0x10, 0x04, 0x00]
        );
        let l = message(LONG, DIRECT, 5, 1, &[3]);
        assert_eq!(l.len(), 20);
        assert_eq!(&l[..5], &[0x11, 0xFF, 0x05, 0x1B, 0x03]);
    }

    #[test]
    fn parses_answers_errors_and_foreign_reports() {
        let mut ok = vec![0x11, 0x02, 0x06, 0x1B, 55, 0x04, 0x01, 0x00];
        ok.resize(20, 0);
        assert_eq!(parse(&ok, 2, 6, 1), Answer::Ok(ok[4..].to_vec()));
        // ina funkcia, iny index alebo notifikacia (sw id 0)
        assert_eq!(parse(&ok, 2, 6, 0), Answer::Foreign);
        assert_eq!(parse(&ok, 3, 6, 1), Answer::Foreign);
        let mut note = ok.clone();
        note[3] = 0x10;
        assert_eq!(parse(&note, 2, 6, 1), Answer::Foreign);
        let e20 = [0x11, 0x02, 0xFF, 0x06, 0x1B, 0x05, 0, 0];
        assert_eq!(
            parse(&e20, 2, 6, 1),
            Answer::Error {
                code: 5,
                hidpp10: false
            }
        );
        let e10 = [0x10, 0x03, 0x8F, 0x00, 0x0B, 0x09, 0x00];
        assert_eq!(
            parse(&e10, 3, 0, 0),
            Answer::Error {
                code: 9,
                hidpp10: true
            }
        );
        assert_eq!(parse(&[0x20, 1, 2, 3, 4, 5, 6], 1, 2, 0), Answer::Foreign);
    }

    #[test]
    fn battery_features() {
        assert_eq!(
            unified_battery(&[55, 0x04, 1, 1]),
            Some(Battery {
                percent: Some(55),
                charging: true
            })
        );
        assert_eq!(
            unified_battery(&[0, 0x02, 0, 0]),
            Some(Battery {
                percent: Some(20),
                charging: false
            })
        );
        assert_eq!(unified_battery(&[0, 0, 0]).unwrap().percent, None);
        assert_eq!(
            battery_status(&[80, 50, 0]),
            Some(Battery {
                percent: Some(80),
                charging: false
            })
        );
        assert!(battery_status(&[30, 20, 1]).unwrap().charging);
        assert_eq!(battery_status(&[0, 0, 3]).unwrap().percent, Some(100));
        assert_eq!(battery_status(&[50, 0, 5]), None);
        let v = battery_voltage(&[0x0F, 0x0A, 0x80]).unwrap(); // 3850 mV
        assert!(v.charging);
        assert!((55..=60).contains(&v.percent.unwrap()), "{v:?}");
        assert_eq!(voltage_percent(3000), 0);
        assert_eq!(voltage_percent(4300), 100);
        assert_eq!(voltage_percent(3811), 50);
        assert_eq!(battery_voltage(&[0, 0, 0]), None);
    }

    #[test]
    fn battery_states_table() {
        let b = |percent: Option<u8>, charging: bool| Some(Battery { percent, charging });
        // 0x1000: (parametre, vysledok) pre kazdy stav 0..=7
        let status = [
            ([40u8, 0, 0], b(Some(40), false)),
            ([40, 0, 1], b(Some(40), true)),
            ([95, 0, 2], b(Some(95), true)),
            ([0, 0, 3], b(Some(100), false)),
            ([99, 0, 3], b(Some(99), false)),
            ([40, 0, 4], b(Some(40), true)),
            ([40, 0, 5], None),
            ([40, 0, 6], None),
            ([40, 0, 7], b(Some(40), false)),
            ([0, 0, 0], b(None, false)),
            ([250, 0, 0], b(Some(100), false)),
        ];
        for (p, want) in status {
            assert_eq!(battery_status(&p), want, "{p:?}");
        }
        // 0x1004: percento ma prednost pred urovnou; bez oboch nic
        let unified = [
            ([0u8, 0x08, 3, 1], b(Some(100), false)),
            ([0, 0x0F, 0, 0], b(Some(100), false)),
            ([0, 0x04, 2, 1], b(Some(50), true)),
            ([0, 0x01, 0, 0], b(Some(5), false)),
            ([0, 0x00, 1, 1], b(None, true)),
            ([150, 0x01, 4, 0], b(Some(100), false)),
        ];
        for (p, want) in unified {
            assert_eq!(unified_battery(&p), want, "{p:?}");
        }
        for short in [&[][..], &[50], &[50, 0]] {
            assert_eq!(battery_status(short), None);
            assert_eq!(unified_battery(short), None);
            assert_eq!(battery_voltage(short), None);
        }
        // krivka napatia je monotonna a v hraniciach
        let mut last = 0;
        for mv in (3000..=4400).step_by(7) {
            let p = voltage_percent(mv);
            assert!(p >= last && p <= 100, "{mv} mV -> {p}");
            last = p;
        }
    }

    #[test]
    fn reports_of_any_length_never_panic() {
        for len in 0..=LONG_LEN {
            for first in [SHORT, LONG, 0x20] {
                let mut b = vec![0u8; len];
                if let Some(x) = b.first_mut() {
                    *x = first;
                }
                for f in [0x8F, 0xFF, 0x01] {
                    if len > 2 {
                        b[2] = f;
                    }
                    let _ = parse(&b, 0, 1, 0);
                }
            }
        }
    }

    #[test]
    fn names_kinds_and_interface_keys() {
        assert_eq!(display_name("MX Master 3S\0\0"), "Logitech MX Master 3S");
        assert_eq!(display_name("Logitech G502"), "Logitech G502");
        assert_eq!(kind_of_type(3), Kind::Mouse);
        assert_eq!(kind_of_type(0), Kind::Keyboard);
        assert_eq!(kind_of_type(8), Kind::Headset);
        assert_eq!(kind_of_type(99), Kind::Other);
        let g = "{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(
            interface_key(&format!(
                r"\\?\HID#VID_046D&PID_C548&MI_02&Col01#8&2d3f9b1&0&0000#{g}"
            )),
            interface_key(&format!(
                r"\\?\hid#vid_046d&pid_c548&mi_02&col02#8&2d3f9b1&0&0001#{g}"
            ))
        );
        assert_ne!(
            interface_key(&format!(
                r"\\?\hid#vid_046d&pid_c548&mi_02&col01#8&2d3f9b1&0&0000#{g}"
            )),
            interface_key(&format!(
                r"\\?\hid#vid_046d&pid_c548&mi_01&col01#8&aaaa&0&0000#{g}"
            ))
        );
        // dve rovnake prijimace: rozna instancia
        assert_ne!(
            interface_key(&format!(
                r"\\?\hid#vid_046d&pid_c548&mi_02&col01#8&1111&0&0000#{g}"
            )),
            interface_key(&format!(
                r"\\?\hid#vid_046d&pid_c548&mi_02&col01#8&2222&0&0000#{g}"
            ))
        );
    }
}
