//! Rychle poznamky: obycajny text, len lokalne. Vytvorenie, uprava, pripnutie, hladanie, mazanie. Zapis je atomicky s
//! kopiou `.bak` (prerusenie zapisu nestrati poznamky); poskodeny subor bez zalohy sa neprepise (poznamky ostanu na disku a UI to
//! povie); poznamky sa nikdy nemazu automaticky a nikdy sa nesynchronizuju ani neexportuju.
use crate::names::{clean_display, new_id};
use crate::store::{load_index, save_index, Loaded, Unusable};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use transom_config::NotesCfg;

pub const FORMAT: &str = "transom-tools-notes";
pub const VERSION: u64 = 1;
/// Najvacsia poznamka (bajty UTF-8) a celkovy objem vsetkych poznamok.
pub const MAX_NOTE_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    pub text: String,
    pub created: i64,
    pub modified: i64,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteError {
    Disabled,
    ReadOnly,
    TooLarge,
    /// Dosiahnuty limit poctu alebo celkoveho objemu: nic sa nemaze, nova poznamka sa nevytvori.
    Full,
    NotFound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Recovered,
    Unusable(Unusable),
}

pub struct Notes {
    dir: PathBuf,
    cfg: NotesCfg,
    notes: Vec<Note>,
    pub status: Status,
    purge_bak: std::cell::Cell<bool>,
}

impl Notes {
    pub fn open(dir: PathBuf, cfg: NotesCfg) -> Notes {
        let (notes, status) = match load_index(&dir.join("index.json"), FORMAT, VERSION) {
            Loaded::Missing => (Vec::new(), Status::Ok),
            Loaded::Ok(v) => (Self::parse(&v), Status::Ok),
            Loaded::Recovered(v) => (Self::parse(&v), Status::Recovered),
            Loaded::Unusable(u) => (Vec::new(), Status::Unusable(u)),
        };
        Notes {
            dir,
            cfg,
            notes,
            status,
            purge_bak: std::cell::Cell::new(false),
        }
    }

