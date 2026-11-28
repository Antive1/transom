//! Mixer aplikacii v paneli Zvuk: kreslenie (audit kontrastu a pretekania), pristupnost, mys, klavesy a koliesko
//! (offscreen, bez zvukoveho zariadenia; snimky mixera su vyrobene rucne).
use crate::audio_pop::{AudioCmd, AudioHit, AudioPanel, AUDIO_H_LP, AUDIO_W_LP};
use std::sync::Arc;
use transom_audio::{Availability, Controls, Device, DeviceState, Snapshot, Volume};
use transom_mixer::{App, OutDevice, Route, Snapshot as MixSnap};
use transom_platform::PointerKind;
use transom_render::audit::Audit;
use transom_render::offscreen::Surface;
use transom_render::{GdiPainter, Size, Theme};

fn audio() -> Snapshot {
    Snapshot {
        generation: 1,
        availability: Availability::Ok,
        devices: vec![Device {
            name: "Speakers (Realtek)".into(),
            key: 1,
            state: DeviceState::Active,
            is_default: true,
        }],
        volume: Some(Volume {
            percent: 40,
            muted: false,
        }),
        controls: Controls::default(),
        default_changes: 0,
    }
}

fn app(key: u64, name: &str, vol: u8, muted: bool, active: bool, route: Route) -> App {
    App {
        key,
        name: name.into(),
        exe: None,
        system: false,
        pids: vec![key as u32],
        volume: vol,
        muted,
        active,
        route,
    }
}

fn mix(routing: bool) -> MixSnap {
    let mut sys = app(9, "System sounds", 100, false, false, Route::Default);
    sys.system = true;
    MixSnap {
        ready: true,
        apps: vec![
            app(1, "Google Chrome", 100, false, true, Route::Default),
            app(
                2,
                "Spotify with a very long name that must never overflow the row",
                35,
                true,
                false,
                Route::Device("usb".into()),
            ),
            sys,
        ],
        devices: vec![
            OutDevice {
                id: "spk".into(),
                name: "Speakers (Realtek)".into(),
                is_default: true,
            },
            OutDevice {
                id: "usb".into(),
                name: "USB Headset".into(),
                is_default: false,
            },
        ],
        routing,
        route_failed: None,
        generation: 1,
    }
}

/// Nakresli panel s mixerom (dvakrat: prve kreslenie pripravi hit-boxy), vrati nalezy auditu.
fn paint(panel: &mut AudioPanel, m: &MixSnap, dpi: u32, th: &Theme) -> Vec<String> {
    let sc = dpi as f32 / 96.0;
    let (w, h) = (
        (AUDIO_W_LP as f32 * sc).round() as i32,
        (AUDIO_H_LP as f32 * 2.5 * sc).round() as i32,
    );
    let surf = Surface::new(w, h);
    let a = Arc::new(audio());
    let mm = Arc::new(m.clone());
    let mut out = Vec::new();
    for _ in 0..2 {
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
        let mut au = Audit::new(&mut gp, th.background);
        panel.paint_with(&mut au, th, &a, Some(Arc::clone(&mm)), None);
        au.finish();
        out = au.issues.iter().map(|i| format!("{i:?}")).collect();
    }
    out
}

fn center(p: &AudioPanel, h: AudioHit) -> (i32, i32) {
    let r = p
        .rect_of(h)
        .unwrap_or_else(|| panic!("{h:?} nie je v paneli"));
    (r.x + r.w / 2, r.y + r.h / 2)
}

