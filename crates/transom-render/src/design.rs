//! Dizajnovy system transom (jediny zdroj pravdy pre vzhlad ovladacov): typograficka stupnica, rozostupy, polomery
//! a jednotne kreslenie tlacidiel, kariet, zaskrtavacich policok a ramika fokusu. Farby su tokeny temy (`theme.rs`);
//! tu sa iba skladaju do stavov. Vsetko su volne funkcie nad existujucimi primitivami `Painter` (ziadna nova metoda
//! traitu), takze funguju cez kazdy obal (`Track`, `Audit`, `Recorder`, posun obsahu).
//!
//! Pravidla (pozri docs/DESIGN.md):
//! - stav sa nikdy neukazuje len farbou: vybrana volba = vypln + okraj + polotucne pismo, fokus = 2 lp ramik okolo,
//!   nebezpecna akcia = cerveny okraj + text (a jej nazov), zapnute policko = znacka;
//! - pristupny nazov ovladaca je vzdy jeho text (ikona je len vizualna pomocka a do nazvu sa nikdy nepridava);
//! - rozmery su v lp (logickych px pri 96 DPI) a prepocitavaju sa `scale()`; tenke ciary maju aspon 1 px.
use crate::painter::{Color, Font, HAlign, Painter, Rect, TextStyle};
use crate::theme::Theme;

/// Typograficke roly (velkost v lp a hrubka). Pismo vybera backend: Segoe UI Variable (opticke velkosti Small /
/// Text / Display podla velkosti) a skutocne polotucne pismo; zaloha Segoe UI (`gdi.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    /// Nadpis stranky nastaveni.
    PageTitle,
    /// Nadpis sekcie (karty).
    Section,
    /// Podnadpis v sekcii (zoznam, skupina volieb) a nadpis popoveru.
    Subsection,
    /// Popisok ovladaca (zaskrtavacie policko, nazov riadku volieb), hodnota.
    Body,
    /// Zvyraznena hodnota.
    BodyStrong,
    /// Text tlacidla a volby (Small: o kusok sirsie, citatelnejsie pri 12 lp).
    Control,
    /// Popis pod ovladacom, vysvetlivka (tlmena farba).
    Secondary,
    /// Doplnkove metaudaje (cas, velkost, zdroj) a pata.
    Caption,
    /// Odznak (STALE, Primary).
    Badge,
}

impl Ty {
    pub const fn size_lp(self) -> f32 {
        match self {
            Ty::PageTitle => 24.0,
            Ty::Section => 15.0,
            Ty::Subsection => 13.0,
            Ty::Body => 13.0,
            Ty::BodyStrong => 13.0,
            Ty::Control => 12.0,
            Ty::Secondary => 12.0,
            Ty::Caption => 11.0,
            Ty::Badge => 10.0,
        }
    }

    pub const fn bold(self) -> bool {
        matches!(
            self,
            Ty::PageTitle | Ty::Section | Ty::Subsection | Ty::BodyStrong | Ty::Badge
        )
    }

    /// Vyska riadku zalomeneho textu v lp (vyslovna stupnica na mriezke 2 lp).
    pub const fn line_lp(self) -> i32 {
        match self {
            Ty::PageTitle => 32,
            Ty::Section => 22,
            Ty::Subsection | Ty::Body | Ty::BodyStrong => 20,
            Ty::Control | Ty::Secondary => 18,
            Ty::Caption => 16,
            Ty::Badge => 14,
        }
    }

    pub const fn style(self, color: Color, align: HAlign) -> TextStyle {
        TextStyle {
            font: Font::Ui,
            size_lp: self.size_lp(),
            bold: self.bold(),
            color,
            align,
        }
    }
}

/// Rozostupy v lp (stupnica 4).
pub mod space {
    pub const XXS: i32 = 2;
    pub const XS: i32 = 4;
    pub const S: i32 = 8;
    pub const M: i32 = 12;
    pub const L: i32 = 16;
    pub const XL: i32 = 24;
    pub const XXL: i32 = 32;
}

/// Polomery v lp.
pub mod radius {
    /// Polia, cipy, riadky zoznamov, zaskrtavacie policka.
    pub const CONTROL: i32 = 6;
    /// Tlacidla (`button`): mekkejsie nez ostatne ovladace.
    pub const BUTTON: i32 = 8;
    /// Karty sekcii a karty v popoveroch.
    pub const CARD: i32 = 12;
}

