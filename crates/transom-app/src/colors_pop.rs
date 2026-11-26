//! Panel Farby (polozka / dlazdica Kapatko): vybrana farba (velka vzorka, HEX, RGB, HSL), tlacidlo kapatka a historia
//! vybranych farieb. Kapatko zavrie panel, pocka na kliknutie kdekolvek na obrazovke a panel otvori znova s novou
//! farbou (HEX je uz v schranke). Historia sa uklada len lokalne (`tools\colors\history.json`).
use crate::pop_kit::{Btn, Cmd, Row, Swatch};
use crate::{log, popup, snd, App};
use transom_core::item::Action;
use transom_i18n::{t, tf};
use transom_tools::colors::{hex, hsl, History, Rgb};

pub const CL_PICK: u32 = 1;
pub const CL_HEX: u32 = 2;
pub const CL_RGB: u32 = 3;
pub const CL_REMOVE: u32 = 4;
pub const CL_CLEAR: u32 = 5;
/// Vzorky historie: `CL_SWATCH + index`.
pub const CL_SWATCH: u32 = 100;

/// Vyska panela (lp).
pub const COLORS_H_LP: i32 = 440;

/// RGB a HSL farby (dva riadky).
pub fn detail(c: Rgb) -> String {
    let (h, s, l) = hsl(c);
    format!("RGB {}, {}, {}\nHSL {h}°, {s} %, {l} %", c.0, c.1, c.2)
}

/// Pristupny nazov farby.
fn acc(c: Rgb) -> String {
    tf(
        "col.acc",
        &[
            &hex(c),
            &c.0.to_string(),
            &c.1.to_string(),
            &c.2.to_string(),
        ],
    )
}

/// Obsah panela.
pub fn rows(history: &[Rgb], sel: Option<Rgb>) -> Vec<Row> {
    let mut rows = vec![Row::Title(t("col.title"))];
    if let Some(c) = sel {
        rows.push(Row::Color {
            color: c,
            title: hex(c),
            detail: detail(c),
            acc: acc(c),
        });
    }
    let mut btns = vec![Btn::new(CL_PICK, t("col.pick"))
        .icon(transom_render::scene::glyph_char(
            transom_core::item::Glyph::Picker,
        ))
        .primary()];
    if sel.is_some() {
        btns.push(Btn::new(CL_HEX, t("col.copy_hex")));
        btns.push(Btn::new(CL_RGB, t("col.copy_rgb")));
    }
    rows.push(Row::Buttons(btns));
    rows.push(Row::Text {
        text: t("pick.hint"),
        strong: false,
    });
    rows.push(Row::Section(t("col.history")));
    if history.is_empty() {
        rows.push(Row::Text {
            text: t("col.empty"),
            strong: false,
        });
    } else {
        rows.push(Row::Swatches(
            history
                .iter()
                .enumerate()
                .map(|(i, c)| Swatch {
                    id: CL_SWATCH + i as u32,
                    color: *c,
                    selected: sel == Some(*c),
                    acc: acc(*c),
                })
                .collect(),
        ));
        rows.push(Row::Text {
            text: t("col.hint"),
            strong: false,
        });
        let mut b = Vec::new();
        if sel.is_some_and(|c| history.contains(&c)) {
            b.push(Btn::new(CL_REMOVE, t("col.remove")));
        }
        b.push(Btn::new(CL_CLEAR, t("col.clear")));
        rows.push(Row::Buttons(b));
    }
    rows
}

/// Stav panela Farby v aplikacii.
#[derive(Default)]
pub struct ColorsState {
    /// Historia (nacita sa pri prvom otvoreni panela alebo prvom vybere).
    pub history: Option<History>,
    /// Vybrana farba (posledna z kapatka alebo kliknuta v historii).
    pub sel: Option<Rgb>,
    /// Kde bol panel otvoreny (znova sa otvori tam po vybere).
    pub anchor: Option<(String, i32)>,
    /// Kapatko spustene z panela: po vybere / zruseni sa panel otvori znova.
    pub reopen: bool,
}

impl ColorsState {
    pub fn history(&mut self) -> &mut History {
        self.history
            .get_or_insert_with(|| History::open(transom_tools::store::tools_dir().join("colors")))
    }

    pub fn list(&mut self) -> Vec<Rgb> {
        self.history().list().to_vec()
    }
}

/// Skopiruje text do schranky (oznaceny ako zapis transom, mimo historie schranky Windows).
fn copy(text: &str) -> bool {
    transom_platform::clipboard::write(&transom_platform::clipboard::Put::Text(text.to_string()))
}

impl App {
    /// Panel Farby sa otvara (`anchor` = kde na liste).
    pub(crate) fn colors_open(&mut self, anchor: Option<(String, i32)>) {
        self.kit.reset();
        if anchor.is_some() {
            self.colors.anchor = anchor;
        }
        if self.colors.sel.is_none() {
            self.colors.sel = self.colors.list().first().copied();
        }
    }

    pub(crate) fn colors_rows(&mut self) -> Vec<Row> {
        let list = self.colors.list();
        rows(&list, self.colors.sel)
    }

