//! Prevod stavu aplikacie na stavove polozky (`transom_core::item`). Cista funkcia bez Win32 - testovatelna.
use transom_core::battery::Battery;
use transom_core::item::{Action, ArtImage, Flex, Glyph, ItemId, Part, Side, StatusItem, Tone};
use transom_core::media::{buttons, SessionInfo};
use transom_i18n::{t, tf};

pub use crate::ai::RingView;

/// Co sa ma zobrazit (odvodene zo stavu aplikacie).
#[derive(Clone, Debug, Default)]
pub struct View<'a> {
    pub clock: &'a str,
    pub battery: Option<Battery>,
    /// Zvolena media session.
    pub media: Option<&'a SessionInfo>,
    /// Obal zvolenej session (uz overeny voci aktualnej skladbe); `None` = zastupny symbol.
    pub art: Option<ArtImage>,
    /// Indikator vyberu `(i, n)`, ak je viac vierohodnych session.
    pub media_pick: Option<(usize, usize)>,
    /// Zobrazit napovedu pre Spotify (vlastne miesto za mediami; docasny oznam ju na chvilu nahradi).
    pub spotify_hint: bool,
    /// Docasny oznam (napr. zlyhanie skratky).
    pub notice: Option<&'a str>,
    /// Kruhovy indikator vyuzitia AI; `None` = polozka sa nezobrazi.
    pub ai: Option<&'a RingView>,
    /// Polozka Zvuk (reproduktor so stavom hlasitosti); `None` = polozka sa nezobrazi.
    pub audio: Option<&'a AudioView>,
    /// Baterie periferii; `None` = polozka nie je zapnuta, prazdny zoznam = ziadne zariadenie (polozka sa nezobrazi).
    pub peripherals: Option<&'a PeriphView>,
    /// Subory Police v liste; `None` alebo prazdne = polozka sa nezobrazi.
    pub shelf: Option<&'a ShelfView>,
    /// Rozlozenie klavesnice a Caps Lock; `None` = polozka sa nezobrazi.
    pub keyboard: Option<&'a transom_platform::keyboard::KeyState>,
    /// Casovac; `None` = polozka sa nezobrazi.
    pub timer: Option<&'a crate::extras::TimerView>,
    /// Jas monitora; `None` = polozka sa nezobrazi.
    pub brightness: Option<&'a crate::extras::BrightView>,
    /// Vytazenie systemu; `None` = polozka sa nezobrazi.
    pub sysmon: Option<&'a crate::extras::SysView>,
    /// Mikrofon a kamera: (glyf, text, nazov); prazdne = polozka sa nezobrazi.
    pub privacy: &'a [(Glyph, String, String)],
    /// Aktivne okno: (aplikacia, titulok); `None` = polozka sa nezobrazi.
    pub window: Option<&'a (String, String)>,
    /// Virtualne plochy: (pocet, aktualna od 0); `None` = polozka sa nezobrazi.
    pub desktops: Option<(usize, usize)>,
    /// Pocasie; `None` = polozka sa nezobrazi.
    pub weather: Option<&'a crate::weather::Weather>,
    /// Dostupna novsia verzia (ikona stahovania uplne vpravo); `None` = ikona sa nezobrazi.
    pub update: Option<&'a str>,
}

/// Najviac suborov Police priamo v liste (dalsie su v paneli Nastroje cez "+N").
pub const SHELF_ON_BAR: usize = 3;
/// Najviac znakov nazvu suboru v liste (dlhsi sa skrati, pripona ostane).
const SHELF_LABEL_CHARS: usize = 18;

/// Subor Police v liste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfChip {
    pub id: String,
    pub name: String,
    pub dir: bool,
    /// Subor je dostupny (nechyba, nie je sietovy ani len v cloude): da sa skopirovat aj potiahnut.
    pub available: bool,
}

/// Co ukazuje polozka Polica: prve subory (v poradi panela: pripnute hore) a pocet ostatnych.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShelfView {
    pub files: Vec<ShelfChip>,
    pub more: usize,
}

/// Subory Police na listu zo snimky sluzby nastrojov.
pub fn shelf_view(rows: &[transom_tools::service::ShelfRow]) -> ShelfView {
    use transom_tools::shelf::State;
    ShelfView {
        files: rows
            .iter()
            .take(SHELF_ON_BAR)
            .map(|r| ShelfChip {
                id: r.id.clone(),
                name: r.name.clone(),
                dir: r.dir,
                available: r.state == State::Available && !r.remote,
            })
            .collect(),
        more: rows.len().saturating_sub(SHELF_ON_BAR),
    }
}

/// Skrateny nazov suboru pre listu: najviac `SHELF_LABEL_CHARS` znakov, kratka pripona (do 5 znakov vratane bodky) ostane.
pub fn shelf_label(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= SHELF_LABEL_CHARS {
        return name.to_string();
    }
    let ext: Vec<char> = match name.rfind('.') {
        Some(i) if i > 0 && name[i..].chars().count() <= 5 => name[i..].chars().collect(),
        _ => Vec::new(),
    };
    let keep = SHELF_LABEL_CHARS - 1 - ext.len();
    let mut out: String = chars[..keep]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_string();
    out.push('…');
    out.extend(ext);
    out
}

