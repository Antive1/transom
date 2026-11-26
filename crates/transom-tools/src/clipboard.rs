//! Historia schranky (OPT-IN, predvolene vypnuta): rozhodovanie, co sa smie zachytit, a ulozisko poloziek. Zachytava sa az po
//! vyslovnom zapnuti a len nove polozky; formaty oznacene aplikaciami ako citlive sa preskakuju (best-effort, nie zaruka),
//! vylucene aplikacie, pozastavenie, limity, retencia, pripnutie, mazanie, hladanie a obnovenie. Obsah sa nikdy nelogue.
//! Cista logika: skutocne citanie schranky robi platforma a odovzda `Snapshot`.
use crate::names::{check_path, clean_display, content_hash, file_name_of, new_id};
use crate::store::{load_index, save_index, Loaded, Unusable};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use transom_config::ClipboardCfg;

pub const FORMAT: &str = "transom-tools-clipboard";
pub const VERSION: u64 = 1;
const MAX_FILES_PER_ENTRY: usize = 100;
/// Najvacsia strana obrazka v historii (body); platforma ju pouziva uz pri citani hlavicky.
pub const MAX_IMAGE_SIDE: u32 = 8192;
const MAX_LOADED: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKind {
    Text,
    Image,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub kind: ClipKind,
    pub added: i64,
    pub pinned: bool,
    /// Nazov .exe zdroja (best-effort; moze byt prazdny alebo nepravdivy).
    pub app: String,
    pub preview: String,
    /// Maly text na hladanie (male pismena).
    pub search: String,
    pub size: u64,
    /// Odtlacok obsahu; nazov suboru v `blobs\` (`<hash>.txt` / `<hash>.png`); pri zoznamoch suborov odtlacok zoznamu ciest (len na odstranenie duplicit; `blobs\` sa nepouziva).
    pub hash: String,
    pub files: Vec<String>,
    pub dims: (u32, u32),
}

/// Co platforma zistila o aktualnom obsahu schranky (bez citania dat vsetkych formatov).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Nazvy formatov v schranke (porovnavaju sa bez ohladu na velkost pismen).
    pub formats: Vec<String>,
    /// Hodnota `CanIncludeInClipboardHistory` (DWORD), ak je pritomna.
    pub can_include_history: Option<u32>,
    /// Hodnota `CanUploadToCloudClipboard` (DWORD), ak je pritomna.
    pub can_upload: Option<u32>,
    pub owner_exe: Option<String>,
    /// Zmena pochadza z nasho vlastneho zapisu (obnova polozky).
    pub own_write: bool,
    pub text: Option<String>,
    pub image_png: Option<Vec<u8>>,
    pub image_dims: (u32, u32),
    pub files: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    Disabled,
    Paused,
    ReadOnly,
    OwnWrite,
    /// Aplikacia oznacila obsah ako citlivy (vylucenie zo spracovania / historie / cloudu alebo "Clipboard Viewer Ignore").
    Sensitive,
    ExcludedApp,
    /// Vsetky pritomne formaty su vypnute v nastaveniach.
    FormatOff,
    TooLarge,
    Empty,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Stored(String),
    /// Rovnaky obsah uz je v historii: posunul sa navrch.
    Duplicate(String),
    Skipped(Skip),
}

/// Obsah polozky na obnovenie do schranky.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Text(String),
    Image(Vec<u8>),
    Files(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Recovered,
    Unusable(Unusable),
}

pub struct History {
    dir: PathBuf,
    cfg: ClipboardCfg,
    entries: Vec<Entry>,
    pub status: Status,
    purge_bak: std::cell::Cell<bool>,
}

fn kind_str(k: ClipKind) -> &'static str {
    match k {
        ClipKind::Text => "text",
        ClipKind::Image => "image",
        ClipKind::Files => "files",
    }
}

fn entry_to_value(e: &Entry) -> Value {
    json!({"id": e.id, "kind": kind_str(e.kind), "added": e.added, "pinned": e.pinned, "app": e.app, "preview": e.preview,
           "search": e.search, "size": e.size, "hash": e.hash, "files": e.files, "w": e.dims.0, "h": e.dims.1})
}

