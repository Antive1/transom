//! Vybezok odlozenych suborov: po prejdeni mysou nad ikonou v strede listy sa pod listou rozbali zoznam odlozenych
//! suborov. Klik na subor ho skopiruje do schranky, potiahnutim sa prenesie inam (kopia alebo odkaz, nikdy presun),
//! riadok "+N" otvori panel Nastroje. Vybezok je neaktivujuce okno (prejdenie mysou nikomu neberie fokus).
//!
//! Cisty stav, kreslenie a vyhodnotenie mysi (bez Win32); okno a jeho zobrazenie riesi `transom_platform::dragdrop`.
use crate::view::shelf_label;
use transom_core::item::Glyph;
use transom_i18n::{t, tf};
use transom_platform::appbar::PointerKind;
use transom_render::design;
use transom_render::{Font, HAlign, Painter, Rect, TextStyle, Theme};

/// Najviac riadkov suborov vo vybezku (dalsie su v paneli Nastroje cez riadok "+N").
pub const MAX_ROWS: usize = 6;
const HEAD_LP: i32 = 54;
const ROW_LP: i32 = 40;
const MORE_LP: i32 = 34;
const PAD_LP: i32 = 12;
const EMPTY_LP: i32 = 48;
/// Posun mysi (lp), od ktoreho je stlacenie na subore tahanie, nie klik.
const DRAG_LP: i32 = 6;

/// Subor vo vybezku.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub id: String,
    pub name: String,
    pub dir: bool,
    pub available: bool,
}

/// Ciel kliknutia.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    File(String),
    More,
}

/// Co ma aplikacia urobit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Skopirovat subor do schranky.
    Copy(String),
    /// Zacat tahanie suboru von.
    Drag(String),
    /// Otvorit panel Nastroje na odlozenych suboroch.
    Open,
}

#[derive(Default)]
pub struct StashNotch {
    pub hover: Option<Hit>,
    press: Option<(String, i32, i32)>,
    hits: Vec<(Rect, Hit)>,
    scale: f32,
}

/// Vyska vybezku (lp) pre `total` odlozenych suborov.
pub fn height_lp(total: usize) -> i32 {
    if total == 0 {
        return HEAD_LP + EMPTY_LP + PAD_LP;
    }
    let rows = total.min(MAX_ROWS) as i32;
    let more = if total > MAX_ROWS { MORE_LP } else { 0 };
    HEAD_LP + rows * ROW_LP + more + PAD_LP
}

fn st(size: f32, bold: bool, color: transom_render::Color, align: HAlign) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    }
}

