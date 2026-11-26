//! Volitelny most pre Claude Code: pouzivatel ho sam nastavi ako `statusLine` v `~/.claude/settings.json`
//! (`{"statusLine": {"type": "command", "command": "<cesta s lomkami>/transom-statusline.exe"}}`; "Pripojit Claude Code" v nastaveniach to urobi bezpecne). Claude Code posle na stdin
//! zdokumentovany JSON (code.claude.com/docs/en/statusline); most:
//! - vypise kratky stavovy riadok (model, odhad ceny relacie, 5h limit),
//! - ak je v nastaveniach transom zapnuty zdroj "statusline", prida do `%LOCALAPPDATA%\transom\claude-statusline.jsonl`
//!   riadok len s cislami (odhad ceny, percenta limitov, cas, hash relacie, id modelu). Ziadne cesty ani text.
//!
//! Most nikdy nezlyha nahlas: pri akejkolvek chybe len vypise "Claude", aby statusline Claude Code fungoval dalej.
use std::io::Read;

/// Najvacsi vstup zo stdin: JSON Claude Code ma par KiB; vacsi (alebo nekonciaci) vstup sa nedocita do pamate a neplatny
/// JSON znamena len riadok "Claude".
const MAX_STDIN: u64 = 1024 * 1024;

fn main() {
    let mut buf = String::new();
    let _ = std::io::stdin().take(MAX_STDIN).read_to_string(&mut buf);
    let input: serde_json::Value = serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null);
    // `--check`: skuska z nastaveni transom - vypise riadok, nic nezapisuje a nic nenacitava z konfiguracie
    if std::env::args().any(|a| a == "--check") {
        let text = if input.is_null() {
            "Claude".to_string()
        } else {
            transom_usage::statusline::status_text(&input, "est.")
        };
        println!("{text}");
        return;
    }
    let cfg = transom_config::load(&transom_config::default_path()).config;
    // `--hook <druh>`: udalost z hookov Claude Code (zaciatok, nastroj, koniec, otazka); zapise sa len druh a cas,
    // nic sa nevypise (hook nesmie nic menit ani spomalit)
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--hook") {
        if cfg.usage.claude_statusline {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let arg = args.get(i + 1).map(String::as_str).unwrap_or("");
            let kind = transom_usage::statusline::hook_kind(arg, &input);
            let _ = transom_usage::statusline::append_record(
                &transom_usage::cache::events_path(),
                &transom_usage::statusline::hook_record(kind, &input, now),
            );
        }
        return;
    }
    // jazyk ako v liste (nastavenie aplikacie, pri "system" jazyk Windows; inak anglictina)
    transom_i18n::set_lang(transom_i18n::resolve(
        &cfg.language,
        &transom_platform::clock::ui_language_tag(),
    ));
    let enabled = cfg.usage.claude_statusline;
    if enabled {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if let Some(rec) = transom_usage::statusline::record(&input, now) {
            // chyba zapisu nesmie zhodit ani spomalit stavovy riadok Claude Code
            let _ = transom_usage::statusline::append_record(
                &transom_usage::cache::statusline_path(),
                &rec,
            );
        }
    }
    let text = if input.is_null() {
        "Claude".to_string()
    } else {
        transom_usage::statusline::status_text(&input, &transom_i18n::t("bridge.estimate"))
    };
    println!("{text}");
}
