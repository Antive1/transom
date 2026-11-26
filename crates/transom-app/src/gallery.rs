//! Galeria vzhladu (len testy; rucna vizualna kontrola): kazdy povrch aplikacie pri 100/150/200 % v oboch temach do
//! priecinka `TRANSOM_GALLERY`:
//!   `TRANSOM_GALLERY=<adresar> cargo test -p transom-app gallery_ -- --ignored --test-threads=1`
//! Kresli sa skutocnym `GdiPainter` do pamate (bez okna, bez zmeny DPI / jazyka / temy systemu) a iba z vyrobenych dat:
//! nikdy zo skutocnych priecinkov, suborov ani konfiguracie pouzivatela. Bez premennej sa nic nezapisuje.
use std::path::{Path, PathBuf};
use transom_render::offscreen::Surface;
use transom_render::Theme;

pub const DPIS: [u32; 3] = [96, 144, 192];

/// Cielovy priecinok (`None` = galeria sa nevytvara). `TRANSOM_GALLERY_FONT=classic` vykresli galeriu zaloznym pismom
/// (Segoe UI), ako na stroji bez Segoe UI Variable.
pub fn dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("TRANSOM_GALLERY")?);
    let _ = std::fs::create_dir_all(&d);
    if std::env::var("TRANSOM_GALLERY_FONT").as_deref() == Ok("classic") {
        transom_render::fonts::force(Some(transom_render::fonts::Family::Classic));
    }
    // `TRANSOM_GALLERY_LANG=ja` (napr.): galeria v inom jazyku (testy inak bezia po anglicky)
    if let Some(l) = std::env::var("TRANSOM_GALLERY_LANG")
        .ok()
        .and_then(|c| transom_i18n::Lang::from_tag(&c))
    {
        transom_i18n::set_lang(l);
    }
    Some(d)
}

pub fn themes() -> [(bool, Theme); 2] {
    [(true, Theme::dark()), (false, Theme::light())]
}

pub fn tag(dark: bool) -> &'static str {
    if dark {
        "dark"
    } else {
        "light"
    }
}

/// lp -> px pri danom DPI.
pub fn px(lp: i32, dpi: u32) -> i32 {
    (lp as f32 * dpi as f32 / 96.0).round() as i32
}

pub fn save(dir: &Path, name: &str, surf: &Surface) {
    let _ = std::fs::write(
        dir.join(format!("{name}.png")),
        transom_render::offscreen::png(surf.w as u32, surf.h as u32, &surf.rgb()),
    );
}

/// Sklo ako v skutocnom okne: obsah sa nakresli dvakrat (nad ciernym a bielym pozadim, `Theme::glass_pass`) a zlozi
/// nad nadych skla (`glass::compose`). Vysledok je RGBA (nepremultiplikovane) - priehladny tam, kde je vidno pozadie.
/// Rohy a vyduty prechod do listy ako v aplikacii (`corner_lp`, `flare_lp`; panel 8 / 0 alebo 8 / 10 pri liste, menu
/// ostrova 14 / 10): pri prechode je obraz o `2 * flare` sirsi a obsah posunuty o `flare`. Vracia (sirka, RGBA).
#[allow(clippy::too_many_arguments)]
pub fn glass_rgba(
    w: i32,
    h: i32,
    dpi: u32,
    th: &Theme,
    rim: transom_platform::glass::Rim,
    corner_lp: i32,
    flare_lp: i32,
    paint: &mut dyn FnMut(&mut dyn transom_render::Painter, &Theme),
) -> (i32, Vec<u8>) {
    glass_rgba_tinted(w, h, dpi, th, rim, corner_lp, flare_lp, None, paint)
}

