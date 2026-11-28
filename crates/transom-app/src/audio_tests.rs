//! Panel Zvuk: vyber vystupu, stavy zariadeni, hlasitost a stlmenie, medze, klavesnica, mys, pristupnost a audit vzhladu
//! (offscreen, bez okna a bez zvukoveho zariadenia; snimky stavu su vyrobene rucne).
use crate::audio_pop::{
    bar_glyph_and_name, step_percent, AudioCmd, AudioHit, AudioPanel, AUDIO_H_LP, AUDIO_W_LP,
};
use std::sync::Arc;
use transom_audio::{Availability, Controls, Device, DeviceState, Snapshot, Volume};
use transom_platform::PointerKind;
use transom_render::audit::Audit;
use transom_render::offscreen::Surface;
use transom_render::{GdiPainter, Size, Theme};

fn dev(name: &str, state: DeviceState, def: bool) -> Device {
    Device {
        name: name.into(),
        key: key(name),
        state,
        is_default: def,
    }
}

fn snap(vol: Option<(u8, bool)>, devices: Vec<Device>) -> Snapshot {
    Snapshot {
        generation: 1,
        availability: Availability::Ok,
        devices,
        volume: vol.map(|(p, m)| Volume {
            percent: p,
            muted: m,
        }),
        controls: Controls::default(),
        default_changes: 0,
    }
}

fn normal() -> Snapshot {
    snap(
        Some((40, false)),
        vec![
            dev("Speakers (Realtek)", DeviceState::Active, true),
            dev("USB Headset", DeviceState::Active, false),
            dev("HDMI Monitor", DeviceState::Unplugged, false),
            dev("Old Dock", DeviceState::Disabled, false),
        ],
    )
}

/// Kluc zariadenia v testoch (z nazvu; skutocny je odtlacok ID).
fn key(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

/// Nakresli panel (dvakrat: prve kreslenie pripravi hit-boxy) a vrati ho.
fn paint(panel: &mut AudioPanel, s: &Snapshot, dpi: u32, h_mul: f32) -> Vec<String> {
    let sc = dpi as f32 / 96.0;
    let (w, h) = (
        (AUDIO_W_LP as f32 * sc).round() as i32,
        (AUDIO_H_LP as f32 * h_mul * sc).round() as i32,
    );
    let th = Theme::dark();
    let surf = Surface::new(w, h);
    let mut out = Vec::new();
    let a = Arc::new(s.clone());
    for _ in 0..2 {
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
        let mut au = Audit::new(&mut gp, th.background);
        panel.paint(&mut au, &th, &a);
        au.finish();
        out = au.issues.iter().map(|i| format!("{i:?}")).collect();
    }
    out
}

fn en() -> std::sync::MutexGuard<'static, ()> {
    crate::test_lang::guard()
}

#[test]
fn the_panel_shows_default_volume_devices_and_labels_the_volume_as_system_output() {
    let _g = en();
    let mut p = AudioPanel::default();
    let issues = paint(&mut p, &normal(), 96, 2.0);
    assert!(issues.is_empty(), "{issues:?}");
    eprintln!("VYSKA obsahu normal = {} px pri 96 DPI", p.content_height());
    let (entries, extras, _, name) = p.a11y();
    assert_eq!(entries.len(), extras.len());
    assert_eq!(name, transom_i18n::t("audio.title"));
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    // vystupy: aktivne zariadenia ako prepinace, predvolene zaskrtnute; odpojene a vypnute sa nevypisuju
    use transom_platform::menu_access::{ROLE_RADIOBUTTON, STATE_CHECKED};
    let find = |s: &str| {
        names
            .iter()
            .position(|n| *n == s)
            .unwrap_or_else(|| panic!("{s}: {names:?}"))
    };
    let sp = find("Speakers (Realtek)");
    assert_eq!(extras[sp].role, ROLE_RADIOBUTTON);
    assert!(extras[sp].state & STATE_CHECKED != 0);
    assert!(extras[sp].action.is_empty(), "predvoleny sa uz nevybera");
    let usb = find("USB Headset");
    assert_eq!(extras[usb].role, ROLE_RADIOBUTTON);
    assert!(extras[usb].state & STATE_CHECKED == 0);
    assert_eq!(extras[usb].action, transom_i18n::t("acc.select"));
    assert!(!names
        .iter()
        .any(|n| n.contains("HDMI Monitor") || n.contains("Old Dock")));
}

