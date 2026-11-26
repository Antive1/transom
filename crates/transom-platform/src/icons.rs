//! Ikony transom: `.ico` vlozene do binarky (generuje ich `transom-render/examples/brand_export.rs` z jednej geometrie znacky)
//! -> `HICON` v presnej velkosti pre dany DPI. Nic sa necita zo suboru za behu.
//!
//! Obrazky v `.ico` su BMP DIB (32 bpp, **nepremultiplikovana** alfa + AND maska) - presne to ocakava
//! `CreateIconFromResourceEx`; test overuje, ze priehladne okraje ostanu priehladne na bielom aj cernom podklade.
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, DestroyIcon, HICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON,
};

/// Ikona aplikacie (dlazdica Ink + lista Signal + "w"): 16-256 px.
pub const APP_ICO: &[u8] = include_bytes!("../../../assets/brand/transom.ico");
/// Ikona v oblasti oznameni pre tmavy panel uloh (svetle "w"): 16-48 px.
pub const TRAY_DARK_ICO: &[u8] = include_bytes!("../../../assets/brand/transom-tray-dark.ico");
/// Ikona v oblasti oznameni pre svetly panel uloh (tmave "w", hlboka lista).
pub const TRAY_LIGHT_ICO: &[u8] = include_bytes!("../../../assets/brand/transom-tray-light.ico");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IcoEntry {
    pub size: u32,
    pub offset: usize,
    pub len: usize,
}

/// Polozky `.ico` (neplatny alebo useknuty subor = prazdny zoznam).
pub fn parse_ico(b: &[u8]) -> Vec<IcoEntry> {
    if b.len() < 6 || b[0..4] != [0, 0, 1, 0] {
        return Vec::new();
    }
    let n = u16::from_le_bytes([b[4], b[5]]) as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let Some(e) = b.get(6 + 16 * i..6 + 16 * (i + 1)) else {
            return Vec::new();
        };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let offset = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        if offset.checked_add(len).is_none_or(|end| end > b.len()) {
            return Vec::new();
        }
        out.push(IcoEntry {
            size: if e[0] == 0 { 256 } else { e[0] as u32 },
            offset,
            len,
        });
    }
    out
}

/// Najmensia polozka, ktora je aspon `want` px velka (inak najvacsia): ikona sa tak nikdy nezvacsuje, ak sa vyhnut.
pub fn pick(entries: &[IcoEntry], want: u32) -> Option<IcoEntry> {
    entries
        .iter()
        .filter(|e| e.size >= want)
        .min_by_key(|e| e.size)
        .or_else(|| entries.iter().max_by_key(|e| e.size))
        .copied()
}

/// `HICON` z `.ico` vo velkosti `want_px` (volajuci ho uvolni `DestroyIcon`, pokial ho nevlastni okno/system).
pub fn hicon(ico: &[u8], want_px: u32) -> Option<HICON> {
    let entries = parse_ico(ico);
    let e = pick(&entries, want_px)?;
    let data = ico.get(e.offset..e.offset + e.len)?;
    // SAFETY: `data` je platny obraz ikony (BITMAPINFOHEADER + pixely + maska), ako dokumentuje CreateIconFromResourceEx.
    unsafe {
        CreateIconFromResourceEx(
            data,
            true,
            0x0003_0000,
            want_px as i32,
            want_px as i32,
            LR_DEFAULTCOLOR,
        )
        .ok()
    }
}

/// Systemova velkost ikony pre dany DPI (`small` = 16 px pri 100 %, inak 32 px).
pub fn system_icon_px(dpi: u32, small: bool) -> u32 {
    // SAFETY: cisto citanie systemovej metriky.
    let v = unsafe { GetSystemMetricsForDpi(if small { SM_CXSMICON } else { SM_CXICON }, dpi) };
    (v.max(16)) as u32
}

