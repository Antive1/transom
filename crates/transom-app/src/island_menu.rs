//! Menu ostrova (klik na stred listy): zalozky Hudba, Schranka, Snimky, AI a Subory nad obsahom. Hudba, schranka
//! a subory pouzivaju existujuce vybezky (kreslia sa do plochy pod zalozkami); snimky a limity AI kresli tento modul.
//! Cisty stav, kreslenie a vyhodnotenie mysi.
use std::path::PathBuf;
use transom_i18n::t;
use transom_platform::appbar::PointerKind;
use transom_render::design;
use transom_render::{Color, Font, HAlign, Painter, Rect, TextStyle, Theme};

/// Vyska menu a riadku zaloziek (lp); karta Hudba je nizsia (len prehravac).
pub const MENU_H_LP: i32 = 300;
pub const TAB_H_LP: i32 = 44;

/// Vyska menu pre kartu.
pub fn height_lp(tab: Option<Tab>) -> i32 {
    match tab {
        Some(Tab::Music) => TAB_H_LP + crate::media_notch::H_LP,
        _ => MENU_H_LP,
    }
}
const ROW_LP: i32 = 34;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Music,
    Clipboard,
    Shots,
    Ai,
    Files,
}

impl Tab {
    fn label(self) -> String {
        t(match self {
            Tab::Music => "island.tab_music",
            Tab::Clipboard => "island.tab_clip",
            Tab::Shots => "island.tab_shots",
            Tab::Ai => "island.tab_ai",
            Tab::Files => "island.tab_files",
        })
    }
}

/// Snimka obrazovky v zozname (nazov suboru sa nikde nelogue).
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub path: PathBuf,
    pub name: String,
    /// Kedy vznikla ("pred 5 min").
    pub when: String,
}

/// Limit AI v zozname: okno, spotrebovane %, text (reset), malo kapacity.
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub window: String,
    pub used: u8,
    pub detail: String,
    pub low: bool,
}

/// Klik v obsahu, ktory riesi tento modul.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuCmd {
    /// Otvorit snimku v priecinku (oznacena).
    Shot(PathBuf),
    /// Otvorit snimku z kniznice (poradie v zozname).
    ShotAt(usize),
    /// Vytiahnut snimku z kniznice do inej aplikacie.
    ShotDrag(usize),
    /// Otvorit priecinok snimok.
    ShotsFolder,
    /// Otvorit panel AI.
    AiPanel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Tab(Tab),
    Shot(usize),
    Folder,
    AiPanel,
}

#[derive(Default)]
pub struct IslandMenu {
    pub tab: Option<Tab>,
    pub tabs: Vec<Tab>,
    pub shots: Vec<Shot>,
    pub limits: Vec<Limit>,
    hits: Vec<(Rect, Hit)>,
    hover: Option<Hit>,
    press: Option<Hit>,
    /// Vyska riadku zaloziek v px (z posledneho kreslenia; posun mysi do obsahu).
    pub tab_h: i32,
    /// Stlacenie na snimke: miesto (tahanie zacne po posune o par px).
    down_at: Option<(i32, i32)>,
    /// Kotva menu (x na obrazovke) - pri zmene vysky ostava na mieste.
    pub anchor: Option<i32>,
    /// Miniatury snimok z kniznice (riadky ako v Nastrojoch); `None` = kniznica je vypnuta (len zoznam nazvov).
    pub thumbs: Option<Vec<crate::tools_pop::Row>>,
    /// Rozmer miniatury v px (z posledneho kreslenia) a riadky bez nej (na nacitanie).
    pub thumb_box: (u32, u32),
    pub wanted: Vec<usize>,
}

fn st(size: f32, bold: bool, color: Color, align: HAlign) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    }
}

impl IslandMenu {
    /// Otvorenie s dostupnymi zalozkami a zvolenou (ak nie je dostupna, prva).
    pub fn open(&mut self, tabs: Vec<Tab>, want: Tab) {
        self.tab = tabs
            .iter()
            .copied()
            .find(|t| *t == want)
            .or(tabs.first().copied());
        self.tabs = tabs;
        self.hover = None;
        self.press = None;
    }

