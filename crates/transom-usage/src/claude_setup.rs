//! Pripojenie Claude Code: bezpecne pridanie zdokumentovaneho `statusLine` do pouzivatelskych nastaveni Claude Code
//! (`%USERPROFILE%\.claude\settings.json`, alebo `CLAUDE_CONFIG_DIR`). Nic ine sa neupravuje a **nikdy** sa nic nerata z
//! prihlasovacich udajov ani prepisov rozhovorov.
//!
//! - **Textova uprava**: vklada/nahradza sa len hodnota kluca `statusLine`; zvysok suboru ostava bajt po bajte (poradie klucov,
//!   odsadenie, koncove riadky). Vysledok sa vzdy overi parserom: vsetky povodne kluce a hodnoty musia ostat rovnake.
//! - **Zaloha pred kazdou upravou** (`settings.json.transom-backup-<cas>`, nikdy sa neprepise), zaznam o tom, co sa zapisalo, v
//!   `%LOCALAPPDATA%\transom\claude-setup.json` a **vratenie spat** (celu zalohu, ak sa subor odvtedy nezmenil; inak chirurgicky
//!   odstranenie nasho kluca a obnova predosleho `statusLine`).
//! - Cudzi `statusLine` sa **neprepise bez vyslovneho rozhodnutia** (`StatusLineState::Other`).
//! - Prikaz: cesta s **lomkami** (Claude Code spusta prikaz cez Git Bash, ktory zjedava spatne lomky), v uvodzovkach, ak ma medzery.
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Subor pouzivatelskych nastaveni Claude Code.
pub fn settings_path() -> PathBuf {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".claude")
        });
    base.join("settings.json")
}

/// Zaznam o poslednom pripojeni (co sa zapisalo a kde je zaloha).
pub fn state_path() -> PathBuf {
    crate::cache::data_dir().join("claude-setup.json")
}

/// Znak, ktory shell (Git Bash) v necitovanom slove nemeni. Pismena a cislice vratane diakritiky su bezpecne.
fn shell_plain(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            '_' | '.' | '/' | ':' | '+' | '@' | '%' | '-' | ',' | '=' | '~'
        )
}

/// Prikaz pre `statusLine.command`: cesta k mostu s lomkami. V dvojitych uvodzovkach je, ak obsahuje medzeru alebo ine znaky,
/// ktore by shell vyhodnotil (`&`, `(`, `;`, `'`, ...); cesty bez takych znakov ostavaju presne ako doteraz.
pub fn command_for(exe: &Path) -> String {
    let p = exe.display().to_string().replace('\\', "/");
    if p.chars().all(shell_plain) {
        p
    } else {
        format!("\"{p}\"")
    }
}

/// Prikaz sa nesmie zapisat, ak by ho shell aj v dvojitych uvodzovkach vyhodnotil (`$`, spatny apostrof) alebo by obsahoval
/// riadkovy znak ci vnutornu uvodzovku. Vonkajsi par uvodzoviek (z `command_for`) je v poriadku.
pub fn command_is_safe(command: &str) -> bool {
    let inner = command
        .strip_prefix('"')
        .and_then(|c| c.strip_suffix('"'))
        .unwrap_or(command);
    !command.is_empty()
        && !inner
            .chars()
            .any(|c| matches!(c, '"' | '$' | '`' | '\n' | '\r' | '\0'))
}

/// Stav `statusLine` v nastaveniach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusLineState {
    /// Subor neexistuje.
    NoFile,
    /// Subor je platny, ale kluc `statusLine` nema.
    Absent,
    /// `statusLine` smeruje na `transom-statusline`.
    Ours,
    /// Iny prikaz (skratena ukazka na zobrazenie pouzivatelovi).
    Other(String),
    /// Subor nie je platny JSON objekt - nikdy sa nemeni.
    Invalid,
}

/// Skrati prikaz na ukazku (na zobrazenie pri rozhodovani; cely sa nikam nelogoval).
fn preview(cmd: &str) -> String {
    let c: String = cmd.chars().take(90).collect();
    if cmd.chars().count() > 90 {
        format!("{c}…")
    } else {
        c
    }
}

/// Prikaz `statusLine.command` zo suboru nastaveni (na kontrolu, ci most existuje).
pub fn current_command(text: &str) -> Option<String> {
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(strip_bom(text)) else {
        return None;
    };
    o.get("statusLine")?
        .get("command")?
        .as_str()
        .map(str::to_string)
}

/// Spustitelny subor z prikazu (bez uvodzoviek; prikaz mostu nema argumenty).
pub fn command_exe(cmd: &str) -> PathBuf {
    PathBuf::from(cmd.trim().trim_matches('"').trim_matches('\''))
}

/// Nas prikaz, ktory ale nevedie na `bridge` (neexistuje alebo je iny subor): treba pripojit znova.
pub fn is_outdated(cmd: &str, bridge: &Path) -> bool {
    let exe = command_exe(cmd);
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    is_our_bridge(cmd) && (!exe.exists() || norm(&exe) != norm(bridge))
}

/// Posudi obsah suboru (`None` = subor neexistuje).
pub fn inspect(text: Option<&str>) -> StatusLineState {
    let Some(text) = text else {
        return StatusLineState::NoFile;
    };
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(strip_bom(text)) else {
        return StatusLineState::Invalid;
    };
    match o.get("statusLine") {
        None => StatusLineState::Absent,
        Some(v) => {
            let cmd = v.get("command").and_then(Value::as_str).unwrap_or_default();
            if is_our_bridge(cmd) {
                StatusLineState::Ours
            } else {
                StatusLineState::Other(preview(cmd))
            }
        }
    }
}

fn strip_bom(t: &str) -> &str {
    t.strip_prefix('\u{feff}').unwrap_or(t)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// Nie je platny JSON objekt.
    Invalid,
    /// Po uprave sa nezhodovali povodne kluce (nemalo by nastat; nic sa nezapise).
    VerificationFailed,
    Io(String),
}

struct Member {
    key: String,
    key_start: usize,
    value_start: usize,
    value_end: usize,
}

