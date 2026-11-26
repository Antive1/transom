//! Vyber suborov pre "Pridat do Police": zdokumentovany spolocny dialog `IFileOpenDialog` (viac suborov, len suborovy system,
//! odkazy `.lnk` sa nenasleduju). Vracia **len cesty**: nic sa neotvara, nespusta ani nekopiruje; ich dalsie spracovanie je rovnake
//! ako pri pusteni na listu (kontrola ciest, duplicity, limity, odkaz alebo kopia podla nastaveni). Dialog je modalny a caka na
//! pouzivatela: volat ho vzdy z vlastneho vlakna (nie z vlakna UI); vlakno si samo inicializuje COM.
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FILEOPENDIALOGOPTIONS, FOS_ALLOWMULTISELECT,
    FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NODEREFERENCELINKS,
    FOS_PATHMUSTEXIST, SIGDN_FILESYSPATH,
};

/// Najviac tolko ciest z jedneho vyberu (rovnako ako pri pusteni).
pub const MAX_PICKED: usize = 500;

/// Volby dialogu: viac suborov, iba skutocne subory (nie kniznice ani virtualne polozky), subor musi existovat, nezapisovat do
/// "nedavnych", odkaz `.lnk` sa nenahradza svojim cielom (prida sa subor, ktory pouzivatel vybral).
pub fn pick_options() -> FILEOPENDIALOGOPTIONS {
    FOS_ALLOWMULTISELECT
        | FOS_FORCEFILESYSTEM
        | FOS_FILEMUSTEXIST
        | FOS_PATHMUSTEXIST
        | FOS_DONTADDTORECENT
        | FOS_NODEREFERENCELINKS
}

/// Cesty z vyberu ocistene na to, co smie ist dalej: neprazdne, bez riadiacich znakov, najviac `MAX_PICKED`.
pub fn clean_picked(paths: Vec<String>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|p| !p.is_empty() && !p.chars().any(char::is_control))
        .take(MAX_PICKED)
        .collect()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Otvori dialog (`owner` = okno lisy, 0 = bez vlastnika) a vrati vybrane cesty; zrusenie = prazdny zoznam. `Err` pri chybe COM.
pub fn pick_files(owner: isize, title: &str) -> windows::core::Result<Vec<String>> {
    // SAFETY: COM pre toto vlakno (STA, dialog ho vyzaduje); parter `CoUninitialize` nizsie, ak inicializacia uspela.
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let r = run(owner, title);
    if init {
        // SAFETY: parter k uspesnemu `CoInitializeEx`; vsetky COM objekty z `run` su uz zahodene.
        unsafe { CoUninitialize() };
    }
    r
}

fn run(owner: isize, title: &str) -> windows::core::Result<Vec<String>> {
    // SAFETY: volania COM nad platnymi objektmi; retazce zijú cez volanie; PWSTR od shellu sa uvolni `CoTaskMemFree`.
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dlg.SetOptions(pick_options())?;
        let t = wide(title);
        let _ = dlg.SetTitle(PCWSTR(t.as_ptr()));
        let owner = (owner != 0).then_some(HWND(owner as *mut _));
        if dlg.Show(owner).is_err() {
            // zrusenie pouzivatelom (`HRESULT_FROM_WIN32(ERROR_CANCELLED)`) nie je chyba
            return Ok(Vec::new());
        }
        let items = dlg.GetResults()?;
        let n = items.GetCount()?;
        let mut out = Vec::new();
        for i in 0..n.min(MAX_PICKED as u32) {
            let Ok(item) = items.GetItemAt(i) else {
                continue;
            };
            let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
                continue;
            };
            if let Ok(s) = name.to_string() {
                out.push(s);
            }
            CoTaskMemFree(Some(name.0 as *const _));
        }
        Ok(clean_picked(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_options_allow_several_real_files_and_never_follow_shortcuts() {
        let o = pick_options();
        for f in [
            FOS_ALLOWMULTISELECT,
            FOS_FORCEFILESYSTEM,
            FOS_FILEMUSTEXIST,
            FOS_PATHMUSTEXIST,
            FOS_DONTADDTORECENT,
            FOS_NODEREFERENCELINKS,
        ] {
            assert!(o.contains(f), "{f:?}");
        }
    }

    #[test]
    fn picked_paths_are_cleaned_and_limited_before_they_reach_the_shelf() {
        let mut v: Vec<String> = (0..MAX_PICKED + 20)
            .map(|i| format!("C:\\a\\{i}.txt"))
            .collect();
        v.push(String::new());
        v.insert(0, "C:\\bad\u{7}\\x.txt".to_string());
        let c = clean_picked(v);
        assert_eq!(c.len(), MAX_PICKED);
        assert!(c
            .iter()
            .all(|p| !p.is_empty() && !p.chars().any(char::is_control)));
    }

    /// Dialog sa vytvori a nastavi bez zobrazenia (ziadne UI, ziadne cakanie na pouzivatela): over zapojenie COM a volieb.
    #[test]
    fn the_dialog_object_can_be_created_and_configured_without_showing_it() {
        std::thread::spawn(|| {
            // SAFETY: COM na testovacom vlakne; ziaden `Show`.
            unsafe {
                let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
                let dlg: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                        .expect("spolocny dialog");
                dlg.SetOptions(pick_options()).unwrap();
                let got = dlg.GetOptions().unwrap();
                assert!(got.contains(FOS_ALLOWMULTISELECT) && got.contains(FOS_FORCEFILESYSTEM));
                let t = wide("Test");
                dlg.SetTitle(PCWSTR(t.as_ptr())).unwrap();
                drop(dlg);
                if init {
                    CoUninitialize();
                }
            }
        })
        .join()
        .unwrap();
    }
}
