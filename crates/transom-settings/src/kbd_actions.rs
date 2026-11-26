//! Klavesnicove akcie nad stavom nastaveni (bez okna, testovatelne): presun polozky rozlozenia a posuvnik nepriehladnosti.
use crate::appearance_page::{step_opacity, SliderKey};
use crate::dnd::{self, Dir, MoveSession};
use crate::{Hit, Ui};
use transom_i18n::t;

impl Ui {
    /// Ulozi cakajucu hodnotu (posuvnik s oneskorenym ulozenim) skor, nez zacne dalsia relacia upravy.
    pub(crate) fn flush_pending(&mut self) {
        if std::mem::take(&mut self.commit_pending) {
            self.save();
        }
        // ukazka hlasitosti zvukov az po ulozeni (raz, nie pri kazdej klavese)
        if std::mem::take(&mut self.sound_preview_pending) {
            self.play_preview();
        }
    }

    /// Enter / medzernik na polozke rozlozenia: zacne klavesnicovy presun (zmeny sa ukladaju az pri potvrdeni).
    pub(crate) fn begin_move(&mut self, id: &str) -> bool {
        if self.read_only {
            self.status = t("status.readonly");
            return false;
        }
        if !transom_config::KNOWN_ITEMS.contains(&id) || self.moving.is_some() {
            return false;
        }
        self.flush_pending();
        self.moving = Some(MoveSession::begin(&self.cfg.layout, id));
        self.status = t("layout.move_help");
        true
    }

    /// Sipka / Home / End pocas presunu: presunie polozku (nahlad hned, bez ulozenia). Vrati, ci sa nieco zmenilo.
    pub(crate) fn nudge_move(&mut self, d: Dir) -> bool {
        let Some(id) = self.moving.as_ref().map(|m| m.id.clone()) else {
            return false;
        };
        let Some(mut tg) = dnd::move_target(&self.cfg.layout, &id, d) else {
            return false;
        };
        if !self.center_ok(&id, tg.0) {
            // stred patri odlozenym suborom: sipka do strany ho preskoci
            let side = match d {
                Dir::Right => transom_config::Group::Right,
                Dir::Left => transom_config::Group::Left,
                _ => return false,
            };
            if Some(side) == dnd::position_of(&self.cfg.layout, &id).map(|p| p.0) || id == "shelf" {
                return false;
            }
            tg = (
                side,
                tg.1.min(dnd::column_items(&self.cfg.layout, side).len()),
            );
        }
        dnd::apply_drop(&mut self.cfg.layout, &id, tg);
        let moved = true;
        if moved {
            // po presune ma byt polozka cela viditelna (pouzije sa geometria z nasledujuceho kreslenia)
            self.reveal = Some(Hit::Chip(id));
        }
        moved
    }

    /// Koniec presunu: `commit` ulozi (ak sa rozlozenie zmenilo), inak obnovi povodne rozlozenie a nic neulozi.
    /// Vrati, ci sa ulozilo.
    pub(crate) fn finish_move(&mut self, commit: bool) -> bool {
        let Some(ses) = self.moving.take() else {
            return false;
        };
        if commit && ses.changed(&self.cfg.layout) {
            self.save();
            return true;
        }
        if !commit {
            ses.cancel(&mut self.cfg.layout);
        }
        self.status.clear();
        false
    }

    /// Klavesa na posuvniku nepriehladnosti: zmena sa prejavi hned, ulozi sa po chvili ticha (volajuci spusti casovac).
    pub(crate) fn slider_key(&mut self, key: SliderKey, fine: bool) -> bool {
        if self.read_only {
            self.status = t("status.readonly");
            return false;
        }
        if self.kfocus == Some(Hit::SoundTrack) {
            let cur = self.cfg.ui_sounds.volume;
            let new = crate::sound_page::step_volume(cur, key, fine);
            if new == cur {
                return false;
            }
            self.cfg.ui_sounds.volume = new;
            self.commit_pending = true;
            self.sound_preview_pending = true;
            return true;
        }
        let cur = self.cfg.appearance.opacity_percent();
        let new = step_opacity(cur, key, fine);
        if new == cur {
            return false;
        }
        self.cfg.appearance.bg_opacity = new;
        self.commit_pending = true;
        true
    }
}
