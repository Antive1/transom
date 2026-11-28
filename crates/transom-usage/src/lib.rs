//! Vyuzitie AI (Claude Code, OpenAI Codex): zber z **lokalnych** zdrojov, cennik s datumom, suhrny za den,
//! poslednych 7 dni a kalendarny mesiac. Bez siete, bez prihlasovacich udajov, bez Win32.
//!
//! Zdroje (vsetky volitelne, predvolene vypnute):
//! - [`claude`]: prepisy Claude Code `~/.claude/projects/**/*.jsonl` - **nezdokumentovany interny format**
//!   (dokumentacia: "The entry format is internal to Claude Code and changes between versions"). Cita sa len
//!   `message.usage`, `message.model`, `timestamp` a identifikatory odpovede na deduplikaciu.
//! - [`statusline`]: zaznamy mostu `transom-statusline` zo **zdokumentovaneho** vstupu statusline Claude Code
//!   (odhad ceny relacie, pre Pro/Max percenta 5h/7d limitu v case poslednej odpovede).
//! - [`codex`]: relacie Codex `~/.codex/sessions/**/*.jsonl` - **nezdokumentovana schema** (dokumentacia
//!   spomina len `CODEX_HOME` a `history.jsonl`). Cita sa `token_usage_record`, model z `turn_context` a posledny
//!   snimok `rate_limits` (zaznamenany Codexom, nie aktualny stav).
//!
//! Sukromie: obsah konverzacii sa neuklada ani neodosiela; do cache ([`cache`]) idu len denne suhrny podla
//! poskytovatela a modelu (ziadne cesty, projekty, ID relacii ani uctov). `auth.json` sa nikdy neotvara.
pub mod account;
pub mod archive;

/// Najvacsi zaznam (`*.jsonl`) lokalneho logu nastroja, ktory sa nacita: vacsi subor sa preskoci (pocita sa ako chyba),
/// aby obrovsky alebo podvrhnuty subor nezahltil pamat ani zber.
pub const MAX_LOG_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Najvacsi subor mosta statusline (len cisla; rastie pripisovanim, preto je strop nizsi).
pub const MAX_BRIDGE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Precita subor najviac `cap` bajtov: cita sa cez `take(cap + 1)`, takze ani klamliva velkost (rastuci subor) nepreskoci
/// strop. Subor nad strop alebo necitatelny = `None`.
pub fn read_capped(path: &std::path::Path, cap: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() > cap {
        return None;
    }
    let mut buf = Vec::new();
    f.take(cap + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= cap).then_some(buf)
}
#[cfg(test)]
mod archive_flow;
pub mod cache;
pub mod claude;
pub mod claude_setup;
pub mod codex;
pub mod grok;
pub mod history;
#[cfg(test)]
mod history_archive_tests;
pub mod limits;
pub mod pricing;
pub mod statusline;

use transom_core::clock::{days_from_civil, Ymd};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Provider {
    ClaudeCode,
    Codex,
    /// xAI Grok CLI (lokalne relacie `~/.grok/sessions`).
    Grok,
}

impl Provider {
    /// Vsetci znami poskytovatelia v stabilnom poradi (poradie = priorita pri rovnosti, napr. pri `Auto`). Novy agent
    /// = dalsi variant + riadok tu + zdroj dat; ziadny plugin runtime.
    pub const ALL: [Provider; 3] = [Provider::ClaudeCode, Provider::Codex, Provider::Grok];

    pub fn label(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "Claude Code",
            Provider::Codex => "OpenAI Codex",
            Provider::Grok => "Grok CLI",
        }
    }
}

/// Jedno pouzitie modelu (jedna odpoved API). Tokeny su rozdelene tak, aby sa nepocitali dvakrat:
/// `input` = vstup **bez** cache, `cache_read` = citanie z cache, `cache_write_*` = zapis do cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub provider: Option<Provider>,
    pub model: String,
    /// Unix sekundy UTC.
    pub ts: i64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    /// Rychlost/uroven sluzby nestandardna (napr. `fast`, `priority`) - cena sa neodhaduje.
    pub nonstandard: bool,
    /// Vstup jedneho volania nad hranicou cennika (napr. dlhy kontext) - cena sa neodhaduje.
    pub over_price_limit: bool,
}