struct Obj {
    close: usize,
    members: Vec<Member>,
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

/// Koniec JSON retazca zacinajuceho uvodzovkou na `i` (index za uzatvaracou uvodzovkou).
fn string_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// Koniec hodnoty zacinajucej na `i`.
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => string_end(b, i),
        b'{' | b'[' => {
            let mut depth = 0i32;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = string_end(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n')
            {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

/// Najvyssia uroven objektu: pozicie klucov a hodnot (text sa nemeni).
fn scan(text: &str) -> Option<Obj> {
    let b = text.as_bytes();
    let mut i = skip_ws(b, if text.starts_with('\u{feff}') { 3 } else { 0 });
    if *b.get(i)? != b'{' {
        return None;
    }
    i += 1;
    let mut members = Vec::new();
    loop {
        i = skip_ws(b, i);
        match *b.get(i)? {
            b'}' => return Some(Obj { close: i, members }),
            b'"' => {
                let key_start = i;
                let ke = string_end(b, i)?;
                let key: String = serde_json::from_str(&text[i..ke]).ok()?;
                i = skip_ws(b, ke);
                if *b.get(i)? != b':' {
                    return None;
                }
                let value_start = skip_ws(b, i + 1);
                let ve = value_end(b, value_start)?;
                members.push(Member {
                    key,
                    key_start,
                    value_start,
                    value_end: ve,
                });
                i = skip_ws(b, ve);
                match *b.get(i)? {
                    b',' => {
                        i = skip_ws(b, i + 1);
                        // zaverecna ciarka (`{"a":1,}`) nie je platny JSON
                        if *b.get(i)? != b'"' {
                            return None;
                        }
                    }
                    b'}' => {}
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
}

/// Odsadenie riadku, na ktorom zacina clen (prazdne, ak nezacina riadok).
fn line_indent(text: &str, member_start: usize) -> String {
    let before = &text[..member_start];
    let ls = before.rfind('\n').map_or(0, |n| n + 1);
    let lead = &before[ls..];
    if lead.chars().all(|c| c == ' ' || c == '\t') {
        lead.to_string()
    } else {
        String::new()
    }
}

/// Hodnota `statusLine` ako text so zachovanim odsadenia okolitych klucov.
pub fn statusline_value(command: &str, indent: &str) -> String {
    let cmd = serde_json::to_string(command).unwrap_or_default();
    if indent.is_empty() {
        format!("{{ \"type\": \"command\", \"command\": {cmd} }}")
    } else {
        format!("{{\n{indent}  \"type\": \"command\",\n{indent}  \"command\": {cmd}\n{indent}}}")
    }
}

fn eol(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Zapise `statusLine` s hodnotou `value_text` (uz platny JSON text hodnoty). `None` vstup = nov, subor. Ak kluc existuje,
/// nahradi sa (volajuci to smie urobit az po rozhodnuti pouzivatela). Vysledok sa overi parserom.
fn set_member(
    text: Option<&str>,
    value_for: &dyn Fn(&str) -> String,
) -> Result<String, SetupError> {
    set_member_key(text, "statusLine", value_for)
}

/// Ako [`set_member`], pre lubovolny kluc najvyssej urovne (`statusLine`, `hooks`).
fn set_member_key(
    text: Option<&str>,
    key: &str,
    value_for: &dyn Fn(&str) -> String,
) -> Result<String, SetupError> {
    let Some(text) = text else {
        let v = value_for("  ");
        let out = format!("{{\n  \"{key}\": {v}\n}}\n");
        return verify_keys(None, &out, &[key]).map(|_| out);
    };
    let obj = scan(text).ok_or(SetupError::Invalid)?;
    let mut out = text.to_string();
    if let Some(m) = obj.members.iter().find(|m| m.key == key) {
        let v = value_for(&line_indent(text, m.key_start));
        out.replace_range(m.value_start..m.value_end, &v);
    } else if let Some(last) = obj.members.last() {
        let indent = {
            let i = line_indent(text, obj.members[0].key_start);
            if i.is_empty() && !text[..obj.members[0].key_start].contains('\n') {
                String::new()
            } else {
                i
            }
        };
        let v = value_for(&indent);
        let nl = eol(text);
        let ins = if indent.is_empty() && !text[last.value_end..obj.close].contains('\n') {
            format!(", \"{key}\": {v}")
        } else {
            format!(",{nl}{indent}\"{key}\": {v}")
        };
        out.insert_str(last.value_end, &ins);
    } else {
        // prazdny objekt `{}`
        let v = value_for("  ");
        let nl = eol(text);
        out.insert_str(obj.close, &format!("{nl}  \"{key}\": {v}{nl}"));
    }
    verify_keys(Some(text), &out, &[key]).map(|_| out)
}

/// Overenie: vysledok je platny objekt a vsetky povodne kluce okrem `changed` maju rovnake hodnoty (a ich pocet sa
/// nezmenil). Pri `hooks` navyse ostanu vsetky cudzie hooky.
fn verify_keys(before: Option<&str>, after: &str, changed: &[&str]) -> Result<(), SetupError> {
    let Ok(Value::Object(a)) = serde_json::from_str::<Value>(strip_bom(after)) else {
        return Err(SetupError::VerificationFailed);
    };
    if let Some(b) = before {
        let Ok(Value::Object(b)) = serde_json::from_str::<Value>(strip_bom(b)) else {
            return Err(SetupError::Invalid);
        };
        for (k, v) in &b {
            if !changed.contains(&k.as_str()) && a.get(k) != Some(v) {
                return Err(SetupError::VerificationFailed);
            }
        }
        // pocet ostatnych klucov sa nesmie zmenit (pridanie/odstranenie menenych ich neovplyvni)
        let others = |m: &serde_json::Map<String, Value>| {
            m.keys().filter(|k| !changed.contains(&k.as_str())).count()
        };
        if others(&a) != others(&b) {
            return Err(SetupError::VerificationFailed);
        }
        if changed.contains(&"hooks") && !foreign_hooks_kept(b.get("hooks"), a.get("hooks")) {
            return Err(SetupError::VerificationFailed);
        }
    }
    Ok(())
}

/// Pridanie nasho `statusLine` (`command`); existujuci kluc sa nahradi.
pub fn apply(text: Option<&str>, command: &str) -> Result<String, SetupError> {
    // koniec riadkov podla suboru (CRLF zostane CRLF)
    let nl = text.map_or("\n", eol);
    let with_status = set_member(text, &|indent| {
        statusline_value(command, indent).replace('\n', nl)
    })?;
    // hooky (zaciatok / nastroj / koniec ulohy / otazka): presny stav prace v ostrove a oznamenia
    let existing = serde_json::from_str::<Value>(strip_bom(&with_status))
        .ok()
        .and_then(|v| v.get("hooks").cloned());
    let hooks = merged_hooks(existing.as_ref(), command);
    set_member_key(Some(&with_status), "hooks", &|indent| {
        json_with_indent(&hooks, indent).replace('\n', nl)
    })
}

/// Udalosti Claude Code, pri ktorych most zapise len druh udalosti (argument `--hook`).
pub const HOOK_EVENTS: [(&str, &str); 6] = [
    ("UserPromptSubmit", "start"),
    ("PreToolUse", "tool"),
    ("PermissionRequest", "ask"),
    ("Stop", "stop"),
    ("StopFailure", "error"),
    ("Notification", "notify"),
];

fn is_our_hook_entry(e: &Value) -> bool {
    e.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
        hs.iter().any(|h| {
            h.get("command")
                .and_then(Value::as_str)
                .is_some_and(is_our_bridge)
        })
    })
}

/// Hooky s nasimi zaznamami (stare nase sa nahradia, cudzie ostanu v povodnom poradi pred nasimi).
pub fn merged_hooks(existing: Option<&Value>, command: &str) -> Value {
    let mut obj = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (ev, arg) in HOOK_EVENTS {
        let mut arr: Vec<Value> = obj
            .get(ev)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        arr.retain(|e| !is_our_hook_entry(e));
        arr.push(serde_json::json!({
            "hooks": [{"type": "command", "command": format!("{command} --hook {arg}")}]
        }));
        obj.insert(ev.to_string(), Value::Array(arr));
    }
    Value::Object(obj)
}

/// Hooky bez nasich zaznamov (prazdne udalosti sa vynechaju); `None` = nic neostalo.
fn hooks_without_ours(existing: &Value) -> Option<Value> {
    let mut obj = existing.as_object()?.clone();
    for (ev, _) in HOOK_EVENTS {
        if let Some(Value::Array(arr)) = obj.get_mut(ev) {
            arr.retain(|e| !is_our_hook_entry(e));
            if arr.is_empty() {
                obj.remove(ev);
            }
        }
    }
    (!obj.is_empty()).then_some(Value::Object(obj))
}

/// Vsetky cudzie hooky z `before` su v `after` (nase sa smu pridat, nahradit alebo odobrat).
fn foreign_hooks_kept(before: Option<&Value>, after: Option<&Value>) -> bool {
    let Some(b) = before.and_then(Value::as_object) else {
        return true;
    };
    let a = after.and_then(Value::as_object);
    b.iter().all(|(ev, v)| match v.as_array() {
        Some(arr) => arr.iter().filter(|e| !is_our_hook_entry(e)).all(|e| {
            a.and_then(|a| a.get(ev))
                .and_then(Value::as_array)
                .is_some_and(|x| x.contains(e))
        }),
        None => a.and_then(|a| a.get(ev)) == Some(v),
    })
}

/// Su v nastaveniach nase hooky pre vsetky udalosti?
pub fn has_our_hooks(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(strip_bom(text)) else {
        return false;
    };
    HOOK_EVENTS.iter().all(|(ev, _)| {
        v.get("hooks")
            .and_then(|h| h.get(*ev))
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(is_our_hook_entry))
    })
}

/// JSON hodnota s odsadenim podla okolia (prvy riadok bez odsadenia).
fn json_with_indent(v: &Value, indent: &str) -> String {
    let pretty = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
    if indent.is_empty() {
        return serde_json::to_string(v).unwrap_or_else(|_| "{}".into());
    }
    pretty
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.to_string()
            } else {
                format!("{indent}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Obnova hodnoty `statusLine` z povodneho textu hodnoty (pri vrateni spat, ked sa subor zmenil).
fn apply_raw(text: &str, raw_value: &str) -> Result<String, SetupError> {
    set_member(Some(text), &|_| raw_value.to_string())
}

/// Odstrani kluc `statusLine` (len ak existuje); zvysok ostane.
pub fn remove(text: &str) -> Result<String, SetupError> {
    let t = remove_member(text, "statusLine")?;
    // nase hooky prec; cudzie ostanu (ak nic neostane, kluc `hooks` sa odstrani)
    let hooks = serde_json::from_str::<Value>(strip_bom(&t))
        .ok()
        .and_then(|v| v.get("hooks").cloned());
    match hooks {
        None => Ok(t),
        Some(h) => match hooks_without_ours(&h) {
            Some(rest) if rest != h => {
                let nl = eol(&t);
                set_member_key(Some(&t), "hooks", &|indent| {
                    json_with_indent(&rest, indent).replace('\n', nl)
                })
            }
            Some(_) => Ok(t),
            None => remove_member(&t, "hooks"),
        },
    }
}

/// Odstrani kluc najvyssej urovne (len ak existuje); zvysok ostane.
fn remove_member(text: &str, key: &str) -> Result<String, SetupError> {
    let obj = scan(text).ok_or(SetupError::Invalid)?;
    let Some(idx) = obj.members.iter().position(|m| m.key == key) else {
        return Ok(text.to_string());
    };
    let m = &obj.members[idx];
    let mut out = text.to_string();
    if idx > 0 {
        // od konca predchadzajuceho clena (vratane ciarky) po koniec tohto
        let prev_end = obj.members[idx - 1].value_end;
        out.replace_range(prev_end..m.value_end, "");
    } else if let Some(next) = obj.members.get(1) {
        // prvy clen: po zaciatok dalsieho (ciarka a medzery patria odstraneneho)
        out.replace_range(m.key_start..next.key_start, "");
    } else {
        // jediny clen: nechaj prazdny objekt
        out.replace_range(m.key_start..m.value_end, "");
    }
    verify_keys(Some(text), &out, &[key]).map(|_| out)
}

// ---- zaznam o pripojeni, zaloha, vratenie ----

/// Co sa naposledy zapisalo.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SetupRecord {
    /// Cesta k zalohe povodneho suboru (prazdne = subor pred tym neexistoval).
    pub backup: String,
    /// FNV-1a 64 hash zapisaneho obsahu (rozpozna, ci sa subor odvtedy zmenil).
    pub written_hash: u64,
    /// Povodny text hodnoty `statusLine` (ak sa nahradil cudzi), na obnovu.
    pub previous_value: Option<String>,
    pub at: i64,
}

pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Povodny text hodnoty `statusLine` (ak existuje).
fn previous_value(text: &str) -> Option<String> {
    let obj = scan(text)?;
    obj.members
        .iter()
        .find(|m| m.key == "statusLine")
        .map(|m| text[m.value_start..m.value_end].to_string())
}

fn backup_prefix(settings: &Path) -> String {
    let name = settings
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "settings.json".into());
    format!("{name}.transom-backup-")
}

/// Zaloha povodnych bajtov do noveho suboru `settings.json.transom-backup-<cas>[-N]` vedla neho. Subor sa vytvara
/// `create_new`: existujuca zaloha ani odkaz podstrceny na jej meno sa nikdy neprepise ani nenasleduje.
fn write_backup(settings: &Path, now: i64, bytes: &[u8]) -> Result<PathBuf, SetupError> {
    use std::io::Write;
    let dir = settings.parent().unwrap_or_else(|| Path::new("."));
    let prefix = backup_prefix(settings);
    for n in 0..1000 {
        let p = if n == 0 {
            dir.join(format!("{prefix}{now}"))
        } else {
            dir.join(format!("{prefix}{now}-{n}"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
        {
            Ok(mut f) => {
                let done = f.write_all(bytes).and_then(|()| f.sync_all());
                drop(f);
                return match done {
                    Ok(()) => Ok(p),
                    Err(e) => {
                        let _ = std::fs::remove_file(&p);
                        Err(SetupError::Io(e.to_string()))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(SetupError::Io(e.to_string())),
        }
    }
    Err(SetupError::Io("no free backup name".into()))
}

/// Cesta zalohy zo zaznamu (subor v datach transom) je naozaj nasa zaloha vedla `settings`: ten isty priecinok a nazov
/// `<settings>.transom-backup-<cislice>[-<cislice>]`. Inak by poskodeny alebo podvrhnuty zaznam mohol skopirovat lubovolny
/// subor na miesto `settings.json`.
fn is_our_backup(settings: &Path, backup: &Path) -> bool {
    let same_dir = match (settings.parent(), backup.parent()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    let Some(rest) = backup
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(&backup_prefix(settings)))
    else {
        return false;
    };
    let mut parts = rest.splitn(2, '-');
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    same_dir && parts.next().is_some_and(digits) && parts.next().is_none_or(digits)
}

/// Pripoji: zaloha, zapis (atomicky cez docasny subor), zaznam. `replace_existing` musi byt `true`, ak je v subore cudzi
/// `statusLine` a pouzivatel vyslovne zvolil nahradenie. Vrati zaznam.
pub fn connect(
    settings: &Path,
    record_path: &Path,
    command: &str,
    replace_existing: bool,
    now: i64,
) -> Result<SetupRecord, SetupError> {
    if !command_is_safe(command) {
        return Err(SetupError::Io(
            "unsafe characters in the bridge path".into(),
        ));
    }
    // striktne UTF-8: nevalidny subor sa nikdy nepreklada do U+FFFD a neprepisuje
    let text = match std::fs::read(settings) {
        Ok(b) => Some(String::from_utf8(b).map_err(|_| SetupError::Invalid)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(SetupError::Io(e.to_string())),
    };
    match inspect(text.as_deref()) {
        StatusLineState::Invalid => return Err(SetupError::Invalid),
        StatusLineState::Other(_) if !replace_existing => {
            return Err(SetupError::Io("existing statusLine not confirmed".into()))
        }
        _ => {}
    }
    let new_text = apply(text.as_deref(), command)?;
    let backup = match &text {
        Some(t) => write_backup(settings, now, t.as_bytes())?
            .display()
            .to_string(),
        None => String::new(),
    };
    // atomicky cez docasny subor s jedinecnym nazvom (`create_new`: existujuci odkaz na tom mieste sa nenasleduje)
    crate::cache::write_atomic(settings, new_text.as_bytes())
        .map_err(|e| SetupError::Io(e.to_string()))?;
    let rec = SetupRecord {
        backup,
        written_hash: fnv(&new_text),
        previous_value: text
            .as_deref()
            .and_then(|t| previous_value(t).filter(|v| !is_our_bridge(v))),
        at: now,
    };
    if let Some(d) = record_path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(record_path, serde_json::to_string(&rec).unwrap_or_default());
    Ok(rec)
}

/// Co spravi vratenie spat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rollback {
    /// Subor je taky, aky sme zapisali: obnovena cela zaloha (alebo subor odstraneny, ak pred tym neexistoval).
    RestoredBackup,
    /// Subor sa odvtedy zmenil: odstraneny len nas `statusLine` (a obnoveny predosly, ak bol).
    RemovedOurEntry,
    /// Nie je co vracat (nas zaznam chyba).
    NothingToRollBack,
}

pub fn rollback(settings: &Path, record_path: &Path) -> Result<Rollback, SetupError> {
    let Ok(raw) = std::fs::read_to_string(record_path) else {
        return Ok(Rollback::NothingToRollBack);
    };
    let rec: SetupRecord = serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
    let current = match std::fs::read(settings) {
        Ok(b) => Some(String::from_utf8(b).map_err(|_| SetupError::Invalid)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(SetupError::Io(e.to_string())),
    };
    let unchanged = current.as_deref().map(fnv) == Some(rec.written_hash);
    let result = if unchanged {
        if rec.backup.is_empty() {
            std::fs::remove_file(settings).map_err(|e| SetupError::Io(e.to_string()))?;
        } else {
            if !is_our_backup(settings, Path::new(&rec.backup)) {
                return Err(SetupError::Invalid);
            }
            // atomicky: obsah zalohy cez docasny subor a premenovanie (nikdy nie napoly zapisany settings.json)
            let bytes = std::fs::read(&rec.backup).map_err(|e| SetupError::Io(e.to_string()))?;
            crate::cache::write_atomic(settings, &bytes)
                .map_err(|e| SetupError::Io(e.to_string()))?;
        }
        Rollback::RestoredBackup
    } else if let Some(cur) = current {
        let mut t = remove(&cur)?;
        if let Some(prev) = &rec.previous_value {
            t = apply_raw(&t, prev)?;
        }
        crate::cache::write_atomic(settings, t.as_bytes())
            .map_err(|e| SetupError::Io(e.to_string()))?;
        Rollback::RemovedOurEntry
    } else {
        Rollback::NothingToRollBack
    };
    let _ = std::fs::remove_file(record_path);
    Ok(result)
}

// ---- stav pripojenia ----

/// Celkovy stav pre nastavenia.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    /// `statusLine` nie je nastaveny (alebo subor neexistuje).
    NotConfigured,
    /// Nastaveny je cudzi `statusLine` (ukazka prikazu).
    Foreign(String),
    /// Nastaveny nas, ale zatial ziadny snimok (Claude Code este neodpovedal / neprebehla relacia).
    ConfiguredNoSnapshot,
    /// Zaznamy prichadzaju; `limits` = aspon jeden niesol limity planu.
    Connected { limits: bool },
    /// Nas `statusLine`, ale prikaz smeruje na most, ktory uz neexistuje alebo je zo starsej kopie (napr. pred
    /// premenovanim na transom): Claude Code ho nevie spustit a zaznamy neprichadzaju. Treba pripojit znova.
    Outdated(String),
    /// Subor nastaveni nie je citatelny (poskodeny JSON, pristup).
    Error(String),
}

/// Zaznamy mostu: (pocet, najnovsi cas, aspon jeden niesol limity).
pub fn bridge_info(path: &Path) -> (usize, Option<i64>, bool) {
    // cita sa len koniec suboru (subor rastie; staci posledny usek)
    const TAIL: u64 = 512 * 1024;
    let Ok(mut f) = std::fs::File::open(path) else {
        return (0, None, false);
    };
    let len = f.metadata().map_or(0, |m| m.len());
    let mut bytes = Vec::new();
    {
        use std::io::{Read, Seek, SeekFrom};
        if len > TAIL {
            let _ = f.seek(SeekFrom::Start(len - TAIL));
        }
        if f.read_to_end(&mut bytes).is_err() {
            return (0, None, false);
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (mut n, mut newest, mut limits) = (0usize, None::<i64>, false);
    for l in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        n += 1;
        if let Some(ts) = v.get("ts").and_then(Value::as_i64) {
            newest = Some(newest.map_or(ts, |o| o.max(ts)));
        }
        limits |= v.get("rl5").is_some_and(|x| !x.is_null())
            || v.get("rl7").is_some_and(|x| !x.is_null());
    }
    (n, newest, limits)
}

pub fn connection(state: &StatusLineState, records: usize, limits: bool) -> Connection {
    match state {
        StatusLineState::NoFile | StatusLineState::Absent => Connection::NotConfigured,
        StatusLineState::Invalid => Connection::Error("settings.json".into()),
        StatusLineState::Other(c) => Connection::Foreign(c.clone()),
        StatusLineState::Ours if records == 0 => Connection::ConfiguredNoSnapshot,
        StatusLineState::Ours => Connection::Connected { limits },
    }
}

/// `statusLine` smeruje na nas most `transom-statusline` (subor Claude Code sa nikdy neupravuje sam od seba).
pub fn is_our_bridge(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    c.contains("transom-statusline")
}

/// Snimka mostu zapisana procesom Claude Code **v presmerovanom profile aplikacie z obchodu** (MSIX Desktop): existuje len
/// subor; vrati cestu a cas zmeny (Unix sekundy). Len diagnostika - obsah sa necita.
pub fn redirected_bridge_file() -> Option<(PathBuf, i64)> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    for e in std::fs::read_dir(local.join("Packages")).ok()?.flatten() {
        if !e.file_name().to_string_lossy().starts_with("Claude_") {
            continue;
        }
        let f = e
            .path()
            .join("LocalCache")
            .join("Local")
            .join("transom")
            .join("claude-statusline.jsonl");
        if let Ok(m) = std::fs::metadata(&f) {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            return Some((f, t));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "C:/transom/target/release/transom-statusline.exe";

    fn user_file() -> String {
        // formatovanie pouzivatela: 2 medzery, poradie klucov nie je abecedne, vnorene polia, koncovy novy riadok
        "{\n  \"model\": \"opus\",\n  \"enabledPlugins\": {\n    \"a@b\": true,\n    \"c@d\": false\n  },\n  \"theme\": \"dark\",\n  \"list\": [1, 2, {\"x\": \"}\"}]\n}\n".to_string()
    }

    fn same_except_statusline(a: &str, b: &str) {
        let (Value::Object(mut x), Value::Object(mut y)) = (
            serde_json::from_str::<Value>(a).unwrap(),
            serde_json::from_str::<Value>(b).unwrap(),
        ) else {
            panic!()
        };
        // nase hooky pribudaju spolu so `statusLine`
        for k in ["statusLine", "hooks"] {
            x.remove(k);
            y.remove(k);
        }
        assert_eq!(x, y);
    }

    #[test]
    fn command_uses_forward_slashes_and_quotes_paths_with_spaces() {
        assert_eq!(
            command_for(Path::new(
                r"C:\transom\target\release\transom-statusline.exe"
            )),
            CMD
        );
        assert_eq!(
            command_for(Path::new(
                r"C:\Program Files\transom\transom-statusline.exe"
            )),
            "\"C:/Program Files/transom/transom-statusline.exe\""
        );
        // v JSON-e sa uvodzovky escapuju, po dekodovani je prikaz presne taky
        let v = statusline_value("\"C:/P F/x.exe\"", "");
        let parsed: Value = serde_json::from_str(&v).unwrap();
        assert_eq!(parsed["command"], "\"C:/P F/x.exe\"");
    }

    #[test]
    fn inspect_distinguishes_all_states() {
        assert_eq!(inspect(None), StatusLineState::NoFile);
        assert_eq!(inspect(Some(&user_file())), StatusLineState::Absent);
        assert_eq!(inspect(Some("{ not json")), StatusLineState::Invalid);
        assert_eq!(inspect(Some("[1,2]")), StatusLineState::Invalid);
        let ours = format!("{{\"statusLine\": {{\"type\":\"command\",\"command\":\"{CMD}\"}}}}");
        assert_eq!(inspect(Some(&ours)), StatusLineState::Ours);
        let other = r#"{"statusLine": {"type":"command","command":"~/.claude/my-line.sh"}}"#;
        assert_eq!(
            inspect(Some(other)),
            StatusLineState::Other("~/.claude/my-line.sh".into())
        );
        // BOM neprekaza
        assert_eq!(inspect(Some("\u{feff}{}")), StatusLineState::Absent);
    }

    #[test]
    fn adding_keeps_every_other_byte_and_key_order() {
        let before = user_file();
        let after = apply(Some(&before), CMD).unwrap();
        // povodny text je predpona (okrem koncoveho `\n}\n`): nic sa neprepisalo ani neprefarbilo
        let cut = before.rfind("\n}").unwrap();
        assert!(after.starts_with(&before[..cut]), "{after}");
        assert!(after.ends_with("\n}\n"));
        same_except_statusline(&before, &after);
        assert_eq!(inspect(Some(&after)), StatusLineState::Ours);
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["statusLine"]["type"], "command");
        assert_eq!(v["statusLine"]["command"], CMD);
        // poradie klucov v texte
        let pos = |k: &str| after.find(&format!("\"{k}\"")).unwrap();
        assert!(
            pos("model") < pos("enabledPlugins")
                && pos("theme") < pos("list")
                && pos("list") < pos("statusLine")
        );
    }

    #[test]
    fn adding_works_for_empty_object_missing_file_compact_json_and_crlf() {
        let created = apply(None, CMD).unwrap();
        assert_eq!(inspect(Some(&created)), StatusLineState::Ours);
        let empty = apply(Some("{}"), CMD).unwrap();
        assert_eq!(inspect(Some(&empty)), StatusLineState::Ours);
        let empty2 = apply(Some("{\n}\n"), CMD).unwrap();
        assert_eq!(inspect(Some(&empty2)), StatusLineState::Ours);
        let compact = apply(Some(r#"{"a":1,"b":[1,2]}"#), CMD).unwrap();
        same_except_statusline(r#"{"a":1,"b":[1,2]}"#, &compact);
        assert!(compact.starts_with(r#"{"a":1,"b":[1,2]"#));
        let crlf = "{\r\n  \"a\": 1\r\n}\r\n";
        let out = apply(Some(crlf), CMD).unwrap();
        assert!(out.contains("1,\r\n  \"statusLine\""), "{out:?}");
        assert!(
            !out.replace("\r\n", "").contains('\n'),
            "ziadne holé \\n medzi \\r\\n"
        );
        // BOM sa zachova
        let bom = apply(Some("\u{feff}{\n  \"a\": 1\n}"), CMD).unwrap();
        assert!(bom.starts_with('\u{feff}'));
    }

    #[test]
    fn invalid_files_are_never_touched() {
        for bad in ["{ not json", "[1]", "", "{\"a\": 1,}", "{\"a\" 1}"] {
            assert_eq!(apply(Some(bad), CMD), Err(SetupError::Invalid), "{bad:?}");
        }
    }

    #[test]
    fn replacing_and_removing_a_statusline_keeps_neighbours() {
        let with_other = "{\n  \"a\": 1,\n  \"statusLine\": {\n    \"type\": \"command\",\n    \"command\": \"my.sh\"\n  },\n  \"z\": \"}\"\n}\n";
        assert_eq!(
            inspect(Some(with_other)),
            StatusLineState::Other("my.sh".into())
        );
        let replaced = apply(Some(with_other), CMD).unwrap();
        same_except_statusline(with_other, &replaced);
        assert_eq!(inspect(Some(&replaced)), StatusLineState::Ours);
        assert!(replaced.contains("\"z\": \"}\""), "{replaced}");
        // odstranenie: vo vsetkych polohach clena (prvy, stredny, posledny, jediny)
        for (src, expect_keys) in [
            ("{\"statusLine\":{\"command\":\"w\"},\"a\":1}", vec!["a"]),
            (
                "{\"a\":1,\"statusLine\":{\"command\":\"w\"},\"b\":2}",
                vec!["a", "b"],
            ),
            (
                "{\n  \"a\": 1,\n  \"statusLine\": {\"command\": \"w\"}\n}\n",
                vec!["a"],
            ),
            ("{\"statusLine\":{\"command\":\"w\"}}", vec![]),
        ] {
            let out = remove(src).unwrap();
            let Value::Object(o) = serde_json::from_str::<Value>(&out).unwrap() else {
                panic!()
            };
            assert_eq!(
                o.keys().cloned().collect::<Vec<_>>(),
                expect_keys,
                "{src} -> {out}"
            );
        }
        assert_eq!(remove("{\"a\":1}").unwrap(), "{\"a\":1}");
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("transom-cs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn connect_backs_up_writes_and_rollback_restores_the_exact_original() {
        let d = tmp("roundtrip");
        let (s, r) = (d.join("settings.json"), d.join("claude-setup.json"));
        let original = user_file();
        std::fs::write(&s, &original).unwrap();
        let rec = connect(&s, &r, CMD, false, 1_700_000_000).unwrap();
        assert!(Path::new(&rec.backup).exists());
        assert_eq!(
            std::fs::read_to_string(&rec.backup).unwrap(),
            original,
            "zaloha = povodny subor"
        );
        let now = std::fs::read_to_string(&s).unwrap();
        same_except_statusline(&original, &now);
        assert_eq!(inspect(Some(&now)), StatusLineState::Ours);
        // vratenie: presne povodny obsah, zaloha ostava
        assert_eq!(rollback(&s, &r), Ok(Rollback::RestoredBackup));
        assert_eq!(std::fs::read_to_string(&s).unwrap(), original);
        assert!(Path::new(&rec.backup).exists());
        assert_eq!(rollback(&s, &r), Ok(Rollback::NothingToRollBack));
        // druha zaloha nikdy neprepise prvu
        let a = connect(&s, &r, CMD, false, 1_700_000_000).unwrap();
        std::fs::write(&s, &original).unwrap();
        let b = connect(&s, &r, CMD, false, 1_700_000_000).unwrap();
        assert_ne!(a.backup, b.backup);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_existing_file_at_the_backup_name_is_never_overwritten_and_no_temp_file_is_left() {
        let d = tmp("backup-taken");
        let (s, r) = (d.join("settings.json"), d.join("claude-setup.json"));
        std::fs::write(&s, user_file()).unwrap();
        let taken = d.join("settings.json.transom-backup-1700000000");
        std::fs::write(&taken, b"cudzi").unwrap();
        let rec = connect(&s, &r, CMD, false, 1_700_000_000).unwrap();
        assert_eq!(std::fs::read(&taken).unwrap(), b"cudzi");
        assert!(
            rec.backup
                .ends_with("settings.json.transom-backup-1700000000-1"),
            "{}",
            rec.backup
        );
        assert_eq!(std::fs::read_to_string(&rec.backup).unwrap(), user_file());
        let leftovers: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Zaznam o pripojeni lezi v datach transom; podvrhnuta cesta zalohy nesmie skopirovat cudzi subor na miesto
    /// `settings.json`.
    #[test]
    fn rollback_only_restores_our_own_backup_next_to_the_settings() {
        let d = tmp("forged");
        let (s, r) = (d.join("settings.json"), d.join("claude-setup.json"));
        std::fs::write(&s, user_file()).unwrap();
        let rec = connect(&s, &r, CMD, false, 1_700_000_000).unwrap();
        let written = std::fs::read_to_string(&s).unwrap();
        let other = d.join("other.txt");
        std::fs::write(&other, b"{}").unwrap();
        let elsewhere = d.join("sub").join("settings.json.transom-backup-1");
        for forged in [&other, &elsewhere] {
            let bad = SetupRecord {
                backup: forged.display().to_string(),
                ..rec.clone()
            };
            std::fs::write(&r, serde_json::to_string(&bad).unwrap()).unwrap();
            assert_eq!(rollback(&s, &r), Err(SetupError::Invalid));
            assert_eq!(
                std::fs::read_to_string(&s).unwrap(),
                written,
                "subor nezmeneny"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn backup_names_are_recognised_exactly() {
        let s = Path::new(r"C:\u\.claude\settings.json");
        let dir = Path::new(r"C:\u\.claude");
        for ok in [
            "settings.json.transom-backup-1700000000",
            "settings.json.transom-backup-1700000000-12",
        ] {
            assert!(is_our_backup(s, &dir.join(ok)), "{ok}");
        }
        for bad in [
            "settings.json",
            "settings.json.transom-backup-",
            "settings.json.transom-backup-x",
            "settings.json.transom-backup-1-",
            "settings.json.transom-backup-1-2-3",
            r"settings.json.transom-backup-1\..\x",
            "other.json.transom-backup-1",
        ] {
            assert!(!is_our_backup(s, &dir.join(bad)), "{bad}");
        }
        assert!(!is_our_backup(
            s,
            Path::new(r"C:\elsewhere\settings.json.transom-backup-1")
        ));
    }

    #[test]
    fn non_utf8_settings_are_never_rewritten_or_backed_up() {
        let d = tmp("non-utf8");
        let (s, r) = (d.join("settings.json"), d.join("claude-setup.json"));
        // platny JSON, ale hodnota obsahuje bajt 0xFF (nie UTF-8): lossy citanie by ho potichu zmenilo na U+FFFD
        let mut bytes = b"{\n  \"note\": \"caf".to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice(b"\"\n}\n");
        std::fs::write(&s, &bytes).unwrap();
        assert_eq!(connect(&s, &r, CMD, false, 7), Err(SetupError::Invalid));
        assert_eq!(
            std::fs::read(&s).unwrap(),
            bytes,
            "subor ostal bajt po bajte"
        );
        let backups = std::fs::read_dir(&d)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("backup")
            })
            .count();
        assert_eq!(backups, 0, "ziadna zaloha ani zaznam");
        assert!(!r.exists());
        // vratenie po tom, co sa subor stal nevalidnym, ho tiez nechava nedotknuty
        std::fs::write(&s, "{\n  \"a\": 1\n}\n").unwrap();
        connect(&s, &r, CMD, false, 8).unwrap();
        let mut broken = std::fs::read(&s).unwrap();
        broken.push(0xFE);
        std::fs::write(&s, &broken).unwrap();
        assert_eq!(rollback(&s, &r), Err(SetupError::Invalid));
        assert_eq!(std::fs::read(&s).unwrap(), broken);
        assert!(r.exists(), "zaznam ostava, vratenie sa da zopakovat");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn command_quotes_every_path_the_shell_would_interpret() {
        // beznu cestu (aj s diakritikou a 8.3 menami) nemenime: uz pripojene instalacie ostanu rozpoznane
        for plain in [
            r"C:\transom\target\release\transom-statusline.exe",
            r"C:\Users\Zoë\transom_1.2+x\transom-statusline.exe",
            r"C:\Users\USER~1\transom-statusline.exe",
        ] {
            let c = command_for(Path::new(plain));
            assert_eq!(c, plain.replace('\\', "/"), "{plain}");
            assert!(command_is_safe(&c));
        }
        // ostatne sa dava do dvojitych uvodzoviek; vnutri nic dalsie shell nevyhodnoti
        for (path, expected) in [
            (r"C:\Program Files\w\b.exe", "\"C:/Program Files/w/b.exe\""),
            (r"C:\a&b\b.exe", "\"C:/a&b/b.exe\""),
            (r"C:\O'Neil\b.exe", "\"C:/O'Neil/b.exe\""),
            (
                r"C:\Program Files (x86)\w\b.exe",
                "\"C:/Program Files (x86)/w/b.exe\"",
            ),
            (r"C:\a;b\b.exe", "\"C:/a;b/b.exe\""),
        ] {
            let c = command_for(Path::new(path));
            assert_eq!(c, expected);
            assert!(command_is_safe(&c), "{c}");
        }
        // co ani uvodzovky nezachrania, sa odmietne
        for bad in [r"C:\a$HOME\b.exe", r"C:\a`id`\b.exe", "C:\\a\nb\\b.exe"] {
            assert!(!command_is_safe(&command_for(Path::new(bad))), "{bad:?}");
        }
        assert!(!command_is_safe(""));
        assert!(!command_is_safe("\"C:/a\"b/x.exe\""), "vnutorna uvodzovka");
        // connect taky prikaz nezapise a nic nezmeni
        let d = tmp("unsafe-cmd");
        let (s, r) = (d.join("settings.json"), d.join("claude-setup.json"));
        std::fs::write(&s, "{\n  \"a\": 1\n}\n").unwrap();
        let bad = command_for(Path::new(r"C:\a$(touch x)\b.exe"));
        assert!(matches!(
            connect(&s, &r, &bad, false, 9),
            Err(SetupError::Io(_))
        ));
        assert_eq!(std::fs::read_to_string(&s).unwrap(), "{\n  \"a\": 1\n}\n");
        assert!(!r.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_settings_file_is_created_and_rollback_removes_it() {
        let d = tmp("missing");
        let (s, r) = (
            d.join("sub").join("settings.json"),
            d.join("claude-setup.json"),
        );
        let rec = connect(&s, &r, CMD, false, 5).unwrap();
        assert_eq!(rec.backup, "");
        assert_eq!(
            inspect(std::fs::read_to_string(&s).ok().as_deref()),
            StatusLineState::Ours
        );
        assert_eq!(rollback(&s, &r), Ok(Rollback::RestoredBackup));
        assert!(!s.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn foreign_statusline_needs_an_explicit_decision_and_is_restored_on_rollback() {
        let d = tmp("foreign");
        let (s, r) = (d.join("settings.json"), d.join("claude-setup.json"));
        let original = "{\n  \"a\": 1,\n  \"statusLine\": { \"type\": \"command\", \"command\": \"mine.sh\" }\n}\n";
        std::fs::write(&s, original).unwrap();
        // bez potvrdenia sa nic nezmeni
        assert!(connect(&s, &r, CMD, false, 9).is_err());
        assert_eq!(std::fs::read_to_string(&s).unwrap(), original);
        assert!(!r.exists());
        // s potvrdenim: nahradene a predosla hodnota ulozena
        let rec = connect(&s, &r, CMD, true, 9).unwrap();
        assert!(rec
            .previous_value
            .as_deref()
            .is_some_and(|p| p.contains("mine.sh")));
        // pouzivatel medzitym pridal dalsi kluc: vratenie nesmie ho zahodit
        let edited = std::fs::read_to_string(&s)
            .unwrap()
            .replace("\"a\": 1,", "\"a\": 1,\n  \"later\": true,");
        std::fs::write(&s, &edited).unwrap();
        assert_eq!(rollback(&s, &r), Ok(Rollback::RemovedOurEntry));
        let after = std::fs::read_to_string(&s).unwrap();
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["later"], true, "{after}");
        assert_eq!(
            v["statusLine"]["command"], "mine.sh",
            "predosly statusLine obnoveny"
        );
        // invalidny subor sa nikdy neupravuje
        std::fs::write(&s, "{ bad").unwrap();
        assert_eq!(connect(&s, &r, CMD, true, 10), Err(SetupError::Invalid));
        assert_eq!(std::fs::read_to_string(&s).unwrap(), "{ bad");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn connection_states_follow_settings_and_snapshots() {
        use Connection::*;
        assert_eq!(
            connection(&StatusLineState::NoFile, 0, false),
            NotConfigured
        );
        assert_eq!(connection(&StatusLineState::Absent, 5, true), NotConfigured);
        assert_eq!(
            connection(&StatusLineState::Other("x".into()), 0, false),
            Foreign("x".into())
        );
        assert_eq!(
            connection(&StatusLineState::Ours, 0, false),
            ConfiguredNoSnapshot
        );
        assert_eq!(
            connection(&StatusLineState::Ours, 3, false),
            Connected { limits: false }
        );
        assert_eq!(
            connection(&StatusLineState::Ours, 3, true),
            Connected { limits: true }
        );
        assert!(matches!(
            connection(&StatusLineState::Invalid, 0, false),
            Error(_)
        ));
    }

    #[test]
    fn bridge_info_counts_records_and_detects_limits_without_reading_anything_else() {
        let d = tmp("bridge");
        let f = d.join("b.jsonl");
        std::fs::write(
            &f,
            "{\"ts\":10,\"rl5\":null,\"cost\":1.0}\nnot json\n{\"ts\":20,\"rl5\":62.0}\n",
        )
        .unwrap();
        assert_eq!(bridge_info(&f), (2, Some(20), true));
        std::fs::write(&f, "{\"ts\":10,\"cost\":1.0,\"rl5\":null,\"rl7\":null}\n").unwrap();
        assert_eq!(bridge_info(&f), (1, Some(10), false));
        assert_eq!(bridge_info(&d.join("none")), (0, None, false));
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn the_bridge_name_is_recognised_as_ours() {
        assert!(is_our_bridge(
            "C:/Projects/transom/target/release/transom-statusline.exe"
        ));
        assert!(is_our_bridge("C:/x/TRANSOM-STATUSLINE.exe"));
        assert!(!is_our_bridge("npx ccstatusline"));
    }

    #[test]
    fn a_bridge_from_an_old_or_missing_copy_is_outdated() {
        let dir = std::env::temp_dir().join(format!("transom-cc-outdated-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let now = dir.join("transom-statusline.exe");
        std::fs::write(&now, b"x").unwrap();
        let cmd = command_for(&now);
        assert!(!is_outdated(&cmd, &now));
        assert!(is_outdated("C:/gone/transom/transom-statusline.exe", &now));
        let other = dir.join("old").join("transom-statusline.exe");
        let _ = std::fs::create_dir_all(other.parent().unwrap());
        std::fs::write(&other, b"x").unwrap();
        assert!(is_outdated(&command_for(&other), &now), "ina kopia");
        assert!(
            !is_outdated("C:/tools/other-statusline.exe", &now),
            "cudzi prikaz nie je nas"
        );
        assert_eq!(
            current_command(r#"{"statusLine": {"type": "command", "command": "C:/a b/x.exe"}}"#)
                .as_deref(),
            Some("C:/a b/x.exe")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hooks_are_added_next_to_foreign_ones_replaced_on_reconnect_and_removed_cleanly() {
        let mine = "C:/x/transom-statusline.exe";
        let foreign = r#"{
  "model": "opus",
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "notify-send done" } ] }
    ]
  }
}
"#;
        let t1 = apply(Some(foreign), mine).unwrap();
        assert!(has_our_hooks(&t1));
        let v: Value = serde_json::from_str(&t1).unwrap();
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "cudzi hook ostal, nas pribudol");
        assert!(stop[0]["hooks"][0]["command"] == "notify-send done");
        // znovu pripojit: nase sa nenasobia
        let t2 = apply(Some(&t1), "C:/y/transom-statusline.exe").unwrap();
        let v2: Value = serde_json::from_str(&t2).unwrap();
        assert_eq!(v2["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(t2.contains("C:/y/transom-statusline.exe --hook stop"));
        assert!(!t2.contains("C:/x/"));
        // odpojit: ostane len cudzi hook a model
        let t3 = remove(&t2).unwrap();
        let v3: Value = serde_json::from_str(&t3).unwrap();
        assert!(v3.get("statusLine").is_none());
        assert_eq!(v3["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(v3["hooks"].get("PreToolUse").is_none());
        assert_eq!(v3["model"], "opus");
        // bez cudzich hookov: kluc `hooks` po odpojeni zmizne
        let t4 = remove(&apply(Some(&user_file()), mine).unwrap()).unwrap();
        assert!(!t4.contains("\"hooks\""), "{t4}");
        same_except_statusline(&user_file(), &t4);
    }
}
