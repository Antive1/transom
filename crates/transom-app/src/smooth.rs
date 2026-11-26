//! Plynule posuvanie panelov: koliesko nastavi ciel (`Glide`), snimky prichadzaju s obnovou obrazovky (60 - 240 Hz)
//! a posuvaju otvoreny panel k nemu. Casovac bezi len kym sa nieco posuva.
use crate::{extras, popup, App, POP_AI, POP_AUDIO, POP_PERIPH, POP_TOOLS, T_GLIDE};

impl App {
    /// Koliesko zmenilo ciel posuvania otvoreneho panela.
    pub(crate) fn glide_start(&mut self) {
        if !transom_platform::system::animations_enabled() {
            // obmedzeny pohyb: hned do ciela
            while self.glide_step_open() {}
            popup::invalidate_popover();
            return;
        }
        transom_platform::appbar::pace_timer(T_GLIDE, true);
        self.glide_tick();
    }

    /// Snimka posuvania; po dobehu sa casovac zastavi.
    pub(crate) fn glide_tick(&mut self) {
        let moving = self.glide_step_open();
        popup::invalidate_popover();
        if !moving {
            transom_platform::appbar::pace_timer(T_GLIDE, false);
        }
    }

    fn glide_step_open(&mut self) -> bool {
        match popup::popover_open() {
            Some(POP_TOOLS) => self.tools_panel.glide_step(),
            Some(POP_AUDIO) => self.audio_panel.glide_step(),
            Some(POP_PERIPH) => self.periph_panel.glide_step(),
            Some(POP_AI) => self.ai.glide_step(),
            Some(k) if extras::is_kit(k) => self.kit.glide_step(),
            _ => false,
        }
    }
}
