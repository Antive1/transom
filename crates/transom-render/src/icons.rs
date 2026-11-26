//! Vektorove ikony ovladania medii (predchadzajuca, prehrat, pauza, dalsia) rasterizovane v Ruste.
//!
//! Preco nie pismo: GDI s `CLEARTYPE_QUALITY` robi na tenkych zvislych tahoch farebne okraje a s
//! `ANTIALIASED_QUALITY` su tahy Segoe Fluent Icons pri 100 % bledé. Tu sa ikona sklada z konvexnych polygonov;
//! zvisle a vodorovne hrany su zarovnane na cele pixely (ostre pri 100/150/200 %), sikme hrany sa vyhladia
//! supersamplingom 4x4. Vysledok je pokrytie 0..=255 a farba je **linearna zmes** popredia a pozadia - ziadne
//! farebne okraje z principu (overuje test).
use crate::painter::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaIcon {
    Prev,
    Play,
    Pause,
    Next,
}

type Poly = Vec<(f32, f32)>;

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Poly {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// Polygony ikony v px pre stvorec `size` x `size`.
fn shapes(icon: MediaIcon, size: i32) -> Vec<Poly> {
    let s = size as f32;
    let snap = |v: f32| v.round();
    // vyska tvaru ~ 0.62 * size, centrovana a zarovnana na cele pixely
    let h = snap(s * 0.62).max(4.0);
    let top = snap((s - h) / 2.0);
    let bot = top + h;
    let mid = (top + bot) / 2.0;
    match icon {
        MediaIcon::Play => {
            // trojuholnik so sirkou ~0.87 vysky, opticky posunuty doprava
            let w = snap(h * 0.87);
            let x0 = snap((s - w) / 2.0 + s * 0.04);
            vec![vec![(x0, top), (x0 + w, mid), (x0, bot)]]
        }
        MediaIcon::Pause => {
            let bar = snap(s * 0.18).max(2.0);
            let gap = snap(s * 0.16).max(2.0);
            let x0 = snap((s - (2.0 * bar + gap)) / 2.0);
            vec![
                rect(x0, top, x0 + bar, bot),
                rect(x0 + bar + gap, top, x0 + 2.0 * bar + gap, bot),
            ]
        }
        MediaIcon::Next | MediaIcon::Prev => {
            let bar = snap(s * 0.12).max(2.0);
            let w = snap(h * 0.72);
            let x0 = snap((s - (w + bar)) / 2.0);
            let tri = vec![(x0, top), (x0 + w, mid), (x0, bot)];
            let b = rect(x0 + w, top, x0 + w + bar, bot);
            let v = vec![tri, b];
            if icon == MediaIcon::Next {
                v
            } else {
                v.into_iter()
                    .map(|p| p.into_iter().map(|(x, y)| (s - x, y)).collect())
                    .collect()
            }
        }
    }
}

fn inside_convex(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let n = poly.len();
    let mut sign = 0i8;
    for i in 0..n {
        let (ax, ay) = poly[i];
        let (bx, by) = poly[(i + 1) % n];
        let c = (bx - ax) * (y - ay) - (by - ay) * (x - ax);
        let sg = if c > 0.0 {
            1
        } else if c < 0.0 {
            -1
        } else {
            0
        };
        if sg != 0 {
            if sign == 0 {
                sign = sg;
            } else if sg != sign {
                return false;
            }
        }
    }
    true
}

/// Pokrytie ikony (riadky zhora nadol), `size * size` hodnot 0..=255.
pub fn coverage(icon: MediaIcon, size: i32) -> Vec<u8> {
    let size = size.max(1);
    let polys = shapes(icon, size);
    const N: i32 = 4;
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut hits = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let x = px as f32 + (sx as f32 + 0.5) / N as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / N as f32;
                    if polys.iter().any(|p| inside_convex(p, x, y)) {
                        hits += 1;
                    }
                }
            }
            out.push(((hits * 255 + (N * N) / 2) / (N * N)) as u8);
        }
    }
    out
}

