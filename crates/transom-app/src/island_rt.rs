//! Ostrov v strede listy - cast `App`, ktora ho riadi: co ukazat (hudba, AI, oznamenia), sledovanie priecinka snimok
//! obrazovky a mostu statusline (len oznamenia Windows o zmene, ziadne periodicke citanie), klik a kreslenie.
//! Nic z toho sa nelogue s obsahom: ziadne nazvy skladieb, texty schranky ani nazvy suborov.
use crate::island::{AiLive, AiState, Compact, MusicLive, Notice, NoticeAct};
use crate::island_menu::{Limit, Shot, Tab};
use crate::{log, notifier, popup, schedule_timer, App, NotchView, N_BRIDGE, N_SHOTS, T_ISLAND};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use transom_core::item::{Action, Glyph, ItemId, Mark, Part, Seg};
use transom_core::media::PlayState;
use transom_i18n::t;
use transom_platform::dragdrop;

/// Claude Code "pracuje", kym most statusline dostal zaznam pred menej nez tolko sekundami (statusline sa obnovuje pri
/// kazdej zmene konverzacie; dlhy prikaz bez vystupu moze byt aj minutu ticho).
const AI_ACTIVE_S: u64 = 45;
/// S hookmi: praca trva od zaciatku / nastroja po koniec; najviac tolko bez dalsej udalosti (spadnuta relacia).
const AI_EVENT_MAX_S: i64 = 15 * 60;
/// Ako dlho po poslednej aktivite ostava logo s bodkou (necinny agent), potom sa skryje (s).
const AI_IDLE_S: u64 = 30 * 60;

/// Farba z obalu: (id obalu, farba; `None` = sivy obal).
type CoverColor = (u64, Option<(u8, u8, u8)>);

/// Sledovanie a posledne videne hodnoty (stav mimo cisteho modelu ostrova).
#[derive(Default)]
pub struct IslandRt {
    pub model: crate::island::Island,
    /// Co sa ukazuje (`None` = pokoj: logo transom).
    pub shown: Option<Compact>,
    /// Zachytavanie zvuku pre ekvalizer (bezi len kym hudba hra a ekvalizer je v liste).
    spectrum: Option<transom_platform::spectrum::Spectrum>,
    /// Zobrazene vysky stlpcov (plynule k urovniam zo zvuku) a cas poslednej snimky.
    eq: [f32; 5],
    eq_at: Option<Instant>,
    /// Naposledy nakreslene vysky (28 krokov ~ px pri 200 %): rovnake = bez prekreslenia.
    eq_drawn: [u8; 5],
    /// Zaciatok nahradnej animacie (hudba nehra na predvolenom vystupe, napr. iny vystup alebo Spotify Connect).
    eq_t0: Option<Instant>,
    /// Farba ekvalizera z obalu (id obalu, farba).
    eq_color: std::cell::Cell<Option<CoverColor>>,
    shots: Option<Box<dyn std::any::Any + Send>>,
    shots_dir: Option<PathBuf>,
    /// Najnovsia snimka, ktora sa uz ohlasila (cas zmeny).
    shot_seen: Option<SystemTime>,
    bridge: Option<Box<dyn std::any::Any + Send>>,
    /// Posledny zaznam mostu: (cas zmeny suboru, limity).
    ai: Option<(SystemTime, transom_usage::statusline::LastLimits)>,
    /// Posledna udalost z hookov Claude Code: (cas zmeny suboru, cas udalosti, druh).
    ev: Option<(SystemTime, i64, String)>,
    /// Codex: posledny subor relacie (cas zmeny, snimok limitov) a najblizsia kontrola.
    codex: Option<(SystemTime, Option<transom_usage::PlanSnapshot>)>,
    /// Snimky limitov pridane do dat panela AI (poskytovatel, cas zaznamu) - pri dalsom sa nahradia.
    injected: Vec<(Option<transom_usage::Provider>, i64)>,
    codex_check: Option<Instant>,
    /// Najnovsia polozka historie schranky, ktora sa uz ohlasila (`added`); `None` = este nenacitane.
    clip_seen: Option<i64>,
    /// Naplanovany tik (`T_ISLAND`): novy sa nastavi len ked je skor (SetTimer by beziaci casovac posunul a castejsie
    /// udalosti nez snimka by ekvalizer zastavili).
    due: Option<Instant>,
    /// Bezi tik prehravaca (cas skladby v otvorenom vybezku).
    media_tick: bool,
    /// LEN test (`--test-open-tools`): otvorit Nastroje v tomto case.
    test_tools: Option<Instant>,
    /// LEN test (`--test-island-click`): klik na ostrov v tomto case.
    test_click: Option<Instant>,
    /// Zaciatok nahladu skladby (posuv dlheho nazvu) a posledna snimka posuvu.
    peek_t0: Option<Instant>,
    peek_at: Option<Instant>,
    /// Skladba a rezim, pre ktore bezi posuv (zmena = posuv od zaciatku).
    peek_key: Option<String>,
}

impl App {
    /// Zapne / vypne sledovanie podla nastavenia (pri starte a po zmene konfiguracie).
    pub(crate) fn sync_island_watches(&mut self) {
        let Some(n) = notifier() else { return };
        let cfg = self.config.notch;
        // snimky obrazovky: priecinok Windows (moze byt presmerovany)
        let dir = cfg
            .screenshot
            .then(transom_platform::shellops::screenshots_folder)
            .flatten()
            .map(PathBuf::from);
        if dir != self.island.shots_dir {
            self.island.shots = None;
            self.island.shot_seen = dir.as_deref().and_then(newest_image).map(|(_, t)| t);
            if let Some(d) = &dir {
                self.island.shots = Some(transom_platform::dirwatch::watch_boxed(
                    &[d.to_string_lossy().into_owned()],
                    Arc::new(move |_| n.notify(N_SHOTS)),
                ));
            }
            log(&format!(
                "ostrov: snimky obrazovky sa sleduju: {}",
                dir.is_some()
            ));
            self.island.shots_dir = dir;
        }
        let want_ai = cfg.island_ai && self.config.usage.claude_statusline;
        if want_ai != self.island.bridge.is_some() {
            self.island.bridge = None;
            self.island.ai = None;
            if want_ai {
                let path = transom_usage::cache::statusline_path();
                if let Some(d) = path.parent() {
                    self.island.bridge = Some(transom_platform::dirwatch::watch_boxed(
                        &[d.to_string_lossy().into_owned()],
                        Arc::new(move |_| n.notify(N_BRIDGE)),
                    ));
                }
                self.bridge_changed();
            }
            log(&format!("ostrov: most statusline sa sleduje: {want_ai}"));
        }
    }

