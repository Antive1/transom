//! Nove polozky lisaty s jednoduchymi panelmi (Casovac, Jas, Mikrofon a kamera) - cast `App`, ktora ich riadi.
//! Panely kresli spolocna sada `pop_kit`; tu sa sklada ich obsah (`Row`) a vykonavaju prikazy.
use crate::pop_kit::{Btn, Cmd, Row};
use crate::timer::{self, Countdown, Finished, PomoCfg, MIN};
use crate::{
    log, notifier, popup, schedule_timer, App, HOTKEY_MIC, N_BRIGHT, N_PRIV, N_SYSMON, POP_BRIGHT,
    POP_COLORS, POP_EDIT, POP_PRIVACY, POP_TIMER, POP_UPDATE, T_TIMER,
};
use transom_i18n::{t, tf};
use transom_mixer::capture::{PrivService, PrivSnap};
use transom_platform::brightness;

/// Sirka panelov zo sady.
pub const KIT_W_LP: i32 = 340;

/// Je `kind` panel zo sady?
pub fn is_kit(kind: u32) -> bool {
    matches!(
        kind,
        POP_TIMER | POP_BRIGHT | POP_PRIVACY | POP_EDIT | POP_UPDATE | POP_COLORS
    )
}

/// Vyska panela.
pub fn kit_h(kind: u32) -> i32 {
    match kind {
        POP_TIMER | POP_EDIT => 600,
        _ => 480,
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Cas pre kreslenie panelu: sklenene panely sa kreslia dvakrat za sebou (cierne a biele pozadie, alfa z rozdielu),
/// oba prechody preto musia vidiet ten isty cas - inak by sa pri prechode cez celu sekundu cislice odpoctu v jednom
/// snimku rozisli a ostali "duchovia". Volania do 50 ms od predchadzajuceho dostanu tu istu hodnotu.
pub fn paint_now_ms() -> i64 {
    use std::cell::Cell;
    use std::time::Instant;
    thread_local! {
        static LAST: Cell<Option<(Instant, i64)>> = const { Cell::new(None) };
    }
    LAST.with(|c| match c.get() {
        Some((at, v)) if at.elapsed().as_millis() < 50 => v,
        _ => {
            let v = now_ms();
            c.set(Some((Instant::now(), v)));
            v
        }
    })
}

// id ovladacov casovaca
const TM_PRESET: u32 = 100; // + minuty
const TM_TOGGLE: u32 = 1;
const TM_RESET: u32 = 2;
const TM_MINUS: u32 = 3;
const TM_PLUS: u32 = 4;
const TM_POMO: u32 = 5;
const TM_POMO_SKIP: u32 = 6;
const TM_SW_TOGGLE: u32 = 7;
const TM_SW_LAP: u32 = 8;
const TM_SW_RESET: u32 = 9;
const TM_SOUND: u32 = 10;
const PRESETS: [u32; 6] = [1, 5, 10, 15, 30, 60];

// id ovladacov panela Jas (posuvniky = index monitora)
const BR_REFRESH: u32 = 1000;

/// Co ukazuje polozka Jas: jas hlavneho (alebo prveho podporovaneho) monitora.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrightView {
    pub level: Option<u32>,
    pub name: String,
}

pub fn bright_view(s: &brightness::Snapshot) -> BrightView {
    let d = s
        .displays
        .iter()
        .filter(|d| d.level.is_some())
        .max_by_key(|d| d.primary);
    match d {
        Some(d) => {
            let l = d.level.unwrap_or(0);
            BrightView {
                level: Some(l),
                name: tf("br.name", &[&l.to_string(), &display_name(d)]),
            }
        }
        None => BrightView {
            level: None,
            name: t("item.brightness"),
        },
    }
}

fn display_name(d: &brightness::Display) -> String {
    let n = if d.name.trim().is_empty() {
        t("br.unnamed")
    } else {
        d.name.trim().to_string()
    };
    if d.primary {
        tf("br.primary", &[&n])
    } else {
        n
    }
}

/// Obsah panela Jas.
pub fn bright_rows(s: &brightness::Snapshot) -> Vec<Row> {
    let mut rows = vec![Row::Title(t("br.title"))];
    if !s.ready {
        rows.push(Row::Text {
            text: t("br.loading"),
            strong: false,
        });
        return rows;
    }
    if s.displays.is_empty() {
        rows.push(Row::Text {
            text: t("br.none"),
            strong: false,
        });
    }
    for (i, d) in s.displays.iter().enumerate() {
        let name = display_name(d);
        rows.push(Row::Section(name.clone()));
        match d.level {
            Some(l) => rows.push(Row::Slider {
                id: i as u32,
                label: tf("br.slider", &[&name]),
                value: l,
                enabled: true,
                glyph: Some(transom_core::item::Glyph::Brightness),
            }),
            None => rows.push(Row::Text {
                text: t("br.unsupported"),
                strong: false,
            }),
        }
        if s.failed.as_deref() == Some(d.key.as_str()) {
            rows.push(Row::Text {
                text: t("br.failed"),
                strong: true,
            });
        }
    }
    rows.push(Row::Gap(4));
    rows.push(Row::Text {
        text: t("br.note"),
        strong: false,
    });
    rows.push(Row::Buttons(vec![Btn::new(BR_REFRESH, t("br.refresh"))]));
    rows
}

const PV_MUTE: u32 = 1;
const PV_SOUND: u32 = 2;

/// Casti polozky Mikrofon a kamera: (glyf, text vedla ikony, pristupny nazov). Prazdne = polozka sa nezobrazi.
pub fn priv_parts(s: &PrivSnap, always: bool) -> Vec<(transom_core::item::Glyph, String, String)> {
    use transom_core::item::Glyph;
    let first = |v: &[String]| {
        let n: String = v
            .first()
            .map(|x| x.chars().take(14).collect())
            .unwrap_or_default();
        if v.len() > 1 {
            format!("{n} +{}", v.len() - 1)
        } else {
            n
        }
    };
    let mut out = Vec::new();
    if s.mic_muted == Some(true) {
        out.push((Glyph::MicOff, String::new(), t("priv.mic_muted")));
    } else if !s.mic_apps.is_empty() {
        out.push((
            Glyph::Mic,
            first(&s.mic_apps),
            tf("priv.mic_used", &[&s.mic_apps.join(", ")]),
        ));
    }
    if !s.cam_apps.is_empty() {
        out.push((
            Glyph::Camera,
            first(&s.cam_apps),
            tf("priv.cam_used", &[&s.cam_apps.join(", ")]),
        ));
    }
    if out.is_empty() && always {
        out.push((Glyph::Mic, String::new(), t("priv.idle")));
    }
    out
}

/// Obsah panela Mikrofon a kamera.
pub fn priv_rows(s: &PrivSnap, hotkey: Option<String>) -> Vec<Row> {
    use transom_core::item::Glyph;
    let mut rows = vec![Row::Title(t("priv.title"))];
    if !s.ready {
        rows.push(Row::Text {
            text: t("priv.loading"),
            strong: false,
        });
        return rows;
    }
    rows.push(Row::Section(t("priv.mic")));
    if s.mic_present {
        rows.push(Row::Toggle {
            id: PV_MUTE,
            label: t("priv.mute"),
            on: s.mic_muted == Some(true),
        });
    } else {
        rows.push(Row::Text {
            text: t("priv.no_mic"),
            strong: false,
        });
    }
    if s.mic_apps.is_empty() {
        rows.push(Row::Text {
            text: t("priv.mic_free"),
            strong: false,
        });
    }
    for a in &s.mic_apps {
        rows.push(Row::Item {
            glyph: Some(Glyph::Mic),
            text: a.clone(),
            detail: t("priv.in_use"),
            selected: false,
        });
    }
    rows.push(Row::Section(t("priv.cam")));
    if s.cam_apps.is_empty() {
        rows.push(Row::Text {
            text: t("priv.cam_free"),
            strong: false,
        });
    }
    for a in &s.cam_apps {
        rows.push(Row::Item {
            glyph: Some(Glyph::Camera),
            text: a.clone(),
            detail: t("priv.in_use"),
            selected: false,
        });
    }
    rows.push(Row::Gap(6));
    rows.push(Row::Text {
        text: match hotkey {
            Some(h) => tf("priv.hotkey", &[&h]),
            None => t("priv.hotkey_off"),
        },
        strong: false,
    });
    rows.push(Row::Text {
        text: t("priv.note"),
        strong: false,
    });
    rows.push(Row::Buttons(vec![Btn::new(PV_SOUND, t("priv.settings"))]));
    rows
}

/// Zaznamy historie schranky, ktore obsahuju niektory z odkazov kalendara (aj skrateny nahlad a webcal:// tvar).
pub fn clip_ids_with_links(
    rows: &[transom_tools::service::ClipRow],
    links: &[String],
) -> Vec<String> {
    rows.iter()
        .filter(|r| {
            let p = r.preview.trim();
            let cut = p.trim_end_matches('…');
            let norm = transom_config::normalize_ics_url(p);
            let norm_cut = transom_config::normalize_ics_url(cut);
            links.iter().any(|l| {
                norm.as_deref() == Some(l.as_str())
                    || (cut.len() >= 24
                        && cut.len() < p.len()
                        && norm_cut.as_deref().is_some_and(|c| l.starts_with(c)))
                    || p.contains(l.trim_start_matches("https://"))
            })
        })
        .map(|r| r.id.clone())
        .collect()
}

/// Polozka Vytazenie systemu: text (s pevnou sirkou cisel, aby sa lista kazdu sekundu neposuvala) a pristupny nazov.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysView {
    pub text: String,
    pub name: String,
}

