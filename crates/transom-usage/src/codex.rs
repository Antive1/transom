//! Lokalne relacie OpenAI Codex: `<codex_home>/sessions/**/*.jsonl` (`codex_home` = `CODEX_HOME` alebo
//! `~/.codex`). **Nezdokumentovana schema** (dokumentacia spomina len `CODEX_HOME` a `history.jsonl`; zdokumentovany
//! je OpenTelemetry export). `auth.json` ani ine subory sa neotvaraju.
//!
//! - Tokeny: riadky `token_usage_record` (`payload.usage`) - jeden na odpoved, deduplikacia podla `response_id`.
//!   Udalosti `token_count` nesu tie iste cisla znova (a kumulativne sucty relacie) - nepocitaju sa.
//!   `input_tokens` **obsahuje** cachovany vstup (overene: `total = input + output`), preto
//!   necachovany vstup = `input - cached - cache_write`.
//! - Model: z `turn_context.payload.model` podla `turn_id`.
//! - Limit planu: posledny `token_count.rate_limits` (plan, percento okna, reset) - snimok **zaznamenany Codexom**
//!   v case behu, nie aktualny stav uctu.
use crate::claude::jsonl_files;
use crate::{
    parse_iso_utc, Event, LimitWindow, PlanSnapshot, Provider, SnapshotSource, SourceError,
    SourceKind, SourceStatus,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn u(v: &Value, k: &str) -> u64 {
    v.get(k)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(crate::MAX_TOKENS_PER_FIELD)
}

#[derive(Default)]
pub(crate) struct FileState {
    models: HashMap<String, String>,
    last_model: String,
}

/// Spracuje jeden riadok; vrati (kluc, udalost) alebo snimok limitu.
pub(crate) fn parse_line(
    st: &mut FileState,
    line: &str,
) -> (Option<(String, Event)>, Option<PlanSnapshot>) {
    let quick = line.contains("turn_context")
        || line.contains("token_usage_record")
        || line.contains("rate_limits");
    if !quick {
        return (None, None);
    }
    let Ok(o) = serde_json::from_str::<Value>(line) else {
        return (None, None);
    };
    let ts = o
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_iso_utc);
    let p = o.get("payload").cloned().unwrap_or(Value::Null);
    match o.get("type").and_then(Value::as_str) {
        Some("turn_context") => {
            if let Some(m) = p.get("model").and_then(Value::as_str) {
                if let Some(t) = p.get("turn_id").and_then(Value::as_str) {
                    st.models.insert(t.to_string(), m.to_string());
                }
                st.last_model = m.to_string();
            }
            (None, None)
        }
        Some("token_usage_record") => {
            let (Some(ts), Some(us)) = (ts, p.get("usage")) else {
                return (None, None);
            };
            let model = p
                .get("turn_id")
                .and_then(Value::as_str)
                .and_then(|t| st.models.get(t).cloned())
                .unwrap_or_else(|| st.last_model.clone());
            let input = u(us, "input_tokens");
            let cached = u(us, "cached_input_tokens");
            let write = u(us, "cache_write_input_tokens");
            let e = Event {
                provider: Some(Provider::Codex),
                model,
                ts,
                input: input.saturating_sub(cached + write),
                output: u(us, "output_tokens"),
                cache_read: cached,
                cache_write_5m: write,
                cache_write_1h: 0,
                nonstandard: false,
                over_price_limit: false,
            };
            let key = p
                .get("response_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{ts}:{input}:{}", e.output));
            (Some((key, e)), None)
        }
        Some("event_msg") if p.get("type").and_then(Value::as_str) == Some("token_count") => {
            let rl = p.get("rate_limits").cloned().unwrap_or(Value::Null);
            let mut windows = Vec::new();
            for k in ["primary", "secondary"] {
                if let Some(w) = rl.get(k).filter(|w| w.is_object()) {
                    if let Some(pct) = w.get("used_percent").and_then(Value::as_f64) {
                        windows.push(LimitWindow {
                            minutes: u(w, "window_minutes") as u32,
                            used_percent: pct,
                            resets_at: w.get("resets_at").and_then(Value::as_i64),
                        });
                    }
                }
            }
            if windows.is_empty() {
                return (None, None);
            }
            let snap = PlanSnapshot {
                provider: Some(Provider::Codex),
                plan: rl
                    .get("plan_type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                windows,
                recorded_at: ts.unwrap_or(0),
                source: SnapshotSource::CodexSession,
            };
            (None, Some(snap))
        }
        _ => (None, None),
    }
}

/// Najviac tolko poslednych dennych priecinkov sa prezera (vlakno sa pise do priecinka dna, ked zacalo).
const SESSION_DAYS: usize = 30;

/// Naposledy zmeneny subor relacie Codexu (`sessions/RRRR/MM/DD/*.jsonl`) a cas jeho zmeny - na zistenie, ci Codex
/// prave pracuje. Codex pise do suboru v priecinku dna, ked vlakno **zacalo** (pokracovanie starsieho vlakna, napr. vo
/// VS Code, pise do starsieho dna), preto sa porovnaju vsetky subory poslednych [`SESSION_DAYS`] dni s reláciami.
pub fn latest_session(codex_home: &Path) -> Option<(std::path::PathBuf, std::time::SystemTime)> {
    fn dirs_desc(p: &Path) -> Vec<std::path::PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(p)
            .map(|r| {
                r.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v.reverse();
        v
    }
    let root = codex_home.join("sessions");
    let days = dirs_desc(&root)
        .into_iter()
        .flat_map(|y| dirs_desc(&y))
        .flat_map(|m| dirs_desc(&m))
        .take(SESSION_DAYS);
    days.filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|r| r.flatten())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        // pri rovnakom case novsi den (cesta s datumom)
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
}

