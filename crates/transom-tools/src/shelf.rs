//! Polica: trvale ulozene polozky suborov. Polozka je **odkaz** na povodny subor (predvolene) alebo **ulozena kopia** v
//! `tools\shelf\files\<id>\`. Povodne subory sa nikdy nemenia, nemazu, neotvaraju ani nespustaju; kopia vznika len na vyslovnu
//! volbu, nikdy neprepise nic existujuce (novy priecinok na id), pri chybe sa upratuje a drzi sa v kvotach.
use crate::names::{check_path, file_name_of, parent_of, path_key, stored_file_name, PathReject};
use crate::store::{load_index, remove_own_dir, save_index, Loaded, Unusable};
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use transom_config::ShelfCfg;

pub const FORMAT: &str = "transom-tools-shelf";
pub const VERSION: u64 = 1;
/// Najviac tolko poloziek z jedneho pustenia (zvysok sa zahodi a ohlasi).
pub const MAX_DROP_ITEMS: usize = 500;
const MAX_LOADED_ITEMS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ref,
    Copy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    /// Povodna cesta (pri odkaze cielova; pri kopii len informacia o povode).
    pub path: String,
    /// Nazov suboru kopie v `files\<id>\` (prazdny pri odkaze).
    pub stored: String,
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub added: i64,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Available,
    /// Zdroj chyba (polozka sa nemaze, len sa ukaze).
    Missing,
    /// Subor je len v cloude / offline: neotvara sa ani nekopiruje bez vyslovneho suhlasu.
    Offline,
    /// Sietova cesta (alebo neodpovedala vcas): nesonduje sa automaticky, nezobrazuje sa ako chybajuca.
    Unchecked,
}

/// Co sa zistilo o ceste bez toho, aby sa citali data (nikdy nesposobi stiahnutie z cloudu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub exists: bool,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
    /// `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN` alebo `OFFLINE`.
    pub offline: bool,
    pub reparse: bool,
    /// Cesta sa nepodarilo skontrolovat (sietova cesta sa nesonduje; sonda neodpovedala vcas). Nie je to "chyba": `exists` je
    /// vtedy `true` a ostatne udaje nie su znamé.
    pub unchecked: bool,
}

/// Zistovanie vlastnosti ciest (v testoch falosne; skutocne cez metadata bez nasledovania odkazov). Sluzba pouziva
/// `lane::LaneProbe`, ktory ich obali casovym limitom; priame volanie z pracovneho vlakna by mohlo visiet na sietovej ceste.
pub trait FileProbe {
    fn probe(&self, path: &Path) -> Probe;
    /// Vyzera cesta sietova? Taka sa nesonduje a nekopiruje sa z nej.
    fn remote(&self, _path: &Path) -> bool {
        false
    }
    /// Otvorenie suboru na kopirovanie (v sluzbe s casovym limitom).
    fn open(&self, path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::File::open(path)
    }
}

pub struct FsProbe;

