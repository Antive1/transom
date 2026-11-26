//! Okno vstavaneho vyhladavaca (popover): vstupne pole (pisanie cez `PopoverChar`), zoznam vysledkov s kategoriami a vybrana
//! akcia (Otvorit / Ukazat v priecinku / Kopirovat cestu). Cisty stav a kreslenie bez okna, aby sa dalo testovat offscreen.
//! Dotaz sa nikdy nelogue; nic sa nespusta, kym pouzivatel nevyberie vysledok (Enter alebo klik).
use crate::audio_pop::{fit, st};
use crate::icon_rt::{builtin_for, Icon, IconSource};
use transom_i18n::t;
use transom_launcher::{allowed_acts, Act, Hit, Kind};
use transom_platform::menu_access::{
    MenuEntry, PageExtra, ROLE_LISTITEM, ROLE_TEXT, STATE_SELECTED,
};
use transom_platform::PointerKind;
use transom_render::{HAlign, Painter, Rect, Theme};

pub const LAUNCHER_W_LP: i32 = 560;
pub const LAUNCHER_H_LP: i32 = 530;
/// Najdlhsi dotaz (znaky); dlhsi vstup sa orezava.
pub const MAX_QUERY: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LauncherCmd {
    /// Zmenil sa dotaz: spusti (alebo zrus) hladanie.
    Query(String),
    /// Vykonat akciu nad vysledkom `i`.
    Do(usize, Act),
    /// Vlozit text zo schranky (vyslovne Ctrl+V).
    Paste,
}

#[derive(Default)]
pub struct LauncherPanel {
    pub query: String,
    pub results: Vec<Hit>,
    pub selected: usize,
    pub hover: Option<usize>,
    /// Vsetky zdroje dobehli (pri prazdnom zozname sa az vtedy ukaze "Ziadne vysledky").
    pub done: bool,
    pub generation: u64,
    /// Index Windows Search nie je dostupny (vysvetli sa obmedzenie).
    pub index_down: bool,
    /// Kratka sprava vo vypise (skopirovane / nepodarilo sa otvorit).
    pub flash: Option<String>,
    pub repaint: bool,
    /// Vysledky patria starsiemu dotazu (po zmene textu, kym nepridu nove): Enter a klik ich neotvoria.
    pub stale: bool,
    /// Kolko riadkov sa pri poslednom kresleni zmestilo (0 = este sa nekreslilo): navigacia nejde na skryte riadky.
    visible: usize,
    /// Zdroj suborov je zapnuty (kym `done` nie je pravda, bezi pomaly zdroj a ukaze sa "Hladam subory...").
    pub files_pending: bool,
    hits: Vec<(Rect, usize)>,
    nodes: Vec<(MenuEntry, PageExtra)>,
}

fn kind_key(k: Kind) -> &'static str {
    match k {
        Kind::App => "lau.k_app",
        Kind::File => "lau.k_file",
        Kind::Folder => "lau.k_folder",
        Kind::Setting => "lau.k_setting",
        Kind::Action => "lau.k_action",
        Kind::Note => "lau.k_note",
        Kind::Shelf => "lau.k_shelf",
    }
}

impl LauncherPanel {
    pub fn reset(&mut self) {
        *self = LauncherPanel::default();
    }

    /// Prijme vysledky; zastarany dotaz (nizsia generacia) sa zahodi.
    pub fn set_results(&mut self, generation: u64, hits: Vec<Hit>, done: bool) -> bool {
        if generation < self.generation {
            return false;
        }
        self.generation = generation;
        self.stale = false;
        // vyber ostane na tej istej polozke, aj ked ju neskore prichadzajuce subory presunu (inak by Enter otvoril inu, nez je zvyraznena)
        let keep = self.results.get(self.selected).map(|h| h.item.target.key());
        self.results = hits;
        self.done = done;
        self.selected = keep
            .and_then(|k| self.results.iter().position(|h| h.item.target.key() == k))
            .unwrap_or(0);
        self.repaint = true;
        true
    }

    fn changed(&mut self) -> Option<LauncherCmd> {
        self.selected = 0;
        self.stale = true;
        self.flash = None;
        self.done = false;
        self.repaint = true;
        Some(LauncherCmd::Query(self.query.clone()))
    }

    /// Napisany znak (bez riadiacich; tie ide cez `key`).
    pub fn ch(&mut self, c: char) -> Option<LauncherCmd> {
        if c.is_control() || self.query.chars().count() >= MAX_QUERY {
            return None;
        }
        self.query.push(c);
        self.changed()
    }

    /// Vlozeny text: prvy riadok, bez riadiacich znakov, orezany na limit.
    pub fn paste(&mut self, text: &str) -> Option<LauncherCmd> {
        let line = text.lines().next().unwrap_or("");
        let room = MAX_QUERY.saturating_sub(self.query.chars().count());
        let add: String = line
            .chars()
            .filter(|c| !c.is_control())
            .take(room)
            .collect();
        if add.is_empty() {
            return None;
        }
        self.query.push_str(&add);
        self.changed()
    }

