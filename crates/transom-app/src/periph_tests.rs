//! Panel a polozka Baterie periferii: stavy zariadeni, pristupnost, klavesnica, mys, vyber na listu a audit vzhladu
//! (offscreen, bez HID a Bluetooth; snimky stavu su vyrobene rucne).
use crate::periph_pop::{
    accessible_name, state_text, PeriphCmd, PeriphHit, PeriphPanel, PERIPH_H_LP, PERIPH_W_LP,
};
use std::sync::Arc;
use transom_core::item::{Action, ItemId, Part};
use transom_peripherals::{Availability, Kind, Peripheral, Snapshot, Source};
use transom_platform::PointerKind;
use transom_render::audit::Audit;
use transom_render::offscreen::Surface;
use transom_render::{GdiPainter, Size, Theme};

fn dev(name: &str, kind: Kind, source: Source, pct: Option<u8>, charging: bool) -> Peripheral {
    Peripheral {
        key: format!("{}:{name}", source.id()),
        name: name.into(),
        kind,
        source,
        percent: pct,
        charging,
        stale: false,
        age_secs: 0,
    }
}

fn snap(devices: Vec<Peripheral>) -> Snapshot {
    Snapshot {
        generation: 1,
        availability: Availability::Ready,
        devices,
    }
}

/// Stav z 2026-11-23 na vyvojovom PC plus typicke pripady (nabijanie bez urovne, spiace, nizka, Bluetooth).
fn normal() -> Snapshot {
    let mut asleep = dev(
        "Logitech MX Master 3S",
        Kind::Mouse,
        Source::Logitech,
        Some(40),
        false,
    );
    asleep.stale = true;
    asleep.age_secs = 25 * 60;
    snap(vec![
        dev(
            "Razer DeathAdder V4 Pro",
            Kind::Mouse,
            Source::Razer,
            Some(64),
            true,
        ),
        asleep,
        dev("NuPhy WH80", Kind::Keyboard, Source::NuPhy, None, true),
        dev(
            "WH-1000XM5",
            Kind::Headset,
            Source::Bluetooth,
            Some(9),
            false,
        ),
        dev(
            "Xbox Wireless Controller",
            Kind::Controller,
            Source::Bluetooth,
            Some(100),
            false,
        ),
    ])
}

fn paint(panel: &mut PeriphPanel, s: &Snapshot, dpi: u32, h_mul: f32, th: &Theme) -> Vec<String> {
    let sc = dpi as f32 / 96.0;
    let (w, h) = (
        (PERIPH_W_LP as f32 * sc).round() as i32,
        (PERIPH_H_LP as f32 * h_mul * sc).round() as i32,
    );
    let surf = Surface::new(w, h);
    let mut out = Vec::new();
    let a = Arc::new(s.clone());
    for _ in 0..2 {
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
        let mut au = Audit::new(&mut gp, th.background);
        panel.paint(&mut au, th, &a);
        au.finish();
        out = au.issues.iter().map(|i| format!("{i:?}")).collect();
    }
    out
}

#[test]
fn every_device_state_is_said_in_words_and_never_invented() {
    let _g = crate::test_lang::guard();
    let s = normal();
    let names: Vec<String> = s.devices.iter().map(accessible_name).collect();
    assert_eq!(names[0], "Razer DeathAdder V4 Pro: 64 %, charging");
    assert_eq!(
        names[1],
        "Logitech MX Master 3S: 40 %, asleep, last value 25 min ago"
    );
    // NuPhy pocas nabijania: ziadne percento, nikdy odhad
    assert_eq!(
        names[2],
        "NuPhy WH80: level unknown, charging, level not reported while charging"
    );
    assert_eq!(names[3], "WH-1000XM5: 9 %, low battery");
    assert_eq!(names[4], "Xbox Wireless Controller: 100 %");
    assert_eq!(state_text(&s.devices[4]), "");
    let mut full = s.devices[0].clone();
    full.percent = Some(100);
    assert_eq!(state_text(&full), "fully charged");
}