/// Vyska tlacidla v lp (cielova plocha >= 24 lp aj pri 100 %).
pub const CONTROL_H_LP: i32 = 32;
/// Hrubka ramika fokusu v lp (plus 1 lp kontrastna medzera k ovladacu).
pub const FOCUS_LP: i32 = 2;
/// Medzera medzi odsekmi v lp.
pub const PARAGRAPH_GAP_LP: i32 = 6;
/// Znak varovania (Segoe Fluent Icons) pri potvrdeni nebezpecnej akcie - druhy znak okrem farby.
pub const WARNING_GLYPH: char = '\u{E7BA}';

/// Druh tlacidla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// Hlavna akcia stranky / dialogu (najviac jedna na skupinu): plna akcentova vypln.
    Primary,
    /// Bezna akcia: jemna vypln a okraj.
    #[default]
    Secondary,
    /// Tichá akcia v hustom zozname: bez vyplne, kym nie je pod mysou.
    Quiet,
    /// Mazanie a ine nevratne akcie: cerveny okraj a text; v druhom kroku (`State::armed`) plna cervena.
    Destructive,
    /// Volba v skupine (segmentovy prepinac): vybrana = akcentovy nadych + akcentovy okraj + polotucne.
    Choice,
}

/// Stav tlacidla (kombinovatelne priznaky).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub hover: bool,
    pub pressed: bool,
    pub focus: bool,
    /// Vybrana volba (`Kind::Choice`).
    pub selected: bool,
    /// Druhy krok nebezpecnej akcie (`Kind::Destructive`): potvrdenie.
    pub armed: bool,
    pub disabled: bool,
}

/// Vysledne farby tlacidla.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub text: Color,
    pub bold: bool,
}

/// Farby tlacidla pre druh a stav (ciste, testovatelne).
pub fn look(th: &Theme, kind: Kind, st: State) -> Look {
    let (hover, pressed) = if st.disabled {
        (false, false)
    } else {
        (st.hover, st.pressed)
    };
    let pick = |rest: Color, hov: Color, prs: Color| {
        if pressed {
            prs
        } else if hover {
            hov
        } else {
            rest
        }
    };
    if st.disabled {
        let filled = !matches!(kind, Kind::Quiet);
        return Look {
            fill: filled.then_some(th.control),
            stroke: filled.then_some(th.edge),
            text: th.disabled,
            bold: kind == Kind::Choice && st.selected,
        };
    }
    match kind {
        Kind::Primary => Look {
            fill: Some(pick(th.accent, th.accent_hover, th.accent_pressed)),
            stroke: None,
            text: th.on_accent,
            bold: true,
        },
        Kind::Secondary => Look {
            fill: Some(pick(th.control, th.control_hover, th.control_pressed)),
            stroke: Some(th.control_edge),
            text: th.foreground,
            bold: false,
        },
        Kind::Quiet => Look {
            fill: (hover || pressed)
                .then(|| pick(th.control, th.control_hover, th.control_pressed)),
            stroke: None,
            text: th.foreground,
            bold: false,
        },
        Kind::Destructive if st.armed => Look {
            fill: Some(pick(th.danger_fill, th.danger_hover, th.danger_hover)),
            stroke: None,
            text: th.on_danger,
            bold: true,
        },
        Kind::Destructive => Look {
            fill: Some(pick(th.control, th.danger_tint, th.danger_tint)),
            stroke: Some(th.danger),
            text: th.danger,
            bold: false,
        },
        Kind::Choice if st.selected => Look {
            fill: Some(th.selected),
            stroke: Some(th.accent),
            text: th.foreground,
            bold: true,
        },
        Kind::Choice => Look {
            fill: Some(pick(th.control, th.control_hover, th.control_pressed)),
            stroke: Some(th.control_edge),
            text: th.foreground,
            bold: false,
        },
    }
}

fn px(p: &dyn Painter, lp: i32) -> i32 {
    (lp as f32 * p.scale()).round() as i32
}

