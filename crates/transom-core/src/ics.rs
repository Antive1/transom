//! Citanie kalendara vo formate iCalendar (RFC 5545) z odkazu ICS (Google Kalendar "Tajna adresa vo formate iCal",
//! Outlook "Publikovat kalendar"). Cista logika bez siete a bez Win32.
//!
//! Podporovane: VEVENT (SUMMARY, DTSTART/DTEND/DURATION, celodenne udalosti, STATUS:CANCELLED), opakovanie RRULE
//! (FREQ DAILY/WEEKLY/MONTHLY/YEARLY, INTERVAL, COUNT, UNTIL, BYDAY aj s poradim "2TU" / "-1FR", BYMONTHDAY, BYMONTH),
//! EXDATE, RECURRENCE-ID (zmenena jedna instancia) a casove pasma z VTIMEZONE (STANDARD/DAYLIGHT s rocnym RRULE).
//! Cas bez pasma ("plavajuci") sa berie ako miestny cas PC (prevod dodava volajuci).
//!
//! Limity proti zlym alebo obrovskym suborom: najviac `MAX_EVENTS` udalosti a `MAX_OCCURRENCES` vyskytov jednej udalosti
//! v okne.
use crate::clock::{civil_from_days, days_from_civil, days_in_month, weekday, Ymd};

pub const MAX_EVENTS: usize = 5000;
pub const MAX_OCCURRENCES: usize = 1000;

/// Okamih zaciatku udalosti.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum When {
    /// Celodenna udalost (miestny den).
    Day(Ymd),
    /// Konkretny okamih (Unix sekundy UTC).
    At(i64),
}

/// Jeden vyskyt udalosti v okne.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub summary: String,
    pub start: When,
    /// Dlzka v sekundach (celodenna: pocet dni * 86400).
    pub duration: i64,
}

/// Miestny cas (rok, mesiac, den, hod, min, sek).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Local {
    pub day: Ymd,
    pub secs: u32,
}

impl Local {
    fn as_naive(self) -> i64 {
        days_from_civil(self.day) * 86_400 + i64::from(self.secs)
    }
    fn from_naive(n: i64) -> Local {
        Local {
            day: civil_from_days(n.div_euclid(86_400)),
            secs: n.rem_euclid(86_400) as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Dt {
    Date(Ymd),
    Utc(Local),
    Zoned(Local, Option<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Rule {
    freq: u8, // 1 den, 2 tyzden, 3 mesiac, 4 rok
    interval: u32,
    count: Option<u32>,
    until: Option<Dt>,
    byday: Vec<(i32, u8)>, // (poradie, 0=po..6=ne), poradie 0 = kazdy
    bymonthday: Vec<i32>,
    bymonth: Vec<u32>,
    /// Poradie vo vyskytoch jedneho obdobia (1 = prvy, -1 = posledny); Outlook tak zapisuje "posledny pracovny den".
    bysetpos: Vec<i32>,
}

#[derive(Clone, Debug, Default)]
struct Ev {
    uid: String,
    summary: String,
    start: Option<Dt>,
    end: Option<Dt>,
    duration: Option<i64>,
    rule: Option<Rule>,
    exdates: Vec<Dt>,
    recurrence_id: Option<Dt>,
    cancelled: bool,
}

#[derive(Clone, Debug, Default)]
struct ZonePart {
    start: Option<Local>,
    offset_to: i32,
    rule: Option<Rule>,
}

#[derive(Clone, Debug, Default)]
struct Zone {
    id: String,
    parts: Vec<ZonePart>,
}

/// Rozparsovany kalendar.
#[derive(Clone, Debug, Default)]
pub struct Calendar {
    events: Vec<Ev>,
    zones: Vec<Zone>,
}

fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// Parametre vlastnosti (meno, hodnota).
type Params = Vec<(String, String)>;

/// (meno, parametre, hodnota)
fn split_prop(line: &str) -> Option<(String, Params, String)> {
    // dvojbodka mimo uvodzoviek oddeluje hodnotu
    let mut in_q = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_q = !in_q,
            ':' if !in_q => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            ))
        })
        .collect();
    Some((name, params, value.to_string()))
}

fn unescape(v: &str) -> String {
    let mut out = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn num<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.parse().ok()
}

/// Cislo z ASCII cifier na bajtoch `a..b` (`None` mimo rozsahu, na hranici viacbajtoveho znaku aj pri inom znaku nez
/// cifra). Rezanie `&s[a..b]` by na hranici viacbajtoveho znaku padlo (cudzi subor zo siete).
fn digits<T: std::str::FromStr>(s: &str, a: usize, b: usize) -> Option<T> {
    let t = s.get(a..b)?;
    if t.is_empty() || !t.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    num(t)
}

/// Najviac poloziek v zoznamoch pravidla (BYDAY, BYMONTHDAY, BYMONTH): platne pravidla ich maju malo, obrovsky zoznam by
/// nasobil pracu kazdeho obdobia.
const MAX_RULE_LIST: usize = 64;
/// Najviac vynimiek (EXDATE) jednej udalosti.
const MAX_EXDATES: usize = 2000;
/// Najviac pasiem (VTIMEZONE) a ich casti.
const MAX_ZONES: usize = 200;
const MAX_ZONE_PARTS: usize = 100;
/// Rozpocet prace rozvinutia jednej opakovanej udalosti (obdobia + kandidati) a celeho volania `occurrences`. Platne
/// udalosti ho nedosiahnu (aj denna s COUNT na desatrocia), zlomyselny kalendar (stare DTSTART, obrovske COUNT, BYDAY na kazdy
/// den) by inak zasekol vlakno okna na minuty.
const MAX_EVENT_WORK: usize = 300_000;
const MAX_CALL_WORK: usize = 20_000_000;

fn parse_date(s: &str) -> Option<Ymd> {
    let y: i32 = digits(s, 0, 4)?;
    let m: u32 = digits(s, 4, 6)?;
    let d: u32 = digits(s, 6, 8)?;
    (1..=12)
        .contains(&m)
        .then_some(())
        .filter(|_| d >= 1 && d <= days_in_month(y, m))
        .map(|_| Ymd::new(y, m, d))
}

fn parse_local(s: &str) -> Option<Local> {
    let day = parse_date(s)?;
    if !matches!(s.as_bytes().get(8), Some(b'T' | b't')) {
        return None;
    }
    let (h, mi, se): (u32, u32, u32) = (digits(s, 9, 11)?, digits(s, 11, 13)?, digits(s, 13, 15)?);
    (h < 24 && mi < 60 && se < 61).then_some(Local {
        day,
        secs: h * 3600 + mi * 60 + se.min(59),
    })
}

fn parse_dt(value: &str, params: &[(String, String)]) -> Option<Dt> {
    let v = value.trim();
    let is_date = params
        .iter()
        .any(|(k, v)| k == "VALUE" && v.eq_ignore_ascii_case("DATE"))
        || (v.len() == 8 && !v.contains('T'));
    if is_date {
        return parse_date(v).map(Dt::Date);
    }
    let l = parse_local(v)?;
    if v.ends_with('Z') || v.ends_with('z') {
        return Some(Dt::Utc(l));
    }
    let tz = params
        .iter()
        .find(|(k, _)| k == "TZID")
        .map(|(_, v)| v.clone());
    Some(Dt::Zoned(l, tz))
}

fn parse_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, rest) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => (1, s),
    };
    let h: i32 = digits(rest, 0, 2)?;
    let m: i32 = digits(rest, 2, 4)?;
    let sec: i32 = digits(rest, 4, 6).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60 + sec))
}

fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix('P')?;
    let mut total = 0i64;
    let mut n = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            '0'..='9' => n.push(c),
            'T' => in_time = true,
            _ => {
                let v: i64 = num(&n)?;
                n.clear();
                let unit = match (c, in_time) {
                    ('W', _) => 7 * 86_400,
                    ('D', _) => 86_400,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                // obrovske hodnoty z cudzieho suboru nesmu pretiect
                total = total.checked_add(v.checked_mul(unit)?)?;
            }
        }
    }
    // dlzka nad 100 rokov je nezmysel; orezat, aby scitanie so zaciatkom nikdy nepretieklo
    let total = total.min(MAX_DURATION);
    Some(if neg { -total } else { total })
}

/// Najdlhsia dlzka udalosti (s): 100 rokov.
const MAX_DURATION: i64 = 100 * 366 * 86_400;

fn wd(s: &str) -> Option<u8> {
    Some(match s {
        "MO" => 0,
        "TU" => 1,
        "WE" => 2,
        "TH" => 3,
        "FR" => 4,
        "SA" => 5,
        "SU" => 6,
        _ => return None,
    })
}

fn parse_rule(v: &str) -> Option<Rule> {
    let mut r = Rule {
        interval: 1,
        ..Rule::default()
    };
    for part in v.split(';') {
        let (k, val) = part.split_once('=')?;
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                r.freq = match val.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => 1,
                    "WEEKLY" => 2,
                    "MONTHLY" => 3,
                    "YEARLY" => 4,
                    _ => return None,
                }
            }
            "INTERVAL" => r.interval = num::<u32>(val).unwrap_or(1).clamp(1, 1000),
            "COUNT" => r.count = num(val),
            "UNTIL" => r.until = parse_dt(val, &[]),
            "BYDAY" => {
                for d in val.split(',') {
                    let d = d.trim().to_ascii_uppercase();
                    // nazov dna su posledne dva ASCII znaky; ine (aj viacbajtove) sa preskocia
                    let Some((ord, name)) = d
                        .len()
                        .checked_sub(2)
                        .and_then(|i| d.get(..i).zip(d.get(i..)))
                    else {
                        continue;
                    };
                    let ord: i32 = if ord.is_empty() {
                        0
                    } else {
                        match num::<i32>(ord) {
                            // RFC 5545: poradie 1..=53 (od zaciatku) alebo -53..=-1 (od konca)
                            Some(n) if n != 0 && (-53..=53).contains(&n) => n,
                            _ => continue,
                        }
                    };
                    if let Some(w) = wd(name) {
                        if r.byday.len() < MAX_RULE_LIST && !r.byday.contains(&(ord, w)) {
                            r.byday.push((ord, w));
                        }
                    }
                }
            }
            "BYMONTHDAY" => {
                r.bymonthday = val
                    .split(',')
                    .filter_map(|x| num::<i32>(x.trim()))
                    .filter(|d| *d != 0 && (-31..=31).contains(d))
                    .take(MAX_RULE_LIST)
                    .collect()
            }
            "BYMONTH" => {
                // vzostupne: vyskyty jedneho roka musia ist po sebe (COUNT, koniec okna)
                let mut ms: Vec<u32> = val
                    .split(',')
                    .filter_map(|x| num(x.trim()))
                    .filter(|m| (1..=12).contains(m))
                    .take(MAX_RULE_LIST)
                    .collect();
                ms.sort_unstable();
                ms.dedup();
                r.bymonth = ms;
            }
            "BYSETPOS" => {
                r.bysetpos = val
                    .split(',')
                    .filter_map(|x| num::<i32>(x.trim()))
                    .filter(|p| *p != 0 && (-366..=366).contains(p))
                    .take(MAX_RULE_LIST)
                    .collect()
            }
            _ => {}
        }
    }
    (r.freq != 0).then_some(r)
}

/// Rozparsuje text ICS (tolerantne: chybne riadky a komponenty sa preskocia).
pub fn parse(text: &str) -> Calendar {
    let mut cal = Calendar::default();
    let mut ev: Option<Ev> = None;
    let mut zone: Option<Zone> = None;
    let mut part: Option<ZonePart> = None;
    let mut depth_other = 0u32;
    for line in unfold(text) {
        let Some((name, params, value)) = split_prop(&line) else {
            continue;
        };
        match (name.as_str(), value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if zone.is_none() => ev = Some(Ev::default()),
            ("END", "VEVENT") => {
                if let Some(e) = ev.take() {
                    if cal.events.len() < MAX_EVENTS && e.start.is_some() {
                        cal.events.push(e);
                    }
                }
            }
            ("BEGIN", "VTIMEZONE") => zone = Some(Zone::default()),
            ("END", "VTIMEZONE") => {
                if let Some(z) = zone.take() {
                    if cal.zones.len() < MAX_ZONES {
                        cal.zones.push(z);
                    }
                }
            }
            ("BEGIN", "STANDARD") | ("BEGIN", "DAYLIGHT") if zone.is_some() => {
                part = Some(ZonePart::default())
            }
            ("END", "STANDARD") | ("END", "DAYLIGHT") => {
                if let (Some(z), Some(p)) = (zone.as_mut(), part.take()) {
                    if z.parts.len() < MAX_ZONE_PARTS {
                        z.parts.push(p);
                    }
                }
            }
            ("BEGIN", _) if ev.is_some() => depth_other += 1,
            ("END", _) if ev.is_some() && depth_other > 0 => depth_other -= 1,
            _ => {
                if let Some(p) = part.as_mut() {
                    match name.as_str() {
                        "DTSTART" => p.start = parse_local(value.trim()),
                        "TZOFFSETTO" => p.offset_to = parse_offset(&value).unwrap_or(0),
                        "RRULE" => p.rule = parse_rule(&value),
                        _ => {}
                    }
                } else if let Some(z) = zone.as_mut() {
                    if name == "TZID" {
                        z.id = value.trim().to_string();
                    }
                } else if let (Some(e), 0) = (ev.as_mut(), depth_other) {
                    match name.as_str() {
                        "UID" => e.uid = value.trim().to_string(),
                        "SUMMARY" => e.summary = unescape(&value).chars().take(200).collect(),
                        "DTSTART" => e.start = parse_dt(&value, &params),
                        "DTEND" => e.end = parse_dt(&value, &params),
                        "DURATION" => e.duration = parse_duration(&value),
                        "RRULE" => e.rule = parse_rule(&value),
                        "EXDATE" => {
                            for v in value.split(',') {
                                if e.exdates.len() >= MAX_EXDATES {
                                    break;
                                }
                                if let Some(d) = parse_dt(v, &params) {
                                    e.exdates.push(d);
                                }
                            }
                        }
                        "RECURRENCE-ID" => e.recurrence_id = parse_dt(&value, &params),
                        "STATUS" => e.cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
                        _ => {}
                    }
                }
            }
        }
    }
    cal
}

