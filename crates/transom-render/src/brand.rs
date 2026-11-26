//! Vizualna identita transom: znacka (logo), wordmark a farby; jediny zdroj pravdy pre ikony aj SVG.
//!
//! Znacka "T na obrazovke": velke **T**, ktoreho priecka je akcentova **lista** cez celu sirku (ako horna lista na
//! obrazovke) a pod nou svetly obrys obrazovky (boky a spodok). Wordmark "transom" je monoline, zaobleny,
//! geometricky - kresleny z tahov (nezavisi od fontu); "t" ma akcent ako T v znacke, ostatne farbu textu. Vsetko su vektory v pevnych jednotkach; z tej
//! istej geometrie vznikaju (1) rastrove obrazky pre okna a ikonu v oblasti oznameni, (2) `.ico` a (3) `.svg` v `assets/brand/`
//! (generuje ich `examples/brand_export.rs`, zhodu s repozitarom hlida test `committed_assets_match_the_generator`).
//! Ziadna zavislost okrem `std`; ziadne cudzie logo ani ikona.

/// RGBA, farba je *nepremultiplikovana*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn hex(v: u32) -> Self {
        Rgba {
            r: ((v >> 16) & 0xFF) as u8,
            g: ((v >> 8) & 0xFF) as u8,
            b: (v & 0xFF) as u8,
            a: 255,
        }
    }
    pub fn css(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Farby identity (dokumentovane v `docs/DESIGN.md`).
pub mod colors {
    use super::Rgba;
    /// "Ink": hlboky grafit - pozadie tmavej listy a dlazdica ikony.
    pub const INK: Rgba = Rgba::hex(0x171B21);
    /// "Signal": matovy zeleno-tyrkysovy akcent tmavej temy a znacky.
    pub const SIGNAL: Rgba = Rgba::hex(0x3FE0B5);
    /// "Signal deep": akcent svetlej temy (4,9 : 1 s bielym textom).
    pub const SIGNAL_DEEP: Rgba = Rgba::hex(0x00806A);
    /// "Mist": svetle pozadie.
    pub const MIST: Rgba = Rgba::hex(0xF2F4F6);
    /// Svetlo "L" na tmavom (a na dlazdici ikony).
    pub const GLYPH_LIGHT: Rgba = Rgba::hex(0xF4F7FA);
}

/// Rola farby primitiva (urcuje sa podla variantu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Tile,
    Bar,
    Glyph,
    WordA,
    WordB,
    /// Zvisly prechod: hore farba `bar`, dole `glyph` (po vyske primitiva) - stonok T, ktory dohora prechadza do listy.
    Fade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub tile: Rgba,
    pub bar: Rgba,
    pub glyph: Rgba,
    pub word_a: Rgba,
    pub word_b: Rgba,
}

impl Palette {
    /// Ikona aplikacie: dlazdica Ink, lista Signal, svetle "L" - citatelna na akomkolvek podklade.
    pub const APP: Palette = Palette {
        tile: colors::INK,
        bar: colors::SIGNAL,
        glyph: colors::GLYPH_LIGHT,
        word_a: colors::GLYPH_LIGHT,
        word_b: colors::SIGNAL,
    };
    /// Ikona v oblasti oznameni pre **tmavy** panel uloh (bez dlazdice).
    pub const ON_DARK: Palette = Palette {
        tile: colors::INK,
        bar: colors::SIGNAL,
        glyph: colors::GLYPH_LIGHT,
        word_a: colors::GLYPH_LIGHT,
        word_b: colors::SIGNAL,
    };
    /// Ikona v oblasti oznameni pre **svetly** panel uloh.
    pub const ON_LIGHT: Palette = Palette {
        tile: colors::MIST,
        bar: colors::SIGNAL_DEEP,
        glyph: colors::INK,
        word_a: colors::INK,
        word_b: colors::SIGNAL_DEEP,
    };

    fn color(&self, r: Role) -> Rgba {
        match r {
            Role::Tile => self.tile,
            Role::Bar => self.bar,
            Role::Glyph => self.glyph,
            Role::WordA => self.word_a,
            Role::WordB => self.word_b,
            Role::Fade => self.bar,
        }
    }

