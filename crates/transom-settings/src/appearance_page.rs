//! Stranka "Pozadie listy": predvolby, vyber farby (systemovy dialog), HEX a RGB s validaciou, priehladne pozadie
//! a nepriehladnost so zivym nahladom. Farba prepisuje len pozadie; text a ikony ostavaju podla temy, kym su na farbe
//! citatelne (inak farby opacnej temy - `Theme::on_background`, rovnako ako lista).
//!
//! Vstupne polia HEX/R/G/B su natívne `EDIT` ovladace (kopirovanie, vyber, IME, citacky obrazovky zdarma);
//! ich poloha vychadza z [`layout`] (rovnaka funkcia ako pri kresleni, takze nikdy nesedia mimo).
use super::*;
use transom_config::{blend, contrast_ratio, format_hex, parse_hex, parse_rgb};

/// Predvolene farby pozadia (RGB): vychodzie farby oboch tem sa berú **z konstant `Theme`** (nemozu sa rozist), dalej
/// farby z identity (Ink-odtiene, hlboka modra, slivka, les, vino) a biela.
pub(crate) fn presets() -> [(u8, u8, u8); 8] {
    let rgb = |c: Color| (c.r, c.g, c.b);
    [
        rgb(Theme::dark().background),
        (0x00, 0x00, 0x00),
        (0x0F, 0x1B, 0x2D),
        (0x24, 0x1B, 0x3A),
        (0x14, 0x26, 0x1C),
        (0x3B, 0x1F, 0x2B),
        rgb(Theme::light().background),
        (0xFF, 0xFF, 0xFF),
    ]
}

/// Podklad pre nahlad priehladnosti: tmava a svetla plocha (jednofarebne, aby bol vplyv alfy zrejmy).
const DESK_DARK: (u8, u8, u8) = (0x23, 0x2B, 0x3A);
const DESK_LIGHT: (u8, u8, u8) = (0xE8, 0xE2, 0xD6);

/// Rozlozenie stranky v px klientskej oblasti (jedna funkcia pre kreslenie, zasahy mysou aj polohu ovladacov).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AppLayout {
    pub previews: [Rect; 2],
    pub theme_btn: Rect,
    pub swatches: Vec<Rect>,
    pub hex: Rect,
    pub rgb: [Rect; 3],
    pub pick: Rect,
    pub transparent: Rect,
    pub track: Rect,
    pub minus: Rect,
    pub plus: Rect,
    pub value: Rect,
    pub info_y: i32,
    pub reset: Rect,
    /// Riadok "ikona transom v oblasti oznameni" (zaskrtavacie policko + text) a pod nim poznamka.
    pub tray: Rect,
    /// Karty sekcii (farba, priehladnost, ikona v oblasti oznameni); spodok druhej a tretej sa doplni pri kresleni
    /// (vyska poznamok zavisi od jazyka).
    pub cards: [Rect; 3],
}

/// Svisla vyska casti stranky pod druhou kartou (kontrast, poznamky a tlacidlo obnovenia) v lp.
const INFO_H_LP: i32 = 152;