#[test]
fn the_slider_and_mute_report_value_state_and_actions_and_unavailable_controls_are_not_interactive()
{
    use transom_platform::menu_access::{ROLE_CHECKBUTTON, ROLE_SLIDER, STATE_CHECKED};
    let _g = en();
    let mut p = AudioPanel::default();
    paint(
        &mut p,
        &snap(Some((40, true)), vec![dev("A", DeviceState::Active, true)]),
        96,
        1.0,
    );
    let (entries, extras, _, _) = p.a11y();
    let sl = entries
        .iter()
        .position(|e| e.name == "Windows system output volume")
        .unwrap();
    assert_eq!(extras[sl].role, ROLE_SLIDER);
    assert_eq!(extras[sl].value, "40 %");
    assert!(
        extras[sl].action.is_empty(),
        "posuvnik sa meni klavesami, nie akciou"
    );
    let mu = entries
        .iter()
        .position(|e| e.name == "Unmute system output")
        .unwrap();
    assert_eq!(extras[mu].role, ROLE_CHECKBUTTON);
    assert!(
        extras[mu].state & STATE_CHECKED != 0,
        "stlmene = zaskrtnute"
    );
    // zariadenie bez regulacie hlasitosti: ovladace nie su v kruhu fokusu a text vysvetli preco
    let mut s = snap(
        Some((40, false)),
        vec![dev("HDMI", DeviceState::Active, true)],
    );
    s.controls = Controls {
        volume: false,
        mute: false,
    };
    let mut p = AudioPanel::default();
    paint(&mut p, &s, 96, 1.0);
    let (entries, _, _, _) = p.a11y();
    assert!(entries
        .iter()
        .any(|e| e.name == transom_i18n::t("audio.s_novolume")));
    let sl = entries
        .iter()
        .find(|e| e.name == "Windows system output volume")
        .unwrap();
    assert!(!sl.enabled);
    assert_eq!(p.hit(sl.x + sl.w / 2, sl.y + sl.h / 2), None);
    let (cmd, _) = p.key(0x09, false, false, &s);
    assert_ne!(p.focus, Some(AudioHit::Slider));
    assert_ne!(p.focus, Some(AudioHit::Mute));
    assert!(cmd.is_none());
    // nikdy sa nezapise: ani klavesa na posuvniku, ani klik na stlmenie
    p.focus = Some(AudioHit::Slider);
    assert_eq!(p.key(0x27, false, false, &s).0, None);
}

#[test]
fn every_availability_state_has_an_honest_status_text_and_no_active_controls() {
    let _g = en();
    for (av, key) in [
        (Availability::Starting, "audio.s_starting"),
        (Availability::ServiceUnavailable, "audio.s_unavailable"),
        (Availability::NoDevice, "audio.s_nodevice"),
    ] {
        let mut s = Snapshot {
            availability: av,
            ..Snapshot::default()
        };
        s.generation = 2;
        let mut p = AudioPanel::default();
        let issues = paint(&mut p, &s, 96, 2.0);
        assert!(issues.is_empty(), "{av:?}: {issues:?}");
        let (entries, _, _, _) = p.a11y();
        assert!(
            entries.iter().any(|e| e.name == transom_i18n::t(key)),
            "{av:?}"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.name.contains(&transom_i18n::t("audio.no_device"))),
            "{av:?}: {:?}",
            entries.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        // posuvnik a stlmenie nie su ovladatelne, tlacidla Windows ostavaju (da sa ist do nastaveni zvuku)
        let (cmd, _) = p.key(0x09, false, false, &s);
        assert!(cmd.is_none());
        assert!(
            matches!(p.focus, Some(AudioHit::SoundSettings)),
            "{av:?}: {:?}",
            p.focus
        );
    }
}

