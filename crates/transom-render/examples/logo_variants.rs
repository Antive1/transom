//! Navrhy znacky transom (vejarove okno nad tramom): `cargo run -p transom-render --example logo_variants -- <out.png>`.
//! Kazdy variant na dlazdici ikony aj bez nej, v 16 / 32 / 64 / 256 px, na tmavom aj svetlom podklade.
use transom_render::brand::{rasterize, Palette, Prim, Rgba, Role};

fn rr(x: f32, y: f32, w: f32, h: f32, r: f32, role: Role) -> Prim {
    Prim::RoundRect {
        x,
        y,
        w,
        h,
        r,
        role,
    }
}
fn st(pts: Vec<(f32, f32)>, width: f32, role: Role) -> Prim {
    Prim::Stroke { pts, width, role }
}

fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    let n = ((a1 - a0).abs() / 6.0).ceil().max(2.0) as usize;
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Obrys obrazovky ako lomena ciara (zaoblene rohy), zacina a konci pod tramom.
fn frame_u(x0: f32, x1: f32, y_top: f32, y_bot: f32, r: f32) -> Vec<(f32, f32)> {
    let mut v = vec![(x0, y_top)];
    v.extend(arc(x0 + r, y_bot - r, r, 180.0, 90.0));
    v.extend(arc(x1 - r, y_bot - r, r, 90.0, 0.0));
    v.push((x1, y_top));
    v
}

/// A "Screen U": T (tram = lista cez celu sirku, stonok) a pod tramom obrys obrazovky (boky + spodok).
fn a(small: bool) -> Vec<Prim> {
    let (bh, sw, fw) = if small {
        (7.0, 5.4, 3.6)
    } else {
        (6.0, 4.6, 2.8)
    };
    vec![
        st(
            frame_u(4.0, 28.0, 4.0 + bh + 2.4, 28.0, 5.0),
            fw,
            Role::Glyph,
        ),
        rr(2.0, 4.0, 28.0, bh, bh / 2.0, Role::Bar),
        st(vec![(16.0, 4.0 + bh - 1.0), (16.0, 23.0)], sw, Role::Bar),
    ]
}

/// B "Desk line": T a pod nim len vodorovna spodna linka (okraj obrazovky / plocha).
fn b(small: bool) -> Vec<Prim> {
    let (bh, sw, fw) = if small {
        (7.0, 5.4, 4.0)
    } else {
        (6.0, 4.6, 3.2)
    };
    vec![
        rr(3.0, 4.0, 26.0, bh, bh / 2.0, Role::Bar),
        st(vec![(16.0, 4.0 + bh - 1.0), (16.0, 21.5)], sw, Role::Bar),
        st(vec![(6.0, 27.0), (26.0, 27.0)], fw, Role::Glyph),
    ]
}

/// C "Screen": cely obrys obrazovky, T vnutri - tram je lista prilepena k hornemu okraju obrazovky.
fn c(small: bool) -> Vec<Prim> {
    let (bh, sw, fw) = if small {
        (5.6, 5.0, 3.2)
    } else {
        (5.0, 4.2, 2.6)
    };
    let (x0, y0, x1, y1) = (3.0, 4.5, 29.0, 27.5);
    let ins = fw / 2.0 + 1.6;
    vec![
        rr(
            x0 - fw / 2.0,
            y0 - fw / 2.0,
            x1 - x0 + fw,
            y1 - y0 + fw,
            5.0,
            Role::Glyph,
        ),
        rr(
            x0 + fw / 2.0,
            y0 + fw / 2.0,
            x1 - x0 - fw,
            y1 - y0 - fw,
            3.5,
            Role::Tile,
        ),
        rr(
            x0 + ins,
            y0 + ins,
            x1 - x0 - 2.0 * ins,
            bh,
            bh / 2.0,
            Role::Bar,
        ),
        st(
            vec![(16.0, y0 + ins + bh - 1.0), (16.0, y1 - ins - 1.0)],
            sw,
            Role::Bar,
        ),
    ]
}

