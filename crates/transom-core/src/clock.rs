//! Hodiny a kalendar bez Win32: volby zobrazenia, uprava formatu casu z jazykoveho nastavenia Windows,
//! predvolby a mriezka mesiaca. Formatovanie samotne (nazvy dni/mesiacov, poradie datumu) robi Windows
//! (`GetTimeFormatEx`, `GetDateFormatEx`) v `transom-platform`.

/// 12/24 hodin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HourMode {
    /// Podla jazykoveho nastavenia Windows.
    #[default]
    Locale,
    H12,
    H24,
}

/// Uvodna nula pri hodine (napr. 07:05 vs 7:05).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeadingZero {
    #[default]
    Locale,
    Yes,
    No,
}

/// Datum vedla casu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateMode {
    #[default]
    None,
    /// Kratky datum podla jazykoveho nastavenia (`DATE_SHORTDATE`).
    Short,
    /// Dlhy datum podla jazykoveho nastavenia (`DATE_LONGDATE`).
    Long,
}

/// Prvy den tyzdna v kalendari.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FirstDay {
    /// `LOCALE_IFIRSTDAYOFWEEK` jazykoveho nastavenia.
    #[default]
    Locale,
    Monday,
    Sunday,
    Saturday,
}

impl FirstDay {
    /// Den v cislovani `LOCALE_IFIRSTDAYOFWEEK` (0 = pondelok ... 6 = nedela); `locale` sa pouzije pri `Locale`.
    pub fn resolve(self, locale: u8) -> u8 {
        match self {
            FirstDay::Locale => locale.min(6),
            FirstDay::Monday => 0,
            FirstDay::Saturday => 5,
            FirstDay::Sunday => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockOptions {
    pub hour: HourMode,
    pub leading_zero: LeadingZero,
    pub seconds: bool,
    /// Skrateny nazov dna (napr. "pi").
    pub weekday: bool,
    pub date: DateMode,
}

impl ClockOptions {
    /// Ako casto sa meni zobrazeny text: so sekundami kazdu sekundu, inak kazdu minutu.
    pub fn tick_seconds(&self) -> bool {
        self.seconds
    }
}

/// Predvolby pre nastavenia: (kluc prekladu, volby).
pub fn presets() -> Vec<(&'static str, ClockOptions)> {
    let base = ClockOptions::default();
    vec![
        ("preset.time", base),
        (
            "preset.seconds",
            ClockOptions {
                seconds: true,
                ..base
            },
        ),
        (
            "preset.weekday",
            ClockOptions {
                weekday: true,
                ..base
            },
        ),
        (
            "preset.date",
            ClockOptions {
                weekday: true,
                date: DateMode::Short,
                ..base
            },
        ),
        (
            "preset.long_date",
            ClockOptions {
                date: DateMode::Long,
                ..base
            },
        ),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Lit(String),
    Field(char, usize),
}

fn tokenize(fmt: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let cs: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\'' {
            // citovany literal az po dalsi apostrof (vratane)
            let mut j = i + 1;
            while j < cs.len() && cs[j] != '\'' {
                j += 1;
            }
            let end = (j + 1).min(cs.len());
            out.push(Tok::Lit(cs[i..end].iter().collect()));
            i = end;
        } else if matches!(c, 'h' | 'H' | 'm' | 's' | 't') {
            let mut j = i;
            while j < cs.len() && cs[j] == c {
                j += 1;
            }
            out.push(Tok::Field(c, j - i));
            i = j;
        } else {
            match out.last_mut() {
                Some(Tok::Lit(s)) if !s.starts_with('\'') => s.push(c),
                _ => out.push(Tok::Lit(c.to_string())),
            }
            i += 1;
        }
    }
    out
}

/// Obrazok formatu casu pre `GetTimeFormatEx` odvodeny z formatu jazykoveho nastavenia (`LOCALE_STIMEFORMAT`,
/// napr. `H:mm:ss`, `h:mm:ss tt`, `tt h:mm:ss`, `H.mm.ss`) podla volieb: zachova oddelovace a poradie locale,
/// zmeni len 12/24 h, uvodnu nulu, AM/PM a sekundy.
pub fn time_picture(locale_fmt: &str, o: &ClockOptions) -> String {
    let mut toks = tokenize(if locale_fmt.trim().is_empty() {
        "H:mm:ss"
    } else {
        locale_fmt
    });
    let locale_12 = toks.iter().any(|t| matches!(t, Tok::Field('h', _)));
    let want_12 = match o.hour {
        HourMode::Locale => locale_12,
        HourMode::H12 => true,
        HourMode::H24 => false,
    };
    for t in toks.iter_mut() {
        if let Tok::Field(c @ ('h' | 'H'), n) = t {
            let zero = match o.leading_zero {
                LeadingZero::Locale => *n >= 2,
                LeadingZero::Yes => true,
                LeadingZero::No => false,
            };
            *c = if want_12 { 'h' } else { 'H' };
            *n = if zero { 2 } else { 1 };
        }
    }
    // sekundy: odstranit pole `s` aj oddelovac pred nim
    if !o.seconds {
        while let Some(i) = toks.iter().position(|t| matches!(t, Tok::Field('s', _))) {
            toks.remove(i);
            if i > 0 && matches!(toks.get(i - 1), Some(Tok::Lit(l)) if !l.starts_with('\'')) {
                toks.remove(i - 1);
            }
        }
    } else if !toks.iter().any(|t| matches!(t, Tok::Field('s', _))) {
        if let Some(i) = toks.iter().position(|t| matches!(t, Tok::Field('m', _))) {
            toks.insert(i + 1, Tok::Lit(":".into()));
            toks.insert(i + 2, Tok::Field('s', 2));
        }
    }
    let has_tt = toks.iter().any(|t| matches!(t, Tok::Field('t', _)));
    if want_12 && !has_tt {
        toks.push(Tok::Lit(" ".into()));
        toks.push(Tok::Field('t', 2));
    } else if !want_12 && has_tt {
        while let Some(i) = toks.iter().position(|t| matches!(t, Tok::Field('t', _))) {
            toks.remove(i);
            // medzera okolo AM/PM
            if matches!(toks.get(i), Some(Tok::Lit(l)) if l.trim().is_empty()) {
                toks.remove(i);
            } else if i > 0 && matches!(toks.get(i - 1), Some(Tok::Lit(l)) if l.trim().is_empty()) {
                toks.remove(i - 1);
            }
        }
    }
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Lit(l) => s.push_str(&l),
            Tok::Field(c, n) => s.extend(std::iter::repeat_n(c, n)),
        }
    }
    s.trim().to_string()
}

/// Spojenie casti textu hodin: [den] [datum] [cas], prazdne casti sa vynechaju.
pub fn compose(weekday: Option<&str>, date: Option<&str>, time: &str) -> String {
    [weekday, date, Some(time)]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Kalendarny den.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ymd {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Ymd {
    pub const fn new(y: i32, m: u32, d: u32) -> Self {
        Ymd { y, m, d }
    }
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 30,
    }
}

/// Pocet dni od 1970-01-01 (algoritmus "days from civil", proleptický gregoriansky kalendar).
pub fn days_from_civil(d: Ymd) -> i64 {
    let y = if d.m <= 2 { d.y - 1 } else { d.y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = d.m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d.d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> Ymd {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    Ymd::new(if m <= 2 { y + 1 } else { y } as i32, m, d)
}

/// Den v tyzdni v cislovani `LOCALE_IFIRSTDAYOFWEEK`: 0 = pondelok ... 6 = nedela.
pub fn weekday(d: Ymd) -> u8 {
    // 1970-01-01 bol stvrtok (= 3)
    ((days_from_civil(d) + 3).rem_euclid(7)) as u8
}

/// Mesiac posunuty o `delta` (napr. -1 = predchadzajuci).
pub fn add_months(y: i32, m: u32, delta: i32) -> (i32, u32) {
    let idx = y * 12 + (m as i32 - 1) + delta;
    (idx.div_euclid(12), (idx.rem_euclid(12) + 1) as u32)
}

/// Mriezka mesiaca: 6 tyzdnov x 7 dni, zacina dnom `first` (0 = pondelok ... 6 = nedela). Dni mimo mesiaca su
/// z predchadzajuceho/nasledujuceho mesiaca (`Ymd::m` sa lisi od `m`).
pub fn month_grid(y: i32, m: u32, first: u8) -> Vec<Ymd> {
    let first_of_month = Ymd::new(y, m, 1);
    let offset = (weekday(first_of_month) as i64 - first.min(6) as i64).rem_euclid(7);
    let start = days_from_civil(first_of_month) - offset;
    (0..42).map(|i| civil_from_days(start + i)).collect()
}

/// Poradie dni v hlavicke kalendara (0 = pondelok ... 6 = nedela) od `first`.
pub fn header_order(first: u8) -> [u8; 7] {
    let mut out = [0u8; 7];
    for (i, o) in out.iter_mut().enumerate() {
        *o = ((first.min(6) as usize + i) % 7) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> ClockOptions {
        ClockOptions::default()
    }

    #[test]
    fn locale_defaults_drop_seconds_only() {
        assert_eq!(time_picture("H:mm:ss", &o()), "H:mm");
        assert_eq!(time_picture("HH:mm:ss", &o()), "HH:mm");
        assert_eq!(time_picture("h:mm:ss tt", &o()), "h:mm tt");
        assert_eq!(time_picture("tt h:mm:ss", &o()), "tt h:mm");
        assert_eq!(time_picture("H.mm.ss", &o()), "H.mm");
        assert_eq!(time_picture("", &o()), "H:mm");
    }

    #[test]
    fn force_12_and_24_hours() {
        let h12 = ClockOptions {
            hour: HourMode::H12,
            ..o()
        };
        let h24 = ClockOptions {
            hour: HourMode::H24,
            ..o()
        };
        assert_eq!(time_picture("H:mm:ss", &h12), "h:mm tt");
        assert_eq!(time_picture("HH:mm:ss", &h12), "hh:mm tt");
        assert_eq!(time_picture("h:mm:ss tt", &h24), "H:mm");
        assert_eq!(time_picture("tt h:mm:ss", &h24), "H:mm");
    }

    #[test]
    fn leading_zero_and_seconds() {
        let z = ClockOptions {
            leading_zero: LeadingZero::Yes,
            seconds: true,
            ..o()
        };
        assert_eq!(time_picture("H:mm:ss", &z), "HH:mm:ss");
        let nz = ClockOptions {
            leading_zero: LeadingZero::No,
            ..o()
        };
        assert_eq!(time_picture("HH:mm:ss", &nz), "H:mm");
        // sekundy chybaju v locale -> doplnit za minuty
        let s = ClockOptions {
            seconds: true,
            ..o()
        };
        assert_eq!(time_picture("H:mm", &s), "H:mm:ss");
        assert!(s.tick_seconds() && !o().tick_seconds());
    }

    #[test]
    fn quoted_literals_survive() {
        assert_eq!(time_picture("H' h 'mm' min 'ss", &o()), "H' h 'mm' min '");
    }

    #[test]
    fn compose_skips_empty_parts() {
        assert_eq!(
            compose(Some("pi"), Some("20. 11. 2026"), "14:05"),
            "pi 20. 11. 2026 14:05"
        );
        assert_eq!(compose(None, None, "14:05"), "14:05");
        assert_eq!(compose(Some(" "), None, "1:05 PM"), "1:05 PM");
    }

    #[test]
    fn weekday_and_dates() {
        assert_eq!(weekday(Ymd::new(1970, 1, 1)), 3); // stvrtok
        assert_eq!(weekday(Ymd::new(2000, 1, 1)), 5); // sobota
        assert_eq!(weekday(Ymd::new(2026, 11, 20)), 4); // piatok
        assert_eq!(weekday(Ymd::new(2026, 11, 1)), 6); // nedela
        for z in [-800_000, -1, 0, 1, 20_412, 100_000] {
            assert_eq!(days_from_civil(civil_from_days(z)), z);
        }
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(add_months(2026, 1, -1), (2025, 12));
        assert_eq!(add_months(2026, 12, 1), (2027, 1));
        assert_eq!(add_months(2026, 11, -23), (2024, 12));
    }

    #[test]
    fn grid_follows_first_day_of_week() {
        // november 2026 zacina nedelou
        let mon = month_grid(2026, 11, 0);
        assert_eq!(mon.len(), 42);
        assert_eq!(mon[0], Ymd::new(2026, 10, 26), "pondelok pred 1. 11.");
        assert_eq!(mon[6], Ymd::new(2026, 11, 1));
        let sun = month_grid(2026, 11, 6);
        assert_eq!(sun[0], Ymd::new(2026, 11, 1), "nedela = prvy den");
        let sat = month_grid(2026, 11, 5);
        assert_eq!(sat[0], Ymd::new(2026, 10, 31));
        // kazdy riadok zacina zvolenym dnom
        for (first, g) in [(0u8, &mon), (6, &sun), (5, &sat)] {
            for row in 0..6 {
                assert_eq!(weekday(g[row * 7]), first);
            }
            assert!(g.iter().filter(|d| d.m == 11).count() == 30);
        }
        assert_eq!(header_order(6), [6, 0, 1, 2, 3, 4, 5]);
        assert_eq!(FirstDay::Locale.resolve(0), 0);
        assert_eq!(FirstDay::Sunday.resolve(0), 6);
        assert_eq!(FirstDay::Saturday.resolve(0), 5);
        assert_eq!(FirstDay::Locale.resolve(99), 6);
    }

    #[test]
    fn presets_are_distinct() {
        let p = presets();
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                assert_ne!(p[i].1, p[j].1);
            }
        }
    }
}
