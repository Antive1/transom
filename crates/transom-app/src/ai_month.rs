//! Mesacny pohlad historie vyuzitia v paneli AI: graf dennych tokenov alebo **odhadu ceny** (API-ekvivalent, nie
//! faktura), navigacia po mesiacoch, filter poskytovatela, rozpis zvoleneho dna a poznamky k uplnosti.
//!
//! Model (stavy dni, filter, mesiac) je v `transom_usage::history` (cisty, testovany). Tu je stav pohladu ([`MonthUi`]) a
//! kreslenie. **Je to historia tokenov / ceny z lokalnych zaznamov - nie historicka spotreba kvoty planu** (tu je len
//! aktualny snimok v kartach nad nim); text pod grafom to hovori vzdy.
//!
//! Stavy dni sa rozlisuju **tvarom, nie farbou**: plny stlpec = zaznamenane, prugovany = castocne (spodny odhad),
//! bodkovana ciara = bez historie (nevieme, nie nula), prazdny obrys = cena neznama, kratka ciarka = skutocna nula.
use crate::ai::{money, provider_short, AiHit};
use crate::ai_cards::Dry;
use transom_core::clock::{add_months, days_in_month, Ymd};
use transom_i18n::{t, tf};
use transom_platform::clock as pclock;
use transom_render::painter::wrap_text;
use transom_render::{Color, Font, HAlign, Painter, Rect, TextStyle, Theme};
use transom_usage::history::{self, DayCell, DayState, Filter, Metric, MonthView, Note, Origin};
use transom_usage::{fmt_tokens, Collected, ModelTotals, Provider};

/// Stav pohladu (nie je trvaly - po zatvoreni panela sa vrati na tento mesiac, tokeny, vsetkych poskytovatelov).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonthUi {
    /// Zobrazeny mesiac; `None` = aktualny mesiac.
    pub ym: Option<(i32, u32)>,
    /// Zvoleny den v mesiaci (1-based).
    pub day: Option<u32>,
    pub metric: Metric,
    pub filter: Filter,
}

impl Default for MonthUi {
    fn default() -> Self {
        MonthUi {
            ym: None,
            day: None,
            metric: Metric::Tokens,
            filter: Filter::All,
        }
    }
}

impl MonthUi {
    /// Zobrazeny mesiac (rok, mesiac).
    pub fn month(&self, today: Ymd) -> (i32, u32) {
        self.ym.unwrap_or((today.y, today.m))
    }

    pub fn is_current(&self, today: Ymd) -> bool {
        self.month(today) == (today.y, today.m)
    }

    /// Najskorsi mesiac, na ktory sa da prejst (mesiac najstarsej znamej historie; bez historie len aktualny).
    fn first_month(earliest: Option<Ymd>, today: Ymd) -> (i32, u32) {
        earliest.map_or((today.y, today.m), |e| (e.y, e.m))
    }

    pub fn can_prev(&self, today: Ymd, earliest: Option<Ymd>) -> bool {
        self.month(today) > Self::first_month(earliest, today)
    }

    pub fn can_next(&self, today: Ymd) -> bool {
        self.month(today) < (today.y, today.m)
    }

    /// Posun o `delta` mesiacov v hraniciach historie a dneska; zmena mesiaca zrusi vyber dna. Vrati, ci sa nieco zmenilo.
    pub fn shift(&mut self, delta: i32, today: Ymd, earliest: Option<Ymd>) -> bool {
        let cur = self.month(today);
        let (y, m) = add_months(cur.0, cur.1, delta);
        let ok = if delta < 0 {
            (y, m) >= Self::first_month(earliest, today)
        } else {
            (y, m) <= (today.y, today.m)
        };
        if !ok || (y, m) == cur {
            return false;
        }
        self.ym = ((y, m) != (today.y, today.m)).then_some((y, m));
        self.day = None;
        true
    }

    /// Navrat na aktualny mesiac (a bez vybraneho dna).
    pub fn this_month(&mut self, today: Ymd) -> bool {
        let changed = !self.is_current(today) || self.day.is_some();
        self.ym = None;
        self.day = None;
        changed
    }

    /// Zvoli den (alebo ho zrusi opakovanym vyberom toho isteho).
    pub fn toggle_day(&mut self, d: u32) {
        self.day = if self.day == Some(d) { None } else { Some(d) };
    }

    /// Sipky: o `delta` dni dalej / spat v ramci mesiaca; bez vyberu zacne dneskom (aktualny mesiac) alebo poslednym dnom.
    pub fn step_day(&mut self, delta: i32, today: Ymd) {
        let (y, m) = self.month(today);
        let n = days_in_month(y, m) as i32;
        let last = if self.is_current(today) {
            today.d as i32
        } else {
            n
        };
        let cur = self
            .day
            .map_or(if delta > 0 { 0 } else { last + 1 }, |d| d as i32);
        self.day = Some((cur + delta).clamp(1, last.max(1)) as u32);
    }
}

