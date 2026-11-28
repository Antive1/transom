//! Priehladne pozadie listy: vrstvene okno (`WS_EX_LAYERED` + `UpdateLayeredWindow`) s alfou po pixeloch, stale
//! kreslene cez GDI (ziadny iny backend, ziadny Acrylic/Mica).
//!
//! GDI nevie kreslit s alfa kanalom (text a ikony zapisu do pixelov len RGB). Alfu preto odvodime z **dvoch
//! prechodov** tej istej sceny na znamom pozadi: raz na cierno (`B`), raz na bielo (`W`). Pre pixel s krytim `a`
//! a farbou `F` plati `B = a*F` a `W = a*F + (1-a)*255`, teda `a = 1 - (W-B)/255` a `B` je uz *premultiplied* farba.
//! Nepriehladne pixely (obal, hover) maju `B == W`, cize `a = 1`. Presne to vyzaduje `UpdateLayeredWindow`.
//!
//! Dosledky: (1) text sa kresli **sivym** vyhladzovanim - ClearType potrebuje nepriehladne znamе pozadie, takze
//! sa pri priehladnom pozadi nepouzije; (2) okraje pismen su skutocne polopriehladne (ziadne "halo" farby pozadia);
//! (3) pri 100 % nepriehladnosti sa vrstvene okno vobec nepouziva (stara cesta s ClearType, bajt po bajte).
//! Pixely s alfou 0 su pre mys priehladne (kliknutie prejde na plochu), preto je alfa najmenej 1 aj pri
//! "uplne priehladnom" pozadi - pravy klik na prazdne miesto listy (menu) a hover dalej funguju.
use windows::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    UpdateLayeredWindowIndirect, ULW_ALPHA, UPDATELAYEREDWINDOWINFO,
};

/// Vzhlad pozadia listy, ktory pozna platforma: alfa 255 = nepriehladne (stara cesta bez vrstveneho okna).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarAppearance {
    /// Alfa pozadia 0..=255 (0 = uplne priehladne; kvoli mysi sa v okne pouzije najmenej 1).
    pub alpha: u8,
    /// Farba pozadia (po zohladneni temy a vlastnej farby).
    pub bg: (u8, u8, u8),
    /// Sklo (akrylove rozmazanie pod listou, pozri [`crate::glass`]); ma prednost pred `alpha`.
    pub glass: Option<crate::glass::GlassStyle>,
}

impl BarAppearance {
    pub const OPAQUE: BarAppearance = BarAppearance {
        alpha: 255,
        bg: (0, 0, 0),
        glass: None,
    };

    pub fn is_opaque(&self) -> bool {
        self.alpha == 255 && self.glass.is_none()
    }
}

/// Obdlznik `[x0, x1) x [y0, y1)` v px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Area {
    pub fn full(w: i32, h: i32) -> Self {
        Area {
            x0: 0,
            y0: 0,
            x1: w,
            y1: h,
        }
    }
    /// Orezane na povrch `w x h` (prazdne = `None`).
    pub fn clip(self, w: i32, h: i32) -> Option<Area> {
        let a = Area {
            x0: self.x0.max(0),
            y0: self.y0.max(0),
            x1: self.x1.min(w),
            y1: self.y1.min(h),
        };
        (a.x1 > a.x0 && a.y1 > a.y0).then_some(a)
    }
    pub fn union(self, o: Area) -> Area {
        Area {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
}

fn ch(v: u32) -> (u32, u32, u32) {
    ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF)
}

fn mul255(a: u32, b: u32) -> u32 {
    (a * b + 127) / 255
}

