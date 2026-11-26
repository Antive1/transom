//! Aktualizacie z vydani na GitHube.
//!
//! Vlakno sa po starte (s oneskorenim) a potom kazdych niekolko hodin opyta `api.github.com` na zoznam vydani (jedna
//! poziadavka GET bez akychkolvek udajov o pouzivatelovi), vyberie najvyssiu verziu s instalatorom a porovna ju so
//! svojou. Nove vydanie lista oznami (oznamenie, polozka menu). Instalacia az na pokyn pouzivatela: stiahne instalator
//! a `SHA256SUMS.txt` z toho isteho vydania, overi kontrolny sucet, a ak je spustena lista podpisana, aj to, ze
//! instalator podpisal ten isty vydavatel; potom spusti instalator (ticho, s opatovnym spustenim listy) a lista skonci.
//! Prenosna kopia alebo vyvojarsky build (vedla exe nie je odinstalator) iba otvori stranku vydania.
use serde_json::Value;
use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Repozitar s vydaniami.
pub const REPO: &str = "Antive1/transom";
/// Zoznam vydani (aj predbezne: vsetky vydania su zatial "early preview").
const API_URL: &str = "https://api.github.com/repos/Antive1/transom/releases?per_page=20";
/// Subory vydania sa stahuju len z tohto miesta.
const DOWNLOAD_PREFIX: &str = "https://github.com/Antive1/transom/releases/download/";
/// Prva kontrola az chvilu po starte (start systemu nezdrzi), potom pravidelne.
const FIRST_DELAY: Duration = Duration::from_secs(90);
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_LIST: usize = 2 * 1024 * 1024;
const MAX_SETUP: usize = 128 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;
/// Parametre instalatora pri aktualizacii: priebeh bez otazok, zatvorit bezacu listu aj nastavenia, potom listu spustit.
/// Volby instalacie (start s Windows, odkaz na ploche) ostanu z predoslej instalacie.
pub const SETUP_ARGS: &str =
    "/SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /FORCECLOSEAPPLICATIONS /RELAUNCH=1";

/// Verzia `x.y.z` (s volitelnym `v` na zaciatku; pripona za `-` alebo `+` sa ignoruje).
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

/// Najdene vydanie s instalatorom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Verzia bez `v` ("0.2.0").
    pub version: String,
    /// Stranka vydania (prehliadac).
    pub page: String,
    /// Nazov a adresa instalatora.
    pub setup_name: String,
    pub setup_url: String,
    /// Adresa `SHA256SUMS.txt`.
    pub sums_url: String,
    /// Co je nove vo vsetkych vydaniach novsich nez bezaca verzia (najnovsie prve).
    pub notes: Notes,
}

/// Poznamky k vydaniu z textu vydania (sekcia CHANGELOG.md): novinky (Added, Changed) a opravy (Fixed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub added: Vec<String>,
    pub fixed: Vec<String>,
}

/// Najviac bodov v jednom zozname (po zluceni vydani) a znakov v jednom bode.
pub const MAX_NOTES: usize = 40;
pub const MAX_NOTE_CHARS: usize = 200;
/// Text vydania sa cita najviac do tejto dlzky (znaky).
const MAX_BODY_CHARS: usize = 64 * 1024;

impl Notes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.fixed.is_empty()
    }

    /// Prida body dalsieho (starsieho) vydania na koniec, s ohranicenim poctu.
    pub fn append(&mut self, other: Notes) {
        for (to, from) in [
            (&mut self.added, other.added),
            (&mut self.fixed, other.fixed),
        ] {
            let room = MAX_NOTES.saturating_sub(to.len());
            to.extend(from.into_iter().take(room));
        }
    }
}

/// Do ktoreho zoznamu patri sekcia `### Nazov` (Keep a Changelog): `Some(false)` novinky, `Some(true)` opravy,
/// `None` = sekcia sa ignoruje.
fn section_kind(title: &str) -> Option<bool> {
    let t = title
        .trim()
        .trim_end_matches(':')
        .trim()
        .to_ascii_lowercase();
    match t.as_str() {
        "added" | "changed" | "new" | "removed" | "deprecated" | "improved" => Some(false),
        "fixed" | "fixes" | "security" => Some(true),
        _ => None,
    }
}

