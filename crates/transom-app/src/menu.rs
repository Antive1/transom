//! Vlastne (nenativne) kontextove menu lisaty: cisty model (polozky, aktivna polozka, klavesnica), rozlozenie z nameranych
//! textov a kreslenie rovnakym rendererom a farbami ako zvysok transom. Ziadne Win32 - okno, zachytenie mysi, fokus a
//! pristupnost su v `transom_platform::popup`.
//!
//! Ovladanie: sipky hore/dole (cyklicky, preskakuju oddelovace a vypnute polozky), Home/End, Enter/medzernik = vybrat,
//! Tab = dalsia polozka, pismeno = skok na polozku s tym zaciatocnym pismenom, Esc = zavriet (vybavuje platforma),
//! mys: hover = aktivna polozka, klik = vybrat. Aktivna polozka je viditelna aj bez farby (zaoblene pozadie + svisla
//! akcentova znacka vlavo).
use transom_render::{Color, Font, HAlign, Painter, Rect, TextStyle, Theme};

const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_SPACE: u32 = 0x20;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;

/// Rozmery v lp.
const PAD: i32 = 6;
const ROW_H: i32 = 34;
const SEP_H: i32 = 11;
const MIN_W: i32 = 200;
const MAX_W: i32 = 380;
const TEXT_X: i32 = 18;
const TEXT_R: i32 = 18;
const FONT_LP: f32 = 13.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// 0 = oddelovac.
    pub id: u32,
    pub label: String,
    pub enabled: bool,
}

impl Item {
    pub fn is_separator(&self) -> bool {
        self.id == 0
    }
}

/// Vysledok stlacenia klavesy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// Klavesa nic nezmenila.
    None,
    /// Zmenila sa aktivna polozka (prekreslit).
    Redraw,
    /// Vybrana polozka `id`.
    Choose(u32),
}

#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub items: Vec<Item>,
    /// Aktivna (zvyraznena / s fokusom) polozka.
    pub active: Option<usize>,
    /// Stlacene tlacidlo mysi nad aktivnou polozkou.
    pub pressed: bool,
    /// Obdlzniky riadkov v px (rovnaky index ako `items`) a celkova velkost; plni `measure`.
    rows: Vec<Rect>,
    pub size: (i32, i32),
}

