//! Scena listy: nakresli stavove polozky (`transom_core::item`) podla rozlozenia (`transom_core::layout`).
//! Pracuje len cez `Painter`; nevie nic o Win32 ani o tom, odkial data pochadzaju.
use crate::icons::{self, MediaIcon};
use crate::painter::{Color, Font, HAlign, Painter, Rect, Rgba, Size, TextStyle};
use crate::theme::Theme;
use transom_core::item::{AiLogo, Glyph, ItemId, Mark, Part, Seg, StatusItem, Tone};
use transom_core::layout::{layout, Metrics, Placed};

/// Velkost pisma hodin v lp.
pub const CLOCK_SIZE_LP: f32 = 14.0;
/// Velkost ostatneho textu v lp.
pub const TEXT_SIZE_LP: f32 = 13.0;
/// Velkost ikon z pisma (Search, Tray) v lp; ikony medii su vektorove (`MEDIA_ICON_LP`).
pub const ICON_SIZE_LP: f32 = 16.0;
/// Symbol baterie: telo (sirka x vyska) a vystupok, v lp.
const BATT_W: f32 = 20.0;
const BATT_H: f32 = 10.0;
const BATT_TIP_W: f32 = 2.0;
const BATT_TIP_H: f32 = 4.0;
/// Blesk pri nabijani (vlavo od tela).
const BOLT_W: f32 = 14.0;
/// Zvislý okraj hover obdlznika v lp.
const HOVER_INSET_LP: f32 = 3.0;

/// Znak ikony v pisme `Font::Icons`.
/// Sirka vycnelku odlozenych suborov (bez okrajov casti): s poctom podla textu, prazdny = kratky jazycek.
fn pill_width(p: &mut dyn Painter, theme: &Theme, label: &str, scale: f32) -> i32 {
    if label.is_empty() {
        return px(36.0, scale);
    }
    let st = TextStyle {
        bold: true,
        size_lp: 12.0,
        ..text_style(theme, ItemId::Shelf, Tone::Normal)
    };
    (p.measure_text(label, &st).w + px(24.0, scale)).max(px(42.0, scale))
}

// ---- ostrov v strede listy ----

/// Kapsula ostrova: vyska, okraje, znacka a medzery (lp).
const ISLAND_PILL_H: f32 = 22.0;
const ISLAND_PILL_PAD: f32 = 10.0;
const ISLAND_MARK: f32 = 14.0;
const ISLAND_GAP: f32 = 6.0;
/// Medzera medzi stranou a kapsulou a najvacsia sirka textu strany (lp).
const ISLAND_SIDE_GAP: f32 = 10.0;
const ISLAND_SIDE_MAX: f32 = 180.0;
/// Obal a ekvalizer v strane (lp).
const ISLAND_ICON: f32 = 18.0;
/// Logo AI (lp).
const ISLAND_LOGO: f32 = 14.0;

fn island_text(theme: &Theme, strong: bool) -> TextStyle {
    TextStyle {
        bold: strong,
        size_lp: 12.0,
        ..text_style(theme, ItemId::Shelf, Tone::Normal)
    }
}

/// Sirka obsahu strany (px).
fn seg_width(p: &mut dyn Painter, theme: &Theme, s: &Seg, scale: f32) -> i32 {
    match s {
        Seg::None => 0,
        Seg::Text { text, strong, .. } => p
            .measure_text(text, &island_text(theme, *strong))
            .w
            .min(px(ISLAND_SIDE_MAX, scale)),
        Seg::Art(_) | Seg::Eq { .. } => px(ISLAND_ICON, scale),
        Seg::Logo { text, .. } => {
            let t = if text.is_empty() {
                0
            } else {
                px(ISLAND_GAP, scale)
                    + p.measure_text(text, &island_text(theme, false))
                        .w
                        .min(px(ISLAND_SIDE_MAX, scale))
            };
            px(ISLAND_LOGO, scale) + t
        }
        Seg::Cover { text, .. } => {
            px(ISLAND_ICON, scale)
                + px(ISLAND_GAP, scale)
                + p.measure_text(text, &island_text(theme, false))
                    .w
                    .min(px(ISLAND_SIDE_MAX, scale))
        }
    }
}

/// Sirka kapsuly (px): znacka, nazov a pocet.
fn island_pill_width(
    p: &mut dyn Painter,
    theme: &Theme,
    label: &str,
    badge: &str,
    scale: f32,
) -> i32 {
    let mut w = 2 * px(ISLAND_PILL_PAD, scale) + px(ISLAND_MARK, scale);
    if !label.is_empty() {
        w += px(ISLAND_GAP, scale) + p.measure_text(label, &island_text(theme, false)).w;
    }
    if !badge.is_empty() {
        w += px(ISLAND_GAP, scale) + badge_width(p, theme, badge, scale);
    }
    w
}

fn badge_width(p: &mut dyn Painter, theme: &Theme, badge: &str, scale: f32) -> i32 {
    let st = TextStyle {
        size_lp: 10.0,
        ..island_text(theme, false)
    };
    (p.measure_text(badge, &st).w + px(8.0, scale)).max(px(16.0, scale))
}

/// Sirka strany ostrova (obe rovnake, aby kapsula ostala v strede) a celkova sirka (px).
fn island_width(p: &mut dyn Painter, theme: &Theme, part: &Part, scale: f32) -> (i32, i32) {
    let Part::Island {
        left,
        right,
        label,
        badge,
        ..
    } = part
    else {
        return (0, 0);
    };
    let side = seg_width(p, theme, left, scale).max(seg_width(p, theme, right, scale));
    let gap = if side > 0 {
        px(ISLAND_SIDE_GAP, scale)
    } else {
        0
    };
    let pill = island_pill_width(p, theme, label, badge, scale);
    (side, pill + 2 * (side + gap))
}

/// Maska hviezdicky `n` x `n` px (pokrytie 0..=255): 8 lucov s oblymi koncami, striedavo dlhsie a kratsie.
pub fn spark(n: i32) -> Vec<u8> {
    let n = n.max(4);
    let c = (n as f32 - 1.0) / 2.0;
    let half = n as f32 / 2.0;
    let thick = (n as f32 * 0.09).max(0.9);
    let mut out = Vec::with_capacity((n * n) as usize);
    for y in 0..n {
        for x in 0..n {
            let mut hits = 0u32;
            // 3 x 3 podvzorky na hladke okraje
            for sy in 0..3 {
                for sx in 0..3 {
                    let qx = x as f32 + (sx as f32 + 0.5) / 3.0 - 0.5 - c;
                    let qy = y as f32 + (sy as f32 + 0.5) / 3.0 - 0.5 - c;
                    let hit = (0..8).any(|k| {
                        let a = k as f32 * std::f32::consts::FRAC_PI_4 + 0.2;
                        let len = if k % 2 == 0 { half * 0.98 } else { half * 0.72 };
                        let (dx, dy) = (a.cos(), a.sin());
                        let t = (qx * dx + qy * dy).clamp(0.0, len);
                        let (ex, ey) = (qx - dx * t, qy - dy * t);
                        (ex * ex + ey * ey).sqrt() <= thick
                    });
                    hits += u32::from(hit);
                }
            }
            out.push((hits * 255 / 9) as u8);
        }
    }
    out
}

/// Vyska stlpca ekvalizera 0..=1 v case `t` (s): kazdy stlpec vlastna zmes sinusov, pozastavene = nizke a nehybne.
pub fn eq_level(bar: usize, t: f32, playing: bool) -> f32 {
    if !playing {
        return 0.18;
    }
    let b = bar as f32;
    let v = 0.55
        + 0.25 * (t * (5.1 + b * 1.3) + b * 1.7).sin()
        + 0.2 * (t * (8.3 + b * 0.7) + b * 0.9).sin();
    v.clamp(0.12, 1.0)
}