/// n-ty den v tyzdni `w` v mesiaci (n zaporne = od konca); `None` ak neexistuje.
fn nth_weekday(y: i32, m: u32, n: i32, w: u8) -> Option<Ymd> {
    let dim = days_in_month(y, m);
    if n > 0 {
        let first = weekday(Ymd::new(y, m, 1));
        let d = 1 + (i64::from(w) - i64::from(first)).rem_euclid(7) as u32 + (n as u32 - 1) * 7;
        (d <= dim).then(|| Ymd::new(y, m, d))
    } else if n < 0 {
        let last = weekday(Ymd::new(y, m, dim));
        let back = (i64::from(last) - i64::from(w)).rem_euclid(7) as u32 + ((-n) as u32 - 1) * 7;
        (back < dim).then(|| Ymd::new(y, m, dim - back))
    } else {
        None
    }
}

impl Zone {
    /// Posun od UTC (s) pre miestny cas `l`: posledny nastup STANDARD/DAYLIGHT pred `l`.
    fn offset_at(&self, l: Local) -> Option<i32> {
        let t = l.as_naive();
        let mut best: Option<(i64, i32)> = None;
        for p in &self.parts {
            let Some(s) = p.start else { continue };
            let mut cands: Vec<i64> = vec![s.as_naive()];
            if let Some(r) = &p.rule {
                // historicke pravidlo (UNTIL v minulosti) uz nove prechody nedava; UNTIL je v UTC, porovnanie s miestnym
                // casom je o par hodin nepresne, co pri rocnych prechodoch nevadi
                let until = r.until.as_ref().map(|u| match u {
                    Dt::Date(d) => Local {
                        day: *d,
                        secs: 86_399,
                    }
                    .as_naive(),
                    Dt::Utc(l) | Dt::Zoned(l, _) => l.as_naive(),
                });
                if r.freq == 4 {
                    for y in [l.day.y - 1, l.day.y] {
                        if y < s.day.y {
                            continue;
                        }
                        let m = r.bymonth.first().copied().unwrap_or(s.day.m);
                        let day = match r.byday.first() {
                            Some((n, w)) => nth_weekday(y, m, if *n == 0 { 1 } else { *n }, *w),
                            None => Some(Ymd::new(y, m, s.day.d.min(days_in_month(y, m)))),
                        };
                        if let Some(d) = day {
                            let c = Local {
                                day: d,
                                secs: s.secs,
                            }
                            .as_naive();
                            // den rezervy: UNTIL je v UTC, kandidat v miestnom case (pasma UTC+ by stratili posledny prechod)
                            if until.is_none_or(|u| c <= u + 86_400) {
                                cands.push(c);
                            }
                        }
                    }
                }
            }
            for c in cands {
                if c <= t && best.is_none_or(|(b, _)| c > b) {
                    best = Some((c, p.offset_to));
                }
            }
        }
        best.map(|b| b.1)
            .or_else(|| self.parts.first().map(|p| p.offset_to))
    }
}

impl Calendar {
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// UTC okamih miestneho casu v pasme `tz` (z VTIMEZONE; nezname pasmo a plavajuci cas = miestny cas PC).
    fn to_utc(&self, l: Local, tz: Option<&str>, local_to_utc: &dyn Fn(Local) -> i64) -> i64 {
        if let Some(z) = tz.and_then(|id| self.zones.iter().find(|z| z.id == id)) {
            if let Some(off) = z.offset_at(l) {
                return l.as_naive() - i64::from(off);
            }
        }
        local_to_utc(l)
    }

    fn when(&self, d: &Dt, local_to_utc: &dyn Fn(Local) -> i64) -> When {
        match d {
            Dt::Date(y) => When::Day(*y),
            Dt::Utc(l) => When::At(l.as_naive()),
            Dt::Zoned(l, tz) => When::At(self.to_utc(*l, tz.as_deref(), local_to_utc)),
        }
    }

