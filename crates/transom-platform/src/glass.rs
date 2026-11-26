//! Sklenene pozadie listy a panelov ("cierne sklo", nastavenia vzhladu `glass_panels` / `glass_bar`).
//!
//! Okno ostava **nevrstvene** a obsah s alfou po pixeloch sa posiela `BitBlt` (premultiplied BGRA). Alfu pixelov
//! nevrstveneho okna zapne `DwmEnableBlurBehindWindow` s prazdnou oblastou rozmazania: od Windows 8 DWM pod oknom
//! nerozmazava, takze cez takmer cierny nadych (88 %) jemne vidno obsah pod oknom. Akrylove rozmazanie
//! (`SetWindowCompositionAttribute`) sa skusalo (spike `glassprobe`) - Windows ho silno zosvetli na sivu, takze
//! priehladnost by sa stratila. Ked volanie zlyha, okno ostane nepriehladne v pozadi nadychu. Sklo respektuje
//! systemovy prepinac *Efekty priehladnosti* aj vysoky kontrast.
//!
//! Vrstvy (vsetko vlastne, kreslene na CPU): nadych, lesk navrchu (svetlo zhora), tenky svetly okraj a obsah
//! odvodeny z dvoch prechodov GDI (pozri [`crate::layered`]).
use windows::Win32::Foundation::HWND;

/// Ktore hrany dostanu svetly okraj (lista hore na obrazovke: len spodna; panel: vsetky).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rim {
    Bottom,
    All,
    /// Pripojene k liste (vybezok pod listou): okraj len po bokoch a dole, bez lesku a okraja hore.
    Attached,
}

/// Nabeh z farby listy: horna cast skla zacina farbou listy a plynulo prejde do skla (vybezok nevyzera oddelene).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lead {
    pub color: (u8, u8, u8),
    /// Krytie farby listy na hornom okraji (0..=255; priehladna lista = slabsi nabeh).
    pub alpha: u8,
    /// Vyska nabehu v px.
    pub px: i32,
    /// Farba textu panela: farba precitana z obrazovky sa pouzije, len ak ma s nim kontrast >= 4,5 : 1 (inak ostane
    /// `color`, ktoru zvolila aplikacia).
    pub text: (u8, u8, u8),
}

fn lum(c: (u8, u8, u8)) -> f32 {
    let f = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.039_28 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
}

/// Kontrast dvoch farieb (WCAG).
pub fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

impl Lead {
    /// Nabeh zacne farbou `seen` (skutocna farba listy na obrazovke), ak je s textom panela citatelny.
    pub fn adopt(&mut self, seen: Option<(u8, u8, u8)>) {
        if let Some(c) = seen.filter(|c| contrast(*c, self.text) >= 4.5) {
            self.color = c;
        }
    }
}

/// Vzhlad skla.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlassStyle {
    /// Farba nadychu.
    pub tint: (u8, u8, u8),
    /// Krytie nadychu 0..=255 (dost na kontrast textu aj nad bielym pozadim pod sklom).
    pub tint_alpha: u8,
    /// Tmava tema: svetly lesk a okraj; svetla: silnejsi lesk, tmavsi okraj.
    pub dark: bool,
    pub rim: Rim,
    /// Nabeh z farby listy hore (len vybezok).
    pub lead: Option<Lead>,
    /// Polomer zaoblenych rohov (px; `for_theme` dava lp, kreslenie ho prepocita na DPI). Rohy aj okraj v nich
    /// kresli sklo samo, vyhladene - bez zubateho vyrezu okna a bez medzery v okraji. 0 = hranate.
    pub corner: i32,
    /// Vyduty prechod do listy (px, len `Rim::Attached`): telo panela je o `flare` uzsie z kazdej strany a v hornych
    /// rohoch sa obluckom rozsiri az k okraju okna - panel "vyteka" z listy. 0 = bez neho.
    pub flare: i32,
}

/// Okraj panela pripojeneho k liste (tmave sklo): rovnako jasny ako spodna ciara listy, aby z nej plynulo pokracoval.
const ATTACHED_RIM: u32 = 30;

