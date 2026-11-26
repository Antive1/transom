//! Razer: 90-bajtove HID feature reporty (fakty o protokole ako v projekte OpenRazer; ziadny prevzaty kod).
//!
//! Len dve citacie prikazy: trieda 0x07, prikaz 0x80 (uroven 0..=255) a 0x84 (nabija sa). Overene 2026-11-23 na
//! DeathAdder V4 Pro: cez prijimac (PID 00BF, rozhranie MI_00, transaction id 0x1F) aj cez kabel (PID 00BE), kde
//! prijimac pocas nabijania hlasi "timeout" a mys odpoveda na kablovom rozhrani.
use crate::model::{kind_from_name, Kind, Presence, Reading, Source};

pub const VID: u16 = 0x1532;
/// 1 bajt cisla reportu + 90 bajtov reportu.
pub const REPORT_LEN: usize = 91;
/// Transaction id: novsie zariadenia 0x1F, ine 0x3F, starsie 0xFF.
pub const TRANSACTION_IDS: [u8; 3] = [0x1F, 0x3F, 0xFF];

pub const STATUS_NEW: u8 = 0x00;
pub const STATUS_BUSY: u8 = 0x01;
pub const STATUS_OK: u8 = 0x02;
/// Prijimac je pritomny, ale mys neodpoveda (spi, vypnuta, mimo dosahu).
pub const STATUS_TIMEOUT: u8 = 0x04;

pub const CLASS_POWER: u8 = 0x07;
pub const CMD_BATTERY: u8 = 0x80;
pub const CMD_CHARGING: u8 = 0x84;

/// Poziadavka na citanie (`buf[0]` = report 0).
pub fn request(tid: u8, class: u8, cmd: u8) -> [u8; REPORT_LEN] {
    let mut b = [0u8; REPORT_LEN];
    let r = &mut b[1..];
    r[1] = tid;
    r[5] = 0x02; // velkost dat
    r[6] = class;
    r[7] = cmd;
    r[88] = crc(r);
    b
}

/// XOR bajtov 2..88 reportu (bez stavu a transaction id).
pub fn crc(report: &[u8]) -> u8 {
    report[2..88].iter().fold(0, |a, b| a ^ b)
}

/// Vysledok jednej odpovede.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Zariadenie este neodpovedalo (stav 0 alebo 1): treba citat znova.
    Pending,
    /// Odpoved na iny prikaz (cudzi program sa pyta tiez): treba poslat znova.
    Foreign,
    /// Hotova odpoved: stav a argument 1 (hodnota).
    Done { status: u8, value: u8 },
}

/// Rozbor odpovede (`buf[0]` = report 0).
pub fn parse(buf: &[u8], tid: u8, class: u8, cmd: u8) -> Answer {
    if buf.len() < REPORT_LEN {
        return Answer::Foreign;
    }
    let r = &buf[1..];
    let status = r[0];
    if status == STATUS_NEW || status == STATUS_BUSY {
        return Answer::Pending;
    }
    if r[1] != tid || r[6] != class || r[7] != cmd {
        return Answer::Foreign;
    }
    Answer::Done {
        status,
        value: r[9],
    }
}

/// 0..=255 -> percento (zaokruhlene).
pub fn percent(raw: u8) -> u8 {
    ((u32::from(raw) * 100 + 127) / 255) as u8
}

/// Nazov zariadenia: produktovy retazec (prijimac hlasi nazov mysi), vzdy s "Razer".
pub fn display_name(product: &str) -> String {
    let p = product.trim();
    if p.is_empty() {
        "Razer".into()
    } else if p.to_ascii_lowercase().starts_with("razer") {
        p.into()
    } else {
        format!("Razer {p}")
    }
}

/// Kluc zariadenia: podla nazvu, aby prijimac a kabel boli to iste zariadenie.
pub fn key(name: &str) -> String {
    format!("razer:{}", name.to_ascii_lowercase())
}

fn kind(name: &str) -> Kind {
    match kind_from_name(name) {
        Kind::Other => Kind::Mouse,
        k => k,
    }
}