#[test]
fn volume_and_mute_commands_are_bounded_exact_and_mute_toggles_the_current_state() {
    let _g = en();
    let s = normal();
    let mut p = AudioPanel::default();
    paint(&mut p, &s, 96, 1.0);
    // klavesy na posuvniku: 5 %, Shift 1 %, Page 10 %, Home/End, nikdy mimo 0..=100
    p.focus = Some(AudioHit::Slider);
    assert_eq!(
        p.key(0x27, false, false, &s).0,
        Some(AudioCmd::SetVolume(45))
    );
    // dalsia klavesa pred potvrdenim systemu pocita z nami zadanej hodnoty (nestraca kroky)
    assert_eq!(
        p.key(0x27, false, false, &s).0,
        Some(AudioCmd::SetVolume(50))
    );
    assert_eq!(
        p.key(0x25, true, false, &s).0,
        Some(AudioCmd::SetVolume(49))
    );
    assert_eq!(
        p.key(0x23, false, false, &s).0,
        Some(AudioCmd::SetVolume(100))
    );
    assert_eq!(p.key(0x27, false, false, &s).0, None, "uz na maxime");
    assert_eq!(
        p.key(0x24, false, false, &s).0,
        Some(AudioCmd::SetVolume(0))
    );
    assert_eq!(p.key(0x25, false, false, &s).0, None, "uz na minime");
    for (cur, vk, shift, want) in [
        (98, 0x27, false, 100),
        (2, 0x25, false, 0),
        (96, 0x21, false, 100),
        (4, 0x22, false, 0),
        (50, 0x26, true, 51),
        (50, 0x28, true, 49),
    ] {
        assert_eq!(step_percent(cur, vk, shift), Some(want), "{cur} {vk:#x}");
    }
    assert_eq!(step_percent(50, 0x41, false), None);
    // stlmenie: Enter na tlacidle prepne aktualny stav; drzanie klavesu (opakovanie) neprepina dookola
    let mut p = AudioPanel::default();
    paint(&mut p, &s, 96, 1.0);
    p.focus = Some(AudioHit::Mute);
    assert_eq!(
        p.key(0x0D, false, false, &s).0,
        Some(AudioCmd::SetMute(true))
    );
    assert_eq!(p.key(0x0D, false, true, &s).0, None);
    let muted = snap(Some((40, true)), s.devices.clone());
    assert_eq!(
        p.key(0x20, false, false, &muted).0,
        Some(AudioCmd::SetMute(false))
    );
}