/// Vypln s 1 px okrajom (okraj je vonkajsi prstenec rovnakeho zaobleneho tvaru).
pub fn fill_with_stroke(
    p: &mut dyn Painter,
    r: Rect,
    rad: i32,
    fill: Option<Color>,
    stroke: Option<Color>,
) {
    // okraj 1 lp (1 px pri 100 a 150 %, 2 px pri 200 %): okraj vybranej volby a nebezpecnej akcie ostava zretelny pri DPI
    let b = p.scale().round().max(1.0) as i32;
    if r.w <= 2 * b || r.h <= 2 * b {
        return;
    }
    match (fill, stroke) {
        (Some(f), Some(s)) => {
            p.fill_round_rect(r, rad, s);
            p.fill_round_rect(
                Rect::new(r.x + b, r.y + b, r.w - 2 * b, r.h - 2 * b),
                (rad - b).max(0),
                f,
            );
        }
        (Some(f), None) => p.fill_round_rect(r, rad, f),
        (None, Some(s)) => p.stroke_round_rect(r, rad, b, s),
        (None, None) => {}
    }
}

/// Ramik fokusu klavesnice (dvojfarebny, aby bol viditelny aj na akcentovej alebo cervenej vyplni): 1 lp linka farby
/// plochy `under` tesne okolo `r` a za nou 2 lp `accent_text`. Zaobleny o polomer ovladaca + odstup. Kreslit len pri
/// fokuse z klavesnice; nikdy nenahradza vyber (vyber ma vlastnu vypln a okraj).
pub fn focus_ring(p: &mut dyn Painter, th: &Theme, r: Rect, radius_lp: i32, under: Color) {
    let w = px(p, FOCUS_LP).max(2);
    let gap = px(p, 1).max(1);
    let rad = px(p, radius_lp);
    p.stroke_round_rect(
        Rect::new(r.x - gap, r.y - gap, r.w + 2 * gap, r.h + 2 * gap),
        rad + gap,
        gap,
        under,
    );
    let o = w + gap;
    p.stroke_round_rect(
        Rect::new(r.x - o, r.y - o, r.w + 2 * o, r.h + 2 * o),
        rad + o,
        w,
        th.accent_text,
    );
}

/// Vnutorny ramik fokusu (kde okolo ovladaca nie je miesto - karty, riadky zoznamu, polozky menu): 2 lp `accent_text`
/// pozdlz okraja `r` a 1 lp linka farby `under` vnutri (kontrast aj voci vyplni ovladaca).
pub fn focus_ring_inset(p: &mut dyn Painter, th: &Theme, r: Rect, radius_lp: i32, under: Color) {
    let w = px(p, FOCUS_LP).max(2);
    let gap = px(p, 1).max(1);
    let rad = px(p, radius_lp);
    p.stroke_round_rect(r, rad, w, th.accent_text);
    p.stroke_round_rect(
        Rect::new(r.x + w, r.y + w, r.w - 2 * w, r.h - 2 * w),
        (rad - w).max(0),
        gap,
        under,
    );
}

/// Popis tlacidla.
#[derive(Clone, Copy, Debug)]
pub struct Button<'a> {
    pub label: &'a str,
    /// Znak Segoe Fluent Icons pred textom (len vizualna pomocka; nazov ovladaca je `label`).
    pub icon: Option<char>,
    pub kind: Kind,
    pub state: State,
    /// Velkost textu v lp (predvolene `Ty::Control`).
    pub size_lp: f32,
    /// Ikona a text zarovnane vlavo (riadok zoznamu), inak na stred.
    pub left: bool,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str, kind: Kind) -> Self {
        Button {
            label,
            icon: None,
            kind,
            state: State::default(),
            size_lp: Ty::Control.size_lp(),
            left: false,
        }
    }
    pub fn icon(mut self, c: char) -> Self {
        self.icon = Some(c);
        self
    }
    pub fn state(mut self, s: State) -> Self {
        self.state = s;
        self
    }
    pub fn size(mut self, lp: f32) -> Self {
        self.size_lp = lp;
        self
    }
}

/// Sirka, ktoru tlacidlo potrebuje (text + ikona + vnutorne okraje) v px.
pub fn button_width(p: &mut dyn Painter, b: &Button) -> i32 {
    let l = look_style(b, Color::rgb(0, 0, 0));
    let tw = p.measure_text(b.label, &l).w;
    let iw = if b.icon.is_some() {
        px(p, 16 + space::S)
    } else {
        0
    };
    tw + iw + 2 * px(p, space::L)
}

fn look_style(b: &Button, color: Color) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: b.size_lp,
        bold: matches!(b.kind, Kind::Primary)
            || (b.kind == Kind::Choice && b.state.selected)
            || (b.kind == Kind::Destructive && b.state.armed),
        color,
        align: HAlign::Center,
    }
}

