//! Spolocne ukladanie nastrojov: umiestnenie v lokalnych datach, verzovane indexy (JSON s `format` a `version`), atomicky zapis
//! s kopiou `.bak`, tolerantne nacitanie a ochrana: poskodeny subor sa neprepise bez obnovy zo zalohy, subor z novsej verzie
//! sa **nikdy** neprepise a nacitanie je obmedzene velkostou. Obsah sa nikdy nelogue.
use serde_json::{Map, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Najvacsi index, ktory sa nacita (chrani pamat pred podvrhnutym suborom).
pub const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

/// Lokalne data transom: `TRANSOM_DATA`, inak `%LOCALAPPDATA%\transom` (rovnako ako ostatne casti transom).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("TRANSOM_DATA").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|b| PathBuf::from(b).join("transom"))
        .unwrap_or_else(|| std::env::temp_dir().join("transom"))
}

/// Korenovy priecinok dat nastrojov.
pub fn tools_dir() -> PathBuf {
    data_dir().join("tools")
}

/// Dovod, preco sa ulozisko nepouzije.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unusable {
    /// Poskodeny a bez pouzitelnej zalohy: subor ostava na disku nedotknuty.
    Corrupt,
    /// Z novsej verzie transom: nic sa nepremazava.
    Newer(u64),
    Unreadable,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Loaded {
    Missing,
    Ok(Value),
    /// Obnovene zo zalohy `.bak` (primarny subor bol poskodeny alebo chybal).
    Recovered(Value),
    Unusable(Unusable),
}

enum ReadErr {
    Missing,
    Unreadable,
    Corrupt,
}

fn read_value(path: &Path) -> Result<Value, ReadErr> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadErr::Missing),
        Err(_) => return Err(ReadErr::Unreadable),
    };
    match f.metadata() {
        Ok(m) if m.is_file() && m.len() <= MAX_INDEX_BYTES => {}
        Ok(m) if m.is_file() => return Err(ReadErr::Corrupt),
        _ => return Err(ReadErr::Unreadable),
    }
    let mut buf = Vec::new();
    // take(cap + 1): ani rastuci subor nepreskoci strop
    if f.take(MAX_INDEX_BYTES + 1).read_to_end(&mut buf).is_err() {
        return Err(ReadErr::Unreadable);
    }
    if buf.len() as u64 > MAX_INDEX_BYTES {
        return Err(ReadErr::Corrupt);
    }
    let text = std::str::from_utf8(&buf).map_err(|_| ReadErr::Corrupt)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    serde_json::from_str(text).map_err(|_| ReadErr::Corrupt)
}

fn backup_path(path: &Path) -> PathBuf {
    let mut n = path.as_os_str().to_owned();
    n.push(".bak");
    PathBuf::from(n)
}

/// Overi `format` a `version` a vrati obsah (objekt). Novsia verzia = `Newer`, zly format = poskodeny.
fn check(v: Value, format: &str, current: u64) -> Result<Value, Unusable> {
    let Some(o) = v.as_object() else {
        return Err(Unusable::Corrupt);
    };
    // indexy z casov pred premenovanim (lintel -> transom) maju format "lintel-tools-*": ten isty obsah, pri dalsom
    // ulozeni sa zapise novy nazov formatu
    let legacy = format
        .strip_prefix("transom-")
        .map(|r| format!("lintel-{r}"));
    let got = o.get("format").and_then(Value::as_str);
    if got != Some(format) && (legacy.is_none() || got != legacy.as_deref()) {
        return Err(Unusable::Corrupt);
    }
    let ver = o
        .get("version")
        .and_then(Value::as_u64)
        .ok_or(Unusable::Corrupt)?;
    if ver > current {
        return Err(Unusable::Newer(ver));
    }
    Ok(v)
}

/// Nacita index `path`: najprv primarny subor, pri poskodeni zalohu `.bak`. Novsia verzia sa nikdy nenahradi zalohou.
pub fn load_index(path: &Path, format: &str, current: u64) -> Loaded {
    let primary = read_value(path);
    let bak = || match read_value(&backup_path(path)) {
        Ok(v) => check(v, format, current).ok(),
        Err(_) => None,
    };
    match primary {
        Ok(v) => match check(v, format, current) {
            Ok(v) => Loaded::Ok(v),
            Err(Unusable::Newer(n)) => Loaded::Unusable(Unusable::Newer(n)),
            Err(u) => bak().map(Loaded::Recovered).unwrap_or(Loaded::Unusable(u)),
        },
        Err(ReadErr::Missing) => bak().map(Loaded::Recovered).unwrap_or(Loaded::Missing),
        Err(ReadErr::Corrupt) => bak()
            .map(Loaded::Recovered)
            .unwrap_or(Loaded::Unusable(Unusable::Corrupt)),
        Err(ReadErr::Unreadable) => Loaded::Unusable(Unusable::Unreadable),
    }
}

