//! Vypise suhrny vyuzitia z lokalnych zdrojov (len cisla a modely, ziadny obsah) - na overenie bez listy.
//!   cargo run -p transom-usage --example usage_summary [-- --today 2026-11-20]
use transom_core::clock::{civil_from_days, Ymd};
use transom_usage::{collect, fmt_tokens, pricing, summarize, Period, Sources};

fn main() {
    let home = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(Into::into)
        .unwrap_or(home.join(".claude"));
    let codex = std::env::var_os("CODEX_HOME")
        .map(Into::into)
        .unwrap_or(home.join(".codex"));
    let src = Sources {
        claude_dir: Some(claude),
        codex_dir: Some(codex),
        grok_dir: None,
        statusline_file: Some(transom_usage::cache::statusline_path()),
        archive: None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // priklad pouziva UTC (lista pouziva lokalne pasmo Windows)
    let utc = |ts: i64| civil_from_days(ts.div_euclid(86_400));
    let t0 = std::time::Instant::now();
    let c = collect(&src, now, &utc);
    println!("zber za {} ms", t0.elapsed().as_millis());
    for s in &c.statuses {
        println!(
            "zdroj: {:?} | subory {} | zaznamy {} | chyba {:?}",
            s.kind, s.files, s.records, s.error
        );
    }
    for p in &c.plans {
        println!(
            "limit: {:?} plan='{}' {:?} zaznamenane {} ({:?})",
            p.provider, p.plan, p.windows, p.recorded_at, p.source
        );
    }
    let today: Ymd = utc(now);
    for period in [Period::Today, Period::Last7, Period::Month] {
        println!("== {period:?} (od {:?})", period.start(today));
        for r in summarize(&c.days, today, period) {
            println!(
                "  {:<14} {:<20} in {:>9} out {:>9} cache r {:>9} w {:>9} req {:>5} odhad ${:.2} (ceny {}){}",
                r.provider.map(|p| p.label()).unwrap_or("?"),
                r.model,
                fmt_tokens(r.input),
                fmt_tokens(r.output),
                fmt_tokens(r.cache_read),
                fmt_tokens(r.cache_write),
                r.requests,
                r.est_usd,
                pricing::prices_date(r.provider),
                if r.unpriced > 0 { format!(" + {} bez ceny", r.unpriced) } else { String::new() }
            );
        }
    }
}