#[test]
fn the_mouse_drags_the_slider_clamps_at_the_ends_and_a_button_needs_press_and_release_on_it() {
    let _g = en();
    let s = normal();
    let mut p = AudioPanel::default();
    paint(&mut p, &s, 96, 1.0);
    let (entries, _, _, _) = p.a11y();
    let sl = entries
        .iter()
        .find(|e| e.name == "Windows system output volume")
        .unwrap()
        .clone();
    let y = sl.y + sl.h / 2;
    // stlacenie na posuvniku nastavi hodnotu, tahanie mimo okna ju orezie na 0..=100
    let (c, _) = p.pointer(PointerKind::Down, sl.x + sl.w / 2, y, &s);
    assert!(
        matches!(c, Some(AudioCmd::SetVolume(v)) if (40..=60).contains(&v)),
        "{c:?}"
    );
    let (c, _) = p.pointer(PointerKind::Move, 99_999, y, &s);
    assert_eq!(c, Some(AudioCmd::SetVolume(100)));
    let (c, _) = p.pointer(PointerKind::Move, -99_999, y, &s);
    assert_eq!(c, Some(AudioCmd::SetVolume(0)));
    let (c, _) = p.pointer(PointerKind::Click, 5, 5, &s);
    assert_eq!(c, None, "uvolnenie koniec tahania, nie akcia");
    // po konci tahania pohyb uz nic nenastavuje
    let (c, _) = p.pointer(PointerKind::Move, sl.x, y, &s);
    assert_eq!(c, None);
    // volba vystupu: klik na iny vystup ho poziada o predvoleny; klik na predvoleny nerobi nic
    let usb = entries
        .iter()
        .find(|e| e.name == "USB Headset")
        .unwrap()
        .clone();
    let spk = entries
        .iter()
        .find(|e| e.name == "Speakers (Realtek)")
        .unwrap()
        .clone();
    let sound = entries
        .iter()
        .find(|e| e.name.starts_with(&transom_i18n::t("audio.sound_settings")))
        .unwrap()
        .clone();
    p.pointer(PointerKind::Down, usb.x + 5, usb.y + 5, &s);
    let (c, _) = p.pointer(PointerKind::Click, sound.x + 5, sound.y + 5, &s);
    assert_eq!(c, None, "stlacenie a uvolnenie nad inym ovladacom");
    p.pointer(PointerKind::Down, usb.x + 5, usb.y + 5, &s);
    let (c, _) = p.pointer(PointerKind::Click, usb.x + 5, usb.y + 5, &s);
    assert_eq!(c, Some(AudioCmd::SetDefault(key("USB Headset"))));
    p.pointer(PointerKind::Down, spk.x + 5, spk.y + 5, &s);
    let (c, _) = p.pointer(PointerKind::Click, spk.x + 5, spk.y + 5, &s);
    assert_eq!(c, None);
    p.pointer(PointerKind::Down, sound.x + 5, sound.y + 5, &s);
    let (c, _) = p.pointer(PointerKind::Click, sound.x + 5, sound.y + 5, &s);
    assert_eq!(c, Some(AudioCmd::SoundSettings));
    // opustenie okna pocas tahania ho zrusi (koniec bez uvolnenia)
    p.pointer(PointerKind::Down, sl.x + 4, y, &s);
    p.pointer(PointerKind::Leave, -1, -1, &s);
    assert_eq!(p.pointer(PointerKind::Move, sl.x + 50, y, &s).0, None);
}

#[test]
fn tab_walks_every_control_once_in_visual_order_and_enter_picks_an_output() {
    let _g = en();
    let s = normal();
    let mut p = AudioPanel::default();
    paint(&mut p, &s, 96, 1.0);
    let mut order = Vec::new();
    for _ in 0..20 {
        p.key(0x09, false, false, &s);
        let f = p.focus.unwrap();
        if order.contains(&f) {
            break;
        }
        order.push(f);
    }
    assert_eq!(
        order,
        vec![
            AudioHit::Output(0),
            AudioHit::Output(1),
            AudioHit::Mute,
            AudioHit::Slider,
            AudioHit::SoundSettings,
        ]
    );
    // Shift+Tab ide spat a z prveho ovladaca na posledny
    p.focus = Some(AudioHit::Output(0));
    p.key(0x09, true, false, &s);
    assert_eq!(p.focus, Some(AudioHit::SoundSettings));
    // Enter na vystupe: iny = poziadat o predvoleny, predvoleny = nic
    p.focus = Some(AudioHit::Output(1));
    assert_eq!(
        p.key(0x0D, false, false, &s).0,
        Some(AudioCmd::SetDefault(key("USB Headset")))
    );
    p.focus = Some(AudioHit::Output(0));
    assert_eq!(p.key(0x0D, false, false, &s).0, None);
}