    /// Pozadie a riadok zaloziek; vracia plochu obsahu (pod zalozkami).
    pub fn paint_frame(&mut self, p: &mut dyn Painter, th: &Theme) -> Rect {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        self.hits.clear();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.background);
        self.tab_h = px(TAB_H_LP);
        // karty ako v paneli Nastroje: text, vybrana tucne s kratkou akcentovou znackou, hover jemnou plochou
        let n = self.tabs.len().max(1) as i32;
        let pad = px(14);
        let w = (sz.w - 2 * pad) / n;
        for (i, tab) in self.tabs.clone().into_iter().enumerate() {
            let r = Rect::new(pad + i as i32 * w, px(8), w, px(30));
            let on = self.tab == Some(tab);
            if !on && self.hover == Some(Hit::Tab(tab)) {
                p.fill_round_rect(r, px(design::radius::CONTROL), th.control_hover);
            }
            let ls = st(
                12.0,
                on,
                if on { th.foreground } else { th.muted },
                HAlign::Center,
            );
            let label = tab.label();
            p.draw_text(&label, Rect::new(r.x, r.y, r.w, r.h - px(4)), &ls);
            if on {
                let lw = p.measure_text(&label, &ls).w.min(r.w - px(12)).max(px(16));
                p.fill_round_rect(
                    Rect::new(r.x + (r.w - lw) / 2, r.y + r.h - px(3), lw, px(3)),
                    px(2),
                    th.accent,
                );
            }
            self.hits.push((r, Hit::Tab(tab)));
        }
        p.fill_rect(
            Rect::new(pad, px(TAB_H_LP) - 1, sz.w - 2 * pad, 1),
            th.control_edge,
        );
        Rect::new(0, self.tab_h, sz.w, sz.h - self.tab_h)
    }

    /// Obsah karty Snimky: mriezka miniatur (3 x 2) ako v Nastrojoch, alebo zoznam nazvov bez kniznice.
    pub fn paint_shots(&mut self, p: &mut dyn Painter, th: &Theme, area: Rect) {
        if let Some(rows) = self.thumbs.clone() {
            self.paint_grid(p, th, area, &rows);
            return;
        }
        self.paint_shot_list(p, th, area);
    }

    fn paint_grid(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        area: Rect,
        rows: &[crate::tools_pop::Row],
    ) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let pad = px(14);
        let gap = px(8);
        let cols = 3;
        let cw = (area.w - 2 * pad - (cols - 1) * gap) / cols;
        let thh = cw * 9 / 16;
        let ch = thh + px(22);
        self.thumb_box = ((cw - 2).max(1) as u32, (thh - 2).max(1) as u32);
        self.wanted.clear();
        if rows.is_empty() {
            p.draw_text(
                &t("island.shots_none"),
                Rect::new(area.x + pad, area.y + px(8), area.w - 2 * pad, px(ROW_LP)),
                &st(12.0, false, th.muted, HAlign::Left),
            );
        }
        for (i, row) in rows.iter().take(6).enumerate() {
            let (c, r) = (i as i32 % cols, i as i32 / cols);
            let cell = Rect::new(
                area.x + pad + c * (cw + gap),
                area.y + px(6) + r * (ch + gap),
                cw,
                ch,
            );
            if self.hover == Some(Hit::Shot(i)) {
                p.fill_round_rect(
                    Rect::new(
                        cell.x - px(4),
                        cell.y - px(4),
                        cell.w + px(8),
                        cell.h + px(6),
                    ),
                    px(design::radius::CONTROL),
                    th.hover,
                );
            }
            let tr = Rect::new(cell.x, cell.y, cw, thh);
            crate::tools_pop::paint_thumb(p, th, tr, &row.thumb);
            if row.thumb == crate::tools_pop::Thumb::Loading {
                self.wanted.push(i);
            }
            p.draw_text(
                &row.subtitle,
                Rect::new(cell.x, cell.y + thh + px(2), cw, px(18)),
                &st(10.5, false, th.muted, HAlign::Left),
            );
            self.hits.push((cell, Hit::Shot(i)));
        }
        self.footer(p, th, area, &t("island.open_folder"), Hit::Folder);
    }

    fn paint_shot_list(&mut self, p: &mut dyn Painter, th: &Theme, area: Rect) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let pad = px(14);
        let mut y = area.y + px(4);
        if self.shots.is_empty() {
            p.draw_text(
                &t("island.shots_none"),
                Rect::new(area.x + pad, y, area.w - 2 * pad, px(ROW_LP)),
                &st(12.0, false, th.muted, HAlign::Left),
            );
            y += px(ROW_LP);
        }
        let max_rows = ((area.h - px(ROW_LP) - px(8)) / px(ROW_LP)).max(0) as usize;
        for (i, sh) in self.shots.clone().iter().take(max_rows).enumerate() {
            let r = Rect::new(area.x + px(8), y, area.w - px(16), px(ROW_LP));
            if self.hover == Some(Hit::Shot(i)) {
                p.fill_round_rect(r, px(design::radius::CONTROL), th.hover);
            }
            p.draw_text(
                "\u{EB9F}",
                Rect::new(r.x + px(6), r.y, px(20), r.h),
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 13.0,
                    bold: false,
                    color: th.accent,
                    align: HAlign::Center,
                },
            );
            let tw = r.w - px(34) - px(90);
            p.draw_text(
                &sh.name,
                Rect::new(r.x + px(32), r.y, tw, r.h),
                &st(12.0, false, th.foreground, HAlign::Left),
            );
            p.draw_text(
                &sh.when,
                Rect::new(r.x + r.w - px(96), r.y, px(88), r.h),
                &st(11.0, false, th.muted, HAlign::Right),
            );
            self.hits.push((r, Hit::Shot(i)));
            y += px(ROW_LP);
        }
        self.footer(p, th, area, &t("island.open_folder"), Hit::Folder);
    }

    /// Obsah karty AI: limity (5 h, tyzden...) ako pruhy.
    pub fn paint_ai(&mut self, p: &mut dyn Painter, th: &Theme, area: Rect) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let pad = px(18);
        let w = area.w - 2 * pad;
        let mut y = area.y + px(8);
        if self.limits.is_empty() {
            p.draw_text(
                &t("island.ai_none"),
                Rect::new(area.x + pad, y, w, px(ROW_LP)),
                &st(12.0, false, th.muted, HAlign::Left),
            );
        }
        for l in &self.limits {
            p.draw_text(
                &l.window,
                Rect::new(area.x + pad, y, w, px(18)),
                &st(12.0, false, th.foreground, HAlign::Left),
            );
            p.draw_text(
                &format!("{} %", l.used),
                Rect::new(area.x + pad, y, w, px(18)),
                &st(
                    12.0,
                    l.low,
                    if l.low { th.low } else { th.foreground },
                    HAlign::Right,
                ),
            );
            let bar = Rect::new(area.x + pad, y + px(22), w, px(6));
            p.fill_round_rect(bar, px(3), th.control_edge);
            let f = ((w as f32) * (l.used as f32 / 100.0)).round() as i32;
            if f > 0 {
                p.fill_round_rect(
                    Rect::new(bar.x, bar.y, f.max(px(6)), bar.h),
                    px(3),
                    if l.low { th.low } else { th.accent },
                );
            }
            p.draw_text(
                &l.detail,
                Rect::new(area.x + pad, y + px(30), w, px(16)),
                &st(10.5, false, th.muted, HAlign::Left),
            );
            y += px(56);
        }
        self.footer(p, th, area, &t("island.open_ai"), Hit::AiPanel);
    }

    fn footer(&mut self, p: &mut dyn Painter, th: &Theme, area: Rect, text: &str, hit: Hit) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let r = Rect::new(
            area.x + px(8),
            area.y + area.h - px(ROW_LP) - px(8),
            area.w - px(16),
            px(ROW_LP),
        );
        if self.hover == Some(hit) {
            p.fill_round_rect(r, px(design::radius::CONTROL), th.hover);
        }
        p.draw_text(text, r, &st(12.0, false, th.accent, HAlign::Center));
        self.hits.push((r, hit));
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| *h)
    }

    /// Mys v menu: (prepnuta zalozka, prikaz, treba prekreslit, mys je v riadku zaloziek / na vlastnom ovladaci).
    /// Obsah hudby, schranky a suborov riesi volajuci (suradnice posunute o `tab_h`).
    pub fn pointer(
        &mut self,
        kind: PointerKind,
        x: i32,
        y: i32,
    ) -> (Option<Tab>, Option<MenuCmd>, bool, bool) {
        let here = if kind == PointerKind::Leave {
            None
        } else {
            self.hit(x, y)
        };
        let redraw = here != self.hover;
        self.hover = here;
        let mut switched = None;
        let mut cmd = None;
        // tahanie snimky von: stlacenie na miniature a posun o viac nez 6 px
        if kind == PointerKind::Move {
            if let (Some(Hit::Shot(i)), Some((dx, dy))) = (self.press, self.down_at) {
                if self.thumbs.is_some() && ((x - dx).abs() > 6 || (y - dy).abs() > 6) {
                    self.press = None;
                    self.down_at = None;
                    return (None, Some(MenuCmd::ShotDrag(i)), redraw, true);
                }
            }
        }
        match kind {
            PointerKind::Down => {
                self.press = here;
                self.down_at = Some((x, y));
            }
            PointerKind::Click => {
                if self.press.is_some() && self.press == here {
                    match here {
                        Some(Hit::Tab(tab)) if self.tab != Some(tab) => {
                            self.tab = Some(tab);
                            switched = Some(tab);
                        }
                        Some(Hit::Shot(i)) if self.thumbs.is_some() => {
                            cmd = Some(MenuCmd::ShotAt(i))
                        }
                        Some(Hit::Shot(i)) => {
                            cmd = self.shots.get(i).map(|s| MenuCmd::Shot(s.path.clone()))
                        }
                        Some(Hit::Folder) => cmd = Some(MenuCmd::ShotsFolder),
                        Some(Hit::AiPanel) => cmd = Some(MenuCmd::AiPanel),
                        _ => {}
                    }
                }
                self.press = None;
            }
            PointerKind::Leave => self.press = None,
            _ => {}
        }
        let own = y < self.tab_h || matches!(self.tab, Some(Tab::Shots | Tab::Ai));
        (switched, cmd, redraw || switched.is_some(), own)
    }
}