/// Odstrani jednoduche znacky markdownu: `**`, `__`, `` ` ``, odkazy `[text](url)` a obrazky `![text](url)`.
pub fn strip_markdown(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let next = chars.get(i + 1).copied();
        match c {
            '*' if next == Some('*') => i += 2,
            '_' if next == Some('_') => i += 2,
            '`' => i += 1,
            '!' if next == Some('[') => i += 1,
            '[' => {
                // [text](url) -> text; ine hranate zatvorky ostanu
                let rest = chars.get(i + 1..).unwrap_or(&[]);
                let link = rest.iter().position(|&c| c == ']').and_then(|e| {
                    let after = rest.get(e + 1..).unwrap_or(&[]);
                    if after.first() != Some(&'(') {
                        return None;
                    }
                    let end = after.iter().position(|&c| c == ')')?;
                    Some((e, e + 1 + end))
                });
                match link {
                    Some((e, end)) => {
                        out.extend(rest.get(..e).unwrap_or(&[]));
                        i += 1 + end + 1;
                    }
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Skrati text na najviac `max` znakov (elipsa na konci).
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.truncate(t.trim_end().len());
    t.push('\u{2026}');
    t
}

/// Poznamky z textu vydania. Body (`- ` alebo `* `) sa beru len pod znamym nadpisom `###`; iny nadpis alebo `---`
/// sekciu ukonci (staticky text pod oddelovacom sa necita). Odsadeny pokracujuci riadok sa pripoji k bodu, vnorene
/// body sa vynechaju. Nikdy nespadne (text je z internetu).
pub fn parse_notes(body: &str) -> Notes {
    let mut n = Notes::default();
    let mut cur: Option<bool> = None;
    // posledny bod, ku ktoremu sa moze pripojit pokracovanie
    let mut open = false;
    let body: String = body.chars().take(MAX_BODY_CHARS).collect();
    for raw in body.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.is_empty() {
            open = false;
            continue;
        }
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            cur = if level >= 3 {
                section_kind(trimmed.trim_start_matches('#'))
            } else {
                None
            };
            open = false;
            continue;
        }
        let rule = trimmed.len() >= 3
            && (trimmed.chars().all(|c| c == '-')
                || trimmed.chars().all(|c| c == '*')
                || trimmed.chars().all(|c| c == '_'));
        if indent < 2 && rule {
            cur = None;
            open = false;
            continue;
        }
        let Some(kind) = cur else { continue };
        let list = if kind { &mut n.fixed } else { &mut n.added };
        let bullet = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "));
        match bullet {
            Some(text) if indent < 2 => {
                let text = strip_markdown(text);
                open = !text.is_empty() && list.len() < MAX_NOTES;
                if open {
                    list.push(clip(&text, MAX_NOTE_CHARS));
                }
            }
            // vnoreny bod sa vynecha
            Some(_) => {}
            // pokracovanie bodu na dalsom riadku
            None if indent >= 2 && open => {
                if let Some(prev) = list.last_mut() {
                    let joined = format!("{prev} {}", strip_markdown(trimmed));
                    *prev = clip(&joined, MAX_NOTE_CHARS);
                }
            }
            None => open = false,
        }
    }
    n
}

/// Nazov suboru vydania alebo znacky je bezpecny: len `A-Z a-z 0-9 . _ -`, nezacina bodkou, rozumna dlzka. Nazov
/// instalatora sa pouzije ako nazov suboru na disku (bez `..`, `\`, `/`, `:`), znacka ako cast adresy.
fn safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('.')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// Adresa suboru vydania: presne `DOWNLOAD_PREFIX<znacka>/<nazov>` (ziadne `../`, `%2e`, dotaz ani ine vydanie/repozitar).
fn asset_url(a: &Value, tag: &str, name: &str) -> Option<String> {
    let u = a.get("browser_download_url")?.as_str()?;
    let rest = u.strip_prefix(DOWNLOAD_PREFIX)?;
    (safe_segment(tag) && safe_segment(name) && rest == format!("{tag}/{name}"))
        .then(|| u.to_string())
}

/// Z odpovede API vyberie najvyssie vydanie (nie koncept), ktore ma instalator aj kontrolne sucty a je novsie nez
/// `current`. `Ok(None)` = transom je aktualny. Ziadna cast odpovede nesmie spadnut (bez `unwrap` a indexovania).
pub fn pick(json: &str, current: &str) -> Result<Option<Release>, String> {
    let cur = parse_version(current).ok_or("neplatna vlastna verzia")?;
    let v: Value = serde_json::from_str(json).map_err(|_| "odpoved nie je JSON".to_string())?;
    let list = v.as_array().ok_or("odpoved nie je zoznam vydani")?;
    let mut best: Option<((u64, u64, u64), Release)> = None;
    // poznamky vsetkych novsich vydani (aj bez instalatora): pri preskoceni verzii sa zlucia
    let mut newer: Vec<((u64, u64, u64), Notes)> = Vec::new();
    for r in list {
        if r.get("draft").and_then(Value::as_bool).unwrap_or(true) {
            continue;
        }
        let Some(tag) = r.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let Some(ver) = parse_version(tag) else {
            continue;
        };
        if ver > cur {
            let body = r.get("body").and_then(Value::as_str).unwrap_or("");
            newer.push((ver, parse_notes(body)));
        }
        let assets = r
            .get("assets")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let name_of = |a: &Value| a.get("name").and_then(Value::as_str).map(str::to_string);
        let setup = assets.iter().find_map(|a| {
            let n = name_of(a)?;
            (n.starts_with("transom-setup-") && n.ends_with("-x64.exe"))
                .then_some(())
                .and_then(|_| Some((n.clone(), asset_url(a, tag, &n)?)))
        });
        let sums = assets
            .iter()
            .find(|a| name_of(a).as_deref() == Some("SHA256SUMS.txt"))
            .and_then(|a| asset_url(a, tag, "SHA256SUMS.txt"));
        let (Some((setup_name, setup_url)), Some(sums_url)) = (setup, sums) else {
            continue;
        };
        // stranka sa otvara cez shell: zostavi sa z overenej znacky (asset_url ju preveril), nie z `html_url` odpovede
        let page = format!("https://github.com/{REPO}/releases/tag/{tag}");
        if best.as_ref().is_none_or(|(b, _)| ver > *b) {
            best = Some((
                ver,
                Release {
                    version: format!("{}.{}.{}", ver.0, ver.1, ver.2),
                    page,
                    setup_name,
                    setup_url,
                    sums_url,
                    notes: Notes::default(),
                },
            ));
        }
    }
    let Some((top, mut rel)) = best.filter(|(v, _)| *v > cur) else {
        return Ok(None);
    };
    // najnovsie prve; to iste cislo verzie (napr. "v1.0.0" a "1.0.0") len raz
    newer.sort_by_key(|n| std::cmp::Reverse(n.0));
    newer.dedup_by(|a, b| a.0 == b.0);
    for (_, n) in newer.into_iter().filter(|(v, _)| *v <= top) {
        rel.notes.append(n);
    }
    Ok(Some(rel))
}

/// Kontrolny sucet suboru `name` zo `SHA256SUMS.txt` (riadky `hash  nazov`).
pub fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let h = it.next()?;
        let n = it.next()?.trim_start_matches('*');
        (n == name && h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| h.to_ascii_lowercase())
    })
}

