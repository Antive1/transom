//! Ikony pre vysledky vyhladavaca cez dokumentovane API shellu. Vsetko sa robi **mimo UI vlakna** (volajuce vlakno ma COM,
//! najlepsie STA) a vysledkom su len pixely (RGBA, nepremultiplikovana alfa) v presnej velkosti `px`, takze po zmene DPI sa
//! ikona nacita znova v ostrej velkosti a ziadny uchyt GDI ani `HICON` nepresiahne toto volanie.
//!
//! Zdroje (`IconSpec`), od najlacnejsieho:
//! * `Ext`: ikona **typu** suboru podla pripony (`SHGetFileInfoW` s `SHGFI_USEFILEATTRIBUTES`: k suboru sa vobec nepristupuje,
//!   funguje aj pre chybajuce, nedostupne, sietove a cloudove subory) + `SHDefExtractIconW` v presnej velkosti;
//! * `Folder`: standardna ikona priecinka (`SHGetStockIconInfo`, `SIID_FOLDER`), bez pristupu na disk;
//! * `Apps`: aplikacia zo zlozky "Aplikacie" (`shell:AppsFolder\...`, klasicka aj balena) cez `IShellItemImageFactory`;
//! * `Item`: subor s vlastnou ikonou (.exe, .ico) cez `IShellItemImageFactory` (cita subor: len lokalny a plne dostupny);
//! * `Shortcut`: odkaz `.lnk` -> ikona jeho ciela (`IShellLink::GetIconLocation` / `GetPath`, **bez** sipky odkazu a bez
//!   `Resolve`, ktore by mohlo hladat ciel po disku alebo sieti).
//!
//! Cesty sa nelogu; chyba je len `None` (volajuci pouzije vstavanu zastupnu ikonu).
use std::ffi::c_void;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDIBits, GetObjectW,
    SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::System::Com::{
    CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, IShellLinkW, SHCreateItemFromParsingName,
    SHDefExtractIconW, SHGetFileInfoW, SHGetStockIconInfo, ShellLink, SHFILEINFOW, SHGFI_ICON,
    SHGFI_ICONLOCATION, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGSI_ICONLOCATION,
    SHSTOCKICONINFO, SIID_FOLDER, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL, HICON};

/// Najvacsia velkost ikony (px), ktoru toto API vyrobi.
pub const MAX_PX: u32 = 256;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IconSpec {
    /// Pripona vratane bodky (`.txt`); bez pripony sa pouzije vseobecny subor.
    Ext(String),
    Folder,
    /// `shell:AppsFolder\<id>`
    Apps(String),
    /// Subor s vlastnou ikonou (.exe, .ico); pri zlyhani sa pouzije ikona typu.
    Item(String),
    /// Odkaz `.lnk`.
    Shortcut(String),
}

