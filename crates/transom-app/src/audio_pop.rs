//! Panel "Zvuk a rychle akcie" (popover): predvolene zariadenie, systemova hlasitost a stlmenie Windows, zoznam vystupov
//! len na citanie, tlacidla na otvorenie natívnych povrchov Windows a skupina rychlych akcii.
//!
//! Poctivost voci pouzivatelovi: panel **nikdy netvrdi, ze sa vystup zmenil**. Windows 11 na zmenu predvoleneho vystupu
//! nema podporovane verejne API, preto tlacidla vystup len otvoria v systeme; zmenu potvrdi az notifikacia Core Audio
//! (`Snapshot::default_changes`). Rychle akcie "otvaraju povrch Windows"; co Windows urobi potom, transom nevie.
//!
//! Tento subor je cisty stav a kreslenie (bez Win32 okna): prikazy (`AudioCmd`) vykonava `App`, takze sa vsetko da
//! testovat offscreen aj s falosnou snimkou stavu.
use crate::icon_rt::{Icon, IconSource};
use std::sync::Arc;
use transom_audio::{Availability, Device, DeviceState, Snapshot};
use transom_i18n::{t, tf};
use transom_mixer::{Route, Snapshot as MixSnap};
use transom_platform::menu_access::{
    MenuEntry, PageExtra, ROLE_CHECKBUTTON, ROLE_LISTITEM, ROLE_PUSHBUTTON, ROLE_SLIDER,
    ROLE_STATICTEXT, STATE_CHECKED, STATE_OFFSCREEN, STATE_SELECTED,
};
use transom_platform::PointerKind;
use transom_render::{Color, Font, HAlign, Painter, Rect, TextStyle, Theme};

/// Riadok pre citacku: (oblast, nazov, rola/stav/hodnota, povoleny, ovladac).
type Node = (Rect, String, PageExtra, bool, Option<AudioHit>);

pub const AUDIO_W_LP: i32 = 340;
pub const AUDIO_H_LP: i32 = 640;
/// Najviac tolko zariadeni sa vypise (dalsie len poctom).
const MAX_DEVICE_ROWS: usize = 4;

/// Zobrazena rychla akcia: id v konfiguracii a kluc nazvu.
pub const QUICK: [(&str, &str); 5] = [
    ("quick_settings", "qa.quick_settings"),
    ("notifications", "qa.notifications"),
    ("search", "qa.search"),
    ("tray", "qa.tray"),
    ("settings", "qa.settings"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioHit {
    Mute,
    Slider,
    Picker,
    SoundSettings,
    Quick(&'static str),
    /// Mixer: stlmenie, posuvnik a vyber vystupu aplikacie (kluc aplikacie).
    AppMute(u64),
    AppSlider(u64),
    AppOut(u64),
    /// Volba vystupu v rozbalenom zozname: 0 = predvoleny, `i + 1` = `devices[i]`.
    AppRoute(u64, u8),
    /// Vystup aplikacii v nastaveniach Windows (ked transom vystup menit nevie).
    AppsSettings,
    /// Zvukove profily: pouzit / ulozit / zmazat profil (index do `AUDIO_PROFILE_SLOTS`), rezim ukladania/mazania.
    ProfApply(u8),
    ProfSave(u8),
    ProfDelete(u8),
    ProfMode(u8),
    /// Pripojit / odpojit Bluetooth zvukove zariadenie (index v snimke).
    BtToggle(u8),
}

/// Co sa robi so sekciou profilov.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProfMode {
    #[default]
    Apply,
    Save,
    Delete,
}

/// Co ma `App` vykonat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCmd {
    SetVolume(u32),
    SetMute(bool),
    /// Otvorit vyber vystupu Windows (`Win+Ctrl+V`).
    Picker,
    /// Otvorit `ms-settings:sound`.
    SoundSettings,
    Quick(&'static str),
    AppVolume(u64, u32),
    AppMute(u64, bool),
    /// `None` = predvoleny vystup, inak index do `MixSnap::devices`.
    AppRoute(u64, Option<u8>),
    AppsSettings,
    ProfApply(u8),
    ProfSave(u8),
    ProfDelete(u8),
    /// (index zariadenia, pripojit)
    BtSet(u8, bool),
}

/// Klavesy posuvnika (rovnake kroky ako v Nastaveniach: 5 %, s Shift 1 %, Page 10 %, Home/End).
pub fn step_percent(cur: u32, vk: u32, shift: bool) -> Option<u32> {
    let small: i32 = if shift { 1 } else { 5 };
    let cur = cur as i32;
    let v = match vk {
        0x25 | 0x28 => cur - small,
        0x27 | 0x26 => cur + small,
        0x21 => cur + 10,
        0x22 => cur - 10,
        0x24 => 0,
        0x23 => 100,
        _ => return None,
    };
    Some(v.clamp(0, 100) as u32)
}

#[derive(Default)]
pub struct AudioPanel {
    /// Plynule posuvanie (snimka: `glide_step`).
    pub glide: transom_render::design::Glide,

    pub hover: Option<AudioHit>,
    pub focus: Option<AudioHit>,
    pressed: Option<AudioHit>,
    /// Percento pocas tahania posuvnika (zobrazuje sa namiesto stavu zo systemu, aby hodnota neskakala).
    drag: Option<u32>,
    /// Naposledy zadana hlasitost, kym ju system nepotvrdi (viac klaves za sebou sa pocita z nej).
    target: Option<(u32, u64)>,
    hits: Vec<(Rect, AudioHit)>,
    track: Rect,
    /// Posuvniky aplikacii z posledneho kreslenia.
    app_tracks: Vec<(u64, Rect)>,
    /// Tahanie posuvnika aplikacie: (kluc, percento).
    app_drag: Option<(u64, u32)>,
    /// Aplikacia s rozbalenym vyberom vystupu.
    pub open_route: Option<u64>,
    /// Snimka mixera z posledneho kreslenia (klik a klavesy pracuju s tym, co bolo vidiet).
    mix: Option<Arc<MixSnap>>,
    /// Ulozene zvukove profily (indexy do `AUDIO_PROFILE_SLOTS`), nastavuje `App` pred kreslenim.
    pub profiles: Vec<u8>,
    pub prof_mode: ProfMode,
    /// Bluetooth zvukove zariadenia (nastavuje `App` pred kreslenim; `None` = sekcia sa nekresli).
    pub bt: Option<Arc<transom_mixer::bt::BtSnap>>,
    nodes: Vec<(MenuEntry, PageExtra, Option<AudioHit>)>,
    name: String,
    pub scroll: i32,
    max_scroll: i32,
    /// Hlaska po akcii ("Poziadane: ...", "Windows ohlasil ...").
    pub msg: Option<String>,
    /// Pocet zmien predvoleneho vystupu pri otvoreni (zmena sa potvrdi, az ked narastie).
    baseline: Option<u64>,
    /// Po paint treba dokreslit (zmenil sa rozsah posuvania).
    pub repaint: bool,
    /// Vyska okna z posledneho kreslenia (px).
    view_h: i32,
    /// Vyska celeho obsahu z posledneho kreslenia (px).
    content_h: i32,
}

pub(crate) fn st(size: f32, bold: bool, color: Color, align: HAlign) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    }
}

