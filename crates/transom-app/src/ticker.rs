//! Riadenie posuvneho textu (marquee) v aplikacii: jeden spolocny cas animacie, geometria a posun **pre kazdu
//! listu zvlast** (monitory maju rozne sirky aj DPI, takze pretecenie byva rozne) a rozhodnutie, co a kedy
//! prekreslit. Zivy stav animacie nikdy nie je sucastou `StatusItem` - inak by kazdy snimok zmenil polozky a
//! prekreslila by sa cela lista (pozri `App::update`). Casovac (`T_MARQUEE`) sa planuje len ak sa nieco hybe;
//! pocas pauz na koncoch textu a pri statickom texte nebezi ziadny.
use std::collections::{HashMap, HashSet};
use transom_core::marquee::{self, Params, Policy, MIN_READABLE_LP};

/// Geometria posuvneho textu na jednej liste (z posledneho kreslenia).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    /// Cast v polozke (pre `MarqueeDraw`).
    pub part: usize,
    /// Miesto textu v px listy a o kolko je text sirsi.
    pub x: i32,
    pub w: i32,
    pub overflow_px: i32,
    /// Mierka DPI listy (rychlost v px = lp/s * mierka).
    pub scale: f32,
}

/// Co treba po kroku animacie urobit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    /// Useky listy (monitor, x, sirka), ktore treba prekreslit (zmenil sa posun alebo sa animacia skoncila).
    pub repaint: Vec<(String, i32, i32)>,
    /// Za kolko ms dalsi krok; `None` = nic sa nehybe, casovac netreba.
    pub next_ms: Option<u32>,
    /// Aspon jeden text sa prave pohybuje (nie pauza): vtedy treba jemne rozlisenie casovaca (`fine_timer`).
    pub moving: bool,
}

/// Meranie plynulosti posuvneho textu (`--ticker-stats`): casy skutocne dokoncenych kreslenia a posuny medzi nimi.
/// Slucky bez pohybu (pauzy) sa nepocitaju (medzera > `GAP_US`). Len diagnostika; bez nej sa nic nezbiera.
#[derive(Default, Debug)]
pub struct FrameStats {
    last: Option<(std::time::Instant, i32)>,
    intervals_us: Vec<u32>,
    deltas: Vec<i32>,
}

/// Medzery dlhsie nez toto (pauza, restart) nie su sucastou plynuleho pohybu.
const GAP_US: u128 = 200_000;

impl FrameStats {
    pub fn record(&mut self, now: std::time::Instant, offset: i32) {
        if let Some((t, o)) = self.last {
            let us = now.duration_since(t).as_micros();
            if us <= GAP_US && offset != o {
                self.intervals_us.push(us as u32);
                self.deltas.push((offset - o).abs());
            }
        }
        self.last = Some((now, offset));
    }

    fn pct(sorted: &[u32], p: f64) -> u32 {
        if sorted.is_empty() {
            return 0;
        }
        sorted[((sorted.len() - 1) as f64 * p).round() as usize]
    }

    /// Jednoriadkovy suhrn: pocet snimok, interval (priemer / smerodajna odchylka / p50 / p95 / max v ms) a rozdelenie
    /// posunu na snimku v px (1 / 2 / 3+). Plynuly pohyb = nizka odchylka intervalu a posuny 1-2 px.
    pub fn summary(&self) -> String {
        let n = self.intervals_us.len();
        if n == 0 {
            return "ziadne snimky pohybu".into();
        }
        let mean = self.intervals_us.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
        let var = self
            .intervals_us
            .iter()
            .map(|&v| (v as f64 - mean).powi(2))
            .sum::<f64>()
            / n as f64;
        let mut s = self.intervals_us.clone();
        s.sort_unstable();
        let ms = |us: f64| us / 1000.0;
        let d = |f: &dyn Fn(i32) -> bool| self.deltas.iter().filter(|&&v| f(v)).count();
        format!(
            "snimok={n} interval_ms: priemer={:.1} odchylka={:.1} p50={:.1} p95={:.1} max={:.1}; posun_px: 1={} 2={} 3+={}",
            ms(mean),
            ms(var.sqrt()),
            ms(Self::pct(&s, 0.5) as f64),
            ms(Self::pct(&s, 0.95) as f64),
            ms(*s.last().unwrap() as f64),
            d(&|v| v == 1),
            d(&|v| v == 2),
            d(&|v| v >= 3),
        )
    }
}