#[test]
fn every_app_gets_a_named_slider_mute_and_output_choice_without_overflow_in_both_themes() {
    let _g = crate::test_lang::guard();
    for dpi in [96, 144] {
        for th in [Theme::dark(), Theme::light()] {
            let mut p = AudioPanel::default();
            p.open_route = Some(2);
            let issues = paint(&mut p, &mix(true), dpi, &th);
            assert!(issues.is_empty(), "{dpi}: {issues:?}");
        }
    }
    let mut p = AudioPanel::default();
    paint(&mut p, &mix(true), 96, &Theme::dark());
    let (entries, extras, _, _) = p.a11y();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"Apps"), "{names:?}");
    let at = |n: &str| {
        names
            .iter()
            .position(|x| *x == n)
            .unwrap_or_else(|| panic!("{n}: {names:?}"))
    };
    let chrome = at("Volume of Google Chrome");
    assert_eq!(extras[chrome].value, "100 %");
    assert_eq!(
        extras[chrome].role,
        transom_platform::menu_access::ROLE_SLIDER
    );
    // stlmena aplikacia je zaskrtnuty prepinac, vystup sa cita s nazvom zariadenia
    let sp = names
        .iter()
        .position(|n| n.starts_with("Unmute Spotify"))
        .unwrap();
    assert_ne!(
        extras[sp].state & transom_platform::menu_access::STATE_CHECKED,
        0
    );
    assert!(names
        .iter()
        .any(|n| n.starts_with("Output for Spotify") && n.ends_with("USB Headset")));
    assert!(names.contains(&"Output for Google Chrome: Default output"));
    // systemove zvuky maju vlastny vystup aj hlasitost
    at("Volume of System sounds");
    at("Output for System sounds: Default output");
    // bez rozhrania politiky: ziadny vyber vystupu, len odkaz na nastavenia Windows
    let mut p = AudioPanel::default();
    assert!(paint(&mut p, &mix(false), 96, &Theme::dark()).is_empty());
    assert!(p.rect_of(AudioHit::AppOut(1)).is_none());
    assert!(p.rect_of(AudioHit::AppsSettings).is_some());
    // nacitavanie a prazdny mixer
    for m in [
        MixSnap::default(),
        MixSnap {
            ready: true,
            routing: true,
            ..MixSnap::default()
        },
    ] {
        let mut p = AudioPanel::default();
        assert!(paint(&mut p, &m, 96, &Theme::dark()).is_empty());
        let (entries, _, _, _) = p.a11y();
        let want = if m.ready {
            "No app is using sound right now."
        } else {
            "Loading apps…"
        };
        assert!(entries.iter().any(|e| e.name == want));
    }
}

#[test]
fn sliders_mute_and_output_choice_turn_into_commands_for_the_right_app() {
    let _g = crate::test_lang::guard();
    let m = mix(true);
    let a = audio();
    let mut p = AudioPanel::default();
    paint(&mut p, &m, 96, &Theme::dark());
    // tahanie posuvnika Spotify: hodnoty idu len tejto aplikacii, nikdy nad 100
    let tr = p.rect_of(AudioHit::AppSlider(2)).unwrap();
    let y = tr.y + tr.h / 2;
    let (c, _) = p.pointer(PointerKind::Down, tr.x + tr.w / 2, y, &a);
    assert!(
        matches!(c, Some(AudioCmd::AppVolume(2, v)) if (40..=60).contains(&v)),
        "{c:?}"
    );
    let (c, _) = p.pointer(PointerKind::Move, tr.x + tr.w + 500, y, &a);
    assert_eq!(c, Some(AudioCmd::AppVolume(2, 100)));
    let (c, _) = p.pointer(PointerKind::Click, tr.x + tr.w + 500, y, &a);
    assert_eq!(c, None);
    // stlmenie: Chrome nie je stlmeny -> stlmit, Spotify je -> zapnut
    for (k, want) in [(1, true), (2, false)] {
        let (x, y) = center(&p, AudioHit::AppMute(k));
        p.pointer(PointerKind::Down, x, y, &a);
        let (c, _) = p.pointer(PointerKind::Click, x, y, &a);
        assert_eq!(c, Some(AudioCmd::AppMute(k, want)));
    }
    // vystup: rozbalit, vybrat USB (index 2 = devices[1]), potom predvoleny
    let (x, y) = center(&p, AudioHit::AppOut(1));
    p.pointer(PointerKind::Down, x, y, &a);
    assert_eq!(p.pointer(PointerKind::Click, x, y, &a).0, None);
    assert_eq!(p.open_route, Some(1));
    paint(&mut p, &m, 96, &Theme::dark());
    let (x, y) = center(&p, AudioHit::AppRoute(1, 2));
    p.pointer(PointerKind::Down, x, y, &a);
    assert_eq!(
        p.pointer(PointerKind::Click, x, y, &a).0,
        Some(AudioCmd::AppRoute(1, Some(1)))
    );
    assert_eq!(p.open_route, None);
    p.open_route = Some(2);
    paint(&mut p, &m, 96, &Theme::dark());
    let (x, y) = center(&p, AudioHit::AppRoute(2, 0));
    p.pointer(PointerKind::Down, x, y, &a);
    assert_eq!(
        p.pointer(PointerKind::Click, x, y, &a).0,
        Some(AudioCmd::AppRoute(2, None))
    );
    // klavesy na posuvniku aplikacie a koliesko nad nim
    paint(&mut p, &m, 96, &Theme::dark());
    p.focus = Some(AudioHit::AppSlider(2));
    assert_eq!(
        p.key(0x27, false, false, &a).0,
        Some(AudioCmd::AppVolume(2, 40))
    );
    assert_eq!(
        p.key(0x23, false, false, &a).0,
        Some(AudioCmd::AppVolume(2, 100))
    );
    assert_eq!(
        p.key(0x24, false, false, &a).0,
        Some(AudioCmd::AppVolume(2, 0))
    );
    // Chrome je na 100: dalej hore sa nezapisuje nic
    p.focus = Some(AudioHit::AppSlider(1));
    assert_eq!(p.key(0x27, false, false, &a).0, None);
    let (x, y) = center(&p, AudioHit::AppSlider(2));
    p.pointer(PointerKind::Move, x, y, &a);
    assert_eq!(p.wheel(-120, &a).0, Some(AudioCmd::AppVolume(2, 33)));
    // Tab prechadza aj ovladace aplikacii
    p.focus = None;
    let mut seen = Vec::new();
    for _ in 0..40 {
        p.key(0x09, false, false, &a);
        seen.push(p.focus);
    }
    for h in [
        AudioHit::AppMute(1),
        AudioHit::AppSlider(9),
        AudioHit::AppOut(2),
    ] {
        assert!(seen.contains(&Some(h)), "{h:?}");
    }
}

