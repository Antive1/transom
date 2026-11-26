//! Ikona transom v oblasti oznameni (`Shell_NotifyIconW`, dokumentovane API; verzia 4 = klavesnicova aktivacia).
//!
//! Ikona pouziva znacku transom v dvoch variantoch podla **temy panela uloh** (`SystemUsesLightTheme`, nie podla temy
//! aplikacie): svetle "w" na tmavom paneli, tmave na svetlom; lista ostava akcentova. Windows 11 moze novu ikonu
//! zaradit do skrytych ikon (to uzivatel meni v nastaveniach Windows; transom do systemu ani registra nezasahuje).
//! Ikona sa pri ukonceni vzdy odstrani (inak by zostala "duch" az do najblizsieho prejdenia mysou).
use crate::icons;
use std::cell::RefCell;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO,
    NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW,
    NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON, WM_APP};

/// Sprava zo systemu pre ikonu (callback; `lParam` nizke slovo = udalost, `wParam` = bod ukotvenia).
pub(crate) const WM_TRANSOM_TRAY: u32 = WM_APP + 0x35;
const TRAY_ID: u32 = 1;
/// `NIN_SELECT` (klik lavym alebo Enter/medzernik cez `NIN_KEYSELECT`) a `WM_CONTEXTMENU`.
pub(crate) const NIN_SELECT: u32 = 0x400;
/// Klik na bublinu (oznamenie) ikony.
pub(crate) const NIN_BALLOONUSERCLICK: u32 = 0x405;
pub(crate) const NIN_KEYSELECT: u32 = 0x401;
pub(crate) const WM_CONTEXTMENU: u32 = 0x007B;

pub use crate::tray_model::TraySpec;
use crate::tray_model::{drive, Ev, Model, Shell};

thread_local! {
    /// Stavovy automat ikony (rozhoduje o prikazoch shellu; testovany bez WinAPI v `tray_model`).
    static MODEL: RefCell<Model> = const { RefCell::new(Model::new()) };
    /// `HICON` aktualnej ikony (patri nam; uvolni sa pri zmene, odstraneni alebo novom pridani).
    static ICON: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

/// Velkost ikony oblasti oznameni pre DPI okna (16 px pri 100 %).
fn icon_px(hwnd: HWND) -> u32 {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    // SAFETY: cisto citanie DPI platneho okna.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    icons::system_icon_px(dpi, true)
}

fn make_icon(hwnd: HWND, light_taskbar: bool) -> Option<HICON> {
    let ico = if light_taskbar {
        icons::TRAY_LIGHT_ICO
    } else {
        icons::TRAY_DARK_ICO
    };
    icons::hicon(ico, icon_px(hwnd))
}

fn data(hwnd: HWND, tip: &str, icon: HICON) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRANSOM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    let tip: Vec<u16> = tip.encode_utf16().take(127).collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    nid
}

/// Skutocny shell: `Shell_NotifyIconW` s ikonou `hwnd` + `TRAY_ID`.
struct Win32 {
    hwnd: HWND,
}

impl Win32 {
    /// Nahradi ulozeny `HICON` novym (stary sa zrusi).
    fn swap_icon(new: HICON) {
        let old = ICON.with(|i| i.replace(new.0 as isize));
        if old != 0 {
            // SAFETY: stara ikona patrila nam; shell ju uz nepouziva (nahradena alebo ikona zanikla).
            unsafe {
                let _ = DestroyIcon(HICON(old as *mut _));
            }
        }
    }
}

impl Shell for Win32 {
    fn add(&mut self, spec: &TraySpec) -> bool {
        let Some(icon) = make_icon(self.hwnd, spec.light_taskbar) else {
            return false;
        };
        let nid = data(self.hwnd, &spec.tip, icon);
        // SAFETY: `nid` je platna struktura s platnym oknom a ikonou.
        let ok = unsafe {
            let ok = Shell_NotifyIconW(NIM_ADD, &nid).as_bool();
            if ok {
                let mut v = nid;
                v.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &v);
            }
            ok
        };
        if ok {
            Self::swap_icon(icon);
        } else {
            // SAFETY: ikona sa nepouzila (shell ju odmietol); pripadna stara ostava ulozena.
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
        ok
    }