/// Co sa naposledy stalo (lista podla toho oznamuje).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// Kontrola na poziadanie: transom je aktualny.
    UpToDate,
    /// Stahuje sa a overuje instalator.
    Downloading,
    /// Instalator sa spustil: lista sa ma ukoncit.
    Launched,
    /// Chyba (kratky popis); `manual` = vysledok akcie pouzivatela (ukaze sa), inak len do logu.
    Failed { msg: String, manual: bool },
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub available: Option<Release>,
    pub phase: Phase,
}

enum Msg {
    Check { manual: bool },
    Install,
    Auto(bool),
}

pub struct Updater {
    tx: Sender<Msg>,
    state: Arc<Mutex<State>>,
}

fn check(current: &str) -> Result<Option<Release>, String> {
    let bytes = transom_platform::http::get(API_URL, MAX_LIST, 20_000)?;
    pick(&String::from_utf8_lossy(&bytes), current)
}

/// Stiahne, overi a spusti instalator. `Ok(())` = spusteny.
fn install(r: &Release) -> Result<(), String> {
    let sums = transom_platform::http::get(&r.sums_url, MAX_SUMS, 30_000)?;
    let want = sum_for(&String::from_utf8_lossy(&sums), &r.setup_name)
        .ok_or("instalator chyba v SHA256SUMS.txt")?;
    let bytes = transom_platform::http::get(&r.setup_url, MAX_SETUP, 120_000)?;
    let got = transom_platform::selfupdate::sha256_hex(&bytes).ok_or("SHA-256 zlyhal")?;
    if got != want {
        return Err("kontrolny sucet instalatora nesedi".into());
    }
    // bezpecnost: novy priecinok, novy subor (nic cudzie sa neprepise ani nenasleduje) a od zapisu po spustenie zamknuty
    // proti zmene; sucet sa znova overi z disku (iny proces nemoze instalator vymenit medzi overenim a spustenim)
    let dir = transom_platform::selfupdate::fresh_dir(&std::env::temp_dir().join("transom-update"))
        .map_err(|e| format!("docasny priecinok: {e}"))?;
    let pinned =
        transom_platform::selfupdate::write_pinned(&dir.join(&r.setup_name), &bytes, &want)
            .map_err(|e| format!("zapis instalatora: {e}"))?;
    let path = pinned.path().to_path_buf();
    // podpisana lista prijme len instalator od toho isteho vydavatela
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| transom_platform::selfupdate::signer(&p));
    if let Some(me) = me {
        match transom_platform::selfupdate::signer(&path) {
            Some(s) if s == me => {}
            _ => {
                drop(pinned);
                let _ = std::fs::remove_file(&path);
                return Err("instalator nie je podpisany vydavatelom transom".into());
            }
        }
    }
    let launched = transom_platform::selfupdate::launch(&path, SETUP_ARGS);
    drop(pinned);
    if !launched {
        return Err("instalator sa nespustil".into());
    }
    Ok(())
}

