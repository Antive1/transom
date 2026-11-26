//! Mesacna historia vyuzitia: denne tokeny alebo **odhad ceny** (API-ekvivalent, nie faktura) z lokalnych prepisov
//! (Claude Code, Codex). Cista funkcia nad `Collected` - bez Win32, bez siete.
//!
//! **Co to je a nie je:** historia tokenov / odhadu ceny podla skutocneho datumu zaznamu (lokalny den). **Nie je to**
//! historicka spotreba kvoty planu (relacne / tyzdenne / mesacne limity su len aktualny snimok v kartach).
//!
//! **Uplnost** (nikdy sa chybajuci udaj nevykresli ako nula): kazdy den ma explicitny stav -
//! - [`DayState::Future`]: este nenastal;
//! - [`DayState::NoHistory`]: den je pred zaciatkom historie zdroja (najstarsi zaznam) alebo poskytovatel nema lokalny
//!   zdroj dennej historie - **nie je to nula**, nevieme;
//! - [`DayState::Incomplete`]: udaj je len spodny odhad - prvy den historie zdroja (mohol zacat uprostred dna), zdroj mal
//!   chyby citania, alebo vo filtri "Vsetci" niektory poskytovatel este nema historiu;
//! - [`DayState::Unpriced`]: (len pre odhad ceny) volania sa neda ocenit - cena je neznama, nie $0;
//! - [`DayState::Recorded`]: den je pokryty; **nulova hodnota je skutocna nula** (v ten den nic nebolo zaznamenane).
//!
//! **Archiv** (volitelny, `archive.rs`): dni, ktore zdroj uz necita, ale archiv ich drzi, su zobrazene s povodom
//! [`Origin::Archived`] (alebo [`Origin::Mixed`] pri filtri "Vsetci"); pokrytie archivu (suvisle rozsahy uspesnych skenov)
//! rozsiruje to, co sa povazuje za pokryte - den mimo pokrytia je stale "bez historie", nikdy nula.
//!
//! Zdroje bez dennej historie: poskytovatel, ktoreho lokalny zdroj je vypnuty (napr. Claude len cez statusline - ten dava
//! cenu relacie a limity, nie denne tokeny), je v `unsupported` a v grafe sa necita ako nula.
use crate::{Collected, DayTotals, ModelTotals, Provider, SourceError, SourceKind};
use transom_core::clock::{days_from_civil, days_in_month, Ymd};

/// Co predstavuju stlpce grafu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Tokens,
    /// Odhad ceny v USD z cennika (nie faktura).
    Cost,
}

/// Filter poskytovatela.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Only(Provider),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayState {
    Future,
    NoHistory,
    Incomplete,
    Unpriced,
    Recorded,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayCell {
    pub date: Ymd,
    pub state: DayState,
    pub tokens: u64,
    pub cost: f64,
    pub requests: u64,
    /// Volania bez ceny (neznamy model, nestandardna rychlost, dlhy kontext) - cena dna je len cast.
    pub unpriced: u64,
    /// Povod sumy dna (citatelne logy / archiv / oboje).
    pub origin: Origin,
    /// Datumy cennikov, z ktorych je archivovany odhad ceny (zoradene, bez duplicit; prazdne pre cerstve dni).
    pub price_dates: Vec<String>,
}

impl DayCell {
    /// Hodnota stlpca pre `metric`; `None`, ak den nema ciselny udaj (nikdy nie nula namiesto "nevieme").
    pub fn value(&self, metric: Metric) -> Option<f64> {
        match self.state {
            DayState::Future | DayState::NoHistory => None,
            DayState::Unpriced if metric == Metric::Cost => None,
            _ => Some(match metric {
                Metric::Tokens => self.tokens as f64,
                Metric::Cost => self.cost,
            }),
        }
    }
}

/// Odkial su sucty dna: z aktualne citatelnych logov, z archivu, alebo oboje (filter "Vsetci").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Live,
    Archived,
    Mixed,
}

