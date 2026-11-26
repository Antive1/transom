//! Most statusline Claude Code (zdokumentovane rozhranie, code.claude.com/docs/en/statusline): Claude Code posle
//! prikazu `statusLine` JSON na stdin. `transom-statusline` z neho vyberie len:
//! - `cost.total_cost_usd` ("Estimated session cost ... May differ from your actual bill"),
//! - `rate_limits.five_hour` / `seven_day` (`used_percentage`, `resets_at`) - podla dokumentacie len pre
//!   claude.ai Pro, Max a Ultra a az po prvej odpovedi v relacii,
//! - `context_window.used_percentage` a `context_window_size` (zdokumentovane; len dve cisla - zaplnenie kontextu
//!   **poslednej relacie**, nie sucet),
//! - `model.id` a hash `session_id` (na rozlisenie relacii; samotne ID sa neuklada),
//!
//! a prida riadok do `%LOCALAPPDATA%\transom\claude-statusline.jsonl`. Nic ine (cesty, `cwd`, prepis) sa neuklada.
//! Cena relacie je kumulativna: denny odhad = sucet prirastkov v ramci relacie.
use crate::{
    LimitWindow, PlanSnapshot, Provider, SnapshotSource, SourceError, SourceKind, SourceStatus,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use transom_core::clock::Ymd;

/// (odhad ceny po dnoch, stav zdroja, posledny snimok limitov)
/// (odhad ceny po dnoch, stav zdroja, snimky limitov, posledne zaplnenie kontextu). Snimky limitov: pre kazde okno
/// najnovsi zaznam, v ktorom sa okno vyskytlo (zaznamy s rovnakym casom tvoria jeden snimok) - okno, ktore Claude Code
/// po resete prestal posielat, tak nezmizne, ale ostane so starym casom zaznamu (zastarane/expirovane).
pub type StatuslineRead = (
    Vec<crate::DayCost>,
    SourceStatus,
    Vec<PlanSnapshot>,
    Option<crate::ContextSnapshot>,
);

/// FNV-1a 64 (hash ID relacie - odlisi relacie bez ulozenia ID).
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Zaznam do suboru mostu zo vstupu statusline (`None` = vstup bez ceny aj bez limitov).
pub fn record(input: &Value, now: i64) -> Option<Value> {
    let cost = input
        .get("cost")
        .and_then(|c| c.get("total_cost_usd"))
        .and_then(Value::as_f64);
    let rl = input.get("rate_limits");
    let win = |k: &str| {
        rl.and_then(|r| r.get(k)).map(|w| {
            (
                w.get("used_percentage").and_then(Value::as_f64),
                w.get("resets_at").and_then(Value::as_i64),
            )
        })
    };
    let (rl5, rl5r) = win("five_hour").unwrap_or((None, None));
    let (rl7, rl7r) = win("seven_day").unwrap_or((None, None));
    let ctx = input.get("context_window");
    let ctx_pct = ctx
        .and_then(|c| c.get("used_percentage"))
        .and_then(Value::as_f64);
    let ctx_size = ctx
        .and_then(|c| c.get("context_window_size"))
        .and_then(Value::as_u64);
    if cost.is_none() && rl5.is_none() && rl7.is_none() && ctx_pct.is_none() {
        return None;
    }
    let sid = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let model = input
        .get("model")
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(json!({
        "ts": now,
        "s": format!("{:016x}", fnv(sid)),
        "model": model,
        "cost": cost,
        "rl5": rl5, "rl5_reset": rl5r,
        "rl7": rl7, "rl7_reset": rl7r,
        "ctx": ctx_pct, "ctx_size": ctx_size,
    }))
}

/// Kratky text, ktory most vypise ako statusline (Claude Code ho zobrazi v terminali).
/// `estimate` = slovo "odhad" v jazyku aplikacie (cena relacie nie je faktura).
pub fn status_text(input: &Value, estimate: &str) -> String {
    let model = input
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(Value::as_str)
        .unwrap_or("Claude");
    let mut parts = vec![model.to_string()];
    if let Some(c) = input
        .get("cost")
        .and_then(|c| c.get("total_cost_usd"))
        .and_then(Value::as_f64)
    {
        parts.push(format!("≈${c:.2} ({estimate})"));
    }
    if let Some(p) = input
        .get("rate_limits")
        .and_then(|r| r.get("five_hour"))
        .and_then(|w| w.get("used_percentage"))
        .and_then(Value::as_f64)
    {
        parts.push(format!("5h {p:.0} %"));
    }
    parts.join(" · ")
}

/// Pripoji jeden zaznam do suboru mostu. Cely riadok (vratane konca riadka) sa zapise **jednym** `write_all` na subor otvoreny
/// na pripajanie, takze paralelne relacie Claude Code neprelozia casti riadkov (`writeln!` robi viac zapisov).
pub fn append_record(path: &Path, rec: &Value) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut line = rec.to_string();
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

/// Precita subor mostu: odhad ceny po dnoch (prirastky v ramci relacie) a posledny snimok limitov.
pub fn read_file(path: &Path, local_day: &dyn Fn(i64) -> Ymd) -> StatuslineRead {
    let mut st = SourceStatus {
        kind: SourceKind::ClaudeStatusline,
        provider: Some(Provider::ClaudeCode),
        ..Default::default()
    };
    let text = match crate::read_capped(path, crate::MAX_BRIDGE_FILE_BYTES) {
        Some(b) => String::from_utf8_lossy(&b).into_owned(),
        None => {
            st.error = Some(SourceError::NoBridgeData);
            return (Vec::new(), st, Vec::new(), None);
        }
    };
    st.files = 1;
    let mut recs: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    recs.sort_by_key(|r| r.get("ts").and_then(Value::as_i64).unwrap_or(0));
    st.records = recs.len();
    let mut last_cost: HashMap<String, f64> = HashMap::new();
    let mut by_day: HashMap<(i32, u32, u32), f64> = HashMap::new();
    // najnovsi zaznam pre kazde okno: minuty -> (cas zaznamu, percento, reset)
    let mut latest: HashMap<u32, (i64, f64, Option<i64>)> = HashMap::new();
    let mut context: Option<crate::ContextSnapshot> = None;
    for r in &recs {
        let ts = r.get("ts").and_then(Value::as_i64).unwrap_or(0);
        st.oldest = Some(st.oldest.map_or(ts, |o| o.min(ts)));
        st.newest = Some(st.newest.map_or(ts, |o| o.max(ts)));
        let s = r.get("s").and_then(Value::as_str).unwrap_or("").to_string();
        if let Some(c) = r.get("cost").and_then(Value::as_f64) {
            let prev = last_cost.insert(s, c).unwrap_or(0.0);
            // pokles = nova relacia s rovnakym hashom (alebo reset) -> cela hodnota je prirastok
            let delta = if c >= prev { c - prev } else { c };
            if delta > 0.0 {
                let d = local_day(ts);
                *by_day.entry((d.y, d.m, d.d)).or_insert(0.0) += delta;
            }
        }
        for (k, minutes) in [
            ("rl5", crate::limits::FIVE_HOURS),
            ("rl7", crate::limits::WEEK),
        ] {
            if let Some(p) = r.get(k).and_then(Value::as_f64) {
                latest.insert(
                    minutes,
                    (ts, p, r.get(format!("{k}_reset")).and_then(Value::as_i64)),
                );
            }
        }
        if let Some(p) = r.get("ctx").and_then(Value::as_f64) {
            context = Some(crate::ContextSnapshot {
                provider: Some(Provider::ClaudeCode),
                used_percent: p,
                window_tokens: r.get("ctx_size").and_then(Value::as_u64).unwrap_or(0),
                recorded_at: ts,
            });
        }
    }
    st.limits_seen = !latest.is_empty();
    // snimky podla casu zaznamu (okna z jedneho zaznamu patria k jednemu snimku)
    let mut ts_list: Vec<i64> = latest.values().map(|v| v.0).collect();
    ts_list.sort_unstable();
    ts_list.dedup();
    let plans: Vec<PlanSnapshot> = ts_list
        .into_iter()
        .map(|ts| {
            let mut windows: Vec<LimitWindow> = latest
                .iter()
                .filter(|(_, v)| v.0 == ts)
                .map(|(m, v)| LimitWindow {
                    minutes: *m,
                    used_percent: v.1,
                    resets_at: v.2,
                })
                .collect();
            windows.sort_by_key(|w| w.minutes);
            PlanSnapshot {
                provider: Some(Provider::ClaudeCode),
                plan: "Pro/Max/Ultra".into(),
                windows,
                recorded_at: ts,
                source: SnapshotSource::ClaudeStatusline,
            }
        })
        .collect();
    let mut v: Vec<_> = by_day.into_iter().collect();
    v.sort_by_key(|a| a.0);
    (v, st, plans, context)
}

/// Druh udalosti z hooku: `start`, `tool`, `stop`, `ask` (povolenie), `idle` (caka na odpoved), `note` (ine).
/// Pri `notify` sa text spravy necita - len typ (`notification_type`, inak podla klucovych slov anglickej spravy).
pub fn hook_kind(arg: &str, input: &Value) -> &'static str {
    match arg {
        "start" => "start",
        "tool" => "tool",
        "stop" => "stop",
        // ziadost o povolenie (hned, nie az po upozorneni) a koniec tahu chybou API
        "ask" => "ask",
        "error" => "error",
        _ => {
            let ty = input
                .get("notification_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            let msg = input
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if ty.contains("permission") || msg.contains("permission") {
                "ask"
            } else if ty.contains("idle") || msg.contains("waiting for your input") {
                "idle"
            } else {
                "note"
            }
        }
    }
}

/// Zaznam udalosti (len druh, cas a hash relacie).
pub fn hook_record(kind: &str, input: &Value, now: i64) -> Value {
    let sid = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    json!({"ts": now, "ev": kind, "s": format!("{:016x}", fnv(sid))})
}

/// Posledna udalost zo suboru (cas, druh) - cita sa len koniec suboru.
pub fn last_event(path: &Path) -> Option<(i64, String)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(4096))).ok()?;
    let mut buf = Vec::new();
    f.take(4096).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines().rev().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        Some((v["ts"].as_i64()?, v["ev"].as_str()?.to_string()))
    })
}

