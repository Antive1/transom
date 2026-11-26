//! Zatazove testy listy: vsetky polozky zapnute a so skutocnymi dlhymi textami, vsetky v jednej skupine (vlavo,
//! v strede, vpravo) aj rozhadzane, pri roznych sirkach monitora a DPI. Kresli sa skutocnym `GdiPainter` cez
//! `Audit` (prekryvy textov, texty mimo listy) a kontroluje sa, ze kazda viditelna polozka je klikatelna.
use crate::ai::RingView;
use crate::extras::{BrightView, SysView, TimerView};
use crate::view::{arrange, build_items, AudioView, PeriphView, ShelfChip, ShelfView, View};
use transom_config::{Group, Layout};
use transom_core::battery::Battery;
use transom_core::item::{Action, Glyph, ItemId, Part, Side};
use transom_core::media::{Controls, PlayState, SessionInfo};
use transom_render::audit::{Audit, Issue};
use transom_render::offscreen::Surface;
use transom_render::{render_bar_with, GdiPainter, Interaction, Size, Theme};

const IDS: [&str; 22] = [
    "sep-1",
    "search",
    "media",
    "sep-2",
    "sep-3",
    "shelf",
    "ai",
    "audio",
    "tools",
    "tool_shelf",
    "tool_clipboard",
    "tool_notes",
    "tool_shots",
    "sep-4",
    "peripherals",
    "sysmon",
    "privacy",
    "brightness",
    "timer",
    "keyboard",
    "battery",
    "clock",
];

struct Data {
    media: SessionInfo,
    ring: RingView,
    audio: AudioView,
    periph: PeriphView,
    shelf: ShelfView,
    keyboard: transom_platform::keyboard::KeyState,
    timer: TimerView,
    bright: BrightView,
    sys: SysView,
    privacy: Vec<(Glyph, String, String)>,
}

