//! Stranka "Periferie": co sa rozpoznava (automaticky, bez vyberu), pocet zariadeni v liste a interval citania. Stranka nic necita zo
//! zariadeni; len upravuje sekciu `peripherals` konfiguracie (lista si zmenu prevezme a sluzbu spusti znova).
use super::*;

impl Ui {
    pub(crate) fn paint_periph_page(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "pp.title");
        y = self.notes(p, x0, y, w, &["pp.help1", "pp.help2"]);
        y += px(12);

        // zdroje sa nevyberaju: transom rozpozna zariadenia sam (Bluetooth, standardna HID bateria, prijimace vyrobcov)
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("pp.auto_h"));
        y = self.notes(p, x0, y, w, &["pp.auto"]);
        y = self.card_end(s, x0, w, y);

        // lista: pocet zariadeni a interval
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("pp.bar_h"));
        let lw = px(crate::CHOICE_LABEL_LP);
        let cur = self.cfg.peripherals.on_bar;
        let opts: Vec<Choice> = transom_config::PERIPHERAL_BAR_COUNTS
            .iter()
            .map(|&n| {
                (
                    if n == 0 { t("pp.all") } else { n.to_string() },
                    Hit::PeriphCount(n),
                    cur == n,
                )
            })
            .collect();
        y += self.choice_row(p, Rect::new(x0, y, w, 0), lw, &t("pp.count"), opts);
        let cur = self.cfg.peripherals.interval_secs;
        let opts: Vec<Choice> = transom_config::PERIPHERAL_INTERVALS
            .iter()
            .map(|&v| {
                (
                    if v < 60 {
                        tf("pp.secs", &[&v.to_string()])
                    } else {
                        tf("pp.mins", &[&(v / 60).to_string()])
                    },
                    Hit::PeriphInterval(v),
                    cur == v,
                )
            })
            .collect();
        y += self.choice_row(p, Rect::new(x0, y, w, 0), lw, &t("pp.interval"), opts);
        y += px(4);
        y = self.notes(p, x0, y, w, &["pp.note"]);
        self.card_end(s, x0, w, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_tests::Env;
    use transom_config::load;

    #[test]
    fn count_and_interval_persist_and_change_nothing_else() {
        let e = Env::new("periph-page");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Peripherals));
        ui.click(Hit::PeriphCount(0));
        ui.click(Hit::PeriphInterval(300));
        let c = load(&e.root.join("config.json")).config;
        assert_eq!(
            (c.peripherals.on_bar, c.peripherals.interval_secs),
            (0, 300)
        );
        let d = transom_config::Config::default();
        assert_eq!(
            (c.layout, c.audio_panel, c.appearance),
            (d.layout, d.audio_panel, d.appearance)
        );
    }

    #[test]
    fn the_page_has_named_controls_and_no_vendor_choices_in_every_language() {
        use crate::page_a11y::Target;
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
            ui.page = Page::Peripherals;
            crate::a11y_tests::paint(&mut ui, 96);
            let (_, _, targets, _) = ui.a11y_snapshot(&[], None);
            for n in transom_config::PERIPHERAL_BAR_COUNTS {
                assert!(
                    targets.contains(&Target::Hit(Hit::PeriphCount(n))),
                    "{lang:?} {n}"
                );
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }
}
