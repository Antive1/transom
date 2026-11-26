//! NuPhy WH80 cez 2.4 GHz prijimac (VID 19F5, PID 8F01): len citacie prikazy 0x04 (GetBase) a 0x05 (GetFunc).
//!
//! Zistenia z 2026-11-23 (pasivne pocuvanie a porovnanie pamate s kablom a bez neho):
//! * uroven je bajt na `mode * 64 + 32` (bity 0..6), bit 7 (nabija sa) firmver nikdy nenastavi;
//! * pocas nabijania kablom firmver hlasi vsade 100 % - skutocnu uroven pocas nabijania nehlasi vobec;
//! * pri zapojeni/odpojeni kabla posle klavesnica sama report `FA FB 07 00 00 <uroven>`: 0x64 (100) po zapojeni,
//!   skutocnu uroven (napr. 0x5A = 90) po odpojeni. Tento bajt je uroven, nie "stav kabla".
//!
//! Pocas nabijania sa preto ukazuje "nabija sa" **bez percent** (nic sa neodhaduje).
use crate::model::{Kind, Presence, Reading, Source};

pub const VID: u16 = 0x19F5;
pub const PID_WH80_DONGLE: u16 = 0x8F01;
/// 1 bajt cisla reportu + 64 bajtov ramca.
pub const REPORT_LEN: usize = 65;
const HEADER_CMD: u8 = 0x55;
const HEADER_REPLY: u8 = 0xAA;
pub const CMD_GET_BASE: u8 = 0x04;
pub const CMD_GET_FUNC: u8 = 0x05;

/// Sucet bajtov 5..65 (Windows buffer s cislom reportu na indexe 0).
pub fn checksum(b: &[u8]) -> u8 {
    b[5..REPORT_LEN].iter().fold(0u8, |a, x| a.wrapping_add(*x))
}

/// Citacia poziadavka.
pub fn request(cmd: u8, offset: u16, len: u8) -> [u8; REPORT_LEN] {
    let mut b = [0u8; REPORT_LEN];
    b[1] = HEADER_CMD;
    b[2] = cmd;
    b[5] = len;
    b[6] = (offset & 0xFF) as u8;
    b[7] = (offset >> 8) as u8;
    b[4] = checksum(&b);
    b
}

/// Data z odpovede na `request(cmd, offset, len)`; `None` = iny report.
pub fn parse_reply(b: &[u8], cmd: u8, offset: u16, len: u8) -> Option<Vec<u8>> {
    if b.len() < REPORT_LEN {
        return None;
    }
    let off = u16::from(b[6]) | (u16::from(b[7]) << 8);
    let ok = b[1] == HEADER_REPLY
        && b[2] == cmd
        && b[5] == len
        && off == offset
        && b[8] == 0
        && b[4] == checksum(b);
    // dlzka dat musi byt v ramci reportu (inak by `len` z volania alebo z odpovede krajal mimo buffera)
    ok.then(|| b.get(9..9 + usize::from(len)).map(<[u8]>::to_vec))
        .flatten()
}

/// Report o kabli `FA FB 07 00 00 <uroven>`: vrati uroven (0..=100).
pub fn cable_event(b: &[u8]) -> Option<u8> {
    // `then`, nie `then_some`: bajt 6 sa smie citat az po kontrole dlzky
    (b.len() >= 7 && b[1] == 0xFA && b[2] == 0xFB && b[3] == 0x07).then(|| b[6] & 0x7F)
}

/// Adresa bajtu urovne pre rezim pripojenia.
pub fn battery_offset(mode: u8) -> u16 {
    u16::from(mode.min(3)) * 64 + 32
}

/// Co vieme o kabli a poslednej skutocnej urovni.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Charge {
    /// Posledna uroven pod 100 % (skutocna, nie "100 kvoli kablu").
    pub last_real: Option<u8>,
    /// Report "kabel zapojeny" prisiel a odvtedy nebola uroven pod 100 %.
    pub cable: bool,
}

impl Charge {
    /// Report o kabli: 100 = zapojeny; menej = odpojeny a nesie skutocnu uroven.
    pub fn on_event(&mut self, level: u8) {
        if level >= 100 {
            self.cable = true;
        } else {
            self.cable = false;
            self.last_real = Some(level);
        }
    }