/// Poznamka k uplnosti historie (text prekladá UI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// Historia poskytovatela v tomto mesiaci zacina az od `from` (skorsie dni nie su nula, ale nezname).
    BeginsAt(Provider, Ymd),
    /// Cely mesiac je pred zaciatkom historie.
    BeforeHistory,
    /// Niektore dni mesiaca su z archivu (zdroj ich uz necita).
    Archived,
    /// Poskytovatel nema zapnuty lokalny zdroj dennej historie.
    Unsupported(Provider),
    /// Niektore subory zdroja sa nepodarilo precitat - sucty mozu byt nizsie.
    ReadErrors(Provider),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonthView {
    pub y: i32,
    pub m: u32,
    pub cells: Vec<DayCell>,
    pub notes: Vec<Note>,
    /// Sucet mesiaca zo znamych dni (tokeny, odhad ceny, neocenene volania). Neznamy den nie je v sucte.
    pub total_tokens: u64,
    pub total_cost: f64,
    pub total_unpriced: u64,
    /// Najvacsia hodnota pre `metric` (mierka grafu; 0 = ziadna).
    pub max: f64,
    /// Pocet dni bez ciselneho udaju (bez historie / cena neznama) a dni so znamou hodnotou (aj skutocna nula). Ak nie je
    /// ziaden znamy den, suma sa nezobrazi ako 0, ale ako "neznama".
    pub unknown_days: usize,
    pub known_days: usize,
    /// Pocet dni so znamou hodnotou, ktore su (aspon ciastocne) z archivu.
    pub archived_days: usize,
}

/// Pokrytie lokalnej historie jednym poskytovatelom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub provider: Provider,
    /// Prvy lokalny den so zaznamom v **aktualne citatelnych** logoch (`None` = zdroj je vypnuty alebo nema zaznam).
    pub from: Option<Ymd>,
    /// Suvisle rozsahy dni, ktore sa uspesne naskenovali a drzi ich archiv (vratane oboch koncov).
    pub ranges: Vec<(Ymd, Ymd)>,
    pub read_errors: bool,
}

impl Coverage {
    /// Je den `d` pokryty (zdroj ho cita, alebo ho pokryva archiv)?
    pub fn covers(&self, d: Ymd) -> bool {
        self.live(d) || self.ranges.iter().any(|(a, b)| *a <= d && d <= *b)
    }

    /// Cita zdroj tento den aj teraz?
    pub fn live(&self, d: Ymd) -> bool {
        self.from.is_some_and(|f| f <= d)
    }

    /// Najskorsi pokryty den.
    pub fn begin(&self) -> Option<Ymd> {
        self.from
            .into_iter()
            .chain(self.ranges.iter().map(|r| r.0))
            .min()
    }
}

/// Pokrytie zo stavov zdrojov (len zdroje lokalnych prepisov - statusline nema denne tokeny) a z archivu.
pub fn coverage(c: &Collected, local_day: &dyn Fn(i64) -> Ymd) -> Vec<Coverage> {
    let ranges_of = |p: Provider| -> Vec<(Ymd, Ymd)> {
        c.cover
            .iter()
            .filter(|r| r.provider == p)
            .map(|r| {
                (
                    Ymd::new(r.from.0, r.from.1, r.from.2),
                    Ymd::new(r.to.0, r.to.1, r.to.2),
                )
            })
            .collect()
    };
    let mut out: Vec<Coverage> = c
        .statuses
        .iter()
        .filter(|s| matches!(s.kind, SourceKind::ClaudeLocal | SourceKind::CodexLocal))
        .filter_map(|s| {
            let provider = s.provider.or(match s.kind {
                SourceKind::ClaudeLocal => Some(Provider::ClaudeCode),
                SourceKind::CodexLocal => Some(Provider::Codex),
                _ => None,
            })?;
            Some(Coverage {
                provider,
                from: s.oldest.map(local_day),
                ranges: ranges_of(provider),
                read_errors: matches!(s.error, Some(SourceError::ReadErrors(_))),
            })
        })
        .collect();
    // poskytovatel, ktoreho lokalny zdroj je teraz vypnuty, ale archiv ma jeho dni
    for p in Provider::ALL {
        if !out.iter().any(|v| v.provider == p) {
            let ranges = ranges_of(p);
            if !ranges.is_empty() {
                out.push(Coverage {
                    provider: p,
                    from: None,
                    ranges,
                    read_errors: false,
                });
            }
        }
    }
    out
}