/// Krytie cierneho nadychu tmaveho skla (najhorsi pripad pre kontrast: biela stranka pod oknom).
pub const DARK_TINT_ALPHA: u8 = 225;

impl GlassStyle {
    /// Tmava tema: cierne sklo - takmer cierny nadych 88 % (nad bielou strankou ~#2B2B2C, obsah pod oknom jemne
    /// presvita; s farbami textu z `Theme::glass_pass` ma tlmeny text aj na karte >= 4,5 : 1). Svetla tema: nadych
    /// temy 80 %.
    pub fn for_theme(tint: (u8, u8, u8), dark: bool, rim: Rim) -> Self {
        GlassStyle {
            tint: if dark { (14, 14, 16) } else { tint },
            tint_alpha: if dark { DARK_TINT_ALPHA } else { 205 },
            dark,
            rim,
            lead: None,
            corner: 8,
            flare: 0,
        }
    }
}

/// Bod (stred pixela) v zaoblenom rohu: vzdialenost od stredu oblucika. `None` = mimo rohov (rovne hrany).
/// Pripojeny panel ma zaoblene len spodne rohy, `All` vsetky styri, lista ziadne.
fn corner_dist(st: &GlassStyle, x: i32, y: i32, w: i32, h: i32) -> Option<f32> {
    let r = st.corner.min(w / 2).min(h / 2);
    if r <= 0 || st.rim == Rim::Bottom {
        return None;
    }
    let top = st.rim == Rim::All;
    let cx = if x < r {
        r as f32
    } else if x >= w - r {
        (w - r) as f32
    } else {
        return None;
    };
    let cy = if y >= h - r {
        (h - r) as f32
    } else if top && y < r {
        r as f32
    } else {
        return None;
    };
    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
    Some((dx * dx + dy * dy).sqrt())
}

/// Vyduty prechod: pixel v bocnom pase sirky `flare` - vzdialenost od stredu obluka (stred mimo panela, vo vyske
/// `flare` na okraji okna). `None` = mimo pasov (telo panela).
fn flare_dist(st: &GlassStyle, x: i32, y: i32, w: i32) -> Option<f32> {
    let f = st.flare;
    if f <= 0 || st.rim != Rim::Attached || (x >= f && x < w - f) {
        return None;
    }
    let cx = if x < f { 0.0 } else { w as f32 };
    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - f as f32);
    Some((dx * dx + dy * dy).sqrt())
}

/// Krytie pixela tvarom panelu (0..=255): vnutri 255, za oblucikom rohu 0, na oblucikuu vyhladene.
pub fn corner_coverage(st: &GlassStyle, x: i32, y: i32, w: i32, h: i32) -> u32 {
    if let Some(d) = flare_dist(st, x, y, w) {
        // vonkajsi pas: pokryte len nad obluckom (smerom k liste), pod vyskou `flare` nic
        if y >= st.flare {
            return 0;
        }
        return ((d - st.flare as f32 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u32;
    }
    let (x, w) = (
        x - st.flare.max(0) * i32::from(st.rim == Rim::Attached),
        w - 2 * st.flare.max(0) * i32::from(st.rim == Rim::Attached),
    );
    match corner_dist(st, x, y, w, h) {
        None => 255,
        Some(d) => {
            let r = st.corner.min(w / 2).min(h / 2) as f32;
            ((r - d + 0.5).clamp(0.0, 1.0) * 255.0).round() as u32
        }
    }
}

/// Skutocna farba na obrazovke v riadku `y` medzi `x0` a `x1` (median ~24 vzoriek, aby ju neskreslil text): farba
/// listy, ako ju pouzivatel naozaj vidi (aj priehladnej nad oknom pod nou). `None` = obrazovku sa nepodarilo citat.
pub fn screen_color(x0: i32, x1: i32, y: i32) -> Option<(u8, u8, u8)> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
    };
    if x1 <= x0 {
        return None;
    }
    // jeden riadok obrazovky jednym BitBlt (GetPixel na obrazovke stoji pri DWM ~4 ms za bod)
    let w = (x1 - x0).min(4096);
    let n = 24;
    let mut r = Vec::with_capacity(n);
    let mut g = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    // SAFETY: docasne DC a DIB sa uvolnia na konci; pixely sa citaju len v hraniciach DIB.
    unsafe {
        let sdc = GetDC(None);
        if sdc.is_invalid() {
            return None;
        }
        let mdc = CreateCompatibleDC(Some(sdc));
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -1,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let ok = match CreateDIBSection(Some(mdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) if !bits.is_null() => {
                let old = SelectObject(mdc, bmp.into());
                let copied = BitBlt(mdc, 0, 0, w, 1, Some(sdc), x0, y, SRCCOPY).is_ok();
                if copied {
                    let px = std::slice::from_raw_parts(bits as *const u32, w as usize);
                    for i in 0..n {
                        let x = (w as usize * (2 * i + 1)) / (2 * n);
                        let c = px[x.min(w as usize - 1)];
                        // DIB je BGRA
                        r.push(((c >> 16) & 0xFF) as u8);
                        g.push(((c >> 8) & 0xFF) as u8);
                        b.push((c & 0xFF) as u8);
                    }
                }
                SelectObject(mdc, old);
                let _ = DeleteObject(bmp.into());
                copied
            }
            _ => false,
        };
        let _ = DeleteDC(mdc);
        ReleaseDC(None, sdc);
        if !ok {
            return None;
        }
    }
    for v in [&mut r, &mut g, &mut b] {
        v.sort_unstable();
    }
    let m = r.len() / 2;
    Some((r[m], g[m], b[m]))
}

