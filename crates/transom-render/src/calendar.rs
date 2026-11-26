//! Kalendar v popoveri pod hodinami: hlavicka s mesiacom a sipkami, hlavicka dni (poradie podla prveho dna
//! tyzdna), mriezka 6 x 7, dnesok (vyplneny akcentom), zvoleny den (ramcek), dni mimo mesiaca stlmene a pata
//! s tlacidlom "Dnes" a dlhym datumom zvoleneho dna. Nazvy a formaty dodava aplikacia z jazykoveho nastavenia.
use crate::painter::{Color, Font, HAlign, Painter, Rect, TextStyle};
use crate::theme::Theme;
use transom_core::clock::Ymd;

/// Rozmer popoveru kalendara v lp.
pub const CAL_W_LP: i32 = 300;
pub const CAL_H_LP: i32 = 480;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalHit {
    Prev,
    Next,
    Today,
    Day(Ymd),
}

/// Druh znacky udalosti: **tvar** rozlisuje druh (nie farba): reset = kruh, fakturacia = kosostvorec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Reset,
    Billing,
    /// Udalost z pripojeneho kalendara (ICS): stvorec.
    Event,
}

/// Riadok udalosti pod mriezkou (znacka + text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventLine {
    pub kind: MarkKind,
    pub text: String,
}

/// Maska pokrytia znacky `n x n` (0..=255), hladke okraje (4 x 4 podvzorky): kruh alebo kosostvorec.
pub fn mark_coverage(kind: MarkKind, n: i32) -> Vec<u8> {
    let n = n.max(2);
    let c = n as f32 / 2.0;
    let mut out = vec![0u8; (n * n) as usize];
    for y in 0..n {
        for x in 0..n {
            let mut hit = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0 - c;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0 - c;
                    let inside = match kind {
                        MarkKind::Reset => px * px + py * py <= c * c,
                        MarkKind::Billing => px.abs() + py.abs() <= c,
                        MarkKind::Event => px.abs() <= c * 0.8 && py.abs() <= c * 0.8,
                    };
                    hit += i32::from(inside);
                }
            }
            out[(y * n + x) as usize] = (hit * 255 / 16) as u8;
        }
    }
    out
}

pub struct CalendarView<'a> {
    /// Napr. "november 2026".
    pub title: &'a str,
    /// Nazvy dni v poradi zobrazenia (od prveho dna tyzdna).
    pub day_names: [&'a str; 7],
    /// 42 dni mriezky (`transom_core::clock::month_grid`).
    pub cells: &'a [Ymd],
    /// Zobrazeny mesiac (dni ineho mesiaca su stlmene).
    pub month: u32,
    pub today: Ymd,
    pub selected: Option<Ymd>,
    /// Dlhy datum zvoleneho (alebo dnesneho) dna.
    pub footer: &'a str,
    /// Text tlacidla "dnes" (preklad dodava aplikacia).
    pub today_label: &'a str,
    pub hovered: Option<CalHit>,
    /// Ktore znacky ma den (reset, fakturacia).
    pub marks: &'a dyn Fn(Ymd) -> (bool, bool, bool),
    /// Udalosti zvoleneho (alebo dnesneho) dna pod mriezkou; `more` = "+ N dalsich" ak sa nezmestili.
    pub events: &'a [EventLine],
    pub more: Option<&'a str>,
    /// Vysvetlivky tvarov (preklad dodava aplikacia): (reset, fakturacia).
    pub legend: (&'a str, &'a str),
    /// Vysvetlivka udalosti z kalendara (ICS), len ak je nejaky pripojeny.
    pub legend_event: Option<&'a str>,
    /// Text, ak zvoleny den nema ziadnu udalost.
    pub no_events: &'a str,
}

fn px(v: i32, s: f32) -> i32 {
    (v as f32 * s).round() as i32
}

fn style(font: Font, size: f32, bold: bool, color: Color, align: HAlign) -> TextStyle {
    TextStyle {
        font,
        size_lp: size,
        bold,
        color,
        align,
    }
}