thread_local! {
    /// Skutocna vyska kontrastu a poznamok z posledneho kreslenia (lp; dlhe preklady a poznamky priehladnosti su vyssie
    /// nez `INFO_H_LP`) - tlacidlo obnovenia ide pod ne.
    static INFO_H: std::cell::Cell<i32> = const { std::cell::Cell::new(INFO_H_LP) };
    /// Vyska karty vyberu monitora z posledneho kreslenia (lp, aj s medzerou; 0 = bez vyberu): karty stranky su pod nou.
    static SCOPE_H: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

pub(crate) fn layout(scale: f32, x0: i32, y0: i32, w: i32) -> AppLayout {
    let px = |v: i32| (v as f32 * scale).round() as i32;
    let (pad_x, pad_y, gap) = (
        px(crate::CARD_PAD_X_LP),
        px(crate::CARD_PAD_Y_LP),
        px(crate::CARD_GAP_LP),
    );
    let card = |top: i32, bottom: i32| Rect::new(x0 - pad_x, top, w + 2 * pad_x, bottom - top);
    // nadpis (PageTitle) + pomocny text (az tri riadky Secondary)
    let mut y = y0 + px(44) + px(62) + px(SCOPE_H.with(|c| c.get()));
    // karta 1: nahlad nad tmavou a svetlou plochou, predvolby, HEX / RGB a vyber farby
    let c1 = y;
    y += pad_y;
    let half = (w - px(10)) / 2;
    let pv_h = px(28);
    let previews = [
        Rect::new(x0, y + px(22), half, pv_h),
        Rect::new(x0 + half + px(10), y + px(22), half, pv_h),
    ];
    y += px(22) + pv_h + px(18);
    // predvolby
    let sw = px(32);
    let theme_btn = Rect::new(x0, y, px(124), sw);
    let swatches = (0..presets().len() as i32)
        .map(|i| Rect::new(x0 + px(134) + i * (sw + px(6)), y, sw, sw))
        .collect();
    y += sw + px(14);
    // HEX + RGB + vyber
    let hex = Rect::new(x0 + px(60), y + px(3), px(90), px(26));
    let rgb = [0, 1, 2].map(|i| Rect::new(x0 + px(250) + i * px(54), y + px(3), px(48), px(26)));
    let pick = Rect::new(
        x0 + px(420),
        y,
        px(150).min(w - px(420)).max(px(110)),
        px(32),
    );
    y += px(32) + px(18);
    let c1b = y + pad_y - px(4);
    // karta 2: priehladnost, nepriehladnost, kontrast, poznamky, obnovenie
    y = c1b + gap;
    let c2 = y;
    y += pad_y;
    let transparent = Rect::new(x0, y, w, px(26));
    y += px(36);
    let track = Rect::new(x0 + px(120), y + px(9), px(220), px(14));
    let minus = Rect::new(track.x + track.w + px(14), y, px(32), px(32));
    let plus = Rect::new(minus.x + px(38), y, px(32), px(32));
    let value = Rect::new(plus.x + px(42), y, px(70), px(32));
    y += px(44);
    let info_y = y;
    let info_h = INFO_H.with(|c| c.get()).max(INFO_H_LP);
    let reset = Rect::new(x0, y + px(info_h - 40), px(220), px(32));
    let c2b = reset.y + reset.h + pad_y;
    // karta 3: ikona transom v oblasti oznameni
    let c3 = c2b + gap;
    let tray = Rect::new(x0, c3 + pad_y, w, px(26));
    let cards = [
        card(c1, c1b),
        card(c2, c2b),
        card(c3, tray.y + tray.h + px(48)),
    ];
    AppLayout {
        previews,
        theme_btn,
        swatches,
        hex,
        rgb,
        pick,
        transparent,
        track,
        minus,
        plus,
        value,
        info_y,
        reset,
        tray,
        cards,
    }
}

/// Nepriehladnost zo zasahu mysou na drahe posuvnika: 10..=100 %.
pub(crate) fn opacity_at(track: Rect, x: i32) -> u32 {
    let f = ((x - track.x) as f32 / track.w.max(1) as f32).clamp(0.0, 1.0);
    let lo = transom_config::MIN_OPACITY_PERCENT as f32;
    (lo + f * (100.0 - lo)).round() as u32
}

fn rgb_of(c: Color) -> (u8, u8, u8) {
    (c.r, c.g, c.b)
}

/// Podvolby ochrany OLED v poradi na stranke: (id, kluc textu). Prva je hlavny prepinac.
pub(crate) const OLED_TOGGLES: [(&str, &str); 6] = [
    ("enabled", "oled.on"),
    ("pixel_shift", "oled.shift"),
    ("dim_text", "oled.dim"),
    ("black_bg", "oled.black"),
    ("no_lines", "oled.lines"),
    ("auto_hide", "oled.hide"),
];

pub(crate) fn oled_get(o: &transom_config::Oled, id: &str) -> bool {
    match id {
        "enabled" => o.enabled,
        "pixel_shift" => o.pixel_shift,
        "dim_text" => o.dim_text,
        "black_bg" => o.black_bg,
        "no_lines" => o.no_lines,
        "auto_hide" => o.auto_hide,
        _ => false,
    }
}

pub(crate) fn oled_toggle(o: &mut transom_config::Oled, id: &str) {
    let v = !oled_get(o, id);
    match id {
        "enabled" => o.enabled = v,
        "pixel_shift" => o.pixel_shift = v,
        "dim_text" => o.dim_text = v,
        "black_bg" => o.black_bg = v,
        "no_lines" => o.no_lines = v,
        "auto_hide" => o.auto_hide = v,
        _ => {}
    }
}

impl Ui {
    /// Farba textu lisaty, ako ju lista prave pouzije (pre vyber vlastnej farby a kontrast).
    pub(crate) fn text_color_now(&self) -> (u8, u8, u8) {
        rgb_of(self.bar_preview_theme().foreground)
    }

    /// Tema lisaty podla nastaveni (vlastne pozadie, farba textu, ochrana OLED) - ta ista cesta ako v liste.
    pub(crate) fn bar_preview_theme(&self) -> Theme {
        let (bg, alpha) = self.cfg.appearance.effective(&self.cfg.oled);
        let text = match self.cfg.appearance.text() {
            transom_config::TextChoice::Auto => transom_render::BarText::Auto,
            transom_config::TextChoice::Light => transom_render::BarText::Light,
            transom_config::TextChoice::Dark => transom_render::BarText::Dark,
            transom_config::TextChoice::Custom((r, g, b)) => {
                transom_render::BarText::Custom(Color::rgb(r, g, b))
            }
        };
        let bg = bg.unwrap_or_else(|| rgb_of(self.theme.background));
        let th = self
            .theme
            .for_bar(Some(Color::rgb(bg.0, bg.1, bg.2)), alpha)
            .with_text(text);
        if self.cfg.oled.dim_text() {
            th.dimmed()
        } else {
            th
        }
    }

    /// Riadok so zaskrtavacim polickom a zalomenym textom (cely riadok je cielom kliknutia). Vrati y pod nim.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_row(
        &mut self,
        p: &mut dyn Painter,
        x0: i32,
        y: i32,
        w: i32,
        hit: Hit,
        key: &str,
        on: bool,
        indent: bool,
    ) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let dx = if indent { px(32) } else { 0 };
        let st = Ty::Body.style(th.foreground, HAlign::Left);
        let lh = px(Ty::Body.line_lp());
        let lines = Self::wrap_lines(p, &t(key), &st, w - dx - px(40));
        let h = (lines.len() as i32 * lh + px(8)).max(px(28));
        let row = Rect::new(x0 + dx, y, w - dx, h);
        if self.hover.as_ref() == Some(&hit) {
            p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
        }
        self.checkbox(
            p,
            Rect::new(row.x + px(4), row.y + px(4), px(20), px(20)),
            on,
        );
        for (k, l) in lines.iter().enumerate() {
            p.draw_text(
                l,
                Rect::new(
                    row.x + px(34),
                    row.y + px(4) + k as i32 * lh,
                    row.w - px(38),
                    lh,
                ),
                &st,
            );
        }
        self.tag(&hit, t(key), Some(on));
        self.hits.push((row, hit));
        y + h + px(4)
    }

    /// Karty "Farba textu" a "Ochrana OLED" pod kartou ikony v oblasti oznameni. Vrati y pod nimi.
    fn paint_text_and_oled(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let a = self.cfg.appearance.clone();
        // farba textu
        let mut y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("app.text_h"));
        let mode = a.text_mode.as_str();
        let custom = match a.text() {
            transom_config::TextChoice::Custom(c) => tf("app.text_custom_is", &[&format_hex(c)]),
            _ => t("app.text_custom"),
        };
        let prev = self.acc_group.replace(t("app.text_h"));
        y = self.flow_buttons(
            p,
            x0,
            y,
            w,
            vec![
                (t("app.text_auto"), Hit::TextMode("auto"), mode == "auto"),
                (t("app.text_light"), Hit::TextMode("light"), mode == "light"),
                (t("app.text_dark"), Hit::TextMode("dark"), mode == "dark"),
                (custom, Hit::TextPick, mode == "custom"),
            ],
        );
        self.acc_group = prev;
        // kontrast textu voci pozadiu lisaty (pri priehladnosti zavisi od plochy pod listou)
        let bt = self.bar_preview_theme();
        let (_, alpha) = a.effective(&self.cfg.oled);
        let ratio = contrast_ratio(rgb_of(bt.foreground), rgb_of(bt.background));
        let low = alpha == 255 && ratio < 4.5;
        let line = if alpha == 255 {
            tf("app.contrast", &[&format!("{ratio:.1}")])
        } else {
            t("app.text_depends")
        };
        p.draw_text(
            &line,
            Rect::new(x0, y, w, px(20)),
            &style(
                13.0,
                low,
                if low { th.danger } else { th.foreground },
                HAlign::Left,
            ),
        );
        y += px(22);
        let mut keys = vec!["app.text_note"];
        if low {
            keys.push("app.text_low");
        }
        y = self.notes(p, x0, y, w, &keys);
        y = self.card_end(s, x0, w, y);
        // ochrana OLED
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("oled.title"));
        y = self.notes(p, x0, y, w, &["oled.help"]) + px(8);
        let o = self.cfg.oled;
        for (i, (id, key)) in OLED_TOGGLES.into_iter().enumerate() {
            // podvolby su viditelne len pri zapnutej ochrane
            if i > 0 && !o.enabled {
                break;
            }
            y = self.check_row(
                p,
                x0,
                y,
                w,
                Hit::OledToggle(id),
                key,
                oled_get(&o, id),
                i > 0,
            );
        }
        if o.enabled && o.auto_hide {
            y = self.notes(p, x0 + px(32), y, w - px(32), &["oled.hide_note"]);
        }
        self.card_end(s, x0, w, y)
    }

    /// Aktualne pozadie (vlastna farba, inak farba temy).
    pub(crate) fn effective_bg(&self) -> (u8, u8, u8) {
        self.cfg
            .appearance
            .color()
            .unwrap_or_else(|| rgb_of(self.theme.background))
    }

    pub(crate) fn paint_appearance(&mut self, p: &mut dyn Painter, x0: i32, y0: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let lay = layout(s, x0, y0, w);
        let a = self.cfg.appearance.clone();
        let (eff_bg, alpha) = a.effective(&self.cfg.oled);
        let bg = eff_bg.unwrap_or_else(|| self.effective_bg());

        self.title(p, x0, y0, w, "app.title");
        self.paragraph(p, x0, y0 + px(44), w, &t("app.help"), 12.0, th.muted);
        // vyber monitora pod pomocnym textom; karty stranky sa posunu o jeho vysku (pri zmene az pri dalsom kresleni)
        {
            let top = y0 + px(44) + px(62);
            let end = self.paint_scope(p, x0, top, w);
            let h = ((end - top) as f32 / s).ceil() as i32;
            if SCOPE_H.with(|c| c.replace(h)) != h {
                self.repaint = true;
            }
        }
        // karty sekcii z rozlozenia (spodok tretej podla skutocnej vysky poznamky - doplni sa nizsie)
        self.cards.extend_from_slice(&lay.cards[..2]);

        // nahlad: skutocna scena nad tmavou a svetlou plochou; pozadie = farba s alfou nad plochou
        for (i, (desk, key)) in [
            (DESK_DARK, "app.preview_dark"),
            (DESK_LIGHT, "app.preview_light"),
        ]
        .into_iter()
        .enumerate()
        {
            let r = lay.previews[i];
            p.draw_text(
                &t(key),
                Rect::new(r.x, r.y - px(22), r.w, px(18)),
                &Ty::Secondary.style(th.muted, HAlign::Left),
            );
            let shown = blend(bg, desk, alpha);
            // text ako na liste (ta ista funkcia: tema / opacna tema / zvolena farba textu, ochrana OLED)
            let mut theme = self.bar_preview_theme();
            theme.background = Color::rgb(shown.0, shown.1, shown.2);
            // okolie listy = plocha (aby bola priehladnost viditelna aj na okrajoch)
            p.fill_rect(
                Rect::new(r.x - px(2), r.y - px(2), r.w + px(4), r.h + px(4)),
                Color::rgb(desk.0, desk.1, desk.2),
            );
            let mut sub = Offset {
                inner: p,
                dx: r.x,
                dy: r.y,
                w: r.w,
                h: r.h,
            };
            render_bar(&mut sub, &theme, &preview_items(), Interaction::default());
        }

        // predvolby
        self.button(
            p,
            lay.theme_btn,
            &t("app.theme_default"),
            Hit::ThemeColor,
            a.color().is_none(),
        );
        for (i, r) in lay.swatches.iter().enumerate() {
            let c = presets()[i];
            let on = a.color() == Some(c);
            // zvolena: akcentovy ramik + medzera + znacka (nie len farba); inak jemny okraj, pod mysou vyraznejsi
            let rad = px(design::radius::CONTROL + 2);
            if on {
                let b = px(2).max(2);
                let g = px(2).max(1);
                p.fill_round_rect(*r, rad, th.accent);
                p.fill_round_rect(
                    Rect::new(r.x + b, r.y + b, r.w - 2 * b, r.h - 2 * b),
                    (rad - b).max(0),
                    th.surface,
                );
                let i = b + g;
                p.fill_round_rect(
                    Rect::new(r.x + i, r.y + i, r.w - 2 * i, r.h - 2 * i),
                    (rad - i).max(0),
                    Color::rgb(c.0, c.1, c.2),
                );
                // znacka v kontrastnej farbe k vzorke
                let lum = 0.2126 * c.0 as f32 + 0.7152 * c.1 as f32 + 0.0722 * c.2 as f32;
                p.draw_text(
                    "\u{E73E}",
                    *r,
                    &TextStyle {
                        font: Font::Icons,
                        size_lp: 12.0,
                        bold: false,
                        color: if lum > 140.0 {
                            Color::rgb(0, 0, 0)
                        } else {
                            Color::rgb(255, 255, 255)
                        },
                        align: HAlign::Center,
                    },
                );
            } else {
                let hov = self.hover == Some(Hit::Swatch(i));
                let b = px(1).max(1);
                // obrys `muted` (>= 3 : 1 na karte): vzorka splyvajuca s plochou ma stale zretelnu hranicu
                p.fill_round_rect(*r, rad, if hov { th.foreground } else { th.muted });
                p.fill_round_rect(
                    Rect::new(r.x + b, r.y + b, r.w - 2 * b, r.h - 2 * b),
                    (rad - b).max(0),
                    Color::rgb(c.0, c.1, c.2),
                );
            }
            let sw = crate::page_a11y::acc_label(&Hit::Swatch(i), "", None);
            self.tag(&Hit::Swatch(i), sw, Some(on));
            self.hits.push((*r, Hit::Swatch(i)));
        }

        // popisky k poliam a vyznacenie neplatnych hodnot (cerveny ramik + text)
        let lbl = |p: &mut dyn Painter, r: Rect, key: &str| {
            p.draw_text(
                &t(key),
                Rect::new(r.x - px(58), r.y, px(54), r.h),
                &Ty::Body.style(th.foreground, HAlign::Right),
            );
        };
        lbl(p, lay.hex, "app.hex");
        p.draw_text(
            &t("app.rgb"),
            Rect::new(lay.rgb[0].x - px(98), lay.hex.y, px(92), lay.hex.h),
            &Ty::Body.style(th.foreground, HAlign::Right),
        );
        let fields = [lay.hex, lay.rgb[0], lay.rgb[1], lay.rgb[2]];
        for (i, f) in fields.iter().enumerate() {
            if self.bad[i] {
                let b = px(2);
                p.fill_rect(Rect::new(f.x - b, f.y - b, f.w + 2 * b, b), th.low);
                p.fill_rect(Rect::new(f.x - b, f.y + f.h, f.w + 2 * b, b), th.low);
                p.fill_rect(Rect::new(f.x - b, f.y, b, f.h), th.low);
                p.fill_rect(Rect::new(f.x + f.w, f.y, b, f.h), th.low);
            }
        }
        if self.bad.iter().any(|b| *b) {
            p.draw_text(
                &t("app.invalid"),
                Rect::new(lay.pick.x, lay.hex.y + px(30), lay.pick.w + px(120), px(16)),
                &TextStyle {
                    bold: true,
                    ..Ty::Caption.style(th.danger, HAlign::Left)
                },
            );
        }
        self.button(p, lay.pick, &t("app.pick"), Hit::Pick, false);

        // priehladne pozadie (zaskrtavacie policko) a nepriehladnost
        let on = a.bg_transparent;
        let row = lay.transparent;
        if self.hover == Some(Hit::Transparent) {
            p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
        }
        self.checkbox(
            p,
            Rect::new(row.x + px(2), row.y + px(3), px(20), px(20)),
            on,
        );
        p.draw_text(
            &t("app.transparent"),
            Rect::new(row.x + px(32), row.y, row.w - px(32), row.h),
            &Ty::Body.style(th.foreground, HAlign::Left),
        );
        self.tag(&Hit::Transparent, t("app.transparent"), Some(on));
        self.hits.push((row, Hit::Transparent));

        let dim = if on { th.muted } else { th.foreground };
        p.draw_text(
            &t("app.opacity"),
            Rect::new(x0, lay.minus.y, px(112), lay.minus.h),
            &Ty::Body.style(dim, HAlign::Left),
        );
        let tr = lay.track;
        let pct = a.opacity_percent();
        let f = (pct - transom_config::MIN_OPACITY_PERCENT) as f32
            / (100 - transom_config::MIN_OPACITY_PERCENT) as f32;
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
            if on { th.muted } else { th.accent },
        );
        let tx = tr.x + (tr.w as f32 * f) as i32;
        p.fill_round_rect(
            Rect::new(tx - px(7), tr.y - px(2), px(14), tr.h + px(4)),
            px(7),
            if on { th.muted } else { th.foreground },
        );
        self.tag(&Hit::OpacityTrack, t("app.opacity"), None);
        self.hits.push((
            Rect::new(tr.x - px(6), tr.y - px(6), tr.w + px(12), tr.h + px(12)),
            Hit::OpacityTrack,
        ));
        self.button(p, lay.minus, "−", Hit::OpacityStep(-5), false);
        self.button(p, lay.plus, "+", Hit::OpacityStep(5), false);
        p.draw_text(
            &tf("app.opacity_value", &[&pct.to_string()]),
            lay.value,
            &Ty::BodyStrong.style(dim, HAlign::Left),
        );

        // kontrast a poznamky
        let fg = self.bar_preview_theme().foreground;
        let ratio = contrast_ratio(rgb_of(fg), bg);
        let low = alpha == 255 && ratio < 4.5;
        let mut y = lay.info_y;
        p.draw_text(
            &format!(
                "{}{}",
                tf("app.contrast", &[&format!("{ratio:.1}")]),
                if alpha == 255 {
                    String::new()
                } else {
                    format!(" · {}", t("app.contrast_depends"))
                }
            ),
            Rect::new(x0, y, w, px(20)),
            &style(
                13.0,
                low,
                if low { th.danger } else { th.foreground },
                HAlign::Left,
            ),
        );
        y += px(20);
        if low {
            p.draw_text(
                &t("app.contrast_low"),
                Rect::new(x0, y, w, px(18)),
                &TextStyle {
                    bold: true,
                    ..Ty::Secondary.style(th.danger, HAlign::Left)
                },
            );
        }
        y += px(24);
        let keys: &[&str] = if alpha == 255 {
            &["app.note_opaque"]
        } else {
            &["app.note_layered1", "app.note_layered2"]
        };
        y = self.notes(p, x0, y, w, keys) + px(6);
        let end = self.notes(p, x0, y, w, &["app.note_reserve"]);
        // vyska bloku podla skutocneho textu; ak sa zmenila, rozlozenie sa prepocita pri dalsom kresleni
        let need = ((end - lay.info_y) as f32 / s).ceil() as i32 + 12 + 40;
        let want = need.max(INFO_H_LP);
        if INFO_H.with(|c| c.replace(want)) != want {
            self.repaint = true;
        }
        self.button(p, lay.reset, &t("app.reset"), Hit::ResetAppearance, false);

        // ikona transom v oblasti oznameni
        let row = lay.tray;
        let on = a.tray_icon;
        if self.hover == Some(Hit::TrayIcon) {
            p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
        }
        self.checkbox(
            p,
            Rect::new(row.x + px(2), row.y + px(3), px(20), px(20)),
            on,
        );
        p.draw_text(
            &t("app.tray_icon"),
            Rect::new(row.x + px(32), row.y, row.w - px(32), row.h),
            &Ty::Body.style(th.foreground, HAlign::Left),
        );
        self.tag(&Hit::TrayIcon, t("app.tray_icon"), Some(on));
        self.hits.push((row, Hit::TrayIcon));
        let mut end = self.notes(
            p,
            row.x + px(32),
            row.y + row.h + px(2),
            w - px(32),
            &["app.tray_note"],
        );
        // cierne sklo: pod panelmi a menu (aj vzhlad tohto okna) a volitelne pod listou
        for (hit, on, key, note) in [
            (
                Hit::GlassPanels,
                a.glass_panels,
                "app.glass_panels",
                "app.glass_panels_note",
            ),
            (
                Hit::GlassBar,
                a.glass_bar,
                "app.glass_bar",
                "app.glass_bar_note",
            ),
        ] {
            let row = Rect::new(row.x, end + px(10), row.w, row.h);
            if self.hover == Some(hit.clone()) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            self.checkbox(
                p,
                Rect::new(row.x + px(2), row.y + px(3), px(20), px(20)),
                on,
            );
            p.draw_text(
                &t(key),
                Rect::new(row.x + px(32), row.y, row.w - px(32), row.h),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            self.tag(&hit, t(key), Some(on));
            self.hits.push((row, hit));
            end = self.notes(
                p,
                row.x + px(32),
                row.y + row.h + px(2),
                w - px(32),
                &[note],
            );
        }
        // priehladnost panelov (len so sklom): styri stupne
        if a.glass_panels {
            let y = end + px(12);
            p.draw_text(
                &t("app.glass_opacity"),
                Rect::new(row.x, y, row.w, px(22)),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            let steps = transom_config::GLASS_OPACITY_STEPS;
            let n = steps.len() as i32;
            let gap = px(6);
            let bw = (row.w - (n - 1) * gap) / n;
            let keys = [
                "app.glass_op_1",
                "app.glass_op_2",
                "app.glass_op_3",
                "app.glass_op_4",
            ];
            for (k, v) in steps.iter().enumerate() {
                let r = Rect::new(
                    row.x + k as i32 * (bw + gap),
                    y + px(26),
                    bw,
                    px(design::CONTROL_H_LP),
                );
                let active = a.glass_opacity == *v;
                self.button(p, r, &t(keys[k]), Hit::GlassOpacity(*v), active);
            }
            end = y + px(26) + px(design::CONTROL_H_LP);
        }
        let c3 = lay.cards[2];
        self.cards.push(Rect::new(
            c3.x,
            c3.y,
            c3.w,
            end + px(crate::CARD_PAD_Y_LP) - c3.y,
        ));
        let next = end + px(crate::CARD_PAD_Y_LP) + px(crate::CARD_GAP_LP);
        self.paint_text_and_oled(p, x0, next, w);
    }

    /// Pole farby dostalo fokus: zapamata sa hodnota spred editacie (na Esc).
    pub(crate) fn edit_focus(&mut self, i: usize) {
        if i < 4 {
            self.edit_session = Some(EditSession::begin(&self.cfg.appearance.bg_color));
        }
    }

    /// Pole farby stratilo fokus: vrati, ci sa ma hodnota ulozit (pisalo sa do nej a nebolo to zrusene Esc).
    pub(crate) fn edit_blur(&mut self, i: usize) -> bool {
        i < 4 && self.edit_session.take().is_some_and(EditSession::blur)
    }

    /// Esc v poli farby: obnovi hodnotu spred editacie (aj texty vsetkych poli) a nic neulozi. Vrati, ci sa nieco obnovilo.
    pub(crate) fn edit_cancel(&mut self, i: usize) -> bool {
        if i >= 4 {
            return false;
        }
        let Some(s) = self.edit_session.take() else {
            return false;
        };
        self.cfg.appearance.bg_color = s.original;
        self.bad = [false; 4];
        self.fill_edits(None);
        true
    }

    /// Nastavi farbu z RGB (napr. z dialogu alebo predvolby) a ulozi.
    pub(crate) fn set_bg(&mut self, c: Option<(u8, u8, u8)>) {
        self.cfg.appearance.bg_color = c.map(format_hex).unwrap_or_default();
        self.bad = [false; 4];
    }

    /// Prepise texty poli podla aktualnej farby (vstupne polia sa pocas toho neberu ako zmena).
    pub(crate) fn fill_edits(&mut self, except: Option<usize>) {
        let c = self.effective_bg();
        let texts = [
            format_hex(c),
            c.0.to_string(),
            c.1.to_string(),
            c.2.to_string(),
        ];
        self.syncing = true;
        for (i, tx) in texts.iter().enumerate() {
            if Some(i) != except && self.edits[i] != 0 {
                set_edit_text(self.edits[i], tx);
            }
        }
        self.syncing = false;
    }

    /// Zmena textu v poli `i`: platna hodnota sa prejavi v nahlade a ulozi, neplatna oznaci pole.
    pub(crate) fn on_edit_change(&mut self, i: usize) {
        if self.syncing || self.edits[i] == 0 {
            return;
        }
        if i == 5 {
            // pole hladania miesta (pocasie): ziadna farba, len dotaz
            let text = get_edit_text(self.edits[5]);
            self.wx.restart = self.wx_query_changed(&text);
            return;
        }
        if i == 4 {
            // pole datumu (kalendar): len zrusi chybovu hlasku; ulozi sa az tlacidlom Pridat/Ulozit
            self.form.error = false;
            return;
        }
        let text = get_edit_text(self.edits[i]);
        // hodnota spred editacie (pre Esc), ak pole nedostalo EN_SETFOCUS
        let before = self.cfg.appearance.bg_color.clone();
        if i == 0 {
            match parse_hex(&text) {
                Some(c) => {
                    // nahlad hned; ulozi sa az pri Enter / Tab / strate fokusu (Esc hodnotu vrati a neulozi nic)
                    self.set_bg(Some(c));
                    self.fill_edits(Some(0));
                    self.mark_typed(before);
                }
                None => self.bad[0] = true,
            }
        } else {
            let all: Vec<String> = (1..4).map(|k| get_edit_text(self.edits[k])).collect();
            // kazdy kanal zvlast (oznaci sa len ten chybny)
            for k in 0..3 {
                self.bad[k + 1] = parse_rgb(&all[k], "0", "0").is_none();
            }
            if let Some(c) = parse_rgb(&all[0], &all[1], &all[2]) {
                self.set_bg(Some(c));
                self.fill_edits(Some(i));
                self.mark_typed(before);
            }
        }
    }

    /// Do pola sa pisalo platnou hodnotou: ulozenie caka na potvrdenie (nie po kazdom znaku).
    fn mark_typed(&mut self, before: String) {
        self.edit_session
            .get_or_insert_with(|| EditSession::begin(&before))
            .typed();
    }
}

