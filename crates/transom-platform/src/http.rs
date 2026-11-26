//! Jednoduche stiahnutie suboru cez HTTPS (WinHTTP) pre kalendar ICS. Len `https://` (aj `webcal://`, ktory sa prepise
//! na https), presmerovania podla WinHTTP, systemove nastavenie proxy, obmedzena velkost a cas. Adresa sa nikdy nelogue
//! (sukromny odkaz kalendara je tajny udaj).
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

/// Upravi a overi adresu: `webcal://` -> `https://`, len https, bez medzier a riadiacich znakov.
pub fn normalize_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 2048 || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let skip = if lower.starts_with("webcal://") {
        9
    } else if lower.starts_with("https://") {
        8
    } else {
        return None;
    };
    let rest = &s[skip..];
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!("https://{rest}"))
}

/// Len pocitac (napr. "calendar.google.com") na zobrazenie v nastaveniach (bez tajnej casti odkazu).
pub fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#', ':'])
        .next()
        .unwrap_or("")
        .to_string()
}

/// Rozlozi normalizovanu adresu `https://...` na (pocitac, port, cesta s dotazom). Fragment (`#...`) sa na server neposiela.
/// Pocitac konci prvym `/`, `?` alebo `#` (rovnako ako v [`normalize_url`]), takze `https://h?x` nikdy neposle `h?x` ako
/// meno pocitaca.
fn split_url(url: &str) -> Option<(&str, u16, String)> {
    let rest = url.strip_prefix("https://")?;
    let rest = rest.split('#').next().unwrap_or("");
    let (hostport, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes().get(i) == Some(&b'/') => (&rest[..i], rest[i..].to_string()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => {
            (h, p.parse::<u16>().ok().filter(|&p| p != 0)?)
        }
        Some((_, p)) if p.bytes().all(|c| c.is_ascii_digit()) => return None,
        _ => (hostport, 443u16),
    };
    if host.is_empty() {
        return None;
    }
    Some((host, port, path))
}

/// Celkovy cas jedneho stiahnutia: `timeout_ms` plati pre kazdu operaciu WinHTTP zvlast, takze server posielajuci po bajte by
/// inak drzal vlakno (a dalsie kalendare v poradi) neobmedzene dlho.
fn total_deadline(timeout_ms: i32) -> std::time::Duration {
    std::time::Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0).max(1_000) * 6)
}

struct H(*mut core::ffi::c_void);

impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: handle z WinHttp*; zatvori sa raz.
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// Nastavi ciselnu volbu WinHTTP (u32).
///
/// # Safety
/// `h` je platny handle WinHTTP (relacia alebo poziadavka) pocas volania.
unsafe fn set_u32(h: *mut core::ffi::c_void, option: u32, value: u32) -> bool {
    let bytes = value.to_ne_bytes();
    // SAFETY: podla zmluvy funkcie je `h` platny; buffer s hodnotou zije pocas volania a WinHTTP ho kopiruje
    unsafe { WinHttpSetOption(Some(h as *const _), option, Some(&bytes)) }.is_ok()
}

/// Adresa, z ktorej odpoved naozaj prisla (po presmerovaniach).
///
/// # Safety
/// `req` je platny handle poziadavky WinHTTP.
unsafe fn final_url(req: *mut core::ffi::c_void) -> Option<String> {
    let mut buf = vec![0u16; 4096];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: buffer ma `len` bajtov a zije pocas volania; WinHTTP zapise najviac `len` bajtov a vrati skutocnu dlzku
    unsafe {
        WinHttpQueryOption(
            req,
            WINHTTP_OPTION_URL,
            Some(buf.as_mut_ptr().cast()),
            &mut len,
        )
    }
    .ok()?;
    let n = (len as usize / 2).min(buf.len());
    let s = &buf[..n];
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    Some(String::from_utf16_lossy(&s[..end]))
}

