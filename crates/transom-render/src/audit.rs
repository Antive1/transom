//! Pomocne obaly `Painter`: `Track` zistuje vysku obsahu (pre posuvanie) a `Audit` pri vykreslovani kontroluje
//! kazdy text: nezmesti sa do svojho obdlznika (orezanie / "..."), presahuje vysku, presahuje povrch, prekryva sa s inym
//! textom alebo ma nizky kontrast voci ploche, na ktorej je nakresleny. Pouziva sa v rezime `--shot` (matica jazykov x
//! stranok x DPI x tem) a v testoch; beznemu behu nic nestoji.
use crate::{Color, Painter, Rect, Size, TextStyle};

/// Zaznamenava najnizsi nakresleny bod (px) - vyska obsahu stranky.
pub struct Track<'a> {
    pub inner: &'a mut dyn Painter,
    pub max_bottom: i32,
}

impl<'a> Track<'a> {
    pub fn new(inner: &'a mut dyn Painter) -> Self {
        Track {
            inner,
            max_bottom: i32::MIN,
        }
    }
    fn note(&mut self, r: Rect) {
        self.max_bottom = self.max_bottom.max(r.y + r.h);
    }
}

impl Painter for Track<'_> {
    fn size(&self) -> Size {
        self.inner.size()
    }
    fn scale(&self) -> f32 {
        self.inner.scale()
    }
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.note(rect);
        self.inner.fill_rect(rect, color);
    }
    fn measure_text(&mut self, text: &str, style: &TextStyle) -> Size {
        self.inner.measure_text(text, style)
    }
    fn draw_text(&mut self, text: &str, rect: Rect, style: &TextStyle) {
        self.note(rect);
        self.inner.draw_text(text, rect, style);
    }
    fn draw_text_scrolled(&mut self, text: &str, clip: Rect, style: &TextStyle, offset_px: i32) {
        self.note(clip);
        self.inner.draw_text_scrolled(text, clip, style, offset_px);
    }
    fn blit(&mut self, rect: Rect, pixels: &[Color]) {
        self.note(rect);
        self.inner.blit(rect, pixels);
    }
    fn blend_rgba(&mut self, rect: Rect, pixels: &[crate::brand::Rgba]) {
        self.note(rect);
        self.inner.blend_rgba(rect, pixels);
    }
    fn push_clip(&mut self, rect: Rect) {
        self.inner.push_clip(rect);
    }
    fn pop_clip(&mut self) {
        self.inner.pop_clip();
    }
    fn clip_box(&self) -> Option<Rect> {
        self.inner.clip_box()
    }
    // zaoblene tvary sa musia preniest nezmenene (Audit za nimi eviduje plochy pod textom)
    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        self.note(rect);
        self.inner.fill_round_rect(rect, radius, color);
    }
    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, border: i32, color: Color) {
        self.note(rect);
        self.inner.stroke_round_rect(rect, radius, border, color);
    }
    fn blend_mask(&mut self, rect: Rect, cov: &[u8], color: Color) {
        self.note(rect);
        self.inner.blend_mask(rect, cov, color);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Issue {
    /// Text je sirsi nez jeho obdlznik (orezal by sa alebo skratil na "...").
    Clipped { text: String, need: i32, have: i32 },
    /// Text je vyssi nez jeho obdlznik.
    TooTall { text: String, need: i32, have: i32 },
    /// Text presahuje povrch okna.
    Offscreen { text: String },
    /// Dva texty sa prekryvaju.
    Overlap { a: String, b: String, area: i32 },
    /// Kontrast textu voci ploche pod nim je pod hranicou.
    Contrast { text: String, ratio: f32, need: f32 },
}

#[derive(Clone)]
struct Ink {
    text: String,
    r: Rect,
}

/// Obal, ktory predava vsetko vnutornemu painteru a zaroven kontroluje texty.
pub struct Audit<'a> {
    pub inner: &'a mut dyn Painter,
    pub issues: Vec<Issue>,
    inks: Vec<Ink>,
    fills: Vec<(Rect, Color)>,
    /// Podretazce textov, ktore sa zamerne skracuju ("...", cesta k suboru, ukazkovy dlhy nazov).
    pub allow_elide: Vec<String>,
    /// Povrch, do ktoreho sa kresli (px); texty mimo sa hlasia.
    pub surface: Rect,
    /// Ak je nastavene, texty mimo tohto obdlznika (napr. mimo viewportu posuvania) sa nekontroluju.
    pub only_inside: Option<Rect>,
    /// Predvolene pozadie, ak pod textom nie je ziadna vyplň.
    pub base_bg: Color,
}

fn lum(c: Color) -> f32 {
    let f = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.039_28 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
}

pub fn ratio(a: Color, b: Color) -> f32 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn inter(a: Rect, b: Rect) -> i32 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if w > 0 && h > 0 {
        w * h
    } else {
        0
    }
}

impl<'a> Audit<'a> {
    pub fn new(inner: &'a mut dyn Painter, base_bg: Color) -> Self {
        let sz = inner.size();
        Audit {
            inner,
            issues: Vec::new(),
            inks: Vec::new(),
            fills: Vec::new(),
            allow_elide: Vec::new(),
            surface: Rect::new(0, 0, sz.w, sz.h),
            only_inside: None,
            base_bg,
        }
    }