/// Cislo zarovnane na `w` znakov cislicovou medzerou (rovnaka sirka ako cislica).
fn pad(n: impl std::fmt::Display, w: usize) -> String {
    let s = n.to_string();
    let k = w.saturating_sub(s.chars().count());
    "\u{2007}".repeat(k) + &s
}

/// Rychlost prenosu: "<1 kB/s", "46 kB/s", "4,6 MB/s" (desatinny oddelovac podla jazyka).
pub fn rate(bps: u64) -> String {
    let kb = bps as f64 / 1000.0;
    let s = if kb < 1.0 {
        t("sys.rate_low")
    } else if kb < 999.5 {
        format!("{} kB/s", kb.round() as u64)
    } else {
        let mb = kb / 1000.0;
        let n = if mb < 99.95 {
            format!("{mb:.1}").replace('.', &t("sys.dec"))
        } else {
            format!("{}", mb.round() as u64)
        };
        format!("{n} MB/s")
    };
    // vzdy 9 znakov (cislicove medzery), aby sa lista neposuvala
    pad(s, 9)
}

pub fn sys_view(s: &transom_platform::sysmon::Sample, c: &transom_config::SysMonCfg) -> SysView {
    let mut parts: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut pct = |on: bool, label: &str, v: Option<u32>, key: &str| {
        if !on {
            return;
        }
        match v {
            Some(v) => {
                parts.push(format!("{label} {} %", pad(v, 3)));
                names.push(tf(key, &[&v.to_string()]));
            }
            None => parts.push(format!("{label} \u{2007}\u{2007}– %")),
        }
    };
    pct(c.cpu, "CPU", s.cpu, "sys.cpu");
    pct(c.ram, "RAM", s.ram, "sys.ram");
    pct(c.gpu, "GPU", s.gpu, "sys.gpu");
    if c.net {
        match (s.down, s.up) {
            (Some(d), Some(u)) => {
                parts.push(format!("↓{} ↑{}", rate(d), rate(u)));
                names.push(tf("sys.net", &[&rate(d), &rate(u)]));
            }
            _ => parts.push("↓– ↑–".into()),
        }
    }
    let mut name = names.join(", ");
    if name.is_empty() {
        name = t("item.sysmon");
    }
    SysView {
        text: parts.join("  "),
        name: tf("sys.name", &[&name]),
    }
}

