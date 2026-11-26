//! Stranka "Zvuky rozhrania": zapnutie (vypnutie = stlmenie), hlasitost zvukov transom a ukazka.
//!
//! Hlasitost je hlasitost **samostatnej relacie transom** v miesadle hlasitosti Windows (linearne, 0..=100 %). Nikdy
//! sa nedotyka prehravania medii, vyberu vystupneho zariadenia, inych aplikacii ani hlavnej hlasitosti systemu.
//! Nastavenie sa uklada do `config.json` (`ui_sounds`) a lista ho prevezme na zaklade spravy o zmene konfiguracie;
//! tlacidlo "Vyskusat" prehra zvuk v tomto procese s rovnakou hlasitostou, aby sa dalo odskusat bez listy.
use super::*;
use crate::appearance_page::SliderKey;
use transom_config::MAX_SOUND_VOLUME;

/// Hlasitost zo zasahu mysou na drahe posuvnika: 0..=100 %.
pub(crate) fn volume_at(track: Rect, x: i32) -> u32 {
    let f = ((x - track.x) as f32 / track.w.max(1) as f32).clamp(0.0, 1.0);
    (f * MAX_SOUND_VOLUME as f32).round() as u32
}

/// Obmedzi hlasitost na povoleny rozsah 0..=100.
pub(crate) fn clamp_volume(v: i32) -> u32 {
    v.clamp(0, MAX_SOUND_VOLUME as i32) as u32
}

/// Nova hlasitost po klavese: krok 5 % (rovnako ako tlacidla +/-), s Shift 1 %, Page 10 %, Home 0 %, End 100 %.
pub(crate) fn step_volume(cur: u32, key: SliderKey, fine: bool) -> u32 {
    let small = if fine { 1 } else { 5 };
    let cur = cur as i32;
    clamp_volume(match key {
        SliderKey::Dec => cur - small,
        SliderKey::Inc => cur + small,
        SliderKey::PageDec => cur - 10,
        SliderKey::PageInc => cur + 10,
        SliderKey::Min => 0,
        SliderKey::Max => MAX_SOUND_VOLUME as i32,
    })
}

impl Ui {
    /// Prehra ukazku pri aktualnej hlasitosti (bez hlasky). Vrati, ci sa zaradila (nie je stlmene, nie je 0 %).
    pub(crate) fn play_preview(&mut self) -> bool {
        let u = self.cfg.ui_sounds;
        if !u.enabled || u.volume == 0 {
            return false;
        }
        self.sound.preview(u.volume)
    }

    /// Tlacidlo "Vyskusat": pri stlmenych zvukoch vysvetli, preco nic nehra.
    pub(crate) fn preview_button(&mut self) {
        let u = self.cfg.ui_sounds;
        if !u.enabled || u.volume == 0 {
            self.status = t("snd.muted_hint");
            return;
        }
        if self.play_preview() {
            self.status = tf("snd.previewed", &[&u.volume.to_string()]);
        }
    }

    pub(crate) fn paint_sound(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "snd.title");
        y = self.notes(p, x0, y, w, &["snd.help1", "snd.help2"]);
        y += px(12);
        y = self.card_begin(s, y);

        // zapnutie (vypnute = stlmenie)
        let on = self.cfg.ui_sounds.enabled;
        let row = Rect::new(x0, y, w, px(28));
        if self.hover == Some(Hit::SoundToggle) {
            p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
        }
        self.checkbox(
            p,
            Rect::new(row.x + px(4), row.y + px(4), px(20), px(20)),
            on,
        );
        p.draw_text(
            &t("snd.enable"),
            Rect::new(row.x + px(34), row.y, row.w - px(34), row.h),
            &Ty::Body.style(th.foreground, HAlign::Left),
        );
        self.tag(&Hit::SoundToggle, t("snd.enable"), Some(on));
        self.hits.push((row, Hit::SoundToggle));
        y += px(42);

        // hlasitost: posuvnik, tlacidla -/+ a hodnota
        let dim = if on { th.foreground } else { th.muted };
        p.draw_text(
            &t("snd.volume"),
            Rect::new(x0, y, px(112), px(32)),
            &Ty::Body.style(dim, HAlign::Left),
        );
        let tr = Rect::new(x0 + px(120), y + px(9), px(220), px(14));
        self.sound_track = tr;
        let vol = self.cfg.ui_sounds.volume.min(MAX_SOUND_VOLUME);
        let f = vol as f32 / MAX_SOUND_VOLUME as f32;
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
            if on { th.accent } else { th.muted },
        );
        let tx = tr.x + (tr.w as f32 * f) as i32;
        p.fill_round_rect(
            Rect::new(tx - px(7), tr.y - px(2), px(14), tr.h + px(4)),
            px(7),
            if on { th.foreground } else { th.muted },
        );
        self.tag(&Hit::SoundTrack, t("snd.volume"), None);
        self.hits.push((
            Rect::new(tr.x - px(6), tr.y - px(6), tr.w + px(12), tr.h + px(12)),
            Hit::SoundTrack,
        ));
        let minus = Rect::new(tr.x + tr.w + px(14), y, px(32), px(32));
        let plus = Rect::new(minus.x + px(38), y, px(32), px(32));
        self.button(p, minus, "−", Hit::SoundStep(-5), false);
        self.button(p, plus, "+", Hit::SoundStep(5), false);
        p.draw_text(
            &tf("snd.volume_value", &[&vol.to_string()]),
            Rect::new(plus.x + px(42), y, px(70), px(32)),
            &Ty::BodyStrong.style(dim, HAlign::Left),
        );
        y += px(48);

        // ukazka (pri stlmenych zvukoch vysvetli, preco nehra)
        let label = t("snd.preview");
        let bw = design::button_width(
            p,
            &design::Button::new(&label, Kind::Secondary).icon('\u{E768}'),
        )
        .max(px(140))
        .min(w);
        self.button(
            p,
            Rect::new(x0, y, bw, px(design::CONTROL_H_LP)),
            &label,
            Hit::SoundPreview,
            false,
        );
        y += px(design::CONTROL_H_LP + 14);
        y = self.notes(p, x0, y, w, &["snd.note_default"]);
        self.card_end(s, x0, w, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_from_the_mouse_is_always_within_bounds_and_linear() {
        let tr = Rect::new(100, 0, 200, 14);
        assert_eq!(volume_at(tr, 100), 0);
        assert_eq!(volume_at(tr, 300), 100);
        assert_eq!(volume_at(tr, 200), 50);
        assert_eq!(volume_at(tr, -9999), 0);
        assert_eq!(volume_at(tr, 99_999), 100);
        assert_eq!(
            volume_at(Rect::new(0, 0, 0, 0), 5),
            100,
            "nulova draha nepadne"
        );
    }

    #[test]
    fn keys_step_clamp_and_jump_to_the_ends() {
        use SliderKey::*;
        assert_eq!(step_volume(25, Inc, false), 30);
        assert_eq!(step_volume(25, Inc, true), 26);
        assert_eq!(step_volume(25, Dec, false), 20);
        assert_eq!(step_volume(3, Dec, false), 0);
        assert_eq!(step_volume(98, PageInc, false), 100);
        assert_eq!(step_volume(5, PageDec, false), 0);
        assert_eq!(step_volume(50, Min, false), 0);
        assert_eq!(step_volume(50, Max, false), 100);
        assert_eq!(clamp_volume(-40), 0);
        assert_eq!(clamp_volume(400), 100);
    }
}