    fn parse(v: &Value) -> Vec<Note> {
        let mut total = 0usize;
        let mut out = Vec::new();
        for n in v
            .get("notes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(5000)
        {
            let Some(o) = n.as_object() else { continue };
            let Some(id) = o
                .get("id")
                .and_then(Value::as_str)
                .filter(|i| crate::names::is_id(i))
            else {
                continue;
            };
            let Some(text) = o.get("text").and_then(Value::as_str) else {
                continue;
            };
            if text.len() > MAX_NOTE_BYTES || total + text.len() > MAX_TOTAL_BYTES {
                continue;
            }
            total += text.len();
            out.push(Note {
                id: id.to_string(),
                text: text.to_string(),
                created: o.get("created").and_then(Value::as_i64).unwrap_or(0),
                modified: o.get("modified").and_then(Value::as_i64).unwrap_or(0),
                pinned: o.get("pinned").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        out
    }

    pub fn set_cfg(&mut self, cfg: NotesCfg) {
        self.cfg = cfg;
    }

    fn read_only(&self) -> bool {
        matches!(self.status, Status::Unusable(_))
    }

    fn total_bytes(&self) -> usize {
        self.notes.iter().map(|n| n.text.len()).sum()
    }

    /// Poznamky v poradi zobrazenia: pripnute prve, potom najnovsie upravene.
    pub fn list(&self) -> Vec<&Note> {
        let mut v: Vec<&Note> = self.notes.iter().collect();
        v.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.modified.cmp(&a.modified)));
        v
    }

    pub fn get(&self, id: &str) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Nazov poznamky: prvy neprazdny riadok (ocisteny, najviac 80 znakov).
    pub fn title(n: &Note) -> String {
        let first = n
            .text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        clean_display(first, 80)
    }

    pub fn search(&self, q: &str) -> Vec<&Note> {
        let q = q.trim().to_lowercase();
        self.list()
            .into_iter()
            .filter(|n| q.is_empty() || n.text.to_lowercase().contains(&q))
            .collect()
    }

    /// Mazanie nie je viazane na `enabled`: vypnute poznamky sa stale daju vymazat (Nastavenia > Vymazat).
    fn guard_destroy(&self) -> Result<(), NoteError> {
        if self.read_only() {
            return Err(NoteError::ReadOnly);
        }
        Ok(())
    }

    fn guard(&self) -> Result<(), NoteError> {
        if !self.cfg.enabled {
            return Err(NoteError::Disabled);
        }
        if self.read_only() {
            return Err(NoteError::ReadOnly);
        }
        Ok(())
    }

    pub fn create(&mut self, text: &str, now: i64) -> Result<String, NoteError> {
        self.guard()?;
        if text.len() > MAX_NOTE_BYTES {
            return Err(NoteError::TooLarge);
        }
        if self.notes.len() >= self.cfg.max_notes as usize
            || self.total_bytes() + text.len() > MAX_TOTAL_BYTES
        {
            return Err(NoteError::Full);
        }
        let id = new_id();
        self.notes.push(Note {
            id: id.clone(),
            text: text.to_string(),
            created: now,
            modified: now,
            pinned: false,
        });
        self.save().map_err(|_| NoteError::ReadOnly)?;
        Ok(id)
    }

    pub fn update(&mut self, id: &str, text: &str, now: i64) -> Result<(), NoteError> {
        self.guard()?;
        if text.len() > MAX_NOTE_BYTES {
            return Err(NoteError::TooLarge);
        }
        let others: usize = self
            .notes
            .iter()
            .filter(|n| n.id != id)
            .map(|n| n.text.len())
            .sum();
        if others + text.len() > MAX_TOTAL_BYTES {
            return Err(NoteError::Full);
        }
        let n = self
            .notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(NoteError::NotFound)?;
        if n.text != text {
            self.purge_bak.set(true); // v zalohe by ostal povodny text (napr. odstranene heslo)
            n.text = text.to_string();
            n.modified = now;
            self.save().map_err(|_| NoteError::ReadOnly)?;
        }
        Ok(())
    }

    pub fn toggle_pin(&mut self, id: &str) -> Result<(), NoteError> {
        self.guard()?;
        let n = self
            .notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(NoteError::NotFound)?;
        n.pinned = !n.pinned;
        self.save().map_err(|_| NoteError::ReadOnly)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), NoteError> {
        self.guard_destroy()?;
        let p = self
            .notes
            .iter()
            .position(|n| n.id == id)
            .ok_or(NoteError::NotFound)?;
        self.notes.remove(p);
        let r = self.save().map_err(|_| NoteError::ReadOnly);
        crate::store::purge_backup(&self.dir.join("index.json"));
        r
    }

    /// Vymaze vsetky poznamky (volitelne bez pripnutych); iba na vyslovny pokyn (dvojkrokovo v UI).
    pub fn clear(&mut self, keep_pinned: bool) -> Result<usize, NoteError> {
        self.guard_destroy()?;
        let before = self.notes.len();
        self.notes.retain(|n| keep_pinned && n.pinned);
        let r = self.save().map_err(|_| NoteError::ReadOnly);
        crate::store::purge_backup(&self.dir.join("index.json"));
        r?;
        Ok(before - self.notes.len())
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn save(&self) -> std::io::Result<()> {
        if self.read_only() {
            return Err(std::io::Error::other("ulozisko sa nepouziva"));
        }
        let mut m = Map::new();
        m.insert(
            "notes".into(),
            Value::Array(
                self.notes
                    .iter()
                    .map(|n| json!({"id": n.id, "text": n.text, "created": n.created, "modified": n.modified, "pinned": n.pinned}))
                    .collect(),
            ),
        );
        let r = save_index(&self.dir.join("index.json"), FORMAT, VERSION, m);
        if self.purge_bak.replace(false) {
            crate::store::purge_backup(&self.dir.join("index.json"));
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::temp_dir;

    fn cfg() -> NotesCfg {
        transom_config::Config::default().tools.notes
    }

    #[test]
    fn notes_are_created_edited_pinned_searched_and_deleted_and_survive_a_restart() {
        let d = temp_dir("notes-crud");
        let mut n = Notes::open(d.clone(), cfg());
        let a = n.create("Nakup\nmlieko, chlieb", 10).unwrap();
        let b = n.create("Napad na projekt", 20).unwrap();
        assert_eq!(Notes::title(n.get(&a).unwrap()), "Nakup");
        assert_eq!(n.list()[0].id, b, "najnovsie prve");
        n.toggle_pin(&a).unwrap();
        assert_eq!(n.list()[0].id, a, "pripnute prve");
        assert_eq!(n.search("MLIEKO").len(), 1);
        n.update(&b, "Napad na projekt (hotovo)", 30).unwrap();
        let again = Notes::open(d.clone(), cfg());
        assert_eq!(again.len(), 2);
        assert_eq!(again.get(&b).unwrap().text, "Napad na projekt (hotovo)");
        assert!(again.get(&a).unwrap().pinned);
        n.delete(&a).unwrap();
        assert_eq!(n.delete(&a), Err(NoteError::NotFound));
        assert_eq!(Notes::open(d.clone(), cfg()).len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_interrupted_write_is_recovered_from_the_backup_without_losing_the_previous_notes() {
        let d = temp_dir("notes-recover");
        let mut n = Notes::open(d.clone(), cfg());
        n.create("jedna", 1).unwrap();
        n.create("dve", 2).unwrap();
        // prerusenie zapisu: primarny subor je useknuty
        let p = d.join("index.json");
        let full = std::fs::read(&p).unwrap();
        std::fs::write(&p, &full[..full.len() / 2]).unwrap();
        let r = Notes::open(d.clone(), cfg());
        assert_eq!(r.status, Status::Recovered);
        assert_eq!(r.len(), 1, "zaloha ma stav pred poslednou poznamkou");
        // ani zaloha nie je pouzitelna: poznamky ostanu na disku a nic sa neprepise
        std::fs::write(d.join("index.json.bak"), b"x").unwrap();
        let mut bad = Notes::open(d.clone(), cfg());
        assert_eq!(bad.status, Status::Unusable(Unusable::Corrupt));
        assert_eq!(bad.create("nova", 3), Err(NoteError::ReadOnly));
        assert_eq!(std::fs::read(&p).unwrap(), &full[..full.len() / 2]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn limits_never_delete_and_a_newer_file_is_never_overwritten() {
        let d = temp_dir("notes-limit");
        let mut c = cfg();
        c.max_notes = 2;
        let mut n = Notes::open(d.clone(), c);
        n.create("a", 1).unwrap();
        n.create("b", 2).unwrap();
        assert_eq!(n.create("c", 3), Err(NoteError::Full));
        assert_eq!(n.len(), 2, "nic sa nezmazalo");
        assert_eq!(
            n.create(&"x".repeat(MAX_NOTE_BYTES + 1), 3),
            Err(NoteError::TooLarge)
        );
        let first = n.list()[0].id.clone();
        assert_eq!(
            n.update(&first, &"y".repeat(MAX_NOTE_BYTES + 1), 4),
            Err(NoteError::TooLarge)
        );
        std::fs::write(
            d.join("index.json"),
            br#"{"format":"transom-tools-notes","version":5,"notes":[]}"#,
        )
        .unwrap();
        std::fs::remove_file(d.join("index.json.bak")).ok();
        let mut newer = Notes::open(d.clone(), cfg());
        assert_eq!(newer.status, Status::Unusable(Unusable::Newer(5)));
        assert_eq!(newer.create("x", 1), Err(NoteError::ReadOnly));
        assert!(std::fs::read_to_string(d.join("index.json"))
            .unwrap()
            .contains("\"version\":5"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn disabled_notes_change_nothing_and_clear_keeps_pinned_on_request() {
        let d = temp_dir("notes-off");
        let mut on = Notes::open(d.clone(), cfg());
        let a = on.create("pripnuta", 1).unwrap();
        on.create("bezna", 2).unwrap();
        on.toggle_pin(&a).unwrap();
        assert_eq!(on.clear(true), Ok(1));
        assert_eq!(on.len(), 1);
        let mut c = cfg();
        c.enabled = false;
        let mut off = Notes::open(d.clone(), c);
        assert_eq!(off.create("x", 1), Err(NoteError::Disabled));
        // mazanie nie je viazane na zapnutie (Nastavenia > Vymazat musi fungovat aj pri vypnutych poznamkach)
        assert_eq!(off.delete(&a), Ok(()));
        assert_eq!(off.len(), 0);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_forged_index_drops_bad_ids_and_oversized_notes() {
        let d = temp_dir("notes-forged");
        let huge = "x".repeat(MAX_NOTE_BYTES + 1);
        let idx = format!(
            r#"{{"format":"transom-tools-notes","version":1,"notes":[
              {{"id":"../a","text":"x"}},{{"id":"0123456789abcdef","text":"{huge}"}},
              {{"id":"0123456789abcdee","text":"dobra"}},{{"id":"0123456789abcded"}}, 7]}}"#
        );
        std::fs::write(d.join("index.json"), idx).unwrap();
        let n = Notes::open(d.clone(), cfg());
        assert_eq!(n.len(), 1);
        assert_eq!(n.list()[0].text, "dobra");
        let _ = std::fs::remove_dir_all(d);
    }
}
