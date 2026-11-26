//! Izolovana kontrola vzhladu pri 100/150/200 % bez zmeny systemoveho DPI: skutocny `render_bar` cez skutocny
//! `GdiPainter` do pamatoveho DC (DIB section) a ulozenie do BMP.
//!   cargo run -p transom-render --example preview -- <vystupny-adresar>
#[cfg(windows)]
fn main() {
    use std::sync::Arc;
    use transom_core::image::Image;
    use transom_core::item::{Action, ArtImage, Flex, Glyph, ItemId, Part, Side, StatusItem, Tone};
    use transom_render::{render_bar, GdiPainter, Interaction, Size, Theme};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };

    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    // syntetický "obal": diagonalny gradient + biely stvorec (overi zmensenie a farby)
    let mut art = Image::solid(96, 96, 0);
    for y in 0..96u32 {
        for x in 0..96u32 {
            let inner = (30..66).contains(&x) && (30..66).contains(&y);
            let rgb = if inner {
                0xF0F0F0
            } else {
                (x * 255 / 95) << 16 | 0x40 << 8 | (y * 255 / 95)
            };
            art.px[(y * 96 + x) as usize] = 0xFF00_0000 | rgb;
        }
    }
    let art = ArtImage {
        id: 1,
        image: Arc::new(art),
    };
    // kruh AI: (vypln, proti smeru, stlmeny, text)
    let ring = |fill: Option<u8>, ccw: bool, muted: bool, label: &str| Part::Ring {
        fill,
        ccw,
        muted,
        alert: false,
        label: label.into(),
        name: String::new(),
        action: Some(Action::AiPanel),
    };
    let items = |playing: bool, with_art: bool, title: &str, ai: Part| {
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
                        art: with_art.then(|| art.clone()),
                        action: Some(Action::MediaOpenSource),
                    },
                    Part::button(Glyph::Prev, Action::MediaPrev, false, ""),
                    Part::button(
                        if playing { Glyph::Pause } else { Glyph::Play },
                        Action::MediaToggle,
                        true,
                        "",
                    ),
                    Part::button(Glyph::Next, Action::MediaNext, true, ""),
                    Part::Text {
                        text: title.into(),
                        flex: Some(Flex {
                            min_lp: 60,
                            max_lp: 420,
                        }),
                        tone: Tone::Normal,
                        action: None,
                    },
                    Part::Text {
                        text: "1/2".into(),
                        flex: None,
                        tone: Tone::Muted,
                        action: Some(Action::MediaCycle),
                    },
                ],
            ),
            StatusItem::new(
                ItemId::Ai,
                Side::Right,
                vec![
                    ai,
                    Part::Text {
                        text: "Claude".into(),
                        flex: Some(Flex {
                            min_lp: 400,
                            max_lp: 400,
                        }),
                        tone: Tone::Muted,
                        action: Some(Action::AiPanel),
                    },
                ],
            ),
            StatusItem::new(ItemId::Clock, Side::Right, vec![Part::text("14:05")]),
        ]
    };
    for (dpi, pct) in [(96u32, 100), (144, 150), (192, 200)] {
        for (theme, tname) in [(Theme::dark(), "dark"), (Theme::light(), "light")] {
            let title = "Miss You – southstar";
            let used = || ring(Some(62), false, false, "62 % used");
            let states = [
                ("normal", true, true, title, used(), Interaction::default()),
                (
                    "hover-next",
                    true,
                    false,
                    title,
                    used(),
                    Interaction {
                        hovered: Some((ItemId::Media, 3)),
                        pressed: None,
                        ..Default::default()
                    },
                ),
                (
                    "pressed-play",
                    false,
                    true,
                    title,
                    used(),
                    Interaction {
                        hovered: Some((ItemId::Media, 2)),
                        pressed: Some((ItemId::Media, 2)),
                        ..Default::default()
                    },
                ),
                (
                    "ring-left",
                    true,
                    true,
                    title,
                    ring(Some(38), true, false, "38 % left"),
                    Interaction::default(),
                ),
                (
                    "ring-last",
                    true,
                    true,
                    title,
                    ring(Some(62), false, true, "~62 % used"),
                    Interaction::default(),
                ),
                (
                    "ring-unavailable",
                    true,
                    true,
                    title,
                    ring(None, false, true, "5 h: unavailable"),
                    Interaction::default(),
                ),
                (
                    "ring-hover",
                    true,
                    true,
                    title,
                    used(),
                    Interaction {
                        hovered: Some((ItemId::Ai, 0)),
                        pressed: None,
                        ..Default::default()
                    },
                ),
                (
                    "ring-pressed",
                    true,
                    true,
                    title,
                    used(),
                    Interaction {
                        hovered: Some((ItemId::Ai, 0)),
                        pressed: Some((ItemId::Ai, 0)),
                        ..Default::default()
                    },
                ),
                (
                    "cjk-ja",
                    true,
                    true,
                    "夜に駆ける – YOASOBI",
                    ring(Some(62), false, false, "62 % 使用"),
                    Interaction::default(),
                ),
                (
                    "cjk-zh",
                    true,
                    true,
                    "晴天 – 周杰伦",
                    ring(Some(38), true, false, "剩余 38 %"),
                    Interaction::default(),
                ),
            ];
            for (sname, playing, with_art, title, ai, ui) in states {
                let w = (560 * dpi / 96) as i32;
                let h = (28 * dpi / 96) as i32;
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
                    let Ok(bmp) =
                        CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                    else {
                        eprintln!("CreateDIBSection zlyhal");
                        return;
                    };
                    let old = SelectObject(dc, bmp.into());
                    let mut p = GdiPainter::new(dc, Size { w, h }, dpi);
                    render_bar(&mut p, &theme, &items(playing, with_art, title, ai), ui);
                    // BMP subor (32 bpp, zhora nadol)
                    let data = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                    let mut f = Vec::with_capacity(54 + data.len());
                    f.extend_from_slice(b"BM");
                    f.extend_from_slice(&(54 + data.len() as u32).to_le_bytes());
                    f.extend_from_slice(&[0; 4]);
                    f.extend_from_slice(&54u32.to_le_bytes());
                    f.extend_from_slice(&40u32.to_le_bytes());
                    f.extend_from_slice(&w.to_le_bytes());
                    f.extend_from_slice(&(-h).to_le_bytes());
                    f.extend_from_slice(&1u16.to_le_bytes());
                    f.extend_from_slice(&32u16.to_le_bytes());
                    f.extend_from_slice(&[0; 24]);
                    f.extend_from_slice(data);
                    let path = format!("{out}/preview-{pct}-{tname}-{sname}.bmp");
                    if let Err(e) = std::fs::write(&path, f) {
                        eprintln!("{path}: {e}");
                    } else {
                        println!("{path} {w}x{h}");
                    }
                    SelectObject(dc, old);
                    let _ = DeleteObject(bmp.into());
                    let _ = DeleteDC(dc);
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {}