    fn modify(&mut self, spec: &TraySpec) -> bool {
        let Some(icon) = make_icon(self.hwnd, spec.light_taskbar) else {
            return false;
        };
        let nid = data(self.hwnd, &spec.tip, icon);
        // SAFETY: `nid` je platna struktura s platnym oknom a ikonou.
        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() };
        if ok {
            Self::swap_icon(icon);
        } else {
            // SAFETY: ikona sa nepouzila.
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
        ok
    }

    fn delete(&mut self) {
        let icon = ICON.with(|i| i.get());
        let nid = data(self.hwnd, "", HICON(icon as *mut _));
        // SAFETY: odstranenie vlastnej ikony; potom mozeme zrusit jej `HICON`.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
        Self::swap_icon(HICON(std::ptr::null_mut()));
    }
}

fn run(hwnd: HWND, ev: Ev) -> bool {
    MODEL.with(|m| drive(&mut m.borrow_mut(), &mut Win32 { hwnd }, ev))
}

/// Prida, zmeni alebo odstrani ikonu (`None`). Opakovane volanie s tou istou specifikaciou nic nerobi. Vrati, ci je
/// po volani ikona v oblasti oznameni (pri `None` vzdy `false`). Ak ju shell odmietne, ostane ziadana a zopakuje sa
/// (`retry`, dalsie `set` alebo `taskbar_created`).
pub fn set(hwnd: HWND, spec: Option<TraySpec>) -> bool {
    run(hwnd, Ev::Set(spec))
}

/// Odstrani ikonu (pri ukonceni). Idempotentne; nevola sa po zniceni vlastnika, ak to nie je nutne.
pub fn remove(hwnd: HWND) {
    run(hwnd, Ev::Set(None));
}

/// Explorer sa restartoval (`TaskbarCreated`): jeho ikony zanikli, ziadana ikona sa prida **raz**.
/// Volat raz za udalost z okna, ktore ikonu vlastni (nie z okna kazdej listy).
pub fn taskbar_created(hwnd: HWND) -> bool {
    run(hwnd, Ev::TaskbarCreated)
}

/// Opakuje neuspesne pridanie (casovac); vrati, ci je ikona pridana.
pub fn retry(hwnd: HWND) -> bool {
    run(hwnd, Ev::Retry)
}

/// Ma sa spustit opakovanie (ikona je ziadana, nie je pridana a este su pokusy).
pub fn wants_retry() -> bool {
    MODEL.with(|m| m.borrow().wants_retry())
}

/// Ukaze oznamenie (bublinu, vo Windows 11 ako oznamenie v centre oznameni) pri ikone; klik na neho pride ako
/// `NIN_BALLOONUSERCLICK`. Len ak je ikona pridana; v rezime Nerusit ho Windows neukaze. Vrati, ci ho shell prijal.
pub fn balloon(hwnd: HWND, title: &str, text: &str) -> bool {
    if !is_added() {
        return false;
    }
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_INFO,
        dwInfoFlags: NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
        ..Default::default()
    };
    let title: Vec<u16> = title.encode_utf16().take(63).collect();
    nid.szInfoTitle[..title.len()].copy_from_slice(&title);
    let text: Vec<u16> = text.encode_utf16().take(255).collect();
    nid.szInfo[..text.len()].copy_from_slice(&text);
    // SAFETY: platna struktura pre existujucu ikonu tohto okna.
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() }
}

/// Je ikona aktualne pridana (testy).
pub fn is_added() -> bool {
    MODEL.with(|m| m.borrow().is_shown())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_is_truncated_to_the_shell_limit() {
        let long = "x".repeat(500);
        let nid = data(
            HWND(std::ptr::null_mut()),
            &long,
            HICON(std::ptr::null_mut()),
        );
        assert_eq!(nid.szTip[126], b'x' as u16);
        assert_eq!(nid.szTip[127], 0, "vzdy ukonceny nulou");
        assert_eq!(nid.uCallbackMessage, WM_TRANSOM_TRAY);
        assert_eq!(nid.uID, TRAY_ID);
    }

    #[test]
    fn spec_equality_drives_idempotence() {
        let a = TraySpec {
            tip: "transom".into(),
            light_taskbar: false,
        };
        assert_eq!(a, a.clone());
        assert_ne!(
            a,
            TraySpec {
                light_taskbar: true,
                ..a.clone()
            }
        );
    }
}