/// Pokrytie kruhoveho indikatora `size` x `size`: (stopa = cely medzikruzok, obluk = vyplnena cast).
/// `fill` 0..=1 (None = len stopa), oblúk zacina na 12 hodinach; `ccw` = proti smeru hodinovych ruciciek.
/// Supersampling 4x4, takze hrany su vyhladene bez farebnych okrajov.
pub fn ring_coverage(size: i32, fill: Option<f32>, ccw: bool) -> (Vec<u8>, Vec<u8>) {
    let size = size.max(1);
    let s = size as f32;
    let c = s / 2.0;
    let outer = c - 0.5;
    let thick = (s * 0.17).max(2.0);
    let inner = outer - thick;
    let frac = fill.map(|f| f.clamp(0.0, 1.0));
    const N: i32 = 4;
    let mut track = Vec::with_capacity((size * size) as usize);
    let mut arc = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut t, mut a) = (0, 0);
            for sy in 0..N {
                for sx in 0..N {
                    let x = px as f32 + (sx as f32 + 0.5) / N as f32 - c;
                    let y = py as f32 + (sy as f32 + 0.5) / N as f32 - c;
                    let d = (x * x + y * y).sqrt();
                    if d > outer || d < inner {
                        continue;
                    }
                    t += 1;
                    if let Some(f) = frac {
                        // uhol od 12 hodin v smere hodinovych ruciciek, 0..1
                        let mut ang = x.atan2(-y) / std::f32::consts::TAU;
                        if ang < 0.0 {
                            ang += 1.0;
                        }
                        if ccw {
                            ang = 1.0 - ang;
                        }
                        if ang < f || f >= 1.0 {
                            a += 1;
                        }
                    }
                }
            }
            track.push(((t * 255 + 8) / 16) as u8);
            arc.push(((a * 255 + 8) / 16) as u8);
        }
    }
    (track, arc)
}

/// Zmes farby `base` k `top` podla pokrytia (0..=255).
pub fn mix(base: Color, top: Color, t: u8) -> Color {
    Color::rgb(
        lerp(base.r, top.r, t),
        lerp(base.g, top.g, t),
        lerp(base.b, top.b, t),
    )
}

fn lerp(a: u8, b: u8, t: u8) -> u8 {
    let (a, b, t) = (a as u32, b as u32, t as u32);
    ((a * (255 - t) + b * t + 127) / 255) as u8
}

/// Pixely ikony: zmes `bg` -> `fg` podla pokrytia.
/// Pokrytie zaobleneho obdlznika `w x h` s polomerom `r` (px); 4x4 supersampling, takze rohy su vyhladene.
pub fn round_rect_coverage(w: i32, h: i32, r: f32) -> Vec<u8> {
    let (w, h) = (w.max(1), h.max(1));
    let r = r.min(w as f32 / 2.0).min(h as f32 / 2.0).max(0.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (hx, hy) = (cx - r, cy - r);
    let mut out = Vec::with_capacity((w * h) as usize);
    const N: i32 = 4;
    for y in 0..h {
        for x in 0..w {
            let mut n = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let px = x as f32 + (sx as f32 + 0.5) / N as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / N as f32;
                    let (qx, qy) = ((px - cx).abs() - hx, (py - cy).abs() - hy);
                    let d =
                        (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0);
                    if d <= r {
                        n += 1;
                    }
                }
            }
            out.push(((n * 255 + 8) / 16) as u8);
        }
    }
    out
}

