//! Panel "Nastroje" (popover): styri sekcie (Polica, Schranka, Poznamky, Snimky) a tlacidlo hladania. Cisty stav a kreslenie
//! (bez okna): data dodava `App` v `ToolsData`, prikazy (`ToolsCmd`) vykonava `App`, takze sa vsetko da testovat offscreen.
//!
//! Sukromie: panel nic nezbiera sam; kazda sekcia ukazuje, ci je zapnuta, a vysvetli, co sa uklada. Obsah riadkov (nazvy
//! suborov, text schranky, poznamky) sa nikdy nelogue. Mazanie je dvojkrokove (Delete a znova Delete). Sekcia Snimky je
//! **iba kniznica na citanie** (miniatura, nazov, datum): nema mazanie, premenovanie ani pripnutie; snimku sa da otvorit alebo
//! ukazat v priecinku.
use crate::audio_pop::{fit, st};
use transom_core::loader::Bitmap;
use transom_i18n::{t, tf};
use transom_platform::menu_access::{
    MenuEntry, PageExtra, ROLE_LISTITEM, ROLE_PAGETAB, ROLE_PUSHBUTTON, ROLE_STATICTEXT, ROLE_TEXT,
    STATE_OFFSCREEN, STATE_SELECTED,
};
use transom_platform::PointerKind;
use transom_render::design::{self, Kind, Ty};
use transom_render::{HAlign, Painter, Rect, Theme};

pub const TOOLS_W_LP: i32 = 380;
pub const TOOLS_H_LP: i32 = 560;

/// Vyska uvodu s `n` dlazdicami (lp): nadpis a riadky po 5.
pub fn home_h_lp(n: i32) -> i32 {
    let rows = ((n.max(1) + 4) / 5).max(1);
    14 + 38 + rows * 72 + 10
}
/// Najviac tolko riadkov sa kresli a vystavuje cez MSAA (zvysok sa posuva; zoznam je tak ohraniceny).
const MAX_ROWS: usize = 200;
/// Vyska riadku (lp): sekcia Snimky je vyssia, aby miniatura bola citatelna.
const ROW_LP: i32 = 46;
const SHOT_ROW_LP: i32 = 64;
/// Plocha miniatury v riadku snimky (lp).
const THUMB_W_LP: i32 = 76;
const THUMB_H_LP: i32 = 48;

/// Predvolena plocha miniatury pri 100 % (bez obrysu): kym sa nenakresli prvy raz.
fn t_acc_press() -> String {
    t("acc.press")
}

fn thumb_w_lp() -> u32 {
    (THUMB_W_LP - 2) as u32
}

fn thumb_h_lp() -> u32 {
    (THUMB_H_LP - 2) as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Shelf,
    Clipboard,
    Notes,
    Shots,
}

impl Section {
    pub const ALL: [Section; 4] = [
        Section::Shelf,
        Section::Clipboard,
        Section::Notes,
        Section::Shots,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            Section::Shelf => "tools.sec_shelf",
            Section::Clipboard => "tools.sec_clipboard",
            Section::Notes => "tools.sec_notes",
            Section::Shots => "tools.sec_shots",
        }
    }

    fn index(self) -> usize {
        Section::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// Miniatura riadku (len sekcia Snimky): ostatne riadky ju nemaju.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Thumb {
    /// Riadok nema stlpec s miniaturou.
    #[default]
    None,
    /// Nacitava sa (kresli sa neutralna plocha a panel po vykresleni nahlasi, ze ju treba).
    Loading,
    /// Hotova (nepriehladne RGBA, `w x h` px; velkost zodpoveda `ToolsPanel::thumb_box`).
    Ready(std::sync::Arc<Bitmap>),
    /// Bez nahladu (cloud, siet, poskodeny alebo nepodporovany subor): kresli sa vstavana ikona obrazka.
    Placeholder,
}

/// Jeden riadok zoznamu (uz ocisteny text: bez riadiacich a obojsmernych znakov).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Stabilne id polozky v ulozisku (prazdne v testoch): vyber a potvrdenie mazania sleduju id, nie index.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// Pravy stlpec: velkost, cas.
    pub value: String,
    pub pinned: bool,
    /// Zdroj (subor) chyba alebo je nedostupny; polozka sa nemaze, len sa ukaze.
    pub unavailable: bool,
    pub thumb: Thumb,
}

/// Data jednej sekcie.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionView {
    pub rows: Vec<Row>,
    /// Zbieranie / funkcia je zapnuta (pri vypnutej schranke sa vysvetli, ze sa nic neuklada).
    pub enabled: bool,
    /// Riadok stavu (napr. "Pozastavene", "Uloziste je poskodene a nepouzije sa").
    pub status: Option<String>,
    /// Vlastny text prazdneho zoznamu (inak vseobecny podla sekcie): kniznica snimok rozlisuje vypnutu, bez priecinkov a prazdnu.
    pub empty: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ToolsData {
    pub sections: [SectionView; 4],
    /// Skupina dlazdic nad zoznamom (prazdna = bez nej).
    pub group: Vec<Tile>,
}

/// Dlazdica skupiny Nastroje: ikona vo farebnom sklenenom stvorci a nazov.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub id: &'static str,
    /// Znak ikony (Segoe Fluent Icons).
    pub icon: char,
    pub label: String,
    /// Farba ikony a nadychu dlazdice.
    pub color: (u8, u8, u8),
    /// Text namiesto ikony (napr. teplota pocasia).
    pub value: Option<String>,
    /// Karta tohto panela (Schranka, Poznamky, Snimky): klik ju len prepne.
    pub section: Option<Section>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolsHit {
    Search,
    Tab(Section),
    Row(usize),
    NoteNew,
    EdSave,
    EdCopy,
    EdDiscard,
    /// Snimky: otvorit vybranu snimku, ukazat ju v priecinku, znovu nacitat zoznam.
    LibOpen,
    LibFolder,
    LibRefresh,
    /// Polica: pridat subory dialogom, posunut vybranu polozku hore / dole.
    ShelfAdd,
    ShelfUp,
    ShelfDown,
    /// Dlazdica skupiny (poradie).
    Tile(u8),
    /// Spat zo sekcie na dlazdice.
    Back,
}

/// Rozpracovana poznamka (editor v paneli). Text sa upravuje na konci (pisanie, Backspace, Enter = novy riadok, Ctrl+V).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Editor {
    pub id: Option<String>,
    pub text: String,
    pub original: String,
}

impl Editor {
    pub fn dirty(&self) -> bool {
        self.text != self.original
    }
}

/// Co ma `App` vykonat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolsCmd {
    Search,
    /// Hlavna akcia riadku (Enter): skopirovat do schranky, otvorit poznamku, ...
    Activate(Section, usize),
    TogglePin(Section, usize),
    /// Potvrdene (druhy Delete) zmazanie.
    Delete(Section, usize),
    /// Snimky: ukazat vybranu snimku v jej priecinku (Ctrl+Enter).
    ShowInFolder(Section, usize),
    /// Snimky: znovu nacitat zoznam (F5).
    Refresh,
    /// Nova poznamka (otvori editor).
    NoteNew,
    /// Ulozit rozpracovanu poznamku a zavriet editor.
    EditorSave,
    /// Skopirovat text poznamky do schranky.
    EditorCopy,
    /// Zahodit zmeny a zavriet editor.
    EditorDiscard,
    /// Potiahnutie riadku mimo panela: tahanie suborov von (Polica a snimky; len kopia, riadok ani povodny subor sa nemeni ani nemaze).
    DragOut(Section, usize),
    /// Polica: otvori vyber suborov (pridaju sa ako pri pusteni na listu; originaly sa nemenia).
    ShelfAdd,
    /// Polica: posunie polozku o jedno miesto (`-1` hore, `1` dole) v ramci pripnutych / nepripnutych.
    Move(Section, usize, i8),
    /// Dlazdica skupiny (pocasie, schranka, casovac, vyrez...): id.
    Tile(&'static str),
    /// Spat zo sekcie na dlazdice.
    Home,
}