fn matches_filter(f: Filter, p: Option<Provider>) -> bool {
    match f {
        Filter::All => true,
        Filter::Only(x) => p == Some(x),
    }
}

/// Najskorsi den, od ktoreho existuje nejaka historia (pre vypnutie "Predosly" pri prvom mesiaci s udajmi).
pub fn earliest(c: &Collected, local_day: &dyn Fn(i64) -> Ymd) -> Option<Ymd> {
    coverage(c, local_day)
        .iter()
        .filter_map(Coverage::begin)
        .min()
}

/// Stav a hodnoty jedneho dna (bez vypoctu sumy mesiaca).
fn cell(
    c: &Collected,
    cov: &[Coverage],
    wanted: &[Provider],
    date: Ymd,
    today: Ymd,
    metric: Metric,
    filter: Filter,
) -> DayCell {
    let mut cell = DayCell {
        date,
        state: DayState::NoHistory,
        tokens: 0,
        cost: 0.0,
        requests: 0,
        unpriced: 0,
        origin: Origin::Live,
        price_dates: Vec::new(),
    };
    if date > today {
        cell.state = DayState::Future;
        return cell;
    }
    let key = (date.y, date.m, date.d);
    let rows: Vec<&DayTotals> = c
        .days
        .iter()
        .filter(|d| d.day == key && matches_filter(filter, d.provider))
        .collect();
    for d in &rows {
        cell.tokens += d.input + d.output + d.cache_read + d.cache_write;
        cell.cost += d.est_usd;
        cell.requests += d.requests;
        cell.unpriced += d.unpriced;
    }
    // pokrytie dna: kazdy chceny poskytovatel, ktoreho zdroj den cita alebo ktoreho den pokryva archiv
    let covering: Vec<&Coverage> = cov
        .iter()
        .filter(|v| wanted.contains(&v.provider))
        .filter(|v| v.covers(date))
        .collect();
    // pôvod: z riadkov dna (archivovane / cerstve); den bez riadkov (skutocna nula) podla pokrytia
    let archived_rows = rows.iter().filter(|d| d.archived).count();
    cell.origin = if !rows.is_empty() {
        match archived_rows {
            0 => Origin::Live,
            n if n == rows.len() => Origin::Archived,
            _ => Origin::Mixed,
        }
    } else {
        let live = covering.iter().filter(|v| v.live(date)).count();
        match (live, covering.len()) {
            (_, 0) => Origin::Live,
            (0, _) => Origin::Archived,
            (l, n) if l == n => Origin::Live,
            _ => Origin::Mixed,
        }
    };
    let mut dates: Vec<String> = rows
        .iter()
        .filter(|d| d.archived && !d.price_date.is_empty())
        .map(|d| d.price_date.clone())
        .collect();
    dates.sort();
    dates.dedup();
    cell.price_dates = dates;
    // hranicny den zdroja mohol zacat uprostred dna - okrem dna, ktore uz drzi **uplny** archivovany zaznam
    let complete_archived = !rows.is_empty() && rows.iter().all(|d| d.archived && !d.partial);
    let boundary = |v: &Coverage| v.from == Some(date) && !complete_archived;
    let partial_rows = rows
        .iter()
        .any(|d| d.partial && covering.iter().any(|v| Some(v.provider) == d.provider));
    cell.state = if covering.is_empty() {
        DayState::NoHistory
    } else if covering.len() < wanted.len()
        // chyby citania = sucty mozu byt nizsie (len dni, ktore sa citaju teraz)
        || covering.iter().any(|v| boundary(v) || (v.read_errors && v.live(date)))
        // den z archivu zapisany ako neuplny ostava dolnym odhadom
        || partial_rows
    {
        DayState::Incomplete
    } else if metric == Metric::Cost && cell.requests > 0 && cell.unpriced == cell.requests {
        DayState::Unpriced
    } else {
        DayState::Recorded
    };
    // aj neucelny den moze mat neocenene volania: cena je len cast (stav ostava Incomplete / Recorded, UI ukaze "≥")
    cell
}