pub fn shade(cov: &[u8], fg: Color, bg: Color) -> Vec<Color> {
    cov.iter()
        .map(|&t| {
            Color::rgb(
                lerp(bg.r, fg.r, t),
                lerp(bg.g, fg.g, t),
                lerp(bg.b, fg.b, t),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [MediaIcon; 4] = [
        MediaIcon::Prev,
        MediaIcon::Play,
        MediaIcon::Pause,
        MediaIcon::Next,
    ];
    /// Velkost boxu ikony pri 100/150/200 % (14 lp).
    const SIZES: [i32; 3] = [14, 21, 28];

    #[test]
    fn icons_are_distinct_and_nonempty_at_all_scales() {
        for size in SIZES {
            let covs: Vec<Vec<u8>> = ALL.iter().map(|&i| coverage(i, size)).collect();
            for (i, c) in covs.iter().enumerate() {
                let ink: u32 = c.iter().map(|&v| v as u32).sum();
                assert!(
                    ink > 255 * (size as u32),
                    "{:?} {size}: prilis malo pokrytia",
                    ALL[i]
                );
                for (j, d) in covs.iter().enumerate().skip(i + 1) {
                    assert_ne!(c, d, "{:?} a {:?} su rovnake pri {size}", ALL[i], ALL[j]);
                }
            }
        }
    }

    #[test]
    fn prev_is_mirror_of_next() {
        for size in SIZES {
            let n = coverage(MediaIcon::Next, size);
            let p = coverage(MediaIcon::Prev, size);
            let s = size as usize;
            for y in 0..s {
                for x in 0..s {
                    let a = n[y * s + x] as i32;
                    let b = p[y * s + (s - 1 - x)] as i32;
                    assert!((a - b).abs() <= 16, "{size}: ({x},{y}) {a} vs {b}");
                }
            }
        }
    }

    #[test]
    fn pause_bars_have_crisp_vertical_edges() {
        // pauza je len z obdlznikov na celych pixeloch -> pokrytie je iba 0 alebo 255
        for size in SIZES {
            assert!(coverage(MediaIcon::Pause, size)
                .iter()
                .all(|&v| v == 0 || v == 255));
        }
    }

    #[test]
    fn shading_never_produces_color_fringes() {
        // Nezavisle od implementacie: z kanala s najvacsim rozdielom fg-bg sa odvodi podiel t a ostatne kanaly
        // musia zodpovedat tomu istemu t (+-1 na zaokruhlenie). ClearType by kanaly posunul nezavisle.
        let pairs = [
            (Color::hex(0xE6E9EE), Color::hex(0x1E2228)), // tmava tema
            (Color::hex(0x1B1F24), Color::hex(0xF3F4F6)), // svetla
            (Color::hex(0x1B1F24), Color::hex(0xDCE0E5)), // svetla, hover
            (Color::hex(0x5C636E), Color::hex(0x1E2228)), // zakazane
        ];
        for (fg, bg) in pairs {
            let ch = |c: Color| [c.r as f32, c.g as f32, c.b as f32];
            let (f, b) = (ch(fg), ch(bg));
            let k = (0..3)
                .max_by(|&x, &y| (f[x] - b[x]).abs().total_cmp(&(f[y] - b[y]).abs()))
                .unwrap_or(0);
            for icon in ALL {
                for size in SIZES {
                    for c in shade(&coverage(icon, size), fg, bg) {
                        let v = ch(c);
                        let t = (v[k] - b[k]) / (f[k] - b[k]);
                        assert!((-0.01..=1.01).contains(&t), "mimo usecky fg-bg");
                        for n in 0..3 {
                            let want = b[n] + t * (f[n] - b[n]);
                            assert!((v[n] - want).abs() <= 1.5, "kanal {n}: {} vs {want}", v[n]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn round_rect_has_solid_core_and_rounded_corners() {
        let (w, h) = (28, 22);
        let c = round_rect_coverage(w, h, 4.0);
        let at = |x: i32, y: i32| c[(y * w + x) as usize];
        assert_eq!(at(14, 11), 255, "stred");
        assert_eq!(at(14, 0), 255, "hrana");
        assert!(at(0, 0) < 40, "roh je zaobleny: {}", at(0, 0));
        assert!(at(w - 1, h - 1) < 40);
        assert!(at(1, 1) > 100 && at(1, 1) < 255, "prechod v rohu");
        // polomer 0 = obycajny obdlznik
        assert!(round_rect_coverage(6, 4, 0.0).iter().all(|&v| v == 255));
        // nikdy nepresiahne polovicu strany
        assert_eq!(round_rect_coverage(4, 4, 99.0).len(), 16);
    }

    #[test]
    fn ring_fill_matches_fraction_and_direction() {
        for size in [16, 24, 32] {
            let (track, empty) = ring_coverage(size, None, false);
            assert!(empty.iter().all(|&v| v == 0), "bez hodnoty ziadny oblúk");
            let ink: u32 = track.iter().map(|&v| v as u32).sum();
            assert!(ink > 0);
            let (_, full) = ring_coverage(size, Some(1.0), false);
            let full_ink: u32 = full.iter().map(|&v| v as u32).sum();
            assert_eq!(full_ink, ink, "plny oblúk = cela stopa");
            let (_, quarter) = ring_coverage(size, Some(0.25), false);
            let q: u32 = quarter.iter().map(|&v| v as u32).sum();
            let ratio = q as f32 / ink as f32;
            assert!((ratio - 0.25).abs() < 0.05, "{size}: {ratio}");
            // 25 % v smere hodinovych ruciciek je vpravo hore, proti smeru vlavo hore
            let s = size as usize;
            let right_top: u32 = (0..s / 2)
                .flat_map(|y| (s / 2..s).map(move |x| (x, y)))
                .map(|(x, y)| quarter[y * s + x] as u32)
                .sum();
            assert!(right_top * 10 > q * 9);
            let (_, ccw) = ring_coverage(size, Some(0.25), true);
            let left_top: u32 = (0..s / 2)
                .flat_map(|y| (0..s / 2).map(move |x| (x, y)))
                .map(|(x, y)| ccw[y * s + x] as u32)
                .sum();
            assert!(left_top * 10 > q * 9);
        }
        // mimo rozsahu sa oreže
        assert_eq!(
            ring_coverage(16, Some(7.0), false).1,
            ring_coverage(16, Some(1.0), false).1
        );
    }

    #[test]
    fn shapes_stay_inside_box() {
        for icon in ALL {
            for size in SIZES {
                for p in shapes(icon, size) {
                    for (x, y) in p {
                        assert!(x >= 0.0 && x <= size as f32 && y >= 0.0 && y <= size as f32);
                    }
                }
            }
        }
    }
}