/// Strana ostrova v `r`: text pri kapsule (`toward_right` = zarovnany doprava, k nej), obal, ekvalizer.
fn paint_seg(
    p: &mut dyn Painter,
    theme: &Theme,
    s: &Seg,
    r: Rect,
    toward_right: bool,
    bg: Color,
    scale: f32,
) {
    match s {
        Seg::None => {}
        Seg::Text {
            text,
            muted,
            strong,
            low,
            scroll,
        } => {
            let mut st = island_text(theme, *strong);
            st.color = if *low {
                theme.low
            } else if *muted {
                theme.muted
            } else {
                theme.foreground
            };
            st.align = if toward_right {
                HAlign::Right
            } else {
                HAlign::Left
            };
            match scroll {
                Some(sc) => draw_scrolling(p, text, &st, r, *sc, scale),
                None => {
                    let t = elide(p, text, &st, r.w);
                    p.draw_text(&t, r, &st);
                }
            }
        }
        Seg::Art(art) => {
            let n = px(ISLAND_ICON, scale).min(r.h);
            let x = if toward_right { r.x + r.w - n } else { r.x };
            paint_cover(
                p,
                theme,
                art.as_ref(),
                Rect::new(x, r.y + (r.h - n) / 2, n, n),
                bg,
                scale,
            );
        }
        Seg::Logo {
            logo,
            text,
            low,
            dot,
        } => {
            let n = px(ISLAND_LOGO, scale);
            let mut st = island_text(theme, *low);
            st.color = if *low { theme.low } else { theme.foreground };
            let gap = px(ISLAND_GAP, scale);
            let tw = if text.is_empty() {
                0
            } else {
                p.measure_text(text, &st).w.min(r.w - n - gap).max(0)
            };
            let total = n + if tw > 0 { gap + tw } else { 0 };
            let x = if toward_right { r.x + r.w - total } else { r.x };
            let lr = Rect::new(x, r.y + (r.h - n) / 2, n, n);
            paint_ai_logo(p, theme, *logo, lr);
            // stavova bodka vpravo hore pri logu (s lemom farby listy, aby sa oddelila)
            if let Some((cr, cg, cb)) = dot {
                let d = px(7.0, scale).max(5);
                let ring = px(1.5, scale).max(1);
                let c = Rect::new(lr.x + lr.w - d + ring, lr.y - ring, d, d);
                p.fill_round_rect(
                    Rect::new(c.x - ring, c.y - ring, d + 2 * ring, d + 2 * ring),
                    (d + 2 * ring) / 2,
                    bg,
                );
                p.fill_round_rect(c, d / 2, Color::rgb(*cr, *cg, *cb));
            }
            if tw > 0 {
                let t = elide(p, text, &st, tw);
                st.align = HAlign::Left;
                p.draw_text(&t, Rect::new(x + n + gap, r.y, tw, r.h), &st);
            }
        }
        Seg::Cover { art, text, scroll } => {
            // obal a nazov tesne pri kapsule (strana moze byt sirsia, ak je druha strana dlhsia)
            let n = px(ISLAND_ICON, scale).min(r.h);
            let st = island_text(theme, false);
            let gap = px(ISLAND_GAP, scale);
            let tw = p.measure_text(text, &st).w.min(r.w - n - gap).max(0);
            let x = if toward_right {
                r.x + r.w - (n + gap + tw)
            } else {
                r.x
            };
            paint_cover(
                p,
                theme,
                art.as_ref(),
                Rect::new(x, r.y + (r.h - n) / 2, n, n),
                bg,
                scale,
            );
            let tr = Rect::new(x + n + gap, r.y, tw, r.h);
            match scroll {
                Some(sc) => draw_scrolling(p, text, &st, tr, *sc, scale),
                None => {
                    let t = elide(p, text, &st, tw);
                    p.draw_text(&t, tr, &st);
                }
            }
        }
        Seg::Eq {
            levels,
            color,
            playing,
        } => {
            let bars = 5;
            let bw = px(3.0, scale).max(2);
            let gap = px(2.0, scale).max(1);
            let full = px(14.0, scale);
            let total = bars * bw + (bars - 1) * gap;
            let x0 = if toward_right { r.x + r.w - total } else { r.x };
            let base = r.y + (r.h + full) / 2;
            let c = match (color, playing) {
                (_, false) => theme.muted,
                (Some((r, g, b)), true) => Color::rgb(*r, *g, *b),
                (None, true) => theme.accent,
            };
            for i in 0..bars {
                let lv = levels[i as usize] as f32 / 255.0;
                let h = ((full as f32) * lv).round().max(px(2.0, scale) as f32) as i32;
                p.fill_round_rect(Rect::new(x0 + i * (bw + gap), base - h, bw, h), bw / 2, c);
            }
        }
    }
}

/// Obal zmenseny na `size x size` px a zlozeny na podklad `bg` (`0xRRGGBB`), z malej pamate (posledne obaly): na `blit`.
pub fn cover_pixels(
    art: &transom_core::item::ArtImage,
    size: u32,
    bg: u32,
) -> std::rc::Rc<Vec<Color>> {
    crate::memo::art(art, size, bg)
}

/// Logo AI (vlastna kresba): Claude oranzova hviezdicka, Codex ">_" v zaoblenom stvorci.
fn paint_ai_logo(p: &mut dyn Painter, theme: &Theme, logo: AiLogo, r: Rect) {
    match logo {
        AiLogo::Claude => p.blend_mask(r, &crate::memo::spark(r.w), Color::rgb(0xD9, 0x77, 0x57)),
        AiLogo::Codex => {
            p.fill_round_rect(r, (r.w / 4).max(2), theme.foreground);
            let st = TextStyle {
                font: Font::Ui,
                size_lp: 10.5,
                bold: true,
                color: theme.background,
                align: HAlign::Center,
            };
            p.draw_text(">_", r, &st);
        }
    }
}

/// Maly obal v ostrove (zaoblene rohy maskou pozadia).
fn paint_cover(
    p: &mut dyn Painter,
    theme: &Theme,
    art: Option<&transom_core::item::ArtImage>,
    a: Rect,
    bg: Color,
    scale: f32,
) {
    match art {
        Some(img) => p.blit(a, &crate::memo::art(img, a.w as u32, rgb(bg))),
        None => p.fill_round_rect(a, px(4.0, scale), theme.placeholder),
    }
}

/// Znacka v kapsule (`n` x `n` px).
fn paint_mark(p: &mut dyn Painter, theme: &Theme, m: Mark, r: Rect) {
    match m {
        Mark::Transom => {
            let pix = crate::memo::transom_mark(r.w.max(1) as u32, theme.dark);
            p.blend_rgba(r, &pix);
        }
        Mark::Spark => p.blend_mask(r, &crate::memo::spark(r.w), Color::rgb(0xD9, 0x77, 0x57)),
        Mark::Icon(c) => {
            let st = TextStyle {
                font: Font::Icons,
                size_lp: 12.0,
                bold: false,
                color: theme.accent,
                align: HAlign::Center,
            };
            p.draw_text(c.encode_utf8(&mut [0; 4]), r, &st);
        }
    }
}

/// Text skrateny na sirku `w` so znakom "…" na konci (zmesti sa = bez zmeny).
/// Posun dlheho textu (px): rovnaky pohyb ako polozka Music (pauza, rozbeh, stala rychlost, dobeh, pauza a spat).
pub fn scroll_offset(over: i32, sc: transom_core::item::Scroll, scale: f32) -> i32 {
    let p = transom_core::marquee::Params::new(f32::from(sc.speed_lp) * scale, sc.rtl);
    transom_core::marquee::frame(u64::from(sc.ms), over, &p).offset_px
}

/// Text, ktory sa nezmesti, sa posuva (orezany na svoje miesto); kratky ostava zarovnany ako obvykle.
fn draw_scrolling(
    p: &mut dyn Painter,
    text: &str,
    st: &TextStyle,
    r: Rect,
    sc: transom_core::item::Scroll,
    scale: f32,
) {
    let full = p.measure_text(text, st).w;
    if full <= r.w {
        p.draw_text(text, r, st);
        return;
    }
    let off = scroll_offset(full - r.w, sc, scale);
    let left = TextStyle {
        align: HAlign::Left,
        ..*st
    };
    p.push_clip(r);
    p.draw_text(text, Rect::new(r.x - off, r.y, full + 2, r.h), &left);
    p.pop_clip();
}

/// Skrati text na sirku `w` s "…" na konci (cely text, ak sa zmesti).
pub fn elide(p: &mut dyn Painter, text: &str, st: &TextStyle, w: i32) -> String {
    if p.measure_text(text, st).w <= w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cand: String = chars[..mid]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
            + "…";
        if p.measure_text(&cand, st).w <= w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return String::new();
    }
    chars[..lo]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_string()
        + "…"
}

/// Maly symbol (zariadenie, subor, klavesnica) na stred slotu `r`. Sirsi znak (napr. Caps Lock) presahuje slot
/// rovnomerne do okrajov casti namiesto orezania.
fn draw_slot_glyph(p: &mut dyn Painter, g: Glyph, r: Rect, color: Color) {
    let st = TextStyle {
        font: Font::Icons,
        size_lp: DEVICE_GLYPH_LP,
        bold: false,
        color,
        align: HAlign::Center,
    };
    let mut buf = [0; 4];
    let ch: &str = glyph_char(g).encode_utf8(&mut buf);
    let w = p.measure_text(ch, &st).w.max(r.w);
    // klavesnica a ovladac maju kresbu v hornej casti znaku (sirsie nez vysoke): o 1 px nizsie, aby boli v strede ako
    // bateria vedla nich (namerane v liste)
    let dy = match g {
        Glyph::Keyboard | Glyph::Controller => (p.scale()).round().max(1.0) as i32,
        _ => 0,
    };
    p.draw_text(ch, Rect::new(r.x + (r.w - w) / 2, r.y + dy, w, r.h), &st);
}