/// Ako [`glass_rgba`], so sklom vo vlastnej farbe listy (`glass::tint_for`).
#[allow(clippy::too_many_arguments)]
pub fn glass_rgba_tinted(
    w: i32,
    h: i32,
    dpi: u32,
    th: &Theme,
    rim: transom_platform::glass::Rim,
    corner_lp: i32,
    flare_lp: i32,
    tint: Option<transom_platform::glass::Tinted>,
    paint: &mut dyn FnMut(&mut dyn transom_render::Painter, &Theme),
) -> (i32, Vec<u8>) {
    use transom_platform::glass::Rim;
    let attached = rim == Rim::Attached;
    let fl = if attached {
        flare_lp * dpi as i32 / 96
    } else {
        0
    };
    let fw = w + 2 * fl;
    let mut run = |bg: transom_render::Color| -> Vec<u32> {
        let surf = Surface::new(fw, h);
        {
            let mut p =
                transom_render::GdiPainter::new(surf.dc, transom_render::Size { w: fw, h }, dpi);
            // pasy vyduteho prechodu su ciste sklo (podklad prechodu)
            transom_render::Painter::fill_rect(&mut p, transom_render::Rect::new(0, 0, fw, h), bg);
            let mut sub = transom_render::painter::Sub {
                inner: &mut p,
                dx: fl,
                dy: 0,
                size: transom_render::Size { w, h },
            };
            paint(&mut sub, &th.glass_pass(bg));
            if attached {
                // horny lem obsahu by pri liste vyzeral ako ciara: horne riadky su ciste sklo (ako v aplikacii)
                let n = (dpi as i32 / 96).max(1);
                transom_render::Painter::fill_rect(
                    &mut p,
                    transom_render::Rect::new(0, 0, fw, n),
                    bg,
                );
            }
        }
        surf.rgb()
            .chunks(3)
            .map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)
            .collect()
    };
    let k = run(transom_render::Color::rgb(0, 0, 0));
    let wh = run(transom_render::Color::rgb(255, 255, 255));
    let mut st = transom_platform::glass::GlassStyle::for_theme(
        (th.background.r, th.background.g, th.background.b),
        th.dark,
        rim,
    );
    st.corner = corner_lp * dpi as i32 / 96;
    st.flare = fl;
    if let Some(t) = tint {
        st.tint = t.tint;
        st.tint_alpha = t.alpha;
    }
    let mut out = vec![0u32; (fw * h) as usize];
    transom_platform::glass::compose(
        &k,
        &wh,
        &mut out,
        fw,
        h,
        transom_platform::layered::Area::full(fw, h),
        &st,
        255,
    );
    let mut rgba = Vec::with_capacity(out.len() * 4);
    for v in out {
        let a = v >> 24;
        let un = |c: u32| (c * 255 + a / 2).checked_div(a).unwrap_or(0).min(255) as u8;
        rgba.extend_from_slice(&[
            un((v >> 16) & 0xFF),
            un((v >> 8) & 0xFF),
            un(v & 0xFF),
            a as u8,
        ]);
    }
    (fw, rgba)
}

