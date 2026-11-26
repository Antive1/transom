//! Plynuly posun dlheho textu (marquee) ako ciste funkcie casu: bez Win32, bez stavu, bez casovaca.
//!
//! Okno (viditelna cast textu) sa posuva od zaciatku textu ku koncu a naspat (ping-pong) a na oboch koncoch
//! ostane `pause_ms`, aby sa dal text docitat. Pohyb ma sinusovy rozbeh a dobeh (smoothstep); trvanie pohybu
//! vyplyva z rychlosti, takze dlhsi text trva dlhsie, nie rychlejsie.
//!
//! "Posun" (`offset_px`) je o kolko px je text posunuty **dolava** oproti zaciatku okna: 0 = viditelny zaciatok
//! textu, `overflow` = viditelny koniec textu.

/// Pauza na kazdom konci (ms): dost na precitanie niekolkych slov.
pub const PAUSE_MS: u32 = 2000;
/// Najkratsi interval medzi snimkami pocas pohybu (ms) = hornu hranicu ~60 snimok/s; skutocny interval je cas do
/// **dalsej zmeny celeho pixelu** (pri pomalom posune dlhsi, aby sa nekreslilo bez zmeny), najviac `FRAME_MAX_MS`.
pub const FRAME_MS: u32 = 16;
/// Najdlhsi interval medzi snimkami pocas pohybu (ms) - poistka pri dobehu, ked sa posun meni velmi pomaly.
pub const FRAME_MAX_MS: u32 = 60;
/// Najkratsie trvanie jedneho pohybu (ms), aby aj mierne pretecenie neskocilo.
pub const MIN_MOVE_MS: u32 = 600;
/// Rozbeh a dobeh (ms): text sa rozbehne na ustalenu rychlost a na konci plynulo zastavi; medzitym ide **stalou
/// rychlostou** (nie sinusovo - plna "smoothstep" krivka robila stred pohybu 1,5x rychlejsi a koniec pomaly).
pub const RAMP_MS: u32 = 400;
/// Najmensia sirka okna (lp), v ktorej sa text este posuva. Uzsie okno ukaze len staticky useky textu s "..." - posuvat
/// niekolko pismen nema zmysel (necitatelne). Prepocet na px robi volajuci (`scale`).
pub const MIN_READABLE_LP: i32 = 120;

/// Smer: `rtl = false` = citanie zlava doprava (zacina zaciatkom, text sa pohybuje doprava->dolava).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Rychlost v px/s (uz prenasobena mierkou DPI).
    pub speed_px_s: f32,
    pub pause_ms: u32,
    /// Opacny smer: zacina koncom textu a okno sa posuva k zaciatku.
    pub rtl: bool,
}

impl Params {
    pub fn new(speed_px_s: f32, rtl: bool) -> Self {
        Params {
            speed_px_s: speed_px_s.max(1.0),
            pause_ms: PAUSE_MS,
            rtl,
        }
    }
}

/// Jeden snimok animacie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Posun textu dolava v px (0..=overflow).
    pub offset_px: i32,
    /// Za kolko ms treba dalsi snimok; `None` = animacia nema co kreslit (text sa zmesti).
    /// Pocas pauzy je to zostavajuci cas pauzy (ziadne medzisnimky), pocas pohybu cas do dalsej zmeny pixelu.
    pub next_ms: Option<u32>,
    /// Prave sa pohybuje (nie pauza).
    pub moving: bool,
}

/// O kolko px je text sirsi nez okno (0 = zmesti sa, nic sa neanimuje).
pub fn overflow_px(text_w: i32, avail_w: i32) -> i32 {
    (text_w - avail_w.max(0)).max(0)
}

/// Rozhodnutie, ci sa ma animovat: povolene v nastaveniach, bez obmedzenia pohybu v systeme, prehravanie
/// (alebo vyslovne povolene aj pri pauze) a text sa nezmesti.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub enabled: bool,
    /// Windows: animacie su vypnute (`SPI_GETCLIENTAREAANIMATION` = false) -> staticky text.
    pub reduced_motion: bool,
    pub playing: bool,
    pub scroll_when_paused: bool,
}

impl Policy {
    pub fn animates(&self, overflow: i32) -> bool {
        self.enabled
            && !self.reduced_motion
            && (self.playing || self.scroll_when_paused)
            && overflow > 0
    }
}