/// Co ukazuje polozka Casovac v liste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerView {
    /// Text vedla ikony (prazdny = len ikona).
    pub label: String,
    pub muted: bool,
    pub name: String,
}

pub fn pomo_cfg(c: &transom_config::TimerCfg) -> PomoCfg {
    PomoCfg {
        work: c.work_min,
        short_break: c.short_break_min,
        long_break: c.long_break_min,
        rounds: c.rounds,
    }
}

/// Polozka Casovac podla stavu (odpocet ma prednost pred stopkami).
pub fn timer_view(tm: &timer::Timers, now: i64) -> TimerView {
    let kind = |tm: &timer::Timers| match tm.pomodoro {
        Some((p, _)) => timer::phase_name(p),
        None => t("timer.countdown"),
    };
    match tm.countdown {
        Countdown::Running { .. } | Countdown::Paused { .. } => {
            let left = timer::fmt_ms(tm.left(now).unwrap_or(0), true);
            let paused = matches!(tm.countdown, Countdown::Paused { .. });
            TimerView {
                name: tf(
                    if paused {
                        "timer.name_paused"
                    } else {
                        "timer.name_left"
                    },
                    &[&kind(tm), &left],
                ),
                label: left,
                muted: paused,
            }
        }
        Countdown::Done => TimerView {
            label: t("timer.done"),
            muted: false,
            name: tf("timer.name_done", &[&kind(tm)]),
        },
        Countdown::Idle if tm.stopwatch.elapsed(now) > 0 => {
            let e = timer::fmt_ms(tm.stopwatch.elapsed(now), false);
            TimerView {
                name: tf("timer.name_sw", &[&e]),
                label: e,
                muted: !tm.stopwatch.running(),
            }
        }
        Countdown::Idle => TimerView {
            label: String::new(),
            muted: false,
            name: t("timer.name_idle"),
        },
    }
}

