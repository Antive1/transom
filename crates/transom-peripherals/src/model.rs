//! Cisty model batérii periférií (bez Win32): co hlasi zdroj, ako sa z toho stane stav pre listu a panel.
//!
//! * Zdroj (`Source`) pri kazdom citani vrati zoznam `Reading`: zariadenie odpovedalo (`Live`), alebo je jeho prijimac
//!   pritomny, ale zariadenie spi / je mimo dosahu (`Asleep`).
//! * `Tracker` si pamata poslednu skutocnu hodnotu kazdeho zariadenia. Spiace zariadenie sa ukazuje s poslednou
//!   hodnotou a priznakom `stale`; zariadenie, ktore nehlasi ziadny zdroj, sa po `forget_after` zahodi.
//! * Nic sa neodhaduje: ked zariadenie uroven nehlasi (napr. NuPhy pocas nabijania), je `percent == None`.
use std::time::{Duration, Instant};

/// Druh zariadenia (len na vyber ikony a poradia; nikdy sa z neho nic neodvodzuje).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Kind {
    Mouse,
    Keyboard,
    Headset,
    Controller,
    #[default]
    Other,
}

/// Odkial je hodnota.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Bluetooth zariadenie, ktorého uroven hlasi Windows (rovnaka hodnota ako v Nastaveniach > Bluetooth).
    Bluetooth,
    /// Razer cez USB prijimac alebo kabel (HID feature reporty, protokol ako v OpenRazer).
    Razer,
    /// Logitech HID++ 2.0 cez prijimac (Unifying, Bolt, Lightspeed) alebo kabel.
    Logitech,
    /// NuPhy cez 2.4 GHz prijimac.
    NuPhy,
    /// Standardna uroven baterie HID (Battery Strength), bez protokolu vyrobcu.
    Hid,
}

impl Source {
    /// Stabilny identifikator (konfiguracia, logy).
    pub fn id(self) -> &'static str {
        match self {
            Source::Bluetooth => "bluetooth",
            Source::Razer => "razer",
            Source::Logitech => "logitech",
            Source::NuPhy => "nuphy",
            Source::Hid => "hid",
        }
    }

    pub const ALL: [Source; 5] = [
        Source::Bluetooth,
        Source::Razer,
        Source::Logitech,
        Source::NuPhy,
        Source::Hid,
    ];
}

/// Stav zariadenia pri jednom citani.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    /// Zariadenie odpovedalo.
    Live,
    /// Prijimac je pritomny, ale zariadenie neodpoveda (spi, vypnute, mimo dosahu).
    Asleep,
}

/// Jedno citanie zo zdroja.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    /// Stabilny kluc zariadenia v ramci zdroja (napr. `razer:1532:00bf`); ten isty pri dalsom citani.
    pub key: String,
    pub name: String,
    pub kind: Kind,
    pub source: Source,
    /// 0..=100; `None` = zariadenie uroven teraz nehlasi.
    pub percent: Option<u8>,
    pub charging: bool,
    pub presence: Presence,
}

/// Zariadenie, ako ho ukazuje lista a panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peripheral {
    pub key: String,
    pub name: String,
    pub kind: Kind,
    pub source: Source,
    pub percent: Option<u8>,
    pub charging: bool,
    /// Zariadenie teraz neodpoveda; `percent` je posledna skutocna hodnota.
    pub stale: bool,
    /// Kolko sekund ubehlo od poslednej skutocnej hodnoty (pri `stale`; inak 0).
    pub age_secs: u64,
}

impl Peripheral {
    /// Uroven je nizka (pri nabijani nikdy).
    pub fn is_low(&self, threshold: u8) -> bool {
        !self.charging && self.percent.is_some_and(|p| p <= threshold)
    }
}

/// Stav sluzby.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Availability {
    /// Prve citanie este neskoncilo.
    #[default]
    Starting,
    Ready,
}

