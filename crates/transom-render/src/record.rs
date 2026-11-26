//! Zaznam kreslenia: `Recorder` meria texty skutocnym painterom, ale kreslenie si len zapamata; `replay` ho potom
//! prehra v rovnakom poradi. Takto sa da pod uz rozlozeny obsah dodatocne nakreslit podklad (karta sekcie v nastaveniach),
//! ktoreho rozmer je znamy az po rozlozeni obsahu - bez druheho rozkladania a bez blikania.
use crate::brand::Rgba;
use crate::{Color, Painter, Rect, Size, TextStyle};

/// Jeden zaznamenany prikaz kreslenia.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Fill(Rect, Color),
    Text(String, Rect, TextStyle),
    Scrolled(String, Rect, TextStyle, i32),
    Blit(Rect, Vec<Color>),
    Blend(Rect, Vec<Rgba>),
    Mask(Rect, Vec<u8>, Color),
    RoundFill(Rect, i32, Color),
    RoundStroke(Rect, i32, i32, Color),
    PushClip(Rect),
    PopClip,
}

/// Painter, ktory meranie predava dalej a kreslenie zaznamenava.
pub struct Recorder<'a> {
    inner: &'a mut dyn Painter,
    ops: Vec<Op>,
}

impl<'a> Recorder<'a> {
    pub fn new(inner: &'a mut dyn Painter) -> Self {
        Recorder {
            inner,
            ops: Vec::new(),
        }
    }

    /// Zaznamenane prikazy (painter sa uvolni).
    pub fn into_ops(self) -> Vec<Op> {
        self.ops
    }
}

/// Prehra zaznam na `p` v povodnom poradi.
pub fn replay(p: &mut dyn Painter, ops: Vec<Op>) {
    for op in ops {
        match op {
            Op::Fill(r, c) => p.fill_rect(r, c),
            Op::Text(t, r, s) => p.draw_text(&t, r, &s),
            Op::Scrolled(t, r, s, o) => p.draw_text_scrolled(&t, r, &s, o),
            Op::Blit(r, px) => p.blit(r, &px),
            Op::Blend(r, px) => p.blend_rgba(r, &px),
            Op::Mask(r, cov, c) => p.blend_mask(r, &cov, c),
            Op::RoundFill(r, rad, c) => p.fill_round_rect(r, rad, c),
            Op::RoundStroke(r, rad, b, c) => p.stroke_round_rect(r, rad, b, c),
            Op::PushClip(r) => p.push_clip(r),
            Op::PopClip => p.pop_clip(),
        }
    }
}

impl Painter for Recorder<'_> {
    fn size(&self) -> Size {
        self.inner.size()
    }
    fn scale(&self) -> f32 {
        self.inner.scale()
    }
    fn clip_box(&self) -> Option<Rect> {
        self.inner.clip_box()
    }
    fn measure_text(&mut self, text: &str, style: &TextStyle) -> Size {
        self.inner.measure_text(text, style)
    }
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.ops.push(Op::Fill(rect, color));
    }
    fn draw_text(&mut self, text: &str, rect: Rect, style: &TextStyle) {
        self.ops.push(Op::Text(text.to_string(), rect, *style));
    }
    fn draw_text_scrolled(&mut self, text: &str, clip: Rect, style: &TextStyle, offset_px: i32) {
        self.ops
            .push(Op::Scrolled(text.to_string(), clip, *style, offset_px));
    }
    fn blit(&mut self, rect: Rect, pixels: &[Color]) {
        self.ops.push(Op::Blit(rect, pixels.to_vec()));
    }
    fn blend_rgba(&mut self, rect: Rect, pixels: &[Rgba]) {
        self.ops.push(Op::Blend(rect, pixels.to_vec()));
    }
    fn blend_mask(&mut self, rect: Rect, cov: &[u8], color: Color) {
        self.ops.push(Op::Mask(rect, cov.to_vec(), color));
    }
    // zaoblene tvary sa zaznamenavaju ako celok (obaly ako `Audit` za nimi evidovali plochy pod textom)
    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        self.ops.push(Op::RoundFill(rect, radius, color));
    }
    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, border: i32, color: Color) {
        self.ops.push(Op::RoundStroke(rect, radius, border, color));
    }
    fn push_clip(&mut self, rect: Rect) {
        self.ops.push(Op::PushClip(rect));
    }
    fn pop_clip(&mut self) {
        self.ops.push(Op::PopClip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Font, HAlign};

    /// Painter, ktory si zapisuje volania (meranie: 7 px na znak).
    #[derive(Default)]
    struct Log(Vec<String>);
    impl Painter for Log {
        fn size(&self) -> Size {
            Size { w: 50, h: 40 }
        }
        fn scale(&self) -> f32 {
            1.5
        }
        fn fill_rect(&mut self, r: Rect, _: Color) {
            self.0.push(format!("fill {}", r.x));
        }
        fn measure_text(&mut self, t: &str, _: &TextStyle) -> Size {
            self.0.push(format!("measure {t}"));
            Size {
                w: 7 * t.chars().count() as i32,
                h: 12,
            }
        }
        fn draw_text(&mut self, t: &str, _: Rect, _: &TextStyle) {
            self.0.push(format!("text {t}"));
        }
        fn blit(&mut self, r: Rect, _: &[Color]) {
            self.0.push(format!("blit {}", r.x));
        }
        fn blend_rgba(&mut self, r: Rect, _: &[Rgba]) {
            self.0.push(format!("blend {}", r.x));
        }
        fn fill_round_rect(&mut self, r: Rect, _: i32, _: Color) {
            self.0.push(format!("round {}", r.x));
        }
        fn push_clip(&mut self, r: Rect) {
            self.0.push(format!("clip {}", r.x));
        }
        fn pop_clip(&mut self) {
            self.0.push("unclip".into());
        }
    }

    #[test]
    fn measuring_goes_through_drawing_waits_and_replays_in_order_after_an_underlay() {
        let st = TextStyle {
            font: Font::Ui,
            size_lp: 13.0,
            bold: false,
            color: Color::rgb(1, 2, 3),
            align: HAlign::Left,
        };
        let mut log = Log::default();
        let ops = {
            let mut rec = Recorder::new(&mut log);
            assert_eq!(rec.scale(), 1.5);
            assert_eq!(rec.size(), Size { w: 50, h: 40 });
            assert_eq!(rec.measure_text("abc", &st).w, 21);
            rec.push_clip(Rect::new(1, 0, 5, 5));
            rec.fill_rect(Rect::new(2, 0, 1, 1), Color::rgb(0, 0, 0));
            rec.draw_text("hi", Rect::new(3, 0, 9, 9), &st);
            rec.fill_round_rect(Rect::new(4, 0, 9, 9), 2, Color::rgb(0, 0, 0));
            rec.blit(Rect::new(5, 0, 1, 1), &[Color::rgb(0, 0, 0)]);
            rec.pop_clip();
            rec.into_ops()
        };
        // pocas zaznamu sa kreslilo nic, iba meralo
        assert_eq!(log.0, ["measure abc"]);
        // podklad pred prehratim, potom obsah v povodnom poradi
        log.fill_rect(Rect::new(0, 0, 50, 40), Color::rgb(9, 9, 9));
        replay(&mut log, ops);
        assert_eq!(
            log.0,
            [
                "measure abc",
                "fill 0",
                "clip 1",
                "fill 2",
                "text hi",
                "round 4",
                "blit 5",
                "unclip"
            ]
        );
    }
}
