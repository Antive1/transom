//! Bocny panel ako strom kategorii (rozbalit / zbalit, podkategorie pod kategoriou) a oddelovace na stranke
//! Rozlozenie. Kreslene offscreen skutocnym rendererom.
use super::*;
use crate::a11y_tests::paint;
use crate::archive_tests::Env;

fn has(ui: &Ui, h: &Hit) -> bool {
    ui.all_hits.iter().any(|(_, x)| x == h)
}

#[test]
fn categories_expand_and_collapse_and_their_subpages_appear_right_under_them() {
    let e = Env::new("nav-tree");
    let mut ui = e.ui();
    paint(&mut ui, 96);
    // na zaciatku je rozbalena len kategoria otvorenej stranky (Lista)
    assert!(has(&ui, &Hit::Nav(Page::Layout)) && has(&ui, &Hit::Nav(Page::Appearance)));
    assert!(!has(&ui, &Hit::Nav(Page::Clipboard)), "Nastroje su zbalene");
    for i in 0..CATS.len() as u8 {
        assert!(has(&ui, &Hit::Cat(i)), "kategoria {i} je vzdy v paneli");
    }
    // klik na kategoriu ju rozbali (stranka sa nemeni) a podkategorie su pod nou, v poradi
    ui.click(Hit::Cat(2));
    paint(&mut ui, 96);
    assert_eq!(ui.page, Page::Layout);
    let y = |h: Hit| {
        ui.all_hits
            .iter()
            .find(|(_, x)| *x == h)
            .map(|(r, _)| r.y)
            .unwrap()
    };
    let cat = y(Hit::Cat(2));
    let subs: Vec<i32> = CATS[2].2.iter().map(|(pg, _)| y(Hit::Nav(*pg))).collect();
    assert!(subs.windows(2).all(|w| w[0] < w[1]), "{subs:?}");
    assert!(
        cat < subs[0] && subs[4] < y(Hit::Cat(3)),
        "pod kategoriou, pred dalsou"
    );
    // druhy klik ju zbali
    ui.click(Hit::Cat(2));
    paint(&mut ui, 96);
    assert!(!has(&ui, &Hit::Nav(Page::Shots)));
    // otvorenie stranky (aj zvonku) rozbali jej kategoriu
    ui.page = Page::Transfer;
    paint(&mut ui, 96);
    assert!(has(&ui, &Hit::Nav(Page::Transfer)) && has(&ui, &Hit::Nav(Page::Language)));
}

#[test]
fn the_tree_never_runs_out_of_the_window_even_with_every_category_open() {
    let e = Env::new("nav-fit");
    for lang in [
        transom_i18n::Lang::En,
        transom_i18n::Lang::De,
        transom_i18n::Lang::Fr,
    ] {
        transom_i18n::set_lang(lang);
        let mut ui = e.ui();
        ui.page = Page::Tools;
        for i in 0..CATS.len() as u8 {
            if !ui.expanded[i as usize] {
                ui.click(Hit::Cat(i));
            }
        }
        for dpi in [96, 144, 192] {
            let (_, ch) = paint(&mut ui, dpi);
            paint(&mut ui, dpi);
            let bottom = ui
                .hits
                .iter()
                .filter(|(_, h)| matches!(h, Hit::Nav(_) | Hit::Cat(_)))
                .map(|(r, _)| r.y + r.h)
                .max()
                .unwrap();
            assert!(
                bottom <= ch,
                "{lang:?} @ {dpi}: strom konci na {bottom} > {ch}"
            );
            // otvorena stranka a naposledy rozbalena kategoria ostanu viditelne
            assert!(has(&ui, &Hit::Nav(Page::Tools)));
        }
    }
}

#[test]
fn separators_are_added_per_group_removed_and_saved_from_the_layout_page() {
    let e = Env::new("nav-sep");
    let mut ui = e.ui();
    ui.click(Hit::Nav(Page::Layout));
    paint(&mut ui, 96);
    assert!(has(&ui, &Hit::AddSep(Group::Left)) && has(&ui, &Hit::AddSep(Group::Right)));
    ui.click(Hit::AddSep(Group::Left));
    paint(&mut ui, 96);
    let saved = e.saved();
    assert!(saved.layout.in_group(Group::Left).any(|s| s.id == "sep-1"));
    // oddelovac ma vlastny riadok s tlacidlom Odstranit (nie "Zobrazit v liste") a pristupny nazov
    assert!(has(&ui, &Hit::RemoveSep("sep-1".into())));
    assert!(!has(&ui, &Hit::Show("sep-1".into())));
    ui.click(Hit::RemoveSep("sep-1".into()));
    assert!(!e.saved().layout.items.iter().any(|s| s.id == "sep-1"));
    // nove tlacidla nastrojov su na stranke ako polozky s polickom
    paint(&mut ui, 96);
    for id in ["tool_shelf", "tool_clipboard", "tool_notes", "tool_shots"] {
        assert!(has(&ui, &Hit::Show(id.into())), "{id}");
    }
}
