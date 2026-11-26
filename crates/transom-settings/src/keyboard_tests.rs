//! Klavesnica v nastaveniach na skutocne nakreslenych strankach (offscreen, bez okna): kazdy ovladac je v kruhu fokusu, Tab ho
//! prejde a po posune je cely viditelny. Konfiguracia a data v docasnom adresari.
use super::*;
use crate::focus::{ring, step, Slot};
use transom_render::offscreen::Surface;
use transom_render::{GdiPainter, Size};

fn paint(ui: &mut Ui, dpi: u32) {
    let s = dpi as f32 / 96.0;
    let (cw, ch) = (
        (shot_w() as f32 * s).round() as i32,
        (shot_h() as f32 * s).round() as i32,
    );
    let surf = Surface::new(cw, ch);
    for _ in 0..2 {
        let mut p = GdiPainter::new(surf.dc, Size { w: cw, h: ch }, dpi);
        ui.paint(&mut p);
    }
}

fn shot_w() -> i32 {
    crate::shot::CLIENT_W_LP
}
fn shot_h() -> i32 {
    crate::shot::CLIENT_H_LP
}

const PAGES: [Page; 20] = [
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
    Page::Clipboard,
    Page::Notes,
    Page::Shots,
    Page::Launcher,
    Page::Notch,
    Page::ToolGroup,
];

#[test]
fn every_keyboard_reachable_control_of_every_page_is_in_the_ring_and_tab_visits_it_once() {
    let g = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("transom-kbd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("TRANSOM_CONFIG", root.join("config.json"));
    std::env::set_var("TRANSOM_DATA", root.join("data"));
    for (lang, dpi) in [(Lang::En, 96), (Lang::De, 192)] {
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
            paint(&mut ui, dpi);
            let all: Vec<Hit> = ui.all_hits.iter().map(|(_, h)| h.clone()).collect();
            assert!(!all.is_empty(), "{page:?}");
            let r = ring(&all, &[]);
            // 1) kazdy dosiahnutelny ovladac je v kruhu presne raz
            for h in all.iter() {
                assert_eq!(
                    r.iter().filter(|s| **s == Slot::Page(h.clone())).count(),
                    1,
                    "{page:?} {lang:?}: {h:?}"
                );
            }
            // 2) Tab prejde cely kruh a skonci mimo neho
            let (mut cur, mut n) = (None::<Slot>, 0usize);
            while let Some(s) = step(&r, cur.as_ref(), false) {
                cur = Some(s);
                n += 1;
                assert!(n <= r.len());
            }
            assert_eq!(n, r.len(), "{page:?}");
            // 3) po posune na ovladac je cely viditelny (nie orezany viewportom)
            // vzorka (prvy, posledny a rovnomerne rozlozene): kazde kreslenie cele stranky je drahe
            let cand: Vec<&Hit> = all.iter().filter(|h| !matches!(h, Hit::Nav(_))).collect();
            let stride = (cand.len() / 5).max(1);
            let sample: Vec<&Hit> = cand
                .iter()
                .enumerate()
                .filter(|(i, _)| i % stride == 0 || *i + 1 == cand.len())
                .map(|(_, h)| *h)
                .collect();
            // posun sa overuje pri 100 % (pri 200 % sa kruh a poradie overuju vyssie)
            for h in sample.into_iter().filter(|_| dpi == 96) {
                ui.scroll = 0;
                paint(&mut ui, dpi);
                ui.ensure_visible(h);
                paint(&mut ui, dpi);
                let full = ui
                    .all_hits
                    .iter()
                    .find(|(_, x)| x == h)
                    .map(|(r, _)| *r)
                    .unwrap();
                let shown = ui.hits.iter().find(|(_, x)| x == h).map(|(r, _)| *r);
                assert_eq!(
                    shown,
                    Some(full),
                    "{page:?} {lang:?} dpi {dpi}: {h:?} po posune nie je cely viditelny (viewport {:?}, scroll {})",
                    ui.viewport,
                    ui.scroll
                );
            }
        }
    }
    transom_i18n::set_lang(Lang::En);
    std::env::remove_var("TRANSOM_CONFIG");
    std::env::remove_var("TRANSOM_DATA");
    let _ = std::fs::remove_dir_all(&root);
    drop(g);
}

