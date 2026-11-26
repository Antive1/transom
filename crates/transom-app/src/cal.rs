//! Stav kalendara v popoveri (zobrazeny mesiac, zvoleny den, hover) a jeho kreslenie.
use crate::ai::window_name;
use crate::caldata::{self, CalEvent, EventKind, Source};
use transom_core::clock::{add_months, header_order, month_grid, ClockOptions, Ymd};
use transom_core::schedule::Repeat;
use transom_i18n::{t, tf};
use transom_platform::clock as pclock;
use transom_render::calendar::{hit, render_calendar, CalHit, CalendarView, EventLine, MarkKind};
use transom_render::{Painter, Rect, Theme};
use transom_usage::{PlanSnapshot, Provider};

/// Vstupy pre udalosti kalendara (z konfiguracie a poslednych dat AI); pocitaju sa pri kazdom kresleni z uz nacitanych
/// udajov (ziadny dotaz na poskytovatela).
pub struct CalCtx<'a> {
    pub plans: &'a [PlanSnapshot],
    /// Poskytovatelia so zapnutym zdrojom (automaticke resety).
    pub providers: &'a [Provider],
    pub dates: &'a [transom_config::ManualDate],
    pub now: i64,
    pub max_age_s: i64,
    /// Pripojene kalendare (ICS) - uz nacitane; prazdne = ziadny.
    pub ics: &'a [std::sync::Arc<transom_core::ics::Calendar>],
    /// Je nejaky kalendar pripojeny (vysvetlivka sa ukaze aj pred prvym nacitanim).
    pub has_ics: bool,
}

/// Vyskyty udalosti z pripojenych kalendarov v dnoch `from..=to` (miestne dni).
pub fn ics_occurrences(
    cals: &[std::sync::Arc<transom_core::ics::Calendar>],
    from: Ymd,
    to: Ymd,
) -> Vec<transom_core::ics::Occurrence> {
    let a = pclock::utc_of_local(from, 0);
    let b = pclock::utc_of_local(
        transom_core::clock::civil_from_days(transom_core::clock::days_from_civil(to) + 1),
        0,
    );
    let l2u = |l: transom_core::ics::Local| pclock::utc_of_local(l.day, l.secs);
    let mut out: Vec<transom_core::ics::Occurrence> = cals
        .iter()
        .flat_map(|c| c.occurrences(a, b, from, to, &l2u))
        .collect();
    out.sort_by_key(|x| x.start);
    out
}

/// Dni (miestne), na ktore vyskyt pripada: celodenna udalost cez vsetky svoje dni, casova v den zaciatku.
pub fn occurrence_days(o: &transom_core::ics::Occurrence) -> Vec<Ymd> {
    use transom_core::clock::{civil_from_days, days_from_civil};
    match o.start {
        transom_core::ics::When::Day(d) => {
            let n = (o.duration / 86_400).clamp(1, 62);
            (0..n)
                .map(|i| civil_from_days(days_from_civil(d) + i))
                .collect()
        }
        transom_core::ics::When::At(a) => vec![pclock::local_day_of_unix(a)],
    }
}

/// Text riadku udalosti z kalendara: cas zaciatku (ak nie je celodenna) a nazov.
pub fn occurrence_text(o: &transom_core::ics::Occurrence) -> String {
    let name = if o.summary.is_empty() {
        t("cal.ics_untitled")
    } else {
        o.summary.clone()
    };
    match o.start {
        transom_core::ics::When::Day(_) => tf("cal.ics_allday", &[&name]),
        transom_core::ics::When::At(a) => format!("{} {name}", time_of(a)),
    }
}

/// Slovny popis opakovania rucneho datumu.
pub fn repeat_text(r: Repeat) -> String {
    match r {
        Repeat::Once => t("cal.repeat.once"),
        Repeat::EveryDays(n) => tf("cal.repeat.days", &[&n.to_string()]),
        Repeat::Monthly => t("cal.repeat.monthly"),
    }
}

/// Cas dna (miestny, podla formatu systemu) z Unix sekund.
fn time_of(ts: i64) -> String {
    let st = pclock::local_of_unix(ts);
    pclock::format_clock_at(&ClockOptions::default(), &st, &pclock::locale_time_format())
}

