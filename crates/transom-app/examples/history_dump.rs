//! Overenie mesacnej historie na skutocnych lokalnych datach (len citanie; nic sa nezapisuje, nepouziva sa cache ani
//! konfiguracia pouzivatela, nevypisuje sa obsah rozhovorov): vypise denne sucty tokenov podla poskytovatela tak, ako ich
//! pocita transom (`collect` + `history::month`), na porovnanie s nezavislym vypoctom (`tools/history-check.ps1`).
//!
//! `cargo run -p transom-app --example history_dump [rok mesiac]` (predvolene aktualny mesiac).
use transom_platform::clock as pclock;
use transom_usage::history::{self, DayState, Filter, Metric};
use transom_usage::{collect, Provider, Sources};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let today = pclock::today();
    let (y, m) = match (
        args.get(1).and_then(|a| a.parse::<i32>().ok()),
        args.get(2).and_then(|a| a.parse::<u32>().ok()),
    ) {
        (Some(y), Some(m)) => (y, m),
        _ => (today.y, today.m),
    };
    let home = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let src = Sources {
        claude_dir: Some(home.join(".claude")),
        codex_dir: Some(home.join(".codex")),
        statusline_file: None,
        // `--archive SUBOR`: zluci denne sucty do archivu v danom subore (overenie; skutocny archiv sa nepouzije)
        archive: args
            .iter()
            .position(|a| a == "--archive")
            .and_then(|i| args.get(i + 1))
            .map(|p| transom_usage::ArchiveSource {
                path: p.into(),
                write: true,
                retention_months: 0,
            }),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let data = collect(&src, now, &pclock::local_day_of_unix);
    for s in &data.statuses {
        println!(
            "# zdroj {:?}: subory={} zaznamy={} najstarsi={:?} chyba={:?}",
            s.kind,
            s.files,
            s.records,
            s.oldest.map(|o| {
                let d = pclock::local_day_of_unix(o);
                format!("{}-{:02}-{:02}", d.y, d.m, d.d)
            }),
            s.error
        );
    }
    let providers = [Provider::ClaudeCode, Provider::Codex];
    println!(
        "date\tstate_all\tclaude_tokens\tcodex_tokens\tall_tokens\tall_cost_usd\tall_requests"
    );
    let all = history::month(
        &data,
        &providers,
        y,
        m,
        today,
        Filter::All,
        Metric::Tokens,
        &pclock::local_day_of_unix,
    );
    let cl = history::month(
        &data,
        &providers,
        y,
        m,
        today,
        Filter::Only(Provider::ClaudeCode),
        Metric::Tokens,
        &pclock::local_day_of_unix,
    );
    let cx = history::month(
        &data,
        &providers,
        y,
        m,
        today,
        Filter::Only(Provider::Codex),
        Metric::Tokens,
        &pclock::local_day_of_unix,
    );
    for (i, c) in all.cells.iter().enumerate() {
        let st = match c.state {
            DayState::Future => "future",
            DayState::NoHistory => "nohistory",
            DayState::Incomplete => "incomplete",
            DayState::Unpriced => "unpriced",
            DayState::Recorded => "recorded",
        };
        println!(
            "{}-{:02}-{:02}\t{st}\t{}\t{}\t{}\t{:.4}\t{}",
            y, m, c.date.d, cl.cells[i].tokens, cx.cells[i].tokens, c.tokens, c.cost, c.requests
        );
    }
    println!("# soucet mesiaca tokenov: {}", all.total_tokens);
}
