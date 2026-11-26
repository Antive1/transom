//! Nahlad schranky vo vybezku: po prejdeni mysou nad tlacidlom Schranka (len so zapnutou historiou) posledne
//! skopirovane veci; klik vrati polozku do schranky. Historiu nikdy nezapina. Cisty stav a kreslenie.
use transom_platform::appbar::PointerKind;
use transom_render::design;
use transom_render::{Font, HAlign, Painter, Rect, TextStyle, Theme};
use transom_tools::clipboard::ClipKind;
use transom_tools::service::ClipRow;

pub const MAX_ROWS: usize = 5;
const HEAD_LP: i32 = 54;
const ROW_LP: i32 = 40;
const PAD_LP: i32 = 12;

pub fn height_lp(n: usize) -> i32 {
    HEAD_LP + (n.clamp(1, MAX_ROWS) as i32) * ROW_LP + PAD_LP
}

#[derive(Default)]
pub struct ClipNotch {
    pub hover: Option<String>,
    press: Option<String>,
    hits: Vec<(Rect, String)>,
}

fn st(size: f32, bold: bool, color: transom_render::Color) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align: HAlign::Left,
    }
}

impl ClipNotch {
    pub fn paint(&mut self, p: &mut dyn Painter, th: &Theme, rows: &[ClipRow]) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        self.hits.clear();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, 0, sz.w - 2 * b, sz.h - b), th.background);
        let pad = px(14);
        let w = sz.w - 2 * pad;
        p.draw_text(
            &transom_i18n::t("tools.sec_clipboard"),
            Rect::new(pad, px(8), w, px(24)),
            &st(14.0, true, th.foreground),
        );
        let hint = st(12.0, false, th.muted);
        let h = crate::audio_pop::fit(p, &transom_i18n::t("clip.peek_hint"), &hint, w);
        p.draw_text(&h, Rect::new(pad, px(31), w, px(18)), &hint);
        let mut y = px(HEAD_LP);
        if rows.is_empty() {
            p.draw_text(
                &transom_i18n::t("tools.empty_clip"),
                Rect::new(pad, y, w, px(ROW_LP)),
                &hint,
            );
            return;
        }
        for r in rows.iter().take(MAX_ROWS) {
            let rr = Rect::new(px(8), y, sz.w - px(16), px(ROW_LP - 2));
            if self.hover.as_deref() == Some(r.id.as_str()) {
                p.fill_round_rect(rr, px(design::radius::CONTROL), th.control_hover);
            }
            let glyph = match r.kind {
                ClipKind::Image => '\u{E8B9}',
                ClipKind::Files => '\u{E8B7}',
                ClipKind::Text => '\u{E8C8}',
            };
            p.draw_text(
                &glyph.to_string(),
                Rect::new(rr.x + px(6), rr.y, px(22), rr.h),
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 14.0,
                    bold: false,
                    color: th.muted,
                    align: HAlign::Center,
                },
            );
            let ns = st(13.0, false, th.foreground);
            let one_line: String = r.preview.split_whitespace().collect::<Vec<_>>().join(" ");
            let text = crate::audio_pop::fit(p, &one_line, &ns, rr.w - px(44));
            p.draw_text(
                &text,
                Rect::new(rr.x + px(34), rr.y, rr.w - px(40), rr.h),
                &ns,
            );
            self.hits.push((rr, r.id.clone()));
            y += px(ROW_LP);
        }
    }

    /// Mys vo vybezku: (polozka na obnovenie do schranky, treba prekreslit).
    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32) -> (Option<String>, bool) {
        let here = if kind == PointerKind::Leave {
            None
        } else {
            self.hits
                .iter()
                .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
                .map(|(_, id)| id.clone())
        };
        let redraw = here != self.hover;
        self.hover = here.clone();
        let out = match kind {
            PointerKind::Down => {
                self.press = here;
                None
            }
            PointerKind::Click => self.press.take().filter(|p| Some(p) == here.as_ref()),
            _ => None,
        };
        (out, redraw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_render::offscreen::Surface;
    use transom_render::GdiPainter;

    fn row(i: usize) -> ClipRow {
        ClipRow {
            id: format!("c{i}"),
            kind: ClipKind::Text,
            preview: format!("copied text number {i}\nsecond line"),
            app: String::new(),
            size: 10,
            pinned: false,
            added: 0,
        }
    }

    #[test]
    fn rows_paint_cleanly_and_a_click_restores_that_item() {
        let _g = crate::test_lang::guard();
        let rows: Vec<ClipRow> = (0..7).map(row).collect();
        let (w, h) = (
            transom_platform::dragdrop::NOTCH_W_LP,
            height_lp(rows.len()),
        );
        let surf = Surface::new(w, h);
        let mut g = GdiPainter::new(surf.dc, transom_render::Size { w, h }, 96);
        let mut a = transom_render::audit::Audit::new(&mut g, Theme::dark().background);
        a.allow_elide = vec!["…".into()];
        let mut n = ClipNotch::default();
        n.paint(&mut a, &Theme::dark(), &rows);
        a.finish();
        assert!(a.issues.is_empty(), "{:?}", a.issues);
        assert_eq!(n.hits.len(), MAX_ROWS);
        let r = n.hits[1].0;
        n.pointer(PointerKind::Down, r.x + 10, r.y + 5);
        assert_eq!(
            n.pointer(PointerKind::Click, r.x + 10, r.y + 5)
                .0
                .as_deref(),
            Some("c1")
        );
    }
}