/// Mesiac `y`-`m` pre `filter` a `metric`. `providers` = poskytovatelia zapnuti pouzivatelom (karty); tie bez lokalneho
/// zdroja dennej historie sa uvedu v poznamkach (`Unsupported`) a nikdy sa necitaju ako nula.
#[allow(clippy::too_many_arguments)]
pub fn month(
    c: &Collected,
    providers: &[Provider],
    y: i32,
    m: u32,
    today: Ymd,
    filter: Filter,
    metric: Metric,
    local_day: &dyn Fn(i64) -> Ymd,
) -> MonthView {
    let cov = coverage(c, local_day);
    let in_filter: Vec<Provider> = providers
        .iter()
        .copied()
        .filter(|p| matches_filter(filter, Some(*p)))
        .collect();
    // chceny = v filtri a s lokalnym zdrojom alebo archivom (tie bez oboch nie su "pokryte")
    let wanted: Vec<Provider> = in_filter
        .iter()
        .copied()
        .filter(|p| cov.iter().any(|v| v.provider == *p))
        .collect();
    let mut notes = Vec::new();
    for p in &in_filter {
        if !wanted.contains(p) {
            notes.push(Note::Unsupported(*p));
        }
    }
    let n = days_in_month(y, m);
    let cells: Vec<DayCell> = (1..=n)
        .map(|d| cell(c, &cov, &wanted, Ymd::new(y, m, d), today, metric, filter))
        .collect();
    let month_start = Ymd::new(y, m, 1);
    let month_end = Ymd::new(y, m, n);
    for v in cov.iter().filter(|v| wanted.contains(&v.provider)) {
        if v.read_errors {
            notes.push(Note::ReadErrors(v.provider));
        }
        if let Some(f) = v.begin() {
            if f > month_start && f <= month_end && f <= today {
                notes.push(Note::BeginsAt(v.provider, f));
            }
        }
    }
    let known = |c: &DayCell| c.value(metric).is_some();
    if !wanted.is_empty()
        && cells
            .iter()
            .all(|c| matches!(c.state, DayState::NoHistory | DayState::Future))
        && month_start <= today
    {
        notes.push(Note::BeforeHistory);
    }
    let (mut tokens, mut cost, mut unpriced) = (0u64, 0.0f64, 0u64);
    let mut max = 0.0f64;
    for c in cells.iter().filter(|c| known(c)) {
        tokens += c.tokens;
        cost += c.cost;
        unpriced += c.unpriced;
        max = max.max(c.value(metric).unwrap_or(0.0));
    }
    let unknown_days = cells
        .iter()
        .filter(|c| matches!(c.state, DayState::NoHistory | DayState::Unpriced))
        .count();
    let known_days = cells.iter().filter(|c| known(c)).count();
    let archived_days = cells
        .iter()
        .filter(|c| known(c) && c.origin != Origin::Live)
        .count();
    if archived_days > 0 {
        notes.push(Note::Archived);
    }
    MonthView {
        y,
        m,
        cells,
        notes,
        total_tokens: tokens,
        total_cost: cost,
        total_unpriced: unpriced,
        max,
        unknown_days,
        known_days,
        archived_days,
    }
}