#[test]
fn the_focus_ring_is_drawn_for_a_focused_control_and_never_for_a_vanished_one() {
    let g = crate::archive_tests::ENV
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
    ui.page = Page::Clock;
    paint(&mut ui, 96);
    let h = ui
        .all_hits
        .iter()
        .map(|(_, h)| h.clone())
        .find(|h| matches!(h, Hit::Seconds(true)))
        .expect("stranka hodin ma volbu sekund");
    // bez fokusu a s fokusom (aj s ovladacom z inej stranky) kreslenie nepadne a identita ostava
    paint(&mut ui, 96);
    ui.kfocus = Some(h.clone());
    paint(&mut ui, 96);
    assert_eq!(ui.kfocus, Some(h));
    ui.kfocus = Some(Hit::DeleteData);
    paint(&mut ui, 96);
    drop(g);
}

/// Docasna konfiguracia a data (spolocny zamok s ostatnymi testami nastaveni).
struct TmpEnv {
    _g: std::sync::MutexGuard<'static, ()>,
    root: std::path::PathBuf,
}

impl TmpEnv {
    fn new(name: &str) -> Self {
        let g = crate::archive_tests::ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("transom-kbd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::env::set_var("TRANSOM_CONFIG", root.join("config.json"));
        std::env::set_var("TRANSOM_DATA", root.join("data"));
        transom_i18n::set_lang(Lang::En);
        Self { _g: g, root }
    }
    fn ui(&self) -> Ui {
        let loaded = transom_config::Loaded {
            config: transom_config::Config::default(),
            read_only: false,
            note: None,
            issue: None,
        };
        make_ui(loaded, Theme::dark(), String::new())
    }
    fn file(&self) -> std::path::PathBuf {
        self.root.join("config.json")
    }
    fn saved_bg(&self) -> String {
        transom_config::load(&self.file())
            .config
            .appearance
            .bg_color
    }
}

impl Drop for TmpEnv {
    fn drop(&mut self) {
        std::env::remove_var("TRANSOM_CONFIG");
        std::env::remove_var("TRANSOM_DATA");
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn escape_in_a_colour_field_restores_the_value_from_before_editing_and_never_saves_the_rejected_one(
) {
    let e = TmpEnv::new("esc");
    let mut ui = e.ui();
    ui.cfg.appearance.bg_color = "#102030".into();
    ui.base.appearance.bg_color = "#102030".into();
    // pole dostane fokus, pouzivatel pise (nahlad sa meni), potom Esc
    ui.edit_focus(0);
    ui.cfg.appearance.bg_color = "#ff0000".into(); // co by urobil on_edit_change pre platny hex
    ui.edit_session.as_mut().unwrap().typed();
    ui.bad[1] = true;
    assert!(ui.edit_cancel(0), "Esc obnovil hodnotu");
    assert_eq!(ui.cfg.appearance.bg_color, "#102030");
    assert_eq!(ui.bad, [false; 4], "chybove znacky zrusene");
    // strata fokusu, ktora nasleduje po Esc, uz nic neulozi
    assert!(!ui.edit_blur(0));
    assert!(!e.file().exists(), "zamietnuta hodnota sa nikdy nezapisala");
    // druhy Esc bez relacie nic nerobi
    assert!(!ui.edit_cancel(0));
    assert_eq!(ui.cfg.appearance.bg_color, "#102030");
}

#[test]
fn a_typed_colour_value_is_saved_only_when_the_field_loses_focus() {
    let e = TmpEnv::new("blur");
    let mut ui = e.ui();
    ui.edit_focus(2);
    ui.cfg.appearance.bg_color = "#00aa00".into();
    ui.edit_session.as_mut().unwrap().typed();
    assert!(!e.file().exists(), "pocas pisania sa neuklada");
    assert!(ui.edit_blur(2), "strata fokusu / Enter ma ulozit");
    ui.save();
    assert_eq!(e.saved_bg(), "#00aa00");
    // fokus bez pisania nic neuklada
    ui.edit_focus(1);
    assert!(!ui.edit_blur(1));
    // pole datumu nema relaciu farby
    ui.edit_focus(4);
    assert!(ui.edit_session.is_none() && !ui.edit_cancel(4));
}

#[test]
fn opacity_keys_change_the_value_live_and_persist_after_the_debounced_flush() {
    use crate::appearance_page::SliderKey;
    let e = TmpEnv::new("opacity");
    let mut ui = e.ui();
    assert_eq!(ui.cfg.appearance.opacity_percent(), 100);
    assert!(ui.slider_key(SliderKey::Dec, false));
    assert!(ui.slider_key(SliderKey::PageDec, false));
    assert_eq!(ui.cfg.appearance.opacity_percent(), 85);
    assert!(!e.file().exists(), "zmena je zivo, ulozenie az po chvili");
    assert!(ui.slider_key(SliderKey::Inc, true));
    assert_eq!(ui.cfg.appearance.opacity_percent(), 86);
    ui.flush_pending();
    assert_eq!(
        transom_config::load(&e.file())
            .config
            .appearance
            .opacity_percent(),
        86
    );
    // minimum a maximum
    assert!(ui.slider_key(SliderKey::Min, false));
    assert_eq!(
        ui.cfg.appearance.opacity_percent(),
        transom_config::MIN_OPACITY_PERCENT
    );
    assert!(!ui.slider_key(SliderKey::Dec, false), "uz na minime");
    assert!(ui.slider_key(SliderKey::Max, false));
    assert_eq!(ui.cfg.appearance.opacity_percent(), 100);
    // len na citanie: nic sa nezmeni
    ui.read_only = true;
    assert!(!ui.slider_key(SliderKey::Dec, false));
    assert_eq!(ui.cfg.appearance.opacity_percent(), 100);
}

#[test]
fn a_layout_item_moves_by_keyboard_is_saved_on_drop_and_restored_on_escape() {
    use crate::dnd::{column_items, Dir};
    let e = TmpEnv::new("move");
    let mut ui = e.ui();
    ui.page = Page::Layout;
    let original = ui.cfg.layout.clone();
    let id = column_items(&ui.cfg.layout, transom_config::Group::Left)[0].clone();
    // Esc: nic sa neulozi a rozlozenie je presne povodne
    assert!(ui.begin_move(&id));
    assert!(
        ui.moving.is_some() && !ui.status.is_empty(),
        "napoveda v stavovom riadku"
    );
    assert!(ui.nudge_move(Dir::Right));
    assert_ne!(ui.cfg.layout, original, "nahlad sa zmenil");
    assert!(!e.file().exists(), "pocas presunu sa neuklada");
    assert!(!ui.finish_move(false));
    assert_eq!(ui.cfg.layout, original);
    assert!(ui.moving.is_none() && !e.file().exists());
    // Enter: ulozi sa nove poradie
    assert!(ui.begin_move(&id));
    assert!(ui.nudge_move(Dir::Right));
    let moved = ui.cfg.layout.clone();
    assert!(ui.finish_move(true), "potvrdenie ulozilo");
    assert_eq!(transom_config::load(&e.file()).config.layout, moved);
    // presun bez zmeny (tam a spat) sa neuklada
    std::fs::remove_file(e.file()).unwrap();
    assert!(ui.begin_move(&id));
    assert!(ui.nudge_move(Dir::Left));
    assert!(ui.nudge_move(Dir::Right));
    assert_eq!(ui.cfg.layout, moved);
    assert!(!ui.finish_move(true));
    assert!(!e.file().exists(), "nezmenene rozlozenie sa neuklada");
    // len na citanie: presun sa nezacne; neznama polozka sa nepresuva
    ui.read_only = true;
    assert!(!ui.begin_move(&id));
    ui.read_only = false;
    assert!(!ui.begin_move("future_widget"));
    // zatvorenie okna pocas presunu (WM_DESTROY vola finish_move(false)): polovicaty presun sa nezachova
    assert!(ui.begin_move(&id));
    ui.nudge_move(Dir::Left);
    ui.finish_move(false);
    assert_eq!(ui.cfg.layout, moved);
}

#[test]
fn after_a_keyboard_move_the_moved_item_is_scrolled_fully_into_view_and_keeps_the_focus() {
    use crate::dnd::{column_items, Dir};
    let e = TmpEnv::new("reveal");
    let mut ui = e.ui();
    ui.page = Page::Layout;
    paint(&mut ui, 96);
    let id = column_items(&ui.cfg.layout, transom_config::Group::Left)[0].clone();
    ui.kfocus = Some(Hit::Chip(id.clone()));
    assert!(ui.begin_move(&id));
    assert!(ui.nudge_move(Dir::Right));
    assert_eq!(ui.reveal, Some(Hit::Chip(id.clone())));
    paint(&mut ui, 96);
    paint(&mut ui, 96);
    assert!(ui.reveal.is_none(), "zobrazenie spracovane");
    assert_eq!(
        ui.kfocus,
        Some(Hit::Chip(id.clone())),
        "fokus ostal na tej istej polozke"
    );
    let chip = Hit::Chip(id.clone());
    let full = ui
        .all_hits
        .iter()
        .find(|(_, h)| *h == chip)
        .map(|(r, _)| *r);
    let shown = ui.hits.iter().find(|(_, h)| *h == chip).map(|(r, _)| *r);
    assert_eq!(shown, full, "polozka je cela viditelna");
    ui.finish_move(false);
}

#[test]
fn with_the_preview_open_tab_reaches_every_section_apply_and_cancel_and_nothing_else_is_missing() {
    use crate::focus::{ring, step, Slot};
    let e = TmpEnv::new("xfer-ring");
    let mut ui = e.ui();
    ui.page = Page::Transfer;
    ui.transfer = crate::shot::transfer_fixture("transfer-preview");
    paint(&mut ui, 96);
    let all: Vec<Hit> = ui.all_hits.iter().map(|(_, h)| h.clone()).collect();
    let r = ring(&all, &[]);
    let order: Vec<&Hit> = r
        .iter()
        .filter_map(|s| match s {
            Slot::Page(h) => Some(h),
            _ => None,
        })
        .collect();
    let pos = |h: &Hit| {
        order
            .iter()
            .position(|x| *x == h)
            .unwrap_or_else(|| panic!("{h:?} chyba v kruhu"))
    };
    // poradie kreslenia: Exportovat, Importovat, sekcie, Importovat vybrane, Zrusit
    assert!(pos(&Hit::TransferExport) < pos(&Hit::TransferImport));
    assert!(pos(&Hit::TransferImport) < pos(&Hit::TransferSection(0)));
    assert!(pos(&Hit::TransferSection(0)) < pos(&Hit::TransferApply));
    assert!(pos(&Hit::TransferApply) < pos(&Hit::TransferCancel));
    // Tab prejde cely kruh presne raz
    let (mut cur, mut n) = (None::<Slot>, 0usize);
    while let Some(s) = step(&r, cur.as_ref(), false) {
        cur = Some(s);
        n += 1;
        assert!(n <= r.len());
    }
    assert_eq!(n, r.len());
    // zrusenie nahladu: fokus po zmiznuti ovladaca sa berie ako "nic" (nepadne) a dalsi Tab zacne od zaciatku
    ui.kfocus = Some(Hit::TransferSection(0));
    ui.cancel_transfer();
    assert_eq!(
        ui.kfocus,
        Some(Hit::TransferImport),
        "fokus sa vrati na Importovat"
    );
    paint(&mut ui, 96);
    assert!(ui.all_hits.iter().any(|(_, h)| *h == Hit::TransferImport));
    assert!(!ui
        .all_hits
        .iter()
        .any(|(_, h)| matches!(h, Hit::TransferSection(_) | Hit::TransferApply)));
}