#[test]
fn every_button_name_says_that_it_opens_a_windows_surface_and_nothing_claims_a_change() {
    let _g = crate::test_lang::guard();
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        let mut p = AudioPanel::default();
        paint(&mut p, &normal(), 96, 1.0);
        let (entries, extras, _, _) = p.a11y();
        for (e, x) in entries.iter().zip(&extras) {
            if x.role == transom_platform::menu_access::ROLE_PUSHBUTTON {
                let label_open = transom_i18n::tf("qa.acc", &["X"]);
                let marker = label_open.replace("X", "");
                assert!(e.name.contains(marker.trim()), "{lang:?}: {:?}", e.name);
            }
        }
        // texty o vystupe nikdy netvrdia hotovu zmenu: prepnutie hlasi len "ohlasene Windows" po notifikacii
        assert!(transom_i18n::tf("audio.requested", &["X"]).contains('X'));
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
}

#[test]
fn the_default_change_is_confirmed_only_after_core_audio_reports_it() {
    let _g = en();
    let a = normal();
    let mut p = AudioPanel::default();
    p.reset(&a);
    // otvorenie vyberu vystupu: hlaska "poziadane", ziadne tvrdenie o zmene
    p.msg = Some(transom_i18n::tf(
        "audio.requested",
        &["Choose output in Windows"],
    ));
    paint(&mut p, &a, 96, 1.0);
    assert!(p.msg.as_deref().unwrap().starts_with("Requested:"));
    assert!(!p.msg.as_deref().unwrap().contains("default output:"));
    // Windows ohlasil zmenu predvoleneho vystupu: az teraz hlaska s novym nazvom
    let mut b = normal();
    b.generation = 2;
    b.default_changes = 1;
    b.devices = vec![
        dev("USB Headset", DeviceState::Active, true),
        dev("Speakers (Realtek)", DeviceState::Active, false),
    ];
    paint(&mut p, &b, 96, 1.0);
    assert_eq!(
        p.msg.as_deref(),
        Some("Windows reported a new default output: USB Headset.")
    );
    // dalsie kreslenie bez novej zmeny hlasku nemeni
    paint(&mut p, &b, 96, 1.0);
    assert!(p.msg.as_deref().unwrap().contains("USB Headset"));
}

#[test]
fn the_bar_icon_reflects_volume_mute_and_unavailability_with_distinct_glyphs_and_names() {
    use transom_core::item::Glyph;
    let _g = en();
    let g = |s: &Snapshot| bar_glyph_and_name(s);
    assert_eq!(g(&snap(Some((0, false)), vec![])).0, Glyph::Volume0);
    assert_eq!(g(&snap(Some((20, false)), vec![])).0, Glyph::Volume1);
    assert_eq!(g(&snap(Some((50, false)), vec![])).0, Glyph::Volume2);
    assert_eq!(g(&snap(Some((90, false)), vec![])).0, Glyph::Volume3);
    // stlmenie ma vlastny tvar (nie len farbu) aj nazov
    let m = g(&snap(Some((90, true)), vec![]));
    assert_eq!((m.0, m.1.as_str()), (Glyph::VolumeMuted, "Audio: muted"));
    assert_eq!(g(&snap(Some((33, false)), vec![])).1, "Audio: 33 %");
    let off = g(&Snapshot::default());
    assert_eq!(off.1, "Audio: unavailable");
}