pub struct ToolsPanel {
    /// Uvod s dlazdicami (otvorenie tlacidlom Nastroje); sekcia sa otvori dlazdicou alebo vlastnym tlacidlom v liste.
    pub home: bool,
    /// Dlazdice z posledneho kreslenia: (id, karta panela).
    tiles: Vec<(&'static str, Option<Section>)>,
    /// Plynule posuvanie (snimka: `glide_step`).
    pub glide: transom_render::design::Glide,

    pub section: Section,
    pub selected: usize,
    pub hover: Option<ToolsHit>,
    pub focus: Option<ToolsHit>,
    pressed: Option<ToolsHit>,
    /// Polozka (id), ktora caka na druhy Delete: ak sa zoznam medzitym zmeni (pridanie, pripnutie, presun), potvrdenie ostava
    /// pri tej istej polozke, nie pri indexe.
    confirm_delete: Option<String>,
    /// Vyber nasleduje polozku (id), nie index: po pripnuti, presune alebo pridani ostava na tej istej polozke.
    selected_id: Option<String>,
    /// Id riadkov z posledneho kreslenia (pre klavesnicu).
    ids: Vec<String>,
    press_at: (i32, i32),
    pub editor: Option<Editor>,
    /// Riadky sekcie Snimky, ktore su vidiet a este nemaju miniaturu (indexy; zbiera ich kreslenie, `App` ich po kresleni vybavi).
    wanted: Vec<usize>,
    /// Velkost plochy miniatury v px z posledneho kreslenia (podla DPI); od nej zavisi kluc cache miniatur.
    thumb_box: (u32, u32),
    hits: Vec<(Rect, ToolsHit)>,
    nodes: Vec<(MenuEntry, PageExtra, Option<ToolsHit>)>,
    pub scroll: i32,
    max_scroll: i32,
    view_h: i32,
    /// Po zmene vyberu klavesnicou sa pri dalsom kresleni posunie zoznam tak, aby bol vybrany riadok viditelny.
    want_visible: bool,
    /// Stlaceny riadok sa potiahol (tahanie von): uvolnenie tlacidla nesmie aktivovat nic pod kurzorom.
    dragged: bool,
    /// Mierka DPI z posledneho kreslenia (prah tahania sa skaluje).
    scale: f32,
    row_rects: Vec<Rect>,
    pub msg: Option<String>,
    pub repaint: bool,
}

/// Vybezok pri tahani suborov nad listu: nadpis a vysvetlenie, ze povodne subory ostavaju nedotknute. Rozmer = `dragdrop::NOTCH_*`.
pub fn paint_notch(p: &mut dyn Painter, th: &Theme) {
    let s = p.scale();
    let px = |v: i32| (v as f32 * s).round() as i32;
    let sz = p.size();
    p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
    let b = px(1).max(1);
    p.fill_rect(Rect::new(b, 0, sz.w - 2 * b, sz.h - b), th.background);
    let pad = px(18);
    let w = sz.w - 2 * pad;
    let head = st(15.0, true, th.foreground, HAlign::Center);
    let sub = st(12.0, false, th.muted, HAlign::Center);
    let a = fit(p, &t("tools.notch_hint"), &head, w);
    let c = fit(p, &t("tools.notch_sub"), &sub, w);
    p.draw_text(&a, Rect::new(pad, px(28), w, px(24)), &head);
    p.draw_text(&c, Rect::new(pad, px(58), w, px(20)), &sub);
}

impl Default for ToolsPanel {
    fn default() -> Self {
        ToolsPanel {
            glide: Default::default(),
            tiles: Vec::new(),
            home: false,
            section: Section::Shelf,
            selected: 0,
            hover: None,
            focus: None,
            pressed: None,
            press_at: (0, 0),
            editor: None,
            wanted: Vec::new(),
            thumb_box: (thumb_w_lp(), thumb_h_lp()),
            confirm_delete: None,
            selected_id: None,
            ids: Vec::new(),
            hits: Vec::new(),
            nodes: Vec::new(),
            scroll: 0,
            max_scroll: 0,
            view_h: 0,
            want_visible: false,
            dragged: false,
            scale: 1.0,
            row_rects: Vec::new(),
            msg: None,
            repaint: false,
        }
    }
}

impl ToolsPanel {
    pub fn reset(&mut self, section: Section) {
        *self = ToolsPanel {
            section,
            ..ToolsPanel::default()
        };
    }

    fn empty_text(sec: Section, v: &SectionView) -> String {
        if let Some(e) = &v.empty {
            return e.clone();
        }
        match sec {
            Section::Shelf => t("tools.empty_shelf"),
            Section::Clipboard if !v.enabled => t("tools.empty_clip_off"),
            Section::Clipboard => t("tools.empty_clip"),
            Section::Notes => t("tools.empty_notes"),
            Section::Shots => t("tools.lib_empty"),
        }
    }

