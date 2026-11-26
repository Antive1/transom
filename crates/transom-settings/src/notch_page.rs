//! Stranka "Island": ostrov v strede listy (zobrazeny = stred patri jemu) a co smie ukazovat. Kazde spravanie je
//! samostatna volba (`NotchCfg`).
use super::*;

/// Volby vybezku: (id, kluc nazvu, kluc vysvetlenia).
pub(crate) const NOTCH_OPTS: [(&str, &str, &str); 7] = [
    ("island", "notch.island_on", "notch.island_on_note"),
    (
        "island_music",
        "notch.island_music",
        "notch.island_music_note",
    ),
    ("island_ai", "notch.island_ai", "notch.island_ai_note"),
    ("screenshot", "notch.screenshot", "notch.screenshot_note"),
    ("clip_saved", "notch.clip_saved", "notch.clip_saved_note"),
    ("timer", "notch.timer", "notch.timer_note"),
    ("event", "notch.event", "notch.event_note"),
];

pub(crate) fn get(c: &transom_config::Config, id: &str) -> bool {
    let n = &c.notch;
    match id {
        "island" => c.center_locked(),
        "stash" => n.stash,
        "media" => n.media,
        "timer" => n.timer,
        "event" => n.event,
        "clipboard" => n.clipboard,
        "weather" => n.weather,
        "island_music" => n.island_music,
        "island_ai" => n.island_ai,
        "screenshot" => n.screenshot,
        "clip_saved" => n.clip_saved,
        _ => false,
    }
}

pub(crate) fn toggle(c: &mut transom_config::Config, id: &str) {
    let v = !get(c, id);
    if id == "island" {
        // ostrov = polozka v strede listy; zobrazeny zamkne stred (ine polozky sa presunu doprava)
        if let Some(s) = c.layout.items.iter_mut().find(|s| s.id == "shelf") {
            s.visible = v;
        }
        c.enforce_center();
        return;
    }
    let n = &mut c.notch;
    match id {
        "stash" => n.stash = v,
        "media" => n.media = v,
        "timer" => n.timer = v,
        "event" => n.event = v,
        "clipboard" => n.clipboard = v,
        "weather" => n.weather = v,
        "island_music" => n.island_music = v,
        "island_ai" => n.island_ai = v,
        "screenshot" => n.screenshot = v,
        "clip_saved" => n.clip_saved = v,
        _ => {}
    }
}

impl Ui {
    pub(crate) fn paint_notch_page(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "notch.title");
        y = self.notes(p, x0, y, w, &["notch.help"]) + px(12);
        y = self.card_begin(s, y);
        for (id, key, note) in NOTCH_OPTS {
            let on = get(&self.cfg, id);
            let row = Rect::new(x0, y, w, px(28));
            let hit = Hit::NotchOpt(id);
            if self.hover.as_ref() == Some(&hit) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            self.checkbox(
                p,
                Rect::new(row.x + px(4), row.y + px(4), px(20), px(20)),
                on,
            );
            p.draw_text(
                &t(key),
                Rect::new(row.x + px(34), row.y, row.w - px(34), row.h),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            self.tag(&hit, t(key), Some(on));
            self.hits.push((row, hit));
            y = self.notes(p, x0 + px(34), y + px(28), w - px(34), &[note]) + px(8);
        }
        self.card_end(s, x0, w, y - px(4));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_tests::Env;

    #[test]
    fn every_notch_option_toggles_and_is_saved() {
        let e = Env::new("notch-opts");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Notch));
        for (id, _, _) in NOTCH_OPTS {
            let before = get(&e.saved(), id);
            ui.click(Hit::NotchOpt(id));
            assert_eq!(get(&e.saved(), id), !before, "{id}");
        }
        let d = transom_config::NotchCfg::default();
        assert!(d.timer && d.event);
        assert!(d.island_music && d.island_ai && d.screenshot && d.clip_saved);
    }
}