fn is_hash(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn entry_from_value(v: &Value) -> Option<Entry> {
    let o = v.as_object()?;
    let id = o.get("id")?.as_str()?;
    if !crate::names::is_id(id) {
        return None;
    }
    let kind = match o.get("kind")?.as_str()? {
        "text" => ClipKind::Text,
        "image" => ClipKind::Image,
        "files" => ClipKind::Files,
        _ => return None,
    };
    let hash = o
        .get("hash")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let files: Vec<String> = o
        .get("files")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|p| check_path(p).is_ok())
                .take(MAX_FILES_PER_ENTRY)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    match kind {
        ClipKind::Text | ClipKind::Image if !is_hash(&hash) => return None,
        ClipKind::Files if files.is_empty() => return None,
        _ => {}
    }
    let num = |k: &str| o.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(Entry {
        id: id.to_string(),
        kind,
        added: o.get("added").and_then(Value::as_i64).unwrap_or(0),
        pinned: o.get("pinned").and_then(Value::as_bool).unwrap_or(false),
        app: clean_display(o.get("app").and_then(Value::as_str).unwrap_or(""), 64),
        preview: clean_display(o.get("preview").and_then(Value::as_str).unwrap_or(""), 120),
        search: clean_display(o.get("search").and_then(Value::as_str).unwrap_or(""), 256)
            .to_lowercase(),
        size: num("size"),
        hash,
        files,
        dims: (
            num("w").min(u64::from(u32::MAX)) as u32,
            num("h").min(u64::from(u32::MAX)) as u32,
        ),
    })
}

impl History {
    pub fn open(dir: PathBuf, cfg: ClipboardCfg) -> History {
        let (entries, status) = match load_index(&dir.join("index.json"), FORMAT, VERSION) {
            Loaded::Missing => (Vec::new(), Status::Ok),
            Loaded::Ok(v) => (Self::parse(&v), Status::Ok),
            Loaded::Recovered(v) => (Self::parse(&v), Status::Recovered),
            Loaded::Unusable(u) => (Vec::new(), Status::Unusable(u)),
        };
        let h = History {
            dir,
            cfg,
            entries,
            status,
            purge_bak: std::cell::Cell::new(false),
        };
        h.sweep_orphans();
        h
    }

