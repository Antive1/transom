//! Sklo: skutocne zobrazene farby (nadych skla nad lubovolnou strankou + priesvitne plochy z dvoch prechodov
//! `Theme::glass_pass`) a kontrast textu na nich v najhorsom pripade.
use transom_platform::glass::GlassStyle;
use transom_render::audit::ratio;
use transom_render::{Color, Theme};

/// Zmes `a` -> `b` s krytim `t` (0..=1).
fn mix(a: Color, b: Color, t: f32) -> Color {
    let l = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
    Color::rgb(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b))
}

/// Plocha nakreslena v prechodoch nad cierne (`k`) a biele (`w`) pozadie, zlozena nad `base`: alfa = 1 - (w - k),
/// farba (premultiplied) = k.
fn over(k: Color, w: Color, base: Color) -> Color {
    let a = 1.0 - (w.g as f32 - k.g as f32) / 255.0;
    let ch = |kc: u8, bc: u8| (kc as f32 + (1.0 - a) * bc as f32).round().min(255.0) as u8;
    Color::rgb(ch(k.r, base.r), ch(k.g, base.g), ch(k.b, base.b))
}

#[test]
fn text_on_glass_and_its_cards_stays_readable_over_any_page() {
    let pages = [
        Color::rgb(255, 255, 255),
        Color::rgb(0, 0, 0),
        Color::rgb(255, 230, 0),
        Color::rgb(200, 16, 46),
        Color::rgb(0, 120, 215),
    ];
    let mut bad = Vec::new();
    for th in [Theme::dark(), Theme::light()] {
        let st = GlassStyle::for_theme(
            (th.background.r, th.background.g, th.background.b),
            th.dark,
            transom_platform::glass::Rim::All,
        );
        let tint = Color::rgb(st.tint.0, st.tint.1, st.tint.2);
        let (k, w) = (
            th.glass_pass(Color::rgb(0, 0, 0)),
            th.glass_pass(Color::rgb(255, 255, 255)),
        );
        for page in pages {
            let base = mix(page, tint, st.tint_alpha as f32 / 255.0);
            let planes = [
                ("panel", base),
                ("card", over(k.surface, w.surface, base)),
                ("control", over(k.control, w.control, base)),
                ("hover", over(k.hover, w.hover, base)),
                (
                    "control_hover",
                    over(k.control_hover, w.control_hover, base),
                ),
            ];
            for (i, (pn, bg)) in planes.into_iter().enumerate() {
                // hlavny text sa kresli na kazdej ploche; tlmeny, cerveny a akcentovy text len na paneli a karte
                let texts: &[(&str, Color)] = if i < 2 {
                    &[
                        ("foreground", k.foreground),
                        ("muted", k.muted),
                        ("low", k.low),
                        ("accent_text", k.accent_text),
                    ]
                } else {
                    &[("foreground", k.foreground)]
                };
                for &(tn, fg) in texts {
                    let r = ratio(fg, bg);
                    if r < 4.5 {
                        bad.push(format!(
                            "{} {tn} na {pn} nad {page:?}: {r:.2}",
                            if th.dark { "tmava" } else { "svetla" }
                        ));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "slaby kontrast:\n{}", bad.join("\n"));
}

#[test]
fn glass_pass_planes_are_translucent_white_or_black_and_never_holes_in_a_card() {
    for th in [Theme::dark(), Theme::light()] {
        let (k, w) = (
            th.glass_pass(Color::rgb(0, 0, 0)),
            th.glass_pass(Color::rgb(255, 255, 255)),
        );
        let alpha = |kc: Color, wc: Color| 255 - (wc.g as i32 - kc.g as i32);
        // pozadie panelu = uplne priehladne (cez neho sklo), plochy maju skutocnu alfu
        assert_eq!(alpha(k.background, w.background), 0);
        let card = alpha(k.surface, w.surface);
        assert!(card > 0, "karta musi byt viditelna");
        // ovladac na karte nesmie mat mensiu alfu nez karta (inak by v nej vyzeral ako diera)
        assert!(alpha(k.control, w.control) >= card);
        assert!(alpha(k.control_hover, w.control_hover) >= card);
        // akcent ostava nepriehladny
        assert_eq!(k.accent, w.accent);
    }
}

/// Sklo vo vlastnej farbe listy (`glass::tint_for`): text na paneli, karte a ovladacoch citatelny nad bielou aj ciernou
/// strankou (najhorsie pripady), pre svetle, tmave a stredne farby a rozne krytie.
#[test]
fn text_on_tinted_glass_and_its_cards_stays_readable() {
    let mut bad = Vec::new();
    for bar in [
        (0u8, 112u8, 224u8),
        (220, 40, 40),
        (128, 128, 128),
        (30, 160, 80),
        (250, 200, 0),
        (255, 255, 255),
        (0x24, 0x1B, 0x3A),
        (120, 40, 200),
        (0, 180, 200),
    ] {
        for alpha in [102u8, 178, 224, 255] {
            let t = transom_platform::glass::tint_for(bar, alpha);
            let th = if t.dark {
                Theme::dark()
            } else {
                Theme::light()
            };
            let tint = Color::rgb(t.tint.0, t.tint.1, t.tint.2);
            let (k, w) = (
                th.glass_pass(Color::rgb(0, 0, 0)),
                th.glass_pass(Color::rgb(255, 255, 255)),
            );
            for page in [Color::rgb(255, 255, 255), Color::rgb(0, 0, 0)] {
                let base = mix(page, tint, t.alpha as f32 / 255.0);
                let planes = [
                    ("panel", base),
                    ("card", over(k.surface, w.surface, base)),
                    ("control", over(k.control, w.control, base)),
                    ("hover", over(k.hover, w.hover, base)),
                    (
                        "control_hover",
                        over(k.control_hover, w.control_hover, base),
                    ),
                ];
                for (i, (pn, bg)) in planes.into_iter().enumerate() {
                    let texts: &[(&str, Color)] = if i < 2 {
                        &[
                            ("foreground", k.foreground),
                            ("muted", k.muted),
                            ("low", k.low),
                            ("accent_text", k.accent_text),
                        ]
                    } else {
                        &[("foreground", k.foreground)]
                    };
                    for &(tn, fg) in texts {
                        let r = ratio(fg, bg);
                        if r < 4.5 {
                            bad.push(format!(
                                "{bar:?}@{alpha} {tn} na {pn} nad {page:?}: {r:.2} ({t:?})"
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "slaby kontrast ({}):\n{}",
        bad.len(),
        bad.join("\n")
    );
}
