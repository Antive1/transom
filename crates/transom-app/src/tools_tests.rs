//! Panel Nastroje: sekcie, prazdne stavy so sukromim, vyber, dvojkrokove mazanie, klavesnica, mys (aj MSAA invoke),
//! pristupnost a audit vzhladu (offscreen, bez okna; data su vyrobene rucne).
use crate::tools_pop::{
    Row, Section, SectionView, ToolsCmd, ToolsData, ToolsHit, ToolsPanel, TOOLS_H_LP, TOOLS_W_LP,
};
use transom_platform::PointerKind;
use transom_render::audit::Audit;
use transom_render::offscreen::Surface;
use transom_render::{GdiPainter, Size, Theme};

fn row(title: &str, sub: &str, value: &str) -> Row {
    Row {
        title: title.into(),
        subtitle: sub.into(),
        value: value.into(),
        ..Row::default()
    }
}

fn data() -> ToolsData {
    let mut d = ToolsData::default();
    d.sections[0] = SectionView {
        enabled: true,
        rows: vec![
            row("report.pdf", "C:\\docs", "1,2 MB"),
            Row {
                pinned: true,
                ..row("photo.jpg", "stored copy", "3 MB")
            },
            Row {
                unavailable: true,
                ..row("gone.txt", "D:\\old", "")
            },
        ],
        status: None,
        empty: None,
    };
    d.sections[1] = SectionView {
        enabled: false,
        rows: vec![],
        status: None,
        empty: None,
    };
    d
}

fn paint(p: &mut ToolsPanel, d: &ToolsData, dpi: u32, h_mul: f32, dark: bool) -> Vec<String> {
    let sc = dpi as f32 / 96.0;
    let (w, h) = (
        (TOOLS_W_LP as f32 * sc).round() as i32,
        (TOOLS_H_LP as f32 * h_mul * sc).round() as i32,
    );
    let th = if dark { Theme::dark() } else { Theme::light() };
    let surf = Surface::new(w, h);
    let mut out = Vec::new();
    for _ in 0..2 {
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
        let mut au = Audit::new(&mut gp, th.background);
        p.paint(&mut au, &th, d);
        au.finish();
        out = au.issues.iter().map(|i| format!("{i:?}")).collect();
    }
    out
}

#[test]
fn rows_show_pinned_and_unavailable_state_in_the_accessible_name_and_the_list_is_read_only_selectable(
) {
    use transom_platform::menu_access::{ROLE_LISTITEM, ROLE_PAGETAB, STATE_SELECTED};
    let _g = crate::test_lang::guard();
    let mut p = ToolsPanel::default();
    let d = data();
    let issues = paint(&mut p, &d, 96, 2.0, true);
    assert!(issues.is_empty(), "{issues:?}");
    let (entries, extras, _, name) = p.a11y();
    assert_eq!(entries.len(), extras.len());
    assert_eq!(name, "Tools");
    let find = |s: &str| {
        entries
            .iter()
            .position(|e| e.name.starts_with(s))
            .unwrap_or_else(|| panic!("{s}"))
    };
    assert_eq!(extras[find("Stashed files")].role, ROLE_PAGETAB);
    assert!(
        extras[find("Stashed files")].state & STATE_SELECTED != 0,
        "aktivna karta je vybrana"
    );
    assert!(extras[find("Clipboard")].state & STATE_SELECTED == 0);
    let photo = find("photo.jpg");
    assert!(
        entries[photo].name.contains("pinned"),
        "{}",
        entries[photo].name
    );
    let gone = find("gone.txt");
    assert!(entries[gone].name.contains("unavailable"));
    assert_eq!(extras[photo].role, ROLE_LISTITEM);
    assert_eq!(extras[find("report.pdf")].value, "1,2 MB");
    // karty su pre screen reader zoradene pred zoznamom a staticke texty nie su fokusovatelne
    assert!(entries.iter().any(|e| e.name == "Tools"));
}

#[test]
fn empty_sections_explain_what_is_stored_and_a_disabled_clipboard_says_nothing_is_saved() {
    let _g = crate::test_lang::guard();
    let d = data();
    let mut p = ToolsPanel::default();
    p.section = Section::Clipboard;
    paint(&mut p, &d, 96, 2.0, true);
    let (entries, _, _, _) = p.a11y();
    assert!(
        entries
            .iter()
            .any(|e| e.name == transom_i18n::t("tools.empty_clip_off")),
        "vypnuta schranka vysvetli, ze sa nic neuklada: {:?}",
        entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>()
    );
    assert!(transom_i18n::t("tools.empty_clip_off").contains("nothing is saved"));
    // zapnuta, ale prazdna
    let mut d2 = data();
    d2.sections[1].enabled = true;
    paint(&mut p, &d2, 96, 2.0, true);
    assert!(p
        .a11y()
        .0
        .iter()
        .any(|e| e.name == transom_i18n::t("tools.empty_clip")));
    // ostatne sekcie
    for (sec, key) in [
        (Section::Notes, "tools.empty_notes"),
        (Section::Shots, "tools.lib_empty"),
        (Section::Shelf, "tools.empty_shelf"),
    ] {
        let mut p = ToolsPanel::default();
        p.section = sec;
        paint(&mut p, &ToolsData::default(), 96, 2.0, true);
        assert!(
            p.a11y().0.iter().any(|e| e.name == transom_i18n::t(key)),
            "{sec:?}"
        );
    }
}

