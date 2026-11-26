//! Kratke oznamenia vo vybezku (sami sa po chvili zbalia): hlasitost pri zmene klavesmi, koniec casovaca a najblizsia
//! udalost z kalendara. Ikona, nazov, podnadpis a pripadne posuvnik hodnoty. Cisty stav a kreslenie.
use transom_render::{Color, Font, HAlign, Painter, Rect, TextStyle, Theme};

/// Oznamenie.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// Znak ikony (Segoe Fluent Icons).
    pub icon: char,
    pub title: String,
    pub sub: String,
    /// Hodnota 0..=100 (posuvnik), `None` = bez neho.
    pub value: Option<u8>,
    /// Hodnota je stlmena (napr. zvuk): posuvnik sedou.
    pub muted: bool,
    /// Vzorka farby namiesto ikony (kapatko).
    pub swatch: Option<(u8, u8, u8)>,
}

/// Vyska vybezku (lp).
pub fn height_lp(i: &Info) -> i32 {
    if i.value.is_some() {
        96
    } else {
        82
    }
}

fn st(size: f32, bold: bool, color: Color) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align: HAlign::Left,
    }
}

pub fn paint(p: &mut dyn Painter, th: &Theme, i: &Info) {
    let s = p.scale();
    let px = |v: i32| (v as f32 * s).round() as i32;
    let sz = p.size();
    p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
    let b = px(1).max(1);
    p.fill_rect(Rect::new(b, 0, sz.w - 2 * b, sz.h - b), th.background);
    let pad = px(18);
    let icon = Rect::new(pad, px(14), px(40), px(40));
    if let Some((r, g, b)) = i.swatch {
        p.fill_round_rect(icon, px(12), th.control_edge);
        let inner = Rect::new(
            icon.x + px(2),
            icon.y + px(2),
            icon.w - px(4),
            icon.h - px(4),
        );
        p.fill_round_rect(inner, px(10), Color::rgb(r, g, b));
    } else {
        p.fill_round_rect(icon, px(12), th.control);
    }
    if i.swatch.is_none() {
        p.draw_text(
            &i.icon.to_string(),
            icon,
            &TextStyle {
                font: Font::Icons,
                size_lp: 18.0,
                bold: false,
                color: th.accent_text,
                align: HAlign::Center,
            },
        );
    }
    let x = icon.x + icon.w + px(14);
    let w = (sz.w - x - pad).max(px(40));
    let t = crate::audio_pop::fit(p, &i.title, &st(14.0, true, th.foreground), w);
    p.draw_text(
        &t,
        Rect::new(x, px(14), w, px(22)),
        &st(14.0, true, th.foreground),
    );
    let sub = crate::audio_pop::fit(p, &i.sub, &st(12.0, false, th.muted), w);
    p.draw_text(
        &sub,
        Rect::new(x, px(36), w, px(18)),
        &st(12.0, false, th.muted),
    );
    if let Some(v) = i.value {
        let track = Rect::new(pad, px(68), sz.w - 2 * pad, px(6));
        p.fill_round_rect(track, px(3), th.control_edge);
        let f = v.min(100) as f32 / 100.0;
        p.fill_round_rect(
            Rect::new(track.x, track.y, (track.w as f32 * f) as i32, track.h),
            px(3),
            if i.muted { th.muted } else { th.accent },
        );
    }
}

/// Najblizsia casova udalost, ktora zacne do `lead_s` sekund od `now` (a este nezacala): (nazov, zaciatok).
pub fn next_event(
    occ: &[transom_core::ics::Occurrence],
    now: i64,
    lead_s: i64,
) -> Option<(String, i64)> {
    occ.iter()
        .filter_map(|o| match o.start {
            transom_core::ics::When::At(a) if a > now && a - now <= lead_s => {
                Some((o.summary.clone(), a))
            }
            _ => None,
        })
        .min_by_key(|(_, a)| *a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::ics::{Occurrence, When};
    use transom_render::offscreen::Surface;
    use transom_render::GdiPainter;

    #[test]
    fn the_next_event_is_the_closest_timed_one_inside_the_lead_window() {
        let occ = vec![
            Occurrence {
                summary: "Later".into(),
                start: When::At(1_000 + 3_000),
                duration: 60,
            },
            Occurrence {
                summary: "Soon".into(),
                start: When::At(1_000 + 300),
                duration: 60,
            },
            Occurrence {
                summary: "Started".into(),
                start: When::At(900),
                duration: 600,
            },
            Occurrence {
                summary: "All day".into(),
                start: When::Day(transom_core::clock::Ymd {
                    y: 2026,
                    m: 11,
                    d: 25,
                }),
                duration: 86_400,
            },
        ];
        assert_eq!(next_event(&occ, 1_000, 600), Some(("Soon".into(), 1_300)));
        assert_eq!(next_event(&occ, 1_000, 200), None);
    }

    #[test]
    fn info_paints_without_clipped_text_with_and_without_a_value() {
        let _g = crate::test_lang::guard();
        for value in [None, Some(42)] {
            let i = Info {
                icon: '\u{E994}',
                title: "Volume".into(),
                sub: "42 %".into(),
                value,
                muted: false,
                swatch: None,
            };
            let (w, h) = (transom_platform::dragdrop::NOTCH_W_LP, height_lp(&i));
            let surf = Surface::new(w, h);
            let mut g = GdiPainter::new(surf.dc, transom_render::Size { w, h }, 96);
            let mut a = transom_render::audit::Audit::new(&mut g, Theme::dark().background);
            paint(&mut a, &Theme::dark(), &i);
            a.finish();
            assert!(a.issues.is_empty(), "{:?}", a.issues);
        }
    }
}