/// Trvanie jedneho pohybu (ms) a rozbehu, plus skutocna rychlost (px/ms): cas pohybu vyplyva z rychlosti, nie je
/// kratsi nez `MIN_MOVE_MS` (pri malom preteceni sa rychlost zniZi, aby posun neskocil).
fn leg(overflow: i32, p: &Params) -> (f32, f32, f32) {
    let ov = overflow as f32;
    let mut cruise_ms = ov / (p.speed_px_s.max(1.0) / 1000.0);
    let r = (RAMP_MS as f32).min(cruise_ms);
    if cruise_ms + r < MIN_MOVE_MS as f32 {
        cruise_ms = MIN_MOVE_MS as f32 - r;
    }
    let r = (RAMP_MS as f32).min(cruise_ms);
    (cruise_ms + r, r, ov / cruise_ms)
}

/// Poloha (0..=overflow, od zaciatku textu) v case `t` od zaciatku pohybu: lichobeznikovy profil rychlosti.
fn leg_pos(t: f32, overflow: i32, m: f32, r: f32, v: f32) -> f32 {
    let ov = overflow as f32;
    let t = t.clamp(0.0, m);
    let pos = if r <= 0.0 {
        v * t
    } else if t < r {
        v * t * t / (2.0 * r)
    } else if t < m - r {
        v * (r / 2.0 + (t - r))
    } else {
        ov - v * (m - t) * (m - t) / (2.0 * r)
    };
    pos.clamp(0.0, ov)
}

/// (poloha v px podla smeru, pohybuje sa, zostavajuci cas segmentu v ms) v case `elapsed_ms`.
fn state_at(elapsed_ms: u64, overflow: i32, p: &Params) -> (i32, bool, u64) {
    let pause = p.pause_ms as u64;
    let (m, r, v) = leg(overflow, p);
    let move_ms = m.ceil() as u64;
    let cycle = 2 * (pause + move_ms);
    let t = elapsed_ms % cycle;
    let (pos, moving, left) = if t < pause {
        (0.0, false, pause - t)
    } else if t < pause + move_ms {
        (leg_pos((t - pause) as f32, overflow, m, r, v), true, 0)
    } else if t < 2 * pause + move_ms {
        (overflow as f32, false, 2 * pause + move_ms - t)
    } else {
        let f = (t - 2 * pause - move_ms) as f32;
        (overflow as f32 - leg_pos(f, overflow, m, r, v), true, 0)
    };
    let mut off = (pos.round() as i32).clamp(0, overflow);
    if p.rtl {
        off = overflow - off;
    }
    (off, moving, left)
}

