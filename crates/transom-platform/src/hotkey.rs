//! Globalna klavesova skratka (`RegisterHotKey`) na riadiacom okne hostitela. Stlacenie pride ako `BarEvent::Hotkey(id)`.
//!
//! Skratku moze drzat ina aplikacia (napr. PowerToys Run s Alt+Space): registracia potom zlyha a volajuci to musi
//! viditelne oznamit. `MOD_NOREPEAT`: drzana skratka sa neopakuje (prepinac by inak blikal).
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};

/// Modifikatory a klavesa skratky (virtualny kod).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u32,
}

fn mods(k: &Keys) -> HOT_KEY_MODIFIERS {
    let mut m = MOD_NOREPEAT;
    for (on, f) in [
        (k.ctrl, MOD_CONTROL),
        (k.alt, MOD_ALT),
        (k.shift, MOD_SHIFT),
        (k.win, MOD_WIN),
    ] {
        if on {
            m |= f;
        }
    }
    m
}

/// Zaregistruje skratku `id` (0x0000..=0xBFFF) na riadiacom okne; predchadzajuca registracia s rovnakym `id` sa zrusi.
/// Volat na vlakne hostitela. `Err` = kod chyby (napr. 1409 `ERROR_HOTKEY_ALREADY_REGISTERED`: skratku drzi ina aplikacia).
pub fn register(id: i32, keys: Keys) -> Result<(), u32> {
    let Some(ctl) = crate::appbar::ctl_hwnd() else {
        return Err(0);
    };
    // SAFETY: skratka sa registruje na nase riadiace okno na jeho vlakne; parametre su cisla.
    unsafe {
        let _ = UnregisterHotKey(Some(ctl), id);
        RegisterHotKey(Some(ctl), id, mods(&keys), keys.vk)
            .map_err(|e| (e.code().0 & 0xFFFF) as u32)
    }
}

/// Zrusi skratku `id` (ak nie je registrovana, nic sa nestane).
pub fn unregister(id: i32) {
    if let Some(ctl) = crate::appbar::ctl_hwnd() {
        // SAFETY: odregistrovanie na nasom riadiacom okne; neexistujuca skratka = chyba bez nasledkov.
        unsafe {
            let _ = UnregisterHotKey(Some(ctl), id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_always_include_norepeat_and_map_each_flag() {
        let k = Keys {
            ctrl: true,
            alt: false,
            shift: true,
            win: false,
            vk: 0x4B,
        };
        assert_eq!(mods(&k), MOD_NOREPEAT | MOD_CONTROL | MOD_SHIFT);
        let k = Keys {
            ctrl: false,
            alt: true,
            shift: false,
            win: true,
            vk: 0x20,
        };
        assert_eq!(mods(&k), MOD_NOREPEAT | MOD_ALT | MOD_WIN);
    }
}
