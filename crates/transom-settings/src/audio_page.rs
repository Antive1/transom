//! Stranka "Panel zvuku": vyber rychlych akcii v paneli Zvuk (polozka `audio` lisaty). Rychle akcie len **otvaraju**
//! natívny povrch Windows (alebo Nastavenia transom); transom nepotvrdzuje, co Windows urobi potom. Hlasitost v paneli
//! je systemova hlasitost Windows a je oddelena od zvukov rozhrania transom (stranka Zvuky rozhrania).
use super::*;

/// Rychle akcie v kanonickom poradi: (id v konfiguracii, kluc nazvu).
pub(crate) const QUICK: [(&str, &str); 5] = [
    ("quick_settings", "qa.quick_settings"),
    ("notifications", "qa.notifications"),
    ("search", "qa.search"),
    ("tray", "qa.tray"),
    ("settings", "qa.settings"),
];

impl Ui {
    pub(crate) fn paint_audio_page(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "aud.title");
        y = self.notes(p, x0, y, w, &["aud.help1", "aud.help2"]);
        y += px(12);
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("aud.qa_h"));
        for (id, key) in QUICK {
            let on = self.cfg.audio_panel.has(id);
            let row = Rect::new(x0, y, w, px(28));
            let hit = Hit::AudioQuick(id);
            if self.hover.as_ref() == Some(&hit) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            self.checkbox(
                p,
                Rect::new(row.x + px(4), row.y + px(4), px(20), px(20)),
                on,
            );
            p.draw_text(
                &t(key),
                Rect::new(row.x + px(34), row.y, row.w - px(34), row.h),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            self.tag(&hit, t(key), Some(on));
            self.hits.push((row, hit));
            y += px(32);
        }
        y += px(6);
        y = self.notes(p, x0, y, w, &["aud.qa_help"]);
        self.card_end(s, x0, w, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_tests::Env;
    use transom_config::load;

    #[test]
    fn toggling_a_quick_action_persists_in_canonical_order_and_changes_nothing_else() {
        let e = Env::new("aud-toggle");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Audio));
        ui.click(Hit::AudioQuick("tray"));
        ui.click(Hit::AudioQuick("notifications"));
        let s = load(&e.root.join("config.json")).config;
        assert_eq!(
            s.audio_panel.quick_actions,
            vec!["quick_settings", "search", "settings"]
        );
        ui.click(Hit::AudioQuick("notifications"));
        let s = load(&e.root.join("config.json")).config;
        assert_eq!(
            s.audio_panel.quick_actions,
            vec!["quick_settings", "notifications", "search", "settings"]
        );
        let d = transom_config::Config::default();
        assert_eq!(
            (s.usage, s.ui_sounds, s.appearance),
            (d.usage, d.ui_sounds, d.appearance)
        );
        // vypnut vsetky = riadok sa v paneli nezobrazi
        for (id, _) in QUICK {
            if ui.cfg.audio_panel.has(id) {
                ui.click(Hit::AudioQuick(id));
            }
        }
        assert!(load(&e.root.join("config.json"))
            .config
            .audio_panel
            .quick_actions
            .is_empty());
    }

    #[test]
    fn the_page_has_named_checkboxes_with_state_for_screen_readers_in_every_language() {
        use crate::page_a11y::Target;
        use transom_platform::menu_access::{ROLE_CHECKBUTTON, STATE_CHECKED};
        let _g = crate::archive_tests::ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            let loaded = transom_config::Loaded {
                config: transom_config::Config::default(),
                read_only: false,
                note: None,
                issue: None,
            };
            let mut ui = make_ui(loaded, Theme::dark(), String::new());
            ui.page = Page::Audio;
            ui.cfg.audio_panel.set("tray", false);
            crate::a11y_tests::paint(&mut ui, 96);
            let (entries, extras, targets, _) = ui.a11y_snapshot(&[], None);
            for (id, key) in QUICK {
                let i = targets
                    .iter()
                    .position(|t| *t == Target::Hit(Hit::AudioQuick(id)))
                    .unwrap_or_else(|| panic!("{lang:?} {id}"));
                assert_eq!(extras[i].role, ROLE_CHECKBUTTON);
                assert_eq!(entries[i].name, t(key));
                assert_eq!(
                    extras[i].state & STATE_CHECKED != 0,
                    id != "tray",
                    "{lang:?} {id}"
                );
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }

    #[test]
    fn the_settings_list_matches_the_config_ids_and_the_panel() {
        let ids: Vec<&str> = QUICK.iter().map(|(i, _)| *i).collect();
        assert_eq!(ids, transom_config::QUICK_ACTIONS.to_vec());
    }
}
