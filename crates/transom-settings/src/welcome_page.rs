//! Stranka "Vitajte": zakladne nastavenie na jednom mieste. Lista ju otvori pri prvom spusteni (este neexistuje subor
//! nastaveni); neskor je v kategorii System. Spustenie s Windows, ostrov, sklo, AI (Claude Code, Codex), pocasie
//! a aktualizacie - vsetko ostatne (a podrobnosti) je na svojich strankach.
use super::*;

impl Ui {
    fn bar_item_visible(&self, id: &str) -> bool {
        self.cfg
            .layout
            .items
            .iter()
            .any(|s| s.id == id && s.visible)
    }

    /// Volba stranky Vitajte: (id, hodnota) -> konfiguracia (alebo spustenie s Windows v registri).
    pub(crate) fn welcome_set(&mut self, id: &str, v: bool) -> bool {
        match id {
            "autostart" => {
                if transom_platform::autostart::set(v) {
                    self.autostart = v;
                } else {
                    self.status = t("wel.autostart_failed");
                }
                // nie je to zmena konfiguracie
                return false;
            }
            "island" => {
                if notch_page::get(&self.cfg, "island") != v {
                    notch_page::toggle(&mut self.cfg, "island");
                }
            }
            "glass_panels" => {
                self.cfg.appearance.glass_panels = v;
                self.theme = window_theme(v);
                self.art = None;
            }
            "glass_bar" => self.cfg.appearance.glass_bar = v,
            "ai" | "weather" => self.cfg.layout.set_visible(id, v),
            "claude" => self.cfg.usage.claude_local = v,
            "codex" => self.cfg.usage.codex_local = v,
            _ => return false,
        }
        true
    }

    pub(crate) fn paint_welcome(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        self.acc_statics.clear();
        let mut y = self.title(p, x0, y, w, "wel.title");
        y = self.paragraph(p, x0, y, w, &t("wel.intro"), 12.0, th.muted) + px(12);
        let lw = px(CHOICE_LABEL_LP);
        let yes_no = |id: &'static str, on: bool| -> Vec<Choice> {
            vec![
                (t("opt.yes"), Hit::Wel(id, true), on),
                (t("opt.no"), Hit::Wel(id, false), !on),
            ]
        };

        // spustenie
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("wel.start_h"));
        let on = self.autostart;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.autostart"),
            yes_no("autostart", on),
        );
        y = self.paragraph(
            p,
            x0,
            y + px(2),
            w,
            &t("wel.autostart_note"),
            12.0,
            th.muted,
        );
        y = self.card_end(s, x0, w, y);

        // vzhlad
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("wel.look_h"));
        let island = notch_page::get(&self.cfg, "island");
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.island"),
            yes_no("island", island),
        );
        y = self.paragraph(p, x0, y + px(2), w, &t("wel.island_note"), 12.0, th.muted) + px(8);
        let gp = self.cfg.appearance.glass_panels;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.glass_panels"),
            yes_no("glass_panels", gp),
        );
        let gb = self.cfg.appearance.glass_bar;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.glass_bar"),
            yes_no("glass_bar", gb),
        );
        y = self.card_end(s, x0, w, y);

        // AI
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("wel.ai_h"));
        y = self.paragraph(p, x0, y, w, &t("wel.ai_note"), 12.0, th.muted) + px(8);
        let ai = self.bar_item_visible("ai");
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.ai_bar"),
            yes_no("ai", ai),
        );
        let cl = self.cfg.usage.claude_local;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.claude"),
            yes_no("claude", cl),
        );
        let cx = self.cfg.usage.codex_local;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.codex"),
            yes_no("codex", cx),
        );
        y = self.paragraph(p, x0, y + px(2), w, &t("wel.cc_note"), 12.0, th.muted) + px(8);
        y = self.flow_buttons(
            p,
            x0,
            y,
            w,
            vec![(t("wel.cc_setup"), Hit::Nav(Page::Ai), false)],
        );
        y = self.card_end(s, x0, w, y);

        // pocasie
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("wel.weather_h"));
        let wx = self.bar_item_visible("weather");
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("wel.weather"),
            yes_no("weather", wx),
        );
        y = self.paragraph(p, x0, y + px(2), w, &t("wel.weather_note"), 12.0, th.muted) + px(8);
        y = self.flow_buttons(
            p,
            x0,
            y,
            w,
            vec![(t("wel.weather_pick"), Hit::Nav(Page::Extras), false)],
        );
        y = self.card_end(s, x0, w, y);

        // aktualizacie
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("wel.updates_h"));
        let up = self.cfg.updates.auto_check;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("upd.auto"),
            vec![
                (t("opt.yes"), Hit::UpdAuto(true), up),
                (t("opt.no"), Hit::UpdAuto(false), !up),
            ],
        );
        y = self.card_end(s, x0, w, y);

        // hotovo
        y = self.paragraph(p, x0, y, w, &t("wel.later"), 12.0, th.muted) + px(10);
        self.flow_buttons(p, x0, y, w, vec![(t("wel.done"), Hit::WelDone, false)]);
    }
}