/// Obsah panela Casovac.
pub fn timer_rows(tm: &timer::Timers, cfg: &transom_config::TimerCfg, now: i64) -> Vec<Row> {
    let mut rows = vec![Row::Title(t("timer.title"))];
    // odpocet
    let left = tm.left(now);
    let shown = match tm.countdown {
        Countdown::Done => t("timer.done"),
        _ => timer::fmt_ms(left.unwrap_or(tm.preset_ms), true),
    };
    let what = match tm.pomodoro {
        Some((p, done)) => tf(
            "timer.pomo_state",
            &[&timer::phase_name(p), &(done + 1).to_string()],
        ),
        None => t("timer.countdown"),
    };
    rows.push(Row::Section(what.clone()));
    rows.push(Row::Big {
        acc: format!("{what}: {shown}"),
        text: shown,
    });
    let (toggle, icon) = match tm.countdown {
        Countdown::Running { .. } => (t("timer.pause"), '\u{E769}'),
        Countdown::Paused { .. } => (t("timer.resume"), '\u{E768}'),
        _ => (t("timer.start"), '\u{E768}'),
    };
    // ako na hodinkach: vlavo zrusit (sivy kruh), vpravo spustit / pozastavit (akcentovy kruh), v strede +-1 min
    rows.push(Row::Transport(vec![
        Btn::new(TM_RESET, t("timer.reset"))
            .enabled(tm.countdown != Countdown::Idle || tm.pomodoro.is_some()),
        Btn::new(TM_MINUS, "−1 min").acc(t("timer.minus")),
        Btn::new(TM_PLUS, "+1 min").acc(t("timer.plus")),
        Btn::new(TM_TOGGLE, toggle.clone())
            .primary()
            .icon(icon)
            .acc(toggle),
    ]));
    rows.push(Row::Chips(
        PRESETS
            .iter()
            .map(|m| {
                Btn::new(TM_PRESET + m, tf("timer.min", &[&m.to_string()]))
                    .acc(tf("timer.start_min", &[&m.to_string()]))
            })
            .collect(),
    ));
    // pomodoro
    rows.push(Row::Section(t("timer.pomodoro")));
    rows.push(Row::Text {
        text: tf(
            "timer.pomo_help",
            &[
                &cfg.work_min.to_string(),
                &cfg.short_break_min.to_string(),
                &cfg.rounds.to_string(),
                &cfg.long_break_min.to_string(),
            ],
        ),
        strong: false,
    });
    let pomo_label = match tm.pomodoro {
        Some(_) if tm.countdown == Countdown::Done => t("timer.pomo_next"),
        Some(_) => t("timer.pomo_restart"),
        None => t("timer.pomo_start"),
    };
    rows.push(Row::Buttons(vec![
        Btn::new(TM_POMO, pomo_label),
        Btn::new(TM_POMO_SKIP, t("timer.pomo_skip")).enabled(
            tm.pomodoro.is_some()
                && matches!(
                    tm.countdown,
                    Countdown::Running { .. } | Countdown::Paused { .. }
                ),
        ),
    ]));
    // stopky
    rows.push(Row::Section(t("timer.stopwatch")));
    let e = timer::fmt_ms(tm.stopwatch.elapsed(now), false);
    rows.push(Row::Text {
        text: e,
        strong: true,
    });
    rows.push(Row::Buttons(vec![
        Btn::new(
            TM_SW_TOGGLE,
            if tm.stopwatch.running() {
                t("timer.pause")
            } else if tm.stopwatch.elapsed(now) > 0 {
                t("timer.resume")
            } else {
                t("timer.start")
            },
        )
        .acc(t("timer.sw_toggle")),
        Btn::new(TM_SW_LAP, t("timer.lap")).enabled(tm.stopwatch.running()),
        Btn::new(TM_SW_RESET, t("timer.reset"))
            .enabled(tm.stopwatch.elapsed(now) > 0)
            .acc(t("timer.sw_reset")),
    ]));
    let n = tm.stopwatch.laps.len();
    for (i, lap) in tm.stopwatch.laps.iter().enumerate().rev().take(5) {
        let prev = if i == 0 { 0 } else { tm.stopwatch.laps[i - 1] };
        rows.push(Row::Item {
            glyph: None,
            text: tf("timer.lap_n", &[&(i + 1).to_string()]),
            detail: format!(
                "{}  (+{})",
                timer::fmt_ms(*lap, false),
                timer::fmt_ms(lap - prev, false)
            ),
            selected: i + 1 == n,
        });
    }
    rows.push(Row::Gap(6));
    rows.push(Row::Toggle {
        id: TM_SOUND,
        label: t("timer.sound"),
        on: cfg.sound,
    });
    rows
}

/// Kluc aplikacie v profile (cesta .exe malymi pismenami, systemove zvuky "system"); aplikacia bez cesty sa neuklada.
pub fn profile_key(a: &transom_mixer::App) -> Option<String> {
    if a.system {
        Some("system".into())
    } else {
        a.exe.as_ref().map(|e| e.to_lowercase())
    }
}

