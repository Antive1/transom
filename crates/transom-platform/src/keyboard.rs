//! Rozlozenie klavesnice aktivneho okna a Caps Lock (polozka Klavesnica).
//!
//! Len dokumentovane citanie stavu: `GetKeyboardLayout` vlakna, ktore ma fokus v okne na popredi (`GetGUIThreadInfo`,
//! takze to funguje aj pre aplikacie Store, kde ramec okna patri inemu vlaknu nez obsah), a prepinac `VK_CAPITAL`
//! (`GetKeyState`). Ziadny hak klavesnice ani surovy vstup: transom nevidi, co sa pise.
use windows::Win32::Globalization::{
    GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SISO639LANGNAME, LOCALE_SLOCALIZEDDISPLAYNAME,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetKeyboardLayout, VK_CAPITAL};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

/// Stav klavesnice pre listu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyState {
    /// Kratky kod jazyka velkymi pismenami (napr. "SK", "EN"); prazdny = nezistene.
    pub lang: String,
    /// Nazov jazyka v jazyku Windows (napr. "slovencina (Slovensko)") pre tooltip a citacku.
    pub name: String,
    pub caps: bool,
}

/// Kod a nazov jazyka z LANGID (spodnych 16 bitov HKL).
pub fn lang_of(langid: u16) -> (String, String) {
    let mut loc = [0u16; 85];
    // SAFETY: buffery maju zadanu dlzku; funkcie vracaju pocet znakov vratane nuly (0 = chyba).
    unsafe {
        let n = LCIDToLocaleName(u32::from(langid), Some(&mut loc), 0);
        if n <= 1 {
            return (String::new(), String::new());
        }
        let name_ptr = windows::core::PCWSTR(loc.as_ptr());
        let read = |kind| {
            let mut buf = [0u16; 128];
            let n = GetLocaleInfoEx(name_ptr, kind, Some(&mut buf));
            if n <= 1 {
                String::new()
            } else {
                String::from_utf16_lossy(&buf[..n as usize - 1])
            }
        };
        (
            read(LOCALE_SISO639LANGNAME).to_uppercase(),
            read(LOCALE_SLOCALIZEDDISPLAYNAME),
        )
    }
}

/// Aktualny stav: rozlozenie vlakna s fokusom v okne na popredi, Caps Lock.
pub fn state() -> KeyState {
    // SAFETY: citanie stavu okien a klavesnice; struktura ma nastavenu velkost.
    unsafe {
        let fg = GetForegroundWindow();
        let mut tid = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        if tid != 0 {
            let mut gi = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if GetGUIThreadInfo(tid, &mut gi).is_ok() && !gi.hwndFocus.0.is_null() {
                let t2 = GetWindowThreadProcessId(gi.hwndFocus, None);
                if t2 != 0 {
                    tid = t2;
                }
            }
        }
        let hkl = GetKeyboardLayout(tid);
        let langid = (hkl.0 as usize & 0xFFFF) as u16;
        let (lang, name) = lang_of(langid);
        KeyState {
            lang,
            name,
            caps: GetKeyState(i32::from(VK_CAPITAL.0)) & 1 != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_ids_map_to_short_codes() {
        assert_eq!(lang_of(0x041B).0, "SK");
        assert_eq!(lang_of(0x0409).0, "EN");
        assert_eq!(lang_of(0x0407).0, "DE");
        assert!(!lang_of(0x041B).1.is_empty());
    }
}
