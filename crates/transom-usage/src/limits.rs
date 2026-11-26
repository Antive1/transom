//! Limity planu (kazde okno, ktore zdroj naozaj hlasi: napr. 5 h, 7 dni alebo 30 dni) pre kruhovy indikator v liste
//! a panel. Ziadne okno sa nepredpoklada: poskytovatel moze hlasit len tyzdenne ci mesacne okno.
//!
//! Pravidla (nemenit bez dovodu):
//! - Percento limitu vznika **len** zo snimku planu ([`crate::PlanSnapshot`]), ktory zaznamenal nastroj (statusline
//!   Claude Code, relacia Codex). Nikdy sa nepocita z tokenov ani z odhadu ceny ([`LimitReading`] nema verejny
//!   konstruktor mimo tohto modulu).
//! - Okno sa identifikuje podla **minut** (300 = 5 h, 10 080 = 7 dni), nie podla textu. 30-dnove okno nie je tyzdenne.
//! - Cerstvost ma tri stavy ([`Freshness`]); po case resetu okna (`now >= resets_at`) stara hodnota uz nepopisuje
//!   aktualne okno - nekresli sa ako aktualna vypln, nepredpoklada sa 0 % ani to, ze reset **uspesne prebehol**:
//!   zobrazi sa "cas resetu uplynul, caka sa na aktualizovane udaje", kym nepride cerstvy zaznam.
//! - Odpocet do resetu ([`reset_state`]) vznika **len** z `resets_at` zdroja a aktualneho casu. Dlzka ani nazov okna sa
//!   do neho nikdy nepouzivaju (ani ako "rozumnost" hodnoty): ak zdroj cas resetu nehlasi, je "nedostupny".
use crate::{PlanSnapshot, Provider, SnapshotSource};

pub const FIVE_HOURS: u32 = 300;
pub const WEEK: u32 = 10_080;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Zaznamenane nedavno (do `max_age`) a okno sa odvtedy neobnovilo.
    Current,
    /// Starsi zaznam, okno sa este neobnovilo - posledna znama hodnota, mozno uz neaktualna.
    LastRecorded,
    /// Okno sa od zaznamu obnovilo (`now >= resets_at`) - hodnota uz neplati.
    Expired,
}

pub fn freshness(recorded_at: i64, resets_at: Option<i64>, now: i64, max_age_s: i64) -> Freshness {
    if resets_at.is_some_and(|r| now >= r) {
        Freshness::Expired
    } else if now - recorded_at > max_age_s {
        Freshness::LastRecorded
    } else {
        Freshness::Current
    }
}

/// Stav odpoctu do resetu jedneho okna. Vznika len z `resets_at` a `now` (pozri hlavicku modulu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetState {
    /// Zdroj cas resetu nehlasi (alebo je neplatny): nic sa neodhaduje a nedopocitava.
    Unavailable,
    /// Reset este nenastal; zostava `secs` (> 0). `last_recorded` = zaznam je starsi (zastaraly), odpocet vychadza
    /// z **posledneho zaznamenaneho** casu resetu a v UI sa tak aj oznaci.
    In { secs: i64, last_recorded: bool },
    /// Zaznamenany cas resetu uz presiel; caka sa na aktualizovane udaje (neprepoklada sa, ze reset prebehol).
    Passed,
}

/// Odpocet do `resets_at` (Unix sekundy) v case `now`. `freshness` urcuje len oznacenie "zastaraly".
/// Hodnota `<= 0` sa berie ako chybajuca (nie je to cas).
pub fn reset_state(resets_at: Option<i64>, freshness: Freshness, now: i64) -> ResetState {
    match resets_at {
        None => ResetState::Unavailable,
        Some(t) if t <= 0 => ResetState::Unavailable,
        Some(t) if now >= t => ResetState::Passed,
        Some(t) => ResetState::In {
            secs: t - now,
            last_recorded: freshness == Freshness::LastRecorded,
        },
    }
}

/// Zostavajuci cas v podobe vhodnej na zobrazenie (jednotky sa skracuju v UI; ziadne mnozne cisla).
/// Zaokruhluje sa **nahor** na cele minuty (na poslednej minute "< 1 min"), od 1 dna nahor na cele hodiny.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanDuration {
    LessThanMinute,
    Minutes(u32),
    HoursMinutes(u32, u32),
    DaysHours(u32, u32),
}