impl StashNotch {
    /// Nakresli vybezok: hlavicka (nazov, pocet a napoveda), riadky suborov, pripadne "+N" alebo prazdny stav.
    pub fn paint(&mut self, p: &mut dyn Painter, th: &Theme, files: &[File], total: usize) {
        let s = p.scale();
        self.scale = s;
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        self.hits.clear();
        // podklad a jemny lem (pri skle su to priesvitne plochy, pozri `Theme::glass_pass`)
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, 0, sz.w - 2 * b, sz.h - b), th.background);
        let pad = px(14);
        let w = sz.w - 2 * pad;
        let title = t("tools.sec_shelf");
        p.draw_text(
            &title,
            Rect::new(pad, px(8), w, px(24)),
            &st(14.0, true, th.foreground, HAlign::Left),
        );
        if total > 0 {
            p.draw_text(
                &total.to_string(),
                Rect::new(pad, px(8), w, px(24)),
                &st(13.0, false, th.muted, HAlign::Right),
            );
        }
        let hint = st(12.0, false, th.muted, HAlign::Left);
        let hint_text = if total > 0 {
            t("stash.hint")
        } else {
            t("tools.notch_hint")
        };
        let h = crate::audio_pop::fit(p, &hint_text, &hint, w);
        p.draw_text(&h, Rect::new(pad, px(31), w, px(18)), &hint);
        let mut y = px(HEAD_LP);
        if total == 0 {
            let sub = st(12.0, false, th.muted, HAlign::Center);
            let line = crate::audio_pop::fit(p, &t("tools.notch_sub"), &sub, w);
            p.draw_text(&line, Rect::new(pad, y, w, px(EMPTY_LP - 8)), &sub);
            return;
        }
        for f in files.iter().take(MAX_ROWS) {
            let r = Rect::new(px(8), y, sz.w - px(16), px(ROW_LP - 2));
            let hit = Hit::File(f.id.clone());
            let hot = self.hover.as_ref() == Some(&hit);
            if hot {
                p.fill_round_rect(r, px(design::radius::CONTROL), th.control_hover);
            }
            let fg = if f.available { th.foreground } else { th.muted };
            let glyph = if f.dir { Glyph::Folder } else { Glyph::File };
            p.draw_text(
                &transom_render::scene::glyph_char(glyph).to_string(),
                Rect::new(r.x + px(6), r.y, px(22), r.h),
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 15.0,
                    bold: false,
                    color: fg,
                    align: HAlign::Center,
                },
            );
            let ns = st(13.0, false, fg, HAlign::Left);
            let name = if f.available {
                shelf_label(&f.name)
            } else {
                tf("stash.missing", &[&shelf_label(&f.name)])
            };
            let name = crate::audio_pop::fit(p, &name, &ns, r.w - px(40));
            p.draw_text(&name, Rect::new(r.x + px(34), r.y, r.w - px(40), r.h), &ns);
            self.hits.push((r, hit));
            y += px(ROW_LP);
        }
        if total > MAX_ROWS {
            let r = Rect::new(px(8), y, sz.w - px(16), px(MORE_LP - 4));
            if self.hover == Some(Hit::More) {
                p.fill_round_rect(r, px(design::radius::CONTROL), th.control_hover);
            }
            let more = tf("stash.more", &[&(total - MAX_ROWS).to_string()]);
            p.draw_text(
                &more,
                Rect::new(r.x + px(10), r.y, r.w - px(20), r.h),
                &st(13.0, true, th.accent_text, HAlign::Left),
            );
            self.hits.push((r, Hit::More));
        }
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| h.clone())
    }

    /// Mys vo vybezku: (prikaz, treba prekreslit).
    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32) -> (Option<Cmd>, bool) {
        let here = if kind == PointerKind::Leave {
            None
        } else {
            self.hit(x, y)
        };
        let redraw = here != self.hover;
        self.hover = here.clone();
        let drag_px = ((DRAG_LP as f32) * self.scale.max(1.0)).round() as i32;
        let cmd = match kind {
            PointerKind::Down => {
                self.press = match &here {
                    Some(Hit::File(id)) => Some((id.clone(), x, y)),
                    _ => None,
                };
                None
            }
            PointerKind::Move => match &self.press {
                Some((id, px0, py0)) if (x - px0).abs() > drag_px || (y - py0).abs() > drag_px => {
                    let id = id.clone();
                    self.press = None;
                    Some(Cmd::Drag(id))
                }
                _ => None,
            },
            PointerKind::Click => {
                let pressed = self.press.take();
                match here {
                    Some(Hit::File(id)) if pressed.is_some_and(|(p, _, _)| p == id) => {
                        Some(Cmd::Copy(id))
                    }
                    Some(Hit::More) => Some(Cmd::Open),
                    _ => None,
                }
            }
            PointerKind::Leave | PointerKind::RightClick => {
                self.press = None;
                None
            }
        };
        (cmd, redraw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_render::offscreen::Surface;
    use transom_render::{GdiPainter, Size};

    fn files(n: usize) -> Vec<File> {
        (0..n)
            .map(|i| File {
                id: format!("f{i}"),
                name: format!("report-{i}.pdf"),
                dir: i == 1,
                available: i != 2,
            })
            .collect()
    }

    fn painted(total: usize) -> (StashNotch, Vec<String>) {
        let _g = crate::test_lang::guard();
        let (w, h) = (transom_platform::dragdrop::NOTCH_W_LP, height_lp(total));
        let surf = Surface::new(w, h);
        let mut g = GdiPainter::new(surf.dc, Size { w, h }, 96);
        let mut a = transom_render::audit::Audit::new(&mut g, Theme::dark().background);
        let mut n = StashNotch::default();
        n.paint(&mut a, &Theme::dark(), &files(total.min(MAX_ROWS)), total);
        a.finish();
        let issues = a.issues.iter().map(|i| format!("{i:?}")).collect();
        (n, issues)
    }

    #[test]
    fn the_height_grows_with_the_files_up_to_a_cap_with_a_more_row() {
        assert!(height_lp(0) > HEAD_LP);
        assert!(height_lp(1) < height_lp(3));
        assert_eq!(height_lp(MAX_ROWS) + MORE_LP, height_lp(MAX_ROWS + 5));
        assert_eq!(height_lp(MAX_ROWS + 1), height_lp(100));
    }

    #[test]
    fn rows_are_drawn_without_clipped_text_and_every_file_and_the_more_row_are_clickable() {
        for total in [0, 1, 3, MAX_ROWS, MAX_ROWS + 4] {
            let (n, issues) = painted(total);
            assert!(issues.is_empty(), "{total}: {issues:?}");
            let files_hit = n
                .hits
                .iter()
                .filter(|(_, h)| matches!(h, Hit::File(_)))
                .count();
            assert_eq!(files_hit, total.min(MAX_ROWS), "{total}");
            assert_eq!(
                n.hits.iter().any(|(_, h)| *h == Hit::More),
                total > MAX_ROWS,
                "{total}"
            );
        }
    }

    #[test]
    fn click_copies_drag_takes_the_file_out_and_more_opens_tools() {
        let (mut n, _) = painted(MAX_ROWS + 2);
        let row = |n: &StashNotch, h: &Hit| n.hits.iter().find(|(_, x)| x == h).unwrap().0;
        let r = row(&n, &Hit::File("f0".into()));
        let (cx, cy) = (r.x + 20, r.y + r.h / 2);
        // klik na ten isty subor = kopia
        assert_eq!(n.pointer(PointerKind::Down, cx, cy).0, None);
        assert_eq!(
            n.pointer(PointerKind::Click, cx, cy).0,
            Some(Cmd::Copy("f0".into()))
        );
        // stlacenie a posun = tahanie (a uvolnenie potom nic nekopiruje)
        n.pointer(PointerKind::Down, cx, cy);
        assert_eq!(
            n.pointer(PointerKind::Move, cx + 3, cy).0,
            None,
            "maly posun je stale klik"
        );
        assert_eq!(
            n.pointer(PointerKind::Move, cx + 20, cy).0,
            Some(Cmd::Drag("f0".into()))
        );
        assert_eq!(n.pointer(PointerKind::Click, cx + 20, cy).0, None);
        // stlacenie na jednom, uvolnenie na inom subore = nic
        let r1 = row(&n, &Hit::File("f1".into()));
        n.pointer(PointerKind::Down, cx, cy);
        assert_eq!(n.pointer(PointerKind::Click, r1.x + 20, r1.y + 5).0, None);
        // "+N" otvori Nastroje
        let m = row(&n, &Hit::More);
        assert_eq!(
            n.pointer(PointerKind::Click, m.x + 5, m.y + 5).0,
            Some(Cmd::Open)
        );
        // odchod mysi zrusi zvyraznenie
        assert!(n.pointer(PointerKind::Leave, 0, 0).1 || n.hover.is_none());
        assert_eq!(n.hover, None);
    }
}
