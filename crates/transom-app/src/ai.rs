//! Vyuzitie AI (Claude Code, Codex): zber na pozadi, cache, kruhovy indikator 5-hodinoveho limitu v liste a panel.
//! Zber bezi len pri zapnutych zdrojoch: po zapneti, pri otvoreni panelu (data starsie nez 5 min), tlacidlom
//! Obnovit, v nastavenom intervale (predvolene 60 min) a pri zmene suboru mostu statusline (kontrola casu zmeny
//! suboru pri minutovom ticku hodin - ziadne dalsie prebudzanie).
//!
//! Percento limitu pochadza **len** zo snimku planu (statusline Claude Code, relacia Codex) - nikdy z tokenov ani
//! z odhadu ceny. Ked zdroj limit nehlasi, kruh je prazdny a text hovori "nedostupne".
use crate::ai_cards;
use crate::ai_month::{self, MonthCtx, MonthUi};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use transom_core::clock::Ymd;
use transom_i18n::{t, tf};
use transom_platform::clock as pclock;
use transom_platform::Notifier;
use transom_render::{Color, Font, HAlign, Painter, Rect, TextStyle, Theme};
use transom_usage::history::{Filter, Metric};
use transom_usage::limits::{
    self, Freshness, HumanDuration, LimitReading, ProviderChoice, ResetState, RingMode,
};
use transom_usage::{
    cache, fmt_tokens, pricing, summarize, Collected, Period, Provider, SnapshotSource,
    SourceError, SourceKind, Sources,
};

/// Kod notifikacie "zber skoncil".
pub const N_USAGE: u32 = 2;
/// Rozmer panelu v lp.
pub const AI_W_LP: i32 = 440;
pub const AI_H_LP: i32 = 760;
/// Hodnota limitu je "aktualna", ak ju nastroj zaznamenal pred menej nez 30 min (a okno sa neobnovilo).
pub const LIMIT_FRESH_S: i64 = 30 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiHit {
    Tab(Period),
    Refresh,
    Settings,
    /// Prepinac Podrobnosti (zdroje, casy zaznamov, chyby) namiesto historie.
    Details,
    /// Volba "Zostava / Spotrebovane" (ulozi sa do konfiguracie, takze meni aj kruh v liste).
    Mode(RingMode),
    /// Mesacny pohlad: predosly / dalsi / tento mesiac, jednotka grafu, filter poskytovatela, den v grafe.
    MonthPrev,
    MonthNext,
    MonthThis,
    Unit(Metric),
    Prov(Filter),
    Day(u32),
}

/// Co sa zobrazi v liste (cista data pre `view`).
#[derive(Clone, Debug, PartialEq)]
pub struct RingView {
    pub fill: Option<u8>,
    pub ccw: bool,
    pub muted: bool,
    pub label: String,
    pub name: String,
    /// Zostava malo kapacity (<= 10 %): ring farbou `low`, text tucne a so znackou "!" (nie len farba).
    pub alert: bool,
    /// Poskytovatel, okno a odpocet: "Claude · 5 h · resets in 2 h 14 min" (volitelny text; skryje sa pri nedostatku
    /// miesta ako celok, nikdy sa neskracuje).
    pub detail: Option<String>,
}

/// Kto spustil obnovu: automaticke (casovac, minutovy tik, otvorenie panelu, start) sa riadia nastavenym intervalom
/// (najmenej 5 min); pouzivatelske (Obnovit, zapnutie/vypnutie zdroja) nie - su to jednorazove ciny, nie dotazovanie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshKind {
    Auto,
    Manual,
}

/// Smie sa teraz spustit **automaticka** obnova? `last_started` = zaciatok predchadzajuceho zberu (0 = nikdy),
/// `interval_min` = nastaveny interval (`Usage::refresh_interval_minutes`, vzdy >= 5; `None` = len na poziadanie).
/// Jediny strazca: casovac, kontrola mostu statusline, otvorenie panelu aj start idu cez neho.
/// - interval: najviac raz za interval (a teda nikdy castejsie nez raz za 5 min),
/// - len na poziadanie: automaticky sa necita nic, okrem **prveho citania, ked este nie su ziadne data ani cache**
///   (inak by zapnuty zdroj ostal prazdny, kym pouzivatel nestlaci Obnovit).
pub fn auto_allowed(now: i64, last_started: i64, interval_min: Option<u32>) -> bool {
    match interval_min {
        None => last_started <= 0,
        Some(m) => {
            let gap = m.max(transom_config::MIN_REFRESH_MINUTES) as i64 * 60;
            last_started <= 0 || now - last_started >= gap
        }
    }
}

/// Po akom case bez uspesneho citania su udaje zastarale: 2 x interval (najmenej 10 min); pri "len na poziadanie"
/// hodina (nikto ich sam neobnovuje, ale cas posledneho uspechu sa ukazuje vzdy).
pub fn stale_after_s(u: &transom_config::Usage) -> i64 {
    match u.refresh_interval_minutes() {
        Some(m) => m as i64 * 60 * 2,
        None => 3600,
    }
}

pub struct AiState {
    /// Plynule posuvanie (snimka: `glide_step`).
    pub glide: transom_render::design::Glide,

    pub data: Option<Collected>,
    pub busy: bool,
    pub tab: Period,
    /// Zobrazit Podrobnosti (diagnostika) namiesto historie.
    pub details: bool,
    pub hits: Vec<(Rect, AiHit)>,
    pub hover: Option<AiHit>,
    /// Mesacny pohlad historie (mesiac, den, jednotka, filter) - len stav relacie panela.
    pub month: MonthUi,
    /// Posun spodnej casti (historia / podrobnosti) v px a jeho najvacsi povoleny posun z posledneho kreslenia.
    pub scroll: i32,
    pub max_scroll: i32,
    /// Po kresleni sa zmenil rozsah posuvania: treba dalsie prekreslenie.
    pub repaint: bool,
    /// Jeden "riadok" posunu a vyska posuvneho okna (px) z posledneho kreslenia - pre klavesnicu a koliesko.
    pub unit: i32,
    pub view_h: i32,
    /// Pohlad mesiaca z posledneho kreslenia (nazvy dni pre pristupnost).
    pub last_month: Option<transom_usage::history::MonthView>,
    /// Vysledok zberu spolu s epochou, v ktorej zber zacal.
    shared: Arc<Mutex<Option<(u64, Collected)>>>,
    /// Generacia lokalnych dat: zvysi sa pri zmazani dat z nastaveni. Zber zo starsej generacie sa zahodi (nezapise cache
    /// ani sa neprijme), aby bezici zber zmazane data nevzkriesil.
    epoch: Arc<AtomicU64>,
    /// Zmazanie prislo pocas zberu: po jeho skonceni treba spustit novy.
    rerun: bool,
    /// Subor cache (v testoch prepisatelny).
    cache_file: PathBuf,
    /// Cas zmeny suboru mostu statusline v case spustenia posledneho zberu (nove zaznamy = ina hodnota).
    collected_mtime: Option<std::time::SystemTime>,
    /// Zaciatok posledneho zberu (Unix sekundy; 0 = nikdy) - brana automatickych obnov.
    last_started: i64,
}

fn bridge_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(cache::statusline_path())
        .and_then(|m| m.modified())
        .ok()
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Zdroje podla konfiguracie (vypnute zdroje sa vobec necitaju).
pub fn sources(u: &transom_config::Usage) -> Sources {
    let home = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(Into::into)
        .unwrap_or(home.join(".claude"));
    let codex = std::env::var_os("CODEX_HOME")
        .map(Into::into)
        .unwrap_or(home.join(".codex"));
    Sources {
        claude_dir: u.claude_local.then_some(claude),
        codex_dir: u.codex_local.then_some(codex),
        statusline_file: u.claude_statusline.then(cache::statusline_path),
        // archiv dennych suctov: zapisuje sa len ked ho pouzivatel zapol (a nepozastavil); existujuci subor sa cita vzdy
        archive: {
            let path = transom_usage::archive::path();
            (u.archive.writing() || path.exists()).then(|| transom_usage::ArchiveSource {
                path,
                write: u.archive.writing(),
                retention_months: u.archive.retention_months,
            })
        },
    }
}

pub fn provider_short(p: Provider) -> &'static str {
    match p {
        Provider::ClaudeCode => "Claude",
        Provider::Codex => "Codex",
    }
}

/// Nazov okna limitu podla minut (nikdy podla textu zdroja).
pub fn window_name(minutes: u32) -> String {
    match minutes {
        limits::FIVE_HOURS => t("ai.limit.5h"),
        limits::WEEK => t("ai.limit.week"),
        m if m > 0 && m.is_multiple_of(1440) => tf("ai.limit.days", &[&(m / 1440).to_string()]),
        m if m > 0 && m.is_multiple_of(60) => tf("ai.limit.hours", &[&(m / 60).to_string()]),
        m => tf("ai.limit.minutes", &[&m.to_string()]),
    }
}

fn source_name(k: SourceKind) -> String {
    t(match k {
        SourceKind::ClaudeLocal => "src.claude_local",
        SourceKind::ClaudeStatusline => "src.claude_statusline",
        SourceKind::CodexLocal => "src.codex_local",
    })
}

pub(crate) fn snapshot_name(s: SnapshotSource) -> String {
    t(match s {
        SnapshotSource::ClaudeStatusline => "snap.claude",
        SnapshotSource::CodexSession => "snap.codex",
    })
}

fn error_text(e: SourceError) -> String {
    match e {
        SourceError::Missing => t("err.missing"),
        SourceError::ReadErrors(n) => tf("err.read", &[&n.to_string()]),
        SourceError::NoBridgeData => t("err.no_bridge"),
    }
}

pub(crate) fn when(ts: i64) -> String {
    if ts > 0 {
        pclock::short_datetime(ts)
    } else {
        t("ai.unknown_time")
    }
}

/// Zostavajuci cas v jazyku aplikacie ("2 h 14 min"); jednotky su skratky (bez mnozneho cisla).
/// Zalomeny text v panele: kazdy riadok len ak je v panele miesto (`room`), inak sa vynecha a pata ukaze "nezmestilo sa".
/// Nic sa neorezava na sirku (dlhsie preklady idu na dalsi riadok). Vrati y pod posledne nakreslenym riadkom.
#[allow(clippy::too_many_arguments)]
fn wrapped(
    p: &mut dyn Painter,
    text: &str,
    x: i32,
    mut y: i32,
    w: i32,
    line: i32,
    style: &TextStyle,
    room: &dyn Fn(i32) -> bool,
) -> i32 {
    for l in transom_render::painter::wrap_text(p, text, style, w) {
        if !room(y) {
            break;
        }
        p.draw_text(&l, Rect::new(x, y, w, line), style);
        y += line;
    }
    y
}

/// Suma v USD na dve desatinne miesta; prazdny sucet (`-0.0`) ani zaokruhlene zaporne nuly sa nikdy nezobrazia ako "-0.00".
pub(crate) fn money(usd: f64) -> String {
    let v = if usd.abs() < 0.005 { 0.0 } else { usd };
    format!("{v:.2}")
}

pub fn format_duration(secs: i64) -> String {
    match limits::human_duration(secs) {
        HumanDuration::LessThanMinute => t("dur.lt1"),
        HumanDuration::Minutes(m) => tf("dur.min", &[&m.to_string()]),
        HumanDuration::HoursMinutes(h, m) => tf("dur.hm", &[&h.to_string(), &m.to_string()]),
        HumanDuration::DaysHours(d, h) => tf("dur.dh", &[&d.to_string(), &h.to_string()]),
    }
}

/// Veta o resete: odpocet, "cas resetu nie je k dispozicii" alebo "cas resetu uplynul; caka sa na udaje".
/// Nikdy sa nedopocitava z dlzky okna (pozri `transom_usage::limits`).
pub fn countdown_text(rs: ResetState) -> String {
    match rs {
        ResetState::Unavailable => t("reset.none"),
        ResetState::Passed => t("reset.passed"),
        ResetState::In { secs, .. } => tf("reset.in", &[&format_duration(secs)]),
    }
}

/// Zapnuti poskytovatelia v stabilnom poradi (`Provider::ALL`).
pub fn enabled_providers(u: &transom_config::Usage) -> Vec<Provider> {
    Provider::ALL
        .into_iter()
        .filter(|p| match p {
            Provider::ClaudeCode => u.claude_local || u.claude_statusline,
            Provider::Codex => u.codex_local,
        })
        .collect()
}

/// Kratky nazov okna pre listu ("5 h", "7 d", "30 d"): len podla minut, nikdy podla textu zdroja.
pub fn window_short(minutes: u32) -> String {
    match minutes {
        m if m > 0 && m.is_multiple_of(1440) => tf("win.d", &[&(m / 1440).to_string()]),
        m if m > 0 && m.is_multiple_of(60) => tf("win.h", &[&(m / 60).to_string()]),
        m => tf("win.min", &[&m.to_string()]),
    }
}

