//! Miniatury obrazkov (kniznica snimok obrazovky): dekodovanie cez WIC (dokumentovane rozhranie Windows; kazdy nainstalovany
//! kodek) priamo na velkost miniatury. Volajuce vlakno musi mat inicializovane COM; volat sa ma **mimo UI vlakna**.
//!
//! Obrana pred pamatou a zdrojmi:
//! * velkost suboru (<= 64 MiB) a atributy sa overia **pred** otvorenim; subor, ktory nie je plne lokalne (cloud), sa nikdy
//!   neotvara, aby sa nestiahol;
//! * rozmery sa precitaju z hlavicky (`GetSize`) a overia (`imaging::dims_ok`) skor, nez sa alokuje akykolvek pixelovy buffer;
//! * dekoduje sa cez skalovac `Fant` rovno do buffera velkosti miniatury (nikdy sa nealokuje cely obrazok);
//! * subor sa otvara vlastnou uchytkou so zdielanim READ|WRITE|DELETE a zatvori sa hned po dekodovani, takze pouzivatel moze
//!   subor v Prieskumnikovi kedykolvek premenovat, presunut alebo zmazat; subor sa nikdy nemeni.
//!
//! Cesty a nazvy sa nelogu; chyba je len druh (`ThumbError`).
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use windows::core::{Interface, GUID};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

/// Najvacsi subor, ktory sa dekoduje.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Pozadie, na ktore sa zlozia priehladne pixely (miniatura je nepriehladna).
const BACKDROP: u8 = 0x80;

const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL_ON_OPEN: u32 = 0x4_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbError {
    /// Subor neexistuje (uz).
    Missing,
    /// Subor sa nedal otvorit (pristup, zamknuty, nie je to subor).
    Unreadable,
    /// Prilis velky subor alebo obrazok.
    TooLarge,
    /// Subor nie je plne lokalne (cloud): nedekoduje sa.
    Cloud,
    /// Nie je to obrazok, ktory by dokazal nejaky nainstalovany kodek, alebo je poskodeny.
    Unsupported,
}

/// Miniatura: nepriehladne RGBA riadky zhora nadol (`w * h` pixelov) a povodne rozmery obrazka.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<[u8; 4]>,
    pub src_w: u32,
    pub src_h: u32,
}

/// Rozmer miniatury: vojde sa do `bw x bh`, pomer stran sa zachova, nikdy sa nezvacsuje a je aspon 1 x 1.
pub fn target_size(w: u32, h: u32, bw: u32, bh: u32) -> (u32, u32) {
    if w == 0 || h == 0 || bw == 0 || bh == 0 {
        return (1, 1);
    }
    let s = (f64::from(bw) / f64::from(w))
        .min(f64::from(bh) / f64::from(h))
        .min(1.0);
    (
        ((f64::from(w) * s).round() as u32).clamp(1, bw),
        ((f64::from(h) * s).round() as u32).clamp(1, bh),
    )
}

pub fn is_cloud_attr(attrs: u32) -> bool {
    attrs & (ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS) != 0
}

