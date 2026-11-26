//! Kniznica snimok obrazovky v aplikacii: spaja citaciu logiku (`transom_tools::library`), sledovanie priecinkov a dekodovanie
//! miniatur z platformy a poskytuje panelu Nastroje hotove riadky. Vsetko drahe (vymenovanie priecinkov, sledovanie, dekodovanie
//! obrazkov) bezi na vlastnych vlaknach; UI vlakno len nahliada do hotovych vysledkov.
//!
//! * Priecinky: predvoleny `FOLDERID_Screenshots` (zistuje sa pri kazdom otvoreni, takze sleduje presmerovanie) a priecinky z
//!   nastaveni; panel ich nikdy nezapisuje ani nemeni subory v nich.
//! * Zivotnost: sledovanie a nacitavanie bezi len kym je karta Snimky otvorena; po zatvoreni sa uvolnia uchytky priecinkov a
//!   zahodia sa miniatury (v pamati zostane len posledny zoznam nazvov).
//! * Miniatury: lenive (len viditelne riadky), cache v pamati ohranicena na 24 MiB, kluc (cesta, cas zmeny, velkost, rozmer)
//!   sama zneplatni zmenene subory aj zmenu DPI; cloud a sietove subory sa nedekoduju.
//!
//! Cesty a nazvy suborov sa nelogu.
use crate::tools_bridge::size_text;
use crate::tools_pop::{Row, SectionView, Thumb};
use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;
use transom_config::ShotsCfg;
use transom_core::loader::{Bitmap, Decode, Loader, LoaderCfg, Lookup};
use transom_i18n::{t, tf};
use transom_tools::library::{plan_dirs, Change, Deps, Entry, FsLister, Library, WatchFactory};
use transom_tools::names::clean_display;

/// Najviac miniatur v pamati (bajty RGBA).
pub const THUMB_BUDGET: usize = 24 * 1024 * 1024;
/// Najviac naraz cakajucich miniatur (viditelne riadky + rezerva).
const MAX_QUEUE: usize = 24;
/// Najviac riadkov v paneli (rovnako ako ostatne sekcie).
const MAX_ROWS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: String,
    pub modified: i64,
    pub bytes: u64,
    pub w: u16,
    pub h: u16,
}

impl ThumbKey {
    fn of(e: &Entry, box_px: (u32, u32)) -> ThumbKey {
        ThumbKey {
            path: e.path.clone(),
            modified: e.modified,
            bytes: e.bytes,
            w: box_px.0.min(1024) as u16,
            h: box_px.1.min(1024) as u16,
        }
    }
}

struct ComGuard(bool);

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: parter k uspesnemu `CoInitializeEx` na tomto vlakne.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

struct ThumbDecoder;

impl Decode<ThumbKey> for ThumbDecoder {
    fn init(&self) -> Box<dyn Any> {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        // SAFETY: COM (MTA) pre pracovne vlakno dekodovania, kym vlakno zije.
        Box::new(ComGuard(
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok(),
        ))
    }

    fn decode(&self, k: &ThumbKey) -> Option<Bitmap> {
        let t = transom_platform::thumb::decode(&k.path, u32::from(k.w), u32::from(k.h)).ok()?;
        Bitmap::new(t.w, t.h, t.rgba)
    }
}

/// Sledovanie priecinkov cez platformu (`ReadDirectoryChangesW`).
struct PlatformWatcher;

impl WatchFactory for PlatformWatcher {
    fn watch(
        &self,
        dirs: &[String],
        notify: Arc<dyn Fn(Change) + Send + Sync>,
    ) -> Box<dyn Any + Send> {
        use transom_platform::dirwatch::Change as C;
        transom_platform::dirwatch::watch_boxed(
            dirs,
            Arc::new(move |c| {
                notify(match c {
                    C::Changed => Change::Changed,
                    C::Overflow => Change::Overflow,
                    C::Unavailable => Change::Unavailable,
                })
            }),
        )
    }
}