    /// Zmena v priecinku mostu (nemusi byt nas subor): precitat koniec suboru, len ak sa zmenil. Nic sa nelogue (zapis
    /// logu do toho isteho priecinka by sa inak tocil dokola).
    pub(crate) fn bridge_changed(&mut self) {
        let mut changed = self.events_changed();
        let path = transom_usage::cache::statusline_path();
        if let Ok(m) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            if !self.island.ai.as_ref().is_some_and(|a| a.0 == m) {
                let l = transom_usage::statusline::last_limits(&path).unwrap_or_default();
                self.island.ai = Some((m, l));
                changed = true;
                // ta ista hodnota aj v paneli a kruhu (zber bezi v intervale; bez toho by sa lisili)
                if l.five.is_some() || l.week.is_some() {
                    let win = |minutes: u32, v: Option<f64>, r: Option<i64>| {
                        v.map(|u| transom_usage::LimitWindow {
                            minutes,
                            used_percent: u,
                            resets_at: r,
                        })
                    };
                    self.live_into_usage(transom_usage::PlanSnapshot {
                        provider: Some(transom_usage::Provider::ClaudeCode),
                        windows: [
                            win(300, l.five, l.five_reset),
                            win(10080, l.week, l.week_reset),
                        ]
                        .into_iter()
                        .flatten()
                        .collect(),
                        recorded_at: l.ts,
                        source: transom_usage::SnapshotSource::ClaudeStatusline,
                        ..Default::default()
                    });
                }
            }
        }
        if changed {
            self.island_sync();
        }
    }

    /// Nova udalost z hookov: koniec ulohy, otazka na povolenie a cakanie na odpoved sa ohlasia (len nove, nie stare
    /// zo suboru pri starte). Text spravy sa nikde necita.
    fn events_changed(&mut self) -> bool {
        let path = transom_usage::cache::events_path();
        let Ok(m) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            return false;
        };
        if self.island.ev.as_ref().is_some_and(|e| e.0 == m) {
            return false;
        }
        let first = self.island.ev.is_none();
        let Some((ts, kind)) = transom_usage::statusline::last_event(&path) else {
            return false;
        };
        // ta ista udalost znova (povolenie hlasi aj neskorsie upozornenie): bez druheho oznamenia
        let repeat = self
            .island
            .ev
            .as_ref()
            .is_some_and(|(_, pts, pk)| *pk == kind && ts - pts < 30);
        let fresh = !first && !repeat && crate::ai::now_unix() - ts < 30;
        self.island.ev = Some((m, ts, kind.clone()));
        if fresh && self.config.notch.island_ai {
            let (icon, title) = match kind.as_str() {
                "stop" => ('\u{E930}', "island.ai_done"),
                "ask" => ('\u{E897}', "island.ai_ask"),
                "idle" => ('\u{E8BD}', "island.ai_idle"),
                "error" => ('\u{E783}', "island.ai_error"),
                _ => return true,
            };
            log(&format!("ostrov: Claude {kind}"));
            self.island_notice(Notice {
                icon,
                title: t(title),
                sub: t("island.ai_sub"),
                act: Some(NoticeAct::Ai),
            });
        }
        true
    }

    /// Nova snimka obrazovky (najnovsi obrazok priecinka novsi nez posledny ohlaseny).
    pub(crate) fn shots_changed(&mut self) {
        let Some(dir) = self.island.shots_dir.clone() else {
            return;
        };
        let Some((path, at)) = newest_image(&dir) else {
            return;
        };
        if self.island.shot_seen.is_some_and(|s| at <= s) {
            return;
        }
        self.island.shot_seen = Some(at);
        log("ostrov: nova snimka obrazovky");
        self.island_notice(Notice {
            icon: transom_render::scene::glyph_char(Glyph::Screenshot),
            title: t("island.shot"),
            sub: t("island.shot_sub"),
            act: Some(NoticeAct::Shot(path)),
        });
    }

    /// Historia schranky sa zmenila: nova polozka = oznamenie (prve nacitanie len zapamata stav).
    pub(crate) fn clip_changed(&mut self) {
        if !self.tools_snap.clip_enabled {
            self.island.clip_seen = None;
            return;
        }
        let newest = self
            .tools_snap
            .clipboard
            .iter()
            .map(|r| r.added)
            .max()
            .unwrap_or(0);
        let Some(seen) = self.island.clip_seen.replace(newest) else {
            return;
        };
        if newest <= seen || !self.config.notch.clip_saved {
            return;
        }
        // snimka obrazovky / vyrez prave ohlaseny (obrazok ide aj do schranky): jedno oznamenie, nie dve
        if matches!(
            self.island
                .model
                .notice(Instant::now())
                .and_then(|n| n.act.as_ref()),
            Some(NoticeAct::Shot(_) | NoticeAct::Stash)
        ) {
            return;
        }
        log("ostrov: ulozene do schranky");
        self.island_notice(Notice {
            icon: transom_render::scene::glyph_char(Glyph::Clipboard),
            title: t("island.clip"),
            sub: t("island.clip_sub"),
            act: Some(NoticeAct::Clipboard),
        });
    }

    /// Kratke oznamenie v ostrove (3 s; klik pocas neho otvori, co k nemu patri).
    pub(crate) fn island_notice(&mut self, n: Notice) {
        self.island.model.notify(n, Instant::now());
        self.island_sync();
    }

    /// Cerstvy snimok limitov (most statusline, relacia Codex) aj do dat panela AI a kruhu v liste: nahradi predosly
    /// takto pridany snimok toho isteho poskytovatela; starsi nez zozbierany sa nepouzije.
    fn live_into_usage(&mut self, mut snap: transom_usage::PlanSnapshot) {
        let Some(d) = self.ai.data.as_mut() else {
            return;
        };
        let prev = self
            .island
            .injected
            .iter()
            .position(|(p, _)| *p == snap.provider);
        if let Some(i) = prev {
            let (p, ts) = self.island.injected.remove(i);
            d.plans
                .retain(|x| !(x.provider == p && x.recorded_at == ts));
        }
        let newest = d
            .plans
            .iter()
            .filter(|x| x.provider == snap.provider)
            .max_by_key(|x| x.recorded_at);
        if newest.is_some_and(|n| n.recorded_at >= snap.recorded_at) {
            return;
        }
        // nazov planu (chip v paneli) z posledneho zozbieraneho snimku
        if snap.plan.is_empty() {
            if let Some(n) = newest {
                snap.plan = n.plan.clone();
            }
        }
        self.island.injected.push((snap.provider, snap.recorded_at));
        d.plans.push(snap);
        crate::invalidate_all();
        if popup::popover_open() == Some(crate::POP_AI) {
            popup::invalidate_popover();
        }
    }

    /// Codex pracuje: najnovsi subor relacie sa zmenil (kontrola kazdych 5 s, len ked je zdroj zapnuty).
    fn codex_poll(&mut self, now: Instant) {
        if !(self.config.notch.island_ai && self.config.usage.codex_local) {
            self.island.codex = None;
            self.island.codex_check = None;
            return;
        }
        if self.island.codex_check.is_some_and(|t| now < t) {
            return;
        }
        self.island.codex_check = Some(now + std::time::Duration::from_secs(5));
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".codex")));
        let Some((path, m)) = home
            .as_deref()
            .and_then(transom_usage::codex::latest_session)
        else {
            return;
        };
        if self.island.codex.as_ref().is_some_and(|c| c.0 == m) {
            return;
        }
        let snap = transom_usage::codex::last_snapshot(&path);
        if let Some(sn) = snap.clone() {
            self.live_into_usage(sn);
        }
        self.island.codex = Some((m, snap));
    }

    /// Codex prave pracuje: limit z posledneho snimku relacie (tyzden pri hranici, inak najkratsie okno).
    fn codex_live(&self) -> Option<(SystemTime, AiLive)> {
        let (m, snap) = self.island.codex.as_ref()?;
        let age = SystemTime::now().duration_since(*m).ok()?;
        // pracuje, kym sa relacia meni; potom necinny (len logo s bodkou), po case skryty
        let state = if age.as_secs() < AI_ACTIVE_S {
            AiState::Thinking
        } else if age.as_secs() < AI_IDLE_S {
            AiState::Idle
        } else {
            return None;
        };
        let now_s = crate::ai::now_unix();
        let mut wins: Vec<&transom_usage::LimitWindow> = snap
            .as_ref()
            .map(|s| {
                s.windows
                    .iter()
                    .filter(|w| w.resets_at.is_none_or(|r| r > now_s))
                    .collect()
            })
            .unwrap_or_default();
        wins.sort_by_key(|w| w.minutes);
        let low = |w: &transom_usage::LimitWindow| {
            100.0 - w.used_percent <= crate::ai_cards::LOW_REMAINING_PCT as f64
        };
        let pick = wins
            .iter()
            .rev()
            .find(|w| w.minutes >= 7 * 24 * 60 && low(w))
            .or(wins.first())
            .copied();
        let live = match pick {
            Some(w) => AiLive {
                name: "Codex".into(),
                state,
                window: if w.minutes >= 7 * 24 * 60 {
                    t("island.week")
                } else {
                    crate::ai::window_short(w.minutes)
                },
                value: Some(
                    self.limit_text(w.used_percent, snap.as_ref().map_or(0, |s| s.recorded_at)),
                ),
                alert: low(w),
            },
            None => AiLive {
                name: "Codex".into(),
                state,
                window: String::new(),
                value: None,
                alert: false,
            },
        };
        Some((*m, live))
    }

    /// "34 % využité" / "66 % zostáva" podla rezimu kruhu; "~" pri starsom zazname.
    fn limit_text(&self, used: f64, recorded_at: i64) -> String {
        let mode = transom_usage::limits::RingMode::parse(&self.config.usage.ring_mode);
        let used = used.round().clamp(0.0, 100.0) as u8;
        let (v, key) = match mode {
            transom_usage::limits::RingMode::Used => (used, "ring.used"),
            transom_usage::limits::RingMode::Remaining => (100 - used, "ring.left"),
        };
        let s = transom_i18n::tf(key, &[&v.to_string()]);
        if crate::ai::now_unix() - recorded_at > crate::ai::LIMIT_FRESH_S {
            format!("~{s}")
        } else {
            s
        }
    }

    /// AI prave pracuje (Claude Code alebo Codex, kto pracoval naposledy): limit do ostrova.
    fn ai_live(&self) -> Option<AiLive> {
        let claude = self.claude_live();
        let codex = self.codex_live();
        match (claude, codex) {
            // pracujuci ma prednost pred necinnym, inak novsi
            (Some((a, x)), Some((b, y))) => Some(match (x.state.busy(), y.state.busy()) {
                (true, false) => x,
                (false, true) => y,
                _ if b > a => y,
                _ => x,
            }),
            (Some((_, x)), None) => Some(x),
            (None, Some((_, y))) => Some(y),
            (None, None) => None,
        }
    }

    /// Claude Code prave pracuje (cerstvy zaznam mostu statusline).
    fn claude_live(&self) -> Option<(SystemTime, AiLive)> {
        let (m, l) = self.island.ai?;
        let now_s = crate::ai::now_unix();
        // okno, ktoreho reset uz prebehol, nema platnu hodnotu (nova relacia este nehlasila novu)
        let live =
            |v: Option<f64>, reset: Option<i64>| v.filter(|_| reset.is_none_or(|r| r > now_s));
        let (ts, r5, r7) = (l.ts, live(l.five, l.five_reset), live(l.week, l.week_reset));
        // s hookmi presne (ako klavesnica Codex): zadanie a nastroje = premysla, povolenie / otazka = caka na teba,
        // koniec ulohy = neprecitane (az do dalsieho zadania), chyba API = chyba; inak podla statusline (cerstvy =
        // premysla, potom necinny)
        let state = match &self.island.ev {
            Some((_, ets, kind)) if now_s - ets < AI_IDLE_S as i64 => {
                let age = now_s - ets;
                match kind.as_str() {
                    "start" | "tool" if age < AI_EVENT_MAX_S => AiState::Thinking,
                    "ask" if age < AI_EVENT_MAX_S => AiState::Waiting,
                    "stop" | "idle" => AiState::Unread,
                    "error" => AiState::Error,
                    _ => AiState::Idle,
                }
            }
            _ => {
                let age = SystemTime::now()
                    .duration_since(m)
                    .map(|d| d.as_secs())
                    .unwrap_or(u64::MAX);
                if age < AI_ACTIVE_S {
                    AiState::Thinking
                } else if age < AI_IDLE_S {
                    AiState::Idle
                } else {
                    return None;
                }
            }
        };
        let m = match &self.island.ev {
            Some((em, ..)) => (*em).max(m),
            None => m,
        };
        let mode = transom_usage::limits::RingMode::parse(&self.config.usage.ring_mode);
        let shown = |used: f64| {
            let used = used.round().clamp(0.0, 100.0) as u8;
            let v = match mode {
                transom_usage::limits::RingMode::Used => used,
                transom_usage::limits::RingMode::Remaining => 100 - used,
            };
            let key = match mode {
                transom_usage::limits::RingMode::Used => "ring.used",
                transom_usage::limits::RingMode::Remaining => "ring.left",
            };
            let mut s = transom_i18n::tf(key, &[&v.to_string()]);
            // starsi zaznam (napr. nova relacia este bez odpovede): "~" ako pri kruhu v liste
            if now_s - ts > crate::ai::LIMIT_FRESH_S {
                s = format!("~{s}");
            }
            s
        };
        // tyzdenny limit pri hranici ma prednost (zostava malo), inak 5 h
        let week_low = r7.is_some_and(|w| 100.0 - w <= crate::ai_cards::LOW_REMAINING_PCT as f64);
        Some((
            m,
            match (week_low, r7, r5) {
                (true, Some(w), _) => AiLive {
                    name: "Claude".into(),
                    state,
                    window: t("island.week"),
                    value: Some(shown(w)),
                    alert: true,
                },
                (_, _, Some(f)) => AiLive {
                    name: "Claude".into(),
                    state,
                    window: crate::ai::window_short(300),
                    value: Some(shown(f)),
                    alert: 100.0 - f <= crate::ai_cards::LOW_REMAINING_PCT as f64,
                },
                _ => AiLive {
                    name: "Claude".into(),
                    state,
                    window: crate::ai::window_short(300),
                    value: None,
                    alert: false,
                },
            },
        ))
    }

    /// Vyber obsahu ostrova (vola sa po kazdej udalosti; lista sa prekresli len pri zmene).
    pub(crate) fn island_sync(&mut self) {
        let now = Instant::now();
        let cfg = self.config.notch;
        let session = self.chosen_session().cloned();
        if let Some(s) = &session {
            self.island
                .model
                .music(Some(&s.media_key()), s.state == PlayState::Playing, now);
        }
        let music = session.filter(|_| cfg.island_music).map(|s| MusicLive {
            app: app_name(&s.app_id),
            title: s.title,
            artist: s.artist,
            playing: s.state == PlayState::Playing,
        });
        self.codex_poll(now);
        let ai = if cfg.island_ai { self.ai_live() } else { None };
        let c = self.island.model.pick(now, music, ai);
        if self.island_shots_open() != self.library.as_ref().is_some_and(|l| l.is_open())
            && !self.tools_open
        {
            self.library_sync();
        }
        if c != self.island.shown {
            // dlhy nazov skladby (nahlad, hudba vedla AI): posuv od zaciatku len pri novej skladbe alebo inom rezime;
            // ina zmena (percenta AI, prehravanie) posuv nerestartuje
            let key = match &c {
                Some(Compact::Peek(m)) => Some(format!("p|{}|{}", m.title, m.artist)),
                Some(Compact::MusicAi(m, _)) => Some(format!("a|{}|{}", m.title, m.artist)),
                _ => None,
            };
            if key != self.island.peek_key {
                self.island.peek_key = key.clone();
                if key.is_some() {
                    self.island.peek_t0 = Some(Instant::now());
                    transom_platform::appbar::pace_timer(crate::T_EQ, true);
                } else if self.island.peek_t0.take().is_some() && self.island.spectrum.is_none() {
                    transom_platform::appbar::pace_timer(crate::T_EQ, false);
                }
            }
            self.island.shown = c;
            crate::invalidate_all();
        }
        // dalsi tik: koniec oznamenia / nazvu skladby / dobehu hudby / aktivity AI, a snimky ekvalizera
        let mut wait: Option<u64> = self
            .island
            .model
            .next_change(now)
            .map(|t| t.duration_since(now).as_millis() as u64 + 20);
        if let Some(t) = self.island.codex_check {
            let ms = t.saturating_duration_since(now).as_millis() as u64 + 20;
            wait = Some(wait.map_or(ms, |w| w.min(ms)));
        }
        if let Some(t) = self.island.test_tools.or(self.island.test_click) {
            let ms = t.saturating_duration_since(now).as_millis() as u64;
            wait = Some(wait.map_or(ms, |w| w.min(ms)));
        }
        // zmeny stavu AI v case: koniec prace (statusline, Codex), hotovo -> necinny, skrytie loga
        let mut at = |since: SystemTime, after_s: &[u64]| {
            if let Ok(d) = SystemTime::now().duration_since(since) {
                if let Some(next) = after_s
                    .iter()
                    .map(|s| s * 1000)
                    .find(|s| *s > d.as_millis() as u64)
                {
                    let left = next - d.as_millis() as u64 + 20;
                    wait = Some(wait.map_or(left, |w| w.min(left)));
                }
            }
        };
        if let Some((m, ..)) = self.island.ai {
            at(m, &[AI_ACTIVE_S, AI_IDLE_S]);
        }
        if let Some((m, _)) = &self.island.codex {
            at(*m, &[AI_ACTIVE_S, AI_IDLE_S]);
        }
        if let Some((_, ets, _)) = &self.island.ev {
            let since =
                std::time::UNIX_EPOCH + std::time::Duration::from_secs((*ets).max(0) as u64);
            at(since, &[AI_EVENT_MAX_S as u64, AI_IDLE_S]);
        }
        self.sync_eq();
        if self.music_card_open() && !self.island.media_tick {
            self.island.media_tick = true;
            schedule_timer(crate::T_MEDIA, 500);
        }
        match wait {
            Some(ms) => {
                let at = now + std::time::Duration::from_millis(ms.min(60_000));
                // beziaci tik, ktory pride skor, sa neposuva (SetTimer by ho pri castych udalostiach stale odsuval)
                if !self.island.due.is_some_and(|d| d > now && d <= at) {
                    self.island.due = Some(at);
                    schedule_timer(T_ISLAND, ms.min(60_000) as u32);
                }
            }
            None => {
                self.island.due = None;
                transom_platform::appbar::cancel_timer(T_ISLAND);
            }
        }
    }

    /// Ekvalizer sa hybe: hudba hra a pohyb je povoleny (nie pri obmedzenych animaciach, skrytej liste a ochrane OLED).
    fn eq_animating(&self) -> bool {
        matches!(&self.island.shown, Some(Compact::Music(m)) if m.playing)
            && transom_platform::system::animations_enabled()
            && !self.config.oled.enabled
            && !self.hidden
    }

    /// Ekvalizer: zachytavanie zvuku a snimky s obnovou obrazovky len kym je co ukazovat.
    fn sync_eq(&mut self) {
        let on = self.eq_animating();
        if on && self.island.spectrum.is_none() {
            let n = notifier();
            self.island.spectrum = Some(transom_platform::spectrum::Spectrum::start(move || {
                // prva nenulova uroven po tichu: snimky sa rozbehnu (inak bezia len pocas pohybu)
                if let Some(n) = n {
                    n.notify(crate::N_EQ);
                }
            }));
            self.island.eq_at = None;
            transom_platform::appbar::pace_timer(crate::T_EQ, true);
            log("ostrov: ekvalizer zo zvuku zapnuty");
        } else if !on && self.island.spectrum.is_some() {
            self.island.spectrum = None;
            self.island.eq = [0.0; 5];
            transom_platform::appbar::pace_timer(crate::T_EQ, false);
            log("ostrov: ekvalizer zo zvuku vypnuty");
        }
    }

    /// Nove urovne zo zvuku: snimky ekvalizera (kym sa stlpce hybu).
    pub(crate) fn eq_wake(&mut self) {
        if self.island.spectrum.is_some() {
            transom_platform::appbar::pace_timer(crate::T_EQ, true);
        }
    }

    /// Snimka ekvalizera (s obnovou obrazovky): stlpce plynulo k urovniam zo zvuku; prekresli sa len ostrov. Ked
    /// sa uz nic nehybe, snimky sa zastavia (dalsie zobudi nova uroven zo zvuku).
    pub(crate) fn eq_frame(&mut self) {
        if self.island.peek_t0.is_some() {
            let now = Instant::now();
            if self
                .island
                .peek_at
                .is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_micros(24_000))
            {
                self.island.peek_at = Some(now);
                self.peek_frame();
            }
            if self.island.spectrum.is_none() {
                return;
            }
        }
        let Some(sp) = &self.island.spectrum else {
            transom_platform::appbar::pace_timer(crate::T_EQ, false);
            return;
        };
        let now = Instant::now();
        // ekvalizer najviac ~60 snimok za sekundu (prekreslenie listy pri 240 Hz by stalo stvrtinu jadra; pohyb
        // stlpcov je plynuly aj takto), ostatne snimky obrazovky sa preskocia
        if self
            .island
            .eq_at
            .is_some_and(|t| now.duration_since(t) < std::time::Duration::from_micros(15_500))
        {
            return;
        }
        // zvuk z predvoleneho vystupu, inak nahradna animacia (hudba hra inde)
        let live = sp.active();
        let target: [u8; 5] = if live {
            self.island.eq_t0 = None;
            sp.levels()
        } else {
            let t = self.island.eq_t0.get_or_insert(now).elapsed().as_secs_f32();
            std::array::from_fn(|i| {
                (transom_render::scene::eq_level(i, t, true) * 255.0).round() as u8
            })
        };
        let dt = self.island.eq_at.map_or(1.0 / 120.0, |t| {
            now.duration_since(t).as_secs_f32().min(0.05)
        });
        self.island.eq_at = Some(now);
        let k = 1.0 - (-dt * 22.0).exp();
        let mut moving = false;
        for (v, t) in self.island.eq.iter_mut().zip(target) {
            let t = t as f32;
            let before = *v;
            *v += (t - *v) * k;
            if (t - *v).abs() < 0.6 {
                *v = t;
            }
            moving |= (*v - before).abs() > 0.05 || (t - *v).abs() > 0.0;
        }
        let drawn = self.island.eq.map(|v| (v / 255.0 * 28.0).round() as u8);
        if drawn != self.island.eq_drawn {
            self.island.eq_drawn = drawn;
            for (m, placed) in &self.layouts {
                if self.hidden_bars.contains(m) {
                    continue;
                }
                if let Some(p) = placed.iter().find(|p| p.item == ItemId::Shelf) {
                    transom_platform::invalidate_span(m, p.x, p.w);
                }
            }
        }
        if live && !moving && self.island.peek_t0.is_none() {
            transom_platform::appbar::pace_timer(crate::T_EQ, false);
            self.island.eq_at = None;
        }
    }

    /// Cas posuvu dlheho nazvu v nahlade skladby (ms od jeho zaciatku); bez animacii `None` (skrati sa).
    fn peek_scroll_ms(&self) -> Option<transom_core::item::Scroll> {
        // rovnako ako polozka Music: jej zapnutie, rychlost a smer; bez animacii / OLED skratene "..."
        if !self.config.media.marquee
            || !transom_platform::system::animations_enabled()
            || self.config.oled.enabled
        {
            return None;
        }
        let t0 = self.island.peek_t0?;
        let (speed, rtl) = self.marquee_motion();
        Some(transom_core::item::Scroll {
            ms: t0.elapsed().as_millis().min(u32::MAX as u128) as u32,
            speed_lp: speed.round().clamp(1.0, 500.0) as u16,
            rtl,
        })
    }

    /// Snimka posuvu nazvu: prekresli len ostrov (najviac ~60 snimok za sekundu).
    fn peek_frame(&mut self) {
        for (m, placed) in &self.layouts {
            if self.hidden_bars.contains(m) {
                continue;
            }
            if let Some(p) = placed.iter().find(|p| p.item == ItemId::Shelf) {
                transom_platform::invalidate_span(m, p.x, p.w);
            }
        }
    }

    /// Farba ekvalizera z obalu (pocita sa raz na obal).
    fn eq_color(&self) -> Option<(u8, u8, u8)> {
        let art = self.art.as_ref()?;
        if let Some((id, c)) = self.island.eq_color.get() {
            if id == art.id {
                return c;
            }
        }
        let c = cover_color(&art.image, self.theme.dark);
        self.island.eq_color.set(Some((art.id, c)));
        c
    }

    /// Tik ostrova: konce oznamenia / nazvu skladby / aktivity a testovaci klik.
    pub(crate) fn island_tick(&mut self) {
        self.island.due = None;
        if self.island.test_tools.is_some_and(|t| Instant::now() >= t) {
            self.island.test_tools = None;
            let at = self.island_anchor();
            let action = if crate::flag("--test-tools-clip") {
                Action::ToolsClipboard
            } else if crate::flag("--test-tools-timer") {
                Action::TimerPanel
            } else if crate::flag("--test-tools-ai") {
                Action::AiPanel
            } else {
                Action::ToolsPanel
            };
            let _ = self.act(action, at);
        }
        if self.island.test_click.is_some_and(|t| Instant::now() >= t) {
            self.island.test_click = None;
            log("ostrov: testovaci klik");
            let at = self.island_anchor();
            self.island_click(at);
            // test: konkretna karta (`--test-island-tab ai`)
            let tab = match crate::arg("--test-island-tab").as_deref() {
                Some("ai") => Some(crate::island_menu::Tab::Ai),
                Some("shots") => Some(crate::island_menu::Tab::Shots),
                Some("files") => Some(crate::island_menu::Tab::Files),
                Some("music") => Some(crate::island_menu::Tab::Music),
                _ => None,
            };
            if let Some(tab) = tab {
                let at = self.island_anchor();
                self.island_menu_open(tab, &at);
            }
        }
        self.island_sync();
    }

    /// Stred ostrova v liste pod kurzorom (kotva panelov), inak v prvej liste.
    fn island_anchor(&self) -> Option<(String, i32)> {
        let under = transom_platform::appbar::bar_center_under_cursor().map(|(m, _)| m);
        let find = |m: &String| {
            self.layouts.get(m).and_then(|l| {
                l.iter()
                    .find(|p| p.item == ItemId::Shelf)
                    .map(|p| (m.clone(), p.x + p.w / 2))
            })
        };
        under
            .as_ref()
            .and_then(find)
            .or_else(|| self.layouts.keys().find_map(find))
    }

    /// Cast listy s ostrovom: v strede vzdy logo a nazov transom, pocas aktivity jej obsah vlavo a vpravo.
    pub(crate) fn island_part(&self) -> Part {
        let text = |text: &str, muted: bool, strong: bool, low: bool| Seg::Text {
            scroll: None,
            text: text.to_string(),
            muted,
            strong,
            low,
        };
        let (left, right, name) = match self.island.shown.clone() {
            None => (Seg::None, Seg::None, t("island.name")),
            Some(Compact::Notice(no)) => (
                text(&no.title, false, false, false),
                text(&no.sub, true, false, false),
                format!("{}: {}", no.title, no.sub),
            ),
            Some(Compact::Ai(a)) if !a.state.busy() => (
                Seg::None,
                Seg::Logo {
                    logo: ai_logo(&a.name),
                    text: String::new(),
                    low: false,
                    dot: Some(status_dot(a.state, self.theme.dark)),
                },
                format!("{} · {}", a.name, t(state_key(a.state))),
            ),
            Some(Compact::Ai(a)) => (
                Seg::Logo {
                    logo: ai_logo(&a.name),
                    text: String::new(),
                    low: false,
                    dot: Some(status_dot(a.state, self.theme.dark)),
                },
                text(
                    &usage_text(&a),
                    !a.alert && a.value.is_none(),
                    a.alert,
                    a.alert,
                ),
                format!("{} · {}", a.name, usage_text(&a)),
            ),
            Some(Compact::MusicAi(m, a)) => (
                Seg::Cover {
                    art: self.art.clone(),
                    text: if m.artist.is_empty() {
                        m.title.clone()
                    } else {
                        format!("{} – {}", m.title, m.artist)
                    },
                    scroll: self.peek_scroll_ms(),
                },
                Seg::Logo {
                    logo: ai_logo(&a.name),
                    text: usage_text(&a),
                    low: a.alert,
                    dot: Some(status_dot(a.state, self.theme.dark)),
                },
                format!(
                    "{}: {} – {}; {} · {}",
                    m.app,
                    m.title,
                    m.artist,
                    a.name,
                    usage_text(&a)
                ),
            ),
            Some(Compact::Peek(m)) => (
                Seg::Cover {
                    art: self.art.clone(),
                    text: m.title.clone(),
                    scroll: self.peek_scroll_ms(),
                },
                Seg::Text {
                    text: m.artist.clone(),
                    muted: true,
                    strong: false,
                    low: false,
                    scroll: self.peek_scroll_ms(),
                },
                format!("{}: {} – {}", m.app, m.title, m.artist),
            ),
            Some(Compact::Music(m)) => {
                let levels = if self.island.spectrum.is_some() {
                    self.island.eq.map(|v| v.round().clamp(0.0, 255.0) as u8)
                } else {
                    // bez pohybu (obmedzene animacie, OLED): pokojne stlpce
                    [90, 140, 110, 70, 50]
                };
                (
                    Seg::Art(self.art.clone()),
                    Seg::Eq {
                        levels,
                        color: self.eq_color(),
                        playing: m.playing,
                    },
                    format!("{}: {} – {}", m.app, m.title, m.artist),
                )
            }
        };
        Part::Island {
            left,
            mark: Mark::Transom,
            label: "transom".into(),
            badge: String::new(),
            right,
            name,
            action: Some(Action::Island),
        }
    }

    /// Klik na ostrov: koniec casovaca / udalost otvoria svoj panel, inak menu ostrova so zalozkou podla aktivity.
    pub(crate) fn island_click(&mut self, at: Option<(String, i32)>) {
        let at = at.or_else(|| self.island_anchor());
        // druhy klik na ostrov pri otvorenom menu ho zavrie (oznamenie sa vsak vzdy otvori na svojej karte)
        let notice = matches!(self.island.shown, Some(Compact::Notice(_)));
        if !notice
            && self.notch_view == NotchView::Island
            && transom_platform::dragdrop::notch_browsing()
        {
            log("ostrov: menu zavrete klikom");
            transom_platform::dragdrop::notch_collapse();
            return self.island_sync();
        }
        let want = match self.island.shown.clone() {
            Some(Compact::Notice(n)) => {
                self.island.model.clear_notice();
                match n.act {
                    Some(NoticeAct::Timer) => {
                        let _ = self.act(Action::TimerPanel, at);
                        return self.island_sync();
                    }
                    Some(NoticeAct::Calendar) => {
                        let _ = self.act(Action::Calendar, at);
                        return self.island_sync();
                    }
                    Some(NoticeAct::Shot(_)) => Tab::Shots,
                    Some(NoticeAct::Clipboard) => Tab::Clipboard,
                    Some(NoticeAct::Stash) | None => Tab::Files,
                    Some(NoticeAct::Ai) => Tab::Ai,
                }
            }
            Some(Compact::Ai(_)) => Tab::Ai,
            Some(Compact::Peek(_) | Compact::Music(_) | Compact::MusicAi(..)) => Tab::Music,
            None if self.chosen_session().is_some() => Tab::Music,
            None => Tab::Files,
        };
        self.island_menu_open(want, &at);
        self.island_sync();
    }

    /// Otvoreny vybezok ukazuje prehravac (karta Hudba alebo vybezok medii).
    fn music_card_open(&self) -> bool {
        dragdrop::notch_browsing()
            && (self.notch_view == NotchView::Media
                || (self.notch_view == NotchView::Island
                    && self.island_menu.tab == Some(Tab::Music)))
    }

    /// Tik prehravaca: cerstva casova os (po presune aj z prehravaca) a prekreslenie casu.
    pub(crate) fn media_tick(&mut self) {
        self.island.media_tick = false;
        if !self.music_card_open() {
            return;
        }
        if let Some(m) = &self.media {
            // snimka z predosleho citania (o 0,5 s staria) a hned nove citanie na dalsi tik
            self.snapshot = m.snapshot();
            m.refresh();
        }
        dragdrop::invalidate_notch();
        self.island.media_tick = true;
        schedule_timer(crate::T_MEDIA, 500);
    }

    /// Menu ostrova ukazuje kartu Snimky (kniznica snimok ma bezat).
    pub(crate) fn island_shots_open(&self) -> bool {
        dragdrop::notch_browsing()
            && self.notch_view == NotchView::Island
            && self.island_menu.tab == Some(Tab::Shots)
    }

    /// Dostupne zalozky menu (podla toho, co je zapnute).
    fn island_tabs(&self) -> Vec<Tab> {
        let mut v = Vec::new();
        if self.chosen_session().is_some() {
            v.push(Tab::Music);
        }
        if self.tools_snap.clip_enabled {
            v.push(Tab::Clipboard);
        }
        if transom_platform::shellops::screenshots_folder().is_some() {
            v.push(Tab::Shots);
        }
        if self.config.usage.any_enabled() {
            v.push(Tab::Ai);
        }
        if self.config.tools.shelf.enabled {
            v.push(Tab::Files);
        }
        v
    }

    /// Otvori menu ostrova pod stredom listy.
    fn island_menu_open(&mut self, want: Tab, at: &Option<(String, i32)>) {
        let tabs = self.island_tabs();
        if tabs.is_empty() {
            return;
        }
        self.island_menu.open(tabs, want);
        self.island_menu.anchor = at
            .as_ref()
            .and_then(|(m, x)| transom_platform::appbar::bar_screen_x(m, *x));
        self.island_menu_data();
        self.media_notch.hover = None;
        self.clip_notch.hover = None;
        self.stash.hover = None;
        self.notch_view = NotchView::Island;
        log("ostrov: menu");
        dragdrop::notch_browse_at(
            self.island_menu.anchor,
            Some(crate::island_menu::height_lp(self.island_menu.tab)),
        );
    }

    /// Kreslenie menu ostrova: zalozky a obsah zvolenej karty.
    pub(crate) fn paint_island_menu(&mut self, p: &mut dyn transom_render::Painter) {
        let th = self.theme;
        let area = self.island_menu.paint_frame(p, &th);
        let Some(tab) = self.island_menu.tab else {
            return;
        };
        match tab {
            Tab::Shots => {
                // miniatury z kniznice snimok (ta ista ako v Nastrojoch; nacitava sa na pozadi)
                let cfg = self.config.tools.screenshots.clone();
                let box_px = self.island_menu.thumb_box;
                self.island_menu.thumbs = match (&mut self.library, cfg.enabled) {
                    (Some(rt), true) => Some(rt.section(&cfg, box_px).rows),
                    _ => None,
                };
                self.island_menu.paint_shots(p, &th, area);
                if let Some(rt) = &self.library {
                    let wanted = std::mem::take(&mut self.island_menu.wanted);
                    if !wanted.is_empty() {
                        rt.request(&wanted, self.island_menu.thumb_box);
                    }
                }
            }
            Tab::Ai => self.island_menu.paint_ai(p, &th, area),
            _ => {
                let mut sub = transom_render::painter::Sub {
                    inner: p,
                    dx: area.x,
                    dy: area.y,
                    size: transom_render::Size {
                        w: area.w,
                        h: area.h,
                    },
                };
                match tab {
                    Tab::Music => {
                        if let Some(info) = self.chosen_session().cloned() {
                            let art = self.art.clone();
                            self.media_notch.paint(&mut sub, &th, &info, art.as_ref());
                        }
                    }
                    Tab::Clipboard => {
                        let rows = self.tools_snap.clipboard.clone();
                        self.clip_notch.paint(&mut sub, &th, &rows);
                    }
                    _ => {
                        let (files, total) = self.stash_files();
                        self.stash.paint(&mut sub, &th, &files, total);
                    }
                }
            }
        }
    }

    /// Prikaz z karty Snimky / AI.
    pub(crate) fn island_menu_cmd(&mut self, c: crate::island_menu::MenuCmd) {
        use crate::island_menu::MenuCmd;
        match c {
            MenuCmd::Shot(p) => {
                log("ostrov: snimka v priecinku");
                let _ = transom_platform::shellops::open_containing(&p.to_string_lossy());
            }
            MenuCmd::ShotDrag(i) => {
                // snimku zo zoznamu kniznice vytiahnut do inej aplikacie (kopia / odkaz, nic sa nepresuva)
                if let Some(e) = self.library.as_ref().and_then(|rt| rt.entry(i)) {
                    log("ostrov: tahanie snimky");
                    popup::request(transom_platform::popup::Request::DragOut(vec![e.path]));
                }
            }
            MenuCmd::ShotAt(i) => {
                log("ostrov: otvorit snimku");
                self.open_shot(i, false);
            }
            MenuCmd::ShotsFolder => {
                if let Some(d) = transom_platform::shellops::screenshots_folder() {
                    let _ = transom_platform::shellops::open_target(&d);
                }
            }
            MenuCmd::AiPanel => {
                let at = self.island_anchor();
                let _ = self.act(Action::AiPanel, at);
            }
        }
    }

    /// Udaje pre zalozky Snimky a AI (citaju sa pri otvoreni a prepnuti zalozky).
    pub(crate) fn island_menu_data(&mut self) {
        self.library_sync();
        match self.island_menu.tab {
            Some(Tab::Shots) if self.config.tools.screenshots.enabled => {}
            Some(Tab::Shots) => {
                let dir = transom_platform::shellops::screenshots_folder().map(PathBuf::from);
                self.island_menu.shots = dir.as_deref().map(recent_images).unwrap_or_default();
            }
            Some(Tab::Ai) => self.island_menu.limits = self.ai_limits(),
            _ => {}
        }
    }

    /// Limity AI zo zberu (5 h, tyzden...) pre menu: hodnota, reset.
    fn ai_limits(&self) -> Vec<Limit> {
        use transom_usage::limits;
        let now = crate::ai::now_unix();
        let plans = self
            .ai
            .data
            .as_ref()
            .map_or(&[][..], |d| d.plans.as_slice());
        crate::ai::enabled_providers(&self.config.usage)
            .into_iter()
            .flat_map(|p| {
                limits::provider_windows(plans, p, now, crate::ai::LIMIT_FRESH_S)
                    .into_iter()
                    .map(move |r| (p, r))
            })
            .map(|(p, r)| {
                let used = r.used();
                let mut detail = crate::ai::provider_short(p).to_string();
                if let limits::ResetState::In { secs, .. } = r.reset(now) {
                    detail.push_str(&format!(
                        " · {}",
                        transom_i18n::tf("reset.in", &[&crate::ai::format_duration(secs)])
                    ));
                }
                Limit {
                    window: crate::ai::window_name(r.minutes),
                    used,
                    detail,
                    low: 100 - used <= crate::ai_cards::LOW_REMAINING_PCT,
                }
            })
            .collect()
    }
}