fn data() -> Data {
    let device = |label: &str, percent| Part::Device {
        glyph: Glyph::Search,
        percent,
        charging: false,
        muted: false,
        low: false,
        label: label.into(),
        name: label.into(),
        action: Some(Action::PeripheralsPanel),
    };
    Data {
        media: SessionInfo {
            key: "Spotify.exe".into(),
            title: "An extremely long song title that never seems to end (Extended Remix)".into(),
            artist: "Some Artist With A Long Name".into(),
            state: PlayState::Playing,
            controls: Controls {
                pause: true,
                prev: true,
                next: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ring: RingView {
            fill: Some(62),
            ccw: false,
            muted: false,
            label: "62 % used".into(),
            name: "Claude".into(),
            alert: false,
            detail: Some("Claude · 5 h · resets in 2 h 14 min".into()),
        },
        audio: AudioView {
            glyph: Glyph::Volume2,
            name: "Speakers".into(),
        },
        periph: PeriphView {
            devices: vec![
                device("DeathAdder V4 Pro", Some(80)),
                device("NuPhy Air75", Some(15)),
                device("JBL Charge 5", None),
            ],
            more: 2,
        },
        shelf: ShelfView {
            files: (0..3)
                .map(|i| ShelfChip {
                    id: format!("f{i}"),
                    name: format!("a-really-long-file-name-number-{i}.pdf"),
                    dir: false,
                    available: true,
                })
                .collect(),
            more: 7,
        },
        keyboard: transom_platform::keyboard::KeyState {
            lang: "SK".into(),
            name: "Slovak (Slovakia)".into(),
            caps: true,
        },
        timer: TimerView {
            label: "24:59".into(),
            muted: false,
            name: "Pomodoro".into(),
        },
        bright: BrightView {
            level: Some(70),
            name: "Brightness".into(),
        },
        sys: SysView {
            text: "CPU 12 % · RAM 64 % · GPU 3 % · ↓ 4,6 MB/s".into(),
            name: "System".into(),
        },
        privacy: vec![(Glyph::Search, "Mic".into(), "Microphone in use".into())],
    }
}

/// Vsetky polozky viditelne; `side` = vsetky v jednej skupine (inak predvolene rozlozenie).
fn layout(side: Option<Group>) -> Layout {
    let mut l = Layout::default();
    // oddelovace aj na krajoch a dva po sebe: rozlozenie ich musi samo skryt
    for _ in 0..4 {
        l.add_separator(Group::Right);
    }
    l.add_separator(Group::Left);
    for (i, id) in IDS.iter().enumerate() {
        l.set_visible(id, true);
        if let Some(g) = side {
            l.move_to(id, g, i);
        }
    }
    l
}

#[test]
fn every_item_on_one_side_renders_without_overlap_or_lost_clicks_at_every_width_and_dpi() {
    let _g = crate::test_lang::guard();
    let d = data();
    let bat = Battery {
        percent: Some(76),
        charging: true,
        on_ac: true,
        saver: false,
    };
    let view = View {
        clock: "Wednesday, November 25, 2026 10:51",
        battery: Some(bat),
        media: Some(&d.media),
        notice: Some("Search did not open – press Win+S"),
        ai: Some(&d.ring),
        audio: Some(&d.audio),
        peripherals: Some(&d.periph),
        shelf: Some(&d.shelf),
        keyboard: Some(&d.keyboard),
        timer: Some(&d.timer),
        brightness: Some(&d.bright),
        sysmon: Some(&d.sys),
        privacy: &d.privacy,
        ..Default::default()
    };
    let mut problems = Vec::new();
    let is_sep = |id: ItemId| matches!(id, ItemId::Separator(_));
    for side in [
        None,
        Some(Group::Left),
        Some(Group::Center),
        Some(Group::Right),
    ] {
        let items = arrange(build_items(&view), &layout(side));
        assert!(
            items.iter().any(|i| i.id == ItemId::Clock),
            "hodiny chybaju v zozname ({side:?})"
        );
        for dpi in [96u32, 120, 144, 192, 288] {
            for width_lp in [640, 1024, 1366, 1920, 2560, 3840] {
                let s = dpi as f32 / 96.0;
                let (w, h) = (
                    (width_lp as f32 * s).round() as i32,
                    (28.0 * s).round() as i32,
                );
                for (dark, th) in [(true, Theme::dark()), (false, Theme::light())] {
                    let surf = Surface::new(w, h);
                    let mut g = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                    let mut a = Audit::new(&mut g, th.background);
                    // zamerne skracovane texty (nazov skladby, subory Police)
                    a.allow_elide = vec!["…".into(), "...".into()];
                    let info = render_bar_with(&mut a, &th, &items, Interaction::default(), None);
                    a.finish();
                    for i in &a.issues {
                        // orezanie pruzneho nazvu skladby je zamerne (posuvny text)
                        if matches!(i, Issue::Clipped { text, .. } if text.contains("song title")) {
                            continue;
                        }
                        problems.push(format!(
                            "{side:?} {width_lp} lp @ {dpi} dpi {}: {i:?}",
                            if dark { "dark" } else { "light" }
                        ));
                    }
                    // oddelovac len medzi dvoma zobrazenymi polozkami (nikdy na kraji skupiny ani dva po sebe)
                    for sd in [Side::Left, Side::Center, Side::Right] {
                        let mut seq: Vec<ItemId> = Vec::new();
                        for pl in &info.placed {
                            let side_ok = items.iter().any(|i| i.id == pl.item && i.side == sd);
                            if side_ok && seq.last() != Some(&pl.item) {
                                seq.push(pl.item);
                            }
                        }
                        for (k, id) in seq.iter().enumerate() {
                            if is_sep(*id) && (k == 0 || k + 1 == seq.len() || is_sep(seq[k + 1])) {
                                problems.push(format!(
                                    "{side:?} {width_lp} lp @ {dpi}: oddelovac navyse {seq:?}"
                                ));
                            }
                        }
                    }
                    // kazda umiestnena cast je klikatelna presne tam, kde je nakreslena
                    for pl in &info.placed {
                        if pl.w > 0 {
                            let hit = transom_core::layout::hit_test(&info.placed, pl.x + pl.w / 2);
                            if hit != Some((pl.item, pl.part)) {
                                problems.push(format!(
                                    "{side:?} {width_lp} lp @ {dpi}: hit {hit:?} != {pl:?}"
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} problemov:\n{}",
        problems.len(),
        problems
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
