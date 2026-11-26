//! Panel "Batérie periférií" (popover): zoznam zariadeni, ktore hlasia uroven baterie (len na citanie).
//!
//! Poctivost: ukazuje sa len to, co zariadenie (alebo Windows pri Bluetooth) naozaj hlasi. Spiace zariadenie ma
//! poslednu hodnotu s vekom, zariadenie bez urovne (NuPhy pocas nabijania) "nabija sa" bez percent; nic sa neodhaduje.
//!
//! Cisty stav a kreslenie bez Win32 okna: prikazy (`PeriphCmd`) vykonava `App`.
use std::sync::Arc;
use transom_core::item::Glyph;
use transom_i18n::{t, tf};
use transom_peripherals::{Availability, Kind, Peripheral, Snapshot, Source};
use transom_platform::menu_access::{
    MenuEntry, PageExtra, ROLE_LISTITEM, ROLE_PUSHBUTTON, ROLE_STATICTEXT, STATE_OFFSCREEN,
};
use transom_platform::PointerKind;
use transom_render::{Font, HAlign, Painter, Rect, TextStyle, Theme};

use crate::audio_pop::{fit, st};

pub const PERIPH_W_LP: i32 = 360;
pub const PERIPH_H_LP: i32 = 480;
/// Uroven do tejto hranice vratane (pod 20 %) je "nizka" (cervena vypln a slovo, nie len farba).
pub const LOW_PERCENT: u8 = transom_render::scene::BATTERY_LOW_BELOW - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriphHit {
    Refresh,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriphCmd {
    Refresh,
    Settings,
}

/// Glyf zariadenia na liste a v paneli.
pub fn glyph_of(kind: Kind) -> Glyph {
    match kind {
        Kind::Mouse => Glyph::Mouse,
        Kind::Keyboard => Glyph::Keyboard,
        Kind::Headset => Glyph::Headset,
        Kind::Controller => Glyph::Controller,
        Kind::Other => Glyph::Device,
    }
}

pub fn source_name(s: Source) -> String {
    t(match s {
        Source::Bluetooth => "per.src_bluetooth",
        Source::Razer => "per.src_razer",
        Source::Logitech => "per.src_logitech",
        Source::NuPhy => "per.src_nuphy",
        Source::Hid => "per.src_hid",
    })
}

/// Vek poslednej hodnoty ("pred 5 min").
fn age_text(secs: u64) -> String {
    if secs < 90 {
        t("per.age_now")
    } else if secs < 90 * 60 {
        tf("per.age_min", &[&(secs / 60).to_string()])
    } else {
        tf("per.age_h", &[&(secs / 3600).to_string()])
    }
}

/// Stav slovami (nikdy nie len farbou): nabija sa, nizka, spi, bez urovne.
pub fn state_text(d: &Peripheral) -> String {
    if d.stale {
        return tf("per.asleep", &[&age_text(d.age_secs)]);
    }
    match (d.charging, d.percent) {
        (true, None) => t("per.charging_nolevel"),
        (true, Some(100)) => t("per.full"),
        (true, Some(_)) => t("per.charging"),
        (false, None) => t("per.nolevel"),
        (false, Some(p)) if p <= LOW_PERCENT => t("per.low"),
        (false, Some(_)) => String::new(),
    }
}

/// Pristupny nazov zariadenia (lista aj panel): "Razer DeathAdder V4 Pro: 64 %, nabija sa".
pub fn accessible_name(d: &Peripheral) -> String {
    let level = match d.percent {
        Some(p) => format!("{p} %"),
        None => t("per.unknown_level"),
    };
    let state = state_text(d);
    if state.is_empty() {
        format!("{}: {level}", d.name)
    } else {
        format!("{}: {level}, {state}", d.name)
    }
}

#[derive(Default)]
pub struct PeriphPanel {
    /// Plynule posuvanie (snimka: `glide_step`).
    pub glide: transom_render::design::Glide,

    pub hover: Option<PeriphHit>,
    pub focus: Option<PeriphHit>,
    pressed: Option<PeriphHit>,
    hits: Vec<(Rect, PeriphHit)>,
    nodes: Vec<(MenuEntry, PageExtra, Option<PeriphHit>)>,
    name: String,
    pub scroll: i32,
    max_scroll: i32,
    pub repaint: bool,
    view_h: i32,
}

type Node = (Rect, String, PageExtra, bool, Option<PeriphHit>);

fn static_extra(role: i32, value: String) -> PageExtra {
    PageExtra {
        role,
        state: 0,
        value,
        action: String::new(),
    }
}

impl PeriphPanel {
    pub fn reset(&mut self) {
        *self = PeriphPanel::default();
    }

    pub fn paint(&mut self, p: &mut dyn Painter, th: &Theme, snap: &Arc<Snapshot>) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, b, sz.w - 2 * b, sz.h - 2 * b), th.background);
        let pad = px(14);
        self.view_h = sz.h;
        let w = sz.w - 2 * pad;
        let line = px(17);
        let mut y = pad - self.scroll;
        let top = y;
        self.hits.clear();
        let mut nodes: Vec<Node> = Vec::new();

        let title = t("per.title");
        p.draw_text(
            &title,
            Rect::new(pad, y, w, px(26)),
            &st(15.0, true, th.foreground, HAlign::Left),
        );
        nodes.push((
            Rect::new(pad, y, w, px(26)),
            title.clone(),
            static_extra(ROLE_STATICTEXT, String::new()),
            false,
            None,
        ));
        y += px(34);

        let note_st = st(11.0, false, th.muted, HAlign::Left);
        let note = |p: &mut dyn Painter, y: &mut i32, text: &str, nodes: &mut Vec<Node>| {
            let r0 = *y;
            for l in transom_render::painter::wrap_text(p, text, &note_st, w)
                .into_iter()
                .take(6)
            {
                p.draw_text(&l, Rect::new(pad, *y, w, line), &note_st);
                *y += line;
            }
            nodes.push((
                Rect::new(pad, r0, w, *y - r0),
                text.to_string(),
                static_extra(ROLE_STATICTEXT, String::new()),
                false,
                None,
            ));
        };

        if snap.availability == Availability::Starting {
            note(p, &mut y, &t("per.starting"), &mut nodes);
            y += px(8);
        } else if snap.devices.is_empty() {
            note(p, &mut y, &t("per.empty"), &mut nodes);
            y += px(8);
        }

        // zariadenia: ikona, nazov, uroven; pod tym pruh a stav slovami + zdroj
        let row_h = px(58);
        for d in &snap.devices {
            let r = Rect::new(pad, y, w, row_h - px(6));
            p.fill_round_rect(r, px(transom_render::design::radius::CONTROL), th.surface);
            let ix = pad + px(10);
            let icon_st = TextStyle {
                font: Font::Icons,
                size_lp: 18.0,
                bold: false,
                color: if d.stale { th.muted } else { th.foreground },
                align: HAlign::Center,
            };
            p.draw_text(
                &transom_render::scene::glyph_char(glyph_of(d.kind)).to_string(),
                Rect::new(ix, y + px(6), px(24), px(24)),
                &icon_st,
            );
            let tx = ix + px(34);
            let level = transom_peripherals::short_label(d);
            let lvl_st = st(13.0, true, th.foreground, HAlign::Right);
            let lvl_w = p.measure_text(&level, &lvl_st).w;
            let right = pad + w - px(10);
            p.draw_text(
                &level,
                Rect::new(right - lvl_w, y + px(6), lvl_w, px(22)),
                &lvl_st,
            );
            let name_st = st(13.0, true, th.foreground, HAlign::Left);
            let name = fit(p, &d.name, &name_st, right - lvl_w - px(10) - tx);
            p.draw_text(
                &name,
                Rect::new(tx, y + px(6), right - tx, px(22)),
                &name_st,
            );
            // pruh urovne (stav nesie aj text vedla, nie len farba)
            let bar = Rect::new(tx, y + px(31), px(64), px(6));
            p.fill_round_rect(bar, px(3), th.control_edge);
            if let Some(pct) = d.percent {
                let fw = (bar.w * i32::from(pct.min(100)) + 50) / 100;
                if fw > 0 {
                    let c = if d.stale {
                        th.muted
                    } else if d.charging {
                        th.good
                    } else if d.is_low(LOW_PERCENT) {
                        th.low
                    } else {
                        th.accent
                    };
                    p.fill_round_rect(Rect::new(bar.x, bar.y, fw, bar.h), px(3), c);
                }
            }
            let mut sub = source_name(d.source);
            let state = state_text(d);
            if !state.is_empty() {
                sub = format!("{state} · {sub}");
            }
            let sub_st = st(
                11.0,
                d.is_low(LOW_PERCENT) && !d.stale,
                th.muted,
                HAlign::Left,
            );
            let sx = bar.x + bar.w + px(8);
            let sub_fit = fit(p, &sub, &sub_st, right - sx);
            p.draw_text(
                &sub_fit,
                Rect::new(sx, y + px(26), right - sx, px(16)),
                &sub_st,
            );
            nodes.push((
                r,
                accessible_name(d),
                static_extra(ROLE_LISTITEM, level),
                false,
                None,
            ));
            y += row_h;
        }
        y += px(4);

        // tlacidla: obnovit, nastavenia
        let bh = px(32);
        let gap = px(8);
        let bw = (w - gap) / 2;
        for (i, (hit, key)) in [
            (PeriphHit::Refresh, "per.refresh"),
            (PeriphHit::Settings, "per.settings"),
        ]
        .into_iter()
        .enumerate()
        {
            let r = Rect::new(pad + i as i32 * (bw + gap), y, bw, bh);
            let hov = self.hover == Some(hit);
            let label = t(key);
            let ls = st(12.0, false, th.foreground, HAlign::Center);
            let text = fit(p, &label, &ls, r.w - px(16));
            let b =
                transom_render::design::Button::new(&text, transom_render::design::Kind::Secondary)
                    .state(transom_render::design::State {
                        hover: hov,
                        pressed: self.pressed == Some(hit) && hov,
                        focus: self.focus == Some(hit),
                        ..Default::default()
                    });
            transom_render::design::button(p, th, r, &b, th.background);
            self.hits.push((r, hit));
            nodes.push((
                r,
                label,
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
        y += bh + px(10);
        note(p, &mut y, &t("per.note"), &mut nodes);

        let content_h = y - top + pad;
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
            .map(|(r, name, mut extra, enabled, hit)| {
                if r.y + r.h > sz.h || r.y < 0 {
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
        self.name = title;
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

    fn hit(&self, x: i32, y: i32) -> Option<PeriphHit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| *h)
    }

    fn activate(hit: PeriphHit) -> PeriphCmd {
        match hit {
            PeriphHit::Refresh => PeriphCmd::Refresh,
            PeriphHit::Settings => PeriphCmd::Settings,
        }
    }

    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32) -> (Option<PeriphCmd>, bool) {
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
                self.pressed = h;
                redraw = true;
            }
            PointerKind::Click => {
                // uvolnenie bez stlacenia = aktivacia cez MSAA
                if let Some(hit) = h {
                    if self.pressed.is_none() || self.pressed == Some(hit) {
                        cmd = Some(Self::activate(hit));
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

    /// Tab / Shift+Tab a sipky presuvaju fokus, Enter / medzernik aktivuje (opakovanie sa ignoruje).
    pub fn key(&mut self, vk: u32, shift: bool, repeat: bool) -> (Option<PeriphCmd>, bool) {
        let ring: Vec<PeriphHit> = self.hits.iter().map(|(_, h)| *h).collect();
        let at = self.focus.and_then(|f| ring.iter().position(|h| *h == f));
        let mv = |back: bool| -> Option<PeriphHit> {
            let n = ring.len();
            if n == 0 {
                return None;
            }
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
            0x25..=0x28 => {
                self.focus = mv(matches!(vk, 0x25 | 0x26));
                self.reveal();
                (None, true)
            }
            0x0D | 0x20 if !repeat => match self.focus {
                Some(f) => (Some(Self::activate(f)), true),
                None => (None, false),
            },
            _ => (None, false),
        }
    }

    pub fn wheel(&mut self, delta: i32) -> bool {
        self.glide.wheel(self.scroll, -delta / 3, self.max_scroll)
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