    /// Klavesa: Up/Down vyber, Enter otvorit (Ctrl+Enter ukazat v priecinku), Ctrl+C kopirovat cestu, Backspace (Ctrl = cely
    /// dotaz), Ctrl+V vlozit. Escape zatvara okno (riesi platforma). Vracia (prikaz, treba prekreslit).
    pub fn key(&mut self, vk: u32, ctrl: bool) -> (Option<LauncherCmd>, bool) {
        let n = self.results.len().min(if self.visible == 0 {
            usize::MAX
        } else {
            self.visible
        });
        match vk {
            0x26 if n > 0 => {
                self.selected = (self.selected + n - 1) % n;
                (None, true)
            }
            0x28 if n > 0 => {
                self.selected = (self.selected + 1) % n;
                (None, true)
            }
            0x08 => {
                if self.query.is_empty() {
                    return (None, false);
                }
                if ctrl {
                    self.query.clear();
                } else {
                    self.query.pop();
                }
                (self.changed(), true)
            }
            0x0D if n > 0 => {
                let act = if ctrl { Act::OpenContaining } else { Act::Open };
                (self.act(self.selected, act), true)
            }
            0x43 if ctrl && n > 0 => (self.act(self.selected, Act::CopyPath), true),
            0x56 if ctrl => (Some(LauncherCmd::Paste), false),
            _ => (None, false),
        }
    }

