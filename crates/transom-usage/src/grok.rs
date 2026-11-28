//! Lokalne relacie xAI Grok CLI: `<grok_home>/sessions/<priecinok>/<relacia>/usage.json` (`grok_home` = `GROK_HOME` alebo
//! `~/.grok`). **Nezdokumentovany format** (overeny podla parsera CodexBar): `usage.json` = `{"turns": [{"endedAt",
//! "totalTokens", "modelUsage": {model: ...}, "primaryModelId"}]}`; ak chyba, `signals.json` =
//! `totalTokensBeforeCompaction + contextTokensUsed` (cas = cas upravy suboru). Grok lokalne nezapisuje rozdelenie na vstup a
//! vystup ani limity planu (tie su len na serveri), preto sa uklada len suct tokenov a cena sa neodhaduje.
use crate::{Event, Provider, SourceError, SourceKind, SourceStatus};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Priecinky relacii: presne dve urovne pod `sessions` (priecinok projektu, relacia).
fn session_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else {
        return out;
    };
    for cwd in rd.flatten().filter(|e| e.path().is_dir()) {
        if let Ok(rd2) = std::fs::read_dir(cwd.path()) {
            out.extend(rd2.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    out
}

/// Udalosti z `usage.json` (kazde kolo jedna udalost).
pub fn parse_usage(text: &str) -> Vec<Event> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(turns) = v.get("turns").and_then(Value::as_array) else {
        return Vec::new();
    };
    turns
        .iter()
        .filter_map(|t| {
            let ts = crate::parse_iso_utc(t.get("endedAt")?.as_str()?)?;
            let total = t.get("totalTokens")?.as_u64()?;
            let model = t
                .get("modelUsage")
                .and_then(Value::as_object)
                .and_then(|m| m.keys().next().cloned())
                .or_else(|| {
                    t.get("primaryModelId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            Some(event(model, ts, total))
        })
        .collect()
}

/// Udalost zo `signals.json` (cela relacia, cas = cas upravy suboru).
pub fn parse_signals(text: &str, mtime: i64) -> Option<Event> {
    let v: Value = serde_json::from_str(text).ok()?;
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let total = n("totalTokensBeforeCompaction") + n("contextTokensUsed");
    if total == 0 {
        return None;
    }
    let model = v
        .get("primaryModelId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(event(model, mtime, total))
}

fn event(model: String, ts: i64, total: u64) -> Event {
    Event {
        provider: Some(Provider::Grok),
        model,
        ts,
        // len suct tokenov (rozdelenie Grok lokalne nezapisuje)
        input: total,
        // cena sa neodhaduje: cennik Groku nepozname a rozdelenie na vstup/vystup chyba
        nonstandard: true,
        ..Event::default()
    }
}

pub fn read_dir(grok_home: &Path) -> (Vec<Event>, SourceStatus) {
    let root = grok_home.join("sessions");
    let dirs = session_dirs(&root);
    let mut events = Vec::new();
    let mut files = 0;
    let mut errors = 0;
    for d in &dirs {
        let usage = d.join("usage.json");
        let from_usage = crate::read_capped(&usage, crate::MAX_LOG_FILE_BYTES)
            .map(|b| parse_usage(&String::from_utf8_lossy(&b)));
        match from_usage {
            Some(ev) if !ev.is_empty() => {
                files += 1;
                events.extend(ev);
            }
            _ => {
                let signals = d.join("signals.json");
                if !signals.exists() {
                    continue;
                }
                files += 1;
                let mtime = std::fs::metadata(&signals)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);
                match crate::read_capped(&signals, crate::MAX_LOG_FILE_BYTES) {
                    Some(b) => events.extend(parse_signals(&String::from_utf8_lossy(&b), mtime)),
                    None => errors += 1,
                }
            }
        }
    }
    let st = SourceStatus {
        kind: SourceKind::GrokLocal,
        provider: Some(Provider::Grok),
        files,
        records: events.len(),
        oldest: events.iter().map(|e| e.ts).min(),
        newest: events.iter().map(|e| e.ts).max(),
        error: if !root.exists() {
            Some(SourceError::Missing)
        } else if errors > 0 {
            Some(SourceError::ReadErrors(errors))
        } else {
            None
        },
        ..SourceStatus::default()
    };
    (events, st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_turns_and_signals_fallback() {
        let u = r#"{"turns":[{"endedAt":"2026-11-20T10:00:00Z","totalTokens":1200,"modelUsage":{"grok-4":{}}},
            {"endedAt":"2026-11-20T11:00:00Z","totalTokens":300,"primaryModelId":"grok-code-fast"},
            {"endedAt":"bad","totalTokens":5}]}"#;
        let ev = parse_usage(u);
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].model.as_str(), ev[0].input), ("grok-4", 1200));
        assert_eq!(ev[1].model, "grok-code-fast");
        assert!(ev
            .iter()
            .all(|e| e.provider == Some(Provider::Grok) && e.nonstandard));
        let s = parse_signals(
            r#"{"totalTokensBeforeCompaction":1000,"contextTokensUsed":234,"primaryModelId":"grok-4"}"#,
            1_800_000_000,
        )
        .unwrap();
        assert_eq!((s.input, s.ts), (1234, 1_800_000_000));
        assert!(parse_signals("{}", 1).is_none());
    }

    #[test]
    fn reads_two_levels_of_session_folders() {
        let d = std::env::temp_dir().join(format!("transom-grok-{}", std::process::id()));
        let s = d.join("sessions").join("cwd-x").join("sess-1");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(
            s.join("usage.json"),
            r#"{"turns":[{"endedAt":"2026-11-20T10:00:00Z","totalTokens":10}]}"#,
        )
        .unwrap();
        let (ev, st) = read_dir(&d);
        assert_eq!(ev.len(), 1);
        assert_eq!((st.files, st.error), (1, None));
        let (_, missing) = read_dir(&d.join("nope"));
        assert_eq!(missing.error, Some(SourceError::Missing));
        let _ = std::fs::remove_dir_all(&d);
    }
}