/// Text riadku udalosti: poskytovatel, co a odkial je datum (automaticky / rucne), pripadne zastarane.
pub fn event_text(e: &CalEvent) -> String {
    match e.source {
        Source::Automatic { stale } => {
            let mut src = t("cal.src.auto");
            if stale {
                src.push_str(&format!(", {}", t("ring.stale")));
            }
            tf(
                "cal.ev.auto",
                &[
                    e.provider.label(),
                    &window_name(e.window_minutes.unwrap_or(0)),
                    &e.at.map(time_of).unwrap_or_default(),
                    &src,
                ],
            )
        }
        Source::Manual(rep) => tf(
            "cal.ev.manual",
            &[
                e.provider.label(),
                &t(match e.kind {
                    EventKind::Reset => "cal.kind.reset",
                    EventKind::Billing => "cal.kind.billing",
                }),
                &t("cal.src.manual"),
                &repeat_text(rep),
            ],
        ),
    }
}

#[derive(Debug)]
pub struct CalState {
    pub y: i32,
    pub m: u32,
    pub selected: Option<Ymd>,
    pub hover: Option<CalHit>,
    pub hits: Vec<(Rect, CalHit)>,
}

impl CalState {
    pub fn new(today: Ymd) -> Self {
        CalState {
            y: today.y,
            m: today.m,
            selected: None,
            hover: None,
            hits: Vec::new(),
        }
    }

    /// Pri otvoreni: zobrazit aktualny mesiac, zvoleny den zabudnut.
    pub fn reset(&mut self, today: Ymd) {
        *self = CalState::new(today);
    }

    /// Spracuje klik; vrati `true`, ak sa zmenil zobrazeny obsah.
    pub fn click(&mut self, h: CalHit, today: Ymd) -> bool {
        match h {
            CalHit::Prev => self.shift(-1),
            CalHit::Next => self.shift(1),
            CalHit::Today => {
                self.y = today.y;
                self.m = today.m;
                self.selected = Some(today);
            }
            CalHit::Day(d) => {
                self.selected = Some(d);
                // klik na den susedneho mesiaca prepne zobrazenie
                if d.m != self.m || d.y != self.y {
                    self.y = d.y;
                    self.m = d.m;
                }
            }
        }
        true
    }

