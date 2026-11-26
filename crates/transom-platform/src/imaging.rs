//! Obrazky pre nastroje: cisty rozbor DIB zo schranky (bez alokacie nad limity) a kodovanie BGRA -> PNG cez WIC (dokumentovane
//! rozhranie Windows). Volajuce vlakno musi mat inicializovane COM (STA alebo MTA).
use windows::core::GUID;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICBitmapEncoder, IWICBitmapFrameEncode, IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::StructuredStorage::{
    CreateStreamOnHGlobal, GetHGlobalFromStream, IPropertyBag2,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CLSCTX_INPROC_SERVER, STATFLAG_NONAME, STATSTG,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 64_000_000;

pub fn dims_ok(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE && u64::from(w) * u64::from(h) <= MAX_PIXELS
}

/// Rozmery z hlavicky DIB (prvych 12 bajtov: velkost hlavicky, sirka, vyska) **bez** pixelov: slúži na zamietnutie nadmerneho
/// obrazka este pred kopirovanim alebo dekodovanim. `None` pri kratkom, nezmyselnom (sirka alebo vyska 0, hlavicka < 40 B)
/// vstupe. Vyska sa vracia ako absolutna hodnota (zaporna = zhora nadol).
pub fn dib_dims(head: &[u8]) -> Option<(u32, u32)> {
    let hdr = le32(head, 0)?;
    let w = le32(head, 4)? as i32;
    let h = le32(head, 8)? as i32;
    if hdr < 40 || w <= 0 || h == 0 {
        return None;
    }
    Some((w as u32, h.unsigned_abs()))
}

/// Smie sa obrazok s tymito rozmermi nacitat? Vzdy plati `dims_ok` (strany, pocet bodov), navyse strana najviac `max_side`.
pub fn within(w: u32, h: u32, max_side: u32) -> bool {
    dims_ok(w, h) && w <= max_side && h <= max_side
}

fn le32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

/// Rozbor `CF_DIB` (BITMAPINFOHEADER + pixely): podporuje 24 a 32 bit (BI_RGB, BI_BITFIELDS so standardnymi maskami), zhora
/// nadol aj zdola nahor. Vrati (sirka, vyska, BGRA zhora nadol s nepriehladnou alfou). Chybny, neuplny alebo nadmerny
/// vstup = `None` (nikdy panika, nikdy alokacia nad limit).
pub fn dib_to_bgra(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let hdr = le32(d, 0)? as usize;
    if hdr < 40 || hdr > d.len() {
        return None;
    }
    let w = le32(d, 4)? as i32;
    let hraw = le32(d, 8)? as i32;
    let planes = u16::from_le_bytes([*d.get(12)?, *d.get(13)?]);
    let bpp = u16::from_le_bytes([*d.get(14)?, *d.get(15)?]);
    let comp = le32(d, 16)?;
    let clr_used = le32(d, 32)? as usize;
    if planes != 1 || w <= 0 || hraw == 0 || !(bpp == 24 || bpp == 32) || !(comp == 0 || comp == 3)
    {
        return None;
    }
    let top_down = hraw < 0;
    let h = hraw.unsigned_abs();
    let w = w as u32;
    if !dims_ok(w, h) {
        return None;
    }
    let mut off = hdr;
    if comp == 3 {
        // BITFIELDS: tri masky (ak nie su uz v hlavicke V4/V5); podporene len standardne BGRA/BGRX
        if hdr == 40 {
            let (r, g, b) = (le32(d, 40)?, le32(d, 44)?, le32(d, 48)?);
            if bpp != 32 || (r, g, b) != (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) {
                return None;
            }
            off += 12;
        } else {
            let (r, g, b) = (le32(d, 40)?, le32(d, 44)?, le32(d, 48)?);
            if bpp != 32 || (r, g, b) != (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) {
                return None;
            }
        }
    }
    off = off.checked_add(clr_used.min(256).checked_mul(4)?)?;
    let bytes_pp = usize::from(bpp / 8);
    let stride = (w as usize).checked_mul(bytes_pp)?.checked_add(3)? & !3;
    let need = stride.checked_mul(h as usize)?;
    let px = d.get(off..off.checked_add(need)?)?;
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h as usize {
        let src_row = if top_down { y } else { h as usize - 1 - y };
        let s = &px[src_row * stride..src_row * stride + (w as usize) * bytes_pp];
        let o = &mut out[y * (w as usize) * 4..(y + 1) * (w as usize) * 4];
        for x in 0..w as usize {
            o[x * 4] = s[x * bytes_pp];
            o[x * 4 + 1] = s[x * bytes_pp + 1];
            o[x * 4 + 2] = s[x * bytes_pp + 2];
            o[x * 4 + 3] = 255;
        }
    }
    Some((w, h, out))
}

/// Zakoduje BGRA (riadok po riadku, bez odsadenia) do PNG cez WIC. Rozmery a dlzka sa overuju skor nez sa nieco alokuje.
pub fn encode_png(w: u32, h: u32, bgra: &[u8]) -> Result<Vec<u8>, String> {
    if !dims_ok(w, h) || bgra.len() as u64 != u64::from(w) * u64::from(h) * 4 {
        return Err("neplatne rozmery".into());
    }
    // SAFETY: WIC volania s platnymi objektmi; pamat streamu sa cita pod zamkom s velkostou zo `Stat`/`GlobalSize`.
    unsafe {
        let e = |x: windows::core::Error| x.to_string();
        let f: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true).map_err(e)?;
        let enc: IWICBitmapEncoder = f
            .CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null::<GUID>())
            .map_err(e)?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)
            .map_err(e)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut props: Option<IPropertyBag2> = None;
        enc.CreateNewFrame(&mut frame, &mut props).map_err(e)?;
        let frame = frame.ok_or("ziadny frame")?;
        frame.Initialize(props.as_ref()).map_err(e)?;
        frame.SetSize(w, h).map_err(e)?;
        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt).map_err(e)?;
        frame.WritePixels(h, w * 4, bgra).map_err(e)?;
        frame.Commit().map_err(e)?;
        enc.Commit().map_err(e)?;
        let mut st = STATSTG::default();
        stream.Stat(&mut st, STATFLAG_NONAME).map_err(e)?;
        let hg = GetHGlobalFromStream(&stream).map_err(e)?;
        let len = (st.cbSize as usize).min(GlobalSize(hg));
        let p = GlobalLock(hg) as *const u8;
        if p.is_null() {
            return Err("zamok pamate".into());
        }
        let v = std::slice::from_raw_parts(p, len).to_vec();
        let _ = GlobalUnlock(hg);
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dib(w: i32, h: i32, bpp: u16, comp: u32, px: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&bpp.to_le_bytes());
        v.extend_from_slice(&comp.to_le_bytes());
        v.extend_from_slice(&[0; 20]);
        if comp == 3 {
            v.extend_from_slice(&0x00FF_0000u32.to_le_bytes());
            v.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
            v.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
        }
        v.extend_from_slice(px);
        v
    }

    #[test]
    fn the_dib_header_is_peeked_without_the_pixels_and_oversized_or_malformed_images_fail_the_gate()
    {
        // hlavicka s udajom 16384 x 16384, ale bez jedineho pixela: rozmery sa precitaju, brana ich odmietne (> 64 Mpx)
        let big = dib(16384, 16384, 32, 0, &[]);
        assert_eq!(dib_dims(&big[..12]), Some((16384, 16384)));
        assert!(
            dib_to_bgra(&big).is_none(),
            "ziadny pixel -> ziadna alokacia"
        );
        assert!(!within(16384, 16384, 8192));
        assert!(!within(8193, 10, 8192) && within(8192, 100, 8192));
        assert!(!within(0, 5, 8192) && !within(5, 0, 8192));
        assert!(
            !within(u32::MAX, u32::MAX, u32::MAX),
            "pocet bodov sa nepreteka"
        );
        // zaporna vyska = zhora nadol; i32::MIN nepadne
        assert_eq!(dib_dims(&dib(10, -7, 24, 0, &[])[..12]), Some((10, 7)));
        assert_eq!(
            dib_dims(&dib(10, i32::MIN, 24, 0, &[])[..12]),
            Some((10, 1 << 31))
        );
        assert!(!within(10, 1 << 31, u32::MAX));
        // poskodene hlavicky: kratke, nulove rozmery, zaporna sirka, zla velkost hlavicky
        assert_eq!(dib_dims(&[]), None);
        assert_eq!(dib_dims(&big[..11]), None);
        assert_eq!(dib_dims(&dib(0, 5, 24, 0, &[])[..12]), None);
        assert_eq!(dib_dims(&dib(5, 0, 24, 0, &[])[..12]), None);
        assert_eq!(dib_dims(&dib(-5, 5, 24, 0, &[])[..12]), None);
        let mut short_hdr = dib(5, 5, 24, 0, &[]);
        short_hdr[..4].copy_from_slice(&12u32.to_le_bytes());
        assert_eq!(dib_dims(&short_hdr[..12]), None);
        // plne dekodovanie odmieta aj rozumne rozmery s useknutymi pixelmi a nepodporovane formaty (1/4/8/16 bit, RLE)
        assert!(dib_to_bgra(&dib(100, 100, 24, 0, &[0; 100])).is_none());
        for bpp in [1u16, 4, 8, 16] {
            assert!(dib_to_bgra(&dib(2, 2, bpp, 0, &[0; 64])).is_none(), "{bpp}");
        }
        assert!(dib_to_bgra(&dib(2, 2, 8, 1, &[0; 64])).is_none(), "RLE8");
    }

    #[test]
    fn a_bottom_up_24_bit_dib_is_flipped_padded_and_made_opaque() {
        // 2x2, riadky po 8 B (6 + 2 odsadenie); dolny riadok prvy: (B,G,R)
        let px = [1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0];
        let (w, h, bgra) = dib_to_bgra(&dib(2, 2, 24, 0, &px)).unwrap();
        assert_eq!((w, h), (2, 2));
        // horny riadok je povodny druhy (7,8,9 / 10,11,12)
        assert_eq!(&bgra[..8], &[7, 8, 9, 255, 10, 11, 12, 255]);
        assert_eq!(&bgra[8..], &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn a_top_down_32_bit_dib_keeps_row_order_and_forces_opaque_alpha() {
        let px = [1, 2, 3, 0, 4, 5, 6, 0];
        let (w, h, bgra) = dib_to_bgra(&dib(2, -1, 32, 0, &px)).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(bgra, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        assert!(
            dib_to_bgra(&dib(1, 1, 32, 3, &[9, 9, 9, 0])).is_some(),
            "BITFIELDS so standardnymi maskami"
        );
    }

    #[test]
    fn hostile_dibs_are_rejected_without_allocating_or_panicking() {
        assert!(dib_to_bgra(&[]).is_none());
        assert!(dib_to_bgra(&[0; 39]).is_none());
        assert!(
            dib_to_bgra(&dib(2, 2, 24, 0, &[1; 4])).is_none(),
            "neuplne pixely"
        );
        assert!(
            dib_to_bgra(&dib(2, 2, 8, 0, &[1; 16])).is_none(),
            "paletove nepodporujeme"
        );
        assert!(
            dib_to_bgra(&dib(2, 2, 24, 1, &[1; 16])).is_none(),
            "RLE nepodporujeme"
        );
        assert!(dib_to_bgra(&dib(0, 2, 24, 0, &[1; 16])).is_none());
        assert!(dib_to_bgra(&dib(-5, 2, 24, 0, &[1; 16])).is_none());
        assert!(
            dib_to_bgra(&dib(100_000, 100_000, 32, 0, &[1; 16])).is_none(),
            "obrovske rozmery bez alokacie"
        );
        assert!(dib_to_bgra(&dib(i32::MAX, i32::MIN, 32, 0, &[1; 16])).is_none());
        let mut bad = dib(2, 2, 24, 0, &[1; 16]);
        bad[0] = 255;
        bad[1] = 255; // hlavicka dlhsia ako data
        assert!(dib_to_bgra(&bad).is_none());
        let mut masks = dib(1, 1, 32, 3, &[1; 4]);
        masks[40] = 1; // nestandardna maska
        assert!(dib_to_bgra(&masks).is_none());
    }

    #[test]
    fn wic_encodes_a_valid_png_and_refuses_bad_input() {
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
        };
        std::thread::spawn(|| {
            // SAFETY: COM na testovacom vlakne.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            let bgra: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
            let png = encode_png(4, 3, &bgra).expect("PNG");
            assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
            assert!(png.len() > 40);
            assert!(encode_png(0, 3, &bgra).is_err());
            assert!(encode_png(4, 3, &bgra[..10]).is_err());
            assert!(encode_png(20_000, 1, &[]).is_err());
            // SAFETY: parter k inicializacii.
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }
}