/// Snimok v case `elapsed_ms` od zaciatku animacie (vynuluje sa pri zmene textu, stavu prehravania, nastaveni).
pub fn frame(elapsed_ms: u64, overflow: i32, p: &Params) -> Frame {
    if overflow <= 0 {
        return Frame {
            offset_px: 0,
            next_ms: None,
            moving: false,
        };
    }
    let (off, moving, left) = state_at(elapsed_ms, overflow, p);
    let next = if moving {
        // dalsi snimok az vtedy, ked sa posun naozaj zmeni o cely pixel (nie kazdych 33 ms naslepo), najskor za FRAME_MS
        (FRAME_MS..=FRAME_MAX_MS)
            .find(|dt| state_at(elapsed_ms + *dt as u64, overflow, p).0 != off)
            .unwrap_or(FRAME_MAX_MS)
    } else {
        left.clamp(1, u32::MAX as u64) as u32
    };
    Frame {
        offset_px: off,
        next_ms: Some(next),
        moving,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(rtl: bool) -> Params {
        Params::new(100.0, rtl) // 100 px/s
    }

    /// overflow 200 px pri 100 px/s: rovna cast 2000 ms + rozbeh/dobeh 400 ms = pohyb 2400 ms, pauza 2000 ms -> cyklus 8800 ms
    const OV: i32 = 200;
    const MOVE: u64 = 2400;
    const CYCLE: u64 = 2 * (2000 + MOVE);

    #[test]
    fn short_text_is_never_animated() {
        assert_eq!(overflow_px(100, 120), 0);
        assert_eq!(overflow_px(120, 120), 0, "presne sa zmesti");
        assert_eq!(overflow_px(121, 120), 1);
        assert_eq!(overflow_px(50, -5), 50, "zaporne okno sa berie ako 0");
        let f = frame(12_345, 0, &p(false));
        assert_eq!(
            f,
            Frame {
                offset_px: 0,
                next_ms: None,
                moving: false
            }
        );
        let f = frame(12_345, -3, &p(true));
        assert_eq!(f.next_ms, None);
    }

    #[test]
    fn ltr_starts_at_text_start_pauses_then_scrolls_to_end() {
        let pr = p(false);
        // pauza na zaciatku
        let f = frame(0, OV, &pr);
        assert_eq!((f.offset_px, f.moving), (0, false));
        assert_eq!(
            f.next_ms,
            Some(2000),
            "bez medzisnimok: dalsi snimok az na konci pauzy"
        );
        assert_eq!(frame(1999, OV, &pr).offset_px, 0);
        assert_eq!(frame(1999, OV, &pr).next_ms, Some(1));
        // pohyb: stred = polovica, koniec = overflow
        let mid = frame(2000 + MOVE / 2, OV, &pr);
        assert!(mid.moving && (mid.offset_px - 100).abs() <= 1, "{mid:?}");
        assert_eq!(
            mid.next_ms,
            Some(FRAME_MS),
            "100 px/s = zmena pixelu kazdych 10 ms, strop 60 snimok/s"
        );
        // pauza na konci: text docitatelny
        let end = frame(2000 + MOVE, OV, &pr);
        assert_eq!((end.offset_px, end.moving), (OV, false));
        assert_eq!(end.next_ms, Some(2000));
        // spat
        let back = frame(4000 + MOVE + MOVE / 2, OV, &pr);
        assert!(back.moving && (back.offset_px - 100).abs() <= 1);
        // cyklus sa opakuje
        assert_eq!(frame(CYCLE, OV, &pr), frame(0, OV, &pr));
        assert_eq!(frame(CYCLE + 3000, OV, &pr), frame(3000, OV, &pr));
    }

    #[test]
    fn rtl_mirrors_ltr() {
        for t in (0..16_000).step_by(37) {
            let a = frame(t, OV, &p(false));
            let b = frame(t, OV, &p(true));
            assert_eq!(a.offset_px + b.offset_px, OV, "t={t}");
            assert_eq!((a.moving, a.next_ms), (b.moving, b.next_ms));
        }
        assert_eq!(
            frame(0, OV, &p(true)).offset_px,
            OV,
            "rtl zacina koncom textu"
        );
    }

    #[test]
    fn offset_stays_in_bounds_and_moves_monotonically_per_leg() {
        for ov in [1, 7, 53, 200, 1000] {
            let pr = p(false);
            let move_ms = leg(ov, &pr).0.ceil() as u64;
            let mut prev = 0;
            for t in 0..(2 * (2000 + move_ms) + 50) {
                let f = frame(t, ov, &pr);
                assert!((0..=ov).contains(&f.offset_px), "ov={ov} t={t} {f:?}");
                if t > 2000 && t < 2000 + move_ms {
                    assert!(f.offset_px >= prev, "dopredu nikdy nekrati: ov={ov} t={t}");
                }
                if t < 2000 + move_ms {
                    prev = f.offset_px;
                }
            }
            // na konci pohybu je text presne na konci (nic sa nedotiahne a nezostane o pixel)
            assert_eq!(frame(2000 + move_ms, ov, &pr).offset_px, ov, "ov={ov}");
        }
    }

    #[test]
    fn speed_is_steady_in_the_middle_and_eases_only_at_the_ends() {
        // 100 px/s: v rovnej casti presne 10 px za 100 ms (stala rychlost), nie sinusova krivka
        let pr = p(false);
        let at = |t: u64| frame(2000 + t, 1000, &pr).offset_px;
        for t in [600u64, 1500, 3000, 6000] {
            let d = at(t + 100) - at(t);
            assert!((9..=11).contains(&d), "t={t}: {d} px za 100 ms");
        }
        // rozbeh: prvych 100 ms sa posunie len o zlomok toho, co v rovnej casti (0,1*100/2/400*100 ~ 1 px)
        assert!(at(100) - at(0) <= 2, "rozbeh: {}", at(100) - at(0));
        // dobeh: poslednych 100 ms (pred koncom pohybu) tiez len par px
        let m = leg(1000, &pr).0.round() as u64;
        assert!(at(m) - at(m - 100) <= 2, "dobeh: {}", at(m) - at(m - 100));
    }

    #[test]
    fn at_moderate_speed_every_frame_moves_exactly_one_pixel() {
        // 40 px/s -> zmena pixelu kazdych 25 ms; dalsi snimok sa planuje presne na tu zmenu (nie naslepo po 33 ms)
        let pr = Params::new(40.0, false);
        let mut t = 2000 + 600; // v rovnej casti
                                // prvych par snimkov sa fazovo "zachyti" na zmeny pixelov (pociatocny cas je lubovolny)
        for _ in 0..3 {
            t += frame(t, 1000, &pr).next_ms.unwrap() as u64;
        }
        let mut off = frame(t, 1000, &pr).offset_px;
        for _ in 0..40 {
            let f = frame(t, 1000, &pr);
            assert_eq!(f.offset_px, off);
            let n = f.next_ms.unwrap();
            assert!((20..=30).contains(&n), "dalsi snimok za {n} ms");
            t += n as u64;
            let next = frame(t, 1000, &pr).offset_px;
            assert_eq!(next - off, 1, "kazdy snimok presne 1 px");
            off = next;
        }
    }

    #[test]
    fn longer_text_takes_longer_not_faster() {
        let pr = p(false);
        // pri dvojnasobnom pretecene je v tom istom case od zaciatku pohybu menej "hotovo"
        let a = frame(2000 + 1000, 200, &pr).offset_px as f32 / 200.0;
        let b = frame(2000 + 1000, 400, &pr).offset_px as f32 / 400.0;
        assert!(b < a, "{a} {b}");
        // minimalna dlzka pohybu aj pri malom preteceni (bez skoku)
        let f = frame(2000 + 100, 3, &pr);
        assert!(f.moving && f.offset_px <= 1, "{f:?}");
    }

    #[test]
    fn restarting_the_clock_restarts_at_the_beginning() {
        // zmena textu / stavu prehravania = nulovy cas -> zaciatok v pauze (okno ukazuje zaciatok textu)
        let pr = p(false);
        assert_eq!(frame(0, OV, &pr).offset_px, 0);
        assert_eq!(frame(0, OV, &p(true)).offset_px, OV);
    }

    #[test]
    fn policy_activation_pause_and_reduced_motion() {
        let base = Policy {
            enabled: true,
            reduced_motion: false,
            playing: true,
            scroll_when_paused: false,
        };
        assert!(base.animates(10));
        assert!(!base.animates(0), "text sa zmesti");
        assert!(
            !Policy {
                enabled: false,
                ..base
            }
            .animates(10),
            "vypnute v nastaveniach"
        );
        assert!(
            !Policy {
                reduced_motion: true,
                ..base
            }
            .animates(10),
            "obmedzeny pohyb = staticky text"
        );
        assert!(
            !Policy {
                playing: false,
                ..base
            }
            .animates(10),
            "pozastavene prehravanie = text stoji"
        );
        assert!(
            Policy {
                playing: false,
                scroll_when_paused: true,
                ..base
            }
            .animates(10),
            "ak je to povolene"
        );
        assert!(!Policy {
            playing: false,
            scroll_when_paused: true,
            reduced_motion: true,
            ..base
        }
        .animates(10));
    }

    #[test]
    fn frame_rate_is_capped_and_pauses_need_no_frames() {
        // pocas celeho cyklu je dalsi snimok bud v rozsahu pohybu (16-60 ms) alebo zvysok pauzy (ziadne medzisnimky)
        let pr = p(false);
        let mut wakes = 0u32;
        let mut t = 0u64;
        while t < CYCLE {
            let f = frame(t, OV, &pr);
            let n = f.next_ms.unwrap();
            if f.moving {
                assert!((FRAME_MS..=FRAME_MAX_MS).contains(&n), "{n}");
            } else {
                assert!((1..=2000).contains(&n));
            }
            wakes += 1;
            t += n as u64;
        }
        // 2 pohyby po 2400 ms pri 16 ms (~300 snimok) + par prebudeni na hraniciach pauz; nie 60 Hz celych 8800 ms (550)
        assert!((250..340).contains(&wakes), "{wakes}");
    }
}