    /// Farba primitiva v bode `p` (pri `Role::Fade` podla polohy po vyske, inak plna).
    fn color_at(&self, prim: &Prim, p: (f32, f32)) -> Rgba {
        let role = role_of(prim);
        if role != Role::Fade {
            return self.color(role);
        }
        let (y0, y1) = fade_span(prim);
        let t = if y1 > y0 {
            ((p.1 - y0) / (y1 - y0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (a, b) = (self.bar, self.glyph);
        let mix = |u: u8, v: u8| (u as f32 + (v as f32 - u as f32) * t).round() as u8;
        Rgba {
            r: mix(a.r, b.r),
            g: mix(a.g, b.g),
            b: mix(a.b, b.b),
            a: 255,
        }
    }
}

/// Geometricky primitiv v pevnych jednotkach (poradie = poradie kreslenia, neskorsie je navrchu).
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    RoundRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        role: Role,
    },
    /// Tah po lomenej ciare so zaoblenymi koncami a spojmi; jediny bod = bodka.
    Stroke {
        pts: Vec<(f32, f32)>,
        width: f32,
        role: Role,
    },
}

/// Hrubsie ťahy pre male velkosti (citatelnost pri 16-24 px).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    /// do 23 px
    Small,
    /// 24-63 px
    Regular,
    /// od 64 px
    Large,
}

impl Weight {
    pub fn for_px(px: u32) -> Weight {
        match px {
            0..=23 => Weight::Small,
            24..=63 => Weight::Regular,
            _ => Weight::Large,
        }
    }
}

/// Plocha znacky v jednotkach: 32 x 32.
pub const MARK_UNITS: f32 = 32.0;

/// Znacka bez dlazdice: 32 x 32 jednotiek. Navrchu lista (priecka T) cez skoro celu sirku, z jej stredu stonok T a pod
/// listou obrys obrazovky (boky a spodok so zaoblenymi rohmi), ktory zacina kusok pod listou.
pub fn mark(weight: Weight) -> Vec<Prim> {
    let (bar_h, stem, frame) = match weight {
        Weight::Small => (7.0, 5.4, 4.0), // obrys 4 jednotky = 2 px pri 16 px, zarovnany na mriezku
        Weight::Regular | Weight::Large => (6.0, 4.6, 2.8),
    };
    let top = 4.0;
    let (x0, x1, y_bot, r) = (4.0, 28.0, 28.0, 5.0);
    let mut outline = vec![(x0, top + bar_h + 2.4)];
    outline.extend(arc(x0 + r, y_bot - r, r, r, 180.0, 90.0, 12));
    outline.extend(arc(x1 - r, y_bot - r, r, r, 90.0, 0.0, 12));
    outline.push((x1, top + bar_h + 2.4));
    vec![
        Prim::Stroke {
            pts: outline,
            width: frame,
            role: Role::Glyph,
        },
        Prim::RoundRect {
            x: 2.0,
            y: top,
            w: 28.0,
            h: bar_h,
            r: bar_h / 2.0,
            role: Role::Bar,
        },
        // stonok: dole svetly, dohora prechadza do farby listy (Fade)
        Prim::Stroke {
            pts: vec![(16.0, top + bar_h - 1.0), (16.0, 23.0)],
            width: stem,
            role: Role::Fade,
        },
    ]
}

/// Znacka na dlazdici (ikona aplikacie): dlazdica 32 x 32 s polomerom 7,5 a znacka zmensena na 74 %.
pub fn tile_mark(weight: Weight) -> Vec<Prim> {
    let mut v = vec![Prim::RoundRect {
        x: 0.0,
        y: 0.0,
        w: 32.0,
        h: 32.0,
        r: 7.5,
        role: Role::Tile,
    }];
    let k = 0.74;
    let t = |p: (f32, f32)| (16.0 + (p.0 - 16.0) * k, 16.0 + (p.1 - 16.0) * k + 0.4);
    for p in mark(weight) {
        v.push(match p {
            Prim::RoundRect {
                x,
                y,
                w,
                h,
                r,
                role,
            } => {
                let (nx, ny) = t((x, y));
                Prim::RoundRect {
                    x: nx,
                    y: ny,
                    w: w * k,
                    h: h * k,
                    r: r * k,
                    role,
                }
            }
            Prim::Stroke { pts, width, role } => Prim::Stroke {
                pts: pts.into_iter().map(t).collect(),
                width: width * k,
                role,
            },
        });
    }
    v
}

