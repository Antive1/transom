//! Lokalna cache suhrnov (`%LOCALAPPDATA%\transom\usage-cache.json`): len denne suhrny podla poskytovatela a
//! modelu, stav zdrojov a snimky limitov - ziadne cesty, projekty, ID relacii ani text. Panel ju zobrazi hned po
//! starte; obnova zdrojov bezi az na poziadanie alebo v nastavenom intervale.
use crate::Collected;
use std::path::{Path, PathBuf};

/// Adresar lokalnych dat transom (`%LOCALAPPDATA%\transom`), prepisatelny `TRANSOM_DATA` (testy).
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("TRANSOM_DATA") {
        return PathBuf::from(p);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("transom")
}

pub fn cache_path() -> PathBuf {
    data_dir().join("usage-cache.json")
}

/// Subor mostu statusline.
pub fn statusline_path() -> PathBuf {
    data_dir().join("claude-statusline.jsonl")
}

/// Udalosti z hookov Claude Code (zaciatok, nastroj, koniec, otazka) - len druh a cas, ziadny text.
pub fn events_path() -> PathBuf {
    data_dir().join("claude-events.jsonl")
}

pub fn save(path: &Path, c: &Collected) -> std::io::Result<()> {
    write_atomic(path, &serde_json::to_vec(c).map_err(std::io::Error::other)?)
}

/// Atomicky zapis: docasny subor s **unikatnym nazvom** (proces + pocitadlo; lista a nastavenia zapisuju do rovnakych
/// suborov), `sync_all` a premenovanie. Pri chybe sa docasny subor zmaze.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}.{n}.tmp", std::process::id()));
    let written = (|| {
        // `create_new`: na mieste docasneho suboru sa nikdy nenasleduje odkaz ani neprepise cudzi subor; zvysok po padu
        // procesu s rovnakym cislom sa zmaze (len samotna polozka, nie ciel odkazu) a skusi sa znova
        let open = || {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
        };
        let mut f = match open() {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&tmp)?;
                open()?
            }
            r => r?,
        };
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

pub fn load(path: &Path) -> Option<Collected> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Zmaze lokalne data transom o vyuziti AI (cache a zaznamy mostu statusline). Data Claude Code a Codex
/// (ich vlastne prepisy) sa nemenia. Vrati zoznam zmazanych suborov.
pub fn delete_local_data() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in [cache_path(), statusline_path()] {
        if p.exists() && std::fs::remove_file(&p).is_ok() {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DayTotals, Provider};

    #[test]
    fn delete_removes_only_transom_files() {
        let dir = std::env::temp_dir().join(format!("transom-usage-del-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // env je pre proces spolocny; ostatne testy tejto crate data_dir nepouzivaju
        std::env::set_var("TRANSOM_DATA", &dir);
        std::fs::write(cache_path(), b"{}").unwrap();
        std::fs::write(statusline_path(), b"{}\n").unwrap();
        let other = dir.join("ine.txt");
        std::fs::write(&other, b"x").unwrap();
        let d = delete_local_data();
        assert_eq!(d.len(), 2);
        assert!(!cache_path().exists() && !statusline_path().exists());
        assert!(other.exists(), "ine subory zostavaju");
        assert!(delete_local_data().is_empty(), "druhe zmazanie nic nerobi");
        std::env::remove_var("TRANSOM_DATA");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrip_contains_no_paths() {
        let dir = std::env::temp_dir().join(format!("transom-usage-cache-{}", std::process::id()));
        let p = dir.join("c.json");
        let c = Collected {
            days: vec![DayTotals {
                provider: Some(Provider::Codex),
                model: "gpt-6-luna".into(),
                day: (2026, 11, 20),
                input: 5,
                ..Default::default()
            }],
            ..Default::default()
        };
        save(&p, &c).unwrap();
        assert_eq!(load(&p).unwrap(), c);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("\\\\") && !text.contains("cwd"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&p).is_none());
    }
}