/// Nakresli tlacidlo do `r` na plochu `under`: vypln/okraj podla druhu a stavu, ikona + text na stred, ramik fokusu.
/// Ikona sa vynecha, ak by sa s nou text nezmestil (text sa nikdy neskracuje kvoli ikone). Potvrdenie nebezpecnej akcie
/// bez vlastnej ikony dostane znak varovania (rozlisenie aj bez farby).
pub fn button(p: &mut dyn Painter, th: &Theme, r: Rect, b: &Button, under: Color) {
    let lk = look(th, b.kind, b.state);
    // potvrdenie nebezpecnej akcie: vzdy znak varovania (druhy rozlisovaci znak okrem farby a textu)
    let icon = if b.kind == Kind::Destructive && b.state.armed {
        Some(WARNING_GLYPH)
    } else {
        b.icon
    };
    let rad = px(p, radius::BUTTON);
    fill_with_stroke(p, r, rad, lk.fill, lk.stroke);
    let st = TextStyle {
        bold: lk.bold,
        ..look_style(b, lk.text)
    };
    let tw = p.measure_text(b.label, &st).w;
    let pad = px(p, space::S);
    let icon_w = px(p, 16);
    let gap = px(p, space::S);
    // tlacidlo len s ikonou (bez textu): ikona v strede vzdy
    let icon_only = b.label.is_empty();
    let gap = if icon_only { 0 } else { gap };
    let with_icon = icon.is_some() && (b.left || icon_only || tw + icon_w + gap + 2 * pad <= r.w);
    if b.left {
        // riadok zoznamu: ikona a text od laveho okraja (dlhy text, napr. cesta, sa skrati "...")
        let mut x = r.x + px(p, space::M);
        if let (true, Some(c)) = (with_icon, icon) {
            let mut buf = [0u8; 4];
            p.draw_text(
                c.encode_utf8(&mut buf),
                Rect::new(x, r.y, icon_w, r.h),
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 12.0,
                    bold: false,
                    color: lk.text,
                    align: HAlign::Center,
                },
            );
            x += icon_w + gap;
        }
        p.draw_text(
            b.label,
            Rect::new(x, r.y, (r.x + r.w - pad - x).max(0), r.h),
            &TextStyle {
                align: HAlign::Left,
                ..st
            },
        );
    } else if let (true, Some(c)) = (with_icon, icon) {
        let total = icon_w + gap + tw;
        let x = r.x + (r.w - total) / 2;
        let mut buf = [0u8; 4];
        p.draw_text(
            c.encode_utf8(&mut buf),
            Rect::new(x, r.y, icon_w, r.h),
            &TextStyle {
                font: Font::Icons,
                size_lp: 12.0,
                bold: false,
                color: lk.text,
                align: HAlign::Center,
            },
        );
        p.draw_text(
            b.label,
            Rect::new(x + icon_w + gap, r.y, tw + 1, r.h),
            &TextStyle {
                align: HAlign::Left,
                ..st
            },
        );
    } else {
        p.draw_text(b.label, r, &st);
    }
    if b.state.focus {
        focus_ring(p, th, r, radius::BUTTON, under);
    }
}

/// Karta sekcie: zdvihnuta plocha `surface` s 1 px okrajom `edge`, polomer `radius::CARD`.
pub fn card(p: &mut dyn Painter, th: &Theme, r: Rect) {
    let rad = px(p, radius::CARD);
    fill_with_stroke(p, r, rad, Some(th.surface), Some(th.edge));
    // odlesk na hornej hrane (svetlo zhora; na skle jasny lem ako pri tekutom skle)
    let b = p.scale().round().max(1.0) as i32;
    if r.w > 2 * rad && r.h > 2 * b {
        p.fill_rect(
            Rect::new(r.x + rad, r.y + b, r.w - 2 * rad, b),
            th.control_hover,
        );
    }
}

/// Vnutorne odsadenie karty sekcie v lp (vodorovne, zvisle) a medzera medzi kartami.
pub const SECTION_INSET_LP: (i32, i32) = (12, 10);
pub const SECTION_GAP_LP: i32 = 8;