/// Profil zo stavu mixera.
pub fn profile_from(slot: &str, m: &transom_mixer::Snapshot) -> transom_config::AudioProfile {
    transom_config::AudioProfile {
        slot: slot.into(),
        apps: m
            .apps
            .iter()
            .filter_map(|a| {
                Some(transom_config::ProfileApp {
                    exe: profile_key(a)?,
                    volume: a.volume,
                    muted: a.muted,
                    device: match &a.route {
                        transom_mixer::Route::Default => None,
                        transom_mixer::Route::Device(d) => Some(d.clone()),
                    },
                })
            })
            .collect(),
    }
}

/// Krok profilu: (kluc aplikacie, nova hlasitost, nove stlmenie, novy vystup - `Some(None)` = predvoleny).
pub type ProfileStep = (u64, Option<u32>, Option<bool>, Option<Option<String>>);

/// Co treba zmenit, aby spustene aplikacie zodpovedali profilu: (kluc aplikacie, hlasitost, stlmenie, vystup).
/// Vystup sa meni len na zariadenie, ktore je teraz pripojene (inak ostane); aplikacie mimo profilu sa nemenia.
pub fn profile_plan(
    p: &transom_config::AudioProfile,
    m: &transom_mixer::Snapshot,
) -> Vec<ProfileStep> {
    let mut out = Vec::new();
    for a in &m.apps {
        let Some(k) = profile_key(a) else { continue };
        let Some(e) = p.apps.iter().find(|e| e.exe == k) else {
            continue;
        };
        let vol = (a.volume != e.volume).then_some(u32::from(e.volume));
        let mute = (a.muted != e.muted).then_some(e.muted);
        let want = match &e.device {
            Some(d) if m.devices.iter().any(|x| &x.id == d) => {
                transom_mixer::Route::Device(d.clone())
            }
            Some(_) => a.route.clone(),
            None => transom_mixer::Route::Default,
        };
        let target = match &want {
            transom_mixer::Route::Default => None,
            transom_mixer::Route::Device(d) => Some(d.clone()),
        };
        let route = (m.routing && want != a.route).then_some(target);
        out.push((a.key, vol, mute, route));
    }
    out
}

