//! Zistenie, ci bezi proces s danym nazvom (pre napovedu "Spotify bezi, ale nema GSMTC session").
//! Jednorazovy snimok Toolhelp (~1 ms); volat len pri existujucich udalostiach, nie v slucke.
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// `true`, ak bezi aspon jeden proces s nazvom suboru `exe` (bez ohladu na velkost pismen, napr. `Spotify.exe`).
pub fn is_running(exe: &str) -> bool {
    let want = exe.to_lowercase();
    // SAFETY: snimka procesov sa zatvori (CloseHandle); PROCESSENTRY32W s nastavenym dwSize.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(e.szExeFile.len());
            if String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase() == want {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_found_and_bogus_is_not() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().to_uppercase();
        assert!(is_running(&name));
        assert!(!is_running("transom-urcite-neexistuje-0x1.exe"));
    }
}
