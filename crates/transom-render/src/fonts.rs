//! Vyber pisma rozhrania. Preferuje sa **Segoe UI Variable** (systemove pismo Windows 11) s optickymi velkostami podla
//! velkosti textu - Small do 12 lp, Text 13-19 lp, Display od 20 lp - a so **skutocnym polotucnym** rezom (pomenovana
//! instancia "... Semibold"; GDI by inak vahu 600 v rodine Regular/Bold vykreslil ako Bold). Ak Segoe UI Variable
//! naozaj nie je k dispozicii (Windows 10, odstranene pismo), pouzije sa Segoe UI a "Segoe UI Semibold".
//!
//! Dostupnost sa overuje raz za proces: GDI pri chybajucom pisme ticho podstrci ine (Arial), preto sa pismo vytvori,
//! vyberie do pamatoveho DC a `GetTextFaceW` musi vratit pozadovany nazov. Nic sa nebali ani nestahuje.
//!
//! Lista pouziva `Font::Tabular` (klasicke Segoe UI): jeho cislice maju rovnaku sirku, na com stoji prekreslovanie hodin
//! po sekundach a stabilna sirka percent a odpoctov (Segoe UI Variable ma cislice proporcionalne - "1" je uzsia); lista tak
//! vyzera rovnako nepriehladna aj priehladna.
//!
//! Text s japonskymi, cinskymi alebo korejskymi znakmi sa kresli klasickym Segoe UI s povodnou vahou (polotucne = 600 v
//! rodine Segoe UI): prepojenie pisma (`FontLink\SystemLink`) pre pomenovane inštancie Segoe UI Variable dava znakom CJK
//! inu velkost a rozostupy nez latinke vedla nich (overene na snimkach v cinstine a japoncine).
use crate::painter::Font;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

/// Rodina pisma rozhrania.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Segoe UI Variable (Windows 11).
    Variable,
    /// Segoe UI (zaloha).
    Classic,
}

/// Nazvy tak, ako ich hlasi GDI (najviac 31 znakov - dlhsie nazvy pomenovanych instancii GDI skracuje).
pub const VAR_SMALL: &str = "Segoe UI Variable Small";
pub const VAR_SMALL_SEMI: &str = "Segoe UI Variable Small Semibol";
pub const VAR_TEXT: &str = "Segoe UI Variable Text";
pub const VAR_TEXT_SEMI: &str = "Segoe UI Variable Text Semibold";
pub const VAR_DISPLAY: &str = "Segoe UI Variable Display";
pub const VAR_DISPLAY_SEMI: &str = "Segoe UI Variable Display Semib";
pub const CLASSIC: &str = "Segoe UI";
pub const CLASSIC_SEMI: &str = "Segoe UI Semibold";
/// Tenky rez klasickeho Segoe UI (rovnako siroke cislice): text lisaty (hodiny, baterie, percenta).
pub const CLASSIC_LIGHT: &str = "Segoe UI Semilight";
pub const ICONS: &str = "Segoe Fluent Icons";

/// Hranice optickych velkosti v lp (opticka velkost sleduje velkost v bodoch: 12 lp = 9 pt je bliz instancii Small).
pub const SMALL_BELOW_LP: f32 = 12.5;
pub const DISPLAY_FROM_LP: f32 = 20.0;

const FW_REGULAR: i32 = 400;
const FW_SEMI: i32 = 600;

/// 0 = automaticky, 1 = vynutit Variable, 2 = vynutit Classic (kontrola zalohy v `--shot` a testoch).
static FORCE: AtomicU8 = AtomicU8::new(0);
static DETECTED: OnceLock<Family> = OnceLock::new();
static CLASSIC_SEMI_OK: OnceLock<bool> = OnceLock::new();
static CLASSIC_LIGHT_OK: OnceLock<bool> = OnceLock::new();
const FW_SEMILIGHT: i32 = 350;

/// Vynuti rodinu (len kontroly vzhladu a testy); `None` = automaticky podla dostupnosti.
pub fn force(f: Option<Family>) {
    FORCE.store(
        match f {
            None => 0,
            Some(Family::Variable) => 1,
            Some(Family::Classic) => 2,
        },
        Ordering::Relaxed,
    );
}