/// Posledny snimok limitov z konca suboru relacie (najviac 256 KB od konca).
pub fn last_snapshot(path: &Path) -> Option<PlanSnapshot> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(256 * 1024)))
        .ok()?;
    let mut buf = Vec::new();
    f.take(256 * 1024).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut st = FileState::default();
    let mut last = None;
    for line in text.lines() {
        if let (_, Some(s)) = parse_line(&mut st, line) {
            last = Some(s);
        }
    }
    last
}

pub fn read_dir(codex_home: &Path) -> (Vec<Event>, SourceStatus, Option<PlanSnapshot>) {
    let root = codex_home.join("sessions");
    let files = jsonl_files(&root);
    let mut seen = HashSet::new();
    let mut events = Vec::new();
    let mut plan: Option<PlanSnapshot> = None;
    let mut errors = 0;
    for f in &files {
        // po bajtoch a stratovo: jeden neplatny UTF-8 bajt nesmie zahodit cely subor
        let Some(bytes) = crate::read_capped(f, crate::MAX_LOG_FILE_BYTES) else {
            errors += 1;
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut st = FileState::default();
        for line in text.lines() {
            let (ev, snap) = parse_line(&mut st, line);
            if let Some((k, e)) = ev {
                if seen.insert(k) {
                    events.push(e);
                }
            }
            if let Some(s) = snap {
                if plan.as_ref().is_none_or(|p| s.recorded_at >= p.recorded_at) {
                    plan = Some(s);
                }
            }
        }
    }
    let st = SourceStatus {
        kind: SourceKind::CodexLocal,
        provider: Some(Provider::Codex),
        files: files.len(),
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
        ..Default::default()
    };
    (events, st, plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURN: &str = r#"{"timestamp":"2026-11-15T21:51:04.000Z","type":"turn_context","payload":{"turn_id":"t1","model":"gpt-6-luna","cwd":"C:\\tajne"}}"#;
    const REC: &str = r#"{"timestamp":"2026-11-15T21:52:00.000Z","type":"token_usage_record","payload":{"turn_id":"t1","response_id":"r1","usage":{"input_tokens":18043,"cached_input_tokens":12800,"cache_write_input_tokens":0,"output_tokens":676,"reasoning_output_tokens":0,"total_tokens":18719},"thread_token_usage":{"input_tokens":999999}}}"#;
    const COUNT: &str = r#"{"timestamp":"2026-11-15T21:52:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":18043,"output_tokens":676}},"rate_limits":{"primary":{"used_percent":34.5,"window_minutes":43200,"resets_at":1795000000},"secondary":null,"plan_type":"go"}}}"#;

    #[test]
    fn record_counts_once_with_model_and_cache_split() {
        let mut st = FileState::default();
        assert!(parse_line(&mut st, TURN).0.is_none());
        let (ev, _) = parse_line(&mut st, REC);
        let (k, e) = ev.unwrap();
        assert_eq!(k, "r1");
        assert_eq!(e.model, "gpt-6-luna");
        assert_eq!(
            (e.input, e.cache_read, e.output),
            (18043 - 12800, 12800, 676)
        );
        // token_count nesie tie iste tokeny: nepocita sa, len snimok planu
        let (ev, snap) = parse_line(&mut st, COUNT);
        assert!(ev.is_none());
        let s = snap.unwrap();
        assert_eq!(s.plan, "go");
        assert_eq!(
            s.windows,
            vec![LimitWindow {
                minutes: 43_200,
                used_percent: 34.5,
                resets_at: Some(1795000000)
            }]
        );
        assert!(s.recorded_at > 0);
    }

    #[test]
    fn reads_only_sessions_dir_and_dedupes() {
        let dir = std::env::temp_dir().join(format!("transom-usage-codex-{}", std::process::id()));
        let s = dir.join("sessions").join("2026").join("11").join("15");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(
            s.join("rollout-a.jsonl"),
            format!("{TURN}\n{REC}\n{COUNT}\n{REC}\n"),
        )
        .unwrap();
        // auth.json a ine subory sa nesmu citat (aj keby obsahovali "zaznamy")
        std::fs::write(dir.join("auth.json"), REC).unwrap();
        std::fs::write(dir.join("history.jsonl"), format!("{REC}\n")).unwrap();
        let (ev, st, plan) = read_dir(&dir);
        assert_eq!(ev.len(), 1);
        assert_eq!(st.files, 1);
        assert!(plan.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_latest_session_and_its_last_limits_are_found_cheaply() {
        let home =
            std::env::temp_dir().join(format!("transom-codex-latest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert!(latest_session(&home).is_none());
        let old = home.join("sessions/2026/11/20");
        let new = home.join("sessions/2026/11/25");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("rollout-a.jsonl"), COUNT).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(
            new.join("rollout-b.jsonl"),
            format!(
                "{COUNT}
"
            ),
        )
        .unwrap();
        let (p, _) = latest_session(&home).unwrap();
        assert_eq!(p, new.join("rollout-b.jsonl"), "najnovsi den");
        // starsie vlakno pokracuje (pise do priecinka dna, ked zacalo): je naposledy zmenene, takze vyhra
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(
            old.join("rollout-a.jsonl"),
            format!(
                "{COUNT}
"
            ),
        )
        .unwrap();
        let (p, _) = latest_session(&home).unwrap();
        assert_eq!(p, old.join("rollout-a.jsonl"), "naposledy zmeneny subor");
        let (p, _) = latest_session(&home).unwrap();
        let snap = last_snapshot(&p).unwrap();
        assert_eq!(snap.windows[0].minutes, 43200);
        assert!((snap.windows[0].used_percent - 34.5).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&home);
    }
}
