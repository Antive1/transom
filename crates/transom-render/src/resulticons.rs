//! Vstavane ikony vysledkov vyhladavaca a zastupne ikony miniatur: vlastne vektory (monoline, zaoblene, mriezka 16 x 16
//! jednotiek), rovnaky jazyk ako `navicons` a znacka transom. Kreslia sa cez `Painter::blend_rgba`, takze su ostre pri kazdom
//! DPI (rasterizuju sa na presnu velkost v px) a ich okraje su vyhladene nad lubovolnym podkladom. Pouzivaju sa pre Nastavenia,
//! Akcie transom, Poznamky, Policu a ako neutralna zastupna ikona, kym (alebo ak sa) systemova ikona nenacita.
use crate::brand::{rasterize, Palette, Prim, Rgba, Role};
use crate::painter::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResultIcon {
    App,
    File,
    Folder,
    Image,
    Setting,
    Action,
    Note,
    Shelf,
}

pub const ALL: [ResultIcon; 8] = [
    ResultIcon::App,
    ResultIcon::File,
    ResultIcon::Folder,
    ResultIcon::Image,
    ResultIcon::Setting,
    ResultIcon::Action,
    ResultIcon::Note,
    ResultIcon::Shelf,
];

const W: f32 = 1.5;

fn stroke(pts: Vec<(f32, f32)>) -> Prim {
    Prim::Stroke {
        pts,
        width: W,
        role: Role::Glyph,
    }
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<(f32, f32)> {
    (0..=28)
        .map(|i| {
            let a = (360.0 * i as f32 / 28.0).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

fn closed(pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut v = pts.to_vec();
    v.push(pts[0]);
    v
}

/// Primitivy ikony v jednotkach 16 x 16.
pub fn prims(icon: ResultIcon) -> Vec<Prim> {
    match icon {
        // okno aplikacie: obrys, titulny pas a bodka
        ResultIcon::App => vec![
            stroke(closed(&[
                (2.2, 3.0),
                (13.8, 3.0),
                (13.8, 13.0),
                (2.2, 13.0),
            ])),
            stroke(vec![(2.2, 6.0), (13.8, 6.0)]),
            stroke(vec![(4.4, 4.5)]),
        ],
        // strana s prehnutym rohom a dvoma riadkami
        ResultIcon::File => vec![
            stroke(closed(&[
                (4.0, 2.2),
                (9.6, 2.2),
                (12.0, 4.6),
                (12.0, 13.8),
                (4.0, 13.8),
            ])),
            stroke(vec![(9.6, 2.2), (9.6, 4.6), (12.0, 4.6)]),
            stroke(vec![(6.2, 8.4), (9.8, 8.4)]),
            stroke(vec![(6.2, 10.8), (9.8, 10.8)]),
        ],
        // priecinok s uskom
        ResultIcon::Folder => vec![stroke(closed(&[
            (2.2, 4.4),
            (6.2, 4.4),
            (7.6, 6.0),
            (13.8, 6.0),
            (13.8, 12.6),
            (2.2, 12.6),
        ]))],
        // obrazok: ram, hory a slnko
        ResultIcon::Image => vec![
            stroke(closed(&[
                (2.2, 3.0),
                (13.8, 3.0),
                (13.8, 13.0),
                (2.2, 13.0),
            ])),
            stroke(vec![
                (3.0, 11.6),
                (6.2, 7.8),
                (8.6, 10.2),
                (10.4, 8.4),
                (13.0, 11.6),
            ]),
            stroke(ellipse(10.8, 5.9, 0.9, 0.9)),
        ],
        // nastavenia: prstenec so ziubmi
        ResultIcon::Setting => {
            let mut v = vec![stroke(ellipse(8.0, 8.0, 2.6, 2.6))];
            for k in 0..8 {
                let a = (45.0 * k as f32).to_radians();
                v.push(stroke(vec![
                    (8.0 + 4.7 * a.cos(), 8.0 + 4.7 * a.sin()),
                    (8.0 + 6.1 * a.cos(), 8.0 + 6.1 * a.sin()),
                ]));
            }
            v.push(stroke(ellipse(8.0, 8.0, 4.6, 4.6)));
            v
        }
        // akcia transom: blesk
        ResultIcon::Action => vec![stroke(closed(&[
            (9.2, 1.8),
            (4.2, 8.8),
            (7.6, 8.8),
            (6.8, 14.2),
            (11.8, 7.0),
            (8.4, 7.0),
        ]))],
        // poznamka: listok s riadkami
        ResultIcon::Note => vec![
            stroke(closed(&[
                (3.0, 2.6),
                (13.0, 2.6),
                (13.0, 13.4),
                (3.0, 13.4),
            ])),
            stroke(vec![(5.4, 6.0), (10.6, 6.0)]),
            stroke(vec![(5.4, 8.4), (10.6, 8.4)]),
            stroke(vec![(5.4, 10.8), (8.6, 10.8)]),
        ],
        // polica: rovnaka ikona ako nastroje (nadoba s vybezkom)
        ResultIcon::Shelf => crate::navicons::prims(crate::navicons::NavIcon::Tools),
    }
}

/// Obrazok ikony `px x px` vo farbe `color` (priehladne pozadie, nepremultiplikovana alfa).
pub fn render(icon: ResultIcon, px: u32, color: Color) -> Vec<Rgba> {
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

/// Ako `render`, ale hotovy obrazok sa pamata (kreslenie panela ho potrebuje pri kazdom prekresleni a rasterizacia nie je
/// zadarmo). Cache je ohranicena (pri prekroceni sa vyprazdni) a zdielana; obrazky su nemenne.
pub fn cached(icon: ResultIcon, px: u32, color: Color) -> std::sync::Arc<Vec<Rgba>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    type Key = (ResultIcon, u32, (u8, u8, u8));
    static CACHE: OnceLock<Mutex<HashMap<Key, Arc<Vec<Rgba>>>>> = OnceLock::new();
    let key = (icon, px, (color.r, color.g, color.b));
    let m = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = m.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return v.clone();
    }
    let img = Arc::new(render(icon, px, color));
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() >= 256 {
        g.clear();
    }
    g.insert(key, img.clone());
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_returns_the_same_image_as_a_fresh_render_and_reuses_it() {
        let col = Color::rgb(10, 20, 30);
        let a = cached(ResultIcon::Folder, 24, col);
        assert_eq!(*a, render(ResultIcon::Folder, 24, col));
        assert!(std::sync::Arc::ptr_eq(
            &a,
            &cached(ResultIcon::Folder, 24, col)
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &a,
            &cached(ResultIcon::Folder, 20, col)
        ));
    }

    #[test]
    fn icons_are_distinct_visible_crisp_and_inside_the_box_at_every_dpi_size() {
        let col = Color::rgb(190, 200, 210);
        for px in [16u32, 20, 24, 30, 32, 40, 48] {
            let imgs: Vec<Vec<Rgba>> = ALL.iter().map(|i| render(*i, px, col)).collect();
            for (i, img) in imgs.iter().enumerate() {
                assert_eq!(img.len(), (px * px) as usize);
                let ink: u32 = img.iter().map(|c| c.a as u32).sum();
                let cover = ink as f32 / (255.0 * (px * px) as f32);
                assert!((0.05..0.6).contains(&cover), "{:?} {px}: {cover}", ALL[i]);
                // farba je presne zadana (bez farebnych lemov), okraje obrazka su prazdne (ikona sa neorezava)
                assert!(img
                    .iter()
                    .filter(|c| c.a > 0)
                    .all(|c| (c.r, c.g, c.b) == (190, 200, 210)));
                let side = px as usize;
                assert!(
                    img[..side].iter().all(|c| c.a == 0),
                    "{:?} horny okraj",
                    ALL[i]
                );
                assert!(
                    img[side * (side - 1)..].iter().all(|c| c.a == 0),
                    "{:?} dolny okraj",
                    ALL[i]
                );
                for j in i + 1..imgs.len() {
                    assert_ne!(img, &imgs[j], "{:?} vs {:?}", ALL[i], ALL[j]);
                }
            }
        }
    }
}