#[test]
fn the_panel_lists_devices_as_read_only_items_and_has_two_buttons() {
    use transom_platform::menu_access::{ROLE_LISTITEM, ROLE_PUSHBUTTON};
    let _g = crate::test_lang::guard();
    let mut p = PeriphPanel::default();
    let issues = paint(&mut p, &normal(), 96, 1.5, &Theme::dark());
    assert!(issues.is_empty(), "{issues:?}");
    let (entries, extras, focus, name) = p.a11y();
    assert_eq!(name, "Peripheral batteries");
    assert!(focus.is_none());
    let items: Vec<&str> = entries
        .iter()
        .zip(&extras)
        .filter(|(_, x)| x.role == ROLE_LISTITEM)
        .map(|(e, _)| e.name.as_str())
        .collect();
    assert_eq!(items.len(), 5);
    assert!(items[0].starts_with("Razer DeathAdder V4 Pro: 64 %"));
    for (e, x) in entries.iter().zip(&extras) {
        if x.role == ROLE_LISTITEM {
            assert!(
                x.action.is_empty() && !e.enabled,
                "zariadenie nie je ovladac"
            );
        }
    }
    let buttons: Vec<&str> = entries
        .iter()
        .zip(&extras)
        .filter(|(_, x)| x.role == ROLE_PUSHBUTTON)
        .map(|(e, _)| e.name.as_str())
        .collect();
    assert_eq!(buttons, ["Refresh", "Settings"]);
}

#[test]
fn starting_and_empty_states_have_their_own_text() {
    let _g = crate::test_lang::guard();
    for (s, key) in [
        (Snapshot::default(), "per.starting"),
        (snap(vec![]), "per.empty"),
    ] {
        let mut p = PeriphPanel::default();
        assert!(paint(&mut p, &s, 96, 1.0, &Theme::light()).is_empty());
        let (entries, ..) = p.a11y();
        assert!(
            entries.iter().any(|e| e.name == transom_i18n::t(key)),
            "{key}"
        );
    }
}

#[test]
fn keyboard_and_mouse_reach_both_buttons_and_only_a_full_click_activates() {
    let _g = crate::test_lang::guard();
    let mut p = PeriphPanel::default();
    paint(&mut p, &normal(), 96, 1.5, &Theme::dark());
    // Tab: Obnovit -> Nastavenia -> dookola; Enter aktivuje, opakovanie nie
    p.key(0x09, false, false);
    assert_eq!(p.focus, Some(PeriphHit::Refresh));
    assert_eq!(p.key(0x0D, false, false).0, Some(PeriphCmd::Refresh));
    assert_eq!(p.key(0x0D, false, true).0, None);
    p.key(0x09, false, false);
    assert_eq!(p.focus, Some(PeriphHit::Settings));
    p.key(0x09, false, false);
    assert_eq!(p.focus, Some(PeriphHit::Refresh));
    p.key(0x09, true, false);
    assert_eq!(p.focus, Some(PeriphHit::Settings));
    // mys: stlacenie a uvolnenie na tom istom tlacidle
    let (entries, extras, ..) = p.a11y();
    let r = entries
        .iter()
        .zip(&extras)
        .find(|(e, _)| e.name == "Settings")
        .map(|(e, _)| (e.x + e.w / 2, e.y + e.h / 2))
        .unwrap();
    let refresh = entries.iter().find(|e| e.name == "Refresh").unwrap();
    p.pointer(PointerKind::Down, refresh.x + 2, refresh.y + 2);
    assert_eq!(
        p.pointer(PointerKind::Click, r.0, r.1).0,
        None,
        "stlacene inde"
    );
    p.pointer(PointerKind::Down, r.0, r.1);
    assert_eq!(
        p.pointer(PointerKind::Click, r.0, r.1).0,
        Some(PeriphCmd::Settings)
    );
}