/// Dekoduje `path` do miniatury najviac `box_w x box_h` px. Neprejde cez cloud, siet ani nadmerne subory.
pub fn decode(path: &str, box_w: u32, box_h: u32) -> Result<Thumb, ThumbError> {
    if box_w == 0 || box_h == 0 || box_w > 1024 || box_h > 1024 {
        return Err(ThumbError::Unsupported);
    }
    let io = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => ThumbError::Missing,
        _ => ThumbError::Unreadable,
    };
    // atributy a velkost bez citania obsahu; symbolicky odkaz / priecinok nie je obrazok
    let md = std::fs::symlink_metadata(path).map_err(io)?;
    if !md.is_file() {
        return Err(ThumbError::Unreadable);
    }
    if is_cloud_attr(md.file_attributes()) {
        return Err(ThumbError::Cloud);
    }
    if md.len() > MAX_FILE_BYTES {
        return Err(ThumbError::TooLarge);
    }
    if md.len() == 0 {
        return Err(ThumbError::Unsupported);
    }
    // vlastna uchytka so zdielanim READ|WRITE|DELETE (1|2|4): neblokuje mazanie ani premenovanie
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(7)
        .open(path)
        .map_err(io)?;
    // SAFETY: WIC objekty zijú len v tejto funkcii; `file` je deklarovany pred nimi, takze sa zatvori az po nich.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|_| ThumbError::Unsupported)?;
        let dec = factory
            .CreateDecoderFromFileHandle(
                file.as_raw_handle() as usize,
                std::ptr::null::<GUID>(),
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(|_| ThumbError::Unsupported)?;
        let frame = dec.GetFrame(0).map_err(|_| ThumbError::Unsupported)?;
        let (mut sw, mut sh) = (0u32, 0u32);
        frame
            .GetSize(&mut sw, &mut sh)
            .map_err(|_| ThumbError::Unsupported)?;
        // rozmery z hlavicky sa overia skor, nez sa alokuje akykolvek pixelovy buffer
        if !crate::imaging::dims_ok(sw, sh) {
            return Err(ThumbError::TooLarge);
        }
        let (tw, th) = target_size(sw, sh, box_w, box_h);
        let src: IWICBitmapSource = frame.cast().map_err(|_| ThumbError::Unsupported)?;
        let scaler = factory
            .CreateBitmapScaler()
            .map_err(|_| ThumbError::Unsupported)?;
        scaler
            .Initialize(&src, tw, th, WICBitmapInterpolationModeFant)
            .map_err(|_| ThumbError::Unsupported)?;
        let conv = factory
            .CreateFormatConverter()
            .map_err(|_| ThumbError::Unsupported)?;
        conv.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(|_| ThumbError::Unsupported)?;
        let mut buf = vec![0u8; (tw as usize) * (th as usize) * 4];
        conv.CopyPixels(std::ptr::null(), tw * 4, &mut buf)
            .map_err(|_| ThumbError::Unsupported)?;
        let rgba = buf
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                // premultiplikovane BGRA zlozene na pozadie -> nepriehladne RGBA
                let a = u32::from(p[3]);
                let over =
                    |c: u8| (u32::from(c) + u32::from(BACKDROP) * (255 - a) / 255).min(255) as u8;
                [over(p[2]), over(p[1]), over(p[0]), 255]
            })
            .collect();
        Ok(Thumb {
            w: tw,
            h: th,
            rgba,
            src_w: sw,
            src_h: sh,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    fn com() {
        // SAFETY: inicializacia COM pre testovacie vlakno (opakovane volanie je v poriadku).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "transom-thumb-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// PNG `w x h`: horna cast cervena (lava polovica) / modra (prava), spodnych 25 % zelene; `px` doplni volajuci.
    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b, a] = f(x, y);
                bgra.extend_from_slice(&[b, g, r, a]);
            }
        }
        crate::imaging::encode_png(w, h, &bgra).expect("png")
    }

    fn banded(x: u32, y: u32) -> [u8; 4] {
        if y >= 150 {
            [0, 200, 0, 255]
        } else if x < 200 {
            [220, 0, 0, 255]
        } else {
            [0, 0, 220, 255]
        }
    }

    #[test]
    fn the_thumbnail_size_keeps_the_aspect_ratio_never_upscales_and_is_at_least_one_pixel() {
        assert_eq!(target_size(400, 200, 100, 100), (100, 50));
        assert_eq!(target_size(200, 400, 100, 100), (50, 100));
        assert_eq!(target_size(3, 2, 100, 100), (3, 2), "nezvacsuje sa");
        assert_eq!(
            target_size(16_384, 1, 100, 100),
            (100, 1),
            "tenky pas ma aspon 1 px"
        );
        assert_eq!(target_size(0, 5, 100, 100), (1, 1));
        assert!(is_cloud_attr(0x40_0000) && is_cloud_attr(0x4_0000) && is_cloud_attr(0x1000));
        assert!(!is_cloud_attr(0x20) && !is_cloud_attr(0x400));
    }

    #[test]
    fn a_png_is_scaled_to_the_box_with_the_right_orientation_and_colours_and_stays_untouched() {
        com();
        let d = scratch("ok");
        let p = d.join("shot.png");
        std::fs::write(&p, png(400, 200, banded)).unwrap();
        let before = std::fs::read(&p).unwrap();
        let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        let t = decode(p.to_str().unwrap(), 100, 100).expect("miniatura");
        assert_eq!((t.w, t.h, t.src_w, t.src_h), (100, 50, 400, 200));
        assert_eq!(t.rgba.len(), 5000);
        let px = |x: usize, y: usize| t.rgba[y * 100 + x];
        assert_eq!(px(10, 10), [220, 0, 0, 255], "vlavo hore cervena");
        assert_eq!(px(90, 10), [0, 0, 220, 255], "vpravo hore modra");
        assert_eq!(
            px(50, 45),
            [0, 200, 0, 255],
            "dole zelena (spravne poradie riadkov)"
        );
        // subor sa nezmenil a nie je zamknuty: da sa premenovat aj zmazat hned po dekodovani
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), mtime);
        let q = d.join("renamed.png");
        std::fs::rename(&p, &q).expect("premenovanie po dekodovani");
        std::fs::remove_file(&q).expect("zmazanie po dekodovani");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn transparency_is_composited_onto_a_neutral_backdrop_and_small_images_are_not_enlarged() {
        com();
        let d = scratch("alpha");
        let p = d.join("a.png");
        std::fs::write(
            &p,
            png(4, 2, |x, _| {
                if x < 2 {
                    [0, 0, 0, 0]
                } else {
                    [255, 255, 255, 255]
                }
            }),
        )
        .unwrap();
        let t = decode(p.to_str().unwrap(), 100, 100).unwrap();
        assert_eq!((t.w, t.h), (4, 2));
        assert_eq!(
            t.rgba[0],
            [0x80, 0x80, 0x80, 255],
            "priehladny pixel = pozadie"
        );
        assert_eq!(t.rgba[3], [255, 255, 255, 255]);
        // BMP (iny kodek): 3 x 2, 24 bit
        let mut bmp = Vec::new();
        let row = |c: [u8; 3]| [c, c, c].concat();
        let pix: Vec<u8> = [row([0, 0, 255]), vec![0; 3], row([255, 0, 0]), vec![0; 3]].concat();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54 + pix.len() as u32).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&3i32.to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&pix);
        let b = d.join("b.bmp");
        std::fs::write(&b, &bmp).unwrap();
        let t = decode(b.to_str().unwrap(), 64, 64).expect("bmp");
        assert_eq!((t.w, t.h), (3, 2));
        // BMP je zdola nahor: prvy riadok v subore (cerveny, BGR 0,0,255) je spodny
        assert_eq!(t.rgba[0], [0, 0, 255, 255], "horny riadok je modry");
        assert_eq!(
            t.rgba[3],
            [255, 0, 0, 255],
            "BMP je zdola nahor: prvy riadok suboru (cerveny) je spodny"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn missing_malformed_empty_locked_oversized_and_non_image_files_fail_with_a_kind_not_a_panic() {
        com();
        let d = scratch("bad");
        let path = |n: &str| d.join(n).to_str().unwrap().to_string();
        assert_eq!(
            decode(&path("nope.png"), 64, 64).unwrap_err(),
            ThumbError::Missing
        );
        // poskodeny obsah, text pod priponou .png, prazdny subor
        std::fs::write(
            d.join("junk.png"),
            (0..2000u32)
                .map(|i| (i * 31 % 251) as u8)
                .collect::<Vec<u8>>(),
        )
        .unwrap();
        assert_eq!(
            decode(&path("junk.png"), 64, 64).unwrap_err(),
            ThumbError::Unsupported
        );
        std::fs::write(d.join("text.png"), "toto nie je obrazok").unwrap();
        assert_eq!(
            decode(&path("text.png"), 64, 64).unwrap_err(),
            ThumbError::Unsupported
        );
        std::fs::write(d.join("empty.png"), b"").unwrap();
        assert_eq!(
            decode(&path("empty.png"), 64, 64).unwrap_err(),
            ThumbError::Unsupported
        );
        // useknuty PNG (napr. subor sa este zapisuje): WIC vrati co sa nacitalo alebo chybu; v oboch pripadoch rozumny vysledok a
        // ziadna panika. Po dokonceni zapisu sa zmeni velkost aj cas, takze cache (kluc s casom a velkostou) nacita novu miniaturu.
        let full = png(300, 300, banded);
        std::fs::write(d.join("cut.png"), &full[..full.len() / 3]).unwrap();
        if let Ok(t) = decode(&path("cut.png"), 64, 64) {
            assert!(t.w > 0 && t.h > 0 && t.rgba.len() == (t.w * t.h) as usize);
        }
        // priecinok pod menom obrazka
        std::fs::create_dir_all(d.join("dir.png")).unwrap();
        assert_eq!(
            decode(&path("dir.png"), 64, 64).unwrap_err(),
            ThumbError::Unreadable
        );
        // nadmerny subor (nad 64 MiB) sa odmietne este pred otvorenim a citanim
        let big = std::fs::File::create(d.join("big.png")).unwrap();
        big.set_len(MAX_FILE_BYTES + 1).unwrap();
        drop(big);
        assert_eq!(
            decode(&path("big.png"), 64, 64).unwrap_err(),
            ThumbError::TooLarge
        );
        // nadmerne rozmery v hlavicke (BMP 30000 x 30000, telo chyba): odmietnute z hlavicky, nic sa nealokuje
        let mut bmp = Vec::new();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54u32 + 16).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&30_000i32.to_le_bytes());
        bmp.extend_from_slice(&30_000i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&[0; 16]);
        std::fs::write(d.join("huge.bmp"), &bmp).unwrap();
        let e = decode(&path("huge.bmp"), 64, 64).unwrap_err();
        assert!(
            matches!(e, ThumbError::TooLarge | ThumbError::Unsupported),
            "{e:?}"
        );
        // subor otvoreny inym procesom bez zdielania sa neda precitat: Unreadable, nie zaseknutie
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(d.join("junk.png"))
            .unwrap();
        assert_eq!(
            decode(&path("junk.png"), 64, 64).unwrap_err(),
            ThumbError::Unreadable
        );
        drop(held);
        // nespravna velkost schranky miniatury
        assert!(
            decode(&path("junk.png"), 0, 64).is_err()
                && decode(&path("junk.png"), 5000, 64).is_err()
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