    pub(crate) fn colors_cmd(&mut self, cmd: Cmd) {
        let Cmd::Click(id) = cmd else { return };
        match id {
            CL_PICK => {
                // panel sa zavrie (inak by zakryval obrazovku); po vybere sa otvori znova
                popup::request(popup::Request::ClosePopover);
                self.colors.reopen = true;
                let _ = self.pick_start();
                return;
            }
            CL_HEX | CL_RGB => {
                if let Some(c) = self.colors.sel {
                    let text = if id == CL_HEX {
                        hex(c)
                    } else {
                        format!("{}, {}, {}", c.0, c.1, c.2)
                    };
                    let ok = copy(&text);
                    log(&format!(
                        "farby: kopirovanie {}",
                        if ok { "ok" } else { "zlyhalo" }
                    ));
                }
            }
            CL_REMOVE => {
                if let Some(c) = self.colors.sel.take() {
                    self.colors.history().remove(c);
                    self.colors.sel = self.colors.list().first().copied();
                    self.kit.reset();
                }
            }
            CL_CLEAR => {
                self.colors.history().clear();
                self.colors.sel = None;
                self.kit.reset();
                log("farby: historia vymazana");
            }
            id if id >= CL_SWATCH => {
                let list = self.colors.list();
                if let Some(c) = list.get((id - CL_SWATCH) as usize) {
                    self.colors.sel = Some(*c);
                    copy(&hex(*c));
                }
            }
            _ => {}
        }
        popup::invalidate_popover();
    }

    /// Kapatko: dalsie kliknutie na obrazovke precita farbu (nepostupi do aplikacie pod mysou).
    pub(crate) fn pick_start(&mut self) -> snd::Outcome {
        if !transom_platform::picker::start() {
            self.colors.reopen = false;
            self.show_notice(t("pick.failed"));
            return snd::Outcome::Failed;
        }
        log("kapatko: spustene");
        snd::Outcome::Done
    }

    /// Vysledok kapatka: HEX do schranky, farba do historie a panel Farby sa otvori znova s nou.
    pub(crate) fn pick_done(&mut self, c: Option<Rgb>) {
        match c {
            Some(c) => {
                log("kapatko: farba vybrana");
                copy(&hex(c));
                self.colors.history().push(c);
                self.colors.sel = Some(c);
            }
            None => log("kapatko: zrusene"),
        }
        if std::mem::take(&mut self.colors.reopen) || c.is_some() {
            let at = self.colors.anchor.clone();
            let _ = self.act(Action::ColorPick, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_show_the_selection_the_picker_button_and_the_history() {
        let _g = crate::test_lang::guard();
        let r = rows(&[], None);
        assert!(r
            .iter()
            .any(|x| matches!(x, Row::Buttons(b) if b[0].id == CL_PICK)));
        assert!(!r
            .iter()
            .any(|x| matches!(x, Row::Color { .. } | Row::Swatches(_))));
        let h = [(1, 2, 3), (0x24, 0x1B, 0x3A)];
        let r = rows(&h, Some((0x24, 0x1B, 0x3A)));
        let Some(Row::Color { title, detail, .. }) =
            r.iter().find(|x| matches!(x, Row::Color { .. }))
        else {
            panic!("vybrana farba");
        };
        assert_eq!(title, "#241B3A");
        assert!(
            detail.contains("RGB 36, 27, 58") && detail.contains("HSL 257°"),
            "{detail}"
        );
        let Some(Row::Swatches(s)) = r.iter().find(|x| matches!(x, Row::Swatches(_))) else {
            panic!("historia");
        };
        assert_eq!(s.len(), 2);
        assert!(s[1].selected && !s[0].selected);
        assert_eq!(s[1].id, CL_SWATCH + 1);
    }

    /// Panel Farby nepriehladny (obe temy) a na skle vo vlastnej farbe listy (modra, biela, fialova) pri 150 %:
    /// `TRANSOM_GALLERY=<adresar> cargo test -p transom-app gallery_colors -- --ignored`.
    #[test]
    #[ignore]
    fn gallery_colors() {
        let _g = crate::test_lang::guard();
        let Some(dir) = crate::gallery::dir() else {
            return;
        };
        let dpi = 144;
        let (w, h) = (
            crate::gallery::px(crate::extras::KIT_W_LP, dpi),
            crate::gallery::px(COLORS_H_LP, dpi),
        );
        let hist = [
            (0x24, 0x1B, 0x3A),
            (0, 112, 224),
            (255, 255, 255),
            (220, 40, 40),
            (30, 160, 80),
            (250, 200, 0),
            (18, 18, 18),
            (120, 40, 200),
            (0, 180, 200),
            (240, 120, 30),
        ];
        let sel = Some((0, 112, 224));
        for (dark, th) in crate::gallery::themes() {
            let surf = transom_render::offscreen::Surface::new(w, h);
            let mut p =
                transom_render::GdiPainter::new(surf.dc, transom_render::Size { w, h }, dpi);
            let mut kit = crate::pop_kit::Kit::default();
            kit.paint(&mut p, &th, &rows(&hist, sel));
            crate::gallery::save(
                &dir,
                &format!("colors-{}", crate::gallery::tag(dark)),
                &surf,
            );
        }
        for (name, bar) in [
            ("blue", (0u8, 112u8, 224u8)),
            ("white", (255, 255, 255)),
            ("purple", (0x24, 0x1B, 0x3A)),
        ] {
            let t = transom_platform::glass::tint_for(bar, 224);
            let th = if t.dark {
                transom_render::Theme::dark()
            } else {
                transom_render::Theme::light()
            };
            let mut kit = crate::pop_kit::Kit::default();
            let r = rows(&hist, sel);
            let (fw, rgba) = crate::gallery::glass_rgba_tinted(
                w,
                h,
                dpi,
                &th,
                transom_platform::glass::Rim::All,
                8,
                0,
                Some(t),
                &mut |p, t| kit.paint(p, t, &r),
            );
            crate::gallery::save_rgba(&dir, &format!("colors-glass-{name}"), fw, h, &rgba);
        }
    }
}
