//! Vlastny rám okna nastaveni (bez systemovej bielej hlavicky): rozlozenie hlavicky, hit-test, poradie fokusu tlacidiel a
//! minimalna velkost - **cisté funkcie bez Win32** (testovatelne; okno ich len vola z `WM_NCHITTEST` / kreslenia).
//!
//! Technika: okno ostava `WS_OVERLAPPEDWINDOW` (systemove menu, Alt+F4, Alt+Space, tien, Aero Snap a Windows 11 snap
//! layouts ostavaju), `WM_NCCALCSIZE` odstrani hlavicku (klientska oblast zacina na hornom okraji) a hlavicka sa kresli
//! v klientskej oblasti. Tlacidla vracaju `HTMINBUTTON` / `HTMAXBUTTON` / `HTCLOSE` (system tak ponuka snap layouts), zvysok
//! hlavicky `HTCAPTION` (tahanie, dvojklik = maximalizovat / obnovit, pravy klik = systemove menu).
//!
//! Cela obrazovka (F11) sa zamerne neriesi: v nastaveniach by len pridala treti stav okna bez prinosu a maximalizacia
//! (vratane Aero Snap) ostava skutocna a vratna.
use transom_render::Rect;

/// Vyska hlavicky (lp) - je to aj horny pas klientskej oblasti.
pub const CAPTION_LP: i32 = 40;
/// Sirka tlacidla hlavicky (lp).
pub const BTN_W_LP: i32 = 46;
/// Horny pas na zmenu velkosti vo vnutri klientskej oblasti (lp) - len ked okno nie je maximalizovane.
pub const RESIZE_LP: i32 = 5;
/// Sirka rohu na zmenu velkosti (lp).
pub const CORNER_LP: i32 = 12;
/// Minimalna velkost klientskej oblasti (lp).
pub const MIN_W_LP: i32 = 640;
pub const MIN_H_LP: i32 = 480;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Min,
    Max,
    Close,
}

/// Poradie tlacidiel (a poradie v pristupnosti).
pub const BTNS: [Btn; 3] = [Btn::Min, Btn::Max, Btn::Close];

impl Btn {
    pub fn index(self) -> usize {
        match self {
            Btn::Min => 0,
            Btn::Max => 1,
            Btn::Close => 2,
        }
    }
}

fn px(v: i32, dpi: u32) -> i32 {
    (v as f32 * dpi as f32 / 96.0).round() as i32
}

pub fn caption_px(dpi: u32) -> i32 {
    px(CAPTION_LP, dpi)
}

/// Rozlozenie hlavicky v px klientskej oblasti sirky `client_w`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geo {
    pub caption: Rect,
    pub min: Rect,
    pub max: Rect,
    pub close: Rect,
    /// Pravy okraj pre nazov (tesne pred tlacidlami).
    pub title_right: i32,
}

impl Geo {
    pub fn button(&self, b: Btn) -> Rect {
        match b {
            Btn::Min => self.min,
            Btn::Max => self.max,
            Btn::Close => self.close,
        }
    }
}

pub fn layout(client_w: i32, dpi: u32) -> Geo {
    let (cap, bw) = (caption_px(dpi), px(BTN_W_LP, dpi));
    let close = Rect::new(client_w - bw, 0, bw, cap);
    let max = Rect::new(client_w - 2 * bw, 0, bw, cap);
    let min = Rect::new(client_w - 3 * bw, 0, bw, cap);
    Geo {
        caption: Rect::new(0, 0, client_w, cap),
        min,
        max,
        close,
        title_right: min.x - px(8, dpi),
    }
}

/// Co je pod bodom klientskej oblasti (`x`, `y` v px).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Client,
    Caption,
    Btn(Btn),
    Top,
    TopLeft,
    TopRight,
}

