//! Lokalny cas: formatovanie podla uzivatelskeho nastavenia Windows (12/24 h) a vypocet casu do dalsej minuty.
use transom_core::clock::{compose, time_picture, ClockOptions, DateMode, Ymd};
use windows::core::PCWSTR;
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Globalization::{
    GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx, DATE_LONGDATE, DATE_SHORTDATE,
    DATE_YEARMONTH, ENUM_DATE_FORMATS_FLAGS, LOCALE_IFIRSTDAYOFWEEK, LOCALE_SSHORTESTDAYNAME1,
    LOCALE_STIMEFORMAT, TIME_FORMAT_FLAGS, TIME_NOSECONDS,
};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// Milisekundy do dalsej hranice minuty (vzdy 1..=60000). Pri presne zaciatku minuty vrati 60000.
pub fn ms_until_next_minute(second: u16, millis: u16) -> u32 {
    let into_minute = u32::from(second.min(59)) * 1000 + u32::from(millis.min(999));
    60_000 - into_minute
}

/// Cas pre casovac: do hranice minuty + kratka rezerva, aby WM_TIMER nepripadol tesne pred hranicu
/// (potom by sa zobrazil este stary cas a lista by zaostala o minutu).
pub fn wake_delay_ms(second: u16, millis: u16) -> u32 {
    ms_until_next_minute(second, millis) + 25
}

fn format_time(st: &SYSTEMTIME) -> String {
    let mut buf = [0u16; 32];
    // lplocalename = null -> LOCALE_NAME_USER_DEFAULT; format = null -> predvoleny format locale
    // SAFETY: buffer s danou dlzkou; `st` je platna struktura; null locale = predvolene.
    let n = unsafe {
        GetTimeFormatEx(
            PCWSTR::null(),
            TIME_NOSECONDS,
            Some(st),
            PCWSTR::null(),
            Some(&mut buf),
        )
    };
    if n > 1 {
        String::from_utf16_lossy(&buf[..(n as usize - 1)])
    } else {
        // zalozny format, ak API zlyha
        format!("{:02}:{:02}", st.wHour, st.wMinute)
    }
}

/// Retazec z `GetLocaleInfoEx` pre jazyk pouzivatela (`LOCALE_NAME_USER_DEFAULT`).
fn locale_info(lctype: u32) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: buffer s danou dlzkou; vratena dlzka obsahuje nulu (overene `n > 1`).
    let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), lctype, Some(&mut buf)) };
    if n > 1 {
        String::from_utf16_lossy(&buf[..(n as usize - 1)])
    } else {
        String::new()
    }
}

/// Format casu jazykoveho nastavenia (`LOCALE_STIMEFORMAT`, napr. `H:mm:ss`).
pub fn locale_time_format() -> String {
    locale_info(LOCALE_STIMEFORMAT)
}

/// Prvy den tyzdna jazykoveho nastavenia: 0 = pondelok ... 6 = nedela (`LOCALE_IFIRSTDAYOFWEEK`, overene v
/// dokumentacii Microsoft Learn 2026-11-20). Pri chybe pondelok.
pub fn locale_first_day() -> u8 {
    locale_info(LOCALE_IFIRSTDAYOFWEEK)
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|d| *d <= 6)
        .unwrap_or(0)
}

/// Najkratsie nazvy dni podla jazykoveho nastavenia, pondelok..nedela (`LOCALE_SSHORTESTDAYNAME1..7`).
pub fn short_day_names() -> [String; 7] {
    std::array::from_fn(|i| locale_info(LOCALE_SSHORTESTDAYNAME1 + i as u32))
}

fn systime(d: Ymd) -> SYSTEMTIME {
    SYSTEMTIME {
        wYear: d.y as u16,
        wMonth: d.m as u16,
        wDay: d.d as u16,
        ..Default::default()
    }
}

fn date_fmt(st: &SYSTEMTIME, flags: ENUM_DATE_FORMATS_FLAGS, picture: Option<&str>) -> String {
    date_fmt_in(None, st, flags, picture)
}

/// Datum s nazvami dni a mesiacov v jazyku transom (nie Windows); ak jazyk Windows nepozna, podla Windows.
fn date_fmt_lang(st: &SYSTEMTIME, flags: ENUM_DATE_FORMATS_FLAGS) -> String {
    let code = transom_i18n::lang().code();
    let t = date_fmt_in(Some(code), st, flags, None);
    if t.is_empty() {
        date_fmt(st, flags, None)
    } else {
        t
    }
}