/// Jedna polozka tooltipu: okno, percenta, odpocet a stav (slovom).
fn window_tip(r: &LimitReading, now: i64) -> String {
    let name = window_name(r.minutes);
    if r.freshness == Freshness::Expired {
        return tf("ring.name.win_passed", &[&name, &r.used().to_string()]);
    }
    let mut s = tf(
        "ring.name.win",
        &[
            &name,
            &r.used().to_string(),
            &r.shown(RingMode::Remaining).to_string(),
            &countdown_text(r.reset(now)),
        ],
    );
    if r.freshness == Freshness::LastRecorded {
        s.push_str(&format!(
            " ({}, {})",
            t("ring.name.stale_tag"),
            when(r.recorded_at)
        ));
    }
    s
}

/// Preco poskytovatel nema ziadne okno limitu a ako to opravit - **bez tajomstiev a ciest**. Claude Code dava limity planu
/// **iba** cez zdokumentovany `statusLine` (`rate_limits`; len claude.ai Pro/Max/Ultra a az po prvej odpovedi); lokalne
/// prepisy davaju len tokeny. Ucet sa neskrabe a nic sa necita z tokenov/cookies (nedokumentovane rozhrania sa nepouzivaju).
pub fn provider_hint(
    p: Provider,
    u: &transom_config::Usage,
    data: Option<&Collected>,
) -> Option<String> {
    use transom_usage::SourceKind;
    match p {
        Provider::ClaudeCode => {
            if !u.claude_statusline {
                return Some(t("ai.hint.off"));
            }
            let st = data.and_then(|d| {
                d.statuses
                    .iter()
                    .find(|s| s.kind == SourceKind::ClaudeStatusline)
            });
            match st {
                // este sa nezbieralo: nic nehlasime (nacitava sa)
                None => None,
                Some(s) if s.error.is_some() || s.records == 0 => Some(t("ai.hint.setup")),
                // zaznamy su, ale ziadny nenesie limity: ucet ich nehlasi (API / Console, alebo este nebola odpoved)
                Some(s) if !s.limits_seen => Some(t("ai.hint.nolimits")),
                Some(_) => None,
            }
        }
        Provider::Codex => Some(t("ai.hint.codex")),
    }
}

/// Kruh v liste z dat a konfiguracie (cista funkcia: `now` sa dodava, testovatelne). Okno kruhu je **najkratsie okno,
/// ktore poskytovatel naozaj hlasi** (nie natvrdo 5 h); cely zoznam okien je v tooltipe.
pub fn ring_view(data: Option<&Collected>, u: &transom_config::Usage, now: i64) -> RingView {
    let off = || RingView {
        fill: None,
        ccw: false,
        muted: true,
        alert: false,
        label: t("ring.off"),
        name: t("ring.name.off"),
        detail: None,
    };
    if !u.any_enabled() {
        return off();
    }
    let plans = data.map(|d| d.plans.as_slice()).unwrap_or(&[]);
    let mode = RingMode::parse(&u.ring_mode);
    let Some(provider) = limits::pick_provider(
        ProviderChoice::parse(&u.bar_provider),
        &enabled_providers(u),
        plans,
        now,
        LIMIT_FRESH_S,
    ) else {
        return off();
    };
    let pname = provider.label();
    let short = provider_short(provider);
    let windows = limits::provider_windows(plans, provider, now, LIMIT_FRESH_S);
    let Some(r) = limits::ring_window(&windows) else {
        return RingView {
            fill: None,
            ccw: false,
            muted: true,
            alert: false,
            label: t("ring.unavailable"),
            // tooltip povie aj dovod a opravu (napr. nenastaveny statusLine)
            name: match provider_hint(provider, u, data) {
                Some(h) => format!("{} {h}", tf("ring.name.none", &[pname])),
                None => tf("ring.name.none", &[pname]),
            },
            detail: Some(short.to_string()),
        };
    };
    let tip = tf(
        "ring.name.head",
        &[
            pname,
            &windows
                .iter()
                .map(|w| window_tip(w, now))
                .collect::<Vec<_>>()
                .join("; "),
        ],
    );
    let base = format!("{short} · {}", window_short(r.minutes));
    // cas resetu uplynul: samostatny stav (nie "zdroj to nehlasi"), caka sa na cerstve udaje
    if r.freshness == Freshness::Expired {
        return RingView {
            fill: None,
            ccw: false,
            muted: true,
            alert: false,
            label: t("ring.passed"),
            name: tip,
            detail: Some(base),
        };
    }
    let stale = r.freshness == Freshness::LastRecorded;
    let alert = r.shown(RingMode::Remaining) <= ai_cards::LOW_REMAINING_PCT;
    let mut label = match mode {
        RingMode::Used => tf("ring.used", &[&r.shown(mode).to_string()]),
        RingMode::Remaining => tf("ring.left", &[&r.shown(mode).to_string()]),
    };
    // "~" = posledna zaznamenana, mozno neaktualna hodnota; "!" = malo kapacity (nie je to len farba)
    if stale {
        label = format!("~{label}");
    }
    if alert {
        label = format!("! {label}");
    }
    let mut detail = base;
    if stale {
        detail.push_str(&format!(" · {}", t("ring.stale")));
    }
    // odpocet len z `resets_at` zdroja; aktualizuje sa pri minutovom tiku (ziadne dalsie casovace ani dotazy)
    if let ResetState::In { secs, .. } = r.reset(now) {
        detail.push_str(&format!(" · {}", tf("reset.in", &[&format_duration(secs)])));
    }
    RingView {
        fill: Some(r.shown(mode)),
        ccw: mode == RingMode::Remaining,
        muted: stale,
        alert,
        label,
        name: tip,
        detail: Some(detail),
    }
}

/// Zamok bez ohladu na otraveny mutex (panika v inom vlakne nesmie zablokovat prijem vysledkov).
fn lock_shared(m: &Mutex<Option<(u64, Collected)>>) -> MutexGuard<'_, Option<(u64, Collected)>> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Telo zberu na samostatnom vlakne. Vysledok sa zapise do cache a odovzda len ak generacia dat (`epoch`) ostala taka, v akej
/// zber zacal; inak (data sa medzitym zmazali) sa zahodi.
///
/// Panika: release zostavenie ma `panic = "abort"`, takze panika v zbere zhodi cely proces (zotavenie z nej by tu bolo len
/// zavadzajuce a v release by sa nikdy nevykonalo). Parsery lokalnych logov preto nesmu panikovat: poskodene alebo nezmyselne
/// riadky sa preskakuju a hodnoty saturuju (testy v `transom-usage`); rozhodnutie je zapisane v docs/PLAN.md (8.24).
fn run_collection(
    collect: impl FnOnce() -> Collected,
    prev: Option<&Collected>,
    shared: &Mutex<Option<(u64, Collected)>>,
    epoch: &AtomicU64,
    started_in: u64,
    cache_file: &Path,
) {
    let mut c = collect();
    // zdroj, ktory sa nepodarilo precitat, si necha cas posledneho USPESNEHO citania
    if let Some(p) = prev {
        c.carry_last_ok(p);
    }
    if epoch.load(Ordering::SeqCst) != started_in {
        return;
    }
    let _ = cache::save(cache_file, &c);
    *lock_shared(shared) = Some((started_in, c));
}

impl AiState {
    pub fn new() -> Self {
        AiState {
            glide: Default::default(),
            data: None,
            busy: false,
            tab: Period::Today,
            details: false,
            hits: Vec::new(),
            hover: None,
            month: MonthUi::default(),
            scroll: 0,
            max_scroll: 0,
            repaint: false,
            unit: 40,
            view_h: 200,
            last_month: None,
            shared: Arc::new(Mutex::new(None)),
            epoch: Arc::new(AtomicU64::new(0)),
            rerun: false,
            cache_file: cache::cache_path(),
            collected_mtime: None,
            last_started: 0,
        }
    }

    /// Nacita cache (pri starte), ak je nejaky zdroj zapnuty. Cache starej schemy sa nenacita (= prazdne).
    pub fn load_cache(&mut self, u: &transom_config::Usage) {
        self.data = if u.any_enabled() {
            cache::load(&cache::cache_path())
        } else {
            None
        };
        // nacitane data uz zahrnaju stav mostu a pocitaju sa ako posledny zber (brana 5 min plati aj po restarte)
        self.collected_mtime = bridge_mtime();
        self.last_started = self
            .data
            .as_ref()
            .and_then(|d| d.statuses.iter().map(|s| s.refreshed_at).max())
            .unwrap_or(0);
    }

    /// Spusti zber na samostatnom vlakne (nikdy na UI vlakne); vysledok pride ako `Notify(N_USAGE)`.
    /// `Auto` ide cez branu [`auto_allowed`]; `Manual` (tlacidlo Obnovit, zmena zdrojov) je vzdy povolene.
    /// Vrati `true`, ak sa zber naozaj spustil.
    pub fn refresh(
        &mut self,
        u: &transom_config::Usage,
        n: Option<Notifier>,
        kind: RefreshKind,
    ) -> bool {
        // bez notifiera (hostitel este neexistuje) by sa koniec zberu nedal oznamit a `busy` by ostal navzdy
        let Some(n) = n else { return false };
        if self.busy && !self.take_ready() {
            return false;
        }
        if !u.any_enabled() {
            return false;
        }
        let now = now_unix();
        if kind == RefreshKind::Auto
            && !auto_allowed(now, self.last_started, u.refresh_interval_minutes())
        {
            return false;
        }
        let (was_started, was_mtime) = (self.last_started, self.collected_mtime);
        self.busy = true;
        self.last_started = now;
        self.collected_mtime = bridge_mtime();
        let src = sources(u);
        let shared = self.shared.clone();
        let epoch = self.epoch.clone();
        let started_in = epoch.load(Ordering::SeqCst);
        let cache_file = self.cache_file.clone();
        let prev = self.data.clone();
        let spawned = std::thread::Builder::new()
            .name("transom-usage".into())
            .spawn(move || {
                run_collection(
                    || transom_usage::collect(&src, now_unix(), &pclock::local_day_of_unix),
                    prev.as_ref(),
                    &shared,
                    &epoch,
                    started_in,
                    &cache_file,
                );
                // oznami sa VZDY (aj po zahodenom vysledku), inak by `busy` ostalo navzdy
                n.notify(N_USAGE);
            });
        if spawned.is_err() {
            // vlakno sa nepodarilo vytvorit: nezablokovat dalsie zbery
            self.busy = false;
            self.last_started = was_started;
            self.collected_mtime = was_mtime;
            return false;
        }
        true
    }

    /// Lokalne data zmazali nastavenia: zahodi sa vsetko v pamati a rozbehnuty zber (ak nejaky je) sa stane zastaranym.
    pub fn usage_deleted(&mut self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.data = None;
        *lock_shared(&self.shared) = None;
        // cache mohol znova vzniknut zberom, ktory zapisal po zmazani, kym sprava o zmazani este nedosla (znova sa vytvori
        // az novym zberom)
        let _ = std::fs::remove_file(&self.cache_file);
        if self.busy {
            self.rerun = true;
        }
    }

    /// Po skonceni zastaraneho zberu: ma sa spustit novy (zmazanie prislo pocas zberu)? Vymaze priznak.
    pub fn take_rerun(&mut self) -> bool {
        std::mem::take(&mut self.rerun)
    }

    /// Ma sa obnovit kvoli novym zaznamom mostu statusline? Lacne `metadata` (vola sa pri minutovom ticku).
    /// Porovnava sa s casom mostu pri **spusteni posledneho zberu**, takze zmena, ktoru brana 5 min zatial
    /// nepustila, sa nestráca - skusi sa znova pri dalsom tiku.
    pub fn bridge_changed(&self, u: &transom_config::Usage) -> bool {
        if !u.claude_statusline {
            return false;
        }
        let m = bridge_mtime();
        m.is_some() && m != self.collected_mtime
    }

    /// Ak uz vysledok caka, prevezme ho (bez blokovania).
    pub fn take_ready(&mut self) -> bool {
        let ready = lock_shared(&self.shared).is_some();
        ready && self.take_result()
    }

    /// Prevezme vysledok zberu (po `Notify(N_USAGE)`).
    pub fn take_result(&mut self) -> bool {
        self.busy = false;
        let r = lock_shared(&self.shared).take();
        match r {
            Some((e, mut c)) if e == self.epoch.load(Ordering::SeqCst) => {
                if let Some(prev) = &self.data {
                    c.carry_last_ok(prev);
                }
                self.data = Some(c);
                true
            }
            // zber zacal pred zmazanim dat: zahodit aj jeho pripadnu cache
            Some(_) => {
                let _ = std::fs::remove_file(&self.cache_file);
                false
            }
            None => false,
        }
    }

    /// Data starsie nez `max_age` (alebo ziadne) - pri otvoreni panelu obnovit.
    pub fn is_old(&self, max_age_s: i64) -> bool {
        let newest = self
            .data
            .as_ref()
            .and_then(|d| d.statuses.iter().map(|s| s.refreshed_at).max())
            .unwrap_or(0);
        now_unix() - newest > max_age_s
    }