/// Stiahne `url` (https). Chyba je kratky popis bez adresy.
///
/// Len TLS 1.2+, presmerovania najviac 5 a nikdy z https na http (aj konecna adresa sa overi), celkovy cas najviac
/// sestnasobok `timeout_ms` (pozri [`total_deadline`]), velkost najviac `max_bytes`.
pub fn get(url: &str, max_bytes: usize, timeout_ms: i32) -> Result<Vec<u8>, String> {
    let url = normalize_url(url).ok_or("adresa nie je https")?;
    let (host, port, path) = split_url(&url).ok_or("zla adresa")?;
    let deadline = std::time::Instant::now() + total_deadline(timeout_ms);
    // SAFETY: postupnost WinHTTP; vsetky handly sa zatvoria (H::drop, poziadavka pred spojenim a relaciou), buffer citania
    // ma zadanu velkost.
    unsafe {
        let session = H(WinHttpOpen(
            &HSTRING::from("transom"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err("WinHttpOpen".into());
        }
        let _ = WinHttpSetTimeouts(session.0, timeout_ms, timeout_ms, timeout_ms, timeout_ms);
        // len TLS 1.2 a 1.3 (ak system 1.3 nepozna, aspon 1.2)
        if !set_u32(
            session.0,
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
        ) {
            let _ = set_u32(
                session.0,
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
            );
        }
        let _ = set_u32(
            session.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP,
        );
        let _ = set_u32(session.0, WINHTTP_OPTION_MAX_HTTP_AUTOMATIC_REDIRECTS, 5);
        let conn = H(WinHttpConnect(session.0, &HSTRING::from(host), port, 0));
        if conn.0.is_null() {
            return Err("spojenie".into());
        }
        let req = H(WinHttpOpenRequest(
            conn.0,
            &HSTRING::from("GET"),
            &HSTRING::from(path.as_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err("poziadavka".into());
        }
        WinHttpSendRequest(req.0, None, None, 0, 0, 0)
            .map_err(|e| format!("odoslanie: {}", e.code().0))?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut())
            .map_err(|e| format!("odpoved: {}", e.code().0))?;
        // obrana do hlbky: po presmerovaniach musi odpoved stale prist cez https
        if !final_url(req.0).is_some_and(|u| u.to_ascii_lowercase().starts_with("https://")) {
            return Err("presmerovanie mimo https".into());
        }
        let mut status = 0u32;
        let mut len = 4u32;
        let _ = WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        );
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }
        let mut out: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if std::time::Instant::now() > deadline {
                return Err("stahovanie trva prilis dlho".into());
            }
            let mut n = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as _, buf.len() as u32, &mut n)
                .map_err(|e| format!("citanie: {}", e.code().0))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..(n as usize).min(buf.len())]);
            if out.len() > max_bytes {
                return Err("subor je prilis velky".into());
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_and_webcal_addresses_are_accepted_and_the_secret_part_is_never_shown() {
        assert_eq!(
            normalize_url(" webcal://p01-caldav.icloud.com/published/2/abc "),
            Some("https://p01-caldav.icloud.com/published/2/abc".into())
        );
        assert_eq!(
            normalize_url(
                "HTTPS://calendar.google.com/calendar/ical/x%40gmail.com/private-123/basic.ics"
            )
            .as_deref(),
            Some("https://calendar.google.com/calendar/ical/x%40gmail.com/private-123/basic.ics")
        );
        for bad in [
            "http://a.b/c.ics",
            "file:///c:/x.ics",
            "https://",
            "https://user@host/x",
            "https://a b/c",
            "javascript:x",
        ] {
            assert_eq!(normalize_url(bad), None, "{bad}");
        }
        assert_eq!(
            host_of("https://outlook.office365.com/owa/calendar/abc/reachcalendar.ics"),
            "outlook.office365.com"
        );
        assert_eq!(host_of("https://h:8443/x"), "h");
    }

    #[test]
    fn urls_split_into_host_port_and_path_like_normalize_url() {
        let ok = |u: &str| split_url(u).map(|(h, p, path)| (h.to_string(), p, path));
        let t = |h: &str, p: u16, path: &str| Some((h.to_string(), p, path.to_string()));
        assert_eq!(ok("https://h/a/b.ics?x=1"), t("h", 443, "/a/b.ics?x=1"));
        assert_eq!(ok("https://h"), t("h", 443, "/"));
        // dotaz bez lomky: pocitac nie je "h?x"
        assert_eq!(ok("https://h?x=1"), t("h", 443, "/?x=1"));
        // fragment sa neposiela
        assert_eq!(ok("https://h/c.ics#tajne"), t("h", 443, "/c.ics"));
        assert_eq!(ok("https://h#x"), t("h", 443, "/"));
        assert_eq!(ok("https://h:8443/x"), t("h", 8443, "/x"));
        assert_eq!(ok("https://[::1]:8443/x"), t("[::1]", 8443, "/x"));
        for bad in [
            "https://h:/x",
            "https://h:0/x",
            "https://h:65536/x",
            "https://:443/x",
            "https:///x",
            "http://h/x",
        ] {
            assert_eq!(split_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_whole_download_has_a_wall_clock_limit() {
        assert_eq!(total_deadline(20_000).as_secs(), 120);
        assert_eq!(total_deadline(120_000).as_secs(), 720);
        // nezmyselne hodnoty nikdy neznamenaju "bez limitu"
        assert_eq!(total_deadline(-1).as_secs(), 6);
        assert_eq!(total_deadline(0).as_secs(), 6);
    }
}