fn arc(cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// Vyska x-vysky pisma wordmarku v jednotkach.
pub const WORD_X_HEIGHT: f32 = 10.0;
/// Hrubka tahu wordmarku.
pub const WORD_STROKE: f32 = 2.3;

/// Wordmark "transom": (primitivy, sirka, vyska) v jednotkach; zaciatok x = 0 (vratane polovice tahu), y: horny okraj
/// stonku t = 0. Pismena su monoline, kreslene z tahov (bez fontu); "t" ma farbu B (akcent, ako T v znacke), "ransom" A.
pub fn wordmark() -> (Vec<Prim>, f32, f32) {
    let sw = WORD_STROKE;
    let h = sw / 2.0;
    let asc = 3.4 + h; // stonok t nad x-vyskou (+ polovica tahu, aby horny okraj bol v y = 0)
    let base = asc + WORD_X_HEIGHT; // zakladna ciara (y)
    let top = asc; // horny okraj x-vysky
    let mut v = Vec::new();
    let gap = 3.0;
    let st = |pts: Vec<(f32, f32)>| Prim::Stroke {
        pts,
        width: sw,
        role: Role::WordA,
    };
    let accent = |pts: Vec<(f32, f32)>| Prim::Stroke {
        pts,
        width: sw,
        role: Role::WordB,
    };
    let mut x = 0.0;

    // t: stonok s ohnutou patou doprava, priecka na x-vyske
    let mut t = vec![(x + h + 1.8, h)];
    t.extend(arc(
        x + h + 1.8 + 2.8,
        base - 2.8,
        2.8,
        2.8,
        180.0,
        90.0,
        10,
    ));
    v.push(accent(t));
    v.push(accent(vec![(x + h, top), (x + h + 5.6, top)]));
    x += sw + 5.6 + gap;
    // r: stonok a oblúk k vrchu
    v.push(st(vec![(x + h, base), (x + h, top + 4.4)]));
    v.push(st(arc(x + h + 4.4, top + 4.4, 4.4, 4.4, 180.0, 270.0, 10)));
    x += sw + 4.4 + gap - 0.6;
    // a: ovál a stonok vpravo
    let (rx, ry) = (4.4, 5.0);
    v.push(st(arc(x + h + rx, top + ry, rx, ry, 0.0, 360.0, 36)));
    v.push(st(vec![(x + h + 2.0 * rx, top), (x + h + 2.0 * rx, base)]));
    x += sw + 2.0 * rx + gap;
    // n
    v.push(st(vec![(x + h, top), (x + h, base)]));
    let mut a = arc(x + h + 4.2, top + 4.6, 4.2, 4.6, 180.0, 360.0, 14);
    a.push((x + h + 8.4, base));
    v.push(st(a));
    x += sw + 8.4 + gap;
    // s: horna misa proti smeru hodin (vpravo hore -> vlavo -> stred), dolna v smere (stred -> vpravo -> vlavo dole)
    let (sr, sh) = (3.7, 2.5);
    let mut s1 = arc(x + h + sr, top + sh, sr, sh, -25.0, -270.0, 20);
    s1.extend(arc(x + h + sr, base - sh, sr, sh, -90.0, 155.0, 20));
    v.push(st(s1));
    x += sw + 2.0 * sr + gap;
    // o
    v.push(st(arc(x + h + 4.8, top + 5.0, 4.8, 5.0, 0.0, 360.0, 36)));
    x += sw + 9.6 + gap;
    // m: stonok a dva oblúky
    v.push(st(vec![(x + h, top), (x + h, base)]));
    let mut m1 = arc(x + h + 3.5, top + 4.0, 3.5, 4.0, 180.0, 360.0, 12);
    m1.push((x + h + 7.0, base));
    v.push(st(m1));
    let mut m2 = arc(x + h + 10.5, top + 4.0, 3.5, 4.0, 180.0, 360.0, 12);
    m2.push((x + h + 14.0, base));
    v.push(st(m2));
    x += sw + 14.0;
    (v, x, base + h)
}

// ---------- rasterizer ----------

/// Zvisly rozsah prechodu `Role::Fade`: od spodneho okraja listy (prechod zacina az pod listou, kde je stonok vidno) po
/// spodny koniec tahu (vratane zaobleneho konca).
fn fade_span(prim: &Prim) -> (f32, f32) {
    match prim {
        Prim::RoundRect { y, h, .. } => (*y, y + h),
        Prim::Stroke { pts, width, .. } => {
            let lo = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
            let hi = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
            (lo, hi + width / 2.0)
        }
    }
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + abx * t, a.1 + aby * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

fn inside(prim: &Prim, p: (f32, f32)) -> bool {
    match prim {
        Prim::RoundRect { x, y, w, h, r, .. } => {
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let (qx, qy) = (
                (p.0 - cx).abs() - (w / 2.0 - r),
                (p.1 - cy).abs() - (h / 2.0 - r),
            );
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            outside + qx.max(qy).min(0.0) <= *r
        }
        Prim::Stroke { pts, width, .. } => {
            let r = width / 2.0;
            if pts.len() == 1 {
                return (p.0 - pts[0].0).powi(2) + (p.1 - pts[0].1).powi(2) <= r * r;
            }
            pts.windows(2).any(|s| dist_to_segment(p, s[0], s[1]) <= r)
        }
    }
}

fn role_of(prim: &Prim) -> Role {
    match prim {
        Prim::RoundRect { role, .. } | Prim::Stroke { role, .. } => *role,
    }
}

/// Vykresli primitivy do obrazka `px_w` x `px_h` (jednotky `[ox, ox + uw] x [oy, oy + uh]` sa roztiahnu na cely obrazok).
/// Supersampling `ss x ss`; vystup = nepremultiplikovane RGBA zhora nadol. Pozadie je priehladne.
pub fn rasterize(
    prims: &[Prim],
    origin: (f32, f32),
    units: (f32, f32),
    px: (u32, u32),
    palette: &Palette,
    ss: u32,
) -> Vec<Rgba> {
    let (pw, ph) = px;
    let mut out = Vec::with_capacity((pw * ph) as usize);
    let (sx, sy) = (units.0 / pw as f32, units.1 / ph as f32);
    let n = (ss * ss) as f32;
    for y in 0..ph {
        for x in 0..pw {
            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for j in 0..ss {
                for i in 0..ss {
                    let p = (
                        origin.0 + (x as f32 + (i as f32 + 0.5) / ss as f32) * sx,
                        origin.1 + (y as f32 + (j as f32 + 0.5) / ss as f32) * sy,
                    );
                    if let Some(pr) = prims.iter().rev().find(|pr| inside(pr, p)) {
                        let c = palette.color_at(pr, p);
                        r += c.r as f32;
                        g += c.g as f32;
                        b += c.b as f32;
                        a += 1.0;
                    }
                }
            }
            out.push(if a == 0.0 {
                Rgba {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 0,
                }
            } else {
                Rgba {
                    r: (r / a).round() as u8,
                    g: (g / a).round() as u8,
                    b: (b / a).round() as u8,
                    a: (255.0 * a / n).round() as u8,
                }
            });
        }
    }
    out
}

/// Obrazok znacky `px x px` (bez dlazdice, priehladne pozadie) s hrubkou podla velkosti.
pub fn render_mark(px: u32, palette: &Palette) -> Vec<Rgba> {
    let ss = if px <= 64 { 8 } else { 4 };
    rasterize(
        &mark(Weight::for_px(px)),
        (0.0, 0.0),
        (MARK_UNITS, MARK_UNITS),
        (px, px),
        palette,
        ss,
    )
}

/// Obrazok ikony aplikacie (dlazdica + znacka) `px x px`.
pub fn render_app_icon(px: u32) -> Vec<Rgba> {
    let ss = if px <= 64 { 8 } else { 4 };
    rasterize(
        &tile_mark(Weight::for_px(px)),
        (0.0, 0.0),
        (MARK_UNITS, MARK_UNITS),
        (px, px),
        &Palette::APP,
        ss,
    )
}

/// Obrazok wordmarku vysoky `px_h` (sirka podla pomeru).
pub fn render_wordmark(px_h: u32, palette: &Palette) -> (Vec<Rgba>, u32, u32) {
    let (prims, w, h) = wordmark();
    let pad = 0.6;
    let (uw, uh) = (w + 2.0 * pad, h + 2.0 * pad);
    let pw = ((uw / uh) * px_h as f32).ceil() as u32;
    let ss = if px_h <= 48 { 8 } else { 4 };
    (
        rasterize(&prims, (-pad, -pad), (uw, uh), (pw, px_h), palette, ss),
        pw,
        px_h,
    )
}

// ---------- SVG ----------

fn svg_prim(p: &Prim, pal: &Palette, indent: &str) -> String {
    let fmt = |v: f32| {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    match p {
        Prim::RoundRect {
            x,
            y,
            w,
            h,
            r,
            role,
        } => {
            format!(
            "{indent}<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\"/>\n",
            fmt(*x), fmt(*y), fmt(*w), fmt(*h), fmt(*r), pal.color(*role).css()
        )
        }
        Prim::Stroke { pts, width, role } => {
            if pts.len() == 1 {
                format!(
                    "{indent}<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\"/>\n",
                    fmt(pts[0].0),
                    fmt(pts[0].1),
                    fmt(width / 2.0),
                    pal.color(*role).css()
                )
            } else {
                let d: String = pts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        format!(
                            "{}{} {}",
                            if i == 0 { "M" } else { " L" },
                            fmt(p.0),
                            fmt(p.1)
                        )
                    })
                    .collect();
                let (defs, stroke) = if *role == Role::Fade {
                    let (y0, y1) = fade_span(p);
                    (
                        format!(
                            "{indent}<defs><linearGradient id=\"fade\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"{}\" x2=\"0\" y2=\"{}\"><stop offset=\"0\" stop-color=\"{}\"/><stop offset=\"1\" stop-color=\"{}\"/></linearGradient></defs>\n",
                            fmt(y0),
                            fmt(y1),
                            pal.bar.css(),
                            pal.glyph.css()
                        ),
                        "url(#fade)".to_string(),
                    )
                } else {
                    (String::new(), pal.color(*role).css())
                };
                format!(
                    "{defs}{indent}<path d=\"{d}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\n",
                    fmt(*width)
                )
            }
        }
    }
}

/// SVG znacky (32 x 32). `tile` = ikona aplikacie s dlazdicou.
pub fn svg_mark(pal: &Palette, tile: bool, title: &str) -> String {
    let prims = if tile {
        tile_mark(Weight::Regular)
    } else {
        mark(Weight::Regular)
    };
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 32\" width=\"128\" height=\"128\" role=\"img\" aria-label=\"{title}\">\n  <title>{title}</title>\n"
    );
    for p in &prims {
        s.push_str(&svg_prim(p, pal, "  "));
    }
    s.push_str("</svg>\n");
    s
}

