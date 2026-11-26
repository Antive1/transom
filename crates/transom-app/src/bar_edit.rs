//! Uprava lisaty priamo na liste (prave tlacidlo > "Upravit listu"): polozky sa tahaju mysou po liste (aj medzi lavou,
//! strednou a pravou skupinou) a pod listou je panel so zoznamom poloziek (zobrazit / skryt), oddelovacmi (pridat /
//! odstranit) a tlacidlom Hotovo. Kym je uprava zapnuta, klik na polozku ju neotvara.
//!
//! Cista logika (ciel pustenia, riadky panela, prikazy) bez Win32; zmeny sa ukladaju tou istou cestou ako ostatne
//! zmeny konfiguracie z listy (`save_config_with`: zamok, len zmenene pole, oznamenie nastaveniam).
use crate::pop_kit::{Btn, Row};
use transom_config::{Group, Layout};
use transom_core::item::{ItemId, Side, StatusItem};
use transom_core::layout::Placed;
use transom_i18n::t;

/// Rozpracovane tahanie polozky po liste.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub monitor: String,
    pub id: ItemId,
    pub start_x: i32,
    pub x: i32,
    /// Mys sa pohla dost na to, aby to bolo tahanie (nie len klik).
    pub moved: bool,
}

/// Stav upravy lisaty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarEdit {
    pub drag: Option<Drag>,
    /// Polozka pod mysou (zvyraznenie).
    pub hover: Option<(String, ItemId)>,
}

/// Id polozky v konfiguracii (opak `view::id_of`); docasny oznam nema miesto v rozlozeni.
pub fn config_id(id: ItemId) -> Option<String> {
    Some(
        match id {
            ItemId::Search => "search",
            ItemId::Media => "media",
            ItemId::Ai => "ai",
            ItemId::Audio => "audio",
            ItemId::Tools => "tools",
            ItemId::Battery => "battery",
            ItemId::Peripherals => "peripherals",
            ItemId::Shelf => "shelf",
            ItemId::Keyboard => "keyboard",
            ItemId::Timer => "timer",
            ItemId::Brightness => "brightness",
            ItemId::SysMon => "sysmon",
            ItemId::Privacy => "privacy",
            ItemId::Clock => "clock",
            ItemId::ToolShelf => "tool_shelf",
            ItemId::ToolClipboard => "tool_clipboard",
            ItemId::ToolNotes => "tool_notes",
            ItemId::ToolShots => "tool_shots",
            ItemId::ToolSnip => "tool_snip",
            ItemId::ToolPicker => "tool_picker",
            ItemId::Window => "window",
            ItemId::Desktops => "desktops",
            ItemId::Focus => "focus",
            ItemId::Weather => "weather",
            ItemId::Separator(n) => return Some(format!("sep-{n}")),
            ItemId::Notice | ItemId::Update => return None,
        }
        .to_string(),
    )
}

/// Usek polozky na liste (od zaciatku prvej po koniec poslednej casti), v poradi zlava doprava.
pub fn spans(placed: &[Placed]) -> Vec<(ItemId, i32, i32)> {
    let mut out: Vec<(ItemId, i32, i32)> = Vec::new();
    for p in placed {
        match out.last_mut() {
            Some((id, _, x1)) if *id == p.item => *x1 = (*x1).max(p.x + p.w),
            _ => out.push((p.item, p.x, p.x + p.w)),
        }
    }
    out
}

fn group_of(side: Side) -> Group {
    match side {
        Side::Left => Group::Left,
        Side::Center => Group::Center,
        Side::Right => Group::Right,
    }
}