    /// Bajt urovne -> (percento, nabija sa).
    ///
    /// * pod 100 %: skutocna uroven, kabel urcite nie je zapojeny (opravi aj zmeskany report o odpojeni);
    /// * 100 %: nabija sa, ak prisiel report o zapojeni alebo ak predtym bola uroven nizsia (zmeskany report);
    ///   inak plna bateria. Nabita do plna a odpojena klavesnica vyzera rovnako ako zapojena - opravi sa sama,
    ///   ked uroven klesne pod 100 %.
    pub fn interpret(&mut self, byte: u8) -> (Option<u8>, bool) {
        let v = (byte & 0x7F).min(100);
        if byte & 0x80 != 0 {
            return (Some(v), true);
        }
        if v < 100 {
            self.cable = false;
            self.last_real = Some(v);
            return (Some(v), false);
        }
        if self.cable || self.last_real.is_some_and(|l| l < 100) {
            (None, true)
        } else {
            (Some(100), false)
        }
    }
}

pub const NAME: &str = "NuPhy WH80";

pub fn reading(percent: Option<u8>, charging: bool, presence: Presence) -> Reading {
    Reading {
        key: "nuphy:wh80".into(),
        name: NAME.into(),
        kind: Kind::Keyboard,
        source: Source::NuPhy,
        percent,
        charging,
        presence,
    }
}

#[cfg(windows)]
#[derive(Default)]
pub struct Provider {
    dev: Option<crate::hid::IoDevice>,
    charge: Charge,
}

#[cfg(windows)]
impl Provider {
    fn open() -> Option<crate::hid::IoDevice> {
        let info = crate::hid::enumerate(&[VID]).into_iter().find(|i| {
            i.pid == PID_WH80_DONGLE
                && i.usage_page == 0x0001
                && i.usage == 0x0000
                && usize::from(i.input_len) == REPORT_LEN
                && usize::from(i.output_len) == REPORT_LEN
        })?;
        let dev = crate::hid::IoDevice::open(&info.path, true)?;
        dev.set_input_buffers(512);
        Some(dev)
    }