/// SVG wordmarku.
pub fn svg_wordmark(pal: &Palette) -> String {
    let (prims, w, h) = wordmark();
    let pad = 0.6;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {:.1} {:.1}\" height=\"64\" role=\"img\" aria-label=\"transom\">\n  <title>transom</title>\n",
        -pad, -pad, w + 2.0 * pad, h + 2.0 * pad
    );
    for p in &prims {
        s.push_str(&svg_prim(p, pal, "  "));
    }
    s.push_str("</svg>\n");
    s
}

/// SVG zostavy: znacka vlavo, wordmark vpravo.
pub fn svg_lockup(pal: &Palette) -> String {
    let (prims, w, h) = wordmark();
    let scale = 20.0 / h; // wordmark vysoky 20 jednotiek vedla znacky 32
    let gap = 8.0;
    let total_w = 32.0 + gap + w * scale;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {total_w:.1} 32\" height=\"64\" role=\"img\" aria-label=\"transom\">\n  <title>transom</title>\n"
    );
    for p in &tile_mark(Weight::Regular) {
        s.push_str(&svg_prim(p, pal, "  "));
    }
    s.push_str(&format!(
        "  <g transform=\"translate({:.2} {:.2}) scale({:.4})\">\n",
        32.0 + gap,
        (32.0 - 20.0) / 2.0,
        scale
    ));
    for p in &prims {
        s.push_str(&svg_prim(p, pal, "    "));
    }
    s.push_str("  </g>\n</svg>\n");
    s
}