/// Text skrateny na sirku `w` s "…" (nikdy nepreteka).
pub(crate) fn fit(p: &mut dyn Painter, text: &str, style: &TextStyle, w: i32) -> String {
    if p.measure_text(text, style).w <= w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if p.measure_text(&s, style).w <= w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

fn device_name(d: &Device) -> String {
    if d.name.is_empty() {
        t("audio.unnamed")
    } else {
        d.name.clone()
    }
}

fn state_word(d: &Device) -> Option<String> {
    match (d.is_default, d.state) {
        (_, DeviceState::Disabled) => Some(t("audio.st_disabled")),
        (_, DeviceState::Unplugged) => Some(t("audio.st_unplugged")),
        (true, DeviceState::Active) => Some(t("audio.st_default")),
        (false, DeviceState::Active) => None,
    }
}

/// Ktoru ikonu hlasitosti ukazat v lište (a ako pomenovat polozku).
pub fn bar_glyph_and_name(s: &Snapshot) -> (transom_core::item::Glyph, String) {
    use transom_core::item::Glyph;
    match (s.availability, s.volume) {
        (Availability::Ok, Some(v)) if v.muted => (Glyph::VolumeMuted, t("audio.name_muted")),
        (Availability::Ok, Some(v)) => (
            match v.percent {
                0 => Glyph::Volume0,
                1..=33 => Glyph::Volume1,
                34..=66 => Glyph::Volume2,
                _ => Glyph::Volume3,
            },
            tf("audio.name_vol", &[&v.percent.to_string()]),
        ),
        _ => (Glyph::Volume0, t("audio.name_off")),
    }
}

impl AudioPanel {
    /// Nove otvorenie panela: cisty stav, zakladna hodnota na potvrdenie zmeny predvoleneho vystupu.
    pub fn reset(&mut self, snap: &Snapshot) {
        *self = AudioPanel {
            baseline: Some(snap.default_changes),
            ..AudioPanel::default()
        };
    }

    /// Hlasitost, ktora sa ukazuje: tahanie, potom nami zadana (kym ju system nepotvrdi), potom stav systemu.
    fn shown_percent(&self, snap: &Snapshot) -> u32 {
        self.drag
            .or(self.target.map(|t| t.0))
            .or(snap.volume.map(|v| v.percent as u32))
            .unwrap_or(0)
    }

    fn can_volume(snap: &Snapshot) -> bool {
        snap.availability == Availability::Ok && snap.volume.is_some() && snap.controls.volume
    }

    fn can_mute(snap: &Snapshot) -> bool {
        snap.availability == Availability::Ok && snap.volume.is_some() && snap.controls.mute
    }

    fn status_text(snap: &Snapshot) -> Option<String> {
        Some(match snap.availability {
            Availability::Starting => t("audio.s_starting"),
            Availability::ServiceUnavailable => t("audio.s_unavailable"),
            Availability::NoDevice => t("audio.s_nodevice"),
            Availability::Ok if !snap.controls.volume => t("audio.s_novolume"),
            Availability::Ok if !snap.controls.mute => t("audio.s_nomute"),
            Availability::Ok => return None,
        })
    }

    /// Potvrdenie zmeny predvoleneho vystupu: len ked ju ohlasil Core Audio.
    fn sync_default_change(&mut self, snap: &Snapshot) {
        match self.baseline {
            Some(b) if snap.default_changes > b => {
                self.baseline = Some(snap.default_changes);
                let name = snap.default_device().map(device_name).unwrap_or_default();
                self.msg = Some(tf("audio.default_changed", &[&name]));
            }
            None => self.baseline = Some(snap.default_changes),
            _ => {}
        }
        // nami zadana hlasitost uz platila alebo system ohlasil inu hodnotu
        if let Some((v, gen)) = self.target {
            let cur = snap.volume.map(|x| x.percent as u32);
            if cur == Some(v) || snap.generation != gen && cur != Some(v) && snap.volume.is_some() {
                self.target = None;
            }
        }
    }

    #[cfg(test)]
    pub fn paint(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        snap: &Arc<Snapshot>,
        quick: &[String],
    ) {
        self.paint_with(p, th, snap, quick, None, None);
    }

    /// Kreslenie aj s mixerom aplikacii (`mix = None`: sekcia sa nekresli).
    pub fn paint_with(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        snap: &Arc<Snapshot>,
        quick: &[String],
        mix: Option<Arc<MixSnap>>,
        icons: Option<&dyn IconSource>,
    ) {
        self.sync_default_change(snap);
        self.mix = mix.clone();
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, b, sz.w - 2 * b, sz.h - 2 * b), th.background);
        let pad = px(12);
        self.view_h = sz.h;
        let w = sz.w - 2 * pad;
        let line = px(17);
        let mut y = pad - self.scroll;
        let top = y;
        self.hits.clear();
        self.nodes.clear();
        self.app_tracks.clear();
        let mut nodes: Vec<Node> = Vec::new();

        // nadpis
        p.draw_text(
            &t("audio.title"),
            Rect::new(pad, y, w, px(26)),
            &st(15.0, true, th.foreground, HAlign::Left),
        );
        nodes.push((
            Rect::new(pad, y, w, px(26)),
            t("audio.title"),
            static_extra(),
            false,
            None,
        ));
        y += px(34);

        let lbl = st(11.0, true, th.accent_text, HAlign::Left);
        let note_st = st(11.0, false, th.muted, HAlign::Left);
        use transom_render::design::section_card as card;

        // karta Vystup: predvolene zariadenie, systemova hlasitost a stav
        y = card(p, th, pad, w, y, |p: &mut dyn Painter, pad, w, mut y| {
            p.draw_text(&t("audio.default_label"), Rect::new(pad, y, w, line), &lbl);
            y += line;
            let default_name = match snap.default_device() {
                Some(d) => device_name(d),
                None => t("audio.no_device"),
            };
            let dn = st(13.0, true, th.foreground, HAlign::Left);
            let shown = fit(p, &default_name, &dn, w);
            p.draw_text(&shown, Rect::new(pad, y, w, px(22)), &dn);
            nodes.push((
                Rect::new(pad, y - line, w, line + px(22)),
                format!("{}: {default_name}", t("audio.default_label")),
                static_extra(),
                false,
                None,
            ));
            y += px(32);

            // systemova hlasitost (jasne oznacena; oddelena od zvukov rozhrania transom)
            let vl = t("audio.volume_label");
            p.draw_text(
                &vl,
                Rect::new(pad, y, w, line),
                &st(12.0, true, th.foreground, HAlign::Left),
            );
            y += line + px(2);
            for l in transom_render::painter::wrap_text(p, &t("audio.volume_note"), &note_st, w)
                .into_iter()
                .take(3)
            {
                p.draw_text(&l, Rect::new(pad, y, w, line), &note_st);
                y += line;
            }
            y += px(8);

            let can_vol = Self::can_volume(snap);
            let can_mute = Self::can_mute(snap);
            let muted = snap.volume.is_some_and(|v| v.muted);
            let pct = self.shown_percent(snap);

            // riadok: stlmenie | posuvnik | hodnota
            let row_h = px(34);
            let mb = Rect::new(pad, y, row_h, row_h);
            let mhover = self.hover == Some(AudioHit::Mute);
            let mfocus = self.focus == Some(AudioHit::Mute);
            let mbg = if mhover && can_mute {
                th.control_hover
            } else {
                th.control
            };
            let rad = px(transom_render::design::radius::CONTROL);
            if muted {
                // stlmene = zapnuty prepinac: akcentova vypln a symbol stlmenia (nie len farba)
                p.fill_round_rect(
                    mb,
                    rad,
                    if mhover && can_mute {
                        th.accent_hover
                    } else {
                        th.accent
                    },
                );
            } else {
                transom_render::design::fill_with_stroke(
                    p,
                    mb,
                    rad,
                    Some(mbg),
                    Some(th.control_edge),
                );
            }
            let (glyph, _) = bar_glyph_and_name(snap);
            p.draw_text(
                &transom_render::scene::glyph_char(glyph).to_string(),
                mb,
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 15.0,
                    bold: false,
                    color: if !can_mute {
                        th.muted
                    } else if muted {
                        th.on_accent
                    } else {
                        th.foreground
                    },
                    align: HAlign::Center,
                },
            );
            if mfocus && can_mute {
                // dvojfarebny ramik okolo (viditelny aj okolo akcentovej vyplne stlmenia)
                transom_render::design::focus_ring(
                    p,
                    th,
                    mb,
                    transom_render::design::radius::CONTROL,
                    th.background,
                );
            }
            let mute_label = if muted {
                t("audio.unmute")
            } else {
                t("audio.mute")
            };
            if can_mute {
                self.hits.push((mb, AudioHit::Mute));
            }
            nodes.push((
                mb,
                mute_label,
                PageExtra {
                    role: ROLE_CHECKBUTTON,
                    state: if muted { STATE_CHECKED } else { 0 },
                    value: String::new(),
                    action: if can_mute {
                        t(if muted { "acc.uncheck" } else { "acc.check" })
                    } else {
                        String::new()
                    },
                },
                can_mute,
                Some(AudioHit::Mute),
            ));

            let val_w = px(52);
            let tr = Rect::new(
                mb.x + mb.w + px(12),
                y + row_h / 2 - px(7),
                (w - row_h - px(12) - val_w - px(10)).max(px(40)),
                px(14),
            );
            self.track = tr;
            let f = pct.min(100) as f32 / 100.0;
            let live = can_vol;
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
                if live && !muted { th.accent } else { th.muted },
            );
            let tx = tr.x + (tr.w as f32 * f) as i32;
            p.fill_round_rect(
                Rect::new(tx - px(7), tr.y - px(2), px(14), tr.h + px(4)),
                px(7),
                if live { th.foreground } else { th.muted },
            );
            if self.focus == Some(AudioHit::Slider) && live {
                transom_render::design::focus_ring(
                    p,
                    th,
                    Rect::new(tr.x - px(6), tr.y - px(3), tr.w + px(12), tr.h + px(6)),
                    6,
                    th.background,
                );
            }
            let slider_rect = Rect::new(tr.x - px(8), y, tr.w + px(16), row_h);
            if live {
                self.hits.push((slider_rect, AudioHit::Slider));
            }
            nodes.push((
                slider_rect,
                vl.clone(),
                PageExtra {
                    role: ROLE_SLIDER,
                    state: 0,
                    value: format!("{pct} %"),
                    action: String::new(),
                },
                live,
                Some(AudioHit::Slider),
            ));
            p.draw_text(
                &tf("snd.volume_value", &[&pct.to_string()]),
                Rect::new(tr.x + tr.w + px(10), y, val_w, row_h),
                &st(
                    12.0,
                    true,
                    if live { th.foreground } else { th.muted },
                    HAlign::Left,
                ),
            );
            y += row_h + px(6);

            // stav (nepodporovana hlasitost, sluzba nedostupna, ziadny vystup)
            if let Some(text) = Self::status_text(snap) {
                let sst = st(11.0, false, th.low, HAlign::Left);
                let r0 = y;
                for l in transom_render::painter::wrap_text(p, &text, &sst, w)
                    .into_iter()
                    .take(3)
                {
                    p.draw_text(&l, Rect::new(pad, y, w, line), &sst);
                    y += line;
                }
                nodes.push((
                    Rect::new(pad, r0, w, y - r0),
                    text,
                    static_extra(),
                    false,
                    None,
                ));
                y += px(4);
            }
            y - px(6)
        });

        // mixer aplikacii, profily a Bluetooth: kazde vo vlastnej karte
        if let Some(m) = &mix {
            y = card(p, th, pad, w, y, |p: &mut dyn Painter, x, w, y| {
                self.paint_apps(p, th, m, icons, (x, w), y, &mut nodes) - px(6)
            });
            y = card(p, th, pad, w, y, |p: &mut dyn Painter, x, w, y| {
                self.paint_profiles(p, th, m, (x, w), y, &mut nodes) - px(6)
            });
        }
        if let Some(b) = self.bt.clone() {
            y = card(p, th, pad, w, y, |p: &mut dyn Painter, x, w, y| {
                self.paint_bt(p, th, &b, (x, w), y, &mut nodes) - px(6)
            });
        }

        // tlacidla na natívne povrchy Windows (nemenia nic samy)
        let bh = px(32);
        let button = |p: &mut dyn Painter,
                      this: &mut AudioPanel,
                      nodes: &mut Vec<Node>,
                      r: Rect,
                      label: &str,
                      acc: String,
                      hit: AudioHit| {
            let hov = this.hover == Some(hit);
            let down = this.pressed == Some(hit) && hov;
            let ls = st(12.0, false, th.foreground, HAlign::Center);
            let text = fit(p, label, &ls, r.w - px(16));
            let b =
                transom_render::design::Button::new(&text, transom_render::design::Kind::Secondary)
                    .state(transom_render::design::State {
                        hover: hov,
                        pressed: down,
                        focus: this.focus == Some(hit),
                        ..Default::default()
                    });
            transom_render::design::button(p, th, r, &b, th.background);
            this.hits.push((r, hit));
            nodes.push((
                r,
                acc,
                PageExtra {
                    role: ROLE_PUSHBUTTON,
                    state: 0,
                    value: String::new(),
                    action: t("acc.press"),
                },
                true,
                Some(hit),
            ));
        };

        // karta Zariadenia: zoznam vystupov (len na citanie) a tlacidla na nastavenia Windows
        y = card(p, th, pad, w, y, |p: &mut dyn Painter, pad, w, mut y| {
            p.draw_text(&t("audio.devices"), Rect::new(pad, y, w, line), &lbl);
            y += line + px(2);
            let rh = px(24);
            for d in snap.devices.iter().take(MAX_DEVICE_ROWS) {
                let r = Rect::new(pad, y, w, rh);
                let active = d.state == DeviceState::Active;
                let color = if active { th.foreground } else { th.muted };
                let word = state_word(d);
                let ww = word
                    .as_ref()
                    .map(|x| {
                        p.measure_text(x, &st(11.0, false, th.muted, HAlign::Left))
                            .w
                            + px(8)
                    })
                    .unwrap_or(0);
                if d.is_default {
                    p.fill_round_rect(r, px(4), th.hover);
                    p.fill_round_rect(
                        Rect::new(r.x + px(5), r.y + rh / 2 - px(3), px(6), px(6)),
                        px(3),
                        th.accent,
                    );
                }
                let nm = st(12.0, d.is_default, color, HAlign::Left);
                let name = device_name(d);
                let text = fit(p, &name, &nm, w - px(24) - ww);
                p.draw_text(
                    &text,
                    Rect::new(r.x + px(18), r.y, w - px(18) - ww, rh),
                    &nm,
                );
                if let Some(word) = &word {
                    p.draw_text(
                        word,
                        Rect::new(r.x + r.w - ww, r.y, ww, rh),
                        &st(11.0, false, th.muted, HAlign::Right),
                    );
                }
                nodes.push((
                    r,
                    match &word {
                        Some(wd) => tf("audio.acc_device", &[&name, wd]),
                        None => name.clone(),
                    },
                    PageExtra {
                        role: ROLE_LISTITEM,
                        state: if d.is_default { STATE_SELECTED } else { 0 },
                        value: String::new(),
                        action: String::new(),
                    },
                    false,
                    None,
                ));
                y += rh;
            }
            if snap.devices.len() > MAX_DEVICE_ROWS {
                p.draw_text(
                    &tf(
                        "audio.more",
                        &[&(snap.devices.len() - MAX_DEVICE_ROWS).to_string()],
                    ),
                    Rect::new(pad + px(18), y, w, line),
                    &lbl,
                );
                y += line;
            }
            y += px(10);

            button(
                p,
                self,
                &mut nodes,
                Rect::new(pad, y, w, bh),
                &t("audio.picker"),
                tf("qa.acc", &[&t("audio.picker")]),
                AudioHit::Picker,
            );
            y += bh + px(6);
            button(
                p,
                self,
                &mut nodes,
                Rect::new(pad, y, w, bh),
                &t("audio.sound_settings"),
                tf("qa.acc", &[&t("audio.sound_settings")]),
                AudioHit::SoundSettings,
            );
            y += bh + px(6);
            for l in transom_render::painter::wrap_text(p, &t("audio.picker_note"), &note_st, w)
                .into_iter()
                .take(5)
            {
                p.draw_text(&l, Rect::new(pad, y, w, line), &note_st);
                y += line;
            }
            y
        });

        // rychle akcie: tlacidla v zalamovanom riadku
        let chosen: Vec<(&'static str, &'static str)> = QUICK
            .iter()
            .filter(|(id, _)| quick.iter().any(|q| q == id))
            .copied()
            .collect();
        if !chosen.is_empty() {
            y = card(p, th, pad, w, y, |p: &mut dyn Painter, pad, w, mut y| {
                p.draw_text(&t("qa.title"), Rect::new(pad, y, w, line), &lbl);
                y += line + px(4);
                let ch = px(30);
                let gap = px(6);
                let mut x = pad;
                for (id, key) in chosen {
                    let label = t(key);
                    let bw = p
                        .measure_text(&label, &st(12.0, false, th.foreground, HAlign::Center))
                        .w
                        + px(22);
                    let bw = bw.min(w);
                    if x > pad && x + bw > pad + w {
                        x = pad;
                        y += ch + gap;
                    }
                    let acc = if id == "settings" {
                        tf("qa.acc_settings", &[&label])
                    } else {
                        tf("qa.acc", &[&label])
                    };
                    button(
                        p,
                        self,
                        &mut nodes,
                        Rect::new(x, y, bw, ch),
                        &label,
                        acc,
                        AudioHit::Quick(id),
                    );
                    x += bw + gap;
                }
                y += ch + px(6);
                let r0 = y;
                for l in transom_render::painter::wrap_text(p, &t("qa.note"), &note_st, w)
                    .into_iter()
                    .take(4)
                {
                    p.draw_text(&l, Rect::new(pad, y, w, line), &note_st);
                    y += line;
                }
                nodes.push((
                    Rect::new(pad, r0, w, y - r0),
                    t("qa.note"),
                    static_extra(),
                    false,
                    None,
                ));
                y
            });
        }

        // hlaska po poslednej akcii (potvrdenie zmeny predvoleneho vystupu pride len z Core Audio)
        if let Some(m) = self.msg.clone() {
            let ms = st(11.0, true, th.accent_text, HAlign::Left);
            let r0 = y;
            for l in transom_render::painter::wrap_text(p, &m, &ms, w)
                .into_iter()
                .take(3)
            {
                p.draw_text(&l, Rect::new(pad, y, w, line), &ms);
                y += line;
            }
            nodes.push((
                Rect::new(pad, r0, w, y - r0),
                m,
                static_extra(),
                false,
                None,
            ));
            y += px(4);
        }

        // rozsah posuvania a stavy mimo viditelnej casti
        let content_h = y - top + 2 * pad;
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
        self.name = t("audio.title");
    }

    /// Celkova vyska obsahu z posledneho kreslenia (px): viac nez vyska okna = obsah sa posuva.
    #[cfg(test)]
    pub fn content_height(&self) -> i32 {
        self.content_h
    }

    /// Polozky pre citacky obrazovky: (polozky, role/stavy/hodnoty, index polozky s fokusom klavesnice, nazov korena).
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

    /// Oblast ovladaca z posledneho kreslenia (testy).
    #[cfg(test)]
    pub fn rect_of(&self, h: AudioHit) -> Option<Rect> {
        self.hits.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<AudioHit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| *h)
    }

    fn volume_at(&self, x: i32) -> u32 {
        let f = ((x - self.track.x) as f32 / self.track.w.max(1) as f32).clamp(0.0, 1.0);
        (f * 100.0).round() as u32
    }

    fn set_cmd(&mut self, snap: &Snapshot, v: u32) -> Option<AudioCmd> {
        let v = v.min(100);
        if !Self::can_volume(snap) || self.shown_percent(snap) == v && self.drag.is_none() {
            return None;
        }
        self.target = Some((v, snap.generation));
        Some(AudioCmd::SetVolume(v))
    }

    fn activate(&mut self, hit: AudioHit, snap: &Snapshot) -> Option<AudioCmd> {
        match hit {
            AudioHit::Mute if Self::can_mute(snap) => {
                Some(AudioCmd::SetMute(!snap.volume.is_some_and(|v| v.muted)))
            }
            AudioHit::Mute | AudioHit::Slider | AudioHit::AppSlider(_) => None,
            AudioHit::AppMute(k) => {
                let app = self.mix.as_ref()?.app(k)?;
                Some(AudioCmd::AppMute(k, !app.muted))
            }
            AudioHit::AppOut(k) => {
                self.open_route = if self.open_route == Some(k) {
                    None
                } else {
                    Some(k)
                };
                None
            }
            AudioHit::AppRoute(k, i) => {
                self.open_route = None;
                Some(AudioCmd::AppRoute(k, i.checked_sub(1)))
            }
            AudioHit::AppsSettings => Some(AudioCmd::AppsSettings),
            AudioHit::ProfApply(i) => Some(AudioCmd::ProfApply(i)),
            AudioHit::BtToggle(i) => {
                let b = self.bt.as_ref()?;
                let d = b.devices.get(usize::from(i))?;
                if b.busy.is_some() {
                    return None;
                }
                Some(AudioCmd::BtSet(i, !d.connected))
            }
            AudioHit::ProfSave(i) => {
                self.prof_mode = ProfMode::Apply;
                Some(AudioCmd::ProfSave(i))
            }
            AudioHit::ProfDelete(i) => {
                self.prof_mode = ProfMode::Apply;
                Some(AudioCmd::ProfDelete(i))
            }
            AudioHit::ProfMode(m) => {
                let want = match m {
                    1 => ProfMode::Save,
                    2 => ProfMode::Delete,
                    _ => ProfMode::Apply,
                };
                self.prof_mode = if self.prof_mode == want {
                    ProfMode::Apply
                } else {
                    want
                };
                None
            }
            AudioHit::Picker => Some(AudioCmd::Picker),
            AudioHit::SoundSettings => Some(AudioCmd::SoundSettings),
            AudioHit::Quick(id) => Some(AudioCmd::Quick(id)),
        }
    }

    /// Mys: vrati prikaz a to, ci treba prekreslit.
    pub fn pointer(
        &mut self,
        kind: PointerKind,
        x: i32,
        y: i32,
        snap: &Snapshot,
    ) -> (Option<AudioCmd>, bool) {
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
                    Some(AudioHit::Slider) => {
                        let v = self.volume_at(x);
                        self.drag = Some(v);
                        cmd = self.set_cmd(snap, v);
                    }
                    Some(AudioHit::AppSlider(k)) => {
                        if let Some(v) = self.app_volume_at(k, x) {
                            cmd = self.app_set(k, v);
                            self.app_drag = Some((k, v));
                        }
                    }
                    Some(other) => self.pressed = Some(other),
                    None => {}
                }
                redraw = true;
            }
            PointerKind::Move => {
                if let Some((k, old)) = self.app_drag {
                    if let Some(v) = self.app_volume_at(k, x) {
                        if v != old {
                            cmd = self.app_set(k, v);
                            self.app_drag = Some((k, v));
                            redraw = true;
                        }
                    }
                }
                if self.drag.is_some() {
                    let v = self.volume_at(x);
                    if self.drag != Some(v) {
                        self.drag = Some(v);
                        cmd = self.set_cmd(snap, v);
                        redraw = true;
                    }
                }
            }
            PointerKind::Click => {
                let app_dragged = self.app_drag.take().is_some();
                if self.drag.take().is_none() && !app_dragged {
                    // uvolnenie bez predchadzajuceho stlacenia = aktivacia cez MSAA (`accDoDefaultAction` posiela len klik)
                    if let Some(hit) = h {
                        if self.pressed.is_none() || self.pressed == Some(hit) {
                            cmd = self.activate(hit, snap);
                        }
                    }
                }
                self.pressed = None;
                redraw = true;
            }
            PointerKind::Leave => {
                self.drag = None;
                self.app_drag = None;
                self.pressed = None;
                redraw = true;
            }
            _ => {}
        }
        (cmd, redraw)
    }

    /// Klavesa: Tab / Shift+Tab, sipky (posuvnik: hlasitost, inak fokus), Home/End, PgUp/PgDn, Enter/Medzernik
    /// (opakovanie sa ignoruje, aby drzanie klavesu neprepinalo stlmenie dookola). Vrati prikaz a prekreslenie.
    pub fn key(
        &mut self,
        vk: u32,
        shift: bool,
        repeat: bool,
        snap: &Snapshot,
    ) -> (Option<AudioCmd>, bool) {
        let ring: Vec<AudioHit> = self.hits.iter().map(|(_, h)| *h).collect();
        let at = self.focus.and_then(|f| ring.iter().position(|h| *h == f));
        let mv = |back: bool| -> Option<AudioHit> {
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
                    Some(f) => (self.activate(f, snap), true),
                    None => (None, false),
                }
            }
            0x21..=0x28 if matches!(self.focus, Some(AudioHit::AppSlider(_))) => {
                let Some(AudioHit::AppSlider(k)) = self.focus else {
                    return (None, false);
                };
                let Some(cur) = self.app_shown(k) else {
                    return (None, false);
                };
                match step_percent(cur, vk, shift) {
                    Some(v) if v != cur => (self.app_set(k, v), true),
                    _ => (None, false),
                }
            }
            0x21..=0x28 if self.focus == Some(AudioHit::Slider) => {
                let cur = self.shown_percent(snap);
                match step_percent(cur, vk, shift) {
                    Some(v) if v != cur => (self.set_cmd(snap, v), true),
                    _ => (None, false),
                }
            }
            0x26 | 0x28 => {
                // mimo posuvnika: sipky hore/dole presuvaju fokus
                self.focus = mv(vk == 0x26);
                self.reveal();
                (None, true)
            }
            _ => (None, false),
        }
    }

    /// Koliesko: nad posuvnikom / stlmenim hlasitost o 2 % za zub, inak posun obsahu.
    pub fn wheel(&mut self, delta: i32, snap: &Snapshot) -> (Option<AudioCmd>, bool) {
        if let Some(AudioHit::AppSlider(k) | AudioHit::AppMute(k)) = self.hover {
            let step = (delta * 2 / 120).clamp(-10, 10);
            let Some(cur) = self.app_shown(k) else {
                return (None, false);
            };
            if step == 0 {
                return (None, false);
            }
            let v = (cur as i32 + step).clamp(0, 100) as u32;
            return (self.app_set(k, v), true);
        }
        if matches!(self.hover, Some(AudioHit::Slider | AudioHit::Mute)) {
            let step = (delta * 2 / 120).clamp(-10, 10);
            if step == 0 || !Self::can_volume(snap) {
                return (None, false);
            }
            let v = (self.shown_percent(snap) as i32 + step).clamp(0, 100) as u32;
            return (self.set_cmd(snap, v), true);
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

    /// Hlasitost aplikacie, ktora sa ukazuje (tahanie, inak snimka mixera).
    fn app_shown(&self, k: u64) -> Option<u32> {
        if let Some((dk, v)) = self.app_drag {
            if dk == k {
                return Some(v);
            }
        }
        self.mix.as_ref()?.app(k).map(|a| u32::from(a.volume))
    }

    fn app_volume_at(&self, k: u64, x: i32) -> Option<u32> {
        let (_, tr) = self.app_tracks.iter().find(|(kk, _)| *kk == k)?;
        let f = ((x - tr.x) as f32 / tr.w.max(1) as f32).clamp(0.0, 1.0);
        Some((f * 100.0).round() as u32)
    }

    fn app_set(&mut self, k: u64, v: u32) -> Option<AudioCmd> {
        let v = v.min(100);
        let cur = self.mix.as_ref()?.app(k).map(|a| u32::from(a.volume))?;
        if cur == v && self.app_drag.is_none_or(|(dk, dv)| dk != k || dv == v) {
            return None;
        }
        Some(AudioCmd::AppVolume(k, v))
    }

    /// Nazov vystupu aplikacie (predvoleny / nazov zariadenia / odpojeny).
    fn route_label(m: &MixSnap, r: &Route) -> String {
        match r {
            Route::Default => t("mix.default_out"),
            Route::Device(id) => match m.devices.iter().find(|d| &d.id == id) {
                Some(d) if !d.name.is_empty() => transom_audio::clean_name(&d.name),
                Some(_) => t("audio.unnamed"),
                None => t("mix.out_missing"),
            },
        }
    }

    /// Sekcia "Aplikacie": ikona, nazov, vystup, stlmenie a posuvnik kazdej aplikacie. Vrati y pod nou.
    #[allow(clippy::too_many_arguments)]
    fn paint_apps(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        m: &MixSnap,
        icons: Option<&dyn IconSource>,
        (pad, w): (i32, i32),
        mut y: i32,
        nodes: &mut Vec<Node>,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let line = px(17);
        let lbl = st(11.0, true, th.accent_text, HAlign::Left);
        let rad = px(transom_render::design::radius::CONTROL);
        p.draw_text(&t("mix.title"), Rect::new(pad, y, w, line), &lbl);
        nodes.push((
            Rect::new(pad, y, w, line),
            t("mix.title"),
            static_extra(),
            false,
            None,
        ));
        y += line + px(4);
        let info = if !m.ready {
            Some(t("mix.loading"))
        } else if m.apps.is_empty() {
            Some(t("mix.empty"))
        } else {
            None
        };
        if let Some(text) = info {
            let r = Rect::new(pad, y, w, line);
            p.draw_text(&text, r, &st(12.0, false, th.muted, HAlign::Left));
            nodes.push((r, text, static_extra(), false, None));
            y += line + px(10);
        }
        if let Some(src) = icons {
            src.begin();
        }
        let ic = px(20);
        for app in &m.apps {
            let k = app.key;
            // riadok 1: ikona, nazov, vystup
            let r1 = Rect::new(pad, y, w, px(26));
            let icon_rect = Rect::new(pad, y + (r1.h - ic) / 2, ic, ic);
            let color = if app.active { th.foreground } else { th.muted };
            let mut drawn = false;
            if let (Some(src), Some(exe)) = (icons, &app.exe) {
                let item = transom_launcher::Item::new(
                    app.name.clone(),
                    "",
                    transom_launcher::Kind::App,
                    transom_launcher::Target::Path(exe.clone()),
                );
                if let Icon::Pixels(b) = src.icon(&item, ic as u32) {
                    if b.w as i32 <= ic && b.h as i32 <= ic {
                        let pix: Vec<transom_render::brand::Rgba> = b
                            .rgba
                            .iter()
                            .map(|c| transom_render::brand::Rgba {
                                r: c[0],
                                g: c[1],
                                b: c[2],
                                a: c[3],
                            })
                            .collect();
                        let (bw, bh) = (b.w as i32, b.h as i32);
                        p.blend_rgba(
                            Rect::new(
                                icon_rect.x + (ic - bw) / 2,
                                icon_rect.y + (ic - bh) / 2,
                                bw,
                                bh,
                            ),
                            &pix,
                        );
                        drawn = true;
                    }
                }
            }
            if !drawn {
                if app.system {
                    p.draw_text(
                        &transom_render::scene::glyph_char(transom_core::item::Glyph::Volume3)
                            .to_string(),
                        icon_rect,
                        &TextStyle {
                            font: Font::Icons,
                            size_lp: 14.0,
                            bold: false,
                            color,
                            align: HAlign::Center,
                        },
                    );
                } else {
                    let g = transom_render::resulticons::cached(
                        transom_render::resulticons::ResultIcon::App,
                        ic as u32,
                        color,
                    );
                    p.blend_rgba(icon_rect, &g);
                }
            }
            // tlacidlo vystupu vpravo
            let out_w = if m.routing {
                (w * 2 / 5).max(px(80))
            } else {
                0
            };
            let name_x = pad + ic + px(8);
            let name_w = w - ic - px(8) - out_w - if out_w > 0 { px(8) } else { 0 };
            let ns = st(12.0, true, color, HAlign::Left);
            let shown = fit(p, &app.name, &ns, name_w);
            p.draw_text(&shown, Rect::new(name_x, r1.y, name_w, r1.h), &ns);
            if m.routing {
                let ob = Rect::new(pad + w - out_w, r1.y, out_w, r1.h);
                let hit = AudioHit::AppOut(k);
                let hov = self.hover == Some(hit);
                let label = Self::route_label(m, &app.route);
                let open = self.open_route == Some(k);
                transom_render::design::fill_with_stroke(
                    p,
                    ob,
                    rad,
                    Some(if hov || open {
                        th.control_hover
                    } else {
                        th.control
                    }),
                    Some(th.control_edge),
                );
                let ls = st(11.0, false, th.foreground, HAlign::Left);
                let chev = px(14);
                let text = fit(p, &label, &ls, ob.w - px(14) - chev);
                p.draw_text(
                    &text,
                    Rect::new(ob.x + px(8), ob.y, ob.w - px(10) - chev, ob.h),
                    &ls,
                );
                p.draw_text(
                    if open { "\u{E70E}" } else { "\u{E70D}" },
                    Rect::new(ob.x + ob.w - chev - px(6), ob.y, chev, ob.h),
                    &TextStyle {
                        font: Font::Icons,
                        size_lp: 8.0,
                        bold: false,
                        color: th.muted,
                        align: HAlign::Center,
                    },
                );
                if self.focus == Some(hit) {
                    transom_render::design::focus_ring(
                        p,
                        th,
                        ob,
                        transom_render::design::radius::CONTROL,
                        th.background,
                    );
                }
                self.hits.push((ob, hit));
                nodes.push((
                    ob,
                    tf("mix.out_acc", &[&app.name, &label]),
                    PageExtra {
                        role: ROLE_PUSHBUTTON,
                        state: 0,
                        value: label.clone(),
                        action: t("acc.press"),
                    },
                    true,
                    Some(hit),
                ));
            }
            y += r1.h + px(4);
            // rozbaleny vyber vystupu
            if m.routing && self.open_route == Some(k) {
                let rh = px(26);
                let default_name = m
                    .devices
                    .iter()
                    .find(|d| d.is_default)
                    .map(|d| transom_audio::clean_name(&d.name))
                    .unwrap_or_default();
                let mut choices: Vec<(String, bool)> = vec![(
                    if default_name.is_empty() {
                        t("mix.default_out")
                    } else {
                        tf("mix.default_is", &[&default_name])
                    },
                    app.route == Route::Default,
                )];
                for d in &m.devices {
                    let n = if d.name.is_empty() {
                        t("audio.unnamed")
                    } else {
                        transom_audio::clean_name(&d.name)
                    };
                    choices.push((n, app.route == Route::Device(d.id.clone())));
                }
                for (i, (label, sel)) in choices.into_iter().enumerate().take(255) {
                    let r = Rect::new(pad + ic + px(8), y, w - ic - px(8), rh);
                    let hit = AudioHit::AppRoute(k, i as u8);
                    if self.hover == Some(hit) || sel {
                        p.fill_round_rect(r, px(4), th.hover);
                    }
                    if sel {
                        p.fill_round_rect(
                            Rect::new(r.x + px(6), r.y + rh / 2 - px(3), px(6), px(6)),
                            px(3),
                            th.accent,
                        );
                    }
                    let ls = st(12.0, sel, th.foreground, HAlign::Left);
                    let text = fit(p, &label, &ls, r.w - px(26));
                    p.draw_text(&text, Rect::new(r.x + px(20), r.y, r.w - px(24), rh), &ls);
                    if self.focus == Some(hit) {
                        transom_render::design::focus_ring(p, th, r, 4, th.background);
                    }
                    self.hits.push((r, hit));
                    nodes.push((
                        r,
                        label,
                        PageExtra {
                            role: ROLE_LISTITEM,
                            state: if sel { STATE_SELECTED } else { 0 },
                            value: String::new(),
                            action: t("acc.press"),
                        },
                        true,
                        Some(hit),
                    ));
                    y += rh;
                }
                let ns = st(11.0, false, th.muted, HAlign::Left);
                let nw = w - ic - px(8);
                for l in transom_render::painter::wrap_text(p, &t("mix.route_note"), &ns, nw)
                    .into_iter()
                    .take(3)
                {
                    p.draw_text(
                        &l,
                        Rect::new(pad + ic + px(8), y, w - ic - px(8), line),
                        &ns,
                    );
                    y += line;
                }
                y += px(4);
            }
            if m.route_failed == Some(k) {
                let es = st(11.0, false, th.danger, HAlign::Left);
                let r0 = y;
                for l in transom_render::painter::wrap_text(p, &t("mix.route_failed"), &es, w)
                    .into_iter()
                    .take(3)
                {
                    p.draw_text(&l, Rect::new(pad, y, w, line), &es);
                    y += line;
                }
                nodes.push((
                    Rect::new(pad, r0, w, y - r0),
                    t("mix.route_failed"),
                    static_extra(),
                    false,
                    None,
                ));
            }
            // riadok 2: stlmenie | posuvnik | hodnota
            let row_h = px(28);
            let mb = Rect::new(pad, y, row_h, row_h);
            let mhit = AudioHit::AppMute(k);
            let mhover = self.hover == Some(mhit);
            if app.muted {
                p.fill_round_rect(mb, rad, if mhover { th.accent_hover } else { th.accent });
            } else {
                transom_render::design::fill_with_stroke(
                    p,
                    mb,
                    rad,
                    Some(if mhover { th.control_hover } else { th.control }),
                    Some(th.control_edge),
                );
            }
            let pct = self.app_shown(k).unwrap_or(u32::from(app.volume));
            let glyph = {
                use transom_core::item::Glyph;
                match (app.muted, pct) {
                    (true, _) => Glyph::VolumeMuted,
                    (_, 0) => Glyph::Volume0,
                    (_, 1..=33) => Glyph::Volume1,
                    (_, 34..=66) => Glyph::Volume2,
                    _ => Glyph::Volume3,
                }
            };
            p.draw_text(
                &transom_render::scene::glyph_char(glyph).to_string(),
                mb,
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 13.0,
                    bold: false,
                    color: if app.muted {
                        th.on_accent
                    } else {
                        th.foreground
                    },
                    align: HAlign::Center,
                },
            );
            if self.focus == Some(mhit) {
                transom_render::design::focus_ring(
                    p,
                    th,
                    mb,
                    transom_render::design::radius::CONTROL,
                    th.background,
                );
            }
            self.hits.push((mb, mhit));
            nodes.push((
                mb,
                tf(
                    if app.muted {
                        "mix.unmute_acc"
                    } else {
                        "mix.mute_acc"
                    },
                    &[&app.name],
                ),
                PageExtra {
                    role: ROLE_CHECKBUTTON,
                    state: if app.muted { STATE_CHECKED } else { 0 },
                    value: String::new(),
                    action: t(if app.muted {
                        "acc.uncheck"
                    } else {
                        "acc.check"
                    }),
                },
                true,
                Some(mhit),
            ));
            let val_w = px(44);
            let tr = Rect::new(
                mb.x + mb.w + px(12),
                y + row_h / 2 - px(7),
                (w - row_h - px(12) - val_w - px(16)).max(px(40)),
                px(14),
            );
            self.app_tracks.push((k, tr));
            let f = pct.min(100) as f32 / 100.0;
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
                if app.muted { th.muted } else { th.accent },
            );
            let tx = tr.x + (tr.w as f32 * f) as i32;
            p.fill_round_rect(
                Rect::new(tx - px(6), tr.y - px(1), px(12), tr.h + px(2)),
                px(6),
                th.foreground,
            );
            let shit = AudioHit::AppSlider(k);
            if self.focus == Some(shit) {
                transom_render::design::focus_ring(
                    p,
                    th,
                    Rect::new(tr.x - px(6), tr.y - px(3), tr.w + px(12), tr.h + px(6)),
                    6,
                    th.background,
                );
            }
            let sr = Rect::new(tr.x - px(8), y, tr.w + px(16), row_h);
            self.hits.push((sr, shit));
            nodes.push((
                sr,
                tf("mix.vol_acc", &[&app.name]),
                PageExtra {
                    role: ROLE_SLIDER,
                    state: 0,
                    value: format!("{pct} %"),
                    action: String::new(),
                },
                true,
                Some(shit),
            ));
            p.draw_text(
                &tf("snd.volume_value", &[&pct.to_string()]),
                Rect::new(tr.x + tr.w + px(16), y, val_w, row_h),
                &st(12.0, true, th.foreground, HAlign::Left),
            );
            y += row_h + px(10);
        }
        if let Some(src) = icons {
            src.end();
        }
        // vystup sa neda menit: odkaz na nastavenia Windows
        if m.ready && !m.routing {
            let ns = st(11.0, false, th.muted, HAlign::Left);
            for l in transom_render::painter::wrap_text(p, &t("mix.no_routing"), &ns, w)
                .into_iter()
                .take(3)
            {
                p.draw_text(&l, Rect::new(pad, y, w, line), &ns);
                y += line;
            }
            let r = Rect::new(pad, y, w, px(32));
            let hit = AudioHit::AppsSettings;
            let hov = self.hover == Some(hit);
            let ls = st(12.0, false, th.foreground, HAlign::Center);
            let text = fit(p, &t("mix.apps_settings"), &ls, r.w - px(16));
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
                tf("qa.acc", &[&t("mix.apps_settings")]),
                PageExtra {
                    role: ROLE_PUSHBUTTON,
                    state: 0,
                    value: String::new(),
                    action: t("acc.press"),
                },
                true,
                Some(hit),
            ));
            y += r.h + px(6);
        }
        y + px(4)
    }

    /// Sekcia "Profily": pouzitie ulozenych profilov, ulozenie aktualneho stavu do profilu a mazanie. Vrati y pod nou.
    #[allow(clippy::too_many_arguments)]
    fn paint_profiles(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        m: &MixSnap,
        (pad, w): (i32, i32),
        mut y: i32,
        nodes: &mut Vec<Node>,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let line = px(17);
        if !m.ready {
            return y;
        }
        let lbl = st(11.0, true, th.accent_text, HAlign::Left);
        p.draw_text(&t("prof.title"), Rect::new(pad, y, w, line), &lbl);
        nodes.push((
            Rect::new(pad, y, w, line),
            t("prof.title"),
            static_extra(),
            false,
            None,
        ));
        y += line + px(4);
        let names: Vec<String> = transom_config::AUDIO_PROFILE_SLOTS
            .iter()
            .map(|s| t(&format!("prof.{s}")))
            .collect();
        // tlacidla: (text, ovladac, hlavne, pristupny nazov)
        let mut chips: Vec<(String, AudioHit, bool, String)> = Vec::new();
        match self.prof_mode {
            ProfMode::Apply => {
                for i in &self.profiles {
                    let n = names[usize::from(*i)].clone();
                    chips.push((
                        n.clone(),
                        AudioHit::ProfApply(*i),
                        false,
                        tf("prof.apply_acc", &[&n]),
                    ));
                }
                if self.profiles.is_empty() {
                    let ns = st(11.0, false, th.muted, HAlign::Left);
                    let r0 = y;
                    for l in transom_render::painter::wrap_text(p, &t("prof.none"), &ns, w)
                        .into_iter()
                        .take(3)
                    {
                        p.draw_text(&l, Rect::new(pad, y, w, line), &ns);
                        y += line;
                    }
                    nodes.push((
                        Rect::new(pad, r0, w, y - r0),
                        t("prof.none"),
                        static_extra(),
                        false,
                        None,
                    ));
                    y += px(4);
                }
                chips.push((t("prof.save"), AudioHit::ProfMode(1), false, t("prof.save")));
                if !self.profiles.is_empty() {
                    chips.push((
                        t("prof.delete"),
                        AudioHit::ProfMode(2),
                        false,
                        t("prof.delete"),
                    ));
                }
            }
            ProfMode::Save => {
                for (i, n) in names.iter().enumerate() {
                    let i = i as u8;
                    let saved = self.profiles.contains(&i);
                    chips.push((
                        n.clone(),
                        AudioHit::ProfSave(i),
                        false,
                        tf(
                            if saved {
                                "prof.overwrite_acc"
                            } else {
                                "prof.save_acc"
                            },
                            &[n],
                        ),
                    ));
                }
                chips.push((
                    t("prof.cancel"),
                    AudioHit::ProfMode(0),
                    false,
                    t("prof.cancel"),
                ));
            }
            ProfMode::Delete => {
                for i in &self.profiles {
                    let n = names[usize::from(*i)].clone();
                    chips.push((
                        n.clone(),
                        AudioHit::ProfDelete(*i),
                        false,
                        tf("prof.delete_acc", &[&n]),
                    ));
                }
                chips.push((
                    t("prof.cancel"),
                    AudioHit::ProfMode(0),
                    false,
                    t("prof.cancel"),
                ));
            }
        }
        if self.prof_mode != ProfMode::Apply {
            let key = if self.prof_mode == ProfMode::Save {
                "prof.save_help"
            } else {
                "prof.delete_help"
            };
            let ns = st(11.0, true, th.foreground, HAlign::Left);
            let r0 = y;
            for l in transom_render::painter::wrap_text(p, &t(key), &ns, w)
                .into_iter()
                .take(3)
            {
                p.draw_text(&l, Rect::new(pad, y, w, line), &ns);
                y += line;
            }
            nodes.push((
                Rect::new(pad, r0, w, y - r0),
                t(key),
                static_extra(),
                false,
                None,
            ));
            y += px(4);
        }
        let ch = px(28);
        let gap = px(6);
        let mut x = pad;
        for (label, hit, _, acc) in chips {
            let ls = st(12.0, false, th.foreground, HAlign::Center);
            let bw = (p.measure_text(&label, &ls).w + px(22)).min(w);
            if x > pad && x + bw > pad + w {
                x = pad;
                y += ch + gap;
            }
            let r = Rect::new(x, y, bw, ch);
            let hov = self.hover == Some(hit);
            let kind = transom_render::design::Kind::Secondary;
            let text = fit(p, &label, &ls, r.w - px(10));
            let b = transom_render::design::Button::new(&text, kind).state(
                transom_render::design::State {
                    hover: hov,
                    pressed: self.pressed == Some(hit) && hov,
                    focus: self.focus == Some(hit),
                    ..Default::default()
                },
            );
            transom_render::design::button(p, th, r, &b, th.background);
            self.hits.push((r, hit));
            nodes.push((
                r,
                acc,
                PageExtra {
                    role: ROLE_PUSHBUTTON,
                    state: 0,
                    value: String::new(),
                    action: t("acc.press"),
                },
                true,
                Some(hit),
            ));
            x += bw + gap;
        }
        y + ch + px(14)
    }

    /// Sekcia "Bluetooth": sparovane zvukove zariadenia s tlacidlom Pripojit / Odpojit. Vrati y pod nou.
    fn paint_bt(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        b: &transom_mixer::bt::BtSnap,
        (pad, w): (i32, i32),
        mut y: i32,
        nodes: &mut Vec<Node>,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let line = px(17);
        if !b.ready || b.devices.is_empty() {
            return y;
        }
        let lbl = st(11.0, true, th.accent_text, HAlign::Left);
        p.draw_text(&t("bt.title"), Rect::new(pad, y, w, line), &lbl);
        nodes.push((
            Rect::new(pad, y, w, line),
            t("bt.title"),
            static_extra(),
            false,
            None,
        ));
        y += line + px(4);
        for (i, d) in b.devices.iter().enumerate().take(8) {
            let hit = AudioHit::BtToggle(i as u8);
            let busy = b.busy.as_deref() == Some(d.key.as_str());
            let label = if busy {
                t(if d.connected {
                    "bt.disconnecting"
                } else {
                    "bt.connecting"
                })
            } else if d.connected {
                t("bt.disconnect")
            } else {
                t("bt.connect")
            };
            let ls = st(12.0, false, th.foreground, HAlign::Center);
            let bw = (p.measure_text(&label, &ls).w + px(22)).min(w / 2);
            let rh = px(30);
            let br = Rect::new(pad + w - bw, y, bw, rh);
            let ns = st(12.0, d.connected, th.foreground, HAlign::Left);
            let state = t(if d.connected {
                "bt.connected"
            } else {
                "bt.not_connected"
            });
            let ss = st(11.0, false, th.muted, HAlign::Left);
            let tw = w - bw - px(8);
            let name = fit(p, &d.name, &ns, tw);
            p.draw_text(&name, Rect::new(pad, y - px(1), tw, px(17)), &ns);
            let st_text = fit(p, &state, &ss, tw);
            p.draw_text(&st_text, Rect::new(pad, y + px(15), tw, px(15)), &ss);
            let hov = self.hover == Some(hit) && b.busy.is_none();
            let text = fit(p, &label, &ls, br.w - px(10));
            let bt =
                transom_render::design::Button::new(&text, transom_render::design::Kind::Secondary)
                    .state(transom_render::design::State {
                        hover: hov,
                        pressed: self.pressed == Some(hit) && hov,
                        focus: self.focus == Some(hit),
                        disabled: b.busy.is_some(),
                        ..Default::default()
                    });
            transom_render::design::button(p, th, br, &bt, th.background);
            if b.busy.is_none() {
                self.hits.push((br, hit));
            }
            nodes.push((
                br,
                tf("bt.acc", &[&label, &d.name, &state]),
                PageExtra {
                    role: ROLE_PUSHBUTTON,
                    state: 0,
                    value: String::new(),
                    action: t("acc.press"),
                },
                b.busy.is_none(),
                Some(hit),
            ));
            y += rh + px(6);
            if b.failed.as_deref() == Some(d.key.as_str()) {
                let es = st(11.0, false, th.danger, HAlign::Left);
                let r0 = y;
                for l in transom_render::painter::wrap_text(p, &t("bt.failed"), &es, w)
                    .into_iter()
                    .take(3)
                {
                    p.draw_text(&l, Rect::new(pad, y, w, line), &es);
                    y += line;
                }
                nodes.push((
                    Rect::new(pad, r0, w, y - r0),
                    t("bt.failed"),
                    static_extra(),
                    false,
                    None,
                ));
                y += px(4);
            }
        }
        y + px(8)
    }

    /// Po presune fokusu klavesnicou: zobrazit cely ovladac (posun obsahu; dokresli sa pri dalsom kresleni).
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

fn static_extra() -> PageExtra {
    PageExtra {
        role: ROLE_STATICTEXT,
        state: 0,
        value: String::new(),
        action: String::new(),
    }
}