#[test]
fn the_bar_item_shows_lowest_first_when_limited_and_counts_the_rest() {
    let _g = crate::test_lang::guard();
    let s = normal();
    let v = crate::view::periph_view(&s, 2);
    // nabijajuce a bez urovne az na koniec; najnizsia uroven prva (9 % sluchadla, potom 40 % spiaca mys)
    let names: Vec<&str> = v
        .devices
        .iter()
        .map(|p| match p {
            Part::Device { name, .. } => name.split(':').next().unwrap(),
            _ => "?",
        })
        .collect();
    assert_eq!(names, ["WH-1000XM5", "Logitech MX Master 3S"]);
    assert_eq!(v.more, 3);
    match &v.devices[0] {
        Part::Device {
            low, label, action, ..
        } => {
            assert!(*low);
            assert_eq!(label, "9 %");
            assert_eq!(*action, Some(Action::PeripheralsPanel));
        }
        p => panic!("{p:?}"),
    }
    match &v.devices[1] {
        Part::Device { muted, low, .. } => assert!(*muted && !*low, "spiace: stlmene, nie nizke"),
        p => panic!("{p:?}"),
    }
    // vsetky: poradie ako v paneli (druh, nazov), nic navyse
    let all = crate::view::periph_view(&s, 0);
    assert_eq!((all.devices.len(), all.more), (5, 0));
    // polozka so zariadeniami + "+3"; bez zariadeni sa nezobrazi vobec
    let items = crate::view::build_items(&crate::view::View {
        clock: "12:00",
        peripherals: Some(&v),
        ..Default::default()
    });
    let it = items.iter().find(|i| i.id == ItemId::Peripherals).unwrap();
    assert_eq!(it.parts.len(), 3);
    assert!(matches!(&it.parts[2], Part::Text { text, .. } if text == "+3"));
    let empty = crate::view::periph_view(&snap(vec![]), 3);
    let items = crate::view::build_items(&crate::view::View {
        clock: "12:00",
        peripherals: Some(&empty),
        ..Default::default()
    });
    assert!(!items.iter().any(|i| i.id == ItemId::Peripherals));
}

