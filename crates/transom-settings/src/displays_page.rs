//! Stranka "Displays": na ktorych monitoroch je lista. Zoznam pripojenych monitorov (nazov, rozlisenie, mierka, primarny),
//! kazdy sa da zapnut/vypnut zvlast; tlacidla "Vsetky / Primarny / Vybrane" menia len **rezim** (ulozeny vyber ostava, aby
//! sa dal vratit). "Identifikovat" na chvilu ukaze cislo a nazov na kazdom monitore. Ulozene monitory, ktore teraz nie su
//! pripojene, sa ukazuju ako "nepripojene" (s poslednym popisom) a nikdy sa ticho nenahradia inym.
use crate::{style, Hit, Ui};
use transom_config::{DisplayRef, Displays, MAX_DISPLAY_REFS};
use transom_i18n::t;
use transom_platform::monitor::{missing_ids, MonitorInfo};
use transom_render::design::{self, Kind, Ty};
use transom_render::{HAlign, Painter, Rect, TextStyle};

/// Popis monitora na jeden riadok: "AOC 2402 · 2560 × 1440 · 100 %".
pub(crate) fn describe(m: &MonitorInfo) -> String {
    let name = if m.name.trim().is_empty() {
        m.device.trim_start_matches(r"\\.\").to_string()
    } else {
        m.name.clone()
    };
    format!(
        "{name} · {} × {} · {} %",
        m.rect.width(),
        m.rect.height(),
        (m.dpi * 100 + 48) / 96
    )
}

/// Je monitor `m` v rezime `d` zapnuty (lista na nom bude)? Pri `selected` bez zhody na ziadnom pripojenom monitore je lista
/// na primarnom ako zaloha (pozri `transom_platform::monitor::select_with_fallback`) - tu sa to nepocita ako "zapnuty".
pub(crate) fn enabled(d: &Displays, m: &MonitorInfo) -> bool {
    match d.mode.as_str() {
        "primary" => m.primary,
        "selected" => d.selected.iter().any(|r| r.id.eq_ignore_ascii_case(&m.id)),
        _ => true,
    }
}

/// Prepne monitor `idx`. Ak este nie je rezim `selected`, vyberie sa z aktualneho stavu (vsetky / primarny) a prepne sa
/// na `selected`. Odmietne (`false`) vypnutie **posledneho** zapnuteho pripojeneho monitora (lista by nikde nebola).
pub(crate) fn toggle(d: &mut Displays, monitors: &[MonitorInfo], idx: usize) -> bool {
    let Some(m) = monitors.get(idx) else {
        return false;
    };
    if d.mode != "selected" {
        // aktualny stav -> vyber (zachovaju sa aj nepripojene ulozene polozky)
        let keep: Vec<DisplayRef> = d
            .selected
            .iter()
            .filter(|r| !monitors.iter().any(|x| x.id.eq_ignore_ascii_case(&r.id)))
            .cloned()
            .collect();
        let mut sel: Vec<DisplayRef> = monitors
            .iter()
            .filter(|x| enabled(d, x))
            .map(|x| DisplayRef {
                id: x.id.clone(),
                label: describe(x),
            })
            .collect();
        sel.extend(keep);
        d.selected = sel;
        d.mode = "selected".into();
    }
    let on = d.selected.iter().any(|r| r.id.eq_ignore_ascii_case(&m.id));
    if on {
        let connected_on = monitors
            .iter()
            .filter(|x| d.selected.iter().any(|r| r.id.eq_ignore_ascii_case(&x.id)))
            .count();
        if connected_on <= 1 {
            return false;
        }
        d.selected.retain(|r| !r.id.eq_ignore_ascii_case(&m.id));
    } else if d.selected.len() < MAX_DISPLAY_REFS {
        d.selected.push(DisplayRef {
            id: m.id.clone(),
            label: describe(m),
        });
    }
    true
}

/// Rezim ("all" / "primary" / "selected"). Vyber sa nemaze; "selected" bez ulozeneho vyberu sa nastavi z pripojenych monitorov
/// (inak by nebola lista nikde).
pub(crate) fn set_mode(d: &mut Displays, monitors: &[MonitorInfo], mode: &str) {
    if mode == "selected" && d.selected.is_empty() {
        d.selected = monitors
            .iter()
            .map(|m| DisplayRef {
                id: m.id.clone(),
                label: describe(m),
            })
            .collect();
    }
    d.mode = mode.into();
}

/// Odstrani ulozeny (nepripojeny) monitor `id` zo vyberu; posledny nikdy (zostane aspon jeden zaznam).
pub(crate) fn remove_saved(d: &mut Displays, id: &str) -> bool {
    if d.selected.len() <= 1 {
        return false;
    }
    d.selected.retain(|r| !r.id.eq_ignore_ascii_case(id));
    true
}

/// Je vybrany rezim "selected" a ziadny vybrany monitor nie je pripojeny (lista je na primarnom ako zaloha)?
pub(crate) fn is_fallback(d: &Displays, monitors: &[MonitorInfo]) -> bool {
    d.mode == "selected"
        && !d.selected.is_empty()
        && d.selected
            .iter()
            .all(|r| missing_ids(monitors, std::slice::from_ref(&r.id)).len() == 1)
}

impl Ui {
    pub(crate) fn paint_displays(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "disp.title");
        y = self.notes(p, x0, y, w, &["disp.help"]) + px(12);
        y = self.card_begin(s, y);
        let d = self.cfg.displays.clone();
        let monitors = self.monitors.clone();
        // rezim: Vsetky / Primarny / Vybrane (menia len rezim; vyber ostava ulozeny)
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            px(crate::CHOICE_LABEL_LP),
            &t("disp.show_on"),
            vec![
                (t("disp.mode.all"), Hit::DispMode("all"), d.mode == "all"),
                (
                    t("disp.mode.primary"),
                    Hit::DispMode("primary"),
                    d.mode == "primary",
                ),
                (
                    t("disp.mode.selected"),
                    Hit::DispMode("selected"),
                    d.mode == "selected",
                ),
            ],
        );
        y += px(6);
        if is_fallback(&d, &monitors) {
            y = self.infobar(p, x0, y, w, &t("disp.fallback"));
        }
        // pripojene monitory: poradove cislo (rovnake ako pri "Identifikovat"), popis, primarny, zapnute
        let ordered = ordered(&monitors);
        for (n, (idx, m)) in ordered.iter().enumerate() {
            let row = Rect::new(x0, y, w, px(40));
            if self.hover == Some(Hit::DispToggle(*idx)) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            self.checkbox(
                p,
                Rect::new(row.x + px(8), row.y + px(10), px(20), px(20)),
                enabled(&d, m),
            );
            p.draw_text(
                &(n + 1).to_string(),
                Rect::new(row.x + px(40), row.y, px(24), row.h),
                &style(16.0, true, th.accent_text, HAlign::Left),
            );
            let mut text = describe(m);
            let mut tw = w - px(76);
            if m.primary {
                let badge = t("disp.primary");
                let bst = TextStyle {
                    bold: true,
                    ..Ty::Caption.style(th.foreground, HAlign::Center)
                };
                let bw = p.measure_text(&badge, &bst).w + px(16);
                tw -= bw + px(8);
                let r = Rect::new(row.x + row.w - bw - px(8), row.y + px(10), bw, px(20));
                p.stroke_round_rect(r, px(10), px(1).max(1), th.muted);
                p.draw_text(&badge, r, &bst);
            }
            // dlhy popis sa zalomi (nic sa neorezava): druhy riadok nizsie
            let lines = Self::wrap_lines(
                p,
                &text,
                &style(13.0, false, th.foreground, HAlign::Left),
                tw,
            );
            if lines.len() > 1 {
                text = lines.join(" ");
            }
            let st = style(13.0, false, th.foreground, HAlign::Left);
            let lh = p.measure_text("Ag", &st).h.max(px(18));
            for (k, l) in lines.iter().enumerate() {
                p.draw_text(
                    l,
                    Rect::new(
                        row.x + px(70),
                        row.y + px(11) - (lines.len() as i32 - 1) * lh / 2 + k as i32 * lh,
                        tw,
                        lh,
                    ),
                    &st,
                );
            }
            self.tag(
                &Hit::DispToggle(*idx),
                format!("{}. {text}", n + 1),
                Some(enabled(&d, m)),
            );
            self.hits.push((row, Hit::DispToggle(*idx)));
            y += px(44).max(lines.len() as i32 * lh + px(14));
        }
        // ulozene, ale teraz nepripojene monitory (s poslednym popisom): nikdy sa ticho nenahradia inym
        let missing: Vec<DisplayRef> = d
            .selected
            .iter()
            .filter(|r| missing_ids(&monitors, std::slice::from_ref(&r.id)).len() == 1)
            .cloned()
            .collect();
        for r in &missing {
            let label = if r.label.is_empty() {
                r.id.clone()
            } else {
                r.label.clone()
            };
            let text = format!("{label} · {}", t("disp.missing"));
            let rm = t("dat.remove");
            let rw = design::button_width(
                p,
                &design::Button::new(&rm, Kind::Secondary).icon('\u{E711}'),
            );
            let tw = w - rw - px(60);
            let mst = Ty::Secondary.style(th.muted, HAlign::Left);
            let lines = Self::wrap_lines(p, &text, &mst, tw);
            let lh = px(Ty::Secondary.line_lp());
            for (k, l) in lines.iter().enumerate() {
                p.draw_text(
                    l,
                    Rect::new(x0 + px(40), y + px(7) + k as i32 * lh, tw, lh),
                    &mst,
                );
            }
            let rr = Rect::new(x0 + w - rw, y, rw, px(design::CONTROL_H_LP));
            self.button(p, rr, &rm, Hit::DispRemove(r.id.clone()), false);
            y += (lines.len() as i32 * lh + px(14)).max(px(40));
        }
        y += px(8);
        let id = t("disp.identify");
        let iw = design::button_width(
            p,
            &design::Button::new(&id, Kind::Secondary).icon('\u{E7F4}'),
        );
        self.button(
            p,
            Rect::new(x0, y, iw.max(px(160)).min(w), px(design::CONTROL_H_LP)),
            &id,
            Hit::DispIdentify,
            false,
        );
        y += px(design::CONTROL_H_LP);
        self.card_end(s, x0, w, y);
    }
}

