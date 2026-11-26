//! Systemove casti aktualizacie: SHA-256 (Windows CNG), podpis Authenticode (WinVerifyTrust + meno podpisovatela),
//! zistenie instalacie cez instalator a spustenie instalatora. Ziadne siete tu nie su (stiahnutie je `http::get`).
use std::path::{Path, PathBuf};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Security::Cryptography::{
    BCryptHash, CertGetNameStringW, BCRYPT_SHA256_ALG_HANDLE, CERT_NAME_SIMPLE_DISPLAY_TYPE,
};
use windows::Win32::Security::WinTrust::{
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// SHA-256 bajtov ako 64 malych hex znakov (`None` pri chybe CNG).
pub fn sha256_hex(data: &[u8]) -> Option<String> {
    let mut out = [0u8; 32];
    // SAFETY: pseudo-handle algoritmu SHA-256 (Windows 10+), vstup aj vystup su platne rezy.
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    if st.is_err() {
        return None;
    }
    Some(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// Overi podpis Authenticode suboru (bez kontroly odvolania, bez UI). `Some(meno podpisovatela)` len pri platnom
/// podpise s doveryhodnym retazcom; `None` = nepodpisany, neplatny alebo chyba.
pub fn signer(path: &Path) -> Option<String> {
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` a `file` ziju pocas oboch volani; stav sa zatvori druhym volanim (WTD_STATEACTION_CLOSE).
    unsafe {
        let status = WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            &mut data as *mut _ as *mut _,
        );
        let mut name = None;
        if status == 0 {
            let prov = WTHelperProvDataFromStateData(data.hWVTStateData);
            if !prov.is_null() {
                let sgnr = WTHelperGetProvSignerFromChain(prov, 0, false, 0);
                if !sgnr.is_null() && (*sgnr).csCertChain > 0 && !(*sgnr).pasCertChain.is_null() {
                    let cert = (*(*sgnr).pasCertChain).pCert;
                    if !cert.is_null() {
                        let mut buf = [0u16; 512];
                        let n = CertGetNameStringW(
                            cert,
                            CERT_NAME_SIMPLE_DISPLAY_TYPE,
                            0,
                            None,
                            Some(&mut buf),
                        ) as usize;
                        if n > 1 {
                            name = Some(String::from_utf16_lossy(&buf[..n - 1]));
                        }
                    }
                }
            }
        }
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            &mut data as *mut _ as *mut _,
        );
        name
    }
}

/// Subor zapisany na overenie a spustenie: kym zije, drzi otvoreny handle len na citanie so zdielanim **len na citanie**,
/// takze ziadny iny proces ho nemoze prepisat, premenovat ani zmazat (ani jeho priecinok) medzi overenim a spustenim.
/// Spustenie procesu zo suboru (citanie + vykonanie) zdielanie povoluje.
pub struct Pinned {
    _file: std::fs::File,
    path: PathBuf,
}

impl Pinned {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Zapise `bytes` do **noveho** suboru `path` (existujuci subor alebo odkaz na tom mieste = chyba, nic sa neprepise ani
/// nenasleduje), znova ho otvori so zdielanim len na citanie a overi SHA-256 obsahu precitaneho **z disku** cez ten handle
/// (`sha256` = 64 malych hex znakov). Pri nezhode alebo chybe sa subor zmaze.
pub fn write_pinned(path: &Path, bytes: &[u8], sha256: &str) -> std::io::Result<Pinned> {
    use std::io::{Read, Write};
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    // cudzi subor, ktory uz na mieste bol, sa nikdy nemaze
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(path)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })()
    .and_then(|()| {
        drop(f);
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)?;
        let mut disk = Vec::with_capacity(bytes.len());
        file.read_to_end(&mut disk)?;
        if sha256_hex(&disk).as_deref() != Some(sha256) {
            return Err(std::io::Error::other(
                "kontrolny sucet suboru na disku nesedi",
            ));
        }
        Ok(Pinned {
            _file: file,
            path: path.to_path_buf(),
        })
    });
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Novy, prazdny priecinok pod `base` (napr. `%TEMP%\transom-update`) s jedinecnym nazvom; nikdy nepouzije existujuci ani
/// cudzi priecinok (vytvara sa `create_dir`, nie `create_dir_all`). Stare priecinky z predoslych aktualizacii sa najprv
/// skusia zmazat (subor spusteneho instalatora sa zmazat neda a ostane do dalsej aktualizacie).
pub fn fresh_dir(base: &Path) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    std::fs::create_dir_all(base)?;
    if let Ok(rd) = std::fs::read_dir(base) {
        for e in rd.flatten() {
            // `remove_dir_all` nenasleduje odkazy ani junctiony (len ich odstrani)
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                let _ = std::fs::remove_dir_all(e.path());
            } else {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    for _ in 0..16 {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("{}-{stamp}-{n}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("priecinok sa nepodarilo vytvorit"))
}

/// Bezi transom z priecinka nainstalovaneho instalatorom (vedla exe je odinstalator Inno Setup)? Prenosna kopia alebo
/// vyvojarsky build sa neaktualizuje sam (otvori sa stranka vydani).
pub fn installed_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    dir.join("unins000.exe").is_file().then_some(dir)
}

/// Spusti subor cez shell (instalator: UAC sa ukaze, ak ho instalacia potrebuje). Vrati, ci sa spustil.
pub fn launch(path: &Path, args: &str) -> bool {
    // SAFETY: platne retazce HSTRING pocas volania; navratova hodnota > 32 = uspech.
    let r = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(path.as_os_str()),
            &HSTRING::from(args),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    r.0 as isize > 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_known_digest() {
        assert_eq!(
            sha256_hex(b"abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn an_unsigned_file_has_no_signer() {
        let p = std::env::temp_dir().join(format!("transom-unsigned-{}.exe", std::process::id()));
        std::fs::write(&p, b"MZ not really a program").unwrap();
        assert_eq!(signer(&p), None);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_signed_system_file_names_its_signer() {
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap();
        let n = signer(&windir.join("System32").join("kernel32.dll"));
        // kernel32 je podpisany katalogom (nie vlozenym podpisom): WinVerifyTrust na subore ho moze nenajst; ak ano,
        // podpisovatel je Microsoft
        if let Some(n) = n {
            assert!(n.contains("Microsoft"), "{n}");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "transom-selfupdate-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_pinned_file_cannot_be_replaced_renamed_or_deleted_until_released() {
        let d = scratch("pin");
        let p = d.join("transom-setup-9.9.9-x64.exe");
        let data = b"MZ installer bytes".to_vec();
        let sum = sha256_hex(&data).unwrap();
        let pin = write_pinned(&p, &data, &sum).unwrap();
        assert_eq!(pin.path(), p.as_path());
        assert!(std::fs::write(&p, b"evil").is_err(), "prepis");
        assert!(std::fs::remove_file(&p).is_err(), "zmazanie");
        assert!(
            std::fs::rename(&p, d.join("x.exe")).is_err(),
            "premenovanie"
        );
        assert!(
            std::fs::rename(&d, d.with_extension("moved")).is_err(),
            "premenovanie priecinka"
        );
        assert_eq!(std::fs::read(&p).unwrap(), data, "citanie je povolene");
        drop(pin);
        std::fs::remove_file(&p).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_pinned_file_never_overwrites_an_existing_file_and_a_wrong_sum_removes_it() {
        let d = scratch("exist");
        let p = d.join("a.exe");
        std::fs::write(&p, b"cudzi").unwrap();
        let sum = sha256_hex(b"x").unwrap();
        assert!(write_pinned(&p, b"x", &sum).is_err());
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"cudzi",
            "existujuci subor ostal"
        );
        let q = d.join("b.exe");
        assert!(write_pinned(&q, b"x", &"0".repeat(64)).is_err());
        assert!(!q.exists(), "zly sucet: subor zmazany");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Program sa da spustit zo suboru, ktory drzi `Pinned` (zdielanie citania povoluje vytvorenie procesu).
    #[test]
    fn a_process_starts_from_a_pinned_file() {
        let d = scratch("run");
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap();
        let exe = std::fs::read(windir.join("System32").join("cmd.exe")).unwrap();
        let sum = sha256_hex(&exe).unwrap();
        let pin = write_pinned(&d.join("c.exe"), &exe, &sum).unwrap();
        let st = std::process::Command::new(pin.path())
            .args(["/d", "/c", "exit 7"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(st.code(), Some(7));
        drop(pin);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_update_gets_a_new_empty_directory() {
        let base = scratch("fresh");
        let a = fresh_dir(&base).unwrap();
        std::fs::write(a.join("old.exe"), b"x").unwrap();
        let b = fresh_dir(&base).unwrap();
        assert_ne!(a, b);
        assert!(!a.exists(), "stary priecinok zmazany");
        assert_eq!(std::fs::read_dir(&b).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Overenie podpisu cita aj subor, ktory drzi `Pinned` (podpisany program s vlozenym podpisom, ak nejaky na stroji je).
    #[test]
    fn the_signer_of_a_pinned_copy_is_read() {
        let mut cands: Vec<PathBuf> = Vec::new();
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(pf) = std::env::var_os(var).map(PathBuf::from) {
                cands.push(
                    pf.join("Microsoft")
                        .join("Edge")
                        .join("Application")
                        .join("msedge.exe"),
                );
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            cands.push(exe);
        }
        let Some((src, who)) = cands.into_iter().find_map(|c| signer(&c).map(|s| (c, s))) else {
            eprintln!("ziadny podpisany program na skusku");
            return;
        };
        let d = scratch("sig");
        let bytes = std::fs::read(&src).unwrap();
        let pin = write_pinned(&d.join("s.exe"), &bytes, &sha256_hex(&bytes).unwrap()).unwrap();
        assert_eq!(signer(pin.path()).as_deref(), Some(who.as_str()));
        drop(pin);
        let _ = std::fs::remove_dir_all(&d);
    }
}