/// Sekcia panelu ako karta: `body(p, x, w, y)` kresli obsah od `y` v sirke `w` od `x` (uz odsadene) a vrati spodok
/// obsahu. Kreslenie sa zaznamena (`record::Recorder`), pod neho sa nakresli karta presne na vysku obsahu a zaznam sa
/// prehra. Bez obsahu (spodok = zaciatok) sa karta nekresli. Vrati `y` pod kartou vratane medzery k dalsej sekcii.
pub fn section_card(
    p: &mut dyn Painter,
    th: &Theme,
    x: i32,
    w: i32,
    y: i32,
    body: impl FnOnce(&mut dyn Painter, i32, i32, i32) -> i32,
) -> i32 {
    let (ix, iy) = (px(p, SECTION_INSET_LP.0), px(p, SECTION_INSET_LP.1));
    let mut rec = crate::record::Recorder::new(p);
    let end = body(&mut rec, x + ix, w - 2 * ix, y + iy);
    let ops = rec.into_ops();
    if end <= y + iy {
        crate::record::replay(p, ops);
        return y;
    }
    card(p, th, Rect::new(x, y, w, end - y + iy));
    crate::record::replay(p, ops);
    end + iy + px(p, SECTION_GAP_LP)
}

/// Zaskrtavacie policko: zapnute = akcentova vypln + znacka (nie len farba), vypnute = okraj; `hover` zvyrazni okraj.
pub fn checkbox(p: &mut dyn Painter, th: &Theme, r: Rect, on: bool, hover: bool, disabled: bool) {
    let rad = px(p, radius::CONTROL);
    if on {
        let fill = if disabled {
            th.disabled
        } else if hover {
            th.accent_hover
        } else {
            th.accent
        };
        p.fill_round_rect(r, rad, fill);
        p.draw_text(
            "\u{E73E}",
            r,
            &TextStyle {
                font: Font::Icons,
                size_lp: (r.h as f32 / p.scale() * 0.6).round().max(8.0),
                bold: false,
                color: if disabled { th.surface } else { th.on_accent },
                align: HAlign::Center,
            },
        );
    } else {
        // vlasova ciara: 1 px pri 100 a 150 %, 2 px pri 200 %
        let b = p.scale().floor().max(1.0) as i32;
        let edge = if disabled {
            th.disabled
        } else if hover {
            th.foreground
        } else {
            th.muted
        };
        p.fill_round_rect(r, rad, edge);
        p.fill_round_rect(
            Rect::new(r.x + b, r.y + b, r.w - 2 * b, r.h - 2 * b),
            (rad - b).max(0),
            if hover && !disabled {
                th.control_hover
            } else {
                th.control
            },
        );
    }
}

/// Ukazovatel posuvania ako v iOS: tenky zaobleny palec pri pravom okraji `view` (bez drahy). Kresli sa len ked
/// sa obsah da posuvat (`max_scroll > 0`); velkost palca = podiel viditelnej casti, poloha = `scroll`.
pub fn scroll_thumb(p: &mut dyn Painter, th: &Theme, view: Rect, scroll: i32, max_scroll: i32) {
    scroll_thumb_grabbed(p, th, view, scroll, max_scroll, false);
}

/// Ako [`scroll_thumb`]; `grabbed` = palec drzany mysou (sirsi a vyraznejsi).
pub fn scroll_thumb_grabbed(
    p: &mut dyn Painter,
    th: &Theme,
    view: Rect,
    scroll: i32,
    max_scroll: i32,
    grabbed: bool,
) {
    let s = p.scale();
    let Some((track, len, off)) = scroll_track(s, view, max_scroll, scroll) else {
        return;
    };
    let w = if grabbed {
        (6.0 * s).round().max(3.0) as i32
    } else {
        (4.0 * s).round().max(2.0) as i32
    };
    let inset = (3.0 * s).round() as i32;
    p.fill_round_rect(
        Rect::new(view.x + view.w - w - inset, track.y + off, w, len),
        w / 2,
        if grabbed { th.foreground } else { th.muted },
    );
}

/// Draha palca posuvnika: (draha, dlzka palca, posun palca). `None` = obsah sa neposuva.
fn scroll_track(scale: f32, view: Rect, max_scroll: i32, scroll: i32) -> Option<(Rect, i32, i32)> {
    if max_scroll <= 0 || view.h <= 0 {
        return None;
    }
    let px = |v: f32| (v * scale).round() as i32;
    let inset = px(3.0);
    let track = view.h - 2 * inset;
    if track <= px(4.0).max(2) {
        return None;
    }
    let content = view.h as i64 + max_scroll as i64;
    let len = ((track as i64 * view.h as i64 / content) as i32)
        .max(px(24.0))
        .min(track);
    let off =
        ((track - len) as i64 * scroll.clamp(0, max_scroll) as i64 / max_scroll as i64) as i32;
    Some((Rect::new(view.x, view.y + inset, view.w, track), len, off))
}

