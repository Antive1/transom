//! Overenie priehladneho pozadia listy (dvojprechodove odvodenie alfy, pozri `transom-platform::layered`) bez zmeny DPI:
//! skutocna scena (`render_bar_with`) cez skutocny `GdiPainter` so sivym vyhladzovanim sa vykresli na cierne a biele
//! pozadie, zlozi sa na premultiplied ARGB a vysledok nad roznymi podkladmi sa porovna s **priamym** vykreslenim tej
//! istej sceny na ten isty podklad. Zaroven ulozi BMP nad "tapetou" (gradient + sum) pre vizualnu kontrolu.
//!   cargo run -p transom-render --example alpha_check -- <vystupny-adresar>
//! Vypise najvacsi a priemerny rozdiel na kanal (0..255) pre kazde DPI, temu a nepriehladnost.
#[cfg(windows)]
fn main() {
    use transom_core::item::{Action, Flex, Glyph, ItemId, Part, Side, StatusItem, Tone};
    use transom_platform::layered::{Area, LayeredBuf};
    use transom_render::{
        render_bar_with, Color, GdiPainter, Interaction, MarqueeDraw, Size, Theme,
    };
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC,
    };

    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let items = || {
        vec![
            StatusItem::new(
                ItemId::Search,
                Side::Left,
                vec![Part::button(Glyph::Search, Action::Search, true, "")],
            ),
            StatusItem::new(
                ItemId::Media,
                Side::Left,
                vec![
                    Part::Art {
                        art: None,
                        action: Some(Action::MediaOpenSource),
                    },
                    Part::button(Glyph::Prev, Action::MediaPrev, false, ""),
                    Part::button(Glyph::Pause, Action::MediaToggle, true, ""),
                    Part::button(Glyph::Next, Action::MediaNext, true, ""),
                    Part::Text {
                        text: "Skladba s dlhym nazvom, ktory sa nezmesti – Interpret".into(),
                        flex: Some(Flex {
                            min_lp: 60,
                            max_lp: 220,
                        }),
                        tone: Tone::Normal,
                        action: None,
                    },
                ],
            ),
            StatusItem::new(
                ItemId::Ai,
                Side::Right,
                vec![Part::Ring {
                    fill: Some(62),
                    ccw: false,
                    muted: false,
                    alert: false,
                    label: "62 % využité".into(),
                    name: String::new(),
                    action: Some(Action::AiPanel),
                }],
            ),
            StatusItem::new(ItemId::Clock, Side::Right, vec![Part::text("17:25")]),
        ]
    };

    unsafe fn dib(w: i32, h: i32) -> (HDC, windows::Win32::Graphics::Gdi::HBITMAP, *mut u32) {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp =
                CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).expect("DIB");
            SelectObject(dc, bmp.into());
            (dc, bmp, bits as *mut u32)
        }
    }

    // podklady (to, cez co presvitá lista): jasny, stredny, tmavy, farebny
    let backdrops: [(u8, u8, u8); 4] = [
        (245, 245, 245),
        (128, 128, 128),
        (20, 20, 20),
        (30, 90, 170),
    ];
    let mut worst_overall = 0i32;
    let mut csv = String::from("dpi,tema,alfa,podklad,max_rozdiel,priemer\n");
    for dpi in [96u32, 144, 192] {
        let (w, h) = ((560 * dpi / 96) as i32, (28 * dpi / 96) as i32);
        for (theme, tname) in [(Theme::dark(), "dark"), (Theme::light(), "light")] {
            for alpha in [255u8, 200, 128, 64, 0] {
                let mut buf = LayeredBuf::new(w, h).expect("buf");
                let bgc = theme.background;
                // dva prechody (cierne a biele pozadie) so sivym vyhladzovanim, rovnaky posun posuvneho textu
                for (dc, backdrop) in [
                    (buf.black_dc(), (0u8, 0u8, 0u8)),
                    (buf.white_dc(), (255, 255, 255)),
                ] {
                    let mut p = GdiPainter::new(dc, Size { w, h }, dpi).grayscale(true);
                    let mut t = theme;
                    t.background = Color::rgb(backdrop.0, backdrop.1, backdrop.2);
                    render_bar_with(
                        &mut p,
                        &t,
                        &items(),
                        Interaction::default(),
                        Some(MarqueeDraw {
                            item: ItemId::Media,
                            part: 4,
                            offset_px: 37 * dpi as i32 / 96,
                        }),
                    );
                }
                buf.compose(Area::full(w, h), (bgc.r, bgc.g, bgc.b), alpha);
                let argb = buf.output().to_vec();
                for k in backdrops {
                    // ocakavanie: ta ista scena vykreslena PRIAMO na pozadie = (farba pozadia s alfou) nad podkladom k
                    let a = alpha as f32 / 255.0;
                    let eff = Color::rgb(
                        (bgc.r as f32 * a + k.0 as f32 * (1.0 - a)).round() as u8,
                        (bgc.g as f32 * a + k.1 as f32 * (1.0 - a)).round() as u8,
                        (bgc.b as f32 * a + k.2 as f32 * (1.0 - a)).round() as u8,
                    );
                    let (dc, bmp, bits) = unsafe { dib(w, h) };
                    let mut p = GdiPainter::new(dc, Size { w, h }, dpi).grayscale(true);
                    let mut t = theme;
                    t.background = eff;
                    render_bar_with(
                        &mut p,
                        &t,
                        &items(),
                        Interaction::default(),
                        Some(MarqueeDraw {
                            item: ItemId::Media,
                            part: 4,
                            offset_px: 37 * dpi as i32 / 96,
                        }),
                    );
                    let direct =
                        unsafe { std::slice::from_raw_parts(bits, (w * h) as usize) }.to_vec();
                    let (mut worst, mut sum, mut n) = (0i32, 0i64, 0i64);
                    for (px, d) in argb.iter().zip(direct.iter()) {
                        let aa = (px >> 24) as f32 / 255.0;
                        let ch = |v: u32, s: u32| ((v >> s) & 0xFF) as f32;
                        for s in [16u32, 8, 0] {
                            let over = ch(*px, s)
                                + [k.0, k.1, k.2][(2 - s as usize / 8) % 3] as f32 * (1.0 - aa);
                            let diff = (over - ch(*d, s)).abs().round() as i32;
                            worst = worst.max(diff);
                            sum += diff as i64;
                            n += 1;
                        }
                    }
                    let mean = sum as f64 / n as f64;
                    worst_overall = worst_overall.max(worst);
                    csv.push_str(&format!("{dpi},{tname},{alpha},{k:?},{worst},{mean:.3}\n"));
                    unsafe {
                        let _ = DeleteObject(bmp.into());
                        let _ = DeleteDC(dc);
                    }
                }
                // nahlad nad "tapetou": gradient jasny -> tmavy + jemny sum, BMP (BGRX, zhora nadol)
                let mut bmp = vec![0u32; (w * h) as usize];
                let mut seed = 12345u32;
                for y in 0..h {
                    for x in 0..w {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        let g = 235.0 - 215.0 * (x as f32 / w as f32);
                        let noise = ((seed >> 24) as f32 / 255.0 - 0.5) * 24.0;
                        let base = [
                            g + noise,
                            g * 0.9 + noise,
                            (g * 0.8 + 40.0 + noise).min(255.0),
                        ];
                        let px = argb[(y * w + x) as usize];
                        let aa = (px >> 24) as f32 / 255.0;
                        let c = |v: f32, s: u32| {
                            (((px >> s) & 0xFF) as f32 + v.clamp(0.0, 255.0) * (1.0 - aa))
                                .round()
                                .clamp(0.0, 255.0) as u32
                        };
                        bmp[(y * w + x) as usize] =
                            c(base[0], 16) << 16 | c(base[1], 8) << 8 | c(base[2], 0);
                    }
                }
                let mut f = Vec::with_capacity(54 + bmp.len() * 4);
                f.extend_from_slice(b"BM");
                f.extend_from_slice(&(54 + bmp.len() as u32 * 4).to_le_bytes());
                f.extend_from_slice(&[0; 4]);
                f.extend_from_slice(&54u32.to_le_bytes());
                f.extend_from_slice(&40u32.to_le_bytes());
                f.extend_from_slice(&w.to_le_bytes());
                f.extend_from_slice(&(-h).to_le_bytes());
                f.extend_from_slice(&1u16.to_le_bytes());
                f.extend_from_slice(&32u16.to_le_bytes());
                f.extend_from_slice(&[0; 24]);
                for px in &bmp {
                    f.extend_from_slice(&px.to_le_bytes());
                }
                let path = format!("{out}/alpha-{dpi}-{tname}-{alpha}.bmp");
                if let Err(e) = std::fs::write(&path, f) {
                    eprintln!("{path}: {e}");
                }
            }
        }
    }
    let _ = std::fs::write(format!("{out}/alpha_check.csv"), &csv);
    print!("{csv}");
    println!("NAJVACSI ROZDIEL NA KANAL: {worst_overall}");
}

#[cfg(not(windows))]
fn main() {}