pub fn human_duration(secs: i64) -> HumanDuration {
    if secs <= 60 {
        return HumanDuration::LessThanMinute;
    }
    let total_min = (secs + 59) / 60;
    if total_min < 60 {
        HumanDuration::Minutes(total_min as u32)
    } else if total_min < 24 * 60 {
        HumanDuration::HoursMinutes((total_min / 60) as u32, (total_min % 60) as u32)
    } else {
        let total_h = (total_min + 59) / 60;
        HumanDuration::DaysHours((total_h / 24) as u32, (total_h % 24) as u32)
    }
}

/// Co ukazuje kruh: spotrebovanu alebo zostavajucu cast limitu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RingMode {
    #[default]
    Used,
    Remaining,
}

impl RingMode {
    /// Tolerantne citanie z konfiguracie (neznama hodnota = predvolena).
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "remaining" | "left" => RingMode::Remaining,
            _ => RingMode::Used,
        }
    }
}

/// Precitana hodnota jedneho okna limitu.
#[derive(Clone, Debug, PartialEq)]
pub struct LimitReading {
    pub provider: Provider,
    pub minutes: u32,
    /// Spotrebovane percento, zaokruhlene a orezane na 0..=100.
    used: u8,
    pub recorded_at: i64,
    pub resets_at: Option<i64>,
    pub freshness: Freshness,
    pub plan: String,
    pub source: SnapshotSource,
}

impl LimitReading {
    pub fn used(&self) -> u8 {
        self.used
    }

    /// Odpocet do resetu tohto okna (len z `resets_at` zdroja; pozri [`reset_state`]).
    pub fn reset(&self, now: i64) -> ResetState {
        reset_state(self.resets_at, self.freshness, now)
    }

    /// Hodnota na zobrazenie; spotrebovane + zostavajuce = vzdy 100.
    pub fn shown(&self, mode: RingMode) -> u8 {
        match mode {
            RingMode::Used => self.used,
            RingMode::Remaining => 100 - self.used,
        }
    }
}

fn clamp_pct(p: f64) -> u8 {
    if p.is_nan() {
        0
    } else {
        p.round().clamp(0.0, 100.0) as u8
    }
}

/// Najnovsi snimok poskytovatela, ktory obsahuje okno s `minutes`.
pub fn reading(
    plans: &[PlanSnapshot],
    provider: Provider,
    minutes: u32,
    now: i64,
    max_age_s: i64,
) -> Option<LimitReading> {
    plans
        .iter()
        .filter(|p| p.provider == Some(provider))
        .filter_map(|p| {
            p.windows
                .iter()
                .find(|w| w.minutes == minutes)
                .map(|w| (p, w))
        })
        .max_by_key(|(p, _)| p.recorded_at)
        .map(|(p, w)| LimitReading {
            provider,
            minutes,
            used: clamp_pct(w.used_percent),
            recorded_at: p.recorded_at,
            resets_at: w.resets_at,
            freshness: freshness(p.recorded_at, w.resets_at, now, max_age_s),
            plan: p.plan.clone(),
            source: p.source,
        })
}

/// Volba poskytovatela pre kruh v liste (z konfiguracie).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProviderChoice {
    #[default]
    Auto,
    Claude,
    Codex,
}

impl ProviderChoice {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => ProviderChoice::Claude,
            "codex" => ProviderChoice::Codex,
            _ => ProviderChoice::Auto,
        }
    }
}

/// Vsetky okna poskytovatela, ktore sa v zaznamoch nasli, od najkratsieho: pre kazdu dlzku okna (minuty) najnovsi zaznam,
/// v ktorom sa okno vyskytlo. Okno, ktore v najnovsom zazname chyba (napr. 5 h po resete), tak nezmizne, ale ukaze sa
/// zo starsieho zaznamu ako zastarane/expirovane. Z nej citaju kruh, tooltip aj karty panela (jedno miesto pravdy).
pub fn provider_windows(
    plans: &[PlanSnapshot],
    provider: Provider,
    now: i64,
    max_age_s: i64,
) -> Vec<LimitReading> {
    let mut minutes: Vec<u32> = plans
        .iter()
        .filter(|p| p.provider == Some(provider))
        .flat_map(|p| p.windows.iter().map(|w| w.minutes))
        .collect();
    minutes.sort_unstable();
    minutes.dedup();
    minutes
        .into_iter()
        .filter_map(|m| reading(plans, provider, m, now, max_age_s))
        .collect()
}

/// Okno pre kruh v liste: **najkratsie hlasene okno** (aj expirovane - vtedy lista ukaze "cas resetu uplynul", nie
/// tichy skok na ine okno). Ziadne okno sa nevymysla (poskytovatel moze mat len tyzdenne alebo 30-dnove).
pub fn ring_window(windows: &[LimitReading]) -> Option<&LimitReading> {
    windows.iter().min_by_key(|r| r.minutes)
}