impl Updater {
    pub fn start(auto: bool, notify: Arc<dyn Fn() + Send + Sync>) -> Option<Updater> {
        let (tx, rx) = channel::<Msg>();
        let state: Arc<Mutex<State>> = Arc::default();
        let st = Arc::clone(&state);
        let current = env!("CARGO_PKG_VERSION");
        std::thread::Builder::new()
            .name("transom-update".into())
            .spawn(move || {
                let set = |f: &dyn Fn(&mut State)| {
                    if let Ok(mut g) = st.lock() {
                        f(&mut g);
                    }
                    notify();
                };
                let mut auto = auto;
                let mut wait = FIRST_DELAY;
                loop {
                    let msg = match rx.recv_timeout(wait) {
                        Ok(m) => m,
                        Err(RecvTimeoutError::Timeout) => {
                            wait = INTERVAL;
                            if !auto {
                                continue;
                            }
                            Msg::Check { manual: false }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    };
                    match msg {
                        Msg::Auto(on) => auto = on,
                        Msg::Check { manual } => match check(current) {
                            Ok(r) => {
                                crate::log(&format!(
                                    "aktualizacie: {}",
                                    r.as_ref().map_or("transom je aktualny".to_string(), |r| {
                                        format!("nova verzia {}", r.version)
                                    })
                                ));
                                set(&|s| {
                                    s.phase = if manual && r.is_none() {
                                        Phase::UpToDate
                                    } else {
                                        Phase::Idle
                                    };
                                    s.available = r.clone();
                                });
                            }
                            Err(e) => {
                                crate::log(&format!("aktualizacie: kontrola zlyhala: {e}"));
                                set(&|s| {
                                    s.phase = Phase::Failed {
                                        msg: e.clone(),
                                        manual,
                                    }
                                });
                            }
                        },
                        Msg::Install => {
                            let rel = st.lock().ok().and_then(|g| g.available.clone());
                            let Some(rel) = rel else { continue };
                            set(&|s| s.phase = Phase::Downloading);
                            match install(&rel) {
                                Ok(()) => {
                                    crate::log(&format!(
                                        "aktualizacie: instalator {} spusteny",
                                        rel.version
                                    ));
                                    set(&|s| s.phase = Phase::Launched);
                                    return;
                                }
                                Err(e) => {
                                    crate::log(&format!("aktualizacie: instalacia zlyhala: {e}"));
                                    set(&|s| {
                                        s.phase = Phase::Failed {
                                            msg: e.clone(),
                                            manual: true,
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            })
            .ok()?;
        Some(Updater { tx, state })
    }

    /// Skontrolovat hned (na poziadanie pouzivatela: vysledok sa oznami aj ked je transom aktualny).
    pub fn check_now(&self) {
        let _ = self.tx.send(Msg::Check { manual: true });
    }

    /// Stiahnut a spustit instalator dostupnej verzie.
    pub fn install(&self) {
        let _ = self.tx.send(Msg::Install);
    }

    /// Zapnut/vypnut pravidelnu kontrolu.
    pub fn set_auto(&self, on: bool) {
        let _ = self.tx.send(Msg::Auto(on));
    }

    pub fn snapshot(&self) -> State {
        self.state.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Vysledok uz oznameny: dalsia udalost sa porovna s cistym stavom.
    pub fn clear_phase(&self) {
        if let Ok(mut g) = self.state.lock() {
            if g.phase != Phase::Downloading {
                g.phase = Phase::Idle;
            }
        }
    }
}

/// Ma transom instalaciu, ktoru moze aktualizovat sam (inak sa otvori stranka vydania)?
pub fn can_self_update() -> bool {
    transom_platform::selfupdate::installed_dir().is_some()
}

/// Kontroly sa nerobia pri testoch, simulaciach a pri presmerovanej konfiguracii alebo datach (izolovane behy).
pub fn allowed(automation: bool) -> bool {
    !automation
        && ["TRANSOM_CONFIG", "TRANSOM_DATA"]
            .iter()
            .all(|k| std::env::var_os(k).is_none_or(|v| v.is_empty()))
}

pub fn open_page(url: &str) {
    let _ = transom_platform::selfupdate::launch(&PathBuf::from(url), "");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, tag: &str) -> String {
        format!(
            r#"{{"name":"{name}","browser_download_url":"https://github.com/Antive1/transom/releases/download/{tag}/{name}"}}"#
        )
    }

    fn rel(tag: &str, draft: bool, files: &[&str]) -> String {
        let a: Vec<String> = files.iter().map(|f| asset(f, tag)).collect();
        format!(
            r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":true,"html_url":"https://github.com/Antive1/transom/releases/tag/{tag}","assets":[{}]}}"#,
            a.join(",")
        )
    }

    #[test]
    fn versions_parse_with_or_without_v_and_suffix() {
        assert_eq!(parse_version("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_version("1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("2.0"), Some((2, 0, 0)));
        assert_eq!(parse_version("x1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn the_highest_newer_release_with_an_installer_wins() {
        let full = ["transom-setup-X-x64.exe", "SHA256SUMS.txt"];
        let json = format!(
            "[{},{},{},{},{}]",
            rel(
                "v0.3.0",
                true,
                &["transom-setup-0.3.0-x64.exe", "SHA256SUMS.txt"]
            ), // koncept
            rel("v0.2.5", false, &["transom-0.2.5-x64-portable.zip"]), // bez instalatora
            rel(
                "v0.2.0",
                false,
                &["transom-setup-0.2.0-x64.exe", "SHA256SUMS.txt"]
            ),
            rel("v0.1.5", false, &full),
            rel("nightly", false, &full),
        );
        let r = pick(&json, "0.1.0").unwrap().unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.setup_name, "transom-setup-0.2.0-x64.exe");
        assert!(
            r.setup_url.starts_with(DOWNLOAD_PREFIX) && r.sums_url.ends_with("/SHA256SUMS.txt")
        );
        assert_eq!(
            r.page,
            "https://github.com/Antive1/transom/releases/tag/v0.2.0"
        );
        assert_eq!(
            pick(&json, "0.2.0").unwrap(),
            None,
            "rovnaka verzia = aktualne"
        );
        assert_eq!(pick(&json, "0.4.0").unwrap(), None);
    }

    #[test]
    fn assets_from_elsewhere_are_ignored_and_bad_answers_never_panic() {
        let json = r#"[{"tag_name":"v9.0.0","draft":false,"assets":[
            {"name":"transom-setup-9.0.0-x64.exe","browser_download_url":"https://evil.example/transom-setup-9.0.0-x64.exe"},
            {"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/Antive1/transom/releases/download/v9.0.0/SHA256SUMS.txt"}]}]"#;
        assert_eq!(pick(json, "0.1.0").unwrap(), None);
        for bad in [
            "",
            "{}",
            "null",
            "[1,2]",
            r#"[{"tag_name":5}]"#,
            r#"{"message":"rate limited"}"#,
        ] {
            let _ = pick(bad, "0.1.0");
        }
        assert!(pick("nie json", "0.1.0").is_err());
    }

    /// Sekcia `## [ver]` z CHANGELOG.md (ako ju vyberie `release.yml`).
    fn changelog_section(ver: &str) -> Option<String> {
        let log = include_str!("../../../CHANGELOG.md");
        let head = format!("## [{ver}]");
        let mut out = Vec::new();
        let mut on = false;
        for l in log.lines() {
            if l.starts_with("## ") {
                if on {
                    break;
                }
                on = l.starts_with(&head);
                continue;
            }
            if on {
                out.push(l);
            }
        }
        on.then(|| out.join("\n"))
    }

    /// Text vydania tak, ako ho sklada `release.yml`: sekcia, oddelovac, staticky text.
    fn release_body(ver: &str) -> String {
        format!(
            "{}\n\n---\n\n{}",
            changelog_section(ver).unwrap_or_default(),
            include_str!("../../../.github/release-notes.md")
        )
    }

    fn rel_body(tag: &str, files: &[&str], body: &str) -> String {
        let r = rel(tag, false, files);
        let b = serde_json::to_string(body).unwrap();
        format!("{},\"body\":{b}}}", r.trim_end_matches('}'))
    }

    #[test]
    fn the_changelog_has_a_section_with_entries_for_this_version() {
        let v = env!("CARGO_PKG_VERSION");
        let sec = changelog_section(v).expect("CHANGELOG.md: sekcia pre aktualnu verziu");
        assert!(sec.lines().any(|l| l.starts_with("- ")), "{sec}");
        assert!(changelog_section("9.9.9").is_none());
    }

    #[test]
    fn real_release_bodies_give_their_bullets_and_nothing_from_the_static_text() {
        let v = env!("CARGO_PKG_VERSION");
        let n = parse_notes(&release_body(v));
        // pocty a znenie sa mozu upravit; struktura nie: Added aj Changed su novinky, Fixed opravy, zalomeny bod je jeden
        let raw = changelog_section(v).unwrap_or_default();
        let bullets = |sec: &str| {
            raw.split("### ")
                .find(|s| s.starts_with(sec))
                .map_or(0, |s| s.lines().filter(|l| l.starts_with("- ")).count())
        };
        assert_eq!(
            n.added.len(),
            bullets("Added") + bullets("Changed"),
            "{n:?}"
        );
        assert_eq!(n.fixed.len(), bullets("Fixed"), "{n:?}");
        assert!(!n.added.is_empty() || !n.fixed.is_empty(), "{n:?}");
        let wrapped = raw
            .lines()
            .position(|l| l.starts_with("  "))
            .and_then(|i| raw.lines().nth(i));
        if let Some(cont) = wrapped {
            assert!(
                n.added
                    .iter()
                    .chain(&n.fixed)
                    .any(|x| x.ends_with(cont.trim())),
                "pokracovanie bodu sa pripoji: {cont:?} {n:?}"
            );
        }
        // tabulka a odstavce statickeho textu ("**transom** – ...") nie su body
        for x in n.added.iter().chain(&n.fixed) {
            assert!(!x.contains("transom-setup") && !x.contains("**"), "{x}");
            assert!(x.chars().count() <= MAX_NOTE_CHARS);
        }
        // samotny staticky text (vydanie bez sekcie) nedava nic
        assert!(parse_notes(include_str!("../../../.github/release-notes.md")).is_empty());
    }

    #[test]
    fn notes_parse_markdown_tolerantly() {
        let body = "Intro text\n- not in a section\n\n### Added\r\n* **Bold** `code` and [a link](https://x.example/a) ![img](i.png)\r\n- second\n  - nested is skipped\n- \n### Removed\n- gone\n### Security\n- patched\n### Fixed\n-no space is not a bullet\n- fix one\n  continued here\n\n  not continued after a blank line\n### Other\n- ignored\n## [0.1.0]\n### Fixed\n- next release in the same body\n---\n- after rule";
        let n = parse_notes(body);
        assert_eq!(
            n.added,
            vec!["Bold code and a link img", "second", "gone"],
            "{n:?}"
        );
        assert_eq!(
            n.fixed,
            vec![
                "patched",
                "fix one continued here",
                "next release in the same body"
            ]
        );
        assert_eq!(strip_markdown("[x] task (y)"), "[x] task (y)");
        assert_eq!(strip_markdown("[open](no end"), "[open](no end");
        assert_eq!(
            strip_markdown("__under__ and *single*"),
            "under and *single*"
        );
    }

    #[test]
    fn notes_are_capped_and_never_panic() {
        let long = "é".repeat(500);
        let many: String = (0..100).map(|i| format!("- item {i} {long}\n")).collect();
        let n = parse_notes(&format!("### Added\n{many}"));
        assert_eq!(n.added.len(), MAX_NOTES);
        assert!(n
            .added
            .iter()
            .all(|x| x.chars().count() <= MAX_NOTE_CHARS && x.ends_with('\u{2026}')));
        for bad in [
            "",
            "#",
            "###",
            "- ",
            "### Added\n- [",
            "### Added\n- [a](",
            "### Fixed\n- ]()[",
            "\u{0}\u{FFFF}",
            "   \n\t- x",
        ] {
            let _ = parse_notes(bad);
        }
        let mut a = Notes {
            added: vec!["x".into(); MAX_NOTES - 1],
            fixed: vec![],
        };
        a.append(Notes {
            added: vec!["y".into(), "z".into()],
            fixed: vec!["f".into()],
        });
        assert_eq!(a.added.len(), MAX_NOTES);
        assert_eq!(a.added.last().map(String::as_str), Some("y"));
        assert_eq!(a.fixed, vec!["f"]);
    }

    #[test]
    fn notes_of_all_newer_releases_are_merged_newest_first() {
        let full = |v: &str| {
            [
                format!("transom-setup-{v}-x64.exe"),
                "SHA256SUMS.txt".to_string(),
            ]
        };
        let f3 = full("0.4.0");
        let f2 = full("0.3.0");
        let files = |f: &[String; 2]| [f[0].clone(), f[1].clone()];
        let [a, b] = files(&f3);
        let [c, d] = files(&f2);
        let json = format!(
            "[{},{},{},{},{}]",
            rel_body(
                "v0.4.0",
                &[&a, &b],
                "### Added\n- four\n### Fixed\n- fix four"
            ),
            // bez instalatora, ale novsie nez bezaca verzia: jej poznamky sa zlucia
            rel_body("v0.3.5", &[], "### Fixed\n- fix three and a half"),
            rel_body("v0.3.0", &[&c, &d], "### Added\n- three"),
            rel_body("v0.2.0", &[], "### Added\n- two (already installed)"),
            // novsie nez vybrane, ale koncept: nepocita sa
            rel("v0.5.0", true, &[]),
        );
        let r = pick(&json, "0.2.0").unwrap().unwrap();
        assert_eq!(r.version, "0.4.0");
        assert_eq!(r.notes.added, vec!["four", "three"]);
        assert_eq!(r.notes.fixed, vec!["fix four", "fix three and a half"]);
        let r = pick(&json, "0.3.0").unwrap().unwrap();
        assert_eq!(r.notes.added, vec!["four"]);
        assert_eq!(r.notes.fixed, vec!["fix four", "fix three and a half"]);
        // telo inym typom nez text: bez poznamok, bez paniky
        let odd = format!(
            "[{}]",
            rel("v0.4.0", false, &[&a, &b]).replace("\"draft\"", "\"body\":42,\"draft\"")
        );
        assert!(pick(&odd, "0.2.0").unwrap().unwrap().notes.is_empty());
    }

    #[test]
    fn checksums_are_found_by_exact_name() {
        let h = "a".repeat(64);
        let sums = format!(
            "{h}  transom-0.2.0-x64-portable.zip\n{}  transom-setup-0.2.0-x64.exe\r\n",
            "B".repeat(64)
        );
        assert_eq!(
            sum_for(&sums, "transom-setup-0.2.0-x64.exe"),
            Some("b".repeat(64))
        );
        assert_eq!(sum_for(&sums, "transom-setup-0.2.0-x64.ex"), None);
        assert_eq!(
            sum_for(
                "short  transom-setup-0.2.0-x64.exe",
                "transom-setup-0.2.0-x64.exe"
            ),
            None
        );
    }
}

#[cfg(test)]
mod live {
    /// Skutocne API GitHubu (siet): `cargo test -p transom-app --bin transom-app live_ -- --ignored`.
    #[test]
    #[ignore]
    fn live_release_list_parses() {
        let r = super::check("0.0.1").expect("kontrola");
        let r = r.expect("aspon jedno vydanie s instalatorom");
        eprintln!("LIVE {r:?}");
        let sums = transom_platform::http::get(&r.sums_url, super::MAX_SUMS, 30_000).unwrap();
        let want = super::sum_for(&String::from_utf8_lossy(&sums), &r.setup_name).unwrap();
        let setup = transom_platform::http::get(&r.setup_url, super::MAX_SETUP, 120_000).unwrap();
        assert_eq!(
            transom_platform::selfupdate::sha256_hex(&setup).unwrap(),
            want
        );
    }
}

#[cfg(test)]
mod security {
    use super::*;

    fn one(tag: &str, setup_name: &str, setup_url: &str, sums_url: &str) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","draft":false,"assets":[
            {{"name":"{setup_name}","browser_download_url":"{setup_url}"}},
            {{"name":"SHA256SUMS.txt","browser_download_url":"{sums_url}"}}]}}]"#
        )
    }

    /// Nazov instalatora sa pouzije ako nazov suboru a adresa sa stahuje: oboje musi byt presne subor tohto vydania.
    #[test]
    fn installer_names_and_urls_outside_the_release_are_rejected() {
        let d = DOWNLOAD_PREFIX;
        let good_sums = format!("{d}v9.0.0/SHA256SUMS.txt");
        let ok = one(
            "v9.0.0",
            "transom-setup-9.0.0-x64.exe",
            &format!("{d}v9.0.0/transom-setup-9.0.0-x64.exe"),
            &good_sums,
        );
        assert_eq!(
            pick(&ok, "0.1.0").unwrap().map(|r| r.setup_name),
            Some("transom-setup-9.0.0-x64.exe".into())
        );
        let cases: Vec<(&str, String, String, String)> = vec![
            // prechod mimo docasny priecinok cez nazov suboru
            (
                "v9.0.0",
                r"transom-setup-\..\..\Startup\evil-x64.exe".into(),
                format!(r"{d}v9.0.0/transom-setup-\..\..\Startup\evil-x64.exe"),
                good_sums.clone(),
            ),
            (
                "v9.0.0",
                "transom-setup-/../../evil-x64.exe".into(),
                format!("{d}v9.0.0/transom-setup-/../../evil-x64.exe"),
                good_sums.clone(),
            ),
            (
                "v9.0.0",
                "transom-setup-C:evil-x64.exe".into(),
                format!("{d}v9.0.0/transom-setup-C:evil-x64.exe"),
                good_sums.clone(),
            ),
            // adresa z ineho repozitara cez ../ alebo kodovane bodky
            (
                "v9.0.0",
                "transom-setup-9.0.0-x64.exe".into(),
                format!(
                    "{d}../../../evil/repo/releases/download/v9.0.0/transom-setup-9.0.0-x64.exe"
                ),
                good_sums.clone(),
            ),
            (
                "v9.0.0",
                "transom-setup-9.0.0-x64.exe".into(),
                format!("{d}v9.0.0/%2e%2e/transom-setup-9.0.0-x64.exe"),
                good_sums.clone(),
            ),
            // ine vydanie, dotaz, iny nazov v adrese
            (
                "v9.0.0",
                "transom-setup-9.0.0-x64.exe".into(),
                format!("{d}v0.0.1/transom-setup-9.0.0-x64.exe"),
                good_sums.clone(),
            ),
            (
                "v9.0.0",
                "transom-setup-9.0.0-x64.exe".into(),
                format!("{d}v9.0.0/transom-setup-9.0.0-x64.exe?x=1"),
                good_sums.clone(),
            ),
            (
                "v9.0.0",
                "transom-setup-9.0.0-x64.exe".into(),
                format!("{d}v9.0.0/transom-setup-0.0.1-x64.exe"),
                good_sums.clone(),
            ),
            // kontrolne sucty z inej cesty
            (
                "v9.0.0",
                "transom-setup-9.0.0-x64.exe".into(),
                format!("{d}v9.0.0/transom-setup-9.0.0-x64.exe"),
                format!("{d}v9.0.0/../v0.0.1/SHA256SUMS.txt"),
            ),
            // znacka s lomkou alebo ../
            (
                "v9.0.0/..",
                "transom-setup-9.0.0-x64.exe".into(),
                format!("{d}v9.0.0/../transom-setup-9.0.0-x64.exe"),
                format!("{d}v9.0.0/../SHA256SUMS.txt"),
            ),
        ];
        for (tag, name, url, sums) in cases {
            let esc = |s: &str| s.replace('\\', r"\\");
            let json = one(tag, &esc(&name), &esc(&url), &sums);
            assert_eq!(pick(&json, "0.1.0").unwrap(), None, "{name} {url} {sums}");
        }
    }

    /// Stranka vydania sa otvara cez ShellExecute: `html_url` z odpovede sa nepouzije (ani s nasou predponou a `../`).
    #[test]
    fn the_release_page_is_built_from_the_validated_tag() {
        let d = DOWNLOAD_PREFIX;
        let json = format!(
            r#"[{{"tag_name":"v9.0.0","draft":false,"html_url":"https://github.com/Antive1/transom/../../evil/repo","assets":[
            {{"name":"transom-setup-9.0.0-x64.exe","browser_download_url":"{d}v9.0.0/transom-setup-9.0.0-x64.exe"}},
            {{"name":"SHA256SUMS.txt","browser_download_url":"{d}v9.0.0/SHA256SUMS.txt"}}]}}]"#
        );
        assert_eq!(
            pick(&json, "0.1.0").unwrap().map(|r| r.page),
            Some("https://github.com/Antive1/transom/releases/tag/v9.0.0".into())
        );
    }

    #[test]
    fn safe_segments_are_plain_file_names() {
        for ok in [
            "v0.2.0",
            "transom-setup-0.2.0-x64.exe",
            "SHA256SUMS.txt",
            "v1.0.0-rc_1",
        ] {
            assert!(safe_segment(ok), "{ok}");
        }
        for bad in [
            "", ".", "..", ".hidden", "a/b", r"a\b", "a:b", "a b", "a%2eb", "a?b", "a#b", "\u{e9}",
        ] {
            assert!(!safe_segment(bad), "{bad}");
        }
        assert!(!safe_segment(&"a".repeat(129)));
    }
}