pub fn glyph_char(g: Glyph) -> char {
    match g {
        Glyph::Search => '\u{E721}',
        Glyph::Tools => '\u{E71D}',
        Glyph::Prev => '\u{E892}',
        Glyph::Play => '\u{E768}',
        Glyph::Pause => '\u{E769}',
        Glyph::Next => '\u{E893}',
        // Segoe Fluent Icons: Volume0..Volume3 a Mute
        Glyph::Volume0 => '\u{E992}',
        Glyph::Volume1 => '\u{E993}',
        Glyph::Volume2 => '\u{E994}',
        Glyph::Volume3 => '\u{E995}',
        Glyph::VolumeMuted => '\u{E74F}',
        // Segoe Fluent Icons: Mouse, KeyboardClassic, Headset, Game, Devices
        Glyph::Mouse => '\u{E962}',
        Glyph::Keyboard => '\u{E765}',
        Glyph::Headset => '\u{E95B}',
        Glyph::Controller => '\u{E7FC}',
        Glyph::Device => '\u{E772}',
        // Segoe Fluent Icons: Page, Folder
        Glyph::File => '\u{E7C3}',
        Glyph::Folder => '\u{E8B7}',
        // Segoe Fluent Icons: Stopwatch, Brightness, Microphone, MicOff, Camera, KeyboardShortcut (Caps)
        Glyph::Timer => '\u{E916}',
        Glyph::Brightness => '\u{E706}',
        Glyph::Mic => '\u{E720}',
        Glyph::MicOff => '\u{EC54}',
        Glyph::Camera => '\u{E722}',
        Glyph::CapsLock => '\u{E84A}',
        // Segoe Fluent Icons: Package (Polica), Paste (Schranka), QuickNote (Poznamky), Picture (Snimky)
        Glyph::Shelf => '\u{E7B8}',
        Glyph::Clipboard => '\u{E77F}',
        Glyph::Note => '\u{E70B}',
        Glyph::Screenshot => '\u{E8B9}',
        // Segoe Fluent Icons: Cut (noznice)
        Glyph::Snip => '\u{E8C6}',
        // Segoe Fluent Icons: Color (paleta)
        Glyph::Picker => '\u{E790}',
        // Segoe Fluent Icons: TaskView
        Glyph::Desktops => '\u{E7C4}',
        // Segoe Fluent Icons: QuietHours (mesiac)
        Glyph::Focus => '\u{E708}',
        // Segoe Fluent Icons: Cloud
        Glyph::Weather => '\u{E753}',
        // Segoe Fluent Icons: Download
        Glyph::Download => '\u{E896}',
    }
}

/// Sirka symbolu zariadenia v casti `Device` (lp) a velkost jeho znaku.
const DEVICE_GLYPH_W: f32 = 18.0;
const DEVICE_GLYPH_LP: f32 = 13.0;
/// Medzera medzi symbolom baterie a textom urovne v casti `Device`.
const DEVICE_TEXT_GAP: f32 = 4.0;
const BOLT: char = '\u{E945}';
/// Medzera medzi symbolom suboru a jeho nazvom v casti `File`.
const FILE_TEXT_GAP: f32 = 2.0;

/// Farba zmesi `top` nad `under` s podielom `t` (0..=1). Hover/pressed, oddelovace a okraj lišty sa pocitaju ako
/// text-nad-skutocnym-pozadim, takze fungujú aj s vlastnou farbou pozadia a pri priehladnosti (v dvojprechodovom
/// kresleni sa z nich stane skutocne polopriehladna vrstva, nie nepriehladna zaplata).
pub fn mix(top: Color, under: Color, t: f32) -> Color {
    let m = |a: u8, b: u8| {
        (a as f32 * t + b as f32 * (1.0 - t))
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgb(m(top.r, under.r), m(top.g, under.g), m(top.b, under.b))
}

/// Podiel textu v farbe prechodu pod mysou, stlaceneho prvku, oddelovaca a okraja.
pub const HOVER_MIX: f32 = 0.10;
pub const PRESSED_MIX: f32 = 0.18;
pub const DIVIDER_MIX: f32 = 0.16;
pub const EDGE_MIX: f32 = 0.12;
/// Polomer rohov hover/pressed ciepu v lp.
pub const CHIP_RADIUS_LP: f32 = 4.0;
/// Vyska oddelovaca medzi polozkami v lp.
const DIVIDER_H_LP: f32 = 14.0;

pub fn hover_color(theme: &Theme, under: Color) -> Color {
    mix(theme.foreground, under, HOVER_MIX)
}

pub fn pressed_color(theme: &Theme, under: Color) -> Color {
    mix(theme.foreground, under, PRESSED_MIX)
}

fn text_style(theme: &Theme, item: ItemId, tone: Tone) -> TextStyle {
    TextStyle {
        // cela lista: klasicke Segoe UI s rovnako sirokymi cislicami (hodiny, percenta a odpocty sa prekresluju na mieste)
        // a rovnakym vzhladom v nepriehladnom aj priehladnom rezime (pozri `fonts`)
        font: Font::Tabular,
        size_lp: if item == ItemId::Clock {
            CLOCK_SIZE_LP
        } else {
            TEXT_SIZE_LP
        },
        // cela lista tenkym pismom (aj hodiny; vacsie pismo ich odlisi dost)
        bold: false,
        color: match tone {
            Tone::Normal => theme.foreground,
            Tone::Muted => theme.muted,
        },
        align: HAlign::Left,
    }
}

fn battery_width(scale: f32, charging: bool) -> i32 {
    ((BATT_W + BATT_TIP_W + if charging { BOLT_W } else { 0.0 }) * scale).round() as i32
}

fn px(v: f32, scale: f32) -> i32 {
    (v * scale).round() as i32
}

/// Nakresli symbol baterie: obrys, vystupok a vyplnenie podla percenta (pri <= 15 % farbou `low`).
fn draw_battery(
    p: &mut dyn Painter,
    area: Rect,
    percent: u8,
    charging: bool,
    fg: Color,
    bg: Color,
    low: Color,
) {
    let s = p.scale();
    let mut x = area.x;
    if charging {
        let st = TextStyle {
            font: Font::Icons,
            size_lp: 12.0,
            bold: false,
            color: fg,
            align: HAlign::Center,
        };
        p.draw_text(
            BOLT.encode_utf8(&mut [0; 4]),
            Rect::new(x, area.y, px(BOLT_W, s), area.h),
            &st,
        );
        x += px(BOLT_W, s);
    }
    let (w, h) = (px(BATT_W, s), px(BATT_H, s));
    let y = area.y + (area.h - h) / 2;
    let t = (s.round() as i32).max(1); // hrubka obrysu
    p.fill_rect(Rect::new(x, y, w, h), fg);
    p.fill_rect(Rect::new(x + t, y + t, w - 2 * t, h - 2 * t), bg);
    let (tw, th) = (px(BATT_TIP_W, s), px(BATT_TIP_H, s));
    p.fill_rect(Rect::new(x + w, y + (h - th) / 2, tw, th), fg);
    let inner_w = w - 4 * t;
    let fill = (inner_w * i32::from(percent.min(100)) + 50) / 100;
    if fill > 0 {
        let c = if percent <= 15 && !charging { low } else { fg };
        p.fill_rect(Rect::new(x + 2 * t, y + 2 * t, fill, h - 4 * t), c);
    }
}

/// Stav interakcie mysou na tejto liste (hover a stlacenie platia len pre klikatelne, povolene casti).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interaction {
    pub hovered: Option<(ItemId, usize)>,
    pub pressed: Option<(ItemId, usize)>,
    /// Posun obsahu (ochrana OLED) v px: vodorovny sa uplatni na rozlozenie (vratene `placed` su posunute, takze
    /// hit-testing a oblasti sedia), zvisly len pri kresleni. Pozadie ostava na mieste.
    pub shift: (i32, i32),
    /// Bez spodneho okraja a oddelovacov (ochrana OLED).
    pub no_lines: bool,
    /// Usek spodneho okraja bez ciary (x od, x do v px): pod otvorenym vybezkom lista plynule prechadza do neho.
    pub gap: Option<(i32, i32)>,
}