/// Unix sekundy z ISO 8601 UTC (`2026-11-20T13:22:01.123Z`); `None` pri inom tvare.
pub fn parse_iso_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // casove pasmo: Z alebo bez (UTC), +hh:mm / -hh:mm
    let rest = s.get(19..)?;
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let off = if rest.is_empty() || rest.starts_with('Z') {
        0
    } else if rest.len() >= 6 && (rest.starts_with('+') || rest.starts_with('-')) {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let oh: i64 = rest.get(1..3)?.parse().ok()?;
        let om: i64 = rest.get(4..6)?.parse().ok()?;
        sign * (oh * 3600 + om * 60)
    } else {
        return None;
    };
    let days = days_from_civil(Ymd::new(y as i32, mo as u32, d as u32));
    Some(days * 86_400 + h * 3600 + mi * 60 + se - off)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Today,
    /// Dnesok a 6 predchadzajucich dni (7 kalendarnych dni v lokalnom case).
    Last7,
    /// Aktualny kalendarny mesiac.
    Month,
}

impl Period {
    /// Prvy den obdobia.
    pub fn start(self, today: Ymd) -> Ymd {
        match self {
            Period::Today => today,
            Period::Last7 => transom_core::clock::civil_from_days(days_from_civil(today) - 6),
            Period::Month => Ymd::new(today.y, today.m, 1),
        }
    }

    pub fn contains(self, today: Ymd, d: Ymd) -> bool {
        d >= self.start(today) && d <= today
    }
}

/// Denny suhrn jedneho modelu (zaklad cache a vsetkych obdobi).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DayTotals {
    pub provider: Option<Provider>,
    pub model: String,
    pub day: (i32, u32, u32),
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub requests: u64,
    /// Odhad ceny v USD z cennika; pocitany len z ocenitelnych volani.
    pub est_usd: f64,
    /// Volania, ktore sa ocenit nedali (neznamy model, nestandardna rychlost, dlhy kontext).
    pub unpriced: u64,
    /// Sucet pochadza z **archivu** (zdroj ho uz necita), nie z aktualne citatelnych logov.
    #[serde(default)]
    pub archived: bool,
    /// Den mohol byt neuplny (hranicny den zdroja / chyby citania; len z archivu - inak sa urcuje z pokrytia).
    #[serde(default)]
    pub partial: bool,
    /// Datum cennika, z ktoreho je odhad ceny (len pre riadky z archivu; inak prazdne = aktualny cennik).
    #[serde(default)]
    pub price_date: String,
}

/// Najvacsia hodnota jedneho tokenoveho pola jedneho zaznamu (2^40, bilion): skutocne zaznamy su rady mensie, poskodena
/// hodnota (napr. `u64::MAX`) sa orezava uz pri citani, aby nijaky sucet nepretiekol.
pub(crate) const MAX_TOKENS_PER_FIELD: u64 = 1 << 40;

/// Zluci udalosti do dennych suhrnov podla lokalneho dna (`local_day` prevedie Unix sekundy na lokalny datum).
pub fn by_day(events: &[Event], local_day: &dyn Fn(i64) -> Ymd) -> Vec<DayTotals> {
    use std::collections::BTreeMap;
    type Key = (Option<Provider>, String, (i32, u32, u32));
    let mut m: BTreeMap<Key, DayTotals> = BTreeMap::new();
    for e in events {
        let d = local_day(e.ts);
        let key = (e.provider, e.model.clone(), (d.y, d.m, d.d));
        let t = m.entry(key).or_insert_with(|| DayTotals {
            provider: e.provider,
            model: e.model.clone(),
            day: (d.y, d.m, d.d),
            ..Default::default()
        });
        t.input = t.input.saturating_add(e.input);
        t.output = t.output.saturating_add(e.output);
        t.cache_read = t.cache_read.saturating_add(e.cache_read);
        t.cache_write = t
            .cache_write
            .saturating_add(e.cache_write_5m.saturating_add(e.cache_write_1h));
        t.requests = t.requests.saturating_add(1);
        match pricing::estimate(e) {
            Some(c) => t.est_usd += c,
            None => t.unpriced += 1,
        }
    }
    m.into_values().collect()
}