fn date_fmt_in(
    locale: Option<&str>,
    st: &SYSTEMTIME,
    flags: ENUM_DATE_FORMATS_FLAGS,
    picture: Option<&str>,
) -> String {
    let mut buf = [0u16; 128];
    let pic: Option<Vec<u16>> =
        picture.map(|p| p.encode_utf16().chain(std::iter::once(0)).collect());
    let loc: Option<Vec<u16>> =
        locale.map(|l| l.encode_utf16().chain(std::iter::once(0)).collect());
    // SAFETY: locale a format su retazce ukoncene nulou zijuce pocas volania; buffer s danou dlzkou.
    let n = unsafe {
        GetDateFormatEx(
            loc.as_ref()
                .map(|v| PCWSTR(v.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            flags,
            Some(st),
            pic.as_ref()
                .map(|v| PCWSTR(v.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            Some(&mut buf),
            PCWSTR::null(),
        )
    };
    if n > 1 {
        String::from_utf16_lossy(&buf[..(n as usize - 1)])
    } else {
        String::new()
    }
}

/// Nazov mesiaca a roka podla jazykoveho nastavenia (`DATE_YEARMONTH`, napr. "november 2026").
pub fn month_title(y: i32, m: u32) -> String {
    let t = date_fmt_lang(&systime(Ymd::new(y, m, 1)), DATE_YEARMONTH);
    if t.is_empty() {
        format!("{m}/{y}")
    } else {
        t
    }
}

/// Kratky datum podla regionalnych nastaveni Windows.
pub fn short_date(d: Ymd) -> String {
    date_fmt(&systime(d), DATE_SHORTDATE, None)
}

/// Dlhy datum podla jazykoveho nastavenia (napr. pre zvoleny den v kalendari).
pub fn long_date(d: Ymd) -> String {
    date_fmt_lang(&systime(d), DATE_LONGDATE)
}

/// Dnesny lokalny datum.
pub fn today() -> Ymd {
    // SAFETY: GetLocalTime nema predpoklady.
    let st = unsafe { GetLocalTime() };
    Ymd::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32)
}

/// Text hodin podla volieb (cas podla upraveneho formatu locale, volitelne den a datum podla locale).
pub fn format_clock_at(o: &ClockOptions, st: &SYSTEMTIME, locale_fmt: &str) -> String {
    let pic = time_picture(locale_fmt, o);
    let wide: Vec<u16> = pic.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 64];
    // SAFETY: format je retazec ukonceny nulou zijuci pocas volania; buffer s danou dlzkou.
    let n = unsafe {
        GetTimeFormatEx(
            PCWSTR::null(),
            TIME_FORMAT_FLAGS(0),
            Some(st),
            PCWSTR(wide.as_ptr()),
            Some(&mut buf),
        )
    };
    let time = if n > 1 {
        String::from_utf16_lossy(&buf[..(n as usize - 1)])
    } else {
        format_time(st)
    };
    let weekday = o
        .weekday
        .then(|| date_fmt(st, ENUM_DATE_FORMATS_FLAGS(0), Some("ddd")));
    let date = match o.date {
        DateMode::None => None,
        DateMode::Short => Some(date_fmt(st, DATE_SHORTDATE, None)),
        DateMode::Long => Some(date_fmt(st, DATE_LONGDATE, None)),
    };
    // dlhy datum v niektorych jazykoch uz obsahuje den - neopakovat
    let weekday = weekday.filter(|w| date.as_ref().is_none_or(|d| !d.contains(w.as_str())));
    compose(weekday.as_deref(), date.as_deref(), &time)
}

/// Text hodin teraz.
pub fn format_clock(o: &ClockOptions) -> String {
    // SAFETY: GetLocalTime nema predpoklady.
    let st = unsafe { GetLocalTime() };
    format_clock_at(o, &st, &locale_time_format())
}

/// Lokalny cas Unix sekund podla casoveho pasma Windows vratane historickych pravidiel letneho casu
/// (`SystemTimeToTzSpecificLocalTimeEx` s aktualnym dynamickym pasmom).
pub fn local_of_unix(ts: i64) -> SYSTEMTIME {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx};
    let t = ((ts + 11_644_473_600).max(0) as u64) * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: t as u32,
        dwHighDateTime: (t >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: konverzie casu nad lokalnymi strukturami.
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() {
            return utc;
        }
        if SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local).is_err() {
            return utc;
        }
    }
    local
}

/// Unix sekundy miestneho casu PC (aktualne pasmo vratane letneho casu; `TzSpecificLocalTimeToSystemTimeEx`).
pub fn utc_of_local(day: Ymd, secs: u32) -> i64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTimeEx};
    let naive = transom_core::clock::days_from_civil(day) * 86_400 + i64::from(secs);
    let local = SYSTEMTIME {
        wYear: day.y.clamp(1601, 30827) as u16,
        wMonth: day.m as u16,
        wDay: day.d as u16,
        wHour: (secs / 3600) as u16,
        wMinute: (secs / 60 % 60) as u16,
        wSecond: (secs % 60) as u16,
        ..Default::default()
    };
    let mut utc = SYSTEMTIME::default();
    let mut ft = FILETIME::default();
    // SAFETY: prevod struktur na zasobniku.
    unsafe {
        if TzSpecificLocalTimeToSystemTimeEx(None, &local, &mut utc).is_err()
            || SystemTimeToFileTime(&utc, &mut ft).is_err()
        {
            return naive;
        }
    }
    let t = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    (t / 10_000_000) as i64 - 11_644_473_600
}

