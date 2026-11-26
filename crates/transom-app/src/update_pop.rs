//! Panel dostupnej aktualizacie (popover zo sady `pop_kit`): co je nove a co sa opravilo (z poznamok k vydaniu),
//! nainstalovana verzia, tlacidla Aktualizovat teraz (prenosna kopia: Otvorit stranku vydania), Neskor a Poznamky
//! k vydaniu. Pocas stahovania je hlavne tlacidlo zakazane, chyba sa ukaze priamo v paneli.
//!
//! Ciste skladanie riadkov bez Win32, testovatelne offscreen.
use crate::pop_kit::{Btn, Kit, Row};
use crate::update::{Notes, Release};
use transom_i18n::{t, tf};

/// Tlacidla panela.
pub const UP_NOW: u32 = 1;
pub const UP_LATER: u32 = 2;
pub const UP_NOTES: u32 = 3;

/// Kolko bodov kazdej sekcie panel ukaze (dalsie: "…a N dalsich").
pub const SHOW_NOTES: usize = 4;
/// Najviac znakov chyby v paneli.
const MAX_ERROR_CHARS: usize = 160;
/// Hranice vysky panela (lp).
pub const MIN_H_LP: i32 = 200;
pub const MAX_H_LP: i32 = 600;

/// Co panel ukazuje.
#[derive(Clone, Debug)]
pub struct UpdView<'a> {
    pub release: &'a Release,
    pub installed: &'a str,
    /// Nainstalovana kopia sa vie aktualizovat sama (inak sa otvori stranka vydania).
    pub self_update: bool,
    /// Stahuje sa a overuje instalator.
    pub busy: bool,
    pub error: Option<&'a str>,
}

fn section(rows: &mut Vec<Row>, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    rows.push(Row::Section(title.to_string()));
    for it in items.iter().take(SHOW_NOTES) {
        rows.push(Row::Bullet(it.clone()));
    }
    let more = items.len().saturating_sub(SHOW_NOTES);
    if more > 0 {
        rows.push(Row::Text {
            text: tf("upd.more", &[&more.to_string()]),
            strong: false,
        });
    }
    rows.push(Row::Gap(4));
}

pub fn rows(v: &UpdView) -> Vec<Row> {
    let r = v.release;
    let mut rows = vec![
        Row::Title(tf("upd.pop_title", &[&r.version])),
        Row::Text {
            text: tf("upd.installed", &[v.installed]),
            strong: false,
        },
        Row::Gap(6),
    ];
    if r.notes.is_empty() {
        rows.push(Row::Text {
            text: t("upd.no_notes"),
            strong: false,
        });
        rows.push(Row::Gap(4));
    } else {
        section(&mut rows, &t("upd.new"), &r.notes.added);
        section(&mut rows, &t("upd.fixes"), &r.notes.fixed);
    }
    if v.busy {
        rows.push(Row::Text {
            text: t("upd.verifying"),
            strong: true,
        });
    } else if let Some(e) = v.error {
        rows.push(Row::Text {
            text: tf("upd.failed", &[&crate::update::clip(e, MAX_ERROR_CHARS)]),
            strong: true,
        });
    }
    if !v.self_update {
        rows.push(Row::Text {
            text: t("upd.portable"),
            strong: false,
        });
    }
    rows.push(Row::Gap(4));
    let mut btns = Vec::new();
    if v.self_update {
        btns.push(Btn::new(UP_NOW, t("upd.now")).primary().enabled(!v.busy));
        btns.push(Btn::new(UP_LATER, t("upd.later")));
        btns.push(Btn::new(UP_NOTES, t("upd.notes")));
    } else {
        btns.push(Btn::new(UP_NOW, t("upd.open_page")).primary());
        btns.push(Btn::new(UP_LATER, t("upd.later")));
    }
    rows.push(Row::Buttons(btns));
    rows
}