    pub fn paint(&mut self, p: &mut dyn Painter, th: &Theme, data: &ToolsData) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        self.scale = s;
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, b, sz.w - 2 * b, sz.h - 2 * b), th.background);
        let pad = px(14);
        let w = sz.w - 2 * pad;
        let line = px(17);
        let view = &data.sections[self.section.index()];
        let rows: &[Row] = &view.rows[..view.rows.len().min(MAX_ROWS)];
        // id riadkov (v testoch bez id sa pouzije poradie), vyber nasleduje polozku
        self.ids = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                if r.id.is_empty() {
                    format!("#{i}")
                } else {
                    r.id.clone()
                }
            })
            .collect();
        if let Some(pos) = self
            .selected_id
            .as_ref()
            .and_then(|id| self.ids.iter().position(|x| x == id))
        {
            self.selected = pos;
        }
        if self.selected >= rows.len() {
            self.selected = rows.len().saturating_sub(1);
        }
        self.selected_id = self.ids.get(self.selected).cloned();
        self.hits.clear();
        self.row_rects.clear();
        let mut nodes: Vec<(Rect, String, PageExtra, bool, Option<ToolsHit>)> = Vec::new();
        let stat = || PageExtra {
            role: ROLE_STATICTEXT,
            ..PageExtra::default()
        };

        if self.editor.is_some() {
            self.paint_editor(p, th);
            return;
        }
        let mut y = pad;
        let modern = !data.group.is_empty();
        self.tiles.clear();
        if modern && self.home {
            // uvod: len skupina dlazdic (nastroje, ktore si pouzivatel zvolil)
            p.draw_text(
                &t("tools.title"),
                Rect::new(pad, y, w, px(26)),
                &st(15.0, true, th.foreground, HAlign::Left),
            );
            nodes.push((
                Rect::new(pad, y, w, px(26)),
                t("tools.title"),
                stat(),
                false,
                None,
            ));
            y += px(38);
            self.paint_tiles(p, th, &data.group, pad, y, w, &mut nodes);
            self.max_scroll = 0;
            self.nodes = nodes
                .into_iter()
                .map(|(r, name, extra, enabled, hit)| {
                    (
                        MenuEntry {
                            name,
                            separator: false,
                            enabled,
                            x: r.x,
                            y: r.y,
                            w: r.w,
                            h: r.h,
                        },
                        extra,
                        hit,
                    )
                })
                .collect();
            return;
        }
        if modern {
            // sekcia: spat na dlazdice a nazov sekcie
            let back = Rect::new(pad - px(6), y, px(30), px(28));
            let hit = ToolsHit::Back;
            if self.hover == Some(hit) {
                p.fill_round_rect(back, px(design::radius::CONTROL), th.control_hover);
            }
            p.draw_text(
                "\u{E76B}",
                back,
                &transom_render::TextStyle {
                    font: transom_render::Font::Icons,
                    size_lp: 12.0,
                    bold: false,
                    color: th.foreground,
                    align: HAlign::Center,
                },
            );
            self.hits.push((back, hit));
            nodes.push((
                back,
                t("tools.back"),
                PageExtra {
                    role: ROLE_PUSHBUTTON,
                    state: 0,
                    value: String::new(),
                    action: t("acc.press"),
                },
                true,
                Some(hit),
            ));
            let title = t(self.section.label_key());
            p.draw_text(
                &title,
                Rect::new(pad + px(30), y, w - px(30), px(28)),
                &st(15.0, true, th.foreground, HAlign::Left),
            );
            nodes.push((
                Rect::new(pad + px(30), y, w - px(30), px(28)),
                title,
                stat(),
                false,
                None,
            ));
            y += px(40);
        } else {
            p.draw_text(
                &t("tools.title"),
                Rect::new(pad, y, w, px(26)),
                &st(15.0, true, th.foreground, HAlign::Left),
            );
            nodes.push((
                Rect::new(pad, y, w, px(26)),
                t("tools.title"),
                stat(),
                false,
                None,
            ));
            y += px(34);
        }

        // nove rozhranie (skupina dlazdic): v sekcii namiesto hladania a kariet hlavicka so spatou sipkou
        if !modern {
            // hladanie (spustac): vyzera ako vyhladavacie pole (lupa + tlmeny text), spravanie aj nazov su tlacidla
            let sr = Rect::new(pad, y, w, px(32));
            {
                let hov = self.hover == Some(ToolsHit::Search);
                design::fill_with_stroke(
                    p,
                    sr,
                    px(design::radius::CONTROL),
                    Some(if hov { th.control_hover } else { th.control }),
                    Some(th.control_edge),
                );
                p.draw_text(
                    "\u{E721}",
                    Rect::new(sr.x + px(10), sr.y, px(16), sr.h),
                    &transom_render::TextStyle {
                        font: transom_render::Font::Icons,
                        size_lp: 12.0,
                        bold: false,
                        color: th.muted,
                        align: HAlign::Center,
                    },
                );
                let ls = Ty::Body.style(th.muted, HAlign::Left);
                let label = fit(p, &t("tools.search"), &ls, sr.w - px(48));
                p.draw_text(
                    &label,
                    Rect::new(sr.x + px(36), sr.y, sr.w - px(44), sr.h),
                    &ls,
                );
                if self.focus == Some(ToolsHit::Search) {
                    design::focus_ring(p, th, sr, design::radius::CONTROL, th.background);
                }
                self.hits.push((sr, ToolsHit::Search));
                nodes.push((
                    sr,
                    t("tools.search"),
                    PageExtra {
                        role: ROLE_PUSHBUTTON,
                        state: 0,
                        value: String::new(),
                        action: t("acc.press"),
                    },
                    true,
                    Some(ToolsHit::Search),
                ));
            }
            y += px(42);

            // karty sekcii: sirka podla textu (zvysok rovnomerne); dlhe preklady najprv mensim pismom, potom na dva riadky
            let labels: Vec<String> = Section::ALL.iter().map(|s| t(s.label_key())).collect();
            let n = labels.len() as i32;
            let mut size = 12.0;
            let mut two_lines = false;
            let natural = |p: &mut dyn Painter, size: f32| -> Vec<i32> {
                labels
                    .iter()
                    .map(|l| {
                        p.measure_text(l, &st(size, true, th.foreground, HAlign::Center))
                            .w
                            + px(14)
                    })
                    .collect()
            };
            let mut nat = natural(p, size);
            if nat.iter().sum::<i32>() > w {
                size = 11.0;
                nat = natural(p, size);
            }
            if nat.iter().sum::<i32>() > w {
                two_lines = true;
                nat = vec![w / n; n as usize];
            }
            let extra = (w - nat.iter().sum::<i32>()).max(0) / n;
            let widths: Vec<i32> = nat.iter().map(|v| v + extra).collect();
            let tab_h = if two_lines { px(42) } else { px(30) };
            let mut x = pad;
            for (k, sec) in Section::ALL.into_iter().enumerate() {
                let tab_w = widths[k];
                let r = Rect::new(x, y, tab_w, tab_h);
                let sel = sec == self.section;
                let hit = ToolsHit::Tab(sec);
                let hov = self.hover == Some(hit);
                if hov && !sel {
                    p.fill_round_rect(r, px(design::radius::CONTROL), th.control_hover);
                }
                let ls = st(
                    size,
                    sel,
                    if sel { th.foreground } else { th.muted },
                    HAlign::Center,
                );
                let lines = if two_lines {
                    transom_render::painter::wrap_text(p, &labels[k], &ls, r.w - px(6))
                        .into_iter()
                        .take(2)
                        .collect::<Vec<_>>()
                } else {
                    vec![fit(p, &labels[k], &ls, r.w - px(6))]
                };
                let lh = (tab_h - px(6)) / lines.len().max(1) as i32;
                for (i, l) in lines.iter().enumerate() {
                    let l = fit(p, l, &ls, r.w - px(6));
                    p.draw_text(&l, Rect::new(r.x, r.y + i as i32 * lh, r.w, lh), &ls);
                }
                if sel {
                    // vybrana karta: polotucne + kratka akcentova znacka pod textom (nie len farba)
                    let lw = lines
                        .iter()
                        .map(|l| p.measure_text(l, &ls).w)
                        .max()
                        .unwrap_or(0)
                        .min(r.w - px(12))
                        .max(px(16));
                    p.fill_round_rect(
                        Rect::new(r.x + (r.w - lw) / 2, r.y + r.h - px(3), lw, px(3)),
                        px(2),
                        th.accent,
                    );
                }
                if self.focus == Some(hit) {
                    design::focus_ring_inset(p, th, r, design::radius::CONTROL, th.background);
                }
                self.hits.push((r, hit));
                nodes.push((
                    r,
                    labels[k].clone(),
                    PageExtra {
                        role: ROLE_PAGETAB,
                        state: if sel { STATE_SELECTED } else { 0 },
                        value: String::new(),
                        action: t("acc.select"),
                    },
                    true,
                    Some(hit),
                ));
                x += tab_w;
            }
            y += tab_h + px(8);
        }

        // Polica: pridat subory dialogom a zmenit poradie (rovnake akcie ako klavesy Ctrl+O a Ctrl+hore / dole)
        if self.section == Section::Shelf && view.enabled {
            let gap = px(8);
            let aw = (w - 2 * gap) * 2 / 5;
            let bw = (w - 2 * gap - aw) / 2;
            let r1 = Rect::new(pad, y, aw, px(32));
            self.button(
                p,
                th,
                r1,
                &t("tools.shelf_add"),
                ToolsHit::ShelfAdd,
                &mut nodes,
            );
            let r2 = Rect::new(pad + aw + gap, y, bw, px(32));
            self.button(
                p,
                th,
                r2,
                &t("tools.shelf_up"),
                ToolsHit::ShelfUp,
                &mut nodes,
            );
            let r3 = Rect::new(pad + aw + gap + bw + gap, y, bw, px(32));
            self.button(
                p,
                th,
                r3,
                &t("tools.shelf_down"),
                ToolsHit::ShelfDown,
                &mut nodes,
            );
            y += px(38);
        }

        // snimky: otvorit, ukazat v priecinku, obnovit (kniznica iba cita; ziadna akcia nic nemeni)
        if self.section == Section::Shots && view.enabled {
            let gap = px(8);
            let bw = (w - 2 * gap) / 3;
            for (i, (key, hit)) in [
                ("tools.lib_open", ToolsHit::LibOpen),
                ("tools.lib_folder", ToolsHit::LibFolder),
                ("tools.lib_refresh", ToolsHit::LibRefresh),
            ]
            .into_iter()
            .enumerate()
            {
                let r = Rect::new(pad + i as i32 * (bw + gap), y, bw, px(32));
                self.button(p, th, r, &t(key), hit, &mut nodes);
            }
            y += px(38);
        }

        // nova poznamka (len v sekcii Poznamky)
        if self.section == Section::Notes && view.enabled {
            let r = Rect::new(pad, y, w, px(32));
            self.button(
                p,
                th,
                r,
                &t("tools.note_new"),
                ToolsHit::NoteNew,
                &mut nodes,
            );
            y += px(38);
        }

        // pevna paticka: stav sekcie, potvrdenie mazania a sprava su vzdy viditelne (nie pod zoznamom, kde by pri dlhom zozname
        // zmizli); zoznam sa kresli a klika len v svojom okne
        let foot_style = st(11.0, false, th.muted, HAlign::Left);
        let msg_style = st(11.0, true, th.accent_text, HAlign::Left);
        // potvrdenie mazania: vyrazne (polotucne, farba nebezpecenstva), nie tlmena poznamka
        let confirm_style = st(11.0, true, th.danger, HAlign::Left);
        let mut foot: Vec<(String, Vec<String>, u8)> = Vec::new();
        let mut foot_lines = 0usize;
        let texts: Vec<(String, u8)> = view
            .status
            .iter()
            .cloned()
            .map(|t| (t, 0))
            .chain(self.confirm_text(rows).map(|t| (t, 2)))
            .chain(self.msg.clone().map(|m| (m, 1)))
            .collect();
        let pick = |k: u8| match k {
            1 => &msg_style,
            2 => &confirm_style,
            _ => &foot_style,
        };
        for (text, strong) in texts {
            if foot_lines >= 4 {
                break;
            }
            let stl = pick(strong);
            let lines: Vec<String> = transom_render::painter::wrap_text(p, &text, stl, w)
                .into_iter()
                .take(2.min(4 - foot_lines))
                .collect();
            foot_lines += lines.len();
            foot.push((text, lines, strong));
        }
        let foot_h = foot_lines as i32 * line;
        let foot_top = sz.h - pad - foot_h;
        let list_bottom = if foot_h > 0 {
            foot_top - px(6)
        } else {
            sz.h - pad
        };
        let vp = Rect::new(0, y, sz.w, (list_bottom - y).max(px(40)));
        self.view_h = vp.h;
        // vyska riadku sa riadi sekciou: tu je jedine miesto, odkial ju berie kreslenie aj posun po klavesnici
        let rh = px(if self.section == Section::Shots {
            SHOT_ROW_LP
        } else {
            ROW_LP
        });
        // vyber po klavesnici musi byt viditelny
        if std::mem::take(&mut self.want_visible) && !rows.is_empty() {
            let rt = self.selected as i32 * (rh + px(2));
            if rt < self.scroll {
                self.scroll = rt;
            } else if rt + rh > self.scroll + vp.h {
                self.scroll = rt + rh - vp.h;
            }
            self.scroll = self.scroll.max(0);
        }
        // dekoduje sa do plochy bez 1 px obrysu zo vsetkych stran
        self.thumb_box = (
            (px(THUMB_W_LP) - 2).max(1) as u32,
            (px(THUMB_H_LP) - 2).max(1) as u32,
        );
        self.wanted.clear();
        let top = y - self.scroll;
        let mut cy = top;
        p.push_clip(vp);
        if rows.is_empty() {
            let text = Self::empty_text(self.section, view);
            let ms = st(12.0, false, th.muted, HAlign::Left);
            let r0 = cy;
            for l in transom_render::painter::wrap_text(p, &text, &ms, w)
                .into_iter()
                .take(6)
            {
                p.draw_text(&l, Rect::new(pad, cy, w, line), &ms);
                cy += line;
            }
            nodes.push((Rect::new(pad, r0, w, cy - r0), text, stat(), false, None));
            cy += px(8);
        }
        for (i, row) in rows.iter().enumerate() {
            let r = Rect::new(pad, cy, w, rh);
            let sel = i == self.selected;
            let hit = ToolsHit::Row(i);
            let armed =
                self.confirm_delete.is_some() && self.confirm_delete == self.ids.get(i).cloned();
            let rad = px(design::radius::CONTROL + 2);
            let row_bg = if armed {
                // druhy krok mazania: riadok ma cerveny nadych a okraj (potvrdenie je pri polozke, nie len v paticke)
                design::fill_with_stroke(p, r, rad, Some(th.danger_tint), Some(th.danger));
                th.danger_tint
            } else if sel {
                p.fill_round_rect(r, rad, th.hover);
                th.hover
            } else if self.hover == Some(hit) {
                let c = transom_render::scene::mix(th.hover, th.background, 0.35);
                p.fill_round_rect(r, rad, c);
                c
            } else {
                th.background
            };
            if sel {
                // vybrany riadok: akcentova znacka vlavo (ako v menu) - vyber nie je len odtien plochy
                p.fill_round_rect(
                    Rect::new(r.x + px(2), r.y + px(12), px(3), r.h - px(24)),
                    px(2),
                    if armed { th.danger } else { th.accent },
                );
            }
            // text na vybranom riadku ma plny kontrast; ostatne riadky maju tlmeny
            let soft = if sel { th.foreground } else { th.muted };
            let vw = if row.value.is_empty() {
                0
            } else {
                p.measure_text(&row.value, &st(12.0, false, soft, HAlign::Right))
                    .w
                    + px(10)
            };
            let tcol = if row.unavailable && !sel {
                th.muted
            } else {
                th.foreground
            };
            let ts = Ty::Body.style(tcol, HAlign::Left);
            // stlpec miniatury (len snimky): text sa posunie vpravo od nej
            let tcol_w = if row.thumb == Thumb::None {
                0
            } else {
                px(THUMB_W_LP) + px(10)
            };
            let tx = r.x + px(12) + tcol_w;
            if row.thumb != Thumb::None {
                let tr = Rect::new(
                    r.x + px(12),
                    r.y + (rh - px(THUMB_H_LP)) / 2,
                    px(THUMB_W_LP),
                    px(THUMB_H_LP),
                );
                paint_thumb(p, th, tr, &row.thumb);
                // riadok je viditelny (aspon ciastocne) a miniatura sa este nacitava: panel to nahlasi `App`
                if row.thumb == Thumb::Loading && r.y + r.h > vp.y - rh && r.y < vp.y + vp.h + rh {
                    self.wanted.push(i);
                }
            }
            let tw_avail = w - px(20) - tcol_w;
            // riadok cakajuci na druhy Delete: znak varovania pred hodnotou (nie len farba plochy)
            let gw = if armed { px(22) } else { 0 };
            if armed {
                p.draw_text(
                    &design::WARNING_GLYPH.to_string(),
                    Rect::new(
                        r.x + r.w - vw - px(8) - gw,
                        r.y + px(if tcol_w > 0 { 10 } else { 4 }),
                        gw,
                        px(20),
                    ),
                    &transom_render::TextStyle {
                        font: transom_render::Font::Icons,
                        size_lp: 13.0,
                        bold: false,
                        color: th.danger,
                        align: HAlign::Center,
                    },
                );
            }
            let title = fit(p, &row.title, &ts, tw_avail - vw - gw);
            p.draw_text(
                &title,
                Rect::new(
                    tx,
                    r.y + px(if tcol_w > 0 { 10 } else { 4 }),
                    tw_avail - vw,
                    px(20),
                ),
                &ts,
            );
            let ss = Ty::Secondary.style(soft, HAlign::Left);
            let mut sub = row.subtitle.clone();
            if row.pinned {
                sub = format!("{} · {}", t("tools.pinned"), sub);
            }
            if row.unavailable {
                sub = format!("{} · {}", t("tools.missing"), sub);
            }
            let sub = fit(p, &sub, &ss, tw_avail);
            p.draw_text(
                &sub,
                Rect::new(
                    tx,
                    r.y + px(if tcol_w > 0 { 30 } else { 24 }),
                    tw_avail,
                    px(18),
                ),
                &ss,
            );
            if vw > 0 {
                p.draw_text(
                    &row.value,
                    Rect::new(
                        r.x + r.w - vw - px(8),
                        r.y + px(if tcol_w > 0 { 10 } else { 4 }),
                        vw,
                        px(20),
                    ),
                    &st(12.0, false, soft, HAlign::Right),
                );
            }
            if self.focus == Some(hit) {
                let fr = if armed {
                    let i = px(2).max(1);
                    Rect::new(r.x + i, r.y + i, r.w - 2 * i, r.h - 2 * i)
                } else {
                    r
                };
                design::focus_ring_inset(p, th, fr, design::radius::CONTROL + 2, row_bg);
            }
            let (vt, vb) = (r.y.max(vp.y), (r.y + r.h).min(vp.y + vp.h));
            if vb > vt {
                self.hits.push((Rect::new(r.x, vt, r.w, vb - vt), hit));
            }
            self.row_rects.push(r);
            let mut name = row.title.clone();
            if row.pinned {
                name = format!("{name}, {}", t("tools.pinned"));
            }
            if row.unavailable {
                name = format!("{name}, {}", t("tools.missing"));
            }
            if row.thumb != Thumb::None && !row.subtitle.is_empty() {
                // citacka precita aj datum (nie je v hodnote riadku, tam je velkost)
                name = format!("{name}, {}", row.subtitle);
            }
            if self.section == Section::Shelf {
                // poradie je dolezite (da sa menit): citacka ho precita pri kazdom riadku a po presune
                name = format!(
                    "{name}, {}",
                    tf(
                        "tools.pos",
                        &[&(i + 1).to_string(), &rows.len().to_string()]
                    )
                );
            }
            nodes.push((
                r,
                name,
                PageExtra {
                    role: ROLE_LISTITEM,
                    state: if sel { STATE_SELECTED } else { 0 },
                    value: row.value.clone(),
                    action: t("acc.select"),
                },
                true,
                Some(hit),
            ));
            cy += rh + px(2);
        }
        p.pop_clip();
        // paticka (oddelena vlasom od zoznamu)
        if foot_h > 0 {
            p.fill_rect(Rect::new(pad, foot_top - px(4), w, px(1).max(1)), th.edge);
        }
        let mut fy = foot_top;
        for (text, lines, strong) in foot {
            let r0 = fy;
            let stl = pick(strong);
            for l in &lines {
                p.draw_text(l, Rect::new(pad, fy, w, line), stl);
                fy += line;
            }
            nodes.push((Rect::new(pad, r0, w, fy - r0), text, stat(), false, None));
        }
        let content_h = cy - top;
        self.max_scroll = (content_h - vp.h).max(0);
        if self.scroll > self.max_scroll {
            self.scroll = self.max_scroll;
            self.repaint = true;
        }
        // posuvnik: len ked sa obsah da posuvat
        transom_render::design::scroll_thumb(p, th, vp, self.scroll, self.max_scroll);
        self.nodes = nodes
            .into_iter()
            .map(|(r, name, mut extra, enabled, hit)| {
                let in_list = matches!(hit, Some(ToolsHit::Row(_)));
                let out = if in_list {
                    r.y < vp.y || r.y + r.h > vp.y + vp.h
                } else {
                    r.y + r.h > sz.h || r.y < 0
                };
                if out {
                    extra.state |= STATE_OFFSCREEN;
                }
                (
                    MenuEntry {
                        name,
                        separator: false,
                        enabled,
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                    },
                    extra,
                    hit,
                )
            })
            .collect();
    }

    /// Viditelne riadky sekcie Snimky bez miniatury z posledneho kreslenia (a vyprazdni zoznam).
    pub fn take_wanted(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.wanted)
    }

    /// Velkost plochy miniatury v px (podla DPI z posledneho kreslenia).
    pub fn thumb_box(&self) -> (u32, u32) {
        self.thumb_box
    }

    fn confirm_text(&self, rows: &[Row]) -> Option<String> {
        let id = self.confirm_delete.as_ref()?;
        let i = self.ids.iter().position(|x| x == id)?;
        rows.get(i).map(|r| tf("tools.delete_confirm", &[&r.title]))
    }

    /// Nastavi vyber (index aj id polozky).
    fn set_selected(&mut self, i: usize) {
        self.selected = i;
        self.selected_id = self.ids.get(i).cloned();
    }

    /// Skupina dlazdic: 5 v riadku, sklenene stvorce (nadych farby nastroja, svetly okraj a lesk hore), pod nimi nazov.
    #[allow(clippy::too_many_arguments)]
    fn paint_tiles(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        tiles: &[Tile],
        x0: i32,
        mut y: i32,
        w: i32,
        nodes: &mut Vec<(Rect, String, PageExtra, bool, Option<ToolsHit>)>,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let cols = 5;
        let cw = w / cols;
        let side = px(48).min(cw - px(10));
        let row_h = side + px(24);
        for (k, t) in tiles.iter().enumerate() {
            let (c, r) = (k as i32 % cols, k as i32 / cols);
            let cell = Rect::new(x0 + c * cw, y + r * row_h, cw, row_h);
            let tile = Rect::new(cell.x + (cw - side) / 2, cell.y, side, side);
            let hit = ToolsHit::Tile(k as u8);
            let hov = self.hover == Some(hit);
            let col = transom_render::Color::rgb(t.color.0, t.color.1, t.color.2);
            let rad = px(14);
            // sklo: nadych farby nad pozadim, svetlejsi pri hover; jemny svetly okraj a lesk v hornej casti
            let base =
                transom_render::scene::mix(col, th.background, if hov { 0.30 } else { 0.20 });
            design::fill_with_stroke(
                p,
                tile,
                rad,
                Some(base),
                Some(transom_render::scene::mix(th.foreground, base, 0.18)),
            );
            let gloss = Rect::new(
                tile.x + px(3),
                tile.y + px(2),
                tile.w - px(6),
                tile.h / 2 - px(2),
            );
            p.fill_round_rect(
                gloss,
                rad - px(3),
                transom_render::scene::mix(th.foreground, base, 0.06),
            );
            match &t.value {
                Some(v) => p.draw_text(v, tile, &st(15.0, false, th.foreground, HAlign::Center)),
                None => p.draw_text(
                    &t.icon.to_string(),
                    tile,
                    &transom_render::TextStyle {
                        font: transom_render::Font::Icons,
                        size_lp: 18.0,
                        bold: false,
                        color: col,
                        align: HAlign::Center,
                    },
                ),
            }
            let ls = st(10.5, false, th.foreground, HAlign::Center);
            let label = fit(p, &t.label, &ls, cw - px(4));
            p.draw_text(
                &label,
                Rect::new(cell.x, tile.y + side + px(3), cw, px(16)),
                &ls,
            );
            if self.focus == Some(hit) {
                design::focus_ring(p, th, tile, 14, th.background);
            }
            self.tiles.push((t.id, t.section));
            self.hits.push((cell, hit));
            nodes.push((
                cell,
                t.label.clone(),
                PageExtra {
                    role: ROLE_PUSHBUTTON,
                    state: 0,
                    value: t.value.clone().unwrap_or_default(),
                    action: t_acc_press(),
                },
                true,
                Some(hit),
            ));
        }
        let rows = (tiles.len() as i32 + cols - 1) / cols;
        y += rows * row_h + px(6);
        y
    }

    fn button(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        r: Rect,
        label: &str,
        hit: ToolsHit,
        nodes: &mut Vec<(Rect, String, PageExtra, bool, Option<ToolsHit>)>,
    ) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let ls = st(12.0, false, th.foreground, HAlign::Center);
        let text = fit(p, label, &ls, r.w - px(16));
        // ikony pomahaju pri prehliadani (nazov tlacidla je vzdy jeho text); ulozenie poznamky je hlavna akcia editora
        let (kind, icon) = match hit {
            // ikona len pri samostatnom sirsom tlacidle; rovnako siroke skupiny (hore/dole, otvorit/priecinok/obnovit) su bez
            // ikon, aby sa pri dlhsom preklade nevynechala len pri niektorom
            ToolsHit::ShelfAdd | ToolsHit::NoteNew => (Kind::Secondary, Some('\u{E710}')),
            ToolsHit::EdSave => (Kind::Primary, None),
            ToolsHit::EdDiscard => (Kind::Destructive, None),
            _ => (Kind::Secondary, None),
        };
        let mut b = design::Button::new(&text, kind).state(design::State {
            hover: self.hover == Some(hit),
            pressed: self.pressed == Some(hit),
            focus: self.focus == Some(hit),
            ..Default::default()
        });
        b.icon = icon;
        design::button(p, th, r, &b, th.background);
        self.hits.push((r, hit));
        nodes.push((
            r,
            label.to_string(),
            PageExtra {
                role: ROLE_PUSHBUTTON,
                state: 0,
                value: String::new(),
                action: t("acc.press"),
            },
            true,
            Some(hit),
        ));
    }

    /// Polozky pre citacky obrazovky: (polozky, role/stavy/hodnoty, index polozky s fokusom, nazov korena).
    pub fn a11y(&self) -> (Vec<MenuEntry>, Vec<PageExtra>, Option<usize>, String) {
        let focus = self
            .focus
            .and_then(|f| self.nodes.iter().position(|n| n.2 == Some(f)));
        (
            self.nodes.iter().map(|n| n.0.clone()).collect(),
            self.nodes.iter().map(|n| n.1.clone()).collect(),
            focus,
            t("tools.title"),
        )
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<ToolsHit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| *h)
    }

    #[cfg(test)]
    pub fn switch_for_test(&mut self, sec: Section) {
        self.switch(sec);
    }

    fn rows_len(&self, data: &ToolsData) -> usize {
        data.sections[self.section.index()].rows.len().min(MAX_ROWS)
    }

    fn switch(&mut self, sec: Section) {
        if sec != self.section {
            self.section = sec;
            self.selected = 0;
            self.selected_id = None;
            self.scroll = 0;
            self.confirm_delete = None;
            self.msg = None;
        }
    }

    fn activate(&mut self, hit: ToolsHit) -> Option<ToolsCmd> {
        match hit {
            ToolsHit::Search => Some(ToolsCmd::Search),
            ToolsHit::Tab(s) => {
                self.switch(s);
                None
            }
            ToolsHit::Row(i) => {
                self.set_selected(i);
                Some(ToolsCmd::Activate(self.section, i))
            }
            ToolsHit::ShelfAdd => Some(ToolsCmd::ShelfAdd),
            ToolsHit::ShelfUp => {
                (!self.ids.is_empty()).then_some(ToolsCmd::Move(Section::Shelf, self.selected, -1))
            }
            ToolsHit::ShelfDown => {
                (!self.ids.is_empty()).then_some(ToolsCmd::Move(Section::Shelf, self.selected, 1))
            }
            ToolsHit::NoteNew => Some(ToolsCmd::NoteNew),
            ToolsHit::EdSave => Some(ToolsCmd::EditorSave),
            ToolsHit::EdCopy => Some(ToolsCmd::EditorCopy),
            ToolsHit::EdDiscard => Some(ToolsCmd::EditorDiscard),
            ToolsHit::LibOpen => {
                (!self.ids.is_empty()).then_some(ToolsCmd::Activate(Section::Shots, self.selected))
            }
            ToolsHit::LibFolder => (!self.ids.is_empty())
                .then_some(ToolsCmd::ShowInFolder(Section::Shots, self.selected)),
            ToolsHit::LibRefresh => Some(ToolsCmd::Refresh),
            ToolsHit::Back => Some(ToolsCmd::Home),
            // vsetky dlazdice riesi aplikacia (sekcia sa otvori v plnej vyske panela)
            ToolsHit::Tile(i) => self.tiles.get(i as usize).map(|(id, _)| ToolsCmd::Tile(id)),
        }
    }

    /// Otvori editor (nova poznamka alebo uprava existujucej).
    pub fn open_editor(&mut self, id: Option<String>, text: String) {
        self.editor = Some(Editor {
            id,
            original: text.clone(),
            text,
        });
        self.focus = None;
        self.repaint = true;
    }

    /// Napisany znak do editora (riadiace znaky ide cez `editor_key`); limit velkosti poznamky.
    pub fn editor_char(&mut self, c: char) -> bool {
        let Some(e) = self.editor.as_mut() else {
            return false;
        };
        let max = transom_tools::notes::MAX_NOTE_BYTES;
        if transom_tools::names::is_unsafe_char(c) || e.text.len() + c.len_utf8() > max {
            return false;
        }
        e.text.push(c);
        true
    }

    /// Vlozeny text (Ctrl+V): bez riadiacich znakov okrem novych riadkov, orezany na limit.
    pub fn editor_paste(&mut self, text: &str) -> bool {
        let Some(e) = self.editor.as_mut() else {
            return false;
        };
        let mut changed = false;
        // poznamka: nove riadky a tabulatory sa zachovaju
        let max = transom_tools::notes::MAX_NOTE_BYTES;
        for c in text.chars() {
            let c = if c == '\r' { continue } else { c };
            if (transom_tools::names::is_unsafe_char(c) && !(c == '\n' || c == '\t'))
                || e.text.len() + c.len_utf8() > max
            {
                continue;
            }
            e.text.push(c);
            changed = true;
        }
        changed
    }

    /// Klavesa v editore: Backspace (Ctrl = posledne slovo), Enter = novy riadok, Ctrl+Enter ulozit, Ctrl+V vlozit (rieši aplikacia).
    pub fn editor_key(&mut self, vk: u32, ctrl: bool) -> (Option<ToolsCmd>, bool) {
        let Some(e) = self.editor.as_mut() else {
            return (None, false);
        };
        match vk {
            0x08 => {
                if ctrl {
                    let t = e.text.trim_end().len();
                    e.text.truncate(t);
                    while e.text.chars().last().is_some_and(|c| !c.is_whitespace()) {
                        e.text.pop();
                    }
                } else {
                    e.text.pop();
                }
                (None, true)
            }
            0x0D if ctrl => (Some(ToolsCmd::EditorSave), true),
            // Ctrl+Delete: zahodit zmeny z klavesnice (Esc a zatvorenie okna ukladaju)
            0x2E if ctrl => (Some(ToolsCmd::EditorDiscard), true),
            0x0D => {
                if e.text.len() < transom_tools::notes::MAX_NOTE_BYTES {
                    e.text.push('\n');
                }
                (None, true)
            }
            _ => (None, false),
        }
    }

    fn paint_editor(&mut self, p: &mut dyn Painter, th: &Theme) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, b, sz.w - 2 * b, sz.h - 2 * b), th.background);
        let pad = px(14);
        let w = sz.w - 2 * pad;
        self.hits.clear();
        self.row_rects.clear();
        let mut nodes: Vec<(Rect, String, PageExtra, bool, Option<ToolsHit>)> = Vec::new();
        let mut y = pad;
        p.draw_text(
            &t("tools.note_title"),
            Rect::new(pad, y, w, px(26)),
            &st(15.0, true, th.foreground, HAlign::Left),
        );
        y += px(32);
        let hint = st(11.0, false, th.muted, HAlign::Left);
        let mut hy = y;
        for l in transom_render::painter::wrap_text(p, &t("tools.note_hint"), &hint, w)
            .into_iter()
            .take(3)
        {
            p.draw_text(&l, Rect::new(pad, hy, w, px(15)), &hint);
            hy += px(15);
        }
        y = hy + px(8);
        // textova plocha
        let area_h = sz.h - y - px(58);
        let area = Rect::new(pad, y, w, area_h.max(px(60)));
        // textove pole: plocha ovladaca s akcentovou spodnou linkou (pole ma fokus)
        p.fill_round_rect(area, px(design::radius::CONTROL), th.control_edge);
        p.fill_round_rect(
            Rect::new(area.x + b, area.y + b, area.w - 2 * b, area.h - 2 * b),
            px(design::radius::CONTROL) - b,
            th.control,
        );
        p.fill_rect(
            Rect::new(
                area.x + px(2),
                area.y + area.h - px(2),
                area.w - px(4),
                px(2),
            ),
            th.accent,
        );
        let ts = st(12.0, false, th.foreground, HAlign::Left);
        let lh = p.measure_text("Ag", &ts).h + px(2);
        let inner = Rect::new(
            area.x + px(10),
            area.y + px(8),
            area.w - px(20),
            area.h - px(16),
        );
        let text = self
            .editor
            .as_ref()
            .map(|e| e.text.clone())
            .unwrap_or_default();
        let mut lines: Vec<String> = Vec::new();
        for para in text.split('\n') {
            let wrapped = transom_render::painter::wrap_text(p, para, &ts, inner.w);
            if wrapped.is_empty() {
                lines.push(String::new());
            } else {
                lines.extend(wrapped);
            }
        }
        let fit_lines = (inner.h / lh).max(1) as usize;
        let start = lines.len().saturating_sub(fit_lines);
        let mut cy = inner.y;
        let mut last_w = 0;
        for (i, l) in lines[start..].iter().enumerate() {
            p.draw_text(l, Rect::new(inner.x, cy, inner.w, lh), &ts);
            if start + i + 1 == lines.len() {
                last_w = p.measure_text(l, &ts).w;
            }
            cy += lh;
        }
        // kurzor na konci textu
        let caret_y = inner.y + (lines.len() - start - 1) as i32 * lh;
        p.fill_rect(
            Rect::new(
                (inner.x + last_w + px(1)).min(inner.x + inner.w - px(2)),
                caret_y,
                px(2).max(1),
                lh - px(2),
            ),
            th.accent_text,
        );
        nodes.push((
            area,
            t("tools.note_title"),
            PageExtra {
                role: ROLE_TEXT,
                state: 0,
                value: text,
                action: String::new(),
            },
            true,
            None,
        ));
        // sprava (napr. limit poznamok): editor ostava otvoreny, text sa nestrati
        if let Some(m) = self.msg.clone() {
            let ms = st(11.0, false, th.foreground, HAlign::Left);
            let m = fit(p, &m, &ms, w);
            p.draw_text(
                &m,
                Rect::new(pad, sz.h - pad - px(32) - px(20), w, px(16)),
                &ms,
            );
        }
        // tlacidla
        let by = sz.h - pad - px(32);
        let bw = (w - 2 * px(8)) / 3;
        let labels = [
            (t("tools.note_save"), ToolsHit::EdSave),
            (t("tools.note_copy"), ToolsHit::EdCopy),
            (t("tools.note_discard"), ToolsHit::EdDiscard),
        ];
        for (i, (label, hit)) in labels.into_iter().enumerate() {
            let r = Rect::new(pad + i as i32 * (bw + px(8)), by, bw, px(32));
            self.button(p, th, r, &label, hit, &mut nodes);
        }
        self.nodes = nodes
            .into_iter()
            .map(|(r, name, extra, enabled, hit)| {
                (
                    MenuEntry {
                        name,
                        separator: false,
                        enabled,
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                    },
                    extra,
                    hit,
                )
            })
            .collect();
    }

    /// Mys: vrati prikaz a to, ci treba prekreslit. Uvolnenie bez predchadzajuceho stlacenia (aktivacia cez MSAA
    /// `accDoDefaultAction` posiela len "klik") sa berie ako aktivacia prvku pod bodom.
    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32) -> (Option<ToolsCmd>, bool) {
        let h = if kind == PointerKind::Leave {
            None
        } else {
            self.hit(x, y)
        };
        let mut redraw = h != self.hover;
        self.hover = h;
        let mut cmd = None;
        match kind {
            PointerKind::Down => {
                self.focus = None;
                self.confirm_delete = None;
                if let Some(ToolsHit::Row(i)) = h {
                    self.set_selected(i);
                }
                self.pressed = h;
                self.dragged = false;
                self.press_at = (x, y);
                redraw = true;
            }
            PointerKind::Move => {
                // potiahnutie stlaceneho riadku o par bodov = tahanie von (klik bez pohybu ostava aktivaciou)
                if let Some(ToolsHit::Row(i)) = self.pressed {
                    let (dx, dy) = (x - self.press_at.0, y - self.press_at.1);
                    let thr = (6.0 * self.scale).max(6.0);
                    if (dx * dx + dy * dy) as f32 > thr * thr {
                        self.pressed = None;
                        self.dragged = true;
                        cmd = Some(ToolsCmd::DragOut(self.section, i));
                        redraw = true;
                    }
                }
            }
            PointerKind::Click => {
                // po potiahnuti (tahanie von) uvolnenie tlacidla nic neaktivuje, bez ohladu na to, co je pod kurzorom
                if !std::mem::take(&mut self.dragged) {
                    if let Some(hit) = h {
                        if self.pressed.is_none() || self.pressed == Some(hit) {
                            cmd = self.activate(hit);
                        }
                    }
                }
                self.pressed = None;
                redraw = true;
            }
            PointerKind::Leave => {
                self.pressed = None;
                redraw = true;
            }
            _ => {}
        }
        (cmd, redraw)
    }

    /// Klavesa: Tab / Shift+Tab (kruh ovladacov), Left/Right prepinaju sekcie na karte, Up/Down vyber riadku,
    /// Enter / medzernik hlavna akcia (opakovanie sa ignoruje), P pripnut, Delete dvojkrokovo (iná klavesa zrusi).
    /// Bez Ctrl (aplikacia vola `key_ctrl`); skratka pre testy.
    #[cfg(test)]
    pub fn key(
        &mut self,
        vk: u32,
        shift: bool,
        repeat: bool,
        data: &ToolsData,
    ) -> (Option<ToolsCmd>, bool) {
        self.key_ctrl(vk, shift, false, repeat, data)
    }

    /// Ako `key`, s klavesom Ctrl: Ctrl+hore / dole posunie vybranu polozku Police, Ctrl+O (aj Insert) prida subory.
    pub fn key_ctrl(
        &mut self,
        vk: u32,
        shift: bool,
        ctrl: bool,
        repeat: bool,
        data: &ToolsData,
    ) -> (Option<ToolsCmd>, bool) {
        let n = self.rows_len(data);
        let pending = self.confirm_delete.take();
        let cur_id = self.ids.get(self.selected).cloned();
        let ring: Vec<ToolsHit> = self.hits.iter().map(|(_, h)| *h).collect();
        let at = self.focus.and_then(|f| ring.iter().position(|h| *h == f));
        let step = |back: bool| -> Option<ToolsHit> {
            if ring.is_empty() {
                return None;
            }
            let len = ring.len();
            Some(match (at, back) {
                (None, false) => ring[0],
                (None, true) => ring[len - 1],
                (Some(i), false) => ring[(i + 1) % len],
                (Some(i), true) => ring[(i + len - 1) % len],
            })
        };
        let mut redraw = pending.is_some();
        let mut cmd = None;
        match vk {
            0x09 => {
                self.focus = step(shift);
                if let Some(ToolsHit::Row(i)) = self.focus {
                    self.set_selected(i);
                    self.want_visible = true;
                }
                redraw = true;
            }
            // Polica: Ctrl+hore / dole zmeni poradie (vyber ostava na tej istej polozke po obnovení zoznamu); Ctrl+O a Insert
            // pridavaju subory (opakovanie sa ignoruje)
            0x26 | 0x28 if ctrl && n > 0 && self.section == Section::Shelf => {
                // drzana kombinacia sa ignoruje cela (nesmie ani menit vyber): jeden stisk = jeden presun
                if !repeat {
                    cmd = Some(ToolsCmd::Move(
                        self.section,
                        self.selected,
                        if vk == 0x28 { 1 } else { -1 },
                    ));
                }
                redraw = true;
            }
            0x4F | 0x2D if !repeat && (vk == 0x2D || ctrl) && self.section == Section::Shelf => {
                cmd = Some(ToolsCmd::ShelfAdd);
                redraw = true;
            }
            0x25 | 0x27 if matches!(self.focus, Some(ToolsHit::Tab(_))) => {
                let i = self.section.index() as i32 + if vk == 0x27 { 1 } else { -1 };
                let sec = Section::ALL[i.rem_euclid(4) as usize];
                self.switch(sec);
                self.focus = Some(ToolsHit::Tab(sec));
                redraw = true;
            }
            0x26 | 0x28 if n > 0 => {
                let i = self.selected as i32 + if vk == 0x28 { 1 } else { -1 };
                self.set_selected(i.clamp(0, n as i32 - 1) as usize);
                self.focus = Some(ToolsHit::Row(self.selected));
                self.want_visible = true;
                redraw = true;
            }
            // snimky: Ctrl+Enter ukaze snimku v jej priecinku (rovnako ako vo vyhladavaci), F5 obnovi zoznam
            0x0D if ctrl && !repeat && n > 0 && self.section == Section::Shots => {
                cmd = Some(ToolsCmd::ShowInFolder(self.section, self.selected));
                redraw = true;
            }
            0x74 if !repeat && self.section == Section::Shots => {
                cmd = Some(ToolsCmd::Refresh);
                redraw = true;
            }
            0x0D | 0x20 if !repeat => {
                if let Some(f) = self.focus {
                    cmd = self.activate(f);
                } else if n > 0 {
                    cmd = Some(ToolsCmd::Activate(self.section, self.selected));
                }
                redraw = true;
            }
            // pripnutie a mazanie sa tyka len poloziek, ktore transom sam uchovava; snimky su cudzie subory a ostavaju nedotknute
            0x50 if n > 0 && !repeat && self.section != Section::Shots => {
                cmd = Some(ToolsCmd::TogglePin(self.section, self.selected));
                redraw = true;
            }
            0x2E if n > 0 && !repeat && self.section != Section::Shots => {
                // potvrdenie plati pre polozku (id), ktoru Delete oznacil, nie pre index
                if pending.is_some() && pending == cur_id {
                    cmd = Some(ToolsCmd::Delete(self.section, self.selected));
                } else {
                    self.confirm_delete = cur_id;
                }
                redraw = true;
            }
            _ => {}
        }
        (cmd, redraw)
    }

    /// Koliesko: posun obsahu.
    pub fn wheel(&mut self, delta: i32) -> bool {
        self.glide.wheel(self.scroll, -delta / 3, self.max_scroll)
    }

    /// Snimka plynuleho posuvania.
    pub fn glide_step(&mut self) -> bool {
        self.glide.step(&mut self.scroll)
    }

    /// Plynuly posun hned do ciela (testy, obmedzene animacie).
    #[cfg(test)]
    pub fn glide_finish(&mut self) {
        self.glide.finish(&mut self.scroll);
    }
}