// ---------- ICO ----------

/// `.ico` z obrazkov (velkost, RGBA zhora nadol): kazdy ako BMP DIB (32 bpp, nepremultiplikovana alfa + AND maska).
pub fn ico_bytes(images: &[(u32, Vec<Rgba>)]) -> Vec<u8> {
    let n = images.len() as u16;
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    for (size, px) in images {
        let s = *size;
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&(s as i32).to_le_bytes());
        d.extend_from_slice(&((2 * s) as i32).to_le_bytes()); // vyska XOR + AND
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0; 24]); // BI_RGB, velkosti 0, rozlisenie 0, palety 0
        for y in (0..s).rev() {
            for x in 0..s {
                let c = px[(y * s + x) as usize];
                d.extend_from_slice(&[c.b, c.g, c.r, c.a]);
            }
        }
        let row = (s as usize).div_ceil(32) * 4;
        d.extend(std::iter::repeat_n(0u8, row * s as usize)); // AND maska: 0 = pouzit obrazok (alfa riesi priehladnost)
        blobs.push(d);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    let mut off = 6 + 16 * images.len();
    for ((size, _), blob) in images.iter().zip(&blobs) {
        out.push(if *size >= 256 { 0 } else { *size as u8 });
        out.push(if *size >= 256 { 0 } else { *size as u8 });
        out.extend_from_slice(&[0, 0]); // pocet farieb, rezervovane
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&(off as u32).to_le_bytes());
        off += blob.len();
    }
    for b in blobs {
        out.extend_from_slice(&b);
    }
    out
}

