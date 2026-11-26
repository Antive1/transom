//! Historia farieb z kapatka: posledne vybrane farby (najnovsia prva, bez duplicit, najviac [`MAX`]). Ukladaju sa len
//! lokalne v `tools\colors\history.json` (verzovane, atomicky), nikdy do `config.json`, exportu ani archivov AI.
use crate::store::{load_index, save_index, Loaded};
use serde_json::{Map, Value};
use std::path::PathBuf;

pub const FORMAT: &str = "transom-tools-colors";
pub const VERSION: u64 = 1;
pub const MAX: usize = 24;

pub type Rgb = (u8, u8, u8);

/// `#RRGGBB` (velke pismena).
pub fn hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// Farba zo zapisu `#RRGGBB` (aj bez `#`, lubovolne pismena).
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// HSL (odtien 0-359 °, sytost a svetlost 0-100 %).
pub fn hsl(c: Rgb) -> (u32, u32, u32) {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f32::EPSILON {
        return (0, 0, (l * 100.0).round() as u32);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (
        (h.round() as u32) % 360,
        (s * 100.0).round().min(100.0) as u32,
        (l * 100.0).round() as u32,
    )
}

pub struct History {
    path: PathBuf,
    items: Vec<Rgb>,
    read_only: bool,
}

impl History {
    /// Historia v priecinku `dir` (zvycajne `tools_dir().join("colors")`).
    pub fn open(dir: PathBuf) -> History {
        let path = dir.join("history.json");
        let (items, read_only) = match load_index(&path, FORMAT, VERSION) {
            Loaded::Missing => (Vec::new(), false),
            Loaded::Ok(v) | Loaded::Recovered(v) => {
                let mut items: Vec<Rgb> = Vec::new();
                for c in v
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().and_then(parse_hex))
                {
                    if !items.contains(&c) && items.len() < MAX {
                        items.push(c);
                    }
                }
                (items, false)
            }
            Loaded::Unusable(_) => (Vec::new(), true),
        };
        History {
            path,
            items,
            read_only,
        }
    }

    pub fn list(&self) -> &[Rgb] {
        &self.items
    }

    /// Prida farbu na zaciatok (rovnaka sa presunie dopredu), najviac [`MAX`].
    pub fn push(&mut self, c: Rgb) {
        self.items.retain(|x| *x != c);
        self.items.insert(0, c);
        self.items.truncate(MAX);
        self.save();
    }

    /// Odstrani farbu.
    pub fn remove(&mut self, c: Rgb) {
        let n = self.items.len();
        self.items.retain(|x| *x != c);
        if self.items.len() != n {
            self.save();
        }
    }

    /// Vymaze historiu aj subor (s jeho zalohou).
    pub fn clear(&mut self) {
        self.items.clear();
        if !self.read_only {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(self.path.with_extension("json.bak"));
        }
    }

    fn save(&self) {
        if self.read_only {
            return;
        }
        let mut m = Map::new();
        m.insert(
            "items".into(),
            Value::Array(self.items.iter().map(|c| Value::String(hex(*c))).collect()),
        );
        let _ = save_index(&self.path, FORMAT, VERSION, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::temp_dir;

    #[test]
    fn hex_round_trips_and_rejects_garbage() {
        assert_eq!(hex((0, 112, 224)), "#0070E0");
        assert_eq!(parse_hex("#0070e0"), Some((0, 112, 224)));
        assert_eq!(parse_hex("FFFFFF"), Some((255, 255, 255)));
        for bad in ["", "#12345", "#GGGGGG", "#1234567", "#12 456"] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn hsl_matches_known_values() {
        assert_eq!(hsl((255, 0, 0)), (0, 100, 50));
        assert_eq!(hsl((0, 255, 0)), (120, 100, 50));
        assert_eq!(hsl((0, 0, 255)), (240, 100, 50));
        assert_eq!(hsl((128, 128, 128)), (0, 0, 50));
        assert_eq!(hsl((255, 255, 255)), (0, 0, 100));
        assert_eq!(hsl((0x24, 0x1B, 0x3A)), (257, 36, 17));
    }

    #[test]
    fn history_is_newest_first_unique_limited_persistent_and_clearable() {
        let d = temp_dir("colors");
        let mut h = History::open(d.clone());
        assert!(h.list().is_empty());
        for i in 0..30u8 {
            h.push((i, i, i));
        }
        h.push((10, 10, 10));
        assert_eq!(h.list().len(), MAX);
        assert_eq!(h.list()[0], (10, 10, 10));
        assert_eq!(h.list().iter().filter(|c| **c == (10, 10, 10)).count(), 1);
        let again = History::open(d.clone());
        assert_eq!(again.list(), h.list());
        h.remove((10, 10, 10));
        assert!(!History::open(d.clone()).list().contains(&(10, 10, 10)));
        h.clear();
        assert!(History::open(d.clone()).list().is_empty());
        assert!(!d.join("history.json").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_newer_file_is_never_overwritten() {
        let d = temp_dir("colors-newer");
        let body = br##"{"format":"transom-tools-colors","version":9,"items":["#FF0000"]}"##;
        std::fs::write(d.join("history.json"), body).unwrap();
        let mut h = History::open(d.clone());
        h.push((1, 2, 3));
        assert_eq!(std::fs::read(d.join("history.json")).unwrap(), body);
        let _ = std::fs::remove_dir_all(d);
    }
}