pub fn destroy(h: HICON) {
    // SAFETY: `h` vytvoril `hicon` a nema ho ziadne okno.
    unsafe {
        let _ = DestroyIcon(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL};

    #[test]
    fn embedded_icons_have_the_expected_sizes() {
        let sizes = |b| parse_ico(b).iter().map(|e| e.size).collect::<Vec<_>>();
        assert_eq!(sizes(APP_ICO), vec![16, 20, 24, 32, 40, 48, 64, 128, 256]);
        assert_eq!(sizes(TRAY_DARK_ICO), vec![16, 20, 24, 32, 48]);
        assert_eq!(sizes(TRAY_LIGHT_ICO), vec![16, 20, 24, 32, 48]);
    }

    #[test]
    fn picks_the_smallest_entry_that_is_big_enough() {
        let e = parse_ico(APP_ICO);
        let size = |w| pick(&e, w).unwrap().size;
        assert_eq!(
            (
                size(16),
                size(17),
                size(20),
                size(30),
                size(33),
                size(100),
                size(500)
            ),
            (16, 20, 20, 32, 40, 128, 256)
        );
        assert!(pick(&[], 16).is_none());
        // poskodeny subor = ziadne polozky, nie panika
        assert!(parse_ico(&APP_ICO[..40]).is_empty());
        assert!(parse_ico(b"nie je ico").is_empty());
        assert!(parse_ico(&[]).is_empty());
    }

    /// Vykresli ikonu na jednofarebny podklad `bg` (BGRX) a vrati pixely.
    fn draw_on(h: HICON, px: i32, bg: u32) -> Vec<u32> {
        // SAFETY: DC a DIB sa vytvoria a uvolnia v tomto bloku; pixely sa citaju v ramci `px * px` z DIB s tou velkostou.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: px,
                    biHeight: -px,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(dc, bmp.into());
            let buf = std::slice::from_raw_parts_mut(bits as *mut u32, (px * px) as usize);
            buf.fill(bg);
            DrawIconEx(dc, 0, 0, h, px, px, 0, None, DI_NORMAL).unwrap();
            let out = buf.to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            out
        }
    }

    #[test]
    fn icon_alpha_is_respected_on_white_and_black() {
        // overuje konvenciu alfy ikony: priehladny roh dlazdice ostane podkladom, stred je plny na oboch podkladoch
        for ico in [APP_ICO] {
            for px in [16, 32, 48] {
                let h = hicon(ico, px as u32).expect("hicon");
                let on_white = draw_on(h, px, 0x00FF_FFFF);
                let on_black = draw_on(h, px, 0x0000_0000);
                destroy(h);
                let c = |v: &[u32], x: i32, y: i32| v[(y * px + x) as usize] & 0x00FF_FFFF;
                assert_eq!(c(&on_white, 0, 0), 0x00FF_FFFF, "{px}: roh na bielom");
                assert_eq!(c(&on_black, 0, 0), 0, "{px}: roh na cernom");
                let mid = (px / 2, px - 2);
                assert_eq!(
                    c(&on_white, mid.0, mid.1),
                    c(&on_black, mid.0, mid.1),
                    "{px}: stred dlazdice je plny"
                );
            }
        }
        // ikona oblasti oznameni nema dlazdicu: pozadie prezera, "w" je plne
        for ico in [TRAY_DARK_ICO, TRAY_LIGHT_ICO] {
            let h = hicon(ico, 32).expect("tray hicon");
            let w = draw_on(h, 32, 0x00FF_FFFF);
            let b = draw_on(h, 32, 0);
            destroy(h);
            assert_eq!(w[0] & 0xFFFFFF, 0xFFFFFF);
            assert_eq!(b[0] & 0xFFFFFF, 0);
            assert!(
                w.iter().zip(&b).any(|(x, y)| x == y),
                "existuju plne kryte pixely"
            );
        }
    }

    #[test]
    fn system_icon_sizes_scale_with_dpi() {
        assert!(
            system_icon_px(96, true) >= 16 && system_icon_px(192, true) >= system_icon_px(96, true)
        );
        assert!(system_icon_px(96, false) >= 32);
    }
}
