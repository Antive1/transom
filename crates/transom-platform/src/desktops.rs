//! Virtualne plochy a aktivne okno (pre polozky listy).
//!
//! Plochy: pocet a aktualna plocha z registra Prieskumnika (`HKCU\...\Explorer\VirtualDesktops`: `VirtualDesktopIDs` =
//! zretazene GUID po 16 bajtov, `CurrentVirtualDesktop` = GUID aktualnej). Ziadne nezdokumentovane COM rozhrania.
//! Prepinanie: skratka `Win+Ctrl+Sipka` (ako pouzivatel na klavesnici). Aktivne okno: nazov a proces popredneho
//! okna; text sa nikdy nelogue.
use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY};

const KEY: windows::core::PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\VirtualDesktops");

fn binary(value: windows::core::PCWSTR) -> Option<Vec<u8>> {
    let mut len = 0u32;
    // SAFETY: najprv zistenie dlzky, potom citanie do bufra tej dlzky.
    unsafe {
        if RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            value,
            RRF_RT_REG_BINARY,
            None,
            None,
            Some(&mut len),
        )
        .is_err()
            || len == 0
            || len > 64 * 1024
        {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            value,
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        )
        .is_err()
        {
            return None;
        }
        buf.truncate(len as usize);
        Some(buf)
    }
}

/// (pocet ploch, poradie aktualnej od 0). `None` = register sa neda precitat (napr. jedina plocha este nebola vytvorena).
pub fn desktops() -> Option<(usize, usize)> {
    let ids = binary(w!("VirtualDesktopIDs"))?;
    let cur = binary(w!("CurrentVirtualDesktop"));
    position(&ids, cur.as_deref())
}

/// Pocet a poradie aktualnej plochy zo zretazenych GUID (16 B) a GUID aktualnej.
pub fn position(ids: &[u8], current: Option<&[u8]>) -> Option<(usize, usize)> {
    let n = ids.len() / 16;
    if n == 0 {
        return None;
    }
    let i = current
        .filter(|c| c.len() == 16)
        .and_then(|c| {
            ids.as_chunks::<16>()
                .0
                .iter()
                .position(|g| g.as_slice() == c)
        })
        .unwrap_or(0);
    Some((n, i))
}

/// Prepne na plochu `target` (od 0) zo `current`: opakovana skratka `Win+Ctrl+Sipka`.
pub fn switch_to(current: usize, target: usize) -> bool {
    use crate::input::{send_shortcut, Shortcut};
    let (s, n) = if target > current {
        (Shortcut::DesktopRight, target - current)
    } else {
        (Shortcut::DesktopLeft, current - target)
    };
    (0..n).all(|_| send_shortcut(s).is_ok())
}

/// Aktivne okno: (nazov aplikacie z procesu, nazov okna). Vlastne okna transom a plocha sa vynechaju (`None`).
pub fn active_window() -> Option<(String, String)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
    let fg = crate::input::foreground();
    let exe = fg
        .exe
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .to_string();
    let own = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .is_some_and(|n| n == fg.exe.to_lowercase());
    if own
        || matches!(
            fg.class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | ""
        )
    {
        return None;
    }
    // SAFETY: citanie textu popredneho okna do bufra.
    let title = unsafe {
        let h = GetForegroundWindow();
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    };
    Some((exe, title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_desktop_is_found_by_its_guid() {
        let ids: Vec<u8> = (0..3u8).flat_map(|i| [i; 16]).collect();
        assert_eq!(position(&ids, Some(&[1u8; 16])), Some((3, 1)));
        assert_eq!(
            position(&ids, Some(&[9u8; 16])),
            Some((3, 0)),
            "neznama = prva"
        );
        assert_eq!(position(&ids, None), Some((3, 0)));
        assert_eq!(position(&[], None), None);
    }
}
