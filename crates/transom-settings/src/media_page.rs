//! Stranka "Media": posuvny nazov skladby (zap/vyp, smer, rychlost, pri pozastaveni) so zivym nahladom.
use super::*;
use transom_core::marquee;

/// Ukazkovy dlhy nazov (v jazyku aplikacie), ktory sa v nahlade nezmesti.
const PREVIEW_W_LP: i32 = 330;

/// Animacie vo Windows (`SPI_GETCLIENTAREAANIMATION`). V testoch pevna hodnota z `TEST_ANIMATIONS` (predvolene zapnute),
/// aby vykreslenie nezaviselo od nastavenia stroja (runner GitHubu ma animacie vypnute).
pub(crate) fn animations_enabled() -> bool {
    #[cfg(test)]
    {
        TEST_ANIMATIONS.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        transom_platform::system::animations_enabled()
    }
}

#[cfg(test)]
thread_local! {
    /// Nastavenie animacii pre testy na tomto vlakne.
    pub(crate) static TEST_ANIMATIONS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

impl Ui {
    pub(crate) fn paint_media(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "med.title");
        y = self.paragraph(p, x0, y, w, &t("med.help"), 12.0, th.muted) + px(12);
        y = self.card_begin(s, y);

        // zivy nahlad: tlacidlo, obal a nazov v rovnakom oramovani ako v lište; text sa posuva podla nastaveni
        let box_r = Rect::new(x0, y, px(PREVIEW_W_LP), px(36));
        p.fill_round_rect(box_r, px(design::radius::CONTROL), th.background);
        p.draw_text(
            &t("med.preview"),
            Rect::new(box_r.x + box_r.w + px(12), y, w - box_r.w - px(12), box_r.h),
            &Ty::Caption.style(th.muted, HAlign::Left),
        );
        let text = t("med.preview_text");
        let st = style(12.0, false, th.foreground, HAlign::Left);
        let clip = Rect::new(box_r.x + px(36), y, box_r.w - px(48), box_r.h);
        let text_w = p.measure_text(&text, &st).w;
        let ov = marquee::overflow_px(text_w, clip.w);
        let m = &self.cfg.media;
        let policy = marquee::Policy {
            enabled: m.marquee,
            reduced_motion: !animations_enabled(),
            playing: true,
            scroll_when_paused: false,
        };
        // vykreslenie play ikony (vektorova) vlavo
        p.draw_text(
            "\u{E768}",
            Rect::new(box_r.x, y, px(32), box_r.h),
            &icon_style(th.foreground),
        );
        if policy.animates(ov) {
            let f = marquee::frame(
                self.anim_start.elapsed().as_millis() as u64,
                ov,
                &marquee::Params::new(
                    m.speed().lp_per_s() * s,
                    m.direction() == transom_config::MarqueeDirection::Rtl,
                ),
            );
            p.draw_text_scrolled(&text, clip, &st, f.offset_px);
        } else {
            // bez posuvania (vypnute posuvanie alebo animacie vo Windows) sa dlhy nazov skrati s "…" ako v liste
            let short = transom_render::scene::elide(p, &text, &st, clip.w);
            p.draw_text(&short, clip, &st);
        }
        y += px(36);
        y = self.card_end(s, x0, w, y);
        y = self.card_begin(s, y);

        let m = self.cfg.media.clone();
        let lw = px(crate::CHOICE_LABEL_LP);
        use transom_config::{MarqueeDirection as D, MarqueeSpeed as S};
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("med.enable"),
            vec![
                (t("opt.yes"), Hit::MarqueeOn(true), m.marquee),
                (t("opt.no"), Hit::MarqueeOn(false), !m.marquee),
            ],
        );
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("med.direction"),
            vec![
                (
                    t("med.dir_ltr"),
                    Hit::MarqueeDir("ltr"),
                    m.direction() == D::Ltr,
                ),
                (
                    t("med.dir_rtl"),
                    Hit::MarqueeDir("rtl"),
                    m.direction() == D::Rtl,
                ),
            ],
        );
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("med.speed"),
            vec![
                (
                    t("med.slow"),
                    Hit::MarqueeSpeed("slow"),
                    m.speed() == S::Slow,
                ),
                (
                    t("med.normal"),
                    Hit::MarqueeSpeed("normal"),
                    m.speed() == S::Normal,
                ),
                (
                    t("med.fast"),
                    Hit::MarqueeSpeed("fast"),
                    m.speed() == S::Fast,
                ),
            ],
        );
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("med.paused"),
            vec![
                (
                    t("med.paused_still"),
                    Hit::MarqueePaused(false),
                    !m.marquee_paused,
                ),
                (
                    t("med.paused_scroll"),
                    Hit::MarqueePaused(true),
                    m.marquee_paused,
                ),
            ],
        );
        y += px(4);
        let anim = animations_enabled();
        y = self.paragraph_b(
            p,
            x0,
            y,
            w,
            &t(if anim { "med.anim_on" } else { "med.anim_off" }),
            12.0,
            !anim,
            if anim { th.muted } else { th.foreground },
        ) + px(design::PARAGRAPH_GAP_LP);
        y = self.notes(p, x0, y, w, &["med.note1", "med.note2", "med.note3"]);
        self.card_end(s, x0, w, y);
    }

    /// Ma bezat casovac animacie nahladu? (len na stranke Media, ak je posuv zapnuty)
    pub(crate) fn preview_animates(&self) -> bool {
        self.page == Page::Media && self.cfg.media.marquee
    }
}