    /// Osirotene subory v `blobs\` (docasne `*.tmp` po padu, obsah bez zaznamu po zlyhanom ulozeni): zmazu sa pri otvoreni.
    /// Dotyka sa len suborov s nasim tvarom nazvu (`<32 hex>.txt|png` alebo `*.tmp`).
    fn sweep_orphans(&self) {
        if self.read_only() {
            return;
        }
        let Ok(rd) = std::fs::read_dir(self.blobs()) else {
            return;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let ours_blob = n
                .rsplit_once('.')
                .is_some_and(|(stem, ext)| is_hash(stem) && matches!(ext, "txt" | "png"));
            let referenced = self.entries.iter().any(|x| {
                self.blob_path(x)
                    .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
                    == Some(n.clone())
            });
            let tmp = n.ends_with(".tmp");
            if e.file_type().is_ok_and(|t| t.is_file()) && (tmp || (ours_blob && !referenced)) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    fn parse(v: &Value) -> Vec<Entry> {
        v.get("entries")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(MAX_LOADED)
                    .filter_map(entry_from_value)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_cfg(&mut self, cfg: ClipboardCfg) {
        self.cfg = cfg;
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn read_only(&self) -> bool {
        matches!(self.status, Status::Unusable(_))
    }

    fn blobs(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    fn blob_path(&self, e: &Entry) -> Option<PathBuf> {
        let ext = match e.kind {
            ClipKind::Text => "txt",
            ClipKind::Image => "png",
            ClipKind::Files => return None,
        };
        is_hash(&e.hash).then(|| self.blobs().join(format!("{}.{ext}", e.hash)))
    }

    /// Je obsah oznaceny ako citlivy? (dobrovolne priznaky aplikacii; nezachyti kazde heslo)
    fn sensitive(s: &Snapshot) -> bool {
        const IGNORED: [&str; 2] = [
            "excludeclipboardcontentfrommonitorprocessing",
            "clipboard viewer ignore",
        ];
        s.formats
            .iter()
            .any(|f| IGNORED.contains(&f.to_ascii_lowercase().as_str()))
            || s.can_include_history == Some(0)
            || s.can_upload == Some(0)
    }

    /// Rozhodne a prip. ulozi zmenu schranky. Cita sa len to, co `Snapshot` obsahuje.
    pub fn capture(&mut self, s: &Snapshot, now: i64) -> Outcome {
        if !self.cfg.enabled {
            return Outcome::Skipped(Skip::Disabled);
        }
        if self.cfg.paused {
            return Outcome::Skipped(Skip::Paused);
        }
        if self.read_only() {
            return Outcome::Skipped(Skip::ReadOnly);
        }
        if s.own_write {
            return Outcome::Skipped(Skip::OwnWrite);
        }
        if Self::sensitive(s) {
            return Outcome::Skipped(Skip::Sensitive);
        }
        let app = s.owner_exe.clone().unwrap_or_default();
        // vylucene aplikacie: ak je zoznam neprazdny a vlastnika sa nepodarilo zistit, nezachytava sa (nie je ako overit vylucenie)
        if (!app.is_empty()
            && self
                .cfg
                .excluded_apps
                .iter()
                .any(|x| x.eq_ignore_ascii_case(&app)))
            || (app.is_empty() && !self.cfg.excluded_apps.is_empty())
        {
            return Outcome::Skipped(Skip::ExcludedApp);
        }
        let (kind, files, hash, size, preview, search, dims, bytes);
        let valid_files: Vec<String> = s
            .files
            .as_ref()
            .map(|f| {
                f.iter()
                    .filter(|p| check_path(p).is_ok())
                    .take(MAX_FILES_PER_ENTRY)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if self.cfg.files && !valid_files.is_empty() {
            kind = ClipKind::Files;
            hash = content_hash(valid_files.join("\n").to_lowercase().as_bytes());
            size = valid_files.len() as u64;
            let names: Vec<String> = valid_files
                .iter()
                .take(3)
                .map(|p| file_name_of(p))
                .collect();
            preview = clean_display(&names.join(", "), 120);
            search = clean_display(
                &valid_files
                    .iter()
                    .map(|p| file_name_of(p))
                    .collect::<Vec<_>>()
                    .join(" "),
                256,
            )
            .to_lowercase();
            files = valid_files;
            dims = (0, 0);
            bytes = None;
        } else if let (true, Some(png)) = (self.cfg.images, s.image_png.as_ref()) {
            let (w, h) = s.image_dims;
            if png.is_empty() || w == 0 || h == 0 {
                return Outcome::Skipped(Skip::Empty);
            }
            if w > MAX_IMAGE_SIDE
                || h > MAX_IMAGE_SIDE
                || png.len() as u64 > u64::from(self.cfg.max_image_mb) * 1024 * 1024
            {
                return Outcome::Skipped(Skip::TooLarge);
            }
            kind = ClipKind::Image;
            hash = content_hash(png);
            size = png.len() as u64;
            preview = format!("{w} × {h}");
            search = String::new();
            files = Vec::new();
            dims = (w, h);
            bytes = Some(png.clone());
        } else if let (true, Some(text)) = (self.cfg.text, s.text.as_ref()) {
            if text.trim().is_empty() {
                return Outcome::Skipped(Skip::Empty);
            }
            if text.len() as u64 > u64::from(self.cfg.max_text_kb) * 1024 {
                return Outcome::Skipped(Skip::TooLarge);
            }
            kind = ClipKind::Text;
            hash = content_hash(text.as_bytes());
            size = text.len() as u64;
            preview = clean_display(text, 120);
            search = clean_display(text, 256).to_lowercase();
            files = Vec::new();
            dims = (0, 0);
            bytes = Some(text.as_bytes().to_vec());
        } else if s.text.is_some() || s.image_png.is_some() || s.files.is_some() {
            return Outcome::Skipped(Skip::FormatOff);
        } else {
            return Outcome::Skipped(Skip::Empty);
        }
        // duplicita: rovnaky obsah sa posunie navrch (cas sa obnovi)
        if let Some(p) = self
            .entries
            .iter()
            .position(|e| e.kind == kind && e.hash == hash)
        {
            let mut e = self.entries.remove(p);
            e.added = now;
            let id = e.id.clone();
            self.entries.insert(0, e);
            let _ = self.save();
            return Outcome::Duplicate(id);
        }
        // miesto: pripnute sa nevytlacaju; ak je plno pripnutych, nic sa neulozi
        while self.entries.len() >= self.cfg.max_items as usize {
            let Some(pos) = self.entries.iter().rposition(|e| !e.pinned) else {
                return Outcome::Skipped(Skip::Full);
            };
            self.drop_at(pos);
        }
        let entry = Entry {
            id: new_id(),
            kind,
            added: now,
            pinned: false,
            app: clean_display(&app, 64),
            preview,
            search,
            size,
            hash,
            files,
            dims,
        };
        if let (Some(b), Some(path)) = (bytes, self.blob_path(&entry)) {
            if std::fs::create_dir_all(self.blobs()).is_err()
                || transom_config::write_atomic(&path, &b).is_err()
            {
                return Outcome::Skipped(Skip::ReadOnly);
            }
        }
        let id = entry.id.clone();
        self.entries.insert(0, entry);
        let _ = self.save();
        Outcome::Stored(id)
    }

    fn drop_at(&mut self, pos: usize) {
        self.purge_bak.set(true);
        let e = self.entries.remove(pos);
        self.remove_blob_if_unused(&e);
    }

    fn remove_blob_if_unused(&self, e: &Entry) {
        if let Some(p) = self.blob_path(e) {
            if !self
                .entries
                .iter()
                .any(|o| o.hash == e.hash && o.kind == e.kind)
            {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    /// Retencia (nepripnute) a limit poctu.
    pub fn enforce(&mut self, now: i64) -> usize {
        if self.read_only() {
            return 0;
        }
        let before = self.entries.len();
        if self.cfg.retention_days == 0 {
            // 0 = ponechat, kym ich pouzivatel nezmaze (zostane len limit poctu)
            return 0;
        }
        let limit = now - i64::from(self.cfg.retention_days) * 86_400;
        let old: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.pinned && e.added < limit)
            .map(|(p, _)| p)
            .collect();
        for p in old.into_iter().rev() {
            self.drop_at(p);
        }
        while self.entries.len() > self.cfg.max_items as usize {
            let Some(pos) = self.entries.iter().rposition(|e| !e.pinned) else {
                break;
            };
            self.drop_at(pos);
        }
        let n = before - self.entries.len();
        if n > 0 {
            let _ = self.save();
        }
        n
    }

    pub fn toggle_pin(&mut self, id: &str) -> bool {
        if self.read_only() {
            return false;
        }
        let Some(e) = self.entries.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        e.pinned = !e.pinned;
        let _ = self.save();
        true
    }

    pub fn delete(&mut self, id: &str) -> bool {
        if self.read_only() {
            return false;
        }
        let Some(p) = self.entries.iter().position(|e| e.id == id) else {
            return false;
        };
        self.drop_at(p);
        let _ = self.save();
        crate::store::purge_backup(&self.dir.join("index.json"));
        true
    }

    /// Vymaze historiu (volitelne bez pripnutych); maze len vlastne subory v `blobs\`.
    pub fn clear(&mut self, keep_pinned: bool) -> usize {
        if self.read_only() {
            return 0;
        }
        let ids: Vec<String> = self
            .entries
            .iter()
            .filter(|e| !(keep_pinned && e.pinned))
            .map(|e| e.id.clone())
            .collect();
        for id in &ids {
            if let Some(p) = self.entries.iter().position(|e| &e.id == id) {
                self.drop_at(p);
            }
        }
        let _ = self.save();
        crate::store::purge_backup(&self.dir.join("index.json"));
        ids.len()
    }

    /// Hladanie podla textu (nahlad, nazvy suborov, aplikacia); prazdny dotaz = vsetko.
    pub fn search(&self, q: &str) -> Vec<&Entry> {
        let q = q.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                q.is_empty()
                    || e.search.contains(&q)
                    || e.preview.to_lowercase().contains(&q)
                    || e.app.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Presny ulozeny obsah na obnovenie do schranky (neuplny zaznam sa neobnovi).
    pub fn payload(&self, id: &str) -> Option<Payload> {
        let e = self.entries.iter().find(|e| e.id == id)?;
        match e.kind {
            ClipKind::Files => Some(Payload::Files(e.files.clone())),
            ClipKind::Text => {
                let b = read_blob(&self.blob_path(e)?, e.size)?;
                Some(Payload::Text(String::from_utf8(b).ok()?))
            }
            ClipKind::Image => Some(Payload::Image(read_blob(&self.blob_path(e)?, e.size)?)),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        if self.read_only() {
            return Err(std::io::Error::other("ulozisko sa nepouziva"));
        }
        let mut m = Map::new();
        m.insert(
            "entries".into(),
            Value::Array(self.entries.iter().map(entry_to_value).collect()),
        );
        let r = save_index(&self.dir.join("index.json"), FORMAT, VERSION, m);
        if self.purge_bak.replace(false) {
            crate::store::purge_backup(&self.dir.join("index.json"));
        }
        r
    }
}

/// Nacita blob presne o ocakavanej velkosti (inak sa povazuje za poskodeny / podvrhnuty).
fn read_blob(p: &Path, size: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.take(size + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 == size).then_some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::temp_dir;

    fn cfg_on() -> ClipboardCfg {
        let mut c = transom_config::Config::default().tools.clipboard;
        c.enabled = true;
        c
    }

    fn text(t: &str) -> Snapshot {
        Snapshot {
            text: Some(t.into()),
            ..Snapshot::default()
        }
    }

    fn open(name: &str, c: ClipboardCfg) -> (History, PathBuf) {
        let d = temp_dir(name);
        (History::open(d.clone(), c), d)
    }

    #[test]
    fn nothing_is_captured_until_it_is_explicitly_enabled_and_pausing_stops_capture() {
        let d = temp_dir("clip-off");
        let mut off = History::open(d.clone(), transom_config::Config::default().tools.clipboard);
        assert_eq!(
            off.capture(&text("tajne"), 1),
            Outcome::Skipped(Skip::Disabled)
        );
        assert!(off.entries().is_empty());
        assert!(
            !d.join("index.json").exists() && !d.join("blobs").exists(),
            "nic sa nezapisalo na disk"
        );
        let mut c = cfg_on();
        c.paused = true;
        let mut p = History::open(d.clone(), c);
        assert_eq!(p.capture(&text("x"), 1), Outcome::Skipped(Skip::Paused));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn text_is_stored_searched_restored_exactly_and_survives_a_restart() {
        let (mut h, d) = open("clip-text", cfg_on());
        let Outcome::Stored(id) = h.capture(&text("Ahoj svet\nDruhy riadok"), 10) else {
            panic!()
        };
        assert_eq!(h.entries()[0].preview, "Ahoj svet Druhy riadok");
        assert_eq!(h.search("DRUHY").len(), 1);
        assert_eq!(h.search("neexistuje").len(), 0);
        assert_eq!(
            h.payload(&id),
            Some(Payload::Text("Ahoj svet\nDruhy riadok".into()))
        );
        let h2 = History::open(d.clone(), cfg_on());
        assert_eq!(h2.entries(), h.entries());
        assert_eq!(
            h2.payload(&id),
            Some(Payload::Text("Ahoj svet\nDruhy riadok".into()))
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn sensitive_formats_own_writes_excluded_apps_and_disabled_formats_are_skipped() {
        let (mut h, d) = open("clip-sens", {
            let mut c = cfg_on();
            c.excluded_apps = vec!["KeePass.exe".into()];
            c
        });
        let base = text("heslo123");
        for (name, snap) in [
            (
                "monitor",
                Snapshot {
                    formats: vec!["ExcludeClipboardContentFromMonitorProcessing".into()],
                    ..base.clone()
                },
            ),
            (
                "viewer",
                Snapshot {
                    formats: vec!["clipboard viewer IGNORE".into()],
                    ..base.clone()
                },
            ),
            (
                "history0",
                Snapshot {
                    can_include_history: Some(0),
                    ..base.clone()
                },
            ),
            (
                "cloud0",
                Snapshot {
                    can_upload: Some(0),
                    ..base.clone()
                },
            ),
        ] {
            assert_eq!(
                h.capture(&snap, 1),
                Outcome::Skipped(Skip::Sensitive),
                "{name}"
            );
        }
        // dovoleny priznak (1) obsah nevylucuje
        assert!(matches!(
            h.capture(
                &Snapshot {
                    can_include_history: Some(1),
                    owner_exe: Some("notepad.exe".into()),
                    ..text("ok")
                },
                2
            ),
            Outcome::Stored(_)
        ));
        assert_eq!(
            h.capture(
                &Snapshot {
                    owner_exe: Some("keepass.EXE".into()),
                    ..base.clone()
                },
                3
            ),
            Outcome::Skipped(Skip::ExcludedApp)
        );
        assert_eq!(
            h.capture(
                &Snapshot {
                    own_write: true,
                    ..base.clone()
                },
                4
            ),
            Outcome::Skipped(Skip::OwnWrite)
        );
        // zdroj nie je znamy (null vlastnik) a existuje zoznam vylucenych aplikacii: fail-closed, nezachyti sa
        assert_eq!(
            h.capture(
                &Snapshot {
                    owner_exe: None,
                    ..text("bez zdroja")
                },
                5
            ),
            Outcome::Skipped(Skip::ExcludedApp)
        );
        assert_eq!(h.entries().len(), 1);
        // vypnuty format
        let mut c = cfg_on();
        c.text = false;
        let mut t = History::open(d.clone(), c);
        assert_eq!(t.capture(&text("x"), 6), Outcome::Skipped(Skip::FormatOff));
        assert_eq!(
            t.capture(&Snapshot::default(), 7),
            Outcome::Skipped(Skip::Empty)
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn size_limits_and_empty_content_are_refused_before_anything_is_written() {
        let mut c = cfg_on();
        c.max_text_kb = 1;
        c.max_image_mb = 1;
        let (mut h, d) = open("clip-size", c);
        assert_eq!(
            h.capture(&text(&"x".repeat(1025)), 1),
            Outcome::Skipped(Skip::TooLarge)
        );
        assert_eq!(
            h.capture(&text("   \n\t"), 1),
            Outcome::Skipped(Skip::Empty)
        );
        let big = Snapshot {
            image_png: Some(vec![1; 1024 * 1024 + 1]),
            image_dims: (10, 10),
            ..Snapshot::default()
        };
        assert_eq!(h.capture(&big, 1), Outcome::Skipped(Skip::TooLarge));
        let wide = Snapshot {
            image_png: Some(vec![1; 10]),
            image_dims: (9000, 10),
            ..Snapshot::default()
        };
        assert_eq!(h.capture(&wide, 1), Outcome::Skipped(Skip::TooLarge));
        let zero = Snapshot {
            image_png: Some(vec![1; 10]),
            image_dims: (0, 10),
            ..Snapshot::default()
        };
        assert_eq!(h.capture(&zero, 1), Outcome::Skipped(Skip::Empty));
        assert!(h.entries().is_empty());
        assert!(!d.join("blobs").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn images_and_file_lists_are_stored_with_valid_paths_only_and_duplicates_move_to_the_top() {
        let (mut h, d) = open("clip-img", cfg_on());
        let img = Snapshot {
            image_png: Some(b"\x89PNG....".to_vec()),
            image_dims: (640, 480),
            ..Snapshot::default()
        };
        let Outcome::Stored(i1) = h.capture(&img, 1) else {
            panic!()
        };
        assert_eq!(h.entries()[0].preview, "640 × 480");
        assert_eq!(
            h.payload(&i1),
            Some(Payload::Image(b"\x89PNG....".to_vec()))
        );
        let files = Snapshot {
            files: Some(vec![
                r"C:\a\b.txt".into(),
                r"rel\x".into(),
                r"\\.\PhysicalDrive0".into(),
                r"C:\c\d.pdf".into(),
            ]),
            ..Snapshot::default()
        };
        let Outcome::Stored(i2) = h.capture(&files, 2) else {
            panic!()
        };
        let e = h.entries().iter().find(|e| e.id == i2).unwrap();
        assert_eq!(e.files, vec![r"C:\a\b.txt", r"C:\c\d.pdf"]);
        assert_eq!(
            h.payload(&i2),
            Some(Payload::Files(vec![
                r"C:\a\b.txt".into(),
                r"C:\c\d.pdf".into()
            ]))
        );
        // rovnaky obrazok znova: duplicita, posunie sa navrch
        assert_eq!(h.capture(&img, 3), Outcome::Duplicate(i1.clone()));
        assert_eq!(h.entries()[0].id, i1);
        assert_eq!(h.entries().len(), 2);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn limits_retention_pinning_deleting_and_clearing_remove_only_unused_blobs() {
        let mut c = cfg_on();
        c.max_items = 3;
        c.retention_days = 1;
        let (mut h, d) = open("clip-limits", c);
        let mut ids = Vec::new();
        for (i, t) in ["a", "b", "c", "d"].iter().enumerate() {
            let Outcome::Stored(id) = h.capture(&text(t), 100 + i as i64) else {
                panic!()
            };
            ids.push(id);
        }
        assert_eq!(h.entries().len(), 3, "najstarsi vytlaceny");
        assert!(h.payload(&ids[0]).is_none());
        let blobs = |d: &Path| {
            std::fs::read_dir(d.join("blobs"))
                .map(|r| r.count())
                .unwrap_or(0)
        };
        assert_eq!(blobs(&d), 3, "blob vytlaceneho zaznamu sa zmazal");
        assert!(h.toggle_pin(&ids[1]));
        // retencia: vsetko nepripnute starsie ako 1 den
        assert_eq!(h.enforce(100 + 2 * 86_400), 2);
        assert_eq!(h.entries().len(), 1);
        assert_eq!(h.entries()[0].id, ids[1]);
        assert_eq!(blobs(&d), 1);
        assert_eq!(h.clear(true), 0, "pripnute ostava");
        assert!(h.delete(&ids[1]));
        assert_eq!(blobs(&d), 0);
        assert!(!h.delete(&ids[1]));
        // plno pripnutych: nova polozka sa neulozi
        let mut full = cfg_on();
        full.max_items = 1;
        let mut f = History::open(temp_dir("clip-full"), full);
        let Outcome::Stored(p) = f.capture(&text("p"), 1) else {
            panic!()
        };
        f.toggle_pin(&p);
        assert_eq!(f.capture(&text("q"), 2), Outcome::Skipped(Skip::Full));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_shared_blob_survives_until_the_last_entry_using_it_is_gone() {
        // dva zaznamy toho isteho obsahu sa neukladaju (duplicita), ale text a obrazok s rovnakymi bajtmi maju rozne pripony
        let (mut h, d) = open("clip-shared", cfg_on());
        h.capture(&text("abc"), 1);
        h.capture(
            &Snapshot {
                image_png: Some(b"abc".to_vec()),
                image_dims: (1, 1),
                ..Snapshot::default()
            },
            2,
        );
        assert_eq!(h.entries().len(), 2);
        let first = h.entries()[1].id.clone();
        h.delete(&first);
        assert_eq!(std::fs::read_dir(d.join("blobs")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_tampered_blob_or_index_is_not_restored_and_a_newer_index_is_never_replaced() {
        let (mut h, d) = open("clip-tamper", cfg_on());
        let Outcome::Stored(id) = h.capture(&text("original"), 1) else {
            panic!()
        };
        let blob = std::fs::read_dir(d.join("blobs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(&blob, b"podvrhnuty obsah s inou dlzkou").unwrap();
        assert_eq!(h.payload(&id), None, "velkost nesedi: neobnovi sa");
        // podvrhnuty index: zle id, zly hash, zly format
        std::fs::write(
            d.join("index.json"),
            r#"{"format":"transom-tools-clipboard","version":1,"entries":[
              {"id":"../x","kind":"text","hash":"00000000000000000000000000000000"},
              {"id":"0123456789abcdef","kind":"text","hash":"..\\..\\evil"},
              {"id":"0123456789abcdee","kind":"files","files":["rel"]},
              {"id":"0123456789abcded","kind":"text","hash":"0123456789abcdef0123456789abcdef","preview":"a\u202Eb"}]}"#,
        )
        .unwrap();
        let h2 = History::open(d.clone(), cfg_on());
        assert_eq!(h2.entries().len(), 1);
        assert_eq!(h2.entries()[0].preview, "ab");
        std::fs::write(
            d.join("index.json"),
            br#"{"format":"transom-tools-clipboard","version":7}"#,
        )
        .unwrap();
        let mut h3 = History::open(d.clone(), cfg_on());
        assert!(matches!(h3.status, Status::Unusable(Unusable::Newer(7))));
        assert_eq!(h3.capture(&text("x"), 5), Outcome::Skipped(Skip::ReadOnly));
        assert!(std::fs::read_to_string(d.join("index.json"))
            .unwrap()
            .contains("\"version\":7"));
        let _ = std::fs::remove_dir_all(d);
    }
}
