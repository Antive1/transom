//! Opakujuce sa kalendarove udalosti (zaciatok limitu / fakturacia) ako **ciste datumove pravidla**: pocita sa len s
//! kalendarnymi dnami (`Ymd`), nikdy s casom ani casovym pasmom, takze letny cas ani zmena pasma datum neposunie
//! (24 alebo 25-hodinovy den je stale jeden den). Pravidlo ma kotvu (znamy datum) a opakovanie:
//! - `Once`: len kotva,
//! - `EveryDays(n)`: kotva + k * n dni (k >= 0; napr. reset kazdych 6 dni),
//! - `Monthly`: rovnaky den v mesiaci ako kotva; v kratsom mesiaci posledny den mesiaca (31. -> 28./29./30.), den kotvy
//!   sa pritom **nikdy nepamata skrateny** (po februari je zasa 31.).
//!
//! Udalosti pred kotvou sa nezobrazuju (pravidlo zacina kotvou).
use crate::clock::{civil_from_days, days_from_civil, days_in_month, Ymd};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Once,
    EveryDays(u32),
    Monthly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub anchor: Ymd,
    pub repeat: Repeat,
}

/// Najvyssi pocet dni medzi opakovaniami (rok); vyssie hodnoty sa orezu.
pub const MAX_EVERY_DAYS: u32 = 366;
/// Poistka na pocet udalosti vo vyseku.
const MAX_OCCURRENCES: usize = 500;

fn day_number(d: Ymd) -> i64 {
    days_from_civil(d)
}

/// Platny kalendarny den (rok 2000-2100, mesiac 1-12, den v rozsahu mesiaca).
pub fn is_valid_date(d: Ymd) -> bool {
    (2000..=2100).contains(&d.y)
        && (1..=12).contains(&d.m)
        && d.d >= 1
        && d.d <= days_in_month(d.y, d.m)
}

/// Rozparsuje datum: `YYYY-MM-DD`, `D.M.YYYY` alebo `D/M/YYYY` (aj s medzerami okolo). `None` pre neplatny den
/// (30. 2., 31. 4., mesiac 13, ...); nikdy sa nic neopravuje ani nezaokruhluje.
pub fn parse_date(text: &str) -> Option<Ymd> {
    let t = text.trim();
    let nums = |sep: char| -> Option<Vec<i32>> {
        let parts: Vec<&str> = t.split(sep).map(str::trim).collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        parts.iter().map(|p| p.parse::<i32>().ok()).collect()
    };
    let d = if let Some(v) = nums('-') {
        // ISO: rok musi mat 4 cislice
        if t.split('-').next().map(str::len) != Some(4) {
            return None;
        }
        Ymd::new(v[0], v[1] as u32, v[2] as u32)
    } else {
        let v = nums('.').or_else(|| nums('/'))?;
        if t.rsplit(['.', '/']).next().map(str::len) != Some(4) {
            return None;
        }
        Ymd::new(v[2], v[1] as u32, v[0] as u32)
    };
    is_valid_date(d).then_some(d)
}

/// Pravidlo z ulozenych retazcov (`date` ISO, `repeat` = `none` / `days` / `monthly`, `every_days`); `None` pre neplatny
/// vstup. Spolocne pre kalendar aj nastavenia.
pub fn rule_from_parts(date: &str, repeat: &str, every_days: u32) -> Option<Rule> {
    let repeat = match repeat {
        "none" => Repeat::Once,
        "days" => Repeat::EveryDays(every_days),
        "monthly" => Repeat::Monthly,
        _ => return None,
    };
    Some(Rule::new(parse_date(date)?, repeat))
}

/// ISO zapis `YYYY-MM-DD`.
pub fn iso(d: Ymd) -> String {
    format!("{:04}-{:02}-{:02}", d.y, d.m, d.d)
}

fn month_occurrence(anchor: Ymd, y: i32, m: u32) -> Ymd {
    Ymd::new(y, m, anchor.d.min(days_in_month(y, m)))
}

impl Rule {
    pub fn new(anchor: Ymd, repeat: Repeat) -> Self {
        let repeat = match repeat {
            Repeat::EveryDays(n) => Repeat::EveryDays(n.clamp(1, MAX_EVERY_DAYS)),
            r => r,
        };
        Rule { anchor, repeat }
    }

    /// Je `d` dnom udalosti?
    pub fn occurs_on(&self, d: Ymd) -> bool {
        if d < self.anchor {
            return false;
        }
        match self.repeat {
            Repeat::Once => d == self.anchor,
            Repeat::EveryDays(n) => {
                (day_number(d) - day_number(self.anchor)) % i64::from(n.max(1)) == 0
            }
            Repeat::Monthly => d == month_occurrence(self.anchor, d.y, d.m),
        }
    }