/// Nakresli plochu miniatury: hotovy obrazok (centrovany, bez dalsieho skalovania), alebo neutralnu plochu so vstavanou
/// ikonou obrazka (nacitava sa / bez nahladu). Obrys oddeluje svetle obrazky od pozadia.
pub(crate) fn paint_thumb(p: &mut dyn Painter, th: &Theme, r: Rect, t: &Thumb) {
    let s = p.scale();
    let px = |v: i32| (v as f32 * s).round() as i32;
    p.fill_round_rect(r, px(4), th.edge);
    let inner = Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2);
    p.fill_round_rect(inner, px(3), th.hover);
    match t {
        Thumb::Ready(b) if b.w > 0 && b.h > 0 && b.w as i32 <= inner.w && b.h as i32 <= inner.h => {
            let (bw, bh) = (b.w as i32, b.h as i32);
            let rect = Rect::new(
                inner.x + (inner.w - bw) / 2,
                inner.y + (inner.h - bh) / 2,
                bw,
                bh,
            );
            let px_: Vec<transom_render::Color> = b
                .rgba
                .iter()
                .map(|c| transom_render::Color::rgb(c[0], c[1], c[2]))
                .collect();
            p.blit(rect, &px_);
        }
        _ => {
            // vstavana ikona obrazka (slnko + hory) vo farbe tlmeneho textu
            let g = (r.h * 5 / 8).max(8);
            let icon = transom_render::resulticons::cached(
                transom_render::resulticons::ResultIcon::Image,
                g as u32,
                th.muted,
            );
            p.blend_rgba(
                Rect::new(r.x + (r.w - g) / 2, r.y + (r.h - g) / 2, g, g),
                &icon,
            );
        }
    }
}
