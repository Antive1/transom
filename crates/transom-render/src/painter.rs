//! Rozhranie `Painter`: jediny sposob, akym scena kresli. Nezavisi od Win32 ani od konkretneho renderera.
//!
//! Zmluva:
//! - Vsetky suradnice a rozmery su vo FYZICKYCH pixeloch (px) povrchu, na ktory sa kresli; pociatok (0, 0) je
//!   lavy horny roh povrchu. Prepocet z logickych pixelov (lp) sa robi cez [`Painter::scale`] (`dpi / 96`).
//! - Velkost pisma sa udava v lp (`TextStyle::size_lp`); backend ju prenasobi `scale()`.
//! - Painter kresli iba na uz pripraveny povrch a nevlastni okno, casovace ani spravy. Kto ho vytvara (hostitel
//!   okna), rozhoduje o zivotnosti povrchu; Painter zije najviac jedno kreslenie (jeden `WM_PAINT`).
//! - Vsetko je jednovlaknove a synchronne; ziadna metoda nevrati chybu - zlyhanie kreslenia (napr. chyba pisma)
//!   sa ticho preskoci, aby jedna chyba nezhodila lištu.
//! - Alfa sa nepouziva: lista je nepriehladna (MVP), preto `Color` nema alfa kanal.
//! - Novy backend (napr. R1 Direct2D) implementuje len tento trait; scena a temy sa nemenia.

/// Farba RGB (8 bitov na kanal), nepriehladna.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// Z `0xRRGGBB`.
    pub const fn hex(v: u32) -> Self {
        Color {
            r: ((v >> 16) & 0xFF) as u8,
            g: ((v >> 8) & 0xFF) as u8,
            b: (v & 0xFF) as u8,
        }
    }
}

pub use crate::brand::Rgba;

/// Obdlznik v px: `x`, `y` je lavy horny roh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Rodina pisma; konkretny font vyberie backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Font {
    /// Text rozhrania (Segoe UI Variable, zaloha Segoe UI; pozri `fonts`).
    #[default]
    Ui,
    /// Ikony (Segoe Fluent Icons na Windows 11).
    Icons,
    /// Cisla, ktore sa prekresluju na mieste (hodiny listy): klasicke Segoe UI s rovnako sirokymi cislicami.
    Tabular,
}

/// Styl textu. Vertikalne sa text vzdy centruje v obdlzniku, do ktoreho sa kresli.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub font: Font,
    /// Velkost pisma v logickych pixeloch (lp).
    pub size_lp: f32,
    pub bold: bool,
    pub color: Color,
    pub align: HAlign,
}

pub trait Painter {
    /// Rozmer kresliaceho povrchu v px.
    fn size(&self) -> Size;

    /// Mierka lp -> px (`dpi / 96`; 1.0 = 100 %, 1.5 = 150 %).
    fn scale(&self) -> f32;

    /// Vyplni obdlznik jednou farbou (nepriehladne).
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// Rozmer, aky by text zabral pri danom style (jeden riadok, bez zalamovania), v px.
    fn measure_text(&mut self, text: &str, style: &TextStyle) -> Size;

    /// Nakresli jednoriadkovy text do `rect` (zarovnanie podla `style.align`, vertikalne na stred).
    /// Text, ktory sa nezmesti, sa orezava; nezalamuje sa.
    fn draw_text(&mut self, text: &str, rect: Rect, style: &TextStyle);

    /// Nakresli text zarovnany vlavo, posunuty o `offset_px` dolava, **orezany presne na `clip`** (bez "...").
    /// Pouziva sa na posuvny text (marquee): nic mimo `clip` sa nenakresli. Predvolene staticky text s "...".
    fn draw_text_scrolled(&mut self, text: &str, clip: Rect, style: &TextStyle, offset_px: i32) {
        let _ = offset_px;
        self.draw_text(text, clip, style);
    }

    /// Ohranicenie aktualnej orezavacej oblasti povrchu (napr. neplatny usek z `WM_PAINT`), ak ho backend pozna.
    /// Scena podla neho preskoci casti mimo (kreslenie mimo orezavacej oblasti by aj tak nic nezmenilo, len by stalo
    /// cas - pri posuvnom texte sa takto prekresluje ~30x/s len usek textu). `None` = nezname, kresli sa vsetko.
    fn clip_box(&self) -> Option<Rect> {
        None
    }

    /// Zmieša RGBA pixely (`rect.w * rect.h`, riadky zhora nadol, **nepremultiplikovana** alfa) **nad existujuci obsah**.
    /// Z neho sa odvodzuju zaoblene vyplne, obrysy, vektorove ikony a logo (okraje nad lubovolnym podkladom
    /// bez farebnych lemov). Predvolene nic (backend bez alfy); pri nespravnom pocte pixelov sa nic nenakresli.
    fn blend_rgba(&mut self, rect: Rect, pixels: &[Rgba]) {
        let _ = (rect, pixels);
    }