    fn act(&self, i: usize, a: Act) -> Option<LauncherCmd> {
        if self.stale {
            return None;
        }
        let h = self.results.get(i)?;
        allowed_acts(&h.item)
            .contains(&a)
            .then_some(LauncherCmd::Do(i, a))
    }

    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32) -> (Option<LauncherCmd>, bool) {
        let h = if kind == PointerKind::Leave {
            None
        } else {
            self.hits
                .iter()
                .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
                .map(|(_, i)| *i)
        };
        let mut redraw = h != self.hover;
        self.hover = h;
        let mut cmd = None;
        match kind {
            PointerKind::Move => {
                if let Some(i) = h {
                    if i != self.selected {
                        self.selected = i;
                        redraw = true;
                    }
                }
            }
            PointerKind::Click => {
                if let Some(i) = h {
                    self.selected = i;
                    cmd = self.act(i, Act::Open);
                    redraw = true;
                }
            }
            _ => {}
        }
        (cmd, redraw)
    }

    pub fn a11y(&self) -> (Vec<MenuEntry>, Vec<PageExtra>, Option<usize>, String) {
        let focus =
            (!self.results.is_empty()).then(|| 1 + self.selected.min(self.results.len() - 1));
        (
            self.nodes.iter().map(|n| n.0.clone()).collect(),
            self.nodes.iter().map(|n| n.1.clone()).collect(),
            focus.or(Some(0)),
            t("lau.name"),
        )
    }

    /// Bez zdroja ikon (vstavane zastupne ikony); pouzivaju ho testy.
    #[cfg(test)]
    pub fn paint(&mut self, p: &mut dyn Painter, th: &Theme) {
        self.paint_with(p, th, None);
    }

    /// Ako `paint`, so zdrojom ikon vysledkov. Ikona je vzdy hned pouzitelna (hotova systemova, alebo vstavana zastupna) a nacitanie
    /// ikon bezi na pozadi, takze kreslenie ani pisanie sa nezdrzia; texty a tlacidla sa od ikony neodvijaju.
    pub fn paint_with(&mut self, p: &mut dyn Painter, th: &Theme, icons: Option<&dyn IconSource>) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, b, sz.w - 2 * b, sz.h - 2 * b), th.background);
        let pad = px(14);
        let w = sz.w - 2 * pad;
        self.hits.clear();
        let mut nodes: Vec<(MenuEntry, PageExtra)> = Vec::new();
        let ent = |r: Rect, name: String| MenuEntry {
            name,
            separator: false,
            enabled: true,
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        };

        // vstupne pole (ma fokus): plocha ovladaca, lupa a akcentova spodna linka ako pole s fokusom vo Windows 11
        let input = Rect::new(pad, pad, w, px(40));
        let rad = px(transom_render::design::radius::CONTROL);
        p.fill_round_rect(input, rad, th.control_edge);
        p.fill_round_rect(
            Rect::new(input.x + b, input.y + b, input.w - 2 * b, input.h - 2 * b),
            (rad - b).max(0),
            th.control,
        );
        p.fill_rect(
            Rect::new(
                input.x + px(2),
                input.y + input.h - px(2),
                input.w - px(4),
                px(2),
            ),
            th.accent,
        );
        p.draw_text(
            "\u{E721}",
            Rect::new(input.x + px(12), input.y, px(16), input.h),
            &transom_render::TextStyle {
                font: transom_render::Font::Icons,
                size_lp: 13.0,
                bold: false,
                color: th.muted,
                align: HAlign::Center,
            },
        );
        let tx = Rect::new(input.x + px(38), input.y, input.w - px(50), input.h);
        if self.query.is_empty() {
            p.draw_text(
                &t("lau.placeholder"),
                tx,
                &st(14.0, false, th.muted, HAlign::Left),
            );
        } else {
            let style = st(14.0, false, th.foreground, HAlign::Left);
            // dlhy dotaz sa zobrazi od konca (kurzor je vzdy viditelny)
            let shown: String = {
                let chars: Vec<char> = self.query.chars().collect();
                let mut start = 0;
                while start < chars.len() {
                    let cand: String = chars[start..].iter().collect();
                    if p.measure_text(&cand, &style).w <= tx.w - px(4) {
                        break;
                    }
                    start += 1;
                }
                chars[start..].iter().collect()
            };
            p.draw_text(&shown, tx, &style);
            let cw = p.measure_text(&shown, &style).w;
            p.fill_rect(
                Rect::new(tx.x + cw + px(1), tx.y + px(9), px(2).max(1), tx.h - px(18)),
                th.accent_text,
            );
        }
        nodes.push((
            ent(input, t("lau.title")),
            PageExtra {
                role: ROLE_TEXT,
                state: 0,
                value: self.query.clone(),
                action: String::new(),
            },
        ));

        // vysledky
        let tit_probe = st(13.0, true, th.foreground, HAlign::Left);
        let det_probe = st(11.0, false, th.muted, HAlign::Left);
        let th_h = p.measure_text("Ag", &tit_probe).h;
        let dh_h = p.measure_text("Ag", &det_probe).h;
        let row_h = (th_h + dh_h + px(14)).max(px(44));
        let mut y = input.y + input.h + px(10);
        let max_rows = ((sz.h - y - px(64)) / row_h).max(1) as usize;
        self.visible = max_rows;
        // stlpec ikony (24 lp; ikona sa nacita v presnej velkosti v pixeloch, takze je ostra pri kazdom DPI)
        let ic = px(24);
        if let Some(src) = icons {
            src.begin();
        }
        for (i, h) in self.results.iter().take(max_rows).enumerate() {
            let r = Rect::new(pad, y, w, row_h);
            let sel = i == self.selected;
            let rad = px(transom_render::design::radius::CONTROL + 2);
            // vybrany riadok: zdvihnuta plocha + akcentova znacka vlavo (ako menu); pod mysou polovicny odtien
            if sel {
                p.fill_round_rect(r, rad, th.hover);
                p.fill_round_rect(
                    Rect::new(r.x + px(2), r.y + px(12), px(3), r.h - px(24)),
                    px(2),
                    th.accent,
                );
            } else if self.hover == Some(i) {
                p.fill_round_rect(
                    r,
                    rad,
                    transom_render::scene::mix(th.hover, th.background, 0.35),
                );
            }
            let tag = t(kind_key(h.item.kind));
            let sub_color = if sel { th.foreground } else { th.muted };
            let tag_style = st(11.0, false, sub_color, HAlign::Right);
            let tag_w = p.measure_text(&tag, &tag_style).w + px(8);
            let inner = Rect::new(r.x + px(10), r.y, r.w - px(20), r.h);
            // ikona vlavo, texty vpravo od nej
            let icon_rect = Rect::new(inner.x, r.y + (r.h - ic) / 2, ic, ic);
            let icon = match icons {
                Some(src) => src.icon(&h.item, ic as u32),
                None => Icon::Builtin(builtin_for(h.item.kind)),
            };
            match &icon {
                Icon::Pixels(b) if b.w as i32 <= ic && b.h as i32 <= ic => {
                    let (bw, bh) = (b.w as i32, b.h as i32);
                    let px_: Vec<transom_render::brand::Rgba> = b
                        .rgba
                        .iter()
                        .map(|c| transom_render::brand::Rgba {
                            r: c[0],
                            g: c[1],
                            b: c[2],
                            a: c[3],
                        })
                        .collect();
                    p.blend_rgba(
                        Rect::new(
                            icon_rect.x + (ic - bw) / 2,
                            icon_rect.y + (ic - bh) / 2,
                            bw,
                            bh,
                        ),
                        &px_,
                    );
                }
                other => {
                    let kind = match other {
                        Icon::Builtin(k) => *k,
                        Icon::Pixels(_) => builtin_for(h.item.kind),
                    };
                    let g = transom_render::resulticons::cached(kind, ic as u32, sub_color);
                    p.blend_rgba(icon_rect, &g);
                }
            }
            let tx = inner.x + ic + px(10);
            let tw = inner.w - (ic + px(10));
            // nazov sa pri vybere nemeni (ziadne preskakovanie sirky); vyber nesie plocha a znacka
            let tit = st(13.0, false, th.foreground, HAlign::Left);
            let det = st(11.0, false, sub_color, HAlign::Left);
            let title = fit(p, &h.item.title, &tit, tw - tag_w - px(8));
            p.draw_text(&title, Rect::new(tx, r.y + px(5), tw - tag_w, th_h), &tit);
            let detail = fit(p, &h.item.detail, &det, tw - tag_w - px(8));
            p.draw_text(
                &detail,
                Rect::new(tx, r.y + px(7) + th_h, tw - tag_w, dh_h),
                &det,
            );
            p.draw_text(&tag, Rect::new(inner.x, r.y, inner.w, r.h), &tag_style);
            nodes.push((
                ent(r, format!("{}, {}", h.item.title, tag)),
                PageExtra {
                    role: ROLE_LISTITEM,
                    state: if sel { STATE_SELECTED } else { 0 },
                    value: h.item.detail.clone(),
                    action: t("acc.press"),
                },
            ));
            self.hits.push((r, i));
            y += row_h;
        }
        if let Some(src) = icons {
            src.end();
        }
        if self.results.is_empty() && !self.query.trim().is_empty() && self.done {
            p.draw_text(
                &t("lau.empty"),
                Rect::new(pad, y + px(8), w, px(24)),
                &st(13.0, false, th.muted, HAlign::Center),
            );
        }

        // pata: obmedzenie indexu, kratka sprava a klavesove skratky (oddelena vlasom)
        let foot = st(11.0, false, th.muted, HAlign::Left);
        let mut fy = sz.h - px(54);
        p.fill_rect(Rect::new(pad, fy - px(8), w, px(1).max(1)), th.edge);
        let note = if let Some(f) = &self.flash {
            Some(f.clone())
        } else if !self.done && !self.query.trim().is_empty() && self.files_pending {
            // pomaly zdroj (index suborov) este bezi: rychle vysledky su uz zobrazene, subory pridu neskor
            Some(t("lau.searching"))
        } else if self.index_down && !self.query.trim().is_empty() {
            Some(t("lau.no_index"))
        } else {
            None
        };
        if let Some(n) = note {
            let n = fit(p, &n, &foot, w);
            p.draw_text(&n, Rect::new(pad, fy, w, px(16)), &foot);
        }
        fy += px(20);
        let hint = fit(p, &t("lau.hint"), &foot, w);
        p.draw_text(&hint, Rect::new(pad, fy, w, px(16)), &foot);
        self.nodes = nodes;
    }
}