/// Citanie cez Windows.
#[cfg(windows)]
#[derive(Default)]
pub struct Provider {
    /// Overena kombinacia (cesta, transaction id) z minulych kol.
    known: Vec<(String, u8)>,
}

#[cfg(windows)]
impl Provider {
    fn query(dev: &crate::hid::FeatureDevice, tid: u8, class: u8, cmd: u8) -> Option<(u8, u8)> {
        let req = request(tid, class, cmd);
        for _ in 0..2 {
            if !dev.set_feature(&req) {
                return None;
            }
            for attempt in 0..6 {
                std::thread::sleep(std::time::Duration::from_millis(if attempt == 0 {
                    60
                } else {
                    80
                }));
                let mut buf = [0u8; REPORT_LEN];
                if !dev.get_feature(&mut buf) {
                    return None;
                }
                match parse(&buf, tid, class, cmd) {
                    Answer::Pending => continue,
                    Answer::Foreign => break,
                    Answer::Done { status, value } => return Some((status, value)),
                }
            }
        }
        None
    }

    /// Precita jedno rozhranie s danym transaction id.
    fn read_one(info: &crate::hid::HidInfo, tid: u8) -> Option<Reading> {
        let dev = crate::hid::FeatureDevice::open(&info.path)?;
        let name = display_name(&info.product);
        let base = |percent, charging, presence| Reading {
            key: key(&name),
            name: name.clone(),
            kind: kind(&name),
            source: Source::Razer,
            percent,
            charging,
            presence,
        };
        match Self::query(&dev, tid, CLASS_POWER, CMD_BATTERY)? {
            (STATUS_OK, raw) => {
                let charging = matches!(
                    Self::query(&dev, tid, CLASS_POWER, CMD_CHARGING),
                    Some((STATUS_OK, v)) if v != 0
                );
                Some(base(Some(percent(raw)), charging, Presence::Live))
            }
            (STATUS_TIMEOUT, _) => Some(base(None, false, Presence::Asleep)),
            _ => None,
        }
    }