/// "pred 5 min" / "pred 2 h" / "pred 3 d" z veku v sekundach.
pub fn ago(secs: u64) -> String {
    let v = if secs < 60 {
        return t("island.just_now");
    } else if secs < 3600 {
        format!("{} min", secs / 60)
    } else if secs < 86_400 {
        format!("{} h", secs / 3600)
    } else {
        format!("{} d", secs / 86_400)
    };
    transom_i18n::tf("island.ago", &[&v])
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_platform::appbar::PointerKind as K;

    #[test]
    fn open_picks_the_wanted_tab_or_the_first_available() {
        let mut m = IslandMenu::default();
        m.open(vec![Tab::Clipboard, Tab::Files], Tab::Music);
        assert_eq!(m.tab, Some(Tab::Clipboard));
        m.open(vec![Tab::Music, Tab::Files], Tab::Files);
        assert_eq!(m.tab, Some(Tab::Files));
        m.open(vec![], Tab::Files);
        assert_eq!(m.tab, None);
    }

    #[test]
    fn a_tab_switches_on_release_over_it() {
        let mut m = IslandMenu::default();
        m.open(vec![Tab::Music, Tab::Ai], Tab::Music);
        m.tab_h = 40;
        m.hits = vec![
            (Rect::new(0, 0, 50, 30), Hit::Tab(Tab::Music)),
            (Rect::new(60, 0, 50, 30), Hit::Tab(Tab::Ai)),
        ];
        m.pointer(K::Down, 70, 10);
        let (sw, cmd, redraw, own) = m.pointer(K::Click, 70, 10);
        assert_eq!((sw, cmd, redraw, own), (Some(Tab::Ai), None, true, true));
        assert_eq!(m.tab, Some(Tab::Ai));
        // obsah pod zalozkami patri volajucemu (okrem snimok a AI)
        m.tab = Some(Tab::Music);
        assert!(!m.pointer(K::Move, 10, 100).3);
    }

    #[test]
    fn ago_texts() {
        assert_eq!(ago(10), t("island.just_now"));
        assert!(ago(300).contains("5 min"));
        assert!(ago(7200).contains("2 h"));
    }
}