/// Rozpis dna podla poskytovatela a modelu (zoradene: poskytovatel, potom tokeny zostupne).
pub fn day_rows(c: &Collected, date: Ymd, filter: Filter) -> Vec<ModelTotals> {
    let key = (date.y, date.m, date.d);
    let mut out: Vec<ModelTotals> = Vec::new();
    for d in c
        .days
        .iter()
        .filter(|d: &&DayTotals| d.day == key && matches_filter(filter, d.provider))
    {
        let row = match out
            .iter_mut()
            .find(|r| r.provider == d.provider && r.model == d.model)
        {
            Some(r) => r,
            None => {
                out.push(ModelTotals {
                    provider: d.provider,
                    model: d.model.clone(),
                    ..Default::default()
                });
                out.last_mut().expect("prave pridane")
            }
        };
        row.input += d.input;
        row.output += d.output;
        row.cache_read += d.cache_read;
        row.cache_write += d.cache_write;
        row.requests += d.requests;
        row.est_usd += d.est_usd;
        row.unpriced += d.unpriced;
    }
    out.sort_by(|a, b| {
        a.provider.cmp(&b.provider).then(
            (b.input + b.output + b.cache_read + b.cache_write)
                .cmp(&(a.input + a.output + a.cache_read + a.cache_write)),
        )
    });
    out
}

