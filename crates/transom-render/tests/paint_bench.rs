//! Meranie ceny kreslenia listy (nie test spravania): skutocny `GdiPainter` do pamatoveho DIB, typicka lista
//! (hudba s dlhym nazvom, ostrov s ekvalizerom, kruh AI, zvuk, bateria, hodiny). Meria cas na snimku, pocet alokacii
//! na snimku a pocet GDI objektov procesu (unik). Spustenie:
//! `cargo test --release -p transom-render --test paint_bench -- --ignored --nocapture`
#![cfg(windows)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use transom_core::item::{
    Action, AiLogo, Flex, Glyph, ItemId, Mark, Part, Scroll, Seg, Side, StatusItem, Tone,
};
use transom_render::offscreen::Surface;
use transom_render::{render_bar_with, GdiPainter, Interaction, MarqueeDraw, Size, Theme};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: preposlane systemovemu alokatoru s tym istym rozlozenim
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: ako vyssie
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: ako vyssie
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Pocet GDI objektov aj pocitadlo alokacii su spolocne pre cely proces: testy tohto suboru sa nesmu prekryvat (inak
/// by sucasne beziaci `pixel_hashes` nafukol "unik" aj alokacie na snimku).
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gdi_objects() -> u32 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetGuiResources(h: isize, flags: u32) -> u32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
    }
    // SAFETY: pseudo-handle aktualneho procesu; 0 = GR_GDIOBJECTS
    unsafe { GetGuiResources(GetCurrentProcess(), 0) }
}

fn items() -> Vec<StatusItem> {
    let s = |v: &str| v.to_string();
    // obal 300 x 300 (typicky nahlad zo Spotify / prehliadaca)
    let img = transom_core::image::Image {
        w: 300,
        h: 300,
        px: (0..300u32 * 300)
            .map(|i| 0xFF00_0000 | i.wrapping_mul(2654435761) & 0x00FF_FFFF)
            .collect(),
    };
    let art = Some(transom_core::item::ArtImage {
        id: 7,
        image: std::sync::Arc::new(img),
    });
    vec![
        StatusItem::new(
            ItemId::Search,
            Side::Left,
            vec![Part::button(
                Glyph::Search,
                Action::Launcher,
                true,
                "Search",
            )],
        ),
        StatusItem::new(
            ItemId::Media,
            Side::Left,
            vec![
                Part::Art {
                    art: art.clone(),
                    action: Some(Action::MediaOpenSource),
                },
                Part::Text {
                    text: s("An extremely long song title that never ends - Some Artist"),
                    flex: Some(Flex {
                        min_lp: 80,
                        max_lp: 260,
                    }),
                    tone: Tone::Normal,
                    action: None,
                },
                Part::button(Glyph::Prev, Action::MediaPrev, true, "Prev"),
                Part::button(Glyph::Pause, Action::MediaToggle, true, "Pause"),
                Part::button(Glyph::Next, Action::MediaNext, true, "Next"),
            ],
        ),
        StatusItem::new(
            ItemId::Shelf,
            Side::Center,
            vec![Part::Island {
                left: Seg::Art(art.clone()),
                mark: Mark::Transom,
                label: s("Spotify"),
                badge: s("3"),
                right: Seg::Eq {
                    levels: [40, 200, 120, 90, 250],
                    color: None,
                    playing: true,
                },
                name: s("island"),
                action: Some(Action::Island),
            }],
        ),
        StatusItem::new(
            ItemId::Ai,
            Side::Right,
            vec![Part::Ring {
                fill: Some(62),
                ccw: false,
                muted: false,
                alert: false,
                label: s("62 %"),
                name: s("AI"),
                action: Some(Action::AiPanel),
            }],
        ),
        StatusItem::new(
            ItemId::Audio,
            Side::Right,
            vec![Part::button(
                Glyph::Volume2,
                Action::AudioPanel,
                true,
                "Vol",
            )],
        ),
        StatusItem::new(
            ItemId::Tools,
            Side::Right,
            vec![Part::button(
                Glyph::Tools,
                Action::ToolsPanel,
                true,
                "Tools",
            )],
        ),
        StatusItem::new(
            ItemId::Battery,
            Side::Right,
            vec![
                Part::Battery {
                    percent: 78,
                    charging: false,
                },
                Part::text("78 %"),
            ],
        ),
        StatusItem::new(
            ItemId::Clock,
            Side::Right,
            vec![Part::text("Thu 26 Nov  14:05")],
        ),
    ]
}

struct Run {
    us: f64,
    allocs: f64,
}