    /// Posun spodnej casti o `delta` px (zaporne = nahor), orezany na povoleny rozsah; `true` = zmenilo sa.
    /// Posun hned (klavesnica).
    pub fn scroll_by(&mut self, delta: i32) -> bool {
        let v = (self.scroll + delta).clamp(0, self.max_scroll);
        let changed = v != self.scroll;
        self.scroll = v;
        changed
    }

    /// Posun kolieskom: plynulo (snimky `glide_step`).
    pub fn wheel_by(&mut self, delta: i32) -> bool {
        self.glide.wheel(self.scroll, delta, self.max_scroll)
    }

    /// Snimka plynuleho posuvania.
    pub fn glide_step(&mut self) -> bool {
        self.glide.step(&mut self.scroll)
    }

    /// Pri otvoreni panela: mesacny pohlad sa vrati na tento mesiac (tokeny, vsetci poskytovatelia) a posun na zaciatok.
    pub fn on_open(&mut self) {
        self.month = MonthUi::default();
        self.scroll = 0;
    }

    /// Zmena obdobia / rezimu zobrazenia vrati posun na zaciatok.
    fn reset_scroll(&mut self) {
        self.scroll = 0;
    }

    /// Kliknutie na prvok panela (okrem Obnovit / Nastavenia / Zostava-Spotrebovane, ktore riesi hostitel). `true` =
    /// treba prekreslit.
    pub fn click(&mut self, h: AiHit, today: Ymd) -> bool {
        let earliest = self
            .data
            .as_ref()
            .and_then(|d| transom_usage::history::earliest(d, &pclock::local_day_of_unix));
        match h {
            AiHit::Tab(p) => {
                self.tab = p;
                self.reset_scroll();
            }
            AiHit::Details => {
                self.details = !self.details;
                self.reset_scroll();
            }
            AiHit::MonthPrev => {
                if !self.month.shift(-1, today, earliest) {
                    return false;
                }
                self.reset_scroll();
            }
            AiHit::MonthNext => {
                if !self.month.shift(1, today, earliest) {
                    return false;
                }
                self.reset_scroll();
            }
            AiHit::MonthThis => {
                if !self.month.this_month(today) {
                    return false;
                }
                self.reset_scroll();
            }
            AiHit::Unit(m) => self.month.metric = m,
            AiHit::Prov(f) => self.month.filter = f,
            AiHit::Day(d) => self.month.toggle_day(d),
            AiHit::Refresh | AiHit::Settings | AiHit::Mode(_) => return false,
        }
        true
    }

    /// Klavesnica v paneli: hore/dole, Page Up/Down, Home/End posuvaju obsah; v mesacnom pohlade sipky vlavo/vpravo
    /// volia den, Page Up/Down s Ctrl... (nepouziva sa) - mesiac meni `[` / `]` nie; **PgUp/PgDn = predosly/dalsi mesiac**
    /// iba ked sa obsah neda posuvat. `true` = treba prekreslit.
    pub fn key(&mut self, vk: u32, today: Ymd) -> bool {
        let (line, page) = (self.unit, (self.view_h - self.unit).max(self.unit));
        let month = self.tab == Period::Month && !self.details;
        match vk {
            0x26 => self.scroll_by(-line),
            0x28 => self.scroll_by(line),
            0x21 if self.max_scroll > 0 => self.scroll_by(-page),
            0x22 if self.max_scroll > 0 => self.scroll_by(page),
            0x21 if month => self.click(AiHit::MonthPrev, today),
            0x22 if month => self.click(AiHit::MonthNext, today),
            0x24 if self.max_scroll > 0 && !month => self.scroll_by(-self.max_scroll),
            0x23 if self.max_scroll > 0 && !month => self.scroll_by(self.max_scroll),
            0x25 | 0x27 if month => {
                self.month.step_day(if vk == 0x25 { -1 } else { 1 }, today);
                true
            }
            0x24 if month => {
                self.month.day = Some(1);
                true
            }
            0x23 if month => {
                let last = {
                    let (y, m) = self.month.month(today);
                    if self.month.is_current(today) {
                        today.d
                    } else {
                        transom_core::clock::days_in_month(y, m)
                    }
                };
                self.month.day = Some(last);
                true
            }
            _ => false,
        }
    }

    /// Nazov prvku panela pre citacky obrazovky (lokalizovany).
    pub fn hit_name(&self, h: AiHit) -> String {
        match h {
            AiHit::Tab(Period::Today) => t("period.today"),
            AiHit::Tab(Period::Last7) => t("period.last7"),
            AiHit::Tab(Period::Month) => t("period.month"),
            AiHit::Refresh => t("ai.refresh"),
            AiHit::Settings => t("ai.settings"),
            AiHit::Details => t("ai.details"),
            AiHit::Mode(RingMode::Remaining) => t("ai.mode.left"),
            AiHit::Mode(RingMode::Used) => t("ai.mode.used"),
            AiHit::MonthPrev => t("ai.m.prev"),
            AiHit::MonthNext => t("ai.m.next"),
            AiHit::MonthThis => t("ai.m.this"),
            AiHit::Unit(Metric::Tokens) => t("ai.m.tokens"),
            AiHit::Unit(Metric::Cost) => t("ai.m.cost"),
            AiHit::Prov(Filter::All) => t("ai.m.all"),
            AiHit::Prov(Filter::Only(p)) => provider_short(p).to_string(),
            AiHit::Day(d) => self
                .last_month
                .as_ref()
                .and_then(|v| v.cells.get(d as usize - 1))
                .map(|c| ai_month::day_name(c, self.month.metric))
                .unwrap_or_default(),
        }
    }

