//! Spolocny zaklad jednoduchych panelov (Casovac, Jas, Mikrofon a kamera): panel sa opise riadkami (`Row`) a sada
//! sa postara o kreslenie, mys, klavesnicu (Tab, sipky, Enter/Medzernik, Home/End/PgUp/PgDn na posuvniku), koliesko,
//! posuvanie obsahu a polozky pre citacku. Kazdy ovladac ma `id` (u32); panel dostane spat len prikaz `Cmd`.
//!
//! Cisty stav a kreslenie bez Win32 okna, testovatelne offscreen.
use crate::audio_pop::{fit, st, step_percent};
use transom_core::item::Glyph;
use transom_i18n::t;
use transom_platform::menu_access::{
    MenuEntry, PageExtra, ROLE_CHECKBUTTON, ROLE_LISTITEM, ROLE_PUSHBUTTON, ROLE_SLIDER,
    ROLE_STATICTEXT, STATE_CHECKED, STATE_OFFSCREEN, STATE_SELECTED,
};
/// MSAA `STATE_SYSTEM_UNAVAILABLE`.
const STATE_UNAVAILABLE: i32 = 0x1;
use transom_platform::PointerKind;
use transom_render::{Font, HAlign, Painter, Rect, TextStyle, Theme};

/// Tlacidlo v riadku tlacidiel.
#[derive(Clone, Debug, PartialEq)]
pub struct Btn {
    pub id: u32,
    pub label: String,
    /// Hlavna akcia (akcentova vypln).
    pub primary: bool,
    pub enabled: bool,
    /// Pristupny nazov, ak sa lisi od textu (napr. "Spustit casovac na 5 min").
    pub acc: Option<String>,
    /// Ikona namiesto textu (Segoe Fluent Icons), napr. prehrat / pozastavit.
    pub icon: Option<char>,
}

impl Btn {
    pub fn new(id: u32, label: impl Into<String>) -> Btn {
        Btn {
            id,
            label: label.into(),
            primary: false,
            enabled: true,
            acc: None,
            icon: None,
        }
    }
    pub fn icon(mut self, c: char) -> Btn {
        self.icon = Some(c);
        self
    }
    pub fn primary(mut self) -> Btn {
        self.primary = true;
        self
    }
    pub fn enabled(mut self, on: bool) -> Btn {
        self.enabled = on;
        self
    }
    pub fn acc(mut self, a: impl Into<String>) -> Btn {
        self.acc = Some(a.into());
        self
    }
}

/// Riadok panela.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// Nadpis panela.
    Title(String),
    /// Nadpis sekcie (maly, tlmeny).
    Section(String),
    /// Zalomeny text (tlmeny alebo zvyrazneny).
    Text {
        text: String,
        strong: bool,
    },
    /// Velka hodnota (napr. zostavajuci cas), so zalozenym popisom pre citacku.
    Big {
        text: String,
        acc: String,
    },
    /// Riadok tlacidiel (zalamuje sa).
    Buttons(Vec<Btn>),
    /// Ovladanie ako na hodinkach: prve tlacidlo vlavo a posledne vpravo ako velke kruhy (posledne zvycajne hlavne),
    /// ostatne male v strede.
    Transport(Vec<Btn>),
    /// Predvolby ako zaoblene "pilulky" (rovnomerne v riadku).
    Chips(Vec<Btn>),
    /// Posuvnik 0..=100 s volitelnou ikonou vlavo a hodnotou vpravo.
    Slider {
        id: u32,
        label: String,
        value: u32,
        enabled: bool,
        glyph: Option<Glyph>,
    },
    /// Zaskrtavaci riadok.
    Toggle {
        id: u32,
        label: String,
        on: bool,
    },
    /// Polozka zoznamu len na citanie (ikona, text, vpravo popis).
    Item {
        glyph: Option<Glyph>,
        text: String,
        detail: String,
        selected: bool,
    },
    /// Bod zoznamu (odrazka a zalomeny text, najviac `BULLET_LINES` riadkov, potom elipsa).
    Bullet(String),
    Gap(i32),
}

/// Najviac riadkov jedneho bodu zoznamu.
pub const BULLET_LINES: usize = 3;

/// Prikaz z panela.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Click(u32),
    /// Posuvnik `id` na hodnotu (0..=100).
    Slide(u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Button(u32),
    Slider(u32),
    Toggle(u32),
}