impl App {
    /// LEN test vzhladu: `--sim-ai 34,91` (5 h %, tyzden %) = AI prave pracuje (90 s); `--test-island timer|shot|clip|
    /// event|stash` = jedno oznamenie po starte. Nic sa nezapisuje do suborov pouzivatela.
    pub(crate) fn island_test_start(&mut self) {
        if let Some(v) = crate::arg("--sim-ai") {
            let mut it = v.split(',').map(|x| x.trim().parse::<f64>().ok());
            let (r5, r7) = (it.next().flatten(), it.next().flatten());
            self.island.ai = Some((
                SystemTime::now(),
                transom_usage::statusline::LastLimits {
                    ts: crate::ai::now_unix(),
                    five: r5,
                    week: r7,
                    ..Default::default()
                },
            ));
        }
        if let Some(v) = crate::arg("--sim-codex") {
            // LEN test vzhladu: Codex prave pracuje (5 h %, tyzden %)
            let mut it = v.split(',').map(|x| x.trim().parse::<f64>().ok());
            let (a, b) = (it.next().flatten(), it.next().flatten());
            let now_s = crate::ai::now_unix();
            let win = |m: u32, u: Option<f64>| {
                u.map(|u| transom_usage::LimitWindow {
                    minutes: m,
                    used_percent: u,
                    resets_at: None,
                })
            };
            self.island.codex = Some((
                SystemTime::now(),
                Some(transom_usage::PlanSnapshot {
                    windows: [win(300, a), win(10080, b)].into_iter().flatten().collect(),
                    recorded_at: now_s,
                    ..Default::default()
                }),
            ));
            self.island.codex_check = Some(Instant::now() + std::time::Duration::from_secs(3600));
        }
        if crate::flag("--test-open-tools") {
            // LEN test vzhladu: po starte otvorit Nastroje pod stredom listy
            self.island.test_tools = Some(Instant::now() + std::time::Duration::from_millis(1500));
        }
        if crate::flag("--test-island-click") {
            self.island.test_click = Some(Instant::now() + std::time::Duration::from_millis(1500));
        }
        let Some(kind) = crate::arg("--test-island") else {
            self.island_sync();
            return;
        };
        let g = transom_render::scene::glyph_char;
        let n = match kind.as_str() {
            "shot" => Notice {
                icon: g(Glyph::Screenshot),
                title: t("island.shot"),
                sub: t("island.shot_sub"),
                act: Some(NoticeAct::Shot(PathBuf::new())),
            },
            "clip" => Notice {
                icon: g(Glyph::Clipboard),
                title: t("island.clip"),
                sub: t("island.clip_sub"),
                act: Some(NoticeAct::Clipboard),
            },
            "event" => Notice {
                icon: '\u{E787}',
                title: "Test".into(),
                sub: transom_i18n::tf(
                    "island.cal_sub",
                    &[&transom_i18n::tf("notch.event_in", &["5"])],
                ),
                act: Some(NoticeAct::Calendar),
            },
            "stash" => Notice {
                icon: g(Glyph::Snip),
                title: t("snip.done"),
                sub: t("island.stash_sub"),
                act: Some(NoticeAct::Stash),
            },
            _ => Notice {
                icon: g(Glyph::Timer),
                title: crate::timer::finished_text(crate::timer::Finished::Countdown),
                sub: t("island.timer_sub"),
                act: Some(NoticeAct::Timer),
            },
        };
        self.island_notice(n);
    }
}