/// Obrazok `w x h` (RGBA, nepremultiplikovana alfa, riadky zhora nadol).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<[u8; 4]>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Rozsiri `%PREMENNE%` (cesty z registra a odkazov ich casto obsahuju); neznama premenna ostane v texte.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        match tail.find('%') {
            Some(j) if j > 0 => match std::env::var(&tail[..j]) {
                Ok(v) => {
                    out.push_str(&v);
                    rest = &tail[j + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = tail;
                }
            },
            _ => {
                out.push('%');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Pripona vratane bodky z cesty ("C:\a\b.TXT" -> ".txt"); prazdne, ak ziadnu nema.
pub fn ext_of(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 16 => {
            format!(".{}", ext.to_ascii_lowercase())
        }
        _ => String::new(),
    }
}

/// Pixely zo 32-bitoveho `HBITMAP` (z `IShellItemImageFactory`). Format alfy dokumentacia neuvadza, preto sa rozpoznava:
/// samé nuly v alfe = nepriehladna ikona bez alfy; inak sa predpoklada premultiplikovana alfa, ak ziadny kanal nepresahuje
/// alfu, a nepremultiplikovana, ak ano.
fn bitmap_pixels(hbmp: HBITMAP) -> Option<Pixels> {
    // SAFETY: `hbmp` je platny a nie je vybrany do ziadneho DC; buffer ma presnu velkost pre `GetDIBits` (top-down, 32 bpp).
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(
            hbmp.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut c_void),
        ) == 0
        {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 || w as u32 > MAX_PX * 2 || h as u32 > MAX_PX * 2 {
            return None;
        }
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let mut bmi = BITMAPINFO {
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
        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        let got = GetDIBits(
            dc,
            hbmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        if got == 0 {
            return None;
        }
        Some(Pixels {
            w: w as u32,
            h: h as u32,
            rgba: bgra_to_rgba(&buf),
        })
    }
}

/// BGRA bajty (alfa podla popisu vyssie) -> nepremultiplikovane RGBA; cisto prazdny obrazok = `None`-ekvivalent (prazdny vektor).
pub fn bgra_to_rgba(buf: &[u8]) -> Vec<[u8; 4]> {
    let px = buf.as_chunks::<4>().0.iter();
    let any_alpha = px.clone().any(|p| p[3] != 0);
    let premult = px
        .clone()
        .all(|p| p[3] == 0 || (p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    px.map(|p| {
        if !any_alpha {
            [p[2], p[1], p[0], 255]
        } else if premult && p[3] != 255 && p[3] != 0 {
            let a = u32::from(p[3]);
            let un = |c: u8| ((u32::from(c) * 255 + a / 2) / a).min(255) as u8;
            [un(p[2]), un(p[1]), un(p[0]), p[3]]
        } else if p[3] == 0 {
            [0, 0, 0, 0]
        } else {
            [p[2], p[1], p[0], p[3]]
        }
    })
    .collect()
}

/// `HICON` -> pixely: ikona sa nakresli na cierne aj biele pozadie a alfa sa odvodi z rozdielu (funguje pre kazdu ikonu,
/// aj stare bez alfa kanala, a nezavisi od formatu bitmapy).
fn hicon_pixels(icon: HICON, px: u32) -> Option<Pixels> {
    let on_black = draw_icon(icon, px, 0x0000_0000)?;
    let on_white = draw_icon(icon, px, 0x00FF_FFFF)?;
    let rgba = on_black
        .iter()
        .zip(&on_white)
        .map(|(b, w)| {
            let ch = |s: u32| (((b >> s) & 0xFF) as i32, ((w >> s) & 0xFF) as i32);
            let ((br, wr), (bg, wg), (bb, wb)) = (ch(16), ch(8), ch(0));
            let diff = ((wr - br) + (wg - bg) + (wb - bb)) / 3;
            let a = (255 - diff).clamp(0, 255);
            if a == 0 {
                [0, 0, 0, 0]
            } else {
                let un = |c: i32| ((c * 255 + a / 2) / a).clamp(0, 255) as u8;
                [un(br), un(bg), un(bb), a as u8]
            }
        })
        .collect();
    Some(Pixels { w: px, h: px, rgba })
}

fn draw_icon(icon: HICON, px: u32, bg: u32) -> Option<Vec<u32>> {
    // SAFETY: DIB sekcia `px x px` vybrana do pamatoveho DC; vsetky GDI objekty sa uvolnia pred navratom.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: px as i32,
                biHeight: -(px as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        if bits.is_null() {
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bmp.into());
        let buf = std::slice::from_raw_parts_mut(bits as *mut u32, (px * px) as usize);
        buf.fill(bg);
        let ok = DrawIconEx(dc, 0, 0, icon, px as i32, px as i32, 0, None, DI_NORMAL).is_ok();
        let out = ok.then(|| buf.to_vec());
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        out
    }
}

/// Ikona zo suboru ikon / DLL (`SHDefExtractIconW`) v presnej velkosti `px`.
fn extract(file: &str, index: i32, px: u32) -> Option<Pixels> {
    let w = wide(&expand_env(file));
    let mut large = HICON::default();
    // SAFETY: platny retazec ukonceny nulou; velka ikona v LOWORD `nIconSize`; `HICON` sa uvolni `DestroyIcon`.
    unsafe {
        let hr = SHDefExtractIconW(
            PCWSTR(w.as_ptr()),
            index,
            0,
            Some(&mut large),
            None,
            px | (px << 16),
        );
        if hr.is_err() || large.is_invalid() {
            return None;
        }
        let out = hicon_pixels(large, px);
        let _ = DestroyIcon(large);
        out
    }
}

fn from_item(parsing: &str, px: u32) -> Option<Pixels> {
    let w = wide(parsing);
    // SAFETY: platny retazec ukonceny nulou; `HBITMAP` sa uvolni `DeleteObject`.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(
            PCWSTR(w.as_ptr()),
            None::<&windows::Win32::System::Com::IBindCtx>,
        )
        .ok()?;
        let f: IShellItemImageFactory = item.cast().ok()?;
        let hbmp = f
            .GetImage(
                SIZE {
                    cx: px as i32,
                    cy: px as i32,
                },
                SIIGBF_ICONONLY,
            )
            .ok()?;
        let out = bitmap_pixels(hbmp);
        let _ = DeleteObject(hbmp.into());
        out
    }
}

fn ext_icon(ext: &str, px: u32) -> Option<Pixels> {
    // pripona sa nahradi menom `x<pripona>`; k disku sa pristupuje len vo forme atributov (USEFILEATTRIBUTES)
    let name = wide(&format!("x{ext}"));
    let mut sfi = SHFILEINFOW::default();
    // SAFETY: platny retazec a struktura; `SHGFI_USEFILEATTRIBUTES` = subor sa neotvara.
    unsafe {
        let r = SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_USEFILEATTRIBUTES | SHGFI_ICONLOCATION,
        );
        if r != 0 {
            let loc = from_wide(&sfi.szDisplayName);
            if !loc.is_empty() {
                if let Some(p) = extract(&loc, sfi.iIcon, px) {
                    return Some(p);
                }
            }
        }
        // zalozny postup: systemova ikona typu v systemovej velkosti
        let mut sfi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_USEFILEATTRIBUTES | SHGFI_ICON | SHGFI_LARGEICON,
        );
        if r == 0 || sfi.hIcon.is_invalid() {
            return None;
        }
        let out = hicon_pixels(sfi.hIcon, px);
        let _ = DestroyIcon(sfi.hIcon);
        out
    }
}

