//! Natívne dialogy Ulozit / Otvorit pre export a import (`GetSaveFileNameW` / `GetOpenFileNameW`; dokumentovane, stale
//! podporovane). Cista cast (nazov, filter, buffer, flagy, struktura) je oddelena od volania, takze sa da testovat bez
//! zobrazenia dialogu. Dialog je modalny a bezi vo vlastnej slucke sprav: volat ho len mimo vypozicky stavu okna.
use std::path::PathBuf;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_DONTADDTORECENT, OFN_EXPLORER,
    OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_NOREADONLYRETURN,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW, OPEN_FILENAME_FLAGS,
};

/// Velkost buffera cesty (znaky UTF-16): dlhe cesty sa nezrezu.
pub(crate) const BUF_LEN: usize = 32768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Save,
    Open,
}

/// Vysledok dialogu.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    Chosen(PathBuf),
    /// Pouzivatel dialog zrusil (nie je to chyba).
    Cancelled,
    /// Chyba dialogu (kod `CommDlgExtendedError`).
    Failed(u32),
}

/// Navrhovany nazov exportu: `transom-settings-YYYY-MM-DD.json`.
pub(crate) fn default_export_name(y: i32, m: u32, d: u32) -> String {
    format!("transom-settings-{y:04}-{m:02}-{d:02}.json")
}

/// Filter s dvojitou nulou: `popis\0vzor\0popis\0vzor\0\0`.
pub(crate) fn filter_wide(pairs: &[(&str, &str)]) -> Vec<u16> {
    let mut v: Vec<u16> = Vec::new();
    for (label, pattern) in pairs {
        v.extend(label.encode_utf16());
        v.push(0);
        v.extend(pattern.encode_utf16());
        v.push(0);
    }
    v.push(0);
    v
}

/// Buffer cesty s predvyplnenym nazvom (orezany tak, aby sa zmestil aj ukoncovaci NUL).
pub(crate) fn buffer_with(initial: &str) -> Vec<u16> {
    let mut b = vec![0u16; BUF_LEN];
    for (i, c) in initial.encode_utf16().take(BUF_LEN - 1).enumerate() {
        b[i] = c;
    }
    b
}

/// Cesta z buffera po prvy NUL; prazdna = `None`.
pub(crate) fn path_from_buffer(buf: &[u16]) -> Option<PathBuf> {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if end == 0 {
        return None;
    }
    Some(PathBuf::from(
        <std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(&buf[..end]),
    ))
}

/// Flagy dialogu: export pyta potvrdenie prepisu a nevracia subor len na citanie; import vyzaduje existujuci subor.
pub(crate) fn flags(kind: Kind) -> OPEN_FILENAME_FLAGS {
    let common =
        OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY | OFN_DONTADDTORECENT;
    match kind {
        Kind::Save => common | OFN_OVERWRITEPROMPT | OFN_NOREADONLYRETURN,
        Kind::Open => common | OFN_FILEMUSTEXIST,
    }
}

/// Texty dialogu (v jazyku aplikacie).
pub(crate) struct Spec {
    pub title: String,
    pub filter: Vec<u16>,
    /// Predvoleny nazov pre Ulozit (pre Otvorit prazdny).
    pub initial: String,
    /// Pripona doplnena, ak pouzivatel napise nazov bez nej (UTF-16 ukoncena nulou; `wide0("json")`).
    pub def_ext: Vec<u16>,
}

/// Text ako UTF-16 ukonceny nulou.
pub(crate) fn wide0(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Struktura `OPENFILENAMEW`; `buf` a `spec` musia zit dlhsie nez volanie dialogu.
pub(crate) fn build_ofn(
    kind: Kind,
    owner: HWND,
    buf: &mut [u16],
    spec: &Spec,
    title: &[u16],
) -> OPENFILENAMEW {
    OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(spec.filter.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: flags(kind),
        lpstrDefExt: PCWSTR(spec.def_ext.as_ptr()),
        ..Default::default()
    }
}

/// Zobrazi dialog (modalne). Vola sa mimo vypozicky stavu okna. Zrusenie nie je chyba.
pub(crate) fn run_dialog(kind: Kind, owner: HWND, spec: &Spec) -> Pick {
    let mut buf = buffer_with(&spec.initial);
    let title: Vec<u16> = spec
        .title
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut ofn = build_ofn(kind, owner, &mut buf, spec, &title);
    // SAFETY: `ofn` odkazuje na buffre, ktore zijú do konca volania; dialog je modalny vo vlakne okna.
    let ok = unsafe {
        match kind {
            Kind::Save => GetSaveFileNameW(&mut ofn),
            Kind::Open => GetOpenFileNameW(&mut ofn),
        }
    }
    .as_bool();
    if ok {
        return path_from_buffer(&buf).map_or(Pick::Cancelled, Pick::Chosen);
    }
    // SAFETY: cisto citanie chyboveho kodu poslednej operacie dialogu.
    let code = unsafe { CommDlgExtendedError() }.0;
    if code == 0 {
        Pick::Cancelled
    } else {
        Pick::Failed(code)
    }
}
