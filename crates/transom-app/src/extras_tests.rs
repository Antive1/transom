//! Nove polozky lisaty (klavesnica, casovac, jas, vytazenie, mikrofon a kamera): co sa zobrazi a s akou akciou.
use crate::view::{build_items, View};
use transom_core::item::{Action, Glyph, ItemId, Part};

fn parts_of(v: &View, id: ItemId) -> Option<Vec<Part>> {
    build_items(v)
        .into_iter()
        .find(|i| i.id == id)
        .map(|i| i.parts)
}

#[test]
fn the_keyboard_item_shows_the_language_and_caps_lock_and_switches_on_click() {
    let _g = crate::test_lang::guard();
    let mut k = transom_platform::keyboard::KeyState {
        lang: "SK".into(),
        name: "Slovak (Slovakia)".into(),
        caps: false,
    };
    assert!(parts_of(&View::default(), ItemId::Keyboard).is_none());
    let p = parts_of(
        &View {
            keyboard: Some(&k),
            ..View::default()
        },
        ItemId::Keyboard,
    )
    .unwrap();
    assert_eq!(p.len(), 1);
    match &p[0] {
        Part::File {
            glyph,
            label,
            name,
            action,
            ..
        } => {
            assert_eq!((*glyph, label.as_str()), (Glyph::Keyboard, "SK"));
            assert!(
                name.contains("Slovak (Slovakia)") && name.contains("Win+Space"),
                "{name}"
            );
            assert_eq!(*action, Some(Action::KeyboardSwitch));
        }
        other => panic!("{other:?}"),
    }
    k.caps = true;
    k.lang.clear();
    let p = parts_of(
        &View {
            keyboard: Some(&k),
            ..View::default()
        },
        ItemId::Keyboard,
    )
    .unwrap();
    assert_eq!(p.len(), 2);
    assert!(matches!(&p[0], Part::File { label, .. } if label == "?"));
    assert!(
        matches!(&p[1], Part::File { glyph: Glyph::CapsLock, name, .. } if name == "Caps Lock is on")
    );
}

/// Nakresli panel zo sady (dvakrat: prve kreslenie pripravi hit-boxy), vrati nalezy auditu.
pub(crate) fn paint_kit(
    kit: &mut crate::pop_kit::Kit,
    rows: &[crate::pop_kit::Row],
    h_lp: i32,
    dpi: u32,
    th: &transom_render::Theme,
) -> Vec<String> {
    use transom_render::audit::Audit;
    use transom_render::offscreen::Surface;
    use transom_render::{GdiPainter, Size};
    let sc = dpi as f32 / 96.0;
    let (w, h) = (
        (crate::extras::KIT_W_LP as f32 * sc).round() as i32,
        (h_lp as f32 * sc).round() as i32,
    );
    let surf = Surface::new(w, h);
    let mut out = Vec::new();
    for _ in 0..2 {
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
        let mut au = Audit::new(&mut gp, th.background);
        kit.paint(&mut au, th, rows);
        au.finish();
        out = au.issues.iter().map(|i| format!("{i:?}")).collect();
    }
    if let Some(dir) = std::env::var_os("TRANSOM_PANEL_PNG") {
        let name = format!(
            "kit-{}-{dpi}-{}.png",
            kit.a11y().3,
            if th.background.r < 128 {
                "dark"
            } else {
                "light"
            }
        );
        let _ = std::fs::write(
            std::path::Path::new(&dir).join(name),
            transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
        );
    }
    out
}