    /// Vyhodnoti prekryvy medzi textami (volat po dokonceni kreslenia).
    pub fn finish(&mut self) {
        let inks = std::mem::take(&mut self.inks);
        for i in 0..inks.len() {
            for j in i + 1..inks.len() {
                let area = inter(inks[i].r, inks[j].r);
                // dotyk okrajov (1-2 px) nie je prekryv; skutocne zasahujuce boxy ano
                if area > 6 && inks[i].text != inks[j].text {
                    self.issues.push(Issue::Overlap {
                        a: inks[i].text.clone(),
                        b: inks[j].text.clone(),
                        area,
                    });
                }
            }
        }
    }

    fn bg_at(&self, r: Rect) -> Color {
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        self.fills
            .iter()
            .rev()
            .find(|(f, _)| cx >= f.x && cx < f.x + f.w && cy >= f.y && cy < f.y + f.h)
            .map(|(_, c)| *c)
            .unwrap_or(self.base_bg)
    }
}

impl Painter for Audit<'_> {
    fn size(&self) -> Size {
        self.inner.size()
    }
    fn scale(&self) -> f32 {
        self.inner.scale()
    }
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fills.push((rect, color));
        self.inner.fill_rect(rect, color);
    }
    fn measure_text(&mut self, text: &str, style: &TextStyle) -> Size {
        self.inner.measure_text(text, style)
    }
    fn draw_text(&mut self, text: &str, rect: Rect, style: &TextStyle) {
        let skip = self.only_inside.is_some_and(|v| inter(v, rect) == 0);
        if !text.trim().is_empty() && !skip {
            let m = self.inner.measure_text(text, style);
            let elide_ok = self.allow_elide.iter().any(|a| text.contains(a.as_str()));
            if m.w > rect.w + 1 && !elide_ok {
                self.issues.push(Issue::Clipped {
                    text: text.to_string(),
                    need: m.w,
                    have: rect.w,
                });
            }
            if m.h > rect.h + 1 {
                self.issues.push(Issue::TooTall {
                    text: text.to_string(),
                    need: m.h,
                    have: rect.h,
                });
            }
            // inkova oblast podla zarovnania (ak sa text zmesti; inak cely obdlznik)
            let w = m.w.min(rect.w);
            let x = match style.align {
                crate::HAlign::Left => rect.x,
                crate::HAlign::Right => rect.x + rect.w - w,
                crate::HAlign::Center => rect.x + (rect.w - w) / 2,
            };
            let ink = Rect::new(x, rect.y + (rect.h - m.h) / 2, w, m.h);
            let inside_surface = ink.x >= self.surface.x
                && ink.y >= self.surface.y
                && ink.x + ink.w <= self.surface.x + self.surface.w
                && ink.y + ink.h <= self.surface.y + self.surface.h;
            // viewport: texty ciastocne mimo posuvaneho okna nie su chyba (su orezane zamerne)
            if !inside_surface && self.only_inside.is_none() {
                self.issues.push(Issue::Offscreen {
                    text: text.to_string(),
                });
            }
            let bg = self.bg_at(ink);
            let need = if matches!(style.font, crate::Font::Icons) {
                3.0
            } else {
                4.5
            };
            let r = ratio(style.color, bg);
            if r < need {
                self.issues.push(Issue::Contrast {
                    text: text.to_string(),
                    ratio: r,
                    need,
                });
            }
            self.inks.push(Ink {
                text: text.to_string(),
                r: ink,
            });
        }
        self.inner.draw_text(text, rect, style);
    }
    fn draw_text_scrolled(&mut self, text: &str, clip: Rect, style: &TextStyle, offset_px: i32) {
        self.inner.draw_text_scrolled(text, clip, style, offset_px);
    }
    fn blit(&mut self, rect: Rect, pixels: &[Color]) {
        self.inner.blit(rect, pixels);
    }
    fn blend_rgba(&mut self, rect: Rect, pixels: &[crate::brand::Rgba]) {
        self.inner.blend_rgba(rect, pixels);
    }
    fn push_clip(&mut self, rect: Rect) {
        self.inner.push_clip(rect);
    }
    fn pop_clip(&mut self) {
        self.inner.pop_clip();
    }
    fn clip_box(&self) -> Option<Rect> {
        self.inner.clip_box()
    }
    // zaoblene vyplne zaznamenaj ako plochy pod textom (kontrast); kreslenie ide cez blend_rgba vnutorneho painteru
    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        self.fills.push((rect, color));
        self.inner.fill_round_rect(rect, radius, color);
    }
    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, border: i32, color: Color) {
        self.inner.stroke_round_rect(rect, radius, border, color);
    }
    fn blend_mask(&mut self, rect: Rect, cov: &[u8], color: Color) {
        self.inner.blend_mask(rect, cov, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Font, HAlign};

    /// Jednoduchy painter bez GDI: kazdy znak je 7 px siroky a text 14 px vysoky.
    #[derive(Default)]
    struct Fake;
    impl Painter for Fake {
        fn size(&self) -> Size {
            Size { w: 400, h: 200 }
        }
        fn scale(&self) -> f32 {
            1.0
        }
        fn fill_rect(&mut self, _: Rect, _: Color) {}
        fn measure_text(&mut self, t: &str, _: &TextStyle) -> Size {
            Size {
                w: 7 * t.chars().count() as i32,
                h: 14,
            }
        }
        fn draw_text(&mut self, _: &str, _: Rect, _: &TextStyle) {}
        fn blit(&mut self, _: Rect, _: &[Color]) {}
    }

    fn st(color: Color, align: HAlign) -> TextStyle {
        TextStyle {
            font: Font::Ui,
            size_lp: 12.0,
            bold: false,
            color,
            align,
        }
    }

    #[test]
    fn detects_clipping_height_overlap_offscreen_and_contrast() {
        let mut f = Fake;
        let white = Color::rgb(255, 255, 255);
        let mut a = Audit::new(&mut f, white);
        let black = Color::rgb(0, 0, 0);
        a.draw_text(
            "0123456789",
            Rect::new(0, 0, 50, 20),
            &st(black, HAlign::Left),
        ); // 70 > 50
        a.draw_text("tall", Rect::new(0, 40, 100, 10), &st(black, HAlign::Left)); // 14 > 10
        a.draw_text("ok", Rect::new(0, 60, 100, 20), &st(black, HAlign::Left));
        a.draw_text(
            "overlap A",
            Rect::new(0, 100, 100, 20),
            &st(black, HAlign::Left),
        );
        a.draw_text(
            "overlap B",
            Rect::new(20, 105, 100, 20),
            &st(black, HAlign::Left),
        );
        a.draw_text(
            "edge",
            Rect::new(380, 150, 100, 20),
            &st(black, HAlign::Left),
        ); // mimo povrchu (400)
        a.draw_text(
            "pale",
            Rect::new(0, 170, 100, 20),
            &st(Color::rgb(230, 230, 230), HAlign::Left),
        ); // slaby kontrast
        a.finish();
        let kinds: Vec<&str> = a
            .issues
            .iter()
            .map(|i| match i {
                Issue::Clipped { .. } => "clip",
                Issue::TooTall { .. } => "tall",
                Issue::Overlap { .. } => "overlap",
                Issue::Offscreen { .. } => "offscreen",
                Issue::Contrast { .. } => "contrast",
            })
            .collect();
        for k in ["clip", "tall", "overlap", "offscreen", "contrast"] {
            assert!(kinds.contains(&k), "{k} chyba: {kinds:?}");
        }
        assert_eq!(
            kinds.iter().filter(|k| **k == "clip").count(),
            1,
            "{kinds:?}"
        );
    }

    #[test]
    fn allow_list_viewport_and_same_text_do_not_raise_issues() {
        let mut f = Fake;
        let mut a = Audit::new(&mut f, Color::rgb(255, 255, 255));
        a.allow_elide.push("C:\\".into());
        a.draw_text(
            "C:\\Users\\a\\very\\long\\path",
            Rect::new(0, 0, 40, 20),
            &st(Color::rgb(0, 0, 0), HAlign::Left),
        );
        a.only_inside = Some(Rect::new(0, 0, 100, 100));
        a.draw_text(
            "bez kontroly",
            Rect::new(0, 300, 10, 20),
            &st(Color::rgb(230, 230, 230), HAlign::Left),
        );
        a.finish();
        assert!(a.issues.is_empty(), "{:?}", a.issues);
    }

    #[test]
    fn contrast_uses_the_fill_under_the_text() {
        let mut f = Fake;
        let mut a = Audit::new(&mut f, Color::rgb(255, 255, 255));
        // biely text na tmavej vyplni je v poriadku, na svetlej nie
        a.fill_rect(Rect::new(0, 0, 100, 30), Color::rgb(20, 20, 20));
        a.draw_text(
            "on dark",
            Rect::new(0, 0, 100, 30),
            &st(Color::rgb(255, 255, 255), HAlign::Left),
        );
        a.draw_text(
            "on light",
            Rect::new(0, 100, 100, 30),
            &st(Color::rgb(255, 255, 255), HAlign::Left),
        );
        assert_eq!(a.issues.len(), 1);
        assert!(matches!(&a.issues[0], Issue::Contrast { text, .. } if text == "on light"));
    }

    #[test]
    fn track_records_the_lowest_drawn_point() {
        let mut f = Fake;
        let mut t = Track::new(&mut f);
        assert_eq!(t.max_bottom, i32::MIN);
        t.fill_rect(Rect::new(0, 10, 5, 20), Color::rgb(0, 0, 0));
        t.draw_text(
            "x",
            Rect::new(0, 100, 5, 28),
            &st(Color::rgb(0, 0, 0), HAlign::Left),
        );
        t.fill_rect(Rect::new(0, 0, 5, 5), Color::rgb(0, 0, 0));
        assert_eq!(t.max_bottom, 128);
    }
}