impl FileProbe for FsProbe {
    fn probe(&self, path: &Path) -> Probe {
        let Ok(m) = std::fs::symlink_metadata(path) else {
            return Probe::default();
        };
        #[cfg(windows)]
        let (offline, reparse) = {
            use std::os::windows::fs::MetadataExt;
            let a = m.file_attributes();
            (
                a & (0x0040_0000 | 0x0004_0000 | 0x0000_1000) != 0,
                a & 0x0400 != 0,
            )
        };
        #[cfg(not(windows))]
        let (offline, reparse) = (false, m.file_type().is_symlink());
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Probe {
            exists: true,
            is_dir: m.is_dir(),
            size: if m.is_file() { m.len() } else { 0 },
            mtime,
            offline,
            reparse,
            unchecked: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    Invalid(PathReject),
    Missing,
    Directory,
    Reparse,
    Offline,
    /// Zo sietovej cesty (alebo takej, ktora neodpovedala) sa nekopiruje; ako odkaz ju Polica prijme.
    Network,
    TooLarge,
    Quota,
    Full,
    CopyFailed,
    Disabled,
    ReadOnly,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AddReport {
    pub added: usize,
    pub duplicates: usize,
    pub rejected: Vec<RejectReason>,
    /// Pustenie malo viac poloziek nez `MAX_DROP_ITEMS`; zvysok sa zahodil.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Recovered,
    /// Ulozisko sa nepouzije (poskodene / z novsej verzie): nic sa nezapisuje ani nemaze.
    Unusable(Unusable),
}

/// Vysledok presunu polozky o jedno miesto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveResult {
    /// Presunuta; nova pozicia v zobrazenom zozname (od 1; pripnute su hore) a pocet poloziek.
    Moved {
        pos: usize,
        total: usize,
    },
    /// Dalej sa v svojej skupine (pripnute / nepripnute) posunut neda; nic sa nezmenilo. `first` = hore.
    Edge {
        first: bool,
    },
    Unknown,
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub location: String,
    pub size: u64,
    pub kind: Kind,
    pub dir: bool,
    pub pinned: bool,
    pub state: State,
}

pub struct Shelf {
    dir: PathBuf,
    cfg: ShelfCfg,
    items: Vec<Item>,
    pub status: Status,
    /// Po dalsom ulozeni sa zmaze `.bak` (v nom by ostali odstranene polozky).
    purge_bak: std::cell::Cell<bool>,
}

fn item_to_value(i: &Item) -> Value {
    json!({
        "id": i.id, "kind": if i.kind == Kind::Ref { "ref" } else { "copy" }, "path": i.path, "stored": i.stored,
        "name": i.name, "dir": i.dir, "size": i.size, "mtime": i.mtime, "added": i.added, "pinned": i.pinned,
    })
}

/// Tolerantne nacitanie jednej polozky; neplatna sa zahodi (nikdy nezhodi cele ulozisko).
fn item_from_value(v: &Value) -> Option<Item> {
    let o = v.as_object()?;
    let id = o.get("id")?.as_str()?;
    if !crate::names::is_id(id) {
        return None;
    }
    let kind = match o.get("kind")?.as_str()? {
        "ref" => Kind::Ref,
        "copy" => Kind::Copy,
        _ => return None,
    };
    let path = o.get("path")?.as_str()?.to_string();
    let stored = o
        .get("stored")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match kind {
        Kind::Ref if check_path(&path).is_err() => return None,
        // nazov kopie musi byt presne taky, ake vyrabame (ziadne oddelovace ani `..`)
        Kind::Copy if stored.is_empty() || stored_file_name(&stored) != stored => return None,
        _ => {}
    }
    if path.encode_utf16().count() > 32_767 {
        return None;
    }
    Some(Item {
        id: id.to_string(),
        kind,
        name: crate::names::clean_display(
            o.get("name")
                .and_then(Value::as_str)
                .unwrap_or(&file_name_of(&path)),
            120,
        ),
        path,
        stored,
        dir: o.get("dir").and_then(Value::as_bool).unwrap_or(false),
        size: o.get("size").and_then(Value::as_u64).unwrap_or(0),
        mtime: o.get("mtime").and_then(Value::as_i64).unwrap_or(0),
        added: o.get("added").and_then(Value::as_i64).unwrap_or(0),
        pinned: o.get("pinned").and_then(Value::as_bool).unwrap_or(false),
    })
}

impl Shelf {
    pub fn open(dir: PathBuf, cfg: ShelfCfg) -> Shelf {
        let (items, status) = match load_index(&dir.join("index.json"), FORMAT, VERSION) {
            Loaded::Missing => (Vec::new(), Status::Ok),
            Loaded::Ok(v) => (Self::parse(&v), Status::Ok),
            Loaded::Recovered(v) => (Self::parse(&v), Status::Recovered),
            Loaded::Unusable(u) => (Vec::new(), Status::Unusable(u)),
        };
        let s = Shelf {
            dir,
            cfg,
            items,
            status,
            purge_bak: std::cell::Cell::new(false),
        };
        s.sweep_orphans();
        s
    }

    /// Osirotene kopie (`files\<id>\` bez polozky v indexe, napr. po zlyhanom ulozeni alebo padu): zmazu sa pri otvoreni.
    /// Dotyka sa len priecinkov s nasim nahodnym id.
    fn sweep_orphans(&self) {
        if self.read_only() {
            return;
        }
        let Ok(rd) = std::fs::read_dir(self.files_dir()) else {
            return;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if crate::names::is_id(&n) && !self.items.iter().any(|i| i.id == n) {
                remove_own_dir(&self.files_dir(), &n);
            }
        }
    }

    fn parse(v: &Value) -> Vec<Item> {
        v.get("items")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(MAX_LOADED_ITEMS)
                    .filter_map(item_from_value)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_cfg(&mut self, cfg: ShelfCfg) {
        self.cfg = cfg;
    }

    fn read_only(&self) -> bool {
        matches!(self.status, Status::Unusable(_))
    }

    fn files_dir(&self) -> PathBuf {
        self.dir.join("files")
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn copy_bytes(&self) -> u64 {
        self.items
            .iter()
            .filter(|i| i.kind == Kind::Copy)
            .fold(0u64, |a, i| a.saturating_add(i.size))
    }

    /// Cesta, ktora sa da ponuknut ako ciel tahania / kopirovania: odkaz = povodny subor, kopia = ulozeny subor.
    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        let i = self.items.iter().find(|i| i.id == id)?;
        Some(match i.kind {
            Kind::Ref => PathBuf::from(&i.path),
            Kind::Copy => self.files_dir().join(&i.id).join(&i.stored),
        })
    }

    /// Prida polozky (najnovsia prva). Povodne subory sa len zistuju cez `probe`; kopia sa robi len pri `copy`.
    pub fn add_paths(
        &mut self,
        paths: &[String],
        copy: bool,
        probe: &dyn FileProbe,
        now: i64,
    ) -> AddReport {
        let mut r = AddReport::default();
        if self.read_only() {
            r.rejected.push(RejectReason::ReadOnly);
            return r;
        }
        if !self.cfg.enabled {
            r.rejected.push(RejectReason::Disabled);
            return r;
        }
        if paths.len() > MAX_DROP_ITEMS {
            r.truncated = true;
        }
        // prijate polozky sa vlozia nakoniec naraz (poradie pustenia sa zachova: prva polozka zoznamu hore)
        let mut accepted: Vec<Item> = Vec::new();
        let pinned_now = self.items.iter().filter(|i| i.pinned).count();
        let room = (self.cfg.max_items as usize).saturating_sub(pinned_now);
        // kopie z tohto pustenia sa do indexu vlozia az na konci, ale do spolocnej kvoty sa pocitaju hned
        let mut batch_copy_bytes = 0u64;
        for p in paths.iter().take(MAX_DROP_ITEMS) {
            if let Err(e) = check_path(p) {
                r.rejected.push(RejectReason::Invalid(e));
                continue;
            }
            let key = path_key(p);
            if self.items.iter().any(|i| path_key(&i.path) == key)
                || accepted.iter().any(|i| path_key(&i.path) == key)
            {
                r.duplicates += 1;
                continue;
            }
            // sietova cesta sa nesonduje vobec (nedostupny server by zdrzal vsetky nastroje); ostatne idu cez limit v `probe`
            let pr = if probe.remote(Path::new(p)) {
                Probe {
                    exists: true,
                    unchecked: true,
                    ..Probe::default()
                }
            } else {
                probe.probe(Path::new(p))
            };
            if !pr.exists {
                r.rejected.push(RejectReason::Missing);
                continue;
            }
            // Polica sa nezvacsi nad limit: pripnute sa nevytlacaju; stare nepripnute sa uvolnia az po prijati (odmietnuta
            // polozka nesmie vytlacit existujuce)
            if accepted.len() >= room {
                r.rejected.push(RejectReason::Full);
                continue;
            }
            let id = crate::names::new_id();
            let mut item = Item {
                id: id.clone(),
                kind: Kind::Ref,
                path: p.clone(),
                stored: String::new(),
                name: crate::names::clean_display(&file_name_of(p), 120),
                dir: pr.is_dir,
                size: pr.size,
                mtime: pr.mtime,
                added: now,
                pinned: false,
            };
            if copy {
                if pr.unchecked {
                    r.rejected.push(RejectReason::Network);
                    continue;
                }
                if pr.is_dir {
                    r.rejected.push(RejectReason::Directory);
                    continue;
                }
                if pr.reparse {
                    r.rejected.push(RejectReason::Reparse);
                    continue;
                }
                if pr.offline {
                    r.rejected.push(RejectReason::Offline);
                    continue;
                }
                let cap = u64::from(self.cfg.max_copy_mb) * 1024 * 1024;
                let total = u64::from(self.cfg.max_total_mb) * 1024 * 1024;
                if pr.size > cap {
                    r.rejected.push(RejectReason::TooLarge);
                    continue;
                }
                if self
                    .copy_bytes()
                    .saturating_add(batch_copy_bytes)
                    .saturating_add(pr.size)
                    > total
                {
                    r.rejected.push(RejectReason::Quota);
                    continue;
                }
                match self.copy_in(probe, Path::new(p), &id, p, cap) {
                    Ok((stored, size)) => {
                        item.kind = Kind::Copy;
                        item.stored = stored;
                        item.size = size;
                        batch_copy_bytes = batch_copy_bytes.saturating_add(size);
                    }
                    Err(_) => {
                        r.rejected.push(RejectReason::CopyFailed);
                        continue;
                    }
                }
            }
            accepted.push(item);
            r.added += 1;
        }
        // uvolni miesto (najstarsie nepripnute) a vloz prijate na vrch v poradi pustenia
        while self.items.len() + accepted.len() > self.cfg.max_items as usize {
            let Some(pos) = self.oldest_unpinned() else {
                break;
            };
            self.drop_at(pos);
        }
        self.items.splice(0..0, accepted);
        r
    }

    /// Skopiruje `src` do nove vytvoreneho `files\<id>\` (nikdy neprepise): docasny subor `create_new`, strop velkosti pri citani,
    /// premenovanie az po uspechu; pri chybe sa priecinok zmaze.
    fn copy_in(
        &self,
        probe: &dyn FileProbe,
        src: &Path,
        id: &str,
        original: &str,
        cap: u64,
    ) -> std::io::Result<(String, u64)> {
        let dir = self.files_dir().join(id);
        std::fs::create_dir_all(self.files_dir())?;
        std::fs::create_dir(&dir)?; // chyba, ak uz existuje
        let name = stored_file_name(original);
        let result = (|| -> std::io::Result<u64> {
            let mut input = probe.open(src)?;
            // len obycajny subor (nie priecinok, rura ani ine zariadenie)
            if !input.metadata()?.is_file() {
                return Err(std::io::Error::other("nie je to obycajny subor"));
            }
            let tmp = dir.join(format!("{name}.part"));
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            let mut buf = vec![0u8; 64 * 1024];
            let mut total = 0u64;
            loop {
                let n = (&mut input)
                    .take(cap + 1 - total.min(cap + 1))
                    .read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > cap {
                    return Err(std::io::Error::other("subor je vacsi nez strop"));
                }
                out.write_all(&buf[..n])?;
            }
            out.sync_all()?;
            drop(out);
            std::fs::rename(&tmp, dir.join(&name))?;
            Ok(total)
        })();
        match result {
            Ok(size) => Ok((name, size)),
            Err(e) => {
                let _ = remove_own_dir(&self.files_dir(), id);
                Err(e)
            }
        }
    }

    /// Uvolni miesto: odstrani najstarsie nepripnute polozky nad limit a po uplynuti retencie.
    fn drop_at(&mut self, pos: usize) {
        self.purge_bak.set(true);
        let it = self.items.remove(pos);
        if it.kind == Kind::Copy {
            remove_own_dir(&self.files_dir(), &it.id);
        }
    }

    /// Retencia (len nepripnute) a limit poctu; vrati pocet odstranenych.
    pub fn enforce(&mut self, now: i64) -> usize {
        if self.read_only() {
            return 0;
        }
        let before = self.items.len();
        if self.cfg.retention_days > 0 {
            let limit = now - i64::from(self.cfg.retention_days) * 86_400;
            let old: Vec<usize> = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| !i.pinned && i.added < limit)
                .map(|(p, _)| p)
                .collect();
            for p in old.into_iter().rev() {
                self.drop_at(p);
            }
        }
        while self.items.len() > self.cfg.max_items as usize {
            let Some(pos) = self.oldest_unpinned() else {
                break;
            };
            self.drop_at(pos);
        }
        before - self.items.len()
    }

    /// Index najstarsej nepripnutej polozky podla casu pridania (pri rovnosti tej, co je v zozname neskor). Vyhadzuje sa podla
    /// veku, nie podla pozicie: poradie moze pouzivatel menit, takze posledna pozicia nemusi byt najstarsia polozka.
    fn oldest_unpinned(&self) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, i)| !i.pinned)
            .min_by_key(|(p, i)| (i.added, std::cmp::Reverse(*p)))
            .map(|(p, _)| p)
    }

    /// Posunie polozku o jedno miesto hore (`delta < 0`) alebo dole (`delta > 0`) v ramci svojej skupiny: pripnute sa vzdy
    /// zobrazuju hore, preto sa pripnute a nepripnute medzi sebou nemiesaju. Meni sa len poradie v indexe (format ostava verzie 1);
    /// subory ani kopie sa nedotykaju.
    pub fn move_item(&mut self, id: &str, delta: i32) -> MoveResult {
        if self.read_only() {
            return MoveResult::ReadOnly;
        }
        let Some(i) = self.items.iter().position(|x| x.id == id) else {
            return MoveResult::Unknown;
        };
        let pinned = self.items[i].pinned;
        let group: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, x)| x.pinned == pinned)
            .map(|(p, _)| p)
            .collect();
        let k = group.iter().position(|p| *p == i).unwrap_or(0);
        let target = if delta < 0 {
            k.checked_sub(1)
        } else {
            Some(k + 1).filter(|t| *t < group.len())
        };
        let Some(t) = target else {
            return MoveResult::Edge { first: delta < 0 };
        };
        self.items.swap(i, group[t]);
        // zobrazene poradie: pripnute su vzdy hore, potom nepripnute
        let base = if pinned {
            0
        } else {
            self.items.iter().filter(|x| x.pinned).count()
        };
        MoveResult::Moved {
            pos: base + t + 1,
            total: self.items.len(),
        }
    }

    pub fn toggle_pin(&mut self, id: &str) -> bool {
        if self.read_only() {
            return false;
        }
        match self.items.iter_mut().find(|i| i.id == id) {
            Some(i) => {
                i.pinned = !i.pinned;
                true
            }
            None => false,
        }
    }

    /// Odstrani polozku zo Police. Povodny subor sa nikdy nedotkne; maze sa len vlastna kopia.
    pub fn remove(&mut self, id: &str) -> bool {
        if self.read_only() {
            return false;
        }
        match self.items.iter().position(|i| i.id == id) {
            Some(p) => {
                self.drop_at(p);
                true
            }
            None => false,
        }
    }

    /// Vymaze Policu (volitelne bez pripnutych); vrati pocet odstranenych poloziek.
    pub fn clear(&mut self, keep_pinned: bool) -> usize {
        if self.read_only() {
            return 0;
        }
        let ids: Vec<String> = self
            .items
            .iter()
            .filter(|i| !(keep_pinned && i.pinned))
            .map(|i| i.id.clone())
            .collect();
        for id in &ids {
            self.remove(id);
        }
        ids.len()
    }

    /// Zoznam pre UI; stav zdroja sa zistuje bez citania dat (volat z pracovneho vlakna: siet moze visiet).
    pub fn rows(&self, probe: &dyn FileProbe) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .items
            .iter()
            .map(|i| {
                let target = self.path_of(&i.id).unwrap_or_default();
                let pr = probe.probe(&target);
                let state = if pr.unchecked {
                    State::Unchecked
                } else if !pr.exists {
                    State::Missing
                } else if pr.offline {
                    State::Offline
                } else {
                    State::Available
                };
                Row {
                    id: i.id.clone(),
                    name: i.name.clone(),
                    location: parent_of(&i.path),
                    size: if i.dir { 0 } else { i.size },
                    kind: i.kind,
                    dir: i.dir,
                    pinned: i.pinned,
                    state,
                }
            })
            .collect();
        // pripnute hore (stabilne: poradie v ramci skupiny sa zachova)
        rows.sort_by_key(|r| !r.pinned);
        rows
    }

    pub fn save(&self) -> std::io::Result<()> {
        if self.read_only() {
            return Err(std::io::Error::other("ulozisko sa nepouziva"));
        }
        let mut m = Map::new();
        m.insert(
            "items".into(),
            Value::Array(self.items.iter().map(item_to_value).collect()),
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
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake(HashMap<String, Probe>);

    impl Fake {
        fn file(mut self, p: &str, size: u64) -> Self {
            self.0.insert(
                p.into(),
                Probe {
                    exists: true,
                    size,
                    ..Probe::default()
                },
            );
            self
        }
        fn with(mut self, p: &str, pr: Probe) -> Self {
            self.0.insert(p.into(), pr);
            self
        }
    }

    impl FileProbe for Fake {
        fn probe(&self, p: &Path) -> Probe {
            self.0
                .get(p.to_string_lossy().as_ref())
                .copied()
                .unwrap_or_default()
        }
    }

    fn cfg() -> ShelfCfg {
        transom_config::Config::default().tools.shelf
    }

    fn paths(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_multi_file_drop_adds_references_newest_first_and_survives_a_restart() {
        let d = temp_dir("shelf-ref");
        let probe = Fake::default()
            .file(r"C:\a\one.txt", 10)
            .file(r"C:\a\two.pdf", 2000);
        let mut s = Shelf::open(d.clone(), cfg());
        let r = s.add_paths(
            &paths(&[r"C:\a\one.txt", r"C:\a\two.pdf"]),
            false,
            &probe,
            1000,
        );
        assert_eq!((r.added, r.duplicates, r.rejected.len()), (2, 0, 0));
        assert_eq!(
            s.items()[0].name,
            "one.pdf".replace("one.pdf", "one.txt"),
            "poradie pustenia sa zachova"
        );
        assert!(s.items().iter().all(|i| i.kind == Kind::Ref));
        s.save().unwrap();
        let s2 = Shelf::open(d.clone(), cfg());
        assert_eq!(s2.status, Status::Ok);
        assert_eq!(s2.items(), s.items());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn duplicates_missing_and_hostile_paths_are_reported_and_never_stored() {
        let d = temp_dir("shelf-dup");
        let probe = Fake::default().file(r"C:\a\x.txt", 1);
        let mut s = Shelf::open(d.clone(), cfg());
        s.add_paths(&paths(&[r"C:\a\x.txt"]), false, &probe, 1);
        let r = s.add_paths(
            &paths(&[
                r"c:/A/X.TXT",
                r"\\?\C:\a\x.txt",
                r"C:\gone.txt",
                r"rel\x",
                r"\\.\PhysicalDrive0",
                r"C:\a\CON",
            ]),
            false,
            &probe,
            2,
        );
        assert_eq!(r.added, 0);
        assert_eq!(r.duplicates, 2);
        assert_eq!(
            r.rejected,
            vec![
                RejectReason::Missing,
                RejectReason::Invalid(PathReject::Relative),
                RejectReason::Invalid(PathReject::Device),
                RejectReason::Invalid(PathReject::Device),
            ]
        );
        assert_eq!(s.len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_huge_drop_is_truncated_and_the_item_limit_evicts_oldest_unpinned_but_never_pinned() {
        let d = temp_dir("shelf-limit");
        let mut probe = Fake::default();
        let many: Vec<String> = (0..600).map(|i| format!(r"C:\f\{i}.txt")).collect();
        for p in &many {
            probe = probe.file(p, 1);
        }
        let mut c = cfg();
        c.max_items = 5;
        let mut s = Shelf::open(d.clone(), c);
        let r = s.add_paths(&many, false, &probe, 1);
        assert!(r.truncated);
        assert!(s.len() <= 5);
        assert_eq!(
            s.items()[0].name,
            "0.txt",
            "poradie pustenia sa zachova, prijatych je najviac 5"
        );
        // pripnute polozky sa nevytlacaju
        let id = s.items()[4].id.clone();
        assert!(s.toggle_pin(&id));
        s.add_paths(&many[500..510], false, &probe, 2);
        assert!(s.items().iter().any(|i| i.id == id && i.pinned));
        assert!(s.len() <= 5);
        // vsetky pripnute a plno: nova polozka sa odmietne (nic sa nezmaze)
        for i in s.items().to_vec() {
            if !i.pinned {
                s.toggle_pin(&i.id);
            }
        }
        let r = s.add_paths(&many[510..511], false, &probe, 3);
        assert_eq!(r.rejected, vec![RejectReason::Full]);
        assert_eq!(s.len(), 5);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn retention_removes_old_unpinned_items_only() {
        let d = temp_dir("shelf-ret");
        let probe = Fake::default()
            .file(r"C:\a\old.txt", 1)
            .file(r"C:\a\pin.txt", 1)
            .file(r"C:\a\new.txt", 1);
        let mut c = cfg();
        c.retention_days = 7;
        let mut s = Shelf::open(d.clone(), c);
        s.add_paths(&paths(&[r"C:\a\old.txt"]), false, &probe, 0);
        s.add_paths(&paths(&[r"C:\a\pin.txt"]), false, &probe, 0);
        let pin = s.items()[0].id.clone();
        s.toggle_pin(&pin);
        s.add_paths(&paths(&[r"C:\a\new.txt"]), false, &probe, 9 * 86_400);
        assert_eq!(s.enforce(10 * 86_400), 1);
        let names: Vec<&str> = s.items().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["new.txt", "pin.txt"]);
        let _ = std::fs::remove_dir_all(d);
    }

    fn names_of(s: &Shelf) -> Vec<String> {
        s.items().iter().map(|i| i.name.clone()).collect()
    }

    #[test]
    fn items_move_within_their_group_stop_at_the_edges_and_the_order_survives_a_restart() {
        let d = temp_dir("shelf-move");
        let probe = Fake::default()
            .file(r"C:\a\a.txt", 1)
            .file(r"C:\a\b.txt", 1)
            .file(r"C:\a\c.txt", 1)
            .file(r"C:\a\p.txt", 1);
        let mut s = Shelf::open(d.clone(), cfg());
        for (i, p) in [r"C:\a\a.txt", r"C:\a\b.txt", r"C:\a\c.txt", r"C:\a\p.txt"]
            .iter()
            .enumerate()
        {
            s.add_paths(&paths(&[p]), false, &probe, 10 + i as i64);
        }
        // novsie hore: p, c, b, a; p pripnuta (skupina pripnutych je zvlast)
        assert_eq!(names_of(&s), ["p.txt", "c.txt", "b.txt", "a.txt"]);
        let id = |s: &Shelf, n: &str| s.items().iter().find(|i| i.name == n).unwrap().id.clone();
        let p = id(&s, "p.txt");
        s.toggle_pin(&p);
        let (a, c) = (id(&s, "a.txt"), id(&s, "c.txt"));
        // a ide hore o dve miesta v skupine nepripnutych: c, b, a -> a, c, b
        assert_eq!(s.move_item(&a, -1), MoveResult::Moved { pos: 3, total: 4 });
        assert_eq!(s.move_item(&a, -1), MoveResult::Moved { pos: 2, total: 4 });
        assert_eq!(s.move_item(&a, -1), MoveResult::Edge { first: true });
        assert_eq!(
            s.rows(&probe)
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>(),
            ["p.txt", "a.txt", "c.txt", "b.txt"],
            "pripnuta vzdy hore"
        );
        // pripnuta je sama v skupine: nikam sa neposunie a neprejde medzi nepripnute
        assert_eq!(s.move_item(&p, 1), MoveResult::Edge { first: false });
        assert_eq!(s.move_item(&p, -1), MoveResult::Edge { first: true });
        assert_eq!(s.move_item(&c, 1), MoveResult::Moved { pos: 4, total: 4 });
        assert_eq!(s.move_item(&c, 1), MoveResult::Edge { first: false });
        assert_eq!(s.move_item("neexistuje", 1), MoveResult::Unknown);
        s.save().unwrap();
        // poradie prezije restart a format ostava verzie 1 (starsie vydania ho nacitaju)
        let s2 = Shelf::open(d.clone(), cfg());
        assert_eq!(names_of(&s2), names_of(&s));
        assert_eq!(s2.status, Status::Ok);
        let raw = std::fs::read_to_string(d.join("index.json")).unwrap();
        assert!(
            raw.contains("\"version\": 1") || raw.contains("\"version\":1"),
            "{raw}"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn eviction_after_a_reorder_drops_the_oldest_added_item_not_the_last_position() {
        let d = temp_dir("shelf-evict-order");
        let mut c = cfg();
        c.max_items = 3;
        let probe = Fake::default()
            .file(r"C:\a\a.txt", 1)
            .file(r"C:\a\b.txt", 1)
            .file(r"C:\a\c.txt", 1)
            .file(r"C:\a\d.txt", 1);
        let mut s = Shelf::open(d.clone(), c);
        for (i, p) in [r"C:\a\a.txt", r"C:\a\b.txt", r"C:\a\c.txt"]
            .iter()
            .enumerate()
        {
            s.add_paths(&paths(&[p]), false, &probe, 10 + i as i64);
        }
        assert_eq!(names_of(&s), ["c.txt", "b.txt", "a.txt"]);
        // pouzivatel presunie najstarsiu (a) na vrch: a, c, b
        let a = s
            .items()
            .iter()
            .find(|i| i.name == "a.txt")
            .unwrap()
            .id
            .clone();
        s.move_item(&a, -1);
        s.move_item(&a, -1);
        assert_eq!(names_of(&s), ["a.txt", "c.txt", "b.txt"]);
        // nova polozka vytlaci najstarsiu podla casu pridania (a), nie poslednu poziciu (b)
        s.add_paths(&paths(&[r"C:\a\d.txt"]), false, &probe, 99);
        assert_eq!(names_of(&s), ["d.txt", "c.txt", "b.txt"]);
        // aj retencia / limit v `enforce` sa riadi vekom
        let mut c2 = cfg();
        c2.max_items = 2;
        s.set_cfg(c2);
        assert_eq!(s.enforce(100), 1);
        assert_eq!(names_of(&s), ["d.txt", "c.txt"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn copies_of_one_drop_count_together_toward_the_total_quota() {
        let d = temp_dir("shelf-same-drop");
        let src_dir = temp_dir("shelf-same-drop-src");
        let mut c = cfg();
        c.max_copy_mb = 1;
        c.max_total_mb = 1;
        // 3 x 600 KiB v jednom pusteni: kazdy je pod stropom suboru, spolu 1,8 MiB > 1 MiB
        let mut ps = Vec::new();
        for i in 0..3 {
            let p = src_dir.join(format!("f{i}.bin"));
            std::fs::write(&p, vec![i as u8; 600 * 1024]).unwrap();
            ps.push(p.to_string_lossy().to_string());
        }
        let mut s = Shelf::open(d.clone(), c);
        let r = s.add_paths(&ps, true, &FsProbe, 1);
        assert!(
            s.copy_bytes() <= 1024 * 1024,
            "kopie z jedneho pustenia prekrocili spolocnu kvotu: {} B",
            s.copy_bytes()
        );
        assert_eq!(r.added, 1, "{r:?}");
        assert_eq!(r.rejected, vec![RejectReason::Quota, RejectReason::Quota]);
        // odmietnute subory nenechali v ulozisku nic
        let leftovers = std::fs::read_dir(d.join("files")).unwrap().count();
        assert_eq!(leftovers, 1);
        for p in &ps {
            assert!(Path::new(p).exists(), "originaly sa nikdy nemenia");
        }
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(src_dir);
    }

    #[test]
    fn a_stored_copy_is_made_without_touching_the_original_and_removing_it_deletes_only_the_copy() {
        let d = temp_dir("shelf-copy");
        let src_dir = temp_dir("shelf-src");
        let src = src_dir.join("hello world.txt");
        std::fs::write(&src, b"obsah").unwrap();
        let sp = src.to_string_lossy().to_string();
        let mut s = Shelf::open(d.clone(), cfg());
        let r = s.add_paths(std::slice::from_ref(&sp), true, &FsProbe, 1);
        assert_eq!((r.added, r.rejected.len()), (1, 0), "{r:?}");
        let it = s.items()[0].clone();
        assert_eq!(it.kind, Kind::Copy);
        let stored = s.path_of(&it.id).unwrap();
        assert_eq!(std::fs::read(&stored).unwrap(), b"obsah");
        assert!(stored.starts_with(d.join("files")));
        assert_eq!(s.copy_bytes(), 5);
        // zdroj sa zmeni / zmizne: kopia ostava, polozka sa nemaze
        std::fs::remove_file(&src).unwrap();
        let rows = s.rows(&FsProbe);
        assert_eq!(
            rows[0].state,
            State::Available,
            "kopia je dostupna aj bez zdroja"
        );
        // odstranenie polozky zmaze len kopiu
        std::fs::write(&src, b"obsah").unwrap();
        assert!(s.remove(&it.id));
        assert!(!stored.exists());
        assert!(src.exists(), "povodny subor sa nikdy nemaze");
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(src_dir);
    }

    #[test]
    fn copy_limits_directories_cloud_placeholders_and_reparse_points_are_refused_without_leftovers()
    {
        let d = temp_dir("shelf-quota");
        let mut c = cfg();
        c.max_copy_mb = 1;
        c.max_total_mb = 1;
        let probe = Fake::default()
            .file(r"C:\a\big.bin", 2 * 1024 * 1024)
            .with(
                r"C:\a\dir",
                Probe {
                    exists: true,
                    is_dir: true,
                    ..Probe::default()
                },
            )
            .with(
                r"C:\a\cloud.docx",
                Probe {
                    exists: true,
                    size: 10,
                    offline: true,
                    ..Probe::default()
                },
            )
            .with(
                r"C:\a\link",
                Probe {
                    exists: true,
                    size: 10,
                    reparse: true,
                    ..Probe::default()
                },
            );
        let mut s = Shelf::open(d.clone(), c);
        let r = s.add_paths(
            &paths(&[
                r"C:\a\big.bin",
                r"C:\a\dir",
                r"C:\a\cloud.docx",
                r"C:\a\link",
            ]),
            true,
            &probe,
            1,
        );
        assert_eq!(
            r.rejected,
            vec![
                RejectReason::TooLarge,
                RejectReason::Directory,
                RejectReason::Offline,
                RejectReason::Reparse
            ]
        );
        assert!(s.is_empty());
        assert!(
            !d.join("files").exists() || std::fs::read_dir(d.join("files")).unwrap().count() == 0
        );
        // ako odkaz sa vsetko dovoli (adresar, cloud, odkaz)
        let r = s.add_paths(
            &paths(&[r"C:\a\dir", r"C:\a\cloud.docx", r"C:\a\link"]),
            false,
            &probe,
            2,
        );
        assert_eq!(r.added, 3);
        let rows = s.rows(&probe);
        assert!(rows
            .iter()
            .any(|r| r.state == State::Offline && r.name == "cloud.docx"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_missing_source_stays_listed_as_missing() {
        let d = temp_dir("shelf-miss");
        let probe = Fake::default().file(r"C:\a\x.txt", 1);
        let mut s = Shelf::open(d.clone(), cfg());
        s.add_paths(&paths(&[r"C:\a\x.txt"]), false, &probe, 1);
        let rows = s.rows(&Fake::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, State::Missing);
        assert_eq!(s.len(), 1, "polozka sa nemaze");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn clear_can_keep_pinned_items_and_a_disabled_or_unusable_shelf_changes_nothing() {
        let d = temp_dir("shelf-clear");
        let probe = Fake::default().file(r"C:\a\1", 1).file(r"C:\a\2", 1);
        let mut s = Shelf::open(d.clone(), cfg());
        s.add_paths(&paths(&[r"C:\a\1", r"C:\a\2"]), false, &probe, 1);
        let keep = s.items()[0].id.clone();
        s.toggle_pin(&keep);
        assert_eq!(s.clear(true), 1);
        assert_eq!(s.len(), 1);
        s.save().unwrap();
        // vypnuta Polica
        let mut c = cfg();
        c.enabled = false;
        let mut off = Shelf::open(d.clone(), c);
        assert_eq!(
            off.add_paths(&paths(&[r"C:\a\1"]), false, &probe, 2)
                .rejected,
            vec![RejectReason::Disabled]
        );
        // novsia verzia: nic sa nezapise ani nezmaze
        std::fs::write(
            d.join("index.json"),
            br#"{"format":"transom-tools-shelf","version":99,"items":[]}"#,
        )
        .unwrap();
        let mut newer = Shelf::open(d.clone(), cfg());
        assert_eq!(newer.status, Status::Unusable(Unusable::Newer(99)));
        assert_eq!(
            newer
                .add_paths(&paths(&[r"C:\a\1"]), false, &probe, 3)
                .rejected,
            vec![RejectReason::ReadOnly]
        );
        assert!(newer.save().is_err());
        assert_eq!(newer.clear(false), 0);
        assert!(std::fs::read_to_string(d.join("index.json"))
            .unwrap()
            .contains("99"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_forged_index_cannot_smuggle_traversal_names_device_paths_or_bad_ids() {
        let d = temp_dir("shelf-forged");
        let idx = r#"{"format":"transom-tools-shelf","version":1,"items":[
            {"id":"0123456789abcdef","kind":"copy","path":"C:\\x","stored":"..\\..\\evil.txt","name":"a"},
            {"id":"0123456789abcdee","kind":"copy","path":"C:\\x","stored":"a\\b.txt","name":"a"},
            {"id":"../etc","kind":"ref","path":"C:\\ok.txt"},
            {"id":"0123456789abcded","kind":"ref","path":"\\\\.\\PhysicalDrive0"},
            {"id":"0123456789abcdec","kind":"ref","path":"relative.txt"},
            {"id":"0123456789abcdeb","kind":"weird","path":"C:\\ok.txt"},
            {"id":"0123456789abcdea","kind":"ref","path":"C:\\fine.txt","name":"fi\u202Ene","pinned":true},
            5, null]}"#;
        std::fs::write(d.join("index.json"), idx).unwrap();
        let s = Shelf::open(d.clone(), cfg());
        assert_eq!(s.len(), 1, "{:?}", s.items());
        assert_eq!(s.items()[0].name, "fine");
        assert!(s.items()[0].pinned);
        let _ = std::fs::remove_dir_all(d);
    }
}
