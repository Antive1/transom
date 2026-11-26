//! Preklady transom. Texty su v zdrojovych suboroch `locales/<kod>.txt` (riadky `kluc = hodnota`, `{0}` = parameter),
//! vlozenych do binarky. Hlada sa v aktualnom jazyku, potom v anglictine, nakoniec sa vrati samotny kluc.
//!
//! Jazyk aplikacie (`set_lang`) sa riadi konfiguraciou (`system` alebo kod); nemeni jazyk Windows. Format datumu,
//! casu a nazvy dni/mesiacov ostavaju podla regionalnych nastaveni Windows.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Sk,
    Cs,
    De,
    Fr,
    Es,
    Pl,
    It,
    Pt,
    Ja,
    ZhHans,
}

impl Lang {
    pub const ALL: [Lang; 11] = [
        Lang::Sk,
        Lang::En,
        Lang::Cs,
        Lang::De,
        Lang::Fr,
        Lang::Es,
        Lang::Pl,
        Lang::It,
        Lang::Pt,
        Lang::Ja,
        Lang::ZhHans,
    ];

    /// Kod v konfiguracii.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Sk => "sk",
            Lang::Cs => "cs",
            Lang::De => "de",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::Pl => "pl",
            Lang::It => "it",
            Lang::Pt => "pt",
            Lang::Ja => "ja",
            Lang::ZhHans => "zh-Hans",
        }
    }

    /// Nazov jazyka v nom samom (neprekladá sa).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Sk => "Slovenčina",
            Lang::Cs => "Čeština",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::Pl => "Polski",
            Lang::It => "Italiano",
            Lang::Pt => "Português",
            Lang::Ja => "日本語",
            Lang::ZhHans => "简体中文",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Lang::En => include_str!("../locales/en.txt"),
            Lang::Sk => include_str!("../locales/sk.txt"),
            Lang::Cs => include_str!("../locales/cs.txt"),
            Lang::De => include_str!("../locales/de.txt"),
            Lang::Fr => include_str!("../locales/fr.txt"),
            Lang::Es => include_str!("../locales/es.txt"),
            Lang::Pl => include_str!("../locales/pl.txt"),
            Lang::It => include_str!("../locales/it.txt"),
            Lang::Pt => include_str!("../locales/pt.txt"),
            Lang::Ja => include_str!("../locales/ja.txt"),
            Lang::ZhHans => include_str!("../locales/zh-Hans.txt"),
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn from_index(i: u8) -> Lang {
        [
            Lang::En,
            Lang::Sk,
            Lang::Cs,
            Lang::De,
            Lang::Fr,
            Lang::Es,
            Lang::Pl,
            Lang::It,
            Lang::Pt,
            Lang::Ja,
            Lang::ZhHans,
        ]
        .get(i as usize)
        .copied()
        .unwrap_or(Lang::En)
    }

    /// Jazyk podla kodu z konfiguracie alebo jazykovej znacky Windows (BCP 47, napr. `sk-SK`, `zh-CN`).
    /// Nepodporovany jazyk (aj tradicna cinstina `zh-TW`, `zh-Hant`) = `None`.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let t = tag.trim().replace('_', "-").to_ascii_lowercase();
        let primary = t.split('-').next().unwrap_or("");
        Some(match primary {
            "en" => Lang::En,
            "sk" => Lang::Sk,
            "cs" => Lang::Cs,
            "de" => Lang::De,
            "fr" => Lang::Fr,
            "es" => Lang::Es,
            "pl" => Lang::Pl,
            "it" => Lang::It,
            "pt" => Lang::Pt,
            "ja" => Lang::Ja,
            "zh" => {
                let traditional = ["hant", "tw", "hk", "mo"];
                if t.split('-').skip(1).any(|p| traditional.contains(&p)) {
                    return None;
                }
                Lang::ZhHans
            }
            _ => return None,
        })
    }
}