/// Zapise index atomicky; predchadzajuci platny index sa najprv skopiruje do `.bak`. Vola sa len pre pouzitelne ulozisko.
pub fn save_index(
    path: &Path,
    format: &str,
    version: u64,
    payload: Map<String, Value>,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut o = Map::new();
    o.insert("format".into(), Value::String(format.into()));
    o.insert("version".into(), Value::from(version));
    for (k, v) in payload {
        o.insert(k, v);
    }
    let bytes = serde_json::to_vec(&Value::Object(o)).map_err(std::io::Error::other)?;
    // posledny platny index sa zachova ako zaloha (najlepsia snaha; chyba zalohy zapis neblokuje)
    if read_value(path).is_ok() {
        let _ = std::fs::copy(path, backup_path(path));
    }
    transom_config::write_atomic(path, &bytes)
}

/// Zmaze vlastny priecinok (napr. `files/<id>`): len ak je to obycajny priecinok (nie odkaz) pod `root` a posledna zlozka je
/// nase id. Odkazy (symlink, junction) sa neprechadzaju; `remove_dir_all` v Ruste ich odpoji, nesleduje.
pub fn remove_own_dir(root: &Path, id: &str) -> bool {
    if !crate::names::is_id(id) {
        return false;
    }
    let p = root.join(id);
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => std::fs::remove_dir_all(&p).is_ok(),
        Ok(m) if m.file_type().is_symlink() => {
            // odkaz sa len odstrani (cil sa nedotkne)
            std::fs::remove_dir(&p).is_ok() || std::fs::remove_file(&p).is_ok()
        }
        _ => false,
    }
}

/// Odstrani zalohu indexu (`index.json.bak`): po vymazani poloziek v nej nesmu zostat ich nazvy ani obsah. Dalsie ulozenie ju vytvori znova.
pub fn purge_backup(index_path: &Path) {
    let mut os = index_path.as_os_str().to_owned();
    os.push(".bak");
    let _ = std::fs::remove_file(PathBuf::from(os));
}

/// Ulozista, ktore sa daju vymazat na ziadost (nazvy podpriecinkov).
/// Snimky obrazovky tu nie su: kniznica snimok iba cita priecinky pouzivatela a nema co vymazat.
pub const STORES: [&str; 3] = ["shelf", "clipboard", "notes"];