/// Vstupy kreslenia.
pub struct MonthCtx<'a> {
    pub data: &'a Collected,
    pub providers: &'a [Provider],
    pub today: Ymd,
    pub local_day: &'a dyn Fn(i64) -> Ymd,
}

impl MonthCtx<'_> {
    pub fn view(&self, ui: &MonthUi) -> MonthView {
        let (y, m) = ui.month(self.today);
        history::month(
            self.data,
            self.providers,
            y,
            m,
            self.today,
            ui.filter,
            ui.metric,
            self.local_day,
        )
    }

    pub fn earliest(&self) -> Option<Ymd> {
        history::earliest(self.data, self.local_day)
    }
}

fn st(size: f32, bold: bool, color: Color, align: HAlign) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    }
}

fn value_text(v: f64, metric: Metric) -> String {
    match metric {
        Metric::Tokens => tf("ai.m.total_tokens", &[&fmt_tokens(v.round() as u64)]),
        Metric::Cost => format!("≈ ${}", money(v)),
    }
}

/// Kratka hodnota osi (bez jednotky).
fn short_value(v: f64, metric: Metric) -> String {
    match metric {
        Metric::Tokens => fmt_tokens(v.round() as u64),
        Metric::Cost => format!("≈ ${}", money(v)),
    }
}

fn tokens_of(r: &ModelTotals) -> u64 {
    r.input + r.output + r.cache_read + r.cache_write
}

