//! Maly rastrovy obrazok (obal albumu / nahlad videa) a cista logika jeho zmensenia a zlozenia na pozadie.
//! Dekodovanie (WIC) robi media vlakno; tu je len prenosny vypocet, ktory sa da testovat.

/// Stvorcovy alebo obdlznikovy obrazok, pixely `0xAARRGGBB` s **premultiplikovanou** alfou, riadky zhora nadol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

impl Image {
    /// Z bajtov BGRA (premultiplikovana alfa, ako `GUID_WICPixelFormat32bppPBGRA`). `None` pri nespravnej dlzke.
    pub fn from_pbgra(w: u32, h: u32, bytes: &[u8]) -> Option<Image> {
        if bytes.len() != (w as usize) * (h as usize) * 4 || w == 0 || h == 0 {
            return None;
        }
        let px = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Some(Image { w, h, px })
    }

    /// Jednofarebny nepriehladny obrazok (testy, nahrada).
    pub fn solid(w: u32, h: u32, rgb: u32) -> Image {
        Image {
            w,
            h,
            px: vec![0xFF00_0000 | (rgb & 0x00FF_FFFF); (w * h) as usize],
        }
    }
}

/// FNV-1a 64 - identita obsahu obrazka (cache: rovnake bajty sa dekoduju len raz).
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn ch(v: u32, shift: u32) -> u32 {
    (v >> shift) & 0xFF
}

/// Stvorec `size` x `size` px: stredovy stvorcovy vyrez obrazka zmenseny priemerovanim plochy (box filter) a
/// zlozeny na nepriehladne pozadie `bg` (`0xRRGGBB`). Vysledok `0x00RRGGBB`, riadky zhora nadol.
pub fn compose_square(img: &Image, size: u32, bg: u32) -> Vec<u32> {
    if size == 0 || img.w == 0 || img.h == 0 || img.px.len() < (img.w * img.h) as usize {
        return vec![bg & 0x00FF_FFFF; (size * size) as usize];
    }
    let side = img.w.min(img.h);
    let ox = (img.w - side) / 2;
    let oy = (img.h - side) / 2;
    let mut out = Vec::with_capacity((size * size) as usize);
    for j in 0..size {
        let y0 = (j * side) / size;
        let y1 = (((j + 1) * side).div_ceil(size)).max(y0 + 1).min(side);
        for i in 0..size {
            let x0 = (i * side) / size;
            let x1 = (((i + 1) * side).div_ceil(size)).max(x0 + 1).min(side);
            let (mut a, mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let row = ((oy + y) * img.w + ox) as usize;
                for x in x0..x1 {
                    let p = img.px[row + x as usize];
                    a += ch(p, 24);
                    r += ch(p, 16);
                    g += ch(p, 8);
                    b += ch(p, 0);
                    n += 1;
                }
            }
            let (a, r, g, b) = (a / n, r / n, g / n, b / n);
            // premultiplikovane: out = src + bg * (1 - a)
            let mix = |src: u32, bgc: u32| (src + bgc * (255 - a) / 255).min(255);
            out.push(mix(r, ch(bg, 16)) << 16 | mix(g, ch(bg, 8)) << 8 | mix(b, ch(bg, 0)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_scales_to_solid() {
        let img = Image::solid(96, 96, 0x336699);
        for size in [22, 33, 44, 96, 120] {
            let out = compose_square(&img, size, 0xFFFFFF);
            assert_eq!(out.len(), (size * size) as usize);
            assert!(out.iter().all(|&p| p == 0x336699), "size {size}");
        }
    }

    #[test]
    fn transparent_shows_background() {
        let img = Image::from_pbgra(2, 2, &[0u8; 16]).unwrap();
        assert!(compose_square(&img, 4, 0x1E2228)
            .iter()
            .all(|&p| p == 0x1E2228));
    }

    #[test]
    fn wide_image_is_center_cropped() {
        // 3x1: cerveny | zeleny | modry -> stredovy stvorec 1x1 je zeleny
        let mut img = Image::solid(3, 1, 0);
        img.px = vec![0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF];
        assert_eq!(compose_square(&img, 2, 0), vec![0x00FF00; 4]);
    }

    #[test]
    fn bad_input_gives_background() {
        assert_eq!(Image::from_pbgra(2, 2, &[0; 3]), None);
        let broken = Image {
            w: 4,
            h: 4,
            px: vec![],
        };
        assert!(compose_square(&broken, 3, 0x123456)
            .iter()
            .all(|&p| p == 0x123456));
    }

    #[test]
    fn hash_distinguishes_content() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
    }
}