/// Co ukazuje polozka Baterie periferii: casti zariadeni (najviac `on_bar`) a pocet ostatnych.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeriphView {
    pub devices: Vec<Part>,
    pub more: usize,
}

/// Zariadenia na listu zo snimky sluzby (`on_bar` 0 = vsetky; pri menej je najnizsia uroven prva).
pub fn periph_view(snap: &transom_peripherals::Snapshot, on_bar: u32) -> PeriphView {
    use crate::periph_pop::{accessible_name, glyph_of, LOW_PERCENT};
    let max = if on_bar == 0 {
        usize::MAX
    } else {
        on_bar as usize
    };
    let lowest_first = max < snap.devices.len();
    let (shown, more) = transom_peripherals::for_bar(&snap.devices, max, lowest_first);
    PeriphView {
        devices: shown
            .into_iter()
            .map(|d| Part::Device {
                glyph: glyph_of(d.kind),
                percent: d.percent,
                charging: d.charging,
                muted: d.stale,
                low: d.is_low(LOW_PERCENT) && !d.stale,
                label: transom_peripherals::short_label(d),
                name: accessible_name(d),
                action: Some(Action::PeripheralsPanel),
            })
            .collect(),
        more,
    }
}

/// Co ukazuje polozka Zvuk: glyf (hlasitost / stlmenie) a pristupny nazov s hodnotou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioView {
    pub glyph: Glyph,
    pub name: String,
}

/// Nazov skladby: najmenej 80 lp (~9 znakov + "...") - uzsi usek by bol necitatelny, tak sa radsej skryje cela cast;
/// pod `MIN_READABLE_LP` (120) sa uz nehybe, len ukaze staticky usek s "..." (pozri `marquee`).
const TITLE_FLEX: Flex = Flex {
    min_lp: 80,
    max_lp: 420,
};
const HINT_FLEX: Flex = Flex {
    min_lp: 90,
    max_lp: 760,
};
const NOTICE_FLEX: Flex = Flex {
    min_lp: 90,
    max_lp: 560,
};

/// Oznacenie poskytovatela za kruhom: volitelna cast (pri nedostatku miesta sa skryje skor nez polozka),
/// nikdy sa neskracuje.
const PROVIDER_FLEX: Flex = Flex {
    min_lp: 400,
    max_lp: 400,
};

fn media_button(glyph: Glyph, action: Action, enabled: bool, key: &str) -> Part {
    let name = if enabled {
        t(key)
    } else {
        tf("media.unsupported", &[&t(key)])
    };
    Part::button(glyph, action, enabled, name)
}

