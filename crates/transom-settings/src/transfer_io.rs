//! Suborove operacie exportu a importu nastaveni (bez okna a bez dialogu: cesta sa vstrekuje, takze sa daju testovat s
//! docasnymi subormi). Pravidla: import cita najviac `MAX_IMPORT_BYTES` (obmedzene cez `take`, nie podla veľkosti pred citanim),
//! len bezny subor (nie priecinok ani zariadenie), UTF-16 sa odmietne zrozumitelnou chybou; export zapisuje atomicky, nikdy
//! nevytvara priecinky a nikdy neprepise vlastne subory transom (konfiguraciu, zamok, lokalne data).
use std::io::Read;
use std::path::{Component, Path, Prefix};
use transom_config::transfer::{self, ExportOpts, ImportDoc, ImportError, MAX_IMPORT_BYTES};
use transom_config::Config;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TransferError {
    /// Chyba suboroveho systemu (text bez cesty).
    Io(String),
    /// Nie je to bezny subor (priecinok, zariadenie) alebo cesta nie je obycajna cesta k suboru.
    NotAFile,
    TooLarge,
    /// Subor je v UTF-16 (BOM).
    Utf16,
    /// Subor nie je platny UTF-8.
    NotUtf8,
    Format(ImportError),
    /// Ciel exportu je subor alebo priecinok samotneho transom.
    PrivatePath,
    /// Nazov suboru bez pripony (dialog ju nepridal).
    NeedExtension,
}

/// Obycajna cesta k suboru: absolutna, `X:\...` alebo UNC (`\\server\share`); odmietne zariadenia (`\\.\CON`), ostatne
/// `\\?\` cesty a relativne nazvy (`CON`, `NUL`). Obrana do hlbky: dialog vracia bezne cesty.
pub(crate) fn is_plain_file_path(p: &Path) -> bool {
    if !p.is_absolute() {
        return false;
    }
    matches!(
        p.components().next(),
        Some(Component::Prefix(pc))
            if matches!(
                pc.kind(),
                Prefix::Disk(_) | Prefix::UNC(..) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(..)
            )
    ) && p.file_name().is_some()
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Je cielovy subor suborom alebo priecinkom samotneho transom (konfiguracia, jej zamok a zalohy, lokalne data)? Export doň
/// nesmie zapisovat (dialog by prepisanie len potvrdil).
pub(crate) fn is_private_target(p: &Path) -> bool {
    let cfg = transom_config::default_path();
    let cfg_dir = cfg.parent().map(Path::to_path_buf).unwrap_or_default();
    let data_dir = transom_usage::cache::data_dir();
    let canon = |x: &Path| std::fs::canonicalize(x).unwrap_or_else(|_| x.to_path_buf());
    let parent = p.parent().map(canon).unwrap_or_default();
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let cfg_name = cfg
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    // konfiguracia a jej zamok / docasne subory v priecinku konfiguracie
    if same_path(&parent, &canon(&cfg_dir)) && !cfg_name.is_empty() && name.starts_with(&cfg_name) {
        return true;
    }
    // zalohy konfiguracie
    if same_path(&parent, &canon(&cfg_dir.join("backups"))) {
        return true;
    }
    // lokalne data transom (cache, archiv AI, zaznamy mostu)
    same_path(&parent, &canon(&data_dir))
}

/// Precita najviac `MAX_IMPORT_BYTES` bajtov; vacsi vstup (aj taky, ktoreho velkost klame alebo narasta) = `TooLarge`.
pub(crate) fn read_limited(r: impl Read, len_hint: u64) -> Result<Vec<u8>, TransferError> {
    if len_hint > MAX_IMPORT_BYTES as u64 {
        return Err(TransferError::TooLarge);
    }
    let mut buf = Vec::with_capacity(len_hint.min(MAX_IMPORT_BYTES as u64) as usize);
    r.take(MAX_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| TransferError::Io(e.to_string()))?;
    if buf.len() > MAX_IMPORT_BYTES {
        return Err(TransferError::TooLarge);
    }
    Ok(buf)
}

/// Dekoduje bajty suboru a overi format.
pub(crate) fn decode_import(bytes: &[u8]) -> Result<ImportDoc, TransferError> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(TransferError::Utf16);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| TransferError::NotUtf8)?;
    transfer::parse_import(text).map_err(TransferError::Format)
}

/// Nacita a overi subor importu. Nic nezapisuje.
pub(crate) fn import_from(path: &Path) -> Result<ImportDoc, TransferError> {
    if !is_plain_file_path(path) {
        return Err(TransferError::NotAFile);
    }
    // priecinok (alebo zariadenie) sa neda otvorit ako subor: zrozumitelna chyba namiesto "pristup zamietnuty"
    if std::fs::metadata(path).is_ok_and(|m| !m.is_file()) {
        return Err(TransferError::NotAFile);
    }
    let f = std::fs::File::open(path).map_err(|e| TransferError::Io(e.to_string()))?;
    let meta = f.metadata().map_err(|e| TransferError::Io(e.to_string()))?;
    if !meta.is_file() {
        return Err(TransferError::NotAFile);
    }
    let bytes = read_limited(f, meta.len())?;
    decode_import(&bytes)
}

/// Zapise export nastaveni (presne `transfer::export`, nic navyse). Atomicky; nevytvara priecinky.
pub(crate) fn export_to(path: &Path, cfg: &Config, opts: &ExportOpts) -> Result<(), TransferError> {
    if !is_plain_file_path(path) {
        return Err(TransferError::NotAFile);
    }
    if path.extension().is_none() {
        return Err(TransferError::NeedExtension);
    }
    if is_private_target(path) {
        return Err(TransferError::PrivatePath);
    }
    transom_config::write_atomic(path, transfer::export(cfg, opts).as_bytes())
        .map_err(|e| TransferError::Io(e.to_string()))
}
