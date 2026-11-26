//! Pristupnost ovladacov stranok nastaveni na skutocne nakreslenych strankach (offscreen, bez okna a bez citacky obrazovky):
//! kazdy ovladac ma neprazdny, preklozeny a nie surovy nazov, spravnu rolu, stav a polohu v klientskej ploche.
use super::*;
use crate::page_a11y::{role_of, Target};
use transom_platform::menu_access::{
    ROLE_CHECKBUTTON, ROLE_PAGETAB, ROLE_RADIOBUTTON, ROLE_SLIDER, STATE_CHECKED, STATE_SELECTED,
};
use transom_render::offscreen::Surface;
use transom_render::{GdiPainter, Size};

pub(crate) fn paint(ui: &mut Ui, dpi: u32) -> (i32, i32) {
    let s = dpi as f32 / 96.0;
    let (cw, ch) = (
        (crate::shot::CLIENT_W_LP as f32 * s).round() as i32,
        (crate::shot::CLIENT_H_LP as f32 * s).round() as i32,
    );
    let surf = Surface::new(cw, ch);
    for _ in 0..2 {
        let mut p = GdiPainter::new(surf.dc, Size { w: cw, h: ch }, dpi);
        ui.paint(&mut p);
    }
    (cw, ch)
}

const PAGES: [Page; 21] = [
    Page::Layout,
    Page::Displays,
    Page::Media,
    Page::Appearance,
    Page::Clock,
    Page::Ai,
    Page::Sound,
    Page::Audio,
    Page::Peripherals,
    Page::Tools,
    Page::Extras,
    Page::Language,
    Page::Transfer,
    Page::Updates,
    Page::Welcome,
    Page::Clipboard,
    Page::Notes,
    Page::Shots,
    Page::Launcher,
    Page::Notch,
    Page::ToolGroup,
];

fn looks_like_a_raw_key(s: &str) -> bool {
    // surovy i18n kluc: male pismena, bodky, podciarkniky a ziadna medzera ("app.opacity", "arc.enable")
    !s.is_empty()
        && s.contains('.')
        && !s.contains(' ')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
}