pub fn build_items(v: &View) -> Vec<StatusItem> {
    let mut out = vec![StatusItem::new(
        ItemId::Search,
        Side::Left,
        vec![Part::button(
            Glyph::Search,
            Action::Launcher,
            true,
            t("search.name"),
        )],
    )];

    let mut media = Vec::new();
    if let Some(s) = v.media {
        let b = buttons(s);
        media.push(Part::Art {
            art: v.art.clone(),
            action: Some(Action::MediaOpenSource),
        });
        media.push(media_button(
            Glyph::Prev,
            Action::MediaPrev,
            b.prev_enabled,
            "media.prev",
        ));
        let (glyph, name) = if b.shows_pause {
            (Glyph::Pause, "media.pause")
        } else {
            (Glyph::Play, "media.play")
        };
        media.push(media_button(
            glyph,
            Action::MediaToggle,
            b.toggle_enabled,
            name,
        ));
        media.push(media_button(
            Glyph::Next,
            Action::MediaNext,
            b.next_enabled,
            "media.next",
        ));
        media.push(Part::Text {
            text: s.display_text(),
            flex: Some(TITLE_FLEX),
            tone: Tone::Normal,
            action: None,
        });
        if let Some((i, n)) = v.media_pick {
            media.push(Part::Text {
                text: format!("{i}/{n}"),
                flex: None,
                tone: Tone::Muted,
                action: Some(Action::MediaCycle),
            });
        }
    }
    out.push(StatusItem::new(ItemId::Media, Side::Left, media));

    // oblast oznamu: docasny oznam ma prednost, inak napoveda pre Spotify (klik ju skryje)
    if let Some(n) = v.notice {
        out.push(StatusItem::new(
            ItemId::Notice,
            Side::Left,
            vec![Part::Text {
                text: n.to_string(),
                flex: Some(NOTICE_FLEX),
                tone: Tone::Muted,
                action: None,
            }],
        ));
    } else if v.spotify_hint {
        out.push(StatusItem::new(
            ItemId::Notice,
            Side::Left,
            vec![Part::Text {
                text: t("spotify.hint"),
                flex: Some(HINT_FLEX),
                tone: Tone::Muted,
                action: Some(Action::DismissHint),
            }],
        ));
    }

    out.push(StatusItem::new(
        ItemId::Tools,
        Side::Right,
        vec![Part::button(
            Glyph::Tools,
            Action::ToolsPanel,
            true,
            t("tools.name"),
        )],
    ));
    // samostatne tlacidla nastrojov (zobrazia sa len, ked su zapnute v rozlozeni)
    for (id, glyph, action, key) in TOOL_BUTTONS {
        out.push(StatusItem::new(
            id,
            Side::Right,
            vec![Part::button(glyph, action, true, t(key))],
        ));
    }
    if let Some(b) = v.battery {
        out.push(StatusItem::new(
            ItemId::Battery,
            Side::Right,
            vec![
                Part::Battery {
                    percent: b.percent.unwrap_or(0),
                    charging: b.charging,
                },
                Part::text(b.label()),
            ],
        ));
    }
    if let Some(a) = v.ai {
        let mut parts = vec![Part::Ring {
            fill: a.fill,
            ccw: a.ccw,
            muted: a.muted,
            alert: a.alert,
            label: a.label.clone(),
            name: a.name.clone(),
            action: Some(Action::AiPanel),
        }];
        if let Some(pv) = &a.detail {
            parts.push(Part::Text {
                text: pv.clone(),
                flex: Some(PROVIDER_FLEX),
                tone: Tone::Muted,
                action: Some(Action::AiPanel),
            });
        }
        out.push(StatusItem::new(ItemId::Ai, Side::Right, parts));
    }
    if let Some(k) = v.keyboard {
        let lang = if k.lang.is_empty() {
            "?".to_string()
        } else {
            k.lang.clone()
        };
        let mut parts = vec![Part::File {
            glyph: Glyph::Keyboard,
            muted: false,
            label: lang.clone(),
            name: tf(
                "kb.name",
                &[if k.name.is_empty() { &lang } else { &k.name }],
            ),
            action: Some(Action::KeyboardSwitch),
        }];
        if k.caps {
            parts.push(Part::File {
                glyph: Glyph::CapsLock,
                muted: false,
                label: t("kb.caps_short"),
                name: t("kb.caps"),
                action: Some(Action::KeyboardSwitch),
            });
        }
        out.push(StatusItem::new(ItemId::Keyboard, Side::Right, parts));
    }
    if !v.privacy.is_empty() {
        let parts = v
            .privacy
            .iter()
            .map(|(g, label, name)| {
                if label.is_empty() {
                    Part::button(*g, Action::PrivacyPanel, true, name.clone())
                } else {
                    Part::File {
                        glyph: *g,
                        muted: false,
                        label: label.clone(),
                        name: name.clone(),
                        action: Some(Action::PrivacyPanel),
                    }
                }
            })
            .collect();
        out.push(StatusItem::new(ItemId::Privacy, Side::Right, parts));
    }
    if let Some(sv) = v.sysmon.filter(|s| !s.text.is_empty()) {
        out.push(StatusItem::new(
            ItemId::SysMon,
            Side::Right,
            vec![Part::File {
                glyph: Glyph::Device,
                muted: false,
                label: sv.text.clone(),
                name: sv.name.clone(),
                action: Some(Action::SysMonOpen),
            }],
        ));
    }
    if let Some(bv) = v.brightness {
        let part = match bv.level {
            Some(l) => Part::File {
                glyph: Glyph::Brightness,
                muted: false,
                label: format!("{l} %"),
                name: bv.name.clone(),
                action: Some(Action::BrightnessPanel),
            },
            None => Part::button(
                Glyph::Brightness,
                Action::BrightnessPanel,
                true,
                bv.name.clone(),
            ),
        };
        out.push(StatusItem::new(ItemId::Brightness, Side::Right, vec![part]));
    }
    if let Some(tv) = v.timer {
        let part = if tv.label.is_empty() {
            Part::button(Glyph::Timer, Action::TimerPanel, true, tv.name.clone())
        } else {
            Part::File {
                glyph: Glyph::Timer,
                muted: tv.muted,
                label: tv.label.clone(),
                name: tv.name.clone(),
                action: Some(Action::TimerPanel),
            }
        };
        out.push(StatusItem::new(ItemId::Timer, Side::Right, vec![part]));
    }
    if let Some(pv) = v.peripherals.filter(|pv| !pv.devices.is_empty()) {
        let mut parts = pv.devices.clone();
        if pv.more > 0 {
            parts.push(Part::Text {
                text: tf("per.more", &[&pv.more.to_string()]),
                flex: None,
                tone: Tone::Muted,
                action: Some(Action::PeripheralsPanel),
            });
        }
        out.push(StatusItem::new(ItemId::Peripherals, Side::Right, parts));
    }
    if let Some((app, title)) = v.window {
        let text = if title.is_empty() || title == app {
            app.clone()
        } else if app.is_empty() {
            title.clone()
        } else {
            format!("{app} \u{2014} {title}")
        };
        out.push(StatusItem::new(
            ItemId::Window,
            Side::Left,
            vec![Part::Text {
                text,
                flex: Some(Flex {
                    min_lp: 80,
                    max_lp: 360,
                }),
                tone: Tone::Muted,
                action: None,
            }],
        ));
    }
    if let Some((n, cur)) = v.desktops.filter(|(n, _)| *n > 0) {
        out.push(StatusItem::new(
            ItemId::Desktops,
            Side::Left,
            vec![Part::File {
                glyph: Glyph::Desktops,
                muted: false,
                label: format!("{} / {n}", cur + 1),
                name: tf("desk.name", &[&(cur + 1).to_string(), &n.to_string()]),
                action: Some(Action::DesktopNext),
            }],
        ));
    }
    if let Some(w) = v.weather {
        let desc = t(crate::weather::desc_key(w.code));
        out.push(StatusItem::new(
            ItemId::Weather,
            Side::Right,
            vec![Part::File {
                glyph: Glyph::Weather,
                muted: false,
                label: crate::weather::temp(w.temp),
                name: tf(
                    "wx.name",
                    &[
                        &crate::weather::place_name(w),
                        &crate::weather::temp(w.temp),
                        &desc,
                    ],
                ),
                action: Some(Action::WeatherOpen),
            }],
        ));
    }
    // odlozene subory: jedna ikona s poctom (zoznam sa ukaze vo vybezku po prejdeni mysou, klik otvori Nastroje);
    // zobrazuje sa aj prazdna, aby stred, ktory im patri, nebol prazdny
    if let Some(sv) = v.shelf {
        let n = sv.files.len() + sv.more;
        out.push(StatusItem::new(
            ItemId::Shelf,
            Side::Center,
            vec![Part::File {
                glyph: Glyph::Shelf,
                muted: n == 0,
                label: if n == 0 { String::new() } else { n.to_string() },
                name: tf("shelf.bar_name", &[&n.to_string()]),
                action: Some(Action::ToolsShelf),
            }],
        ));
    }
    if let Some(a) = v.audio {
        out.push(StatusItem::new(
            ItemId::Audio,
            Side::Right,
            vec![Part::button(
                a.glyph,
                Action::AudioPanel,
                true,
                a.name.clone(),
            )],
        ));
    }
    out.push(StatusItem::new(
        ItemId::Clock,
        Side::Right,
        vec![Part::Text {
            text: v.clock.to_string(),
            flex: None,
            tone: Tone::Normal,
            action: Some(Action::Calendar),
        }],
    ));
    if let Some(ver) = v.update {
        out.push(StatusItem::new(
            ItemId::Update,
            Side::Right,
            vec![Part::button(
                Glyph::Download,
                Action::UpdatePanel,
                true,
                tf("upd.icon_name", &[ver]),
            )],
        ));
    }
    out
}

