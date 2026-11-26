//! Casovac, pomodoro a stopky (cisty model, bez Win32). Vsetko sa pocita z casovych znaciek (ms od epochy), nie
//! odpocitavanim tikov: uspanie PC, pomaly tik ani prekreslenie neposunu koniec. Polozka lisaty a panel citaju stav.
use transom_i18n::{t, tf};

/// Faza pomodora.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Work,
    Break,
    LongBreak,
}

/// Odpocet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Countdown {
    #[default]
    Idle,
    Running {
        end_ms: i64,
        total_ms: i64,
    },
    Paused {
        left_ms: i64,
        total_ms: i64,
    },
    /// Skoncil (ukazuje sa "Hotovo", kym ho pouzivatel nezrusi alebo nespusti novy).
    Done,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stopwatch {
    /// Bezi od (ms); `None` = zastavene.
    pub since: Option<i64>,
    /// Nameraný cas pred poslednym spustenim.
    pub acc_ms: i64,
    /// Kola (celkovy cas v okamihu kola), najnovsie posledne.
    pub laps: Vec<i64>,
}

impl Stopwatch {
    pub fn elapsed(&self, now: i64) -> i64 {
        self.acc_ms + self.since.map_or(0, |s| (now - s).max(0))
    }
    pub fn running(&self) -> bool {
        self.since.is_some()
    }
    pub fn toggle(&mut self, now: i64) {
        match self.since.take() {
            Some(s) => self.acc_ms += (now - s).max(0),
            None => self.since = Some(now),
        }
    }
    pub fn lap(&mut self, now: i64) {
        if self.running() {
            self.laps.push(self.elapsed(now));
            if self.laps.len() > 99 {
                self.laps.remove(0);
            }
        }
    }
    pub fn reset(&mut self) {
        *self = Stopwatch::default();
    }
}

/// Dlzky pomodora v minutach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PomoCfg {
    pub work: u32,
    pub short_break: u32,
    pub long_break: u32,
    /// Pocet pracovnych kol pred dlhou prestavkou.
    pub rounds: u32,
}

impl Default for PomoCfg {
    fn default() -> Self {
        PomoCfg {
            work: 25,
            short_break: 5,
            long_break: 15,
            rounds: 4,
        }
    }
}