#[test]
fn the_panel_audit_is_clean_in_every_language_theme_dpi_and_state() {
    let _g = crate::test_lang::guard();
    let long = "Reproduktory s velmi dlhym nazvom zariadenia (Realtek High Definition Audio) a dalsi popis pre kontrolu orezania";
    let many: Vec<Device> = (0..20)
        .map(|i| {
            dev(
                &format!("Device {i} {long}"),
                if i % 3 == 0 {
                    DeviceState::Active
                } else {
                    DeviceState::Unplugged
                },
                i == 0,
            )
        })
        .collect();
    let mut unsupported = normal();
    unsupported.controls = Controls {
        volume: false,
        mute: true,
    };
    let states: Vec<(&str, Snapshot)> = vec![
        ("normal", normal()),
        ("muted", snap(Some((0, true)), normal().devices)),
        ("full", snap(Some((100, false)), normal().devices)),
        ("many-long", snap(Some((55, false)), many)),
        ("unsupported", unsupported),
        (
            "nodevice",
            Snapshot {
                availability: Availability::NoDevice,
                generation: 2,
                ..Snapshot::default()
            },
        ),
        (
            "service",
            Snapshot {
                availability: Availability::ServiceUnavailable,
                generation: 2,
                ..Snapshot::default()
            },
        ),
    ];
    let mut bad = Vec::new();
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for dpi in [96u32, 144, 192] {
            for (name, s) in &states {
                let mut p = AudioPanel::default();
                p.msg = Some(transom_i18n::tf(
                    "audio.requested",
                    &[&transom_i18n::t("audio.sound_settings")],
                ));
                // dostatocne vysoka plocha: audit kontroluje cely obsah (v skutocnom okne ho posuva koliesko)
                let issues = paint(&mut p, s, dpi, 2.0);
                for i in issues {
                    bad.push(format!("{} {dpi} {name}: {i}", lang.code()));
                }
            }
        }
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
    assert!(
        bad.is_empty(),
        "{} nalezov, napr.:\n{}",
        bad.len(),
        bad.iter().take(15).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn a_new_app_has_no_audio_service_and_automation_never_starts_one() {
    let _g = crate::test_lang::guard();
    let a = crate::App::new(transom_render::ThemeMode::default(), None, None, false);
    assert!(a.audio.is_none() && !a.audio_allowed);
    // bez povolenia sa sluzba nespusti ani pri viditelnej polozke
    let mut a = a;
    a.config.layout.set_visible("audio", true);
    a.sync_audio();
    assert!(a.audio.is_none());
    assert!(a.audio_wanted_in_layout());
    // snimka bez sluzby = "este sa nacitalo" (ikona je neaktivna, nehlasi sa ziadna hlasitost)
    assert_eq!(a.audio_snapshot().availability, Availability::Starting);
}

/// Galeria (pozri `crate::gallery`): panel Zvuk (normalny so zameranym posuvnikom, stlmeny, bez zariadenia).
#[test]
#[ignore]
fn gallery_audio() {
    let _g = crate::test_lang::guard();
    let Some(dir) = crate::gallery::dir() else {
        return;
    };
    let states: Vec<(&str, Snapshot)> = vec![
        ("normal", normal()),
        ("muted", snap(Some((0, true)), normal().devices)),
        (
            "nodevice",
            Snapshot {
                availability: Availability::NoDevice,
                generation: 2,
                ..Snapshot::default()
            },
        ),
    ];
    for dpi in crate::gallery::DPIS {
        for (dark, th) in crate::gallery::themes() {
            for (name, s) in &states {
                let (w, h) = (
                    crate::gallery::px(AUDIO_W_LP, dpi),
                    crate::gallery::px(AUDIO_H_LP, dpi),
                );
                let surf = Surface::new(w, h);
                let mut p = AudioPanel::default();
                if *name == "normal" {
                    p.focus = Some(AudioHit::Slider);
                }
                let a = Arc::new(s.clone());
                for _ in 0..2 {
                    let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                    p.paint(&mut gp, &th, &a);
                }
                crate::gallery::save(
                    &dir,
                    &format!("audio-{name}-{}-{dpi}", crate::gallery::tag(dark)),
                    &surf,
                );
            }
        }
    }
    // predvolene sklo (podklady promo videa)
    let dpi = crate::gallery::hero_dpi();
    let (w, h) = (
        crate::gallery::px(AUDIO_W_LP, dpi),
        crate::gallery::px(AUDIO_H_LP, dpi),
    );
    let mut p = AudioPanel::default();
    let a = Arc::new(normal());
    let (gw, rgba) = crate::gallery::glass_rgba_panel(w, h, dpi, &Theme::dark(), &mut |gp, t| {
        p.paint(gp, t, &a)
    });
    crate::gallery::save_rgba(&dir, "audio-glass", gw, h, &rgba);
}

/// Pomocny test na pozretie vzhladu: `TRANSOM_PANEL_PNG=<adresar> cargo test -p transom-app panel_png -- --ignored`.
#[test]
#[ignore]
fn panel_png() {
    let _g = crate::test_lang::guard();
    let Some(dir) = std::env::var_os("TRANSOM_PANEL_PNG") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut states: Vec<(&str, Snapshot)> = vec![
        ("normal", normal()),
        ("muted", snap(Some((0, true)), normal().devices)),
        (
            "nodevice",
            Snapshot {
                availability: Availability::NoDevice,
                generation: 2,
                ..Snapshot::default()
            },
        ),
    ];
    let mut unsupported = normal();
    unsupported.controls = Controls {
        volume: false,
        mute: true,
    };
    states.push(("unsupported", unsupported));
    for (lang, code) in [
        (transom_i18n::Lang::En, "en"),
        (transom_i18n::Lang::De, "de"),
        (transom_i18n::Lang::Ja, "ja"),
    ] {
        transom_i18n::set_lang(lang);
        for (name, s) in &states {
            for dark in [true, false] {
                let th = if dark { Theme::dark() } else { Theme::light() };
                let (w, h) = (AUDIO_W_LP, AUDIO_H_LP);
                let surf = Surface::new(w, h);
                let mut p = AudioPanel::default();
                p.focus = Some(AudioHit::Slider);
                let a = Arc::new(s.clone());
                for _ in 0..2 {
                    let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
                    p.paint(&mut gp, &th, &a);
                }
                let f = dir.join(format!(
                    "audio-{code}-{name}-{}.png",
                    if dark { "dark" } else { "light" }
                ));
                let _ = std::fs::write(
                    f,
                    transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
                );
            }
        }
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
}

/// Pomocny test: lista so vsetkymi stavmi ikony Zvuk (`TRANSOM_PANEL_PNG=<adresar> ... bar_png -- --ignored`).
#[test]
#[ignore]
fn bar_png() {
    use transom_core::item::Glyph;
    let _g = crate::test_lang::guard();
    let Some(dir) = std::env::var_os("TRANSOM_PANEL_PNG") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for dark in [true, false] {
        let th = if dark { Theme::dark() } else { Theme::light() };
        let (w, h) = (360, 28);
        let surf = Surface::new(w, h);
        let mut p = GdiPainter::new(surf.dc, Size { w, h }, 96);
        let views: Vec<crate::view::AudioView> = [
            Glyph::Volume0,
            Glyph::Volume1,
            Glyph::Volume2,
            Glyph::Volume3,
            Glyph::VolumeMuted,
        ]
        .iter()
        .map(|g| crate::view::AudioView {
            glyph: *g,
            name: String::new(),
        })
        .collect();
        // jedna polozka na raz sa nezmesti: kazdy stav zvlast vedla seba v jednom obrazku cez 5 listov
        let surf2 = Surface::new(w, h * 5);
        for (i, v) in views.iter().enumerate() {
            let items = crate::view::build_items(&crate::view::View {
                clock: "14:05",
                audio: Some(v),
                ..Default::default()
            });
            let mut l = transom_config::Layout::default();
            l.set_visible("audio", true);
            let items = crate::view::arrange(items, &l);
            let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
            transom_render::render_bar_with(
                &mut gp,
                &th,
                &items,
                transom_render::Interaction::default(),
                None,
            );
            let rgb = surf.rgb();
            let _ = (&mut p, &surf2, i, rgb);
            let f = dir.join(format!(
                "bar-{}-{i}.png",
                if dark { "dark" } else { "light" }
            ));
            let _ = std::fs::write(
                f,
                transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
            );
        }
    }
}