/// Samostatne tlacidla nastrojov: (polozka, ikona, akcia, kluc pristupneho nazvu).
pub const TOOL_BUTTONS: [(ItemId, Glyph, Action, &str); 7] = [
    (
        ItemId::ToolShelf,
        Glyph::Shelf,
        Action::ToolsShelf,
        "tools.sec_shelf",
    ),
    (
        ItemId::ToolClipboard,
        Glyph::Clipboard,
        Action::ToolsClipboard,
        "tools.sec_clipboard",
    ),
    (
        ItemId::ToolNotes,
        Glyph::Note,
        Action::ToolsNotes,
        "tools.sec_notes",
    ),
    (
        ItemId::ToolShots,
        Glyph::Screenshot,
        Action::ToolsShots,
        "tools.sec_shots",
    ),
    (
        ItemId::ToolSnip,
        Glyph::Snip,
        Action::SnipToStash,
        "snip.name",
    ),
    (
        ItemId::ToolPicker,
        Glyph::Picker,
        Action::ColorPick,
        "pick.name",
    ),
    (
        ItemId::Focus,
        Glyph::Focus,
        Action::FocusPanel,
        "focus.name",
    ),
];

/// Id polozky v konfiguracii -> polozka lisaty.
pub fn id_of(s: &str) -> Option<ItemId> {
    if let Some(n) = transom_config::separator_no(s) {
        return Some(ItemId::Separator(n));
    }
    Some(match s {
        "tool_shelf" => ItemId::ToolShelf,
        "tool_clipboard" => ItemId::ToolClipboard,
        "tool_notes" => ItemId::ToolNotes,
        "tool_shots" => ItemId::ToolShots,
        "tool_snip" => ItemId::ToolSnip,
        "tool_picker" => ItemId::ToolPicker,
        "window" => ItemId::Window,
        "desktops" => ItemId::Desktops,
        "focus" => ItemId::Focus,
        "weather" => ItemId::Weather,
        "search" => ItemId::Search,
        "media" => ItemId::Media,
        "ai" => ItemId::Ai,
        "audio" => ItemId::Audio,
        "tools" => ItemId::Tools,
        "battery" => ItemId::Battery,
        "peripherals" => ItemId::Peripherals,
        "shelf" => ItemId::Shelf,
        "keyboard" => ItemId::Keyboard,
        "timer" => ItemId::Timer,
        "brightness" => ItemId::Brightness,
        "sysmon" => ItemId::SysMon,
        "privacy" => ItemId::Privacy,
        "clock" => ItemId::Clock,
        _ => return None,
    })
}