/// Jazyk aplikacie: nastavenie `setting` (`system` alebo kod); pri `system` jazyk Windows `windows_tag`;
/// nepodporovany alebo neznamy = anglictina.
pub fn resolve(setting: &str, windows_tag: &str) -> Lang {
    let s = setting.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("system") {
        Lang::from_tag(windows_tag).unwrap_or(Lang::En)
    } else {
        Lang::from_tag(s).unwrap_or(Lang::En)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Nastavi jazyk aplikacie (plati pre vsetky dalsie volania `t`/`tf` v procese).
pub fn set_lang(l: Lang) {
    CURRENT.store(l.index() as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    Lang::from_index(CURRENT.load(Ordering::Relaxed))
}

type Table = HashMap<&'static str, &'static str>;

fn parse(src: &'static str) -> Table {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once(" = ")?;
            Some((k.trim(), v))
        })
        .collect()
}

fn table(l: Lang) -> &'static Table {
    static TABLES: [OnceLock<Table>; 11] = [const { OnceLock::new() }; 11];
    TABLES[l.index()].get_or_init(|| parse(l.source()))
}

/// Text v jazyku `l` (zaloha: anglictina, potom kluc).
pub fn t_in(l: Lang, key: &str) -> String {
    table(l)
        .get(key)
        .or_else(|| table(Lang::En).get(key))
        .map(|s| s.to_string())
        .unwrap_or_else(|| key.to_string())
}

/// Text v aktualnom jazyku.
pub fn t(key: &str) -> String {
    t_in(lang(), key)
}

/// Text s parametrami `{0}`, `{1}`, …
pub fn tf(key: &str, args: &[&str]) -> String {
    fill(&t(key), args)
}

