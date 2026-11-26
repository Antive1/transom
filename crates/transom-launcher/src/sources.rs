//! Zdroje vysledkov. Kazdy zdroj je zrusitelny a nesmie panikovat (release ma `panic = "abort"`): chyba zdroja = prazdny zoznam (a, pri subory, stav
//! "index nedostupny", z ktoreho UI vysvetli obmedzenie).
use crate::engine::Cancel;
use crate::{Item, Kind, Target};
use std::sync::{Arc, RwLock};

pub trait Source: Send + Sync {
    fn name(&self) -> &'static str;
    /// Pomale zdroje (subory) bezia az po rychlych a ich vysledky pridavaju postupne.
    fn slow(&self) -> bool {
        false
    }
    fn search(&self, query: &str, cancel: &Cancel) -> Vec<Item>;
}

/// Pevny zoznam (akcie transom, nastavenia Windows, poznamky, Polica): hlada sa pri kazdom dotaze v pamati.
pub struct ListSource {
    name: &'static str,
    items: RwLock<Vec<Item>>,
}

impl ListSource {
    pub fn new(name: &'static str, items: Vec<Item>) -> Arc<ListSource> {
        Arc::new(ListSource {
            name,
            items: RwLock::new(items),
        })
    }

    pub fn set(&self, items: Vec<Item>) {
        if let Ok(mut g) = self.items.write() {
            *g = items;
        }
    }
}

impl Source for ListSource {
    fn name(&self) -> &'static str {
        self.name
    }
    fn search(&self, _query: &str, _cancel: &Cancel) -> Vec<Item> {
        self.items.read().map(|g| g.clone()).unwrap_or_default()
    }
}

/// Dostupnost indexu Windows Search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexState {
    Available,
    /// Sluzba nie je spustena / dotaz zlyhal: pouzije sa plytke prehladanie nastavenych priecinkov.
    Unavailable,
}

/// Index Windows Search nie je dostupny (sluzba nebezi, dotaz zlyhal, nepovolene).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexUnavailable;

/// Dotaz na index suborov (v platforme cez OLE DB); vracia cesty, najviac `limit`. `Err` = index nedostupny.
pub trait FileSearch: Send + Sync {
    fn query(
        &self,
        term: &str,
        limit: usize,
        cancel: &Cancel,
    ) -> Result<Vec<String>, IndexUnavailable>;
}

/// Zdroj suborov: najprv index, pri nedostupnosti plytke prehladanie `fallback` (len priamy obsah priecinka, nikdy rekurzia),
/// vzdy s vynechanim `excluded` (porovnanie bez ohladu na velkost pismen, na hranici cesty).
pub struct FilesSource {
    pub index: Arc<dyn FileSearch>,
    pub fallback: Vec<String>,
    pub excluded: Vec<String>,
    pub limit: usize,
    pub state: std::sync::atomic::AtomicU8,
}

pub const FALLBACK_MAX_ENTRIES: usize = 2000;

impl FilesSource {
    pub fn new(
        index: Arc<dyn FileSearch>,
        fallback: Vec<String>,
        excluded: Vec<String>,
        limit: usize,
    ) -> FilesSource {
        FilesSource {
            index,
            fallback,
            excluded,
            limit,
            state: std::sync::atomic::AtomicU8::new(0),
        }
    }

    pub fn index_state(&self) -> IndexState {
        if self.state.load(std::sync::atomic::Ordering::Relaxed) == 1 {
            IndexState::Unavailable
        } else {
            IndexState::Available
        }
    }
}

/// Je `path` v (alebo pod) vynechanym priecinkom?
pub fn is_excluded(path: &str, excluded: &[String]) -> bool {
    let p = path.replace('/', "\\").to_lowercase();
    excluded.iter().any(|e| {
        let e = e.replace('/', "\\").to_lowercase();
        let e = e.trim_end_matches('\\');
        !e.is_empty() && (p == e || p.starts_with(&format!("{e}\\")))
    })
}

fn item_for(path: &str, is_dir: bool) -> Item {
    let name = path.rsplit('\\').next().unwrap_or(path);
    let parent = path.rsplit_once('\\').map_or("", |(a, _)| a);
    Item::new(
        name,
        parent,
        if is_dir { Kind::Folder } else { Kind::File },
        Target::Path(path.to_string()),
    )
}

impl Source for FilesSource {
    fn name(&self) -> &'static str {
        "files"
    }
    fn slow(&self) -> bool {
        true
    }
    fn search(&self, query: &str, cancel: &Cancel) -> Vec<Item> {
        use std::sync::atomic::Ordering::Relaxed;
        let q = query.trim();
        if q.chars().count() < 2 || cancel.is_cancelled() {
            return Vec::new();
        }
        match self.index.query(q, self.limit, cancel) {
            Ok(paths) => {
                self.state.store(0, Relaxed);
                paths
                    .into_iter()
                    .filter(|p| !is_excluded(p, &self.excluded))
                    .map(|p| {
                        let dir = std::fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                        item_for(&p, dir)
                    })
                    .collect()
            }
            Err(IndexUnavailable) => {
                self.state.store(1, Relaxed);
                let mut out = Vec::new();
                for dir in &self.fallback {
                    if cancel.is_cancelled() || is_excluded(dir, &self.excluded) {
                        continue;
                    }
                    let Ok(rd) = std::fs::read_dir(dir) else {
                        continue;
                    };
                    for e in rd.flatten().take(FALLBACK_MAX_ENTRIES) {
                        if cancel.is_cancelled() {
                            return out;
                        }
                        let p = e.path().to_string_lossy().to_string();
                        if is_excluded(&p, &self.excluded) {
                            continue;
                        }
                        // len zhoda v nazve (obsah suborov sa nikdy necita)
                        let name = e.file_name().to_string_lossy().to_string();
                        if crate::rank::score(q, &name, "").is_some() {
                            let d = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            out.push(item_for(&p, d));
                        }
                    }
                }
                out
            }
        }
    }
}

/// Escapovany vyraz pre `LIKE` v SQL Windows Search: povolene su pismena, cislice, medzera a bezne znaky nazvov; apostrof sa
/// zdvoji, `%`, `_`, `[` a `]` sa odstranuju (ziadne zastupne znaky od pouzivatela), riadiace znaky a ostatne symboly sa zahodia.
/// Dlzka sa obmedzi. Vysledok nikdy neobsahuje nic, co by menilo strukturu dotazu.
pub fn sql_like_term(q: &str) -> String {
    q.chars()
        .filter(|c| {
            c.is_alphanumeric()
                || matches!(
                    c,
                    ' ' | '.' | '-' | '\'' | '(' | ')' | '&' | '+' | ',' | '_'
                )
        })
        .take(80)
        .flat_map(|c| if c == '\'' { vec!['\'', '\''] } else { vec![c] })
        .collect()
}

/// Aplikacie: zoznam odkazov z ponuky Start (nacitany raz a obnovovany na poziadanie, nie pri kazdom dotaze).
pub fn app_item(name: &str, lnk: &str) -> Item {
    Item::new(name, lnk, Kind::App, Target::Path(lnk.to_string()))
}