fn inside(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

pub fn zone(x: i32, y: i32, client_w: i32, dpi: u32, maximized: bool) -> Zone {
    let g = layout(client_w, dpi);
    if y < 0 || y >= g.caption.h || x < 0 || x >= client_w {
        return Zone::Client;
    }
    // horny pas na zmenu velkosti ma prednost (aj nad tlacidlom v pravom hornom rohu)
    if !maximized && y < px(RESIZE_LP, dpi) {
        let corner = px(CORNER_LP, dpi);
        return if x < corner {
            Zone::TopLeft
        } else if x >= client_w - corner {
            Zone::TopRight
        } else {
            Zone::Top
        };
    }
    for b in BTNS {
        if inside(g.button(b), x, y) {
            return Zone::Btn(b);
        }
    }
    Zone::Caption
}

/// Najmensia velkost **okna** (px) pre `WM_GETOBJECT`/`WM_GETMINMAXINFO`: klientska oblast + lavy/pravy/dolny rám, najviac
/// velkost pracovnej plochy.
pub fn min_track(dpi: u32, frame_x: i32, frame_y: i32, work_w: i32, work_h: i32) -> (i32, i32) {
    (
        (px(MIN_W_LP, dpi) + 2 * frame_x).min(work_w.max(1)),
        (px(MIN_H_LP, dpi) + frame_y).min(work_h.max(1)),
    )
}

/// Systemova udalost tlacidla (`WM_SYSCOMMAND`): maximalizovat alebo obnovit podla stavu.
pub fn sys_command(b: Btn, maximized: bool) -> usize {
    match (b, maximized) {
        (Btn::Min, _) => 0xF020,     // SC_MINIMIZE
        (Btn::Max, false) => 0xF030, // SC_MAXIMIZE
        (Btn::Max, true) => 0xF120,  // SC_RESTORE
        (Btn::Close, _) => 0xF060,   // SC_CLOSE
    }
}

/// Znak ikony (Segoe Fluent Icons / MDL2): minimalizovat, maximalizovat, obnovit, zavriet.
pub fn glyph(b: Btn, maximized: bool) -> &'static str {
    match (b, maximized) {
        (Btn::Min, _) => "\u{E921}",
        (Btn::Max, false) => "\u{E922}",
        (Btn::Max, true) => "\u{E923}",
        (Btn::Close, _) => "\u{E8BB}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: i32 = 824;

    #[test]
    fn layout_scales_with_dpi_and_buttons_fill_the_right_edge() {
        for dpi in [96u32, 120, 144, 192] {
            let cw = px(W, dpi);
            let g = layout(cw, dpi);
            assert_eq!(g.close.x + g.close.w, cw, "close na pravom okraji");
            assert_eq!(g.max.x + g.max.w, g.close.x);
            assert_eq!(g.min.x + g.min.w, g.max.x);
            assert!(g.min.h == g.caption.h && g.caption.h == caption_px(dpi));
            assert!(g.title_right < g.min.x);
        }
        assert_eq!(caption_px(96), 40);
        assert_eq!(caption_px(192), 80);
    }

    #[test]
    fn zones_cover_caption_buttons_and_resize_edges() {
        for dpi in [96u32, 144, 192] {
            let cw = px(W, dpi);
            let g = layout(cw, dpi);
            let mid = |r: Rect| (r.x + r.w / 2, r.y + r.h / 2);
            for b in BTNS {
                let (x, y) = mid(g.button(b));
                assert_eq!(zone(x, y, cw, dpi, false), Zone::Btn(b), "{b:?} dpi {dpi}");
                assert_eq!(zone(x, y, cw, dpi, true), Zone::Btn(b));
            }
            // tahanie: lavy okraj hlavicky pod pasom zmeny velkosti, a aj stred
            let y = px(RESIZE_LP, dpi) + 2;
            assert_eq!(zone(px(100, dpi), y, cw, dpi, false), Zone::Caption);
            assert_eq!(
                zone(px(300, dpi), caption_px(dpi) - 1, cw, dpi, false),
                Zone::Caption
            );
            // pod hlavickou je klient
            assert_eq!(
                zone(px(300, dpi), caption_px(dpi), cw, dpi, false),
                Zone::Client
            );
            assert_eq!(zone(-1, 5, cw, dpi, false), Zone::Client);
            assert_eq!(zone(5, -1, cw, dpi, false), Zone::Client);
            // horny pas a rohy (len ked nie je maximalizovane); roh ma prednost pred tlacidlom Close
            assert_eq!(zone(px(300, dpi), 0, cw, dpi, false), Zone::Top);
            assert_eq!(zone(1, 0, cw, dpi, false), Zone::TopLeft);
            assert_eq!(zone(cw - 2, 0, cw, dpi, false), Zone::TopRight);
            assert_eq!(
                zone(px(300, dpi), 0, cw, dpi, true),
                Zone::Caption,
                "maximalizovane: ziadna zmena velkosti"
            );
            assert_eq!(zone(cw - 2, 0, cw, dpi, true), Zone::Btn(Btn::Close));
        }
    }

    #[test]
    fn every_caption_pixel_is_exactly_one_zone_and_buttons_never_overlap() {
        let dpi = 144;
        let cw = px(W, dpi);
        let g = layout(cw, dpi);
        for (i, a) in BTNS.iter().enumerate() {
            for b in &BTNS[i + 1..] {
                let (ra, rb) = (g.button(*a), g.button(*b));
                assert!(ra.x + ra.w <= rb.x || rb.x + rb.w <= ra.x);
            }
        }
        let mut counts = [0usize; 6];
        for y in 0..g.caption.h {
            for x in 0..cw {
                let k = match zone(x, y, cw, dpi, false) {
                    Zone::Client => 0,
                    Zone::Caption => 1,
                    Zone::Btn(_) => 2,
                    Zone::Top => 3,
                    Zone::TopLeft => 4,
                    Zone::TopRight => 5,
                };
                counts[k] += 1;
            }
        }
        assert_eq!(counts[0], 0, "cela hlavicka je nieco");
        assert_eq!(counts.iter().sum::<usize>(), (cw * g.caption.h) as usize);
        assert!(counts[1] > 0 && counts[2] > 0 && counts[3] > 0);
    }

    #[test]
    fn min_track_is_capped_to_the_work_area() {
        assert_eq!(min_track(96, 8, 8, 1920, 1040), (656, 488));
        assert_eq!(min_track(192, 16, 16, 1920, 1040), (1312, 976));
        // maly monitor: nikdy vacsie ako pracovna plocha
        assert_eq!(min_track(96, 8, 8, 600, 400), (600, 400));
    }

    #[test]
    fn button_commands_and_glyphs_follow_the_maximized_state() {
        assert_eq!(sys_command(Btn::Max, false), 0xF030);
        assert_eq!(sys_command(Btn::Max, true), 0xF120);
        assert_eq!(sys_command(Btn::Min, true), 0xF020);
        assert_eq!(sys_command(Btn::Close, false), 0xF060);
        assert_ne!(glyph(Btn::Max, false), glyph(Btn::Max, true));
    }
}