/// Vyska panela v lp podla obsahu (namerane offscreen pri 100, 150 a 200 %, zalamovanie sa pri inom DPI moze lisit),
/// aby kratke poznamky nemali prazdne miesto a dlhe sa nemuseli posuvat. Rezerva na riadok stavu (stahovanie), ktory
/// sa moze objavit po otvoreni.
pub fn height_lp(v: &UpdView, w_lp: i32) -> i32 {
    use windows::Win32::Graphics::Gdi::{CreateCompatibleDC, DeleteDC};
    // pamatovy DC bez vlastnej bitmapy (kreslenie do 1x1 je neskodne, meria sa text); zlyhanie = pevna vyska, nikdy panika
    // SAFETY: vlastny DC, uvolni sa nizsie
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.is_invalid() {
        return crate::extras::kit_h(crate::POP_UPDATE);
    }
    let measure = |v: &UpdView, dpi: u32| -> i32 {
        let sc = dpi as f32 / 96.0;
        let (w, h) = (
            (w_lp as f32 * sc).round() as i32,
            (MAX_H_LP as f32 * sc).round() as i32,
        );
        let mut gp = transom_render::GdiPainter::new(dc, transom_render::Size { w, h }, dpi);
        let mut kit = Kit::default();
        kit.paint(&mut gp, &transom_render::Theme::dark(), &rows(v));
        (kit.content_h as f32 / sc).ceil() as i32
    };
    let busy = UpdView {
        busy: v.self_update,
        ..v.clone()
    };
    let mut h = 0;
    for dpi in [96, 144, 192] {
        h = h.max(measure(v, dpi)).max(measure(&busy, dpi));
    }
    // SAFETY: DC vytvoreny vyssie, painter uz neexistuje
    unsafe {
        let _ = DeleteDC(dc);
    }
    (h + 2).clamp(MIN_H_LP, MAX_H_LP)
}

/// Ikona na liste: je novsia verzia, kontrola aktualizacii je zapnuta a ikona nie je vypnuta v nastaveniach.
pub fn icon_version<'a>(
    avail: Option<&'a Release>,
    cfg: &transom_config::Updates,
) -> Option<&'a str> {
    avail
        .filter(|_| cfg.auto_check && cfg.show_icon)
        .map(|r| r.version.as_str())
}

/// Text oznamenia o novej verzii (bublina pri ikone v oblasti oznameni: najviac 255 znakov).
pub fn notice_text(r: &Release) -> String {
    let n = &r.notes;
    let text = match (n.added.len(), n.fixed.len()) {
        (0, 0) => tf("upd.available", &[&r.version]),
        (a, 0) => tf("upd.available_new", &[&r.version, &a.to_string()]),
        (0, f) => tf("upd.available_fixes", &[&r.version, &f.to_string()]),
        (a, f) => tf(
            "upd.available_notes",
            &[&r.version, &a.to_string(), &f.to_string()],
        ),
    };
    crate::update::clip(&text, 255)
}

