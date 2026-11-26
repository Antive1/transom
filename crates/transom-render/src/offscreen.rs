//! Odfiltrovane kreslenie do pamate (DIB + DC) pre kontroly vzhladu a testy: skutocny `GdiPainter` bez okna a PNG
//! zapisovac (nekomprimovany deflate - snimky su len na kontrolu). Nic nemeni v systeme (DPI, jazyk, tema).
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC,
};

pub struct Surface {
    pub dc: HDC,
    bmp: windows::Win32::Graphics::Gdi::HBITMAP,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Surface {
        // SAFETY: DC a DIB vlastni `Surface` (uvolni Drop); pri chybe CreateDIBSection test spadne (len offscreen nastroje).
        unsafe {
            let dc = CreateCompatibleDC(None);
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
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp =
                CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).expect("DIB");
            SelectObject(dc, bmp.into());
            Surface {
                dc,
                bmp,
                bits: bits as *mut u32,
                w,
                h,
            }
        }
    }
    pub fn rgb(&self) -> Vec<u8> {
        let n = (self.w * self.h) as usize;
        // SAFETY: DIB ma `w * h` pixelov a zije s `self`.
        let px = unsafe { std::slice::from_raw_parts(self.bits, n) };
        let mut out = Vec::with_capacity(n * 3);
        for v in px {
            out.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, *v as u8]);
        }
        out
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: objekty vytvoril `Surface::new`, uvolnia sa raz.
        unsafe {
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, e) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *e = c;
    }
    !data.iter().fold(!0u32, |c, b| {
        table[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for d in data {
        a = (a + *d as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

/// Minimalny PNG (RGB 8 bit, nekomprimovany deflate - snimky su len na kontrolu).
pub fn png(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * w as usize * 3..(y + 1) * w as usize * 3]);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |ty: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = ty.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_has_valid_signature_checksums_and_size() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| i as u8).collect();
        let b = png(4, 3, &rgb);
        assert_eq!(&b[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // IHDR: sirka 4, vyska 3
        assert_eq!(&b[16..24], &[0, 0, 0, 4, 0, 0, 0, 3]);
        // kazdy chunk: CRC sedi
        let mut i = 8;
        let mut seen = vec![];
        while i < b.len() {
            let n = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
            let body = &b[i + 4..i + 8 + n];
            let crc = u32::from_be_bytes(b[i + 8 + n..i + 12 + n].try_into().unwrap());
            assert_eq!(crc32(body), crc, "{:?}", &body[..4]);
            seen.push(String::from_utf8_lossy(&body[..4]).to_string());
            i += 12 + n;
        }
        assert_eq!(seen, ["IHDR", "IDAT", "IEND"]);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn large_images_are_split_into_stored_blocks() {
        let rgb = vec![7u8; 300 * 100 * 3];
        let b = png(300, 100, &rgb);
        assert!(b.len() > rgb.len());
    }
}