/// Pouzivana rodina pisma rozhrania.
pub fn family() -> Family {
    match FORCE.load(Ordering::Relaxed) {
        1 => Family::Variable,
        2 => Family::Classic,
        _ => *DETECTED.get_or_init(|| {
            // vsetky pouzivane instancie musia existovat (pri ciastocnej instalacii radsej cela zaloha)
            let all = [
                VAR_SMALL,
                VAR_SMALL_SEMI,
                VAR_TEXT,
                VAR_TEXT_SEMI,
                VAR_DISPLAY,
                VAR_DISPLAY_SEMI,
            ]
            .iter()
            .all(|f| available(f));
            if all {
                Family::Variable
            } else {
                Family::Classic
            }
        }),
    }
}

/// Japonske, cinske a korejske znaky (a ich interpunkcia).
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Nazov pisma a vaha pre konkretny text: text so znakmi CJK ide klasickym Segoe UI s povodnou vahou (prepojenie pisma je
/// ladene pre Segoe UI), ostatny podla `face`.
pub fn face_for_text(
    family: Family,
    font: Font,
    size_lp: f32,
    semibold: bool,
    text: &str,
) -> (&'static str, i32) {
    if font == Font::Ui && text.chars().any(is_cjk) {
        return (CLASSIC, if semibold { FW_SEMI } else { FW_REGULAR });
    }
    face(family, font, size_lp, semibold)
}

/// Nazov pisma a vaha (GDI `lfWeight`) pre styl.
pub fn face(family: Family, font: Font, size_lp: f32, semibold: bool) -> (&'static str, i32) {
    match font {
        Font::Icons => (ICONS, FW_REGULAR),
        // klasicke Segoe UI s rovnako sirokymi cislicami; vaha 600 sa v nom kresli ako Bold (tak ako doteraz)
        Font::Tabular => {
            if semibold {
                (CLASSIC, FW_SEMI)
            } else if *CLASSIC_LIGHT_OK.get_or_init(|| available(CLASSIC_LIGHT)) {
                // tenke moderne cislice v liste
                (CLASSIC_LIGHT, FW_SEMILIGHT)
            } else {
                (CLASSIC, FW_REGULAR)
            }
        }
        Font::Ui => match family {
            Family::Variable => {
                let (reg, semi) = if size_lp < SMALL_BELOW_LP {
                    (VAR_SMALL, VAR_SMALL_SEMI)
                } else if size_lp < DISPLAY_FROM_LP {
                    (VAR_TEXT, VAR_TEXT_SEMI)
                } else {
                    (VAR_DISPLAY, VAR_DISPLAY_SEMI)
                };
                if semibold {
                    (semi, FW_SEMI)
                } else {
                    (reg, FW_REGULAR)
                }
            }
            Family::Classic => {
                if !semibold {
                    (CLASSIC, FW_REGULAR)
                } else if *CLASSIC_SEMI_OK.get_or_init(|| available(CLASSIC_SEMI)) {
                    (CLASSIC_SEMI, FW_SEMI)
                } else {
                    (CLASSIC, FW_SEMI)
                }
            }
        },
    }
}

/// Je pismo s tymto nazvom naozaj nainstalovane? (GDI by inak potichu vybralo nahradne.) Vysledok sa pamata.
pub fn available(name: &'static str) -> bool {
    static CACHE: Mutex<Vec<(&'static str, bool)>> = Mutex::new(Vec::new());
    if let Some(v) = CACHE
        .lock()
        .ok()
        .and_then(|c| c.iter().find(|(n, _)| *n == name).map(|(_, v)| *v))
    {
        return v;
    }
    let v = probe(name);
    if let Ok(mut c) = CACHE.lock() {
        c.push((name, v));
    }
    v
}

/// Porovnanie nazvu, ktory GDI vratilo, s pozadovanym (bez ohladu na velkost; GDI moze vratit plny alebo skrateny nazov).
pub fn same_face(requested: &str, got: &str) -> bool {
    let r: String = requested
        .chars()
        .take(31)
        .collect::<String>()
        .to_lowercase();
    let g = got.to_lowercase();
    !g.is_empty() && (g == r || g.starts_with(&r))
}

#[cfg(windows)]
fn probe(name: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateFontW, DeleteDC, DeleteObject, GetTextFaceW, SelectObject,
        CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DEFAULT_QUALITY, OUT_DEFAULT_PRECIS,
    };
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: docasny pamatovy DC a pismo, oboje sa uvolni; GetTextFaceW pise najviac do dlzky buffra.
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return false;
        }
        let f = CreateFontW(
            -16,
            0,
            0,
            0,
            FW_REGULAR,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            DEFAULT_PITCH.0 as u32,
            PCWSTR(wide.as_ptr()),
        );
        if f.is_invalid() {
            let _ = DeleteDC(dc);
            return false;
        }
        let old = SelectObject(dc, f.into());
        let mut buf = [0u16; 64];
        let n = GetTextFaceW(dc, Some(&mut buf)).max(0) as usize;
        SelectObject(dc, old);
        let _ = DeleteObject(f.into());
        let _ = DeleteDC(dc);
        // n zahrna koncovu nulu
        let got = String::from_utf16_lossy(&buf[..n.saturating_sub(1).min(buf.len())]);
        same_face(name, &got)
    }
}