#[derive(Default)]
pub struct Ticker {
    /// Diagnostika plynulosti (`--ticker-stats`): podla listy.
    stats: Option<HashMap<String, FrameStats>>,
    started: Option<std::time::Instant>,
    /// (text, prehrava sa) - zmena restartuje animaciu od zaciatku.
    key: Option<(String, bool)>,
    geom: HashMap<String, Geom>,
    /// Posun, ktory sa naposledy poziadal o vykreslenie, podla listy.
    last_off: HashMap<String, i32>,
    /// Listy, na ktorych sa text posledny krok animoval (po skonceni treba obnovit staticky text s "...").
    animating: HashSet<String>,
}

impl Ticker {
    /// Zapne zber plynulosti (`--ticker-stats`).
    pub fn enable_stats(&mut self) {
        self.stats = Some(HashMap::new());
    }

    /// Suhrn plynulosti po listach (prazdny retazec, ak je zber vypnuty).
    pub fn stats_summary(&self) -> String {
        let Some(st) = &self.stats else {
            return String::new();
        };
        let mut v: Vec<_> = st.iter().collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v.iter()
            .map(|(m, s)| {
                format!(
                    "[{}] {}",
                    m.chars()
                        .rev()
                        .take(8)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>(),
                    s.summary()
                )
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Ms od zaciatku animacie (0, ak este nebezala).
    pub fn elapsed_ms(&self) -> u64 {
        self.started
            .map_or(0, |s| s.elapsed().as_millis().min(u64::MAX as u128) as u64)
    }

    /// Restartuje animaciu od zaciatku (zmena textu, stavu prehravania, nastaveni).
    pub fn reset(&mut self) {
        self.started = Some(std::time::Instant::now());
        self.last_off.clear();
    }

    /// Nastavi kluc (text + prehrava sa); pri zmene restartuje. Vrati `true`, ak sa zmenil.
    pub fn set_key(&mut self, key: Option<(String, bool)>) -> bool {
        if self.key == key {
            return false;
        }
        self.key = key;
        self.reset();
        true
    }

    /// Zapise geometriu z kreslenia (`None` = text na liste nepretieka). Vrati `true`, ak sa zmenila
    /// (aplikacia vtedy znova vyhodnoti, ci treba casovac).
    pub fn set_geom(&mut self, monitor: &str, g: Option<Geom>) -> bool {
        let old = self.geom.get(monitor).copied();
        if old == g {
            return false;
        }
        match g {
            Some(g) => {
                self.geom.insert(monitor.to_string(), g);
            }
            None => {
                self.geom.remove(monitor);
                self.last_off.remove(monitor);
            }
        }
        true
    }

    pub fn forget(&mut self, monitor: &str) {
        self.geom.remove(monitor);
        self.last_off.remove(monitor);
        self.animating.remove(monitor);
    }

    /// Posuva sa text v tomto okne? Politika + **minimalna citatelna sirka okna**: v uzsom okne ostane staticky useky
    /// textu s "..." (posuvat niekolko pismen nema zmysel).
    fn animates(policy: &Policy, g: &Geom) -> bool {
        policy.animates(g.overflow_px) && g.w as f32 >= MIN_READABLE_LP as f32 * g.scale
    }

    fn params(g: &Geom, speed_lp_s: f32, rtl: bool) -> Params {
        Params::new(speed_lp_s * g.scale, rtl)
    }

    /// Aktualny posun na liste, ak sa tam text prave animuje (inak `None` = staticky text s "...").
    pub fn offset(
        &self,
        monitor: &str,
        policy: &Policy,
        speed_lp_s: f32,
        rtl: bool,
        elapsed_ms: u64,
    ) -> Option<(usize, i32)> {
        let g = self.geom.get(monitor)?;
        Self::animates(policy, g).then(|| {
            let f = marquee::frame(elapsed_ms, g.overflow_px, &Self::params(g, speed_lp_s, rtl));
            (g.part, f.offset_px)
        })
    }

    /// Zaznamena, ake posuny sa prave kreslia (paint), aby krok nevyziadal zbytocne prekreslenie.
    pub fn painted(&mut self, monitor: &str, offset: Option<i32>) {
        match offset {
            Some(o) => {
                if let Some(st) = self.stats.as_mut() {
                    st.entry(monitor.to_string())
                        .or_default()
                        .record(std::time::Instant::now(), o);
                }
                self.last_off.insert(monitor.to_string(), o);
            }
            None => {
                self.last_off.remove(monitor);
            }
        }
    }

    /// Jeden krok animacie v case `elapsed_ms`: ktore useky prekreslit a kedy znova. Listy v `skip` (napr. skryte
    /// plnou obrazovkou) sa preskocia. Pocas pauzy na konci textu je `next_ms` zostatok pauzy a nic sa neprekresluje.
    pub fn step(
        &mut self,
        policy: &Policy,
        speed_lp_s: f32,
        rtl: bool,
        elapsed_ms: u64,
        skip: &HashSet<String>,
    ) -> Step {
        let mut out = Step::default();
        let mut now_animating = HashSet::new();
        let mut monitors: Vec<_> = self.geom.keys().cloned().collect();
        monitors.sort();
        for m in monitors {
            let g = self.geom[&m];
            if skip.contains(&m) || !Self::animates(policy, &g) {
                continue;
            }
            let f = marquee::frame(
                elapsed_ms,
                g.overflow_px,
                &Self::params(&g, speed_lp_s, rtl),
            );
            now_animating.insert(m.clone());
            out.moving |= f.moving;
            if self.last_off.get(&m) != Some(&f.offset_px) {
                self.last_off.insert(m.clone(), f.offset_px);
                out.repaint.push((m.clone(), g.x, g.w));
            }
            out.next_ms = match (out.next_ms, f.next_ms) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        // listy, na ktorych sa animacia prave skoncila (pauza, vypnutie, text sa zmestil): obnovit staticky text
        for m in self.animating.difference(&now_animating) {
            if let Some(g) = self.geom.get(m) {
                out.repaint.push((m.clone(), g.x, g.w));
            }
            self.last_off.remove(m);
        }
        self.animating = now_animating;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy {
            enabled: true,
            reduced_motion: false,
            playing: true,
            scroll_when_paused: false,
        }
    }

    fn g(part: usize, x: i32, ov: i32) -> Geom {
        Geom {
            part,
            x,
            w: 200,
            overflow_px: ov,
            scale: 1.0,
        }
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    /// 100 px/s = `Fast`-ish; v testoch pouzivame lp/s = 100 pri mierke 1.0.
    const V: f32 = 100.0;

    #[test]
    fn nothing_moves_without_overflow_or_geometry() {
        let mut t = Ticker::default();
        assert_eq!(t.step(&policy(), V, false, 0, &none()), Step::default());
        t.set_geom("a", Some(g(1, 40, 0)));
        let s = t.step(&policy(), V, false, 500, &none());
        assert_eq!(
            (s.repaint.len(), s.next_ms),
            (0, None),
            "text sa zmesti = ziadny casovac"
        );
        assert_eq!(t.offset("a", &policy(), V, false, 0), None);
    }

    #[test]
    fn first_step_paints_start_then_pause_needs_no_frames_and_motion_paints_only_the_text_span() {
        let mut t = Ticker::default();
        t.set_geom("a", Some(g(1, 40, 200)));
        // prvy krok: nic nebolo nakreslene -> prekreslit usek textu; pauza 2000 ms
        let s = t.step(&policy(), V, false, 0, &none());
        assert_eq!(s.repaint, vec![("a".into(), 40, 200)]);
        assert_eq!(s.next_ms, Some(2000));
        // paint zaznamena posun; dalsi krok v pauze nic neprekresluje
        t.painted("a", Some(0));
        let s = t.step(&policy(), V, false, 1500, &none());
        assert!(s.repaint.is_empty(), "pauza: ziadne prekreslenie");
        assert_eq!(s.next_ms, Some(500));
        // pohyb: prekresluje sa len usek textu (x, sirka), nie cela lista
        let s = t.step(&policy(), V, false, 3000, &none());
        assert_eq!(s.repaint, vec![("a".into(), 40, 200)]);
        assert_eq!(s.next_ms, Some(transom_core::marquee::FRAME_MS));
    }

    #[test]
    fn each_monitor_has_its_own_overflow_and_span() {
        let mut t = Ticker::default();
        t.set_geom("wide", Some(g(1, 40, 0))); // zmesti sa
        t.set_geom("narrow", Some(g(1, 40, 200)));
        let s = t.step(&policy(), V, false, 3000, &none());
        assert_eq!(s.repaint.len(), 1);
        assert_eq!(s.repaint[0].0, "narrow");
        // ina sirka okna na inom monitore: iny posun v tom istom case
        t.set_geom("wide", Some(g(1, 40, 400)));
        // (stala rychlost: kratsi text dobehne skor - tesne pred jeho koncom je uz v dobehu, dlhsi este ide naplno)
        let a = t.offset("narrow", &policy(), V, false, 4300).unwrap().1;
        let b = t.offset("wide", &policy(), V, false, 4300).unwrap().1;
        assert!(b < a * 2 && b != a, "{a} {b}");
    }

    #[test]
    fn stops_and_restores_static_text_when_policy_turns_off() {
        let mut t = Ticker::default();
        t.set_geom("a", Some(g(1, 40, 200)));
        t.step(&policy(), V, false, 3000, &none());
        assert!(t.offset("a", &policy(), V, false, 3000).is_some());
        // pozastavene prehravanie / vypnute / obmedzeny pohyb: staticky text, jedno posledne prekreslenie, ziadny casovac
        for off in [
            Policy {
                playing: false,
                ..policy()
            },
            Policy {
                enabled: false,
                ..policy()
            },
            Policy {
                reduced_motion: true,
                ..policy()
            },
        ] {
            let mut t2 = Ticker::default();
            t2.set_geom("a", Some(g(1, 40, 200)));
            t2.step(&policy(), V, false, 3000, &none());
            assert_eq!(t2.offset("a", &off, V, false, 3000), None);
            let s = t2.step(&off, V, false, 3100, &none());
            assert_eq!(s.repaint, vec![("a".into(), 40, 200)], "obnovit text s ...");
            assert_eq!(s.next_ms, None);
            let s = t2.step(&off, V, false, 3200, &none());
            assert_eq!(s, Step::default(), "uz nic");
        }
        // pri pozastaveni a povolenom "posuvat aj pri pauze" sa hybe dalej
        let p = Policy {
            playing: false,
            scroll_when_paused: true,
            ..policy()
        };
        assert!(t.offset("a", &p, V, false, 3000).is_some());
    }

    #[test]
    fn hidden_monitors_are_skipped() {
        let mut t = Ticker::default();
        t.set_geom("a", Some(g(1, 40, 200)));
        t.set_geom("b", Some(g(1, 40, 200)));
        let skip: HashSet<String> = ["a".to_string()].into();
        let s = t.step(&policy(), V, false, 3000, &skip);
        assert_eq!(
            s.repaint.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            vec!["b"]
        );
        let all: HashSet<String> = ["a".to_string(), "b".to_string()].into();
        let mut t3 = Ticker::default();
        t3.set_geom("a", Some(g(1, 40, 200)));
        assert_eq!(
            t3.step(&policy(), V, false, 3000, &all).next_ms,
            None,
            "vsetky listy skryte = ziadny casovac"
        );
    }

    #[test]
    fn key_change_restarts_and_geometry_change_is_reported_once() {
        let mut t = Ticker::default();
        assert!(t.set_key(Some(("Song".into(), true))));
        assert!(!t.set_key(Some(("Song".into(), true))));
        assert!(
            t.set_key(Some(("Song".into(), false))),
            "pozastavenie je zmena"
        );
        assert!(t.set_key(Some(("Other".into(), false))));
        assert!(t.set_key(None));
        assert!(t.set_geom("a", Some(g(1, 40, 200))));
        assert!(
            !t.set_geom("a", Some(g(1, 40, 200))),
            "rovnaka geometria nie je zmena (nezacyklit sa)"
        );
        assert!(t.set_geom("a", Some(g(1, 40, 201))));
        assert!(t.set_geom("a", None));
        assert!(!t.set_geom("a", None));
    }

    #[test]
    fn direction_changes_the_start_position() {
        let mut t = Ticker::default();
        t.set_geom("a", Some(g(1, 40, 200)));
        assert_eq!(t.offset("a", &policy(), V, false, 0).unwrap().1, 0);
        assert_eq!(t.offset("a", &policy(), V, true, 0).unwrap().1, 200);
    }

    #[test]
    fn speed_scales_with_dpi_so_the_duration_stays_the_same() {
        let mut t = Ticker::default();
        // 200 % DPI: pretecenie aj rychlost su dvojnasobne -> rovnaky podiel v rovnakom case
        t.set_geom(
            "a",
            Some(Geom {
                scale: 1.0,
                w: 300,
                ..g(1, 0, 100)
            }),
        );
        t.set_geom(
            "b",
            Some(Geom {
                scale: 2.0,
                w: 600,
                ..g(1, 0, 200)
            }),
        );
        let a = t.offset("a", &policy(), 50.0, false, 3000).unwrap().1;
        let b = t.offset("b", &policy(), 50.0, false, 3000).unwrap().1;
        assert!((b - 2 * a).abs() <= 2, "{a} {b}");
    }

    #[test]
    fn frame_stats_ignore_pauses_and_describe_cadence() {
        let mut st = FrameStats::default();
        let t0 = std::time::Instant::now();
        let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
        // plynuly pohyb: 20 ms intervaly, 1 px; potom 2 s pauza (sa nepocita); potom pohyb s posunom 3 px
        for i in 0..11u64 {
            st.record(at(i * 20), i as i32);
        }
        st.record(at(200 + 2000), 10); // pauza, ziadny posun
        st.record(at(2220), 13);
        let s = st.summary();
        assert!(s.contains("snimok=11"), "{s}");
        assert!(
            s.contains("priemer=20.0") && s.contains("odchylka=0.0"),
            "{s}"
        );
        assert!(s.contains("posun_px: 1=10 2=0 3+=1"), "{s}");
        assert_eq!(FrameStats::default().summary(), "ziadne snimky pohybu");
    }

    #[test]
    fn a_window_narrower_than_the_readable_minimum_stays_static() {
        let mut t = Ticker::default();
        let narrow = |w: i32, scale: f32| Geom {
            part: 1,
            x: 40,
            w,
            overflow_px: 300,
            scale,
        };
        // 119 lp pri 100 % = 119 px: pod hranicou 120 lp -> ziadny pohyb ani casovac, staticky useky s "..."
        t.set_geom("a", Some(narrow(119, 1.0)));
        assert_eq!(t.offset("a", &policy(), V, false, 3000), None);
        let s = t.step(&policy(), V, false, 3000, &none());
        assert_eq!((s.repaint.len(), s.next_ms, s.moving), (0, None, false));
        // 120 lp: uz sa posuva
        t.set_geom("a", Some(narrow(120, 1.0)));
        assert!(t.offset("a", &policy(), V, false, 3000).is_some());
        // hranica sa skaluje s DPI: 200 % -> 240 px
        t.set_geom("a", Some(narrow(239, 2.0)));
        assert_eq!(t.offset("a", &policy(), V, false, 3000), None);
        t.set_geom("a", Some(narrow(240, 2.0)));
        let s = t.step(&policy(), V, false, 3000, &none());
        assert!(s.moving && s.next_ms.is_some());
    }

    #[test]
    fn step_reports_motion_only_while_moving_not_in_pauses() {
        let mut t = Ticker::default();
        t.set_geom("a", Some(g(1, 40, 200)));
        assert!(
            !t.step(&policy(), V, false, 500, &none()).moving,
            "pauza na zaciatku"
        );
        assert!(t.step(&policy(), V, false, 3000, &none()).moving, "pohyb");
        assert!(
            !t.step(&policy(), V, false, 4800, &none()).moving,
            "pauza na konci"
        );
    }
}