/// Usporiada polozky podla konfiguracie: skupina (vlavo/stred/vpravo), poradie a viditelnost. Neznáme id sa
/// preskocia (zachovaju sa v subore), skryte polozky sa vynechaju. Docasny oznam (nie je v konfiguracii) ide hned
/// za media v ich skupine, pri skrytych mediach na koniec lavej skupiny. Ikona dostupnej aktualizacie (tiez nie je
/// v konfiguracii) ide vzdy uplne doprava.
pub fn arrange(items: Vec<StatusItem>, layout: &transom_config::Layout) -> Vec<StatusItem> {
    use transom_config::Group;
    let mut pool: Vec<Option<StatusItem>> = items.into_iter().map(Some).collect();
    let mut take = |id: ItemId| -> Option<StatusItem> {
        pool.iter_mut()
            .find(|i| i.as_ref().is_some_and(|x| x.id == id))
            .and_then(Option::take)
    };
    let notice = take(ItemId::Notice);
    let update = take(ItemId::Update);
    let mut out = Vec::new();
    let mut notice_placed = false;
    for slot in layout.items.iter().filter(|s| s.visible) {
        let Some(id) = id_of(&slot.id) else { continue };
        let side = match slot.group {
            Group::Left => Side::Left,
            Group::Center => Side::Center,
            Group::Right => Side::Right,
        };
        // oddelovac nema data: vznikne priamo z rozlozenia
        if let ItemId::Separator(_) = id {
            out.push(StatusItem::new(id, side, vec![Part::Divider]));
            continue;
        }
        if let Some(mut it) = take(id) {
            it.side = side;
            out.push(it);
            if id == ItemId::Media {
                if let Some(mut n) = notice.clone() {
                    n.side = side;
                    out.push(n);
                    notice_placed = true;
                }
            }
        }
    }
    if !notice_placed {
        if let Some(mut n) = notice {
            n.side = Side::Left;
            out.push(n);
        }
    }
    if let Some(mut u) = update {
        u.side = Side::Right;
        out.push(u);
    }
    out
}

#[cfg(test)]
mod arrange_tests {
    use super::*;
    use transom_config::{Group, Layout};

    fn ring() -> RingView {
        RingView {
            fill: Some(62),
            ccw: false,
            muted: false,
            alert: false,
            label: "62 % used".into(),
            name: "AI".into(),
            detail: Some("Claude".into()),
        }
    }

    fn all() -> Vec<StatusItem> {
        build_items(&View {
            clock: "14:05",
            notice: Some("oznam"),
            ai: Some(&ring()),
            ..Default::default()
        })
    }

    fn sides(v: &[StatusItem]) -> Vec<(ItemId, Side)> {
        v.iter().map(|i| (i.id, i.side)).collect()
    }

    #[test]
    fn default_layout_matches_previous_bar() {
        let _lang = crate::test_lang::guard();
        let mut l = Layout::default();
        l.set_visible("ai", true);
        let v = arrange(all(), &l);
        assert_eq!(
            sides(&v),
            vec![
                (ItemId::Search, Side::Left),
                (ItemId::Media, Side::Left),
                (ItemId::Notice, Side::Left),
                (ItemId::Ai, Side::Right),
                (ItemId::Tools, Side::Right),
                (ItemId::Clock, Side::Right),
            ]
        );
    }

    #[test]
    fn groups_order_and_hidden_items_follow_config() {
        let _lang = crate::test_lang::guard();
        let mut l = Layout::default();
        l.move_to("clock", Group::Center, 0);
        l.move_to("search", Group::Right, 99);
        l.set_visible("tools", false);
        let v = arrange(all(), &l);
        assert_eq!(
            sides(&v),
            vec![
                (ItemId::Media, Side::Left),
                (ItemId::Notice, Side::Left),
                // hodiny idu do stredu pred Policu (jej slot je v strede uz predvolene)
                (ItemId::Clock, Side::Center),
                (ItemId::Search, Side::Right),
            ]
        );
    }

