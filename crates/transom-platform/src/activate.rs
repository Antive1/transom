//! Aktivacia aplikacie zvolenej media session (klik na obal). Len zdokumentovane API:
//! - `SHGetPropertyStoreForWindow` + `PKEY_AppUserModel_ID`: AUMID, ktore si okno samo nastavilo (presna zhoda);
//! - inak proces okna: ak GSMTC vratil nazov suboru (`Spotify.exe` pri nebalenych aplikaciach), porovna sa s nim;
//!   ak vratil kratke meno (`Brave`), porovna sa s nazvom suboru bez pripony - to je **odhad**, oznaceny v vysledku;
//! - `SetForegroundWindow` (povolene, lebo proces listy prijal posledny vstup - klik pouzivatela) a kontrola, ci sa
//!   popredie naozaj zmenilo;
//! - ak aplikacia nebezi: `ShellExecute("shell:AppsFolder\<AUMID>")` (zname priecinky Shellu) - len pre skutocne AUMID.
//!
//! GSMTC neposkytuje kartu prehliadaca ani URL videa: pri YouTube sa aktivuje okno prehliadaca (prednostne to,
//! ktoreho titulok obsahuje nazov media, t. j. aktivna karta s videom); konkretnu kartu nevyberie.
use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
    GWL_EXSTYLE, GW_OWNER, SW_RESTORE, SW_SHOWNORMAL, WS_EX_TOOLWINDOW,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activation {
    /// Windows prijal `SetForegroundWindow` pre okno `hwnd`. Prepnutie popredia je asynchronne: volajuci ho ma
    /// overit o chvilu neskor (`input::foreground`). `how` = podla coho sa okno naslo.
    Requested {
        hwnd: isize,
        exe: String,
        how: &'static str,
    },
    /// Aplikacia nebezala, spustena cez `shell:AppsFolder`.
    Launched,
    /// Okno sa naslo, ale Windows popredie nezmenil.
    Refused {
        exe: String,
    },
    NotFound,
}

/// Rozhodnutie, ci okno patri session (cista logika, testovatelna).
/// Vrati sposob zhody: presne AUMID okna, nazov procesu z GSMTC, alebo odhad podla mena.
pub fn match_window(session_app: &str, window_aumid: &str, exe: &str) -> Option<&'static str> {
    let app = session_app.trim().to_lowercase();
    if app.is_empty() {
        return None;
    }
    if !window_aumid.is_empty() && window_aumid.to_lowercase() == app {
        return Some("AUMID okna");
    }
    let exe = exe.to_lowercase();
    if app.ends_with(".exe") {
        return (exe == app).then_some("názov procesu z GSMTC");
    }
    let stem = exe.strip_suffix(".exe").unwrap_or(&exe);
    // kratke meno (napr. "Brave"): rovnaky nazov suboru, alebo AUMID okna s rovnakou predponou ("Brave.XYZ")
    if stem == app
        || (!window_aumid.is_empty() && window_aumid.to_lowercase().starts_with(&format!("{app}.")))
    {
        return Some("odhad podľa názvu aplikácie");
    }
    None
}

struct Win {
    hwnd: isize,
    title: String,
    exe: String,
    aumid: String,
}

fn window_aumid(h: HWND) -> String {
    // SAFETY: COM objekt z SHGetPropertyStoreForWindow zije pocas bloku; hodnota sa skopiruje do String.
    unsafe {
        let Ok(store) = SHGetPropertyStoreForWindow::<IPropertyStore>(h) else {
            return String::new();
        };
        store
            .GetValue(&PKEY_AppUserModel_ID)
            .map(|v| v.to_string())
            .unwrap_or_default()
    }
}

fn exe_of(h: HWND) -> String {
    // SAFETY: handle procesu z OpenProcess sa zatvori (CloseHandle); buffer mena ma zadanu dlzku a `n` je skutocna dlzka.
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut b = [0u16; 260];
        let mut n = b.len() as u32;
        let ok =
            QueryFullProcessImageNameW(p, PROCESS_NAME_FORMAT(0), PWSTR(b.as_mut_ptr()), &mut n)
                .is_ok();
        let _ = CloseHandle(p);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&b[..n as usize]);
        full.rsplit('\\').next().unwrap_or("").to_string()
    }
}

unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` je ukazovatel na `Vec<isize>` z `top_windows`, ktory zije pocas celeho EnumWindows (synchronne volanie, jedno vlakno).
    unsafe {
        let v = &mut *(lp.0 as *mut Vec<isize>);
        let owner = GetWindow(h, GW_OWNER).unwrap_or_default();
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if IsWindowVisible(h).as_bool() && owner.0.is_null() && ex & WS_EX_TOOLWINDOW.0 == 0 {
            v.push(h.0 as isize);
        }
    }
    BOOL(1)
}

fn top_windows() -> Vec<Win> {
    let mut hs: Vec<isize> = Vec::new();
    // SAFETY: EnumWindows vola `collect` synchronne; `hs` zije az do navratu.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hs as *mut _ as isize));
    }
    hs.into_iter()
        .filter_map(|raw| {
            let h = HWND(raw as *mut _);
            let mut b = [0u16; 512];
            // SAFETY: buffer s danou dlzkou, okno moze medzitym zaniknut (vtedy vrati 0).
            let n = unsafe { GetWindowTextW(h, &mut b) };
            if n <= 0 {
                return None;
            }
            Some(Win {
                hwnd: raw,
                title: String::from_utf16_lossy(&b[..n as usize]),
                exe: exe_of(h),
                aumid: window_aumid(h),
            })
        })
        .collect()
}

/// Aktivuje (alebo spusti) aplikaciu session `session_app` (GSMTC `SourceAppUserModelId`); `title_hint` = nazov
/// media (preferuje sa okno, ktoreho titulok ho obsahuje). Volat mimo vypozicky stavu hostitela (posiela spravy).
pub fn activate_app(session_app: &str, title_hint: &str) -> Activation {
    let wins = top_windows();
    let mut cands: Vec<(&Win, &'static str)> = wins
        .iter()
        .filter_map(|w| match_window(session_app, &w.aumid, &w.exe).map(|how| (w, how)))
        .collect();
    // presna zhoda ma prednost pred odhadom
    if cands
        .iter()
        .any(|(_, h)| *h != "odhad podľa názvu aplikácie")
    {
        cands.retain(|(_, h)| *h != "odhad podľa názvu aplikácie");
    }
    let hint = title_hint.trim().to_lowercase();
    let pick = cands
        .iter()
        .find(|(w, _)| !hint.is_empty() && w.title.to_lowercase().contains(&hint))
        .or_else(|| cands.first());
    if let Some((w, how)) = pick {
        let h = HWND(w.hwnd as *mut _);
        // SAFETY: volania s handle okna (moze medzitym zaniknut: funkcie vratia chybu, nic sa nedereferencuje).
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            if SetForegroundWindow(h).as_bool() || GetForegroundWindow() == h {
                return Activation::Requested {
                    hwnd: w.hwnd,
                    exe: w.exe.clone(),
                    how,
                };
            }
        }
        return Activation::Refused { exe: w.exe.clone() };
    }
    // nebezi: spustit cez zname priecinky Shellu, len ak ide o AUMID (nie o nazov .exe); AUMID si moze nastavit
    // lubovolny proces, preto sa spusta iba platny tvar (bez oddelovacov ciest a dalsich znakov Shellu)
    if !session_app.to_lowercase().ends_with(".exe")
        && !session_app.trim().is_empty()
        && is_valid_aumid(session_app)
    {
        let target: Vec<u16> = format!("shell:AppsFolder\\{session_app}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: retazce ukoncene nulou ziju pocas volania ShellExecuteW.
        let r = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(target.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if r.0 as isize > 32 {
            return Activation::Launched;
        }
    }
    Activation::NotFound
}

/// Platny AUMID: pismena, cislice a `. _ ! -` (napr. `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`), najviac 128 znakov,
/// ziadne `\`, `/`, `:`, medzery ani `..`. Cokolvek ine sa cez Shell nespusta.
pub(crate) fn is_valid_aumid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '!' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_well_formed_aumids_are_launched_through_the_shell() {
        for ok in [
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic",
            "Chrome",
        ] {
            assert!(is_valid_aumid(ok), "{ok}");
        }
        for bad in [
            "",
            "..\\evil",
            "a/b",
            "a\\b",
            "C:\\Windows\\System32\\calc",
            "app name",
            "a;b",
            "a&b",
            "%TEMP%",
            "a..b",
            &"x".repeat(129),
        ] {
            assert!(!is_valid_aumid(bad), "{bad}");
        }
    }

    #[test]
    fn matching_rules() {
        // nebalena aplikacia: GSMTC vracia nazov suboru
        assert_eq!(
            match_window("Spotify.exe", "", "Spotify.exe"),
            Some("názov procesu z GSMTC")
        );
        assert_eq!(
            match_window("Spotify.exe", "", "spotify.EXE"),
            Some("názov procesu z GSMTC")
        );
        assert_eq!(match_window("Spotify.exe", "", "brave.exe"), None);
        // presne AUMID okna
        assert_eq!(
            match_window("Brave.ABC", "Brave.ABC", "brave.exe"),
            Some("AUMID okna")
        );
        // kratke meno: len odhad
        assert_eq!(
            match_window("Brave", "", "brave.exe"),
            Some("odhad podľa názvu aplikácie")
        );
        assert_eq!(
            match_window("Brave", "Brave.XYZ123", "brave.exe"),
            Some("odhad podľa názvu aplikácie")
        );
        assert_eq!(match_window("Brave", "", "bravebrowser-helper.exe"), None);
        assert_eq!(match_window("", "", "x.exe"), None);
        // balena aplikacia (Store): AUMID
        assert_eq!(
            match_window(
                "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic",
                "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic",
                "ApplicationFrameHost.exe"
            ),
            Some("AUMID okna")
        );
    }
}
