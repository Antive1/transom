//! Nastavenia pre jeden monitor (stranky Layout a Appearance): vyber "Vsetky monitory / 1 / 2 ..." a prepinac vlastnych
//! nastaveni vybraneho monitora.
//!
//! Stranky upravuju vzdy `cfg.layout` a `cfg.appearance`. Ked je vybrany monitor s vlastnym rozlozenim (vzhladom), jeho
//! hodnota sa s nimi v pamati **vymeni** (`reswap`), takze vsetok kod stranok upravuje prave ju. Pred ulozenim sa vymena
//! vrati (`unswap`), na disk ide vzdy normalny tvar konfiguracie.
use crate::{Hit, Page, Ui};
use transom_config::MonitorOverride;
use transom_i18n::t;
use transom_platform::monitor::MonitorInfo;
use transom_render::design::{self, Kind};
use transom_render::{Painter, Rect};

/// Cast nastaveni, ktoru moze mat monitor vlastnu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Part {
    Layout,
    Look,
}

impl Part {
    fn index(self) -> usize {
        match self {
            Part::Layout => 0,
            Part::Look => 1,
        }
    }
}

/// Kratky nazov monitora pre tlacidlo vyberu ("AOC 2402", bez nazvu cislo zariadenia).
pub(crate) fn short_name(m: &MonitorInfo) -> String {
    if m.name.trim().is_empty() {
        m.device.trim_start_matches(r"\\.\").to_string()
    } else {
        m.name.trim().to_string()
    }
}

impl Ui {
    /// Ukazat vyber monitora? Pri viacerych pripojenych monitoroch, alebo ked uz niektory ma vlastne nastavenia.
    pub(crate) fn scope_shown(&self) -> bool {
        self.monitors.len() > 1 || !self.cfg.monitors.is_empty()
    }

    /// Cast nastaveni, ktoru upravuje aktualna stranka.
    pub(crate) fn scope_part(&self) -> Part {
        if self.page == Page::Appearance {
            Part::Look
        } else {
            Part::Layout
        }
    }

    /// Vymeni cast `part` vybraneho monitora so spolocnou (ak ju monitor ma). Vrati, ci sa vymenila.
    fn swap_part(&mut self, part: Part) -> bool {
        let Some(id) = self.scope.clone() else {
            return false;
        };
        let Some(m) = self
            .cfg
            .monitors
            .iter_mut()
            .find(|m| m.id.eq_ignore_ascii_case(&id))
        else {
            return false;
        };
        match part {
            Part::Layout => match m.layout.as_mut() {
                Some(l) => {
                    std::mem::swap(l, &mut self.cfg.layout);
                    true
                }
                None => false,
            },
            Part::Look => match m.appearance.as_mut() {
                Some(a) => {
                    std::mem::swap(a, &mut self.cfg.appearance);
                    true
                }
                None => false,
            },
        }
    }

    /// Vrati vymenu: `cfg` je v normalnom (ulozenom) tvare.
    pub(crate) fn unswap(&mut self) {
        for part in [Part::Layout, Part::Look] {
            if self.swapped[part.index()] {
                self.swap_part(part);
                self.swapped[part.index()] = false;
            }
        }
    }

    /// Vlastne nastavenia vybraneho monitora do `cfg.layout` / `cfg.appearance` (ak ich ma).
    pub(crate) fn reswap(&mut self) {
        for part in [Part::Layout, Part::Look] {
            if !self.swapped[part.index()] {
                self.swapped[part.index()] = self.swap_part(part);
            }
        }
    }

    /// Prepne upravovany monitor (`None` = spolocne nastavenia vsetkych monitorov).
    pub(crate) fn set_scope(&mut self, id: Option<String>) {
        self.unswap();
        self.scope = id;
        self.reswap();
    }

    /// Ma vybrany monitor vlastnu cast `part`?
    pub(crate) fn scope_own(&self, part: Part) -> bool {
        let Some(id) = &self.scope else {
            return false;
        };
        self.cfg.monitor(id).is_some_and(|m| match part {
            Part::Layout => m.layout.is_some(),
            Part::Look => m.appearance.is_some(),
        })
    }

    /// Zapne (kopia spolocnych) alebo zrusi vlastnu cast `part` vybraneho monitora.
    pub(crate) fn set_own(&mut self, part: Part, own: bool) {
        let Some(id) = self.scope.clone() else {
            return;
        };
        self.unswap();
        let pos = self
            .cfg
            .monitors
            .iter()
            .position(|m| m.id.eq_ignore_ascii_case(&id));
        if own {
            let label = self
                .monitors
                .iter()
                .find(|m| m.id.eq_ignore_ascii_case(&id))
                .map(crate::displays_page::describe)
                .unwrap_or_default();
            let i = pos.unwrap_or_else(|| {
                self.cfg.monitors.push(MonitorOverride {
                    id: id.clone(),
                    label: String::new(),
                    layout: None,
                    appearance: None,
                });
                self.cfg.monitors.len() - 1
            });
            let (layout, look) = (self.cfg.layout.clone(), self.cfg.appearance.clone());
            let m = &mut self.cfg.monitors[i];
            if !label.is_empty() {
                m.label = label;
            }
            match part {
                Part::Layout => m.layout = Some(layout),
                Part::Look => m.appearance = Some(look),
            }
        } else if let Some(i) = pos {
            let m = &mut self.cfg.monitors[i];
            match part {
                Part::Layout => m.layout = None,
                Part::Look => m.appearance = None,
            }
            if m.layout.is_none() && m.appearance.is_none() {
                self.cfg.monitors.remove(i);
            }
        }
        self.reswap();
    }

    /// Vybrany monitor uz nie je pripojeny, alebo stranka nema vyber: spat na spolocne nastavenia.
    pub(crate) fn check_scope(&mut self) {
        let Some(id) = &self.scope else {
            return;
        };
        let connected = self.monitors.iter().any(|m| m.id.eq_ignore_ascii_case(id));
        if !connected || !matches!(self.page, Page::Layout | Page::Appearance) {
            self.set_scope(None);
        }
    }

    /// Karta vyberu monitora na vrchu stranky; vrati y pod kartou (s medzerou). Bez vyberu nic nekresli.
    pub(crate) fn paint_scope(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        if !self.scope_shown() {
            return y;
        }
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let th = self.theme;
        let mut y = self.card_begin(s, y);
        let mut opts = vec![(t("scope.all"), Hit::Scope(None), self.scope.is_none())];
        let monitors = self.monitors.clone();
        for (n, (_, m)) in crate::displays_page::ordered(&monitors).iter().enumerate() {
            let on = self
                .scope
                .as_ref()
                .is_some_and(|id| id.eq_ignore_ascii_case(&m.id));
            opts.push((
                format!("{} · {}", n + 1, short_name(m)),
                Hit::Scope(Some(m.id.clone())),
                on,
            ));
        }
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            px(crate::CHOICE_LABEL_LP),
            &t("scope.label"),
            opts,
        );
        let part = self.scope_part();
        let (note, btn, own) = match (&self.scope, self.scope_own(part)) {
            (None, _) => (t("scope.all_note"), None, false),
            (Some(_), true) => (t("scope.own_note"), Some(t("scope.use_shared")), false),
            (Some(_), false) => (t("scope.shared_note"), Some(t("scope.make_own")), true),
        };
        y = self.paragraph(p, x0, y + px(4), w, &note, 12.0, th.muted);
        if let Some(b) = btn {
            y += px(10);
            let bw = design::button_width(p, &design::Button::new(&b, Kind::Secondary));
            self.button(
                p,
                Rect::new(x0, y, bw.max(px(160)).min(w), px(design::CONTROL_H_LP)),
                &b,
                Hit::ScopeOwn(own),
                false,
            );
            y += px(design::CONTROL_H_LP);
        }
        self.card_end(s, x0, w, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_config::{Appearance, Config};

    fn ui() -> Ui {
        let loaded = transom_config::Loaded {
            config: Config::default(),
            read_only: false,
            note: None,
            issue: None,
        };
        crate::make_ui(loaded, transom_render::Theme::dark(), String::new())
    }

    #[test]
    fn own_settings_are_a_copy_edited_through_the_swap_and_saved_in_normal_form() {
        let mut ui = ui();
        ui.page = Page::Appearance;
        ui.cfg.appearance.bg_color = "#112233".into();
        ui.set_scope(Some("MON-B".into()));
        assert!(!ui.scope_own(Part::Look));
        ui.set_own(Part::Look, true);
        assert!(ui.scope_own(Part::Look));
        // uprava ide do vlastneho vzhladu monitora, spolocny ostava
        ui.cfg.appearance.bg_color = "#FFFFFF".into();
        ui.unswap();
        assert_eq!(ui.cfg.appearance.bg_color, "#112233");
        assert_eq!(
            ui.cfg
                .monitor("mon-b")
                .and_then(|m| m.appearance.as_ref())
                .map(|a| a.bg_color.as_str()),
            Some("#FFFFFF")
        );
        ui.reswap();
        assert_eq!(ui.cfg.appearance.bg_color, "#FFFFFF");
        // spat na spolocne: zaznam monitora zmizne, upravuje sa spolocny vzhlad
        ui.set_own(Part::Look, false);
        assert!(ui.cfg.monitors.is_empty());
        assert_eq!(ui.cfg.appearance.bg_color, "#112233");
        let _ = Appearance::default();
    }

    #[test]
    fn layout_and_look_are_independent_and_scope_change_restores_shared() {
        let mut ui = ui();
        ui.page = Page::Layout;
        ui.set_scope(Some("A".into()));
        ui.set_own(Part::Layout, true);
        let shared = ui.cfg.layout.clone();
        ui.cfg.layout.items.retain(|s| s.id != "clock");
        assert!(!ui.scope_own(Part::Look), "vzhlad ostal spolocny");
        ui.set_scope(None);
        assert_eq!(ui.cfg.layout, shared, "spolocne rozlozenie nezmenene");
        assert!(ui.cfg.layout_for("a").items.iter().all(|s| s.id != "clock"));
    }
}