/// Suhrn modelu za obdobie.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelTotals {
    pub provider: Option<Provider>,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub requests: u64,
    pub est_usd: f64,
    pub unpriced: u64,
}

/// Suhrny za obdobie podla poskytovatela a modelu (zoradene podla poskytovatela, potom tokenov zostupne).
pub fn summarize(days: &[DayTotals], today: Ymd, period: Period) -> Vec<ModelTotals> {
    let mut out: Vec<ModelTotals> = Vec::new();
    for d in days {
        let day = Ymd::new(d.day.0, d.day.1, d.day.2);
        if !period.contains(today, day) {
            continue;
        }
        let row = match out
            .iter_mut()
            .find(|r| r.provider == d.provider && r.model == d.model)
        {
            Some(r) => r,
            None => {
                out.push(ModelTotals {
                    provider: d.provider,
                    model: d.model.clone(),
                    ..Default::default()
                });
                out.last_mut().expect("prave pridane")
            }
        };
        row.input += d.input;
        row.output += d.output;
        row.cache_read += d.cache_read;
        row.cache_write += d.cache_write;
        row.requests += d.requests;
        row.est_usd += d.est_usd;
        row.unpriced += d.unpriced;
    }
    out.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then((b.input + b.output + b.cache_read).cmp(&(a.input + a.output + a.cache_read)))
    });
    out
}

/// Stav jedneho zdroja (pre panel a nastavenia).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceStatus {
    /// Druh zdroja (text pre pouzivatela prekladá UI).
    pub kind: SourceKind,
    pub provider: Option<Provider>,
    /// Kedy sa zdroj naposledy cital (Unix sekundy; aj neuspesne); 0 = nikdy.
    pub refreshed_at: i64,
    /// Kedy sa zdroj naposledy **uspesne** precital (bez chyby; Unix sekundy); 0 = nikdy. Pri neuspesnom citani
    /// sa prenasa z predchadzajuceho zberu (`Collected::carry_last_ok`), takze neuspech nevyzera ako cerstvy udaj.
    #[serde(default)]
    pub last_ok_at: i64,
    pub files: usize,
    pub records: usize,
    /// Najstarsi a najnovsi zaznam (Unix sekundy).
    pub oldest: Option<i64>,
    pub newest: Option<i64>,
    pub error: Option<SourceError>,
    /// Statusline most: aspon jeden zaznam niesol limity planu (`rate_limits`). `false` pri existujucich zaznamoch =
    /// ucet limity nehlasi (dokumentacia: len claude.ai Pro/Max/Ultra, az po prvej odpovedi) - diagnostika, nie chyba.
    #[serde(default)]
    pub limits_seen: bool,
}

/// Druh zdroja dat o vyuziti.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SourceKind {
    /// Lokalne prepisy Claude Code (nezdokumentovany format).
    #[default]
    ClaudeLocal,
    /// Most statusline Claude Code (zdokumentovane rozhranie).
    ClaudeStatusline,
    /// Lokalne relacie Codex (nezdokumentovana schema).
    CodexLocal,
    /// Lokalne relacie Grok CLI (nezdokumentovany format).
    GrokLocal,
}

/// Problem so zdrojom (text pre pouzivatela prekladá UI).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SourceError {
    /// Adresar zdroja neexistuje.
    Missing,
    /// Niektore subory sa nepodarilo precitat.
    ReadErrors(usize),
    /// Most statusline este nic nezaznamenal.
    NoBridgeData,
}