/// Z dvoch prechodov (`black`, `white`: 0x00RRGGBB, `w` px na riadok) zlozi *premultiplied* ARGB (0xAARRGGBB) pre
/// oblast `a`: obsah (text, ikony, nepriehladne cast) lezi **nad** pozadim `bg` s alfou `bg_alpha`.
pub fn compose(
    black: &[u32],
    white: &[u32],
    out: &mut [u32],
    w: i32,
    a: Area,
    bg: (u8, u8, u8),
    bg_alpha: u8,
) {
    let ba = bg_alpha as u32;
    let (bgr, bgg, bgb) = (
        mul255(bg.0 as u32, ba),
        mul255(bg.1 as u32, ba),
        mul255(bg.2 as u32, ba),
    );
    for y in a.y0..a.y1 {
        for x in a.x0..a.x1 {
            let i = (y * w + x) as usize;
            let (Some(&b), Some(&wh)) = (black.get(i), white.get(i)) else {
                continue;
            };
            let ((br, bgc, bb), (wr, wg, wb)) = (ch(b), ch(wh));
            let diff = (wr.saturating_sub(br) + wg.saturating_sub(bgc) + wb.saturating_sub(bb)) / 3;
            let cov = 255 - diff.min(255); // kryt obsahu 0..=255
            let inv = 255 - cov;
            // premultiplied farba obsahu nemoze presiahnut krytie
            let (cr, cg, cb) = (br.min(cov), bgc.min(cov), bb.min(cov));
            let mut oa = cov + mul255(inv, ba);
            let (mut or, mut og, mut ob) = (
                cr + mul255(inv, bgr),
                cg + mul255(inv, bgg),
                cb + mul255(inv, bgb),
            );
            if oa == 0 {
                // alfa 0 by pustila mys na plochu: najmenej 1 (farba 0 = premultiplied)
                oa = 1;
                (or, og, ob) = (0, 0, 0);
            }
            let (or, og, ob) = (or.min(oa), og.min(oa), ob.min(oa));
            out[i] = oa << 24 | or << 16 | og << 8 | ob;
        }
    }
}

/// Jeden 32-bitovy DIB s vlastnym DC (zhora nadol, BGRA).
struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    len: usize,
}