/// Posledna odpoved [`system_allows`] (0 = nezname, 1 = nie, 2 = ano) a kedy bola zistena (ms od `EPOCH`).
static ALLOWS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static ALLOWS_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// Najdlhsie, kym sa zapamatana odpoved overi znova aj bez spravy o zmene nastaveni.
const ALLOWS_TTL_MS: u64 = 5_000;

/// [`system_allows`] zapamatane: lista sa pyta pri kazdej snimke (az 60x za sekundu, viackrat na snimku) a odpoved
/// stoji citanie registra (~8 us). Zabudne sa pri kazdej `WM_SETTINGCHANGE` (riadiace okno listy, pozri
/// [`forget_system_allows`]) a najneskor po `ALLOWS_TTL_MS`.
pub fn system_allows_cached() -> bool {
    allows_at(
        EPOCH
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
            + 1,
    )
}

fn allows_at(now: u64) -> bool {
    use std::sync::atomic::Ordering;
    let known = ALLOWS.load(Ordering::Relaxed);
    if known != 0 && now.saturating_sub(ALLOWS_AT.load(Ordering::Relaxed)) < ALLOWS_TTL_MS {
        return known == 2;
    }
    let v = system_allows();
    ALLOWS.store(if v { 2 } else { 1 }, Ordering::Relaxed);
    ALLOWS_AT.store(now, Ordering::Relaxed);
    v
}

/// Zmena systemovych nastaveni (priehladnost, vysoky kontrast): dalsie [`system_allows_cached`] precita znova.
pub fn forget_system_allows() {
    ALLOWS.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Povoluje system sklo? `EnableTransparency` (Nastavenia > Prisposobenie > Farby > Efekty priehladnosti) a nie
/// vysoky kontrast.
pub fn system_allows() -> bool {
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut hc = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: pvParam ukazuje na HIGHCONTRASTW so spravnym cbSize.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut HIGHCONTRASTW as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    let high_contrast = ok.is_ok() && (hc.dwFlags & HCF_HIGHCONTRASTON).0 != 0;
    !high_contrast && crate::system::transparency_enabled()
}

/// Zapne / vypne priehladnost okna (alfa pixelov, bez rozmazania): `DwmEnableBlurBehindWindow` s prazdnou
/// oblastou rozmazania. `false` = volanie zlyhalo (okno ostane nepriehladne).
pub fn set_glass(hwnd: HWND, on: bool) -> bool {
    use windows::Win32::Graphics::Dwm::{
        DwmEnableBlurBehindWindow, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
    };
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};
    // SAFETY: platne okno; oblast sa po volani uvolni (DWM si ju skopiruje).
    unsafe {
        let rgn = CreateRectRgn(0, 0, -1, -1);
        let bb = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: on.into(),
            hRgnBlur: rgn,
            fTransitionOnMaximized: false.into(),
        };
        let ok = DwmEnableBlurBehindWindow(hwnd, &bb).is_ok();
        let _ = DeleteObject(rgn.into());
        ok
    }
}