/// Odkial je snimok limitu planu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SnapshotSource {
    /// Statusline Claude Code (stav pri poslednej odpovedi v relacii).
    #[default]
    ClaudeStatusline,
    /// Relacia Codex (`token_count.rate_limits`, zaznamenane Codexom).
    CodexSession,
}

/// Jedno okno limitu planu: dlzka v minutach (300 = 5 h, 10 080 = 7 dni, 43 200 = 30 dni), spotrebovane percento
/// a cas resetu (Unix sekundy), ako ich nahlasil nastroj.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LimitWindow {
    pub minutes: u32,
    pub used_percent: f64,
    pub resets_at: Option<i64>,
}

impl SourceStatus {
    /// Data su zastarale: posledne **uspesne** citanie je starsie ako `max_age_s` (napr. 2x interval obnovy)
    /// alebo nikdy neprebehlo.
    pub fn is_stale(&self, now: i64, max_age_s: i64) -> bool {
        self.last_ok_at <= 0 || now - self.last_ok_at > max_age_s
    }

    /// Lokalna historia nemusi pokryt cele obdobie: najstarsi zaznam je az po zaciatku obdobia
    /// (napr. Claude Code maze prepisy po `cleanupPeriodDays`, predvolene 30 dni).
    pub fn may_be_incomplete(&self, period_start: Ymd, local_day: &dyn Fn(i64) -> Ymd) -> bool {
        match self.oldest {
            Some(o) => local_day(o) > period_start,
            None => true,
        }
    }
}

/// Snimok limitu planu, ako ho **zapisal nastroj** v case `recorded_at` (nie aktualny stav uctu).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanSnapshot {
    pub provider: Option<Provider>,
    /// Napr. `go` (Codex) alebo `pro/max` podla zdroja; prazdne = nezname.
    pub plan: String,
    pub windows: Vec<LimitWindow>,
    pub recorded_at: i64,
    pub source: SnapshotSource,
}

/// Zaplnenie kontextu okna, ako ho **zapisal nastroj** v poslednej relacii (Claude Code statusline,
/// `context_window.used_percentage` a `context_window_size`; zdokumentovane). Nie je to kvota planu ani sucet relacii.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContextSnapshot {
    pub provider: Option<Provider>,
    pub used_percent: f64,
    /// Velkost kontextoveho okna v tokenoch (0 = zdroj ju nehlasi).
    pub window_tokens: u64,
    pub recorded_at: i64,
}

/// Odhad ceny za lokalny den (rok, mesiac, den) v USD.
pub type DayCost = ((i32, u32, u32), f64);

/// Vysledok jedneho zberu.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Collected {
    pub days: Vec<DayTotals>,
    pub statuses: Vec<SourceStatus>,
    pub plans: Vec<PlanSnapshot>,
    /// Odhad ceny relacii zo statusline podla dna (USD) - samostatne, nie scitane s tokenmi z prepisov.
    pub statusline_cost_by_day: Vec<DayCost>,
    /// Posledne zaplnenie kontextu zo zdrojov, ktore ho hlasia (zatial len statusline Claude Code).
    #[serde(default)]
    pub context: Vec<ContextSnapshot>,
    /// Pokrytie archivu: suvisle rozsahy dni, ktore sa uspesne naskenovali (pre mesacnu historiu).
    #[serde(default)]
    pub cover: Vec<archive::Cover>,
    /// Stav archivu pri tomto zbere.
    #[serde(default)]
    pub archive: ArchiveStatus,
}

/// Stav archivu dennych suctov pri zbere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArchiveStatus {
    /// Archiv sa nepouziva (vypnuty a ziadny subor).
    #[default]
    Off,
    /// Zapnuty: tento zber zlucil denne sucty do archivu.
    Writing,
    /// Pozastaveny alebo vypnuty, ale subor existuje: len sa cita (nic sa nezapisuje).
    ReadOnly,
    /// Subor archivu je poskodeny alebo novsej schemy: nepouzije sa a neprepise.
    Unreadable,
}