/// `frames` snimok; `clip` = prekresluje sa len usek (ako snimka posuvu / ekvalizera), `passes` = 2 pre vrstvene okno.
fn run(frames: u32, clip: Option<(i32, i32)>, passes: u32, dpi: u32) -> Run {
    use windows::Win32::Graphics::Gdi::{IntersectClipRect, RestoreDC, SaveDC};
    let s = dpi as f32 / 96.0;
    let (w, h) = ((2560.0 * s) as i32, (32.0 * s) as i32);
    let surf = Surface::new(w, h);
    let th = Theme::dark();
    let its = items();
    // zahriatie (fonty, masky rohov, prvy layout)
    for _ in 0..5 {
        let mut p = GdiPainter::new(surf.dc, Size { w, h }, dpi);
        render_bar_with(&mut p, &th, &its, Interaction::default(), None);
    }
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let t0 = std::time::Instant::now();
    for f in 0..frames {
        for pass in 0..passes {
            // SAFETY: platny DC povrchu; SaveDC/RestoreDC parovo
            let saved = clip.map(|(x, cw)| unsafe {
                let sv = SaveDC(surf.dc);
                IntersectClipRect(surf.dc, x, 0, x + cw, h);
                sv
            });
            let mut p = GdiPainter::new(surf.dc, Size { w, h }, dpi).grayscale(passes == 2);
            let draw = Some(MarqueeDraw {
                item: ItemId::Media,
                part: 1,
                offset_px: (f % 200) as i32,
            });
            let _ = pass;
            render_bar_with(&mut p, &th, &its, Interaction::default(), draw);
            if let Some(sv) = saved {
                unsafe {
                    let _ = RestoreDC(surf.dc, sv);
                }
            }
        }
    }
    let dt = t0.elapsed();
    let a1 = ALLOCS.load(Ordering::Relaxed);
    Run {
        us: dt.as_secs_f64() * 1e6 / frames as f64,
        allocs: (a1 - a0) as f64 / frames as f64,
    }
}

#[test]
#[ignore]
fn paint_cost_per_frame() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let g0 = gdi_objects();
    for dpi in [96u32, 144] {
        let s = dpi as f32 / 96.0;
        let full = run(400, None, 1, dpi);
        // usek ostrova v strede (~ekvalizer) a usek nazvu skladby (~posuv)
        let island = run(
            1500,
            Some(((1180.0 * s) as i32, (200.0 * s) as i32)),
            1,
            dpi,
        );
        let title = run(1500, Some(((120.0 * s) as i32, (260.0 * s) as i32)), 1, dpi);
        let layered = run(
            1500,
            Some(((1180.0 * s) as i32, (200.0 * s) as i32)),
            2,
            dpi,
        );
        for (n, r) in [
            ("cela lista", full),
            ("usek ostrova", island),
            ("usek nazvu", title),
            ("ostrov 2 prechody", layered),
        ] {
            println!(
                "{dpi} dpi {n:>18}: {:8.1} us/snimka, {:7.1} alokacii/snimka",
                r.us, r.allocs
            );
        }
    }
    let g1 = gdi_objects();
    println!("GDI objekty: {g0} -> {g1}");
    assert!(g1 <= g0 + 64, "unik GDI objektov: {g0} -> {g1}");
}

fn island(left: Seg, mark: Mark, badge: &str, right: Seg) -> StatusItem {
    StatusItem::new(
        ItemId::Shelf,
        Side::Center,
        vec![Part::Island {
            left,
            mark,
            label: "transom".into(),
            badge: badge.into(),
            right,
            name: "i".into(),
            action: Some(Action::Island),
        }],
    )
}

fn ring(fill: Option<u8>, ccw: bool, muted: bool, alert: bool) -> StatusItem {
    StatusItem::new(
        ItemId::Ai,
        Side::Right,
        vec![Part::Ring {
            fill,
            ccw,
            muted,
            alert,
            label: "41 %".into(),
            name: "AI".into(),
            action: Some(Action::AiPanel),
        }],
    )
}

fn txt(t: &str, muted: bool, strong: bool, low: bool, scroll: Option<Scroll>) -> Seg {
    Seg::Text {
        text: t.into(),
        muted,
        strong,
        low,
        scroll,
    }
}

