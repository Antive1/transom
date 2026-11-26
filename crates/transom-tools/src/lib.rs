//! Uloziste nastrojov transom (Polica, historia schranky, poznamky) a kniznica snimok obrazovky (len citanie): lokalne, sukromne.
//!
//! * `names`: ocistenie textov, kontrola ciest a nazvov, id, kluce duplicit.
//! * `store`: verzovane indexy, zaloha, ochrana pred poskodenim a novsou verziou, bezpecne mazanie vlastnych priecinkov.
//! * `shelf`, `clipboard`, `notes`: modely a pravidla jednotlivych nastrojov (limity, retencia, pripnutie, mazanie).
//! * `library`: kniznica snimok obrazovky (skenovanie priecinkov, sledovanie zmien, bez zapisu a bez mazania).
//!
//! Pravidla: nic sa neposiela ani nelogue; obsah (cesty, texty, poznamky, obrazky) zije len v `%LOCALAPPDATA%\transom\tools` a
//! nikdy v `config.json`, exporte nastaveni ani archive AI. Povodne subory pouzivatela sa nikdy nemenia ani nemazu.
pub mod clipboard;
pub mod colors;
pub mod lane;
pub mod library;
pub mod names;
pub mod notes;
pub mod recents;
#[cfg(test)]
mod review_tests;
pub mod service;
pub mod shelf;
pub mod store;

pub use store::{data_dir, tools_dir, Unusable};

#[cfg(test)]
mod tests {
    /// Sukromne predvolene hodnoty: pouzitie ulozisk bez akcie pouzivatela nezapise na disk nic a schranka nezachyti nic.
    #[test]
    fn opening_all_stores_with_default_settings_writes_nothing_and_the_clipboard_collects_nothing()
    {
        let d = crate::store::temp_dir("lib-private");
        let t = transom_config::Config::default().tools;
        let s = crate::shelf::Shelf::open(d.join("shelf"), t.shelf.clone());
        let mut c = crate::clipboard::History::open(d.join("clipboard"), t.clipboard.clone());
        let n = crate::notes::Notes::open(d.join("notes"), t.notes.clone());
        assert!(s.is_empty() && n.is_empty());
        let snap = crate::clipboard::Snapshot {
            text: Some("cokolvek".into()),
            ..Default::default()
        };
        assert_eq!(
            c.capture(&snap, 1),
            crate::clipboard::Outcome::Skipped(crate::clipboard::Skip::Disabled)
        );
        let any: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().collect();
        assert!(any.is_empty(), "nic sa nezapisalo: {any:?}");
        let _ = std::fs::remove_dir_all(d);
    }
}