    pub fn poll(&mut self) -> Vec<Reading> {
        let infos: Vec<crate::hid::HidInfo> = crate::hid::enumerate(&[VID])
            .into_iter()
            .filter(|i| usize::from(i.feature_len) >= REPORT_LEN)
            .collect();
        let mut out = Vec::new();
        let mut known = Vec::new();
        for info in &infos {
            // najprv overeny transaction id, potom ostatne
            let mut tids: Vec<u8> = self
                .known
                .iter()
                .filter(|(p, _)| *p == info.path)
                .map(|(_, t)| *t)
                .collect();
            for t in TRANSACTION_IDS {
                if !tids.contains(&t) {
                    tids.push(t);
                }
            }
            let verified = self.known.iter().any(|(p, _)| *p == info.path);
            let mut asleep = None;
            for tid in tids {
                match Self::read_one(info, tid) {
                    Some(r) if r.presence == Presence::Live => {
                        known.push((info.path.clone(), tid));
                        out.push(r);
                        asleep = None;
                        break;
                    }
                    Some(r) => {
                        // "timeout" vracia aj nespravny transaction id: pri overenom id netreba skusat dalsie
                        asleep.get_or_insert(r);
                        if verified {
                            break;
                        }
                    }
                    None => {}
                }
            }
            out.extend(asleep);
        }
        // zachovat znamy tid aj pre spiace zariadenia
        for (p, t) in &self.known {
            if !known.iter().any(|(q, _)| q == p) && infos.iter().any(|i| &i.path == p) {
                known.push((p.clone(), *t));
            }
        }
        self.known = known;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout_and_crc_match_the_captured_probe() {
        let b = request(0x1F, CLASS_POWER, CMD_BATTERY);
        assert_eq!(b.len(), 91);
        assert_eq!(&b[..9], &[0, 0, 0x1F, 0, 0, 0, 0x02, 0x07, 0x80]);
        // XOR 0x02 ^ 0x07 ^ 0x80
        assert_eq!(b[89], 0x85);
        assert_eq!(b[90], 0);
    }

    fn response(status: u8, tid: u8, class: u8, cmd: u8, value: u8) -> [u8; REPORT_LEN] {
        let mut b = [0u8; REPORT_LEN];
        b[1] = status;
        b[2] = tid;
        b[7] = class;
        b[8] = cmd;
        b[10] = value;
        b
    }

    #[test]
    fn parse_real_answers() {
        // zachytene: "status=2 tid=1F cls=7 id=80 args=00-A6" (dongle) a nabijanie "args=00-01" (kabel)
        assert_eq!(
            parse(&response(2, 0x1F, 7, 0x80, 0xA6), 0x1F, 7, 0x80),
            Answer::Done {
                status: 2,
                value: 0xA6
            }
        );
        assert_eq!(percent(0xA6), 65);
        assert_eq!(percent(0xA4), 64);
        assert_eq!(percent(255), 100);
        assert_eq!(percent(0), 0);
        assert_eq!(
            parse(&response(4, 0x1F, 7, 0x80, 0), 0x1F, 7, 0x80),
            Answer::Done {
                status: STATUS_TIMEOUT,
                value: 0
            }
        );
        assert_eq!(
            parse(&response(1, 0x1F, 7, 0x80, 0), 0x1F, 7, 0x80),
            Answer::Pending
        );
        assert_eq!(
            parse(&response(0, 0, 0, 0, 0), 0x1F, 7, 0x80),
            Answer::Pending
        );
        assert_eq!(
            parse(&response(2, 0x1F, 0x0F, 0x04, 9), 0x1F, 7, 0x80),
            Answer::Foreign
        );
        assert_eq!(parse(&[0u8; 10], 0x1F, 7, 0x80), Answer::Foreign);
    }

    #[test]
    fn percent_scale_and_short_or_foreign_reports() {
        // (surova hodnota, percento)
        for (raw, want) in [
            (0u8, 0u8),
            (1, 0),
            (2, 1),
            (127, 50),
            (128, 50),
            (254, 100),
            (255, 100),
        ] {
            assert_eq!(percent(raw), want, "{raw}");
        }
        let mut last = 0;
        for raw in 0..=255u8 {
            assert!(percent(raw) >= last);
            last = percent(raw);
        }
        for len in [0, 1, 10, REPORT_LEN - 1] {
            assert_eq!(
                parse(&vec![STATUS_OK; len], 0x1F, CLASS_POWER, CMD_BATTERY),
                Answer::Foreign
            );
        }
        // odpoved s inym transaction id / prikazom = cudzia
        let mut ok = request(0x1F, CLASS_POWER, CMD_BATTERY);
        ok[1] = STATUS_OK;
        ok[10] = 200;
        assert_eq!(
            parse(&ok, 0x1F, CLASS_POWER, CMD_BATTERY),
            Answer::Done {
                status: STATUS_OK,
                value: 200
            }
        );
        assert_eq!(parse(&ok, 0x3F, CLASS_POWER, CMD_BATTERY), Answer::Foreign);
        assert_eq!(parse(&ok, 0x1F, CLASS_POWER, CMD_CHARGING), Answer::Foreign);
        ok[1] = STATUS_BUSY;
        assert_eq!(parse(&ok, 0x3F, CLASS_POWER, CMD_CHARGING), Answer::Pending);
    }

    #[test]
    fn names_and_keys_join_receiver_and_cable() {
        assert_eq!(display_name("DeathAdder V4 Pro"), "Razer DeathAdder V4 Pro");
        assert_eq!(
            display_name("Razer DeathAdder V4 Pro"),
            "Razer DeathAdder V4 Pro"
        );
        assert_eq!(display_name(" "), "Razer");
        assert_eq!(
            key(&display_name("DeathAdder V4 Pro")),
            key(&display_name("Razer DeathAdder V4 Pro"))
        );
        assert_eq!(kind("Razer DeathAdder V4 Pro"), Kind::Mouse);
        assert_eq!(kind("Razer Barracuda X headset"), Kind::Headset);
    }
}