impl App {
    fn profile_slot(i: u8) -> Option<&'static str> {
        transom_config::AUDIO_PROFILE_SLOTS
            .get(usize::from(i))
            .copied()
    }

    /// Ulozi aktualne hlasitosti, stlmenie a vystupy spustenych aplikacii do profilu.
    pub(crate) fn profile_save(&mut self, i: u8) {
        let (Some(slot), Some(m)) = (Self::profile_slot(i), &self.mixer) else {
            return;
        };
        let snap = m.snapshot();
        if !snap.ready {
            return;
        }
        let p = profile_from(slot, &snap);
        let n = p.apps.len();
        self.save_config_with(|c| {
            c.audio_profiles.retain(|x| x.slot != p.slot);
            c.audio_profiles.push(p.clone());
        });
        log(&format!("zvukovy profil {slot}: ulozenych {n} aplikacii"));
        self.audio_panel.msg = Some(tf(
            "prof.saved",
            &[&t(&format!("prof.{slot}")), &n.to_string()],
        ));
    }

    /// Pouzije profil na spustene aplikacie (len rozdiely; vystup len na pripojene zariadenie).
    pub(crate) fn profile_apply(&mut self, i: u8) {
        let Some(slot) = Self::profile_slot(i) else {
            return;
        };
        let Some(p) = self
            .config
            .audio_profiles
            .iter()
            .find(|p| p.slot == slot)
            .cloned()
        else {
            return;
        };
        let Some(m) = &self.mixer else { return };
        let snap = m.snapshot();
        let plan = profile_plan(&p, &snap);
        for (k, vol, mute, route) in &plan {
            if let Some(v) = vol {
                m.set_volume(*k, *v);
            }
            if let Some(mu) = mute {
                m.set_mute(*k, *mu);
            }
            if let Some(r) = route {
                m.set_route(*k, r.clone());
            }
        }
        log(&format!(
            "zvukovy profil {slot}: pouzity na {} aplikacii",
            plan.len()
        ));
        self.audio_panel.msg = Some(tf(
            "prof.applied",
            &[&t(&format!("prof.{slot}")), &plan.len().to_string()],
        ));
    }

    pub(crate) fn profile_delete(&mut self, i: u8) {
        let Some(slot) = Self::profile_slot(i) else {
            return;
        };
        self.save_config_with(|c| c.audio_profiles.retain(|x| x.slot != slot));
        self.audio_panel.msg = Some(tf("prof.deleted", &[&t(&format!("prof.{slot}"))]));
    }

    /// Ulozi zmenu konfiguracie (pod zamkom, len toto pole) a hned ju pouzije.
    pub(crate) fn save_config_with(&mut self, f: impl Fn(&mut transom_config::Config)) {
        // pravidlo stredu (odlozene subory) plati pre kazdu zmenu ulozenu z listy
        let g = |c: &mut transom_config::Config| {
            f(c);
            c.enforce_center();
        };
        match transom_config::update(&self.config_path, g) {
            Ok(_) => transom_platform::post_broadcast(transom_config::CHANGED_MESSAGE, 0),
            Err(e) => log(&format!("konfiguracia sa neulozila: {e}")),
        }
        g(&mut self.config);
    }

    /// Panel zo sady sa otvara: pripravit data.
    pub(crate) fn kit_open(&mut self, kind: u32) {
        self.kit.reset();
        if kind == POP_PRIVACY {
            self.sync_privacy(true);
            if let Some(p) = &self.privacy {
                p.refresh();
            }
        }
        if kind == POP_BRIGHT {
            self.sync_bright(true);
            if let Some(b) = &self.bright {
                b.refresh();
            }
        }
    }

    /// Sluzba jasu bezi, kym je polozka Jas viditelna alebo je panel otvoreny.
    pub(crate) fn sync_bright(&mut self, force: bool) {
        let want =
            force || self.item_visible("brightness") || popup::popover_open() == Some(POP_BRIGHT);
        if want && self.bright.is_none() {
            if let Some(n) = notifier() {
                match brightness::Service::start(std::sync::Arc::new(move || n.notify(N_BRIGHT))) {
                    Ok(s) => self.bright = Some(s),
                    Err(e) => log(&format!("jas: sluzbu sa nepodarilo spustit: {e}")),
                }
            }
        } else if !want && self.bright.take().is_some() {
            log("jas: sluzba zastavena");
        }
    }

    /// Merac vytazenia bezi, kym je polozka viditelna (vlastne vlakno, interval z nastaveni).
    pub(crate) fn sync_sysmon(&mut self) {
        use transom_platform::sysmon::{Monitor, Want};
        let c = &self.config.sysmon;
        let want = Want {
            gpu: c.gpu,
            net: c.net,
            interval: std::time::Duration::from_secs(u64::from(c.interval_secs.clamp(1, 10))),
        };
        if !self.item_visible("sysmon") {
            if self.sysmon.take().is_some() {
                log("vytazenie: merac zastaveny");
            }
            return;
        }
        match &self.sysmon {
            Some((m, w)) if *w != want => {
                m.set(want);
                self.sysmon = self.sysmon.take().map(|(m, _)| (m, want));
            }
            Some(_) => {}
            None => {
                if let Some(n) = notifier() {
                    match Monitor::start(want, std::sync::Arc::new(move || n.notify(N_SYSMON))) {
                        Ok(m) => self.sysmon = Some((m, want)),
                        Err(e) => log(&format!("vytazenie: merac sa nespustil: {e}")),
                    }
                }
            }
        }
    }

    /// Sluzba mikrofonu a kamery bezi, kym je polozka viditelna alebo panel otvoreny; skratka stlmenia len pri viditelnej
    /// polozke (aby transom neobsadzal klavesy, ked funkciu nepouzivate). Skratka funguje aj v hre na celu obrazovku.
    pub(crate) fn sync_privacy(&mut self, force: bool) {
        let visible = self.item_visible("privacy");
        let want = force || visible || popup::popover_open() == Some(POP_PRIVACY);
        if want && self.privacy.is_none() {
            if let Some(n) = notifier() {
                match PrivService::start(
                    std::sync::Arc::new(move || n.notify(N_PRIV)),
                    std::time::Duration::from_millis(1500),
                ) {
                    Ok(s) => self.privacy = Some(s),
                    Err(e) => log(&format!("mikrofon: sluzba sa nespustila: {e}")),
                }
            }
        } else if !want && self.privacy.take().is_some() {
            log("mikrofon a kamera: sluzba zastavena");
        }
        // skratka
        let wanted = visible.then(|| self.config.privacy.hotkey()).flatten();
        let label = wanted.map(|h| h.label()).unwrap_or_default();
        if self.mic_hotkey_tried.as_deref() == Some(label.as_str()) {
            return;
        }
        self.mic_hotkey_tried = Some(label.clone());
        if std::mem::take(&mut self.mic_hotkey_ok) {
            transom_platform::hotkey::unregister(HOTKEY_MIC);
        }
        let Some(h) = wanted else { return };
        let keys = transom_platform::hotkey::Keys {
            ctrl: h.ctrl,
            alt: h.alt,
            shift: h.shift,
            win: h.win,
            vk: h.vk,
        };
        match transom_platform::hotkey::register(HOTKEY_MIC, keys) {
            Ok(()) => {
                self.mic_hotkey_ok = true;
                log(&format!("skratka mikrofonu: {label}"));
            }
            Err(code) => {
                log(&format!(
                    "skratka mikrofonu {label}: registracia zlyhala (chyba {code})"
                ));
                self.show_notice(tf("notice.hotkey_taken", &[&label]));
            }
        }
    }

    /// Skratka: prepne stlmenie mikrofonu; oznam ukaze stav podla spatneho citania.
    pub(crate) fn mic_hotkey(&mut self) {
        let snap = self.privacy_snapshot();
        if snap.ready && !snap.mic_present {
            self.show_notice(t("priv.no_mic"));
            return;
        }
        match &self.privacy {
            Some(p) => p.toggle_mute(),
            None => transom_mixer::capture::toggle_mute_once(|_| {}),
        }
        self.mic_notice_pending = true;
    }

    pub(crate) fn privacy_snapshot(&self) -> std::sync::Arc<PrivSnap> {
        self.privacy
            .as_ref()
            .map(|p| p.snapshot())
            .unwrap_or_default()
    }

    /// Nova snimka mikrofonu a kamery: lista, panel a pripadny oznam po skratke.
    pub(crate) fn on_privacy(&mut self) {
        if std::mem::take(&mut self.mic_notice_pending) {
            match self.privacy_snapshot().mic_muted {
                Some(true) => self.show_notice(t("priv.notice_muted")),
                Some(false) => self.show_notice(t("priv.notice_on")),
                None => {}
            }
        }
        crate::invalidate_all();
        if popup::popover_open() == Some(POP_PRIVACY) {
            popup::invalidate_popover();
        }
    }

    /// Pripojene kalendare: synchronizacia bezi, kym je v nastaveniach aspon jeden odkaz.
    pub(crate) fn sync_ics(&mut self) {
        let urls = self.config.ics_calendars.clone();
        if urls.is_empty() {
            if self.ics.take().is_some() {
                log("kalendare ICS: vypnute");
            }
            return;
        }
        match &mut self.ics {
            Some(s) => s.set_urls(urls),
            None => {
                if let Some(n) = notifier() {
                    self.ics = crate::calsync::IcsSync::start(
                        urls,
                        std::sync::Arc::new(move || n.notify(crate::N_ICS)),
                    );
                }
            }
        }
    }

    pub(crate) fn bright_snapshot(&self) -> std::sync::Arc<brightness::Snapshot> {
        self.bright
            .as_ref()
            .map(|b| b.snapshot())
            .unwrap_or_default()
    }

    pub(crate) fn kit_closed(&mut self, kind: u32) {
        // sluzba jasu pre zatvoreny panel nebezi, ak polozka Jas nie je v liste
        if kind == POP_BRIGHT && !self.item_visible("brightness") && self.bright.take().is_some() {
            log("jas: sluzba zastavena");
        }
        if kind == POP_PRIVACY && !self.item_visible("privacy") && self.privacy.take().is_some() {
            log("mikrofon a kamera: sluzba zastavena");
        }
    }

    pub(crate) fn kit_rows(&mut self, kind: u32) -> Vec<Row> {
        match kind {
            POP_TIMER => timer_rows(&self.timers, &self.config.timer, paint_now_ms()),
            POP_BRIGHT => bright_rows(&self.bright_snapshot()),
            POP_EDIT => crate::bar_edit::rows(&self.config.layout, self.config.center_locked()),
            POP_UPDATE => self
                .update_view()
                .map(|v| crate::update_pop::rows(&v))
                .unwrap_or_default(),
            POP_COLORS => self.colors_rows(),
            POP_PRIVACY => priv_rows(
                &self.privacy_snapshot(),
                self.config.privacy.hotkey().map(|h| h.label()),
            ),
            _ => Vec::new(),
        }
    }

    pub(crate) fn paint_kit(&mut self, kind: u32, p: &mut dyn transom_render::Painter) {
        let rows = self.kit_rows(kind);
        let th = self.theme;
        self.kit.paint(p, &th, &rows);
        let (entries, extras, focus, name) = self.kit.a11y();
        popup::set_panel_accessibility(entries, focus);
        transom_platform::menu_access::set_panel_extras(name, extras);
        if std::mem::take(&mut self.kit.repaint) {
            popup::invalidate_popover();
        }
    }

    pub(crate) fn kit_cmd(&mut self, kind: u32, cmd: Cmd) {
        if cmd == Cmd::Click(crate::pop_kit::BACK) {
            self.tools_home();
            return;
        }
        if kind == POP_COLORS {
            self.colors_cmd(cmd);
            return;
        }
        if kind == POP_EDIT {
            let Cmd::Click(id) = cmd else { return };
            match crate::bar_edit::decode(&self.config.layout, id) {
                Some(crate::bar_edit::EditCmd::Done) => {
                    popup::request(transom_platform::popup::Request::ClosePopover);
                }
                Some(c) => {
                    log(&format!("uprava lisaty: {c:?}"));
                    self.save_config_with(|cfg| crate::bar_edit::apply(&mut cfg.layout, &c));
                    crate::invalidate_all();
                    popup::invalidate_popover();
                }
                None => {}
            }
            return;
        }
        if kind == POP_UPDATE {
            if let Cmd::Click(id) = cmd {
                self.update_cmd(id);
            }
        } else if kind == POP_TIMER {
            self.timer_cmd(cmd);
        } else if kind == POP_PRIVACY {
            match cmd {
                Cmd::Click(PV_MUTE) => {
                    let m = self.privacy_snapshot().mic_muted == Some(true);
                    if let Some(p) = &self.privacy {
                        p.set_mute(!m);
                    }
                }
                Cmd::Click(PV_SOUND) => {
                    popup::request(popup::Request::Launch {
                        path: "ms-settings:privacy-microphone".into(),
                        args: String::new(),
                    });
                }
                _ => {}
            }
        } else if kind == POP_BRIGHT {
            let snap = self.bright_snapshot();
            match cmd {
                Cmd::Click(BR_REFRESH) => {
                    if let Some(b) = &self.bright {
                        b.refresh();
                    }
                }
                Cmd::Slide(i, v) => {
                    if let (Some(b), Some(d)) = (&self.bright, snap.displays.get(i as usize)) {
                        b.set(&d.key, v);
                    }
                }
                Cmd::Click(_) => {}
            }
        }
        popup::invalidate_popover();
    }

    fn timer_cmd(&mut self, cmd: Cmd) {
        let now = now_ms();
        let cfg = pomo_cfg(&self.config.timer);
        let Cmd::Click(id) = cmd else { return };
        match id {
            TM_TOGGLE => self.timers.toggle(now),
            TM_RESET => self.timers.reset(),
            TM_MINUS => self.timers.adjust(now, -MIN),
            TM_PLUS => self.timers.adjust(now, MIN),
            TM_POMO => self.timers.pomodoro_start(now, &cfg),
            TM_POMO_SKIP => {
                if let Some(f) = self.timers.pomodoro_skip(now, &cfg) {
                    self.timer_finished(f, false);
                }
            }
            TM_SW_TOGGLE => self.timers.stopwatch.toggle(now),
            TM_SW_LAP => self.timers.stopwatch.lap(now),
            TM_SW_RESET => self.timers.stopwatch.reset(),
            TM_SOUND => {
                let on = !self.config.timer.sound;
                self.save_config_with(|c| c.timer.sound = on);
            }
            id if id > TM_PRESET => self.timers.start(now, i64::from(id - TM_PRESET) * MIN),
            _ => {}
        }
        log(&format!("casovac: prikaz {id}"));
        // stav sa uz zmenil (porovnanie v `update` by zmenu nevidelo): prekreslit listu
        crate::invalidate_all();
        self.sync_timer();
    }

    /// Naplanuje dalsi tik casovaca (zmena zobrazenej sekundy alebo koniec), kym nieco bezi.
    pub(crate) fn sync_timer(&mut self) {
        match self.timers.next_tick_ms(now_ms()) {
            Some(d) => schedule_timer(T_TIMER, d),
            None => transom_platform::appbar::cancel_timer(T_TIMER),
        }
    }

    pub(crate) fn timer_tick(&mut self) {
        let now = now_ms();
        let cfg = pomo_cfg(&self.config.timer);
        if let Some(f) = self.timers.tick(now, &cfg) {
            self.timer_finished(f, true);
        }
        crate::invalidate_all();
        if popup::popover_open() == Some(POP_TIMER) {
            popup::invalidate_popover();
        }
        self.sync_timer();
    }

    /// Koniec odpoctu alebo fazy: oznam v liste a (ak je zapnuty) systemovy zvuk upozornenia.
    fn timer_finished(&mut self, f: Finished, sound: bool) {
        log(&format!("casovac: koniec {f:?}"));
        if sound && self.config.timer.sound {
            transom_platform::system::notify_sound();
        }
        let text = timer::finished_text(f);
        if self.config.notch.timer {
            self.island_notice(crate::island::Notice {
                icon: transom_render::scene::glyph_char(transom_core::item::Glyph::Timer),
                title: text.clone(),
                sub: t("island.timer_sub"),
                act: Some(crate::island::NoticeAct::Timer),
            });
        }
        self.show_notice(text);
    }
}