    /// Prvy vyskyt v den `from` alebo neskor (`None` = jednorazova udalost uz minula).
    pub fn next_on_or_after(&self, from: Ymd) -> Option<Ymd> {
        let from = from.max(self.anchor);
        match self.repeat {
            Repeat::Once => (self.anchor >= from).then_some(self.anchor),
            Repeat::EveryDays(n) => {
                let n = i64::from(n.max(1));
                let diff = day_number(from) - day_number(self.anchor);
                let k = (diff + n - 1).div_euclid(n);
                Some(civil_from_days(day_number(self.anchor) + k * n))
            }
            Repeat::Monthly => {
                let (mut y, mut m) = (from.y, from.m);
                for _ in 0..2 {
                    let c = month_occurrence(self.anchor, y, m);
                    if c >= from {
                        return Some(c);
                    }
                    (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                }
                None
            }
        }
    }

    /// Vsetky vyskyty v dnoch `from..=to` (vzostupne, najviac `MAX_OCCURRENCES`).
    pub fn occurrences(&self, from: Ymd, to: Ymd) -> Vec<Ymd> {
        let mut out = Vec::new();
        let mut cur = self.next_on_or_after(from);
        while let Some(d) = cur {
            if d > to || out.len() >= MAX_OCCURRENCES {
                break;
            }
            out.push(d);
            cur = match self.repeat {
                Repeat::Once => None,
                _ => self.next_on_or_after(civil_from_days(day_number(d) + 1)),
            };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Ymd {
        Ymd::new(y, m, day)
    }

    #[test]
    fn parsing_is_strict_and_never_corrects_dates() {
        assert_eq!(parse_date("2026-11-20"), Some(d(2026, 11, 20)));
        assert_eq!(parse_date(" 2026-11-20 "), Some(d(2026, 11, 20)));
        assert_eq!(parse_date("20.11.2026"), Some(d(2026, 11, 20)));
        assert_eq!(parse_date("5/3/2027"), Some(d(2027, 3, 5)));
        for bad in [
            "",
            "2026-13-01",
            "2026-02-30",
            "2026-04-31",
            "2025-02-29",
            "26-11-20",
            "2026-1-1-1",
            "20.11.26",
            "abc",
            "1999-12-31",
            "2101-01-01",
            "2026-00-10",
            "2026-10-00",
            "2026/11/20",
            "2026-11-",
            "-",
        ] {
            assert_eq!(parse_date(bad), None, "{bad:?}");
        }
        assert_eq!(
            parse_date("2028-02-29"),
            Some(d(2028, 2, 29)),
            "priestupny rok"
        );
        assert_eq!(iso(d(2026, 3, 5)), "2026-03-05");
        assert_eq!(parse_date(&iso(d(2026, 3, 5))), Some(d(2026, 3, 5)));
    }

    #[test]
    fn once_occurs_only_on_its_anchor() {
        let r = Rule::new(d(2026, 12, 1), Repeat::Once);
        assert!(
            r.occurs_on(d(2026, 12, 1))
                && !r.occurs_on(d(2026, 12, 2))
                && !r.occurs_on(d(2026, 11, 30))
        );
        assert_eq!(r.next_on_or_after(d(2026, 11, 20)), Some(d(2026, 12, 1)));
        assert_eq!(r.next_on_or_after(d(2026, 12, 1)), Some(d(2026, 12, 1)));
        assert_eq!(r.next_on_or_after(d(2026, 12, 2)), None, "minula");
        assert_eq!(
            r.occurrences(d(2026, 1, 1), d(2027, 1, 1)),
            vec![d(2026, 12, 1)]
        );
    }

    #[test]
    fn six_day_recurrence_is_unambiguous_across_months_years_and_leap_days() {
        // reset kazdych 6 dni od 2026-11-20
        let r = Rule::new(d(2026, 11, 20), Repeat::EveryDays(6));
        assert_eq!(
            r.occurrences(d(2026, 11, 1), d(2026, 12, 31)),
            vec![
                d(2026, 11, 20),
                d(2026, 11, 26),
                d(2026, 12, 2),
                d(2026, 12, 8),
                d(2026, 12, 14),
                d(2026, 12, 20),
                d(2026, 12, 26)
            ]
        );
        // dalsi vyskyt: v den udalosti je to ten den, den po nej az dalsi
        assert_eq!(r.next_on_or_after(d(2026, 11, 26)), Some(d(2026, 11, 26)));
        assert_eq!(r.next_on_or_after(d(2026, 11, 27)), Some(d(2026, 12, 2)));
        // pred kotvou: zacina kotvou
        assert_eq!(r.next_on_or_after(d(2026, 1, 1)), Some(d(2026, 11, 20)));
        assert!(!r.occurs_on(d(2026, 11, 14)), "pred kotvou sa nezobrazuje");
        // cez prelom roka a priestupny den: 2027-12-30 + 6 = 2028-01-05; +... 2028-02-29 je vyskyt ak (dni % 6 == 0)
        let r2 = Rule::new(d(2027, 12, 30), Repeat::EveryDays(6));
        assert!(r2.occurs_on(d(2028, 1, 5)));
        let diff = days_from_civil(d(2028, 2, 29)) - days_from_civil(d(2027, 12, 30));
        assert_eq!(r2.occurs_on(d(2028, 2, 29)), diff % 6 == 0);
        // kazdy vyskyt je presne 6 dni po predchadzajucom (ziaden posun pri prechode letneho casu: pocita sa len s dnami)
        let all = r2.occurrences(d(2027, 12, 1), d(2028, 6, 30));
        for w in all.windows(2) {
            assert_eq!(days_from_civil(w[1]) - days_from_civil(w[0]), 6);
        }
        // interval sa orezava na 1..=366
        assert_eq!(
            Rule::new(d(2026, 1, 1), Repeat::EveryDays(0)).repeat,
            Repeat::EveryDays(1)
        );
        assert_eq!(
            Rule::new(d(2026, 1, 1), Repeat::EveryDays(9999)).repeat,
            Repeat::EveryDays(366)
        );
    }

    #[test]
    fn dst_transition_days_are_ordinary_calendar_days() {
        // EU letny cas 2027-03-28 (23 h den) a 2027-10-31 (25 h den); USA 2027-03-14 a 2027-11-07: pravidlo pracuje len s datumami
        let r = Rule::new(d(2027, 3, 26), Repeat::EveryDays(1));
        let v = r.occurrences(d(2027, 3, 26), d(2027, 3, 31));
        assert_eq!(
            v,
            vec![
                d(2027, 3, 26),
                d(2027, 3, 27),
                d(2027, 3, 28),
                d(2027, 3, 29),
                d(2027, 3, 30),
                d(2027, 3, 31)
            ]
        );
        let r = Rule::new(d(2027, 10, 30), Repeat::EveryDays(1));
        assert_eq!(r.occurrences(d(2027, 10, 30), d(2027, 11, 2)).len(), 4);
        // mesacne s kotvou na DST dni
        let m = Rule::new(d(2027, 3, 28), Repeat::Monthly);
        assert!(m.occurs_on(d(2027, 4, 28)) && m.occurs_on(d(2027, 10, 28)));
    }

    #[test]
    fn monthly_clamps_to_the_last_day_but_remembers_the_anchor_day() {
        let r = Rule::new(d(2027, 1, 31), Repeat::Monthly);
        let v = r.occurrences(d(2027, 1, 1), d(2027, 6, 30));
        assert_eq!(
            v,
            vec![
                d(2027, 1, 31),
                d(2027, 2, 28),
                d(2027, 3, 31),
                d(2027, 4, 30),
                d(2027, 5, 31),
                d(2027, 6, 30)
            ]
        );
        // priestupny rok: 29. 2.
        let v =
            Rule::new(d(2028, 1, 31), Repeat::Monthly).occurrences(d(2028, 2, 1), d(2028, 3, 31));
        assert_eq!(v, vec![d(2028, 2, 29), d(2028, 3, 31)]);
        // bezny den
        let r = Rule::new(d(2026, 11, 5), Repeat::Monthly);
        assert_eq!(r.next_on_or_after(d(2026, 11, 6)), Some(d(2026, 12, 5)));
        assert_eq!(
            r.next_on_or_after(d(2026, 12, 31)),
            Some(d(2027, 1, 5)),
            "cez prelom roka"
        );
        assert_eq!(r.next_on_or_after(d(2026, 11, 5)), Some(d(2026, 11, 5)));
        assert!(!r.occurs_on(d(2026, 10, 5)), "pred kotvou nie");
        // 31. v mesiaci s 30 dnami sa neopakuje dvakrat
        let r = Rule::new(d(2026, 10, 31), Repeat::Monthly);
        assert!(r.occurs_on(d(2026, 11, 30)) && !r.occurs_on(d(2026, 11, 29)));
        // next_on_or_after a occurs_on su konzistentne pre kazdy den roka
        for r in [
            Rule::new(d(2026, 1, 31), Repeat::Monthly),
            Rule::new(d(2026, 3, 1), Repeat::EveryDays(6)),
            Rule::new(d(2026, 5, 15), Repeat::Once),
        ] {
            let mut day = d(2026, 1, 1);
            for _ in 0..800 {
                let next = r.next_on_or_after(day);
                let expect = (0..800)
                    .map(|k| civil_from_days(days_from_civil(day) + k))
                    .find(|c| r.occurs_on(*c));
                assert_eq!(next, expect, "{r:?} od {day:?}");
                day = civil_from_days(days_from_civil(day) + 37);
            }
        }
    }
}