#[test]
fn keyboard_ring_tabs_arrows_enter_without_repeat_pin_and_two_step_delete() {
    let _g = crate::test_lang::guard();
    let d = data();
    let mut p = ToolsPanel::default();
    paint(&mut p, &d, 96, 2.0, true);
    // Tab: hladat, 4 karty, nastroje Police (pridat, hore, dole), potom riadky
    let mut order = Vec::new();
    for _ in 0..16 {
        p.key(0x09, false, false, &d);
        let f = p.focus.unwrap();
        if order.contains(&f) {
            break;
        }
        order.push(f);
    }
    assert_eq!(
        order,
        vec![
            ToolsHit::Search,
            ToolsHit::Tab(Section::Shelf),
            ToolsHit::Tab(Section::Clipboard),
            ToolsHit::Tab(Section::Notes),
            ToolsHit::Tab(Section::Shots),
            ToolsHit::ShelfAdd,
            ToolsHit::ShelfUp,
            ToolsHit::ShelfDown,
            ToolsHit::Row(0),
            ToolsHit::Row(1),
            ToolsHit::Row(2),
        ]
    );
    // sipky na karte prepinaju sekciu (cyklicky); zmena sekcie nuluje vyber
    p.focus = Some(ToolsHit::Tab(Section::Shelf));
    p.key(0x27, false, false, &d);
    assert_eq!(p.section, Section::Clipboard);
    p.key(0x25, false, false, &d);
    p.key(0x25, false, false, &d);
    assert_eq!(p.section, Section::Shots, "zo Police dolava = posledna");
    p.switch_for_test(Section::Shelf);
    paint(&mut p, &d, 96, 2.0, true);
    // vyber riadku a Enter bez opakovania
    p.key(0x28, false, false, &d);
    assert_eq!(p.selected, 1);
    assert_eq!(
        p.key(0x0D, false, false, &d).0,
        Some(ToolsCmd::Activate(Section::Shelf, 1))
    );
    assert_eq!(
        p.key(0x0D, false, true, &d).0,
        None,
        "drzany Enter sa ignoruje"
    );
    assert_eq!(
        p.key(0x50, false, false, &d).0,
        Some(ToolsCmd::TogglePin(Section::Shelf, 1))
    );
    // Delete je dvojkrokovy: prvy len pyta, druhy maze, ina klavesa potvrdenie zrusi
    assert_eq!(p.key(0x2E, false, false, &d).0, None);
    paint(&mut p, &d, 96, 2.0, true);
    assert!(p
        .a11y()
        .0
        .iter()
        .any(|e| e.name.contains("photo.jpg") && e.name.starts_with("Press Delete again")));
    assert_eq!(
        p.key(0x2E, false, false, &d).0,
        Some(ToolsCmd::Delete(Section::Shelf, 1))
    );
    p.key(0x2E, false, false, &d);
    p.key(0x28, false, false, &d);
    assert_eq!(
        p.key(0x2E, false, false, &d).0,
        None,
        "ina klavesa zrusila potvrdenie"
    );
    // vyber sa nikdy nedostane mimo zoznam
    for _ in 0..10 {
        p.key(0x28, false, false, &d);
    }
    assert_eq!(p.selected, 2);
    for _ in 0..10 {
        p.key(0x26, false, false, &d);
    }
    assert_eq!(p.selected, 0);
}

#[test]
fn mouse_selects_a_row_and_msaa_invoke_without_a_press_activates_the_control_under_the_point() {
    let _g = crate::test_lang::guard();
    let d = data();
    let mut p = ToolsPanel::default();
    paint(&mut p, &d, 96, 2.0, true);
    let (entries, _, _, _) = p.a11y();
    let at = |name: &str| {
        let e = entries
            .iter()
            .find(|e| e.name.starts_with(name))
            .unwrap_or_else(|| panic!("{name}"));
        (e.x + e.w / 2, e.y + e.h / 2)
    };
    // myska: stlacenie + uvolnenie na tom istom riadku
    let (x, y) = at("photo.jpg");
    p.pointer(PointerKind::Down, x, y);
    assert_eq!(p.selected, 1);
    assert_eq!(
        p.pointer(PointerKind::Click, x, y).0,
        Some(ToolsCmd::Activate(Section::Shelf, 1))
    );
    // stlacenie na jednom a uvolnenie nad inym nic nevykona
    let (x2, y2) = at("report.pdf");
    p.pointer(PointerKind::Down, x, y);
    assert_eq!(p.pointer(PointerKind::Click, x2, y2).0, None);
    // MSAA: len klik (bez stlacenia) aktivuje: tlacidlo Hladat a karta Schranka
    let (sx, sy) = at("Search");
    assert_eq!(
        p.pointer(PointerKind::Click, sx, sy).0,
        Some(ToolsCmd::Search)
    );
    let (cx, cy) = at("Clipboard");
    assert_eq!(p.pointer(PointerKind::Click, cx, cy).0, None);
    assert_eq!(p.section, Section::Clipboard, "karta sa prepla");
    // opustenie okna zrusi rozpracovane stlacenie
    p.pointer(PointerKind::Down, sx, sy);
    p.pointer(PointerKind::Leave, -1, -1);
    assert_eq!(
        p.pointer(PointerKind::Click, sx, sy).0,
        Some(ToolsCmd::Search),
        "bez stlacenia = invoke"
    );
}