    pub fn shift(&mut self, delta: i32) {
        let (y, m) = add_months(self.y, self.m, delta);
        self.y = y;
        self.m = m;
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<CalHit> {
        hit(&self.hits, x, y)
    }

    /// Nakresli kalendar; `first` = prvy den tyzdna (0 = pondelok).
    pub fn paint(
        &mut self,
        p: &mut dyn Painter,
        theme: &Theme,
        first: u8,
        today: Ymd,
        ctx: &CalCtx,
    ) {
        let names = pclock::short_day_names();
        let order = header_order(first);
        let day_names: [&str; 7] = std::array::from_fn(|i| names[order[i] as usize].as_str());
        let cells = month_grid(self.y, self.m, first);
        let title = pclock::month_title(self.y, self.m);
        let footer = pclock::long_date(self.selected.unwrap_or(today));
        // udalosti viditelnej mriezky (aj dni susednych mesiacov); den -> znacky; riadky zvoleneho dna
        let (from, to) = (cells[0], cells[cells.len() - 1]);
        let evs = caldata::events(
            ctx.plans,
            ctx.providers,
            ctx.dates,
            from,
            to,
            ctx.now,
            ctx.max_age_s,
            &pclock::local_day_of_unix,
        );
        let day = self.selected.unwrap_or(today);
        let day_events: Vec<&CalEvent> = evs.iter().filter(|e| e.date == day).collect();
        // udalosti pripojenych kalendarov: dni so znackou a riadky zvoleneho dna (pred udalostami AI)
        let occ = ics_occurrences(ctx.ics, from, to);
        let mut ics_days: Vec<Ymd> = Vec::new();
        let mut all_lines: Vec<EventLine> = Vec::new();
        for o in &occ {
            let days = occurrence_days(o);
            if days.contains(&day) {
                all_lines.push(EventLine {
                    kind: MarkKind::Event,
                    text: occurrence_text(o),
                });
            }
            ics_days.extend(days);
        }
        all_lines.extend(day_events.iter().map(|e| EventLine {
            kind: match e.kind {
                EventKind::Reset => MarkKind::Reset,
                EventKind::Billing => MarkKind::Billing,
            },
            text: event_text(e),
        }));
        const MAX_LINES: usize = 4;
        let total = all_lines.len();
        let lines: Vec<EventLine> = all_lines.into_iter().take(MAX_LINES).collect();
        let more = (total > MAX_LINES).then(|| tf("cal.more", &[&(total - MAX_LINES).to_string()]));
        let marks = |d: Ymd| {
            let (a, b) = caldata::kinds_on(&evs, d);
            (a, b, ics_days.contains(&d))
        };
        let legend_event = ctx.has_ics.then(|| t("cal.kind.event"));
        let v = CalendarView {
            title: &title,
            day_names,
            cells: &cells,
            month: self.m,
            today,
            selected: self.selected,
            footer: &footer,
            today_label: &transom_i18n::t("cal.today"),
            hovered: self.hover,
            marks: &marks,
            events: &lines,
            more: more.as_deref(),
            legend: (&t("cal.kind.reset"), &t("cal.kind.billing")),
            legend_event: legend_event.as_deref(),
            no_events: &t("cal.none_day"),
        };
        self.hits = render_calendar(p, theme, &v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_and_selection() {
        let _lang = crate::test_lang::guard();
        let today = Ymd::new(2026, 11, 20);
        let mut c = CalState::new(today);
        c.click(CalHit::Next, today);
        assert_eq!((c.y, c.m), (2026, 12));
        c.click(CalHit::Next, today);
        assert_eq!((c.y, c.m), (2027, 1));
        c.click(CalHit::Today, today);
        assert_eq!((c.y, c.m, c.selected), (2026, 11, Some(today)));
        // den z predchadzajuceho mesiaca prepne mesiac
        c.click(CalHit::Day(Ymd::new(2026, 10, 30)), today);
        assert_eq!((c.y, c.m), (2026, 10));
        c.reset(today);
        assert_eq!((c.y, c.m, c.selected), (2026, 11, None));
    }

    // ---- offscreen kontrola popoveru kalendara so znackami udalosti ----
    use transom_render::audit::Audit;
    use transom_render::calendar::{CAL_H_LP, CAL_W_LP};
    use transom_render::offscreen::Surface;
    use transom_render::{GdiPainter, Size};
    use transom_usage::{LimitWindow, SnapshotSource};

    fn manual(p: &str, k: &str, d: &str, r: &str) -> transom_config::ManualDate {
        transom_config::ManualDate {
            provider: p.into(),
            kind: k.into(),
            date: d.into(),
            repeat: r.into(),
            every_days: 6,
        }
    }

    /// Den 26. 11. 2026 s peatimi udalostmi (automaticky reset + 4 rucne) -> 4 riadky + "+ 1 dalsie"; ostatne dni znacky.
    fn paint_fixture(p: &mut dyn Painter, th: &Theme, selected: Option<Ymd>) {
        let noon =
            |y, m, d| transom_core::clock::days_from_civil(Ymd::new(y, m, d)) * 86_400 + 12 * 3600;
        let plans = vec![transom_usage::PlanSnapshot {
            provider: Some(Provider::Codex),
            plan: "go".into(),
            windows: vec![LimitWindow {
                minutes: 300,
                used_percent: 10.0,
                resets_at: Some(noon(2026, 11, 26)),
            }],
            recorded_at: noon(2026, 11, 20),
            source: SnapshotSource::CodexSession,
        }];
        let dates = vec![
            manual("claude", "reset", "2026-11-20", "days"),
            manual("codex", "billing", "2026-10-26", "monthly"),
            manual("claude", "billing", "2026-11-26", "none"),
            manual("codex", "reset", "2026-11-26", "none"),
        ];
        let ctx = CalCtx {
            plans: &plans,
            providers: &[Provider::Codex],
            dates: &dates,
            now: noon(2026, 11, 20),
            max_age_s: 3600 * 24,
            ics: &[],
            has_ics: false,
        };
        let mut c = CalState::new(Ymd::new(2026, 11, 20));
        c.selected = selected;
        c.paint(p, th, 0, Ymd::new(2026, 11, 20), &ctx);
    }

    #[test]
    fn calendar_popover_audit_is_clean_in_all_languages_with_markers_and_event_lines() {
        let _lang = crate::test_lang::guard();
        let mut issues = Vec::new();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            for dpi in [96u32, 192] {
                let s = dpi as f32 / 96.0;
                let (w, h) = (
                    (CAL_W_LP as f32 * s).round() as i32,
                    (CAL_H_LP as f32 * s).round() as i32,
                );
                for dark in [true, false] {
                    let th = if dark { Theme::dark() } else { Theme::light() };
                    for sel in [Some(Ymd::new(2026, 11, 26)), None] {
                        let surf = Surface::new(w, h);
                        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                        let mut a = Audit::new(&mut gp, th.background);
                        paint_fixture(&mut a, &th, sel);
                        a.finish();
                        for i in &a.issues {
                            issues.push(format!(
                                "{} {dpi} dark={dark} sel={}: {i:?}",
                                lang.code(),
                                sel.is_some()
                            ));
                        }
                    }
                }
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert!(
            issues.is_empty(),
            "{}",
            issues.join(
                "
"
            )
        );
    }

    /// Galeria (pozri `crate::gallery`): kalendar s vybranym dnom pri kazdom DPI a teme.
    #[test]
    #[ignore]
    fn gallery_calendar() {
        let _lang = crate::test_lang::guard();
        let Some(dir) = crate::gallery::dir() else {
            return;
        };
        for dpi in crate::gallery::DPIS {
            for (dark, th) in crate::gallery::themes() {
                let (w, h) = (
                    crate::gallery::px(CAL_W_LP, dpi),
                    crate::gallery::px(CAL_H_LP, dpi),
                );
                let surf = Surface::new(w, h);
                let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                paint_fixture(&mut gp, &th, Some(Ymd::new(2026, 11, 26)));
                crate::gallery::save(
                    &dir,
                    &format!("calendar-{}-{dpi}", crate::gallery::tag(dark)),
                    &surf,
                );
            }
        }
    }

    /// Galeria: kalendar na skle (dva prechody nad nadychom skla, RGBA) pri 150 % v oboch temach.
    #[test]
    #[ignore]
    fn gallery_calendar_glass() {
        let _lang = crate::test_lang::guard();
        let Some(dir) = crate::gallery::dir() else {
            return;
        };
        let dpi = crate::gallery::hero_dpi();
        let (w, h) = (
            crate::gallery::px(CAL_W_LP, dpi),
            crate::gallery::px(CAL_H_LP, dpi),
        );
        for (dark, th) in crate::gallery::themes() {
            // panel: rohy 8 lp (ako `popup::paint_glass`)
            let (gw, rgba) = crate::gallery::glass_rgba_panel(w, h, dpi, &th, &mut |p, t| {
                paint_fixture(p, t, Some(Ymd::new(2026, 11, 26)))
            });
            crate::gallery::save_rgba(
                &dir,
                &format!("calendar-glass-{}-{dpi}", crate::gallery::tag(dark)),
                gw,
                h,
                &rgba,
            );
        }
    }

    /// Ulozi snimky do `TRANSOM_SHOT_DIR` (rucna kontrola; `cargo test -- --ignored dump_calendar`).
    #[test]
    #[ignore]
    fn dump_calendar_pngs() {
        let _lang = crate::test_lang::guard();
        let Some(dir) = std::env::var_os("TRANSOM_SHOT_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        for lang in [transom_i18n::Lang::En, transom_i18n::Lang::Sk] {
            transom_i18n::set_lang(lang);
            for dark in [true, false] {
                let th = if dark { Theme::dark() } else { Theme::light() };
                let dpi = crate::gallery::hero_dpi();
                let s = dpi as f32 / 96.0;
                let (w, h) = (
                    (CAL_W_LP as f32 * s).round() as i32,
                    (CAL_H_LP as f32 * s).round() as i32,
                );
                let surf = Surface::new(w, h);
                let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                paint_fixture(&mut gp, &th, Some(Ymd::new(2026, 11, 26)));
                let f = dir.join(format!(
                    "calendar-{}-{}.png",
                    lang.code(),
                    if dark { "dark" } else { "light" }
                ));
                let _ = std::fs::write(
                    f,
                    transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
                );
            }
        }
    }
}
