//! Cisty model systemoveho vystupu zvuku (bez Win32): zariadenia, hlasitost, dostupnost a ich normalizacia.
//! Vsetko, co sa zobrazuje, prechadza tadial: nazvy sa ocistia (riadiace, obojsmerne a nulovej sirky znaky), duplicity
//! sa rozlisia, hlasitost je vzdy cele cislo 0..=100 a NaN / mimo rozsahu sa nikdy nepreleje do UI. ID zariadenia
//! (neprehladny retazec Windows) sa nikdy nezobrazuje ani nelogue: `Snapshot` ho neobsahuje.

/// Stav koncoveho bodu (`DEVICE_STATE_*` bez `NOTPRESENT`, tie sa nezobrazuju).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Active,
    Disabled,
    Unplugged,
}

/// Zariadenie tak, ako ho vrati backend (vratane ID na rozpoznanie predvolenej zmeny).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDevice {
    pub id: String,
    pub name: String,
    pub state: DeviceState,
    /// Predvolene zariadenie pre rolu `eConsole`.
    pub is_default: bool,
}

/// Zariadenie na zobrazenie: ocisteny a rozlisenym nazov, bez ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Ocisteny nazov (moze byt prazdny: UI ukaze "Zariadenie bez nazvu").
    pub name: String,
    /// Stabilny kluc zariadenia (odtlacok ID, nie ID samotne) na volbu predvoleneho vystupu.
    pub key: u64,
    pub state: DeviceState,
    pub is_default: bool,
}

/// Hlasitost a stlmenie predvoleneho vystupu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub percent: u8,
    pub muted: bool,
}

/// Co predvoleny vystup vie ovladat (zistuje sa citanim; chyba = nepodporuje).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controls {
    pub volume: bool,
    pub mute: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Controls {
            volume: true,
            mute: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Availability {
    /// Este sa nenacitalo.
    #[default]
    Starting,
    Ok,
    /// Zvukova sluzba bezi, ale nie je ziadny aktivny vystup.
    NoDevice,
    /// Zvukova sluzba (Windows Audio) nie je dostupna alebo sa spojenie s nou stratilo.
    ServiceUnavailable,
}

/// Chyba backendu (hodnota, nie panika).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    NoDevice,
    Unavailable(String),
    Unsupported,
    Failed(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDevice => write!(f, "ziadne vystupne zariadenie"),
            AudioError::Unavailable(m) => write!(f, "zvukova sluzba nie je dostupna ({m})"),
            AudioError::Unsupported => write!(f, "zariadenie tuto funkciu nepodporuje"),
            AudioError::Failed(m) => write!(f, "operacia zlyhala ({m})"),
        }
    }
}

/// Snimka stavu pre UI (lacno klonovatelna cez `Arc`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Zvysuje sa pri kazdej zmene (UI sa prekresli, len ak sa zmenila).
    pub generation: u64,
    pub availability: Availability,
    pub devices: Vec<Device>,
    /// Hlasitost predvoleneho vystupu (`None` = nie je znama: ziadny vystup alebo sluzba nedostupna).
    pub volume: Option<Volume>,
    pub controls: Controls,
    /// Kolkokrat Windows ohlasil **zmenu** predvoleneho vystupu na iny (iba z notifikacie Core Audio). UI podla toho
    /// potvrdi prepnutie; pred tym sa o zmene nikdy netvrdi.
    pub default_changes: u64,
}

impl Snapshot {
    /// Nazov predvoleneho zariadenia (ak nejake je).
    pub fn default_device(&self) -> Option<&Device> {
        self.devices.iter().find(|d| d.is_default)
    }
}

/// Hlasitost 0.0..=1.0 -> cele percento (NaN a mimo rozsahu sa orezu).
pub fn percent_of(scalar: f32) -> u8 {
    if !scalar.is_finite() {
        return 0;
    }
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// Cele percento -> hlasitost 0.0..=1.0 (mimo 0..=100 sa orezie).
pub fn scalar_of(percent: u32) -> f32 {
    percent.min(100) as f32 / 100.0
}

const MAX_NAME_CHARS: usize = 80;

fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
        )
}

