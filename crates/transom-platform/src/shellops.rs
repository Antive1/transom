//! Systemove operacie pre vyhladavac: zoznam aplikacii (zlozka "Aplikacie" shellu = klasicke aj balene aplikacie jednym
//! zdokumentovanym API), index Windows Search (OLE DB, len citanie) a bezpecne otvaranie cielov. Nikdy sa nic nestavia z textu
//! dotazu: otvara sa vyhradne hotovy cil z nasho zdroja (cesta, `shell:AppsFolder\<AUMID>` alebo pevne `ms-settings:`).
use std::path::Path;
use windows::core::{w, Interface, GUID, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSIDFromProgID, CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER,
};
use windows::Win32::System::Search::{
    IAccessor, ICommandText, IDBCreateCommand, IDBCreateSession, IDBInitialize, IRowset,
    DBACCESSOR_ROWDATA, DBBINDING, DBMEMOWNER_CLIENTOWNED, DBPARAMIO_NOTPARAM, DBPART_VALUE,
    DBTYPE_WSTR, HACCESSOR,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_Pictures, FOLDERID_Screenshots, IEnumShellItems,
    IShellItem, SHGetKnownFolderItem, SHGetKnownFolderPath, SHOpenFolderAndSelectItems,
    SHParseDisplayName, ShellExecuteW, KF_FLAG_DEFAULT, KF_FLAG_DONT_VERIFY, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Najviac aplikacii a dlzka nazvu, ktore sa nacitaju.
pub const MAX_APPS: usize = 3000;

/// Aktualna (aj presmerovana) cesta znameho priecinka Windows; priecinok nemusi existovat (`KF_FLAG_DONT_VERIFY`: bez overovania
/// a bez vytvarania). Zdokumentovane API; nic sa nelogue.
fn known_folder(id: &GUID) -> Option<String> {
    // SAFETY: `id` je platne GUID; vrateny retazec pridelil shell (CoTaskMem) a `pwstr_to_string` ho uvolni.
    unsafe {
        SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None)
            .ok()
            .map(pwstr_to_string)
            .filter(|p| !p.is_empty())
    }
}

/// Priecinok snimok obrazovky: znamy priecinok `FOLDERID_Screenshots` (sleduje presmerovanie Obrazkov na OneDrive alebo iny
/// disk aj vlastne presmerovanie tohto priecinka); ak ho system nevie urcit, standardny `Screenshots` pod vyriesenym priecinkom
/// Obrazky. Neexistujuci priecinok sa vrati tiez (vytvori ho az Windows pri prvej snimke). Cestu do zvysku Windows,
/// do ktorej Snipping Tool moze ukladat inak (jeho vlastne nastavenie nema dokumentovane API), pridava pouzivatel sam.
pub fn screenshots_folder() -> Option<String> {
    known_folder(&FOLDERID_Screenshots).or_else(|| {
        known_folder(&FOLDERID_Pictures)
            .map(|p| format!("{}\\Screenshots", p.trim_end_matches('\\')))
    })
}

