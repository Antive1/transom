//! Udalosti kalendara: reset limitov AI a fakturacia/obnovenie predplatneho.
//!
//! - **Automaticky** reset: len ak zdroj zaznamenal `resets_at` okna (spolahlivy casovy znak; aj zo starsieho zaznamu -
//!   vtedy je udalost oznacena ako zastarana) a reset este nenastal. Den sa berie ako **miestny kalendarny den**
//!   okamihu resetu (`local_day`), takze zmena casoveho pasma ani letny cas nic neposunu "naviac".
//! - **Fakturacia / obnovenie** sa z ziadneho zdokumentovaneho zdroja zistit neda (statusline ani Codex ju nehlasia, ucet sa
//!   neskrabe a nic sa neodvodzuje z limitov), preto je len **rucna** (`usage.dates`). Rucny datum sa nikdy neprepisuje
//!   automatickym: ak aj zdroj hlasi reset v ten isty den, v kalendari su dve udalosti s oznacenim "automaticky" / "rucne".
//! - Rucne pravidla (jednorazovo / kazdych N dni / mesacne) su cisto datumove (`transom_core::schedule`).
use transom_config::ManualDate;
use transom_core::clock::Ymd;
use transom_core::schedule::{rule_from_parts, Repeat, Rule};
use transom_usage::limits::{self, Freshness};
use transom_usage::{PlanSnapshot, Provider};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventKind {
    Reset,
    Billing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Zo zdroja dat (`stale` = z posledneho zaznamu, nie cerstvy).
    Automatic { stale: bool },
    /// Rucne zadane v nastaveniach (s opakovanim).
    Manual(Repeat),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalEvent {
    pub date: Ymd,
    pub kind: EventKind,
    pub provider: Provider,
    pub source: Source,
    /// Okno limitu (len automaticky reset).
    pub window_minutes: Option<u32>,
    /// Okamih resetu (Unix sekundy; len automaticky reset) - pre zobrazenie casu.
    pub at: Option<i64>,
}

/// Rucny zaznam ako pravidlo (`None` = neplatny zaznam, vynechava sa).
pub fn manual_rule(m: &ManualDate) -> Option<(Provider, EventKind, Rule)> {
    let provider = match m.provider.as_str() {
        "claude" => Provider::ClaudeCode,
        "codex" => Provider::Codex,
        _ => return None,
    };
    let kind = match m.kind.as_str() {
        "reset" => EventKind::Reset,
        "billing" => EventKind::Billing,
        _ => return None,
    };
    Some((
        provider,
        kind,
        rule_from_parts(&m.date, &m.repeat, m.every_days)?,
    ))
}

#[cfg(test)]
/// Najblizsi vyskyt rucneho zaznamu v den `today` alebo neskor (pre nastavenia: "dalsi: ..."); `None` = jednorazovy
/// datum uz minul alebo zaznam je neplatny.
pub fn next_occurrence(m: &ManualDate, today: Ymd) -> Option<Ymd> {
    manual_rule(m)?.2.next_on_or_after(today)
}

/// Udalosti v dnoch `from..=to` (vzostupne podla dna, potom reset pred fakturaciou, poskytovatela).
/// `providers` = poskytovatelia so zapnutym zdrojom (automaticke resety len pre nich); rucne zaznamy sa ukazuju vzdy.
#[allow(clippy::too_many_arguments)]
pub fn events(
    plans: &[PlanSnapshot],
    providers: &[Provider],
    dates: &[ManualDate],
    from: Ymd,
    to: Ymd,
    now: i64,
    max_age_s: i64,
    local_day: &dyn Fn(i64) -> Ymd,
) -> Vec<CalEvent> {
    let mut out = Vec::new();
    for &prov in providers {
        for r in limits::provider_windows(plans, prov, now, max_age_s) {
            // len spolahlivy cas resetu, ktory este nenastal (po case resetu sa nepredpoklada nic)
            let Some(t) = r
                .resets_at
                .filter(|t| *t > now && r.freshness != Freshness::Expired)
            else {
                continue;
            };
            let date = local_day(t);
            if date >= from && date <= to {
                out.push(CalEvent {
                    date,
                    kind: EventKind::Reset,
                    provider: prov,
                    source: Source::Automatic {
                        stale: r.freshness == Freshness::LastRecorded,
                    },
                    window_minutes: Some(r.minutes),
                    at: Some(t),
                });
            }
        }
    }
    for m in dates {
        let Some((provider, kind, rule)) = manual_rule(m) else {
            continue;
        };
        for date in rule.occurrences(from, to) {
            out.push(CalEvent {
                date,
                kind,
                provider,
                source: Source::Manual(rule.repeat),
                window_minutes: None,
                at: None,
            });
        }
    }
    out.sort_by_key(|e| {
        (
            e.date,
            e.kind,
            e.provider,
            matches!(e.source, Source::Manual(_)),
            e.window_minutes,
        )
    });
    out
}

/// Ktore druhy znaciek sa maju nakreslit v dni (tvar, nie farba: reset = kruh, fakturacia = kosostvorec).
pub fn kinds_on(events: &[CalEvent], day: Ymd) -> (bool, bool) {
    let on = |k| events.iter().any(|e| e.date == day && e.kind == k);
    (on(EventKind::Reset), on(EventKind::Billing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_usage::{LimitWindow, SnapshotSource};

    const NOW: i64 = 1_795_000_000; // ~2026-11-18 10:26 UTC

    /// Miestny den = UTC + posun v hodinach (simulacia casoveho pasma; letny cas = iny posun v inom case).
    fn day_at(offset_h: i64) -> impl Fn(i64) -> Ymd {
        move |ts| transom_core::clock::civil_from_days((ts + offset_h * 3600).div_euclid(86_400))
    }

    fn plan(p: Provider, at: i64, ws: &[(u32, Option<i64>)]) -> PlanSnapshot {
        PlanSnapshot {
            provider: Some(p),
            plan: String::new(),
            windows: ws
                .iter()
                .map(|&(minutes, resets_at)| LimitWindow {
                    minutes,
                    used_percent: 10.0,
                    resets_at,
                })
                .collect(),
            recorded_at: at,
            source: SnapshotSource::CodexSession,
        }
    }

    fn manual(provider: &str, kind: &str, date: &str, repeat: &str, n: u32) -> ManualDate {
        ManualDate {
            provider: provider.into(),
            kind: kind.into(),
            date: date.into(),
            repeat: repeat.into(),
            every_days: n,
        }
    }

    fn range() -> (Ymd, Ymd) {
        (Ymd::new(2026, 11, 1), Ymd::new(2026, 12, 31))
    }

    #[test]
    fn automatic_reset_from_a_reliable_timestamp_names_provider_window_and_date() {
        let plans = vec![plan(
            Provider::Codex,
            NOW - 60,
            &[(43_200, Some(NOW + 25 * 86_400)), (300, Some(NOW + 3_600))],
        )];
        let (a, b) = range();
        let ev = events(
            &plans,
            &[Provider::Codex],
            &[],
            a,
            b,
            NOW,
            1_800,
            &day_at(0),
        );
        assert_eq!(ev.len(), 2);
        let five = ev.iter().find(|e| e.window_minutes == Some(300)).unwrap();
        assert_eq!(
            (five.kind, five.provider, five.source),
            (
                EventKind::Reset,
                Provider::Codex,
                Source::Automatic { stale: false }
            )
        );
        assert_eq!(five.date, Ymd::new(2026, 11, 18));
        let month = ev
            .iter()
            .find(|e| e.window_minutes == Some(43_200))
            .unwrap();
        assert_eq!(month.date, Ymd::new(2026, 12, 13));
        assert_eq!(month.at, Some(NOW + 25 * 86_400));
    }

    #[test]
    fn stale_missing_passed_and_disabled_sources_produce_no_misleading_events() {
        let (a, b) = range();
        // zastaraly zaznam (starsi nez max_age): udalost je, ale oznacena zastarana
        let stale = vec![plan(
            Provider::Codex,
            NOW - 5 * 3600,
            &[(43_200, Some(NOW + 86_400))],
        )];
        let ev = events(
            &stale,
            &[Provider::Codex],
            &[],
            a,
            b,
            NOW,
            1_800,
            &day_at(0),
        );
        assert_eq!(ev[0].source, Source::Automatic { stale: true });
        // chybajuci cas resetu: ziadna udalost (nic sa nedopocita)
        let none = vec![plan(
            Provider::Codex,
            NOW - 60,
            &[(300, None), (10_080, None)],
        )];
        assert!(events(&none, &[Provider::Codex], &[], a, b, NOW, 1_800, &day_at(0)).is_empty());
        // reset uz presiel (aj hodnota <= 0): nic
        let passed = vec![plan(
            Provider::Codex,
            NOW - 600,
            &[(300, Some(NOW - 5)), (10_080, Some(0))],
        )];
        assert!(events(
            &passed,
            &[Provider::Codex],
            &[],
            a,
            b,
            NOW,
            1_800,
            &day_at(0)
        )
        .is_empty());
        // zdroj poskytovatela nie je zapnuty: ziadne automaticke udalosti
        let off = vec![plan(Provider::Codex, NOW - 60, &[(300, Some(NOW + 3_600))])];
        assert!(events(
            &off,
            &[Provider::ClaudeCode],
            &[],
            a,
            b,
            NOW,
            1_800,
            &day_at(0)
        )
        .is_empty());
        // mimo zobrazeneho useku
        let far = vec![plan(
            Provider::Codex,
            NOW - 60,
            &[(43_200, Some(NOW + 90 * 86_400))],
        )];
        assert!(events(&far, &[Provider::Codex], &[], a, b, NOW, 1_800, &day_at(0)).is_empty());
    }

    #[test]
    fn reset_day_is_the_local_calendar_day_across_midnight_and_time_zones() {
        // reset o 23:30 UTC: v UTC je to 20. 11., v UTC+2 uz 21. 11., v UTC-5 stale 20. 11.
        let t = transom_core::clock::days_from_civil(Ymd::new(2026, 11, 20)) * 86_400
            + 23 * 3600
            + 30 * 60;
        let plans = vec![plan(Provider::Codex, t - 3600, &[(300, Some(t))])];
        let (a, b) = range();
        let day = |off| {
            events(
                &plans,
                &[Provider::Codex],
                &[],
                a,
                b,
                t - 1800,
                99_999,
                &day_at(off),
            )[0]
            .date
        };
        assert_eq!(day(0), Ymd::new(2026, 11, 20));
        assert_eq!(day(2), Ymd::new(2026, 11, 21));
        assert_eq!(day(-5), Ymd::new(2026, 11, 20));
        // prechod letneho casu: rovnaky okamih, ina hodnota posunu - vzdy jeden den (nikdy dva ani ziadny)
        assert_eq!(day(1), Ymd::new(2026, 11, 21));
    }

    #[test]
    fn manual_dates_once_six_day_and_monthly_with_next_occurrence() {
        let dates = vec![
            manual("claude", "reset", "2026-11-20", "days", 6),
            manual("codex", "billing", "2026-11-05", "monthly", 6),
            manual("claude", "billing", "2026-12-24", "none", 6),
        ];
        let (a, b) = range();
        let ev = events(&[], &[], &dates, a, b, NOW, 1_800, &day_at(0));
        let resets: Vec<Ymd> = ev
            .iter()
            .filter(|e| e.kind == EventKind::Reset)
            .map(|e| e.date)
            .collect();
        assert_eq!(resets.first(), Some(&Ymd::new(2026, 11, 20)));
        assert_eq!(resets[1], Ymd::new(2026, 11, 26), "kazdych 6 dni");
        let billing: Vec<Ymd> = ev
            .iter()
            .filter(|e| e.kind == EventKind::Billing && e.provider == Provider::Codex)
            .map(|e| e.date)
            .collect();
        assert_eq!(billing, vec![Ymd::new(2026, 11, 5), Ymd::new(2026, 12, 5)]);
        assert!(ev.iter().any(|e| e.provider == Provider::ClaudeCode
            && e.kind == EventKind::Billing
            && e.date == Ymd::new(2026, 12, 24)));
        assert!(ev.iter().all(|e| matches!(e.source, Source::Manual(_))));
        // "dalsi vyskyt" je jednoznacny a upravitelny
        let today = Ymd::new(2026, 11, 27);
        assert_eq!(
            next_occurrence(&dates[0], today),
            Some(Ymd::new(2026, 12, 2))
        );
        assert_eq!(
            next_occurrence(&dates[1], today),
            Some(Ymd::new(2026, 12, 5))
        );
        assert_eq!(
            next_occurrence(&dates[2], Ymd::new(2026, 12, 25)),
            None,
            "jednorazovy datum minul"
        );
        assert_eq!(
            next_occurrence(&manual("claude", "reset", "2026-02-30", "none", 6), today),
            None
        );
        // rucne zaznamy sa ukazuju aj pre poskytovatela bez zapnuteho zdroja (je to rozhodnutie pouzivatela)
        assert!(!events(&[], &[], &dates, a, b, NOW, 1_800, &day_at(0)).is_empty());
    }

    #[test]
    fn automatic_and_manual_events_on_the_same_day_are_both_kept_and_labelled() {
        let t = NOW + 3_600;
        let plans = vec![plan(Provider::ClaudeCode, NOW - 60, &[(300, Some(t))])];
        let day = day_at(0)(t);
        let iso = transom_core::schedule::iso(day);
        let dates = vec![manual("claude", "reset", &iso, "none", 6)];
        let ev = events(
            &plans,
            &[Provider::ClaudeCode],
            &dates,
            day,
            day,
            NOW,
            1_800,
            &day_at(0),
        );
        assert_eq!(
            ev.len(),
            2,
            "rucny datum sa nikdy ticho neprepise automatickym"
        );
        assert!(ev
            .iter()
            .any(|e| matches!(e.source, Source::Automatic { .. })));
        assert!(ev.iter().any(|e| matches!(e.source, Source::Manual(_))));
        assert_eq!(kinds_on(&ev, day), (true, false));
        assert_eq!(kinds_on(&ev, day.max(Ymd::new(2026, 1, 1))), (true, false));
        assert_eq!(kinds_on(&ev, Ymd::new(2020, 1, 1)), (false, false));
    }

    #[test]
    fn markers_use_distinct_kinds_and_both_can_share_a_day() {
        let dates = vec![
            manual("claude", "reset", "2026-11-20", "none", 6),
            manual("codex", "billing", "2026-11-20", "none", 6),
        ];
        let d = Ymd::new(2026, 11, 20);
        let ev = events(&[], &[], &dates, d, d, NOW, 1_800, &day_at(0));
        assert_eq!(kinds_on(&ev, d), (true, true));
    }
}
