//! Mesacna historia s archivom: povod dni (zdroj / archiv / oboje), pokrytie, neuplne dni a chybajuce udaje.
use super::*;
use crate::archive::Cover;
use crate::history::{self, DayState, Filter, Metric, Note, Origin};
use transom_core::clock::Ymd;

const CL: Provider = Provider::ClaudeCode;
const CX: Provider = Provider::Codex;

fn d(y: i32, m: u32, dd: u32) -> Ymd {
    Ymd::new(y, m, dd)
}
fn utc(ts: i64) -> Ymd {
    transom_core::clock::civil_from_days(ts.div_euclid(86_400))
}
fn ts(y: i32, m: u32, dd: u32) -> i64 {
    transom_core::clock::days_from_civil(d(y, m, dd)) * 86_400 + 3600 * 8
}
fn row(p: Provider, day: (i32, u32, u32), tokens: u64, archived: bool, partial: bool) -> DayTotals {
    DayTotals {
        provider: Some(p),
        model: "m".into(),
        day,
        input: tokens,
        requests: 1,
        est_usd: tokens as f64 / 100.0,
        archived,
        partial,
        price_date: if archived {
            "2026-11-15".into()
        } else {
            String::new()
        },
        ..Default::default()
    }
}
fn live(p: Provider, oldest: Ymd) -> SourceStatus {
    SourceStatus {
        kind: if p == CL {
            SourceKind::ClaudeLocal
        } else {
            SourceKind::CodexLocal
        },
        provider: Some(p),
        oldest: Some(ts(oldest.y, oldest.m, oldest.d)),
        ..Default::default()
    }
}
fn cover(p: Provider, f: (i32, u32, u32), t: (i32, u32, u32)) -> Cover {
    Cover {
        provider: p,
        from: f,
        to: t,
    }
}
fn month(c: &Collected, f: Filter) -> history::MonthView {
    history::month(
        c,
        &[CL, CX],
        2026,
        11,
        d(2026, 11, 20),
        f,
        Metric::Tokens,
        &utc,
    )
}
fn st(v: &history::MonthView, day: u32) -> DayState {
    v.cells[day as usize - 1].state
}

/// Claude: archiv drzi 2.-9. 11. (logy zmazane), zdroj cita od 10. 11.; 2. 11. bol hranicny (neuplny) pri archivacii.
fn archived_then_live() -> Collected {
    Collected {
        days: vec![
            row(CL, (2026, 11, 2), 100, true, true),
            row(CL, (2026, 11, 3), 200, true, false),
            row(CL, (2026, 11, 5), 300, true, false),
            row(CL, (2026, 11, 10), 400, false, true), // hranicny den zdroja
            row(CL, (2026, 11, 12), 500, false, false),
        ],
        statuses: vec![live(CL, d(2026, 11, 10))],
        cover: vec![cover(CL, (2026, 11, 2), (2026, 11, 20))],
        ..Default::default()
    }
}

#[test]
fn archived_days_are_shown_with_origin_price_date_and_a_note() {
    let v = month(&archived_then_live(), Filter::Only(CL));
    let c3 = &v.cells[2];
    assert_eq!(
        (c3.state, c3.origin, c3.tokens),
        (DayState::Recorded, Origin::Archived, 200)
    );
    assert_eq!(c3.price_dates, vec!["2026-11-15".to_string()]);
    assert_eq!(
        v.cells[11].origin,
        Origin::Live,
        "12. 11. je z citatelnych logov"
    );
    assert!(v.cells[11].price_dates.is_empty());
    assert!(v.notes.contains(&Note::Archived));
    assert!(v.archived_days >= 2);
    // sucet mesiaca zahrna archivovane aj cerstve dni
    assert_eq!(v.total_tokens, 100 + 200 + 300 + 400 + 500);
}

#[test]
fn days_inside_archived_coverage_without_rows_are_real_zeros_from_the_archive() {
    let v = month(&archived_then_live(), Filter::Only(CL));
    // 4. 11. je v pokryti archivu a nema riadky: skutocna nula, povod archiv (nie "nevieme")
    let c4 = &v.cells[3];
    assert_eq!(
        (c4.state, c4.origin),
        (DayState::Recorded, Origin::Archived)
    );
    assert_eq!(c4.value(Metric::Tokens), Some(0.0));
    // 1. 11. je pred pokrytim: bez historie, nie nula
    assert_eq!(st(&v, 1), DayState::NoHistory);
    assert_eq!(v.cells[0].value(Metric::Tokens), None);
}