/// Pomocny test na pozretie vzhladu: `TRANSOM_PANEL_PNG=<adresar> cargo test -p transom-app mixer_png -- --ignored`.
#[test]
#[ignore]
fn mixer_png() {
    let _g = crate::test_lang::guard();
    let Some(dir) = std::env::var_os("TRANSOM_PANEL_PNG") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    for (lang, code) in [
        (transom_i18n::Lang::En, "en"),
        (transom_i18n::Lang::Sk, "sk"),
    ] {
        transom_i18n::set_lang(lang);
        for dark in [true, false] {
            let th = if dark { Theme::dark() } else { Theme::light() };
            let (w, h) = (AUDIO_W_LP, AUDIO_H_LP * 2);
            let surf = Surface::new(w, h);
            let mut p = AudioPanel::default();
            p.open_route = Some(2);
            p.focus = Some(AudioHit::AppSlider(1));
            let a = Arc::new(audio());
            let m = Arc::new(mix(true));
            for _ in 0..2 {
                let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
                p.paint_with(&mut gp, &th, &a, Some(Arc::clone(&m)), None);
            }
            let f = dir.join(format!(
                "mixer-{code}-{}.png",
                if dark { "dark" } else { "light" }
            ));
            let _ = std::fs::write(
                f,
                transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
            );
        }
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
}

#[test]
fn profiles_save_running_apps_apply_only_differences_and_never_route_to_missing_outputs() {
    use crate::audio_pop::ProfMode;
    use crate::extras::{profile_from, profile_plan};
    let _g = crate::test_lang::guard();
    let mut m = mix(true);
    // Chrome bez cesty .exe sa do profilu neulozi; Spotify a systemove zvuky ano
    m.apps[1].exe = Some(r"C:\Spotify\Spotify.exe".into());
    let p = profile_from("games", &m);
    assert_eq!(p.apps.len(), 2);
    assert_eq!(p.apps[0].exe, r"c:\spotify\spotify.exe");
    assert_eq!(
        (
            p.apps[0].volume,
            p.apps[0].muted,
            p.apps[0].device.as_deref()
        ),
        (35, true, Some("usb"))
    );
    assert_eq!(p.apps[1].exe, "system");
    // nic sa nezmenilo: ziadne prikazy
    assert!(profile_plan(&p, &m)
        .iter()
        .all(|(_, v, mu, r)| v.is_none() && mu.is_none() && r.is_none()));
    // zmeny: hlasitost a stlmenie spat, vystup len na pripojene zariadenie
    let mut now = m.clone();
    now.apps[1].volume = 80;
    now.apps[1].muted = false;
    now.apps[1].route = Route::Default;
    let plan = profile_plan(&p, &now);
    let sp = plan.iter().find(|x| x.0 == 2).unwrap();
    assert_eq!(
        (sp.1, sp.2, sp.3.clone()),
        (Some(35), Some(true), Some(Some("usb".into())))
    );
    now.devices.retain(|d| d.id != "usb");
    let sp = profile_plan(&p, &now)
        .into_iter()
        .find(|x| x.0 == 2)
        .unwrap();
    assert_eq!(sp.3, None, "odpojeny vystup: trasa ostava");
    // panel: rezimy profilov, audit a citacka
    let mut panel = AudioPanel::default();
    panel.profiles = vec![0, 2];
    for mode in [ProfMode::Apply, ProfMode::Save, ProfMode::Delete] {
        panel.prof_mode = mode;
        assert!(
            paint(&mut panel, &m, 96, &Theme::dark()).is_empty(),
            "{mode:?}"
        );
    }
    panel.prof_mode = ProfMode::Apply;
    paint(&mut panel, &m, 96, &Theme::light());
    let (entries, _, _, _) = panel.a11y();
    for want in [
        "Use the Games profile",
        "Use the Music profile",
        "Save current…",
        "Delete…",
    ] {
        assert!(entries.iter().any(|e| e.name == want), "{want}");
    }
    let a = audio();
    let (x, y) = center(&panel, AudioHit::ProfMode(1));
    panel.pointer(PointerKind::Down, x, y, &a);
    assert_eq!(panel.pointer(PointerKind::Click, x, y, &a).0, None);
    assert_eq!(panel.prof_mode, ProfMode::Save);
    paint(&mut panel, &m, 96, &Theme::dark());
    let (x, y) = center(&panel, AudioHit::ProfSave(1));
    panel.pointer(PointerKind::Down, x, y, &a);
    assert_eq!(
        panel.pointer(PointerKind::Click, x, y, &a).0,
        Some(AudioCmd::ProfSave(1))
    );
    assert_eq!(panel.prof_mode, ProfMode::Apply);
}

#[test]
fn paired_bluetooth_audio_devices_get_a_connect_button_and_report_failure() {
    use transom_mixer::bt::{BtDevice, BtSnap};
    let _g = crate::test_lang::guard();
    let b = BtSnap {
        ready: true,
        devices: vec![
            BtDevice {
                key: "{a}".into(),
                name: "JBL Charge 5".into(),
                connected: false,
            },
            BtDevice {
                key: "{b}".into(),
                name: "WH-1000XM4".into(),
                connected: true,
            },
        ],
        busy: None,
        failed: Some("{a}".into()),
        generation: 2,
    };
    let mut panel = AudioPanel::default();
    panel.bt = Some(Arc::new(b.clone()));
    for th in [Theme::dark(), Theme::light()] {
        assert!(paint(&mut panel, &mix(true), 144, &th).is_empty());
    }
    paint(&mut panel, &mix(true), 96, &Theme::dark());
    let (entries, _, _, _) = panel.a11y();
    assert!(entries
        .iter()
        .any(|e| e.name == "Connect: JBL Charge 5 (Not connected)"));
    assert!(entries
        .iter()
        .any(|e| e.name == "Disconnect: WH-1000XM4 (Connected)"));
    assert!(entries
        .iter()
        .any(|e| e.name.starts_with("The device did not respond")));
    let a = audio();
    for (i, want) in [(0u8, true), (1, false)] {
        let (x, y) = center(&panel, AudioHit::BtToggle(i));
        panel.pointer(PointerKind::Down, x, y, &a);
        assert_eq!(
            panel.pointer(PointerKind::Click, x, y, &a).0,
            Some(AudioCmd::BtSet(i, want))
        );
    }
    // pocas pripajania su tlacidla zakazane
    panel.bt = Some(Arc::new(BtSnap {
        busy: Some("{a}".into()),
        ..b
    }));
    paint(&mut panel, &mix(true), 96, &Theme::dark());
    assert!(panel.rect_of(AudioHit::BtToggle(0)).is_none());
    let (entries, _, _, _) = panel.a11y();
    assert!(entries
        .iter()
        .any(|e| e.name.starts_with("Connecting…: JBL")));
}