/// Velkosti ikony aplikacie (Explorer, panel uloh, Alt+Tab, nastavenia pri 100-300 % DPI).
pub const APP_ICON_SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];
/// Velkosti ikony v oblasti oznameni (16 px pri 100 %, 20 pri 125 %, 24 pri 150 %, 32 pri 200 %).
pub const TRAY_ICON_SIZES: [u32; 5] = [16, 20, 24, 32, 48];

pub fn app_ico() -> Vec<u8> {
    ico_bytes(&APP_ICON_SIZES.map(|s| (s, render_app_icon(s))))
}

pub fn tray_ico(pal: &Palette) -> Vec<u8> {
    ico_bytes(&TRAY_ICON_SIZES.map(|s| (s, render_mark(s, pal))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_sum(img: &[Rgba]) -> u64 {
        img.iter().map(|c| c.a as u64).sum()
    }

    #[test]
    fn mark_is_clear_at_small_sizes() {
        for px in [16u32, 20, 24, 32] {
            let img = render_mark(px, &Palette::ON_DARK);
            let cover = alpha_sum(&img) as f64 / (255.0 * (px * px) as f64);
            assert!((0.22..0.60).contains(&cover), "{px}: pokrytie {cover}");
            // lista s T (akcent) aj obrys obrazovky (svetly) su rozlisitelne: obe farby sa vyskytuju s plnou krytim
            let bar = img
                .iter()
                .filter(|c| c.a == 255 && c.g as u16 > c.r as u16 + 60)
                .count();
            let glyph = img
                .iter()
                .filter(|c| c.a == 255 && c.r > 220 && c.g > 220)
                .count();
            assert!(bar >= (px as usize) * 2, "{px}: lista {bar}");
            assert!(glyph >= px as usize, "{px}: obrys {glyph}");
            // ziadny okraj nie je odrezany: stlpec 0 a riadok 0 su (skoro) prazdne
            assert!(
                img[..px as usize].iter().all(|c| c.a < 40),
                "{px}: horny okraj"
            );
        }
    }

    #[test]
    fn app_icon_has_an_opaque_tile_with_rounded_corners() {
        for px in [16u32, 32, 256] {
            let img = render_app_icon(px);
            let at = |x: u32, y: u32| img[(y * px + x) as usize];
            assert_eq!(at(0, 0).a, 0, "{px}: roh je priehladny");
            assert_eq!(at(px / 2, 1).a, 255, "{px}: stred hornej hrany je plny");
            assert_eq!(at(px - 1, px - 1).a, 0);
            // dlazdica je Ink
            let c = at(px / 2, px - 2);
            assert_eq!(
                (c.r, c.g, c.b),
                (colors::INK.r, colors::INK.g, colors::INK.b),
                "{px}"
            );
        }
    }

    #[test]
    fn mark_weights_follow_size() {
        assert_eq!(Weight::for_px(16), Weight::Small);
        assert_eq!(Weight::for_px(23), Weight::Small);
        assert_eq!(Weight::for_px(24), Weight::Regular);
        assert_eq!(Weight::for_px(64), Weight::Large);
    }

    #[test]
    fn wordmark_is_wide_monoline_and_two_colored() {
        let (prims, w, h) = wordmark();
        assert!(w > 80.0 && w < 100.0 && h > 14.0 && h < 18.0, "{w} x {h}");
        let a = prims.iter().filter(|p| role_of(p) == Role::WordA).count();
        let b = prims.iter().filter(|p| role_of(p) == Role::WordB).count();
        assert_eq!((a, b), (11, 2), "ransom = text, t = akcent");
        for p in &prims {
            if let Prim::Stroke { width, .. } = p {
                assert_eq!(*width, WORD_STROKE, "monoline");
            }
        }
        let (img, pw, ph) = render_wordmark(24, &Palette::ON_DARK);
        assert_eq!(img.len() as u32, pw * ph);
        assert!(pw > 24 * 5, "sirka {pw}");
        // pismena sa nedotykaju okraja obrazka (pad)
        assert!(img.iter().take(pw as usize).all(|c| c.a < 80));
    }

    #[test]
    fn ico_container_is_well_formed() {
        let ico = app_ico();
        assert_eq!(&ico[0..4], &[0, 0, 1, 0]);
        let n = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        assert_eq!(n, APP_ICON_SIZES.len());
        let mut total = 6 + 16 * n;
        for i in 0..n {
            let e = &ico[6 + 16 * i..6 + 16 * (i + 1)];
            let size = if e[0] == 0 { 256 } else { e[0] as u32 };
            assert_eq!(size, APP_ICON_SIZES[i]);
            let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
            assert_eq!(off, total);
            let row = (size as usize).div_ceil(32) * 4;
            assert_eq!(
                len,
                40 + (size * size * 4) as usize + row * size as usize,
                "{size}"
            );
            total += len;
        }
        assert_eq!(total, ico.len());
        assert_eq!(tray_ico(&Palette::ON_LIGHT)[4], TRAY_ICON_SIZES.len() as u8);
    }

    /// Subory v `assets/brand/` sa generuju z tejto geometrie (`examples/brand_export.rs`); ak sa geometria zmeni a
    /// subory nie, test padne - nemozu sa rozist (SVG, ikona aplikacie, ikony v oblasti oznameni).
    #[test]
    fn committed_assets_match_the_generator() {
        macro_rules! asset {
            ($f:literal) => {
                include_bytes!(concat!("../../../assets/brand/", $f)) as &[u8]
            };
        }
        assert_eq!(
            asset!("transom-mark.svg"),
            svg_mark(&Palette::APP, true, "transom").as_bytes()
        );
        assert_eq!(
            asset!("transom-mark-glyph-dark.svg"),
            svg_mark(&Palette::ON_DARK, false, "transom").as_bytes()
        );
        assert_eq!(
            asset!("transom-mark-glyph-light.svg"),
            svg_mark(&Palette::ON_LIGHT, false, "transom").as_bytes()
        );
        assert_eq!(
            asset!("transom-wordmark-dark.svg"),
            svg_wordmark(&Palette::ON_DARK).as_bytes()
        );
        assert_eq!(
            asset!("transom-wordmark-light.svg"),
            svg_wordmark(&Palette::ON_LIGHT).as_bytes()
        );
        assert_eq!(
            asset!("transom-lockup.svg"),
            svg_lockup(&Palette::APP).as_bytes()
        );
        assert!(
            asset!("transom-tray-dark.ico") == tray_ico(&Palette::ON_DARK).as_slice(),
            "tray tmavy ico"
        );
        assert!(
            asset!("transom-tray-light.ico") == tray_ico(&Palette::ON_LIGHT).as_slice(),
            "tray svetly ico"
        );
        assert!(
            asset!("transom.ico") == app_ico().as_slice(),
            "ikona aplikacie"
        );
    }

    #[test]
    fn svgs_are_well_formed_and_use_brand_colors() {
        let m = svg_mark(&Palette::APP, true, "transom");
        assert!(m.starts_with("<svg ") && m.trim_end().ends_with("</svg>"));
        assert!(m.contains(&colors::SIGNAL.css()) && m.contains(&colors::INK.css()));
        let w = svg_wordmark(&Palette::ON_DARK);
        assert!(w.contains("stroke-linecap=\"round\"") && w.matches("<path").count() >= 9);
        let l = svg_lockup(&Palette::APP);
        assert!(l.contains("<g transform"));
        for s in [&m, &w, &l] {
            assert_eq!(s.matches('<').count(), s.matches('>').count());
        }
    }
}