#[test]
fn the_audit_is_clean_in_every_language_theme_dpi_and_state() {
    let _g = crate::test_lang::guard();
    let long = "Velmi dlhy nazov suboru s priponou (kopia) - pre kontrolu orezania v zozname nastrojov.final.v2.docx";
    let many = SectionView {
        enabled: true,
        rows: (0..30)
            .map(|i| Row {
                pinned: i % 3 == 0,
                unavailable: i % 5 == 0,
                ..row(
                    &format!("{i} {long}"),
                    &format!("C:\\\\Users\\\\x\\\\{long}"),
                    "128,5 MB",
                )
            })
            .collect(),
        status: Some(transom_i18n::t("tools.missing")),
        empty: None,
    };
    let mut bad = Vec::new();
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for dpi in [96u32, 144, 192] {
            for dark in [true, false] {
                for (name, d, sec) in [
                    ("normal", data(), Section::Shelf),
                    ("clip-off", data(), Section::Clipboard),
                    ("notes-empty", ToolsData::default(), Section::Notes),
                    ("shots-empty", ToolsData::default(), Section::Shots),
                    (
                        "many-long",
                        {
                            let mut x = ToolsData::default();
                            x.sections[0] = many.clone();
                            x
                        },
                        Section::Shelf,
                    ),
                ] {
                    let mut p = ToolsPanel::default();
                    p.section = sec;
                    p.msg = Some(transom_i18n::t("tools.missing"));
                    for i in paint(&mut p, &d, dpi, 3.0, dark) {
                        bad.push(format!("{} {dpi} {name} {dark}: {i}", lang.code()));
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
        bad.iter().take(12).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_bar_item_is_called_tools_and_the_windows_tray_button_is_gone_from_the_layout() {
    let _g = crate::test_lang::guard();
    let items = crate::view::build_items(&crate::view::View {
        clock: "10:00",
        ..Default::default()
    });
    assert!(items
        .iter()
        .any(|i| i.id == transom_core::item::ItemId::Tools));
    let tools = items
        .iter()
        .find(|i| i.id == transom_core::item::ItemId::Tools)
        .unwrap();
    assert_eq!(
        tools.parts[0].action(),
        Some(transom_core::item::Action::ToolsPanel)
    );
    // ziadna cast lisaty uz neposiela Win+B (Action::Tray ostava len rychlej akcii v paneli Zvuk)
    for i in &items {
        for p in &i.parts {
            assert_ne!(
                p.action(),
                Some(transom_core::item::Action::Tray),
                "{:?}",
                i.id
            );
        }
    }
    assert!(!transom_config::KNOWN_ITEMS.contains(&"tray"));
    assert!(transom_config::KNOWN_ITEMS.contains(&"tools"));
}

#[test]
fn the_note_editor_edits_at_the_end_respects_the_size_limit_and_never_accepts_control_characters() {
    let mut p = ToolsPanel::default();
    assert!(!p.editor_char('a'), "bez editora sa nic nepise");
    p.open_editor(None, String::new());
    for c in "Ahoj".chars() {
        assert!(p.editor_char(c));
    }
    assert!(!p.editor_char('\u{7}'));
    assert_eq!(
        p.editor_key(0x0D, false),
        (None, true),
        "Enter = novy riadok"
    );
    p.editor_char('x');
    assert_eq!(p.editor.as_ref().unwrap().text, "Ahoj\nx");
    assert_eq!(p.editor_key(0x08, false), (None, true));
    assert_eq!(p.editor.as_ref().unwrap().text, "Ahoj\n");
    p.editor_paste("jedna\r\ndva\u{1}\u{202E}x");
    assert_eq!(
        p.editor.as_ref().unwrap().text,
        "Ahoj\njedna\ndva\u{202E}x".replace('\u{202E}', "")
    );
    p.editor_key(0x08, true);
    assert!(
        p.editor.as_ref().unwrap().text.ends_with("dva")
            || p.editor.as_ref().unwrap().text.ends_with('\n')
    );
    assert_eq!(
        p.editor_key(0x0D, true).0,
        Some(ToolsCmd::EditorSave),
        "Ctrl+Enter uklada"
    );
    assert!(p.editor.as_ref().unwrap().dirty());
    // limit velkosti
    let mut q = ToolsPanel::default();
    q.open_editor(None, "x".repeat(transom_tools::notes::MAX_NOTE_BYTES));
    assert!(!q.editor_char('y'));
    assert!(!q.editor_paste("more"));
    assert_eq!(q.editor_key(0x0D, false), (None, true));
    assert_eq!(
        q.editor.as_ref().unwrap().text.len(),
        transom_tools::notes::MAX_NOTE_BYTES
    );
    // otvorena a nezmenena poznamka nie je "dirty"
    let mut r = ToolsPanel::default();
    r.open_editor(Some("id".into()), "text".into());
    assert!(!r.editor.as_ref().unwrap().dirty());
}

#[test]
fn the_note_editor_and_the_new_note_button_pass_the_audit_in_every_language_dpi_and_theme() {
    let _g = crate::test_lang::guard();
    let mut bad = Vec::new();
    let long = "Velmi dlhy riadok poznamky s mnohymi slovami ".repeat(30);
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for dpi in [96u32, 144, 192] {
            for dark in [true, false] {
                for (name, text) in [
                    ("empty", String::new()),
                    ("short", "Nakup\nmlieko".to_string()),
                    ("long", long.clone()),
                ] {
                    let mut p = ToolsPanel::default();
                    p.open_editor(None, text);
                    for i in paint(&mut p, &data(), dpi, 1.0, dark) {
                        bad.push(format!("{} {dpi} editor-{name} {dark}: {i}", lang.code()));
                    }
                }
                let mut p = ToolsPanel::default();
                p.section = Section::Notes;
                let mut d = data();
                d.sections[2].enabled = true;
                for i in paint(&mut p, &d, dpi, 1.0, dark) {
                    bad.push(format!("{} {dpi} notes-new {dark}: {i}", lang.code()));
                }
            }
        }
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
    assert!(
        bad.is_empty(),
        "{} nalezov, napr.:\n{}",
        bad.len(),
        bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_editor_exposes_an_edit_field_and_three_named_buttons_and_the_new_note_button_is_clickable() {
    use transom_platform::menu_access::{ROLE_PUSHBUTTON, ROLE_TEXT};
    let _g = crate::test_lang::guard();
    let mut p = ToolsPanel::default();
    p.open_editor(Some("n1".into()), "hello".into());
    paint(&mut p, &data(), 96, 1.0, true);
    let (entries, extras, _, _) = p.a11y();
    let texts: Vec<_> = extras
        .iter()
        .enumerate()
        .filter(|(_, e)| e.role == ROLE_TEXT)
        .collect();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].1.value, "hello");
    for name in ["Save", "Copy text", "Discard changes"] {
        let i = entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(extras[i].role, ROLE_PUSHBUTTON);
    }
    let save = entries.iter().find(|e| e.name == "Save").unwrap();
    assert_eq!(
        p.pointer(PointerKind::Click, save.x + 3, save.y + 3).0,
        Some(ToolsCmd::EditorSave)
    );
    // tlacidlo "Nova poznamka" v sekcii Poznamky
    let mut q = ToolsPanel::default();
    q.section = Section::Notes;
    let mut d = data();
    d.sections[2].enabled = true;
    paint(&mut q, &d, 96, 1.0, true);
    let (entries, _, _, _) = q.a11y();
    let nn = entries
        .iter()
        .find(|e| e.name == "New note")
        .expect("tlacidlo");
    assert_eq!(
        q.pointer(PointerKind::Click, nn.x + 3, nn.y + 3).0,
        Some(ToolsCmd::NoteNew)
    );
}

fn shot_rows(n: usize) -> Vec<Row> {
    use crate::tools_pop::Thumb;
    use transom_core::loader::Bitmap;
    (0..n)
        .map(|i| Row {
            id: format!("C:\\Shots\\s{i}.png"),
            thumb: match i % 3 {
                0 => Thumb::Loading,
                1 => Thumb::Ready(std::sync::Arc::new(
                    Bitmap::new(74, 37, vec![[30, 120, 200, 255]; 74 * 37]).unwrap(),
                )),
                _ => Thumb::Placeholder,
            },
            ..row(&format!("Screenshot {i}.png"), "2026-11-22 10:15", "1,2 MB")
        })
        .collect()
}

#[test]
fn screenshots_are_a_read_only_library_with_open_folder_and_refresh_actions_and_nothing_destructive(
) {
    use transom_platform::menu_access::ROLE_PUSHBUTTON;
    let _g = crate::test_lang::guard();
    let mut p = ToolsPanel::default();
    p.section = Section::Shots;
    let mut d = data();
    d.sections[3] = SectionView {
        enabled: true,
        rows: shot_rows(4),
        status: Some(transom_i18n::t("tools.lib_readonly")),
        empty: None,
    };
    paint(&mut p, &d, 96, 2.0, true);
    let (entries, extras, _, _) = p.a11y();
    // tri vyslovne tlacidla; ziadne snimanie, ukladanie ani mazanie
    for name in ["Open", "Show in folder", "Refresh"] {
        let i = entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(extras[i].role, ROLE_PUSHBUTTON);
    }
    for gone in [
        "Capture window or screen…",
        "Save image from clipboard",
        "Save screenshot",
        "Discard",
    ] {
        assert!(entries.iter().all(|e| e.name != gone), "{gone}");
    }
    let click = |p: &mut ToolsPanel, name: &str| {
        let e = entries.iter().find(|e| e.name == name).unwrap().clone();
        p.pointer(PointerKind::Click, e.x + 3, e.y + 3).0
    };
    assert_eq!(
        click(&mut p, "Open"),
        Some(ToolsCmd::Activate(Section::Shots, 0))
    );
    assert_eq!(
        click(&mut p, "Show in folder"),
        Some(ToolsCmd::ShowInFolder(Section::Shots, 0))
    );
    assert_eq!(click(&mut p, "Refresh"), Some(ToolsCmd::Refresh));
    // klavesnica: Enter otvori, Ctrl+Enter ukaze v priecinku, F5 obnovi; F2, Delete a P (premenovanie, mazanie, pripnutie) nic
    assert_eq!(
        p.key(0x0D, false, false, &d).0,
        Some(ToolsCmd::Activate(Section::Shots, 0))
    );
    assert_eq!(
        p.key_ctrl(0x0D, false, true, false, &d).0,
        Some(ToolsCmd::ShowInFolder(Section::Shots, 0))
    );
    assert_eq!(p.key(0x74, false, false, &d).0, Some(ToolsCmd::Refresh));
    for vk in [0x71u32, 0x2E, 0x50] {
        assert_eq!(p.key(vk, false, false, &d).0, None, "vk={vk:#x}");
    }
    assert_eq!(
        p.key(0x2E, false, false, &d).0,
        None,
        "ani druhe Delete nic nezmaze"
    );
    // F5 a Ctrl+Enter plati len v sekcii Snimky
    let mut other = ToolsPanel::default();
    assert_eq!(other.key(0x74, false, false, &data()).0, None);
    assert_eq!(
        other.key_ctrl(0x0D, false, true, false, &data()).0,
        Some(ToolsCmd::Activate(Section::Shelf, 0)),
        "mimo Snimok je Ctrl+Enter obycajna aktivacia"
    );
    // riadky: nazov a datum (v nazve pre citacku) a velkost (v hodnote)
    let r0 = entries
        .iter()
        .find(|e| e.name.starts_with("Screenshot 0.png"))
        .unwrap()
        .clone();
    assert!(r0.name.contains("2026-11-22 10:15"), "{}", r0.name);
    let i0 = entries.iter().position(|e| e.name == r0.name).unwrap();
    assert_eq!(extras[i0].value, "1,2 MB");
    // potiahnutie riadku von zostava (len kopiu riesi aplikacia)
    p.pointer(PointerKind::Down, r0.x + 10, r0.y + 10);
    let (cmd, _) = p.pointer(PointerKind::Move, r0.x + 40, r0.y + 40);
    assert_eq!(cmd, Some(ToolsCmd::DragOut(Section::Shots, 0)));
}

#[test]
fn only_visible_rows_without_a_thumbnail_are_reported_and_the_box_follows_the_dpi() {
    let _g = crate::test_lang::guard();
    let mut d = ToolsData::default();
    d.sections[3] = SectionView {
        enabled: true,
        rows: (0..60)
            .map(|i| Row {
                thumb: crate::tools_pop::Thumb::Loading,
                ..row(&format!("s{i}.png"), "d", "1 KB")
            })
            .collect(),
        status: None,
        empty: None,
    };
    let mut p = ToolsPanel::default();
    p.section = Section::Shots;
    paint(&mut p, &d, 96, 1.0, true);
    let wanted = p.take_wanted();
    // viditelne riadky (a najviac jeden pod nimi), nie vsetkych 60
    assert!(!wanted.is_empty() && wanted.len() <= 9, "{wanted:?}");
    assert_eq!(wanted[0], 0);
    assert!(
        p.take_wanted().is_empty(),
        "zoznam sa po prevzati vyprazdni"
    );
    assert_eq!(p.thumb_box(), (74, 46));
    paint(&mut p, &d, 192, 1.0, true);
    assert_eq!(
        p.thumb_box(),
        (150, 94),
        "plocha sa skaluje s DPI (bez obrysu)"
    );
    // po posunuti sa hlasia riadky, ktore su teraz vidiet
    assert!(p.wheel(-1500));
    p.glide_finish();
    paint(&mut p, &d, 96, 1.0, true);
    let later = p.take_wanted();
    assert!(!later.is_empty() && later[0] > 5, "{later:?}");
    // vyber sipkou: aj pri vyssich riadkoch snimok ostane vybrany riadok viditelny
    let mut q = ToolsPanel::default();
    q.section = Section::Shots;
    paint(&mut q, &d, 96, 1.0, true);
    for _ in 0..40 {
        q.key(0x28, false, false, &d);
        paint(&mut q, &d, 96, 1.0, true);
    }
    let (entries, extras, _, _) = q.a11y();
    let i = entries
        .iter()
        .position(|e| e.name.starts_with("s40.png,"))
        .unwrap();
    assert_eq!(
        extras[i].state & transom_platform::menu_access::STATE_OFFSCREEN,
        0,
        "vybrany riadok je viditelny"
    );
}

#[test]
fn the_screenshot_library_passes_the_audit_in_every_language_theme_and_dpi_and_explains_empty_states(
) {
    let _g = crate::test_lang::guard();
    let long = "Screenshot 2026-11-22 101530 - velmi dlhy nazov snimky pre kontrolu orezania v zozname.png";
    let mut bad = Vec::new();
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        let texts = [
            transom_i18n::t("tools.lib_empty"),
            transom_i18n::t("tools.lib_off"),
            transom_i18n::t("tools.lib_nofolders"),
            transom_i18n::t("tools.lib_loading"),
            transom_i18n::tf("tools.lib_problem", &["2"]),
            transom_i18n::tf("tools.lib_trunc", &["300", "12345"]),
            transom_i18n::t("tools.lib_readonly"),
            transom_i18n::t("tools.lib_open_err"),
            transom_i18n::t("tools.lib_open"),
            transom_i18n::t("tools.lib_folder"),
            transom_i18n::t("tools.lib_refresh"),
        ];
        assert!(
            texts
                .iter()
                .all(|t| !t.is_empty() && !t.starts_with("tools.")),
            "{lang:?}: {texts:?}"
        );
        for dpi in [96u32, 144, 192] {
            for dark in [true, false] {
                for (name, view) in [
                    (
                        "rows",
                        SectionView {
                            enabled: true,
                            rows: {
                                let mut r = shot_rows(12);
                                r[0].title = long.into();
                                r[0].subtitle = format!(
                                    "2026-11-22 10:15 · {}",
                                    transom_i18n::t("tools.lib_nopreview")
                                );
                                r
                            },
                            status: Some(transom_i18n::tf("tools.lib_trunc", &["300", "12345"])),
                            empty: None,
                        },
                    ),
                    (
                        "empty",
                        SectionView {
                            enabled: true,
                            rows: vec![],
                            status: Some(transom_i18n::tf("tools.lib_problem", &["2"])),
                            empty: Some(transom_i18n::t("tools.lib_empty")),
                        },
                    ),
                    (
                        "off",
                        SectionView {
                            enabled: false,
                            rows: vec![],
                            status: None,
                            empty: Some(transom_i18n::t("tools.lib_off")),
                        },
                    ),
                ] {
                    let mut d = data();
                    d.sections[3] = view;
                    let mut p = ToolsPanel::default();
                    p.section = Section::Shots;
                    for i in paint(&mut p, &d, dpi, 3.0, dark) {
                        bad.push(format!("{} {dpi} {name} {dark}: {i}", lang.code()));
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
        bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    );
}

type A11y = (
    Vec<transom_platform::menu_access::MenuEntry>,
    Vec<transom_platform::menu_access::PageExtra>,
    Option<usize>,
    String,
);

/// Pristupnost panela v typickych stavoch (pre vypis pred/po zmene vzhladu v `crate::gallery`).
pub(crate) fn a11y_samples() -> Vec<(String, A11y)> {
    let mut d = data();
    d.sections[3] = SectionView {
        enabled: true,
        rows: shot_rows(3),
        status: None,
        empty: None,
    };
    let mut out = Vec::new();
    for sec in Section::ALL {
        let mut p = ToolsPanel::default();
        p.switch_for_test(sec);
        paint(&mut p, &d, 96, 1.0, true);
        out.push((format!("{sec:?}"), p.a11y()));
    }
    let mut p = ToolsPanel::default();
    paint(&mut p, &d, 96, 1.0, true);
    p.focus = Some(ToolsHit::Row(0));
    p.key(0x2E, false, false, &d);
    paint(&mut p, &d, 96, 1.0, true);
    out.push(("confirm".into(), p.a11y()));
    let mut p = ToolsPanel::default();
    p.switch_for_test(Section::Notes);
    p.open_editor(None, "a\nb".into());
    paint(&mut p, &d, 96, 1.0, true);
    out.push(("editor".into(), p.a11y()));
    out
}

/// Galeria (pozri `crate::gallery`): vsetky sekcie Nastrojov, fokus na tlacidle, dvojkrokove mazanie a editor poznamky.
#[test]
#[ignore]
fn gallery_tools() {
    let _g = crate::test_lang::guard();
    let Some(dir) = crate::gallery::dir() else {
        return;
    };
    let mut d = data();
    d.sections[1] = SectionView {
        enabled: true,
        rows: vec![
            row("Meeting moved to 15:00", "Text · Outlook", "10:42"),
            Row {
                pinned: true,
                ..row("https://example.com/report", "Text · Edge", "09:15")
            },
            row("Image 1920 × 1080", "Image · Paint", "Yesterday"),
        ],
        status: None,
        empty: None,
    };
    d.sections[2] = SectionView {
        enabled: true,
        rows: vec![
            Row {
                pinned: true,
                ..row("Shopping list", "milk, bread, coffee", "Today")
            },
            row("Ideas for the bar", "compact clock, calmer colours", "Mon"),
        ],
        status: None,
        empty: None,
    };
    d.sections[3] = SectionView {
        enabled: true,
        rows: shot_rows(7),
        status: None,
        empty: None,
    };
    for dpi in crate::gallery::DPIS {
        for (dark, th) in crate::gallery::themes() {
            let (w, h) = (
                crate::gallery::px(TOOLS_W_LP, dpi),
                crate::gallery::px(TOOLS_H_LP, dpi),
            );
            let shoot = |p: &mut ToolsPanel, name: &str| {
                let surf = Surface::new(w, h);
                for _ in 0..2 {
                    let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                    p.paint(&mut gp, &th, &d);
                }
                crate::gallery::save(
                    &dir,
                    &format!("tools-{name}-{}-{dpi}", crate::gallery::tag(dark)),
                    &surf,
                );
            };
            for sec in Section::ALL {
                let mut p = ToolsPanel::default();
                p.switch_for_test(sec);
                shoot(&mut p, &format!("{sec:?}").to_lowercase());
            }
            // fokus na tlacidle Pridat a vybrany druhy riadok
            let mut p = ToolsPanel::default();
            shoot(&mut p, "warmup");
            p.focus = Some(ToolsHit::ShelfAdd);
            p.key(0x28, false, false, &d);
            p.focus = Some(ToolsHit::ShelfAdd);
            shoot(&mut p, "shelf-focus");
            // dvojkrokove mazanie (prvy krok: potvrdenie)
            let mut p = ToolsPanel::default();
            shoot(&mut p, "warmup");
            p.focus = Some(ToolsHit::Row(0));
            p.key(0x2E, false, false, &d);
            shoot(&mut p, "shelf-confirm");
            // editor poznamky
            let mut p = ToolsPanel::default();
            p.switch_for_test(Section::Notes);
            p.open_editor(None, "Shopping list\nmilk, bread, coffee".into());
            shoot(&mut p, "notes-editor");
            let _ = std::fs::remove_file(dir.join(format!(
                "tools-warmup-{}-{dpi}.png",
                crate::gallery::tag(dark)
            )));
        }
    }
}

fn long_data(n: usize) -> ToolsData {
    let mut d = ToolsData::default();
    d.sections[0] = SectionView {
        enabled: true,
        rows: (0..n)
            .map(|i| row(&format!("file-{i}.txt"), "C:/docs", "1 KB"))
            .collect(),
        status: None,
        empty: None,
    };
    d
}

#[test]
fn a_long_list_never_paints_or_clicks_over_the_header_and_keeps_the_selected_row_and_messages_visible(
) {
    use transom_platform::menu_access::STATE_OFFSCREEN;
    let _g = crate::test_lang::guard();
    let d = long_data(30);
    let mut p = ToolsPanel::default();
    p.msg = Some("Fixed footer message".into());
    paint(&mut p, &d, 96, 1.0, true);
    // zoznam sa posunie koliesko: hlavicka (Search, karty) ostava klikatelna a riadky pod nou sa nedaju trafit
    assert!(p.wheel(-1500));
    p.glide_finish();
    paint(&mut p, &d, 96, 1.0, true);
    let (entries, extras, _, _) = p.a11y();
    let search = entries.iter().find(|e| e.name == "Search").unwrap();
    assert_eq!(
        p.hit(search.x + search.w / 2, search.y + search.h / 2),
        Some(ToolsHit::Search)
    );
    let tab = entries.iter().find(|e| e.name == "Clipboard").unwrap();
    assert_eq!(
        p.hit(tab.x + 4, tab.y + 4),
        Some(ToolsHit::Tab(Section::Clipboard))
    );
    // sprava je v pevnej paticke: viditelna aj pri dlhom zozname
    let (mi, m) = entries
        .iter()
        .enumerate()
        .find(|(_, e)| e.name == "Fixed footer message")
        .unwrap();
    assert!(m.y >= 0 && m.y + m.h <= (560.0) as i32 && extras[mi].state & STATE_OFFSCREEN == 0);
    // vyber sipkou: riadok sa dostane do viditelnej casti
    let mut q = ToolsPanel::default();
    paint(&mut q, &d, 96, 1.0, true);
    for _ in 0..25 {
        q.key(0x28, false, false, &d);
        paint(&mut q, &d, 96, 1.0, true);
    }
    let (entries, extras, _, _) = q.a11y();
    let i = entries
        .iter()
        .position(|e| e.name.starts_with("file-25.txt,"))
        .unwrap();
    assert_eq!(
        extras[i].state & STATE_OFFSCREEN,
        0,
        "vybrany riadok je viditelny"
    );
    assert!(q.scroll > 0);
}

#[test]
fn releasing_after_a_drag_never_activates_the_row_or_tab_under_the_cursor() {
    let _g = crate::test_lang::guard();
    let d = long_data(3);
    let mut p = ToolsPanel::default();
    paint(&mut p, &d, 96, 2.0, true);
    let (entries, _, _, _) = p.a11y();
    let r0 = entries
        .iter()
        .find(|e| e.name.starts_with("file-0.txt,"))
        .unwrap()
        .clone();
    let r1 = entries
        .iter()
        .find(|e| e.name.starts_with("file-1.txt,"))
        .unwrap()
        .clone();
    assert_eq!(p.pointer(PointerKind::Down, r0.x + 10, r0.y + 10).0, None);
    // potiahnutie nad druhy riadok spusti tahanie von
    let (cmd, _) = p.pointer(PointerKind::Move, r1.x + 10, r1.y + 10);
    assert_eq!(cmd, Some(ToolsCmd::DragOut(Section::Shelf, 0)));
    // uvolnenie nad inym riadkom (tahanie dobehlo) nesmie nic aktivovat
    assert_eq!(p.pointer(PointerKind::Click, r1.x + 10, r1.y + 10).0, None);
    // obycajny klik bez pohybu stale aktivuje
    p.pointer(PointerKind::Down, r1.x + 10, r1.y + 10);
    assert_eq!(
        p.pointer(PointerKind::Click, r1.x + 10, r1.y + 10).0,
        Some(ToolsCmd::Activate(Section::Shelf, 1))
    );
}

#[test]
fn the_note_editor_discards_from_the_keyboard() {
    let _g = crate::test_lang::guard();
    let mut p = ToolsPanel::default();
    p.open_editor(None, String::new());
    p.editor_char('x');
    assert_eq!(p.editor_key(0x2E, true).0, Some(ToolsCmd::EditorDiscard));
    assert_eq!(
        p.editor_key(0x2E, false).0,
        None,
        "samotne Delete nic nerobi"
    );
}

fn ided(items: &[(&str, &str)]) -> ToolsData {
    let mut d = ToolsData::default();
    d.sections[0] = SectionView {
        enabled: true,
        rows: items
            .iter()
            .map(|(id, title)| Row {
                id: (*id).into(),
                ..row(title, "C:/docs", "1 KB")
            })
            .collect(),
        status: None,
        empty: None,
    };
    d
}

#[test]
fn ctrl_up_and_down_reorder_only_the_shelf_and_the_selection_follows_the_item_by_id() {
    let _g = crate::test_lang::guard();
    let d = ided(&[("a", "one.txt"), ("b", "two.txt"), ("c", "three.txt")]);
    let mut p = ToolsPanel::default();
    paint(&mut p, &d, 96, 2.0, true);
    p.key(0x28, false, false, &d); // vyber b
    assert_eq!(p.selected, 1);
    // bez Ctrl sipky len vyberaju, s Ctrl posuvaju (opakovanie sa ignoruje)
    assert_eq!(
        p.key_ctrl(0x26, false, true, false, &d).0,
        Some(ToolsCmd::Move(Section::Shelf, 1, -1))
    );
    assert_eq!(
        p.key_ctrl(0x28, false, true, false, &d).0,
        Some(ToolsCmd::Move(Section::Shelf, 1, 1))
    );
    assert_eq!(
        p.key_ctrl(0x28, false, true, true, &d).0,
        None,
        "drzana klavesa"
    );
    // sluzba zmenila poradie na b, a, c: vyber ostal na polozke b (index 0), nie na indexe 1
    let d2 = ided(&[("b", "two.txt"), ("a", "one.txt"), ("c", "three.txt")]);
    paint(&mut p, &d2, 96, 2.0, true);
    assert_eq!(p.selected, 0, "vyber nasleduje id");
    // aj po pripnuti (zoradi sa na vrch) a po pridani novej polozky navrch vyber ostane pri tej istej polozke
    let d3 = ided(&[
        ("n", "new.txt"),
        ("b", "two.txt"),
        ("a", "one.txt"),
        ("c", "three.txt"),
    ]);
    paint(&mut p, &d3, 96, 2.0, true);
    assert_eq!(p.selected, 1);
    // ina sekcia: Ctrl+hore / dole nic nerobi a Ctrl+O nepridava subory
    p.switch_for_test(Section::Notes);
    let mut dn = ToolsData::default();
    dn.sections[2] = SectionView {
        enabled: true,
        rows: vec![row("n1", "", ""), row("n2", "", "")],
        status: None,
        empty: None,
    };
    paint(&mut p, &dn, 96, 2.0, true);
    assert_eq!(p.key_ctrl(0x26, false, true, false, &dn).0, None);
    assert_eq!(p.key_ctrl(0x4F, false, true, false, &dn).0, None);
    assert_eq!(p.key_ctrl(0x2D, false, false, false, &dn).0, None);
}

#[test]
fn adding_files_works_from_the_keyboard_and_the_buttons_with_accessible_names() {
    use transom_platform::menu_access::ROLE_PUSHBUTTON;
    let _g = crate::test_lang::guard();
    let d = ided(&[("a", "one.txt"), ("b", "two.txt")]);
    let mut p = ToolsPanel::default();
    paint(&mut p, &d, 96, 2.0, true);
    assert_eq!(
        p.key_ctrl(0x4F, false, true, false, &d).0,
        Some(ToolsCmd::ShelfAdd)
    );
    assert_eq!(
        p.key_ctrl(0x2D, false, false, false, &d).0,
        Some(ToolsCmd::ShelfAdd)
    );
    assert_eq!(
        p.key_ctrl(0x4F, false, true, true, &d).0,
        None,
        "opakovanie"
    );
    assert_eq!(
        p.key_ctrl(0x4F, false, false, false, &d).0,
        None,
        "samotne O nic nerobi"
    );
    let (entries, extras, _, _) = p.a11y();
    for (name, cmd) in [
        ("Add files…", ToolsCmd::ShelfAdd),
        ("Move up", ToolsCmd::Move(Section::Shelf, 0, -1)),
        ("Move down", ToolsCmd::Move(Section::Shelf, 0, 1)),
    ] {
        let i = entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(extras[i].role, ROLE_PUSHBUTTON);
        let e = &entries[i];
        assert_eq!(
            p.pointer(PointerKind::Click, e.x + e.w / 2, e.y + e.h / 2)
                .0,
            Some(cmd),
            "{name}"
        );
    }
    // tlacidla Police su len v Polici
    p.switch_for_test(Section::Notes);
    paint(&mut p, &ToolsData::default(), 96, 2.0, true);
    assert!(!p.a11y().0.iter().any(|e| e.name == "Add files…"));
}

#[test]
fn shelf_rows_announce_their_position_and_the_delete_confirmation_stays_with_the_item() {
    let _g = crate::test_lang::guard();
    let d = ided(&[("a", "one.txt"), ("b", "two.txt"), ("c", "three.txt")]);
    let mut p = ToolsPanel::default();
    paint(&mut p, &d, 96, 2.0, true);
    let names: Vec<String> = p.a11y().0.into_iter().map(|e| e.name).collect();
    for (i, t) in ["one.txt", "two.txt", "three.txt"].iter().enumerate() {
        assert!(names.contains(&format!("{t}, {} of 3", i + 1)), "{names:?}");
    }
    // Delete na b; medzitym pribudne nova polozka navrch (b je teraz na indexe 2): druhy Delete maze b, nie inu polozku
    p.key(0x28, false, false, &d);
    assert_eq!(p.key(0x2E, false, false, &d).0, None);
    let d2 = ided(&[
        ("n", "new.txt"),
        ("a", "one.txt"),
        ("b", "two.txt"),
        ("c", "three.txt"),
    ]);
    paint(&mut p, &d2, 96, 2.0, true);
    assert_eq!(p.selected, 2, "vyber ostal pri b");
    assert!(p
        .a11y()
        .0
        .iter()
        .any(|e| e.name.contains("two.txt") && e.name.contains("Press Delete again")));
    assert_eq!(
        p.key(0x2E, false, false, &d2).0,
        Some(ToolsCmd::Delete(Section::Shelf, 2))
    );
    // vyber sa medzi dvoma Delete presunul na inu polozku: druhy Delete znova len pyta (nic sa nezmaze)
    let mut q = ToolsPanel::default();
    paint(&mut q, &d, 96, 2.0, true);
    assert_eq!(q.key(0x2E, false, false, &d).0, None);
    q.key(0x28, false, false, &d);
    assert_eq!(
        q.key(0x2E, false, false, &d).0,
        None,
        "ina polozka: nema sa zmazat"
    );
}

#[test]
fn pasting_into_the_note_editor_keeps_newlines_and_tabs_but_not_control_characters_and_respects_the_limit(
) {
    // poznamka si nove riadky a tabulatory zachova, riadiace znaky a CR zahodi
    let mut n = ToolsPanel::default();
    n.open_editor(None, String::new());
    assert!(n.editor_paste("a\nb\tc\u{7}\r"));
    assert_eq!(n.editor.as_ref().unwrap().text, "a\nb\tc");
    n.editor_paste(&"x".repeat(transom_tools::notes::MAX_NOTE_BYTES * 2));
    assert!(n.editor.as_ref().unwrap().text.len() <= transom_tools::notes::MAX_NOTE_BYTES);
}