/// Lokalny kalendarny den Unix sekund (pre denne suhrny vyuzitia).
pub fn local_day_of_unix(ts: i64) -> Ymd {
    let st = local_of_unix(ts);
    Ymd::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32)
}

/// Kratky lokalny datum a cas (napr. "20. 11. 2026 14:05") podla jazykoveho nastavenia.
pub fn short_datetime(ts: i64) -> String {
    let st = local_of_unix(ts);
    let d = date_fmt(&st, DATE_SHORTDATE, None);
    let o = ClockOptions::default();
    let t = format_clock_at(&o, &st, &locale_time_format());
    compose(None, Some(&d), &t)
}

/// Aktualna sekunda lokalneho casu (0..=59).
pub fn current_second() -> u16 {
    // SAFETY: GetLocalTime nema predpoklady.
    unsafe { GetLocalTime() }.wSecond
}

/// Milisekundy do dalsej celej sekundy (1..=1000).
pub fn ms_until_next_second(millis: u16) -> u32 {
    1000 - u32::from(millis.min(999))
}

/// Dalsie prebudenie: pri sekundach na hranicu sekundy, inak na hranicu minuty (vzdy s malou rezervou).
pub fn next_wake_delay_for(seconds: bool) -> u32 {
    // SAFETY: GetLocalTime nema predpoklady.
    let st = unsafe { GetLocalTime() };
    if seconds {
        ms_until_next_second(st.wMilliseconds) + 15
    } else {
        wake_delay_ms(st.wSecond, st.wMilliseconds)
    }
}

/// Jazyk zobrazenia Windows (prvy preferovany jazyk UI, napr. `sk-SK`); pri chybe prazdny retazec.
/// Len cita - jazyk Windows sa nikdy nemeni.
pub fn ui_language_tag() -> String {
    use windows::core::PWSTR;
    use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    let (mut n, mut len) = (0u32, 0u32);
    // SAFETY: prvy dotaz len zisti dlzku zoznamu.
    if unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut n, None, &mut len) }.is_err()
        || len == 0
    {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: buffer ma dlzku `len` znakov, ako ju vratil prvy dotaz.
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut n,
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    }
    .is_err()
    {
        return String::new();
    }
    let first = buf.split(|&c| c == 0).next().unwrap_or(&[]);
    String::from_utf16_lossy(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_minute_boundaries() {
        assert_eq!(ms_until_next_minute(0, 0), 60_000);
        assert_eq!(ms_until_next_minute(59, 999), 1);
        assert_eq!(ms_until_next_minute(30, 500), 29_500);
        assert_eq!(ms_until_next_minute(59, 0), 1_000);
    }

    #[test]
    fn out_of_range_inputs_are_clamped() {
        assert_eq!(ms_until_next_minute(61, 1500), 1); // leap second / sprosty vstup
    }

    #[test]
    fn wake_has_slack_after_boundary() {
        assert_eq!(wake_delay_ms(59, 999), 26);
        assert!(wake_delay_ms(0, 0) > 60_000);
    }

    #[test]
    fn second_boundary() {
        assert_eq!(ms_until_next_second(0), 1000);
        assert_eq!(ms_until_next_second(999), 1);
        assert_eq!(ms_until_next_second(5000), 1);
    }

    #[test]
    fn locale_formatting_smoke() {
        // nezavisle od jazyka stroja: cas obsahuje minuty, nazvy dni su neprazdne, prvy den je 0..=6
        let st = SYSTEMTIME {
            wYear: 2026,
            wMonth: 11,
            wDay: 20,
            wHour: 7,
            wMinute: 5,
            wSecond: 9,
            ..Default::default()
        };
        let o = ClockOptions {
            hour: transom_core::clock::HourMode::H24,
            leading_zero: transom_core::clock::LeadingZero::Yes,
            seconds: true,
            ..Default::default()
        };
        assert!(format_clock_at(&o, &st, "H:mm:ss").contains("07"));
        assert!(format_clock_at(&o, &st, "H:mm:ss").contains("09"));
        assert!(locale_first_day() <= 6);
        assert!(short_day_names().iter().all(|n| !n.is_empty()));
        assert!(month_title(2026, 11).contains("2026"));
        // Unix 0 je 1. 1. 1970 UTC; lokalne je to 31. 12. 1969 alebo 1. 1. 1970 podla pasma
        let d = local_day_of_unix(0);
        assert!(d == Ymd::new(1970, 1, 1) || d == Ymd::new(1969, 12, 31));
        assert!(!short_datetime(1_795_180_921).is_empty());
    }

    #[test]
    fn fallback_format_is_padded() {
        let st = SYSTEMTIME {
            wHour: 7,
            wMinute: 5,
            ..Default::default()
        };
        // formatovanie cez API musi vratit nieco s cislicami 7 a 05 (alebo 07:05)
        let s = format_time(&st);
        assert!(s.contains('7') && s.contains("05"), "{s}");
    }
}