/// Farba stavovej bodky AI: pracuje (zelena) / caka na pouzivatela (oranzova).
fn status_dot(state: AiState, dark: bool) -> (u8, u8, u8) {
    // farby klavesnice Codex Agent Keys: necinny biela, neprecitane zelena, premysla modra, povolenie / otazka
    // broskynova, chyba cervena
    match state {
        // na svetlej liste by biela zanikla: siva
        AiState::Idle if !dark => (120, 120, 124),
        AiState::Idle => (255, 255, 255),
        AiState::Unread => (155, 243, 150),
        AiState::Thinking => (156, 213, 254),
        AiState::Waiting => (255, 208, 184),
        AiState::Error => (255, 115, 115),
    }
}

/// Kluc nazvu stavu (pristupny nazov ostrova).
fn state_key(state: AiState) -> &'static str {
    match state {
        AiState::Idle => "island.st_idle",
        AiState::Thinking => "island.st_thinking",
        AiState::Waiting => "island.st_waiting",
        AiState::Unread => "island.st_unread",
        AiState::Error => "island.st_error",
    }
}

/// Logo podla nazvu AI.
fn ai_logo(name: &str) -> transom_core::item::AiLogo {
    if name == "Codex" {
        transom_core::item::AiLogo::Codex
    } else {
        transom_core::item::AiLogo::Claude
    }
}