/// Nakresli kalendar na cely povrch a vrati klikatelne oblasti.
pub fn render_calendar(p: &mut dyn Painter, t: &Theme, v: &CalendarView) -> Vec<(Rect, CalHit)> {
    let s = p.scale();
    let size = p.size();
    let mut hits = Vec::new();
    // pozadie s jemnym okrajom (vlas `edge`, ako ostatne popovery)
    p.fill_rect(Rect::new(0, 0, size.w, size.h), t.edge);
    let b = px(1, s).max(1);
    p.fill_rect(
        Rect::new(b, b, size.w - 2 * b, size.h - 2 * b),
        t.background,
    );

    let pad = px(12, s);
    let head_h = px(32, s);
    let btn = px(30, s);
    let hover_bg = |h: CalHit| v.hovered == Some(h);

    // hlavicka: mesiac + sipky
    p.draw_text(
        v.title,
        Rect::new(pad, pad, size.w - 2 * pad - 2 * btn, head_h),
        &style(Font::Ui, 15.0, true, t.foreground, HAlign::Left),
    );
    for (i, (h, glyph)) in [(CalHit::Prev, '\u{E76B}'), (CalHit::Next, '\u{E76C}')]
        .iter()
        .enumerate()
    {
        let r = Rect::new(size.w - pad - (2 - i as i32) * btn, pad, btn, head_h);
        if hover_bg(*h) {
            p.fill_round_rect(r, px(crate::design::radius::CONTROL, s), t.control_hover);
        }
        p.draw_text(
            &glyph.to_string(),
            r,
            &style(Font::Icons, 12.0, false, t.foreground, HAlign::Center),
        );
        hits.push((r, *h));
    }

    // hlavicka dni
    let grid_x = pad;
    let cell_w = (size.w - 2 * pad) / 7;
    let row_y = pad + head_h + px(4, s);
    let day_h = px(24, s);
    for (i, name) in v.day_names.iter().enumerate() {
        p.draw_text(
            name,
            Rect::new(grid_x + i as i32 * cell_w, row_y, cell_w, day_h),
            &style(Font::Ui, 12.0, false, t.muted, HAlign::Center),
        );
    }

    // mriezka
    let cell_h = px(34, s);
    let grid_y = row_y + day_h;
    for (i, d) in v.cells.iter().take(42).enumerate() {
        let r = Rect::new(
            grid_x + (i % 7) as i32 * cell_w,
            grid_y + (i / 7) as i32 * cell_h,
            cell_w,
            cell_h,
        );
        let inset = px(2, s);
        let inner = Rect::new(r.x + inset, r.y + inset, r.w - 2 * inset, r.h - 2 * inset);
        let is_today = *d == v.today;
        let is_sel = v.selected == Some(*d);
        let hit = CalHit::Day(*d);
        // dnesok = plna zaoblena znacka, zvoleny den = zaobleny akcentovy obrys (+ polotucne cislo), pod mysou plocha
        let rad = inner.h.min(inner.w) / 2;
        if is_today {
            p.fill_round_rect(inner, rad, t.accent);
        } else if hover_bg(hit) {
            p.fill_round_rect(inner, rad, t.hover);
        }
        if is_sel && !is_today {
            let bw = px(2, s).max(1);
            p.stroke_round_rect(inner, rad, bw, t.accent);
        }
        let color = if is_today {
            t.on_accent
        } else if d.m != v.month {
            // dni susednych mesiacov: stlmene, ale stale s kontrastom aspon 4,5 : 1 (nie farba "zakazane")
            t.muted
        } else {
            t.foreground
        };
        p.draw_text(
            &d.d.to_string(),
            inner,
            &style(Font::Ui, 13.0, is_today || is_sel, color, HAlign::Center),
        );
        // znacky udalosti pod cislom dna: kruh = reset, kosostvorec = fakturacia (rozlisitelne aj bez farby)
        let (has_reset, has_billing, has_event) = (v.marks)(*d);
        if has_reset || has_billing || has_event {
            let n = px(7, s);
            let gap = px(3, s);
            let count = i32::from(has_reset) + i32::from(has_billing) + i32::from(has_event);
            let total = count * n + (count - 1) * gap;
            let mut mx = inner.x + (inner.w - total) / 2;
            let my = inner.y + inner.h - n - px(1, s);
            let col = if is_today {
                t.on_accent
            } else if d.m != v.month {
                t.muted
            } else {
                t.accent_text
            };
            for (on, kind) in [
                (has_reset, MarkKind::Reset),
                (has_billing, MarkKind::Billing),
                (has_event, MarkKind::Event),
            ] {
                if on {
                    p.blend_mask(Rect::new(mx, my, n, n), &mark_coverage(kind, n), col);
                    mx += n + gap;
                }
            }
        }
        hits.push((r, hit));
    }

    // pata: "Dnes" + datum
    let foot_y = grid_y + 6 * cell_h + px(4, s);
    let foot_h = px(28, s);
    // sirka tlacidla "Dnes" podla prekladu (napr. francuzske "Aujourd'hui" sa nesmie orezat)
    let today_w =
        (p.measure_text(
            v.today_label,
            &style(Font::Ui, 13.0, false, t.accent_text, HAlign::Center),
        )
        .w + px(16, s))
        .max(px(56, s));
    let tr = Rect::new(pad, foot_y, today_w, foot_h);
    if hover_bg(CalHit::Today) {
        p.fill_rect(tr, t.hover);
    }
    p.draw_text(
        v.today_label,
        tr,
        &style(Font::Ui, 13.0, false, t.accent_text, HAlign::Center),
    );
    hits.push((tr, CalHit::Today));
    p.draw_text(
        v.footer,
        Rect::new(
            pad + today_w + px(8, s),
            foot_y,
            size.w - 2 * pad - today_w - px(8, s),
            foot_h,
        ),
        &style(Font::Ui, 12.0, false, t.muted, HAlign::Right),
    );
    // udalosti zvoleneho dna + vysvetlivky tvarov
    let ev_y0 = foot_y + foot_h + px(4, s);
    p.fill_rect(
        Rect::new(pad, ev_y0 - px(2, s), size.w - 2 * pad, 1),
        t.hover,
    );
    let line_h = px(17, s);
    let mark_n = px(8, s);
    let text_x = pad + mark_n + px(6, s);
    let text_w = size.w - pad - text_x;
    let mut y = ev_y0 + px(2, s);
    let est = style(Font::Ui, 11.0, false, t.foreground, HAlign::Left);
    if v.events.is_empty() {
        p.draw_text(
            v.no_events,
            Rect::new(pad, y, size.w - 2 * pad, line_h),
            &style(Font::Ui, 11.0, false, t.muted, HAlign::Left),
        );
        y += line_h;
    }
    for e in v.events {
        let lines = crate::painter::wrap_text(p, &e.text, &est, text_w);
        p.blend_mask(
            Rect::new(pad, y + (line_h - mark_n) / 2, mark_n, mark_n),
            &mark_coverage(e.kind, mark_n),
            t.accent_text,
        );
        for l in lines {
            p.draw_text(&l, Rect::new(text_x, y, text_w, line_h), &est);
            y += line_h;
        }
    }
    if let Some(m) = v.more {
        p.draw_text(
            m,
            Rect::new(pad, y, size.w - 2 * pad, line_h),
            &style(Font::Ui, 11.0, false, t.muted, HAlign::Left),
        );
    }
    // vysvetlivky na spodnom okraji
    let leg_y = size.h - pad - line_h;
    let mut lx = pad;
    let mut legend = vec![
        (MarkKind::Reset, v.legend.0),
        (MarkKind::Billing, v.legend.1),
    ];
    if let Some(e) = v.legend_event {
        legend.push((MarkKind::Event, e));
    }
    for (kind, label) in legend {
        p.blend_mask(
            Rect::new(lx, leg_y + (line_h - mark_n) / 2, mark_n, mark_n),
            &mark_coverage(kind, mark_n),
            t.muted,
        );
        let st = style(Font::Ui, 11.0, false, t.muted, HAlign::Left);
        let w = p.measure_text(label, &st).w;
        p.draw_text(
            label,
            Rect::new(lx + mark_n + px(5, s), leg_y, w + px(4, s), line_h),
            &st,
        );
        lx += mark_n + px(5, s) + w + px(16, s);
    }
    hits
}

