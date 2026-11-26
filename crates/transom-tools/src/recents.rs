//! Posledne volby vyhladavaca (iba nazvy vybranych vysledkov, nie dotazy): **predvolene vypnute**, ukladaju sa lokalne v
//! `tools\launcher\recent.json` (verzovane, atomicky), nikdy do `config.json`, exportu ani archivov AI. Vypnutie nastavenia subor zmaze.
use crate::names::clean_display;
use crate::store::{load_index, save_index, Loaded};
use serde_json::{Map, Value};
use std::path::PathBuf;

pub const FORMAT: &str = "transom-tools-recents";
pub const VERSION: u64 = 1;
pub const MAX: usize = 20;

pub struct Recents {
    path: PathBuf,
    items: Vec<String>,
    read_only: bool,
}

impl Recents {
    pub fn open(dir: PathBuf) -> Recents {
        let path = dir.join("recent.json");
        let (items, read_only) = match load_index(&path, FORMAT, VERSION) {
            Loaded::Missing => (Vec::new(), false),
            Loaded::Ok(v) | Loaded::Recovered(v) => (
                v.get("items")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| clean_display(s, 200)))
                            .filter(|s| !s.is_empty())
                            .take(MAX)
                            .collect()
                    })
                    .unwrap_or_default(),
                false,
            ),
            Loaded::Unusable(_) => (Vec::new(), true),
        };
        Recents {
            path,
            items,
            read_only,
        }
    }

    pub fn list(&self) -> Vec<String> {
        self.items.clone()
    }

    /// Zapamata volbu (najnovsia prva, bez duplicit, najviac `MAX`).
    pub fn push(&mut self, title: &str) {
        let t = clean_display(title, 200);
        if t.is_empty() || self.read_only {
            return;
        }
        self.items.retain(|x| *x != t);
        self.items.insert(0, t);
        self.items.truncate(MAX);
        let mut m = Map::new();
        m.insert(
            "items".into(),
            Value::Array(self.items.iter().cloned().map(Value::String).collect()),
        );
        let _ = save_index(&self.path, FORMAT, VERSION, m);
    }

    /// Zmaze zoznam aj subor (vypnutie nastavenia, "Vymazat").
    pub fn clear(&mut self) {
        self.items.clear();
        if !self.read_only {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(self.path.with_extension("json.bak"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::temp_dir;

    #[test]
    fn recents_are_newest_first_unique_limited_persistent_and_removable() {
        let d = temp_dir("recents");
        let mut r = Recents::open(d.clone());
        assert!(r.list().is_empty());
        for i in 0..30 {
            r.push(&format!("app {i}"));
        }
        r.push("app 29");
        r.push("app 10\u{202E}");
        assert_eq!(r.list().len(), MAX);
        assert_eq!(r.list()[0], "app 10", "ciste znaky");
        assert_eq!(Recents::open(d.clone()).list(), r.list());
        r.clear();
        assert!(Recents::open(d.clone()).list().is_empty());
        assert!(!d.join("recent.json").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_newer_or_corrupt_file_is_not_overwritten() {
        let d = temp_dir("recents-bad");
        std::fs::write(
            d.join("recent.json"),
            br#"{"format":"transom-tools-recents","version":9,"items":["x"]}"#,
        )
        .unwrap();
        let mut r = Recents::open(d.clone());
        r.push("a");
        assert!(r.list().is_empty());
        assert!(std::fs::read_to_string(d.join("recent.json"))
            .unwrap()
            .contains("\"version\":9"));
        r.clear();
        assert!(
            d.join("recent.json").exists(),
            "ani vymazanie sa nedotkne cudzej verzie"
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
