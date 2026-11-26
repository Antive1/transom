//! Skupina dlazdic v paneli Nastroje: nastroje, ktore si pouzivatel zvolil (Nastavenia - Nastroje), po ruke na jednom
//! mieste. Schranka, poznamky a snimky su karty toho isteho panela; ostatne otvoria svoj panel alebo akciu.
use crate::tools_pop::{Section, Tile};
use crate::{log, popup, App};
use transom_core::item::{Action, Glyph};
use transom_i18n::t;
use transom_render::scene::glyph_char;

impl App {
    /// Dlazdice podla nastavenia (len dostupne: pocasie s udajmi, karty so zapnutou funkciou).
    pub(crate) fn tool_tiles(&self) -> Vec<Tile> {
        let mut v = Vec::new();
        for id in &self.config.tool_group {
            let tile = |id: &'static str, g: Glyph, key: &str, color: (u8, u8, u8)| Tile {
                id,
                icon: glyph_char(g),
                label: t(key),
                color,
                value: None,
                section: None,
            };
            let t = match id.as_str() {
                "weather" => {
                    let Some(w) = self.weather.as_ref().and_then(|w| w.snapshot()) else {
                        continue;
                    };
                    Tile {
                        value: Some(crate::weather::temp(w.temp)),
                        ..tile("weather", Glyph::Weather, "tile.weather", (255, 184, 0))
                    }
                }
                "files" => Tile {
                    section: Some(Section::Shelf),
                    ..tile("files", Glyph::Shelf, "tile.files", (10, 132, 255))
                },
                "clipboard" => Tile {
                    section: Some(Section::Clipboard),
                    ..tile(
                        "clipboard",
                        Glyph::Clipboard,
                        "tile.clipboard",
                        (64, 156, 255),
                    )
                },
                "notes" => Tile {
                    section: Some(Section::Notes),
                    ..tile("notes", Glyph::Note, "tile.notes", (255, 214, 10))
                },
                "shots" => Tile {
                    section: Some(Section::Shots),
                    ..tile("shots", Glyph::Screenshot, "tile.shots", (191, 90, 242))
                },
                "timer" => tile("timer", Glyph::Timer, "tile.timer", (255, 149, 0)),
                "brightness" => tile(
                    "brightness",
                    Glyph::Brightness,
                    "tile.brightness",
                    (255, 204, 0),
                ),
                "snip" => tile("snip", Glyph::Snip, "tile.snip", (48, 209, 188)),
                "picker" => tile("picker", Glyph::Picker, "tile.picker", (255, 55, 95)),
                "focus" => tile("focus", Glyph::Focus, "tile.focus", (94, 92, 230)),
                _ => continue,
            };
            v.push(t);
        }
        v
    }

    /// Dlazdica, ktora nie je kartou panela: otvori svoj panel na mieste Nastrojov alebo spusti akciu.
    pub(crate) fn tool_tile(&mut self, id: &str) {
        let action = match id {
            // sekcie panela Nastroje (otvori sa znova v plnej vyske)
            "files" => Action::ToolsShelf,
            "clipboard" => Action::ToolsClipboard,
            "notes" => Action::ToolsNotes,
            "shots" => Action::ToolsShots,
            "weather" => Action::WeatherOpen,
            "timer" => Action::TimerPanel,
            "brightness" => Action::BrightnessPanel,
            "snip" => Action::SnipToStash,
            "picker" => Action::ColorPick,
            "focus" => Action::FocusPanel,
            _ => return,
        };
        log(&format!("nastroje: dlazdica {id}"));
        // panel Nastroje sa najprv zavrie (ten isty druh panela by sa inak prepinacom len zavrel; vyrez a kapatko ho
        // nechcu mat otvoreny)
        popup::request(transom_platform::popup::Request::ClosePopover);
        let at = self.popover_anchor.clone();
        let out = self.act(action, at);
        // panely otvorene z Nastrojov maju sipku spat
        match action {
            Action::TimerPanel | Action::BrightnessPanel => self.kit.back = true,
            Action::WeatherOpen => self.weather_back = true,
            _ => {}
        }
        self.sound.cue(crate::snd::Origin::Popover, out);
    }

    /// Spat na dlazdice Nastrojov (zo sekcie alebo z panela otvoreneho z nich).
    pub(crate) fn tools_home(&mut self) {
        popup::request(transom_platform::popup::Request::ClosePopover);
        let at = self.popover_anchor.clone();
        let _ = self.act(Action::ToolsPanel, at);
    }
}