fn mul255(a: u32, b: u32) -> u32 {
    (a * b + 127) / 255
}

/// Prekryje premultiplied `[r,g,b,a]` farbou `(r, g, b)` s krytim `k` (0..=255).
fn over_rgb(px: [u32; 4], c: (u8, u8, u8), k: u32) -> [u32; 4] {
    let inv = 255 - k;
    [
        mul255(c.0 as u32, k) + mul255(px[0], inv),
        mul255(c.1 as u32, k) + mul255(px[1], inv),
        mul255(c.2 as u32, k) + mul255(px[2], inv),
        k + mul255(px[3], inv),
    ]
}

/// Prekryje premultiplied `[r,g,b,a]` farbou `c` s krytim `k` (0..=255).
fn over(px: [u32; 4], c: u32, k: u32) -> [u32; 4] {
    let inv = 255 - k;
    [
        mul255(c, k) + mul255(px[0], inv),
        mul255(c, k) + mul255(px[1], inv),
        mul255(c, k) + mul255(px[2], inv),
        k + mul255(px[3], inv),
    ]
}

/// Pozadie skla v bode (premultiplied `[r,g,b,a]`): nadych, lesk zhora (horna polovica, linearne do nuly) a okraj.
pub fn backdrop_at(st: &GlassStyle, x: i32, y: i32, w: i32, h: i32) -> [u32; 4] {
    let ta = st.tint_alpha as u32;
    let mut px = [
        mul255(st.tint.0 as u32, ta),
        mul255(st.tint.1 as u32, ta),
        mul255(st.tint.2 as u32, ta),
        ta,
    ];
    // nabeh z farby listy (panel / vybezok pod listou): hore plna farba listy, plynulo (smoothstep) do skla; nabeh
    // zaberie najviac dve tretiny vysky, aby aj nizky vybezok skoncil sklom
    if let Some(l) = st.lead.filter(|l| l.px > 0) {
        let span = l.px.min(h * 2 / 3).max(1);
        if y < span {
            let t = y as f32 / span as f32;
            let f = 1.0 - t * t * (3.0 - 2.0 * t);
            let k = (l.alpha as f32 * f).round() as u32;
            px = over_rgb(px, l.color, k);
        }
    }
    // lesk: biele svetlo zhora; panel vyssi nez lista -> lesk len v hornych ~40 px (pripojeny vybezok lesk nema)
    let span = (h / 2).clamp(1, 44);
    if y < span && st.rim != Rim::Attached {
        let top = if st.dark { 30 } else { 70 };
        let k = top * (span - y) as u32 / span as u32;
        px = over(px, 255, k);
    }
    // okraj: rovne hrany 1 px, v zaoblenych rohoch pokracuje po oblucku (bez medzery); vo vydutom prechode po jeho
    // oblucku
    if let Some(d) = flare_dist(st, x, y, w) {
        let f = st.flare as f32;
        let strength = if y < st.flare {
            (1.0 - (d - (f + 0.5)).abs()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if strength > 0.0 {
            // ako spodna ciara listy (plynule pokracuje do okraja panela)
            let k = (ATTACHED_RIM as f32 * strength).round() as u32;
            px = if st.dark {
                over(px, 255, k)
            } else {
                over(px, 0, (30.0 * strength).round() as u32)
            };
        }
        return px;
    }
    let fl = if st.rim == Rim::Attached {
        st.flare.max(0)
    } else {
        0
    };
    let (x, w) = (x - fl, w - 2 * fl);
    let strength = match corner_dist(st, x, y, w, h) {
        Some(d) => {
            let r = st.corner.min(w / 2).min(h / 2) as f32;
            (1.0 - (d - (r - 0.5)).abs()).clamp(0.0, 1.0)
        }
        None => {
            let edge = match st.rim {
                Rim::Bottom => y == h - 1,
                Rim::All => x == 0 || y == 0 || x == w - 1 || y == h - 1,
                // bocny okraj az pod vydutym prechodom (ten ma vlastny okraj po oblucku)
                Rim::Attached => ((x == 0 || x == w - 1) && y >= fl - 3) || y == h - 1,
            };
            if edge {
                1.0
            } else {
                0.0
            }
        }
    };
    if strength > 0.0 {
        let k = |v: u32| (v as f32 * strength).round() as u32;
        px = if st.dark {
            let base = if st.rim == Rim::Attached {
                ATTACHED_RIM
            } else {
                44
            };
            over(
                px,
                255,
                k(if y == 0 && st.rim != Rim::Attached {
                    70
                } else {
                    base
                }),
            )
        } else {
            over(px, 0, k(30))
        };
    }
    px
}

/// Zlozi oblast `a` z dvoch prechodov (`black`, `white`, 0x00RRGGBB) nad sklo do `out` (premultiplied 0xAARRGGBB).
/// `content` (0..=255) nasobi krytie obsahu - roztmievanie pri otvarani.
#[allow(clippy::too_many_arguments)]
pub fn compose(
    black: &[u32],
    white: &[u32],
    out: &mut [u32],
    w: i32,
    h: i32,
    a: crate::layered::Area,
    st: &GlassStyle,
    content: u8,
) {
    let k = content as u32;
    for y in a.y0..a.y1 {
        for x in a.x0..a.x1 {
            let i = (y * w + x) as usize;
            let (Some(&b), Some(&wh), Some(o)) = (black.get(i), white.get(i), out.get_mut(i))
            else {
                continue;
            };
            let c = |v: u32, s: u32| (v >> s) & 0xFF;
            let (br, bg, bb) = (c(b, 16), c(b, 8), c(b, 0));
            let diff = (c(wh, 16).saturating_sub(br)
                + c(wh, 8).saturating_sub(bg)
                + c(wh, 0).saturating_sub(bb))
                / 3;
            let cov = mul255(255 - diff.min(255), k);
            let (cr, cg, cb) = (
                mul255(br, k).min(cov),
                mul255(bg, k).min(cov),
                mul255(bb, k).min(cov),
            );
            let inv = 255 - cov;
            let g = backdrop_at(st, x, y, w, h);
            let oa = (cov + mul255(g[3], inv)).min(255);
            let or = (cr + mul255(g[0], inv)).min(oa);
            let og = (cg + mul255(g[1], inv)).min(oa);
            let ob = (cb + mul255(g[2], inv)).min(oa);
            // tvar panelu: za zaoblenym rohom nic (ani obsah), na oblucku vyhladene (premultiplied = vsetky kanaly)
            let m = corner_coverage(st, x, y, w, h);
            let (oa, or, og, ob) = if m == 255 {
                (oa, or, og, ob)
            } else {
                (mul255(oa, m), mul255(or, m), mul255(og, m), mul255(ob, m))
            };
            *o = oa << 24 | or << 16 | og << 8 | ob;
        }
    }
}

/// Pruzina otvarania (kriticky skoro tlmena, prekmit ~3 %): 0 -> 1 za ~0,38 s. `t` v sekundach.
pub fn spring(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    let w = std::f32::consts::TAU * 2.8;
    let z = 0.75f32;
    let wd = w * (1.0 - z * z).sqrt();
    let e = (-z * w * t).exp();
    1.0 - e * ((wd * t).cos() + z / (1.0 - z * z).sqrt() * (wd * t).sin())
}

/// Trvanie animacie otvarania panelu (po nom je pruzina do 1,5 % od ciela).
pub const OPEN_MS: u32 = 380;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layered::Area;

    #[test]
    fn the_remembered_system_answer_matches_a_fresh_read_and_is_forgotten_on_request() {
        use std::sync::atomic::Ordering;
        let fresh = system_allows();
        assert_eq!(system_allows_cached(), fresh);
        assert_ne!(ALLOWS.load(Ordering::Relaxed), 0, "zapamatane");
        // zapamatana opacna odpoved sa vrati, kym sa nezabudne (dokaz, ze sa register necita znova)
        ALLOWS.store(if fresh { 1 } else { 2 }, Ordering::Relaxed);
        assert_eq!(system_allows_cached(), !fresh);
        forget_system_allows();
        assert_eq!(system_allows_cached(), fresh);
        // po uplynuti platnosti sa precita znova aj bez spravy
        let at = ALLOWS_AT.load(Ordering::Relaxed);
        ALLOWS.store(if fresh { 1 } else { 2 }, Ordering::Relaxed);
        assert_eq!(allows_at(at + ALLOWS_TTL_MS - 1), !fresh);
        assert_eq!(allows_at(at + ALLOWS_TTL_MS), fresh);
    }

    const ST: GlassStyle = GlassStyle {
        tint: (32, 32, 36),
        tint_alpha: 174,
        dark: true,
        rim: Rim::All,
        lead: None,
        corner: 0,
        flare: 0,
    };

    #[test]
    fn rounded_corners_are_transparent_outside_and_keep_a_continuous_rim() {
        let st = GlassStyle {
            rim: Rim::Attached,
            corner: 10,
            flare: 0,
            ..ST
        };
        let (w, h) = (60, 40);
        // za oblucikom spodneho rohu nic, horne rohy pripojeneho panela su hranate
        assert_eq!(corner_coverage(&st, 0, h - 1, w, h), 0);
        assert_eq!(corner_coverage(&st, w - 1, h - 1, w, h), 0);
        assert_eq!(corner_coverage(&st, 0, 0, w, h), 255);
        assert_eq!(corner_coverage(&st, 30, 20, w, h), 255);
        // okraj ide po oblucku: na 45 stupnoch je pixel svetlejsi nez vnutro (ziadna medzera v rohu)
        let d = (10.0f32 - 0.5) / std::f32::consts::SQRT_2;
        let (x, y) = ((10.0 - d) as i32, (h as f32 - 10.0 + d) as i32);
        assert!(corner_coverage(&st, x, y, w, h) > 0);
        assert!(backdrop_at(&st, x, y, w, h)[0] > backdrop_at(&st, 30, 20, w, h)[0]);
        // vsetky styri rohy pri `All`
        let all = GlassStyle {
            rim: Rim::All,
            ..st
        };
        assert_eq!(corner_coverage(&all, 0, 0, w, h), 0);
        assert_eq!(corner_coverage(&all, w - 1, 0, w, h), 0);
        // vyhladenie: na oblucku su medzistupne, nie len 0 / 255
        let mids = (0..w)
            .flat_map(|x| (0..h).map(move |y| (x, y)))
            .map(|(x, y)| corner_coverage(&all, x, y, w, h))
            .filter(|c| *c > 0 && *c < 255)
            .count();
        assert!(mids > 8, "{mids}");
    }

    #[test]
    fn empty_glass_is_the_tinted_backdrop_and_never_opaque() {
        let (w, h) = (20, 100);
        let black = vec![0u32; (w * h) as usize];
        let white = vec![0x00FF_FFFFu32; (w * h) as usize];
        let mut out = vec![0u32; (w * h) as usize];
        compose(&black, &white, &mut out, w, h, Area::full(w, h), &ST, 255);
        let mid = out[(60 * w + 10) as usize];
        assert_eq!(mid >> 24, 174, "nadych bez lesku a okraja");
        // lesk navrchu je svetlejsi nez stred
        let top = out[(w + 10) as usize];
        assert!((top >> 16 & 0xFF) > (mid >> 16 & 0xFF));
        assert!(out.iter().all(|p| p >> 24 < 255));
    }

    #[test]
    fn opaque_content_wins_and_fades_with_the_content_factor() {
        let (w, h) = (4, 60);
        let black = vec![0x00FF_FFFFu32; (w * h) as usize]; // biely text plne kryty
        let white = black.clone();
        let mut out = vec![0u32; (w * h) as usize];
        compose(&black, &white, &mut out, w, h, Area::full(w, h), &ST, 255);
        assert_eq!(out[(50 * w + 2) as usize], 0xFFFF_FFFF);
        compose(&black, &white, &mut out, w, h, Area::full(w, h), &ST, 0);
        assert_eq!(
            out[(50 * w + 2) as usize] >> 24,
            174,
            "bez obsahu ostava sklo"
        );
    }

    #[test]
    fn premultiplied_invariant_holds_everywhere() {
        let (w, h) = (8, 50);
        let black: Vec<u32> = (0..w * h)
            .map(|i| (i as u32).wrapping_mul(2654435761) & 0x7F7F7F)
            .collect();
        let white: Vec<u32> = black.iter().map(|b| b | 0x808080).collect();
        let mut out = vec![0u32; (w * h) as usize];
        compose(&black, &white, &mut out, w, h, Area::full(w, h), &ST, 200);
        for p in out {
            let a = p >> 24;
            assert!((p >> 16 & 0xFF) <= a && (p >> 8 & 0xFF) <= a && (p & 0xFF) <= a);
        }
    }

    #[test]
    fn spring_starts_at_zero_settles_at_one_with_small_overshoot() {
        assert_eq!(spring(0.0), 0.0);
        let peak = (1..200)
            .map(|i| spring(i as f32 / 500.0))
            .fold(0.0f32, f32::max);
        assert!(peak > 1.0 && peak < 1.05, "{peak}");
        assert!((spring(OPEN_MS as f32 / 1000.0) - 1.0).abs() < 0.015);
    }

    #[test]
    fn an_attached_notch_starts_in_the_bar_colour_without_a_top_rim_or_sheen() {
        let st = GlassStyle {
            rim: Rim::Attached,
            lead: Some(Lead {
                color: (200, 20, 40),
                alpha: 255,
                px: 20,
                text: (255, 255, 255),
            }),
            ..ST
        };
        let (w, h) = (50, 100);
        // horny riadok = farba listy (plne kryta)
        let top = backdrop_at(&st, 10, 0, w, h);
        assert_eq!(top[3], 255);
        assert!(top[0] > 190 && top[1] < 30, "{top:?}");
        // nabeh plynulo klesa (monotonne) a za nim je cisty nadych (bez lesku)
        let reds: Vec<u32> = (0..=20).map(|y| backdrop_at(&st, 10, y, w, h)[0]).collect();
        assert!(reds.windows(2).all(|p| p[0] >= p[1]), "{reds:?}");
        assert!(backdrop_at(&st, 10, 10, w, h)[0] < top[0]);
        let plain = GlassStyle { lead: None, ..st };
        assert_eq!(
            backdrop_at(&st, 10, 30, w, h),
            backdrop_at(&plain, 10, 30, w, h)
        );
        let sheen = GlassStyle {
            rim: Rim::All,
            lead: None,
            ..st
        };
        assert!(
            backdrop_at(&sheen, 10, 5, w, h)[0] > backdrop_at(&plain, 10, 5, w, h)[0],
            "pripojeny nema lesk"
        );
        // okraj po bokoch a dole, hore nie
        assert_ne!(
            backdrop_at(&plain, 0, 50, w, h),
            backdrop_at(&plain, 10, 50, w, h)
        );
        assert_ne!(
            backdrop_at(&plain, 10, h - 1, w, h),
            backdrop_at(&plain, 10, 50, w, h)
        );
        assert_eq!(
            backdrop_at(&plain, 10, 0, w, h),
            backdrop_at(&plain, 10, 1, w, h)
        );
    }

    #[test]
    fn the_on_screen_bar_colour_is_used_only_when_the_panel_text_stays_readable() {
        let mut l = Lead {
            color: (23, 27, 33),
            alpha: 255,
            px: 40,
            text: (232, 236, 241),
        };
        l.adopt(Some((0, 0, 0)));
        assert_eq!(
            l.color,
            (0, 0, 0),
            "cierna lista + svetly text: prevezme sa"
        );
        l.adopt(Some((250, 250, 250)));
        assert_eq!(
            l.color,
            (0, 0, 0),
            "biela lista + svetly text: necitatelne, ostava predosla"
        );
        l.adopt(None);
        assert_eq!(l.color, (0, 0, 0));
        assert!(contrast((0, 0, 0), (255, 255, 255)) > 20.0);
    }
}