fn folder_icon(px: u32) -> Option<Pixels> {
    let mut sii = SHSTOCKICONINFO {
        cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: platna struktura s nastavenym `cbSize`.
    unsafe {
        SHGetStockIconInfo(SIID_FOLDER, SHGSI_ICONLOCATION, &mut sii).ok()?;
    }
    extract(&from_wide(&sii.szPath), sii.iIcon, px)
}

fn shortcut_icon(lnk: &str, px: u32) -> Option<Pixels> {
    let w = wide(lnk);
    // SAFETY: COM objekty zijú len v tomto volani; buffery maju pevnu velkost.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let mut path = [0u16; 520];
        let mut idx = 0i32;
        if link.GetIconLocation(&mut path, &mut idx).is_ok() {
            let p = from_wide(&path);
            if !p.is_empty() {
                if let Some(i) = extract(&p, idx, px) {
                    return Some(i);
                }
            }
        }
        // bez vlastnej ikony odkazu: ikona ciela (ulozena cesta, bez `Resolve`)
        let mut target = [0u16; 520];
        if link
            .GetPath(
                &mut target,
                std::ptr::null_mut(),
                windows::Win32::UI::Shell::SLGP_RAWPATH.0 as u32,
            )
            .is_ok()
        {
            let t = expand_env(&from_wide(&target));
            if !t.is_empty() {
                return from_item(&t, px);
            }
        }
        None
    }
}