/// Snimka pre UI (lacna kopia cez `Arc`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub generation: u64,
    pub availability: Availability,
    /// Zoradene: druh, potom nazov (stabilne poradie, nic neskace pri zmene percent).
    pub devices: Vec<Peripheral>,
}

struct Entry {
    reading: Reading,
    /// Kedy prisla posledna skutocna (`Live`) hodnota; `None` = zatial ziadna (zariadenie od startu spi).
    live_at: Option<Instant>,
    /// Kedy zariadenie naposledy nahlasil nejaky zdroj (aj ako spiace).
    seen_at: Instant,
}

/// Pamat poslednych hodnot; pouziva ju vlakno sluzby.
pub struct Tracker {
    entries: Vec<Entry>,
    /// Zariadenie, ktore ziadny zdroj nehlasi dlhsie nez toto, sa zahodi.
    pub forget_after: Duration,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker {
            entries: Vec::new(),
            forget_after: Duration::from_secs(180),
        }
    }
}

impl Tracker {
    pub fn new(forget_after: Duration) -> Tracker {
        Tracker {
            entries: Vec::new(),
            forget_after,
        }
    }

    /// Zapracuje citanie vsetkych zdrojov z jedneho kola (`readings`) v case `now`.
    pub fn update(&mut self, readings: Vec<Reading>, now: Instant) {
        // To iste zariadenie moze v jednom kole hlasit viac ciest (prijimac "spi", kabel odpoveda): plati zive.
        let mut merged: Vec<Reading> = Vec::with_capacity(readings.len());
        for r in readings {
            match merged.iter_mut().find(|m| m.key == r.key) {
                Some(m) if m.presence == Presence::Asleep && r.presence == Presence::Live => *m = r,
                Some(_) => {}
                None => merged.push(r),
            }
        }
        // zariadenie, ktore v tomto kole nehlasil ziadny zdroj (prijimac vytiahnuty, Bluetooth vypnute), uz nie je
        // zive: ukaze sa posledna hodnota ako spiaca, kym sa po `forget_after` nezabudne
        for e in self.entries.iter_mut() {
            if !merged.iter().any(|m| m.key == e.reading.key) {
                e.reading.presence = Presence::Asleep;
                e.reading.charging = false;
            }
        }
        for r in merged {
            match self.entries.iter_mut().find(|e| e.reading.key == r.key) {
                Some(e) => {
                    e.seen_at = now;
                    match r.presence {
                        Presence::Live => {
                            e.reading = r;
                            e.live_at = Some(now);
                        }
                        Presence::Asleep => {
                            // posledna hodnota ostava, nabijanie uz nevieme potvrdit
                            e.reading.presence = Presence::Asleep;
                            e.reading.charging = false;
                            if !r.name.is_empty() && e.live_at.is_none() {
                                e.reading.name = r.name;
                            }
                        }
                    }
                }
                None => self.entries.push(Entry {
                    live_at: (r.presence == Presence::Live).then_some(now),
                    seen_at: now,
                    reading: r,
                }),
            }
        }
        let forget = self.forget_after;
        self.entries
            .retain(|e| now.saturating_duration_since(e.seen_at) < forget);
        // spiace zariadenie bez jedinej skutocnej hodnoty nema co ukazat
        self.entries
            .retain(|e| e.live_at.is_some() || e.reading.presence == Presence::Live);
    }

    /// Stav na zobrazenie v case `now` (zoradeny).
    pub fn devices(&self, now: Instant) -> Vec<Peripheral> {
        let mut out: Vec<Peripheral> = self
            .entries
            .iter()
            .map(|e| {
                let stale = e.reading.presence == Presence::Asleep;
                Peripheral {
                    key: e.reading.key.clone(),
                    name: e.reading.name.clone(),
                    kind: e.reading.kind,
                    source: e.reading.source,
                    percent: e.reading.percent,
                    charging: e.reading.charging && !stale,
                    stale,
                    age_secs: if stale {
                        e.live_at
                            .map(|t| now.saturating_duration_since(t).as_secs())
                            .unwrap_or(0)
                    } else {
                        0
                    },
                }
            })
            .collect();
        out.sort_by(|a, b| {
            a.kind
                .cmp(&b.kind)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.key.cmp(&b.key))
        });
        out
    }
}