pub struct LibraryRt {
    lib: Library,
    thumbs: Loader<ThumbKey>,
    open: bool,
    /// Cesty riadkov z posledneho `section` (index riadku v paneli -> subor).
    painted: Vec<String>,
    /// Pocet nacitani zoznamu, pri ktorom sa naposledy upratala cache miniatur.
    tidied: u64,
}

impl LibraryRt {
    /// `wake` sa zavola z pracovnych vlakien po kazdej zmene zoznamu alebo miniatury (zluci ho volajuci: jedna sprava).
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>) -> LibraryRt {
        let deps = Deps::new(Arc::new(FsLister), Arc::new(PlatformWatcher), wake.clone());
        LibraryRt {
            lib: Library::start(deps),
            thumbs: Loader::start(
                LoaderCfg {
                    name: "transom-thumbs",
                    threads: 1,
                    budget_bytes: THUMB_BUDGET,
                    max_queue: MAX_QUEUE,
                    max_failed: 512,
                },
                Arc::new(ThumbDecoder),
                wake,
            ),
            open: false,
            painted: Vec::new(),
            tidied: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Arc<transom_tools::library::Snapshot> {
        self.lib.snapshot()
    }

    /// Karta Snimky je viditelna: zisti priecinky, zacne ich sledovat a nacita zoznam.
    pub fn open(&mut self, cfg: &ShotsCfg) {
        let default = transom_platform::shellops::screenshots_folder();
        let dirs = plan_dirs(default.as_deref(), cfg);
        self.thumbs.forget_failed();
        self.lib.open(dirs, cfg.max_items as usize);
        self.open = true;
    }

    /// Karta sa zatvorila: sledovanie sa ukonci (uvolnia sa uchytky priecinkov) a miniatury sa zahodia.
    pub fn close(&mut self) {
        if self.open {
            self.lib.close();
            self.open = false;
        }
        self.thumbs.want(&[]);
        self.thumbs.clear();
        self.painted.clear();
    }

    pub fn refresh(&self) {
        self.lib.refresh();
    }

    /// Subor riadku `i` z posledneho kreslenia (tak, ako ho pouzivatel videl, aj ked sa zoznam medzitym obnovil).
    pub fn entry(&self, i: usize) -> Option<Entry> {
        let path = self.painted.get(i)?;
        self.lib
            .snapshot()
            .items
            .iter()
            .find(|e| &e.path == path)
            .cloned()
    }

    /// Riadky a stavove texty pre panel; zaroven zapamata cesty riadkov a upraci cache po novom nacitani zoznamu.
    pub fn section(&mut self, cfg: &ShotsCfg, box_px: (u32, u32)) -> SectionView {
        let snap = self.lib.snapshot();
        if snap.generation != self.tidied {
            self.tidied = snap.generation;
            // zmenene a zmazane subory (a ine rozmery po zmene DPI) opustaju cache hned po novom nacitani zoznamu
            let live: HashSet<(String, i64, u64, u16, u16)> = snap
                .items
                .iter()
                .map(|e| {
                    let k = ThumbKey::of(e, box_px);
                    (k.path, k.modified, k.bytes, k.w, k.h)
                })
                .collect();
            self.thumbs
                .retain(|k| live.contains(&(k.path.clone(), k.modified, k.bytes, k.w, k.h)));
        }
        let rows: Vec<Row> = snap
            .items
            .iter()
            .take(MAX_ROWS)
            .map(|e| {
                let thumb = if e.cloud || e.remote {
                    Thumb::Placeholder
                } else {
                    match self.thumbs.get(&ThumbKey::of(e, box_px)) {
                        Lookup::Ready(b) => Thumb::Ready(b),
                        Lookup::Failed => Thumb::Placeholder,
                        Lookup::Pending | Lookup::Absent => Thumb::Loading,
                    }
                };
                let mut sub = if e.modified > 0 {
                    transom_platform::clock::short_datetime(e.modified)
                } else {
                    String::new()
                };
                if e.cloud {
                    sub = format!("{sub} · {}", t("tools.lib_cloud"));
                } else if e.remote {
                    sub = format!("{sub} · {}", t("tools.lib_nopreview"));
                }
                Row {
                    id: e.path.clone(),
                    title: clean_display(&e.name, 120),
                    subtitle: sub,
                    value: size_text(e.bytes),
                    thumb,
                    ..Row::default()
                }
            })
            .collect();
        self.painted = snap
            .items
            .iter()
            .take(MAX_ROWS)
            .map(|e| e.path.clone())
            .collect();
        let (shown, total) = (snap.items.len(), snap.total);
        let status = if !cfg.enabled {
            None
        } else if snap.scanning && snap.items.is_empty() {
            Some(t("tools.lib_loading"))
        } else if snap.problems() > 0 {
            Some(tf("tools.lib_problem", &[&snap.problems().to_string()]))
        } else if snap.truncated() {
            Some(tf(
                "tools.lib_trunc",
                &[&shown.to_string(), &total.to_string()],
            ))
        } else {
            Some(t("tools.lib_readonly"))
        };
        let empty = if !cfg.enabled {
            Some(t("tools.lib_off"))
        } else if snap.items.is_empty() && (snap.scanning || !snap.open) {
            Some(t("tools.lib_loading"))
        } else if snap.dirs.is_empty() {
            Some(t("tools.lib_nofolders"))
        } else {
            None
        };
        SectionView {
            rows,
            enabled: cfg.enabled,
            status,
            empty,
        }
    }

    /// Viditelne riadky bez miniatury (z `ToolsPanel::take_wanted`): zacnu sa nacitavat; zastarane poziadavky sa zahodia.
    pub fn request(&self, wanted: &[usize], box_px: (u32, u32)) {
        let snap = self.lib.snapshot();
        let keys: Vec<ThumbKey> = wanted
            .iter()
            .filter_map(|i| {
                let p = self.painted.get(*i)?;
                let e = snap.items.iter().find(|e| &e.path == p)?;
                (!e.cloud && !e.remote).then(|| ThumbKey::of(e, box_px))
            })
            .collect();
        self.thumbs.want(&keys);
    }

    #[cfg(test)]
    pub fn cached_thumbs(&self) -> usize {
        self.thumbs.cached_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant, UNIX_EPOCH};

    fn wait(mut f: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(10) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut bgra = Vec::new();
        for _ in 0..w * h {
            bgra.extend_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
        }
        transom_platform::imaging::encode_png(w, h, &bgra).unwrap()
    }

    fn put(dir: &std::path::Path, name: &str, secs: u64, body: &[u8]) {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    /// Cele zapojenie na skutocnom disku, ale iba v scratch priecinku (skutocny priecinok snimok sa nikdy nenacita: predvoleny
    /// priecinok je vypnuty).
    #[test]
    fn the_library_lists_newest_first_loads_lazy_thumbnails_follows_changes_and_releases_the_folder_on_close(
    ) {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        // SAFETY: COM pre testovacie vlakno (kodovanie PNG).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let root = std::env::temp_dir().join(format!("transom-libtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shots = root.join("Shots");
        std::fs::create_dir_all(&shots).unwrap();
        put(
            &shots,
            "old.png",
            1_700_000_000,
            &png(200, 100, [220, 0, 0]),
        );
        put(
            &shots,
            "mid.png",
            1_700_000_100,
            &png(200, 100, [0, 200, 0]),
        );
        put(&shots, "notes.txt", 1_700_000_900, b"not an image");
        put(
            &shots,
            "broken.png",
            1_700_000_050,
            b"this is not a png at all",
        );

        let wakes = Arc::new(AtomicUsize::new(0));
        let w2 = wakes.clone();
        let mut rt = LibraryRt::start(Arc::new(move || {
            w2.fetch_add(1, Ordering::SeqCst);
        }));
        let mut cfg = transom_config::Config::default().tools.screenshots;
        cfg.default_folder = false;
        cfg.folders = vec![shots.to_str().unwrap().to_string()];
        let bx = (74u32, 46u32);
        assert!(!rt.is_open());
        rt.open(&cfg);
        assert!(rt.is_open());
        assert!(
            wait(|| rt.section(&cfg, bx).rows.len() == 3),
            "tri obrazky, text sa nezobrazi"
        );
        let v = rt.section(&cfg, bx);
        assert_eq!(
            v.rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["mid.png", "broken.png", "old.png"],
            "najnovsie prve"
        );
        assert!(v
            .rows
            .iter()
            .all(|r| r.thumb != Thumb::None && !r.subtitle.is_empty() && !r.value.is_empty()));
        assert!(v.enabled && v.empty.is_none() && v.status.is_some());
        // lenive nacitanie: nic sa nedekoduje, kym panel nepozada o viditelne riadky
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(rt.cached_thumbs(), 0);
        rt.request(&[0, 1, 2], bx);
        assert!(wait(|| rt.cached_thumbs() == 3));
        let v = rt.section(&cfg, bx);
        match &v.rows[0].thumb {
            Thumb::Ready(b) => assert_eq!(
                (b.w, b.h),
                (74, 37),
                "miniatura sa vojde do plochy a zachova pomer"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            v.rows[1].thumb,
            Thumb::Placeholder,
            "poskodeny subor ma zastupnu ikonu a znovu sa nedekoduje"
        );
        assert!(matches!(v.rows[2].thumb, Thumb::Ready(_)));
        // novy subor sa objavi bez pozadovania (sledovanie), novsi je prvy
        put(&shots, "new.png", 1_700_000_500, &png(80, 80, [0, 0, 220]));
        assert!(
            wait(|| rt.section(&cfg, bx).rows.len() == 4),
            "pribudol subor"
        );
        assert_eq!(rt.section(&cfg, bx).rows[0].title, "new.png");
        // zmazany subor zmizne a jeho miniatura opusti cache
        std::fs::remove_file(shots.join("old.png")).unwrap();
        assert!(
            wait(|| rt.section(&cfg, bx).rows.len() == 3),
            "subor zmizol"
        );
        assert!(wait(|| rt.cached_thumbs() <= 2));
        // vypnuta kniznica ukaze vysvetlenie, nie zoznam
        let mut off = cfg.clone();
        off.enabled = false;
        let v = rt.section(&off, bx);
        assert!(!v.enabled && v.empty.is_some() && v.status.is_none());
        // po zatvoreni sa uvolnia uchytky aj miniatury a priecinok sa da premenovat; subory ostali nedotknute
        rt.close();
        assert!(!rt.is_open());
        assert_eq!(rt.cached_thumbs(), 0, "miniatury sa zahodili");
        let moved = root.join("Moved");
        assert!(
            wait(|| std::fs::rename(&shots, &moved).is_ok()),
            "uchytka priecinka sa uvolnila"
        );
        assert!(moved.join("mid.png").is_file() && moved.join("notes.txt").is_file());
        assert!(wakes.load(Ordering::SeqCst) > 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_is_listed_for_an_empty_configuration_and_an_empty_folder_is_explained() {
        let mut rt = LibraryRt::start(Arc::new(|| {}));
        let mut cfg = transom_config::Config::default().tools.screenshots;
        cfg.default_folder = false;
        cfg.folders.clear();
        rt.open(&cfg);
        assert!(wait(|| {
            let s = rt.snapshot();
            s.open && !s.scanning
        }));
        let v = rt.section(&cfg, (74, 46));
        assert!(v.rows.is_empty() && v.empty.is_some());
        assert!(
            rt.snapshot().dirs.is_empty(),
            "bez priecinkov sa nic neskenuje"
        );
        rt.close();
    }
}
