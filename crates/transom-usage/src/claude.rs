//! Lokalne prepisy Claude Code: `<claude_dir>/projects/**/*.jsonl` (`claude_dir` = `CLAUDE_CONFIG_DIR` alebo
//! `~/.claude`). **Nezdokumentovany interny format** - moze sa zmenit s kazdou verziou; pri zmene sa zaznamy
//! jednoducho nenajdu (nie zle cisla). Z riadku typu `assistant` sa cita len `message.usage`, `message.model`,
//! `message.id`, `requestId` a `timestamp`; text konverzacie sa nikam neuklada.
//!
//! Jedna odpoved API sa v prepise opakuje vo viacerych riadkoch (jeden na blok obsahu) s rovnakym `usage` -
//! deduplikuje sa globalne podla (`message.id`, `requestId`). Prepisy subagentov su v podadresaroch.
use crate::{parse_iso_utc, Event, Provider, SourceError, SourceKind, SourceStatus};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn u(v: &Value, k: &str) -> u64 {
    // strop na jedno pole: nezmyselna hodnota z poskodeneho riadku nesmie pretiect pri scitovani
    v.get(k)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(crate::MAX_TOKENS_PER_FIELD)
}

/// Rozparsuje jeden riadok; vrati kluc deduplikacie a udalost, ak riadok nesie vyuzitie.
pub fn parse_line(line: &str) -> Option<(String, Event)> {
    // lacny filter pred parsovanim JSON (vacsina riadkov vyuzitie nema)
    if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
        return None;
    }
    let o: Value = serde_json::from_str(line).ok()?;
    if o.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let m = o.get("message")?;
    let usage = m.get("usage")?;
    let model = m
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let ts = parse_iso_utc(o.get("timestamp")?.as_str()?)?;
    // rozpis zapisu do cache podla TTL; ak je mensi nez sucet (pozorovane v 8 odpovediach), zvysok sa pripocita k 5 min
    // zapisu - serverove nastroje vkladaju 5-minutove zapisy automaticky - aby sa sucet tokenov nestratil
    let total_write = u(usage, "cache_creation_input_tokens");
    let (w5, w1h) = match usage.get("cache_creation") {
        Some(c) if c.is_object() => {
            let (a, b) = (
                u(c, "ephemeral_5m_input_tokens"),
                u(c, "ephemeral_1h_input_tokens"),
            );
            (a + total_write.saturating_sub(a + b), b)
        }
        _ => (total_write, 0),
    };
    let nonstd = |k: &str| {
        usage
            .get(k)
            .and_then(Value::as_str)
            .is_some_and(|s| s != "standard")
    };
    let e = Event {
        provider: Some(Provider::ClaudeCode),
        model,
        ts,
        input: u(usage, "input_tokens"),
        output: u(usage, "output_tokens"),
        cache_read: u(usage, "cache_read_input_tokens"),
        cache_write_5m: w5,
        cache_write_1h: w1h,
        nonstandard: nonstd("speed") || nonstd("service_tier"),
        over_price_limit: false,
    };
    if e.input + e.output + e.cache_read + e.cache_write_5m + e.cache_write_1h == 0 {
        return None;
    }
    let id = m.get("id").and_then(Value::as_str).unwrap_or("");
    let req = o.get("requestId").and_then(Value::as_str).unwrap_or("");
    let key = if id.is_empty() && req.is_empty() {
        // bez identity: kazdy riadok samostatne (nededuplikovat nahodou rozne odpovede)
        format!("{ts}:{}:{}", e.input, e.output)
    } else {
        format!("{id}|{req}")
    };
    Some((key, e))
}