/// Poskytovatel pre kruh. Vyslovne zvoleny sa nikdy potichu nevymeni (bez dat = "nedostupne").
/// `Auto`: spomedzi zapnutych prvy s **aktualnou** hodnotou okna kruhu; inak prvy s poslednou zaznamenanou; inak
/// prvy s expirovanou; inak prvy zapnuty (nic zapnute = `None`). Poradie = poradie v `enabled`.
pub fn pick_provider(
    choice: ProviderChoice,
    enabled: &[Provider],
    plans: &[PlanSnapshot],
    now: i64,
    max_age_s: i64,
) -> Option<Provider> {
    match choice {
        ProviderChoice::Claude => Some(Provider::ClaudeCode),
        ProviderChoice::Codex => Some(Provider::Codex),
        ProviderChoice::Auto => {
            let state = |p: Provider| {
                let w = provider_windows(plans, p, now, max_age_s);
                ring_window(&w).map(|r| r.freshness)
            };
            for wanted in [
                Freshness::Current,
                Freshness::LastRecorded,
                Freshness::Expired,
            ] {
                if let Some(p) = enabled.iter().copied().find(|p| state(*p) == Some(wanted)) {
                    return Some(p);
                }
            }
            enabled.first().copied()
        }
    }
}

/// Poskytovatel s najvacsou rezervou v okne dlzky `minutes`: porovnavaju sa len **aktualne** hodnoty rovnakej dlzky okna
/// aspon dvoch poskytovatelov; pri zhode (alebo ak vyhrava zastarana/expirovana hodnota) nikto.
pub fn most_left(
    plans: &[PlanSnapshot],
    providers: &[Provider],
    minutes: u32,
    now: i64,
    max_age_s: i64,
) -> Option<Provider> {
    let cands: Vec<(Provider, u8)> = providers
        .iter()
        .filter_map(|p| reading(plans, *p, minutes, now, max_age_s).map(|r| (*p, r)))
        .filter(|(_, r)| r.freshness == Freshness::Current)
        .map(|(p, r)| (p, r.shown(RingMode::Remaining)))
        .collect();
    if cands.len() < 2 {
        return None;
    }
    let best = cands.iter().map(|(_, v)| *v).max()?;
    let mut top = cands.iter().filter(|(_, v)| *v == best);
    let first = top.next()?;
    top.next().is_none().then_some(first.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LimitWindow;

    fn snap(provider: Provider, at: i64, windows: Vec<(u32, f64, Option<i64>)>) -> PlanSnapshot {
        PlanSnapshot {
            provider: Some(provider),
            plan: "x".into(),
            windows: windows
                .into_iter()
                .map(|(minutes, used_percent, resets_at)| LimitWindow {
                    minutes,
                    used_percent,
                    resets_at,
                })
                .collect(),
            recorded_at: at,
            source: SnapshotSource::ClaudeStatusline,
        }
    }

    #[test]
    fn used_and_remaining_always_sum_to_100() {
        for p in [
            0.0,
            0.4,
            0.5,
            12.5,
            37.49,
            62.51,
            99.5,
            100.0,
            140.0,
            -3.0,
            f64::NAN,
        ] {
            let plans = vec![snap(Provider::ClaudeCode, 100, vec![(FIVE_HOURS, p, None)])];
            let r = reading(&plans, Provider::ClaudeCode, FIVE_HOURS, 100, 3600).unwrap();
            assert!(r.used() <= 100);
            assert_eq!(
                u32::from(r.shown(RingMode::Used)) + u32::from(r.shown(RingMode::Remaining)),
                100,
                "{p}"
            );
        }
        let plans = vec![snap(
            Provider::ClaudeCode,
            1,
            vec![(FIVE_HOURS, 62.4, None)],
        )];
        let r = reading(&plans, Provider::ClaudeCode, FIVE_HOURS, 1, 60).unwrap();
        assert_eq!(
            (r.shown(RingMode::Used), r.shown(RingMode::Remaining)),
            (62, 38)
        );
        assert_eq!(RingMode::parse("remaining"), RingMode::Remaining);
        assert_eq!(RingMode::parse("nezmysel"), RingMode::Used);
    }

    #[test]
    fn windows_are_matched_by_minutes_not_labels() {
        // Codex plan go: len 30-dnove okno -> ziadne 5 h ani tyzden
        let plans = vec![snap(Provider::Codex, 10, vec![(43_200, 30.0, Some(99))])];
        assert!(reading(&plans, Provider::Codex, FIVE_HOURS, 10, 60).is_none());
        assert!(reading(&plans, Provider::Codex, WEEK, 10, 60).is_none());
        assert!(reading(&plans, Provider::Codex, 43_200, 10, 60).is_some());
        // ina instancia poskytovatela sa nemiesa
        assert!(reading(&plans, Provider::ClaudeCode, 43_200, 10, 60).is_none());
    }

    #[test]
    fn newest_snapshot_wins() {
        let plans = vec![
            snap(Provider::ClaudeCode, 200, vec![(FIVE_HOURS, 70.0, None)]),
            snap(Provider::ClaudeCode, 100, vec![(FIVE_HOURS, 10.0, None)]),
            snap(Provider::ClaudeCode, 300, vec![(WEEK, 40.0, None)]),
        ];
        assert_eq!(
            reading(&plans, Provider::ClaudeCode, FIVE_HOURS, 300, 999)
                .unwrap()
                .used(),
            70
        );
        assert_eq!(
            reading(&plans, Provider::ClaudeCode, WEEK, 300, 999)
                .unwrap()
                .used(),
            40
        );
    }

    #[test]
    fn reset_state_valid_stale_missing_and_passed() {
        let now = 1_000_000;
        // platny a cerstvy
        assert_eq!(
            reset_state(Some(now + 8_040), Freshness::Current, now),
            ResetState::In {
                secs: 8_040,
                last_recorded: false
            }
        );
        // zastaraly zaznam: odpocet sa ukaze, ale oznaceny ako z posledneho zaznamu
        assert_eq!(
            reset_state(Some(now + 8_040), Freshness::LastRecorded, now),
            ResetState::In {
                secs: 8_040,
                last_recorded: true
            }
        );
        // chybajuci cas = nedostupny (nic sa nedopocitava)
        assert_eq!(
            reset_state(None, Freshness::Current, now),
            ResetState::Unavailable
        );
        assert_eq!(
            reset_state(None, Freshness::LastRecorded, now),
            ResetState::Unavailable
        );
        // neplatny (nie je to cas) = nedostupny
        assert_eq!(
            reset_state(Some(0), Freshness::Current, now),
            ResetState::Unavailable
        );
        assert_eq!(
            reset_state(Some(-5), Freshness::Current, now),
            ResetState::Unavailable
        );
        // presiel (aj presne teraz): caka sa na nove udaje, nic sa nepredpoklada
        assert_eq!(
            reset_state(Some(now), Freshness::Current, now),
            ResetState::Passed
        );
        assert_eq!(
            reset_state(Some(now - 1), Freshness::LastRecorded, now),
            ResetState::Passed
        );
        assert_eq!(
            reset_state(Some(now - 99_999), Freshness::Expired, now),
            ResetState::Passed
        );
        // odpocet nezavisi od dlzky okna ani od toho, kolko je zaznam stary (len od resets_at a now)
        let a = reset_state(Some(now + 600), Freshness::Current, now);
        let b = reset_state(Some(now + 600), Freshness::Current, now);
        assert_eq!(a, b);
    }

    #[test]
    fn human_duration_rounds_up_and_picks_units() {
        use HumanDuration::*;
        for (secs, want) in [
            (1, LessThanMinute),
            (59, LessThanMinute),
            (60, LessThanMinute),
            (61, Minutes(2)),
            (119, Minutes(2)),
            (120, Minutes(2)),
            (121, Minutes(3)),
            (59 * 60, Minutes(59)),
            (59 * 60 + 1, HoursMinutes(1, 0)),
            (3_600, HoursMinutes(1, 0)),
            (8_040, HoursMinutes(2, 14)), // "2 h 14 min"
            (8_041, HoursMinutes(2, 15)),
            (5 * 3600, HoursMinutes(5, 0)),
            (23 * 3600 + 59 * 60, HoursMinutes(23, 59)),
            (23 * 3600 + 59 * 60 + 1, DaysHours(1, 0)),
            (24 * 3600, DaysHours(1, 0)),
            (24 * 3600 + 1, DaysHours(1, 1)),
            (3 * 86_400 + 4 * 3600, DaysHours(3, 4)),
            (7 * 86_400, DaysHours(7, 0)),
            (30 * 86_400 - 1, DaysHours(30, 0)),
        ] {
            assert_eq!(human_duration(secs), want, "{secs}");
        }
    }

    #[test]
    fn countdown_is_pure_seconds_so_dates_zones_and_dst_do_not_matter() {
        // pocitaju sa len sekundy medzi dvoma okamihmi: zmena kalendarneho dna, casovej zony ani letneho casu ich
        // neovplyvni (zobrazenie absolutneho casu resetu je vec lokalneho formatu, nie odpoctu)
        let day = 86_400;
        let midnight = 1_795_219_200; // 2026-11-20T00:00:00Z
                                      // 23:59:30 -> reset o polnoci: 30 s = "< 1 min"; o 90 s = 2 min (hranica dna sa nezapocituje inak)
        assert_eq!(
            human_duration(midnight - (midnight - 30)),
            HumanDuration::LessThanMinute
        );
        assert_eq!(human_duration(90), HumanDuration::Minutes(2));
        // 23:30 -> reset 02:00 nasledujuceho dna (cez polnoc): 2 h 30 min + 0 = 26 h 30 min -> 1 d 3 h (nahor)
        let now = midnight - 30 * 60;
        let reset = midnight + day + 2 * 3600;
        let ResetState::In { secs, .. } = reset_state(Some(reset), Freshness::Current, now) else {
            panic!()
        };
        assert_eq!(secs, 26 * 3600 + 30 * 60);
        assert_eq!(human_duration(secs), HumanDuration::DaysHours(1, 3));
        // prechod na letny cas (EU 2026-03-29 01:00Z): 24 h po sebe iducich sekund = "1 d 0 h" v oboch smeroch
        let dst = 1_774_746_000; // 2026-03-29T01:00:00Z
        let ResetState::In { secs, .. } =
            reset_state(Some(dst + day), Freshness::Current, dst - 3600 + 3600)
        else {
            panic!()
        };
        assert_eq!(human_duration(secs), HumanDuration::DaysHours(1, 0));
        // koniec roka
        let ny = 1_798_761_600; // 2027-01-01T00:00:00Z
        let ResetState::In { secs, .. } =
            reset_state(Some(ny), Freshness::Current, ny - 5 * 3600 - 14 * 60)
        else {
            panic!()
        };
        assert_eq!(human_duration(secs), HumanDuration::HoursMinutes(5, 14));
    }

    // ---- skutocne parsery: odpocet pre kazdeho poskytovatela a jeho okna ----

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("transom-reset-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn local(ts: i64) -> transom_core::clock::Ymd {
        transom_core::clock::civil_from_days(ts.div_euclid(86_400))
    }

    /// Claude Code: vstup `statusLine` -> zaznam mostu -> skutocny `read_file` -> `reading` -> odpocet.
    #[derive(Clone, Copy)]
    enum Win {
        /// Okno vo vstupe nie je (napr. ho Claude Code po case resetu vynechal).
        Absent,
        /// Okno je, ale bez `resets_at`.
        NoReset,
        At(i64),
    }

    fn claude_reset(
        five: Win,
        week: Win,
        recorded: i64,
        now: i64,
    ) -> (Option<ResetState>, Option<ResetState>) {
        let mut rl = serde_json::Map::new();
        let mut win = |name: &str, p: f64, w: Win| {
            let mut m = serde_json::Map::new();
            m.insert("used_percentage".into(), serde_json::json!(p));
            match w {
                Win::Absent => return,
                Win::NoReset => {}
                Win::At(r) => {
                    m.insert("resets_at".into(), serde_json::json!(r));
                }
            }
            rl.insert(name.into(), serde_json::Value::Object(m));
        };
        win("five_hour", 62.0, five);
        win("seven_day", 40.0, week);
        let input = serde_json::json!({"session_id": "s", "model": {"id": "m"}, "cost": {"total_cost_usd": 1.0},
                                       "rate_limits": rl});
        let rec = crate::statusline::record(&input, recorded).expect("zaznam");
        let d = temp_dir("claude");
        let f = d.join("claude-statusline.jsonl");
        std::fs::write(&f, format!("{rec}\n")).unwrap();
        let (_, _, plan, _) = crate::statusline::read_file(&f, &local);
        let _ = std::fs::remove_dir_all(&d);
        let plans: Vec<PlanSnapshot> = plan.into_iter().collect();
        let at = |m| reading(&plans, Provider::ClaudeCode, m, now, 1800).map(|r| r.reset(now));
        (at(FIVE_HOURS), at(WEEK))
    }

    #[test]
    fn claude_statusline_five_hour_and_weekly_countdowns() {
        let rec = 1_795_000_000;
        // cerstvy zaznam: 5 h konci o 2 h 14 min, tyzden o 3 d 4 h
        let now = rec + 60;
        let (five, week) = claude_reset(
            Win::At(rec + 8_100),
            Win::At(rec + 3 * 86_400 + 4 * 3600),
            rec,
            now,
        );
        assert_eq!(
            five,
            Some(ResetState::In {
                secs: 8_040,
                last_recorded: false
            })
        );
        assert_eq!(
            week,
            Some(ResetState::In {
                secs: 3 * 86_400 + 4 * 3600 - 60,
                last_recorded: false
            })
        );
        // zastaraly zaznam (> 30 min), okno este nepresiel: odpocet z posledneho zaznamu
        let now = rec + 3_000;
        let (five, _) = claude_reset(Win::At(rec + 8_100), Win::At(rec + 400_000), rec, now);
        assert_eq!(
            five,
            Some(ResetState::In {
                secs: 5_100,
                last_recorded: true
            })
        );
        // zdroj cas resetu nehlasi: nedostupny (nikdy sa nedopocita z "5 h")
        let (five, week) = claude_reset(Win::NoReset, Win::NoReset, rec, rec + 10);
        assert_eq!(
            (five, week),
            (Some(ResetState::Unavailable), Some(ResetState::Unavailable))
        );
        // iba tyzden hlasi cas: 5 h nedostupny, tyzden odpocet
        let (five, week) = claude_reset(Win::NoReset, Win::At(rec + 86_400), rec, rec + 10);
        assert_eq!(five, Some(ResetState::Unavailable));
        assert!(matches!(week, Some(ResetState::In { .. })));
        // cas resetu uz presiel: caka sa na nove udaje (nie "reset prebehol")
        let (five, week) = claude_reset(
            Win::At(rec + 8_100),
            Win::At(rec + 400_000),
            rec,
            rec + 9_000,
        );
        assert_eq!(five, Some(ResetState::Passed));
        assert!(matches!(
            week,
            Some(ResetState::In {
                last_recorded: true,
                ..
            })
        ));
        // okno chyba vo vstupe (Claude Code ho po case resetu zahodi) = ziadne okno, ziadny odpocet
        let (five, week) = claude_reset(Win::Absent, Win::At(rec + 400_000), rec, rec + 10);
        assert_eq!(five, None);
        assert!(matches!(week, Some(ResetState::In { .. })));
        // vstup bez `rate_limits` (API kluc / este ziadna odpoved): ziadne okno = ziadny odpocet
        let (five, week) = {
            let input = serde_json::json!({"session_id": "s", "cost": {"total_cost_usd": 1.0}});
            let rec = crate::statusline::record(&input, rec).expect("zaznam s cenou");
            let d = temp_dir("claude-none");
            let f = d.join("x.jsonl");
            std::fs::write(&f, format!("{rec}\n")).unwrap();
            let (_, _, plan, _) = crate::statusline::read_file(&f, &local);
            let _ = std::fs::remove_dir_all(&d);
            let plans: Vec<PlanSnapshot> = plan.into_iter().collect();
            (
                reading(&plans, Provider::ClaudeCode, FIVE_HOURS, rec_now(), 1800),
                reading(&plans, Provider::ClaudeCode, WEEK, rec_now(), 1800),
            )
        };
        assert!(five.is_none() && week.is_none());
    }

    fn rec_now() -> i64 {
        1_795_000_100
    }

    /// Codex: relacia `token_count.rate_limits` -> skutocny `read_dir` -> `reading` -> odpocet.
    fn codex_windows(primary: &str, secondary: &str, now: i64) -> Vec<(u32, Option<ResetState>)> {
        let d = temp_dir("codex");
        let sess = d.join("sessions").join("2026").join("11").join("20");
        std::fs::create_dir_all(&sess).unwrap();
        let line = format!(
            r#"{{"timestamp":"2026-11-20T10:00:00.000Z","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":10,"output_tokens":5}}}},"rate_limits":{{"primary":{primary},"secondary":{secondary},"plan_type":"plus"}}}}}}"#
        );
        std::fs::write(sess.join("rollout-a.jsonl"), format!("{line}\n")).unwrap();
        let (_, _, plan) = crate::codex::read_dir(&d);
        let _ = std::fs::remove_dir_all(&d);
        let plans: Vec<PlanSnapshot> = plan.into_iter().collect();
        [FIVE_HOURS, WEEK, 43_200]
            .into_iter()
            .map(|m| {
                (
                    m,
                    reading(&plans, Provider::Codex, m, now, 1800).map(|r| r.reset(now)),
                )
            })
            .collect()
    }

    #[test]
    fn codex_windows_report_their_own_reset_or_none() {
        // plan s oknami 5 h a 7 dni (predpoklad, nezdokumentovana schema): kazde ma vlastny cas resetu
        let rec = 1_795_000_000; // cas zaznamu v teste = cas zaznamu v subore sa neziskava z textu; pouzije sa mtime/zaznam
        let _ = rec;
        let now = parse_now();
        let p = format!(
            r#"{{"used_percent":20.0,"window_minutes":300,"resets_at":{}}}"#,
            now + 8_040
        );
        let s = format!(
            r#"{{"used_percent":35.0,"window_minutes":10080,"resets_at":{}}}"#,
            now + 2 * 86_400
        );
        let w = codex_windows(&p, &s, now);
        assert!(
            matches!(w[0].1, Some(ResetState::In { secs: 8_040, .. })),
            "{w:?}"
        );
        assert!(matches!(w[1].1, Some(ResetState::In { .. })), "{w:?}");
        assert_eq!(w[2].1, None, "30-dnove okno tento plan nehlasi");
        // plan `go`: len 30-dnove okno, 5 h a tyzden neexistuju (nic sa nedopocita)
        let p = format!(
            r#"{{"used_percent":30.0,"window_minutes":43200,"resets_at":{}}}"#,
            now + 20 * 86_400
        );
        let w = codex_windows(&p, "null", now);
        assert_eq!((w[0].1, w[1].1), (None, None));
        assert!(matches!(w[2].1, Some(ResetState::In { .. })));
        // okno bez casu resetu: nedostupny
        let p = r#"{"used_percent":20.0,"window_minutes":300}"#;
        let w = codex_windows(p, "null", now);
        assert_eq!(w[0].1, Some(ResetState::Unavailable));
        // cas resetu uz presiel
        let p = format!(
            r#"{{"used_percent":20.0,"window_minutes":300,"resets_at":{}}}"#,
            now - 5
        );
        let w = codex_windows(&p, "null", now);
        assert_eq!(w[0].1, Some(ResetState::Passed));
    }

    fn parse_now() -> i64 {
        // zaznam Codexu ma cas z textovej znacky "2026-11-20T10:00:00Z"
        crate::parse_iso_utc("2026-11-20T10:00:00.000Z").unwrap() + 120
    }

    #[test]
    fn freshness_states() {
        assert_eq!(freshness(1000, Some(5000), 1500, 600), Freshness::Current);
        assert_eq!(
            freshness(1000, Some(5000), 2000, 600),
            Freshness::LastRecorded
        );
        assert_eq!(
            freshness(1000, Some(5000), 5000, 600),
            Freshness::Expired,
            "po resete neplati"
        );
        assert_eq!(
            freshness(1000, None, 1_000_000, 600),
            Freshness::LastRecorded,
            "bez resetu len starne"
        );
    }

    const BOTH: [Provider; 2] = [Provider::ClaudeCode, Provider::Codex];

    #[test]
    fn provider_selection() {
        let claude5 = snap(
            Provider::ClaudeCode,
            100,
            vec![(FIVE_HOURS, 20.0, Some(10_000))],
        );
        let codex30 = snap(Provider::Codex, 100, vec![(43_200, 30.0, None)]);
        let plans = vec![codex30.clone(), claude5.clone()];
        let auto = |en: &[Provider], plans: &[PlanSnapshot], now| {
            pick_provider(ProviderChoice::Auto, en, plans, now, 600)
        };
        // oba aktualne: prvy v poradi (Claude)
        assert_eq!(auto(&BOTH, &plans, 200), Some(Provider::ClaudeCode));
        // Claude vypnuty: Codex a jeho 30-dnove okno
        assert_eq!(auto(&[Provider::Codex], &plans, 200), Some(Provider::Codex));
        // 5 h Claude expirovala, Codex ma len zastarany zaznam bez resetu -> Codex (posledna zaznamenana > expirovana)
        assert_eq!(auto(&BOTH, &plans, 20_000), Some(Provider::Codex));
        // nikto nic nema: prvy zapnuty (zobrazi "nedostupne"); nic zapnute = None
        assert_eq!(auto(&BOTH, &[], 200), Some(Provider::ClaudeCode));
        assert_eq!(auto(&[], &plans, 200), None);
        // vyslovna volba sa nemeni, ani bez dat
        assert_eq!(
            pick_provider(ProviderChoice::Codex, &BOTH, &[claude5], 200, 600),
            Some(Provider::Codex)
        );
        assert_eq!(ProviderChoice::parse("codex"), ProviderChoice::Codex);
        assert_eq!(ProviderChoice::parse("???"), ProviderChoice::Auto);
    }

    #[test]
    fn auto_picks_codex_with_a_stale_30_day_window_when_claude_has_no_plan_data() {
        // zivy pripad: Claude zapnuty, ale bez limitov (statusline este nic nezapisal); Codex ma len 30-dnove okno
        let codex30 = snap(
            Provider::Codex,
            1_000,
            vec![(43_200, 31.0, Some(1_000 + 25 * 86_400))],
        );
        let plans = vec![codex30];
        let now = 1_000 + 3 * 3_600; // zaznam je starsi nez 30 min
        assert_eq!(
            pick_provider(ProviderChoice::Auto, &BOTH, &plans, now, 1_800),
            Some(Provider::Codex)
        );
        let w = provider_windows(&plans, Provider::Codex, now, 1_800);
        let r = ring_window(&w).unwrap();
        assert_eq!((r.minutes, r.freshness), (43_200, Freshness::LastRecorded));
        // Claude: ziadne okna -> kruh "nedostupne", nic sa nevymysla (ani 5 h)
        assert!(provider_windows(&plans, Provider::ClaudeCode, now, 1_800).is_empty());
        assert!(ring_window(&[]).is_none());
    }

    #[test]
    fn provider_windows_lists_only_reported_windows_sorted_and_keeps_windows_missing_from_the_latest_record(
    ) {
        // Codex "go": len 30 dni
        let go = vec![snap(Provider::Codex, 10, vec![(43_200, 30.0, Some(99))])];
        let w = provider_windows(&go, Provider::Codex, 10, 60);
        assert_eq!(w.iter().map(|r| r.minutes).collect::<Vec<_>>(), [43_200]);
        // Claude: 7 d aj 5 h, zoradene od najkratsieho; 5 h chyba v najnovsom zazname a ostava zo stareho
        let plans = vec![
            snap(
                Provider::ClaudeCode,
                100,
                vec![(FIVE_HOURS, 70.0, Some(5_000)), (WEEK, 10.0, Some(900_000))],
            ),
            snap(
                Provider::ClaudeCode,
                6_000,
                vec![(WEEK, 12.0, Some(900_000))],
            ),
        ];
        let w = provider_windows(&plans, Provider::ClaudeCode, 6_100, 1_800);
        assert_eq!(
            w.iter().map(|r| r.minutes).collect::<Vec<_>>(),
            [FIVE_HOURS, WEEK]
        );
        assert_eq!(w[0].used(), 70);
        assert_eq!(
            w[0].freshness,
            Freshness::Expired,
            "5 h po resete: neplati, nezmizne"
        );
        assert_eq!(w[1].used(), 12, "tyzdenne okno je z najnovsieho zaznamu");
        assert_eq!(w[1].freshness, Freshness::Current);
        // len tyzdenne okno (napr. plan bez 5 h)
        let weekly = vec![snap(Provider::ClaudeCode, 1, vec![(WEEK, 5.0, None)])];
        let w = provider_windows(&weekly, Provider::ClaudeCode, 2, 60);
        assert_eq!(ring_window(&w).unwrap().minutes, WEEK);
    }

    #[test]
    fn ring_keeps_the_shortest_reported_window_even_when_it_has_expired() {
        let plans = vec![snap(
            Provider::ClaudeCode,
            100,
            vec![(FIVE_HOURS, 70.0, Some(5_000)), (WEEK, 10.0, Some(900_000))],
        )];
        let w = provider_windows(&plans, Provider::ClaudeCode, 6_000, 99_999);
        let r = ring_window(&w).unwrap();
        // po resete 5 h okna sa kruh neprepne potichu na tyzden: ostane 5 h v stave "cas resetu uplynul"
        assert_eq!((r.minutes, r.freshness), (FIVE_HOURS, Freshness::Expired));
        assert_eq!(r.reset(6_000), ResetState::Passed);
    }

    #[test]
    fn most_left_compares_only_current_readings_of_the_same_window() {
        let a = snap(
            Provider::ClaudeCode,
            100,
            vec![(WEEK, 70.0, Some(9_999_999))],
        );
        let b = snap(Provider::Codex, 100, vec![(WEEK, 20.0, Some(9_999_999))]);
        let plans = vec![a.clone(), b.clone()];
        assert_eq!(
            most_left(&plans, &BOTH, WEEK, 200, 600),
            Some(Provider::Codex)
        );
        // jeden poskytovatel = nie je s kym porovnavat
        assert_eq!(
            most_left(std::slice::from_ref(&a), &BOTH, WEEK, 200, 600),
            None
        );
        // zhoda = ziadny odznak
        let tie = vec![
            a.clone(),
            snap(Provider::Codex, 100, vec![(WEEK, 70.0, None)]),
        ];
        assert_eq!(most_left(&tie, &BOTH, WEEK, 200, 600), None);
        // zastarana hodnota nikdy nevyhra (ani nesuti)
        assert_eq!(most_left(&plans, &BOTH, WEEK, 5_000, 600), None);
        // rozne dlzky okien sa nemiesaju
        let c = snap(Provider::Codex, 100, vec![(43_200, 1.0, None)]);
        assert_eq!(most_left(&[a, c], &BOTH, WEEK, 200, 600), None);
    }
}
