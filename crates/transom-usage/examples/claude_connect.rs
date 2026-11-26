//! Pomocny nastroj na overenie pripojenia Claude Code bez okna nastaveni (pouziva tie iste funkcie ako tlacidla).
//! `claude_connect <bridge.exe> connect|replace|rollback|status` - cesty podla `CLAUDE_CONFIG_DIR` a `TRANSOM_DATA`.
use transom_usage::claude_setup as cs;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let exe = std::path::PathBuf::from(a.get(1).cloned().unwrap_or_default());
    let cmd = cs::command_for(&exe);
    let (s, r) = (cs::settings_path(), cs::state_path());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match a.get(2).map(String::as_str) {
        Some("connect") => println!(
            "{:?}",
            cs::connect(&s, &r, &cmd, false, now).map(|x| x.backup)
        ),
        Some("replace") => println!(
            "{:?}",
            cs::connect(&s, &r, &cmd, true, now).map(|x| x.backup)
        ),
        Some("rollback") => println!("{:?}", cs::rollback(&s, &r)),
        _ => {
            let text = std::fs::read_to_string(&s).ok();
            let (n, newest, limits) = cs::bridge_info(&transom_usage::cache::statusline_path());
            let state = cs::inspect(text.as_deref());
            println!(
                "{:?} records={n} newest={newest:?} limits={limits}",
                cs::connection(&state, n, limits)
            );
        }
    }
}
