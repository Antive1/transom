//! Vybezok s prehravanou hudbou: po prejdeni mysou nad mediami v liste sa pod nimi vysunie obal albumu, nazov,
//! interpret, priebeh skladby a ovladanie (spat, prehrat / pozastavit, dalej). Klik na obal aktivuje aplikaciu.
//! Cisty stav, kreslenie a vyhodnotenie mysi; okno je ten isty vybezok ako pri odlozenych suboroch.
use transom_core::item::{Action, ArtImage, Glyph};
use transom_core::media::{PlayState, SessionInfo};
use transom_platform::appbar::PointerKind;
use transom_render::design;
use transom_render::{Color, Font, HAlign, Painter, Rect, TextStyle, Theme};

/// Vyska vybezku (lp).
pub const H_LP: i32 = 132;
const ART_LP: i32 = 96;
const PAD_LP: i32 = 14;
const BTN_LP: i32 = 34;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Art,
    Prev,
    Toggle,
    Next,
    /// Casova os (klik / tahanie = presun).
    Seek,
}

#[derive(Default)]
pub struct MediaNotch {
    pub hover: Option<Hit>,
    press: Option<Hit>,
    hits: Vec<(Rect, Hit)>,
    /// Casova os (len ked relacia dovoli presun): (x, sirka) v px.
    track: Option<(i32, i32)>,
    /// Tahanie po casovej osi: zobrazena cast 0..=1 (kym sa nepusti).
    drag: Option<f32>,
    /// Pusteny presun: cast 0..=1 (vezme volajuci).
    pub seek: Option<f32>,
}

fn st(size: f32, bold: bool, color: Color, align: HAlign) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    }
}