    /// Prvky panela pre pristupnost (nazov, obdlznik, aktivny = zvoleny alebo pod mysou).
    pub fn a11y(&self) -> (Vec<transom_platform::popup::MenuEntry>, Option<usize>) {
        let mut active = None;
        let entries = self
            .hits
            .iter()
            .enumerate()
            .map(|(i, (r, h))| {
                if self.hover == Some(*h) {
                    active = Some(i);
                }
                transom_platform::popup::MenuEntry {
                    name: self.hit_name(*h),
                    separator: false,
                    enabled: true,
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                }
            })
            .collect();
        (entries, active)
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<AiHit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| *h)
    }

    /// Nakresli panel: hlavicka (nazov, Podrobnosti), karty aktualnych limitov, potom **historia** (druhotna) alebo
    /// **Podrobnosti** (zdroje, casy zaznamov, chyby - technicke udaje mimo hlavneho pohladu) a pata so stavom obnovy.
    pub fn paint(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        u: &transom_config::Usage,
        today: Ymd,
        now: i64,
    ) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        let mut hits = Vec::new();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, b, sz.w - 2 * b, sz.h - 2 * b), th.background);
        let st = |size: f32, bold: bool, color: Color, align: HAlign| TextStyle {
            font: Font::Ui,
            size_lp: size,
            bold,
            color,
            align,
        };
        let pad = px(14);
        let w = sz.w - 2 * pad;
        let mut y = pad;
        let line = px(18);
        let foot_h = px(28);
        let foot_y = sz.h - pad - foot_h;
        // jeden riadok stavu obnovy nad patou (vzdy viditelny)
        let status_y = foot_y - line - px(4);
        let mode = RingMode::parse(&u.ring_mode);

        // hlavicka: nazov + prepinac Podrobnosti
        p.draw_text(
            &t("ai.title"),
            Rect::new(pad, y, w / 2, px(26)),
            &st(15.0, true, th.foreground, HAlign::Left),
        );
        if u.any_enabled() {
            let label = t("ai.details");
            let tw = p
                .measure_text(&label, &st(12.0, true, th.foreground, HAlign::Left))
                .w;
            let bw = tw + px(34);
            let r = Rect::new(sz.w - pad - bw, y, bw, px(26));
            // zapnute podrobnosti: vybrana volba (nadych + akcentovy okraj + polotucne), pod mysou plocha ovladaca
            if self.details {
                transom_render::design::fill_with_stroke(
                    p,
                    r,
                    px(transom_render::design::radius::CONTROL),
                    Some(th.selected),
                    Some(th.accent),
                );
            } else if self.hover == Some(AiHit::Details) {
                p.fill_round_rect(
                    r,
                    px(transom_render::design::radius::CONTROL),
                    th.control_hover,
                );
            }
            p.draw_text(
                &label,
                Rect::new(r.x + px(10), r.y, tw + px(4), r.h),
                &st(12.0, self.details, th.foreground, HAlign::Left),
            );
            // sipka dole (zbalene) / hore (rozbalene)
            p.draw_text(
                if self.details { "\u{E70E}" } else { "\u{E70D}" },
                Rect::new(r.x + r.w - px(24), r.y, px(18), r.h),
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 10.0,
                    bold: false,
                    color: th.muted,
                    align: HAlign::Center,
                },
            );
            hits.push((r, AiHit::Details));
        }
        y += px(34);

        self.last_month = None;
        if !u.any_enabled() {
            for k in ["ai.off1", "ai.off2", "ai.off3", "ai.off4"] {
                y = wrapped(
                    p,
                    &t(k),
                    pad,
                    y,
                    w,
                    line,
                    &st(12.0, false, th.muted, HAlign::Left),
                    &|_| true,
                );
            }
        } else if let Some(d) = &self.data {
            let max_age = stale_after_s(u);
            // 0. volba "Zostava / Spotrebovane": plati pre karty limitov aj kruh v liste (ulozi sa do konfiguracie)
            y += ai_month::mode_toggle(p, th, mode, self.hover, pad, y, &mut hits) + px(8);
            // 1. aktualne limity (karta na poskytovatela) su na zaciatku posuvneho obsahu: pri bezne velkosti sa vidia hned
            let hint_for = |pr: Provider| provider_hint(pr, u, Some(d));
            let cards =
                ai_cards::build(&d.plans, &d.context, &enabled_providers(u), now, &hint_for);
            let rows = summarize(&d.days, today, self.tab);
            // 2. posuvne okno (karty limitov, potom historia / mesacny pohlad / podrobnosti) so zretelnym posuvnikom
            let vp_top = y;
            self.unit = px(40);
            let vp = Rect::new(0, vp_top, sz.w, (status_y - px(2) - vp_top).max(px(60)));
            let after_cards;
            let hits_start = hits.len();
            let mut month_view = None;
            p.push_clip(vp);
            let origin = vp_top - self.scroll;
            let bottom = {
                let mut tp = transom_render::audit::Track::new(p);
                let b = {
                    let tpd: &mut dyn Painter = &mut tp;
                    after_cards = ai_cards::paint(tpd, th, &cards, pad, origin, w, now, mode);
                    let origin = after_cards;
                    if self.details {
                        // 2b. Podrobnosti: zdroje a casy zaznamov, chyby, odhady
                        self.paint_details(
                            tpd,
                            th,
                            u,
                            d,
                            &rows,
                            today,
                            now,
                            max_age,
                            origin,
                            &|_| true,
                            &st,
                        )
                    } else if self.tab == Period::Month {
                        // 2c. mesacny pohlad: graf dennych tokenov / odhadu ceny, rozpis dna, poznamky k uplnosti
                        let providers = enabled_providers(u);
                        let ctx = MonthCtx {
                            data: d,
                            providers: &providers,
                            today,
                            local_day: &pclock::local_day_of_unix,
                        };
                        let view = ctx.view(&self.month);
                        let yb = ai_month::paint(
                            tpd,
                            th,
                            &ctx,
                            &self.month,
                            &view,
                            self.hover,
                            pad,
                            origin,
                            w,
                            &mut hits,
                        );
                        month_view = Some(view);
                        yb
                    } else {
                        // 2a. historia (druhotna): zalozky, suma odhadu, tokeny, modely
                        self.paint_history(tpd, th, &rows, origin, &|_| true, &st, &mut hits)
                    }
                };
                b.max(tp.max_bottom)
            };
            self.last_month = month_view;
            p.pop_clip();
            // klik mimo okna spodnej casti (orezaneho) sa nepocita
            let mut k = hits_start;
            while k < hits.len() {
                let r = hits[k].0;
                let (x1, y1) = (r.x.max(vp.x), r.y.max(vp.y));
                let (x2, y2) = ((r.x + r.w).min(vp.x + vp.w), (r.y + r.h).min(vp.y + vp.h));
                if x2 > x1 && y2 > y1 && !(r.y + r.h <= vp.y || r.y >= vp.y + vp.h) {
                    hits[k].0 = Rect::new(x1, y1, x2 - x1, y2 - y1);
                    k += 1;
                } else {
                    hits.remove(k);
                }
            }
            self.view_h = vp.h;
            let content_h = (bottom + self.scroll - vp_top + px(6)).max(0);
            let max_scroll = (content_h - vp.h).max(0);
            if max_scroll != self.max_scroll || self.scroll > max_scroll {
                self.max_scroll = max_scroll;
                self.scroll = self.scroll.min(max_scroll);
                self.repaint = true;
            }
            // posuvnik: viditelna drahy a palec (obsah sa posuva kolieskom, sipkami hore/dole a Page Up/Down)
            if self.max_scroll > 0 {
                let tw = px(5);
                let tx = sz.w - px(7);
                p.fill_round_rect(Rect::new(tx, vp.y, tw, vp.h), tw / 2, th.pressed);
                let th_h = (vp.h * vp.h / (vp.h + self.max_scroll)).max(px(28));
                let ty = vp.y + (vp.h - th_h) * self.scroll / self.max_scroll;
                p.fill_round_rect(Rect::new(tx, ty, tw, th_h), tw / 2, th.muted);
            }
        } else {
            let msg = if self.busy {
                t("ai.loading")
            } else {
                t("ai.empty")
            };
            p.draw_text(
                &msg,
                Rect::new(pad, y, w, line),
                &st(12.0, false, th.muted, HAlign::Left),
            );
        }

        // stav obnovy: posledne uspesne obnovenie, zastaranost a rezim automatickej obnovy
        if u.any_enabled() {
            let last = self.data.as_ref().map_or(0, |d| d.last_ok());
            let stale = self
                .data
                .as_ref()
                .is_none_or(|d| d.statuses.iter().any(|s| s.is_stale(now, stale_after_s(u))));
            let mut l1 = if last > 0 {
                tf("ai.updated", &[&when(last)])
            } else {
                t("ai.refreshed_never")
            };
            if stale {
                l1.push_str(&format!(" · {}", t("ai.stale")));
            }
            p.draw_text(
                &l1,
                Rect::new(pad, status_y, w, line),
                &st(
                    11.0,
                    stale,
                    if stale { th.foreground } else { th.muted },
                    HAlign::Left,
                ),
            );
        }
        // pata
        let labels = [
            (
                AiHit::Refresh,
                if self.busy {
                    t("ai.refreshing")
                } else {
                    t("ai.refresh")
                },
            ),
            (AiHit::Settings, t("ai.settings")),
        ];
        let mut bx = pad;
        for (h, label) in labels {
            let bw = p
                .measure_text(&label, &st(12.0, false, th.foreground, HAlign::Center))
                .w
                + px(28);
            let r = Rect::new(bx, foot_y, bw.max(px(96)), foot_h);
            let b = transom_render::design::Button::new(
                &label,
                transom_render::design::Kind::Secondary,
            )
            .state(transom_render::design::State {
                hover: self.hover == Some(h),
                ..Default::default()
            });
            transom_render::design::button(p, th, r, &b, th.background);
            hits.push((r, h));
            bx += r.w + px(8);
        }
        self.hits = hits;
    }

    /// Karta historie: zalozky obdobia, odhad ceny (API-ekvivalent, nie faktura), tokeny a modely s pruhmi. Vrati y pod nou.
    #[allow(clippy::too_many_arguments)]
    fn paint_history(
        &self,
        p: &mut dyn Painter,
        th: &Theme,
        rows: &[transom_usage::ModelTotals],
        y0: i32,
        room: &dyn Fn(i32) -> bool,
        st: &dyn Fn(f32, bool, Color, HAlign) -> TextStyle,
        hits: &mut Vec<(Rect, AiHit)>,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        let (pad, inner) = (px(14), px(12));
        let (x, w) = (pad + inner, sz.w - 2 * pad - 2 * inner);
        let line = px(18);
        // obsah zoradeny podla odhadu ceny (najdrahsie hore); najviac 3 modely
        let mut list: Vec<&transom_usage::ModelTotals> = rows.iter().collect();
        list.sort_by(|a, b| b.est_usd.total_cmp(&a.est_usd));
        let total_usd: f64 = rows.iter().map(|r| r.est_usd).sum();
        let unpriced: u64 = rows.iter().map(|r| r.unpriced).sum();
        let (inp, out, cr, cw): (u64, u64, u64, u64) = rows.iter().fold((0, 0, 0, 0), |a, r| {
            (
                a.0 + r.input,
                a.1 + r.output,
                a.2 + r.cache_read,
                a.3 + r.cache_write,
            )
        });
        let tokens = inp + out + cr + cw;
        let input_side = inp + cr + cw;
        let cache_pct = if input_side > 0 {
            (cr as f64 * 100.0 / input_side as f64).round() as u64
        } else {
            0
        };
        let shown = list.len().min(3);
        let prov = rows.first().and_then(|r| r.provider);
        let _ = prov;
        let est = t("ai.hist.caption");
        let est_lines = transom_render::painter::wrap_text(
            p,
            &est,
            &st(11.0, false, th.muted, HAlign::Left),
            w,
        )
        .len() as i32;
        // vyska karty vopred (pozadie sa kresli pred obsahom)
        let head_h = px(30);
        let body_h = if rows.is_empty() {
            line + px(6)
        } else {
            px(30)
                + px(18)
                + est_lines * px(17)
                + shown as i32 * px(22)
                + if list.len() > shown { px(18) } else { 0 }
                + px(6)
        };
        let _ = room;
        let card_h = inner + head_h + body_h + inner;
        transom_render::design::card(p, th, Rect::new(pad, y0, sz.w - 2 * pad, card_h));
        let mut y = y0 + inner;
        ai_month::tabs(p, th, self.tab, self.hover, x, y, w, hits);
        y += head_h;
        if rows.is_empty() {
            p.draw_text(
                &t("ai.no_period"),
                Rect::new(x, y, w, line),
                &st(12.0, false, th.muted, HAlign::Left),
            );
            return y0 + card_h + px(8);
        }
        // velka suma (odhad, nie faktura) + pripadne neocenene volania
        let big = format!("≈ ${}", money(total_usd));
        let big_st = st(22.0, true, th.foreground, HAlign::Left);
        let big_w = p.measure_text(&big, &big_st).w;
        p.draw_text(&big, Rect::new(x, y, big_w + px(4), px(30)), &big_st);
        if unpriced > 0 {
            // neocenene volania drobne vedla sumy (suma je len cast: nie je to nula ani faktura)
            p.draw_text(
                &tf("ai.unpriced", &[&unpriced.to_string()]),
                Rect::new(x + big_w + px(10), y + px(6), w - big_w - px(10), px(20)),
                &st(11.0, false, th.muted, HAlign::Left),
            );
        }
        y += px(30);
        p.draw_text(
            &tf(
                "ai.hist.tokens",
                &[&fmt_tokens(tokens), &cache_pct.to_string()],
            ),
            Rect::new(x, y, w, px(18)),
            &st(11.0, false, th.muted, HAlign::Left),
        );
        y += px(18);
        for l in
            transom_render::painter::wrap_text(p, &est, &st(11.0, false, th.muted, HAlign::Left), w)
        {
            p.draw_text(
                &l,
                Rect::new(x, y, w, px(17)),
                &st(11.0, false, th.muted, HAlign::Left),
            );
            y += px(17);
        }
        y += px(6);
        // modely: nazov, pruh podielu na odhade, suma
        let max = list.iter().map(|r| r.est_usd).fold(0.0_f64, f64::max);
        let cost_w = px(96);
        let name_w = px(120);
        for r in list.iter().take(shown) {
            let model = r.model.strip_prefix("claude-").unwrap_or(&r.model);
            p.draw_text(
                model,
                Rect::new(x, y, name_w, px(20)),
                &st(12.0, false, th.foreground, HAlign::Left),
            );
            let cost = if r.est_usd > 0.0 || r.unpriced == 0 {
                format!("≈ ${}", money(r.est_usd))
            } else {
                t("ai.cost_unknown")
            };
            p.draw_text(
                &cost,
                Rect::new(x + w - cost_w, y, cost_w, px(20)),
                &st(12.0, false, th.foreground, HAlign::Right),
            );
            let bar_x = x + name_w + px(6);
            let bar_w = (w - name_w - cost_w - px(12)).max(px(20));
            let bh = px(6);
            let by = y + px(7);
            p.fill_round_rect(Rect::new(bar_x, by, bar_w, bh), bh / 2, th.control_edge);
            if max > 0.0 && r.est_usd > 0.0 {
                let fw = ((bar_w as f64 * r.est_usd / max).round() as i32).max(bh);
                p.fill_round_rect(Rect::new(bar_x, by, fw.min(bar_w), bh), bh / 2, th.accent);
            }
            y += px(22);
        }
        if list.len() > shown {
            p.draw_text(
                &tf("ai.more", &[&(list.len() - shown).to_string()]),
                Rect::new(x, y, w, px(18)),
                &st(11.0, false, th.muted, HAlign::Left),
            );
        }
        y0 + card_h + px(8)
    }

    /// Podrobnosti (diagnostika): zdroje limitov a casy zaznamov, stav zdrojov tokenov (chyby, neuplnost), ceny relacie
    /// zo statusline a poznamky k odhadu. Technicke udaje su tu, nie v hlavnom pohlade.
    #[allow(clippy::too_many_arguments)]
    fn paint_details(
        &self,
        p: &mut dyn Painter,
        th: &Theme,
        u: &transom_config::Usage,
        d: &Collected,
        rows: &[transom_usage::ModelTotals],
        today: Ymd,
        now: i64,
        max_age: i64,
        y0: i32,
        room: &dyn Fn(i32) -> bool,
        st: &dyn Fn(f32, bool, Color, HAlign) -> TextStyle,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        let (pad, line) = (px(14), px(18));
        let w = sz.w - 2 * pad;
        let mut y = y0 + px(2);
        let muted = st(11.0, false, th.muted, HAlign::Left);
        for prov in enabled_providers(u) {
            if !room(y + line) {
                break;
            }
            p.draw_text(
                prov.label(),
                Rect::new(pad, y, w, line),
                &st(12.0, true, th.foreground, HAlign::Left),
            );
            y += line;
            // preco su limity nedostupne a ako to opravit (diagnostika bez tajomstiev)
            if let Some(h) = provider_hint(prov, u, Some(d))
                .filter(|_| limits::provider_windows(&d.plans, prov, now, LIMIT_FRESH_S).is_empty())
            {
                y = wrapped(p, &format!("⚠ {h}"), pad, y, w, line, &muted, room);
            }
            if prov == Provider::ClaudeCode {
                if let Some(sl) = d
                    .statuses
                    .iter()
                    .find(|x| x.kind == transom_usage::SourceKind::ClaudeStatusline)
                {
                    y = wrapped(
                        p,
                        &tf(
                            "ai.diag.records",
                            &[
                                &sl.records.to_string(),
                                &sl.newest.map_or(t("ai.unknown_time"), when),
                            ],
                        ),
                        pad,
                        y,
                        w,
                        line,
                        &muted,
                        room,
                    );
                }
                y = wrapped(p, &t("ai.diag.local"), pad, y, w, line, &muted, room);
                if let Some(c) = d.context.iter().find(|c| c.provider == Some(prov)) {
                    let pct = format!("{:.0}", c.used_percent.clamp(0.0, 100.0));
                    let text = if c.window_tokens > 0 {
                        tf(
                            "ai.ctx",
                            &[&pct, &fmt_tokens(c.window_tokens), &when(c.recorded_at)],
                        )
                    } else {
                        tf("ai.ctx.nosize", &[&pct, &when(c.recorded_at)])
                    };
                    y = wrapped(p, &text, pad, y, w, line, &muted, room);
                }
            }
            // zdroj a cas zaznamu kazdeho okna (rovnake zaznamy sa vypisu raz)
            let wins = limits::provider_windows(&d.plans, prov, now, LIMIT_FRESH_S);
            let mut seen: Vec<(SnapshotSource, i64)> = Vec::new();
            for r in &wins {
                let key = (r.source, r.recorded_at);
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                y = wrapped(p, &ai_cards::meta_text(r), pad, y, w, line, &muted, room);
            }
            if wins.iter().any(|r| r.freshness == Freshness::LastRecorded) {
                y = wrapped(
                    p,
                    &format!("⚠ {}", t("ai.limit.last")),
                    pad,
                    y,
                    w,
                    line,
                    &muted,
                    room,
                );
            }
            // zdroje tokenov: cas posledneho USPESNEHO citania; zastarane, chyba, neuplnost
            for sstat in d.statuses.iter().filter(|x| x.provider == Some(prov)) {
                let mut info = tf("ai.source", &[&source_name(sstat.kind)]);
                if sstat.last_ok_at > 0 {
                    info.push_str(&format!(" · {}", when(sstat.last_ok_at)));
                }
                if sstat.is_stale(now, max_age) {
                    info.push_str(&format!(" · {}", t("ai.stale")));
                }
                y = wrapped(p, &info, pad, y, w, line, &muted, room);
                let extra = if let Some(e) = sstat.error {
                    Some(error_text(e))
                } else if sstat.kind != SourceKind::ClaudeStatusline
                    && sstat.may_be_incomplete(self.tab.start(today), &pclock::local_day_of_unix)
                {
                    sstat.oldest.map(|o| tf("ai.incomplete", &[&when(o)]))
                } else {
                    None
                };
                if let Some(x) = extra {
                    y = wrapped(
                        p,
                        &format!("⚠ {x}"),
                        pad + px(10),
                        y,
                        w - px(10),
                        line,
                        &muted,
                        room,
                    );
                }
            }
            if prov == Provider::ClaudeCode
                && d.statuses
                    .iter()
                    .any(|x| x.kind == SourceKind::ClaudeStatusline)
            {
                let start = self.tab.start(today);
                let usd: f64 = d
                    .statusline_cost_by_day
                    .iter()
                    .filter(|((yy, mm, dd), _)| {
                        let x = Ymd::new(*yy, *mm, *dd);
                        x >= start && x <= today
                    })
                    .map(|(_, c)| c)
                    .sum();
                y = wrapped(
                    p,
                    &tf("ai.statusline_cost", &[&money(usd)]),
                    pad,
                    y,
                    w,
                    line,
                    &muted,
                    room,
                );
            }
            y += px(6);
        }
        // interval automatickej obnovy
        let every = match u.refresh_interval_minutes() {
            Some(m) => tf("ai.auto_every", &[&m.to_string()]),
            None => t("ai.auto_off"),
        };
        y = wrapped(p, &every, pad, y, w, line, &muted, room);
        // poznamky k odhadu ceny (nie faktura)
        if !rows.is_empty() {
            for l in [
                tf(
                    "ai.est1",
                    &[pricing::prices_date(Some(Provider::ClaudeCode))],
                ),
                tf("ai.est1", &[pricing::prices_date(Some(Provider::Codex))]),
                t("ai.est2"),
            ] {
                y = wrapped(p, &l, pad, y, w, line, &muted, room);
            }
        }
        y
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn money_never_shows_negative_zero() {
        let _lang = crate::test_lang::guard();
        assert_eq!(money(-0.0), "0.00");
        assert_eq!(
            money([].iter().sum::<f64>()),
            "0.00",
            "prazdny sucet je -0.0"
        );
        assert_eq!(money(-0.003), "0.00");
        assert_eq!(money(64.299), "64.30");
        assert_eq!(money(0.07), "0.07");
    }

    use super::*;
    use transom_usage::{LimitWindow, PlanSnapshot};

    fn usage(claude: bool, codex: bool) -> transom_config::Usage {
        transom_config::Usage {
            claude_statusline: claude,
            codex_local: codex,
            ..Default::default()
        }
    }

    fn data(plans: Vec<PlanSnapshot>) -> Collected {
        Collected {
            plans,
            ..Default::default()
        }
    }

    fn snap(p: Provider, at: i64, minutes: u32, used: f64, reset: Option<i64>) -> PlanSnapshot {
        PlanSnapshot {
            provider: Some(p),
            plan: String::new(),
            windows: vec![LimitWindow {
                minutes,
                used_percent: used,
                resets_at: reset,
            }],
            recorded_at: at,
            source: SnapshotSource::ClaudeStatusline,
        }
    }

    #[test]
    fn automatic_refresh_gate_never_allows_polling_faster_than_the_interval() {
        let _lang = crate::test_lang::guard();
        let t0 = 1_000_000;
        assert!(auto_allowed(t0, 0, Some(5)), "este nikdy nebezal");
        assert!(!auto_allowed(t0 + 1, t0, Some(5)));
        assert!(!auto_allowed(t0 + 299, t0, Some(5)));
        assert!(auto_allowed(t0 + 300, t0, Some(5)));
        assert!(
            !auto_allowed(t0 - 50, t0, Some(5)),
            "hodiny sli dozadu = nie"
        );
        // interval nizsi nez minimum (poskodeny/rucne upraveny subor) sa berie ako 5 min
        assert!(!auto_allowed(t0 + 120, t0, Some(1)));
        assert!(auto_allowed(t0 + 300, t0, Some(1)));
        // 60 min: ani minutovy tik z mostu, ani otvorenie panelu nespusti zber skor
        assert!(!auto_allowed(t0 + 59 * 60, t0, Some(60)));
        assert!(auto_allowed(t0 + 60 * 60, t0, Some(60)));
        // len na poziadanie: iba prve citanie, ked nic nie je
        assert!(auto_allowed(t0, 0, None));
        assert!(!auto_allowed(t0 + 10 * 86_400, t0, None));
        // minutovy tik: pocet automatickych zberov za 6 hodin podla intervalu
        for (interval, runs) in [
            (Some(5), 72),
            (Some(15), 24),
            (Some(60), 6),
            (Some(240), 2),
            (None, 1),
        ] {
            let (mut last, mut n) = (0, 0);
            for minute in 0..360 {
                let now = t0 + minute * 60;
                if auto_allowed(now, last, interval) {
                    last = now;
                    n += 1;
                }
            }
            assert_eq!(n, runs, "{interval:?}");
        }
    }

    #[test]
    fn stale_threshold_follows_the_interval() {
        let _lang = crate::test_lang::guard();
        let mut u = transom_config::Usage::default();
        assert_eq!(stale_after_s(&u), 600, "predvolene 2 x 5 min");
        u.refresh_minutes = 30;
        assert_eq!(stale_after_s(&u), 3600);
        u.refresh_minutes = 0;
        assert_eq!(stale_after_s(&u), 3600, "len na poziadanie");
        u.refresh_minutes = 2;
        assert_eq!(stale_after_s(&u), 600, "interval sa zvysi na minimum 5 min");
    }

    fn race_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("transom-ai-race-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn state_with_cache(cache: &Path) -> AiState {
        let mut a = AiState::new();
        a.cache_file = cache.to_path_buf(); // nikdy skutocna cache pouzivatela
        a
    }

    #[test]
    fn data_deleted_during_a_running_collection_is_never_resurrected() {
        let d = race_dir("delete-during");
        let cache = d.join("usage-cache.json");
        let mut a = state_with_cache(&cache);
        a.busy = true; // zber bezi
        let started_in = a.epoch.load(Ordering::SeqCst);
        // nastavenia zmazali data (sprava dosla na lištu), kym zber este bezi
        a.usage_deleted();
        assert!(a.rerun, "po skonceni zastaraneho zberu treba novy");
        // zber dobehne: zastaraly vysledok sa nezapise ani neodovzda
        run_collection(
            Collected::default,
            None,
            &a.shared,
            &a.epoch,
            started_in,
            &cache,
        );
        assert!(!cache.exists(), "cache sa nevzkriesila");
        assert!(lock_shared(&a.shared).is_none());
        assert!(!a.take_result(), "ziadny prijaty vysledok");
        assert!(!a.busy && a.data.is_none());
        assert!(
            a.take_rerun() && !a.take_rerun(),
            "novy zber sa pusti prave raz"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_result_that_slipped_through_before_the_delete_is_dropped_with_its_cache() {
        let d = race_dir("late-result");
        let cache = d.join("usage-cache.json");
        let mut a = state_with_cache(&cache);
        a.busy = true;
        let started_in = a.epoch.load(Ordering::SeqCst);
        // zber stihol prejst kontrolou epochy a zapisat (cache aj vysledok) pred prichodom spravy o zmazani
        run_collection(
            Collected::default,
            None,
            &a.shared,
            &a.epoch,
            started_in,
            &cache,
        );
        assert!(cache.exists() && lock_shared(&a.shared).is_some());
        a.usage_deleted();
        assert!(!cache.exists(), "cache zmazana spolu s datami");
        assert!(lock_shared(&a.shared).is_none() && a.data.is_none());
        // pokial by vysledok predsa ostal v schranke (iny poradie), prijem ho tiez zahodi
        *lock_shared(&a.shared) = Some((started_in, Collected::default()));
        assert!(!a.take_result() && a.data.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_normal_collection_is_cached_and_accepted_and_a_poisoned_lock_never_blocks_the_next_one() {
        let d = race_dir("normal-and-poison");
        let cache = d.join("usage-cache.json");
        let mut a = state_with_cache(&cache);
        a.busy = true;
        let e = a.epoch.load(Ordering::SeqCst);
        run_collection(Collected::default, None, &a.shared, &a.epoch, e, &cache);
        assert!(cache.exists());
        assert!(a.take_ready() && a.data.is_some() && !a.busy);
        // (pad zberu sa tu neskusa: release ma `panic = "abort"`, takze by zhodil proces; pozri docs/PLAN.md 8.24)
        // otraveny mutex (panika s drzanym zamkom; mozne len v zostaveniach s unwind, napr. testy) neblokuje prijem
        let shared = a.shared.clone();
        let _ = std::thread::spawn(move || {
            let _g = shared.lock().unwrap();
            panic!("otravenie zamku");
        })
        .join();
        assert!(!a.take_ready());
        run_collection(Collected::default, None, &a.shared, &a.epoch, e, &cache);
        assert!(a.take_ready(), "po otraveni zamku sa vysledok stale prijme");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn disabled_sources_read_nothing() {
        let _lang = crate::test_lang::guard();
        let u = transom_config::Usage::default();
        let s = sources(&u);
        assert!(s.claude_dir.is_none() && s.codex_dir.is_none() && s.statusline_file.is_none());
        let mut a = AiState::new();
        a.load_cache(&u);
        assert!(a.data.is_none());
        let r = ring_view(None, &u, 0);
        assert_eq!((r.fill, r.label.as_str()), (None, "AI: off"));
        a.refresh(&u, None, RefreshKind::Manual);
        assert!(!a.busy);
    }

    #[test]
    fn no_refresh_without_notifier_or_sources() {
        let _lang = crate::test_lang::guard();
        let mut a = AiState::new();
        a.refresh(&usage(false, true), None, RefreshKind::Auto);
        assert!(!a.busy);
        assert!(!a.take_ready(), "ziadny cakajuci vysledok");
    }

    #[test]
    fn ring_used_vs_remaining_and_text_cue() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            62.4,
            Some(99_999),
        )]);
        let mut u = usage(true, false);
        let r = ring_view(Some(&d), &u, 1100);
        assert_eq!((r.fill, r.ccw, r.muted), (Some(62), false, false));
        assert_eq!(r.label, "62 % used");
        assert!(r.name.contains("Claude Code") && r.name.contains("62 % used"));
        u.ring_mode = "remaining".into();
        let r = ring_view(Some(&d), &u, 1100);
        assert_eq!(
            (r.fill, r.ccw),
            (Some(38), true),
            "zostavajuce kresli proti smeru"
        );
        assert_eq!(r.label, "38 % left");
    }

    #[test]
    fn ring_last_recorded_expired_and_unavailable() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let u = usage(true, true);
        // stara hodnota pred resetom: "~", stlmene a slovo "stale" v texte aj tooltipe
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            40.0,
            Some(1_000_000),
        )]);
        let r = ring_view(Some(&d), &u, 1000 + LIMIT_FRESH_S + 1);
        assert!(r.muted && r.label.starts_with('~') && r.fill == Some(40));
        assert!(!r.alert);
        let detail = r.detail.clone().unwrap();
        assert!(
            detail.starts_with("Claude · 5 h · stale · resets in"),
            "{detail}"
        );
        assert!(
            r.name.contains("stale – last recorded value") && r.name.contains("resets in"),
            "{}",
            r.name
        );
        // po resete okna: ziadna vypln, ziadny odhad 0 %; okno ostava pomenovane
        let r = ring_view(Some(&d), &u, 1_000_001);
        assert_eq!(r.fill, None);
        assert_eq!(
            r.label, "reset passed",
            "uplynul cas resetu != zdroj ho nehlasi"
        );
        assert_eq!(r.detail.as_deref(), Some("Claude · 5 h"));
        assert!(
            r.name
                .contains("reset time passed; waiting for updated usage"),
            "{}",
            r.name
        );
        assert!(
            !r.name.contains("resets in"),
            "po resete sa nepredpoklada nic"
        );
        // Codex s 30-dnovym oknom: kruh ukazuje **30 d** (nie "5 h nedostupne"), ziadny prepocet na ine okno
        let mut c = snap(Provider::Codex, 1000, 43_200, 30.0, None);
        c.source = SnapshotSource::CodexSession;
        let u2 = transom_config::Usage {
            codex_local: true,
            bar_provider: "codex".into(),
            ..Default::default()
        };
        let r = ring_view(Some(&data(vec![c])), &u2, 1100);
        assert_eq!(r.fill, Some(30));
        assert_eq!(r.detail.as_deref(), Some("Codex · 30 d"));
        assert!(r.name.contains("30-day window"), "{}", r.name);
        assert!(!r.name.contains("5-hour"), "{}", r.name);
        // poskytovatel bez akychkolvek okien: "unavailable", bez percenta a bez vymysleneho okna
        let r = ring_view(Some(&data(vec![])), &u2, 1100);
        assert_eq!((r.fill, r.label.as_str()), (None, "unavailable"));
        assert_eq!(r.detail.as_deref(), Some("Codex"));
        assert!(r.name.contains("no plan limit"), "{}", r.name);
    }

    #[test]
    fn auto_shows_codex_30_day_stale_when_claude_has_no_plan_data() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        // zivy pripad: Claude zapnuty bez limitov, Codex ma len starsi zaznam 30-dnoveho okna
        let mut c = snap(
            Provider::Codex,
            1000,
            43_200,
            31.0,
            Some(1000 + 25 * 86_400),
        );
        c.source = SnapshotSource::CodexSession;
        let u = usage(true, true);
        let now = 1000 + 3 * 3600;
        let r = ring_view(Some(&data(vec![c])), &u, now);
        assert_eq!(r.fill, Some(31));
        assert!(r.muted && r.label == "~31 % used", "{}", r.label);
        assert!(
            r.detail
                .as_deref()
                .unwrap()
                .starts_with("Codex · 30 d · stale · resets in 24 d"),
            "{:?}",
            r.detail
        );
    }

    #[test]
    fn low_capacity_is_marked_with_a_symbol_not_only_a_colour() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let u = usage(true, false);
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            95.0,
            Some(1000 + 3600),
        )]);
        let r = ring_view(Some(&d), &u, 1100);
        assert!(r.alert);
        assert_eq!(r.label, "! 95 % used");
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            90.0,
            Some(1000 + 3600),
        )]);
        assert!(
            ring_view(Some(&d), &u, 1100).alert,
            "prave 10 % zostava = malo"
        );
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            89.0,
            Some(1000 + 3600),
        )]);
        assert!(!ring_view(Some(&d), &u, 1100).alert);
        // zostavajuci rezim: "!" aj tam
        let u2 = transom_config::Usage {
            ring_mode: "remaining".into(),
            ..usage(true, false)
        };
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            95.0,
            Some(1000 + 3600),
        )]);
        assert_eq!(ring_view(Some(&d), &u2, 1100).label, "! 5 % left");
    }

    #[test]
    fn ring_uses_the_shortest_reported_window_and_lists_all_in_the_tooltip() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let u = usage(true, false);
        // 5 h + 7 d v jednom zazname
        let mut p = snap(Provider::ClaudeCode, 1000, 300, 62.0, Some(1100 + 8_040));
        p.windows.push(transom_usage::LimitWindow {
            minutes: 10_080,
            used_percent: 31.0,
            resets_at: Some(1100 + 300_000),
        });
        let r = ring_view(Some(&data(vec![p.clone()])), &u, 1100);
        assert_eq!(r.fill, Some(62));
        assert!(r
            .detail
            .as_deref()
            .unwrap()
            .starts_with("Claude · 5 h · resets in 2 h 14 min"));
        assert!(
            r.name.contains("5-hour window") && r.name.contains("7-day window"),
            "{}",
            r.name
        );
        // plan len s tyzdnom (bez 5 h): kruh = 7 d, nic sa nevymysla
        p.windows.remove(0);
        let r = ring_view(Some(&data(vec![p])), &u, 1100);
        assert_eq!(r.fill, Some(31));
        assert!(r.detail.as_deref().unwrap().starts_with("Claude · 7 d"));
        assert!(!r.name.contains("5-hour"), "{}", r.name);
    }

    #[test]
    fn window_short_names_follow_minutes() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert_eq!(window_short(300), "5 h");
        assert_eq!(window_short(10_080), "7 d");
        assert_eq!(window_short(43_200), "30 d");
        assert_eq!(window_short(45), "45 min");
        assert_eq!(window_short(90), "90 min");
    }

    #[test]
    fn countdown_texts_and_tooltip_follow_the_reset_state() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert_eq!(format_duration(8_040), "2 h 14 min");
        assert_eq!(format_duration(30), "< 1 min");
        assert_eq!(format_duration(45 * 60), "45 min");
        assert_eq!(format_duration(3 * 86_400 + 4 * 3600), "3 d 4 h");
        assert_eq!(
            countdown_text(ResetState::Unavailable),
            "reset time unavailable"
        );
        assert_eq!(
            countdown_text(ResetState::Passed),
            "reset time passed; waiting for updated usage"
        );
        assert_eq!(
            countdown_text(ResetState::In {
                secs: 8_040,
                last_recorded: false
            }),
            "resets in 2 h 14 min"
        );
        let u = usage(true, false);
        // cerstvy zaznam so znamym resetom: odpocet v tooltipe
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            62.0,
            Some(1100 + 8_040),
        )]);
        let r = ring_view(Some(&d), &u, 1100);
        assert!(r.name.ends_with("resets in 2 h 14 min"), "{}", r.name);
        // zdroj cas resetu nehlasi: tooltip to povie, nic sa nedopocita
        let d = data(vec![snap(Provider::ClaudeCode, 1000, 300, 62.0, None)]);
        let r = ring_view(Some(&d), &u, 1100);
        assert!(r.name.ends_with("reset time unavailable"), "{}", r.name);
        // v liste sa nevymysla ani "resets in"
        assert_eq!(r.detail.as_deref(), Some("Claude · 5 h"));
        // odpocet sa mení iba s casom (minutovy tik), nie s ziadnym dalsim zberom
        let d = data(vec![snap(
            Provider::ClaudeCode,
            1000,
            300,
            62.0,
            Some(10_000),
        )]);
        let a = ring_view(Some(&d), &u, 1100).name;
        let b = ring_view(Some(&d), &u, 1100 + 60).name;
        assert_ne!(a, b, "o minutu menej");
        assert_eq!(
            a,
            ring_view(Some(&d), &u, 1100).name,
            "rovnaky cas = rovnaky text"
        );
    }

    #[test]
    fn explicit_provider_is_kept_without_data() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let d = data(vec![snap(Provider::ClaudeCode, 1000, 300, 10.0, None)]);
        let u = transom_config::Usage {
            claude_statusline: true,
            codex_local: true,
            bar_provider: "codex".into(),
            ..Default::default()
        };
        let r = ring_view(Some(&d), &u, 1100);
        assert_eq!(r.detail.as_deref(), Some("Codex"));
        assert_eq!(r.fill, None, "Claude data sa nepouziju za Codex");
    }

    #[test]
    fn window_names_by_minutes() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        assert_eq!(window_name(300), "5-hour window");
        assert_eq!(window_name(10_080), "7-day window");
        assert_eq!(window_name(43_200), "30-day window");
    }

    // ---- offscreen kontroly: skutocny renderer (GDI) bez okna; nic nemeni v systeme (DPI, jazyk, tema) ----

    use transom_render::audit::Audit;
    use transom_render::offscreen::Surface;
    use transom_render::{GdiPainter, Size};
    use transom_usage::{ContextSnapshot, DayTotals, SourceStatus};

    const NOW: i64 = 1_795_000_000;

    fn win(minutes: u32, used: f64, reset: Option<i64>) -> transom_usage::LimitWindow {
        transom_usage::LimitWindow {
            minutes,
            used_percent: used,
            resets_at: reset,
        }
    }

    fn plan(p: Provider, at: i64, plan: &str, ws: Vec<transom_usage::LimitWindow>) -> PlanSnapshot {
        PlanSnapshot {
            provider: Some(p),
            plan: plan.into(),
            windows: ws,
            recorded_at: at,
            source: if p == Provider::Codex {
                SnapshotSource::CodexSession
            } else {
                SnapshotSource::ClaudeStatusline
            },
        }
    }

    fn status(
        kind: transom_usage::SourceKind,
        p: Provider,
        err: Option<SourceError>,
    ) -> SourceStatus {
        SourceStatus {
            kind,
            provider: Some(p),
            refreshed_at: NOW - 100,
            last_ok_at: if err.is_none() { NOW - 100 } else { 0 },
            files: 3,
            records: 40,
            oldest: Some(NOW - 5 * 86_400),
            newest: Some(NOW - 100),
            error: err,
            limits_seen: false,
        }
    }

    fn day(p: Provider, model: &str) -> DayTotals {
        DayTotals {
            provider: Some(p),
            model: model.into(),
            day: (2026, 11, 20),
            input: 894,
            output: 751_500,
            cache_read: 194_610_000,
            cache_write: 1_300_000,
            requests: 120,
            est_usd: 64.3,
            unpriced: 2,
            ..Default::default()
        }
    }

    /// Pevne pripady (najhorsie pripady pre rozlozenie panela): (nazov, data, zdroje).
    fn fixtures() -> Vec<(&'static str, Collected)> {
        use transom_usage::SourceKind::{ClaudeLocal, ClaudeStatusline, CodexLocal};
        let sts = |errs: bool| {
            vec![
                status(
                    ClaudeLocal,
                    Provider::ClaudeCode,
                    errs.then_some(SourceError::ReadErrors(2)),
                ),
                status(
                    ClaudeStatusline,
                    Provider::ClaudeCode,
                    errs.then_some(SourceError::NoBridgeData),
                ),
                status(
                    CodexLocal,
                    Provider::Codex,
                    errs.then_some(SourceError::Missing),
                ),
            ]
        };
        let days = vec![
            day(Provider::ClaudeCode, "claude-opus-5-5"),
            day(Provider::ClaudeCode, "claude-sonnet-5-5"),
            day(Provider::ClaudeCode, "claude-haiku-5-5"),
            day(Provider::Codex, "gpt-6-luna"),
        ];
        let ctx = vec![ContextSnapshot {
            provider: Some(Provider::ClaudeCode),
            used_percent: 42.0,
            window_tokens: 1_000_000,
            recorded_at: NOW - 600,
        }];
        let current = |at: i64| {
            vec![
                plan(
                    Provider::ClaudeCode,
                    at,
                    "Pro/Max/Ultra",
                    vec![
                        win(300, 62.0, Some(NOW + 8_040)),
                        win(10_080, 31.0, Some(NOW + 300_000)),
                    ],
                ),
                plan(
                    Provider::Codex,
                    at,
                    "go",
                    vec![win(43_200, 31.0, Some(NOW + 2_200_000))],
                ),
            ]
        };
        // NAJHORSI pripad: dvaja poskytovatelia, kazdy 3 okna, vsetko zastarane, chyby vo vsetkych zdrojoch
        let worst = vec![
            plan(
                Provider::ClaudeCode,
                NOW - 7_200,
                "Pro/Max/Ultra",
                vec![
                    win(300, 95.0, Some(NOW + 3_000)),
                    win(10_080, 31.0, Some(NOW + 300_000)),
                    win(43_200, 12.0, Some(NOW + 2_000_000)),
                ],
            ),
            plan(
                Provider::Codex,
                NOW - 7_200,
                "OpenAI Codex (go)",
                vec![
                    win(300, 40.0, Some(NOW + 3_000)),
                    win(10_080, 10.0, None),
                    win(43_200, 31.0, Some(NOW + 2_200_000)),
                ],
            ),
        ];
        let mk = |plans: Vec<PlanSnapshot>, errs: bool| Collected {
            plans,
            days: days.clone(),
            statuses: sts(errs),
            context: ctx.clone(),
            ..Default::default()
        };
        // bohata historia: Claude od 3. 11. (denne tokeny, 2 modely), Codex od 12. 11.; dni bez zaznamu, neocenene volania
        let rich = {
            let mut days: Vec<DayTotals> = Vec::new();
            for d in 3..=20u32 {
                if d % 7 == 0 {
                    continue; // dni bez zaznamu (skutocna nula v pokrytom rozsahu)
                }
                let k = u64::from(d * 37 % 11 + 1);
                for (model, share) in [("claude-opus-5-5", 3u64), ("claude-sonnet-5-5", 1)] {
                    days.push(DayTotals {
                        provider: Some(Provider::ClaudeCode),
                        model: model.into(),
                        day: (2026, 11, d),
                        input: 900 * k,
                        output: 40_000 * k * share,
                        cache_read: 6_000_000 * k * share,
                        cache_write: 300_000 * k,
                        requests: 10 * k,
                        est_usd: 1.5 * k as f64 * share as f64,
                        unpriced: if d == 10 { 10 * k } else { 0 },
                        ..Default::default()
                    });
                }
                if d >= 12 {
                    days.push(DayTotals {
                        provider: Some(Provider::Codex),
                        model: "gpt-6-luna".into(),
                        day: (2026, 11, d),
                        input: 500 * k,
                        output: 20_000 * k,
                        cache_read: 2_000_000 * k,
                        requests: 4 * k,
                        est_usd: 0.4 * k as f64,
                        ..Default::default()
                    });
                }
            }
            let mut st = sts(false);
            st[0].oldest = Some(NOW - 17 * 86_400);
            st[2].oldest = Some(NOW - 8 * 86_400);
            Collected {
                plans: current(NOW - 60),
                days,
                statuses: st,
                context: ctx.clone(),
                ..Default::default()
            }
        };
        // archiv: dni 1.-9. 11. uz zdroj necita (z archivu, s datumom cennika; 3. 11. bol hranicny = neuplny), od 10. 11. su logy
        let archived = {
            let mut c = rich.clone();
            for d in c.days.iter_mut().filter(|d| d.day.2 < 10) {
                d.archived = true;
                d.price_date = "2026-11-15".into();
                d.partial = d.day.2 == 3;
            }
            c.statuses[0].oldest = Some(NOW - 8 * 86_400);
            c.cover = vec![
                transom_usage::archive::Cover {
                    provider: Provider::ClaudeCode,
                    from: (2026, 11, 1),
                    to: (2026, 11, 20),
                },
                transom_usage::archive::Cover {
                    provider: Provider::Codex,
                    from: (2026, 11, 12),
                    to: (2026, 11, 20),
                },
            ];
            c.archive = transom_usage::ArchiveStatus::Writing;
            c
        };
        vec![
            ("history-archived", archived),
            ("history-rich", rich),
            ("current", mk(current(NOW - 60), false)),
            ("worst-stale-errors", mk(worst.clone(), true)),
            // reset presiel (5 h, 7 d) a bez casu resetu
            (
                "expired-and-no-reset",
                mk(
                    vec![
                        plan(
                            Provider::ClaudeCode,
                            NOW - 60,
                            "",
                            vec![win(300, 70.0, Some(NOW - 10)), win(10_080, 5.0, None)],
                        ),
                        plan(
                            Provider::Codex,
                            NOW - 60,
                            "go",
                            vec![win(43_200, 31.0, None)],
                        ),
                    ],
                    false,
                ),
            ),
            // Claude: statusLine nie je pripojeny (most nema ziadne zaznamy) a Codex ma okno -> karta s navodom
            (
                "claude-setup",
                Collected {
                    plans: vec![plan(
                        Provider::Codex,
                        NOW - 60,
                        "go",
                        vec![win(43_200, 31.0, Some(NOW + 2_100_000))],
                    )],
                    days: days.clone(),
                    statuses: vec![
                        status(
                            ClaudeStatusline,
                            Provider::ClaudeCode,
                            Some(SourceError::NoBridgeData),
                        ),
                        status(CodexLocal, Provider::Codex, None),
                    ],
                    ..Default::default()
                },
            ),
            // zivy pripad: Claude bez limitov, Codex len 30-dnove okno so starym zaznamom
            (
                "codex-30d-only",
                mk(
                    vec![plan(
                        Provider::Codex,
                        NOW - 3 * 3600,
                        "go",
                        vec![win(43_200, 31.0, Some(NOW + 2_100_000))],
                    )],
                    false,
                ),
            ),
        ]
    }

    fn usage_all() -> transom_config::Usage {
        transom_config::Usage {
            claude_local: true,
            claude_statusline: true,
            codex_local: true,
            ..Default::default()
        }
    }

    /// Nastavi mesacny pohlad pre variant `v` (0 = tokeny / vsetci, 1 = odhad ceny so zvolenym dnom, 2 = Codex v predoslom mesiaci).
    fn month_variant(st: &mut AiState, v: u8) {
        st.month = MonthUi::default();
        match v {
            1 => {
                st.month.metric = Metric::Cost;
                st.month.day = Some(9);
            }
            2 => {
                st.month.filter = Filter::Only(Provider::Codex);
                st.month.ym = Some((2026, 10));
            }
            _ => {}
        }
    }

    fn panel_audit(langs: &[transom_i18n::Lang], dpis: &[u32], dark: &[bool]) -> Vec<String> {
        let mut out = Vec::new();
        let today = Ymd::new(2026, 11, 20);
        for &lang in langs {
            transom_i18n::set_lang(lang);
            for &dpi in dpis {
                let s = dpi as f32 / 96.0;
                // vysoka plocha: spodna (posuvna) cast sa vo vyske okna vacsinou zmesti cela, takze sa auditu cely obsah
                let (w, h) = (
                    (AI_W_LP as f32 * s).round() as i32,
                    (AI_H_LP as f32 * 2.0 * s).round() as i32,
                );
                for &d in dark {
                    let th = if d { Theme::dark() } else { Theme::light() };
                    for (name, data) in fixtures() {
                        for (tab, details, mv, ring) in [
                            (Period::Today, false, 0, "used"),
                            (Period::Last7, false, 0, "remaining"),
                            (Period::Month, false, 0, "used"),
                            (Period::Month, false, 1, "remaining"),
                            (Period::Month, false, 2, "used"),
                            (Period::Today, true, 0, "remaining"),
                        ] {
                            let mut st = AiState::new();
                            st.data = Some(data.clone());
                            st.tab = tab;
                            st.details = details;
                            month_variant(&mut st, mv);
                            let mut u = usage_all();
                            u.ring_mode = ring.into();
                            let surf = Surface::new(w, h);
                            let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                            let mut a = Audit::new(&mut gp, th.background);
                            // cesty a ukazkove texty sa neskracuju zamerne; zalamuju sa vsetky texty panela
                            st.paint(&mut a, &th, &u, today, NOW);
                            a.finish();
                            for i in &a.issues {
                                out.push(format!(
                                    "{} {dpi} {} {name} {tab:?} m{mv} {ring}: {i:?}",
                                    lang.code(),
                                    if d { "dark" } else { "light" }
                                ));
                            }
                        }
                    }
                }
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
        out
    }

    #[test]
    fn popover_audit_is_clean_for_worst_cases_in_key_languages() {
        let _lang = crate::test_lang::guard();
        use transom_i18n::Lang;
        let issues = panel_audit(
            &[Lang::En, Lang::De, Lang::Fr, Lang::Ja],
            &[96, 192],
            &[true],
        );
        assert!(issues.is_empty(), "{}", issues.join("\n"));
    }

    /// Cela matica: 11 jazykov x 96/144/192 DPI x obe temy (pomale; spusti sa `cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn popover_audit_full_matrix() {
        let _lang = crate::test_lang::guard();
        let issues = panel_audit(&transom_i18n::Lang::ALL, &[96, 144, 192], &[true, false]);
        assert!(issues.is_empty(), "{}", issues.join("\n"));
    }

    #[test]
    fn limits_section_always_fits_and_the_scrolling_region_keeps_room() {
        let _lang = crate::test_lang::guard();
        use transom_i18n::Lang;
        for lang in [Lang::En, Lang::De, Lang::Fr, Lang::Ja, Lang::Pl] {
            transom_i18n::set_lang(lang);
            for dpi in [96u32, 144, 192] {
                let s = dpi as f32 / 96.0;
                let w = (AI_W_LP as f32 * s).round() as i32;
                let h = (AI_H_LP as f32 * s).round() as i32;
                let th = Theme::dark();
                let (_, worst) = fixtures()
                    .into_iter()
                    .find(|(n, _)| *n == "worst-stale-errors")
                    .unwrap();
                let cards =
                    ai_cards::build(&worst.plans, &worst.context, &Provider::ALL, NOW, &|_| None);
                let surf = Surface::new(w, h);
                let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                let cw = w - 2 * (14.0 * s).round() as i32;
                let lim = ai_cards::height(&mut gp, &th, &cards, cw, NOW, RingMode::Used);
                // rozpocet: panel minus hlavicka (pad + 34), riadok Zostava/Spotrebovane (34), pata (28 + pad 14), riadok stavu
                // obnovy (22): karty limitov sa zmestia do posuvneho okna aj v najhorsom pripade bez posuvania
                let px = |v: f32| (v * s).round() as i32;
                let budget = h - px(14.0 + 34.0 + 34.0 + 14.0 + 28.0 + 22.0 + 6.0);
                assert!(
                    lim <= budget,
                    "{} {dpi}: limity {lim} px > rozpocet {budget} px",
                    lang.code()
                );
            }
        }
        transom_i18n::set_lang(Lang::En);
    }

    /// Najdlhsi text lišty (zastarane, 99 d odpocet, 30-dnove okno) sa vo vsetkych jazykoch zmesti do 400 lp
    /// (inak by sa cast skryla); meria skutocne GDI pri 96 aj 192 DPI.
    #[test]
    fn bar_detail_fits_the_flex_limit_in_every_language() {
        let _lang = crate::test_lang::guard();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            // zastarany Codex, 30-dnove okno, odpocet v dnoch a hodinach (najdlhsia forma jednotiek)
            let mut c = snap(
                Provider::Codex,
                NOW - 7_200,
                43_200,
                31.0,
                Some(NOW + 99 * 86_400 + 23 * 3_600),
            );
            c.source = SnapshotSource::CodexSession;
            let d = data(vec![c]);
            let r = ring_view(Some(&d), &usage(false, true), NOW);
            let detail = r.detail.clone().unwrap();
            for dpi in [96u32, 192] {
                let s = dpi as f32 / 96.0;
                let surf = Surface::new(10, 10);
                let mut gp = GdiPainter::new(surf.dc, Size { w: 10, h: 10 }, dpi);
                let st = transom_render::TextStyle {
                    font: transom_render::Font::Ui,
                    size_lp: transom_render::scene::TEXT_SIZE_LP,
                    bold: false,
                    color: transom_render::Color::rgb(0, 0, 0),
                    align: transom_render::HAlign::Left,
                };
                let w = transom_render::Painter::measure_text(&mut gp, &detail, &st).w;
                assert!(
                    (w as f32 / s) <= 380.0,
                    "{} {dpi}: '{detail}' = {} lp (limit 400)",
                    lang.code(),
                    w as f32 / s
                );
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }

    /// Galeria (pozri `crate::gallery`): panel AI (aktualne limity, bohata historia, mesiac) pri kazdom DPI a teme.
    #[test]
    #[ignore]
    fn gallery_ai() {
        let _lang = crate::test_lang::guard();
        let Some(dir) = crate::gallery::dir() else {
            return;
        };
        let today = Ymd::new(2026, 11, 20);
        for dpi in crate::gallery::DPIS {
            for (dark, th) in crate::gallery::themes() {
                for (name, data) in fixtures() {
                    if name != "current" && name != "history-rich" && name != "worst-stale-errors" {
                        continue;
                    }
                    for (tab, details) in [
                        (Period::Today, false),
                        (Period::Today, true),
                        (Period::Month, false),
                    ] {
                        if tab == Period::Month && name != "history-rich" {
                            continue;
                        }
                        if details && name != "current" {
                            continue;
                        }
                        let (w, h) = (
                            crate::gallery::px(AI_W_LP, dpi),
                            crate::gallery::px(AI_H_LP, dpi),
                        );
                        let surf = Surface::new(w, h);
                        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                        let mut st = AiState::new();
                        st.data = Some(data.clone());
                        st.details = details;
                        st.tab = tab;
                        month_variant(&mut st, 0);
                        st.paint(&mut gp, &th, &usage_all(), today, NOW);
                        st.paint(&mut gp, &th, &usage_all(), today, NOW);
                        crate::gallery::save(
                            &dir,
                            &format!(
                                "ai-{name}{}{}-{}-{dpi}",
                                if details { "-details" } else { "" },
                                if tab == Period::Month { "-month" } else { "" },
                                crate::gallery::tag(dark)
                            ),
                            &surf,
                        );
                    }
                }
            }
        }
    }

    /// Ulozi snimky panela do `TRANSOM_SHOT_DIR` (rucna vizualna kontrola; `cargo test -- --ignored dump`).
    #[test]
    #[ignore]
    fn dump_popover_pngs() {
        let _lang = crate::test_lang::guard();
        let Some(dir) = std::env::var_os("TRANSOM_SHOT_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let today = Ymd::new(2026, 11, 20);
        for lang in [
            transom_i18n::Lang::En,
            transom_i18n::Lang::Sk,
            transom_i18n::Lang::De,
        ] {
            transom_i18n::set_lang(lang);
            for dark in [true, false] {
                let th = if dark { Theme::dark() } else { Theme::light() };
                for (name, data) in fixtures() {
                    for (tab, details, mv, scrolled) in [
                        (Period::Today, false, 0u8, false),
                        (Period::Today, true, 0, false),
                        (Period::Month, false, 0, false),
                        (Period::Month, false, 0, true),
                        (Period::Month, false, 1, true),
                        (Period::Month, false, 2, true),
                    ] {
                        if !name.starts_with("history-") && tab == Period::Month {
                            continue;
                        }
                        let dpi = crate::gallery::hero_dpi();
                        let s = dpi as f32 / 96.0;
                        let (w, h) = (
                            (AI_W_LP as f32 * s).round() as i32,
                            (AI_H_LP as f32 * s).round() as i32,
                        );
                        let surf = Surface::new(w, h);
                        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, dpi);
                        let mut st = AiState::new();
                        st.data = Some(data.clone());
                        st.details = details;
                        st.tab = tab;
                        month_variant(&mut st, mv);
                        st.paint(&mut gp, &th, &usage_all(), today, NOW);
                        if scrolled {
                            st.scroll = st.max_scroll;
                            st.paint(&mut gp, &th, &usage_all(), today, NOW);
                        }
                        let f = dir.join(format!(
                            "popover-{}-{}-{name}{}{}{}.png",
                            lang.code(),
                            if dark { "dark" } else { "light" },
                            if details { "-details" } else { "" },
                            if tab == Period::Month {
                                format!("-month{mv}")
                            } else {
                                String::new()
                            },
                            if scrolled { "-scrolled" } else { "" }
                        ));
                        let _ = std::fs::write(
                            f,
                            transom_render::offscreen::png(w as u32, h as u32, &surf.rgb()),
                        );
                    }
                }
            }
        }
    }

    fn state_with(days: Vec<DayTotals>, statuses: Vec<SourceStatus>) -> AiState {
        let mut st = AiState::new();
        st.data = Some(Collected {
            days,
            statuses,
            ..Default::default()
        });
        st
    }

    const T_: fn() -> Ymd = || Ymd::new(2026, 11, 20);

    #[test]
    fn clicks_change_period_month_unit_and_filter_and_reset_the_scroll() {
        let (_, rich) = fixtures().into_iter().next().unwrap();
        let mut st = AiState::new();
        st.data = Some(rich);
        st.max_scroll = 300;
        st.scroll = 120;
        assert!(st.click(AiHit::Tab(Period::Month), T_()));
        assert_eq!(
            (st.tab, st.scroll),
            (Period::Month, 0),
            "zmena obdobia vrati posun na zaciatok"
        );
        assert!(st.click(AiHit::Unit(Metric::Cost), T_()));
        assert_eq!(st.month.metric, Metric::Cost);
        assert!(st.click(AiHit::Prov(Filter::Only(Provider::Codex)), T_()));
        assert_eq!(st.month.filter, Filter::Only(Provider::Codex));
        assert!(st.click(AiHit::Day(9), T_()));
        assert_eq!(st.month.day, Some(9));
        assert!(
            st.click(AiHit::Day(9), T_()),
            "druhy klik na ten isty den ho zrusi"
        );
        assert_eq!(st.month.day, None);
        // predosly mesiac: rich fixture ma historiu od 1. 11., takze pred novembrom sa neda ist
        assert!(!st.click(AiHit::MonthPrev, T_()));
        assert!(
            !st.click(AiHit::MonthNext, T_()),
            "dalsi mesiac po dnesku nie je"
        );
        // tlacidla, ktore riesi hostitel, panel sam nemeni
        for h in [AiHit::Refresh, AiHit::Settings, AiHit::Mode(RingMode::Used)] {
            assert!(!st.click(h, T_()));
        }
        // Details a tab
        st.scroll = 10;
        assert!(st.click(AiHit::Details, T_()));
        assert!(st.details && st.scroll == 0);
    }

    #[test]
    fn opening_the_panel_resets_month_view_but_not_the_chosen_mode() {
        let mut st = AiState::new();
        st.month.metric = Metric::Cost;
        st.month.filter = Filter::Only(Provider::Codex);
        st.month.ym = Some((2026, 10));
        st.month.day = Some(3);
        st.scroll = 50;
        st.on_open();
        assert_eq!(st.month, MonthUi::default());
        assert_eq!(st.scroll, 0);
    }

    #[test]
    fn keyboard_scrolls_and_in_month_view_selects_days_and_months() {
        let mut st = state_with(vec![], vec![]);
        st.unit = 40;
        st.view_h = 300;
        st.max_scroll = 200;
        assert!(st.key(0x28, T_()), "dole");
        assert_eq!(st.scroll, 40);
        assert!(st.key(0x22, T_()), "Page Down");
        assert_eq!(st.scroll, 200, "najviac po koniec");
        assert!(st.key(0x21, T_()));
        assert!(st.scroll < 200);
        st.scroll = 100;
        assert!(st.key(0x24, T_()) && st.scroll == 0, "Home");
        assert!(st.key(0x23, T_()) && st.scroll == 200, "End");
        // mesacny pohlad: sipky vyberaju den
        st.tab = Period::Month;
        st.scroll = 0;
        assert!(st.key(0x25, T_()));
        assert_eq!(st.month.day, Some(20), "bez vyberu zacne dneskom");
        assert!(st.key(0x25, T_()));
        assert_eq!(st.month.day, Some(19));
        assert!(st.key(0x24, T_()) && st.month.day == Some(1));
        assert!(st.key(0x23, T_()) && st.month.day == Some(20));
        // ineho klavesu panel nespracuje
        assert!(!st.key(0x41, T_()));
        // bez posuvania: Page Up/Down meni mesiac (tu bez historie sa neda ist spat)
        st.max_scroll = 0;
        assert!(!st.key(0x21, T_()));
    }

    #[test]
    fn scroll_is_clamped_to_the_range() {
        let mut st = AiState::new();
        st.max_scroll = 100;
        assert!(!st.scroll_by(-10), "uz je hore");
        assert!(st.scroll_by(60) && st.scroll == 60);
        assert!(st.scroll_by(500) && st.scroll == 100);
        assert!(!st.scroll_by(1));
        assert!(st.scroll_by(-500) && st.scroll == 0);
    }

    #[test]
    fn real_height_scrolls_when_the_content_is_taller_and_keeps_the_footer() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let th = Theme::dark();
        let (w, h) = (AI_W_LP, AI_H_LP);
        let (_, rich) = fixtures().into_iter().next().unwrap();
        let mut st = AiState::new();
        st.data = Some(rich);
        st.tab = Period::Month;
        let surf = Surface::new(w, h);
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
        st.paint(&mut gp, &th, &usage_all(), Ymd::new(2026, 11, 20), NOW);
        assert!(
            st.max_scroll > 0,
            "mesacny pohlad je vyssi nez okno: posuvnik je potrebny"
        );
        // paticka (Obnovit / Nastavenia) a prepinac Zostava/Spotrebovane su stale klikatelne
        for h in [
            AiHit::Refresh,
            AiHit::Settings,
            AiHit::Mode(RingMode::Used),
            AiHit::Mode(RingMode::Remaining),
        ] {
            assert!(st.hits.iter().any(|(_, x)| *x == h), "{h:?} chyba");
        }
        // prvky posuvnej casti nikdy nepresahuju okno obsahu (orezane): kazdy hit lezi v okne panela
        for (r, hit) in &st.hits {
            assert!(r.y >= 0 && r.y + r.h <= h, "{hit:?} {r:?} mimo okna");
        }
        // po posunuti na koniec je posledny riadok mesacneho pohladu viditelny (a stale nic nezasahuje do patičky)
        st.scroll = st.max_scroll;
        st.paint(&mut gp, &th, &usage_all(), Ymd::new(2026, 11, 20), NOW);
        assert_eq!(st.scroll, st.max_scroll);
        assert!(
            st.hits.iter().any(|(_, x)| matches!(x, AiHit::Day(_))),
            "dni grafu su po posunuti stale v okne alebo orezane"
        );
    }

    #[test]
    fn accessibility_names_cover_every_visible_control_in_every_language() {
        let _lang = crate::test_lang::guard();
        let th = Theme::dark();
        let (w, h) = (AI_W_LP, AI_H_LP * 2);
        let (_, rich) = fixtures().into_iter().next().unwrap();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            let mut st = AiState::new();
            st.data = Some(rich.clone());
            st.tab = Period::Month;
            st.month.day = Some(9);
            let surf = Surface::new(w, h);
            let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
            st.paint(&mut gp, &th, &usage_all(), Ymd::new(2026, 11, 20), NOW);
            let (entries, _) = st.a11y();
            assert_eq!(entries.len(), st.hits.len());
            for (e, (_, hit)) in entries.iter().zip(&st.hits) {
                assert!(!e.name.trim().is_empty(), "{lang:?} {hit:?}: prazdny nazov");
                assert!(
                    !e.name.contains("ai."),
                    "{lang:?} {hit:?}: chybajuci preklad ({})",
                    e.name
                );
            }
            // den obsahuje datum a stav / hodnotu slovom (nie len farbu / tvar)
            let day = entries
                .iter()
                .find(|e| e.name.contains("2026"))
                .expect("den s datumom");
            assert!(day.name.contains(':'), "{}", day.name);
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }

    #[test]
    fn cost_view_is_labelled_as_an_estimate_and_not_as_plan_quota_in_every_language() {
        let _lang = crate::test_lang::guard();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            let (cost, tokens) = (t("ai.m.cap_cost"), t("ai.m.cap_tokens"));
            for (k, v) in [("cost", &cost), ("tokens", &tokens)] {
                assert!(!v.starts_with("ai."), "{lang:?} {k}: chyba preklad");
            }
            assert_ne!(cost, tokens, "{lang:?}");
            // znak odhadu "≈" v popise ceny (v kazdom jazyku), a popis tokenov hovori "nie kvota"
            assert!(cost.contains('≈'), "{lang:?}: {cost}");
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }

    #[test]
    fn archived_days_are_named_and_described_as_archived_with_the_price_list_date() {
        let _lang = crate::test_lang::guard();
        let th = Theme::dark();
        let (w, h) = (AI_W_LP, AI_H_LP * 2);
        let (_, archived) = fixtures()
            .into_iter()
            .find(|(n, _)| *n == "history-archived")
            .unwrap();
        let mut st = AiState::new();
        st.data = Some(archived);
        st.tab = Period::Month;
        st.month.day = Some(5);
        // filter Claude: Codex ma pokrytie az od 12. 11., takze vo filtri Vsetci by dni pred nim boli neuplne
        st.month.filter = Filter::Only(Provider::ClaudeCode);
        let surf = Surface::new(w, h);
        let mut gp = GdiPainter::new(surf.dc, Size { w, h }, 96);
        st.paint(&mut gp, &th, &usage_all(), Ymd::new(2026, 11, 20), NOW);
        let view = st.last_month.clone().expect("mesacny pohlad");
        assert!(view.archived_days > 0);
        // den z archivu: nazov obsahuje slovo "archived"; den z logov nie
        assert!(
            st.hit_name(AiHit::Day(5)).contains(&t("ai.m.leg.arch")),
            "{}",
            st.hit_name(AiHit::Day(5))
        );
        assert!(!st.hit_name(AiHit::Day(12)).contains(&t("ai.m.leg.arch")));
        // 3. 11. bol pri archivacii hranicny: ostava neuplny (dolny odhad), 5. 11. je uplny
        assert_eq!(
            view.cells[2].state,
            transom_usage::history::DayState::Incomplete
        );
        assert_eq!(
            view.cells[4].state,
            transom_usage::history::DayState::Recorded
        );
        assert_eq!(view.cells[4].price_dates, vec!["2026-11-15".to_string()]);
        // poznamka o archive je medzi poznamkami mesiaca
        assert!(view.notes.contains(&transom_usage::history::Note::Archived));
    }

    #[test]
    fn claude_unavailable_gets_a_specific_reason_without_secrets() {
        let _lang = crate::test_lang::guard();
        use transom_usage::SourceKind::{ClaudeLocal, ClaudeStatusline};
        let on = |statusline: bool, local: bool| transom_config::Usage {
            claude_statusline: statusline,
            claude_local: local,
            ..Default::default()
        };
        let st = |records: usize, limits: bool, err: Option<SourceError>| SourceStatus {
            kind: ClaudeStatusline,
            provider: Some(Provider::ClaudeCode),
            records,
            limits_seen: limits,
            error: err,
            ..Default::default()
        };
        let with = |s: SourceStatus| Collected {
            statuses: vec![s],
            ..Default::default()
        };
        let hint = |u: &transom_config::Usage, d: Option<&Collected>| {
            provider_hint(Provider::ClaudeCode, u, d)
        };
        // 1. zdroj statusLine vypnuty (aj ked lokalne prepisy bezia): tokeny nedokazuju limit planu
        let only_local = with(SourceStatus {
            kind: ClaudeLocal,
            provider: Some(Provider::ClaudeCode),
            ..Default::default()
        });
        assert_eq!(
            hint(&on(false, true), Some(&only_local)),
            Some(t("ai.hint.off"))
        );
        // 2. zapnuty, ale most nema ziadne zaznamy / subor: statusLine nie je pripojeny
        assert_eq!(
            hint(
                &on(true, false),
                Some(&with(st(0, false, Some(SourceError::NoBridgeData))))
            ),
            Some(t("ai.hint.setup"))
        );
        assert_eq!(
            hint(&on(true, false), Some(&with(st(0, false, None)))),
            Some(t("ai.hint.setup"))
        );
        // 3. zaznamy su, ale ziaden nenesie limity: ucet limity nehlasi
        assert_eq!(
            hint(&on(true, false), Some(&with(st(12, false, None)))),
            Some(t("ai.hint.nolimits"))
        );
        // 4. este sa nezbieralo: nic neohlasujeme (nacitava sa)
        assert_eq!(hint(&on(true, false), None), None);
        // 5. limity videne: ziadna vyhovorka (okna sa kresla)
        assert_eq!(
            hint(&on(true, false), Some(&with(st(12, true, None)))),
            None
        );
        // texty nikdy neobsahuju cestu ani tajomstvo
        for k in [
            "ai.hint.off",
            "ai.hint.setup",
            "ai.hint.nolimits",
            "ai.hint.codex",
        ] {
            for lang in transom_i18n::Lang::ALL {
                let txt = transom_i18n::t_in(lang, k);
                for bad in [
                    "AppData",
                    "credentials",
                    "token:",
                    "cookie",
                    "C:\\",
                    ".json",
                ] {
                    assert!(!txt.contains(bad), "{k} {lang:?}: {txt}");
                }
            }
        }
        // kruh: tooltip nesie dovod
        let r = ring_view(
            Some(&with(st(0, false, Some(SourceError::NoBridgeData)))),
            &transom_config::Usage {
                bar_provider: "claude".into(),
                ..on(true, false)
            },
            1000,
        );
        assert_eq!(r.label, t("ring.unavailable"));
        assert!(r.name.contains(&t("ai.hint.setup")), "{}", r.name);
        assert_eq!(r.fill, None, "nedostupne nikdy nie je 0 %");
    }
}