#[test]
fn the_timer_item_and_panel_follow_the_countdown_pomodoro_and_stopwatch() {
    use crate::extras::{timer_rows, timer_view};
    use crate::pop_kit::{Cmd, Kit};
    use crate::timer::{Timers, MIN};
    use transom_platform::PointerKind;
    let _g = crate::test_lang::guard();
    let cfg = transom_config::TimerCfg::default();
    let mut tm = Timers::new();
    // necinnost: len ikona
    let v = timer_view(&tm, 0);
    assert!(v.label.is_empty() && v.name == "Timer");
    let p = parts_of(
        &View {
            timer: Some(&v),
            ..View::default()
        },
        ItemId::Timer,
    )
    .unwrap();
    assert!(matches!(
        &p[0],
        Part::Icon {
            glyph: Glyph::Timer,
            action: Some(Action::TimerPanel),
            ..
        }
    ));
    // bezi: zostavajuci cas v liste aj v nazve
    tm.start(0, 25 * MIN);
    let v = timer_view(&tm, 61_000);
    assert_eq!(v.label, "23:59");
    assert_eq!(v.name, "Countdown: 23:59 left");
    let p = parts_of(
        &View {
            timer: Some(&v),
            ..View::default()
        },
        ItemId::Timer,
    )
    .unwrap();
    assert!(matches!(&p[0], Part::File { label, muted: false, .. } if label == "23:59"));
    tm.toggle(61_000);
    assert!(timer_view(&tm, 999_999).muted);
    tm.reset();
    tm.stopwatch.toggle(0);
    assert_eq!(timer_view(&tm, 75_000).label, "1:15");
    // panel: audit v oboch temach a mierkach, s kolami stopiek a pomodorom
    tm.stopwatch.lap(30_000);
    tm.stopwatch.lap(75_000);
    tm.pomodoro_start(0, &crate::extras::pomo_cfg(&cfg));
    for dpi in [96, 144] {
        for th in [
            transom_render::Theme::dark(),
            transom_render::Theme::light(),
        ] {
            let mut kit = Kit::default();
            let issues = paint_kit(
                &mut kit,
                &timer_rows(&tm, &cfg, 80_000),
                crate::extras::kit_h(crate::POP_TIMER),
                dpi,
                &th,
            );
            assert!(issues.is_empty(), "{dpi}: {issues:?}");
        }
    }
    let mut kit = Kit::default();
    let rows = timer_rows(&Timers::new(), &cfg, 0);
    paint_kit(
        &mut kit,
        &rows,
        crate::extras::kit_h(crate::POP_TIMER),
        96,
        &transom_render::Theme::dark(),
    );
    let (entries, _, _, name) = kit.a11y();
    assert_eq!(name, "Timer");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    for want in [
        "Start a 5-minute timer",
        "Start pomodoro",
        "Start or pause the stopwatch",
        "Play a sound when time is up",
        "Countdown: 5:00",
    ] {
        assert!(names.contains(&want), "{want}: {names:?}");
    }
    // klik na predvolbu 10 min = prikaz s id 110; zakazane tlacidlo (Kolo) nema ciel
    let r = kit.rect_of(110).unwrap();
    kit.pointer(PointerKind::Down, r.x + 2, r.y + 2);
    assert_eq!(
        kit.pointer(PointerKind::Click, r.x + 2, r.y + 2).0,
        Some(Cmd::Click(110))
    );
    assert!(kit.rect_of(8).is_none());
    // klavesnica: Tab na prve povolene tlacidlo zlava (Reset je pri necinnom casovaci zakazany: -1 min), Enter ho
    // stlaci
    let mut kit2 = Kit::default();
    paint_kit(&mut kit2, &rows, 600, 96, &transom_render::Theme::dark());
    kit2.key(0x09, false, false);
    assert_eq!(kit2.key(0x0D, false, false).0, Some(Cmd::Click(3)));
}

#[test]
fn the_brightness_item_shows_the_main_monitor_and_the_panel_a_slider_per_monitor() {
    use crate::extras::{bright_rows, bright_view};
    use crate::pop_kit::{Cmd, Kit};
    use transom_platform::brightness::{Display, Snapshot};
    use transom_platform::PointerKind;
    let _g = crate::test_lang::guard();
    let snap = Snapshot {
        ready: true,
        displays: vec![
            Display {
                key: "a".into(),
                name: "Dell U2720Q".into(),
                level: Some(35),
                primary: false,
            },
            Display {
                key: "b".into(),
                name: "LG ULTRAGEAR+".into(),
                level: Some(80),
                primary: true,
            },
            Display {
                key: "c".into(),
                name: String::new(),
                level: None,
                primary: false,
            },
        ],
        failed: Some("a".into()),
        generation: 3,
    };
    let v = bright_view(&snap);
    assert_eq!(v.level, Some(80));
    assert_eq!(v.name, "Brightness 80 % (LG ULTRAGEAR+ (main))");
    let p = parts_of(
        &View {
            brightness: Some(&v),
            ..View::default()
        },
        ItemId::Brightness,
    )
    .unwrap();
    assert!(
        matches!(&p[0], Part::File { glyph: Glyph::Brightness, label, action: Some(Action::BrightnessPanel), .. } if label == "80 %")
    );
    // bez odpovede monitora: len ikona
    let none = bright_view(&Snapshot {
        ready: true,
        ..Snapshot::default()
    });
    let p = parts_of(
        &View {
            brightness: Some(&none),
            ..View::default()
        },
        ItemId::Brightness,
    )
    .unwrap();
    assert!(matches!(
        &p[0],
        Part::Icon {
            glyph: Glyph::Brightness,
            ..
        }
    ));
    for dpi in [96, 144] {
        for th in [
            transom_render::Theme::dark(),
            transom_render::Theme::light(),
        ] {
            let mut kit = Kit::default();
            let issues = paint_kit(&mut kit, &bright_rows(&snap), 480, dpi, &th);
            assert!(issues.is_empty(), "{dpi}: {issues:?}");
        }
    }
    let mut kit = Kit::default();
    paint_kit(
        &mut kit,
        &bright_rows(&snap),
        480,
        96,
        &transom_render::Theme::dark(),
    );
    let (entries, extras, _, _) = kit.a11y();
    let i = entries
        .iter()
        .position(|e| e.name == "Brightness of LG ULTRAGEAR+ (main)")
        .unwrap();
    assert_eq!(extras[i].value, "80 %");
    assert!(entries
        .iter()
        .any(|e| e.name == "The monitor did not confirm the change."));
    assert!(entries
        .iter()
        .any(|e| e.name.starts_with("The monitor is not answering")));
    // tahanie posuvnika druheho monitora = index 1; klavesy po 5 %
    let r = kit.rect_of(1).unwrap();
    let (c, _) = kit.pointer(PointerKind::Down, r.x + r.w - 9, r.y + r.h / 2);
    assert_eq!(c, Some(Cmd::Slide(1, 100)));
    kit.pointer(PointerKind::Click, r.x + r.w - 9, r.y + r.h / 2);
    kit.key(0x09, false, false);
    assert_eq!(kit.key(0x27, false, false).0, Some(Cmd::Slide(0, 40)));
    // nacitavanie
    let mut kit = Kit::default();
    assert!(paint_kit(
        &mut kit,
        &bright_rows(&Snapshot::default()),
        480,
        96,
        &transom_render::Theme::dark()
    )
    .is_empty());
}