/// Kam sa polozka `dragged` pusti pri x: skupina podla tretiny lisaty (predvidatelne aj pre prazdnu skupinu) a polozka
/// tej skupiny, pred ktoru sa vlozi (prva so stredom vpravo od x; `None` = na koniec skupiny). Vrati aj x ciarky
/// vlozenia na liste.
///
/// `center_locked` (stred patri odlozenym suborom): stredna tretina sa rozdeli na polovice a polozka ide do lavej /
/// pravej skupiny; odlozene subory samy sa nikam nepresuvaju (`None`).
pub fn drop_target(
    placed: &[Placed],
    items: &[StatusItem],
    width: i32,
    dragged: ItemId,
    x: i32,
    center_locked: bool,
) -> Option<(Group, Option<ItemId>, i32)> {
    if center_locked && dragged == ItemId::Shelf {
        return None;
    }
    let g = if x < width / 3 || (center_locked && x < width / 2) {
        Group::Left
    } else if x < width * 2 / 3 && !center_locked {
        Group::Center
    } else {
        Group::Right
    };
    let side_of = |id: ItemId| items.iter().find(|i| i.id == id).map(|i| group_of(i.side));
    let in_group: Vec<(ItemId, i32, i32)> = spans(placed)
        .into_iter()
        .filter(|(id, _, _)| *id != dragged && side_of(*id) == Some(g))
        .collect();
    Some(
        match in_group.iter().find(|(_, x0, x1)| (x0 + x1) / 2 > x) {
            Some((id, x0, _)) => (g, Some(*id), *x0),
            None => {
                let line = in_group.last().map_or(
                    match g {
                        Group::Left => 0,
                        Group::Center => width / 2,
                        Group::Right => width,
                    },
                    |(_, _, x1)| *x1,
                );
                (g, None, line)
            }
        },
    )
}

// Id ovladacov panela upravy.
pub const ED_DONE: u32 = 1;
pub const ED_RESET: u32 = 2;
const ED_TOGGLE: u32 = 1000;
const ED_REMOVE: u32 = 2000;
const ED_ADD: u32 = 3000;

/// Prikaz panela upravy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditCmd {
    Toggle(String),
    RemoveSep(String),
    AddSep(Group),
    Reset,
    Done,
}

const GROUPS: [(Group, &str); 3] = [
    (Group::Left, "group.left"),
    (Group::Center, "group.center"),
    (Group::Right, "group.right"),
];

/// Nazov polozky rozlozenia (rovnake texty ako stranka Rozlozenie v nastaveniach).
pub fn label(id: &str) -> String {
    if transom_config::separator_no(id).is_some() {
        t("item.separator")
    } else {
        t(&format!("item.{id}"))
    }
}

/// Riadky panela upravy podla rozlozenia.
pub fn rows(layout: &Layout, center_locked: bool) -> Vec<Row> {
    let mut out = vec![
        Row::Title(t("edit.title")),
        Row::Text {
            text: t("edit.help"),
            strong: false,
        },
    ];
    let full = layout
        .items
        .iter()
        .filter(|s| transom_config::separator_no(&s.id).is_some())
        .count()
        >= transom_config::MAX_SEPARATORS as usize;
    for (gi, (g, key)) in GROUPS.iter().enumerate() {
        out.push(Row::Section(t(key)));
        if center_locked && *g == Group::Center {
            // stred patri odlozenym suborom (zapinaju sa v nastaveniach nastrojov)
            out.push(Row::Text {
                text: t("layout.center_locked"),
                strong: false,
            });
            continue;
        }
        for (i, s) in layout.items.iter().enumerate() {
            if s.group != *g || !transom_config::is_layout_id(&s.id) {
                continue;
            }
            if let Some(n) = transom_config::separator_no(&s.id) {
                out.push(Row::Buttons(vec![Btn::new(
                    ED_REMOVE + n as u32,
                    format!("{} – {}", t("item.separator"), t("layout.sep_remove")),
                )]));
            } else {
                out.push(Row::Toggle {
                    id: ED_TOGGLE + i as u32,
                    label: label(&s.id),
                    on: s.visible,
                });
            }
        }
        out.push(Row::Buttons(vec![Btn::new(
            ED_ADD + gi as u32,
            format!("+ {}", t("layout.sep_add")),
        )
        .enabled(!full)
        .acc(format!("{}: {}", t(key), t("layout.sep_add")))]));
    }
    out.push(Row::Gap(8));
    out.push(Row::Buttons(vec![
        Btn::new(ED_DONE, t("edit.done")).primary(),
        Btn::new(ED_RESET, t("layout.reset")),
    ]));
    out
}

