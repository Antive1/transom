//! Pripojenie Claude Code na stranke AI a sukromie: stav, pripojenie, vratenie a skuska mostu. Logika suborov je v
//! `transom_usage::claude_setup` (testovana); tu je len spojenie s oknom a lokalizovane spravy. Nikdy sa nic nelogguje ani
//! nezobrazuje zo vstupu statusLine (skuska pouziva vlastny vymysleny vzorok).
use std::path::PathBuf;
use transom_i18n::{t, tf};
use transom_usage::claude_setup as cs;

#[derive(Clone, Debug, Default)]
pub struct Cc {
    /// Stav posledneho nacitania (`None` = este nenacitane / ukazka).
    pub conn: Option<cs::Connection>,
    pub records: usize,
    pub newest: Option<i64>,
    /// Existuje zaznam o nasom zapise (je co vratit).
    pub can_rollback: bool,
    /// Druhy krok nahradenia cudzieho `statusLine`.
    pub confirm_replace: bool,
    /// Vysledok poslednej skusky mostu (lokalizovany text).
    pub test: Option<String>,
    /// Zaznamy mostu existuju len v sukromnom ulozisku aplikacie Claude Desktop (MSIX).
    pub redirected: bool,
}

impl Cc {
    pub fn conn(&self) -> cs::Connection {
        self.conn.clone().unwrap_or(cs::Connection::NotConfigured)
    }

    /// Cesta k suboru nastaveni Claude Code (na zobrazenie).
    pub fn settings_path() -> PathBuf {
        cs::settings_path()
    }

    pub fn bridge_exe() -> PathBuf {
        std::env::current_exe()
            .map(|p| p.with_file_name("transom-statusline.exe"))
            .unwrap_or_default()
    }

    /// Prikaz zapisovany do `statusLine.command` (lomky, uvodzovky pri medzerach).
    pub fn command() -> String {
        cs::command_for(&Self::bridge_exe())
    }

    /// Znovu nacita stav zo suborov (len metadata kluca `statusLine` a pocet zaznamov mosta).
    pub fn refresh(&mut self) {
        let text = std::fs::read(Self::settings_path())
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let state = cs::inspect(text.as_deref());
        let (n, newest, limits) = cs::bridge_info(&transom_usage::cache::statusline_path());
        self.records = n;
        self.newest = newest;
        self.conn = Some(cs::connection(&state, n, limits));
        // nas prikaz, ale most neexistuje / je zo starsej kopie: zaznamy neprichadzaju, ponuknut nove pripojenie
        if matches!(state, cs::StatusLineState::Ours) {
            if let Some(cmd) = text.as_deref().and_then(cs::current_command) {
                let no_hooks = text.as_deref().is_some_and(|t| !cs::has_our_hooks(t));
                if cs::is_outdated(&cmd, &Self::bridge_exe()) || no_hooks {
                    self.conn = Some(cs::Connection::Outdated(cmd));
                }
            }
        }
        self.can_rollback = cs::state_path().exists();
        self.redirected = n == 0 && cs::redirected_bridge_file().is_some();
        if !matches!(self.conn(), cs::Connection::Foreign(_)) {
            self.confirm_replace = false;
        }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Pripoji. Vrati (sprava pre stavovy riadok, uspech).
    pub fn connect(&mut self, replace: bool) -> (String, bool) {
        if !Self::bridge_exe().is_file() {
            return (t("cc.err.nobridge"), false);
        }
        let r = cs::connect(
            &Self::settings_path(),
            &cs::state_path(),
            &Self::command(),
            replace,
            Self::now(),
        );
        self.confirm_replace = false;
        self.test = None;
        let out = match r {
            Ok(rec) if rec.backup.is_empty() => (t("cc.done.created"), true),
            Ok(_) => (t("cc.done.connected"), true),
            Err(cs::SetupError::Invalid) => (t("cc.err.invalid"), false),
            Err(cs::SetupError::VerificationFailed) => (t("cc.err.verify"), false),
            Err(cs::SetupError::Io(e)) => (tf("cc.err.io", &[&e]), false),
        };
        self.refresh();
        out
    }

    /// Vrati nastavenia Claude Code do stavu pred pripojenim.
    pub fn disconnect(&mut self) -> String {
        let r = cs::rollback(&Self::settings_path(), &cs::state_path());
        self.test = None;
        let msg = match r {
            Ok(cs::Rollback::RestoredBackup) => t("cc.done.restored"),
            Ok(cs::Rollback::RemovedOurEntry) => t("cc.done.removed"),
            Ok(cs::Rollback::NothingToRollBack) => t("cc.done.nothing"),
            Err(cs::SetupError::Invalid) => t("cc.err.invalid"),
            Err(cs::SetupError::VerificationFailed) => t("cc.err.verify"),
            Err(cs::SetupError::Io(e)) => tf("cc.err.io", &[&e]),
        };
        self.refresh();
        msg
    }

    /// Skuska mostu: spusti `transom-statusline --check` s vymysleny vzorkom (nic sa nezapisuje) a porovna so stavom zaznamov.
    pub fn run_test(&mut self) {
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        self.refresh();
        let exe = Self::bridge_exe();
        let sample = r#"{"model":{"id":"claude-sample","display_name":"Sample"},"cost":{"total_cost_usd":0.01},"rate_limits":{"five_hour":{"used_percentage":12.5,"resets_at":4102444800}},"context_window":{"used_percentage":3}}"#;
        let run = Command::new(&exe)
            .arg("--check")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .and_then(|mut c| {
                if let Some(mut si) = c.stdin.take() {
                    let _ = si.write_all(sample.as_bytes());
                }
                c.wait_with_output()
            });
        let mut msg = match run {
            Ok(o) if o.status.success() && !o.stdout.is_empty() => {
                let text = String::from_utf8_lossy(&o.stdout);
                tf("cc.test.ok", &[text.trim()])
            }
            Ok(o) => tf("cc.test.fail", &[&format!("{:?}", o.status.code())]),
            Err(e) => tf("cc.test.fail", &[&e.kind().to_string()]),
        };
        msg.push(' ');
        msg.push_str(&match self.conn() {
            cs::Connection::NotConfigured => t("cc.test.notconf"),
            cs::Connection::Foreign(_) => t("cc.test.foreign"),
            cs::Connection::Error(_) => t("cc.err.invalid"),
            cs::Connection::ConfiguredNoSnapshot => t("cc.test.nosnap"),
            cs::Connection::Outdated(_) => t("cc.test.outdated"),
            cs::Connection::Connected { limits: true } => {
                tf("cc.test.limits", &[&self.records.to_string()])
            }
            cs::Connection::Connected { limits: false } => {
                tf("cc.test.nolimits", &[&self.records.to_string()])
            }
        });
        if self.redirected {
            msg.push(' ');
            msg.push_str(&t("cc.test.redirected"));
        }
        self.test = Some(msg);
    }
}
