//! Spustenie s Windows: hodnota `transom` v `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (rovnaka, akú zapisuje
//! instalator, takze ju odinstalator zmaze). Prikaz je cesta k `transom-app.exe` v uvodzovkach. Len pre aktualneho
//! pouzivatela, bez prav spravcu; portable kopia ukazuje na seba.
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Nazov hodnoty (rovnaky ako v `installer/transom.iss`).
pub const VALUE: &str = "transom";

/// Prikaz v hodnote Run: cesta v uvodzovkach (cesta moze obsahovat medzery).
pub fn command_for(exe: &std::path::Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Cesta k `transom-app.exe` vedla beziaceho programu (lista aj nastavenia lezia v tom istom priecinku).
pub fn app_exe() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("transom-app.exe"))
}

fn read(name: &str) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: buffer a jeho velkost v bajtoch su platne pocas volania; retazce su HSTRING ukoncene nulou.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn write(name: &str, value: Option<&str>) -> bool {
    let mut key = HKEY::default();
    // SAFETY: vytvorenie / otvorenie kluca pouzivatela; handle sa zatvori nizsie.
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            Some(0),
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if r.is_err() {
        return false;
    }
    let ok = match value {
        Some(v) => {
            let wide: Vec<u16> = v.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: data su bajty retazca UTF-16 vratane koncovej nuly; kluc je otvoreny na zapis.
            let bytes =
                unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
            // SAFETY: platny kluc a nazov hodnoty.
            unsafe { RegSetValueExW(key, &HSTRING::from(name), Some(0), REG_SZ, Some(bytes)) }
                .is_ok()
        }
        // SAFETY: platny kluc a nazov hodnoty; chybajuca hodnota nie je chyba.
        None => {
            let r = unsafe { RegDeleteValueW(key, &HSTRING::from(name)) };
            r.is_ok() || r == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
        }
    };
    // SAFETY: kluc otvoreny vyssie.
    let _ = unsafe { RegCloseKey(key) };
    ok
}

/// Je spustenie s Windows zapnute (hodnota existuje)?
pub fn enabled() -> bool {
    read(VALUE).is_some_and(|v| !v.trim().is_empty())
}

/// Zapne (prikaz na `transom-app.exe` vedla tohto programu) alebo vypne spustenie s Windows. `false` = zapis zlyhal.
pub fn set(on: bool) -> bool {
    if !on {
        return write(VALUE, None);
    }
    match app_exe() {
        Some(exe) => write(VALUE, Some(&command_for(&exe))),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_quotes_paths_with_spaces() {
        let p =
            std::path::Path::new(r"C:\Users\A B\AppData\Local\Programs\transom\transom-app.exe");
        assert_eq!(
            command_for(p),
            r#""C:\Users\A B\AppData\Local\Programs\transom\transom-app.exe""#
        );
    }

    /// Zapis a zmazanie pod inym nazvom hodnoty (skutocne spustenie s Windows sa nemeni).
    #[test]
    fn a_value_round_trips_and_deleting_a_missing_value_is_fine() {
        let name = format!("transom-test-{}", std::process::id());
        assert!(write(&name, Some(r#""C:\x y\transom-app.exe""#)));
        assert_eq!(read(&name).as_deref(), Some(r#""C:\x y\transom-app.exe""#));
        assert!(write(&name, None));
        assert_eq!(read(&name), None);
        assert!(
            write(&name, None),
            "zmazanie chybajucej hodnoty nie je chyba"
        );
    }
}