/// Poradove cislo dna v roku-mesiaci pre testy (pocet dni medzi dvoma datumami).
pub fn days_between(a: Ymd, b: Ymd) -> i64 {
    days_from_civil(b) - days_from_civil(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceKind, SourceStatus};

    /// Lokalny cas = UTC+1 (nie UTC: udalosti okolo polnoci musia padnut do spravneho lokalneho dna).
    fn local(ts: i64) -> Ymd {
        transom_core::clock::civil_from_days((ts + 3600).div_euclid(86_400))
    }
    fn ts(y: i32, m: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(Ymd::new(y, m, d)) * 86_400 + h * 3600 + mi * 60 - 3600
    }
    fn d(y: i32, m: u32, dd: u32) -> Ymd {
        Ymd::new(y, m, dd)
    }
    fn day(p: Provider, model: &str, date: (i32, u32, u32), tokens: u64, usd: f64) -> DayTotals {
        DayTotals {
            provider: Some(p),
            model: model.into(),
            day: date,
            input: tokens,
            requests: 1,
            est_usd: usd,
            ..Default::default()
        }
    }
    fn status(kind: SourceKind, p: Provider, oldest: Option<i64>) -> SourceStatus {
        SourceStatus {
            kind,
            provider: Some(p),
            oldest,
            ..Default::default()
        }
    }
    const CL: Provider = Provider::ClaudeCode;
    const CX: Provider = Provider::Codex;

    fn data() -> Collected {
        Collected {
            days: vec![
                // Claude: historia od 5. 11. (prvy den = castocny), dni 5, 6, 9
                day(CL, "claude-a", (2026, 11, 5), 100, 1.0),
                day(CL, "claude-a", (2026, 11, 6), 200, 2.0),
                day(CL, "claude-b", (2026, 11, 6), 50, 0.5),
                day(CL, "claude-a", (2026, 11, 9), 400, 4.0),
                // Codex: historia od 8. 11.
                day(CX, "gpt-x", (2026, 11, 9), 1000, 3.0),
            ],
            statuses: vec![
                status(SourceKind::ClaudeLocal, CL, Some(ts(2026, 11, 5, 8, 0))),
                status(SourceKind::CodexLocal, CX, Some(ts(2026, 11, 8, 10, 0))),
                // statusline nema denne tokeny: nesmie rozsirit pokrytie
                status(
                    SourceKind::ClaudeStatusline,
                    CL,
                    Some(ts(2026, 11, 1, 8, 0)),
                ),
            ],
            ..Default::default()
        }
    }
    fn nov(f: Filter, m: Metric) -> MonthView {
        month(&data(), &[CL, CX], 2026, 11, d(2026, 11, 12), f, m, &local)
    }
    fn st(v: &MonthView, day: u32) -> DayState {
        v.cells[day as usize - 1].state
    }

    #[test]
    fn month_has_one_cell_per_day_including_boundaries_and_leap_february() {
        let c = Collected::default();
        let t = d(2030, 1, 1);
        for (y, m, n) in [
            (2026, 11, 30),
            (2026, 12, 31),
            (2024, 2, 29),
            (2026, 2, 28),
            (2026, 1, 31),
        ] {
            let v = month(&c, &[CL], y, m, t, Filter::All, Metric::Tokens, &local);
            assert_eq!(v.cells.len(), n, "{y}-{m}");
            assert_eq!(v.cells[0].date, d(y, m, 1));
            assert_eq!(v.cells[n - 1].date, d(y, m, n as u32));
        }
    }

    #[test]
    fn days_before_history_are_unknown_not_zero_and_future_days_are_future() {
        let v = nov(Filter::Only(CL), Metric::Tokens);
        assert_eq!(st(&v, 1), DayState::NoHistory);
        assert_eq!(st(&v, 4), DayState::NoHistory);
        assert_eq!(v.cells[0].value(Metric::Tokens), None, "nevieme != 0");
        assert_eq!(
            st(&v, 5),
            DayState::Incomplete,
            "prvy den historie mohol zacat uprostred dna"
        );
        assert_eq!(st(&v, 7), DayState::Recorded);
        assert_eq!(
            v.cells[6].value(Metric::Tokens),
            Some(0.0),
            "pokryty den bez zaznamu = skutocna nula"
        );
        assert_eq!(st(&v, 12), DayState::Recorded, "dnes");
        assert_eq!(st(&v, 13), DayState::Future);
        assert_eq!(v.cells[12].value(Metric::Tokens), None);
        assert!(v.notes.contains(&Note::BeginsAt(CL, d(2026, 11, 5))));
    }

    #[test]
    fn provider_filter_selects_rows_and_totals() {
        let all = nov(Filter::All, Metric::Tokens);
        let cl = nov(Filter::Only(CL), Metric::Tokens);
        let cx = nov(Filter::Only(CX), Metric::Tokens);
        assert_eq!(all.cells[8].tokens, 1400); // 9. 11. Claude 400 + Codex 1000
        assert_eq!(cl.cells[8].tokens, 400);
        assert_eq!(cx.cells[8].tokens, 1000);
        assert_eq!(cl.total_tokens, 100 + 200 + 50 + 400);
        assert_eq!(cx.total_tokens, 1000);
        assert_eq!(all.total_tokens, cl.total_tokens + cx.total_tokens);
        // Codex pred 8. 11. nema historiu
        assert_eq!(st(&cx, 7), DayState::NoHistory);
        assert_eq!(st(&cx, 8), DayState::Incomplete);
        assert_eq!(st(&cx, 10), DayState::Recorded);
    }

    #[test]
    fn all_providers_marks_days_only_some_cover_as_incomplete() {
        let v = nov(Filter::All, Metric::Tokens);
        // 6. a 7. 11.: Claude uz ma historiu, Codex este nie => spodny odhad, nie uplny den
        assert_eq!(st(&v, 6), DayState::Incomplete);
        assert_eq!(st(&v, 7), DayState::Incomplete);
        // 8. = prvy den Codexu (castocny), 9. 11. pokryte oboma
        assert_eq!(st(&v, 8), DayState::Incomplete);
        assert_eq!(st(&v, 9), DayState::Recorded);
        assert_eq!(st(&v, 3), DayState::NoHistory, "ani jeden nema historiu");
        assert!(v.notes.contains(&Note::BeginsAt(CX, d(2026, 11, 8))));
    }

    #[test]
    fn provider_without_local_source_is_unsupported_and_never_read_as_zero() {
        let mut c = data();
        c.statuses.retain(|s| s.kind != SourceKind::CodexLocal);
        let v = month(
            &c,
            &[CL, CX],
            2026,
            11,
            d(2026, 11, 12),
            Filter::Only(CX),
            Metric::Tokens,
            &local,
        );
        assert!(v.notes.contains(&Note::Unsupported(CX)));
        assert!(v.cells[..12]
            .iter()
            .all(|c| c.state == DayState::NoHistory && c.value(Metric::Tokens).is_none()));
        assert_eq!(v.total_tokens, 0);
        // vo filtri Vsetci sa pokryva len Claude a Codex je v poznamke
        let all = month(
            &c,
            &[CL, CX],
            2026,
            11,
            d(2026, 11, 12),
            Filter::All,
            Metric::Tokens,
            &local,
        );
        assert!(all.notes.contains(&Note::Unsupported(CX)));
        assert_eq!(st(&all, 7), DayState::Recorded);
    }

    #[test]
    fn read_errors_make_covered_days_incomplete_with_a_note() {
        let mut c = data();
        c.statuses[0].error = Some(SourceError::ReadErrors(2));
        let v = month(
            &c,
            &[CL],
            2026,
            11,
            d(2026, 11, 12),
            Filter::Only(CL),
            Metric::Tokens,
            &local,
        );
        assert_eq!(st(&v, 7), DayState::Incomplete);
        assert_eq!(st(&v, 4), DayState::NoHistory);
        assert!(v.notes.contains(&Note::ReadErrors(CL)));
    }

    #[test]
    fn whole_month_before_history_says_so() {
        let v = month(
            &data(),
            &[CL, CX],
            2026,
            10,
            d(2026, 11, 12),
            Filter::All,
            Metric::Tokens,
            &local,
        );
        assert!(v.cells.iter().all(|c| c.state == DayState::NoHistory));
        assert!(v.notes.contains(&Note::BeforeHistory));
        assert_eq!(v.total_tokens, 0);
        assert_eq!(v.unknown_days, 31);
        assert_eq!(
            v.known_days, 0,
            "ziaden znamy den => suma sa nema ukazat ako 0"
        );
        // buduci mesiac: ziadna poznamka "pred historiou", dni su Future
        let f = month(
            &data(),
            &[CL],
            2026,
            12,
            d(2026, 11, 12),
            Filter::All,
            Metric::Tokens,
            &local,
        );
        assert!(f.cells.iter().all(|c| c.state == DayState::Future));
        assert!(!f.notes.contains(&Note::BeforeHistory));
    }

    #[test]
    fn cost_is_an_estimate_with_unpriced_days_unknown_not_zero() {
        let mut c = data();
        // 10. 11.: len neocenene volania; 6. 11.: jedno neocenene volanie navyse (cast ceny)
        c.days.push(DayTotals {
            provider: Some(CL),
            model: "claude-unknown".into(),
            day: (2026, 11, 10),
            input: 70,
            requests: 3,
            unpriced: 3,
            ..Default::default()
        });
        c.days[1].unpriced = 1;
        c.days[1].requests = 2;
        let v = month(
            &c,
            &[CL],
            2026,
            11,
            d(2026, 11, 12),
            Filter::Only(CL),
            Metric::Cost,
            &local,
        );
        assert_eq!(st(&v, 10), DayState::Unpriced);
        assert_eq!(v.cells[9].value(Metric::Cost), None, "neznama cena != $0");
        assert_eq!(
            v.cells[9].value(Metric::Tokens),
            Some(70.0),
            "tokeny su znamy udaj"
        );
        // 6. 11.: spolu 2.0 + 0.5 USD odhad, 1 neocenene volanie => cast ceny
        assert!((v.cells[5].cost - 2.5).abs() < 1e-9);
        assert_eq!(v.cells[5].unpriced, 1);
        // sucet mesiaca ma len ocenitelne dni a pocet neocenenych volani
        assert!((v.total_cost - (1.0 + 2.5 + 4.0)).abs() < 1e-9);
        assert_eq!(
            v.total_unpriced, 1,
            "neocenene volania dna Unpriced sa do sumy nepocitaju (den je neznamy)"
        );
        // v rezime tokenov je ten isty den znamy
        let t = month(
            &c,
            &[CL],
            2026,
            11,
            d(2026, 11, 12),
            Filter::Only(CL),
            Metric::Tokens,
            &local,
        );
        assert_eq!(st(&t, 10), DayState::Recorded);
    }

    #[test]
    fn events_near_local_midnight_land_in_the_right_local_day() {
        // 23:30 UTC = 00:30 lokalne nasledujuceho dna (UTC+1): rovnaka logika ako `by_day`
        let ev = |utc: i64| crate::Event {
            provider: Some(CL),
            model: "claude-a".into(),
            ts: utc,
            input: 10,
            ..Default::default()
        };
        let a = days_from_civil(d(2026, 11, 9)) * 86_400 + 23 * 3600 + 30 * 60; // 9. 11. 23:30 UTC = 10. 11. 00:30 lokalne
        let b = days_from_civil(d(2026, 11, 9)) * 86_400 + 22 * 3600 + 59 * 60; // 9. 11. 22:59 UTC = 9. 11. 23:59 lokalne
        let c = Collected {
            days: crate::by_day(&[ev(a), ev(b)], &local),
            statuses: vec![status(
                SourceKind::ClaudeLocal,
                CL,
                Some(ts(2026, 11, 1, 0, 0)),
            )],
            ..Default::default()
        };
        let v = month(
            &c,
            &[CL],
            2026,
            11,
            d(2026, 11, 20),
            Filter::All,
            Metric::Tokens,
            &local,
        );
        assert_eq!(v.cells[8].tokens, 10, "9. 11. lokalne");
        assert_eq!(v.cells[9].tokens, 10, "10. 11. lokalne");
        // hranica mesiaca: 30. 11. 23:30 UTC = 1. 12. 00:30 lokalne
        let eom = days_from_civil(d(2026, 11, 30)) * 86_400 + 23 * 3600 + 30 * 60;
        let c2 = Collected {
            days: crate::by_day(&[ev(eom)], &local),
            statuses: c.statuses.clone(),
            ..Default::default()
        };
        let nov = month(
            &c2,
            &[CL],
            2026,
            11,
            d(2026, 12, 5),
            Filter::All,
            Metric::Tokens,
            &local,
        );
        let dec = month(
            &c2,
            &[CL],
            2026,
            12,
            d(2026, 12, 5),
            Filter::All,
            Metric::Tokens,
            &local,
        );
        assert_eq!(nov.total_tokens, 0, "polnocny zaznam nepatri do novembra");
        assert_eq!(dec.cells[0].tokens, 10, "1. 12. lokalne");
    }

    #[test]
    fn day_rows_break_down_provider_and_model_and_sort() {
        let c = data();
        let rows = day_rows(&c, d(2026, 11, 6), Filter::All);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model, "claude-a");
        assert_eq!(rows[1].model, "claude-b");
        let nine = day_rows(&c, d(2026, 11, 9), Filter::All);
        assert_eq!(nine.len(), 2);
        assert_eq!(nine[0].provider, Some(CL), "najprv poskytovatel");
        assert_eq!(day_rows(&c, d(2026, 11, 9), Filter::Only(CX)).len(), 1);
        assert!(day_rows(&c, d(2026, 11, 20), Filter::All).is_empty());
    }

    #[test]
    fn max_scale_ignores_unknown_days_and_earliest_is_the_oldest_local_source() {
        let v = nov(Filter::All, Metric::Tokens);
        assert_eq!(v.max, 1400.0);
        assert_eq!(earliest(&data(), &local), Some(d(2026, 11, 5)));
        assert_eq!(earliest(&Collected::default(), &local), None);
        assert_eq!(days_between(d(2026, 11, 5), d(2026, 11, 9)), 4);
    }

    #[test]
    fn daily_sum_equals_the_period_summary() {
        // sucet dni mesiaca (tokeny) musi sediet so `summarize` za mesiac - ta ista definicia tokenov
        let c = data();
        let today = d(2026, 11, 12);
        let v = nov(Filter::All, Metric::Tokens);
        let model_sum: u64 = crate::summarize(&c.days, today, crate::Period::Month)
            .iter()
            .map(|r| r.input + r.output + r.cache_read + r.cache_write)
            .sum();
        assert_eq!(v.total_tokens, model_sum);
    }
}