/// Painter posunuty zvisle o `dy` (obsah lisaty pri posune obsahu).
struct ShiftY<'a> {
    inner: &'a mut dyn Painter,
    dy: i32,
}

impl ShiftY<'_> {
    fn r(&self, r: Rect) -> Rect {
        Rect::new(r.x, r.y + self.dy, r.w, r.h)
    }
}

impl Painter for ShiftY<'_> {
    fn size(&self) -> Size {
        self.inner.size()
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
            .map(|c| Rect::new(c.x, c.y - self.dy, c.w, c.h))
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

/// Box vektorovej ikony medii v lp (14/21/28 px pri 100/150/200 %).
pub const MEDIA_ICON_LP: f32 = 14.0;
/// Priemer kruhoveho indikatora v lp (16/24/32 px pri 100/150/200 %).
pub const RING_LP: f32 = 16.0;
/// Medzera za textom kruhu v lp.
const RING_TEXT_GAP_LP: f32 = 6.0;
/// Okraj obalu v lp: obal = vyska lisaty - 2 * okraj (22 lp pri vyske 28 lp), vnutri hover obdlznika.
pub const ART_INSET_LP: f32 = 3.0;

fn media_icon(g: Glyph) -> Option<MediaIcon> {
    match g {
        Glyph::Prev => Some(MediaIcon::Prev),
        Glyph::Play => Some(MediaIcon::Play),
        Glyph::Pause => Some(MediaIcon::Pause),
        Glyph::Next => Some(MediaIcon::Next),
        Glyph::Search
        | Glyph::Tools
        | Glyph::Volume0
        | Glyph::Volume1
        | Glyph::Volume2
        | Glyph::Volume3
        | Glyph::VolumeMuted
        | Glyph::Mouse
        | Glyph::Keyboard
        | Glyph::Headset
        | Glyph::Controller
        | Glyph::Device
        | Glyph::File
        | Glyph::Folder
        | Glyph::Timer
        | Glyph::Brightness
        | Glyph::Mic
        | Glyph::MicOff
        | Glyph::Camera
        | Glyph::CapsLock
        | Glyph::Shelf
        | Glyph::Clipboard
        | Glyph::Note
        | Glyph::Screenshot
        | Glyph::Snip
        | Glyph::Picker
        | Glyph::Desktops
        | Glyph::Focus
        | Glyph::Weather
        | Glyph::Download => None,
    }
}

fn rgb(c: Color) -> u32 {
    (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}

/// Aktualny posun posuvneho textu (marquee): ktora cast a o kolko px je posunuta dolava.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarqueeDraw {
    pub item: ItemId,
    pub part: usize,
    pub offset_px: i32,
}

/// Cast textu, ktora sa nezmesti do svojho miesta (kandidat na posuv): miesto textu a o kolko px je sirsi.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextOverflow {
    pub item: ItemId,
    pub part: usize,
    /// Miesto, do ktoreho sa text kresli (px listy; uz bez okrajov textu) - len tato oblast sa pri posune prekresluje.
    pub x: i32,
    pub w: i32,
    pub overflow_px: i32,
}

/// Vysledok kreslenia: rozlozenie a texty, ktore pretecuju.
#[derive(Clone, Debug, Default)]
pub struct RenderInfo {
    pub placed: Vec<Placed>,
    pub overflow: Vec<TextOverflow>,
}

/// Je cast kandidatom na posuv? Len nazov skladby (flexibilny text v polozke Media).
fn is_ticker(item: ItemId, part: &Part) -> bool {
    item == ItemId::Media && matches!(part, Part::Text { flex: Some(_), .. })
}

/// Nakresli celu listu: nepriehladne pozadie, potom polozky. Vrati rozlozenie (pre hit-testing v aplikacii).
pub fn render_bar(
    p: &mut dyn Painter,
    theme: &Theme,
    items: &[StatusItem],
    ui: Interaction,
) -> Vec<Placed> {
    render_bar_with(p, theme, items, ui, None).placed
}