/// Sucet mesiaca podla modelov (zo vsetkych dni mesiaca, ktore maju udaje) - rozpis, ked nie je zvoleny den.
fn month_rows(c: &Collected, y: i32, m: u32, filter: Filter) -> Vec<ModelTotals> {
    let mut out: Vec<ModelTotals> = Vec::new();
    for d in 1..=days_in_month(y, m) {
        for r in history::day_rows(c, Ymd::new(y, m, d), filter) {
            match out
                .iter_mut()
                .find(|x| x.provider == r.provider && x.model == r.model)
            {
                Some(x) => {
                    x.input += r.input;
                    x.output += r.output;
                    x.cache_read += r.cache_read;
                    x.cache_write += r.cache_write;
                    x.requests += r.requests;
                    x.est_usd += r.est_usd;
                    x.unpriced += r.unpriced;
                }
                None => out.push(r),
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(tokens_of(r)));
    out
}

/// Text poznamky o uplnosti (lokalizovany; datumy podla jazyka Windows).
pub fn note_text(n: Note) -> String {
    match n {
        Note::BeginsAt(p, d) => tf(
            "ai.m.note.begins",
            &[provider_short(p), &pclock::long_date(d)],
        ),
        Note::BeforeHistory => t("ai.m.note.before"),
        Note::Archived => t("ai.m.note.archived"),
        Note::Unsupported(p) => tf("ai.m.note.unsupported", &[provider_short(p)]),
        Note::ReadErrors(p) => tf("ai.m.note.errors", &[provider_short(p)]),
    }
}

/// Nazov dna pre pristupnost / tooltip: datum a hodnota alebo stav slovom.
pub fn day_name(c: &DayCell, metric: Metric) -> String {
    let date = pclock::long_date(c.date);
    let what = match c.state {
        DayState::Future => t("ai.m.day.future"),
        DayState::NoHistory => t("ai.m.leg.none"),
        DayState::Unpriced if metric == Metric::Cost => t("ai.m.leg.price"),
        DayState::Incomplete => format!(
            "{} ({})",
            value_text(c.value(metric).unwrap_or(0.0), metric),
            t("ai.m.leg.part")
        ),
        _ => value_text(c.value(metric).unwrap_or(0.0), metric),
    };
    // archivovane dni sa oznacia aj slovom (pristupnost; v grafe ich odlisuje pruh pod osou)
    if c.origin != Origin::Live && c.value(metric).is_some() {
        return format!("{date}: {what} ({})", t("ai.m.leg.arch"));
    }
    format!("{date}: {what}")
}

#[derive(Clone, Copy)]
struct Seg<'a> {
    label: &'a str,
    hit: AiHit,
    active: bool,
}

/// Segmentovany prepinac (aktivna volba = vyplnena + tucna, nie len farba). Vrati sirku.
#[allow(clippy::too_many_arguments)]
fn segmented(
    p: &mut dyn Painter,
    th: &Theme,
    items: &[Seg],
    x: i32,
    y: i32,
    h: i32,
    hover: Option<AiHit>,
    hits: &mut Vec<(Rect, AiHit)>,
) -> i32 {
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    let pad = px(10.0);
    let widths: Vec<i32> = items
        .iter()
        .map(|i| {
            (p.measure_text(i.label, &st(11.0, true, th.foreground, HAlign::Center))
                .w
                + 2 * pad)
                .max(px(40.0))
        })
        .collect();
    let total: i32 = widths.iter().sum::<i32>() + px(4.0);
    // segmentovy prepinac: drazka ovladaca, vybrany segment = nadych + akcentovy okraj + polotucne (nie len farba)
    transom_render::design::fill_with_stroke(
        p,
        Rect::new(x, y, total, h),
        px(6.0),
        Some(th.control),
        Some(th.control_edge),
    );
    let mut cx = x + px(2.0);
    for (i, it) in items.iter().enumerate() {
        let r = Rect::new(cx, y + px(2.0), widths[i], h - px(4.0));
        if it.active {
            transom_render::design::fill_with_stroke(
                p,
                r,
                px(4.0),
                Some(th.selected),
                Some(th.accent),
            );
        } else if hover == Some(it.hit) {
            p.fill_round_rect(r, px(4.0), th.control_hover);
        }
        p.draw_text(
            it.label,
            r,
            &st(11.0, it.active, th.foreground, HAlign::Center),
        );
        hits.push((r, it.hit));
        cx += widths[i];
    }
    total
}

/// Volba "Zostava / Spotrebovane" nad kartami (plati pre karty aj kruh v liste). Vrati vysku riadku.
pub fn mode_toggle(
    p: &mut dyn Painter,
    th: &Theme,
    mode: transom_usage::limits::RingMode,
    hover: Option<AiHit>,
    x: i32,
    y: i32,
    hits: &mut Vec<(Rect, AiHit)>,
) -> i32 {
    use transom_usage::limits::RingMode;
    let s = p.scale();
    let h = (26.0 * s).round() as i32;
    let (l, u) = (t("ai.mode.left"), t("ai.mode.used"));
    let items = [
        Seg {
            label: &l,
            hit: AiHit::Mode(RingMode::Remaining),
            active: mode == RingMode::Remaining,
        },
        Seg {
            label: &u,
            hit: AiHit::Mode(RingMode::Used),
            active: mode == RingMode::Used,
        },
    ];
    segmented(p, th, &items, x, y, h, hover, hits);
    h
}

/// Hlavicka sekcie so zalozkami obdobia (Dnes / 7 dni / Mesiac) - spolocna pre kartu historie aj mesacny pohlad.
#[allow(clippy::too_many_arguments)]
pub fn tabs(
    p: &mut dyn Painter,
    th: &Theme,
    active: transom_usage::Period,
    hover: Option<AiHit>,
    x: i32,
    y: i32,
    w: i32,
    hits: &mut Vec<(Rect, AiHit)>,
) {
    use transom_usage::Period;
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    p.draw_text(
        &t("ai.sec.history"),
        Rect::new(x, y, w / 3, px(24.0)),
        &st(12.0, true, th.muted, HAlign::Left),
    );
    let names = ["period.today", "period.last7", "period.month"];
    let tab_w = names
        .iter()
        .map(|k| {
            p.measure_text(&t(k), &st(11.0, true, th.foreground, HAlign::Center))
                .w
        })
        .max()
        .unwrap_or(0)
        + px(18.0);
    for (i, (per, key)) in [
        (Period::Today, "period.today"),
        (Period::Last7, "period.last7"),
        (Period::Month, "period.month"),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(x + w - (3 - i as i32) * tab_w, y, tab_w, px(24.0));
        let on = active == per;
        if on {
            transom_render::design::fill_with_stroke(
                p,
                r,
                px(4.0),
                Some(th.selected),
                Some(th.accent),
            );
        } else if hover == Some(AiHit::Tab(per)) {
            p.fill_round_rect(r, px(4.0), th.control_hover);
        }
        p.draw_text(&t(key), r, &st(11.0, on, th.foreground, HAlign::Center));
        hits.push((r, AiHit::Tab(per)));
    }
}

/// Znacka stavu dna v legende (rovnaky tvar ako v grafe).
fn swatch(p: &mut dyn Painter, th: &Theme, state: DayState, x: i32, y: i32) {
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    let (w, h) = (px(12.0), px(10.0));
    bar_shape(p, th, state, Rect::new(x, y, w, h), true);
}

/// Tvar stlpca podla stavu (`full` = plna vyska obdlzniku; inak ho kresli graf s vlastnou vyskou).
fn bar_shape(p: &mut dyn Painter, th: &Theme, state: DayState, r: Rect, _legend: bool) {
    let s = p.scale();
    let px1 = (1.0 * s).round().max(1.0) as i32;
    match state {
        DayState::Recorded => p.fill_rect(r, th.accent),
        DayState::Incomplete => {
            // prugovany stlpec = spodny odhad
            let (on, off) = ((2.0 * s).round().max(1.0) as i32, px1);
            let mut k = 0;
            while k < r.w {
                p.fill_rect(Rect::new(r.x + k, r.y, on.min(r.w - k), r.h), th.accent);
                k += on + off;
            }
        }
        DayState::NoHistory => {
            // bodkovana ciara na zaklade = nevieme (nie nula)
            let d = (2.0 * s).round().max(1.0) as i32;
            let mut k = 0;
            while k < r.w {
                p.fill_rect(
                    Rect::new(r.x + k, r.y + r.h - d, d.min(r.w - k), d),
                    th.muted,
                );
                k += 2 * d;
            }
        }
        DayState::Unpriced => {
            // prazdny obrys = cena je neznama
            p.stroke_round_rect(r, 1, px1, th.muted);
        }
        DayState::Future => {}
    }
}

/// Nakresli mesacny pohlad ako kartu; vrati vysku karty. `bg_h` = vyska pozadia (kreslenie), `None` = len meranie.
#[allow(clippy::too_many_arguments)]
fn draw(
    p: &mut dyn Painter,
    th: &Theme,
    ctx: &MonthCtx,
    ui: &MonthUi,
    view: &MonthView,
    hover: Option<AiHit>,
    x0: i32,
    y0: i32,
    w0: i32,
    bg_h: Option<i32>,
    hits: &mut Vec<(Rect, AiHit)>,
) -> i32 {
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    let inner = px(12.0);
    let (x, w) = (x0 + inner, w0 - 2 * inner);
    if let Some(h) = bg_h {
        transom_render::design::card(p, th, Rect::new(x0, y0, w0, h));
    }
    let mut y = y0 + inner;
    let line = px(18.0);

    // 1. nadpis + zalozky obdobia
    tabs(p, th, transom_usage::Period::Month, hover, x, y, w, hits);
    y += px(30.0);

    // 2. navigacia: predosly / mesiac / dalsi + "Tento mesiac"
    let (ym_y, ym_m) = ui.month(ctx.today);
    let bsz = px(26.0);
    let icon = |glyph: &str, r: Rect, on: bool, p: &mut dyn Painter| {
        p.draw_text(
            glyph,
            r,
            &TextStyle {
                font: Font::Icons,
                size_lp: 11.0,
                bold: false,
                color: if on { th.foreground } else { th.muted },
                align: HAlign::Center,
            },
        );
    };
    let prev = Rect::new(x, y, bsz, bsz);
    let can_prev = ui.can_prev(ctx.today, ctx.earliest());
    if can_prev && hover == Some(AiHit::MonthPrev) {
        p.fill_round_rect(prev, px(4.0), th.control_hover);
    }
    icon("\u{E76B}", prev, can_prev, p);
    if can_prev {
        hits.push((prev, AiHit::MonthPrev));
    }
    let title = pclock::month_title(ym_y, ym_m);
    let tstyle = st(13.0, true, th.foreground, HAlign::Left);
    let tw = p.measure_text(&title, &tstyle).w;
    p.draw_text(
        &title,
        Rect::new(x + bsz + px(6.0), y, tw + px(4.0), bsz),
        &tstyle,
    );
    let next = Rect::new(x + bsz + px(6.0) + tw + px(10.0), y, bsz, bsz);
    let can_next = ui.can_next(ctx.today);
    if can_next && hover == Some(AiHit::MonthNext) {
        p.fill_round_rect(next, px(4.0), th.control_hover);
    }
    icon("\u{E76C}", next, can_next, p);
    if can_next {
        hits.push((next, AiHit::MonthNext));
    }
    if !ui.is_current(ctx.today) {
        let label = t("ai.m.this");
        let lw = p
            .measure_text(&label, &st(11.0, true, th.foreground, HAlign::Center))
            .w
            + px(20.0);
        let r = Rect::new(x + w - lw, y, lw, bsz);
        transom_render::design::fill_with_stroke(
            p,
            r,
            px(4.0),
            Some(if hover == Some(AiHit::MonthThis) {
                th.control_hover
            } else {
                th.control
            }),
            Some(th.control_edge),
        );
        p.draw_text(&label, r, &st(11.0, true, th.foreground, HAlign::Center));
        hits.push((r, AiHit::MonthThis));
    }
    y += bsz + px(8.0);

    // 3. jednotka (tokeny / odhad ceny) a filter poskytovatela; ak sa nezmestia vedla seba, filter ide na dalsi riadok
    let seg_h = px(26.0);
    let unit_label = [t("ai.m.tokens"), t("ai.m.cost")];
    let unit = [
        Seg {
            label: &unit_label[0],
            hit: AiHit::Unit(Metric::Tokens),
            active: ui.metric == Metric::Tokens,
        },
        Seg {
            label: &unit_label[1],
            hit: AiHit::Unit(Metric::Cost),
            active: ui.metric == Metric::Cost,
        },
    ];
    let uw = segmented(p, th, &unit, x, y, seg_h, hover, hits);
    if ctx.providers.len() > 1 {
        let all = t("ai.m.all");
        let mut labels: Vec<(String, Filter)> = vec![(all, Filter::All)];
        labels.extend(
            ctx.providers
                .iter()
                .map(|&pr| (provider_short(pr).to_string(), Filter::Only(pr))),
        );
        let segs: Vec<Seg> = labels
            .iter()
            .map(|(l, f)| Seg {
                label: l,
                hit: AiHit::Prov(*f),
                active: ui.filter == *f,
            })
            .collect();
        // sirka bez kreslenia (Dry), potom umiestnenie vpravo alebo na novy riadok
        let fw = segmented(&mut Dry(p), th, &segs, 0, 0, seg_h, None, &mut Vec::new());
        if uw + px(10.0) + fw <= w {
            segmented(p, th, &segs, x + w - fw, y, seg_h, hover, hits);
        } else {
            y += seg_h + px(6.0);
            segmented(p, th, &segs, x, y, seg_h, hover, hits);
        }
    }
    y += seg_h + px(10.0);

    // 4. graf: denne stlpce; os s dnami; mierka vlavo hore
    let area_h = px(80.0);
    let axis_h = px(22.0);
    let n = view.cells.len() as i32;
    let metric = ui.metric;
    let col_x = |i: i32| x + (i as i64 * w as i64 / n as i64) as i32;
    let base = y + area_h;
    let gap = px(2.0).max(1);
    for (i, c) in view.cells.iter().enumerate() {
        let i = i as i32;
        let (cx0, cx1) = (col_x(i), col_x(i + 1));
        let col = Rect::new(cx0, y, cx1 - cx0, area_h + axis_h);
        let day = c.date.d;
        let selected = ui.day == Some(day);
        let hot = hover == Some(AiHit::Day(day));
        if selected || hot {
            p.fill_round_rect(col, px(4.0), th.hover);
        }
        if selected {
            p.stroke_round_rect(col, px(4.0), px(1.0).max(1), th.accent);
        }
        let bw = (cx1 - cx0 - gap).max(1);
        let bx = cx0 + gap / 2;
        match c.state {
            DayState::Future => {}
            DayState::NoHistory => bar_shape(
                p,
                th,
                DayState::NoHistory,
                Rect::new(bx, base - px(4.0), bw, px(4.0)),
                false,
            ),
            DayState::Unpriced if metric == Metric::Cost => bar_shape(
                p,
                th,
                DayState::Unpriced,
                Rect::new(bx, base - px(14.0), bw, px(14.0)),
                false,
            ),
            st_ => {
                let v = c.value(metric).unwrap_or(0.0);
                let frac = if view.max > 0.0 { v / view.max } else { 0.0 };
                let mut bh = (frac * (area_h - px(12.0)) as f64).round() as i32;
                if v > 0.0 {
                    bh = bh.max(px(3.0));
                } else {
                    bh = px(2.0); // skutocna nula: kratka ciarka (nie prazdno)
                }
                let r = Rect::new(bx, base - bh, bw, bh);
                if v > 0.0 {
                    bar_shape(p, th, st_, r, false);
                } else {
                    p.fill_rect(r, th.muted);
                }
            }
        }
        // pruh pod osou = sucet je z archivu (plny) alebo ciastocne z archivu (bodkovany); nie je to len farba
        if c.origin != Origin::Live && c.value(metric).is_some() {
            let sh = px(3.0).max(2);
            let sr = Rect::new(bx, base + px(2.0), bw, sh);
            if c.origin == Origin::Archived {
                p.fill_rect(sr, th.muted);
            } else {
                let d = px(2.0).max(1);
                let mut k = 0;
                while k < sr.w {
                    p.fill_rect(Rect::new(sr.x + k, sr.y, d.min(sr.w - k), sh), th.muted);
                    k += 2 * d;
                }
            }
        }
        if c.state != DayState::Future {
            hits.push((col, AiHit::Day(day)));
        }
    }
    // zaklad grafu
    p.fill_rect(Rect::new(x, base, w, px(1.0).max(1)), th.edge);
    // mierka (najvacsia hodnota) vlavo hore
    if view.max > 0.0 {
        p.draw_text(
            &short_value(view.max, metric),
            Rect::new(x, y - px(2.0), w / 3, px(14.0)),
            &st(10.0, false, th.muted, HAlign::Left),
        );
    }
    // popisky dni: 1, 5, 10, ..., posledny; zvoleny den vzdy; dnesok je pod tym oznaceny bodkou
    let ls = st(10.0, false, th.muted, HAlign::Center);
    for (i, c) in view.cells.iter().enumerate() {
        let d = c.date.d as i32;
        let n_days = n;
        let show =
            d == 1 || d % 5 == 0 && n_days - d >= 3 || d == n_days || ui.day == Some(c.date.d);
        if !show {
            continue;
        }
        let i = i as i32;
        let (cx0, cx1) = (col_x(i), col_x(i + 1));
        let lw = px(26.0);
        let cxm = (cx0 + cx1) / 2;
        let sel = ui.day == Some(c.date.d);
        p.draw_text(
            &d.to_string(),
            Rect::new(cxm - lw / 2, base + px(6.0), lw, px(15.0)),
            &if sel {
                st(10.0, true, th.foreground, HAlign::Center)
            } else {
                ls
            },
        );
    }
    if view.cells.iter().any(|c| c.date == ctx.today) {
        let i = (ctx.today.d - 1) as i32;
        let cxm = (col_x(i) + col_x(i + 1)) / 2;
        let d = px(4.0);
        p.fill_round_rect(
            Rect::new(cxm - d / 2, base + axis_h - d, d, d),
            d / 2,
            th.accent,
        );
    }
    y += area_h + axis_h + px(6.0);

    // 5. legenda (len stavy, ktore su v mesiaci)
    // `None` stav = polozka "archivovane" (pruh pod osou)
    let mut legend: Vec<(Option<DayState>, String)> = Vec::new();
    for (state, key) in [
        (DayState::Recorded, "ai.m.leg.rec"),
        (DayState::Incomplete, "ai.m.leg.part"),
        (DayState::NoHistory, "ai.m.leg.none"),
        (DayState::Unpriced, "ai.m.leg.price"),
    ] {
        let present = view
            .cells
            .iter()
            .any(|c| c.state == state && (state != DayState::Unpriced || metric == Metric::Cost));
        if present {
            legend.push((Some(state), t(key)));
        }
    }
    if view.archived_days > 0 {
        legend.push((None, t("ai.m.leg.arch")));
    }
    let lst = st(10.0, false, th.muted, HAlign::Left);
    let mut lx = x;
    let mut ly = y;
    for (state, label) in &legend {
        let lw = p.measure_text(label, &lst).w;
        let item_w = px(12.0) + px(5.0) + lw + px(12.0);
        if lx > x && lx + item_w > x + w {
            lx = x;
            ly += px(15.0);
        }
        match state {
            Some(s) => swatch(p, th, *s, lx, ly + px(2.0)),
            // pruh pod osou = z archivu
            None => p.fill_rect(
                Rect::new(lx, ly + px(5.0), px(12.0), px(3.0).max(2)),
                th.muted,
            ),
        }
        p.draw_text(
            label,
            Rect::new(lx + px(17.0), ly, lw + px(4.0), px(14.0)),
            &lst,
        );
        lx += item_w;
    }
    if !legend.is_empty() {
        y = ly + px(20.0);
    }

    // 6. sucet mesiaca (zo znamych dni) + popis, co to je
    let total = match metric {
        Metric::Tokens => view.total_tokens as f64,
        Metric::Cost => view.total_cost,
    };
    // mesiac bez jedineho znameho dna: suma je neznama ("—"), nikdy nie nula
    let big = if view.known_days == 0 {
        "—".to_string()
    } else {
        value_text(total, metric)
    };
    let big_st = st(20.0, true, th.foreground, HAlign::Left);
    let bw = p.measure_text(&big, &big_st).w;
    p.draw_text(&big, Rect::new(x, y, bw + px(4.0), px(28.0)), &big_st);
    if metric == Metric::Cost && view.total_unpriced > 0 {
        p.draw_text(
            &tf("ai.unpriced", &[&view.total_unpriced.to_string()]),
            Rect::new(
                x + bw + px(10.0),
                y + px(6.0),
                (w - bw - px(10.0)).max(0),
                px(20.0),
            ),
            &st(11.0, false, th.muted, HAlign::Left),
        );
    }
    y += px(28.0);
    let cap = t(match metric {
        Metric::Tokens => "ai.m.cap_tokens",
        Metric::Cost => "ai.m.cap_cost",
    });
    let cst = st(11.0, false, th.muted, HAlign::Left);
    for l in wrap_text(p, &cap, &cst, w) {
        p.draw_text(&l, Rect::new(x, y, w, px(16.0)), &cst);
        y += px(16.0);
    }
    y += px(8.0);

    // 7. rozpis zvoleneho dna (alebo mesiaca): datum / mesiac, hodnota alebo stav slovom, modely
    let sel_cell = ui
        .day
        .and_then(|d| view.cells.get(d as usize - 1))
        .filter(|c| c.state != DayState::Future);
    let (head, rows): (String, Vec<ModelTotals>) = match sel_cell {
        Some(c) => (
            pclock::long_date(c.date),
            history::day_rows(ctx.data, c.date, ui.filter),
        ),
        None => (title.clone(), month_rows(ctx.data, ym_y, ym_m, ui.filter)),
    };
    let hst = st(12.0, true, th.foreground, HAlign::Left);
    p.draw_text(&head, Rect::new(x, y, w, line), &hst);
    if let Some(c) = sel_cell {
        if let Some(v) = c.value(metric) {
            let tx = value_text(v, metric);
            p.draw_text(
                &tx,
                Rect::new(x, y, w, line),
                &st(12.0, true, th.foreground, HAlign::Right),
            );
        }
    }
    y += px(20.0);
    // sprava o stave vybraneho dna
    let msg: Option<String> = sel_cell.and_then(|c| {
        let mut parts: Vec<String> = Vec::new();
        match c.state {
            DayState::NoHistory => parts.push(t("ai.m.day.none")),
            DayState::Incomplete => parts.push(t("ai.m.day.part")),
            DayState::Unpriced if metric == Metric::Cost => {
                parts.push(tf("ai.m.day.price", &[&c.unpriced.to_string()]))
            }
            DayState::Recorded if c.requests == 0 => parts.push(t("ai.m.day.zero")),
            _ => {}
        }
        // povod sumy: z archivu (zdroj dna uz necita) + datum cennika, z ktoreho je odhad ceny
        if c.value(metric).is_some() {
            match (c.origin, c.price_dates.is_empty()) {
                (Origin::Live, _) => {}
                (Origin::Mixed, _) => parts.push(t("ai.m.day.mixed")),
                (Origin::Archived, true) => parts.push(t("ai.m.day.archived")),
                (Origin::Archived, false) => {
                    parts.push(tf("ai.m.day.archived_price", &[&c.price_dates.join(", ")]))
                }
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    });
    let mst = st(11.0, false, th.muted, HAlign::Left);
    if let Some(m) = &msg {
        for l in wrap_text(p, m, &mst, w) {
            p.draw_text(&l, Rect::new(x, y, w, px(16.0)), &mst);
            y += px(16.0);
        }
    }
    let shown = rows.len().min(3);
    let right_w = px(150.0);
    for r in rows.iter().take(shown) {
        let model = r.model.strip_prefix("claude-").unwrap_or(&r.model);
        let name = format!(
            "{} · {}",
            provider_short(r.provider.unwrap_or(Provider::ClaudeCode)),
            model
        );
        p.draw_text(
            &name,
            Rect::new(x, y, w - right_w - px(6.0), px(20.0)),
            &st(11.0, false, th.foreground, HAlign::Left),
        );
        let cost = if r.est_usd > 0.0 || r.unpriced == 0 {
            format!("≈ ${}", money(r.est_usd))
        } else {
            t("ai.cost_unknown")
        };
        p.draw_text(
            &format!("{} · {}", fmt_tokens(tokens_of(r)), cost),
            Rect::new(x + w - right_w, y, right_w, px(20.0)),
            &st(11.0, false, th.foreground, HAlign::Right),
        );
        y += px(20.0);
    }
    if rows.len() > shown {
        p.draw_text(
            &tf("ai.more", &[&(rows.len() - shown).to_string()]),
            Rect::new(x, y, w, px(18.0)),
            &mst,
        );
        y += px(18.0);
    }
    if rows.is_empty() && msg.is_none() {
        p.draw_text(&t("ai.no_period"), Rect::new(x, y, w, px(18.0)), &mst);
        y += px(18.0);
    }

    // 8. poznamky k uplnosti historie
    if !view.notes.is_empty() {
        y += px(6.0);
    }
    for n in &view.notes {
        let text = note_text(*n);
        for l in wrap_text(p, &text, &mst, w) {
            p.draw_text(&l, Rect::new(x, y, w, px(16.0)), &mst);
            y += px(16.0);
        }
    }
    y + inner - y0
}

/// Nakresli mesacny pohlad (karta na celu sirku `w` od `x`); vrati y pod nou. Vyska sa zmeria "nasucho".
#[allow(clippy::too_many_arguments)]
pub fn paint(
    p: &mut dyn Painter,
    th: &Theme,
    ctx: &MonthCtx,
    ui: &MonthUi,
    view: &MonthView,
    hover: Option<AiHit>,
    x: i32,
    y: i32,
    w: i32,
    hits: &mut Vec<(Rect, AiHit)>,
) -> i32 {
    let h = draw(
        &mut Dry(p),
        th,
        ctx,
        ui,
        view,
        hover,
        x,
        y,
        w,
        None,
        &mut Vec::new(),
    );
    draw(p, th, ctx, ui, view, hover, x, y, w, Some(h), hits);
    y + h + (8.0 * p.scale()).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, dd: u32) -> Ymd {
        Ymd::new(y, m, dd)
    }
    const T: fn() -> Ymd = || Ymd::new(2026, 11, 20);

    #[test]
    fn navigation_stays_between_the_first_history_month_and_this_month() {
        let mut ui = MonthUi::default();
        let earliest = Some(d(2026, 9, 14));
        assert!(ui.is_current(T()));
        assert!(!ui.can_next(T()), "dalsi mesiac po dnesku nie je");
        assert!(ui.can_prev(T(), earliest));
        assert!(ui.shift(-1, T(), earliest));
        assert_eq!(ui.month(T()), (2026, 10));
        assert!(ui.shift(-1, T(), earliest));
        assert_eq!(ui.month(T()), (2026, 9));
        assert!(
            !ui.can_prev(T(), earliest),
            "pred septembrom nie je ziadna historia"
        );
        assert!(!ui.shift(-1, T(), earliest));
        assert!(ui.shift(1, T(), earliest));
        assert!(ui.shift(1, T(), earliest));
        assert!(
            ui.is_current(T()) && ui.ym.is_none(),
            "aktualny mesiac sa ulozi ako None"
        );
        assert!(!ui.shift(1, T(), earliest));
        // bez historie sa neda ist spat
        let mut ui = MonthUi::default();
        assert!(!ui.can_prev(T(), None));
        assert!(!ui.shift(-1, T(), None));
    }

    #[test]
    fn navigation_crosses_year_boundaries() {
        let t = d(2027, 1, 10);
        let mut ui = MonthUi::default();
        let e = Some(d(2026, 11, 3));
        assert!(ui.shift(-1, t, e));
        assert_eq!(ui.month(t), (2026, 12));
        assert!(ui.shift(-1, t, e));
        assert_eq!(ui.month(t), (2026, 11));
        assert!(ui.shift(1, t, e) && ui.shift(1, t, e));
        assert_eq!(ui.month(t), (2027, 1));
    }

    #[test]
    fn this_month_resets_month_and_selected_day() {
        let mut ui = MonthUi::default();
        ui.shift(-1, T(), Some(d(2026, 1, 1)));
        ui.day = Some(5);
        assert!(ui.this_month(T()));
        assert!(ui.is_current(T()) && ui.day.is_none());
        assert!(
            !ui.this_month(T()),
            "uz je tento mesiac a bez dna: nic sa nezmenilo"
        );
    }

    #[test]
    fn changing_month_clears_the_selected_day_and_toggle_selects_or_clears() {
        let mut ui = MonthUi::default();
        ui.toggle_day(7);
        assert_eq!(ui.day, Some(7));
        ui.toggle_day(7);
        assert_eq!(ui.day, None);
        ui.toggle_day(9);
        ui.shift(-1, T(), Some(d(2026, 1, 1)));
        assert_eq!(ui.day, None);
    }

    #[test]
    fn arrow_keys_step_days_within_the_month_and_never_into_the_future() {
        let mut ui = MonthUi::default();
        ui.step_day(-1, T()); // bez vyberu: od dneska (20) spat
        assert_eq!(ui.day, Some(20));
        ui.step_day(1, T());
        assert_eq!(ui.day, Some(20), "nie za dnesok");
        for _ in 0..40 {
            ui.step_day(-1, T());
        }
        assert_eq!(ui.day, Some(1));
        // minuly mesiac: do posledneho dna
        let mut ui = MonthUi::default();
        ui.shift(-1, T(), Some(d(2026, 1, 1)));
        ui.step_day(-1, T());
        assert_eq!(ui.day, Some(31), "oktober ma 31 dni");
        ui.step_day(1, T());
        assert_eq!(ui.day, Some(31));
    }

    #[test]
    fn unit_and_filter_persist_across_month_changes() {
        let mut ui = MonthUi {
            metric: Metric::Cost,
            filter: Filter::Only(Provider::Codex),
            ..Default::default()
        };
        ui.shift(-1, T(), Some(d(2026, 1, 1)));
        assert_eq!(ui.metric, Metric::Cost);
        assert_eq!(ui.filter, Filter::Only(Provider::Codex));
    }
}