impl Dib {
    fn new(w: i32, h: i32) -> Option<Dib> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: DC a DIB sa vytvoria tu; pri chybe sa uvolni to, co vzniklo; inak ich vlastni `Dib` (Drop).
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(dc);
                return None;
            };
            if bits.is_null() {
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bmp.into());
            Some(Dib {
                dc,
                bmp,
                old,
                bits: bits as *mut u32,
                len: (w as usize) * (h as usize),
            })
        }
    }

    fn pixels(&self) -> &[u32] {
        // SAFETY: `bits` ukazuje na `len` px DIB sekcie, ktora zije, kym zije `Dib`.
        unsafe { std::slice::from_raw_parts(self.bits, self.len) }
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        // SAFETY: ako vyssie; vyhradny pristup cez `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.len) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: objekty vytvoril `Dib::new`, povodny bitmap sa vrati do DC a vsetko sa uvolni raz.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Povrchy vrstveneho okna: dva prechody (cierne a biele pozadie) a vystup (premultiplied ARGB).
pub struct LayeredBuf {
    pub w: i32,
    pub h: i32,
    black: Dib,
    white: Dib,
    out: Dib,
}

impl LayeredBuf {
    pub fn new(w: i32, h: i32) -> Option<LayeredBuf> {
        if w <= 0 || h <= 0 {
            return None;
        }
        Some(LayeredBuf {
            w,
            h,
            black: Dib::new(w, h)?,
            white: Dib::new(w, h)?,
            out: Dib::new(w, h)?,
        })
    }

    pub fn black_dc(&self) -> HDC {
        self.black.dc
    }

    pub fn white_dc(&self) -> HDC {
        self.white.dc
    }

    /// Zlozi oblast `a` z oboch prechodov do vystupu.
    pub fn compose(&mut self, a: Area, bg: (u8, u8, u8), bg_alpha: u8) {
        let (b, w) = (self.black.pixels(), self.white.pixels());
        // vystup je samostatny DIB, takze vypozicky sa neprekryvaju
        // SAFETY: `out` je samostatny DIB s `len` pixelmi; ziadna ina vypozicka jeho pamate nezije.
        let out = unsafe { std::slice::from_raw_parts_mut(self.out.bits, self.out.len) };
        compose(b, w, out, self.w, a, bg, bg_alpha);
    }

    /// Vystup (len na citanie, na testy a kontrolu).
    pub fn output(&self) -> &[u32] {
        self.out.pixels()
    }

    pub fn black_mut(&mut self) -> &mut [u32] {
        self.black.pixels_mut()
    }

    /// Posle oblast `a` vystupu oknu (`UpdateLayeredWindow` s alfou po pixeloch; okno ostava na mieste).
    pub fn present(&self, hwnd: HWND, a: Area) -> bool {
        // SAFETY: DC obrazovky sa uvolni (ReleaseDC); DIB a struktury ziju pocas volania.
        unsafe {
            let screen = GetDC(None);
            let size = SIZE {
                cx: self.w,
                cy: self.h,
            };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let dirty = RECT {
                left: a.x0,
                top: a.y0,
                right: a.x1,
                bottom: a.y1,
            };
            let info = UPDATELAYEREDWINDOWINFO {
                cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
                hdcDst: screen,
                pptDst: std::ptr::null(),
                psize: &size,
                hdcSrc: self.out.dc,
                pptSrc: &src,
                crKey: Default::default(),
                pblend: &blend,
                dwFlags: ULW_ALPHA,
                prcDirty: &dirty,
            };
            let ok = UpdateLayeredWindowIndirect(hwnd, &info).as_bool();
            ReleaseDC(None, screen);
            ok
        }
    }

    /// Zlozi oblast `a` z oboch prechodov nad sklo (pozri [`crate::glass::compose`]).
    pub fn compose_glass(&mut self, a: Area, st: &crate::glass::GlassStyle, content: u8) {
        let (b, w) = (self.black.pixels(), self.white.pixels());
        // vystup je samostatny DIB, takze vypozicky sa neprekryvaju
        // SAFETY: `out` je samostatny DIB s `len` pixelmi; ziadna ina vypozicka jeho pamate nezije.
        let out = unsafe { std::slice::from_raw_parts_mut(self.out.bits, self.out.len) };
        crate::glass::compose(b, w, out, self.w, self.h, a, st, content);
    }

    /// Vystup mimo obdlznika `a` je uplne priehladny (pocas vysuvania: este neodkryta cast okna). Okno sa tak
    /// neorezava oblastou - pri skle by DWM novo odkryty pas pri kazdom zvacseni oblasti na snimku ukazal bielo.
    pub fn clear_outside(&mut self, a: Area) {
        // SAFETY: `out` je samostatny DIB s `len` pixelmi; ziadna ina vypozicka jeho pamate nezije.
        let out = unsafe { std::slice::from_raw_parts_mut(self.out.bits, self.out.len) };
        let w = self.w.max(0) as usize;
        for (y, row) in out.chunks_mut(w.max(1)).enumerate() {
            let y = y as i32;
            if y < a.y0 || y >= a.y1 {
                row.fill(0);
            } else {
                let (x0, x1) = (
                    a.x0.clamp(0, self.w) as usize,
                    a.x1.clamp(0, self.w) as usize,
                );
                row[..x0].fill(0);
                row[x1.max(x0)..].fill(0);
            }
        }
    }

    /// Skopiruje oblast `a` vystupu (premultiplied BGRA) do DC okna so sklom (`BitBlt`, alfa sa zachova).
    pub fn blit(&self, hdc: HDC, a: Area) {
        // SAFETY: BitBlt z nasho DIB do platneho DC okna v ramci velkosti DIB.
        unsafe {
            let _ = BitBlt(
                hdc,
                a.x0,
                a.y0,
                a.x1 - a.x0,
                a.y1 - a.y0,
                Some(self.out.dc),
                a.x0,
                a.y0,
                SRCCOPY,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(r: u32, g: u32, b: u32) -> u32 {
        r << 16 | g << 8 | b
    }

    /// Ocakavany vysledok: obsah (farba `f`, krytie `a`/255) nad pozadim (`bg`, `ba`) nad znamou farbou `k`.
    fn over_k(f: [u32; 3], a: u32, bg: [u32; 3], ba: u32, k: [u32; 3]) -> [f32; 3] {
        let mut o = [0.0; 3];
        for c in 0..3 {
            let under = bg[c] as f32 * ba as f32 / 255.0 + k[c] as f32 * (1.0 - ba as f32 / 255.0);
            o[c] = f[c] as f32 * a as f32 / 255.0 + under * (1.0 - a as f32 / 255.0);
        }
        o
    }

    /// Zlozi jeden pixel (obsah `f` s krytim `a` na cierno/bielo) a vrati (ARGB premultiplied).
    fn one(f: [u32; 3], a: u32, bg: (u8, u8, u8), ba: u8) -> u32 {
        let b = px(mul255(f[0], a), mul255(f[1], a), mul255(f[2], a));
        let w = px(
            mul255(f[0], a) + (255 - a),
            mul255(f[1], a) + (255 - a),
            mul255(f[2], a) + (255 - a),
        );
        let mut out = [0u32];
        compose(&[b], &[w], &mut out, 1, Area::full(1, 1), bg, ba);
        out[0]
    }

    /// Zlozi vystup nad nepriehladnou farbou `k` ako to robi system (premultiplied "over").
    fn on_k(argb: u32, k: [u32; 3]) -> [f32; 3] {
        let a = (argb >> 24) as f32 / 255.0;
        let (r, g, b) = ch(argb);
        [
            r as f32 + k[0] as f32 * (1.0 - a),
            g as f32 + k[1] as f32 * (1.0 - a),
            b as f32 + k[2] as f32 * (1.0 - a),
        ]
    }

    #[test]
    fn clear_outside_leaves_only_the_revealed_rectangle() {
        let mut b = LayeredBuf::new(6, 5).expect("buf");
        // SAFETY: vystupny DIB s w*h pixelmi; testovacia vypozicka.
        let out = unsafe { std::slice::from_raw_parts_mut(b.out.bits, b.out.len) };
        out.fill(0xFF11_2233);
        b.clear_outside(Area {
            x0: 1,
            y0: 0,
            x1: 4,
            y1: 3,
        });
        // SAFETY: ako vyssie.
        let out = unsafe { std::slice::from_raw_parts(b.out.bits, b.out.len) };
        for y in 0..5 {
            for x in 0..6 {
                let inside = (1..4).contains(&x) && y < 3;
                assert_eq!(out[y * 6 + x] != 0, inside, "{x},{y}");
            }
        }
    }

    #[test]
    fn empty_background_gets_exactly_the_requested_alpha() {
        for ba in [255u8, 200, 128, 26, 1] {
            let o = one([0, 0, 0], 0, (30, 34, 40), ba);
            assert_eq!(o >> 24, ba as u32, "alfa {ba}");
            let (r, g, b) = ch(o);
            // premultiplied farba pozadia
            assert!((r as i32 - mul255(30, ba as u32) as i32).abs() <= 1, "{r}");
            assert!((g as i32 - mul255(34, ba as u32) as i32).abs() <= 1);
            assert!((b as i32 - mul255(40, ba as u32) as i32).abs() <= 1);
        }
    }

    #[test]
    fn fully_transparent_keeps_alpha_one_so_the_mouse_still_hits_the_bar() {
        let o = one([0, 0, 0], 0, (30, 34, 40), 0);
        assert_eq!(o, 1 << 24, "alfa 1 a farba 0 (premultiplied)");
        // obsah sa pritom kresli normalne
        let t = one([230, 233, 238], 255, (30, 34, 40), 0);
        assert_eq!(t >> 24, 255);
        assert_eq!(ch(t), (230, 233, 238));
    }

    #[test]
    fn opaque_content_ignores_background_alpha() {
        for ba in [0u8, 80, 255] {
            let o = one([10, 120, 250], 255, (200, 200, 200), ba);
            assert_eq!((o >> 24, ch(o)), (255, (10, 120, 250)), "ba={ba}");
        }
    }

    #[test]
    fn partial_coverage_gives_partial_alpha_without_background_halo() {
        // polopriehladny okraj pismena nad uplne priehladnym pozadim: alfa = krytie, farba = F (premultiplied)
        let o = one([240, 240, 240], 128, (30, 34, 40), 0);
        assert!(
            (o >> 24) as i32 - 128 <= 1 && (o >> 24) as i32 - 128 >= -1,
            "{}",
            o >> 24
        );
        let (r, ..) = ch(o);
        assert!(
            (r as i32 - 120).abs() <= 2,
            "premultiplied 240*128/255 ~ 120, nie zmes s pozadim: {r}"
        );
    }

    #[test]
    fn composite_over_any_color_matches_direct_rendering() {
        // nahrada "vykreslit priamo na farbe K": vysledok nad K sa musi zhodovat do +-2 so vzorcom
        let bg = (30u8, 34u8, 40u8);
        for ba in [255u8, 200, 128, 60, 0] {
            for a in [0u32, 1, 40, 128, 200, 254, 255] {
                for f in [[240, 240, 240], [10, 120, 250], [0, 0, 0], [255, 255, 255]] {
                    for k in [[0, 0, 0], [255, 255, 255], [200, 30, 90], [90, 160, 40]] {
                        let want =
                            over_k(f, a, [bg.0 as u32, bg.1 as u32, bg.2 as u32], ba as u32, k);
                        let got = on_k(one(f, a, bg, ba), k);
                        for c in 0..3 {
                            assert!(
                                (want[c] - got[c]).abs() <= 3.0,
                                "ba={ba} a={a} f={f:?} k={k:?} c={c}: {want:?} vs {got:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn composes_only_the_requested_area() {
        let (w, h) = (4, 2);
        let black = vec![0u32; 8];
        let white = vec![px(255, 255, 255); 8];
        let mut out = vec![0xDEAD_BEEFu32; 8];
        compose(
            &black,
            &white,
            &mut out,
            w,
            Area {
                x0: 1,
                y0: 0,
                x1: 3,
                y1: 1,
            },
            (255, 0, 0),
            255,
        );
        for (i, v) in out.iter().enumerate() {
            let inside = i == 1 || i == 2;
            assert_eq!(*v != 0xDEAD_BEEF, inside, "i={i}");
        }
        assert_eq!(out[1], 0xFF_FF_00_00);
        let _ = h;
    }

    #[test]
    fn area_clip_and_union() {
        assert_eq!(
            Area {
                x0: -5,
                y0: -1,
                x1: 10,
                y1: 99
            }
            .clip(8, 4),
            Some(Area {
                x0: 0,
                y0: 0,
                x1: 8,
                y1: 4
            })
        );
        assert_eq!(
            Area {
                x0: 9,
                y0: 0,
                x1: 12,
                y1: 4
            }
            .clip(8, 4),
            None
        );
        assert_eq!(
            Area {
                x0: 2,
                y0: 2,
                x1: 2,
                y1: 3
            }
            .clip(8, 4),
            None,
            "prazdna"
        );
        let u = Area {
            x0: 1,
            y0: 1,
            x1: 3,
            y1: 2,
        }
        .union(Area {
            x0: 5,
            y0: 0,
            x1: 6,
            y1: 4,
        });
        assert_eq!(
            u,
            Area {
                x0: 1,
                y0: 0,
                x1: 6,
                y1: 4
            }
        );
    }

    #[test]
    fn gdi_buffers_are_allocated_and_composed() {
        let mut b = LayeredBuf::new(16, 4).expect("DIB");
        // cierny prechod: prazdny, biely: samy 255 -> cele pozadie
        let white = b.white_dc();
        assert!(!white.is_invalid());
        let n = b.black_mut().len();
        assert_eq!(n, 64);
        for v in b.white.pixels_mut() {
            *v = 0x00FF_FFFF;
        }
        b.compose(Area::full(16, 4), (10, 20, 30), 100);
        assert!(b.output().iter().all(|p| p >> 24 == 100));
        assert!(LayeredBuf::new(0, 4).is_none());
    }
}