/// Druh podla nazvu (Bluetooth nehlasi druh spolahlivo; len pre ikonu).
pub fn kind_from_name(name: &str) -> Kind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["controller", "gamepad", "joy-con", "dualsense", "dualshock"]) {
        Kind::Controller
    } else if has(&[
        "mouse",
        "mx master",
        "mx anywhere",
        "deathadder",
        "viper",
        "basilisk",
    ]) {
        Kind::Mouse
    } else if has(&["keyboard", "keys", "k380", "nuphy"]) {
        Kind::Keyboard
    } else if has(&[
        "headset",
        "headphone",
        "buds",
        "airpods",
        "earbuds",
        "wh-",
        "wf-",
        "arctis",
        "hyperx",
    ]) {
        Kind::Headset
    } else {
        Kind::Other
    }
}

/// Text urovne na liste: `64 %`; ked zariadenie uroven nehlasi, pomlcka.
pub fn short_label(d: &Peripheral) -> String {
    match d.percent {
        Some(p) => format!("{p} %"),
        None => "–".into(),
    }
}

/// Zariadenia na lištu: najviac `max` (najnizsia uroven prva, ak `lowest_first`), plus pocet ostatnych.
pub fn for_bar(
    devices: &[Peripheral],
    max: usize,
    lowest_first: bool,
) -> (Vec<&Peripheral>, usize) {
    let mut v: Vec<&Peripheral> = devices.iter().collect();
    if lowest_first {
        // nabijajuce a bez hodnoty az na koniec; potom najnizsia uroven
        v.sort_by_key(|d| (d.charging || d.percent.is_none(), d.percent.unwrap_or(101)));
    }
    let rest = v.len().saturating_sub(max);
    v.truncate(max);
    (v, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(key: &str, pct: Option<u8>, charging: bool, presence: Presence) -> Reading {
        Reading {
            key: key.into(),
            name: format!("Dev {key}"),
            kind: Kind::Mouse,
            source: Source::Razer,
            percent: pct,
            charging,
            presence,
        }
    }

    #[test]
    fn asleep_keeps_last_value_and_marks_stale() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.update(vec![r("a", Some(64), true, Presence::Live)], t0);
        let d = t.devices(t0);
        assert_eq!(d[0].percent, Some(64));
        assert!(d[0].charging && !d[0].stale);

        let t1 = t0 + Duration::from_secs(30);
        t.update(vec![r("a", None, false, Presence::Asleep)], t1);
        let d = t.devices(t1);
        assert_eq!(d[0].percent, Some(64), "posledna skutocna hodnota");
        assert!(
            d[0].stale && !d[0].charging,
            "spiace zariadenie sa nenabija"
        );
        assert_eq!(d[0].age_secs, 30);
    }

    #[test]
    fn live_path_wins_over_sleeping_receiver_in_one_round() {
        // Razer pocas nabijania: prijimac hlasi timeout, kabel odpoveda (poradie v kole nesmie rozhodovat)
        let t0 = Instant::now();
        for order in [false, true] {
            let mut t = Tracker::default();
            let mut v = vec![
                r("a", None, false, Presence::Asleep),
                r("a", Some(64), true, Presence::Live),
            ];
            if order {
                v.reverse();
            }
            t.update(v, t0);
            let d = t.devices(t0);
            assert_eq!(d.len(), 1);
            assert_eq!(d[0].percent, Some(64));
            assert!(d[0].charging && !d[0].stale);
        }
    }

    #[test]
    fn device_asleep_since_start_is_not_shown() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        t.update(vec![r("a", None, false, Presence::Asleep)], t0);
        assert!(t.devices(t0).is_empty());
    }

    #[test]
    fn unreported_device_is_forgotten_after_timeout() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut t = Tracker::default();
        t.update(vec![r("a", Some(50), true, Presence::Live)], t0);
        t.update(vec![], at(179));
        let d = t.devices(at(179));
        assert_eq!(d.len(), 1);
        // nehlasene v poslednom kole = uz nie zive: posledna hodnota, spiace, bez nabijania
        assert!(d[0].stale && !d[0].charging, "{d:?}");
        assert_eq!((d[0].percent, d[0].age_secs), (Some(50), 179));
        t.update(vec![], at(181));
        assert!(t.devices(at(181)).is_empty());
    }

    #[test]
    fn device_that_comes_back_is_live_again() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut t = Tracker::default();
        t.update(vec![r("a", Some(50), false, Presence::Live)], t0);
        t.update(vec![], at(60));
        assert!(t.devices(at(60))[0].stale);
        t.update(vec![r("a", Some(49), true, Presence::Live)], at(70));
        let d = t.devices(at(70));
        assert!(!d[0].stale && d[0].charging);
        assert_eq!(d[0].percent, Some(49));
    }

    #[test]
    fn live_value_replaces_and_sorting_is_stable() {
        let t0 = Instant::now();
        let mut t = Tracker::default();
        let mut kb = r("k", Some(90), false, Presence::Live);
        kb.kind = Kind::Keyboard;
        kb.name = "NuPhy WH80".into();
        let mut m = r("m", Some(20), false, Presence::Live);
        m.name = "zeta mouse".into();
        let mut m2 = r("m2", Some(70), false, Presence::Live);
        m2.name = "Alpha mouse".into();
        t.update(vec![kb, m, m2], t0);
        let names: Vec<String> = t.devices(t0).into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["Alpha mouse", "zeta mouse", "NuPhy WH80"]);
        t.update(vec![r("m", Some(19), false, Presence::Live)], t0);
        assert_eq!(
            t.devices(t0).iter().find(|d| d.key == "m").unwrap().percent,
            Some(19)
        );
    }

    #[test]
    fn bar_selection_prefers_lowest_and_counts_rest() {
        let mk = |k: &str, p: Option<u8>, c: bool| Peripheral {
            key: k.into(),
            name: k.into(),
            kind: Kind::Other,
            source: Source::Bluetooth,
            percent: p,
            charging: c,
            stale: false,
            age_secs: 0,
        };
        let all = vec![
            mk("a", Some(80), false),
            mk("b", Some(10), false),
            mk("c", Some(5), true),
            mk("d", None, true),
        ];
        let (v, rest) = for_bar(&all, 2, true);
        assert_eq!(
            v.iter().map(|d| d.key.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(rest, 2);
        let (v, rest) = for_bar(&all, 9, false);
        assert_eq!(v.len(), 4);
        assert_eq!(rest, 0);
        assert!(all[1].is_low(15) && !all[2].is_low(15) && !all[0].is_low(15));
    }

    #[test]
    fn kinds_and_labels() {
        assert_eq!(kind_from_name("Xbox Wireless Controller"), Kind::Controller);
        assert_eq!(kind_from_name("Razer DeathAdder V4 Pro"), Kind::Mouse);
        assert_eq!(kind_from_name("WH-1000XM5"), Kind::Headset);
        assert_eq!(kind_from_name("Logitech K380 Keys"), Kind::Keyboard);
        assert_eq!(kind_from_name("Something"), Kind::Other);
        let mut d = Peripheral {
            key: "x".into(),
            name: "x".into(),
            kind: Kind::Mouse,
            source: Source::Razer,
            percent: Some(7),
            charging: false,
            stale: false,
            age_secs: 0,
        };
        assert_eq!(short_label(&d), "7 %");
        d.percent = None;
        assert_eq!(short_label(&d), "–");
    }
}