type Node = (Rect, String, PageExtra, bool, Option<Hit>);

#[derive(Default)]
pub struct Kit {
    /// Plynule posuvanie (snimka: `glide_step`).
    pub glide: transom_render::design::Glide,

    hover: Option<Hit>,
    focus: Option<Hit>,
    pressed: Option<Hit>,
    drag: Option<(u32, u32)>,
    hits: Vec<(Rect, Hit)>,
    /// Posuvniky z posledneho kreslenia: (id, draha, hodnota, povoleny).
    tracks: Vec<(u32, Rect, u32, bool)>,
    nodes: Vec<(MenuEntry, PageExtra, Option<Hit>)>,
    name: String,
    pub scroll: i32,
    max_scroll: i32,
    view_h: i32,
    /// Po kresleni treba dokreslit (zmenil sa rozsah posuvania).
    pub repaint: bool,
    /// Otvorene z Nastrojov: v nadpise sipka spat (`Cmd::Click(BACK)`).
    pub back: bool,
    /// Vyska obsahu v px z posledneho kreslenia (vratane okrajov): podla nej sa da zvolit vyska panela.
    pub content_h: i32,
}

/// Id tlacidla spat (nadpis panela otvoreneho z Nastrojov).
pub const BACK: u32 = u32::MAX;

fn extra(role: i32, state: i32, value: String, action: String) -> PageExtra {
    PageExtra {
        role,
        state,
        value,
        action,
    }
}

impl Kit {
    pub fn reset(&mut self) {
        *self = Kit::default();
    }

    /// Hodnota posuvnika pocas tahania (inak `None`).
    pub fn dragging(&self, id: u32) -> Option<u32> {
        self.drag.filter(|d| d.0 == id).map(|d| d.1)
    }