/// Limity z posledneho zaznamu mostu (pozri [`last_limits`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LastLimits {
    /// Cas zaznamu (unix s).
    pub ts: i64,
    /// Spotrebovane percento a reset (unix s) 5-hodinoveho a tyzdenneho okna.
    pub five: Option<f64>,
    pub five_reset: Option<i64>,
    pub week: Option<f64>,
    pub week_reset: Option<i64>,
}

/// Posledne limity zo suboru mostu (lacne: len koniec suboru) z najnovsieho zaznamu, ktory nejaky limit nesie. Na
/// ostrov v liste, ked prave pracuje Claude Code; nic ine sa necita.
pub fn last_limits(path: &Path) -> Option<LastLimits> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(16 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.take(16 * 1024).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    // prvy riadok moze byt orezany (zaciatok mimo okna): nevadi, nepreparsuje sa
    text.lines().rev().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        let (r5, r7) = (v["rl5"].as_f64(), v["rl7"].as_f64());
        (r5.is_some() || r7.is_some()).then(|| LastLimits {
            ts: v["ts"].as_i64().unwrap_or(0),
            five: r5,
            five_reset: v["rl5_reset"].as_i64(),
            week: r7,
            week_reset: v["rl7_reset"].as_i64(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_events_keep_only_the_kind_and_the_last_one_is_read() {
        assert_eq!(hook_kind("start", &json!({})), "start");
        assert_eq!(
            hook_kind(
                "notify",
                &json!({"message": "Claude needs your permission to use Bash"})
            ),
            "ask"
        );
        assert_eq!(
            hook_kind(
                "notify",
                &json!({"message": "Claude is waiting for your input"})
            ),
            "idle"
        );
        assert_eq!(
            hook_kind("notify", &json!({"notification_type": "idle_prompt"})),
            "idle"
        );
        let r = hook_record(
            "ask",
            &json!({"session_id": "abc", "message": "secret text"}),
            5,
        );
        assert!(!r.to_string().contains("secret"), "text spravy sa neuklada");
        let dir = std::env::temp_dir().join(format!("transom-hook-ev-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("e.jsonl");
        append_record(&p, &hook_record("start", &json!({}), 1)).unwrap();
        append_record(&p, &hook_record("stop", &json!({}), 2)).unwrap();
        assert_eq!(last_event(&p), Some((2, "stop".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_limits_reads_the_newest_record_with_limits_from_the_end() {
        let dir =
            std::env::temp_dir().join(format!("transom-usage-sl-last-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("b.jsonl");
        assert_eq!(last_limits(&p), None);
        let mut text = String::new();
        // dlhy zaciatok (cita sa len koniec)
        for i in 0..400 {
            let pad = "x".repeat(60);
            text.push_str(&format!(
                "{{\"ts\":{i},\"rl5\":1.0,\"rl7\":2.0,\"pad\":\"{pad}\"}}\n"
            ));
        }
        text.push_str("{\"ts\":900,\"rl5\":34.0,\"rl5_reset\":1000,\"rl7\":91.0}\n");
        text.push_str("{\"ts\":901,\"cost\":0.5,\"rl5\":null,\"rl7\":null}\n");
        std::fs::write(&p, text).unwrap();
        assert_eq!(
            last_limits(&p),
            Some(LastLimits {
                ts: 900,
                five: Some(34.0),
                five_reset: Some(1000),
                week: Some(91.0),
                week_reset: None,
            })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn input(sid: &str, cost: f64, rl5: Option<f64>) -> Value {
        let mut v = json!({
            "session_id": sid,
            "cwd": "C:\\tajne",
            "transcript_path": "C:\\tajne.jsonl",
            "model": {"id": "claude-opus-5-5", "display_name": "Opus 5.5"},
            "cost": {"total_cost_usd": cost}
        });
        if let Some(p) = rl5 {
            v["rate_limits"] = json!({"five_hour": {"used_percentage": p, "resets_at": 1795200000},
                                      "seven_day": {"used_percentage": 40.0, "resets_at": 1795500000}});
        }
        v
    }

    #[test]
    fn record_keeps_only_numbers_and_hash() {
        let r = record(&input("sess-1", 1.25, Some(12.0)), 100).unwrap();
        let s = r.to_string();
        assert!(
            !s.contains("tajne") && !s.contains("sess-1"),
            "ziadne cesty ani ID relacie"
        );
        assert_eq!(r["cost"], 1.25);
        assert_eq!(r["rl5"], 12.0);
        assert_eq!(r["rl7_reset"], 1795500000);
        assert!(record(&json!({"model": {"id": "x"}}), 1).is_none());
        assert_eq!(
            status_text(&input("a", 1.234, Some(12.0)), "odhad"),
            "Opus 5.5 · ≈$1.23 (odhad) · 5h 12 %"
        );
    }

    #[test]
    fn concurrent_appends_stay_whole_lines_and_a_damaged_tail_is_skipped() {
        let dir =
            std::env::temp_dir().join(format!("transom-usage-sl-append-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("sub").join("sl.jsonl");
        let day = |ts: i64| transom_core::clock::civil_from_days(ts.div_euclid(86_400));
        let base = 20_412 * 86_400;
        // 8 relacii pise naraz po 40 zaznamov: kazdy riadok musi ostat cely a platny
        std::thread::scope(|s| {
            for t in 0..8 {
                let f = &f;
                s.spawn(move || {
                    for i in 0..40 {
                        let r = record(
                            &input(&format!("S{t}"), 0.01 * f64::from(i + 1), Some(10.0)),
                            base + i64::from(i),
                        )
                        .unwrap();
                        append_record(f, &r).unwrap();
                    }
                });
            }
        });
        let text = std::fs::read_to_string(&f).unwrap();
        assert_eq!(text.lines().count(), 320);
        assert!(
            text.lines()
                .all(|l| serde_json::from_str::<Value>(l).is_ok()),
            "ziadny prelozeny riadok"
        );
        let (by, st, _, _) = read_file(&f, &day);
        assert_eq!(st.records, 320);
        assert!((by[0].1 - 8.0 * 0.40).abs() < 1e-6, "8 relacii x 0,40 USD");
        // useknuty posledny riadok a nevalidne UTF-8 (napr. zapis preruseny vypnutim): tolerantne, ostatne zaznamy platia
        let mut bytes = std::fs::read(&f).unwrap();
        bytes.extend_from_slice(b"{\"ts\":1,\"cost\":9.9,\"s\":\"x");
        bytes.extend_from_slice(&[0xFF, 0xFE, b'\n']);
        std::fs::write(&f, &bytes).unwrap();
        let (by2, st2, _, _) = read_file(&f, &day);
        assert_eq!(st2.records, 320, "poskodeny koniec sa preskoci");
        assert!((by2[0].1 - by[0].1).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daily_cost_from_cumulative_session_totals() {
        let dir = std::env::temp_dir().join(format!("transom-usage-sl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("sl.jsonl");
        let day = |ts: i64| transom_core::clock::civil_from_days(ts.div_euclid(86_400));
        let d1 = 20_412 * 86_400; // jeden den
        let lines = [
            record(&input("A", 1.0, None), d1 + 100).unwrap(),
            record(&input("A", 1.5, None), d1 + 200).unwrap(),
            record(&input("B", 0.25, Some(30.0)), d1 + 300).unwrap(),
            // dalsi den: relacia A pokracuje
            record(&input("A", 2.0, None), d1 + 86_400 + 10).unwrap(),
        ];
        std::fs::write(
            &f,
            lines
                .iter()
                .map(|l| l.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let (by, st, plans, _) = read_file(&f, &day);
        assert_eq!(st.records, 4);
        assert_eq!(by.len(), 2);
        assert!((by[0].1 - 1.75).abs() < 1e-9, "1,0 + 0,5 + 0,25");
        assert!((by[1].1 - 0.5).abs() < 1e-9, "prirastok A na druhy den");
        let p = plans.last().unwrap();
        assert_eq!(
            p.windows[0],
            LimitWindow {
                minutes: 300,
                used_percent: 30.0,
                resets_at: Some(1795200000)
            }
        );
        assert_eq!(p.windows[1].minutes, 10_080);
        let _ = std::fs::remove_dir_all(&dir);
        let (_, st, _, _) = read_file(&f, &day);
        assert!(
            st.error.is_some(),
            "chybajuci subor = vysvetlenie, nie nula"
        );
    }

    #[test]
    fn window_that_disappears_from_the_latest_record_keeps_its_older_snapshot() {
        let dir = std::env::temp_dir().join(format!("transom-usage-sl2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("sl.jsonl");
        let day = |ts: i64| transom_core::clock::civil_from_days(ts.div_euclid(86_400));
        // 1. zaznam: 5 h aj 7 d; 2. zaznam (neskor): len 7 d (Claude Code po resete 5 h okno uz neposiela)
        let a = record(&input("A", 1.0, Some(80.0)), 1_000).unwrap();
        let mut b_in = input("A", 1.1, None);
        b_in["rate_limits"] =
            json!({"seven_day": {"used_percentage": 41.0, "resets_at": 1795500000}});
        let b = record(&b_in, 9_000).unwrap();
        std::fs::write(
            &f,
            format!(
                "{a}
{b}
"
            ),
        )
        .unwrap();
        let (_, _, plans, _) = read_file(&f, &day);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(plans.len(), 2, "dva snimky podla casu zaznamu");
        let five = plans
            .iter()
            .find(|p| p.windows.iter().any(|w| w.minutes == 300))
            .unwrap();
        assert_eq!((five.recorded_at, five.windows.len()), (1_000, 1));
        assert_eq!(five.windows[0].used_percent, 80.0);
        let week = plans.iter().find(|p| p.recorded_at == 9_000).unwrap();
        assert_eq!(week.windows[0].minutes, 10_080);
        assert_eq!(
            week.windows[0].used_percent, 41.0,
            "tyzden je z najnovsieho zaznamu"
        );
    }

    #[test]
    fn context_window_is_two_documented_numbers_and_the_latest_wins() {
        let mut i1 = input("A", 1.0, None);
        i1["context_window"] = json!({"used_percentage": 8, "context_window_size": 200000,
            "total_input_tokens": 15500, "current_usage": {"input_tokens": 1}});
        let r = record(&i1, 100).unwrap();
        assert_eq!(r["ctx"], 8.0);
        assert_eq!(r["ctx_size"], 200000);
        assert!(
            !r.to_string().contains("15500"),
            "len percento a velkost okna"
        );
        // null (pred prvou odpovedou) = ziadny kontext; samotny kontext (bez ceny a limitov) je platny zaznam
        let mut early = json!({"model": {"id": "x"}, "context_window": {"used_percentage": null}});
        assert!(record(&early, 1).is_none());
        early["context_window"]["used_percentage"] = json!(3);
        assert!(record(&early, 1).is_some());
        let dir = std::env::temp_dir().join(format!("transom-usage-sl3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("sl.jsonl");
        let mut i2 = input("B", 2.0, None);
        i2["context_window"] = json!({"used_percentage": 42, "context_window_size": 1000000});
        let day = |ts: i64| transom_core::clock::civil_from_days(ts.div_euclid(86_400));
        std::fs::write(
            &f,
            format!(
                "{}
{}
",
                record(&i1, 100).unwrap(),
                record(&i2, 200).unwrap()
            ),
        )
        .unwrap();
        let (_, _, _, ctx) = read_file(&f, &day);
        let _ = std::fs::remove_dir_all(&dir);
        let c = ctx.unwrap();
        assert_eq!(
            (c.used_percent, c.window_tokens, c.recorded_at),
            (42.0, 1_000_000, 200)
        );
    }
}