    /// Vyskyty udalosti, ktore zasahuju do okna `[from, to)` (Unix sekundy UTC). Celodenne sa porovnavaju podla dna
    /// (`from_day..to_day` vratane). Zoradene podla zaciatku.
    pub fn occurrences(
        &self,
        from: i64,
        to: i64,
        from_day: Ymd,
        to_day: Ymd,
        local_to_utc: &dyn Fn(Local) -> i64,
    ) -> Vec<Occurrence> {
        let mut out = Vec::new();
        // spolocny rozpocet prace na vsetky opakovane udalosti (vola sa aj z vlakna okna: cudzi kalendar nesmie listu zaseknut)
        let budget = std::cell::Cell::new(MAX_CALL_WORK);
        // zmenene instancie: (uid, povodny zaciatok)
        let overridden: Vec<(String, When)> = self
            .events
            .iter()
            .filter_map(|e| {
                Some((
                    e.uid.clone(),
                    self.when(e.recurrence_id.as_ref()?, local_to_utc),
                ))
            })
            .collect();
        for e in &self.events {
            if e.cancelled && e.recurrence_id.is_none() && e.rule.is_none() {
                continue;
            }
            let Some(start) = &e.start else { continue };
            let s0 = self.when(start, local_to_utc);
            let duration = match (&e.end, e.duration) {
                (Some(end), _) => match (self.when(end, local_to_utc), s0) {
                    (When::At(b), When::At(a)) => (b - a).max(0),
                    (When::Day(b), When::Day(a)) => {
                        (days_from_civil(b) - days_from_civil(a)).max(1) * 86_400
                    }
                    _ => 0,
                },
                (None, Some(d)) => d.max(0),
                (None, None) => match s0 {
                    When::Day(_) => 86_400,
                    When::At(_) => 0,
                },
            };
            let starts: Vec<When> = match (&e.rule, e.recurrence_id.is_some()) {
                (Some(r), false) => {
                    self.expand(start, r, (from, to), to_day, local_to_utc, &budget)
                }
                _ => vec![s0],
            };
            let ex: Vec<When> = e
                .exdates
                .iter()
                .map(|d| self.when(d, local_to_utc))
                .collect();
            for w in starts {
                if ex.contains(&w)
                    || (e.recurrence_id.is_none()
                        && overridden.iter().any(|(u, o)| *u == e.uid && *o == w))
                {
                    continue;
                }
                if e.cancelled {
                    continue;
                }
                let hit = match w {
                    When::At(a) => a < to && a.saturating_add(duration.max(1)) > from,
                    When::Day(d) => {
                        let last =
                            civil_from_days(days_from_civil(d) + (duration / 86_400).max(1) - 1);
                        d <= to_day && last >= from_day
                    }
                };
                if hit {
                    out.push(Occurrence {
                        summary: e.summary.clone(),
                        start: w,
                        duration,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.start.cmp(&b.start).then(a.summary.cmp(&b.summary)));
        out
    }

    /// Zaciatky opakovanej udalosti do konca okna (v miestnom case jej pasma, potom na UTC).
    fn expand(
        &self,
        start: &Dt,
        r: &Rule,
        (from, to): (i64, i64),
        to_day: Ymd,
        l2u: &dyn Fn(Local) -> i64,
        budget: &std::cell::Cell<usize>,
    ) -> Vec<When> {
        // praca tejto udalosti (obdobia + kandidati + prejdene dni); po vycerpani sa dalsie vyskyty nehladaju
        let mut work = 0usize;
        let mut spend = |n: usize| -> bool {
            work += n;
            let left = budget.get();
            if work > MAX_EVENT_WORK {
                return false;
            }
            if left < n {
                budget.set(0);
                return false;
            }
            budget.set(left - n);
            true
        };
        let (base, tz, is_day) = match start {
            Dt::Date(d) => (Local { day: *d, secs: 0 }, None, true),
            Dt::Utc(l) => (*l, Some("\u{1}utc"), false),
            Dt::Zoned(l, tz) => (*l, tz.as_deref(), false),
        };
        let conv = |l: Local| -> When {
            if is_day {
                When::Day(l.day)
            } else if tz == Some("\u{1}utc") {
                When::At(l.as_naive())
            } else {
                When::At(self.to_utc(l, tz, l2u))
            }
        };
        let until = r.until.as_ref().map(|u| self.when(u, l2u));
        let past_end = |w: &When| -> bool {
            match w {
                When::At(a) => *a >= to,
                When::Day(d) => *d > to_day,
            }
        };
        let after_until = |w: &When| -> bool {
            match (until, w) {
                (Some(When::At(u)), When::At(a)) => a > &u,
                (Some(When::Day(u)), When::Day(d)) => d > &u,
                (Some(When::Day(u)), When::At(a)) => *a >= (days_from_civil(u) + 1) * 86_400,
                (Some(When::At(u)), When::Day(d)) => days_from_civil(*d) * 86_400 > u,
                (None, _) => false,
            }
        };
        let mut out = Vec::new();
        let mut produced = 0u32;
        let step = i64::from(r.interval.max(1));
        let bwd: Vec<u8> = r.byday.iter().map(|b| b.1).collect();
        let month_ok = |m: u32| r.bymonth.is_empty() || r.bymonth.contains(&m);
        // bez COUNT sa obdobia pred oknom preskocia (denna udalost od roku 2010 nemusi prejst tisice dni); dva dni rezerva
        // na posun pasma
        let k0 = if r.count.is_none() {
            let gap = (from - 2 * 86_400) - (base.as_naive() - 14 * 3600);
            let k = match r.freq {
                1 => gap / (step * 86_400),
                2 => gap / (step * 7 * 86_400),
                3 => gap / (step * 31 * 86_400),
                _ => gap / (step * 366 * 86_400),
            };
            (k - 1).max(0)
        } else {
            0
        };
        // kandidati na jedno obdobie (den / tyzden / mesiac / rok)
        for k in k0..k0 + 20_000 {
            if !spend(1) {
                return out;
            }
            let mut cands: Vec<Local> = Vec::new();
            match r.freq {
                1 => {
                    // BYMONTH / BYMONTHDAY / BYDAY pri dennom opakovani len filtruju (RFC 5545)
                    let c = Local::from_naive(base.as_naive() + k * step * 86_400);
                    let dim = days_in_month(c.day.y, c.day.m) as i32;
                    let md_ok = r.bymonthday.is_empty()
                        || r.bymonthday
                            .iter()
                            .any(|md| (if *md < 0 { dim + 1 + md } else { *md }) == c.day.d as i32);
                    if month_ok(c.day.m)
                        && md_ok
                        && (bwd.is_empty() || bwd.contains(&weekday(c.day)))
                    {
                        cands.push(c);
                    }
                }
                2 => {
                    let week0 = days_from_civil(base.day) - i64::from(weekday(base.day));
                    let ws = week0 + k * step * 7;
                    let days: Vec<u8> = if bwd.is_empty() {
                        vec![weekday(base.day)]
                    } else {
                        bwd.clone()
                    };
                    let mut ds: Vec<u8> = days;
                    ds.sort_unstable();
                    ds.dedup();
                    for d in ds {
                        let day = civil_from_days(ws + i64::from(d));
                        if month_ok(day.m) {
                            cands.push(Local {
                                day,
                                secs: base.secs,
                            });
                        }
                    }
                }
                3 | 4 => {
                    let months: Vec<(i32, u32)> = if r.freq == 3 {
                        let idx = base.day.y as i64 * 12 + i64::from(base.day.m) - 1 + k * step;
                        let (y, m) = (idx.div_euclid(12) as i32, (idx.rem_euclid(12) + 1) as u32);
                        if month_ok(m) {
                            vec![(y, m)]
                        } else {
                            Vec::new()
                        }
                    } else {
                        let y = base.day.y + (k * step) as i32;
                        let ms = if r.bymonth.is_empty() {
                            vec![base.day.m]
                        } else {
                            r.bymonth.clone()
                        };
                        ms.into_iter()
                            .filter(|m| (1..=12).contains(m))
                            .map(|m| (y, m))
                            .collect()
                    };
                    for (y, m) in months {
                        let dim = days_in_month(y, m) as i32;
                        let mut by_md: Vec<Ymd> = Vec::new();
                        for md in &r.bymonthday {
                            let d = if *md < 0 { dim + 1 + md } else { *md };
                            if (1..=dim).contains(&d) {
                                by_md.push(Ymd::new(y, m, d as u32));
                            }
                        }
                        let mut by_wd: Vec<Ymd> = Vec::new();
                        for (n, w) in &r.byday {
                            if *n != 0 {
                                by_wd.extend(nth_weekday(y, m, *n, *w));
                            } else {
                                if !spend(dim as usize) {
                                    return out;
                                }
                                for d in 1..=dim as u32 {
                                    if weekday(Ymd::new(y, m, d)) == *w {
                                        by_wd.push(Ymd::new(y, m, d));
                                    }
                                }
                            }
                        }
                        // obe pravidla naraz sa prenikaju (napr. piatok 13.), inak plati to, ktore je zadane
                        // podla pravidla, nie podla vysledku: prazdny prienik v kratkom mesiaci je prazdny
                        let mut days: Vec<Ymd> = match (r.bymonthday.is_empty(), r.byday.is_empty())
                        {
                            (false, false) => {
                                by_md.into_iter().filter(|d| by_wd.contains(d)).collect()
                            }
                            (false, true) => by_md,
                            _ => by_wd,
                        };
                        if r.bymonthday.is_empty() && r.byday.is_empty() {
                            // bez pravidla den: ten isty den v mesiaci (31. sa v kratsom mesiaci vynecha)
                            if (base.day.d as i32) <= dim {
                                days.push(Ymd::new(y, m, base.day.d));
                            }
                        }
                        days.sort();
                        days.dedup();
                        cands.extend(days.into_iter().map(|d| Local {
                            day: d,
                            secs: base.secs,
                        }));
                    }
                }
                _ => return out,
            }
            if !r.bysetpos.is_empty() {
                // kandidati obdobia su vzostupne; BYSETPOS vyberie poradie z nich (1 = prvy, -1 = posledny)
                let n = cands.len() as i64;
                let mut pick: Vec<usize> = r
                    .bysetpos
                    .iter()
                    .filter_map(|&p| {
                        let i = if p > 0 {
                            i64::from(p) - 1
                        } else {
                            n + i64::from(p)
                        };
                        (0..n).contains(&i).then_some(i as usize)
                    })
                    .collect();
                pick.sort_unstable();
                pick.dedup();
                cands = pick.into_iter().map(|i| cands[i]).collect();
            }
            if !spend(cands.len()) {
                return out;
            }
            let mut any_future = false;
            for c in cands {
                if c < base {
                    continue;
                }
                let w = conv(c);
                if after_until(&w) || r.count.is_some_and(|n| produced >= n) {
                    return out;
                }
                produced += 1;
                if past_end(&w) {
                    return out;
                }
                any_future = true;
                out.push(w);
                if out.len() >= MAX_OCCURRENCES * 4 {
                    // vela starych vyskytov pred oknom: drzat len posledne
                    out.drain(..MAX_OCCURRENCES * 3);
                }
            }
            let _ = any_future;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
        days_from_civil(Ymd::new(y, m, d)) * 86_400 + i64::from(h * 3600 + mi * 60)
    }

    /// Miestny cas PC v testoch: UTC+1.
    fn l2u(l: Local) -> i64 {
        l.as_naive() - 3600
    }

    const TZ: &str = "BEGIN:VTIMEZONE\r\nTZID:Europe/Bratislava\r\nBEGIN:DAYLIGHT\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nDTSTART:19700329T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nEND:DAYLIGHT\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nDTSTART:19701025T030000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";

    fn cal(events: &str) -> Calendar {
        parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{TZ}{events}END:VCALENDAR\r\n"
        ))
    }

    fn all(c: &Calendar, y0: (i32, u32, u32), y1: (i32, u32, u32)) -> Vec<Occurrence> {
        let a = Ymd::new(y0.0, y0.1, y0.2);
        let b = Ymd::new(y1.0, y1.1, y1.2);
        c.occurrences(
            days_from_civil(a) * 86_400,
            (days_from_civil(b) + 1) * 86_400,
            a,
            b,
            &l2u,
        )
    }

    #[test]
    fn single_all_day_utc_and_folded_events_are_read() {
        let c = cal("BEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Porada\\, t\u{00ed}m\r\n DEV\r\nDTSTART;TZID=Europe/Bratislava:20261124T140000\r\nDTEND;TZID=Europe/Bratislava:20261124T150000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:2\r\nSUMMARY:Sviatok\r\nDTSTART;VALUE=DATE:20261124\r\nDTEND;VALUE=DATE:20261125\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:3\r\nSUMMARY:Let\r\nDTSTART:20260701T080000Z\r\nDURATION:PT1H30M\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:4\r\nSTATUS:CANCELLED\r\nSUMMARY:Zrusene\r\nDTSTART:20261124T090000Z\r\nEND:VEVENT\r\n");
        let o = all(&c, (2026, 11, 24), (2026, 11, 24));
        assert_eq!(o.len(), 2, "{o:?}");
        assert_eq!(o[0].summary, "Sviatok");
        assert_eq!(o[0].start, When::Day(Ymd::new(2026, 11, 24)));
        // 14:00 v Bratislave v novembri = 13:00 UTC; rozdelene riadky sa spoja
        assert_eq!(o[1].summary, "Porada, tímDEV");
        assert_eq!(
            (o[1].start, o[1].duration),
            (When::At(utc(2026, 11, 24, 13, 0)), 3600)
        );
        // v lete +2 h
        let o = all(&c, (2026, 7, 1), (2026, 7, 1));
        assert_eq!(
            (o[0].start, o[0].duration),
            (When::At(utc(2026, 7, 1, 8, 0)), 5400)
        );
    }

    #[test]
    fn weekly_rules_follow_daylight_saving_and_skip_exceptions() {
        let c = cal("BEGIN:VEVENT\r\nUID:w\r\nSUMMARY:Standup\r\nDTSTART;TZID=Europe/Bratislava:20261020T090000\r\nDTEND;TZID=Europe/Bratislava:20261020T091500\r\nRRULE:FREQ=WEEKLY;BYDAY=TU,TH;UNTIL=20261112T230000Z\r\nEXDATE;TZID=Europe/Bratislava:20261029T090000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:w\r\nRECURRENCE-ID;TZID=Europe/Bratislava:20261105T090000\r\nSUMMARY:Standup (presunute)\r\nDTSTART;TZID=Europe/Bratislava:20261105T110000\r\nDTEND;TZID=Europe/Bratislava:20261105T111500\r\nEND:VEVENT\r\n");
        let o = all(&c, (2026, 10, 1), (2026, 12, 31));
        let s: Vec<(String, i64)> = o
            .iter()
            .map(|x| match x.start {
                When::At(a) => (x.summary.clone(), a),
                When::Day(_) => unreachable!(),
            })
            .collect();
        let want = vec![
            ("Standup", utc(2026, 10, 20, 7, 0)),
            ("Standup", utc(2026, 10, 22, 7, 0)),
            // 25. 10. koniec letneho casu: 9:00 je uz 8:00 UTC
            ("Standup", utc(2026, 10, 27, 8, 0)),
            ("Standup", utc(2026, 11, 3, 8, 0)),
            ("Standup (presunute)", utc(2026, 11, 5, 10, 0)),
            ("Standup", utc(2026, 11, 10, 8, 0)),
            ("Standup", utc(2026, 11, 12, 8, 0)),
        ];
        let want: Vec<(String, i64)> = want.into_iter().map(|(a, b)| (a.to_string(), b)).collect();
        assert_eq!(s, want);
    }

    #[test]
    fn monthly_yearly_counts_and_floating_times() {
        let c = cal("BEGIN:VEVENT\r\nUID:m\r\nSUMMARY:Platba\r\nDTSTART;VALUE=DATE:20260131\r\nRRULE:FREQ=MONTHLY;COUNT=4\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:y\r\nSUMMARY:Narodeniny\r\nDTSTART;VALUE=DATE:20000229\r\nRRULE:FREQ=YEARLY\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:n\r\nSUMMARY:Porada tima\r\nDTSTART:20260101T100000\r\nRRULE:FREQ=MONTHLY;BYDAY=-1FR;INTERVAL=2\r\nEND:VEVENT\r\n");
        let o = all(&c, (2026, 1, 1), (2028, 12, 31));
        let pay: Vec<When> = o
            .iter()
            .filter(|x| x.summary == "Platba")
            .map(|x| x.start)
            .collect();
        // 31. len v mesiacoch, ktore ho maju; COUNT=4 z existujucich vyskytov
        assert_eq!(
            pay,
            vec![
                When::Day(Ymd::new(2026, 1, 31)),
                When::Day(Ymd::new(2026, 3, 31)),
                When::Day(Ymd::new(2026, 5, 31)),
                When::Day(Ymd::new(2026, 7, 31)),
            ]
        );
        let bday: Vec<When> = o
            .iter()
            .filter(|x| x.summary == "Narodeniny")
            .map(|x| x.start)
            .collect();
        assert_eq!(bday, vec![When::Day(Ymd::new(2028, 2, 29))]);
        // posledny piatok kazdy druhy mesiac, plavajuci cas = miestny cas PC (tu UTC+1)
        let team: Vec<When> = o
            .iter()
            .filter(|x| x.summary == "Porada tima")
            .map(|x| x.start)
            .take(2)
            .collect();
        assert_eq!(
            team,
            vec![
                When::At(utc(2026, 1, 30, 9, 0)),
                When::At(utc(2026, 3, 27, 9, 0))
            ]
        );
    }

    #[test]
    fn hostile_input_never_panics_or_explodes() {
        // denna udalost od roku 1900 ma vyskyty aj v roku 2026 (preskocenie obdobi pred oknom)
        let old = parse(
            "BEGIN:VEVENT
UID:o
SUMMARY:x
DTSTART:19000101T120000Z
RRULE:FREQ=DAILY
END:VEVENT
",
        );
        let o = all(&old, (2026, 3, 1), (2026, 3, 3));
        assert_eq!(o.len(), 3);
        assert_eq!(o[0].start, When::At(utc(2026, 3, 1, 12, 0)));
        for t in [
            "",
            "BEGIN:VEVENT",
            "BEGIN:VEVENT\nDTSTART:2026\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20261301T250000\nRRULE:FREQ=SECONDLY\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:19000101T000000Z\nRRULE:FREQ=DAILY\nSUMMARY:x\nEND:VEVENT",
            "ž:é\n\u{0}\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:2026ž1124\nEND:VEVENT",
        ] {
            let c = parse(t);
            let o = all(&c, (2026, 1, 1), (2026, 12, 31));
            assert!(o.len() <= 400, "{}", o.len());
        }
        assert_eq!(parse_duration("P1DT2H"), Some(93_600));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_offset("-0530"), Some(-19_800));
    }
}

#[cfg(test)]
mod perf {
    use super::*;

    /// Velky kalendar (5000 tyzdennych udalostí od roku 2015, s VTIMEZONE): mriezka mesiaca sa spocita rychlo.
    #[test]
    fn a_big_calendar_expands_a_month_grid_quickly() {
        let mut s = String::from("BEGIN:VCALENDAR\n");
        for i in 0..5000 {
            s.push_str(&format!(
                "BEGIN:VEVENT\nUID:{i}\nSUMMARY:E{i}\nDTSTART:2015{:02}{:02}T090000Z\nDTEND:2015{:02}{:02}T100000Z\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\nEND:VEVENT\n",
                i % 12 + 1,
                i % 28 + 1,
                i % 12 + 1,
                i % 28 + 1
            ));
        }
        s.push_str("END:VCALENDAR\n");
        let c = parse(&s);
        assert_eq!(c.event_count(), 5000);
        let a = Ymd::new(2026, 10, 26);
        let b = Ymd::new(2026, 12, 6);
        let t0 = std::time::Instant::now();
        let o = c.occurrences(
            days_from_civil(a) * 86_400,
            (days_from_civil(b) + 1) * 86_400,
            a,
            b,
            &|l| days_from_civil(l.day) * 86_400 + i64::from(l.secs),
        );
        let ms = t0.elapsed().as_millis();
        eprintln!("ICS 5000 udalosti: {} vyskytov za {ms} ms", o.len());
        assert_eq!(o.len(), 5000 * 12);
        assert!(ms < 3000, "{ms} ms");
    }
}

#[cfg(test)]
mod audit {
    use super::*;

    fn window(c: &Calendar, a: Ymd, b: Ymd) -> Vec<Occurrence> {
        c.occurrences(
            days_from_civil(a) * 86_400,
            (days_from_civil(b) + 1) * 86_400,
            a,
            b,
            &|l| l.as_naive(),
        )
    }

    #[test]
    fn non_ascii_bytes_inside_numeric_fields_never_panic() {
        for t in [
            "BEGIN:VEVENT\nDTSTART:202\u{17e}61124\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20261124T1\u{17e}2345\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20261124T12\u{20ac}45Z\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20261124T120000Z\nRRULE:FREQ=WEEKLY;BYDAY=\u{20ac},\u{17e}a,1\u{20ac}\nEND:VEVENT",
            "BEGIN:VTIMEZONE\nTZID:x\nBEGIN:STANDARD\nDTSTART:1970010\u{17e}T000000\nTZOFFSETTO:+0\u{17e}0\nEND:STANDARD\nEND:VTIMEZONE",
            "BEGIN:VEVENT\nDTSTART;TZID=x:20261124T120000\nEXDATE:2026112\u{17e}T120000\nEND:VEVENT",
        ] {
            let c = parse(t);
            let _ = window(&c, Ymd::new(2026, 1, 1), Ymd::new(2026, 12, 31));
        }
        assert_eq!(parse_offset("+0\u{17e}0"), None);
        assert_eq!(parse_date("202\u{17e}61124"), None);
        assert_eq!(parse_local("20261124T1\u{17e}2345"), None);
    }

    #[test]
    fn yearly_months_listed_out_of_order_are_expanded_in_date_order() {
        // BYMONTH=10,3: marec pred oktobrom; COUNT pocita v poradi datumov a koniec okna nevynecha marec
        let c = parse(
            "BEGIN:VEVENT
UID:y
SUMMARY:x
DTSTART;VALUE=DATE:20260115
RRULE:FREQ=YEARLY;BYMONTH=10,3;BYMONTHDAY=15;COUNT=3
END:VEVENT
",
        );
        let days: Vec<When> = window(&c, Ymd::new(2026, 1, 1), Ymd::new(2028, 12, 31))
            .into_iter()
            .map(|o| o.start)
            .collect();
        assert_eq!(
            days,
            vec![
                When::Day(Ymd::new(2026, 3, 15)),
                When::Day(Ymd::new(2026, 10, 15)),
                When::Day(Ymd::new(2027, 3, 15)),
            ]
        );
        let c = parse(
            "BEGIN:VEVENT
UID:y
SUMMARY:x
DTSTART;VALUE=DATE:20260115
RRULE:FREQ=YEARLY;BYMONTH=10,3;BYMONTHDAY=15
END:VEVENT
",
        );
        assert_eq!(
            window(&c, Ymd::new(2026, 3, 1), Ymd::new(2026, 3, 31)).len(),
            1,
            "marec v okne, oktober za nim"
        );
    }

    fn starts(rule: &str, dtstart: &str, a: Ymd, b: Ymd) -> Vec<Ymd> {
        let c = parse(&format!(
            "BEGIN:VEVENT\nUID:r\nSUMMARY:x\nDTSTART:{dtstart}\nRRULE:{rule}\nEND:VEVENT\n"
        ));
        window(&c, a, b)
            .into_iter()
            .map(|o| match o.start {
                When::Day(d) => d,
                When::At(t) => civil_from_days(t.div_euclid(86_400)),
            })
            .collect()
    }

    #[test]
    fn by_rules_filter_and_select_like_rfc_5545() {
        let d = Ymd::new;
        let (jan, mar) = (d(2026, 1, 1), d(2026, 3, 31));
        // tabulka: (pravidlo, DTSTART, od, do, ocakavane dni)
        let cases: Vec<(&str, &str, Ymd, Ymd, Vec<Ymd>)> = vec![
            // Outlook: "prvy pondelok v mesiaci" = BYDAY=MO;BYSETPOS=1 (nie kazdy pondelok)
            (
                "FREQ=MONTHLY;BYDAY=MO;BYSETPOS=1",
                "20260105T090000Z",
                jan,
                mar,
                vec![d(2026, 1, 5), d(2026, 2, 2), d(2026, 3, 2)],
            ),
            // "posledny pracovny den v mesiaci"
            (
                "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
                "20260130T090000Z",
                jan,
                mar,
                vec![d(2026, 1, 30), d(2026, 2, 27), d(2026, 3, 31)],
            ),
            // druhy a posledny vyskyt z mnoziny obdobia
            (
                "FREQ=MONTHLY;BYDAY=TU,TH;BYSETPOS=2,-1",
                "20260101T090000Z",
                jan,
                d(2026, 1, 31),
                vec![d(2026, 1, 6), d(2026, 1, 29)],
            ),
            // DAILY + BYDAY = filter (pracovne dni), nie kazdy den
            (
                "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR",
                "20261123T090000Z",
                d(2026, 11, 23),
                d(2026, 11, 29),
                vec![
                    d(2026, 11, 23),
                    d(2026, 11, 24),
                    d(2026, 11, 25),
                    d(2026, 11, 26),
                    d(2026, 11, 27),
                ],
            ),
            // DAILY + BYMONTH / BYMONTHDAY = filter
            (
                "FREQ=DAILY;BYMONTH=2;BYMONTHDAY=1,-1",
                "20260101T090000Z",
                jan,
                mar,
                vec![d(2026, 2, 1), d(2026, 2, 28)],
            ),
            // piatok 13.: BYDAY a BYMONTHDAY sa prenikaju (nie zjednocuju)
            (
                "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13",
                "20260101T090000Z",
                jan,
                d(2026, 12, 31),
                vec![d(2026, 2, 13), d(2026, 3, 13), d(2026, 11, 13)],
            ),
            // prienik je prazdny v mesiacoch bez 31. (nie vsetky piatky)
            (
                "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=31",
                "20260101T090000Z",
                jan,
                d(2026, 12, 31),
                vec![d(2026, 7, 31)],
            ),
            (
                "FREQ=MONTHLY;BYDAY=5MO;BYMONTHDAY=29,30,31",
                "20260101T090000Z",
                jan,
                d(2026, 6, 30),
                vec![d(2026, 3, 30), d(2026, 6, 29)],
            ),
            // WEEKLY + BYMONTH = len tyzdne v decembri
            (
                "FREQ=WEEKLY;BYDAY=MO;BYMONTH=12",
                "20261102T090000Z",
                d(2026, 11, 1),
                d(2026, 12, 31),
                vec![
                    d(2026, 12, 7),
                    d(2026, 12, 14),
                    d(2026, 12, 21),
                    d(2026, 12, 28),
                ],
            ),
            // MONTHLY + BYMONTH
            (
                "FREQ=MONTHLY;BYMONTHDAY=1;BYMONTH=1,3",
                "20260101T090000Z",
                jan,
                d(2026, 12, 31),
                vec![d(2026, 1, 1), d(2026, 3, 1)],
            ),
            // COUNT pocita az vybrane vyskyty
            (
                "FREQ=MONTHLY;BYDAY=MO;BYSETPOS=1;COUNT=2",
                "20260105T090000Z",
                jan,
                d(2026, 12, 31),
                vec![d(2026, 1, 5), d(2026, 2, 2)],
            ),
        ];
        for (rule, dt, a, b, want) in cases {
            assert_eq!(starts(rule, dt, a, b), want, "{rule}");
        }
    }

    #[test]
    fn a_historic_time_zone_rule_with_until_no_longer_applies() {
        // America/New_York s pravidlom 1987-2006 (UNTIL) a novym od 2007: koniec oktobra 2026 je stale letny cas (-4 h)
        let tz = "BEGIN:VTIMEZONE\nTZID:America/New_York\n\
BEGIN:DAYLIGHT\nTZOFFSETFROM:-0500\nTZOFFSETTO:-0400\nDTSTART:19870405T020000\nRRULE:FREQ=YEARLY;BYMONTH=4;BYDAY=1SU;UNTIL=20060402T070000Z\nEND:DAYLIGHT\n\
BEGIN:STANDARD\nTZOFFSETFROM:-0400\nTZOFFSETTO:-0500\nDTSTART:19671029T020000\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU;UNTIL=20061029T060000Z\nEND:STANDARD\n\
BEGIN:DAYLIGHT\nTZOFFSETFROM:-0500\nTZOFFSETTO:-0400\nDTSTART:20070311T020000\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\nEND:DAYLIGHT\n\
BEGIN:STANDARD\nTZOFFSETFROM:-0400\nTZOFFSETTO:-0500\nDTSTART:20071104T020000\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\nEND:STANDARD\n\
END:VTIMEZONE\n";
        let c = parse(&format!(
            "{tz}BEGIN:VEVENT\nUID:a\nSUMMARY:a\nDTSTART;TZID=America/New_York:20261030T090000\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:b\nSUMMARY:b\nDTSTART;TZID=America/New_York:20261105T090000\nEND:VEVENT\n\
BEGIN:VEVENT\nUID:c\nSUMMARY:c\nDTSTART;TZID=America/New_York:20260320T090000\nEND:VEVENT\n"
        ));
        let o = window(&c, Ymd::new(2026, 3, 1), Ymd::new(2026, 11, 30));
        let at = |s: &str| o.iter().find(|x| x.summary == s).map(|x| x.start).unwrap();
        let utc = |m: u32, d: u32, h: i64| {
            When::At(days_from_civil(Ymd::new(2026, m, d)) * 86_400 + h * 3600)
        };
        assert_eq!(
            at("a"),
            utc(10, 30, 13),
            "po poslednej nedeli oktobra stale EDT"
        );
        assert_eq!(at("b"), utc(11, 5, 14), "po prvej nedeli novembra EST");
        assert_eq!(at("c"), utc(3, 20, 13), "po druhej nedeli marca EDT");
    }

    #[test]
    fn huge_numbers_in_rules_and_durations_never_overflow() {
        for rule in [
            "FREQ=MONTHLY;BYDAY=999999999MO",
            "FREQ=MONTHLY;BYDAY=-2147483648MO",
            "FREQ=MONTHLY;BYDAY=2147483647MO",
            "FREQ=MONTHLY;BYMONTHDAY=-2147483648",
            "FREQ=MONTHLY;BYMONTHDAY=2147483647",
            "FREQ=YEARLY;BYMONTH=4294967295",
            "FREQ=MONTHLY;BYSETPOS=-2147483648;BYDAY=MO",
            "FREQ=DAILY;COUNT=4294967295",
        ] {
            let c = parse(&format!(
                "BEGIN:VEVENT\nDTSTART:20260105T090000Z\nDURATION:P99999999999999W\nRRULE:{rule}\nEND:VEVENT\n"
            ));
            let _ = window(&c, Ymd::new(2026, 1, 1), Ymd::new(2026, 3, 31));
        }
        // pretecenie = neplatna dlzka; extremna (ale zapisatelna) dlzka sa oreze
        assert_eq!(parse_duration("P99999999999999999999D"), None);
        assert_eq!(parse_duration("P99999999999999W"), None);
        assert_eq!(parse_duration("P9999999W"), Some(MAX_DURATION));
        assert_eq!(parse_duration("-P9999999W"), Some(-MAX_DURATION));
    }
}

#[cfg(test)]
mod hostile {
    use super::*;

    fn grid(c: &Calendar) -> Vec<Occurrence> {
        let a = Ymd::new(2026, 1, 1);
        let b = Ymd::new(2026, 12, 31);
        c.occurrences(
            days_from_civil(a) * 86_400,
            (days_from_civil(b) + 1) * 86_400,
            a,
            b,
            &|l| l.as_naive(),
        )
    }

    /// Viacbajtove znaky na miestach, kde parser rezal retazec na bajtoch (predtym pad celej listy pri synchronizacii).
    #[test]
    fn multibyte_characters_inside_dates_and_offsets_never_panic() {
        for t in [
            "BEGIN:VEVENT\nDTSTART:202\u{e9}1124\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART;VALUE=DATE:20\u{17e}61124\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:2026\u{e9}124\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T1\u{e9}2345\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T12\u{e9}345\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T1234\u{e9}5\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T120000\nRRULE:FREQ=DAILY;UNTIL=202\u{e9}0101\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T120000\nEXDATE:2026010\u{e9}T120000\nEND:VEVENT",
            "BEGIN:VTIMEZONE\nTZID:X\nBEGIN:STANDARD\nDTSTART:19701025T030000\nTZOFFSETTO:+1\u{e9}00\nEND:STANDARD\nEND:VTIMEZONE",
            "BEGIN:VTIMEZONE\nTZID:X\nBEGIN:STANDARD\nDTSTART:19701025T030000\nTZOFFSETTO:-01\u{e9}0\nEND:STANDARD\nEND:VTIMEZONE",
            "BEGIN:VTIMEZONE\nTZID:X\nBEGIN:STANDARD\nDTSTART:1970\u{1f600}T030000\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE",
        ] {
            let c = parse(t);
            let _ = grid(&c);
        }
        assert_eq!(parse_date("202\u{e9}1124"), None);
        assert_eq!(parse_local("20260101T1\u{e9}2345"), None);
        assert_eq!(parse_offset("+1\u{e9}00"), None);
        assert_eq!(parse_offset("+01\u{e9}"), None);
        // cifry so znamienkom / medzerou sa neprijmu ako cast datumu
        assert_eq!(parse_date("+0261124"), None);
        assert_eq!(parse_date("2026 1124"), None);
        assert_eq!(parse_offset("+0100"), Some(3600));
        assert_eq!(parse_offset("+013015"), Some(5415));
    }

    /// Obrovske cisla v trvani a poradi dna nesmu pretiect (testy bezia s kontrolou pretecenia).
    #[test]
    fn huge_numbers_in_durations_and_rules_never_overflow() {
        assert_eq!(parse_duration("P9223372036854775807W"), None);
        assert_eq!(parse_duration("P99999999999999999999D"), None);
        assert_eq!(parse_duration("P100000D"), Some(MAX_DURATION));
        assert_eq!(parse_duration("-P100000D"), Some(-MAX_DURATION));
        for t in [
            "BEGIN:VEVENT\nDTSTART:20260101T120000Z\nDURATION:P9999999999W\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T120000Z\nDURATION:P3000000DT1H\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T120000\nRRULE:FREQ=MONTHLY;BYDAY=2147483647MO,-2147483648FR,99TU\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T120000\nRRULE:FREQ=YEARLY;BYMONTH=4294967295,0,13;BYMONTHDAY=-2147483648,2147483647\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:20260101T120000\nRRULE:FREQ=DAILY;INTERVAL=4294967295;COUNT=4294967295\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART:99991231T235959Z\nRRULE:FREQ=YEARLY\nEND:VEVENT",
            "BEGIN:VEVENT\nDTSTART;VALUE=DATE:00000101\nRRULE:FREQ=DAILY\nEND:VEVENT",
        ] {
            let c = parse(t);
            let o = grid(&c);
            assert!(o.len() <= 400, "{t}: {}", o.len());
        }
        let r = parse_rule("FREQ=MONTHLY;BYDAY=2147483647MO,1MO,1MO,-1FR").unwrap();
        assert_eq!(r.byday, vec![(1, 0), (-1, 4)], "neplatne poradie sa zahodi");
    }

    /// Zlomyselny kalendar (tisice udalosti od roku 1900 s COUNT a BYDAY na kazdy den, obrovske zoznamy) sa spocita rychlo
    /// a s obmedzenou pamatou (predtym minuty prace na vlakne okna).
    #[test]
    fn pathological_rules_are_bounded_in_time() {
        let byday = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
            .repeat(2000)
            .join(",");
        let monthdays = (0..28_000)
            .map(|d| (d % 28 + 1).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let exdates = vec!["19000101T000000Z"; 5_000].join(",");
        // spolu ~8 MiB (strop stiahnutia): 30 udalosti s obrovskymi zoznamami a tisice kratkych s BYDAY na kazdy den
        let mut s = String::from("BEGIN:VCALENDAR\n");
        for i in 0..30 {
            s.push_str(&format!(
                "BEGIN:VEVENT\nUID:{i}\nSUMMARY:x\nDTSTART:19000101T000000Z\nRRULE:FREQ=MONTHLY;COUNT=4000000000;BYDAY={byday};BYMONTHDAY={monthdays}\nEXDATE:{exdates}\nEND:VEVENT\n"
            ));
        }
        for i in 0..MAX_EVENTS {
            s.push_str(&format!(
                "BEGIN:VEVENT\nUID:k{i}\nSUMMARY:x\nDTSTART:19000101T000000Z\nRRULE:FREQ=MONTHLY;COUNT=4000000000;BYDAY=MO,TU,WE,TH,FR,SA,SU\nEND:VEVENT\n"
            ));
        }
        for i in 0..1000 {
            s.push_str(&format!(
                "BEGIN:VTIMEZONE\nTZID:Z{i}\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\n"
            ));
        }
        s.push_str("END:VCALENDAR\n");
        let t0 = std::time::Instant::now();
        let c = parse(&s);
        assert!(c.zones.len() <= MAX_ZONES);
        assert!(c.events.iter().all(|e| e.exdates.len() <= MAX_EXDATES
            && e.rule.as_ref().is_some_and(
                |r| r.byday.len() <= MAX_RULE_LIST && r.bymonthday.len() <= MAX_RULE_LIST
            )));
        let parsed = t0.elapsed().as_millis();
        let t1 = std::time::Instant::now();
        let _ = grid(&c);
        let ms = t1.elapsed().as_millis();
        eprintln!(
            "zlomyselny ICS ({} B): rozbor {parsed} ms, mriezka {ms} ms",
            s.len()
        );
        // ladiaci build; vydanie je radovo rychlejsie
        assert!(ms < 15_000, "{ms} ms");
    }
}