#[test]
fn the_system_monitor_text_keeps_its_width_and_names_every_value() {
    use crate::extras::{rate, sys_view};
    use transom_platform::sysmon::Sample;
    let _g = crate::test_lang::guard();
    let all = transom_config::SysMonCfg {
        cpu: true,
        ram: true,
        gpu: true,
        net: true,
        interval_secs: 2,
    };
    let s = |cpu, gpu, d| Sample {
        cpu: Some(cpu),
        ram: Some(52),
        gpu: Some(gpu),
        down: Some(d),
        up: Some(4_664),
    };
    let a = sys_view(&s(5, 11, 2_725), &all);
    let b = sys_view(&s(100, 0, 4_600_000), &all);
    assert_eq!(
        a.text.chars().count(),
        b.text.chars().count(),
        "{} | {}",
        a.text,
        b.text
    );
    assert!(a.text.starts_with("CPU \u{2007}\u{2007}5 %"), "{}", a.text);
    assert!(b.text.contains("4.6 MB/s"), "{}", b.text);
    assert_eq!(
        a.name,
        "System use: processor 5 %, memory 52 %, graphics 11 %, download \u{2007}\u{2007}\u{2007}3 kB/s, upload \u{2007}\u{2007}\u{2007}5 kB/s. Click to open Task Manager"
    );
    assert_eq!(rate(500), "\u{2007}\u{2007}<1 kB/s");
    assert_eq!(rate(999_000), "\u{2007}999 kB/s");
    assert_eq!(rate(150_000_000), "\u{2007}150 MB/s");
    // prve meranie: pomlcky, rovnaka sirka; len vybrane hodnoty
    let first = sys_view(
        &Sample {
            ram: Some(52),
            ..Sample::default()
        },
        &transom_config::SysMonCfg::default(),
    );
    assert_eq!(first.text, "CPU \u{2007}\u{2007}– %  RAM \u{2007}52 %");
    let p = parts_of(
        &View {
            sysmon: Some(&first),
            ..View::default()
        },
        ItemId::SysMon,
    )
    .unwrap();
    assert!(matches!(
        &p[0],
        Part::File {
            action: Some(Action::SysMonOpen),
            ..
        }
    ));
}