/// Nacita ikonu `spec` v presnej velkosti `px` (1..=`MAX_PX`). Volajuce vlakno ma COM. `None` = nepodarilo sa (zastupna ikona).
pub fn load(spec: &IconSpec, px: u32) -> Option<Pixels> {
    if px == 0 || px > MAX_PX {
        return None;
    }
    // shell pri prvom pouziti inicializuje spolocne cache ikon a pri sucasnych volaniach z viacerych vlakien obcas vrati chybu
    // (namerane: `SHGetFileInfoW` pre `.txt` zlyhal v paralelnych testoch); volania sa preto radia za sebou. Sietove cesty sem
    // nechodia (volajuci pre ne pouzije `Ext`), takze jedno volanie netrva dlho.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    match spec {
        IconSpec::Ext(e) => ext_icon(e, px),
        IconSpec::Folder => folder_icon(px),
        IconSpec::Apps(p) => from_item(p, px),
        IconSpec::Item(p) => from_item(p, px).or_else(|| ext_icon(&ext_of(p), px)),
        IconSpec::Shortcut(p) => shortcut_icon(p, px)
            .or_else(|| from_item(p, px))
            .or_else(|| ext_icon(".lnk", px)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS};

    fn com() {
        // SAFETY: inicializacia COM (STA) pre testovacie vlakno.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "transom-shellicon-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Podiel pixelov s viditelnou alfou.
    fn ink(p: &Pixels) -> f32 {
        p.rgba.iter().filter(|c| c[3] > 32).count() as f32 / p.rgba.len() as f32
    }

    fn good(p: Option<Pixels>, px: u32, what: &str) -> Pixels {
        let p = p.unwrap_or_else(|| panic!("{what}: ikona sa nenacitala"));
        assert_eq!((p.w, p.h), (px, px), "{what}: velkost");
        assert_eq!(p.rgba.len(), (px * px) as usize);
        let k = ink(&p);
        assert!((0.08..=1.0).contains(&k), "{what}: viditelny obsah {k}");
        p
    }

    #[test]
    fn environment_variables_and_extensions_are_parsed_safely() {
        std::env::set_var("TRANSOM_ICON_TEST", r"C:\X");
        assert_eq!(expand_env(r"%TRANSOM_ICON_TEST%\a.dll"), r"C:\X\a.dll");
        assert_eq!(
            expand_env("%NOT_SET_TRANSOM_ICON%\\a"),
            "%NOT_SET_TRANSOM_ICON%\\a"
        );
        assert_eq!(expand_env("100%"), "100%");
        assert_eq!(expand_env("%%"), "%%");
        assert_eq!(expand_env(""), "");
        assert_eq!(ext_of(r"C:\a\Report.DOCX"), ".docx");
        assert_eq!(ext_of(r"C:\a\noext"), "");
        assert_eq!(ext_of(r"C:\a\.hidden"), "");
        assert_eq!(ext_of("archive.tar.gz"), ".gz");
        assert_eq!(ext_of(r"C:\a.b\file"), "");
    }

    #[test]
    fn bitmap_alpha_is_recognised_as_premultiplied_straight_or_missing() {
        // bez alfy (samé nuly): nepriehladne
        assert_eq!(
            bgra_to_rgba(&[10, 20, 30, 0, 1, 2, 3, 0]),
            vec![[30, 20, 10, 255], [3, 2, 1, 255]]
        );
        // premultiplikovana: 50 % cervena (255 * 128 / 255 = 128) -> plna cervena s alfou 128
        let v = bgra_to_rgba(&[0, 0, 128, 128, 0, 0, 0, 0]);
        assert_eq!(v[0], [255, 0, 0, 128]);
        assert_eq!(v[1], [0, 0, 0, 0]);
        // nepremultiplikovana (kanal nad alfou): ostane
        assert_eq!(
            bgra_to_rgba(&[0, 0, 255, 128, 0, 0, 0, 255])[0],
            [255, 0, 0, 128]
        );
        // plne kryte pixely sa nemenia
        assert_eq!(
            bgra_to_rgba(&[1, 2, 3, 255, 0, 0, 128, 128])[0],
            [3, 2, 1, 255]
        );
    }

    #[test]
    fn file_type_folder_executable_app_and_unknown_icons_load_crisply_at_every_dpi_size() {
        com();
        for px in [16u32, 20, 24, 32, 48] {
            good(load(&IconSpec::Ext(".txt".into()), px), px, ".txt");
            good(load(&IconSpec::Ext(".png".into()), px), px, ".png");
            good(
                load(&IconSpec::Ext(".zzzunknown".into()), px),
                px,
                "neznama pripona",
            );
            good(load(&IconSpec::Ext(String::new()), px), px, "bez pripony");
            good(load(&IconSpec::Folder, px), px, "priecinok");
            let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            good(
                load(&IconSpec::Item(format!(r"{sys}\System32\notepad.exe")), px),
                px,
                "exe",
            );
        }
        assert!(load(&IconSpec::Ext(".txt".into()), 0).is_none());
        assert!(load(&IconSpec::Ext(".txt".into()), MAX_PX + 1).is_none());
        // aplikacie zo zlozky Aplikacie (nazvy sa nevypisuju)
        let apps = crate::shellops::list_apps();
        assert!(!apps.is_empty(), "zlozka Aplikacie nie je prazdna");
        let loaded = apps
            .iter()
            .take(12)
            .filter(|(_, t)| load(&IconSpec::Apps(t.clone()), 32).is_some_and(|p| ink(&p) > 0.05))
            .count();
        assert!(
            loaded >= 6,
            "ikony aplikacii sa nacitali: {loaded} z {}",
            apps.len().min(12)
        );
    }

    #[test]
    fn missing_inaccessible_corrupt_and_special_files_fall_back_to_the_type_icon_without_touching_the_disk(
    ) {
        com();
        let d = scratch("fallback");
        // chybajuce subory: ikona podla pripony (aj exe a priecinok, ktory neexistuje)
        for p in [
            r"C:\transom-nonexistent\a.docx",
            r"C:\transom-nonexistent\b.exe",
            r"\\transom-nonexistent-server\share\c.pdf",
        ] {
            good(load(&IconSpec::Item(p.into()), 32), 32, "chybajuci");
        }
        good(
            load(
                &IconSpec::Shortcut(r"C:\transom-nonexistent\x.lnk".into()),
                32,
            ),
            32,
            "chybajuci odkaz",
        );
        // poskodeny .exe a zamknuty .png (otvoreny bez zdielania)
        std::fs::write(
            d.join("junk.exe"),
            (0..4000u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>(),
        )
        .unwrap();
        good(
            load(
                &IconSpec::Item(d.join("junk.exe").to_str().unwrap().into()),
                32,
            ),
            32,
            "poskodeny exe",
        );
        std::fs::write(d.join("locked.png"), b"x").unwrap();
        let held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(d.join("locked.png"))
                .unwrap()
        };
        good(
            load(
                &IconSpec::Item(d.join("locked.png").to_str().unwrap().into()),
                32,
            ),
            32,
            "zamknuty subor",
        );
        drop(held);
        // subor bez citatelnych prav: zamietnutie cez zakazane ACL by menilo systemove nastavenia, preto sa simuluje cestou, ktoru
        // shell neotvori (`CON`): vysledok je zastupna ikona, nie zaseknutie ani chyba
        good(
            load(&IconSpec::Item(r"\\.\CON".into()), 32),
            32,
            "zariadenie",
        );
        // hostile vstupy sa neprevedu na nic nebezpecne
        for bad in ["", "\u{7}", "a\0b", &"x".repeat(5000)] {
            let _ = load(&IconSpec::Item(bad.into()), 16);
            let _ = load(&IconSpec::Apps(bad.into()), 16);
            let _ = load(&IconSpec::Shortcut(bad.into()), 16);
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_shortcut_shows_the_icon_of_its_target_without_the_link_arrow() {
        com();
        let d = scratch("lnk");
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let target = format!(r"{sys}\System32\notepad.exe");
        let lnk = d.join("Notepad link.lnk");
        // SAFETY: vytvorenie odkazu cez IShellLinkW + IPersistFile v scratch priecinku.
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.SetPath(PCWSTR(wide(&target).as_ptr())).unwrap();
            let f: IPersistFile = link.cast().unwrap();
            f.Save(PCWSTR(wide(lnk.to_str().unwrap()).as_ptr()), true)
                .unwrap();
        }
        assert!(lnk.exists());
        let from_lnk = good(
            load(&IconSpec::Shortcut(lnk.to_str().unwrap().into()), 48),
            48,
            "odkaz",
        );
        let from_exe = good(load(&IconSpec::Item(target), 48), 48, "ciel");
        // sipka odkazu by bola vlavo dole: stvrtina vlavo dole sa musi zhodovat s ikonou ciela
        let corner = |p: &Pixels| -> Vec<[u8; 4]> {
            (30..48usize)
                .flat_map(|y| (0..18usize).map(move |x| (x, y)))
                .map(|(x, y)| p.rgba[y * 48 + x])
                .collect()
        };
        let (a, b) = (corner(&from_lnk), corner(&from_exe));
        let diff: u64 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| {
                (0..4)
                    .map(|i| (i32::from(x[i]) - i32::from(y[i])).unsigned_abs() as u64)
                    .sum::<u64>()
            })
            .sum();
        let mean = diff as f64 / (a.len() as f64 * 4.0);
        assert!(
            mean < 12.0,
            "vlavo dole nema byt sipka odkazu (rozdiel {mean:.1})"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_shell_item_path_and_the_hicon_path_agree_on_orientation_and_alpha() {
        com();
        let d = scratch("ico");
        let ico = d.join("brand.ico");
        std::fs::write(&ico, crate::icons::APP_ICO).unwrap();
        // ikona znacky je asymetricka (lista dole) a ma priehladne rohy: obe cesty musia dat to iste
        let via_shell = good(
            load(&IconSpec::Item(ico.to_str().unwrap().into()), 32),
            32,
            "ico cez shell",
        );
        let h = crate::icons::hicon(crate::icons::APP_ICO, 32).expect("hicon");
        let via_icon = hicon_pixels(h, 32).expect("pixely");
        crate::icons::destroy(h);
        let rgb_diff = |a: &Pixels, b: &Pixels| -> f64 {
            let n = a.rgba.len() as f64;
            a.rgba
                .iter()
                .zip(&b.rgba)
                .map(|(x, y)| {
                    let (xa, ya) = (f64::from(x[3]) / 255.0, f64::from(y[3]) / 255.0);
                    // porovnanie na bielom podklade (nezavisle od formatu alfy)
                    (0..3)
                        .map(|i| {
                            let cx = f64::from(x[i]) * xa + 255.0 * (1.0 - xa);
                            let cy = f64::from(y[i]) * ya + 255.0 * (1.0 - ya);
                            (cx - cy).abs()
                        })
                        .sum::<f64>()
                        / 3.0
                })
                .sum::<f64>()
                / n
        };
        assert!(
            rgb_diff(&via_shell, &via_icon) < 6.0,
            "{:.2}",
            rgb_diff(&via_shell, &via_icon)
        );
        let flipped = Pixels {
            rgba: via_icon.rgba.chunks(32).rev().flatten().copied().collect(),
            ..via_icon.clone()
        };
        assert!(
            rgb_diff(&via_shell, &flipped) > rgb_diff(&via_shell, &via_icon) + 2.0,
            "test rozlisi prevratene poradie riadkov"
        );
        assert_eq!(via_shell.rgba[0][3], 0, "roh je priehladny");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn repeated_loading_leaks_no_gdi_objects() {
        com();
        let count = || {
            // SAFETY: citanie poctu GDI objektov vlastneho procesu.
            unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
        };
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let specs = [
            IconSpec::Ext(".txt".into()),
            IconSpec::Folder,
            IconSpec::Item(format!(r"{sys}\System32\notepad.exe")),
            IconSpec::Item(r"C:\transom-nonexistent\a.docx".into()),
        ];
        for s in &specs {
            let _ = load(s, 32); // zahriatie (prve volanie moze vytvorit trvale objekty shellu)
        }
        let before = count();
        for _ in 0..80 {
            for s in &specs {
                let _ = load(s, 24);
            }
        }
        let after = count();
        assert!(after <= before + 6, "GDI objekty {before} -> {after}");
    }
}