/// Pristupnost panela (polozky, role/stavy, fokus, nazov korena).
#[cfg(test)]
type A11y = (Vec<MenuEntry>, Vec<PageExtra>, Option<usize>, String);

/// Pristupnost panela v typickych stavoch (pre vypis pred/po zmene vzhladu v `crate::gallery`).
#[cfg(test)]
pub(crate) fn a11y_samples() -> Vec<(String, A11y)> {
    use transom_launcher::{Item, Target};
    let hit = |t: &str, k: Kind, tg: Target| Hit {
        item: Item::new(t, "detail", k, tg),
        score: 1,
    };
    let mut out = Vec::new();
    for (name, q, res) in [
        (
            "results",
            "no",
            vec![
                hit("Notepad", Kind::App, Target::Path(r"C:\n.lnk".into())),
                hit("Notebooks", Kind::Folder, Target::Path(r"C:\d\nb".into())),
                hit("Sound", Kind::Setting, Target::Uri("ms-settings:sound")),
            ],
        ),
        ("empty", "", Vec::new()),
        ("none", "qqq", Vec::new()),
    ] {
        let mut p = LauncherPanel::default();
        for c in q.chars() {
            p.ch(c);
        }
        p.set_results(p.generation, res, true);
        let surf = transom_render::offscreen::Surface::new(560, 530);
        for _ in 0..2 {
            let mut gp = transom_render::GdiPainter::new(
                surf.dc,
                transom_render::Size { w: 560, h: 530 },
                96,
            );
            p.paint(&mut gp, &Theme::dark());
        }
        out.push((name.to_string(), p.a11y()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_launcher::{Item, Target};

    fn hit(title: &str, kind: Kind, target: Target) -> Hit {
        Hit {
            item: Item::new(title, "detail", kind, target),
            score: 1,
        }
    }

    fn panel() -> LauncherPanel {
        let mut p = LauncherPanel::default();
        p.set_results(
            1,
            vec![
                hit("Notepad", Kind::App, Target::Path(r"C:\n.lnk".into())),
                hit("Sound", Kind::Setting, Target::Uri("ms-settings:sound")),
            ],
            true,
        );
        p
    }

    #[test]
    fn typing_editing_and_pasting_produce_queries_with_limits_and_no_control_characters() {
        let mut p = LauncherPanel::default();
        assert_eq!(p.ch('a'), Some(LauncherCmd::Query("a".into())));
        assert_eq!(p.ch('\n'), None);
        assert_eq!(p.ch('\u{7}'), None);
        assert_eq!(p.ch('č'), Some(LauncherCmd::Query("ač".into())));
        assert_eq!(p.key(0x08, false).0, Some(LauncherCmd::Query("a".into())));
        assert_eq!(p.key(0x08, true).0, Some(LauncherCmd::Query(String::new())));
        assert_eq!(p.key(0x08, false).0, None, "prazdny dotaz sa nemeni");
        assert_eq!(
            p.paste("jeden\ndruhy"),
            Some(LauncherCmd::Query("jeden".into()))
        );
        assert_eq!(p.paste("\u{1}\u{2}"), None);
        let mut q = LauncherPanel::default();
        for _ in 0..MAX_QUERY + 50 {
            q.ch('x');
        }
        assert_eq!(q.query.chars().count(), MAX_QUERY);
        assert_eq!(q.paste("more"), None, "plne pole nic nepridava");
        assert_eq!(p.key(0x56, true).0, Some(LauncherCmd::Paste));
    }

    #[test]
    fn selection_wraps_and_enter_ctrl_enter_and_ctrl_c_respect_what_the_result_allows() {
        let mut p = panel();
        assert_eq!(p.key(0x28, false), (None, true));
        assert_eq!(p.selected, 1);
        assert!(p.key(0x28, false).1);
        assert_eq!(p.selected, 0, "kruh");
        assert!(p.key(0x26, false).1);
        assert_eq!(p.selected, 1);
        // nastavenie: iba Otvorit; Ctrl+Enter a Ctrl+C nic nerobia
        assert_eq!(p.key(0x0D, false).0, Some(LauncherCmd::Do(1, Act::Open)));
        assert_eq!(p.key(0x0D, true).0, None);
        assert_eq!(p.key(0x43, true).0, None);
        p.selected = 0;
        assert_eq!(
            p.key(0x0D, true).0,
            Some(LauncherCmd::Do(0, Act::OpenContaining))
        );
        assert_eq!(p.key(0x43, true).0, Some(LauncherCmd::Do(0, Act::CopyPath)));
        let mut empty = LauncherPanel::default();
        assert_eq!(
            empty.key(0x0D, false),
            (None, false),
            "bez vysledkov Enter nic nerobi"
        );
    }

    /// Audit vzhladu (orezanie, prekryv, kontrast) pre vsetky jazyky, 3 DPI a obe temy: prazdny dotaz, plne vysledky, dlhe texty,
    /// stav "Ziadne vysledky" a obmedzenie indexu.
    #[test]
    fn the_launcher_passes_the_layout_and_contrast_audit_in_every_language_dpi_and_theme() {
        use transom_render::audit::Audit;
        let _g = crate::test_lang::guard();
        let long = "Velmi dlhy nazov suboru s mnohymi slovami ".repeat(4);
        let mut bad = Vec::new();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            for dpi in [96u32, 144, 192] {
                for dark in [true, false] {
                    let th = if dark { Theme::dark() } else { Theme::light() };
                    let sc = dpi as f32 / 96.0;
                    let (w, h) = (
                        (LAUNCHER_W_LP as f32 * sc).round() as i32,
                        (LAUNCHER_H_LP as f32 * sc).round() as i32,
                    );
                    for (name, q, hits, done, down, flash) in [
                        ("empty", "", vec![], false, false, None),
                        ("none", "zzz", vec![], true, true, None),
                        (
                            "full",
                            "n",
                            (0..8)
                                .map(|i| {
                                    let kinds = [
                                        Kind::App,
                                        Kind::File,
                                        Kind::Folder,
                                        Kind::Setting,
                                        Kind::Action,
                                        Kind::Note,
                                        Kind::Shelf,
                                        Kind::App,
                                    ];
                                    hit(
                                        &format!("{i} {long}"),
                                        kinds[i],
                                        Target::Path(format!("C:\\{long}")),
                                    )
                                })
                                .collect(),
                            true,
                            false,
                            Some(t("lau.copied")),
                        ),
                    ] {
                        let mut p = LauncherPanel {
                            query: if q == "n" {
                                long.clone()
                            } else {
                                q.to_string()
                            },
                            index_down: down,
                            flash,
                            ..Default::default()
                        };
                        p.set_results(1, hits, done);
                        let surf = transom_render::offscreen::Surface::new(w, h);
                        for _ in 0..2 {
                            let mut gp = transom_render::GdiPainter::new(
                                surf.dc,
                                transom_render::Size { w, h },
                                dpi,
                            );
                            let mut au = Audit::new(&mut gp, th.background);
                            p.paint(&mut au, &th);
                            au.finish();
                            for i in &au.issues {
                                bad.push(format!("{} {dpi} {name} {dark}: {i:?}", lang.code()));
                            }
                        }
                    }
                }
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert!(
            bad.is_empty(),
            "{} nalezov, napr.:\n{}",
            bad.len(),
            bad.iter().take(12).cloned().collect::<Vec<_>>().join("\n")
        );
    }

    /// Galeria (pozri `crate::gallery`): dotaz s vysledkami vsetkych druhov a vstavanymi ikonami, vybrany a podsvieteny riadok.
    #[test]
    #[ignore]
    fn gallery_launcher() {
        let _g = crate::test_lang::guard();
        let Some(dir) = crate::gallery::dir() else {
            return;
        };
        struct Builtins;
        impl IconSource for Builtins {
            fn icon(&self, item: &Item, _px: u32) -> Icon {
                Icon::Builtin(builtin_for(item.kind))
            }
        }
        // realisticke popisy (galeria sluzi aj na snimky do README)
        let d = |t: &str, detail: &str, k: Kind, tg: Target| Hit {
            item: Item::new(t, detail, k, tg),
            score: 1,
        };
        let results = vec![
            d(
                "Notepad",
                "Windows app",
                Kind::App,
                Target::Path(r"C:\n.lnk".into()),
            ),
            d(
                "notes-2026.txt",
                r"C:\Users\you\Documents",
                Kind::File,
                Target::Path(r"C:\d\notes-2026.txt".into()),
            ),
            d(
                "Notebooks",
                r"C:\Users\you\Documents",
                Kind::Folder,
                Target::Path(r"C:\d\Notebooks".into()),
            ),
            d(
                "Notifications",
                "System › Notifications",
                Kind::Setting,
                Target::Uri("ms-settings:notifications"),
            ),
            d(
                "Open notes",
                "Quick Notes",
                Kind::Action,
                Target::Uri("transom:notes"),
            ),
            d(
                "Note: shopping list",
                "milk, coffee, batteries",
                Kind::Note,
                Target::Uri("transom:note:1"),
            ),
            d(
                "notebook-cover.png",
                "2,4 MB",
                Kind::Shelf,
                Target::Uri("transom:shelf:1"),
            ),
        ];
        for dpi in crate::gallery::DPIS {
            for (dark, th) in crate::gallery::themes() {
                for (name, q, res, hover) in [
                    ("results", "no", results.clone(), Some(2usize)),
                    ("empty", "", Vec::new(), None),
                    ("none", "qqqzz", Vec::new(), None),
                ] {
                    let mut p = LauncherPanel::default();
                    for c in q.chars() {
                        p.ch(c);
                    }
                    p.set_results(p.generation, res, true);
                    p.hover = hover;
                    let (w, h) = (
                        crate::gallery::px(LAUNCHER_W_LP, dpi),
                        crate::gallery::px(LAUNCHER_H_LP, dpi),
                    );
                    let surf = transom_render::offscreen::Surface::new(w, h);
                    for _ in 0..2 {
                        let mut gp = transom_render::GdiPainter::new(
                            surf.dc,
                            transom_render::Size { w, h },
                            dpi,
                        );
                        p.paint_with(&mut gp, &th, Some(&Builtins));
                    }
                    crate::gallery::save(
                        &dir,
                        &format!("launcher-{name}-{}-{dpi}", crate::gallery::tag(dark)),
                        &surf,
                    );
                }
            }
        }
        // predvolene sklo (podklady promo videa)
        let dpi = crate::gallery::hero_dpi();
        let (w, h) = (
            crate::gallery::px(LAUNCHER_W_LP, dpi),
            crate::gallery::px(LAUNCHER_H_LP, dpi),
        );
        let mut p = LauncherPanel::default();
        for c in "no".chars() {
            p.ch(c);
        }
        p.set_results(p.generation, results.clone(), true);
        p.hover = Some(2);
        let (gw, rgba) = crate::gallery::glass_rgba_panel(
            w,
            h,
            dpi,
            &transom_render::Theme::dark(),
            &mut |gp, t| p.paint_with(gp, t, Some(&Builtins)),
        );
        crate::gallery::save_rgba(&dir, "launcher-glass", gw, h, &rgba);
    }

    /// Falosny zdroj ikon: neparne riadky maju hotove pixely presnej velkosti (cervene), parne vstavanu ikonu; pocita volania.
    struct FakeIcons {
        begins: std::cell::Cell<u32>,
        ends: std::cell::Cell<u32>,
        asked: std::cell::RefCell<Vec<(String, u32)>>,
    }

    impl IconSource for FakeIcons {
        fn begin(&self) {
            self.begins.set(self.begins.get() + 1);
            self.asked.borrow_mut().clear();
        }

        fn icon(&self, item: &Item, px: u32) -> Icon {
            self.asked.borrow_mut().push((item.title.clone(), px));
            if item.title.ends_with('1') || item.title.ends_with('3') {
                Icon::Pixels(std::sync::Arc::new(
                    transom_core::loader::Bitmap::new(
                        px,
                        px,
                        vec![[255, 0, 0, 255]; (px * px) as usize],
                    )
                    .unwrap(),
                ))
            } else {
                Icon::Builtin(builtin_for(item.kind))
            }
        }

        fn end(&self) {
            self.ends.set(self.ends.get() + 1);
        }
    }

    #[test]
    fn result_icons_are_drawn_in_the_exact_pixel_size_for_the_dpi_for_visible_rows_only_and_never_change_the_names(
    ) {
        let _g = crate::test_lang::guard();
        let kinds = [
            Kind::App,
            Kind::File,
            Kind::Folder,
            Kind::Setting,
            Kind::Action,
            Kind::Note,
            Kind::Shelf,
            Kind::App,
        ];
        let hits: Vec<Hit> = (0..30)
            .map(|i| {
                hit(
                    &format!("Result {i}"),
                    kinds[i % kinds.len()],
                    Target::Path(format!("C:\\x\\r{i}.txt")),
                )
            })
            .collect();
        for (dpi, dark) in [(96u32, true), (120, false), (144, true), (192, false)] {
            let th = if dark { Theme::dark() } else { Theme::light() };
            let sc = dpi as f32 / 96.0;
            let (w, h) = (
                (LAUNCHER_W_LP as f32 * sc).round() as i32,
                (LAUNCHER_H_LP as f32 * sc).round() as i32,
            );
            let mut p = LauncherPanel::default();
            p.set_results(1, hits.clone(), true);
            let surf = transom_render::offscreen::Surface::new(w, h);
            let mut gp =
                transom_render::GdiPainter::new(surf.dc, transom_render::Size { w, h }, dpi);
            // bez zdroja ikon (testy, zlyhanie): nazvy pre citacku su rovnake
            p.paint(&mut gp, &th);
            let plain: Vec<String> = p.a11y().0.into_iter().map(|e| e.name).collect();
            let fake = FakeIcons {
                begins: 0.into(),
                ends: 0.into(),
                asked: Default::default(),
            };
            p.paint_with(&mut gp, &th, Some(&fake));
            let with: Vec<String> = p.a11y().0.into_iter().map(|e| e.name).collect();
            assert_eq!(
                plain, with,
                "{dpi}: ikony nemenia nazvy ani poradie uzlov pristupnosti"
            );
            // jedno kreslenie = jedno begin / end; ikona sa pyta len pre kreslene riadky v presnej velkosti (24 lp * DPI)
            assert_eq!((fake.begins.get(), fake.ends.get()), (1, 1));
            let want_px = (24.0 * sc).round() as u32;
            let asked = fake.asked.borrow().clone();
            assert!(
                !asked.is_empty() && asked.len() <= p.visible,
                "{dpi}: {}",
                asked.len()
            );
            assert!(
                asked.iter().all(|(_, px)| *px == want_px),
                "{dpi}: {asked:?}"
            );
            assert!(asked.len() < hits.len(), "riadky mimo okna sa nepytaju");
            // hotova ikona je naozaj nakreslena (cervene pixely v stlpci ikony druheho riadku)
            let rgb = surf.rgb();
            let at = |x: i32, y: i32| {
                let i = ((y * w + x) * 3) as usize;
                (rgb[i], rgb[i + 1], rgb[i + 2])
            };
            let (_, r1) = p
                .a11y()
                .0
                .into_iter()
                .zip(p.a11y().1)
                .find(|(e, _)| e.name.starts_with("Result 1,"))
                .map(|(e, x)| (x, e))
                .unwrap();
            let (cx, cy) = (
                r1.x + (10.0 * sc).round() as i32 + want_px as i32 / 2,
                r1.y + r1.h / 2,
            );
            assert_eq!(
                at(cx, cy),
                (255, 0, 0),
                "{dpi}: hotova ikona je v strede stlpca ikony"
            );
            // riadok so vstavanou ikonou ma v tom istom mieste farbu ikony (nie pozadia)
            let (_, r0) = p
                .a11y()
                .0
                .into_iter()
                .zip(p.a11y().1)
                .find(|(e, _)| e.name.starts_with("Result 0,"))
                .map(|(e, x)| (x, e))
                .unwrap();
            let col = |x: i32, y: i32| at(x, y);
            let bg = col(r0.x + 3, r0.y + 3);
            let ix0 = r0.x + (10.0 * sc).round() as i32;
            let drawn = (0..want_px as i32)
                .flat_map(|dy| (0..want_px as i32).map(move |dx| (dx, dy)))
                .any(|(dx, dy)| col(ix0 + dx, r0.y + (r0.h - want_px as i32) / 2 + dy) != bg);
            assert!(drawn, "{dpi}: vstavana ikona je nakreslena");
        }
    }

    #[test]
    fn the_layout_audit_stays_clean_with_icons_in_the_row() {
        use transom_render::audit::Audit;
        let _g = crate::test_lang::guard();
        let long = "Velmi dlhy nazov suboru s mnohymi slovami ".repeat(4);
        let mut bad = Vec::new();
        for lang in [
            transom_i18n::Lang::En,
            transom_i18n::Lang::Sk,
            transom_i18n::Lang::De,
            transom_i18n::Lang::Ja,
        ] {
            transom_i18n::set_lang(lang);
            for dpi in [96u32, 144, 192] {
                for dark in [true, false] {
                    let th = if dark { Theme::dark() } else { Theme::light() };
                    let sc = dpi as f32 / 96.0;
                    let (w, h) = (
                        (LAUNCHER_W_LP as f32 * sc).round() as i32,
                        (LAUNCHER_H_LP as f32 * sc).round() as i32,
                    );
                    let mut p = LauncherPanel {
                        query: long.clone(),
                        ..Default::default()
                    };
                    let kinds = [
                        Kind::App,
                        Kind::File,
                        Kind::Folder,
                        Kind::Setting,
                        Kind::Action,
                        Kind::Note,
                        Kind::Shelf,
                        Kind::App,
                    ];
                    p.set_results(
                        1,
                        (0..8)
                            .map(|i| {
                                hit(
                                    &format!("{i} {long}"),
                                    kinds[i],
                                    Target::Path(format!("C:\\{long}")),
                                )
                            })
                            .collect(),
                        true,
                    );
                    let fake = FakeIcons {
                        begins: 0.into(),
                        ends: 0.into(),
                        asked: Default::default(),
                    };
                    let surf = transom_render::offscreen::Surface::new(w, h);
                    let mut gp = transom_render::GdiPainter::new(
                        surf.dc,
                        transom_render::Size { w, h },
                        dpi,
                    );
                    let mut au = Audit::new(&mut gp, th.background);
                    p.paint_with(&mut au, &th, Some(&fake));
                    au.finish();
                    for i in &au.issues {
                        bad.push(format!("{} {dpi} {dark}: {i:?}", lang.code()));
                    }
                }
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert!(
            bad.is_empty(),
            "{} nalezov, napr.:\n{}",
            bad.len(),
            bad.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
        );
    }

    #[test]
    fn stale_generations_are_ignored_and_the_selection_is_clamped() {
        let mut p = panel();
        p.selected = 1;
        assert!(!p.set_results(0, vec![], true), "starsia generacia");
        assert_eq!(p.results.len(), 2);
        assert!(p.set_results(
            2,
            vec![hit("A", Kind::App, Target::Path("x".into()))],
            false
        ));
        assert_eq!(p.selected, 0);
        assert!(!p.done);
    }

    #[test]
    fn painting_exposes_an_edit_field_and_named_result_items_and_clicks_open_the_row() {
        let _g = crate::test_lang::guard();
        let th = transom_render::Theme::dark();
        let surf = transom_render::offscreen::Surface::new(560, 470);
        let mut painter =
            transom_render::GdiPainter::new(surf.dc, transom_render::Size { w: 560, h: 470 }, 96);
        let mut p = panel();
        p.ch('n');
        // po zmene textu su vysledky zastarane a neotvaraju sa, kym neprídu nove
        assert!(p.stale);
        assert_eq!(
            p.key(0x0D, false).0,
            None,
            "Enter na zastaranom vysledku nic neotvori"
        );
        let fresh = p.results.clone();
        p.set_results(2, fresh, true);
        assert!(!p.stale);
        p.paint(&mut painter, &th);
        let (entries, extras, focus, name) = p.a11y();
        assert_eq!(entries.len(), 3, "pole + 2 vysledky");
        assert_eq!(extras[0].role, ROLE_TEXT);
        assert_eq!(extras[0].value, "n");
        assert!(entries[1].name.contains("Notepad") && entries[1].name.contains("App"));
        assert_ne!(extras[1].state & STATE_SELECTED, 0);
        assert_eq!(focus, Some(1));
        assert!(!name.is_empty());
        let r = &entries[2];
        let (cmd, _) = p.pointer(PointerKind::Click, r.x + 5, r.y + 5);
        assert_eq!(cmd, Some(LauncherCmd::Do(1, Act::Open)));
        let (none, _) = p.pointer(PointerKind::Click, 2, 2);
        assert_eq!(none, None, "klik mimo riadkov nic neotvori");
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use transom_launcher::{Item, Target};

    fn hit(t: &str) -> Hit {
        Hit {
            item: Item::new(t, "d", Kind::File, Target::Path(format!("C:/x/{t}"))),
            score: 1,
        }
    }

    #[test]
    fn the_selection_follows_the_highlighted_item_when_late_results_reorder_the_list() {
        let mut p = LauncherPanel::default();
        p.set_results(1, vec![hit("a"), hit("b"), hit("c")], false);
        p.key(0x28, false); // vybrane "b"
        assert_eq!(p.selected, 1);
        // neskoro prichadzajuce subory vlozia nieco na zaciatok: "b" ostava vybrane
        p.set_results(1, vec![hit("new"), hit("a"), hit("b"), hit("c")], true);
        assert_eq!(p.results[p.selected].item.title, "b");
        // vybrana polozka zmizla: vyber sa vrati na prvu
        p.set_results(1, vec![hit("z")], true);
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn navigation_never_reaches_rows_that_were_not_drawn() {
        let _g = crate::test_lang::guard();
        let th = transom_render::Theme::dark();
        let surf = transom_render::offscreen::Surface::new(560, 300);
        let mut gp =
            transom_render::GdiPainter::new(surf.dc, transom_render::Size { w: 560, h: 300 }, 96);
        let mut p = LauncherPanel::default();
        let many: Vec<Hit> = (0..8).map(|i| hit(&format!("r{i}"))).collect();
        p.set_results(1, many, true);
        p.paint(&mut gp, &th);
        let shown = p.visible;
        assert!(
            (1..8).contains(&shown),
            "okno 300 px nezmesti 8 riadkov: {shown}"
        );
        for _ in 0..20 {
            p.key(0x28, false);
            assert!(
                p.selected < shown,
                "vyber {} je mimo {shown} nakreslenych",
                p.selected
            );
        }
        p.key(0x26, false);
        assert!(p.selected < shown);
    }
}