/// "5 h · 34 % využité" (alebo len okno, ked zdroj limit nehlasi).
fn usage_text(a: &AiLive) -> String {
    match &a.value {
        Some(v) if a.window.is_empty() => v.clone(),
        Some(v) => format!("{} · {v}", a.window),
        None => a.window.clone(),
    }
}

/// Kratky nazov aplikacie z AUMID relacie medii ("Spotify.exe" -> "Spotify",
/// "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" -> "ZuneMusic"); nerozpoznatelny = "Hudba".
pub fn app_name(aumid: &str) -> String {
    let s = aumid.split('!').next().unwrap_or("");
    let s = s.split('_').next().unwrap_or("");
    let s = s
        .strip_suffix(".exe")
        .or_else(|| s.strip_suffix(".EXE"))
        .unwrap_or(s);
    let s = s.rsplit(['.', '\\', '/']).next().unwrap_or("").trim();
    // hash namiesto nazvu (napr. Firefox) alebo nic
    if s.is_empty() || (s.len() >= 12 && s.chars().all(|c| c.is_ascii_hexdigit())) {
        return t("island.music");
    }
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// Farba ekvalizera z obalu: priemer sytych pixelov (sive a tmave sa preskakuju), zosvetlena / stmavena podla
/// listy. `None` = obal je sivy (pouzije sa akcent).
pub fn cover_color(img: &transom_core::image::Image, dark: bool) -> Option<(u8, u8, u8)> {
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    let step = (img.px.len() / 4096).max(1);
    for p in img.px.iter().step_by(step) {
        let (pr, pg, pb) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
        let (mx, mn) = (pr.max(pg).max(pb), pr.min(pg).min(pb));
        // sytost a jas: len farebne pixely
        if mx < 50 || mx - mn < 40 {
            continue;
        }
        let w = (mx - mn) as u64;
        r += pr as u64 * w;
        g += pg as u64 * w;
        b += pb as u64 * w;
        n += w;
    }
    if n == 0 {
        return None;
    }
    let (mut fr, mut fg, mut fb) = ((r / n) as f32, (g / n) as f32, (b / n) as f32);
    // citatelnost na liste: na tmavej aspon jas ~170, na svetlej najviac ~120
    let lum = 0.2126 * fr + 0.7152 * fg + 0.0722 * fb;
    let want = if dark { lum.max(170.0) } else { lum.min(120.0) };
    if lum > 1.0 {
        let k = want / lum;
        fr = (fr * k).min(255.0);
        fg = (fg * k).min(255.0);
        fb = (fb * k).min(255.0);
    }
    // sytá farba narazila na 255 a je stale tmava: primiesat bielu (na svetlej liste ubrat) az do ciela
    let lum2 = 0.2126 * fr + 0.7152 * fg + 0.0722 * fb;
    if dark && lum2 < want {
        let t = (want - lum2) / (255.0 - lum2).max(1.0);
        fr += (255.0 - fr) * t;
        fg += (255.0 - fg) * t;
        fb += (255.0 - fb) * t;
    } else if !dark && lum2 > want {
        let t = (lum2 - want) / lum2.max(1.0);
        fr -= fr * t;
        fg -= fg * t;
        fb -= fb * t;
    }
    Some((fr as u8, fg as u8, fb as u8))
}

/// Najnovsie obrazky v priecinku (najviac 6) s vekom; nazvy sa nikde nelogu.
fn recent_images(dir: &std::path::Path) -> Vec<Shot> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(PathBuf, SystemTime)> = rd
        .flatten()
        .filter(|e| is_image(&e.path()))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then_some(())?;
            Some((e.path(), m.created().or_else(|_| m.modified()).ok()?))
        })
        .collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    let now = SystemTime::now();
    v.into_iter()
        .take(6)
        .map(|(p, t)| Shot {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            when: crate::island_menu::ago(now.duration_since(t).map_or(0, |d| d.as_secs())),
            path: p,
        })
        .collect()
}