#[test]
fn gaps_between_archived_ranges_stay_unknown_and_never_become_zero() {
    let mut c = archived_then_live();
    c.statuses.clear(); // zdroj je vypnuty: ostava len archiv
    c.days.retain(|r| r.archived);
    c.cover = vec![
        cover(CL, (2026, 11, 2), (2026, 11, 6)),
        cover(CL, (2026, 11, 15), (2026, 11, 18)),
    ];
    let v = month(&c, Filter::Only(CL));
    assert!(
        !v.notes.contains(&Note::Unsupported(CL)),
        "archiv pokryva poskytovatela aj bez zdroja"
    );
    assert_eq!(st(&v, 5), DayState::Recorded);
    for day in 7..=14 {
        assert_eq!(
            st(&v, day),
            DayState::NoHistory,
            "{day}. 11. je diera medzi skenmi"
        );
        assert_eq!(v.cells[day as usize - 1].value(Metric::Tokens), None);
    }
    assert_eq!(st(&v, 16), DayState::Recorded);
    assert_eq!(st(&v, 19), DayState::NoHistory);
}

#[test]
fn partial_archived_day_stays_incomplete_but_a_complete_archived_boundary_day_is_complete() {
    let v = month(&archived_then_live(), Filter::Only(CL));
    assert_eq!(
        st(&v, 2),
        DayState::Incomplete,
        "2. 11. bol pri archivacii hranicny a neuplny"
    );
    assert_eq!(st(&v, 3), DayState::Recorded);
    assert_eq!(
        st(&v, 10),
        DayState::Incomplete,
        "hranicny den aktualnych logov"
    );
    // hranicny den zdroja, ktory vsak uz drzi UPLNY archivovany zaznam, je uplny
    let mut c = archived_then_live();
    c.days.retain(|r| r.day != (2026, 11, 10));
    c.days.push(row(CL, (2026, 11, 10), 450, true, false));
    let v = month(&c, Filter::Only(CL));
    assert_eq!(st(&v, 10), DayState::Recorded);
    assert_eq!(v.cells[9].origin, Origin::Archived);
}

#[test]
fn all_providers_can_mix_archived_and_live_days() {
    let mut c = archived_then_live();
    c.days.push(row(CX, (2026, 11, 11), 50, false, false));
    c.days.push(row(CX, (2026, 11, 3), 60, true, false)); // Codex: archivovany den 3. 11.
    c.statuses.push(live(CX, d(2026, 11, 8)));
    c.cover.push(cover(CX, (2026, 11, 3), (2026, 11, 20)));
    let v = month(&c, Filter::All);
    // 3. 11.: Claude archiv + Codex archiv => archiv
    assert_eq!(v.cells[2].origin, Origin::Archived);
    // 12. 11.: len Claude (live) a Codex pokryty live => live
    assert_eq!(v.cells[11].origin, Origin::Live);
    // zmiesany den: Claude z archivu a Codex z logov
    c.days.push(row(CX, (2026, 11, 5), 20, false, false));
    let v = month(&c, Filter::All);
    assert_eq!(
        v.cells[4].origin,
        Origin::Mixed,
        "Claude 5. 11. z archivu, Codex z logov"
    );
    assert_eq!(v.cells[4].tokens, 320);
}

#[test]
fn earliest_day_includes_archived_coverage() {
    let c = archived_then_live();
    assert_eq!(
        history::earliest(&c, &utc),
        Some(d(2026, 11, 2)),
        "archiv siaha dalej nez citatelne logy"
    );
    let none = Collected::default();
    assert_eq!(history::earliest(&none, &utc), None);
}

#[test]
fn estimated_cost_for_archived_days_carries_the_price_table_date_and_is_not_unknown() {
    let v = history::month(
        &archived_then_live(),
        &[CL],
        2026,
        11,
        d(2026, 11, 20),
        Filter::Only(CL),
        Metric::Cost,
        &utc,
    );
    let c5 = &v.cells[4];
    assert_eq!(c5.state, DayState::Recorded);
    assert!((c5.cost - 3.0).abs() < 1e-9);
    assert_eq!(c5.price_dates, vec!["2026-11-15".to_string()]);
    assert!(v.total_cost > 0.0);
}
