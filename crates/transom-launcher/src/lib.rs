//! Vstavane rychle hladanie transom: zdroje, radenie a zrusitelne postupne hladanie na pozadi. Cisty kod bez Win32 a bez siete:
//! zdroje dodava aplikacia (zoznam aplikacii, nastavenia Windows, akcie transom, poznamky, Polica) a platforma (index Windows
//! Search cez trait `FileSearch`). Nic sa nespusta, kym pouzivatel nevyberie vysledok (spustenie robi volajuci z `Target`).
//! Dotaz sa **nikdy** nevklada do prikazoveho riadka ani do skriptu; do SQL idu len escapovane znaky (`sql_like_term`).
pub mod engine;
pub mod index;
pub mod rank;
pub mod sources;

pub use engine::{Cancel, Engine};
pub use rank::{rank, score, Hit};

/// Kategoria vysledku (zobrazuje sa ako stitok).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    App,
    File,
    Folder,
    Setting,
    Action,
    Note,
    Shelf,
}

/// Co sa pri vybere vykona. Vzdy ide o hotovy cieľ z nasho zdroja, nikdy o text dotazu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Subor, priecinok alebo odkaz (otvori sa cez shell s predvolenym slovesom).
    Path(String),
    /// Cielova stranka Nastaveni Windows (`ms-settings:...`), len z pevneho zoznamu.
    Uri(&'static str),
    /// Akcia transom podla stabilneho id.
    Action(&'static str),
    /// Poznamka podla id.
    Note(String),
    /// Polozka Police podla id.
    Shelf(String),
}

impl Target {
    /// Stabilny kluc na zlucenie duplicit.
    pub fn key(&self) -> String {
        match self {
            Target::Path(p) => format!("p:{}", p.to_lowercase()),
            Target::Uri(u) => format!("u:{u}"),
            Target::Action(a) => format!("a:{a}"),
            Target::Note(n) => format!("n:{n}"),
            Target::Shelf(s) => format!("s:{s}"),
        }
    }

    /// Cesta, ak ide o subor alebo priecinok (pre "Otvorit umiestnenie" a "Kopirovat cestu").
    pub fn path(&self) -> Option<&str> {
        match self {
            Target::Path(p) => Some(p),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    /// Druhy riadok (cesta, popis). Hlada sa v nom len ako v zhode s nizsou prioritou.
    pub detail: String,
    pub kind: Kind,
    pub target: Target,
}

impl Item {
    pub fn new(
        title: impl Into<String>,
        detail: impl Into<String>,
        kind: Kind,
        target: Target,
    ) -> Item {
        Item {
            title: title.into(),
            detail: detail.into(),
            kind,
            target,
        }
    }
}

/// Povolene sposoby pouzitia vysledku (podla druhu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Open,
    OpenContaining,
    CopyPath,
}

pub fn allowed_acts(item: &Item) -> Vec<Act> {
    let mut v = vec![Act::Open];
    // aplikacie z priecinka "Aplikacie" (`shell:AppsFolder\...`) nemaju cestu, ktoru by sa dalo ukazat alebo kopirovat
    let real_path = item.target.path().is_some_and(|p| !p.starts_with("shell:"));
    if matches!(item.kind, Kind::App | Kind::File | Kind::Folder) && real_path {
        v.push(Act::OpenContaining);
        v.push(Act::CopyPath);
    }
    v
}

/// Zdroje zapnute v konfiguracii.
pub fn enabled_sources(c: &transom_config::LauncherCfg) -> Vec<&'static str> {
    let mut v = Vec::new();
    for (on, name) in [
        (c.actions, "actions"),
        (c.apps, "apps"),
        (c.settings, "settings"),
        (c.notes, "notes"),
        (c.shelf, "shelf"),
        (c.files, "files"),
    ] {
        if on {
            v.push(name);
        }
    }
    v
}

#[cfg(test)]
mod tests;