/// Vsetky `*.jsonl` pod `dir` (rekurzivne), bez nasledovania symbolickych odkazov.
pub(crate) fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && p.extension().is_some_and(|x| x == "jsonl") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Precita `<claude_dir>/projects`. Iny adresar (napr. `history.jsonl`, nastavenia) sa neotvara.
pub fn read_dir(claude_dir: &Path) -> (Vec<Event>, SourceStatus) {
    let root = claude_dir.join("projects");
    let files = jsonl_files(&root);
    let mut seen = HashSet::new();
    let mut events = Vec::new();
    let mut errors = 0usize;
    for f in &files {
        // po bajtoch a stratovo: jeden neplatny UTF-8 bajt nesmie zahodit cely subor
        let Some(bytes) = crate::read_capped(f, crate::MAX_LOG_FILE_BYTES) else {
            errors += 1;
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            if let Some((k, e)) = parse_line(line) {
                if seen.insert(k) {
                    events.push(e);
                }
            }
        }
    }
    let st = SourceStatus {
        kind: SourceKind::ClaudeLocal,
        provider: Some(Provider::ClaudeCode),
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
    (events, st)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"{"type":"assistant","requestId":"req_1","timestamp":"2026-11-20T10:00:00.000Z","message":{"id":"msg_1","model":"claude-opus-5-5","content":[{"type":"text","text":"TAJNY OBSAH"}],"usage":{"input_tokens":10,"cache_creation_input_tokens":300,"cache_read_input_tokens":5000,"output_tokens":42,"service_tier":"standard","speed":"standard","cache_creation":{"ephemeral_1h_input_tokens":200,"ephemeral_5m_input_tokens":100}}}}"#;

    #[test]
    fn parses_usage_fields_and_cache_split() {
        let (k, e) = parse_line(LINE).unwrap();
        assert_eq!(k, "msg_1|req_1");
        assert_eq!((e.input, e.output, e.cache_read), (10, 42, 5000));
        assert_eq!((e.cache_write_5m, e.cache_write_1h), (100, 200));
        assert_eq!(e.model, "claude-opus-5-5");
        assert!(!e.nonstandard);
        assert_eq!(e.provider, Some(Provider::ClaudeCode));
    }

    #[test]
    fn absurd_token_values_are_clamped_and_never_overflow_the_sums() {
        let max = u64::MAX.to_string();
        let l = LINE
            .replace(
                r#""input_tokens":10,"#,
                &format!(r#""input_tokens":{max},"#),
            )
            .replace(
                r#""output_tokens":42"#,
                &format!(r#""output_tokens":{max}"#),
            )
            .replace(
                r#""cache_read_input_tokens":5000"#,
                &format!(r#""cache_read_input_tokens":{max}"#),
            )
            .replace(
                r#""cache_creation_input_tokens":300"#,
                &format!(r#""cache_creation_input_tokens":{max}"#),
            )
            .replace(
                r#""ephemeral_1h_input_tokens":200"#,
                &format!(r#""ephemeral_1h_input_tokens":{max}"#),
            )
            .replace(
                r#""ephemeral_5m_input_tokens":100"#,
                &format!(r#""ephemeral_5m_input_tokens":{max}"#),
            );
        let (_, e) = parse_line(&l).expect("riadok sa nezahodi");
        assert_eq!(e.input, crate::MAX_TOKENS_PER_FIELD);
        assert_eq!(e.output, crate::MAX_TOKENS_PER_FIELD);
        assert_eq!(e.cache_read, crate::MAX_TOKENS_PER_FIELD);
        // sucet poli aj dennych suhrnov (tisice takych riadkov) nepadne a nepretecie
        let events = vec![e; 5000];
        let days = crate::by_day(&events, &|ts| {
            crate::Ymd::new(2026, 11, (ts / 86_400 % 28 + 1) as u32)
        });
        let total: u128 = days
            .iter()
            .map(|d| u128::from(d.input) + u128::from(d.output) + u128::from(d.cache_read))
            .sum();
        assert_eq!(total, 5000 * 3 * u128::from(crate::MAX_TOKENS_PER_FIELD));
        // obycajne hodnoty sa neorezavaju
        assert_eq!(parse_line(LINE).unwrap().1.cache_read, 5000);
    }

    #[test]
    fn truncated_last_line_invalid_utf8_and_duplicates_do_not_lose_or_double_count() {
        let dir = std::env::temp_dir().join(format!(
            "transom-usage-claude-damaged-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let proj = dir.join("projects").join("p1");
        std::fs::create_dir_all(&proj).unwrap();
        let second = LINE.replace("msg_1", "msg_2").replace("req_1", "req_2");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(LINE.as_bytes());
        bytes.push(b'\n');
        bytes.extend_from_slice(LINE.as_bytes()); // duplicit (rovnake msg|req): nepocita sa dvakrat
        bytes.push(b'\n');
        bytes.extend_from_slice(b"{\"type\":\"assistant\",\"usage\":\xFF\xFE nevalidne UTF-8\n");
        bytes.extend_from_slice(second.as_bytes());
        bytes.push(b'\n');
        bytes.extend_from_slice(&LINE.as_bytes()[..LINE.len() / 2]); // useknuty posledny riadok (bez konca riadka)
        std::fs::write(proj.join("a.jsonl"), &bytes).unwrap();
        // cudzi / prazdny subor vedla toho nic nerozbije
        std::fs::write(proj.join("empty.jsonl"), b"").unwrap();
        let (events, st) = read_dir(&dir);
        assert_eq!(
            events.len(),
            2,
            "dva rozne zaznamy: duplicit, nevalidny a useknuty riadok sa preskocia"
        );
        assert_eq!(st.files, 2);
        assert_eq!((st.records, st.error.is_none()), (2, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incomplete_breakdown_keeps_total() {
        // sucet 9618, rozpis len 1h 7279 -> zvysok 2339 ako 5 min (tvar z realnych dat)
        let l = LINE
            .replace(r#""cache_creation_input_tokens":300"#, r#""cache_creation_input_tokens":9618"#)
            .replace(
                r#""cache_creation":{"ephemeral_1h_input_tokens":200,"ephemeral_5m_input_tokens":100}"#,
                r#""cache_creation":{"ephemeral_1h_input_tokens":7279,"ephemeral_5m_input_tokens":0}"#,
            );
        let e = parse_line(&l).unwrap().1;
        assert_eq!((e.cache_write_5m, e.cache_write_1h), (2339, 7279));
        assert_eq!(e.cache_write_5m + e.cache_write_1h, 9618);
    }

    #[test]
    fn ignores_other_lines_and_marks_fast() {
        assert!(parse_line(r#"{"type":"user","message":{"content":"x"}}"#).is_none());
        assert!(parse_line("nie je json").is_none());
        let fast = LINE.replace(r#""speed":"standard""#, r#""speed":"fast""#);
        assert!(parse_line(&fast).unwrap().1.nonstandard);
        // bez rozpisu cache_creation -> cely zapis ako 5 min
        let plain = LINE.replace(
            r#","cache_creation":{"ephemeral_1h_input_tokens":200,"ephemeral_5m_input_tokens":100}"#,
            "",
        );
        let e = parse_line(&plain).unwrap().1;
        assert_eq!((e.cache_write_5m, e.cache_write_1h), (300, 0));
    }

    #[test]
    fn dedupes_globally_across_files() {
        let dir = std::env::temp_dir().join(format!("transom-usage-claude-{}", std::process::id()));
        let p = dir.join("projects").join("a");
        std::fs::create_dir_all(p.join("sub").join("subagents")).unwrap();
        // ta ista odpoved 2x v jednom subore a raz v inom (napr. pokracovana relacia)
        std::fs::write(p.join("s1.jsonl"), format!("{LINE}\n{LINE}\n")).unwrap();
        std::fs::write(
            p.join("sub").join("subagents").join("s2.jsonl"),
            format!("{LINE}\n"),
        )
        .unwrap();
        // subor mimo projects/ sa nesmie citat
        std::fs::write(dir.join("history.jsonl"), format!("{LINE}\n")).unwrap();
        let (ev, st) = read_dir(&dir);
        assert_eq!(ev.len(), 1);
        assert_eq!(st.files, 2);
        assert!(st.error.is_none());
        let _ = std::fs::remove_dir_all(&dir);
        let (ev, st) = read_dir(&dir);
        assert!(ev.is_empty() && st.error.is_some());
    }
}