    /// Orezanie vsetkeho dalsieho kreslenia na `rect` (v zasobniku; parovy `pop_clip`). Pouziva sa na posuvny obsah.
    fn push_clip(&mut self, rect: Rect) {
        let _ = rect;
    }

    fn pop_clip(&mut self) {}

    /// Maska pokrytia `rect.w * rect.h` (0..=255) vyfarbena `color` nad existujuci obsah.
    fn blend_mask(&mut self, rect: Rect, cov: &[u8], color: Color) {
        if cov.len() != (rect.w.max(0) * rect.h.max(0)) as usize {
            return;
        }
        let px: Vec<Rgba> = cov
            .iter()
            .map(|&a| Rgba {
                r: color.r,
                g: color.g,
                b: color.b,
                a,
            })
            .collect();
        self.blend_rgba(rect, &px);
    }

    /// Zaoblena vypln (AA) s polomerom `radius` px; bez alfa backendu obycajny obdlznik.
    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        let cov = crate::icons::round_rect_coverage(rect.w, rect.h, radius as f32);
        self.blend_mask(rect, &cov, color);
    }

    /// Obrys zaobleneho obdlznika (AA) hrubky `border` px.
    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, border: i32, color: Color) {
        if rect.w <= 2 * border || rect.h <= 2 * border || border <= 0 {
            return;
        }
        let outer = crate::icons::round_rect_coverage(rect.w, rect.h, radius as f32);
        let inner = crate::icons::round_rect_coverage(
            rect.w - 2 * border,
            rect.h - 2 * border,
            (radius - border).max(0) as f32,
        );
        let mut cov = outer;
        for y in 0..rect.h - 2 * border {
            for x in 0..rect.w - 2 * border {
                let o = ((y + border) * rect.w + x + border) as usize;
                let i = (y * (rect.w - 2 * border) + x) as usize;
                cov[o] = cov[o].saturating_sub(inner[i]);
            }
        }
        self.blend_mask(rect, &cov, color);
    }

    /// Prenesie nepriehladne pixely (`rect.w * rect.h`, riadky zhora nadol) presne na `rect` bez zmeny velkosti.
    /// Pri nespravnom pocte pixelov sa nic nenakresli.
    fn blit(&mut self, rect: Rect, pixels: &[Color]);
}

use crate::fonts::is_cjk;