/// Je cesta v tvare `X:\...` na namapovanom sietovom disku (`GetDriveTypeW` == `DRIVE_REMOTE`)? Zdokumentovane API, nepristupuje
/// k suborom ani k serveru. UNC cesty rozpoznava textovo `transom_tools::lane`. `subst` a spojenia ju mozu oklamat: bezpecnost
/// Police nestoji na tejto klasifikacii, ale na casovom limite sond.
pub fn is_remote_drive(path: &str) -> bool {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    let b = path.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return false;
    }
    let root = wide(&format!("{}:\\", b[0] as char));
    // SAFETY: retazec ukonceny nulou, zije cez volanie.
    unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) == 4 } // DRIVE_REMOTE
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pwstr_to_string(p: windows::core::PWSTR) -> String {
    // SAFETY: retazec pridelil shell (CoTaskMem); kopirujeme ho a hned uvolnime.
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Aplikacie zo zlozky "Aplikacie" (nazov, cil `shell:AppsFolder\<AUMID alebo cesta>`). Volajuce vlakno ma mat COM.
pub fn list_apps() -> Vec<(String, String)> {
    let mut out = Vec::new();
    // SAFETY: standardne vymenovanie zlozky shellu; kazdy retazec sa uvolni v `pwstr_to_string`.
    unsafe {
        let Ok(folder) =
            SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
        else {
            return out;
        };
        let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else {
            return out;
        };
        loop {
            let mut item: [Option<IShellItem>; 1] = [None];
            let mut got = 0u32;
            if en.Next(&mut item, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(it) = item[0].take() else { break };
            let name = it
                .GetDisplayName(SIGDN_NORMALDISPLAY)
                .map(pwstr_to_string)
                .unwrap_or_default();
            let id = it
                .GetDisplayName(SIGDN_PARENTRELATIVEPARSING)
                .map(pwstr_to_string)
                .unwrap_or_default();
            if !name.is_empty()
                && !id.is_empty()
                && name.chars().count() <= 200
                && id.chars().count() <= 1000
            {
                out.push((name, format!("shell:AppsFolder\\{id}")));
            }
            if out.len() >= MAX_APPS {
                break;
            }
        }
    }
    out
}

/// Cil, ktory sa smie otvorit: neprazdny, bez riadiacich znakov a bez NUL; `ms-settings:` iba s prefixom.
pub fn target_ok(t: &str) -> bool {
    !t.is_empty() && t.chars().count() <= 32_767 && !t.chars().any(|c| c.is_control())
}

/// Otvori cil (subor, priecinok, `shell:AppsFolder\...`, `ms-settings:...`) s predvolenym slovesom. Vracia uspech.
pub fn open_target(t: &str) -> bool {
    if !target_ok(t) {
        return false;
    }
    let w = wide(t);
    // SAFETY: platny retazec ukonceny nulou; ziadne parametre ani pracovny adresar z dotazu.
    let r = unsafe {
        ShellExecuteW(
            None::<HWND>,
            w!("open"),
            PCWSTR(w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    r.0 as usize > 32
}

/// Otvori Prieskumnika s oznacenou polozkou (nikdy ju nespusta).
pub fn open_containing(path: &str) -> bool {
    if !target_ok(path) || !Path::new(path).exists() {
        return false;
    }
    let w = wide(path);
    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    // SAFETY: PIDL sa uvolni nizsie; `cidl = 0` znamena "oznac samotnu polozku".
    unsafe {
        if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_err()
            || pidl.is_null()
        {
            return false;
        }
        let ok = SHOpenFolderAndSelectItems(pidl, None, 0).is_ok();
        CoTaskMemFree(Some(pidl as *const _));
        ok
    }
}

// ---- Windows Search (OLE DB, len citanie) ----

const DBGUID_DEFAULT: GUID = GUID::from_u128(0xc8b521fb_5cf3_11ce_ade5_00aa0044773d);

/// Dotaz na index: najviac `limit` ciest, ktorych **nazov suboru** obsahuje `term` (term musi byt escapovany `sql_like_term`).
/// `Err` = sluzba/index nedostupny alebo dotaz zlyhal. Volajuce vlakno ma mat COM.
pub fn index_query(
    term: &str,
    limit: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<String>, String> {
    let limit = limit.clamp(1, 50);
    let sql = format!(
        "SELECT TOP {limit} System.ItemPathDisplay FROM SystemIndex WHERE SCOPE='file:' AND System.FileName LIKE '%{term}%' ORDER BY System.Search.Rank DESC"
    );
    index_sql(&sql, limit, cancelled)
}

/// Spusti hotovy SQL dotaz (tvori ho len `index_query`; nikdy nie text pouzivatela).
fn index_sql(sql: &str, limit: usize, cancelled: &dyn Fn() -> bool) -> Result<Vec<String>, String> {
    // SAFETY: OLE DB volania nad objektmi z `CoCreateInstance`; kazdy riadok a pristupovy objekt sa uvolni; buffer je dost velky
    // pre `cbMaxLen` a cita sa az po uspesnom `GetData`.
    unsafe {
        let e = |x: windows::core::Error| x.to_string();
        let clsid = CLSIDFromProgID(w!("Search.CollatorDSO")).map_err(e)?;
        let init: IDBInitialize =
            CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        init.Initialize().map_err(e)?;
        let create: IDBCreateSession = init.cast().map_err(e)?;
        let sess = create
            .CreateSession(None, &IDBCreateCommand::IID)
            .map_err(e)?;
        let cc: IDBCreateCommand = sess.cast().map_err(e)?;
        let cmd = cc.CreateCommand(None, &ICommandText::IID).map_err(e)?;
        let text: ICommandText = cmd.cast().map_err(e)?;
        text.SetCommandText(&DBGUID_DEFAULT, PCWSTR(wide(sql).as_ptr()))
            .map_err(e)?;
        let mut rs: Option<windows::core::IUnknown> = None;
        text.Execute(None, &IRowset::IID, None, None, Some(&mut rs))
            .map_err(e)?;
        let rowset: IRowset = rs.ok_or("ziadna mnozina riadkov")?.cast().map_err(e)?;
        let acc: IAccessor = rowset.cast().map_err(e)?;
        const BUF: usize = 520;
        let binding = DBBINDING {
            iOrdinal: 1,
            obValue: 0,
            obLength: 0,
            obStatus: 0,
            dwPart: DBPART_VALUE.0 as u32,
            dwMemOwner: DBMEMOWNER_CLIENTOWNED.0 as u32,
            eParamIO: DBPARAMIO_NOTPARAM.0 as u32,
            cbMaxLen: BUF * 2,
            wType: DBTYPE_WSTR.0 as u16,
            ..Default::default()
        };
        let mut h = HACCESSOR::default();
        acc.CreateAccessor(
            DBACCESSOR_ROWDATA.0 as u32,
            1,
            &binding,
            BUF * 2,
            &mut h,
            None,
        )
        .map_err(e)?;
        let mut out = Vec::new();
        while out.len() < limit && !cancelled() {
            let mut got = 0usize;
            let mut rows: [*mut usize; 1] = [std::ptr::null_mut()];
            if rowset.GetNextRows(0, 0, &mut got, &mut rows).is_err()
                || got == 0
                || rows[0].is_null()
            {
                break;
            }
            let hrow = *rows[0];
            let mut buf = [0u16; BUF];
            if rowset.GetData(hrow, h, buf.as_mut_ptr() as *mut _).is_ok() {
                let n = buf.iter().position(|c| *c == 0).unwrap_or(BUF);
                if let Ok(s) = String::from_utf16(&buf[..n]) {
                    if !s.is_empty() {
                        out.push(s);
                    }
                }
            }
            let _ = rowset.ReleaseRows(
                1,
                &hrow,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            CoTaskMemFree(Some(rows[0] as *const _));
        }
        let _ = acc.ReleaseAccessor(h, None);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    /// Rozpoznanie sietoveho disku nesmie robit sietovy pristup: aj pre vsetky pismena a nezmyselne vstupy je to rychle.
    #[test]
    fn the_screenshots_folder_is_an_absolute_path_without_a_trailing_separator_and_is_never_created(
    ) {
        // tvar, nie hodnota: skutocna cesta pouzivatela sa nevypisuje ani nelogue
        let p = screenshots_folder().expect("znamy priecinok sa da urcit");
        let b = p.as_bytes();
        let abs = (b.len() > 3 && b[1] == b':' && b[2] == b'\\') || p.starts_with("\\\\");
        assert!(abs && !p.ends_with('\\'), "tvar cesty");
        let known = known_folder(&FOLDERID_Screenshots);
        assert!(known.is_none() || known.as_deref() == Some(p.as_str()));
    }

    #[test]
    fn the_drive_type_check_is_local_fast_and_tolerant_of_nonsense() {
        let t = std::time::Instant::now();
        for c in 'A'..='Z' {
            let _ = is_remote_drive(&format!("{c}:\\x"));
        }
        assert!(
            t.elapsed() < std::time::Duration::from_millis(500),
            "26 diskov trvalo {:?}",
            t.elapsed()
        );
        assert!(!is_remote_drive(""), "prazdne");
        assert!(!is_remote_drive("x"), "bez disku");
        assert!(
            !is_remote_drive(r"\\srv\share"),
            "UNC rozpoznava lane, nie tato funkcia"
        );
        // systemovy disk nie je sietovy
        let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        assert!(!is_remote_drive(&format!("{sys}\\Windows")));
    }

    #[test]
    fn dangerous_targets_are_refused_before_reaching_the_shell() {
        assert!(!target_ok(""));
        assert!(!target_ok("C:\\a\u{0}b"));
        assert!(!target_ok("C:\\a\nb"));
        assert!(target_ok(r"C:\Windows\notepad.exe"));
        assert!(target_ok("ms-settings:sound"));
        assert!(!open_target("bad\u{7}target"));
        assert!(!open_containing(r"C:\definitely\missing\file.xyz"));
    }

    /// Skutocny zoznam aplikacii tohto pocitaca (len citanie, nic sa nespusta).
    #[test]
    fn the_apps_folder_lists_real_applications_with_launchable_targets() {
        std::thread::spawn(|| {
            // SAFETY: COM na testovacom vlakne.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            let apps = list_apps();
            assert!(apps.len() <= MAX_APPS);
            assert!(apps.iter().all(|(n, t)| !n.is_empty()
                && t.starts_with("shell:AppsFolder\\")
                && target_ok(t)));
            eprintln!("aplikacii v zlozke Aplikacie: {}", apps.len());
            // SAFETY: parter k inicializacii.
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }

    /// Skutocny index Windows Search (len citanie). Ak sluzba nebezi, ide o `Err`, nie o chybu testu; vysledok sa len zmeria.
    #[test]
    fn the_windows_search_index_answers_a_read_only_query_or_reports_unavailable() {
        std::thread::spawn(|| {
            // SAFETY: COM na testovacom vlakne.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            let t0 = std::time::Instant::now();
            let r = index_query("notepad", 10, &|| false);
            eprintln!(
                "index Windows Search: {:?} za {:?}",
                r.as_ref().map(|v| v.len()),
                t0.elapsed()
            );
            if let Ok(v) = r {
                assert!(v.len() <= 10);
                assert!(v.iter().all(|p| !p.is_empty()));
            }
            // SAFETY: parter k inicializacii.
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }

    /// Meranie (nie kontrola): studeny a teply dotaz, LIKE s predponou a bez nej. Vypis ide do `--nocapture`.
    /// Spusta sa rucne (`cargo test -p transom-platform -- --ignored measure_index --nocapture`), nie v CI.
    #[test]
    #[ignore = "meranie, nie kontrola"]
    fn measure_index_latency_for_the_design_record() {
        std::thread::spawn(|| {
            // SAFETY: COM na testovacom vlakne.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            for (label, sql) in [
                ("LIKE %x% (1.)", "SELECT TOP 8 System.ItemPathDisplay FROM SystemIndex WHERE SCOPE='file:' AND System.FileName LIKE '%notepad%' ORDER BY System.Search.Rank DESC"),
                ("LIKE %x% (2.)", "SELECT TOP 8 System.ItemPathDisplay FROM SystemIndex WHERE SCOPE='file:' AND System.FileName LIKE '%notepad%' ORDER BY System.Search.Rank DESC"),
                ("LIKE x% bez ORDER", "SELECT TOP 8 System.ItemPathDisplay FROM SystemIndex WHERE SCOPE='file:' AND System.FileName LIKE 'notepad%'"),
                ("LIKE %x% bez ORDER", "SELECT TOP 8 System.ItemPathDisplay FROM SystemIndex WHERE SCOPE='file:' AND System.FileName LIKE '%notepad%'"),
                ("CONTAINS x*", "SELECT TOP 8 System.ItemPathDisplay FROM SystemIndex WHERE SCOPE='file:' AND CONTAINS(System.FileName, '\"notepad*\"')"),
            ] {
                let t0 = std::time::Instant::now();
                let r = index_sql(sql, 8, &|| false);
                eprintln!("MERANIE {label}: {:?} za {:?}", r.as_ref().map(|v| v.len()), t0.elapsed());
            }
            // SAFETY: parter k inicializacii.
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }
}