/// Prikaz pre klik na ovladac `id` (podla rozlozenia, z ktoreho sa kreslilo).
pub fn decode(layout: &Layout, id: u32) -> Option<EditCmd> {
    Some(match id {
        ED_DONE => EditCmd::Done,
        ED_RESET => EditCmd::Reset,
        i if (ED_TOGGLE..ED_REMOVE).contains(&i) => {
            EditCmd::Toggle(layout.items.get((i - ED_TOGGLE) as usize)?.id.clone())
        }
        i if (ED_REMOVE..ED_ADD).contains(&i) => {
            EditCmd::RemoveSep(format!("sep-{}", i - ED_REMOVE))
        }
        i if (ED_ADD..ED_ADD + 3).contains(&i) => EditCmd::AddSep(GROUPS[(i - ED_ADD) as usize].0),
        _ => return None,
    })
}

/// Vykona prikaz na rozlozeni (`Done` nic nemeni).
pub fn apply(layout: &mut Layout, cmd: &EditCmd) {
    match cmd {
        EditCmd::Toggle(id) => {
            let on = layout.items.iter().any(|s| &s.id == id && s.visible);
            layout.set_visible(id, !on);
        }
        EditCmd::RemoveSep(id) => {
            layout.remove_separator(id);
        }
        EditCmd::AddSep(g) => {
            layout.add_separator(*g);
        }
        EditCmd::Reset => *layout = Layout::default(),
        EditCmd::Done => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::item::Part;

    fn item(id: ItemId, side: Side) -> StatusItem {
        StatusItem::new(id, side, vec![Part::text("x")])
    }

    fn pl(item: ItemId, x: i32, w: i32) -> Placed {
        Placed {
            item,
            part: 0,
            x,
            w,
        }
    }

    #[test]
    fn every_layout_id_maps_back_and_forth() {
        for id in transom_config::KNOWN_ITEMS {
            let item = crate::view::id_of(id).expect(id);
            assert_eq!(config_id(item).as_deref(), Some(id));
        }
        for n in 1..=transom_config::MAX_SEPARATORS {
            let s = format!("sep-{n}");
            assert_eq!(config_id(crate::view::id_of(&s).unwrap()), Some(s));
        }
        assert_eq!(config_id(ItemId::Notice), None);
    }

    #[test]
    fn the_drop_target_follows_the_thirds_of_the_bar_and_the_item_centres() {
        let items = vec![
            item(ItemId::Search, Side::Left),
            item(ItemId::Media, Side::Left),
            item(ItemId::Shelf, Side::Center),
            item(ItemId::Battery, Side::Right),
            item(ItemId::Clock, Side::Right),
        ];
        let placed = vec![
            pl(ItemId::Search, 0, 30),
            pl(ItemId::Media, 30, 100),
            pl(ItemId::Shelf, 480, 40),
            pl(ItemId::Battery, 900, 40),
            pl(ItemId::Clock, 940, 60),
        ];
        let w = 1000;
        // vlavo pred Media (stred 80)
        assert_eq!(
            drop_target(&placed, &items, w, ItemId::Clock, 50, false),
            Some((Group::Left, Some(ItemId::Media), 30))
        );
        // vlavo za poslednou polozkou
        assert_eq!(
            drop_target(&placed, &items, w, ItemId::Clock, 300, false),
            Some((Group::Left, None, 130))
        );
        // stred: pred Policou
        assert_eq!(
            drop_target(&placed, &items, w, ItemId::Search, 400, false),
            Some((Group::Center, Some(ItemId::Shelf), 480))
        );
        // vpravo: tahana polozka sa sama nepocita
        assert_eq!(
            drop_target(&placed, &items, w, ItemId::Battery, 950, false),
            Some((Group::Right, Some(ItemId::Clock), 940))
        );
        // zamknuty stred: stredna tretina patri susednym skupinam, odlozene subory sa nepresuvaju
        assert_eq!(
            drop_target(&placed, &items, w, ItemId::Search, 400, true).map(|t| t.0),
            Some(Group::Left)
        );
        assert_eq!(
            drop_target(&placed, &items, w, ItemId::Search, 600, true).map(|t| t.0),
            Some(Group::Right)
        );
        assert_eq!(
            drop_target(&placed, &items, w, ItemId::Shelf, 100, true),
            None
        );
        // prazdna skupina: ciarka na jej kraji
        let only_right = vec![item(ItemId::Clock, Side::Right)];
        assert_eq!(
            drop_target(
                &[pl(ItemId::Clock, 940, 60)],
                &only_right,
                w,
                ItemId::Clock,
                10,
                false
            ),
            Some((Group::Left, None, 0))
        );
    }

    #[test]
    fn panel_rows_toggle_items_add_and_remove_separators_and_finish() {
        let _g = crate::test_lang::guard();
        let mut l = Layout::default();
        // kazda polozka rozlozenia ma riadok; klik na nu prepne viditelnost
        let toggles: Vec<u32> = rows(&l, false)
            .iter()
            .filter_map(|r| match r {
                Row::Toggle { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(toggles.len(), transom_config::KNOWN_ITEMS.len());
        let clock = l.items.iter().position(|s| s.id == "clock").unwrap() as u32;
        let cmd = decode(&l, ED_TOGGLE + clock).unwrap();
        assert_eq!(cmd, EditCmd::Toggle("clock".into()));
        apply(&mut l, &cmd);
        assert!(!l.items.iter().any(|s| s.id == "clock" && s.visible));
        // oddelovac v strede, potom jeho odstranenie
        let c = decode(&l, ED_ADD + 1).unwrap();
        apply(&mut l, &c);
        assert!(l.in_group(Group::Center).any(|s| s.id == "sep-1"));
        let has_remove = rows(&l, false)
            .iter()
            .any(|r| matches!(r, Row::Buttons(b) if b.iter().any(|b| b.id == ED_REMOVE + 1)));
        assert!(has_remove, "oddelovac ma tlacidlo Odstranit");
        let c = decode(&l, ED_REMOVE + 1).unwrap();
        apply(&mut l, &c);
        assert!(!l.items.iter().any(|s| s.id == "sep-1"));
        // obnovit predvolene a hotovo
        let c = decode(&l, ED_RESET).unwrap();
        apply(&mut l, &c);
        assert_eq!(l, Layout::default());
        assert_eq!(decode(&l, ED_DONE), Some(EditCmd::Done));
        assert_eq!(decode(&l, 99_999), None);
    }

    /// Rucna kontrola panela upravy (sklo nie, len rozlozenie): `TRANSOM_GALLERY=<adresar> cargo test -p transom-app
    /// gallery_edit -- --ignored`.
    #[test]
    #[ignore]
    fn gallery_edit_panel() {
        let _g = crate::test_lang::guard();
        let Some(dir) = crate::gallery::dir() else {
            return;
        };
        let mut l = Layout::default();
        l.add_separator(Group::Right);
        for (dark, th) in crate::gallery::themes() {
            let (w, h) = (crate::gallery::px(crate::extras::KIT_W_LP, 96), 900);
            let surf = transom_render::offscreen::Surface::new(w, h);
            let mut p = transom_render::GdiPainter::new(surf.dc, transom_render::Size { w, h }, 96);
            let mut kit = crate::pop_kit::Kit::default();
            kit.paint(&mut p, &th, &rows(&l, false));
            crate::gallery::save(
                &dir,
                &format!("edit-panel-{}", crate::gallery::tag(dark)),
                &surf,
            );
        }
    }
}