    #[test]
    fn the_update_icon_is_rightmost_without_being_in_the_saved_layout() {
        let _lang = crate::test_lang::guard();
        let mk = |update: Option<&str>| {
            build_items(&View {
                clock: "14:05",
                update,
                ..Default::default()
            })
        };
        // bez dostupnej verzie ziadna ikona
        assert!(!arrange(mk(None), &Layout::default())
            .iter()
            .any(|i| i.id == ItemId::Update));
        // rozlozenie ulozene pred touto funkciou (polozku nepozna) aj hodiny v strede: ikona je posledna vpravo
        let mut l = Layout::default();
        assert!(!l.items.iter().any(|s| s.id == "update"));
        for layout in [l.clone(), {
            l.move_to("clock", Group::Center, 0);
            l
        }] {
            let v = arrange(mk(Some("0.3.0")), &layout);
            let last = v.iter().rfind(|i| i.side == Side::Right).unwrap();
            assert_eq!(last.id, ItemId::Update);
            assert_eq!(v.iter().filter(|i| i.id == ItemId::Update).count(), 1);
            match &last.parts[..] {
                [Part::Icon {
                    glyph: Glyph::Download,
                    action: Some(Action::UpdatePanel),
                    enabled: true,
                    name,
                }] => assert_eq!(name, "Update available: transom 0.3.0"),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn notice_survives_hidden_media_and_unknown_ids_are_skipped() {
        let _lang = crate::test_lang::guard();
        let mut l = Layout::default();
        l.set_visible("media", false);
        l.items.insert(
            0,
            transom_config::Slot {
                id: "weather".into(),
                group: Group::Left,
                visible: true,
            },
        );
        let v = arrange(all(), &l);
        assert!(v
            .iter()
            .any(|i| i.id == ItemId::Notice && i.side == Side::Left));
        assert!(!v.iter().any(|i| i.id == ItemId::Media));
        // vsetko skryte -> prazdna lista (vstup do nastaveni ostava cez prave tlacidlo na liste)
        let mut none = Layout::default();
        for s in none.items.iter_mut() {
            s.visible = false;
        }
        let v = arrange(
            build_items(&View {
                clock: "1",
                ..Default::default()
            }),
            &none,
        );
        assert!(v.is_empty());
    }

    #[test]
    fn clock_opens_calendar_and_ai_opens_panel() {
        let _lang = crate::test_lang::guard();
        let v = all();
        let clock = v.iter().find(|i| i.id == ItemId::Clock).unwrap();
        assert_eq!(clock.parts[0].action(), Some(Action::Calendar));
        let ai = v.iter().find(|i| i.id == ItemId::Ai).unwrap();
        assert_eq!(ai.parts[0].action(), Some(Action::AiPanel));
        assert!(
            matches!(&ai.parts[0], Part::Ring { fill: Some(62), label, .. } if label == "62 % used")
        );
        assert!(matches!(&ai.parts[1], Part::Text { text, flex: Some(_), .. } if text == "Claude"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::media::{Controls, PlayState};

    fn ids(items: &[StatusItem]) -> Vec<ItemId> {
        items
            .iter()
            .filter(|i| !i.parts.is_empty())
            .map(|i| i.id)
            .collect()
    }

    #[test]
    fn desktop_without_battery_or_media() {
        let _lang = crate::test_lang::guard();
        let items = build_items(&View {
            clock: "14:05",
            ..Default::default()
        });
        assert_eq!(
            ids(&items),
            // samostatne tlacidla nastrojov sa stavaju vzdy (zobrazi ich az rozlozenie)
            vec![
                ItemId::Search,
                ItemId::Tools,
                ItemId::ToolShelf,
                ItemId::ToolClipboard,
                ItemId::ToolNotes,
                ItemId::ToolShots,
                ItemId::ToolSnip,
                ItemId::ToolPicker,
                ItemId::Focus,
                ItemId::Clock
            ]
        );
    }

    #[test]
    fn media_shows_all_controls_disabling_unsupported() {
        let _lang = crate::test_lang::guard();
        let s = SessionInfo {
            key: "Brave".into(),
            title: "Video".into(),
            state: PlayState::Playing,
            controls: Controls {
                pause: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let items = build_items(&View {
            clock: "1",
            media: Some(&s),
            media_pick: Some((1, 2)),
            ..Default::default()
        });
        let m = items.iter().find(|i| i.id == ItemId::Media).unwrap();
        // obal, prev, toggle, next, nazov, indikator
        assert_eq!(m.parts.len(), 6);
        assert!(
            matches!(m.parts[0], Part::Art { art: None, .. }),
            "bez obalu = zastupny symbol"
        );
        // akcie: nepodporovane prev/next nemaju akciu (klik nic neodosle), toggle ano
        let actions: Vec<_> = m.parts.iter().map(|p| p.action()).collect();
        assert_eq!(
            actions,
            vec![
                Some(Action::MediaOpenSource),
                None,
                Some(Action::MediaToggle),
                None,
                None,
                Some(Action::MediaCycle)
            ]
        );
        match &m.parts[1] {
            Part::Icon {
                glyph: Glyph::Prev,
                enabled: false,
                name,
                ..
            } => assert_eq!(name, &tf("media.unsupported", &[&t("media.prev")])),
            p => panic!("{p:?}"),
        }
        assert!(matches!(
            &m.parts[2],
            Part::Icon {
                glyph: Glyph::Pause,
                enabled: true,
                ..
            }
        ));
        assert!(matches!(
            &m.parts[3],
            Part::Icon {
                glyph: Glyph::Next,
                enabled: false,
                ..
            }
        ));
    }

    #[test]
    fn paused_session_shows_play_and_session_change_replaces_art() {
        let _lang = crate::test_lang::guard();
        let mut s = SessionInfo {
            key: "Spotify.exe".into(),
            title: "Song".into(),
            state: PlayState::Paused,
            controls: Controls {
                play: true,
                prev: true,
                next: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let art = ArtImage {
            id: 9,
            image: std::sync::Arc::new(transom_core::image::Image::solid(2, 2, 0)),
        };
        let v = View {
            clock: "1",
            media: Some(&s),
            art: Some(art.clone()),
            ..Default::default()
        };
        let items = build_items(&v);
        let m = items.iter().find(|i| i.id == ItemId::Media).unwrap();
        assert!(
            matches!(&m.parts[2], Part::Icon { glyph: Glyph::Play, name, .. } if *name == t("media.play"))
        );
        assert_eq!(
            m.parts[0],
            Part::Art {
                art: Some(art),
                action: Some(Action::MediaOpenSource)
            }
        );
        // ina session / skladba bez obalu -> zastupny symbol (aplikacia posle art: None)
        s.title = "Iná".into();
        let items = build_items(&View {
            clock: "1",
            media: Some(&s),
            art: None,
            ..Default::default()
        });
        let m = items.iter().find(|i| i.id == ItemId::Media).unwrap();
        assert_eq!(
            m.parts[0],
            Part::Art {
                art: None,
                action: Some(Action::MediaOpenSource)
            }
        );
        // bez session: ziadne media casti (ani obal)
        let items = build_items(&View {
            clock: "1",
            ..Default::default()
        });
        assert!(items
            .iter()
            .find(|i| i.id == ItemId::Media)
            .unwrap()
            .parts
            .is_empty());
    }

    #[test]
    fn spotify_hint_shown_even_with_other_session_and_yields_to_notice() {
        let _lang = crate::test_lang::guard();
        let notice_text = |items: &[StatusItem]| {
            items
                .iter()
                .find(|i| i.id == ItemId::Notice)
                .and_then(|i| match &i.parts[0] {
                    Part::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
        };
        let brave = SessionInfo {
            key: "Brave".into(),
            title: "Video".into(),
            state: PlayState::Paused,
            ..Default::default()
        };
        // ina session je zvolena (napr. pozastavena karta prehliadaca) -> napoveda aj tak viditelna
        let items = build_items(&View {
            clock: "1",
            media: Some(&brave),
            spotify_hint: true,
            ..Default::default()
        });
        assert!(notice_text(&items)
            .unwrap()
            .contains("Show desktop overlay"));
        let m = items.iter().find(|i| i.id == ItemId::Media).unwrap();
        assert!(!m.parts.is_empty(), "media zostavaju");
        // docasny oznam ju na chvilu nahradi
        let items = build_items(&View {
            clock: "1",
            spotify_hint: true,
            notice: Some("Search sa neotvoril"),
            ..Default::default()
        });
        assert_eq!(notice_text(&items).as_deref(), Some("Search sa neotvoril"));
    }

    #[test]
    fn battery_item_present_when_battery() {
        let _lang = crate::test_lang::guard();
        let b = Battery {
            percent: Some(42),
            charging: true,
            on_ac: true,
            saver: false,
        };
        let items = build_items(&View {
            clock: "1",
            battery: Some(b),
            ..Default::default()
        });
        assert_eq!(
            ids(&items),
            vec![
                ItemId::Search,
                ItemId::Tools,
                ItemId::ToolShelf,
                ItemId::ToolClipboard,
                ItemId::ToolNotes,
                ItemId::ToolShots,
                ItemId::ToolSnip,
                ItemId::ToolPicker,
                ItemId::Focus,
                ItemId::Battery,
                ItemId::Clock
            ]
        );
    }
}

#[cfg(test)]
mod shelf_tests {
    use super::*;
    use transom_tools::service::ShelfRow;
    use transom_tools::shelf::State;

    fn row(i: usize, state: State, remote: bool) -> ShelfRow {
        ShelfRow {
            id: format!("id{i}"),
            name: format!("subor-{i}.pdf"),
            location: r"C:\Docs".into(),
            size: 1,
            copy: false,
            dir: i == 1,
            pinned: false,
            state,
            remote,
            path: format!(r"C:\Docs\subor-{i}.pdf"),
        }
    }

    #[test]
    fn long_names_are_shortened_and_keep_a_short_extension() {
        assert_eq!(shelf_label("kratky.pdf"), "kratky.pdf");
        let l = shelf_label("Velmi dlhy nazov dokumentu 2026.pdf");
        assert_eq!(l.chars().count(), 18);
        assert!(l.ends_with("….pdf"), "{l}");
        let l = shelf_label("archiv-bez-pripony-velmi-dlhy");
        assert_eq!(l.chars().count(), 18);
        assert!(l.ends_with('…'));
        // dlha "pripona" nie je pripona
        assert!(shelf_label("nazov.dlhapripona-xxxxxxxxxx").ends_with('…'));
        // viacbajtove znaky sa nerozdelia
        assert_eq!(
            shelf_label("žltý kôň úpel ďatelina.txt").chars().count(),
            18
        );
    }

    #[test]
    fn the_bar_shows_the_first_files_then_plus_n_and_marks_unavailable_ones() {
        let rows = vec![
            row(0, State::Available, false),
            row(1, State::Available, false),
            row(2, State::Missing, false),
            row(3, State::Available, false),
            row(4, State::Available, false),
        ];
        let sv = shelf_view(&rows);
        assert_eq!(sv.files.len(), SHELF_ON_BAR);
        assert_eq!(sv.more, 2);
        assert_eq!(
            sv.files.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            vec!["id0", "id1", "id2"],
            "poradie ako v paneli"
        );
        let items = build_items(&View {
            clock: "14:05",
            shelf: Some(&sv),
            ..Default::default()
        });
        let it = items.iter().find(|i| i.id == ItemId::Shelf).unwrap();
        assert_eq!(it.side, Side::Center);
        // jedna ikona s celkovym poctom; klik otvori Nastroje na odlozenych suboroch
        assert_eq!(it.parts.len(), 1);
        match &it.parts[0] {
            Part::File {
                glyph,
                label,
                muted,
                action,
                ..
            } => {
                assert_eq!((*glyph, label.as_str(), *muted), (Glyph::Shelf, "5", false));
                assert_eq!(*action, Some(Action::ToolsShelf));
            }
            p => panic!("{p:?}"),
        }
        // sietovy a offline subor nie je "dostupny" (nekopiruje sa ani netaha)
        let sv = shelf_view(&[
            row(0, State::Unchecked, true),
            row(1, State::Offline, false),
        ]);
        assert!(sv.files.iter().all(|f| !f.available) && sv.more == 0);
        // prazdna Polica = ziadna polozka
        let empty = ShelfView::default();
        let items = build_items(&View {
            clock: "14:05",
            shelf: Some(&empty),
            ..Default::default()
        });
        // prazdne odlozene subory: ikona ostane (stred im patri), bez cisla a stlmena
        let it = items.iter().find(|i| i.id == ItemId::Shelf).unwrap();
        assert!(matches!(&it.parts[0], Part::File { label, muted: true, .. } if label.is_empty()));
        // vypnute (ziadny pohlad) = ziadna polozka
        let items = build_items(&View {
            clock: "14:05",
            ..Default::default()
        });
        assert!(!items.iter().any(|i| i.id == ItemId::Shelf));
    }

    /// Len rucne (`TRANSOM_PANEL_PNG=<adresar> cargo test -p transom-app shelf_bar_png -- --ignored`): lista so subormi
    /// Police v strede (sama a vedla hodin v strede) v oboch temach a pri 100 / 150 %.
    #[test]
    #[ignore]
    fn shelf_bar_png() {
        use transom_render::offscreen::Surface;
        use transom_render::{GdiPainter, Size, Theme};
        let Some(dir) = std::env::var_os("TRANSOM_PANEL_PNG") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let mut rows: Vec<ShelfRow> = (0..5).map(|i| row(i, State::Available, false)).collect();
        rows[0].name = "Faktura-2026-november-final.pdf".into();
        rows[2].state = State::Missing;
        let sv = shelf_view(&rows);
        let mut clock_center = transom_config::Layout::default();
        clock_center.move_to("clock", transom_config::Group::Center, 0);
        for (name, layout) in [
            ("alone", transom_config::Layout::default()),
            ("clock", clock_center),
        ] {
            for (tag, th) in [("dark", Theme::dark()), ("light", Theme::light())] {
                for dpi in [96u32, 144] {
                    let sc = dpi as f32 / 96.0;
                    let (w, h) = ((1600.0 * sc) as i32, (28.0 * sc) as i32);
                    let surf = Surface::new(w, h);
                    let items = arrange(
                        build_items(&View {
                            clock: "14:05",
                            shelf: Some(&sv),
                            ..Default::default()
                        }),
                        &layout,
                    );
                    let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                    transom_render::render_bar_with(
                        &mut gp,
                        &th,
                        &items,
                        transom_render::Interaction::default(),
                        None,
                    );
                    let _ = std::fs::write(
                        dir.join(format!("shelf-{name}-{tag}-{dpi}.png")),
                        transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
                    );
                }
            }
        }
    }

    #[test]
    fn the_default_layout_puts_the_shelf_in_the_center() {
        let sv = shelf_view(&[row(0, State::Available, false)]);
        let items = arrange(
            build_items(&View {
                clock: "14:05",
                shelf: Some(&sv),
                ..Default::default()
            }),
            &transom_config::Layout::default(),
        );
        let it = items.iter().find(|i| i.id == ItemId::Shelf).unwrap();
        assert_eq!(it.side, Side::Center);
    }
}