/// Relacia upravy poli farby (HEX, R, G, B): hodnota spred editacie a ci sa do pola pisalo. Cista logika: Esc obnovi
/// `original` bez ulozenia, strata fokusu uklada len ak sa pisalo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditSession {
    pub original: String,
    dirty: bool,
}

impl EditSession {
    pub fn begin(bg_color: &str) -> Self {
        Self {
            original: bg_color.to_string(),
            dirty: false,
        }
    }
    pub fn typed(&mut self) {
        self.dirty = true;
    }
    /// Strata fokusu (Enter, Tab, klik inam): ulozit, ak sa pisalo.
    pub fn blur(self) -> bool {
        self.dirty
    }
}

/// Klavesova zmena nepriehladnosti.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SliderKey {
    Dec,
    Inc,
    PageDec,
    PageInc,
    Min,
    Max,
}

/// Obmedzi nepriehladnost na povoleny rozsah (`MIN_OPACITY_PERCENT`..100).
pub(crate) fn clamp_opacity(v: i32) -> u32 {
    v.clamp(transom_config::MIN_OPACITY_PERCENT as i32, 100) as u32
}

/// Nova nepriehladnost po klavese: krok 5 % (rovnako ako tlacidla +/-), s Shift 1 %, Page 10 %, Home minimum, End 100 %.
pub(crate) fn step_opacity(cur: u32, key: SliderKey, fine: bool) -> u32 {
    let small = if fine { 1 } else { 5 };
    let cur = cur as i32;
    clamp_opacity(match key {
        SliderKey::Dec => cur - small,
        SliderKey::Inc => cur + small,
        SliderKey::PageDec => cur - 10,
        SliderKey::PageInc => cur + 10,
        SliderKey::Min => 0,
        SliderKey::Max => 100,
    })
}