#[test]
fn the_privacy_item_shows_who_uses_the_microphone_or_camera_and_mute_is_one_click() {
    use crate::extras::{priv_parts, priv_rows};
    use crate::pop_kit::{Cmd, Kit};
    use transom_mixer::capture::PrivSnap;
    use transom_platform::PointerKind;
    let _g = crate::test_lang::guard();
    let idle = PrivSnap {
        ready: true,
        mic_present: true,
        mic_muted: Some(false),
        ..PrivSnap::default()
    };
    // necinnost: ikona len pri "vzdy ukazat"
    assert!(priv_parts(&idle, false).is_empty());
    let p = priv_parts(&idle, true);
    assert_eq!(
        (p[0].0, p[0].2.as_str()),
        (Glyph::Mic, "Microphone and camera are not in use")
    );
    let busy = PrivSnap {
        mic_apps: vec!["Discord".into(), "OBS Studio".into()],
        cam_apps: vec!["WindowsCamera".into()],
        ..idle.clone()
    };
    let p = priv_parts(&busy, false);
    assert_eq!(p.len(), 2);
    assert_eq!(
        p[0],
        (
            Glyph::Mic,
            "Discord +1".into(),
            "Microphone in use: Discord, OBS Studio".into()
        )
    );
    assert_eq!(p[1].0, Glyph::Camera);
    let items = build_items(&View {
        privacy: &p,
        ..View::default()
    });
    let it = items.iter().find(|i| i.id == ItemId::Privacy).unwrap();
    assert!(it.parts.iter().all(|x| matches!(
        x,
        Part::File {
            action: Some(Action::PrivacyPanel),
            ..
        }
    )));
    // stlmeny mikrofon ma prednost a vlastny tvar
    let muted = PrivSnap {
        mic_muted: Some(true),
        ..busy.clone()
    };
    assert_eq!(priv_parts(&muted, false)[0].0, Glyph::MicOff);
    for dpi in [96, 144] {
        for th in [
            transom_render::Theme::dark(),
            transom_render::Theme::light(),
        ] {
            let mut kit = Kit::default();
            let rows = priv_rows(&busy, Some("Ctrl+Alt+M".into()));
            assert!(paint_kit(&mut kit, &rows, 480, dpi, &th).is_empty());
        }
    }
    let mut kit = Kit::default();
    paint_kit(
        &mut kit,
        &priv_rows(&muted, None),
        480,
        96,
        &transom_render::Theme::dark(),
    );
    let (entries, extras, _, _) = kit.a11y();
    let i = entries
        .iter()
        .position(|e| e.name == "Mute the microphone")
        .unwrap();
    assert_ne!(
        extras[i].state & transom_platform::menu_access::STATE_CHECKED,
        0
    );
    assert!(entries.iter().any(|e| e.name == "Discord, in use"));
    assert!(entries
        .iter()
        .any(|e| e.name == "The mute shortcut is off (Settings)."));
    let r = kit.rect_of(1).unwrap();
    kit.pointer(PointerKind::Down, r.x + 5, r.y + 5);
    assert_eq!(
        kit.pointer(PointerKind::Click, r.x + 5, r.y + 5).0,
        Some(Cmd::Click(1))
    );
    // bez mikrofonu: ziadny prepinac
    let mut kit = Kit::default();
    let none = PrivSnap {
        ready: true,
        ..PrivSnap::default()
    };
    paint_kit(
        &mut kit,
        &priv_rows(&none, None),
        480,
        96,
        &transom_render::Theme::dark(),
    );
    assert!(kit.rect_of(1).is_none());
}

#[test]
fn a_calendar_link_added_from_the_clipboard_is_found_in_the_clipboard_history() {
    use transom_tools::clipboard::ClipKind;
    use transom_tools::service::ClipRow;
    let row = |id: &str, preview: &str| ClipRow {
        id: id.into(),
        kind: ClipKind::Text,
        preview: preview.into(),
        app: String::new(),
        size: 0,
        pinned: false,
        added: 0,
    };
    let link = "https://calendar.google.com/calendar/ical/x%40gmail.com/private-abc123/basic.ics"
        .to_string();
    let rows = vec![
        row(
            "a",
            "https://calendar.google.com/calendar/ical/x%40gmail.com/private-abc123/basic.ics",
        ),
        row(
            "b",
            "webcal://calendar.google.com/calendar/ical/x%40gmail.com/private-abc123/basic.ics",
        ),
        row(
            "c",
            "https://calendar.google.com/calendar/ical/x%40gmail.com/priv…",
        ),
        row("d", "Nakupny zoznam"),
        row("e", "https://calendar.google.com/"),
    ];
    assert_eq!(
        crate::extras::clip_ids_with_links(&rows, &[link]),
        vec!["a", "b", "c"]
    );
}

#[test]
fn calendar_links_and_sound_profiles_are_never_exported() {
    let c = transom_config::Config {
        ics_calendars: vec![
            "https://calendar.google.com/calendar/ical/x/private-secret/basic.ics".into(),
        ],
        audio_profiles: vec![transom_config::AudioProfile {
            slot: "games".into(),
            apps: vec![transom_config::ProfileApp {
                exe: r"c:\games\secret-game.exe".into(),
                volume: 50,
                muted: false,
                device: Some("{0.0.0.00000000}.{dev-id}".into()),
            }],
        }],
        ..transom_config::Config::default()
    };
    let out = transom_config::transfer::export(
        &c,
        &transom_config::transfer::ExportOpts {
            app_version: "t".into(),
            exported_at: "t".into(),
        },
    );
    for secret in [
        "private-secret",
        "secret-game",
        "dev-id",
        "ics_calendars",
        "audio_profiles",
    ] {
        assert!(!out.contains(secret), "{secret} v exporte");
    }
}