impl Collected {
    /// Pri zdroji, ktory sa tentokrat nepodarilo precitat bez chyby, prevezme cas posledneho uspesneho citania
    /// z predchadzajuceho zberu (podla druhu zdroja). Neuspech tak nikdy neposunie "posledne uspesne obnovenie".
    pub fn carry_last_ok(&mut self, prev: &Collected) {
        for st in self.statuses.iter_mut().filter(|s| s.last_ok_at <= 0) {
            if let Some(p) = prev.statuses.iter().find(|p| p.kind == st.kind) {
                st.last_ok_at = p.last_ok_at;
            }
        }
    }

    /// Najnovsi cas uspesneho citania spomedzi zdrojov (0 = nikdy).
    pub fn last_ok(&self) -> i64 {
        self.statuses
            .iter()
            .map(|s| s.last_ok_at)
            .max()
            .unwrap_or(0)
    }
}

/// Zdroje na citanie (z konfiguracie) a ich umiestnenie.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub claude_dir: Option<std::path::PathBuf>,
    pub codex_dir: Option<std::path::PathBuf>,
    pub grok_dir: Option<std::path::PathBuf>,
    pub statusline_file: Option<std::path::PathBuf>,
    /// Archiv dennych suctov (`None` = nepouziva sa).
    pub archive: Option<ArchiveSource>,
}

/// Nastavenie archivu pre zber.
#[derive(Clone, Debug)]
pub struct ArchiveSource {
    pub path: std::path::PathBuf,
    /// Zapisovat (zapnute a nie pozastavene); inak sa existujuci subor len cita.
    pub write: bool,
    /// Retencia v mesiacoch (0 = bez obmedzenia); uplatni sa len pri zapise.
    pub retention_months: u32,
}

/// Vysledky skenu lokalnych zdrojov z ich stavu (pre zlucenie do archivu).
fn scans_of(statuses: &[SourceStatus], local_day: &dyn Fn(i64) -> Ymd) -> Vec<archive::Scan> {
    statuses
        .iter()
        .filter_map(|st| {
            let provider = match st.kind {
                SourceKind::ClaudeLocal => Provider::ClaudeCode,
                SourceKind::CodexLocal => Provider::Codex,
                SourceKind::GrokLocal => Provider::Grok,
                SourceKind::ClaudeStatusline => return None,
            };
            Some(archive::Scan {
                provider,
                source: st.kind,
                oldest: st.oldest.map(local_day),
                errors: matches!(st.error, Some(SourceError::ReadErrors(_))),
                missing: matches!(st.error, Some(SourceError::Missing)),
            })
        })
        .collect()
}

/// Zber zo zapnutych zdrojov (blokuje - volat mimo UI vlakna). `now` = Unix sekundy.
pub fn collect(src: &Sources, now: i64, local_day: &dyn Fn(i64) -> Ymd) -> Collected {
    let mut out = Collected::default();
    let mut events = Vec::new();
    if let Some(dir) = &src.claude_dir {
        let (ev, st) = claude::read_dir(dir);
        events.extend(ev);
        out.statuses.push(SourceStatus {
            refreshed_at: now,
            last_ok_at: if st.error.is_none() { now } else { 0 },
            ..st
        });
    }
    if let Some(dir) = &src.codex_dir {
        let (ev, st, plan) = codex::read_dir(dir);
        events.extend(ev);
        out.statuses.push(SourceStatus {
            refreshed_at: now,
            last_ok_at: if st.error.is_none() { now } else { 0 },
            ..st
        });
        out.plans.extend(plan);
    }
    if let Some(d) = &src.grok_dir {
        let (ev, st) = grok::read_dir(d);
        events.extend(ev);
        out.statuses.push(SourceStatus {
            refreshed_at: now,
            last_ok_at: if st.error.is_none() { now } else { 0 },
            ..st
        });
    }
    if let Some(f) = &src.statusline_file {
        let (cost, st, plan, ctx) = statusline::read_file(f, local_day);
        out.statusline_cost_by_day = cost;
        out.context.extend(ctx);
        out.statuses.push(SourceStatus {
            refreshed_at: now,
            last_ok_at: if st.error.is_none() { now } else { 0 },
            ..st
        });
        out.plans.extend(plan);
    }
    // Grok: plan len z prihlasenia (OIDC = SuperGrok); limity lokalne nie su
    if src.grok_dir.is_some() {
        if let Some(plan) = account::grok_account().0 {
            out.plans.push(PlanSnapshot {
                provider: Some(Provider::Grok),
                plan,
                windows: Vec::new(),
                recorded_at: now,
                source: SnapshotSource::default(),
            });
        }
    }
    // statusline nevie, ci ide o Pro alebo Max: skutocny plan uctu z lokalneho suboru Claude Code
    if let Some(name) = account::claude_plan() {
        for p in out
            .plans
            .iter_mut()
            .filter(|p| p.provider == Some(Provider::ClaudeCode))
        {
            p.plan = name.clone();
        }
    }
    out.days = by_day(&events, local_day);
    if let Some(ar) = &src.archive {
        apply_archive(&mut out, ar, now, local_day);
    }
    out
}