pub fn hit(hits: &[(Rect, CalHit)], x: i32, y: i32) -> Option<CalHit> {
    hits.iter()
        .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
        .map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::painter::Size;
    use transom_core::clock::month_grid;

    #[derive(Default)]
    struct Rec {
        fills: Vec<(Rect, Color)>,
        texts: Vec<(String, Rect, TextStyle)>,
    }
    impl Painter for Rec {
        fn size(&self) -> Size {
            Size { w: 300, h: 330 }
        }
        fn scale(&self) -> f32 {
            1.0
        }
        fn fill_rect(&mut self, r: Rect, c: Color) {
            self.fills.push((r, c));
        }
        fn fill_round_rect(&mut self, r: Rect, _: i32, c: Color) {
            self.fills.push((r, c));
        }
        fn measure_text(&mut self, t: &str, _: &TextStyle) -> Size {
            Size {
                w: 7 * t.len() as i32,
                h: 16,
            }
        }
        fn draw_text(&mut self, t: &str, r: Rect, s: &TextStyle) {
            self.texts.push((t.into(), r, *s));
        }
        fn blit(&mut self, _: Rect, _: &[Color]) {}
    }

    fn view<'a>(cells: &'a [Ymd], sel: Option<Ymd>) -> CalendarView<'a> {
        CalendarView {
            title: "november 2026",
            day_names: ["po", "ut", "st", "št", "pi", "so", "ne"],
            cells,
            month: 11,
            today: Ymd::new(2026, 11, 20),
            selected: sel,
            footer: "piatok 20. novembra 2026",
            today_label: "Dnes",
            hovered: None,
            marks: &|_| (false, false, false),
            events: &[],
            more: None,
            legend: ("reset", "fakturacia"),
            legend_event: None,
            no_events: "ziadne udalosti",
        }
    }

    #[test]
    fn today_selected_and_outside_days_are_marked() {
        let cells = month_grid(2026, 11, 0);
        let t = Theme::dark();
        let mut p = Rec::default();
        let hits = render_calendar(&mut p, &t, &view(&cells, Some(Ymd::new(2026, 11, 3))));
        // dnesok: vyplneny akcentom a text v on_accent
        assert!(p.fills.iter().any(|(_, c)| *c == t.accent));
        let today = p
            .texts
            .iter()
            .find(|(s, _, st)| s == "20" && st.color == t.on_accent);
        assert!(today.is_some());
        // 26. oktober (mimo mesiaca) stlmeny
        assert!(p
            .texts
            .iter()
            .any(|(s, _, st)| s == "26" && st.color == t.muted));
        // zvoleny den ma ramcek (4 obdlzniky akcentu) a tucne pismo
        assert!(p
            .texts
            .iter()
            .any(|(s, _, st)| s == "3" && st.bold && st.color == t.foreground));
        // 42 dni + 2 sipky + Dnes
        assert_eq!(hits.len(), 45);
        // hlavicka dni v zadanom poradi
        let names: Vec<_> = p
            .texts
            .iter()
            .filter(|(_, _, st)| st.color == t.muted && st.size_lp == 12.0)
            .map(|(s, _, _)| s.clone())
            .take(7)
            .collect();
        assert_eq!(names, vec!["po", "ut", "st", "št", "pi", "so", "ne"]);
    }

    #[test]
    fn hits_map_to_days_and_buttons() {
        let cells = month_grid(2026, 11, 6);
        let mut p = Rec::default();
        let hits = render_calendar(&mut p, &Theme::light(), &view(&cells, None));
        let (r, _) = hits
            .iter()
            .find(|(_, h)| *h == CalHit::Day(Ymd::new(2026, 11, 1)))
            .unwrap();
        assert_eq!(
            hit(&hits, r.x + 1, r.y + 1),
            Some(CalHit::Day(Ymd::new(2026, 11, 1)))
        );
        // pri nedeli ako prvom dni je 1. 11. v lavom hornom rohu mriezky
        let first = hits
            .iter()
            .find(|(_, h)| matches!(h, CalHit::Day(_)))
            .unwrap();
        assert_eq!(first.1, CalHit::Day(Ymd::new(2026, 11, 1)));
        assert!(hits.iter().any(|(_, h)| *h == CalHit::Prev));
        assert!(hits.iter().any(|(_, h)| *h == CalHit::Today));
        assert_eq!(hit(&hits, -5, -5), None);
    }

    /// Painter, ktory zaznamenava aj vyplne masiek (`blend_rgba`): znacky udalosti.
    #[derive(Default)]
    struct RecBlend {
        blends: Vec<(Rect, Vec<u8>)>,
    }
    impl Painter for RecBlend {
        fn size(&self) -> Size {
            Size { w: 300, h: 440 }
        }
        fn scale(&self) -> f32 {
            1.0
        }
        fn fill_rect(&mut self, _: Rect, _: Color) {}
        fn measure_text(&mut self, t: &str, _: &TextStyle) -> Size {
            Size {
                w: 7 * t.chars().count() as i32,
                h: 16,
            }
        }
        fn draw_text(&mut self, _: &str, _: Rect, _: &TextStyle) {}
        fn blit(&mut self, _: Rect, _: &[Color]) {}
        fn blend_rgba(&mut self, r: Rect, px: &[crate::brand::Rgba]) {
            self.blends.push((r, px.iter().map(|c| c.a).collect()));
        }
    }

    #[test]
    fn marker_shapes_differ_by_kind_and_are_visible_without_color() {
        for n in [6, 7, 8, 12] {
            let (c, d) = (
                mark_coverage(MarkKind::Reset, n),
                mark_coverage(MarkKind::Billing, n),
            );
            assert_ne!(c, d, "kruh a kosostvorec maju iny tvar pri {n} px");
            let sum = |v: &[u8]| v.iter().map(|x| *x as u32).sum::<u32>();
            // kruh zabera ~ pi/4 plochy, kosostvorec 1/2: kruh je vzdy plnsi
            assert!(sum(&c) > sum(&d), "{n}");
            assert!(sum(&d) > 0);
            // znacky su symetricke (vodorovne aj zvisle)
            for m in [&c, &d] {
                for y in 0..n as usize {
                    for x in 0..n as usize / 2 {
                        assert_eq!(
                            m[y * n as usize + x],
                            m[y * n as usize + (n as usize - 1 - x)],
                            "{n}"
                        );
                    }
                }
            }
        }
        // stred kosostvorca je plny, roh prazdny; kruh ma prazdny roh tiez, ale plnsi ("stvorcovitejsi") okraj
        let d = mark_coverage(MarkKind::Billing, 8);
        assert_eq!(d[0], 0);
        assert!(d[3 * 8 + 3] > 200);
    }

    #[test]
    fn days_with_events_get_the_right_marks_and_the_legend_names_both_shapes() {
        let cells = month_grid(2026, 11, 0);
        let marks = |d: Ymd| match d.d {
            3 if d.m == 11 => (true, false, false),
            4 if d.m == 11 => (false, true, false),
            5 if d.m == 11 => (true, true, false),
            _ => (false, false, false),
        };
        let mut v = view(&cells, None);
        v.marks = &marks;
        let lines = [EventLine {
            kind: MarkKind::Reset,
            text: "Claude Code: reset".into(),
        }];
        v.events = &lines;
        let mut p = RecBlend::default();
        render_calendar(&mut p, &Theme::dark(), &v);
        let n = 7;
        let reset = mark_coverage(MarkKind::Reset, n);
        let bill = mark_coverage(MarkKind::Billing, n);
        let in_cells = |m: &[u8]| p.blends.iter().filter(|(r, c)| r.w == n && c == m).count();
        // 3.: 1 kruh; 4.: 1 kosostvorec; 5.: obe (dva tvary vedla seba) = spolu 2 kruhy a 2 kosostvorce v mriezke
        assert_eq!(
            in_cells(&reset),
            2,
            "{:?}",
            p.blends.iter().map(|(r, _)| r.w).collect::<Vec<_>>()
        );
        assert_eq!(in_cells(&bill), 2);
        // vysvetlivky (2) + znacka pri riadku udalosti (1) maju velkost 8 px
        assert_eq!(p.blends.iter().filter(|(r, _)| r.w == 8).count(), 3);
        // dve znacky v jednom dni sa neprekryvaju
        let five: Vec<&Rect> = p
            .blends
            .iter()
            .filter(|(r, _)| r.w == n)
            .map(|(r, _)| r)
            .filter(|r| {
                r.y == p
                    .blends
                    .iter()
                    .find(|(q, c)| q.w == n && c == &bill)
                    .unwrap()
                    .0
                    .y
                    || true
            })
            .collect();
        for a in &five {
            for b in &five {
                if !std::ptr::eq(*a, *b) && a.y == b.y {
                    assert!(a.x + a.w <= b.x || b.x + b.w <= a.x, "{a:?} {b:?}");
                }
            }
        }
    }
}