#[cfg(not(windows))]
fn probe(_: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optical_sizes_follow_the_text_size_and_semibold_uses_the_named_instance() {
        let v = Family::Variable;
        assert_eq!(face(v, Font::Ui, 11.0, false), (VAR_SMALL, 400));
        assert_eq!(face(v, Font::Ui, 10.0, true), (VAR_SMALL_SEMI, 600));
        assert_eq!(face(v, Font::Ui, 12.0, false), (VAR_SMALL, 400));
        assert_eq!(face(v, Font::Ui, 13.0, false), (VAR_TEXT, 400));
        assert_eq!(face(v, Font::Ui, 15.0, true), (VAR_TEXT_SEMI, 600));
        assert_eq!(face(v, Font::Ui, 19.5, true), (VAR_TEXT_SEMI, 600));
        assert_eq!(face(v, Font::Ui, 20.0, false), (VAR_DISPLAY, 400));
        assert_eq!(face(v, Font::Ui, 24.0, true), (VAR_DISPLAY_SEMI, 600));
        // hodiny a ikony sa rodinou nemenia
        for fam in [Family::Variable, Family::Classic] {
            assert_eq!(face(fam, Font::Tabular, 14.0, true), (CLASSIC, 600));
            assert_eq!(face(fam, Font::Icons, 12.0, false).0, ICONS);
        }
        assert_eq!(face(Family::Classic, Font::Ui, 24.0, false), (CLASSIC, 400));
        // GDI skracuje nazvy na 31 znakov: vsetky nazvy sa zmestia
        for f in [
            VAR_SMALL,
            VAR_SMALL_SEMI,
            VAR_TEXT,
            VAR_TEXT_SEMI,
            VAR_DISPLAY,
            VAR_DISPLAY_SEMI,
            CLASSIC_SEMI,
        ] {
            assert!(f.chars().count() <= 31, "{f}");
        }
    }

    #[test]
    fn text_with_cjk_uses_classic_segoe_ui_with_the_old_weight_everything_else_follows_the_family()
    {
        let v = Family::Variable;
        assert_eq!(
            face_for_text(v, Font::Ui, 13.0, true, "连接 Claude Code"),
            (CLASSIC, 600)
        );
        assert_eq!(
            face_for_text(v, Font::Ui, 13.0, false, "クリップボード"),
            (CLASSIC, 400)
        );
        assert_eq!(
            face_for_text(v, Font::Ui, 13.0, true, "Clipboard"),
            (VAR_TEXT_SEMI, 600)
        );
        assert_eq!(
            face_for_text(v, Font::Icons, 12.0, false, "\u{E721}").0,
            ICONS
        );
        assert!(is_cjk('の') && is_cjk('数') && is_cjk('（') && !is_cjk('a') && !is_cjk('é'));
    }

    #[test]
    fn face_names_compare_case_insensitively_and_accept_the_full_name_of_a_truncated_one() {
        assert!(same_face(
            VAR_DISPLAY_SEMI,
            "Segoe UI Variable Display Semibold"
        ));
        assert!(same_face(VAR_TEXT, "segoe ui variable text"));
        assert!(!same_face(VAR_TEXT, "Arial"));
        assert!(!same_face(CLASSIC_SEMI, "Segoe UI"));
        assert!(!same_face(VAR_TEXT, ""));
    }

    /// Skutocna kontrola tohto stroja: chybajuce pismo sa nikdy nehlasi ako dostupne (GDI by podstrcilo Arial) a Segoe UI
    /// (sucast kazdych Windows 10/11) je dostupne.
    #[test]
    fn a_missing_font_is_never_reported_as_available() {
        assert!(!available("transom no such font 0x7f"));
        assert!(available(CLASSIC));
        // vysledok automatickej detekcie je jedna z dvoch rodin a je stabilny
        let a = family();
        assert_eq!(a, family());
    }
}