/// Ukazkove vydanie s poznamkami (test `--test-update` a audity): ziadna siet.
pub fn sample() -> Release {
    Release {
        version: "0.3.0".into(),
        page: format!("https://github.com/{}/releases/tag/v0.3.0", crate::update::REPO),
        setup_name: "transom-setup-0.3.0-x64.exe".into(),
        setup_url: String::new(),
        sums_url: String::new(),
        notes: Notes {
            added: vec![
                "The update notification says what is new, and clicking it opens this panel.".into(),
                "A download icon at the right end of the bar while an update is available.".into(),
                "Weather in the island menu with an hourly forecast for the rest of the day and a much longer description that has to wrap onto several lines and is finally shortened with an ellipsis.".into(),
                "Clipboard history can pin items.".into(),
                "Quick notes sync their order.".into(),
                "Sound profiles switch with a hotkey.".into(),
            ],
            fixed: vec![
                "The calendar no longer skips a week in March.".into(),
                "Battery levels of sleeping mice stay visible.".into(),
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extras_tests::paint_kit;
    use transom_platform::menu_access::{ROLE_PUSHBUTTON, STATE_OFFSCREEN};

    const UNAVAILABLE: i32 = 0x1;

    fn views(r: &Release) -> Vec<(&'static str, UpdView<'_>)> {
        let base = UpdView {
            release: r,
            installed: "0.2.0",
            self_update: true,
            busy: false,
            error: None,
        };
        vec![
            ("ready", base.clone()),
            (
                "busy",
                UpdView {
                    busy: true,
                    ..base.clone()
                },
            ),
            (
                "error",
                UpdView {
                    error: Some(
                        "kontrolny sucet instalatora nesedi: a very long technical reason that goes on and on and on and on and on and on and on and on and on and on and on and on and on and on and on and on and on",
                    ),
                    ..base.clone()
                },
            ),
            (
                "portable",
                UpdView {
                    self_update: false,
                    ..base
                },
            ),
        ]
    }

    /// Panel v kazdom jazyku, mierke a teme: bez nalezov auditu (orezany text, kontrast) a cely obsah sa zmesti do
    /// vysky, ktoru panel dostane (bez posuvania); tlacidla su vzdy viditelne.
    #[test]
    fn the_update_panel_passes_the_audit_in_every_language_scale_and_theme() {
        let _g = crate::test_lang::guard();
        let full = sample();
        let empty = Release {
            notes: Notes::default(),
            ..sample()
        };
        let mut fails = Vec::new();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            for r in [&empty, &full] {
                // pripravena verzia s poznamkami posledna (`TRANSOM_PANEL_PNG` ulozi posledne kreslenie)
                for (name, v) in views(r).into_iter().rev() {
                    let rows = rows(&v);
                    let h_lp = height_lp(&v, crate::extras::KIT_W_LP);
                    for dpi in [96u32, 144, 192] {
                        for th in [
                            transom_render::Theme::dark(),
                            transom_render::Theme::light(),
                        ] {
                            let mut kit = Kit::default();
                            let mut issues = paint_kit(&mut kit, &rows, h_lp, dpi, &th);
                            // zakazane tlacidlo (pocas stahovania) je tlmene zamerne: WCAG 1.4.3 neaktivne ovladace
                            // z poziadavky na kontrast vynima; vsetko ostatne plati
                            if v.busy {
                                let label = format!("text: {:?}", t("upd.now"));
                                issues
                                    .retain(|i| !(i.starts_with("Contrast") && i.contains(&label)));
                            }
                            if !issues.is_empty() {
                                fails.push(format!("{lang:?} {name} {dpi}: {issues:?}"));
                            }
                            let h_px = (h_lp as f32 * dpi as f32 / 96.0).round() as i32;
                            if kit.content_h > h_px + 2 {
                                fails.push(format!(
                                    "{lang:?} {name} {dpi}: obsah {} > panel {h_px}",
                                    kit.content_h
                                ));
                            }
                            let (entries, extras, _, _) = kit.a11y();
                            for (e, x) in entries.iter().zip(&extras) {
                                if x.role == ROLE_PUSHBUTTON && x.state & STATE_OFFSCREEN != 0 {
                                    fails.push(format!("{lang:?} {name} {dpi}: {} mimo", e.name));
                                }
                            }
                        }
                    }
                }
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }

    #[test]
    fn the_panel_shows_new_and_fixes_with_more_and_the_right_buttons() {
        use crate::pop_kit::Cmd;
        use transom_platform::PointerKind;
        let _g = crate::test_lang::guard();
        let r = sample();
        let vs = views(&r);
        let paint = |v: &UpdView| {
            let mut kit = Kit::default();
            paint_kit(&mut kit, &rows(v), 600, 96, &transom_render::Theme::dark());
            let (entries, extras, _, root) = kit.a11y();
            let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
            let states: Vec<i32> = extras.iter().map(|x| x.state).collect();
            (kit, names, states, root)
        };
        let has = |n: &[String], w: &str| n.iter().any(|x| x == w);
        let (mut kit, n, st, root) = paint(&vs[0].1);
        assert_eq!(root, "transom 0.3.0 is available");
        for want in [
            "Installed: 0.2.0",
            "New",
            "…and 2 more",
            "Fixes",
            "The calendar no longer skips a week in March.",
            "Clipboard history can pin items.",
            "Update now",
            "Later",
            "Release notes",
        ] {
            assert!(has(&n, want), "{want}: {n:?}");
        }
        // 6 noviniek: 4 body a "…a 2 dalsie"; 2 opravy bez dalsich
        assert_eq!(n.iter().filter(|x| x.starts_with("…and")).count(), 1);
        assert!(!has(&n, "Quick notes sync their order."));
        let now = n.iter().position(|x| x == "Update now").unwrap();
        assert_eq!(st[now] & UNAVAILABLE, 0);
        for id in [UP_NOW, UP_LATER, UP_NOTES] {
            let b = kit.rect_of(id).unwrap();
            kit.pointer(PointerKind::Down, b.x + 3, b.y + 3);
            assert_eq!(
                kit.pointer(PointerKind::Click, b.x + 3, b.y + 3).0,
                Some(Cmd::Click(id))
            );
        }
        // stahovanie: hlavne tlacidlo zakazane, stav v paneli
        let (kit, n, st, _) = paint(&vs[1].1);
        assert!(has(&n, "Downloading and verifying…"));
        let now = n.iter().position(|x| x == "Update now").unwrap();
        assert_ne!(st[now] & UNAVAILABLE, 0);
        assert!(kit.rect_of(UP_NOW).is_none(), "zakazane tlacidlo nema ciel");
        // chyba v paneli (skratena)
        let (_, n, _, _) = paint(&vs[2].1);
        let err = n
            .iter()
            .find(|x| x.starts_with("Update failed: kontrolny"))
            .unwrap();
        assert!(err.chars().count() < 200 && err.ends_with('…'), "{err}");
        // prenosna kopia: stranka vydania namiesto instalacie, bez samostatnych poznamok
        let (_, n, _, _) = paint(&vs[3].1);
        assert!(has(&n, "Open release page") && !has(&n, "Update now"));
        assert!(!has(&n, "Release notes"));
        // bez poznamok
        let empty = Release {
            notes: Notes::default(),
            ..sample()
        };
        let (_, n, _, _) = paint(&UpdView {
            release: &empty,
            ..vs[0].1.clone()
        });
        assert!(has(&n, "No release notes were published for this version."));
        assert!(!has(&n, "New") && !has(&n, "Fixes"));
    }

    #[test]
    fn the_bar_icon_needs_an_update_the_check_and_the_icon_setting() {
        let r = sample();
        let cfg = |auto_check, show_icon| transom_config::Updates {
            auto_check,
            show_icon,
        };
        assert_eq!(icon_version(Some(&r), &cfg(true, true)), Some("0.3.0"));
        assert_eq!(icon_version(None, &cfg(true, true)), None);
        assert_eq!(icon_version(Some(&r), &cfg(false, true)), None);
        assert_eq!(icon_version(Some(&r), &cfg(true, false)), None);
        // vychodzie nastavenie: ikona sa ukaze
        assert!(icon_version(Some(&r), &transom_config::Updates::default()).is_some());
    }

    #[test]
    fn the_notice_says_what_is_new_and_fits_a_balloon_in_every_language() {
        let _g = crate::test_lang::guard();
        let mut r = sample();
        assert_eq!(
            notice_text(&r),
            "transom 0.3.0 is available (new: 6, fixes: 2) – click to see what’s new."
        );
        let mut only_new = r.clone();
        only_new.notes.fixed.clear();
        assert!(notice_text(&only_new).contains("(new: 6)"));
        let mut only_fixes = r.clone();
        only_fixes.notes.added.clear();
        assert!(notice_text(&only_fixes).contains("(fixes: 2)"));
        r.version = "1234567.1234567.1234567".into();
        let mut none = r.clone();
        none.notes = Notes::default();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            for rel in [&r, &only_new, &only_fixes, &none] {
                let s = notice_text(rel);
                assert!(s.encode_utf16().count() <= 255, "{lang:?}: {s}");
                assert!(s.contains(&rel.version), "{lang:?}: {s}");
            }
            assert!(transom_i18n::t("upd.title").encode_utf16().count() <= 63);
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }
}