/// Dosadi parametre jednym prechodom: `{1}` vo vnutri dosadeneho textu (napr. nazov skladby) sa uz nenahradza.
fn fill(pattern: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let arg = tail.find('}').and_then(|j| {
            let n: usize = tail[..j]
                .parse()
                .ok()
                .filter(|_| tail[..j].bytes().all(|b| b.is_ascii_digit()))?;
            Some((args.get(n)?, j))
        });
        match arg {
            Some((a, j)) => {
                out.push_str(a);
                rest = &tail[j + 1..];
            }
            None => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> Vec<String> {
        let mut v: Vec<String> = (0..5)
            .map(|i| format!("{{{i}}}"))
            .filter(|p| s.contains(p.as_str()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn every_language_has_every_key_with_same_placeholders() {
        let en = table(Lang::En);
        assert!(en.len() > 100);
        for l in Lang::ALL {
            let tb = table(l);
            for (k, v) in en {
                let tr = tb
                    .get(k)
                    .unwrap_or_else(|| panic!("{}: chyba kluc {k}", l.code()));
                assert!(
                    !tr.trim().is_empty() || v.trim().is_empty(),
                    "{}: prazdny {k}",
                    l.code()
                );
                assert_eq!(
                    placeholders(tr),
                    placeholders(v),
                    "{}: parametre v {k}",
                    l.code()
                );
            }
            for k in tb.keys() {
                assert!(en.contains_key(k), "{}: kluc navyse {k}", l.code());
            }
        }
    }

    /// Zdrojaky ostatnych crate (`crates/*/src`, `examples`), bez tejto kniznice.
    fn workspace_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if p.file_name().is_some_and(|n| n != "target") {
                        walk(&p, out);
                    }
                } else if p.extension().is_some_and(|x| x == "rs") {
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        out.push((p.display().to_string(), text));
                    }
                }
            }
        }
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let mut out = Vec::new();
        for e in std::fs::read_dir(&crates).unwrap().flatten() {
            if e.file_name() != "transom-i18n" {
                walk(&e.path(), &mut out);
            }
        }
        assert!(out.len() > 50, "zdrojaky sa nenasli v {}", crates.display());
        out
    }

    /// Kluce z volani `t("..")`, `tf("..", ..)` a `t_in(.., "..")` (len retazcove literaly s bodkou).
    fn keys_in_calls(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        let b = src.as_bytes();
        for (name, skip_arg) in [("t(", false), ("tf(", false), ("t_in(", true)] {
            let mut from = 0;
            while let Some(i) = src[from..].find(name).map(|i| i + from) {
                from = i + name.len();
                if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
                    continue;
                }
                let mut rest = &src[from..];
                if skip_arg {
                    let Some(c) = rest.find(',') else { continue };
                    rest = &rest[c + 1..];
                }
                let rest = rest.trim_start();
                let Some(lit) = rest.strip_prefix('"') else {
                    continue;
                };
                let Some(end) = lit.find('"') else { continue };
                let key = &lit[..end];
                if key.contains('.') && !key.contains(' ') && !key.contains('{') {
                    out.push(key.to_string());
                }
            }
        }
        out
    }

    #[test]
    fn every_key_used_in_code_exists_and_every_key_is_used() {
        let en = table(Lang::En);
        let sources = workspace_sources();
        let mut missing = Vec::new();
        for (path, src) in &sources {
            for k in keys_in_calls(src) {
                if !en.contains_key(k.as_str()) {
                    missing.push(format!("{path}: {k}"));
                }
            }
        }
        assert!(missing.is_empty(), "kluce bez prekladu: {missing:#?}");
        // kluce skladane za behu (`format!("item.{id}")` ...): rodiny podla predpony
        const DYNAMIC: [&str; 5] = ["item.", "prof.", "wd.", "day.", "nav."];
        let all: String = sources.iter().map(|(_, s)| s.as_str()).collect();
        let mut unused: Vec<&str> = en
            .keys()
            .copied()
            .filter(|k| !DYNAMIC.iter().any(|p| k.starts_with(p)))
            .filter(|k| !all.contains(&format!("\"{k}\"")))
            .collect();
        unused.sort_unstable();
        assert!(unused.is_empty(), "nepouzite kluce: {unused:#?}");
    }

    #[test]
    fn call_scanner_finds_keys_in_all_call_forms() {
        let src = r#"let a = t("x.one"); tf("x.two", &[&n]); t_in(Lang::En, "x.three"); transom_i18n::t("x.four");
            sort("not.key"); t(&format!("x.{id}")); t("no_dot");"#;
        assert_eq!(
            keys_in_calls(src),
            vec!["x.one", "x.four", "x.two", "x.three"]
        );
    }

    #[test]
    fn resolve_and_fallback() {
        assert_eq!(resolve("system", "sk-SK"), Lang::Sk);
        assert_eq!(resolve("system", "pt-BR"), Lang::Pt);
        assert_eq!(resolve("system", "zh-CN"), Lang::ZhHans);
        assert_eq!(resolve("system", "zh-Hans-CN"), Lang::ZhHans);
        assert_eq!(
            resolve("system", "zh-TW"),
            Lang::En,
            "tradicna cinstina nie je podporovana"
        );
        assert_eq!(resolve("system", "zh-Hant-HK"), Lang::En);
        assert_eq!(
            resolve("system", "hu-HU"),
            Lang::En,
            "nepodporovany jazyk Windows -> anglictina"
        );
        assert_eq!(resolve("system", ""), Lang::En);
        assert_eq!(
            resolve("de", "sk-SK"),
            Lang::De,
            "vyslovna volba ma prednost"
        );
        assert_eq!(
            resolve("klingon", "sk-SK"),
            Lang::En,
            "neznamy kod -> anglictina"
        );
        assert_eq!(resolve(" SYSTEM ", "ja-JP"), Lang::Ja);
        for l in Lang::ALL {
            assert_eq!(Lang::from_tag(l.code()), Some(l));
        }
    }

    #[test]
    fn placeholders_are_filled_once_and_unknown_ones_stay() {
        // (vzor, parametre, vysledok)
        let cases: [(&str, &[&str], &str); 7] = [
            ("{0} – {1}", &["{1}", "x"], "{1} – x"),
            ("{1} {0} {1}", &["a", "b"], "b a b"),
            ("{0}", &[], "{0}"),
            ("{2} {0}", &["a"], "{2} a"),
            ("{x} {} {0", &["a"], "{x} {} {0"),
            ("{+0} {00}", &["a"], "{+0} a"),
            ("ž{0}ž", &["€"], "ž€ž"),
        ];
        for (p, a, want) in cases {
            assert_eq!(fill(p, a), want, "{p:?}");
        }
    }

    #[test]
    fn lookup_and_args() {
        set_lang(Lang::En);
        assert_eq!(t("menu.settings"), "Settings");
        assert_eq!(tf("ring.used", &["62"]), "62 % used");
        assert_eq!(t("neexistuje.kluc"), "neexistuje.kluc");
        set_lang(Lang::Sk);
        assert_eq!(lang(), Lang::Sk);
        assert_ne!(t("menu.settings"), "Settings");
        set_lang(Lang::En);
    }
}
