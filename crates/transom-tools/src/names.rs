//! Nazvy, cesty a identifikatory pre ulozisko nastrojov: ocistenie zobrazovanych textov, kontrola ciest, bezpecne nazvy suborov,
//! kluce na rozpoznanie duplicit a nahodne id. Nic tu nesahá na disk.
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

/// Riadiace, obojsmerne a nulovej sirky znaky: nikdy sa nezobrazuju ani neukladaju do textov z vonka (nazvy, vlozeny text).
pub fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
        )
}

/// Text na zobrazenie: bez riadiacich, obojsmernych a nulovej sirky znakov, medzery zlucene, orezany na `max` znakov ("…").
pub fn clean_display(raw: &str, max: usize) -> String {
    let s: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !is_unsafe_char(*c))
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > max {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        s
    }
}

/// Posledna zlozka cesty (nazov suboru), bez oddelovacov.
pub fn file_name_of(path: &str) -> String {
    path.trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_string()
}

/// Cast cesty bez posledneho nazvu (priecinok).
pub fn parent_of(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rfind(['\\', '/']) {
        Some(i) => t[..i].to_string(),
        None => String::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathReject {
    Empty,
    TooLong,
    Relative,
    Device,
    ControlChars,
}

/// Cesta, ktoru Polica prijme: absolutna lokalna (`C:\...`) alebo UNC (`\\server\share\...`). Zariadenia (`\\.\`,
/// `\\?\GLOBALROOT`, `CON`...), relativne cesty, riadiace znaky a nezmyselne dlzky sa odmietnu. Nic sa tu neotvara.
pub fn check_path(p: &str) -> Result<(), PathReject> {
    if p.is_empty() {
        return Err(PathReject::Empty);
    }
    if p.encode_utf16().count() > 32_767 {
        return Err(PathReject::TooLong);
    }
    if p.chars().any(|c| c.is_control()) {
        return Err(PathReject::ControlChars);
    }
    let lower = p.to_ascii_lowercase();
    if lower.starts_with(r"\\.\") || lower.starts_with("//./") || lower.starts_with(r"\??\") {
        return Err(PathReject::Device);
    }
    // pomenovane rury a postove schranky cez UNC (`\\localhost\pipe\x`, aj `\\?\UNC\...` a s lomitkami) su zariadenia, nie subory
    {
        let norm = lower.replace('/', "\\");
        let rest = norm.strip_prefix(r"\\?\unc\").or_else(|| {
            norm.strip_prefix(r"\\")
                .filter(|r| !r.starts_with('?') && !r.starts_with('.'))
        });
        if let Some(rest) = rest {
            let mut parts = rest.split('\\');
            let host = parts.next().unwrap_or("");
            let share = parts.next().unwrap_or("");
            // `\\`, `\\server` a `\\server\` nie su cesty k nicomu na zdielanom disku
            if host.is_empty() || share.is_empty() {
                return Err(PathReject::Relative);
            }
            if matches!(share, "pipe" | "mailslot") {
                return Err(PathReject::Device);
            }
        }
    }
    if let Some(rest) = lower.strip_prefix(r"\\?\") {
        // len `\\?\C:\...` a `\\?\UNC\...` su obycajne cesty; ostatne (GLOBALROOT, Volume{...}, zariadenia) nie
        let drive = rest.len() >= 3 && rest.as_bytes()[1] == b':' && rest.as_bytes()[2] == b'\\';
        if !(drive || rest.starts_with(r"unc\")) {
            return Err(PathReject::Device);
        }
    }
    let b = p.as_bytes();
    let drive = b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/');
    let unc = p.starts_with(r"\\") && !p.starts_with(r"\\.") && !p.starts_with(r"\\?");
    if !(drive || unc || lower.starts_with(r"\\?\")) {
        return Err(PathReject::Relative);
    }
    // rezervovane nazvy zariadeni ako posledna zlozka (aj s priponou)
    let last = file_name_of(p);
    if is_reserved_stem(last.split('.').next().unwrap_or("")) {
        return Err(PathReject::Device);
    }
    Ok(())
}

fn is_reserved_stem(stem: &str) -> bool {
    let s = stem.trim_end_matches([' ', '.']).to_ascii_uppercase();
    if matches!(
        s.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    // COM1-9 a LPT1-9, ako aj COM/LPT s hornym indexom 1-3 (U+00B9, U+00B2, U+00B3), ktore Windows tiez rezervuje
    let Some(rest) = s.strip_prefix("COM").or_else(|| s.strip_prefix("LPT")) else {
        return false;
    };
    let mut it = rest.chars();
    matches!(
        (it.next(), it.next()),
        (Some('1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
    )
}

/// Kluc na rozpoznanie duplicit: bez `\\?\`, lomitka na spatne, bez koncoveho oddelovaca, male pismena (Windows).
pub fn path_key(p: &str) -> String {
    let t = p.strip_prefix(r"\\?\").unwrap_or(p);
    let t = if let Some(r) = t.strip_prefix(r"UNC\").or_else(|| t.strip_prefix(r"unc\")) {
        format!(r"\\{r}")
    } else {
        t.to_string()
    };
    t.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Invalid,
    Reserved,
    TooLong,
}

/// Bezpecny nazov suboru (bez pripony): zakazane znaky, riadiace/obojsmerne, koncove bodky a medzery, rezervovane nazvy a
/// dlzka. Vracia ocisteny nazov alebo dovod odmietnutia; nikdy nenechá `..`, oddelovace ani `:` (alternativne prudy).
pub fn sanitize_file_stem(raw: &str) -> Result<String, NameError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(NameError::Empty);
    }
    if s.chars().any(|c| {
        is_unsafe_char(c) || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(NameError::Invalid);
    }
    if s.ends_with('.') || s.ends_with(' ') || s.starts_with('.') && s.trim_matches('.').is_empty()
    {
        return Err(NameError::Invalid);
    }
    if is_reserved_stem(s.split('.').next().unwrap_or("")) {
        return Err(NameError::Reserved);
    }
    if s.encode_utf16().count() > 100 {
        return Err(NameError::TooLong);
    }
    Ok(s.to_string())
}

/// Nazov pre ulozenu kopiu: z nazvu povodneho suboru, vzdy platny (nahradi zakazane znaky, vyplni prazdny nazov).
pub fn stored_file_name(original: &str) -> String {
    let name = file_name_of(original);
    let mut out: String = name
        .chars()
        .map(|c| {
            if is_unsafe_char(c)
                || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    out = out.trim_end_matches(['.', ' ']).to_string();
    if out.is_empty() || out.trim_matches('.').is_empty() {
        out = "item".into();
    }
    if is_reserved_stem(out.split('.').next().unwrap_or("")) {
        out.insert(0, '_');
    }
    if out.encode_utf16().count() > 120 {
        let ext = out
            .rsplit_once('.')
            .map(|(_, e)| e.to_string())
            .filter(|e| e.len() <= 10);
        // skracuje sa podla jednotiek UTF-16 (nie znakov): 100 znakov mimo BMP by bolo 200 jednotiek
        let mut keep = String::new();
        let mut units = 0;
        for c in out.chars() {
            units += c.len_utf16();
            if units > 100 {
                break;
            }
            keep.push(c);
        }
        out = match ext {
            Some(e) => format!("{keep}.{e}"),
            None => {
                // po skrateni nesmie nazov koncit medzerou ani bodkou: inak by `stored_file_name(stored) != stored`, nacitanie
                // indexu by polozku odmietlo a osirotena kopia by sa pri dalsom otvoreni zmazala
                let t = keep.trim_end_matches(['.', ' ']);
                if t.is_empty() {
                    "item".to_string()
                } else {
                    t.to_string()
                }
            }
        };
    }
    out
}

/// Nahodne id (16 hex znakov): `RandomState` (nahodne kluce zo systemu) + cas + pocitadlo. Nie je to kryptograficke id;
/// sluzi len na nazvy poloziek a suborov v ulozisku (nikdy sa nepouziva ako tajomstvo).
pub fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    let mut h = RandomState::new().build_hasher();
    N.fetch_add(1, Ordering::Relaxed).hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut h);
    std::process::id().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Je to id v tvare, ktory vyrabame (16 malych hex znakov)? Podcesty ulozista sa skladaju len z takych id.
pub fn is_id(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Odtlacok obsahu na rozpoznanie duplicit (SipHash s pevnymi klucmi + dlzka); nie je to bezpecnostny hash.
pub fn content_hash(bytes: &[u8]) -> String {
    #[allow(deprecated)]
    let mut a = std::hash::SipHasher::new_with_keys(0x9e37_79b9_7f4a_7c15, 0x1234_5678_9abc_def0);
    bytes.hash(&mut a);
    let x = a.finish();
    #[allow(deprecated)]
    let mut b = std::hash::SipHasher::new_with_keys(0xdead_beef_cafe_f00d, 0x0fed_cba9_8765_4321);
    bytes.len().hash(&mut b);
    bytes.hash(&mut b);
    format!("{:016x}{:016x}", x, b.finish())
}

/// Civilny datum a cas (UTC) z unixovych sekund: (rok, mesiac, den, hodina, minuta, sekunda).
pub fn civil(ts: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // algoritmus Howarda Hinnanta (civil_from_days)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (secs / 3600) as u32,
        (secs % 3600 / 60) as u32,
        (secs % 60) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_text_loses_control_bidi_and_zero_width_characters_and_is_capped() {
        assert_eq!(clean_display("  a\u{202E}b\u{200B}c\0 \n d ", 50), "abc d");
        let long = "x".repeat(500);
        let c = clean_display(&long, 40);
        assert_eq!(c.chars().count(), 40);
        assert!(c.ends_with('…'));
    }

    #[test]
    fn paths_are_checked_without_touching_the_disk() {
        for ok in [
            r"C:\a\b.txt",
            r"c:/a/b.txt",
            r"\\srv\share\x.pdf",
            r"\\?\C:\long\x.txt",
            r"\\?\UNC\srv\share\x",
        ] {
            assert_eq!(check_path(ok), Ok(()), "{ok}");
        }
        for (bad, why) in [
            ("", PathReject::Empty),
            (r"rel\x.txt", PathReject::Relative),
            ("x.txt", PathReject::Relative),
            (r"\\.\PhysicalDrive0", PathReject::Device),
            (r"\\?\GLOBALROOT\Device\X", PathReject::Device),
            (r"\\?\Volume{1234}\x", PathReject::Device),
            (r"C:\dir\CON", PathReject::Device),
            (r"C:\dir\nul.txt", PathReject::Device),
            (r"C:\dir\COM1", PathReject::Device),
            ("C:\\a\u{0}b", PathReject::ControlChars),
        ] {
            assert_eq!(check_path(bad), Err(why), "{bad:?}");
        }
        assert_eq!(
            check_path(&format!(r"C:\{}", "a".repeat(33_000))),
            Err(PathReject::TooLong)
        );
        // COM0 a LPT10 nie su rezervovane
        assert_eq!(check_path(r"C:\x\COM0"), Ok(()));
    }

    #[test]
    fn bare_unc_roots_and_unicode_device_variants_are_refused() {
        // chyba host alebo zdielanie: nie je to cesta k suboru ani k priecinku na zdielanom disku
        for bad in [
            r"\\",
            r"\\\",
            r"\\srv",
            r"\\srv\",
            "//srv",
            r"\\?\UNC\",
            r"\\?\UNC\srv",
            r"\\?\UNC\srv\",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?} sa nema prijat");
        }
        // koren zdielania (`\\srv\share`) a hlbsie cesty sa prijimaju
        for ok in [r"\\srv\share", r"\\srv\share\a", r"\\?\UNC\srv\share"] {
            assert_eq!(check_path(ok), Ok(()), "{ok}");
        }
        // Windows rezervuje aj COM a LPT s horny indexom 1-3
        for bad in [
            "C:\\dir\\COM\u{b9}",
            "C:\\dir\\com\u{b2}.txt",
            "C:\\dir\\LPT\u{b3}",
        ] {
            assert_eq!(check_path(bad), Err(PathReject::Device), "{bad:?}");
        }
        assert_eq!(stored_file_name("C:\\x\\COM\u{b9}.txt"), "_COM\u{b9}.txt");
    }

    #[test]
    fn stored_names_are_valid_and_stable_so_a_copy_survives_a_restart() {
        // dlhy nazov bez pripony, na 100. znaku medzera: po skrateni nesmie koncit medzerou ani bodkou, inak ho nacitanie
        // indexu odmietne (`stored_file_name(stored) != stored`) a osirotena kopia sa zmaze
        let cases = [
            format!("C:\\x\\{} {}", "a".repeat(99), "b".repeat(30)),
            format!("C:\\x\\{}.{}", "a".repeat(99), "b".repeat(30)),
            format!("C:\\x\\{}. {}", "a".repeat(98), "b".repeat(30)),
            format!("C:\\x\\{} .txt", "a".repeat(150)),
            format!("C:\\x\\{}", "\u{1F600}".repeat(130)),
            format!("C:\\x\\{}", " ".repeat(200)),
            format!("C:\\x\\{}", ".".repeat(200)),
            "C:\\x\\a .".to_string(),
        ];
        for c in cases {
            let s = stored_file_name(&c);
            assert_eq!(stored_file_name(&s), s, "nestabilny nazov {s:?}");
            assert!(!s.is_empty() && !s.ends_with([' ', '.']), "{s:?}");
            assert!(s.encode_utf16().count() <= 120, "{s:?}");
        }
    }

    #[test]
    fn duplicate_keys_ignore_case_separators_long_path_prefix_and_trailing_slash() {
        let k = path_key(r"C:\Users\A\File.TXT");
        assert_eq!(path_key(r"\\?\c:\users\a\file.txt"), k);
        assert_eq!(path_key("c:/Users/A/File.txt"), k);
        assert_eq!(path_key(r"C:\Users\A\Dir\"), path_key(r"C:\Users\A\Dir"));
        assert_ne!(path_key(r"C:\a\b"), path_key(r"C:\a\c"));
        assert_eq!(path_key(r"\\?\UNC\srv\share\x"), path_key(r"\\srv\share\x"));
    }

    #[test]
    fn file_names_are_sanitized_for_rename_and_for_stored_copies() {
        assert_eq!(sanitize_file_stem("My shot"), Ok("My shot".into()));
        for (bad, why) in [
            ("", NameError::Empty),
            ("  ", NameError::Empty),
            ("a/b", NameError::Invalid),
            ("a\\b", NameError::Invalid),
            ("a:b", NameError::Invalid),
            ("..", NameError::Invalid),
            ("name.", NameError::Invalid),
            ("a\u{202E}b", NameError::Invalid),
            ("a\tb", NameError::Invalid),
            ("CON", NameError::Reserved),
            ("nul.txt", NameError::Reserved),
            ("Lpt3", NameError::Reserved),
            (&"x".repeat(101), NameError::TooLong),
        ] {
            assert_eq!(sanitize_file_stem(bad), Err(why), "{bad:?}");
        }
        assert_eq!(stored_file_name(r"C:\x\a:b*.txt"), "a_b_.txt");
        assert_eq!(stored_file_name(r"C:\x\CON"), "_CON");
        assert_eq!(stored_file_name(r"C:\x\.."), "item");
        assert!(stored_file_name(&format!(r"C:\x\{}.pdf", "n".repeat(300))).ends_with(".pdf"));
        assert!(
            stored_file_name(&format!(r"C:\x\{}.pdf", "n".repeat(300)))
                .encode_utf16()
                .count()
                <= 120
        );
    }

    #[test]
    fn ids_are_well_formed_unique_and_checked_before_use_in_paths() {
        let a = new_id();
        let b = new_id();
        assert!(is_id(&a) && is_id(&b) && a != b);
        for bad in [
            "",
            "..",
            "../a",
            "ABCDEF0123456789",
            "abc",
            "0123456789abcdeg",
            "0123456789abcdef0",
        ] {
            assert!(!is_id(bad), "{bad}");
        }
    }

    #[test]
    fn content_hash_is_stable_and_differs_for_different_content() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
        assert_eq!(content_hash(b"").len(), 32);
    }

    #[test]
    fn civil_dates_match_known_timestamps() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_000_000_000), (2001, 9, 9, 1, 46, 40));
        assert_eq!(civil(1_795_262_400), (2026, 11, 21, 12, 0, 0));
        assert_eq!(civil(-1), (1969, 12, 31, 23, 59, 59));
    }

    #[test]
    fn paths_split_into_name_and_parent() {
        assert_eq!(file_name_of(r"C:\a\b\c.txt"), "c.txt");
        assert_eq!(file_name_of(r"C:\a\b\"), "b");
        assert_eq!(parent_of(r"C:\a\b\c.txt"), r"C:\a\b");
        assert_eq!(parent_of("c.txt"), "");
    }
}