/// Zluci denne sucty do archivu (ak sa zapisuje) a nahradi `days` zjednotenym pohladom: cerstve (citatelne) dni + dni, ktore
/// zdroj uz nema. Poskodeny / novsi archiv sa nepouzije a neprepise.
fn apply_archive(
    out: &mut Collected,
    ar: &ArchiveSource,
    now: i64,
    local_day: &dyn Fn(i64) -> Ymd,
) {
    use archive::Loaded;
    let stamp_before = archive::stamp(&ar.path);
    let loaded = archive::load(&ar.path);
    let (mut a, existed) = match loaded {
        Loaded::Unreadable => {
            out.archive = ArchiveStatus::Unreadable;
            return;
        }
        Loaded::Missing if !ar.write => return,
        Loaded::Missing => (archive::Archive::default(), false),
        Loaded::Ok(a) => (a, true),
    };
    let today = local_day(now);
    let scans = scans_of(&out.statuses, local_day);
    let before = (a.rows.clone(), a.cover.clone());
    let (_, live) = archive::merge(&mut a, &scans, &out.days, today, &|p| {
        pricing::prices_date(Some(p)).to_string()
    });
    if ar.write {
        archive::prune(&mut a, today, ar.retention_months);
        // zapisuje sa len pri zmene obsahu (opakovany sken nic nezapise ani nemeni cas suboru)
        if !existed || (a.rows.clone(), a.cover.clone()) != before {
            a.updated_at = now;
            // ak sa subor medzitym zmenil zvonka (zmazanie / retencia z nastaveni), nezapise sa: zber by ho vzkriesil
            let _ = archive::save_if_unchanged(&ar.path, &a, stamp_before);
        }
        out.archive = ArchiveStatus::Writing;
    } else {
        out.archive = ArchiveStatus::ReadOnly;
    }
    // po retencii (len pri zapise) a zlucenom pohlade: riadky -> `DayTotals` s povodom
    let live_keys: std::collections::HashSet<(Provider, &str, (i32, u32, u32))> = a
        .rows
        .iter()
        .zip(&live)
        .filter(|(_, l)| **l)
        .map(|(r, _)| (r.provider, r.model.as_str(), r.day))
        .collect();
    out.days = a
        .rows
        .iter()
        .map(|r| r.to_day(!live_keys.contains(&(r.provider, r.model.as_str(), r.day))))
        .collect();
    out.cover = a.cover.clone();
}