/// Ulozi RGBA ako `<name>.rgba` (hlavicka: sirka a vyska u32 LE, potom pixely) - na skladanie snimok nad tapetu.
pub fn save_rgba(dir: &Path, name: &str, w: i32, h: i32, rgba: &[u8]) {
    let mut f = Vec::with_capacity(8 + rgba.len());
    f.extend_from_slice(&(w as u32).to_le_bytes());
    f.extend_from_slice(&(h as u32).to_le_bytes());
    f.extend_from_slice(rgba);
    let _ = std::fs::write(dir.join(format!("{name}.rgba")), f);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::RingView;
    use crate::view::{arrange, build_items, AudioView, View};
    use std::sync::Arc;
    use transom_core::battery::Battery;
    use transom_core::image::Image;
    use transom_core::item::{ArtImage, Glyph, ItemId};
    use transom_core::media::{Controls, PlayState, SessionInfo};
    use transom_render::{render_bar_with, Color, GdiPainter, Interaction, Size};

    fn dump(
        out: &mut String,
        title: &str,
        a: (
            Vec<transom_platform::menu_access::MenuEntry>,
            Vec<transom_platform::menu_access::PageExtra>,
            Option<usize>,
            String,
        ),
    ) {
        out.push_str(&format!("== {title} root={:?} focus={:?}\n", a.3, a.2));
        for (e, x) in a.0.iter().zip(&a.1) {
            out.push_str(&format!(
                "{:?} role={} state={} value={:?} action={:?} enabled={}\n",
                e.name,
                x.role,
                x.state & !transom_platform::menu_access::STATE_OFFSCREEN,
                x.value,
                x.action,
                e.enabled
            ));
        }
    }

    /// Vypis pristupnosti popoverov Nastroje a Hladanie (nazov, rola, stav, hodnota, akcia) do `TRANSOM_GALLERY`
    /// (porovnanie pred a po zmene vzhladu).
    #[test]
    #[ignore]
    fn gallery_a11y_dump() {
        let _g = crate::test_lang::guard();
        let Some(dir) = dir() else { return };
        let mut out = String::new();
        for lang in [
            transom_i18n::Lang::En,
            transom_i18n::Lang::De,
            transom_i18n::Lang::Ja,
        ] {
            transom_i18n::set_lang(lang);
            for (name, p) in crate::tools_tests::a11y_samples() {
                dump(&mut out, &format!("{lang:?} tools {name}"), p);
            }
            for (name, p) in crate::launcher_pop::a11y_samples() {
                dump(&mut out, &format!("{lang:?} launcher {name}"), p);
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
        std::fs::write(dir.join("app-a11y.txt"), out).unwrap();
    }

    /// Harok komponentov: kazdy druh tlacidla v kazdom stave (pokoj, hover, stlacene, fokus, vybrane, potvrdenie, zakazane)
    /// na pozadi okna aj na karte, zaskrtavacie policka a karta - pri kazdom DPI a teme.
    #[test]
    #[ignore]
    fn gallery_components() {
        use transom_render::design::{self, Button, Kind, State};
        use transom_render::Painter;
        let _g = crate::test_lang::guard();
        let Some(dir) = dir() else { return };
        let kinds = [
            ("Primary", Kind::Primary),
            ("Secondary", Kind::Secondary),
            ("Quiet", Kind::Quiet),
            ("Destructive", Kind::Destructive),
            ("Choice", Kind::Choice),
        ];
        let d = State::default();
        let states = [
            ("rest", d),
            ("hover", State { hover: true, ..d }),
            (
                "pressed",
                State {
                    hover: true,
                    pressed: true,
                    ..d
                },
            ),
            ("focus", State { focus: true, ..d }),
            (
                "selected",
                State {
                    selected: true,
                    ..d
                },
            ),
            ("armed", State { armed: true, ..d }),
            (
                "disabled",
                State {
                    disabled: true,
                    ..d
                },
            ),
        ];
        for dpi in DPIS {
            for (dark, th) in themes() {
                let (cw, rh) = (px(112, dpi), px(48, dpi));
                let (w, h) = (
                    px(150, dpi) + cw * 7 * 2 + px(40, dpi),
                    rh * 6 + px(120, dpi),
                );
                let surf = Surface::new(w, h);
                let mut p = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                p.fill_rect(transom_render::Rect::new(0, 0, w, h), th.background);
                let card_x = px(150, dpi) + cw * 7 + px(20, dpi);
                design::card(
                    &mut p,
                    &th,
                    transom_render::Rect::new(card_x - px(10, dpi), 0, cw * 7 + px(20, dpi), h),
                );
                for (half, under, x0) in [(0, th.background, px(150, dpi)), (1, th.surface, card_x)]
                {
                    for (si, (sn, _)) in states.iter().enumerate() {
                        if half == 0 {
                            p.draw_text(
                                sn,
                                transom_render::Rect::new(
                                    x0 + si as i32 * cw,
                                    px(8, dpi),
                                    cw,
                                    px(20, dpi),
                                ),
                                &transom_render::design::Ty::Caption
                                    .style(th.muted, transom_render::HAlign::Center),
                            );
                        }
                    }
                    for (ki, (kn, kind)) in kinds.iter().enumerate() {
                        let y = px(36, dpi) + ki as i32 * rh;
                        if half == 0 {
                            p.draw_text(
                                kn,
                                transom_render::Rect::new(
                                    px(10, dpi),
                                    y,
                                    px(136, dpi),
                                    px(32, dpi),
                                ),
                                &transom_render::design::Ty::Body
                                    .style(th.foreground, transom_render::HAlign::Left),
                            );
                        }
                        for (si, (_, st)) in states.iter().enumerate() {
                            let r = transom_render::Rect::new(
                                x0 + si as i32 * cw + px(8, dpi),
                                y,
                                cw - px(16, dpi),
                                px(32, dpi),
                            );
                            let b = Button::new("Action", *kind).state(*st);
                            design::button(&mut p, &th, r, &b, under);
                        }
                    }
                    // zaskrtavacie policka: vypnute, zapnute, pod mysou, zakazane
                    let y = px(36, dpi) + 5 * rh;
                    for (i, (on, hov, dis)) in [
                        (false, false, false),
                        (true, false, false),
                        (false, true, false),
                        (true, true, false),
                        (false, false, true),
                        (true, false, true),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let r = transom_render::Rect::new(
                            x0 + i as i32 * px(40, dpi) + px(8, dpi),
                            y + px(6, dpi),
                            px(20, dpi),
                            px(20, dpi),
                        );
                        design::checkbox(&mut p, &th, r, on, hov, dis);
                    }
                }
                save(&dir, &format!("components-{}-{dpi}", tag(dark)), &surf);
            }
        }
    }

    /// Lista so vsetkymi castami (hladat, media s obalom, AI kruh s detailom, nastroje, zvuk, bateria, hodiny) v roznych
    /// stavoch: predvolena tema, hover, stlacenie, vlastna farba a priehladny rezim (sive vyhladzovanie nad podkladom).
    #[test]
    #[ignore]
    fn gallery_bar() {
        let _g = crate::test_lang::guard();
        let Some(dir) = dir() else { return };
        let mut art = Image::solid(64, 64, 0);
        for y in 0..64u32 {
            for x in 0..64u32 {
                art.px[(y * 64 + x) as usize] =
                    0xFF00_0000 | (x * 4) << 16 | 0x50 << 8 | (200 - y * 2);
            }
        }
        let art = ArtImage {
            id: 7,
            image: Arc::new(art),
        };
        let media = SessionInfo {
            key: "Spotify.exe".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            state: PlayState::Playing,
            controls: Controls {
                pause: true,
                prev: true,
                next: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let ring = RingView {
            fill: Some(62),
            ccw: false,
            muted: false,
            label: "62 % used".into(),
            name: String::new(),
            alert: false,
            detail: Some("Claude · 5 h · resets in 2 h 14 min".into()),
        };
        let audio = AudioView {
            glyph: Glyph::Volume2,
            name: String::new(),
        };
        let bat = Battery {
            percent: Some(76),
            charging: false,
            on_ac: false,
            saver: false,
        };
        let view = View {
            clock: "17:25",
            battery: Some(bat),
            media: Some(&media),
            art: Some(art),
            ai: Some(&ring),
            audio: Some(&audio),
            ..Default::default()
        };
        let mut layout = transom_config::Layout::default();
        layout.set_visible("audio", true);
        layout.set_visible("ai", true);
        let items = arrange(build_items(&view), &layout);
        let find = |id: ItemId| items.iter().position(|i| i.id == id).map(|_| id);
        // (nazov, tema, vlastne pozadie, sive vyhladzovanie, interakcia)
        let mut cases: Vec<(String, Theme, bool)> = Vec::new();
        for (dark, th) in themes() {
            cases.push((format!("bar-{}", tag(dark)), th, false));
        }
        let mut deep = Theme::dark();
        deep.background = Color::hex(0x1F3A5F);
        cases.push(("bar-custom-blue".into(), deep, false));
        let mut cream = Theme::light();
        cream.background = Color::hex(0xF3E9D2);
        cases.push(("bar-custom-cream".into(), cream, false));
        // priehladny rezim: text sa kresli sivo nad ciernym / bielym podkladom (z neho sa potom odvodzuje alfa)
        let mut over_black = Theme::dark();
        over_black.background = Color::rgb(0, 0, 0);
        cases.push(("bar-transparent-dark".into(), over_black, true));
        let mut over_white = Theme::light();
        over_white.background = Color::rgb(255, 255, 255);
        cases.push(("bar-transparent-light".into(), over_white, true));
        for dpi in DPIS {
            let (w, h) = (px(1180, dpi), px(28, dpi));
            for (name, th, gray) in &cases {
                for (state, ui) in [
                    ("", Interaction::default()),
                    (
                        "-hover",
                        Interaction {
                            hovered: find(ItemId::Search).map(|id| (id, 0)),
                            pressed: None,
                            ..Default::default()
                        },
                    ),
                    (
                        "-pressed",
                        Interaction {
                            hovered: find(ItemId::Media).map(|id| (id, 2)),
                            pressed: find(ItemId::Media).map(|id| (id, 2)),
                            ..Default::default()
                        },
                    ),
                ] {
                    if !state.is_empty() && (name.contains("custom") || name.contains("transp")) {
                        continue;
                    }
                    let surf = Surface::new(w, h);
                    let mut p = GdiPainter::new(surf.dc, Size { w, h }, dpi).grayscale(*gray);
                    render_bar_with(&mut p, th, &items, ui, None);
                    save(&dir, &format!("{name}{state}-{dpi}"), &surf);
                }
            }
        }
    }

    /// Pocasie na liste (jasno, oblacno, dazd a sneh v niekolkych snimkach animacie, burka) a bateria (bezna,
    /// nabijanie, pod 20 %) v tmavej a svetlej teme.
    #[test]
    #[ignore]
    fn gallery_weather_battery() {
        let _g = crate::test_lang::guard();
        let Some(dir) = dir() else { return };
        let cases: Vec<(String, u32, u8, Battery)> = {
            let b = |p: u8, c: bool| Battery {
                percent: Some(p),
                charging: c,
                on_ac: c,
                saver: false,
            };
            let mut v = vec![
                ("clear".to_string(), 0, 0, b(76, false)),
                ("cloudy".to_string(), 3, 0, b(64, true)),
                ("storm".to_string(), 95, 0, b(19, false)),
            ];
            for f in [0u8, 3, 6, 9] {
                v.push((format!("rain-{f:02}"), 63, f, b(12, true)));
            }
            for f in [0u8, 9, 18, 27] {
                v.push((format!("snow-{f:02}"), 73, f, b(20, false)));
            }
            v
        };
        for dpi in [144u32] {
            for (dark, th) in themes() {
                for (name, code, phase, bat) in &cases {
                    let w = crate::weather::Weather {
                        place: "Test".into(),
                        temp: -2.0,
                        code: *code,
                        days: Vec::new(),
                    };
                    let view = View {
                        clock: "17:25",
                        battery: Some(*bat),
                        weather: Some(&w),
                        weather_phase: *phase,
                        ..Default::default()
                    };
                    let mut layout = transom_config::Layout::default();
                    layout.set_visible("weather", true);
                    let items = arrange(build_items(&view), &layout);
                    let (bw, h) = (px(600, dpi), px(28, dpi));
                    let surf = Surface::new(bw, h);
                    let mut p = GdiPainter::new(surf.dc, Size { w: bw, h }, dpi);
                    let _ = render_bar_with(&mut p, &th, &items, Interaction::default(), None);
                    save(&dir, &format!("wx-{name}-{}-{dpi}", tag(dark)), &surf);
                }
            }
        }
    }

    /// Ostrov v strede listy vo vsetkych stavoch (pokoj s logom, nova skladba, hudba, AI pracuje, hudba + AI,
    /// oznamenie) a menu ostrova (Hudba, AI) - nepriehladne aj na skle. Len vymyslene data (snimky do README).
    #[test]
    #[ignore]
    fn gallery_island() {
        use crate::island_menu::{IslandMenu, Limit, Tab};
        use transom_core::item::{Action, AiLogo, Mark, Part, Seg, Side, StatusItem};
        use transom_platform::glass::Rim;
        let _g = crate::test_lang::guard();
        let Some(dir) = dir() else { return };
        let mut img = Image::solid(64, 64, 0);
        for y in 0..64u32 {
            for x in 0..64u32 {
                img.px[(y * 64 + x) as usize] =
                    0xFF00_0000 | (x * 4) << 16 | 0x50 << 8 | (200 - y * 2);
            }
        }
        let art = ArtImage {
            id: 7,
            image: Arc::new(img),
        };
        let media = SessionInfo {
            key: "Spotify.exe".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            state: PlayState::Playing,
            controls: Controls {
                pause: true,
                prev: true,
                next: true,
                toggle: true,
                ..Default::default()
            },
            timeline: transom_core::media::Timeline {
                start: 0,
                end: 2_430_000_000,
                position: 1_020_000_000,
                last_updated: 0,
            },
            ..Default::default()
        };
        let bat = Battery {
            percent: Some(76),
            charging: false,
            on_ac: false,
            saver: false,
        };
        let audio = AudioView {
            glyph: Glyph::Volume2,
            name: String::new(),
        };
        let view = View {
            clock: "17:25",
            battery: Some(bat),
            audio: Some(&audio),
            ..Default::default()
        };
        let mut layout = transom_config::Layout::default();
        layout.set_visible("audio", true);
        let base = arrange(build_items(&view), &layout);
        let text = |t: &str, muted: bool| Seg::Text {
            text: t.into(),
            muted,
            strong: false,
            low: false,
            scroll: None,
        };
        let claude = |t: &str| Seg::Logo {
            logo: AiLogo::Claude,
            text: t.into(),
            low: false,
            dot: Some((156, 213, 254)),
        };
        let states: Vec<(&str, Seg, Seg)> = vec![
            ("idle", Seg::None, Seg::None),
            (
                "peek",
                Seg::Cover {
                    art: Some(art.clone()),
                    text: "Midnight City".into(),
                    scroll: None,
                },
                text("M83", true),
            ),
            (
                "music",
                Seg::Art(Some(art.clone())),
                Seg::Eq {
                    levels: [110, 190, 140, 230, 90],
                    color: Some((236, 72, 153)),
                    playing: true,
                },
            ),
            ("ai", claude(""), text("62 % used · 5 h", false)),
            (
                "music-ai",
                Seg::Cover {
                    art: Some(art.clone()),
                    text: "Midnight City – M83".into(),
                    scroll: None,
                },
                claude("62 %"),
            ),
            (
                "notice",
                text("Screenshot saved", false),
                text("Click to open", true),
            ),
        ];
        let dpi = 144;
        let (w, h) = (px(1180, dpi), px(28, dpi));
        for (state, left, right) in &states {
            let mut items = base.clone();
            let island = StatusItem::new(
                ItemId::Shelf,
                Side::Center,
                vec![Part::Island {
                    left: left.clone(),
                    mark: Mark::Transom,
                    label: "transom".into(),
                    badge: String::new(),
                    right: right.clone(),
                    name: "island".into(),
                    action: Some(Action::Island),
                }],
            );
            match items.iter_mut().find(|i| i.id == ItemId::Shelf) {
                Some(it) => *it = island,
                None => items.push(island),
            }
            for (dark, th) in themes() {
                let surf = Surface::new(w, h);
                {
                    let mut p = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                    render_bar_with(&mut p, &th, &items, Interaction::default(), None);
                }
                save(&dir, &format!("island-{state}-{}-{dpi}", tag(dark)), &surf);
                let (_, rgba) = glass_rgba(w, h, dpi, &th, Rim::Bottom, 0, 0, &mut |p, t| {
                    let _ = render_bar_with(p, t, &items, Interaction::default(), None);
                });
                save_rgba(
                    &dir,
                    &format!("island-{state}-glass-{}-{dpi}", tag(dark)),
                    w,
                    h,
                    &rgba,
                );
            }
        }
        // menu ostrova: karta Hudba (prehravac) a AI (limity)
        let tabs = vec![Tab::Music, Tab::Clipboard, Tab::Shots, Tab::Ai, Tab::Files];
        for tab in [Tab::Music, Tab::Ai] {
            let (mw, mh) = (
                px(transom_platform::dragdrop::NOTCH_W_LP, dpi),
                px(crate::island_menu::height_lp(Some(tab)), dpi),
            );
            let name = if tab == Tab::Music { "music" } else { "ai" };
            let mut paint = |p: &mut dyn transom_render::Painter, th: &Theme| {
                let mut m = IslandMenu::default();
                m.open(tabs.clone(), tab);
                m.limits = vec![
                    Limit {
                        window: "Claude · 5 h".into(),
                        used: 62,
                        detail: "resets in 2 h 14 min".into(),
                        low: false,
                    },
                    Limit {
                        window: "Claude · week".into(),
                        used: 31,
                        detail: "resets in 3 d 12 h".into(),
                        low: false,
                    },
                    Limit {
                        window: "Codex · month".into(),
                        used: 88,
                        detail: "resets in 25 d".into(),
                        low: true,
                    },
                ];
                let area = m.paint_frame(p, th);
                if tab == Tab::Ai {
                    m.paint_ai(p, th, area);
                } else {
                    let mut sub = transom_render::painter::Sub {
                        inner: p,
                        dx: area.x,
                        dy: area.y,
                        size: Size {
                            w: area.w,
                            h: area.h,
                        },
                    };
                    crate::media_notch::MediaNotch::default().paint(
                        &mut sub,
                        th,
                        &media,
                        Some(&art),
                    );
                }
            };
            for (dark, th) in themes() {
                let surf = Surface::new(mw, mh);
                {
                    let mut p = GdiPainter::new(surf.dc, Size { w: mw, h: mh }, dpi);
                    paint(&mut p, &th);
                }
                save(
                    &dir,
                    &format!("islandmenu-{name}-{}-{dpi}", tag(dark)),
                    &surf,
                );
                // menu ostrova: rohy 14 lp a vyduty prechod 10 lp (ako vybezok v aplikacii)
                let (gw, rgba) = glass_rgba(mw, mh, dpi, &th, Rim::Attached, 14, 10, &mut paint);
                save_rgba(
                    &dir,
                    &format!("islandmenu-{name}-glass-{}-{dpi}", tag(dark)),
                    gw,
                    mh,
                    &rgba,
                );
            }
        }
    }
}
