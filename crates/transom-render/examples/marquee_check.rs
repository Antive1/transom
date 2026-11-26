//! Prisna kontrola posuvneho textu cez skutocne GDI pri 96/144/192 DPI v oboch temach (bez zmeny systemoveho DPI):
//! (1) mimo miesta textu je kazdy pixel identicky s vykreslenim bez nazvu (tlacidla, obal a hodiny stoja),
//! (2) v mieste textu sa nieco kresli pri kazdom posune (okno ukazuje cast textu) a
//! (3) rozne posuny davaju rozne obrazky (text sa naozaj hybe).
//! Uklada aj BMP stredneho snimku. Pouzitie: cargo run -p transom-render --example marquee_check -- <adresar>
#[cfg(windows)]
fn main() {
    use transom_core::item::{Action, Flex, Glyph, ItemId, Part, Side, StatusItem, Tone};
    use transom_render::{render_bar_with, GdiPainter, Interaction, MarqueeDraw, Size, Theme};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };

    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let items = |title: &str| {
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
                        text: title.into(),
                        flex: Some(Flex {
                            min_lp: 60,
                            max_lp: 220,
                        }),
                        tone: Tone::Normal,
                        action: None,
                    },
                ],
            ),
            StatusItem::new(ItemId::Clock, Side::Right, vec![Part::text("17:25")]),
        ]
    };
    let title =
        "Skladba s dlhym nazvom, ktory sa nezmesti – Interpret s dlhym menom (Remastered 2026)";
    let mut failures = 0;
    for dpi in [96u32, 144, 192] {
        let (w, h) = ((560 * dpi / 96) as i32, (28 * dpi / 96) as i32);
        for (theme, tname) in [(Theme::dark(), "dark"), (Theme::light(), "light")] {
            let render = |t: &str,
                          m: Option<MarqueeDraw>|
             -> (Vec<u32>, transom_render::RenderInfo) {
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
                    let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                        .unwrap();
                    SelectObject(dc, bmp.into());
                    let mut p = GdiPainter::new(dc, Size { w, h }, dpi);
                    let info =
                        render_bar_with(&mut p, &theme, &items(t), Interaction::default(), m);
                    let px =
                        std::slice::from_raw_parts(bits as *const u32, (w * h) as usize).to_vec();
                    let _ = DeleteObject(bmp.into());
                    let _ = DeleteDC(dc);
                    (px, info)
                }
            };
            let (_, info) = render(title, None);
            let Some(ov) = info.overflow.first().copied() else {
                println!("{dpi} {tname}: CHYBA - nazov nepretieka");
                failures += 1;
                continue;
            };
            let (empty, _) = render("", None);
            let mut frames = Vec::new();
            for off in [0, ov.overflow_px / 2, ov.overflow_px] {
                let m = MarqueeDraw {
                    item: ItemId::Media,
                    part: ov.part,
                    offset_px: off,
                };
                let (img, _) = render(title, Some(m));
                let (mut outside_diff, mut inside_ink) = (0, 0);
                for y in 0..h {
                    for x in 0..w {
                        let i = (y * w + x) as usize;
                        let inside = x >= ov.x && x < ov.x + ov.w;
                        if img[i] != empty[i] {
                            if inside {
                                inside_ink += 1;
                            } else {
                                outside_diff += 1;
                            }
                        }
                    }
                }
                let ok = outside_diff == 0 && inside_ink > 0;
                println!(
                    "{dpi:3} {tname:5} posun {off:4}/{}: mimo={outside_diff} v_texte={inside_ink} {}",
                    ov.overflow_px,
                    if ok { "OK" } else { "CHYBA" }
                );
                if !ok {
                    failures += 1;
                }
                frames.push((off, img));
            }
            let distinct = frames[0].1 != frames[1].1
                && frames[1].1 != frames[2].1
                && frames[0].1 != frames[2].1;
            println!(
                "{dpi:3} {tname:5} posuny davaju rozne snimky: {}",
                if distinct { "OK" } else { "CHYBA" }
            );
            if !distinct {
                failures += 1;
            }
            let (off, img) = &frames[1];
            let mut f = Vec::with_capacity(54 + img.len() * 4);
            f.extend_from_slice(b"BM");
            f.extend_from_slice(&(54 + img.len() as u32 * 4).to_le_bytes());
            f.extend_from_slice(&[0; 4]);
            f.extend_from_slice(&54u32.to_le_bytes());
            f.extend_from_slice(&40u32.to_le_bytes());
            f.extend_from_slice(&w.to_le_bytes());
            f.extend_from_slice(&(-h).to_le_bytes());
            f.extend_from_slice(&1u16.to_le_bytes());
            f.extend_from_slice(&32u16.to_le_bytes());
            f.extend_from_slice(&[0; 24]);
            for px in img {
                f.extend_from_slice(&px.to_le_bytes());
            }
            let _ = std::fs::write(format!("{out}/marquee-{dpi}-{tname}-mid{off}.bmp"), f);
        }
    }
    println!(
        "{}",
        if failures == 0 {
            "VSETKO OK".to_string()
        } else {
            format!("CHYBY: {failures}")
        }
    );
    if failures > 0 {
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {}