/// D "Monitor": ako A, ale spodok obrazovky je rovny a pod nim maly podstavec.
fn d(small: bool) -> Vec<Prim> {
    let (bh, sw, fw) = if small {
        (6.6, 5.2, 3.4)
    } else {
        (5.6, 4.4, 2.6)
    };
    vec![
        st(
            frame_u(4.0, 28.0, 3.5 + bh + 2.2, 23.5, 4.0),
            fw,
            Role::Glyph,
        ),
        st(vec![(12.0, 28.0), (20.0, 28.0)], fw, Role::Glyph),
        st(vec![(16.0, 23.5), (16.0, 28.0)], fw, Role::Glyph),
        rr(2.0, 3.5, 28.0, bh, bh / 2.0, Role::Bar),
        st(vec![(16.0, 3.5 + bh - 1.0), (16.0, 19.5)], sw, Role::Bar),
    ]
}

fn on_tile(prims: Vec<Prim>) -> Vec<Prim> {
    let mut v = vec![rr(0.0, 0.0, 32.0, 32.0, 7.5, Role::Tile)];
    let k = 0.74;
    let t = |p: (f32, f32)| (16.0 + (p.0 - 16.0) * k, 16.0 + (p.1 - 16.0) * k + 0.3);
    for p in prims {
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

fn blit(img: &mut [[u8; 4]], w: u32, x0: u32, y0: u32, prims: &[Prim], px: u32, pal: &Palette) {
    let ss = if px <= 64 { 8 } else { 4 };
    let pix: Vec<Rgba> = rasterize(prims, (0.0, 0.0), (32.0, 32.0), (px, px), pal, ss);
    for y in 0..px {
        for x in 0..px {
            let p = pix[(y * px + x) as usize];
            let a = p.a as f32 / 255.0;
            let d = &mut img[((y0 + y) * w + x0 + x) as usize];
            d[0] = (p.r as f32 * a + d[0] as f32 * (1.0 - a)) as u8;
            d[1] = (p.g as f32 * a + d[1] as f32 * (1.0 - a)) as u8;
            d[2] = (p.b as f32 * a + d[2] as f32 * (1.0 - a)) as u8;
        }
    }
}

fn fill(img: &mut [[u8; 4]], w: u32, x0: u32, y0: u32, cw: u32, ch: u32, c: [u8; 3]) {
    for y in y0..y0 + ch {
        for x in x0..x0 + cw {
            img[(y * w + x) as usize] = [c[0], c[1], c[2], 255];
        }
    }
}

fn main() {
    let out = std::env::args().nth(1).expect("vystupny .png");
    let variants: [fn(bool) -> Vec<Prim>; 4] = [a, b, c, d];
    let col = 320u32;
    let (w, h) = (col * 4, 560u32);
    let mut img = vec![[0u8; 4]; (w * h) as usize];
    let dark = [0x2Bu8, 0x2F, 0x36];
    let light = [0xE6u8, 0xE9, 0xED];
    let bar = [0x10u8, 0x12, 0x16];
    for (vi, f) in variants.iter().enumerate() {
        let x0 = vi as u32 * col;
        // 1) ikona aplikacie 160 px na tmavom
        fill(&mut img, w, x0, 0, col, 200, dark);
        blit(
            &mut img,
            w,
            x0 + (col - 160) / 2,
            20,
            &on_tile(f(false)),
            160,
            &Palette::APP,
        );
        // 2) znacka bez dlazdice 96 px na svetlom
        fill(&mut img, w, x0, 200, col, 140, light);
        blit(
            &mut img,
            w,
            x0 + (col - 96) / 2,
            222,
            &f(false),
            96,
            &Palette::ON_LIGHT,
        );
        // 3) skutocne male velkosti na "paneli uloh" (tmavy) a svetlom: 16, 24, 32, 48
        fill(&mut img, w, x0, 340, col, 110, bar);
        fill(&mut img, w, x0, 450, col, 110, light);
        let mut x = x0 + 30;
        for px in [16u32, 24, 32, 48] {
            let small = px < 24;
            blit(
                &mut img,
                w,
                x,
                340 + (110 - px) / 2,
                &f(small),
                px,
                &Palette::ON_DARK,
            );
            blit(
                &mut img,
                w,
                x,
                450 + (110 - px) / 2,
                &f(small),
                px,
                &Palette::ON_LIGHT,
            );
            x += px + 30;
        }
    }
    // oddelovace stlpcov
    for vi in 1..4u32 {
        fill(&mut img, w, vi * col, 0, 2, h, [0x80, 0x80, 0x80]);
    }
    let rgb: Vec<u8> = img.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    std::fs::write(out, transom_render::offscreen::png(w, h, &rgb)).unwrap();
}