/// Chytenie posuvnika mysou: kde v palci sa drzi (px od vrchu palca).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollGrab {
    pub off: i32,
}

/// Sirka pasu pri pravom okraji, za ktory sa posuvnik chyta (lp).
pub const SCROLL_GRAB_LP: f32 = 12.0;

/// Stlacenie mysi v bode (`x`, `y`): ak je v pase posuvnika (a obsah sa posuva), chyti ho. Klik na palec ho drzi tam,
/// kde sa chytil; klik vedla palca ho presunie stredom pod mys. Vrati chytenie a novy posun.
pub fn scroll_grab(
    scale: f32,
    view: Rect,
    scroll: i32,
    max_scroll: i32,
    x: i32,
    y: i32,
) -> Option<(ScrollGrab, i32)> {
    let (track, len, off) = scroll_track(scale, view, max_scroll, scroll)?;
    let strip = (SCROLL_GRAB_LP * scale).round() as i32;
    if x < view.x + view.w - strip || x >= view.x + view.w || y < view.y || y >= view.y + view.h {
        return None;
    }
    let top = track.y + off;
    if y >= top && y < top + len {
        return Some((ScrollGrab { off: y - top }, scroll));
    }
    let g = ScrollGrab { off: len / 2 };
    Some((g, scroll_drag(scale, view, max_scroll, g, y)))
}

/// Udalost mysi pre [`ScrollBar`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollEv {
    Down,
    Move,
    Up,
}

/// Posuvnik okna, ktory sa da chytit a tahat mysou: oblast a mierka z posledneho kreslenia a chytenie.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScrollBar {
    pub view: Rect,
    pub scale: f32,
    pub grab: Option<ScrollGrab>,
}

impl ScrollBar {
    /// Nakresli posuvnik (pri kresleni okna) a zapamata si jeho oblast pre mys.
    pub fn paint(&mut self, p: &mut dyn Painter, th: &Theme, view: Rect, scroll: i32, max: i32) {
        self.view = view;
        self.scale = p.scale();
        if max <= 0 {
            self.grab = None;
        }
        scroll_thumb_grabbed(p, th, view, scroll, max, self.grab.is_some());
    }

    /// Mys: `Some(prekreslit)`, ak udalost patri posuvniku (stlacenie v jeho pase, tahanie, uvolnenie po tahani);
    /// `None` = patri obsahu.
    pub fn pointer(
        &mut self,
        ev: ScrollEv,
        x: i32,
        y: i32,
        scroll: &mut i32,
        max: i32,
        glide: &mut Glide,
    ) -> Option<bool> {
        match ev {
            ScrollEv::Down => {
                let (g, s) = scroll_grab(self.scale, self.view, *scroll, max, x, y)?;
                glide.stop();
                self.grab = Some(g);
                *scroll = s;
                Some(true)
            }
            ScrollEv::Move => {
                let g = self.grab?;
                let s = scroll_drag(self.scale, self.view, max, g, y);
                let changed = s != *scroll;
                *scroll = s;
                Some(changed)
            }
            ScrollEv::Up => self.grab.take().map(|_| true),
        }
    }

    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }
}