    /// Spracuje cakajuce reporty (aj o kabli); vrati data odpovede, ak prisla.
    fn exchange(&mut self, cmd: u8, offset: u16, len: u8) -> Result<Option<Vec<u8>>, ()> {
        use std::time::{Duration, Instant};
        let Some(dev) = &self.dev else {
            return Err(());
        };
        // najprv vycerpat, co prislo od posledneho kola (report o kabli)
        let mut buf = [0u8; REPORT_LEN];
        while let Some(n) = dev.read(&mut buf, Duration::from_millis(1)) {
            if n == 0 {
                break;
            }
            if let Some(l) = cable_event(&buf) {
                self.charge.on_event(l);
            }
        }
        if !dev.write(&request(cmd, offset, len), Duration::from_millis(500)) {
            return Err(());
        }
        let deadline = Instant::now() + Duration::from_millis(400);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some(_) = dev.read(&mut buf, left) else {
                break;
            };
            if let Some(l) = cable_event(&buf) {
                self.charge.on_event(l);
                continue;
            }
            if let Some(d) = parse_reply(&buf, cmd, offset, len) {
                return Ok(Some(d));
            }
        }
        Ok(None)
    }

    pub fn poll(&mut self) -> Vec<Reading> {
        if self.dev.is_none() {
            self.dev = Self::open();
            if self.dev.is_none() {
                return Vec::new();
            }
        }
        let mode = match self.exchange(CMD_GET_BASE, 0, 16) {
            Err(()) => {
                self.dev = None; // prijimac zmizol
                return Vec::new();
            }
            Ok(Some(d)) if d[0] <= 3 => d[0],
            Ok(Some(_)) => 1,
            Ok(None) => return vec![reading(None, false, Presence::Asleep)],
        };
        match self.exchange(CMD_GET_FUNC, battery_offset(mode), 1) {
            Err(()) => {
                self.dev = None;
                Vec::new()
            }
            Ok(Some(d)) => {
                let (p, c) = self.charge.interpret(d[0]);
                vec![reading(p, c, Presence::Live)]
            }
            Ok(None) => vec![reading(None, false, Presence::Asleep)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let mut v: Vec<u8> = s
            .split('-')
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect();
        v.resize(REPORT_LEN, 0);
        v
    }

    #[test]
    fn request_matches_the_existing_mod() {
        let b = request(CMD_GET_FUNC, 0x60, 1);
        assert_eq!(&b[..9], &[0, 0x55, 0x05, 0, 0x61, 0x01, 0x60, 0x00, 0x00]);
        assert_eq!(b[4], checksum(&b));
    }

    #[test]
    fn parses_captured_replies_and_events() {
        // zachytene 2026-11-23 na MI_01 (odpovede na GetBase a GetFunc 0x60, report o kabli)
        let base = hex("00-AA-04-00-23-10-00-00-00-01-04-AA-BB-A2-A2-A2-A2-03-00-00-00-1E");
        let mut fixed = base.clone();
        fixed[4] = checksum(&fixed);
        assert_eq!(parse_reply(&fixed, CMD_GET_BASE, 0, 16).unwrap()[0], 1);
        let func = hex("00-AA-05-00-C5-01-60-00-00-64");
        assert_eq!(parse_reply(&func, CMD_GET_FUNC, 0x60, 1), Some(vec![0x64]));
        let func90 = hex("00-AA-05-00-BB-01-60-00-00-5A");
        assert_eq!(
            parse_reply(&func90, CMD_GET_FUNC, 0x60, 1),
            Some(vec![0x5A])
        );
        // zly offset / prikaz = cudzi report
        assert_eq!(parse_reply(&func, CMD_GET_FUNC, 0x20, 1), None);
        assert_eq!(parse_reply(&func, CMD_GET_BASE, 0x60, 1), None);
        assert_eq!(cable_event(&hex("00-FA-FB-07-00-00-5A")), Some(90));
        assert_eq!(cable_event(&hex("00-FA-FB-07-00-00-64")), Some(100));
        assert_eq!(cable_event(&func), None);
        assert_eq!(battery_offset(1), 0x60);
    }

    #[test]
    fn a_reply_claiming_more_data_than_the_report_holds_is_rejected() {
        for len in [56u8, 57, 200, 255] {
            let mut b = vec![0u8; REPORT_LEN];
            b[1] = HEADER_REPLY;
            b[2] = CMD_GET_BASE;
            b[5] = len;
            b[4] = checksum(&b);
            let got = parse_reply(&b, CMD_GET_BASE, 0, len);
            assert_eq!(got.is_some(), len == 56, "len {len}");
        }
        // kratky buffer a cudzie reporty
        assert_eq!(parse_reply(&[0xAA; 10], CMD_GET_BASE, 0, 1), None);
        assert_eq!(cable_event(&[0, 0xFA, 0xFB, 0x07, 0, 0]), None);
    }

    #[test]
    fn charging_without_invented_numbers() {
        let mut c = Charge::default();
        assert_eq!(c.interpret(0x5A), (Some(90), false));
        // kabel zapojeny: firmver hlasi 100 -> nabija sa, uroven nevieme
        c.on_event(100);
        assert_eq!(c.interpret(0x64), (None, true));
        // odpojeny: report nesie skutocnu uroven
        c.on_event(91);
        assert_eq!(c.last_real, Some(91));
        assert_eq!(c.interpret(0x5B), (Some(91), false));
    }

    #[test]
    fn missed_event_is_inferred_and_corrected() {
        let mut c = Charge::default();
        // od startu 100 bez historie: plna bateria, nie kabel
        assert_eq!(c.interpret(0x64), (Some(100), false));
        // nizsia uroven, potom 100 bez reportu = kabel
        assert_eq!(c.interpret(0x50), (Some(80), false));
        assert_eq!(c.interpret(0x64), (None, true));
        // menej nez 100 = urcite odpojeny (aj ked report chybal)
        assert_eq!(c.interpret(0x52), (Some(82), false));
        assert!(!c.cable);
        // report o zapojeni plati aj bez historie
        let mut fresh = Charge::default();
        fresh.on_event(100);
        assert_eq!(fresh.interpret(0x64), (None, true));
        // nabita do plna a odpojena (report 100) sa opravi pri prvej urovni pod 100 %
        assert_eq!(fresh.interpret(0x63), (Some(99), false));
        assert_eq!(fresh.interpret(0x63), (Some(99), false));
    }
}