/// Sady poloziek na kontrolu pixelov: vsetky druhy casti, strany ostrova a znacky.
fn item_sets() -> Vec<(&'static str, Vec<StatusItem>)> {
    let base = items();
    let art = match &base[1].parts[0] {
        Part::Art { art, .. } => art.clone(),
        _ => None,
    };
    let sc = Some(Scroll {
        ms: 1234,
        speed_lp: 40,
        rtl: false,
    });
    let with = |extra: Vec<StatusItem>| {
        let mut v: Vec<StatusItem> = base
            .iter()
            .filter(|i| i.id != ItemId::Shelf && i.id != ItemId::Ai)
            .cloned()
            .collect();
        v.extend(extra);
        v
    };
    vec![
        ("base", base.clone()),
        (
            "texty",
            with(vec![
                island(
                    txt("Download finished", false, false, false, None),
                    Mark::Icon('\u{E896}'),
                    "12",
                    txt("report-final-v2.pdf and more", true, false, false, sc),
                ),
                ring(Some(0), false, false, false),
            ]),
        ),
        (
            "obal",
            with(vec![
                island(
                    Seg::Cover {
                        art: art.clone(),
                        text: "Song title that is long enough to scroll".into(),
                        scroll: sc,
                    },
                    Mark::Spark,
                    "",
                    Seg::Logo {
                        logo: AiLogo::Claude,
                        text: "73 %".into(),
                        low: true,
                        dot: Some((200, 120, 30)),
                    },
                ),
                ring(Some(100), true, false, true),
            ]),
        ),
        (
            "bez obalu",
            with(vec![
                island(
                    Seg::Cover {
                        art: None,
                        text: "Short".into(),
                        scroll: None,
                    },
                    Mark::Transom,
                    "3",
                    Seg::Logo {
                        logo: AiLogo::Codex,
                        text: String::new(),
                        low: false,
                        dot: None,
                    },
                ),
                ring(None, false, true, false),
            ]),
        ),
        (
            "eq",
            with(vec![
                island(
                    Seg::None,
                    Mark::Transom,
                    "",
                    Seg::Eq {
                        levels: [0, 255, 3, 128, 77],
                        color: Some((30, 200, 90)),
                        playing: false,
                    },
                ),
                ring(Some(62), true, true, false),
                StatusItem::new(
                    ItemId::Peripherals,
                    Side::Right,
                    vec![Part::Device {
                        glyph: Glyph::Mouse,
                        percent: Some(9),
                        charging: true,
                        muted: false,
                        low: true,
                        label: "9 %".into(),
                        name: "m".into(),
                        action: Some(Action::PeripheralsPanel),
                    }],
                ),
                StatusItem::new(
                    ItemId::Tools,
                    Side::Left,
                    vec![
                        Part::File {
                            glyph: Glyph::Shelf,
                            muted: false,
                            label: "4".into(),
                            name: "s".into(),
                            action: None,
                        },
                        Part::Divider,
                        Part::File {
                            glyph: Glyph::Folder,
                            muted: true,
                            label: "Projects".into(),
                            name: "f".into(),
                            action: None,
                        },
                        Part::Text {
                            text: "muted hint".into(),
                            flex: None,
                            tone: Tone::Muted,
                            action: None,
                        },
                    ],
                ),
                StatusItem::new(
                    ItemId::Battery,
                    Side::Right,
                    vec![Part::Battery {
                        percent: 12,
                        charging: true,
                    }],
                ),
            ]),
        ),
    ]
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn interactions() -> Vec<(&'static str, Interaction)> {
    vec![
        ("none", Interaction::default()),
        (
            "hover-ring",
            Interaction {
                hovered: Some((ItemId::Ai, 0)),
                ..Default::default()
            },
        ),
        (
            "press-audio",
            Interaction {
                pressed: Some((ItemId::Audio, 0)),
                hovered: Some((ItemId::Audio, 0)),
                ..Default::default()
            },
        ),
        (
            "hover-island",
            Interaction {
                hovered: Some((ItemId::Shelf, 0)),
                ..Default::default()
            },
        ),
        (
            "press-island",
            Interaction {
                pressed: Some((ItemId::Shelf, 0)),
                ..Default::default()
            },
        ),
        (
            "shift",
            Interaction {
                shift: (3, 1),
                gap: Some((500, 700)),
                ..Default::default()
            },
        ),
        (
            "nolines",
            Interaction {
                no_lines: true,
                ..Default::default()
            },
        ),
    ]
}

/// Odtlacky pixelov sceny a primitiv (na porovnanie pred a po zmene renderera na tom istom stroji; zavisia od pisiem
/// stroja, preto sa neukladaju do repozitara). Vypise suhrn; s `TRANSOM_HASH_OUT` zapise riadky `nazov hash` do suboru.
#[test]
#[ignore]
fn pixel_hashes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    use transom_render::{Color, Painter, Rect};
    use windows::Win32::Graphics::Gdi::{IntersectClipRect, RestoreDC, SaveDC};
    let mut out = String::new();
    let sets = item_sets();
    for dpi in [96u32, 120, 144, 192] {
        let s = dpi as f32 / 96.0;
        let (w, h) = ((1600.0 * s) as i32, (32.0 * s) as i32);
        for (dark, th0) in [(true, Theme::dark()), (false, Theme::light())] {
            for (gray, bg) in [
                (false, None),
                (true, Some(Color::rgb(0, 0, 0))),
                (true, Some(Color::rgb(255, 255, 255))),
            ] {
                let mut th = th0;
                if let Some(c) = bg {
                    th.background = c;
                }
                for (sn, its) in &sets {
                    for (un, ui) in interactions() {
                        for off in [None, Some(37)] {
                            let surf = Surface::new(w, h);
                            let draw = off.map(|o| MarqueeDraw {
                                item: ItemId::Media,
                                part: 1,
                                offset_px: o,
                            });
                            let mut p =
                                GdiPainter::new(surf.dc, Size { w, h }, dpi).grayscale(gray);
                            render_bar_with(&mut p, &th, its, ui, draw);
                            // prekreslenie vyrezu (snimka posuvu / ekvalizera) navrch
                            // SAFETY: platny DC povrchu; SaveDC/RestoreDC parovo
                            unsafe {
                                let sv = SaveDC(surf.dc);
                                IntersectClipRect(surf.dc, w / 2 - 90, 0, w / 2 + 70, h);
                                let mut p =
                                    GdiPainter::new(surf.dc, Size { w, h }, dpi).grayscale(gray);
                                render_bar_with(&mut p, &th, its, ui, draw);
                                let _ = RestoreDC(surf.dc, sv);
                            }
                            out.push_str(&format!(
                                "{dpi} {} {gray} {bg:?} {sn} {un} {off:?} {:016x}\n",
                                if dark { "dark" } else { "light" },
                                fnv(&surf.rgb())
                            ));
                        }
                    }
                }
            }
        }
    }
    // primitiva: zaoblene vyplne a obrysy roznych velkosti, masky a RGBA aj s orezanim a mimo povrchu
    let surf = Surface::new(300, 200);
    let mut p = GdiPainter::new(surf.dc, Size { w: 300, h: 200 }, 96);
    p.fill_rect(Rect::new(0, 0, 300, 200), Color::rgb(40, 50, 60));
    let mut x = 0;
    for r in [0, 1, 2, 3, 5, 8, 13, 40] {
        p.fill_round_rect(Rect::new(x, 2, 30, 20), r, Color::rgb(200, 100, 50));
        p.stroke_round_rect(Rect::new(x, 30, 30, 20), r, 1, Color::rgb(20, 230, 50));
        p.stroke_round_rect(Rect::new(x, 60, 30, 20), r, 3, Color::rgb(220, 230, 250));
        x += 35;
    }
    p.push_clip(Rect::new(10, 90, 100, 30));
    p.fill_round_rect(Rect::new(0, 85, 300, 50), 12, Color::rgb(90, 0, 200));
    p.pop_clip();
    let cov: Vec<u8> = (0..64 * 32).map(|i| (i * 7 % 256) as u8).collect();
    p.blend_mask(Rect::new(5, 130, 64, 32), &cov, Color::rgb(255, 255, 0));
    let px: Vec<transom_render::painter::Rgba> = (0..50 * 30)
        .map(|i| transom_render::painter::Rgba {
            r: (i % 256) as u8,
            g: (i * 3 % 256) as u8,
            b: 99,
            a: (i * 11 % 256) as u8,
        })
        .collect();
    p.blend_rgba(Rect::new(100, 130, 50, 30), &px);
    p.blend_rgba(Rect::new(280, 180, 50, 30), &px);
    p.blend_mask(Rect::new(-10, -5, 64, 32), &cov, Color::rgb(0, 255, 255));
    // velka maska (nad hranicou docasneho buffra)
    let big: Vec<u8> = (0..300 * 200).map(|i| (i % 251) as u8).collect();
    p.blend_mask(Rect::new(0, 0, 300, 200), &big, Color::rgb(10, 20, 30));
    out.push_str(&format!("primitiva {:016x}\n", fnv(&surf.rgb())));
    let lines = out.lines().count();
    if let Ok(path) = std::env::var("TRANSOM_HASH_OUT") {
        std::fs::write(path, &out).expect("zapis odtlackov");
    }
    println!("{lines} odtlackov, suhrn {:016x}", fnv(out.as_bytes()));
}