/// Cas "m:ss" (pri hodine a viac "h:mm:ss").
pub fn clock(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

impl MediaNotch {
    pub fn paint(
        &mut self,
        p: &mut dyn Painter,
        th: &Theme,
        info: &SessionInfo,
        art: Option<&ArtImage>,
    ) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let sz = p.size();
        self.hits.clear();
        p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
        let b = px(1).max(1);
        p.fill_rect(Rect::new(b, 0, sz.w - 2 * b, sz.h - b), th.background);
        let pad = px(PAD_LP);
        // obal
        let n = px(ART_LP);
        let ar = Rect::new(pad, (sz.h - n) / 2, n, n);
        match art {
            Some(a) => p.blit(ar, &transom_render::scene::cover_pixels(a, n as u32, 0)),
            None => {
                p.fill_round_rect(ar, px(design::radius::CONTROL), th.placeholder);
                p.draw_text(
                    "\u{E8D6}",
                    ar,
                    &TextStyle {
                        font: Font::Icons,
                        size_lp: 28.0,
                        bold: false,
                        color: th.muted,
                        align: HAlign::Center,
                    },
                );
            }
        }
        self.hits.push((ar, Hit::Art));
        // texty
        let x = ar.x + n + px(14);
        let w = (sz.w - x - pad).max(px(40));
        let title = crate::audio_pop::fit(
            p,
            &info.title,
            &st(15.0, false, th.foreground, HAlign::Left),
            w,
        );
        p.draw_text(
            &title,
            Rect::new(x, ar.y, w, px(22)),
            &st(15.0, false, th.foreground, HAlign::Left),
        );
        let artist =
            crate::audio_pop::fit(p, &info.artist, &st(12.0, false, th.muted, HAlign::Left), w);
        p.draw_text(
            &artist,
            Rect::new(x, ar.y + px(22), w, px(18)),
            &st(12.0, false, th.muted, HAlign::Left),
        );
        // priebeh
        let ty = ar.y + px(48);
        if let Some(tl) = info.timeline.valid_at(
            transom_core::media::now_filetime(),
            info.state == PlayState::Playing,
        ) {
            let f = self.drag.unwrap_or(if tl.duration_s > 0.0 {
                (tl.position_s / tl.duration_s).clamp(0.0, 1.0) as f32
            } else {
                0.0
            });
            let seekable = info.controls.seek && tl.duration_s > 0.0;
            let active = seekable && (self.drag.is_some() || self.hover == Some(Hit::Seek));
            let th_h = if active { px(6) } else { px(4) };
            let track = Rect::new(x, ty + px(8) - th_h / 2, w, th_h);
            p.fill_round_rect(track, th_h / 2, th.control_edge);
            p.fill_round_rect(
                Rect::new(x, track.y, ((w as f32) * f) as i32, track.h),
                th_h / 2,
                th.accent,
            );
            if active {
                // jazdec na pozicii
                let k = px(12);
                let cx = x + ((w as f32) * f) as i32;
                p.fill_round_rect(
                    Rect::new(cx - k / 2, track.y + track.h / 2 - k / 2, k, k),
                    k / 2,
                    th.foreground,
                );
            }
            self.track = None;
            if seekable {
                self.track = Some((x, w));
                // vyssia plocha na klik a tahanie nez samotna ciara
                self.hits.push((
                    Rect::new(x - px(6), ty - px(2), w + px(12), px(20)),
                    Hit::Seek,
                ));
            }
            let times = st(10.0, false, th.muted, HAlign::Left);
            let shown = if self.drag.is_some() {
                f as f64 * tl.duration_s
            } else {
                tl.position_s
            };
            p.draw_text(&clock(shown), Rect::new(x, ty + px(12), w, px(14)), &times);
            p.draw_text(
                &clock(tl.duration_s),
                Rect::new(x, ty + px(12), w, px(14)),
                &TextStyle {
                    align: HAlign::Right,
                    ..times
                },
            );
        }
        // ovladanie
        let by = ar.y + n - px(BTN_LP);
        let bw = px(BTN_LP);
        let cx = x + w / 2;
        let playing = info.state == PlayState::Playing;
        let buttons = [
            (Hit::Prev, Glyph::Prev, info.controls.prev),
            (
                Hit::Toggle,
                if playing { Glyph::Pause } else { Glyph::Play },
                info.controls.toggle || info.controls.play || info.controls.pause,
            ),
            (Hit::Next, Glyph::Next, info.controls.next),
        ];
        for (i, (hit, glyph, enabled)) in buttons.into_iter().enumerate() {
            let r = Rect::new(cx + (i as i32 - 1) * (bw + px(10)) - bw / 2, by, bw, bw);
            if enabled && self.hover == Some(hit) {
                p.fill_round_rect(r, bw / 2, th.control_hover);
            }
            p.draw_text(
                &transom_render::scene::glyph_char(glyph).to_string(),
                r,
                &TextStyle {
                    font: Font::Icons,
                    size_lp: if hit == Hit::Toggle { 18.0 } else { 14.0 },
                    bold: false,
                    color: if enabled { th.foreground } else { th.disabled },
                    align: HAlign::Center,
                },
            );
            if enabled {
                self.hits.push((r, hit));
            }
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, h)| *h)
    }

    /// Mys vo vybezku: (akcia, treba prekreslit). Akcia az pri uvolneni nad tym istym ovladacom.
    pub fn pointer(&mut self, kind: PointerKind, x: i32, y: i32) -> (Option<Action>, bool) {
        let here = if kind == PointerKind::Leave {
            None
        } else {
            self.hit(x, y)
        };
        let redraw = here != self.hover;
        self.hover = here;
        let act = match kind {
            PointerKind::Down => {
                self.press = here;
                if here == Some(Hit::Seek) {
                    self.drag = self.fraction(x);
                    return (None, true);
                }
                None
            }
            PointerKind::Move if self.drag.is_some() => {
                // tahanie: poloha sleduje mys aj mimo ciary
                self.drag = self.fraction(x);
                return (None, true);
            }
            PointerKind::Click if self.drag.is_some() => {
                self.seek = self.fraction(x).or(self.drag);
                self.drag = None;
                self.press = None;
                return (None, true);
            }
            PointerKind::Click => {
                let fire = self.press.take().filter(|p| Some(*p) == here);
                fire.and_then(|h| match h {
                    Hit::Art => Some(Action::MediaOpenSource),
                    Hit::Prev => Some(Action::MediaPrev),
                    Hit::Toggle => Some(Action::MediaToggle),
                    Hit::Next => Some(Action::MediaNext),
                    Hit::Seek => None,
                })
            }
            _ => {
                if kind == PointerKind::Leave {
                    self.press = None;
                    self.drag = None;
                }
                None
            }
        };
        (act, redraw)
    }

    /// Cast casovej osi pod `x` (0..=1).
    fn fraction(&self, x: i32) -> Option<f32> {
        let (x0, w) = self.track?;
        Some(((x - x0) as f32 / w.max(1) as f32).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::media::{Controls, Timeline};
    use transom_render::offscreen::Surface;
    use transom_render::GdiPainter;

    fn info() -> SessionInfo {
        SessionInfo {
            key: "Spotify.exe".into(),
            title: "An extremely long song title that never seems to end (Extended Remix)".into(),
            artist: "Some Artist".into(),
            state: PlayState::Playing,
            controls: Controls {
                pause: true,
                prev: true,
                next: true,
                toggle: true,
                ..Default::default()
            },
            timeline: Timeline {
                start: 0,
                end: 2_000_000_000,
                position: 650_000_000,
                last_updated: 0,
            },
            ..Default::default()
        }
    }

    #[test]
    fn the_times_read_naturally() {
        assert_eq!(clock(5.0), "0:05");
        assert_eq!(clock(65.4), "1:05");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn paints_without_clipped_text_and_the_buttons_fire_on_release() {
        let _g = crate::test_lang::guard();
        let (w, h) = (transom_platform::dragdrop::NOTCH_W_LP, H_LP);
        let surf = Surface::new(w, h);
        let mut g = GdiPainter::new(surf.dc, transom_render::Size { w, h }, 96);
        let mut a = transom_render::audit::Audit::new(&mut g, Theme::dark().background);
        a.allow_elide = vec!["…".into()];
        let mut n = MediaNotch::default();
        n.paint(&mut a, &Theme::dark(), &info(), None);
        a.finish();
        assert!(a.issues.is_empty(), "{:?}", a.issues);
        let r = n.hits.iter().find(|(_, h)| *h == Hit::Toggle).unwrap().0;
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        assert_eq!(n.pointer(PointerKind::Down, cx, cy).0, None);
        assert_eq!(
            n.pointer(PointerKind::Click, cx, cy).0,
            Some(Action::MediaToggle)
        );
        // stlacenie na jednom a uvolnenie mimo = nic
        n.pointer(PointerKind::Down, cx, cy);
        assert_eq!(n.pointer(PointerKind::Click, 1, 1).0, None);
        // zakazane tlacidlo (napr. bez "dalej") nie je klikatelne
        let mut i = info();
        i.controls.next = false;
        let mut n = MediaNotch::default();
        let surf = Surface::new(w, h);
        let mut g = GdiPainter::new(surf.dc, transom_render::Size { w, h }, 96);
        n.paint(&mut g, &Theme::dark(), &i, None);
        assert!(!n.hits.iter().any(|(_, h)| *h == Hit::Next));
    }
}