#[test]
fn the_panel_audit_is_clean_in_every_language_theme_and_dpi() {
    let _g = crate::test_lang::guard();
    let long = "Wireless device with a really long product name for the clipping check";
    let mut many: Vec<Peripheral> = (0..12)
        .map(|i| {
            dev(
                &format!("{long} {i}"),
                Kind::Other,
                Source::Bluetooth,
                Some(i * 8),
                i % 2 == 0,
            )
        })
        .collect();
    many[3].stale = true;
    many[3].age_secs = 5 * 3600;
    let states = [
        ("normal", normal()),
        ("many", snap(many)),
        ("empty", snap(vec![])),
        ("starting", Snapshot::default()),
    ];
    let mut bad = Vec::new();
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for th in [Theme::dark(), Theme::light()] {
            for dpi in [96u32, 144, 192] {
                for (name, s) in &states {
                    let mut p = PeriphPanel::default();
                    for i in paint(&mut p, s, dpi, 3.0, &th) {
                        bad.push(format!("{} {dpi} {} {name}: {i}", lang.code(), th.dark));
                    }
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
fn a_new_app_never_starts_the_peripheral_service_without_permission() {
    let _g = crate::test_lang::guard();
    let mut a = crate::App::new(transom_render::ThemeMode::default(), None, None, false);
    assert!(a.periph.is_none() && !a.periph_allowed);
    assert!(a.periph_wanted(), "polozka je predvolene viditelna");
    a.sync_periph();
    assert!(a.periph.is_none(), "testy nikdy necitaju HID ani Bluetooth");
    assert_eq!(a.periph_snapshot().availability, Availability::Starting);
    // zdroje sa nevyberaju (transom rozpozna zariadenia sam): stara konfiguracia bez zdrojov sluzbu nevypne
    a.config.peripherals.sources.clear();
    assert!(a.periph_wanted());
    // so skrytou polozkou nie je co spustat
    a.config.peripherals = transom_config::Peripherals::default();
    a.config.layout.set_visible("peripherals", false);
    assert!(!a.periph_wanted());
}

/// Len rucne (`TRANSOM_PANEL_PNG=<adresar> cargo test -p transom-app live_periph_png -- --ignored`): precita skutocne
/// zariadenia tohto PC (len citanie) a vykresli listu a panel do PNG v oboch temach. Pri automatickych testoch sa
/// HID ani Bluetooth nikdy necita.
#[test]
#[ignore]
fn live_periph_png() {
    let _g = crate::test_lang::guard();
    let Some(dir) = std::env::var_os("TRANSOM_PANEL_PNG") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let svc = transom_peripherals::start_system(
        transom_peripherals::Source::ALL.to_vec(),
        std::time::Duration::from_secs(3600),
        Arc::new(|| {}),
    )
    .unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while svc.snapshot().availability != Availability::Ready && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let s = svc.snapshot();
    for d in &s.devices {
        eprintln!("ZARIADENIE {}", accessible_name(d));
    }
    for (dark, th) in [(true, Theme::dark()), (false, Theme::light())] {
        let tag = if dark { "dark" } else { "light" };
        for dpi in [96u32, 144] {
            let sc = dpi as f32 / 96.0;
            let (w, h) = ((640.0 * sc) as i32, (28.0 * sc) as i32);
            let surf = Surface::new(w, h);
            let v = crate::view::periph_view(&s, 3);
            let items = crate::view::build_items(&crate::view::View {
                clock: "14:05",
                peripherals: Some(&v),
                ..Default::default()
            });
            let items = crate::view::arrange(items, &transom_config::Layout::default());
            let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
            transom_render::render_bar_with(
                &mut gp,
                &th,
                &items,
                transom_render::Interaction::default(),
                None,
            );
            let _ = std::fs::write(
                dir.join(format!("live-bar-{tag}-{dpi}.png")),
                transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
            );
        }
        let (w, h) = (PERIPH_W_LP, PERIPH_H_LP);
        let surf = Surface::new(w, h);
        let mut p = PeriphPanel::default();
        for _ in 0..2 {
            let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
            p.paint(&mut gp, &th, &s);
        }
        let _ = std::fs::write(
            dir.join(format!("live-panel-{tag}.png")),
            transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
        );
    }
}

/// Len rucne (`TRANSOM_PANEL_PNG=<adresar> ... states_periph_png -- --ignored`): lista a panel so vsetkymi stavmi
/// (nabija sa, spi, nizka, bez urovne, plna) v oboch temach.
#[test]
#[ignore]
fn states_periph_png() {
    let _g = crate::test_lang::guard();
    let Some(dir) = std::env::var_os("TRANSOM_PANEL_PNG") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let s = normal();
    for (tag, th) in [("dark", Theme::dark()), ("light", Theme::light())] {
        let (w, h) = (900, 28);
        let surf = Surface::new(w, h);
        let v = crate::view::periph_view(&s, 0);
        let items = crate::view::build_items(&crate::view::View {
            clock: "14:05",
            peripherals: Some(&v),
            ..Default::default()
        });
        let items = crate::view::arrange(items, &transom_config::Layout::default());
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
        transom_render::render_bar_with(
            &mut gp,
            &th,
            &items,
            transom_render::Interaction::default(),
            None,
        );
        let _ = std::fs::write(
            dir.join(format!("states-bar-{tag}.png")),
            transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
        );
        let (w, h) = (PERIPH_W_LP, PERIPH_H_LP);
        let surf = Surface::new(w, h);
        let mut p = PeriphPanel::default();
        let a = Arc::new(s.clone());
        for _ in 0..2 {
            let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
            p.paint(&mut gp, &th, &a);
        }
        let _ = std::fs::write(
            dir.join(format!("states-panel-{tag}.png")),
            transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
        );
    }
}