/// Nazov zariadenia na zobrazenie: bez riadiacich, obojsmernych a nulovej sirky znakov, orezany, bez okrajovych medzier.
pub fn clean_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !is_unsafe_char(*c))
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > MAX_NAME_CHARS {
        let mut t: String = s.chars().take(MAX_NAME_CHARS - 1).collect();
        t.push('…');
        t
    } else {
        s
    }
}

/// Odtlacok ID zariadenia (ID sa do snimky nedostane).
pub fn key_of(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

/// Zariadenia na zobrazenie: ocistene nazvy, rovnake nazvy sa rozlisia "(2)", "(3)" (len v zobrazeni), predvolene
/// zariadenie prve a potom aktivne pred neaktivnymi (stabilne podla povodneho poradia).
pub fn display_devices(raw: &[RawDevice]) -> Vec<Device> {
    let mut out: Vec<Device> = Vec::with_capacity(raw.len());
    for d in raw {
        let base = clean_name(&d.name);
        let same = out
            .iter()
            .filter(|o| {
                let n = &o.name;
                *n == base || (n.starts_with(&base) && n[base.len()..].starts_with(" ("))
            })
            .count();
        let name = if !base.is_empty() && same > 0 {
            format!("{base} ({})", same + 1)
        } else {
            base
        };
        out.push(Device {
            name,
            key: key_of(&d.id),
            state: d.state,
            is_default: d.is_default,
        });
    }
    out.sort_by_key(|d| (!d.is_default, d.state != DeviceState::Active));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: &str, name: &str, state: DeviceState, def: bool) -> RawDevice {
        RawDevice {
            id: id.into(),
            name: name.into(),
            state,
            is_default: def,
        }
    }

    #[test]
    fn percent_is_whole_bounded_and_never_nan() {
        for (f, want) in [
            (0.0, 0),
            (1.0, 100),
            (0.1527, 15),
            (0.155, 16),
            (0.005, 1),
            (0.004, 0),
            (-0.5, 0),
            (1.5, 100),
            (f32::NAN, 0),
            (f32::INFINITY, 0),
            (f32::NEG_INFINITY, 0),
        ] {
            assert_eq!(percent_of(f), want, "{f}");
        }
        // zapis -> citanie je stabilny pre vsetky percenta (ziadny roztraseny posuvnik)
        for p in 0..=100u32 {
            assert_eq!(percent_of(scalar_of(p)) as u32, p, "{p}");
        }
        assert_eq!(scalar_of(250), 1.0);
        assert_eq!(scalar_of(u32::MAX), 1.0);
    }

    #[test]
    fn names_lose_control_bidi_and_zero_width_characters_and_are_capped() {
        assert_eq!(
            clean_name("  Repro\u{202E}duktory \u{200B}(Realtek)\0 "),
            "Reproduktory (Realtek)"
        );
        assert_eq!(clean_name("A\n\tB"), "A B");
        assert_eq!(clean_name(""), "");
        let long = "x".repeat(500);
        let c = clean_name(&long);
        assert_eq!(c.chars().count(), 80);
        assert!(c.ends_with('…'));
        assert_eq!(clean_name("😀 Slúchadlá"), "😀 Slúchadlá");
    }

    #[test]
    fn duplicates_are_numbered_only_in_the_display_and_the_default_comes_first() {
        let d = display_devices(&[
            raw("1", "Headset", DeviceState::Unplugged, false),
            raw("2", "Speakers", DeviceState::Active, false),
            raw("3", "Speakers", DeviceState::Active, true),
            raw("4", "Speakers", DeviceState::Active, false),
            raw("5", "", DeviceState::Active, false),
        ]);
        let names: Vec<&str> = d.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names[0], "Speakers (2)", "predvolene je prve: {names:?}");
        assert!(d[0].is_default);
        assert!(names.contains(&"Speakers"));
        assert!(names.contains(&"Speakers (3)"));
        assert!(
            names.contains(&""),
            "prazdny nazov ostava prazdny (UI ho preklada)"
        );
        // neaktivne zariadenia su az za aktivnymi
        assert_eq!(d.last().unwrap().state, DeviceState::Unplugged);
    }
}