fn is_image(p: &std::path::Path) -> bool {
    p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
        matches!(
            x.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"
        )
    })
}

/// Najnovsi obrazok v priecinku (cesta, cas zmeny); nazvy sa nikde nelogu.
fn newest_image(dir: &std::path::Path) -> Option<(PathBuf, SystemTime)> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| {
                    matches!(
                        x.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"
                    )
                })
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then_some(())?;
            // cas vzniku: upraveny starsi obrazok sa znova neohlasi
            Some((e.path(), m.created().or_else(|_| m.modified()).ok()?))
        })
        .max_by_key(|(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_colour_skips_grey_and_keeps_contrast() {
        let grey = transom_core::image::Image {
            w: 2,
            h: 1,
            px: vec![0xFF80_8080, 0xFF20_2020],
        };
        assert_eq!(cover_color(&grey, true), None);
        let red = transom_core::image::Image {
            w: 2,
            h: 1,
            px: vec![0xFF80_1010, 0xFF80_8080],
        };
        let (r, g, b) = cover_color(&red, true).unwrap();
        assert!(r > g && r > b);
        let lum = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
        assert!(lum >= 160.0, "na tmavej liste svetla: {lum}");
    }

    #[test]
    fn app_names_from_media_session_ids() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("Brave"), "Brave");
        assert_eq!(
            app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(app_name("308046B0AF4A39CB"), t("island.music"));
        assert_eq!(app_name(""), t("island.music"));
    }

    #[test]
    fn newest_image_ignores_other_files_and_folders() {
        let d = std::env::temp_dir().join(format!("transom-island-shots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub.png")).unwrap();
        assert_eq!(newest_image(&d), None);
        std::fs::write(d.join("a.PNG"), b"x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        assert_eq!(newest_image(&d).map(|(p, _)| p), Some(d.join("a.PNG")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