    pub fn paint(&mut self, p: &mut dyn Painter, th: &Theme, rows: &[Row]) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, b, sz.w - 2 * b, sz.h - 2 * b), th.background);
        let pad = px(14);
        let w = sz.w - 2 * pad;
        let line = px(17);
        self.view_h = sz.h;
        let mut y = pad - self.scroll;
        let top = y;
        self.hits.clear();
        self.tracks.clear();
        let mut nodes: Vec<Node> = Vec::new();
        let rad = px(transom_render::design::radius::CONTROL);
        let stat = || extra(ROLE_STATICTEXT, 0, String::new(), String::new());
        for row in rows {
            match row {
                Row::Title(text) => {
                    let mut r = Rect::new(pad, y, w, px(26));
                    if self.back {
                        let b = Rect::new(pad - px(6), y - px(1), px(30), px(28));
                        let hit = Hit::Button(BACK);
                        if self.hover == Some(hit) {
                            p.fill_round_rect(b, rad, th.control_hover);
                        }
                        p.draw_text(
                            "\u{E76B}",
                            b,
                            &TextStyle {
                                font: Font::Icons,
                                size_lp: 12.0,
                                bold: false,
                                color: th.foreground,
                                align: HAlign::Center,
                            },
                        );
                        self.hits.push((b, hit));
                        nodes.push((
                            b,
                            transom_i18n::t("tools.back"),
                            extra(
                                ROLE_PUSHBUTTON,
                                0,
                                String::new(),
                                transom_i18n::t("acc.press"),
                            ),
                            true,
                            Some(hit),
                        ));
                        r = Rect::new(pad + px(30), y, w - px(30), px(26));
                    }
                    let ts = st(15.0, true, th.foreground, HAlign::Left);
                    let shown = fit(p, text, &ts, r.w);
                    p.draw_text(&shown, r, &ts);
                    nodes.push((r, text.clone(), stat(), false, None));
                    if self.name.is_empty() {
                        self.name = text.clone();
                    }
                    y += px(34);
                }
                Row::Section(text) => {
                    let r = Rect::new(pad, y, w, line);
                    let ls = st(11.0, false, th.muted, HAlign::Left);
                    let shown = fit(p, text, &ls, w);
                    p.draw_text(&shown, r, &ls);
                    nodes.push((r, text.clone(), stat(), false, None));
                    y += line + px(4);
                }
                Row::Text { text, strong } => {
                    let ts = if *strong {
                        st(12.0, true, th.foreground, HAlign::Left)
                    } else {
                        st(11.0, false, th.muted, HAlign::Left)
                    };
                    let r0 = y;
                    for l in transom_render::painter::wrap_text(p, text, &ts, w)
                        .into_iter()
                        .take(6)
                    {
                        p.draw_text(&l, Rect::new(pad, y, w, line), &ts);
                        y += line;
                    }
                    nodes.push((
                        Rect::new(pad, r0, w, y - r0),
                        text.clone(),
                        stat(),
                        false,
                        None,
                    ));
                    y += px(4);
                }
                Row::Big { text, acc } => {
                    let r = Rect::new(pad, y, w, px(58));
                    // velke tenke cislice (ako hodinky)
                    let bs = TextStyle {
                        font: Font::Ui,
                        size_lp: 42.0,
                        bold: false,
                        color: th.foreground,
                        align: HAlign::Center,
                    };
                    p.draw_text(text, r, &bs);
                    nodes.push((r, acc.clone(), stat(), false, None));
                    y += px(64);
                }
                Row::Transport(btns) => {
                    let big = px(58);
                    let small = px(30);
                    let n = btns.len();
                    let mids: Vec<&Btn> = btns.iter().skip(1).take(n.saturating_sub(2)).collect();
                    // stredne male tlacidla: sirka podla textu
                    let ls = st(12.0, false, th.foreground, HAlign::Center);
                    let mw: Vec<i32> = mids
                        .iter()
                        .map(|b| p.measure_text(&b.label, &ls).w + px(22))
                        .collect();
                    let gap = px(8);
                    let total: i32 = mw.iter().sum::<i32>() + gap * (mids.len() as i32 - 1).max(0);
                    let mut x = pad + (w - total) / 2;
                    let mut place: Vec<(&Btn, Rect, bool)> = Vec::new();
                    if let Some(first) = btns.first() {
                        place.push((first, Rect::new(pad, y, big, big), true));
                    }
                    for (b, bw) in mids.iter().zip(&mw) {
                        place.push((b, Rect::new(x, y + (big - small) / 2, *bw, small), false));
                        x += bw + gap;
                    }
                    if n > 1 {
                        let last = &btns[n - 1];
                        place.push((last, Rect::new(pad + w - big, y, big, big), true));
                    }
                    for (bt, r, round) in place {
                        let hit = Hit::Button(bt.id);
                        let hov = self.hover == Some(hit) && bt.enabled;
                        let pressed = self.pressed == Some(hit) && hov;
                        let (fill, fg) = if !bt.enabled {
                            (th.control, th.disabled)
                        } else if bt.primary {
                            (
                                if pressed {
                                    th.accent_pressed
                                } else if hov {
                                    th.accent_hover
                                } else {
                                    th.accent
                                },
                                th.on_accent,
                            )
                        } else {
                            (
                                if pressed {
                                    th.control_pressed
                                } else if hov {
                                    th.control_hover
                                } else {
                                    th.control
                                },
                                th.foreground,
                            )
                        };
                        let rr = if round { r.w / 2 } else { r.h / 2 };
                        p.fill_round_rect(r, rr, fill);
                        match bt.icon {
                            Some(c) => p.draw_text(
                                &c.to_string(),
                                r,
                                &TextStyle {
                                    font: Font::Icons,
                                    size_lp: if round { 18.0 } else { 12.0 },
                                    bold: false,
                                    color: fg,
                                    align: HAlign::Center,
                                },
                            ),
                            None => {
                                let ts = st(12.0, false, fg, HAlign::Center);
                                let text = fit(p, &bt.label, &ts, r.w - px(6));
                                p.draw_text(&text, r, &ts);
                            }
                        }
                        if self.focus == Some(hit) {
                            transom_render::design::focus_ring(
                                p,
                                th,
                                r,
                                (rr as f32 / s) as i32,
                                th.background,
                            );
                        }
                        if bt.enabled {
                            self.hits.push((r, hit));
                        }
                        nodes.push((
                            r,
                            bt.acc.clone().unwrap_or_else(|| bt.label.clone()),
                            extra(
                                ROLE_PUSHBUTTON,
                                if bt.enabled { 0 } else { STATE_UNAVAILABLE },
                                String::new(),
                                if bt.enabled {
                                    t("acc.press")
                                } else {
                                    String::new()
                                },
                            ),
                            bt.enabled,
                            Some(hit),
                        ));
                    }
                    y += big + px(14);
                }
                Row::Chips(btns) => {
                    let n = btns.len().max(1) as i32;
                    let gap = px(6);
                    let cw = (w - gap * (n - 1)) / n;
                    let ch = px(28);
                    for (k, bt) in btns.iter().enumerate() {
                        let r = Rect::new(pad + k as i32 * (cw + gap), y, cw, ch);
                        let hit = Hit::Button(bt.id);
                        let hov = self.hover == Some(hit) && bt.enabled;
                        p.fill_round_rect(
                            r,
                            ch / 2,
                            if hov { th.control_hover } else { th.control },
                        );
                        let ts = st(11.5, false, th.foreground, HAlign::Center);
                        let text = fit(p, &bt.label, &ts, r.w - px(6));
                        p.draw_text(&text, r, &ts);
                        if self.focus == Some(hit) {
                            transom_render::design::focus_ring(p, th, r, 14, th.background);
                        }
                        self.hits.push((r, hit));
                        nodes.push((
                            r,
                            bt.acc.clone().unwrap_or_else(|| bt.label.clone()),
                            extra(ROLE_PUSHBUTTON, 0, String::new(), t("acc.press")),
                            true,
                            Some(hit),
                        ));
                    }
                    y += ch + px(12);
                }
                Row::Buttons(btns) => {
                    let ch = px(30);
                    let gap = px(6);
                    let mut x = pad;
                    for bt in btns {
                        let ls = st(12.0, false, th.foreground, HAlign::Center);
                        let bw = (p.measure_text(&bt.label, &ls).w + px(24)).min(w);
                        if x > pad && x + bw > pad + w {
                            x = pad;
                            y += ch + gap;
                        }
                        let r = Rect::new(x, y, bw, ch);
                        let hit = Hit::Button(bt.id);
                        let hov = self.hover == Some(hit) && bt.enabled;
                        let text = fit(p, &bt.label, &ls, r.w - px(12));
                        let kind = if bt.primary {
                            transom_render::design::Kind::Primary
                        } else {
                            transom_render::design::Kind::Secondary
                        };
                        let b = transom_render::design::Button::new(&text, kind).state(
                            transom_render::design::State {
                                hover: hov,
                                pressed: self.pressed == Some(hit) && hov,
                                focus: self.focus == Some(hit),
                                disabled: !bt.enabled,
                                ..Default::default()
                            },
                        );
                        transom_render::design::button(p, th, r, &b, th.background);
                        if bt.enabled {
                            self.hits.push((r, hit));
                        }
                        nodes.push((
                            r,
                            bt.acc.clone().unwrap_or_else(|| bt.label.clone()),
                            extra(
                                ROLE_PUSHBUTTON,
                                if bt.enabled { 0 } else { STATE_UNAVAILABLE },
                                String::new(),
                                if bt.enabled {
                                    t("acc.press")
                                } else {
                                    String::new()
                                },
                            ),
                            bt.enabled,
                            Some(hit),
                        ));
                        x += bw + gap;
                    }
                    y += ch + px(10);
                }
                Row::Slider {
                    id,
                    label,
                    value,
                    enabled,
                    glyph,
                } => {
                    let shown = self.dragging(*id).unwrap_or(*value).min(100);
                    let row_h = px(30);
                    let gw = if glyph.is_some() { px(26) } else { 0 };
                    if let Some(g) = glyph {
                        p.draw_text(
                            &transom_render::scene::glyph_char(*g).to_string(),
                            Rect::new(pad, y, gw, row_h),
                            &TextStyle {
                                font: Font::Icons,
                                size_lp: 14.0,
                                bold: false,
                                color: if *enabled { th.foreground } else { th.muted },
                                align: HAlign::Left,
                            },
                        );
                    }
                    let val_w = px(48);
                    let tr = Rect::new(
                        pad + gw + px(4),
                        y + row_h / 2 - px(7),
                        (w - gw - px(4) - val_w - px(16)).max(px(40)),
                        px(14),
                    );
                    let f = shown as f32 / 100.0;
                    p.fill_round_rect(
                        Rect::new(tr.x, tr.y + tr.h / 2 - px(2), tr.w, px(4)),
                        px(2),
                        th.control_edge,
                    );
                    p.fill_round_rect(
                        Rect::new(
                            tr.x,
                            tr.y + tr.h / 2 - px(2),
                            (tr.w as f32 * f) as i32,
                            px(4),
                        ),
                        px(2),
                        if *enabled { th.accent } else { th.muted },
                    );
                    let tx = tr.x + (tr.w as f32 * f) as i32;
                    p.fill_round_rect(
                        Rect::new(tx - px(7), tr.y - px(2), px(14), tr.h + px(4)),
                        px(7),
                        if *enabled { th.foreground } else { th.muted },
                    );
                    let hit = Hit::Slider(*id);
                    if self.focus == Some(hit) && *enabled {
                        transom_render::design::focus_ring(
                            p,
                            th,
                            Rect::new(tr.x - px(6), tr.y - px(3), tr.w + px(12), tr.h + px(6)),
                            6,
                            th.background,
                        );
                    }
                    p.draw_text(
                        &transom_i18n::tf("snd.volume_value", &[&shown.to_string()]),
                        Rect::new(tr.x + tr.w + px(16), y, val_w, row_h),
                        &st(
                            12.0,
                            true,
                            if *enabled { th.foreground } else { th.muted },
                            HAlign::Left,
                        ),
                    );
                    let sr = Rect::new(tr.x - px(8), y, tr.w + px(16), row_h);
                    if *enabled {
                        self.hits.push((sr, hit));
                    }
                    self.tracks.push((*id, tr, shown, *enabled));
                    nodes.push((
                        sr,
                        label.clone(),
                        extra(
                            ROLE_SLIDER,
                            if *enabled { 0 } else { STATE_UNAVAILABLE },
                            format!("{shown} %"),
                            String::new(),
                        ),
                        *enabled,
                        Some(hit),
                    ));
                    y += row_h + px(6);
                }
                Row::Toggle { id, label, on } => {
                    let ts = st(12.0, false, th.foreground, HAlign::Left);
                    let lines = transom_render::painter::wrap_text(p, label, &ts, w - px(34));
                    let h = (lines.len().min(4) as i32 * line + px(8)).max(px(28));
                    let r = Rect::new(pad, y, w, h);
                    let hit = Hit::Toggle(*id);
                    if self.hover == Some(hit) {
                        p.fill_round_rect(r, rad, th.hover);
                    }
                    let cb = Rect::new(r.x + px(4), r.y + px(5), px(18), px(18));
                    if *on {
                        p.fill_round_rect(cb, px(4), th.accent);
                        p.draw_text(
                            "\u{E73E}",
                            cb,
                            &TextStyle {
                                font: Font::Icons,
                                size_lp: 11.0,
                                bold: false,
                                color: th.on_accent,
                                align: HAlign::Center,
                            },
                        );
                    } else {
                        transom_render::design::fill_with_stroke(
                            p,
                            cb,
                            px(4),
                            Some(th.control),
                            Some(th.control_edge),
                        );
                    }
                    for (k, l) in lines.iter().take(4).enumerate() {
                        p.draw_text(
                            l,
                            Rect::new(
                                r.x + px(30),
                                r.y + px(4) + k as i32 * line,
                                w - px(34),
                                line,
                            ),
                            &ts,
                        );
                    }
                    if self.focus == Some(hit) {
                        transom_render::design::focus_ring(p, th, r, 4, th.background);
                    }
                    self.hits.push((r, hit));
                    nodes.push((
                        r,
                        label.clone(),
                        extra(
                            ROLE_CHECKBUTTON,
                            if *on { STATE_CHECKED } else { 0 },
                            String::new(),
                            t(if *on { "acc.uncheck" } else { "acc.check" }),
                        ),
                        true,
                        Some(hit),
                    ));
                    y += h + px(4);
                }
                Row::Item {
                    glyph,
                    text,
                    detail,
                    selected,
                } => {
                    let rh = px(26);
                    let r = Rect::new(pad, y, w, rh);
                    if *selected {
                        p.fill_round_rect(r, px(4), th.hover);
                    }
                    let mut x = r.x + px(6);
                    if let Some(g) = glyph {
                        p.draw_text(
                            &transom_render::scene::glyph_char(*g).to_string(),
                            Rect::new(x, r.y, px(18), rh),
                            &TextStyle {
                                font: Font::Icons,
                                size_lp: 12.0,
                                bold: false,
                                color: th.foreground,
                                align: HAlign::Left,
                            },
                        );
                        x += px(24);
                    }
                    let ds = st(11.0, false, th.muted, HAlign::Right);
                    let dw = if detail.is_empty() {
                        0
                    } else {
                        (p.measure_text(detail, &ds).w + px(8)).min(w / 2)
                    };
                    let ns = st(12.0, *selected, th.foreground, HAlign::Left);
                    let tw = r.x + r.w - dw - x - px(4);
                    let shown = fit(p, text, &ns, tw);
                    p.draw_text(&shown, Rect::new(x, r.y, tw, rh), &ns);
                    if dw > 0 {
                        let d = fit(p, detail, &ds, dw - px(4));
                        p.draw_text(&d, Rect::new(r.x + r.w - dw - px(4), r.y, dw, rh), &ds);
                    }
                    let name = if detail.is_empty() {
                        text.clone()
                    } else {
                        format!("{text}, {detail}")
                    };
                    nodes.push((
                        r,
                        name,
                        extra(
                            ROLE_LISTITEM,
                            if *selected { STATE_SELECTED } else { 0 },
                            String::new(),
                            String::new(),
                        ),
                        false,
                        None,
                    ));
                    y += rh + px(2);
                }
                Row::Bullet(text) => {
                    let ts = st(12.0, false, th.foreground, HAlign::Left);
                    let ind = px(14);
                    let tw = w - ind;
                    let mut lines = transom_render::painter::wrap_text(p, text, &ts, tw);
                    if lines.len() > BULLET_LINES {
                        let rest = lines.split_off(BULLET_LINES - 1).join(" ");
                        lines.push(fit(p, &format!("{rest}\u{2026}"), &ts, tw));
                    }
                    let r0 = y;
                    p.draw_text("\u{2022}", Rect::new(pad + px(2), y, ind, line), &ts);
                    for l in &lines {
                        let shown = fit(p, l, &ts, tw);
                        p.draw_text(&shown, Rect::new(pad + ind, y, tw, line), &ts);
                        y += line;
                    }
                    let y1 = y.max(r0 + line);
                    nodes.push((
                        Rect::new(pad, r0, w, y1 - r0),
                        text.clone(),
                        stat(),
                        false,
                        None,
                    ));
                    y = y1 + px(3);
                }
                Row::Gap(g) => y += px(*g),
            }
        }
        let content_h = y - top + pad;
        self.content_h = content_h;
        self.max_scroll = (content_h - sz.h).max(0);
        if self.scroll > self.max_scroll {
            self.scroll = self.max_scroll;
            self.repaint = true;
        }
        // posuvnik: len ked sa obsah da posuvat
        transom_render::design::scroll_thumb(
            p,
            th,
            Rect::new(0, 0, sz.w, sz.h),
            self.scroll,
            self.max_scroll,
        );
        self.nodes = nodes
            .into_iter()
            .map(|(r, name, mut ex, enabled, hit)| {
                if r.y + r.h > sz.h || r.y < 0 {
                    ex.state |= STATE_OFFSCREEN;
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
                    ex,
                    hit,
                )
            })
            .collect();
    }

    pub fn a11y(&self) -> (Vec<MenuEntry>, Vec<PageExtra>, Option<usize>, String) {
        let focus = self
            .focus
            .and_then(|f| self.nodes.iter().position(|n| n.2 == Some(f)));
        (
            self.nodes.iter().map(|n| n.0.clone()).collect(),
            self.nodes.iter().map(|n| n.1.clone()).collect(),
            focus,
            self.name.clone(),
        )
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| *h)
    }

    /// Oblast ovladaca `id` z posledneho kreslenia (testy).
    #[cfg(test)]
    pub fn rect_of(&self, id: u32) -> Option<Rect> {
        self.hits
            .iter()
            .find(
                |(_, h)| matches!(h, Hit::Button(i) | Hit::Slider(i) | Hit::Toggle(i) if *i == id),
            )
            .map(|(r, _)| *r)
    }

    fn value_at(&self, id: u32, x: i32) -> Option<u32> {
        let (_, tr, _, on) = self.tracks.iter().find(|t| t.0 == id)?;
        if !on {
            return None;
        }
        let f = ((x - tr.x) as f32 / tr.w.max(1) as f32).clamp(0.0, 1.0);
        Some((f * 100.0).round() as u32)
    }

    fn current(&self, id: u32) -> Option<u32> {
        self.dragging(id)
            .or_else(|| self.tracks.iter().find(|t| t.0 == id).map(|t| t.2))
    }

    fn activate(h: Hit) -> Option<Cmd> {
        match h {
            Hit::Button(i) | Hit::Toggle(i) => Some(Cmd::Click(i)),
            Hit::Slider(_) => None,
        }
    }

    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32) -> (Option<Cmd>, bool) {
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
                match h {
                    Some(Hit::Slider(id)) => {
                        if let Some(v) = self.value_at(id, x) {
                            self.drag = Some((id, v));
                            cmd = Some(Cmd::Slide(id, v));
                        }
                    }
                    Some(other) => self.pressed = Some(other),
                    None => {}
                }
                redraw = true;
            }
            PointerKind::Move => {
                if let Some((id, old)) = self.drag {
                    if let Some(v) = self.value_at(id, x) {
                        if v != old {
                            self.drag = Some((id, v));
                            cmd = Some(Cmd::Slide(id, v));
                            redraw = true;
                        }
                    }
                }
            }
            PointerKind::Click => {
                if self.drag.take().is_none() {
                    if let Some(hit) = h {
                        if self.pressed.is_none() || self.pressed == Some(hit) {
                            cmd = Self::activate(hit);
                        }
                    }
                }
                self.pressed = None;
                redraw = true;
            }
            PointerKind::Leave => {
                self.drag = None;
                self.pressed = None;
                redraw = true;
            }
            _ => {}
        }
        (cmd, redraw)
    }

    pub fn key(&mut self, vk: u32, shift: bool, repeat: bool) -> (Option<Cmd>, bool) {
        let ring: Vec<Hit> = self.hits.iter().map(|(_, h)| *h).collect();
        let at = self.focus.and_then(|f| ring.iter().position(|h| *h == f));
        let mv = |back: bool| -> Option<Hit> {
            if ring.is_empty() {
                return None;
            }
            let n = ring.len();
            Some(match (at, back) {
                (None, false) => ring[0],
                (None, true) => ring[n - 1],
                (Some(i), false) => ring[(i + 1) % n],
                (Some(i), true) => ring[(i + n - 1) % n],
            })
        };
        match vk {
            0x09 => {
                self.focus = mv(shift);
                self.reveal();
                (None, true)
            }
            0x0D | 0x20 => {
                if repeat {
                    return (None, false);
                }
                match self.focus {
                    Some(f) => (Self::activate(f), true),
                    None => (None, false),
                }
            }
            0x21..=0x28 if matches!(self.focus, Some(Hit::Slider(_))) => {
                let Some(Hit::Slider(id)) = self.focus else {
                    return (None, false);
                };
                let Some(cur) = self.current(id) else {
                    return (None, false);
                };
                match step_percent(cur, vk, shift) {
                    Some(v) if v != cur => (Some(Cmd::Slide(id, v)), true),
                    _ => (None, false),
                }
            }
            0x26 | 0x28 => {
                self.focus = mv(vk == 0x26);
                self.reveal();
                (None, true)
            }
            _ => (None, false),
        }
    }

    /// Koliesko: nad posuvnikom hodnota o 2 % za zub, inak posun obsahu.
    pub fn wheel(&mut self, delta: i32) -> (Option<Cmd>, bool) {
        if let Some(Hit::Slider(id)) = self.hover {
            let step = (delta * 2 / 120).clamp(-10, 10);
            let Some(cur) = self.current(id) else {
                return (None, false);
            };
            if step == 0 {
                return (None, false);
            }
            let v = (cur as i32 + step).clamp(0, 100) as u32;
            return (Some(Cmd::Slide(id, v)), v != cur);
        }
        (
            None,
            self.glide.wheel(self.scroll, -delta / 3, self.max_scroll),
        )
    }

    /// Snimka plynuleho posuvania.
    pub fn glide_step(&mut self) -> bool {
        self.glide.step(&mut self.scroll)
    }

    fn reveal(&mut self) {
        let Some(f) = self.focus else { return };
        let Some((r, _)) = self.hits.iter().find(|(_, h)| *h == f) else {
            return;
        };
        let margin = 10;
        let before = self.scroll;
        if r.y < margin {
            self.scroll = (self.scroll + r.y - margin).max(0);
        } else if r.y + r.h > self.view_h - margin {
            self.scroll = (self.scroll + r.y + r.h - (self.view_h - margin))
                .min(self.max_scroll.max(self.scroll));
        }
        if self.scroll != before {
            self.repaint = true;
        }
    }
}