/// Ukazkove polozky nahladu pozadia (male: hladat, pauza, kruh, hodiny).
fn preview_items() -> Vec<StatusItem> {
    vec![
        StatusItem::new(
            ItemId::Search,
            Side::Left,
            vec![Part::button(Glyph::Search, Action::Search, true, "")],
        ),
        StatusItem::new(
            ItemId::Media,
            Side::Left,
            vec![Part::button(Glyph::Pause, Action::MediaToggle, true, "")],
        ),
        StatusItem::new(
            ItemId::Ai,
            Side::Right,
            vec![Part::Ring {
                fill: Some(62),
                ccw: false,
                muted: false,
                alert: false,
                label: "62 %".into(),
                name: String::new(),
                action: None,
            }],
        ),
        StatusItem::new(ItemId::Clock, Side::Right, vec![Part::text("17:25")]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_colour_and_oled_options_are_saved_and_the_preview_uses_them() {
        use crate::archive_tests::Env;
        let e = Env::new("app-oled");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Appearance));
        let saved = || transom_config::load(&e.root.join("config.json")).config;
        ui.click(Hit::TextMode("light"));
        assert_eq!(saved().appearance.text_mode, "light");
        assert_eq!(ui.bar_preview_theme().foreground, Theme::dark().foreground);
        ui.click(Hit::TextMode("auto"));
        assert_eq!(saved().appearance.text(), transom_config::TextChoice::Auto);
        // vlastna farba ide cez systemovy dialog: klik ho len naplanuje a nic neulozi
        assert!(!ui.pending_text_pick);
        ui.click(Hit::TextPick);
        assert!(ui.pending_text_pick);
        assert_eq!(saved().appearance.text_mode, "auto");
        // OLED: predvolene vypnute; hlavny prepinac zapne ochranu s predvolenymi podvolbami (bez skryvania)
        assert!(!saved().oled.enabled);
        ui.click(Hit::OledToggle("enabled"));
        let o = saved().oled;
        assert!(o.enabled && o.pixel_shift() && o.black_bg() && !o.auto_hide());
        ui.click(Hit::OledToggle("auto_hide"));
        ui.click(Hit::OledToggle("black_bg"));
        let o = saved().oled;
        assert!(o.auto_hide() && !o.black_bg());
        // nahlad: cierne pozadie a stlmeny text, ked su zapnute
        ui.click(Hit::OledToggle("black_bg"));
        let th = ui.bar_preview_theme();
        assert_eq!(th.background, Color::rgb(0, 0, 0));
        assert!(transom_render::audit::ratio(th.foreground, th.background) >= 4.5);
        for (id, _) in OLED_TOGGLES {
            let before = oled_get(&ui.cfg.oled, id);
            ui.click(Hit::OledToggle(id));
            assert_eq!(oled_get(&saved().oled, id), !before, "{id}");
        }
    }

    #[test]
    fn layout_is_consistent_at_all_dpis_and_fits_the_window() {
        for scale in [1.0f32, 1.5, 2.0] {
            let w = (580.0 * scale) as i32;
            let l = layout(scale, (204.0 * scale) as i32, (16.0 * scale) as i32, w);
            // nic sa neprekryva: policia v jednom riadku idu zlava doprava
            let row = [l.hex, l.rgb[0], l.rgb[1], l.rgb[2], l.pick];
            for pair in row.windows(2) {
                assert!(
                    pair[0].x + pair[0].w <= pair[1].x,
                    "scale {scale}: {pair:?}"
                );
            }
            // zotrvava v okne (720 lp vysoke)
            assert!(
                l.reset.y + l.reset.h < (700.0 * scale) as i32,
                "scale {scale}: {l:?}"
            );
            assert_eq!(l.swatches.len(), presets().len());
            for pair in l.swatches.windows(2) {
                assert!(pair[0].x + pair[0].w < pair[1].x);
            }
            assert!(
                l.swatches.last().map(|r| r.x + r.w).unwrap() <= l.hex.x + w,
                "predvolby v sirke"
            );
        }
    }

    #[test]
    fn slider_maps_to_limits() {
        let tr = Rect::new(100, 0, 200, 14);
        assert_eq!(opacity_at(tr, -50), transom_config::MIN_OPACITY_PERCENT);
        assert_eq!(opacity_at(tr, 100), transom_config::MIN_OPACITY_PERCENT);
        assert_eq!(opacity_at(tr, 300), 100);
        assert_eq!(opacity_at(tr, 9999), 100);
        let mid = opacity_at(tr, 200);
        assert!((54..=56).contains(&mid), "{mid}");
        // monotonne
        let mut prev = 0;
        for x in 100..=300 {
            let v = opacity_at(tr, x);
            assert!(v >= prev);
            prev = v;
        }
    }

    #[test]
    fn theme_default_presets_match_the_themes() {
        assert_eq!(presets()[0], rgb_of(Theme::dark().background));
        assert_eq!(presets()[6], rgb_of(Theme::light().background));
    }

    #[test]
    fn opacity_keys_step_by_5_fine_by_1_page_by_10_and_stay_in_range() {
        use SliderKey::*;
        let min = transom_config::MIN_OPACITY_PERCENT;
        assert_eq!(step_opacity(80, Dec, false), 75);
        assert_eq!(step_opacity(80, Inc, false), 85);
        assert_eq!(step_opacity(80, Dec, true), 79);
        assert_eq!(step_opacity(80, Inc, true), 81);
        assert_eq!(step_opacity(80, PageDec, false), 70);
        assert_eq!(step_opacity(80, PageInc, true), 90, "Page nie je jemny");
        assert_eq!(step_opacity(80, Min, false), min);
        assert_eq!(step_opacity(40, Max, false), 100);
        // okraje: nikdy pod minimum ani nad 100
        assert_eq!(step_opacity(min, Dec, false), min);
        assert_eq!(step_opacity(min + 3, PageDec, false), min);
        assert_eq!(step_opacity(100, Inc, false), 100);
        assert_eq!(step_opacity(97, PageInc, false), 100);
        // zhoduje sa s tlacidlami +/- (OpacityStep)
        assert_eq!(clamp_opacity(80 + 5), step_opacity(80, Inc, false));
        assert_eq!(clamp_opacity(0), min);
        assert_eq!(clamp_opacity(1000), 100);
    }

    #[test]
    fn an_edit_session_saves_only_if_typed_and_remembers_the_original() {
        let mut s = EditSession::begin("#112233");
        assert_eq!(s.original, "#112233");
        assert!(!s.clone().blur(), "bez pisania sa neuklada");
        s.typed();
        assert!(s.clone().blur(), "po pisani sa pri strate fokusu ulozi");
        assert_eq!(EditSession::begin("").original, "", "prazdne = farba temy");
    }
}