/// Kratky zapis poctu tokenov: 950, 12,3 k, 4,56 M.
pub fn fmt_tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1} k", n as f64 / 1e3).replace('.', ",")
    } else if n < 1_000_000_000 {
        format!("{:.2} M", n as f64 / 1e6).replace('.', ",")
    } else {
        format!("{:.2} G", n as f64 / 1e9).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_capped_refuses_oversized_files_and_never_reads_past_the_cap() {
        let d = std::env::temp_dir().join(format!("transom-capped-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("a.jsonl");
        std::fs::write(&f, b"0123456789").unwrap();
        assert_eq!(read_capped(&f, 10).as_deref(), Some(&b"0123456789"[..]));
        assert_eq!(read_capped(&f, 9), None, "o bajt vacsi nez strop");
        assert_eq!(read_capped(&f, 1000).map(|b| b.len()), Some(10));
        assert_eq!(read_capped(&d.join("missing"), 10), None);
        assert_eq!(read_capped(&d, 10), None, "priecinok");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Testovaci "lokalny cas" = UTC+1.
    fn local(ts: i64) -> Ymd {
        transom_core::clock::civil_from_days((ts + 3600).div_euclid(86_400))
    }

    fn ev(model: &str, iso: &str, input: u64, output: u64) -> Event {
        Event {
            provider: Some(Provider::ClaudeCode),
            model: model.into(),
            ts: parse_iso_utc(iso).unwrap(),
            input,
            output,
            ..Default::default()
        }
    }

    #[test]
    fn iso_parsing() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_utc("2026-11-20T13:22:01.123Z"),
            Some(1_795_180_921)
        );
        assert_eq!(
            parse_iso_utc("2026-11-20T14:22:01+01:00"),
            Some(1_795_180_921)
        );
        assert_eq!(parse_iso_utc("2026-11-20T13:22:01"), Some(1_795_180_921));
        assert_eq!(parse_iso_utc("nie"), None);
        assert_eq!(parse_iso_utc("2026-13-20T13:22:01Z"), None);
        // ne-ASCII znak za sekundami nesmie sposobit paniku (release = panic abort)
        assert_eq!(parse_iso_utc("2026-11-20T13:22:01\u{e9}x"), None);
        assert_eq!(parse_iso_utc("2026-11-20T13:22:0\u{e9}"), None);
    }

    #[test]
    fn periods_use_local_calendar_days() {
        let today = Ymd::new(2026, 11, 20);
        assert_eq!(Period::Last7.start(today), Ymd::new(2026, 11, 14));
        assert_eq!(Period::Month.start(today), Ymd::new(2026, 11, 1));
        assert!(Period::Last7.contains(today, Ymd::new(2026, 11, 14)));
        assert!(!Period::Last7.contains(today, Ymd::new(2026, 11, 13)));
        assert!(
            !Period::Today.contains(today, Ymd::new(2026, 11, 21)),
            "buducnost nie"
        );
        // prelom mesiaca: 7 dni siaha do predchadzajuceho mesiaca, mesiac nie
        let d = Ymd::new(2026, 12, 3);
        assert_eq!(Period::Last7.start(d), Ymd::new(2026, 11, 27));
        assert!(!Period::Month.contains(d, Ymd::new(2026, 11, 30)));
    }

    #[test]
    fn aggregation_by_local_day_and_period() {
        let events = vec![
            // 23:30 UTC 19. 11. = 00:30 lokalne 20. 11. -> patri dnesku
            ev("claude-opus-5-5", "2026-11-19T23:30:00Z", 1000, 100),
            ev("claude-opus-5-5", "2026-11-20T10:00:00Z", 2000, 200),
            ev("claude-sonnet-5-5", "2026-11-15T10:00:00Z", 500, 50),
            ev("claude-opus-5-5", "2026-11-01T08:00:00Z", 10, 1),
            ev("claude-opus-5-5", "2026-10-31T08:00:00Z", 7, 7),
        ];
        let days = by_day(&events, &local);
        let today = Ymd::new(2026, 11, 20);
        let t = summarize(&days, today, Period::Today);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].input, t[0].output, t[0].requests), (3000, 300, 2));
        let w = summarize(&days, today, Period::Last7);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].model, "claude-opus-5-5", "zoradene podla objemu");
        let m = summarize(&days, today, Period::Month);
        let opus = m.iter().find(|r| r.model == "claude-opus-5-5").unwrap();
        assert_eq!(opus.input, 3010, "31. 10. nepatri do novembra");
    }

    #[test]
    fn estimate_and_unpriced_are_separate() {
        let mut unknown = ev("claude-neznamy", "2026-11-20T10:00:00Z", 1_000_000, 0);
        unknown.provider = Some(Provider::ClaudeCode);
        let known = ev("claude-opus-5-5", "2026-11-20T10:00:00Z", 1_000_000, 0);
        let days = by_day(&[unknown, known], &local);
        let r = summarize(&days, Ymd::new(2026, 11, 20), Period::Today);
        let u = r.iter().find(|x| x.model == "claude-neznamy").unwrap();
        assert_eq!(
            (u.est_usd, u.unpriced),
            (0.0, 1),
            "neznamy model = bez ceny, nie $0 odhad"
        );
        let k = r.iter().find(|x| x.model == "claude-opus-5-5").unwrap();
        assert!((k.est_usd - 4.0).abs() < 1e-9);
        assert_eq!(k.unpriced, 0);
    }

    #[test]
    fn failed_read_never_advances_last_successful_refresh() {
        let ok = |kind, t| SourceStatus {
            kind,
            refreshed_at: t,
            last_ok_at: t,
            ..Default::default()
        };
        let prev = Collected {
            statuses: vec![
                ok(SourceKind::ClaudeLocal, 1000),
                ok(SourceKind::CodexLocal, 1000),
            ],
            ..Default::default()
        };
        // dalsi zber: Claude v poriadku, Codex zlyhal (chyba) - cas citania sa posunul, uspech nie
        let mut next = Collected {
            statuses: vec![
                ok(SourceKind::ClaudeLocal, 2000),
                SourceStatus {
                    kind: SourceKind::CodexLocal,
                    refreshed_at: 2000,
                    last_ok_at: 0,
                    error: Some(SourceError::Missing),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        next.carry_last_ok(&prev);
        assert_eq!(next.statuses[0].last_ok_at, 2000);
        assert_eq!(
            (next.statuses[1].refreshed_at, next.statuses[1].last_ok_at),
            (2000, 1000)
        );
        assert_eq!(next.last_ok(), 2000);
        // zdroj, ktory nikdy nepresiel: ostava 0 = "nikdy", t.j. zastarale (nie cerstva nula)
        let mut fresh = Collected {
            statuses: vec![SourceStatus {
                kind: SourceKind::ClaudeStatusline,
                refreshed_at: 5,
                error: Some(SourceError::NoBridgeData),
                ..Default::default()
            }],
            ..Default::default()
        };
        fresh.carry_last_ok(&prev);
        assert_eq!(fresh.statuses[0].last_ok_at, 0);
        assert!(fresh.statuses[0].is_stale(10, 7200));
        // zastaralost sa meria od posledneho USPESNEHO citania, nie od posledneho pokusu
        assert!(next.statuses[1].is_stale(1000 + 8000, 7200));
        assert!(!next.statuses[1].is_stale(1000 + 3600, 7200));
    }

    #[test]
    fn stale_and_incomplete_flags() {
        let st = SourceStatus {
            refreshed_at: 1000,
            last_ok_at: 1000,
            oldest: Some(parse_iso_utc("2026-11-05T10:00:00Z").unwrap()),
            ..Default::default()
        };
        assert!(!st.is_stale(1000 + 3600, 7200));
        assert!(st.is_stale(1000 + 8000, 7200));
        assert!(
            SourceStatus::default().is_stale(5, 7200),
            "nikdy necitane = zastarale"
        );
        let today = Ymd::new(2026, 11, 20);
        assert!(
            st.may_be_incomplete(Period::Month.start(today), &local),
            "1. 11. < najstarsi 5. 11."
        );
        assert!(!st.may_be_incomplete(Period::Last7.start(today), &local));
        assert!(SourceStatus::default().may_be_incomplete(today, &local));
    }

    #[test]
    fn token_formatting() {
        assert_eq!(fmt_tokens(950), "950");
        assert_eq!(fmt_tokens(12_345), "12,3 k");
        assert_eq!(fmt_tokens(4_560_000), "4,56 M");
        assert_eq!(fmt_tokens(2_590_000_000), "2,59 G");
    }
}