#[test]
fn every_control_of_every_page_has_a_name_role_state_and_a_place_in_the_client_area() {
    let _env = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("transom-a11y-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("TRANSOM_CONFIG", root.join("config.json"));
    std::env::set_var("TRANSOM_DATA", root.join("data"));
    for (lang, dpi) in [(Lang::En, 96), (Lang::De, 96), (Lang::Ja, 192)] {
        transom_i18n::set_lang(lang);
        let loaded = transom_config::Loaded {
            config: transom_config::Config::default(),
            read_only: false,
            note: None,
            issue: None,
        };
        let mut ui = make_ui(loaded, Theme::dark(), String::new());
        for page in PAGES {
            ui.page = page;
            ui.scroll = 0;
            let (cw, ch) = paint(&mut ui, dpi);
            let (entries, extras, targets, _) = ui.a11y_snapshot(&[], None);
            let ctx = format!("{page:?} {lang:?} dpi {dpi}");
            assert_eq!(entries.len(), extras.len());
            assert_eq!(entries.len(), targets.len());
            assert!(!entries.is_empty(), "{ctx}");
            // 1) kazdy ovladac z hit-mapy je v snimke (a nic navyse bez ovladaca)
            let mut hits: Vec<&Hit> = Vec::new();
            for (_, h) in &ui.all_hits {
                if !hits.contains(&h) {
                    hits.push(h);
                }
            }
            let named: Vec<&Hit> = targets
                .iter()
                .filter_map(|t| match t {
                    Target::Hit(h) => Some(h),
                    Target::Edit(_) | Target::Static => None,
                })
                .collect();
            for h in &hits {
                assert!(named.contains(h), "{ctx}: ovladac bez nazvu {h:?}");
            }
            for (e, x) in entries.iter().zip(&extras) {
                // 2) nazov: neprazdny, nie surovy kluc, nie len symbol
                assert!(!e.name.trim().is_empty(), "{ctx}: prazdny nazov");
                assert!(
                    !looks_like_a_raw_key(&e.name),
                    "{ctx}: surovy kluc {:?}",
                    e.name
                );
                assert!(
                    e.name.chars().any(|c| c.is_alphanumeric()),
                    "{ctx}: nazov bez pismen {:?}",
                    e.name
                );
                // 3) poloha v klientskej ploche (aj mimo viditelneho okna, tam je OFFSCREEN)
                assert!(e.w > 0 && e.h > 0, "{ctx}: {:?}", e.name);
                assert!(x.role != 0, "{ctx}: {:?}", e.name);
                let off = x.state & transom_platform::menu_access::STATE_OFFSCREEN != 0;
                if !off {
                    assert!(
                        e.x >= 0 && e.y >= 0 && e.x + e.w <= cw && e.y + e.h <= ch,
                        "{ctx}: {:?} {e:?} mimo {cw}x{ch}",
                        e.name
                    );
                }
            }
            // 4) zhoda roly s hit-boxom
            for (t, x) in targets.iter().zip(&extras) {
                if let Target::Hit(h) = t {
                    assert_eq!(x.role, role_of(h), "{ctx}: {h:?}");
                }
            }
            // 5) presne jedna vybrana karta navigacie a zodpovedajuca nasej stranke
            let selected: Vec<&str> = entries
                .iter()
                .zip(&extras)
                .filter(|(_, x)| x.role == ROLE_PAGETAB && x.state & STATE_SELECTED != 0)
                .map(|(e, _)| e.name.as_str())
                .collect();
            assert_eq!(selected.len(), 1, "{ctx}: {selected:?}");
            // 6) v skupine volieb je najviac jedna zvolena (skupina = nazov pred ": ")
            let mut groups: std::collections::HashMap<String, usize> = Default::default();
            for (e, x) in entries.iter().zip(&extras) {
                if x.role == ROLE_RADIOBUTTON && x.state & STATE_CHECKED != 0 {
                    let g = e.name.rsplit_once(": ").map_or("", |(g, _)| g).to_string();
                    *groups.entry(g).or_default() += 1;
                }
            }
            for (g, n) in &groups {
                assert!(*n <= 1, "{ctx}: skupina {g:?} ma {n} zvolenych");
            }
            // 7) nazvy sa v ramci stranky nezdvojuju (rola + nazov), inak by citacka obrazovky ovladace nerozlisila
            let mut keys: Vec<(i32, &str)> = entries
                .iter()
                .zip(&extras)
                .map(|(e, x)| (x.role, e.name.as_str()))
                .collect();
            keys.sort();
            for w in keys.windows(2) {
                assert!(w[0] != w[1], "{ctx}: zdvojeny nazov {:?}", w[0]);
            }
            // 8) akcia ma kazdy ovladac okrem posuvnika a polozky rozlozenia
            for (e, x) in entries.iter().zip(&extras) {
                if x.role == ROLE_SLIDER {
                    assert!(
                        x.action.is_empty() && x.value.ends_with(" %"),
                        "{ctx}: {:?}",
                        e.name
                    );
                } else if matches!(x.role, ROLE_CHECKBUTTON | ROLE_RADIOBUTTON | ROLE_PAGETAB) {
                    assert!(!x.action.is_empty(), "{ctx}: {:?}", e.name);
                }
            }
        }
    }
    transom_i18n::set_lang(Lang::En);
    std::env::remove_var("TRANSOM_CONFIG");
    std::env::remove_var("TRANSOM_DATA");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_keyboard_focus_is_reported_as_the_focused_child_and_states_follow_the_configuration() {
    let _env = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    transom_i18n::set_lang(Lang::En);
    let loaded = transom_config::Loaded {
        config: transom_config::Config::default(),
        read_only: false,
        note: None,
        issue: None,
    };
    let mut ui = make_ui(loaded, Theme::dark(), String::new());
    ui.page = Page::Appearance;
    paint(&mut ui, 96);
    // fokus klavesnice na posuvniku: child ID = jeho poradie, hodnota = aktualne percento
    ui.kfocus = Some(Hit::OpacityTrack);
    let (entries, extras, targets, focus) = ui.a11y_snapshot(&[], None);
    let f = focus.expect("fokus na posuvniku");
    assert_eq!(targets[f], Target::Hit(Hit::OpacityTrack));
    assert_eq!(extras[f].role, ROLE_SLIDER);
    assert_eq!(extras[f].value, "100 %");
    assert_eq!(entries[f].name, t("app.opacity"));
    // po zmene hodnoty sa zmeni aj hodnota v snimke
    ui.cfg.appearance.bg_opacity = 70;
    let (_, extras, _, _) = ui.a11y_snapshot(&[], None);
    assert_eq!(extras[f].value, "70 %");
    // tlacidla +/- maju zrozumitelne nazvy, nie symboly
    let (entries, _, targets, _) = ui.a11y_snapshot(&[], None);
    let name_of = |h: Hit| {
        targets
            .iter()
            .position(|t| *t == Target::Hit(h.clone()))
            .map(|i| entries[i].name.clone())
            .unwrap()
    };
    assert_eq!(name_of(Hit::OpacityStep(-5)), t("acc.opacity_dec"));
    assert_eq!(name_of(Hit::OpacityStep(5)), t("acc.opacity_inc"));
    // pole ma fokus: snimka ho oznaci ako fokusovanu polozku (nie posuvnik)
    let edit = [(1usize, (10, 10, 40, 20), "255".to_string())];
    let (entries, extras, targets, focus) = ui.a11y_snapshot(&edit, Some(1));
    let f = focus.unwrap();
    assert_eq!(targets[f], Target::Edit(1));
    assert_eq!(extras[f].value, "255");
    assert!(entries[f].name.starts_with(&t("app.rgb")));
    // nazov korena obsahuje nazov stranky v jazyku aplikacie
    assert!(ui.a11y_page_name().contains(&t("nav.appearance")));
    // stav: zvolena farba z predvolieb sa objavi ako zvolena volba
    ui.cfg.appearance.bg_color = String::new();
    paint(&mut ui, 96);
    let (entries, extras, _, _) = ui.a11y_snapshot(&[], None);
    let theme_default = entries
        .iter()
        .zip(&extras)
        .find(|(e, _)| e.name == t("app.theme_default"))
        .expect("volba farby podla temy");
    assert!(
        theme_default.1.state & STATE_CHECKED != 0,
        "farba z temy je zvolena"
    );
}

#[test]
fn the_import_preview_exposes_named_sections_actions_and_read_only_text_for_screen_readers() {
    let _env = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for (lang, dpi, state) in [
        (Lang::En, 96, "transfer-preview"),
        (Lang::De, 96, "transfer-confirm"),
        (Lang::Ja, 192, "transfer-preview"),
    ] {
        transom_i18n::set_lang(lang);
        let loaded = transom_config::Loaded {
            config: transom_config::Config::default(),
            read_only: false,
            note: None,
            issue: None,
        };
        let mut ui = make_ui(loaded, Theme::dark(), String::new());
        ui.page = Page::Transfer;
        ui.transfer = crate::shot::transfer_fixture(state);
        paint(&mut ui, dpi);
        let (entries, extras, targets, _) = ui.a11y_snapshot(&[], None);
        let ctx = format!("{lang:?} {state}");
        // ovladace: Exportovat, Importovat, sekcie (checkboxy s poctom zmien), Importovat vybrane, Zrusit
        let role_of_hit = |h: Hit| {
            targets
                .iter()
                .position(|t| *t == Target::Hit(h.clone()))
                .map(|i| {
                    (
                        entries[i].name.clone(),
                        extras[i].role,
                        extras[i].state,
                        extras[i].action.clone(),
                    )
                })
        };
        let (n, r, _, a) = role_of_hit(Hit::TransferExport).expect("Exportovat");
        assert_eq!(
            (r, !n.is_empty(), !a.is_empty()),
            (transom_platform::menu_access::ROLE_PUSHBUTTON, true, true),
            "{ctx}"
        );
        assert!(role_of_hit(Hit::TransferImport).is_some(), "{ctx}");
        assert!(
            role_of_hit(Hit::TransferCancel).is_some() && role_of_hit(Hit::TransferApply).is_some(),
            "{ctx}"
        );
        let sections: Vec<_> = (0..6u8)
            .filter_map(|i| role_of_hit(Hit::TransferSection(i)))
            .collect();
        assert!(sections.len() >= 4, "{ctx}: sekcie so zmenami su ovladace");
        for (name, role, st, action) in &sections {
            assert_eq!(*role, ROLE_CHECKBUTTON, "{ctx}");
            assert!(st & STATE_CHECKED != 0, "{ctx}: vybrane predvolene: {name}");
            assert!(
                !action.is_empty() && name.contains(": ") || name.contains('–'),
                "{ctx}: {name}"
            );
        }
        // staticke texty: len na citanie, bez fokusu a bez akcie; nazvy bez riadiacich znakov
        let statics: Vec<usize> = targets
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == Target::Static)
            .map(|(i, _)| i)
            .collect();
        assert!(statics.len() >= 5, "{ctx}: nadpis, zmeny, ignorovane");
        for &i in &statics {
            assert_eq!(
                extras[i].role,
                transom_platform::menu_access::ROLE_STATICTEXT,
                "{ctx}"
            );
            assert!(extras[i].action.is_empty(), "{ctx}");
            assert!(!entries[i].name.chars().any(char::is_control), "{ctx}");
            assert!(entries[i].name.chars().count() <= 121, "{ctx}");
        }
        // dlhy nazov suboru z fixtury je v nadpise skrateny na rozumnu dlzku, nie cela cesta
        assert!(
            entries.iter().any(|e| e.name.starts_with(
                &tf("xfer.preview_h", &[""])
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .to_string()
            ) || e.name.contains("transom-settings")),
            "{ctx}"
        );
    }
    transom_i18n::set_lang(Lang::En);
}

/// Vypis pristupnosti (nazov, rola, stav bez OFFSCREEN, hodnota, akcia) kazdej stranky v en/de/ja do `TRANSOM_A11Y_DUMP`
/// (porovnanie pred a po zmene vzhladu: nazvy, roly ani stavy sa nesmu zmenit). `cargo test -p transom-settings
/// a11y_dump -- --ignored`.
#[test]
#[ignore]
fn a11y_dump() {
    let Some(dir) = std::env::var_os("TRANSOM_A11Y_DUMP") else {
        return;
    };
    let _env = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("transom-a11y-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("TRANSOM_CONFIG", root.join("config.json"));
    std::env::set_var("TRANSOM_DATA", root.join("data"));
    let mut out = String::new();
    for lang in [Lang::En, Lang::De, Lang::Ja] {
        transom_i18n::set_lang(lang);
        for dark in [true, false] {
            let loaded = transom_config::Loaded {
                config: transom_config::Config::default(),
                read_only: false,
                note: None,
                issue: None,
            };
            let th = if dark { Theme::dark() } else { Theme::light() };
            let mut ui = make_ui(loaded, th, String::new());
            for page in PAGES {
                ui.page = page;
                ui.scroll = 0;
                paint(&mut ui, 96);
                let (entries, extras, _, _) = ui.a11y_snapshot(&[], None);
                out.push_str(&format!("== {lang:?} {page:?} dark={dark}\n"));
                for (e, x) in entries.iter().zip(&extras) {
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
                out.push_str(&format!("page={:?}\n", ui.a11y_page_name()));
            }
        }
    }
    transom_i18n::set_lang(Lang::En);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::write(std::path::Path::new(&dir).join("settings-a11y.txt"), out).unwrap();
}

/// Snimky fokusu klavesnice na typickych ovladacoch (vybrana volba, hlavne tlacidlo, potvrdenie mazania, policko v karte,
/// navigacia) do `TRANSOM_A11Y_DUMP` (vizualna kontrola ramika; `cargo test -p transom-settings focus_shots -- --ignored`).
#[test]
#[ignore]
fn focus_shots() {
    let Some(dir) = std::env::var_os("TRANSOM_A11Y_DUMP") else {
        return;
    };
    let _env = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("transom-focus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("TRANSOM_CONFIG", root.join("config.json"));
    std::env::set_var("TRANSOM_DATA", root.join("data"));
    transom_i18n::set_lang(Lang::En);
    let cases: Vec<(&str, Page, Hit, bool)> = vec![
        ("choice", Page::Clock, Hit::Preset(0), false),
        ("primary", Page::Ai, Hit::DateAdd, false),
        ("armed", Page::Ai, Hit::DeleteData, true),
        (
            "checkbox",
            Page::Tools,
            Hit::ToolsToggle("clip.text"),
            false,
        ),
        ("nav", Page::Media, Hit::Nav(Page::Media), false),
    ];
    for dark in [true, false] {
        for dpi in [96u32, 192] {
            for (name, page, hit, armed) in &cases {
                let loaded = transom_config::Loaded {
                    config: transom_config::Config::default(),
                    read_only: false,
                    note: None,
                    issue: None,
                };
                let th = if dark { Theme::dark() } else { Theme::light() };
                let mut ui = make_ui(loaded, th, String::new());
                ui.page = *page;
                ui.scroll = 0;
                ui.confirm_delete = *armed;
                ui.kfocus = Some(hit.clone());
                let s = dpi as f32 / 96.0;
                let (cw, ch) = (
                    (crate::shot::CLIENT_W_LP as f32 * s).round() as i32,
                    (crate::shot::CLIENT_H_LP as f32 * s).round() as i32,
                );
                let surf = Surface::new(cw, ch);
                // prve kreslenie zisti polohy ovladacov, az potom sa posunie k ovladacu s fokusom (ako po stlaceni Tab)
                for i in 0..3 {
                    if i == 1 {
                        ui.reveal = Some(hit.clone());
                    }
                    let mut p = GdiPainter::new(surf.dc, Size { w: cw, h: ch }, dpi);
                    ui.paint(&mut p);
                }
                let f = std::path::Path::new(&dir).join(format!(
                    "focus-{name}-{}-{dpi}.png",
                    if dark { "dark" } else { "light" }
                ));
                let _ = std::fs::write(
                    f,
                    transom_render::offscreen::png(cw as u32, ch as u32, &surf.rgb()),
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