/// Tahanie chyteneho posuvnika: posun obsahu pre polohu mysi `y`.
pub fn scroll_drag(scale: f32, view: Rect, max_scroll: i32, g: ScrollGrab, y: i32) -> i32 {
    let Some((track, len, _)) = scroll_track(scale, view, max_scroll, 0) else {
        return 0;
    };
    let room = (track.h - len).max(1);
    let pos = (y - g.off - track.y).clamp(0, room);
    ((pos as i64 * max_scroll as i64 + room as i64 / 2) / room as i64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::ratio;

    #[test]
    fn scrollbar_grab_follows_the_mouse_and_reaches_both_ends() {
        let view = Rect::new(0, 0, 300, 400);
        let max = 1200;
        // mimo pasu pri pravom okraji sa nechyta
        assert!(scroll_grab(1.0, view, 0, max, 200, 50).is_none());
        // bez posuvania sa nechyta
        assert!(scroll_grab(1.0, view, 0, 0, 295, 50).is_none());
        // chytenie palca hore: posun sa nemeni, tahanie dole az na koniec
        let (g, s) = scroll_grab(1.0, view, 0, max, 295, 10).expect("palec");
        assert_eq!(s, 0);
        assert_eq!(scroll_drag(1.0, view, max, g, 10), 0);
        assert_eq!(scroll_drag(1.0, view, max, g, 2000), max);
        assert_eq!(scroll_drag(1.0, view, max, g, -50), 0);
        let mid = scroll_drag(1.0, view, max, g, 200);
        assert!(mid > max / 3 && mid < max * 2 / 3, "{mid}");
        // klik pod palec ho presunie pod mys
        let (_, s) = scroll_grab(1.0, view, 0, max, 295, 390).expect("draha");
        assert!(s > max * 9 / 10, "{s}");
    }

    fn states() -> Vec<(&'static str, State)> {
        let mut v = vec![("rest", State::default())];
        v.push((
            "hover",
            State {
                hover: true,
                ..State::default()
            },
        ));
        v.push((
            "pressed",
            State {
                hover: true,
                pressed: true,
                ..State::default()
            },
        ));
        v
    }

    const KINDS: [Kind; 5] = [
        Kind::Primary,
        Kind::Secondary,
        Kind::Quiet,
        Kind::Destructive,
        Kind::Choice,
    ];

    /// Text kazdeho tlacidla ma >= 4,5 : 1 voci svojej vyplni (alebo voci plochе, na ktorej lezi, ak vypln nema) v kazdom
    /// druhu a stave, v oboch temach, na pozadi okna aj na karte.
    #[test]
    fn every_button_kind_and_state_keeps_text_contrast_in_both_themes_on_window_and_card() {
        for th in [Theme::dark(), Theme::light()] {
            for under in [th.background, th.surface] {
                for kind in KINDS {
                    for (sn, mut st) in states() {
                        for (sel, armed) in [(false, false), (true, false), (false, true)] {
                            st.selected = sel;
                            st.armed = armed;
                            let lk = look(&th, kind, st);
                            let bg = lk.fill.unwrap_or(under);
                            let r = ratio(lk.text, bg);
                            assert!(
                                r >= 4.5,
                                "{} {kind:?} {sn} sel={sel} armed={armed}: {r:.2}",
                                if th.dark { "dark" } else { "light" }
                            );
                        }
                    }
                }
            }
        }
    }

    /// Hover a stlacenie su odlisne od pokoja (a od seba) v kazdom povolenom druhu; zakazane tlacidlo na mys nereaguje.
    #[test]
    fn hover_pressed_and_rest_are_distinct_and_disabled_ignores_the_pointer() {
        for th in [Theme::dark(), Theme::light()] {
            for kind in KINDS {
                let rest = look(&th, kind, State::default());
                let hov = look(
                    &th,
                    kind,
                    State {
                        hover: true,
                        ..State::default()
                    },
                );
                let prs = look(
                    &th,
                    kind,
                    State {
                        hover: true,
                        pressed: true,
                        ..State::default()
                    },
                );
                assert_ne!(rest, hov, "{kind:?}");
                if kind != Kind::Destructive {
                    assert_ne!(hov, prs, "{kind:?}");
                }
                let dis = State {
                    disabled: true,
                    ..State::default()
                };
                let d = look(&th, kind, dis);
                let dh = look(
                    &th,
                    kind,
                    State {
                        hover: true,
                        pressed: true,
                        ..dis
                    },
                );
                assert_eq!(d, dh, "{kind:?}: zakazane tlacidlo nereaguje na mys");
                assert_ne!(d.text, rest.text, "{kind:?}: zakazane vyzera inak");
            }
        }
    }

    /// Vybrana volba sa lisi od nevybranej aj bez farby (hrubka pisma) a jej okraj ma >= 3 : 1 voci ploche;
    /// nebezpecne tlacidlo sa lisi od bezneho okrajom aj textom a potvrdenie je plna cervena.
    #[test]
    fn selection_and_danger_are_never_colour_only_and_their_outlines_are_visible() {
        for th in [Theme::dark(), Theme::light()] {
            let off = look(&th, Kind::Choice, State::default());
            let on = look(
                &th,
                Kind::Choice,
                State {
                    selected: true,
                    ..State::default()
                },
            );
            assert!(on.bold && !off.bold);
            for under in [th.background, th.surface] {
                assert!(ratio(on.stroke.unwrap(), under) >= 3.0);
                assert!(ratio(th.danger, under) >= 4.5);
                assert!(ratio(th.accent_text, under) >= 3.0, "fokus");
            }
            let d = look(&th, Kind::Destructive, State::default());
            let s = look(&th, Kind::Secondary, State::default());
            assert_ne!(d.stroke, s.stroke);
            assert_ne!(d.text, s.text);
            let armed = look(
                &th,
                Kind::Destructive,
                State {
                    armed: true,
                    ..State::default()
                },
            );
            assert_eq!(armed.fill, Some(th.danger_fill));
            assert!(armed.bold);
        }
    }

    #[test]
    fn the_type_ramp_is_ordered_and_only_headings_values_and_badges_are_semibold() {
        let order = [
            Ty::PageTitle,
            Ty::Section,
            Ty::Body,
            Ty::Secondary,
            Ty::Caption,
            Ty::Badge,
        ];
        for w in order.windows(2) {
            assert!(w[0].size_lp() > w[1].size_lp(), "{w:?}");
        }
        assert!(!Ty::Body.bold() && !Ty::Secondary.bold() && !Ty::Caption.bold());
        assert!(!Ty::Control.bold());
        assert!(Ty::PageTitle.bold() && Ty::Section.bold() && Ty::BodyStrong.bold());
        // riadok je vyssi nez pismo a vsetky vysky su na mriezke 2 lp (cele px pri 150 %)
        for t in order {
            assert!(t.line_lp() as f32 >= t.size_lp() * 1.3, "{t:?}");
            assert_eq!(t.line_lp() % 2, 0, "{t:?}");
        }
    }
}

/// Plynule posuvanie: koliesko meni ciel, kazda snimka (s obnovou obrazovky) posunie polohu k nemu - exponencialne
/// a nezavisle od frekvencie (60 aj 240 Hz vyzera rovnako, len plynulejsie).
#[derive(Clone, Debug, Default)]
pub struct Glide {
    target: Option<i32>,
    pos: f32,
    last: Option<std::time::Instant>,
}

impl Glide {
    /// Koliesko: posun ciela o `by` px (od rozbehnuteho ciela, inak od `cur`), v rozsahu `0..=max`. `true` = treba
    /// animovat.
    pub fn wheel(&mut self, cur: i32, by: i32, max: i32) -> bool {
        if self.target.is_none() {
            self.pos = cur as f32;
            self.last = None;
        }
        let t = (self.target.unwrap_or(cur) + by).clamp(0, max.max(0));
        if t == cur && self.target.is_none() {
            return false;
        }
        self.target = Some(t);
        true
    }

    /// Zastavi rozbehnute posuvanie (napr. ked sa chyti posuvnik mysou).
    pub fn stop(&mut self) {
        self.target = None;
        self.last = None;
    }

    /// Snimka: posunie `cur` k cielu; `false` = hotovo (alebo nic nebezi).
    pub fn step(&mut self, cur: &mut i32) -> bool {
        let Some(t) = self.target else {
            return false;
        };
        // poloha sa zmenila inak (klavesnica, zmena obsahu): pokracovat od nej
        if self.pos.round() as i32 != *cur {
            self.pos = *cur as f32;
        }
        let now = std::time::Instant::now();
        let dt = self.last.map_or(1.0 / 120.0, |l| {
            now.duration_since(l).as_secs_f32().min(0.05)
        });
        self.last = Some(now);
        self.pos += (t as f32 - self.pos) * (1.0 - (-dt * 18.0).exp());
        if (t as f32 - self.pos).abs() < 0.5 {
            self.pos = t as f32;
            self.target = None;
            self.last = None;
        }
        *cur = self.pos.round() as i32;
        self.target.is_some()
    }

    /// Bez animacie (obmedzeny pohyb vo Windows): hned do ciela.
    pub fn finish(&mut self, cur: &mut i32) {
        if let Some(t) = self.target.take() {
            *cur = t;
            self.pos = t as f32;
        }
    }
}

#[cfg(test)]
mod glide_tests {
    use super::Glide;

    #[test]
    fn wheel_sets_a_clamped_target_and_steps_reach_it() {
        let mut g = Glide::default();
        let mut cur = 0;
        assert!(!g.wheel(cur, -40, 100), "pri vrchu nahor nic");
        assert!(g.wheel(cur, 40, 100));
        assert!(g.wheel(cur, 200, 100), "ciel sa scita a oreze");
        let mut n = 0;
        while g.step(&mut cur) {
            std::thread::sleep(std::time::Duration::from_millis(4));
            n += 1;
            assert!(n < 500);
        }
        assert_eq!(cur, 100);
        assert!(n > 2, "posun prebehol vo viacerych snimkach");
        g.wheel(cur, -30, 100);
        g.finish(&mut cur);
        assert_eq!(cur, 70);
    }
}