/// Monitory v poradi zobrazenia (primarny prvy, potom zlava doprava) s povodnym indexom: z toho cisla "1, 2, ..." na stranke aj v
/// identifikacnych oknach.
pub(crate) fn ordered(monitors: &[MonitorInfo]) -> Vec<(usize, &MonitorInfo)> {
    let mut v: Vec<(usize, &MonitorInfo)> = monitors.iter().enumerate().collect();
    v.sort_by_key(|(_, m)| (!m.primary, m.rect.left, m.rect.top));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_platform::monitor::Rect as MRect;

    fn mon(id: &str, left: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            device: format!("\\\\.\\DISPLAY{}", if primary { 1 } else { 2 }),
            id: id.into(),
            rect: MRect {
                left,
                top: 0,
                right: left + 2560,
                bottom: 1440,
            },
            dpi: 96,
            primary,
            name: if primary {
                "AOC 2402".into()
            } else {
                String::new()
            },
        }
    }

    fn two() -> Vec<MonitorInfo> {
        vec![mon("B", -1080, false), mon("A", 0, true)]
    }

    #[test]
    fn description_shows_name_resolution_and_scaling() {
        let a = mon("A", 0, true);
        assert_eq!(describe(&a), "AOC 2402 · 2560 × 1440 · 100 %");
        let mut b = mon("B", 0, false);
        b.dpi = 144;
        assert_eq!(
            describe(&b),
            "DISPLAY2 · 2560 × 1440 · 150 %",
            "bez nazvu sa pouzije cislo zariadenia"
        );
        b.dpi = 192;
        assert!(describe(&b).ends_with("200 %"));
        b.dpi = 120;
        assert!(describe(&b).ends_with("125 %"));
    }

    #[test]
    fn toggling_from_all_creates_a_selection_without_the_toggled_monitor_and_keeps_the_last_one() {
        let mon_list = two();
        let mut d = Displays::default();
        assert!(toggle(&mut d, &mon_list, 0), "B vypnut");
        assert_eq!(d.mode, "selected");
        assert_eq!(
            d.selected.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["A"]
        );
        // posledny zapnuty sa vypnut neda
        assert!(!toggle(&mut d, &mon_list, 1), "A je posledny");
        assert_eq!(d.selected.len(), 1);
        // zapnut spat
        assert!(toggle(&mut d, &mon_list, 0));
        assert_eq!(d.selected.len(), 2);
        assert!(
            d.selected.iter().all(|r| !r.label.is_empty()),
            "popis sa uklada pre pripad odpojenia"
        );
    }

    #[test]
    fn switching_modes_never_erases_the_saved_selection() {
        let mon_list = two();
        let mut d = Displays::default();
        toggle(&mut d, &mon_list, 0);
        let saved = d.selected.clone();
        set_mode(&mut d, &mon_list, "all");
        set_mode(&mut d, &mon_list, "primary");
        assert_eq!(d.selected, saved, "individualny vyber ostal");
        set_mode(&mut d, &mon_list, "selected");
        assert_eq!((d.mode.as_str(), d.selected.clone()), ("selected", saved));
        // "Vybrane" bez ulozeneho vyberu = vsetky pripojene (lista nesmie nikde chybat)
        let mut e = Displays::default();
        set_mode(&mut e, &mon_list, "selected");
        assert_eq!(e.selected.len(), 2);
        // z rezimu primarny: zapnutie druheho monitora = vyber {primarny, druhy}
        let mut p = Displays {
            mode: "primary".into(),
            selected: vec![],
        };
        assert!(toggle(&mut p, &mon_list, 0));
        assert_eq!(p.selected.len(), 2);
    }

    #[test]
    fn enabled_state_follows_the_mode() {
        let mon_list = two();
        let all = Displays::default();
        assert!(mon_list.iter().all(|m| enabled(&all, m)));
        let prim = Displays {
            mode: "primary".into(),
            selected: vec![],
        };
        assert_eq!(mon_list.iter().filter(|m| enabled(&prim, m)).count(), 1);
        assert!(enabled(&prim, &mon_list[1]));
        let sel = Displays {
            mode: "selected".into(),
            selected: vec![DisplayRef {
                id: "b".into(),
                label: String::new(),
            }],
        };
        assert!(
            enabled(&sel, &mon_list[0]) && !enabled(&sel, &mon_list[1]),
            "ID bez ohladu na velkost pismen"
        );
    }

    #[test]
    fn missing_saved_monitor_is_listed_not_replaced_and_fallback_is_reported() {
        let mon_list = two();
        let d = Displays {
            mode: "selected".into(),
            selected: vec![
                DisplayRef {
                    id: "gone".into(),
                    label: "Dell U2720Q".into(),
                },
                DisplayRef {
                    id: "A".into(),
                    label: String::new(),
                },
            ],
        };
        assert!(!is_fallback(&d, &mon_list), "jeden vybrany je pripojeny");
        let only_gone = Displays {
            mode: "selected".into(),
            selected: vec![DisplayRef {
                id: "gone".into(),
                label: "Dell U2720Q".into(),
            }],
        };
        assert!(
            is_fallback(&only_gone, &mon_list),
            "ziadny vybrany nie je pripojeny"
        );
        assert!(!is_fallback(&Displays::default(), &mon_list));
        assert!(
            !is_fallback(&only_gone, &[mon("gone", 0, true)]),
            "po opatovnom pripojeni uz nie"
        );
        // toggle neprepise nepripojeny zaznam
        let mut d2 = Displays {
            selected: only_gone.selected.clone(),
            ..Displays::default()
        };
        assert!(toggle(&mut d2, &mon_list, 0));
        assert!(
            d2.selected.iter().any(|r| r.id == "gone"),
            "nepripojeny zaznam ostava"
        );
        // odstranit sa da len vedome a nikdy posledny
        let mut d3 = d.clone();
        assert!(remove_saved(&mut d3, "gone"));
        assert!(!remove_saved(&mut d3, "A"));
    }

    #[test]
    fn display_numbers_follow_primary_first_then_left_to_right() {
        let m = two();
        let o = ordered(&m);
        assert_eq!(
            o.iter().map(|(_, m)| m.id.as_str()).collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(o[1].0, 0, "povodny index zostava pre ukladanie");
    }
}
