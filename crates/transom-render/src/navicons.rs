//! Navigacne ikony okna nastaveni: vlastne vektory (monoline, zaoblene, mriezka 16 x 16 jednotiek), rovnaky jazyk ako
//! znacka transom. Kresli sa cez `Painter::blend_rgba`, takze okraje su vyhladene nad lubovolnym podkladom.
use crate::brand::{rasterize, Palette, Prim, Rgba, Role};
use crate::painter::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavIcon {
    Layout,
    Display,
    Media,
    Background,
    Clock,
    Ai,
    Sound,
    Panel,
    /// Baterie periferii.
    Battery,
    Tools,
    /// Doplnky (nove polozky lisaty): kruh so znamienkom plus.
    Extras,
    Language,
    Transfer,
}

pub const ALL: [NavIcon; 13] = [
    NavIcon::Layout,
    NavIcon::Display,
    NavIcon::Media,
    NavIcon::Background,
    NavIcon::Clock,
    NavIcon::Ai,
    NavIcon::Sound,
    NavIcon::Panel,
    NavIcon::Battery,
    NavIcon::Tools,
    NavIcon::Extras,
    NavIcon::Language,
    NavIcon::Transfer,
];

const W: f32 = 1.5;

fn stroke(pts: Vec<(f32, f32)>) -> Prim {
    Prim::Stroke {
        pts,
        width: W,
        role: Role::Glyph,
    }
}

fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    let n = 20;
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<(f32, f32)> {
    (0..=28)
        .map(|i| {
            let a = (360.0 * i as f32 / 28.0).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// Primitivy ikony v jednotkach 16 x 16.
pub fn prims(icon: NavIcon) -> Vec<Prim> {
    match icon {
        // tri stlpce lisaty: obrys + dva delitka
        NavIcon::Layout => vec![
            stroke(vec![
                (2.2, 3.2),
                (13.8, 3.2),
                (13.8, 12.8),
                (2.2, 12.8),
                (2.2, 3.2),
            ]),
            stroke(vec![(6.1, 3.2), (6.1, 12.8)]),
            stroke(vec![(9.9, 3.2), (9.9, 12.8)]),
        ],
        // monitor: obrazovka a podstavec
        NavIcon::Display => vec![
            stroke(vec![
                (2.2, 3.0),
                (13.8, 3.0),
                (13.8, 10.6),
                (2.2, 10.6),
                (2.2, 3.0),
            ]),
            stroke(vec![(8.0, 10.6), (8.0, 13.0)]),
            stroke(vec![(5.0, 13.0), (11.0, 13.0)]),
        ],
        // prehravanie: zaoblený trojuholnik
        NavIcon::Media => vec![stroke(vec![
            (5.0, 3.4),
            (12.6, 8.0),
            (5.0, 12.6),
            (5.0, 3.4),
        ])],
        // kontrast: kruh s vyplnenou lavou polovicou
        NavIcon::Background => vec![
            stroke(ellipse(8.0, 8.0, 5.6, 5.6)),
            Prim::Stroke {
                pts: arc(8.0, 8.0, 2.8, 90.0, 270.0),
                width: 5.6,
                role: Role::Glyph,
            },
        ],
        // hodiny: kruh + rucicky
        NavIcon::Clock => vec![
            stroke(ellipse(8.0, 8.0, 5.6, 5.6)),
            stroke(vec![(8.0, 4.6), (8.0, 8.0), (10.6, 9.6)]),
        ],
        // ukazovatel: oblúk 270 stupnov + ihla
        NavIcon::Ai => vec![
            stroke(arc(8.0, 8.4, 5.6, 135.0, 405.0)),
            stroke(vec![(8.0, 8.4), (10.6, 5.6)]),
        ],
        // reproduktor: telo so zvukovymi vlnami
        NavIcon::Sound => vec![
            stroke(vec![
                (2.6, 6.2),
                (5.4, 6.2),
                (8.6, 3.4),
                (8.6, 12.6),
                (5.4, 9.8),
                (2.6, 9.8),
                (2.6, 6.2),
            ]),
            stroke(arc(8.6, 8.0, 3.2, -48.0, 48.0)),
            stroke(arc(8.6, 8.0, 5.4, -48.0, 48.0)),
        ],
        // panel rychlych akcii: 2 x 2 dlazdice
        // polica / vybezok: obrys nadoby s "vybezkom" navrchu a dve ciarky obsahu
        NavIcon::Tools => vec![
            stroke(vec![
                (2.2, 5.6),
                (13.8, 5.6),
                (13.8, 13.2),
                (2.2, 13.2),
                (2.2, 5.6),
            ]),
            stroke(vec![(5.6, 5.6), (5.6, 2.8), (10.4, 2.8), (10.4, 5.6)]),
            stroke(vec![(5.0, 9.0), (11.0, 9.0)]),
            stroke(vec![(5.0, 11.2), (9.0, 11.2)]),
        ],
        NavIcon::Panel => vec![
            stroke(vec![
                (2.6, 2.6),
                (7.0, 2.6),
                (7.0, 7.0),
                (2.6, 7.0),
                (2.6, 2.6),
            ]),
            stroke(vec![
                (9.0, 2.6),
                (13.4, 2.6),
                (13.4, 7.0),
                (9.0, 7.0),
                (9.0, 2.6),
            ]),
            stroke(vec![
                (2.6, 9.0),
                (7.0, 9.0),
                (7.0, 13.4),
                (2.6, 13.4),
                (2.6, 9.0),
            ]),
            stroke(vec![
                (9.0, 9.0),
                (13.4, 9.0),
                (13.4, 13.4),
                (9.0, 13.4),
                (9.0, 9.0),
            ]),
        ],
        // bateria: obrys s vystupkom a ciastocna vypln
        NavIcon::Battery => vec![
            stroke(vec![
                (2.2, 5.0),
                (12.4, 5.0),
                (12.4, 11.0),
                (2.2, 11.0),
                (2.2, 5.0),
            ]),
            stroke(vec![(14.0, 7.0), (14.0, 9.0)]),
            Prim::Stroke {
                pts: vec![(4.6, 8.0), (8.4, 8.0)],
                width: 2.6,
                role: Role::Glyph,
            },
        ],
        NavIcon::Extras => vec![
            stroke(ellipse(8.0, 8.0, 5.8, 5.8)),
            stroke(vec![(8.0, 5.0), (8.0, 11.0)]),
            stroke(vec![(5.0, 8.0), (11.0, 8.0)]),
        ],
        // zemegula: kruh, poludnik, rovnik
        NavIcon::Language => vec![
            stroke(ellipse(8.0, 8.0, 5.6, 5.6)),
            stroke(ellipse(8.0, 8.0, 2.5, 5.6)),
            stroke(vec![(2.4, 8.0), (13.6, 8.0)]),
        ],
        // export / import: dve opacne sipky (hore a dole)
        NavIcon::Transfer => vec![
            stroke(vec![(5.0, 13.0), (5.0, 3.4)]),
            stroke(vec![(2.6, 5.8), (5.0, 3.4), (7.4, 5.8)]),
            stroke(vec![(11.0, 3.0), (11.0, 12.6)]),
            stroke(vec![(8.6, 10.2), (11.0, 12.6), (13.4, 10.2)]),
        ],
    }
}

/// Obrazok ikony `px x px` vo farbe `color` (priehladne pozadie).
pub fn render(icon: NavIcon, px: u32, color: Color) -> Vec<Rgba> {
    let c = Rgba {
        r: color.r,
        g: color.g,
        b: color.b,
        a: 255,
    };
    let pal = Palette {
        tile: c,
        bar: c,
        glyph: c,
        word_a: c,
        word_b: c,
    };
    rasterize(&prims(icon), (0.0, 0.0), (16.0, 16.0), (px, px), &pal, 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_distinct_visible_and_inside_the_box() {
        let col = Color::rgb(200, 210, 220);
        for px in [16u32, 24, 32] {
            let imgs: Vec<Vec<Rgba>> = ALL.iter().map(|i| render(*i, px, col)).collect();
            for (i, img) in imgs.iter().enumerate() {
                let ink: u32 = img.iter().map(|c| c.a as u32).sum();
                let cover = ink as f32 / (255.0 * (px * px) as f32);
                assert!((0.06..0.55).contains(&cover), "{:?} {px}: {cover}", ALL[i]);
                // farba je presne zadana farba (bez farebnych lemov)
                assert!(img
                    .iter()
                    .filter(|c| c.a > 0)
                    .all(|c| (c.r, c.g, c.b) == (200, 210, 220)));
                // okraje obrazka su prazdne (ikona sa neorezava)
                assert!(
                    img[..px as usize].iter().all(|c| c.a == 0),
                    "{:?} horny okraj",
                    ALL[i]
                );
                for j in i + 1..imgs.len() {
                    assert_ne!(img, &imgs[j], "{:?} vs {:?}", ALL[i], ALL[j]);
                }
            }
        }
    }
}