impl Menu {
    pub fn new(items: Vec<(u32, String)>) -> Menu {
        Menu {
            items: items
                .into_iter()
                .map(|(id, label)| Item {
                    id,
                    label,
                    enabled: true,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn selectable(&self, i: usize) -> bool {
        self.items
            .get(i)
            .is_some_and(|it| !it.is_separator() && it.enabled)
    }

    fn style(color: Color) -> TextStyle {
        TextStyle {
            font: Font::Ui,
            size_lp: FONT_LP,
            bold: false,
            color,
            align: HAlign::Left,
        }
    }

    /// Zmera texty a spocita rozlozenie pre mierku `p.scale()`; sirka = najdlhsi nazov (v hraniciach), vyska = riadky.
    pub fn measure(&mut self, p: &mut dyn Painter) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let st = Self::style(Color::rgb(0, 0, 0));
        let widest = self
            .items
            .iter()
            .filter(|i| !i.is_separator())
            .map(|i| p.measure_text(&i.label, &st).w)
            .max()
            .unwrap_or(0);
        let inner = (widest + px(TEXT_X + TEXT_R)).clamp(px(MIN_W - 2 * PAD), px(MAX_W - 2 * PAD));
        let w = inner + 2 * px(PAD);
        let mut y = px(PAD);
        self.rows.clear();
        for it in &self.items {
            let h = if it.is_separator() {
                px(SEP_H)
            } else {
                px(ROW_H)
            };
            self.rows.push(Rect::new(px(PAD), y, inner, h));
            y += h;
        }
        self.size = (w, y + px(PAD));
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[Rect] {
        &self.rows
    }

    /// Polozky s obdlznikmi pre citacky obrazovky (MSAA `MENUITEM` / `SEPARATOR`).
    pub fn entries(&self) -> Vec<transom_platform::popup::MenuEntry> {
        self.items
            .iter()
            .zip(&self.rows)
            .map(|(it, r)| transom_platform::popup::MenuEntry {
                name: it.label.clone(),
                separator: it.is_separator(),
                enabled: it.enabled,
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            })
            .collect()
    }

    /// Polozka pod bodom (len vyberatelna).
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.rows.iter().enumerate().find_map(|(i, r)| {
            (self.selectable(i) && x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
                .then_some(i)
        })
    }

    /// Nastavi aktivnu polozku podla mysi (`None` = mimo); vrati, ci treba prekreslit.
    pub fn hover(&mut self, i: Option<usize>) -> bool {
        let changed = self.active != i;
        self.active = i;
        changed
    }

    fn step(&self, from: Option<usize>, dir: i32) -> Option<usize> {
        let n = self.items.len() as i32;
        if n == 0 {
            return None;
        }
        let mut i = match (from, dir) {
            (Some(i), _) => i as i32,
            (None, d) if d > 0 => -1,
            (None, _) => n,
        };
        for _ in 0..n {
            i = (i + dir).rem_euclid(n);
            if self.selectable(i as usize) {
                return Some(i as usize);
            }
        }
        None
    }

    fn edge(&self, last: bool) -> Option<usize> {
        let range: Vec<usize> = (0..self.items.len()).collect();
        let mut it = range.into_iter().filter(|&i| self.selectable(i));
        if last {
            it.next_back()
        } else {
            it.next()
        }
    }

    /// Klavesnica. Esc sa tu nespracuje (zatvara platforma).
    pub fn key(&mut self, vk: u32) -> Nav {
        let to = |m: &mut Menu, i: Option<usize>| {
            if i.is_some() && i != m.active {
                m.active = i;
                Nav::Redraw
            } else {
                Nav::None
            }
        };
        match vk {
            VK_DOWN | VK_TAB => {
                let n = self.step(self.active, 1);
                to(self, n)
            }
            VK_UP => {
                let n = self.step(self.active, -1);
                to(self, n)
            }
            VK_HOME => {
                let n = self.edge(false);
                to(self, n)
            }
            VK_END => {
                let n = self.edge(true);
                to(self, n)
            }
            VK_RETURN | VK_SPACE => match self.active {
                Some(i) if self.selectable(i) => Nav::Choose(self.items[i].id),
                _ => Nav::None,
            },
            // pismeno: dalsia vyberatelna polozka, ktorej nazov zacina na to pismeno (od aktivnej dalej, cyklicky)
            0x41..=0x5A => {
                let ch = char::from_u32(vk).unwrap_or(' ');
                let n = self.items.len();
                let start = self.active.map_or(0, |a| a + 1);
                let found = (0..n).map(|k| (start + k) % n.max(1)).find(|&i| {
                    self.selectable(i)
                        && self.items[i]
                            .label
                            .chars()
                            .next()
                            .is_some_and(|c| c.to_uppercase().next() == Some(ch))
                });
                to(self, found)
            }
            _ => Nav::None,
        }
    }

    /// Kliknutie mysou na polozku `i` (po uvolneni nad tou istou, nad ktorou bolo stlacenie).
    pub fn choose(&self, i: usize) -> Option<u32> {
        self.selectable(i).then(|| self.items[i].id)
    }

    /// Kresli cele menu do povrchu (okno ma velkost `size`).
    pub fn paint(&self, p: &mut dyn Painter, th: &Theme) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        let whole = Rect::new(0, 0, sz.w, sz.h);
        // jemny okraj (vlas) a podklad; rohy zaobluje DWM (Windows 11)
        p.fill_rect(whole, th.edge);
        p.fill_rect(
            Rect::new(1, 1, (sz.w - 2).max(0), (sz.h - 2).max(0)),
            th.background,
        );
        for (i, (it, r)) in self.items.iter().zip(&self.rows).enumerate() {
            if it.is_separator() {
                p.fill_rect(
                    Rect::new(
                        r.x + px(8),
                        r.y + r.h / 2,
                        (r.w - px(16)).max(0),
                        1.max(px(1)),
                    ),
                    th.edge,
                );
                continue;
            }
            let active = self.active == Some(i) && it.enabled;
            if active {
                let bg = if self.pressed { th.pressed } else { th.hover };
                p.fill_round_rect(*r, px(transom_render::design::radius::CONTROL + 2), bg);
                // znacka fokusu: viditelna aj bez rozlisovania farieb
                p.fill_round_rect(
                    Rect::new(r.x + px(3), r.y + px(8), px(3), (r.h - px(16)).max(2)),
                    px(2),
                    th.accent,
                );
            }
            let color = if it.enabled {
                th.foreground
            } else {
                th.disabled
            };
            p.draw_text(
                &it.label,
                Rect::new(
                    r.x + px(TEXT_X),
                    r.y,
                    (r.w - px(TEXT_X + TEXT_R)).max(0),
                    r.h,
                ),
                &Self::style(color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_render::Size;

    /// Mierka a sirka znaku nastavitelne: test rozlozenia pre rozne DPI.
    struct Fake {
        scale: f32,
        char_w: i32,
        size: Size,
    }
    impl Painter for Fake {
        fn size(&self) -> Size {
            self.size
        }
        fn scale(&self) -> f32 {
            self.scale
        }
        fn fill_rect(&mut self, _: Rect, _: Color) {}
        fn measure_text(&mut self, t: &str, _: &TextStyle) -> Size {
            Size {
                w: self.char_w * t.chars().count() as i32,
                h: 14,
            }
        }
        fn draw_text(&mut self, _: &str, _: Rect, _: &TextStyle) {}
        fn blit(&mut self, _: Rect, _: &[Color]) {}
    }
    fn fake(scale: f32) -> Fake {
        Fake {
            scale,
            char_w: (7.0 * scale) as i32,
            size: Size { w: 400, h: 400 },
        }
    }

    fn std_items() -> Menu {
        Menu::new(vec![
            (1, "Settings".into()),
            (3, "Customize bar".into()),
            (6, "AI usage".into()),
            (4, "Hide bar".into()),
            (0, String::new()),
            (9, "Quit transom".into()),
        ])
    }

    #[test]
    fn arrows_wrap_and_skip_separators_and_disabled_rows() {
        let mut m = std_items();
        m.items[3].enabled = false; // Hide bar vypnute
        assert_eq!(m.key(VK_DOWN), Nav::Redraw);
        assert_eq!(m.active, Some(0));
        m.key(VK_DOWN);
        m.key(VK_DOWN);
        assert_eq!(m.active, Some(2));
        m.key(VK_DOWN); // preskoci vypnute (3) aj oddelovac (4)
        assert_eq!(m.active, Some(5));
        m.key(VK_DOWN); // cyklicky na zaciatok
        assert_eq!(m.active, Some(0));
        m.key(VK_UP); // cyklicky na koniec
        assert_eq!(m.active, Some(5));
        m.key(VK_UP);
        assert_eq!(m.active, Some(2));
        assert_eq!(m.key(VK_HOME), Nav::Redraw);
        assert_eq!(m.active, Some(0));
        assert_eq!(m.key(VK_HOME), Nav::None, "uz je na prvej");
        assert_eq!(m.key(VK_END), Nav::Redraw);
        assert_eq!(m.active, Some(5));
    }

    #[test]
    fn up_from_nothing_selects_the_last_and_down_the_first() {
        let mut m = std_items();
        m.key(VK_UP);
        assert_eq!(m.active, Some(5));
        let mut m = std_items();
        m.key(VK_TAB);
        assert_eq!(m.active, Some(0));
    }

    #[test]
    fn enter_and_space_choose_only_an_active_enabled_item() {
        let mut m = std_items();
        assert_eq!(m.key(VK_RETURN), Nav::None, "bez aktivnej polozky nic");
        m.key(VK_DOWN);
        m.key(VK_DOWN);
        assert_eq!(m.key(VK_RETURN), Nav::Choose(3));
        assert_eq!(m.key(VK_SPACE), Nav::Choose(3));
        m.items[1].enabled = false;
        assert_eq!(m.key(VK_RETURN), Nav::None, "vypnuta polozka sa nevyberie");
        assert_eq!(m.choose(1), None);
        assert_eq!(m.choose(4), None, "oddelovac sa nevyberie");
        assert_eq!(m.choose(5), Some(9));
    }

    #[test]
    fn letters_jump_to_the_next_item_with_that_initial() {
        let mut m = std_items();
        assert_eq!(m.key(0x51), Nav::Redraw); // Q
        assert_eq!(m.active, Some(5));
        assert_eq!(m.key(0x43), Nav::Redraw); // C
        assert_eq!(m.active, Some(1));
        assert_eq!(m.key(0x5A), Nav::None, "Z nic nema");
        assert_eq!(m.key(0x48), Nav::Redraw); // H
        assert_eq!(m.active, Some(3));
    }

    #[test]
    fn empty_or_all_disabled_menu_never_panics() {
        let mut m = Menu::new(vec![]);
        for vk in [VK_DOWN, VK_UP, VK_HOME, VK_END, VK_RETURN, 0x41] {
            assert_eq!(m.key(vk), Nav::None);
        }
        let mut m = Menu::new(vec![(1, "A".into()), (0, String::new())]);
        m.items[0].enabled = false;
        assert_eq!(m.key(VK_DOWN), Nav::None);
        assert_eq!(m.key(VK_UP), Nav::None);
    }

    #[test]
    fn layout_fits_text_never_overlaps_and_scales_with_dpi() {
        for &sc in &[1.0f32, 1.25, 1.5, 2.0] {
            for label in [
                "Quit",
                "Přizpůsobit lištu do posledního detailu a ještě dál a dál",
                "新しい設定",
            ] {
                let mut m = Menu::new(vec![
                    (1, label.into()),
                    (0, String::new()),
                    (9, "Quit transom".into()),
                ]);
                m.measure(&mut fake(sc));
                let (w, h) = m.size;
                let px = |v: i32| (v as f32 * sc).round() as i32;
                assert!(w >= px(MIN_W) - 1 && w <= px(MAX_W) + 1, "w={w} sc={sc}");
                let mut prev_end = 0;
                for r in m.rows() {
                    assert!(r.y >= prev_end, "riadky sa neprekryvaju");
                    assert!(r.x >= 0 && r.x + r.w <= w);
                    prev_end = r.y + r.h;
                }
                assert!(prev_end <= h);
                // vyska = riadky + okraje (oddelovac je nizsi nez riadok)
                assert_eq!(h, 2 * px(PAD) + 2 * px(ROW_H) + px(SEP_H));
            }
        }
    }

    #[test]
    fn hit_ignores_separators_disabled_and_outside() {
        let mut m = std_items();
        m.measure(&mut fake(1.0));
        let r0 = m.rows()[0];
        assert_eq!(m.hit(r0.x + 5, r0.y + 5), Some(0));
        let sep = m.rows()[4];
        assert_eq!(m.hit(sep.x + 5, sep.y + 2), None);
        assert_eq!(m.hit(-3, 5), None);
        assert_eq!(m.hit(r0.x + r0.w, r0.y + 5), None, "pravy okraj je vonku");
        m.items[0].enabled = false;
        assert_eq!(m.hit(r0.x + 5, r0.y + 5), None);
        assert!(m.hover(Some(1)));
        assert!(!m.hover(Some(1)));
        assert!(m.hover(None));
    }

    // ---- offscreen kontrola skutocnym rendererom (GDI) bez okna: vsetky jazyky x DPI x temy ----

    use transom_render::audit::Audit;
    use transom_render::offscreen::Surface;
    use transom_render::GdiPainter;

    /// Vyrenderuje menu (aktivna polozka `active`) a vrati nalezy auditu; volitelne ulozi PNG (`TRANSOM_MENU_PNG` = priecinok).
    fn render_menu(
        lang: transom_i18n::Lang,
        dpi: u32,
        dark: bool,
        active: Option<usize>,
        hidden: bool,
    ) -> Vec<String> {
        transom_i18n::set_lang(lang);
        let th = if dark { Theme::dark() } else { Theme::light() };
        let mut m = Menu::new(crate::menu_items(hidden));
        m.active = active;
        // meranie na skutocnom DC v danom DPI (ako `popover_size`)
        let probe = Surface::new(8, 8);
        let mut mp = GdiPainter::new(probe.dc, Size { w: 8, h: 8 }, dpi);
        m.measure(&mut mp);
        let (w, h) = m.size;
        let surf = Surface::new(w, h);
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
        let mut a = Audit::new(&mut gp, th.background);
        m.paint(&mut a, &th);
        a.finish();
        if let Ok(dir) = std::env::var("TRANSOM_MENU_PNG") {
            let _ = std::fs::create_dir_all(&dir);
            let f = format!(
                "{dir}/menu-{}-{dpi}-{}-{}{}.png",
                lang.code(),
                if dark { "dark" } else { "light" },
                if hidden { "hidden" } else { "bar" },
                active.map_or(String::new(), |i| format!("-sel{i}"))
            );
            let _ = std::fs::write(
                f,
                transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
            );
        }
        a.issues
            .iter()
            .map(|i| format!("{} {dpi} {dark} {hidden}: {i:?}", lang.code()))
            .collect()
    }

    /// Galeria (pozri `crate::gallery`): menu s aktivnou polozkou pri kazdom DPI a teme.
    #[test]
    #[ignore]
    fn gallery_menu() {
        let _g = crate::test_lang::guard();
        let Some(dir) = crate::gallery::dir() else {
            return;
        };
        for dpi in crate::gallery::DPIS {
            for (dark, th) in crate::gallery::themes() {
                for (sfx, active) in [("", None), ("-sel", Some(2))] {
                    let mut m = Menu::new(crate::menu_items(false));
                    m.active = active;
                    let probe = Surface::new(8, 8);
                    let mut mp = GdiPainter::new(probe.dc, Size { w: 8, h: 8 }, dpi);
                    m.measure(&mut mp);
                    let (w, h) = m.size;
                    let surf = Surface::new(w, h);
                    let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                    m.paint(&mut gp, &th);
                    crate::gallery::save(
                        &dir,
                        &format!("menu{sfx}-{}-{dpi}", crate::gallery::tag(dark)),
                        &surf,
                    );
                }
            }
        }
    }

    #[test]
    fn menu_audit_is_clean_in_every_language_dpi_and_theme() {
        let _lang = crate::test_lang::guard();
        let mut out = Vec::new();
        for lang in transom_i18n::Lang::ALL {
            for dpi in [96u32, 144, 192] {
                for dark in [false, true] {
                    for hidden in [false, true] {
                        for active in [None, Some(0), Some(3)] {
                            out.extend(render_menu(lang, dpi, dark, active, hidden));
                        }
                    }
                }
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert!(out.is_empty(), "{out:#?}");
    }
}