/// Ako [`render_bar`], plus posuvny text: ak `marquee` zodpoveda casti, ktora pretieka, nakresli sa posunuta a
/// orezana na svoje miesto; inak (zmesti sa, ina cast, bez animacie) staticky s "...". Vrati aj pretecenie.
pub fn render_bar_with(
    p: &mut dyn Painter,
    theme: &Theme,
    items: &[StatusItem],
    ui: Interaction,
    marquee: Option<MarqueeDraw>,
) -> RenderInfo {
    let size = p.size();
    let scale = p.scale();
    p.fill_rect(Rect::new(0, 0, size.w, size.h), theme.background);
    // vlas na spodnej hrane: lista ma zretelny okraj voci plochu (v rezerve lišty, vyska sa nemeni)
    let hair = px(1.0, scale).max(1);
    if !ui.no_lines {
        let c = mix(theme.foreground, theme.background, EDGE_MIX);
        match ui.gap {
            Some((a, b)) => {
                let (a, b) = (a.clamp(0, size.w), b.clamp(0, size.w));
                p.fill_rect(Rect::new(0, size.h - hair, a, hair), c);
                p.fill_rect(Rect::new(b, size.h - hair, size.w - b, hair), c);
            }
            None => p.fill_rect(Rect::new(0, size.h - hair, size.w, hair), c),
        }
    }
    let metrics = Metrics::default();
    let placed = {
        let mut measure = |part: &Part| -> i32 {
            match part {
                Part::Text { text, tone, .. } => {
                    // velkost pisma zavisi od polozky; hodiny su jedine s pevnym textom vacsim pismom
                    let item = items
                        .iter()
                        .find(|it| it.parts.iter().any(|q| std::ptr::eq(q, part)))
                        .map(|it| it.id)
                        .unwrap_or(ItemId::Media);
                    p.measure_text(text, &text_style(theme, item, *tone)).w
                }
                Part::Battery { charging, .. } => battery_width(scale, *charging),
                Part::Ring {
                    label,
                    muted,
                    alert,
                    ..
                } => {
                    if label.is_empty() {
                        0
                    } else {
                        let tone = if *muted { Tone::Muted } else { Tone::Normal };
                        let mut st = text_style(theme, ItemId::Ai, tone);
                        st.bold = *alert;
                        p.measure_text(label, &st).w + px(RING_TEXT_GAP_LP, scale)
                    }
                }
                Part::Device {
                    charging,
                    muted,
                    label,
                    ..
                } => {
                    let tone = if *muted { Tone::Muted } else { Tone::Normal };
                    px(DEVICE_GLYPH_W, scale)
                        + battery_width(scale, *charging)
                        + px(DEVICE_TEXT_GAP, scale)
                        + p.measure_text(label, &text_style(theme, ItemId::Peripherals, tone))
                            .w
                }
                // odlozene subory: vycnelok (jazycek s poctom), nie ikona
                Part::File {
                    glyph: Glyph::Shelf,
                    label,
                    ..
                } => pill_width(p, theme, label, scale),
                Part::File { muted, label, .. } => {
                    let tone = if *muted { Tone::Muted } else { Tone::Normal };
                    px(DEVICE_GLYPH_W, scale)
                        + px(FILE_TEXT_GAP, scale)
                        + p.measure_text(label, &text_style(theme, ItemId::Shelf, tone))
                            .w
                }
                Part::Island { .. } => island_width(p, theme, part, scale).1,
                Part::Icon { .. } | Part::Art { .. } | Part::Divider => 0,
            }
        };
        layout(items, size.w, scale, &metrics, &mut measure)
    };
    // posun obsahu (ochrana OLED): vodorovne cez rozlozenie (v ramci lisaty), zvisle len pri kresleni
    let mut placed = placed;
    if ui.shift.0 != 0 && !placed.is_empty() {
        let lo = placed.iter().map(|q| q.x).min().unwrap_or(0).max(0);
        let hi = placed.iter().map(|q| q.x + q.w).max().unwrap_or(0);
        let dx = ui.shift.0.clamp(-lo, (size.w - hi).max(0));
        for q in placed.iter_mut() {
            q.x += dx;
        }
    }
    let mut shifted = ShiftY {
        inner: &mut *p,
        dy: ui.shift.1,
    };
    let p: &mut dyn Painter = &mut shifted;
    let pad = px(metrics.text_pad_lp as f32, scale);
    let inset = px(HOVER_INSET_LP, scale);
    let mut overflow = Vec::new();
    let dirty = p.clip_box();
    // jemne oddelovace medzi susednymi polozkami tej istej skupiny (vlavo / stred / vpravo)
    if !ui.no_lines {
        let mut spans: Vec<(ItemId, i32, i32)> = Vec::new();
        for pl in &placed {
            match spans.iter_mut().find(|(id, _, _)| *id == pl.item) {
                Some(s) => {
                    s.1 = s.1.min(pl.x);
                    s.2 = s.2.max(pl.x + pl.w);
                }
                None => spans.push((pl.item, pl.x, pl.x + pl.w)),
            }
        }
        spans.sort_by_key(|s| s.1);
        let dh = px(DIVIDER_H_LP, scale);
        let dw = px(1.0, scale).max(1);
        let color = mix(theme.foreground, theme.background, DIVIDER_MIX);
        for pair in spans.windows(2) {
            let side = |id: ItemId| items.iter().find(|i| i.id == id).map(|i| i.side);
            let (a, b) = (pair[0], pair[1]);
            let gap = b.1 - a.2;
            if side(a.0) != side(b.0) || gap < px(4.0, scale) {
                continue;
            }
            let r = Rect::new(a.2 + (gap - dw) / 2, (size.h - hair - dh) / 2, dw, dh);
            let shown = dirty.is_none_or(|d| r.x < d.x + d.w && d.x < r.x + r.w);
            if shown {
                p.fill_rect(r, color);
            }
        }
    }
    for pl in &placed {
        let Some(item) = items.iter().find(|i| i.id == pl.item) else {
            continue;
        };
        let Some(part) = item.parts.get(pl.part) else {
            continue;
        };
        let rect = Rect::new(pl.x, 0, pl.w, size.h);
        // cast mimo orezavacej oblasti sa nekresli (pretecenie textu sa vsak hlasi vzdy: animacia ho potrebuje)
        let visible = dirty.is_none_or(|d| {
            rect.x < d.x + d.w
                && d.x < rect.x + rect.w
                && rect.y < d.y + d.h
                && d.y < rect.y + rect.h
        });
        if !visible {
            if let Part::Text { text, .. } = part {
                if is_ticker(pl.item, part) {
                    let st = text_style(theme, pl.item, Tone::Normal);
                    let ov = transom_core::marquee::overflow_px(
                        p.measure_text(text, &st).w,
                        (pl.w - 2 * pad).max(0),
                    );
                    if ov > 0 {
                        overflow.push(TextOverflow {
                            item: pl.item,
                            part: pl.part,
                            x: pl.x + pad,
                            w: (pl.w - 2 * pad).max(0),
                            overflow_px: ov,
                        });
                    }
                }
            }
            continue;
        }
        let me = Some((pl.item, pl.part));
        let active = part.action().is_some();
        let state_bg = if active && ui.pressed == me {
            Some(pressed_color(theme, theme.background))
        } else if active && ui.hovered == me {
            Some(hover_color(theme, theme.background))
        } else {
            None
        };
        // ostrov: hover a stlacenie len na kapsule s logom (nie cez hudbu a AI po stranach) - kresli ju vetva ostrova
        let state_bg = if matches!(part, Part::Island { .. }) {
            None
        } else {
            state_bg
        };
        let island_state = if matches!(part, Part::Island { .. }) && active {
            if ui.pressed == me {
                Some(pressed_color(theme, theme.background))
            } else if ui.hovered == me {
                Some(hover_color(theme, theme.background))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(c) = state_bg {
            // zaoblený čip (AA) nad skutočným pozadím
            let (cw, ch) = (pl.w.max(1), (size.h - 2 * inset - hair).max(1));
            let cov = crate::memo::round_rect(cw, ch, px(CHIP_RADIUS_LP, scale));
            p.blit(
                Rect::new(pl.x, inset, cw, ch),
                &icons::shade(&cov, c, theme.background),
            );
        }
        let bg = state_bg.unwrap_or(theme.background);
        match part {
            Part::Icon { glyph, enabled, .. } => {
                let fg = if *enabled {
                    theme.foreground
                } else {
                    theme.disabled
                };
                if let Some(mi) = media_icon(*glyph) {
                    let n = px(MEDIA_ICON_LP, scale).clamp(1, pl.w.max(1));
                    let r = Rect::new(pl.x + (pl.w - n) / 2, (size.h - n) / 2, n, n);
                    p.blit(r, &icons::shade(&crate::memo::media_icon(mi, n), fg, bg));
                } else {
                    let st = TextStyle {
                        font: Font::Icons,
                        size_lp: ICON_SIZE_LP,
                        bold: false,
                        color: fg,
                        align: HAlign::Center,
                    };
                    p.draw_text(glyph_char(*glyph).encode_utf8(&mut [0; 4]), rect, &st);
                }
            }
            Part::Ring {
                fill,
                ccw,
                muted,
                alert,
                label,
                ..
            } => {
                // kruh v lavom stvorci (sirka tlacidla), text za nim
                let slot = px(28.0, scale).min(pl.w);
                let n = px(RING_LP, scale).clamp(1, slot.max(1));
                let r = Rect::new(pl.x + (slot - n) / 2, (size.h - n) / 2, n, n);
                if !label.is_empty() {
                    let tone = if *muted { Tone::Muted } else { Tone::Normal };
                    let mut st = text_style(theme, ItemId::Ai, tone);
                    st.bold = *alert;
                    p.draw_text(
                        label,
                        Rect::new(pl.x + slot, 0, (pl.w - slot).max(0), size.h),
                        &st,
                    );
                }
                let arc_color = if *alert {
                    theme.low
                } else if *muted {
                    theme.muted
                } else {
                    theme.accent
                };
                let cov = crate::memo::ring(n, *fill, *ccw);
                let (track, arc) = (&cov.0, &cov.1);
                let px_: Vec<Color> = icons::shade(track, theme.disabled, bg)
                    .into_iter()
                    .zip(arc.iter())
                    .map(|(c, &a)| icons::mix(c, arc_color, a))
                    .collect();
                p.blit(r, &px_);
            }
            Part::Art { art, .. } => {
                let n = (size.h - 2 * px(ART_INSET_LP, scale)).clamp(1, pl.w.max(1));
                let r = Rect::new(pl.x + (pl.w - n) / 2, (size.h - n) / 2, n, n);
                match art {
                    Some(a) => p.blit(r, &crate::memo::art(a, n as u32, rgb(bg))),
                    None => p.fill_rect(r, theme.placeholder),
                }
            }
            Part::Text { text, tone, .. } => {
                let st = text_style(theme, pl.item, *tone);
                let inner = Rect::new(pl.x + pad, 0, (pl.w - 2 * pad).max(0), size.h);
                let mut scrolled = false;
                if is_ticker(pl.item, part) {
                    let ov =
                        transom_core::marquee::overflow_px(p.measure_text(text, &st).w, inner.w);
                    if ov > 0 {
                        overflow.push(TextOverflow {
                            item: pl.item,
                            part: pl.part,
                            x: inner.x,
                            w: inner.w,
                            overflow_px: ov,
                        });
                        if let Some(m) = marquee.filter(|m| m.item == pl.item && m.part == pl.part)
                        {
                            p.draw_text_scrolled(text, inner, &st, m.offset_px.clamp(0, ov));
                            scrolled = true;
                        }
                    }
                }
                if !scrolled {
                    // pruzny text skrateny pre nedostatok miesta (napr. oznam pri plnej liste): "..." na konci, nie
                    // orezanie uprostred slova
                    // (nazov skladby sa kresli cely: posuvny text ho posuva, staticky je orezany)
                    if is_ticker(pl.item, part) {
                        p.draw_text(text, inner, &st);
                    } else {
                        let shown = elide(p, text, &st, inner.w);
                        p.draw_text(&shown, inner, &st);
                    }
                }
            }
            Part::Divider => {
                // tenka zvisla ciara (polovica vysky listy) farbou textu stlmenou k pozadiu
                let lw = (scale.round() as i32).max(1);
                let lh = size.h / 2;
                p.fill_rect(
                    Rect::new(pl.x + (pl.w - lw) / 2, (size.h - lh) / 2, lw, lh),
                    mix(theme.foreground, bg, 0.35),
                );
            }
            Part::Battery { percent, charging } => {
                let inner = Rect::new(pl.x + pad, 0, (pl.w - 2 * pad).max(0), size.h);
                draw_battery(
                    p,
                    inner,
                    *percent,
                    *charging,
                    theme.foreground,
                    bg,
                    theme.low,
                );
            }
            Part::Device {
                glyph,
                percent,
                charging,
                muted,
                low,
                label,
                ..
            } => {
                let tone = if *muted { Tone::Muted } else { Tone::Normal };
                let fg = if *muted {
                    theme.muted
                } else {
                    theme.foreground
                };
                let mut x = pl.x + pad;
                let gw = px(DEVICE_GLYPH_W, scale);
                draw_slot_glyph(p, *glyph, Rect::new(x, 0, gw, size.h), fg);
                x += gw;
                let bw = battery_width(scale, *charging);
                // `draw_battery` kresli farbou `low` pod 15 %; ta je cervena len pri nizkej urovni zariadenia, inak
                // ostava farba popredia (stav "nizka" nesie aj pristupny nazov a panel, nie len farba)
                let pct = percent.unwrap_or(0);
                let low_color = if *low { theme.low } else { fg };
                draw_battery(
                    p,
                    Rect::new(x, 0, bw, size.h),
                    pct,
                    *charging,
                    fg,
                    bg,
                    low_color,
                );
                x += bw + px(DEVICE_TEXT_GAP, scale);
                let st = text_style(theme, ItemId::Peripherals, tone);
                p.draw_text(
                    label,
                    Rect::new(x, 0, (pl.x + pl.w - pad - x).max(0), size.h),
                    &st,
                );
            }
            Part::Island {
                left,
                mark,
                label,
                badge,
                right,
                ..
            } => {
                // ostrov: strany rovnako siroke, kapsula presne v strede casti
                let (side, total) = island_width(p, theme, part, scale);
                let x0 = pl.x + (pl.w - total) / 2;
                let gap = if side > 0 {
                    px(ISLAND_SIDE_GAP, scale)
                } else {
                    0
                };
                let pw = island_pill_width(p, theme, label, badge, scale);
                let ph = px(ISLAND_PILL_H, scale);
                let pill = Rect::new(x0 + side + gap, (size.h - ph) / 2, pw, ph);
                let row = |x: i32| Rect::new(x, 0, side, size.h);
                paint_seg(p, theme, left, row(x0), true, bg, scale);
                paint_seg(p, theme, right, row(pill.x + pw + gap), false, bg, scale);
                // hover / stlacenie: svetlejsia kapsula (len ona)
                let pill_fill = match island_state {
                    Some(_) if ui.pressed == me => mix(theme.foreground, bg, 0.30),
                    Some(_) => mix(theme.foreground, bg, 0.24),
                    None => mix(theme.foreground, bg, 0.16),
                };
                p.fill_round_rect(pill, ph / 2, pill_fill);
                let m = px(ISLAND_MARK, scale);
                let mut x = pill.x + px(ISLAND_PILL_PAD, scale);
                paint_mark(p, theme, *mark, Rect::new(x, pill.y + (ph - m) / 2, m, m));
                x += m;
                if !label.is_empty() {
                    x += px(ISLAND_GAP, scale);
                    let st = island_text(theme, false);
                    let tw = p.measure_text(label, &st).w;
                    p.draw_text(label, Rect::new(x, pill.y, tw, ph), &st);
                    x += tw;
                }
                if !badge.is_empty() {
                    x += px(ISLAND_GAP, scale);
                    let bw = badge_width(p, theme, badge, scale);
                    let bh = px(16.0, scale);
                    let b = Rect::new(x, pill.y + (ph - bh) / 2, bw, bh);
                    p.fill_round_rect(b, bh / 2, theme.accent);
                    let st = TextStyle {
                        size_lp: 10.0,
                        color: theme.on_accent,
                        align: HAlign::Center,
                        ..island_text(theme, false)
                    };
                    p.draw_text(badge, b, &st);
                }
            }
            Part::File {
                glyph: Glyph::Shelf,
                label,
                ..
            } => {
                // vycnelok odlozenych suborov: zaoblena "kapsula" na stred casti, v nej pocet (prazdne = tenky jazycek)
                let w = pill_width(p, theme, label, scale);
                let h = if label.is_empty() {
                    px(7.0, scale).max(4)
                } else {
                    px(22.0, scale)
                };
                let r = Rect::new(
                    pl.x + pad + (pl.w - 2 * pad - w) / 2,
                    (size.h - h) / 2,
                    w,
                    h,
                );
                p.fill_round_rect(r, h / 2, mix(theme.foreground, bg, 0.2));
                if !label.is_empty() {
                    let st = TextStyle {
                        bold: true,
                        size_lp: 12.0,
                        align: HAlign::Center,
                        ..text_style(theme, ItemId::Shelf, Tone::Normal)
                    };
                    p.draw_text(label, r, &st);
                }
            }
            Part::File {
                glyph,
                muted,
                label,
                ..
            } => {
                let tone = if *muted { Tone::Muted } else { Tone::Normal };
                let fg = if *muted {
                    theme.muted
                } else {
                    theme.foreground
                };
                let mut x = pl.x + pad;
                let gw = px(DEVICE_GLYPH_W, scale);
                draw_slot_glyph(p, *glyph, Rect::new(x, 0, gw, size.h), fg);
                x += gw + px(FILE_TEXT_GAP, scale);
                p.draw_text(
                    label,
                    Rect::new(x, 0, (pl.x + pl.w - pad - x).max(0), size.h),
                    &text_style(theme, ItemId::Shelf, tone),
                );
            }
        }
    }
    RenderInfo { placed, overflow }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::painter::{Size, TextStyle};
    use transom_core::item::{Action, ArtImage, Side};

    #[derive(Default)]
    struct Rec {
        size: Size,
        fills: Vec<(Rect, Color)>,
        texts: Vec<(String, Rect, TextStyle)>,
        /// (text, clip, posun) z `draw_text_scrolled`
        scrolled: Vec<(String, Rect, i32)>,
        /// Orezavacia oblast, ktoru ohlasi `clip_box` (len v testoch).
        clip: Option<Rect>,
        blits: Vec<(Rect, Vec<Color>)>,
    }

    impl Painter for Rec {
        fn size(&self) -> Size {
            self.size
        }
        fn scale(&self) -> f32 {
            1.0
        }
        fn fill_rect(&mut self, rect: Rect, color: Color) {
            self.fills.push((rect, color));
        }
        fn measure_text(&mut self, t: &str, _: &TextStyle) -> Size {
            Size {
                w: 7 * t.chars().count() as i32,
                h: 16,
            }
        }
        fn draw_text(&mut self, text: &str, rect: Rect, style: &TextStyle) {
            self.texts.push((text.to_string(), rect, *style));
        }
        fn clip_box(&self) -> Option<Rect> {
            self.clip
        }
        fn draw_text_scrolled(&mut self, text: &str, clip: Rect, _: &TextStyle, offset_px: i32) {
            self.scrolled.push((text.to_string(), clip, offset_px));
        }
        fn blit(&mut self, rect: Rect, pixels: &[Color]) {
            assert_eq!(
                pixels.len(),
                (rect.w * rect.h) as usize,
                "blit: zly pocet pixelov"
            );
            self.blits.push((rect, pixels.to_vec()));
        }
    }

    /// Hover/pressed cip: blit vysky `size.h - 2 * inset - vlas` (28 - 6 - 1 = 21 px pri 100 %).
    fn chip_of(p: &Rec) -> Option<(Rect, Vec<Color>)> {
        p.blits
            .iter()
            .find(|(r, _)| r.h == 21 && r.w >= 20)
            .map(|(r, v)| (*r, v.clone()))
    }

    fn none() -> Interaction {
        Interaction::default()
    }

    fn hover(id: ItemId, part: usize) -> Interaction {
        Interaction {
            hovered: Some((id, part)),
            pressed: None,
            ..Default::default()
        }
    }

    fn clock(t: &str) -> StatusItem {
        StatusItem::new(ItemId::Clock, Side::Right, vec![Part::text(t)])
    }

    fn media_item(title: &str) -> StatusItem {
        StatusItem::new(
            ItemId::Media,
            Side::Left,
            vec![
                Part::button(
                    transom_core::item::Glyph::Play,
                    Action::MediaToggle,
                    true,
                    "p",
                ),
                Part::Text {
                    text: title.into(),
                    flex: Some(transom_core::item::Flex {
                        min_lp: 60,
                        max_lp: 100,
                    }),
                    tone: transom_core::item::Tone::Normal,
                    action: None,
                },
            ],
        )
    }

    fn draw_media(title: &str, marquee: Option<MarqueeDraw>, w: i32) -> (Rec, RenderInfo) {
        let mut p = Rec {
            size: Size { w, h: 28 },
            ..Default::default()
        };
        let info = render_bar_with(
            &mut p,
            &Theme::dark(),
            &[media_item(title), clock("14:05")],
            none(),
            marquee,
        );
        (p, info)
    }

    #[test]
    fn a_long_island_title_moves_like_the_music_item() {
        use transom_core::item::Scroll;
        let sc = |ms| Scroll {
            ms,
            speed_lp: 35,
            rtl: false,
        };
        // kratky text sa nehybe, dlhy najprv stoji a potom ide plynulo
        assert_eq!(scroll_offset(0, sc(5000), 1.0), 0);
        assert_eq!(scroll_offset(100, sc(0), 1.0), 0);
        let p = transom_core::marquee::Params::new(35.0, false);
        for ms in (0..20_000).step_by(37) {
            assert_eq!(
                scroll_offset(100, sc(ms), 1.0),
                transom_core::marquee::frame(u64::from(ms), 100, &p).offset_px
            );
        }
        // dlhsi text = dlhsi pohyb (rovnaka rychlost)
        let reach = |over: i32| {
            (0..30_000)
                .step_by(10)
                .find(|&ms| scroll_offset(over, sc(ms), 1.0) == over)
        };
        assert!(reach(200) > reach(60));
    }

    #[test]
    fn long_title_reports_overflow_and_scrolls_clipped_to_its_text_region() {
        let long = "A very long song title that cannot fit";
        // staticky (bez animacie): obycajny text s "...", pretecenie sa vsak hlasi
        let (p, info) = draw_media(long, None, 800);
        assert!(p.scrolled.is_empty());
        assert!(p.texts.iter().any(|t| t.0 == long));
        let ov = info.overflow[0];
        assert_eq!((ov.item, ov.part), (ItemId::Media, 1));
        let text_part = info
            .placed
            .iter()
            .find(|q| q.item == ItemId::Media && q.part == 1)
            .unwrap();
        // miesto textu = cast bez okrajov textu (6 lp)
        assert_eq!((ov.x, ov.w), (text_part.x + 6, text_part.w - 12));
        assert_eq!(ov.overflow_px, 7 * long.chars().count() as i32 - ov.w);
        // animovany: nakresleny posunuty a orezany presne na svoje miesto; ostatne casti stoja
        let m = MarqueeDraw {
            item: ItemId::Media,
            part: 1,
            offset_px: 25,
        };
        let (p, info2) = draw_media(long, Some(m), 800);
        assert_eq!(p.scrolled.len(), 1);
        let (txt, clip, off) = &p.scrolled[0];
        assert_eq!((txt.as_str(), *off), (long, 25));
        assert_eq!((clip.x, clip.w), (ov.x, ov.w), "clip = presne miesto textu");
        assert!(
            !p.texts.iter().any(|t| t.0 == long),
            "nekresli sa druhy raz staticky"
        );
        assert!(p.texts.iter().any(|t| t.0 == "14:05"), "hodiny ostavaju");
        assert_eq!(
            info2.placed, info.placed,
            "posun nemeni rozlozenie (tlacidla a obal stoja)"
        );
        // posun sa orezava na rozsah 0..=pretecenie
        let big = MarqueeDraw {
            offset_px: 99_999,
            ..m
        };
        let (p, _) = draw_media(long, Some(big), 800);
        assert_eq!(p.scrolled[0].2, ov.overflow_px);
        let neg = MarqueeDraw {
            offset_px: -50,
            ..m
        };
        assert_eq!(draw_media(long, Some(neg), 800).0.scrolled[0].2, 0);
    }

    #[test]
    fn parts_outside_the_clip_box_are_skipped_but_overflow_is_still_reported() {
        let long = "A very long song title that cannot fit";
        let mut p = Rec {
            size: Size { w: 800, h: 28 },
            ..Default::default()
        };
        let all = render_bar_with(
            &mut p,
            &Theme::dark(),
            &[media_item(long), clock("14:05")],
            none(),
            None,
        );
        let title = *all
            .overflow
            .iter()
            .find(|o| o.item == ItemId::Media)
            .unwrap();
        // neplatny je len usek hodin (sekundovy tik): nic z media sa nekresli, pretecenie ale ostava (aby sa
        // animacia nezrusila), hodiny sa nakreslia
        let clock_part = all.placed.iter().find(|q| q.item == ItemId::Clock).unwrap();
        let mut p = Rec {
            size: Size { w: 800, h: 28 },
            clip: Some(Rect::new(clock_part.x, 0, clock_part.w, 28)),
            ..Default::default()
        };
        let info = render_bar_with(
            &mut p,
            &Theme::dark(),
            &[media_item(long), clock("14:05")],
            none(),
            None,
        );
        assert_eq!(
            info.overflow,
            vec![title],
            "geometria posuvneho textu sa nestraca"
        );
        assert!(p.texts.iter().any(|t| t.0 == "14:05"));
        assert!(
            !p.texts.iter().any(|t| t.0 == long),
            "mimo orezu sa text nekresli"
        );
        // neplatny je len usek textu (snimok animacie): kresli sa iba on, tlacidla a hodiny nie
        let m = MarqueeDraw {
            item: ItemId::Media,
            part: 1,
            offset_px: 12,
        };
        let mut p = Rec {
            size: Size { w: 800, h: 28 },
            clip: Some(Rect::new(title.x, 0, title.w, 28)),
            ..Default::default()
        };
        let info = render_bar_with(
            &mut p,
            &Theme::dark(),
            &[media_item(long), clock("14:05")],
            none(),
            Some(m),
        );
        assert_eq!(p.scrolled.len(), 1);
        assert!(!p.texts.iter().any(|t| t.0 == "14:05"), "hodiny stoja");
        assert_eq!(info.overflow, vec![title]);
        // bez orezu (nezname) sa kresli vsetko
        let mut p = Rec {
            size: Size { w: 800, h: 28 },
            ..Default::default()
        };
        render_bar_with(
            &mut p,
            &Theme::dark(),
            &[media_item(long), clock("14:05")],
            none(),
            None,
        );
        assert!(p.texts.iter().any(|t| t.0 == "14:05") && p.texts.iter().any(|t| t.0 == long));
    }

    #[test]
    fn short_title_is_never_scrolled_even_if_marquee_requested() {
        let m = MarqueeDraw {
            item: ItemId::Media,
            part: 1,
            offset_px: 10,
        };
        let (p, info) = draw_media("Short", Some(m), 800);
        assert!(info.overflow.is_empty());
        assert!(p.scrolled.is_empty(), "text, ktory sa zmesti, sa nehybe");
        assert!(p.texts.iter().any(|t| t.0 == "Short"));
    }

    #[test]
    fn marquee_applies_only_to_the_requested_part() {
        // zhoda len podla (polozka, cast): ina cast alebo ina polozka sa neposuva
        let long = "A very long song title that cannot fit";
        for m in [
            MarqueeDraw {
                item: ItemId::Media,
                part: 0,
                offset_px: 9,
            },
            MarqueeDraw {
                item: ItemId::Clock,
                part: 1,
                offset_px: 9,
            },
        ] {
            let (p, info) = draw_media(long, Some(m), 800);
            assert!(p.scrolled.is_empty());
            assert_eq!(info.overflow.len(), 1);
        }
    }

    #[test]
    fn fills_whole_surface_first_then_draws_clock_at_right_edge() {
        let mut p = Rec {
            size: Size { w: 2560, h: 28 },
            ..Default::default()
        };
        let t = Theme::dark();
        let placed = render_bar(&mut p, &t, &[clock("14:05")], none());
        // cele pozadie, potom 1 px vlas na spodnej hrane (text nad pozadim, nie pevna farba)
        assert_eq!(
            p.fills,
            vec![
                (Rect::new(0, 0, 2560, 28), t.background),
                (
                    Rect::new(0, 27, 2560, 1),
                    mix(t.foreground, t.background, EDGE_MIX)
                ),
            ]
        );
        assert_eq!(p.texts.len(), 1);
        let (txt, rect, st) = &p.texts[0];
        assert_eq!(txt, "14:05");
        assert_eq!(st.color, t.foreground);
        assert_eq!(st.size_lp, CLOCK_SIZE_LP);
        assert_eq!(st.font, Font::Tabular, "hodiny: rovnako siroke cislice");
        // vpravo: koniec textu + okraj textu (6) + okraj lisaty (4) = sirka
        assert_eq!(rect.x + rect.w, 2560 - 4 - 6);
        assert_eq!(placed.len(), 1);
    }

    #[test]
    fn no_items_draws_only_background() {
        let mut p = Rec {
            size: Size { w: 100, h: 28 },
            ..Default::default()
        };
        render_bar(&mut p, &Theme::light(), &[], none());
        assert_eq!(p.fills.len(), 2, "pozadie + vlas");
        assert!(p.texts.is_empty());
    }

    #[test]
    fn theme_colors_are_used() {
        let mut p = Rec {
            size: Size { w: 200, h: 28 },
            ..Default::default()
        };
        render_bar(&mut p, &Theme::light(), &[clock("x")], none());
        assert_eq!(p.fills[0].1, Theme::light().background);
        assert_eq!(p.texts[0].2.color, Theme::light().foreground);
    }

    #[test]
    fn hover_highlights_only_clickable_part() {
        let items = vec![
            StatusItem::new(
                ItemId::Search,
                Side::Left,
                vec![Part::button(Glyph::Search, Action::Search, true, "Hladat")],
            ),
            clock("14:05"),
        ];
        let t = Theme::dark();
        let mut p = Rec {
            size: Size { w: 800, h: 28 },
            ..Default::default()
        };
        render_bar(&mut p, &t, &items, hover(ItemId::Search, 0));
        // zaoblený čip (blit) s farbou "text nad pozadim"; stred ciepu = plna farba, roh je zaobleny
        let chip = chip_of(&p).expect("hover cip");
        assert_eq!((chip.0.x, chip.0.w), (4, 28));
        let want = hover_color(&t, t.background);
        assert_eq!(
            chip.1[(chip.0.h / 2 * chip.0.w + chip.0.w / 2) as usize],
            want
        );
        assert_ne!(
            chip.1[0], want,
            "roh je zaobleny (priehladny/zmiesany s pozadim)"
        );
        assert_eq!(p.texts[0].0, "\u{E721}");
        assert_eq!(p.texts[0].2.font, Font::Icons);
        // hover nad hodinami (bez akcie) sa nekresli
        let mut p = Rec {
            size: Size { w: 800, h: 28 },
            ..Default::default()
        };
        render_bar(&mut p, &t, &items, hover(ItemId::Clock, 0));
        assert!(chip_of(&p).is_none());
    }

    #[test]
    fn battery_fill_is_proportional() {
        let items = vec![StatusItem::new(
            ItemId::Battery,
            Side::Right,
            vec![Part::Battery {
                percent: 50,
                charging: false,
            }],
        )];
        let t = Theme::dark();
        let mut p = Rec {
            size: Size { w: 300, h: 28 },
            ..Default::default()
        };
        render_bar(&mut p, &t, &items, none());
        // pozadie, vlas, obrys, vnutro, vystupok, vyplnenie (20 - 4 = 16 px * 50 % = 8 px)
        assert_eq!(p.fills.len(), 6);
        assert_eq!(p.fills[5].0.w, 8);
        assert_eq!(p.fills[5].1, t.foreground);
        // nizka bateria sa vyfarbi inak
        let items = vec![StatusItem::new(
            ItemId::Battery,
            Side::Right,
            vec![Part::Battery {
                percent: 10,
                charging: false,
            }],
        )];
        let mut p = Rec {
            size: Size { w: 300, h: 28 },
            ..Default::default()
        };
        render_bar(&mut p, &t, &items, none());
        assert_eq!(p.fills.last().unwrap().1, t.low);
    }

    fn media(enabled_prev: bool, art: Option<ArtImage>) -> Vec<StatusItem> {
        vec![StatusItem::new(
            ItemId::Media,
            Side::Left,
            vec![
                Part::Art { art, action: None },
                Part::button(
                    Glyph::Prev,
                    Action::MediaPrev,
                    enabled_prev,
                    "Predchadzajuca",
                ),
                Part::button(Glyph::Pause, Action::MediaToggle, true, "Pozastavit"),
                Part::button(Glyph::Next, Action::MediaNext, true, "Dalsia"),
            ],
        )]
    }

    fn rec(w: i32) -> Rec {
        Rec {
            size: Size { w, h: 28 },
            ..Default::default()
        }
    }

    #[test]
    fn the_oled_shift_moves_content_and_hit_areas_together_and_no_lines_drops_the_lines() {
        let t = Theme::dark();
        let items = vec![media_item("Skladba"), clock("14:05")];
        let mut p0 = rec(600);
        let base = render_bar(&mut p0, &t, &items, none());
        for (dx, dy) in [(2, 1), (-2, 0), (1, 1)] {
            let mut p = rec(600);
            let ui = Interaction {
                shift: (dx, dy),
                ..Default::default()
            };
            let moved = render_bar(&mut p, &t, &items, ui);
            // posun je obmedzeny okrajmi lisaty (hodiny pri pravom okraji): vsetky casti o rovnaky dx v <0, 600>
            let d = moved[0].x - base[0].x;
            assert!(d.abs() <= dx.abs(), "{d}");
            for (a, b) in base.iter().zip(&moved) {
                assert_eq!(b.x - a.x, d, "vsetky casti spolu");
                assert!(b.x >= 0 && b.x + b.w <= 600);
            }
            // text je posunuty aj zvisle; pozadie nie
            let (_, r0, _) = p0.texts.iter().find(|x| x.0 == "14:05").unwrap();
            let (_, r1, _) = p.texts.iter().find(|x| x.0 == "14:05").unwrap();
            assert_eq!(r1.y - r0.y, dy);
            assert_eq!(p.fills[0].0, Rect::new(0, 0, 600, 28), "pozadie na mieste");
        }
        // bez ciar: chyba spodny vlas (plna sirka, vyska 1 px)
        let hair = |p: &Rec| p.fills.iter().any(|(r, _)| r.w == 600 && r.h == 1);
        assert!(hair(&p0));
        let mut p = rec(600);
        render_bar(
            &mut p,
            &t,
            &items,
            Interaction {
                no_lines: true,
                ..Default::default()
            },
        );
        assert!(!hair(&p));
    }

    #[test]
    fn media_icons_are_vector_blits_and_disabled_is_dimmed() {
        let t = Theme::dark();
        let mut p = rec(600);
        render_bar(&mut p, &t, &media(false, None), none());
        // obal (zastupny symbol) je fill, tri ikony su blity 14x14 centrovane v 28 px tlacidlach
        assert_eq!(p.blits.len(), 3);
        for (r, _) in &p.blits {
            assert_eq!((r.w, r.h, r.y), (14, 14, 7));
        }
        let max_of = |px: &Vec<Color>| {
            *px.iter()
                .max_by_key(|c| c.r as u32 + c.g as u32 + c.b as u32)
                .unwrap()
        };
        // zakazana "predchadzajuca": najsvetlejsi pixel = farba disabled, povolene = foreground
        assert_eq!(max_of(&p.blits[0].1), t.disabled);
        assert_eq!(max_of(&p.blits[1].1), t.foreground);
        assert!(p.texts.is_empty(), "ikony medii sa nekreslia pismom");
    }

    #[test]
    fn disabled_button_gets_no_hover_or_pressed() {
        let t = Theme::dark();
        let mut p = rec(600);
        let ui = Interaction {
            hovered: Some((ItemId::Media, 1)),
            pressed: Some((ItemId::Media, 1)),
            ..Default::default()
        };
        render_bar(&mut p, &t, &media(false, None), ui);
        assert!(
            chip_of(&p).is_none(),
            "vypnute tlacidlo nema hover ani pressed"
        );
    }

    #[test]
    fn pressed_wins_over_hover_and_icon_blends_with_pressed_bg() {
        let t = Theme::light();
        let mut p = rec(600);
        let ui = Interaction {
            hovered: Some((ItemId::Media, 2)),
            pressed: Some((ItemId::Media, 2)),
            ..Default::default()
        };
        render_bar(&mut p, &t, &media(true, None), ui);
        let (r, _) = chip_of(&p).expect("stlacene pozadie");
        // ikona (14 px) je cela vnutri stlaceneho ciepu (vyska 28 - 2*3 - vlas)
        let (b, px) = p
            .blits
            .iter()
            .find(|(b, _)| b.w == 14 && b.h == 14 && b.x >= r.x && b.x + b.w <= r.x + r.w)
            .expect("ikona");
        assert!(b.y >= r.y && b.y + b.h <= r.y + r.h && b.x >= r.x && b.x + b.w <= r.x + r.w);
        assert_eq!(
            px[0],
            pressed_color(&t, t.background),
            "roh ikony = pozadie stlacenia"
        );
        assert_ne!(
            pressed_color(&t, t.background),
            hover_color(&t, t.background)
        );
    }

    #[test]
    fn art_placeholder_and_image() {
        let t = Theme::dark();
        let mut p = rec(600);
        render_bar(&mut p, &t, &media(true, None), none());
        assert!(p
            .fills
            .iter()
            .any(|(r, c)| *c == t.placeholder && r.w == 22 && r.h == 22));
        let art = ArtImage {
            id: 1,
            image: std::sync::Arc::new(transom_core::image::Image::solid(96, 96, 0xFF8800)),
        };
        let mut p = rec(600);
        render_bar(&mut p, &t, &media(true, Some(art)), none());
        assert!(!p.fills.iter().any(|(_, c)| *c == t.placeholder));
        let (r, px) = &p.blits[0];
        assert_eq!((r.w, r.h, r.y), (22, 22, 3));
        assert!(px.iter().all(|c| *c == Color::hex(0xFF8800)));
    }
}