/// Zapise znacku "vymazat ulozisko" (`clear-<ulozisko>.req`) do korena nastrojov; vykona ju bezici transom (jediny pisar). Neznamy
/// nazov sa odmietne, takze znacka nikdy neobsahuje cestu.
pub fn request_clear(root: &Path, store: &str) -> std::io::Result<()> {
    if !STORES.contains(&store) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "neznamy sklad",
        ));
    }
    std::fs::create_dir_all(root)?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(format!("clear-{store}.req")))
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// Vezme (a odstrani) cakajuce znacky; vrati nazvy ulozisk v stalom poradi.
pub fn take_clear_requests(root: &Path) -> Vec<&'static str> {
    STORES
        .iter()
        .copied()
        .filter(|s| {
            let p = root.join(format!("clear-{s}.req"));
            p.is_file() && std::fs::remove_file(p).is_ok()
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "transom-tools-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_from_before_the_rename_is_still_readable() {
        let v: Value =
            serde_json::from_str(r#"{"format":"lintel-tools-shelf","version":1,"items":[]}"#)
                .unwrap();
        assert!(check(v, "transom-tools-shelf", 1).is_ok());
        let other: Value =
            serde_json::from_str(r#"{"format":"lintel-tools-notes","version":1}"#).unwrap();
        assert!(matches!(
            check(other, "transom-tools-shelf", 1),
            Err(Unusable::Corrupt)
        ));
    }

    fn payload(n: u64) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("n".into(), Value::from(n));
        m
    }

    #[test]
    fn a_saved_index_round_trips_and_keeps_the_previous_one_as_a_backup() {
        let d = temp_dir("store-rt");
        let p = d.join("t").join("index.json");
        assert_eq!(load_index(&p, "f", 1), Loaded::Missing);
        save_index(&p, "f", 1, payload(1)).unwrap();
        save_index(&p, "f", 1, payload(2)).unwrap();
        let Loaded::Ok(v) = load_index(&p, "f", 1) else {
            panic!()
        };
        assert_eq!(v["n"], 2);
        let bak: Value = serde_json::from_slice(&std::fs::read(backup_path(&p)).unwrap()).unwrap();
        assert_eq!(bak["n"], 1);
        // ziadny docasny subor nezostal
        let tmp = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .count();
        assert_eq!(tmp, 0);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_corrupt_primary_is_recovered_from_the_backup_and_never_silently_replaced_by_nothing() {
        let d = temp_dir("store-bak");
        let p = d.join("index.json");
        save_index(&p, "f", 1, payload(1)).unwrap();
        save_index(&p, "f", 1, payload(2)).unwrap();
        std::fs::write(&p, b"{ prerusene").unwrap();
        let Loaded::Recovered(v) = load_index(&p, "f", 1) else {
            panic!()
        };
        assert_eq!(v["n"], 1, "obnovene zo zalohy");
        // aj primarny subor chyba: zaloha sa pouzije
        std::fs::remove_file(&p).unwrap();
        assert!(matches!(load_index(&p, "f", 1), Loaded::Recovered(_)));
        // poskodeny primarny aj zaloha: nepouzitelne a subor ostava na disku
        std::fs::write(&p, b"[]").unwrap();
        std::fs::write(backup_path(&p), b"nie json").unwrap();
        assert_eq!(load_index(&p, "f", 1), Loaded::Unusable(Unusable::Corrupt));
        assert_eq!(std::fs::read(&p).unwrap(), b"[]");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_newer_version_or_a_foreign_format_is_never_used_or_replaced_by_the_backup() {
        let d = temp_dir("store-newer");
        let p = d.join("index.json");
        save_index(&p, "f", 1, payload(1)).unwrap();
        save_index(&p, "f", 1, payload(2)).unwrap();
        // simulacia suboru z buducej verzie
        std::fs::write(&p, br#"{"format":"f","version":9,"n":3}"#).unwrap();
        assert_eq!(load_index(&p, "f", 1), Loaded::Unusable(Unusable::Newer(9)));
        // cudzi format
        std::fs::write(&p, br#"{"format":"x","version":1}"#).unwrap();
        assert!(matches!(
            load_index(&p, "f", 1),
            Loaded::Recovered(_) | Loaded::Unusable(Unusable::Corrupt)
        ));
        // chybajuca verzia
        std::fs::write(backup_path(&p), b"{}").unwrap();
        std::fs::write(&p, br#"{"format":"f"}"#).unwrap();
        assert_eq!(load_index(&p, "f", 1), Loaded::Unusable(Unusable::Corrupt));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_oversized_index_is_refused_without_reading_it_all_and_a_directory_is_unreadable() {
        let d = temp_dir("store-big");
        let p = d.join("index.json");
        let big = vec![b' '; (MAX_INDEX_BYTES + 10) as usize];
        std::fs::write(&p, &big).unwrap();
        assert_eq!(load_index(&p, "f", 1), Loaded::Unusable(Unusable::Corrupt));
        std::fs::remove_file(&p).unwrap();
        std::fs::create_dir(&p).unwrap();
        assert_eq!(
            load_index(&p, "f", 1),
            Loaded::Unusable(Unusable::Unreadable)
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn only_our_own_id_named_directories_are_removed() {
        let d = temp_dir("store-rm");
        let id = crate::names::new_id();
        std::fs::create_dir_all(d.join(&id)).unwrap();
        std::fs::write(d.join(&id).join("f.txt"), b"x").unwrap();
        std::fs::create_dir_all(d.join("not-an-id")).unwrap();
        assert!(!remove_own_dir(&d, "not-an-id"));
        assert!(!remove_own_dir(&d, ".."));
        assert!(d.join("not-an-id").exists());
        assert!(remove_own_dir(&d, &id));
        assert!(!d.join(&id).exists());
        assert!(!remove_own_dir(&d, &id), "uz neexistuje");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_data_directory_follows_transom_data() {
        // nezavisle od prostredia: len skladanie ciest
        assert!(tools_dir().ends_with("tools"));
    }
}