/// Co sa stalo pri tiku.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finished {
    /// Obycajny odpocet skoncil.
    Countdown,
    /// Skoncila faza pomodora (dalsia sa uz pripadne spustila).
    Phase { ended: Phase, next: Option<Phase> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timers {
    pub countdown: Countdown,
    /// Aktivne pomodoro: (faza, pocet dokoncenych pracovnych kol).
    pub pomodoro: Option<(Phase, u32)>,
    pub stopwatch: Stopwatch,
    /// Posledna zvolena dlzka odpoctu (ms) - pre "Spustit" a +-1 min v necinnosti.
    pub preset_ms: i64,
}

pub const MIN: i64 = 60_000;

impl Timers {
    pub fn new() -> Timers {
        Timers {
            preset_ms: 5 * MIN,
            ..Timers::default()
        }
    }

    /// Zostavajuci cas odpoctu (ms), ak bezi alebo je pozastaveny.
    pub fn left(&self, now: i64) -> Option<i64> {
        match self.countdown {
            Countdown::Running { end_ms, .. } => Some((end_ms - now).max(0)),
            Countdown::Paused { left_ms, .. } => Some(left_ms),
            _ => None,
        }
    }

    /// Nieco bezi (treba tikat).
    pub fn ticking(&self) -> bool {
        matches!(self.countdown, Countdown::Running { .. }) || self.stopwatch.running()
    }

    /// Spusti odpocet na `ms` (zrusi pomodoro).
    pub fn start(&mut self, now: i64, ms: i64) {
        let ms = ms.clamp(1000, 24 * 60 * MIN);
        self.preset_ms = ms;
        self.pomodoro = None;
        self.countdown = Countdown::Running {
            end_ms: now + ms,
            total_ms: ms,
        };
    }

    /// Spusti / pozastavi / obnovi odpocet.
    pub fn toggle(&mut self, now: i64) {
        self.countdown = match self.countdown {
            Countdown::Running { end_ms, total_ms } => Countdown::Paused {
                left_ms: (end_ms - now).max(0),
                total_ms,
            },
            Countdown::Paused { left_ms, total_ms } => Countdown::Running {
                end_ms: now + left_ms,
                total_ms,
            },
            Countdown::Idle | Countdown::Done => {
                let ms = self.preset_ms.max(1000);
                self.pomodoro = None;
                Countdown::Running {
                    end_ms: now + ms,
                    total_ms: ms,
                }
            }
        };
    }

    /// Prida (alebo uberie) cas: beziacemu / pozastavenemu odpoctu, inak predvolenej dlzke.
    pub fn adjust(&mut self, now: i64, delta: i64) {
        match &mut self.countdown {
            Countdown::Running { end_ms, total_ms } => {
                *end_ms = (*end_ms + delta).max(now + 1000);
                *total_ms = (*total_ms + delta).max(1000);
            }
            Countdown::Paused { left_ms, total_ms } => {
                *left_ms = (*left_ms + delta).max(1000);
                *total_ms = (*total_ms + delta).max(1000);
            }
            Countdown::Idle | Countdown::Done => {
                self.preset_ms = (self.preset_ms + delta).clamp(MIN, 24 * 60 * MIN);
                self.countdown = Countdown::Idle;
            }
        }
    }

    pub fn reset(&mut self) {
        self.countdown = Countdown::Idle;
        self.pomodoro = None;
    }

    fn phase_ms(p: Phase, cfg: &PomoCfg) -> i64 {
        i64::from(match p {
            Phase::Work => cfg.work,
            Phase::Break => cfg.short_break,
            Phase::LongBreak => cfg.long_break,
        })
        .clamp(1, 24 * 60)
            * MIN
    }

    fn run_phase(&mut self, now: i64, p: Phase, done: u32, cfg: &PomoCfg) {
        let ms = Self::phase_ms(p, cfg);
        self.pomodoro = Some((p, done));
        self.countdown = Countdown::Running {
            end_ms: now + ms,
            total_ms: ms,
        };
    }

    /// Spusti pomodoro od pracovnej fazy (alebo dalsie pracovne kolo po prestavke).
    pub fn pomodoro_start(&mut self, now: i64, cfg: &PomoCfg) {
        let done = self.pomodoro.map_or(0, |p| p.1);
        self.run_phase(now, Phase::Work, done, cfg);
    }

    /// Preskoci aktualnu fazu (ako keby skoncila).
    pub fn pomodoro_skip(&mut self, now: i64, cfg: &PomoCfg) -> Option<Finished> {
        self.pomodoro?;
        if let Countdown::Running { end_ms, .. } = &mut self.countdown {
            *end_ms = now;
        } else if let Countdown::Paused { .. } = self.countdown {
            self.countdown = Countdown::Running {
                end_ms: now,
                total_ms: 1000,
            };
        }
        self.tick(now, cfg)
    }

    /// Tik: koniec odpoctu alebo fazy. Po praci sa prestavka spusti sama; po prestavke sa caka na pouzivatela.
    pub fn tick(&mut self, now: i64, cfg: &PomoCfg) -> Option<Finished> {
        let Countdown::Running { end_ms, .. } = self.countdown else {
            return None;
        };
        if now < end_ms {
            return None;
        }
        match self.pomodoro {
            None => {
                self.countdown = Countdown::Done;
                Some(Finished::Countdown)
            }
            Some((Phase::Work, done)) => {
                let done = done + 1;
                let next = if done % cfg.rounds.max(1) == 0 {
                    Phase::LongBreak
                } else {
                    Phase::Break
                };
                self.run_phase(now, next, done, cfg);
                Some(Finished::Phase {
                    ended: Phase::Work,
                    next: Some(next),
                })
            }
            Some((p, done)) => {
                self.pomodoro = Some((p, done));
                self.countdown = Countdown::Done;
                Some(Finished::Phase {
                    ended: p,
                    next: None,
                })
            }
        }
    }

    /// Ms do najblizsieho tiku (zmena zobrazenej sekundy alebo koniec), ak nieco bezi.
    pub fn next_tick_ms(&self, now: i64) -> Option<u32> {
        if !self.ticking() {
            return None;
        }
        let mut d = 1000 - now.rem_euclid(1000);
        if let Some(l) = self.left(now) {
            if matches!(self.countdown, Countdown::Running { .. }) {
                d = d.min(l.rem_euclid(1000).max(1)).min(l.max(1));
            }
        }
        Some(d.clamp(20, 1000) as u32)
    }
}

/// "m:ss" alebo "h:mm:ss" (odpocet sa zaokruhluje nahor, aby "0:00" znamenalo naozaj koniec).
pub fn fmt_ms(ms: i64, ceil: bool) -> String {
    let s = if ceil { (ms + 999) / 1000 } else { ms / 1000 }.max(0);
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn phase_name(p: Phase) -> String {
    t(match p {
        Phase::Work => "timer.ph_work",
        Phase::Break => "timer.ph_break",
        Phase::LongBreak => "timer.ph_long",
    })
}

/// Text oznamu po skonceni.
pub fn finished_text(f: Finished) -> String {
    match f {
        Finished::Countdown => t("timer.done_notice"),
        Finished::Phase {
            ended,
            next: Some(n),
        } => tf("timer.phase_next", &[&phase_name(ended), &phase_name(n)]),
        Finished::Phase { ended, next: None } => tf("timer.phase_end", &[&phase_name(ended)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_runs_from_timestamps_pauses_adjusts_and_finishes_once() {
        let cfg = PomoCfg::default();
        let mut tm = Timers::new();
        assert!(!tm.ticking() && tm.next_tick_ms(0).is_none());
        tm.start(10_000, 2 * MIN);
        assert_eq!(tm.left(10_000), Some(2 * MIN));
        // uspanie PC: po navrate je cas spravne prec, nie "zamrznuty"
        assert_eq!(tm.left(10_000 + 90_000), Some(30_000));
        tm.toggle(40_000);
        assert_eq!(
            tm.countdown,
            Countdown::Paused {
                left_ms: 90_000,
                total_ms: 2 * MIN
            }
        );
        assert_eq!(tm.left(999_999), Some(90_000), "pozastavene stoji");
        tm.adjust(999_999, MIN);
        tm.toggle(1_000_000);
        assert_eq!(tm.left(1_000_000), Some(150_000));
        assert_eq!(tm.tick(1_100_000, &cfg), None);
        assert_eq!(tm.tick(1_150_000, &cfg), Some(Finished::Countdown));
        assert_eq!(tm.countdown, Countdown::Done);
        assert_eq!(tm.tick(1_200_000, &cfg), None, "koniec len raz");
        // necinnost: +-1 min meni predvolbu, nie pod 1 min
        tm.reset();
        tm.adjust(0, -10 * MIN);
        assert_eq!(tm.preset_ms, MIN);
        tm.toggle(0);
        assert_eq!(tm.left(0), Some(MIN));
    }

    #[test]
    fn pomodoro_starts_breaks_by_itself_and_waits_after_a_break() {
        let cfg = PomoCfg {
            work: 25,
            short_break: 5,
            long_break: 15,
            rounds: 2,
        };
        let mut tm = Timers::new();
        tm.pomodoro_start(0, &cfg);
        assert_eq!(tm.left(0), Some(25 * MIN));
        let f = tm.tick(25 * MIN, &cfg);
        assert_eq!(
            f,
            Some(Finished::Phase {
                ended: Phase::Work,
                next: Some(Phase::Break)
            })
        );
        assert_eq!(tm.pomodoro, Some((Phase::Break, 1)));
        assert_eq!(tm.left(25 * MIN), Some(5 * MIN));
        let f = tm.tick(30 * MIN, &cfg);
        assert_eq!(
            f,
            Some(Finished::Phase {
                ended: Phase::Break,
                next: None
            })
        );
        assert_eq!(tm.countdown, Countdown::Done);
        // dalsie kolo; po 2. pracovnom kole dlha prestavka
        tm.pomodoro_start(31 * MIN, &cfg);
        let f = tm.pomodoro_skip(32 * MIN, &cfg);
        assert_eq!(
            f,
            Some(Finished::Phase {
                ended: Phase::Work,
                next: Some(Phase::LongBreak)
            })
        );
        assert_eq!(tm.left(32 * MIN), Some(15 * MIN));
        // obycajny odpocet pomodoro zrusi
        tm.start(40 * MIN, MIN);
        assert_eq!(tm.pomodoro, None);
    }

    #[test]
    fn the_stopwatch_measures_pauses_and_records_laps() {
        let mut sw = Stopwatch::default();
        sw.toggle(1000);
        sw.lap(4000);
        sw.toggle(6000);
        assert_eq!(sw.elapsed(999_999), 5000);
        sw.lap(999_999);
        assert_eq!(sw.laps, vec![3000], "pozastavene stopky kolo nezaznamenaju");
        sw.toggle(10_000);
        assert_eq!(sw.elapsed(12_500), 7500);
        sw.reset();
        assert_eq!(sw, Stopwatch::default());
    }

    #[test]
    fn times_are_formatted_and_ticks_land_on_second_boundaries() {
        assert_eq!(fmt_ms(0, true), "0:00");
        assert_eq!(fmt_ms(1, true), "0:01");
        assert_eq!(fmt_ms(59_001, true), "1:00");
        assert_eq!(fmt_ms(59_999, false), "0:59");
        assert_eq!(fmt_ms(3_600_000 + 61_000, false), "1:01:01");
        let mut tm = Timers::new();
        tm.start(0, 10_500);
        // dalsia zmena zobrazenia: 10,5 s -> 10 s o 500 ms
        assert_eq!(tm.next_tick_ms(0), Some(500));
        tm.reset();
        tm.stopwatch.toggle(250);
        assert_eq!(tm.next_tick_ms(250), Some(750));
    }
}