/// Zalomi `text` na riadky sirky najviac `w` px podla merania `p`: po slovach, slovo dlhsie nez riadok a text bez
/// medzier (japoncina, cinstina) sa lomi po znakoch. Usek CJK pokracuje na rozpisanom riadku (inak by kratke slovo
/// pred nim - "この PC", "transom" - ostalo osamotene na riadku); dlhe latinske slovo (cesta) zacina novy riadok.
/// Nikdy nic neorezava ani neskracuje; vzdy aspon jeden riadok.
pub fn wrap_text(p: &mut dyn Painter, text: &str, st: &TextStyle, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let fits = |p: &mut dyn Painter, s: &str| p.measure_text(s, st).w <= w;
    for word in text.split_whitespace() {
        let cand = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if fits(p, &cand) {
            cur = cand;
            continue;
        }
        // CJK sa smie lamat kdekolvek: usek pokracuje na rozpisanom riadku (aj ked by sa zmestil na novy riadok cely)
        let continue_line = !cur.is_empty() && word.chars().next().is_some_and(is_cjk);
        if continue_line {
            // usek CJK: doplni rozpisany riadok po znakoch a pokracuje dalej
            let mut piece = format!("{cur} ");
            cur.clear();
            for ch in word.chars() {
                let mut next = piece.clone();
                next.push(ch);
                if !fits(p, &next) && !piece.trim().is_empty() {
                    lines.push(piece.trim_end().to_string());
                    piece = ch.to_string();
                } else {
                    piece = next;
                }
            }
            cur = piece;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        let mut piece = String::new();
        for ch in word.chars() {
            let mut next = piece.clone();
            next.push(ch);
            if !piece.is_empty() && !fits(p, &next) {
                lines.push(std::mem::take(&mut piece));
                piece.push(ch);
            } else {
                piece = next;
            }
        }
        cur = piece;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Cast plochy ako samostatny kresliaci povrch: posun o (`dx`, `dy`) a vlastna velkost (napr. obsah karty pod
/// riadkom zaloziek). Kreslenie mimo nej sa neoreze samo - volajuci moze pouzit `push_clip`.
pub struct Sub<'a> {
    pub inner: &'a mut dyn Painter,
    pub dx: i32,
    pub dy: i32,
    pub size: Size,
}

impl Sub<'_> {
    fn r(&self, r: Rect) -> Rect {
        Rect::new(r.x + self.dx, r.y + self.dy, r.w, r.h)
    }
}

impl Painter for Sub<'_> {
    fn size(&self) -> Size {
        self.size
    }
    fn scale(&self) -> f32 {
        self.inner.scale()
    }
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let r = self.r(rect);
        self.inner.fill_rect(r, color);
    }
    fn measure_text(&mut self, text: &str, style: &TextStyle) -> Size {
        self.inner.measure_text(text, style)
    }
    fn draw_text(&mut self, text: &str, rect: Rect, style: &TextStyle) {
        let r = self.r(rect);
        self.inner.draw_text(text, r, style);
    }
    fn draw_text_scrolled(&mut self, text: &str, clip: Rect, style: &TextStyle, offset_px: i32) {
        let r = self.r(clip);
        self.inner.draw_text_scrolled(text, r, style, offset_px);
    }
    fn clip_box(&self) -> Option<Rect> {
        self.inner
            .clip_box()
            .map(|c| Rect::new(c.x - self.dx, c.y - self.dy, c.w, c.h))
    }
    fn blend_rgba(&mut self, rect: Rect, pixels: &[Rgba]) {
        let r = self.r(rect);
        self.inner.blend_rgba(r, pixels);
    }
    fn push_clip(&mut self, rect: Rect) {
        let r = self.r(rect);
        self.inner.push_clip(r);
    }
    fn pop_clip(&mut self) {
        self.inner.pop_clip();
    }
    fn blend_mask(&mut self, rect: Rect, cov: &[u8], color: Color) {
        let r = self.r(rect);
        self.inner.blend_mask(r, cov, color);
    }
    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        let r = self.r(rect);
        self.inner.fill_round_rect(r, radius, color);
    }
    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, border: i32, color: Color) {
        let r = self.r(rect);
        self.inner.stroke_round_rect(r, radius, border, color);
    }
    fn blit(&mut self, rect: Rect, pixels: &[Color]) {
        let r = self.r(rect);
        self.inner.blit(r, pixels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Painter len na meranie: kazdy znak je 10 px siroky.
    struct Mono;
    impl Painter for Mono {
        fn size(&self) -> Size {
            Size { w: 100, h: 100 }
        }
        fn scale(&self) -> f32 {
            1.0
        }
        fn fill_rect(&mut self, _: Rect, _: Color) {}
        fn measure_text(&mut self, t: &str, _: &TextStyle) -> Size {
            Size {
                w: 10 * t.chars().count() as i32,
                h: 14,
            }
        }
        fn draw_text(&mut self, _: &str, _: Rect, _: &TextStyle) {}
        fn blit(&mut self, _: Rect, _: &[Color]) {}
    }

    fn st() -> TextStyle {
        TextStyle {
            font: Font::Ui,
            size_lp: 12.0,
            bold: false,
            color: Color::rgb(0, 0, 0),
            align: HAlign::Left,
        }
    }

    #[test]
    fn wrap_breaks_on_words_then_characters_and_never_loses_text() {
        let mut p = Mono;
        // 5 znakov na riadok
        assert_eq!(wrap_text(&mut p, "ab cd ef", &st(), 50), ["ab cd", "ef"]);
        assert_eq!(
            wrap_text(&mut p, "abcdefghijkl", &st(), 50),
            ["abcde", "fghij", "kl"]
        );
        // text bez medzier (japoncina): lom po znakoch
        assert_eq!(
            wrap_text(&mut p, "日本語のテキスト", &st(), 40),
            ["日本語の", "テキスト"]
        );
        assert_eq!(wrap_text(&mut p, "", &st(), 50), [""]);
        assert_eq!(wrap_text(&mut p, "  ", &st(), 50), [""]);
        // dlhe slovo uprostred: predosle slova ostanu na svojom riadku
        assert_eq!(
            wrap_text(&mut p, "a bcdefghij k", &st(), 50),
            ["a", "bcdef", "ghij", "k"]
        );
        // zachovanie znakov (okrem medzier na zlome)
        let joined: String = wrap_text(&mut p, "ab cd ef gh", &st(), 50).join(" ");
        assert_eq!(joined, "ab cd ef gh");
        // dlhy usek CJK za kratkym slovom pokracuje na tom istom riadku (ziadne osamotene "PC")
        assert_eq!(
            wrap_text(&mut p, "PC に暗号化せずに保存", &st(), 50),
            ["PC に暗", "号化せずに", "保存"]
        );
        let all: String =
            wrap_text(&mut p, "この PC に暗号化せずに保存されます", &st(), 60).concat();
        assert_eq!(all.replace(' ', ""), "このPCに暗号化せずに保存されます");
    }

    #[test]
    fn hex_splits_channels() {
        assert_eq!(Color::hex(0x1E2228), Color::rgb(0x1E, 0x22, 0x28));
        assert_eq!(Color::hex(0xFFFFFF), Color::rgb(255, 255, 255));
    }
}
