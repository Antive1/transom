//! Systemove nastavenia: svetly/tmavy rezim, animacie.
use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

/// `true`, ak system pouziva tmavy rezim pre panel/shell (`SystemUsesLightTheme` = 0); pri chybe/chybajucej
/// hodnote sa vrati `true` (tmava je predvolena v MVP) - nikdy nepada.
pub fn system_uses_dark() -> bool {
    read_dword(w!("SystemUsesLightTheme"))
        .or_else(|| read_dword(w!("AppsUseLightTheme")))
        .map(|v| v == 0)
        .unwrap_or(true)
}

/// Plynuly pohyb: ease-out (kubicky), `t` v 0..=1.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Su v systeme zapnute animacie (Nastavenia > Pristupnost > Vizualne efekty > Efekty animacie;
/// `SPI_GETCLIENTAREAANIMATION`)? `false` = obmedzeny pohyb: posuvny text ostane staticky. Pri chybe `true`.
pub fn animations_enabled() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut on: i32 = 1;
    // SAFETY: pvParam ukazuje na BOOL (i32), ktory SPI_GETCLIENTAREAANIMATION naplni.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut i32 as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || on != 0
}

/// Systemovy prepinac *Efekty priehladnosti* (`EnableTransparency`); chybajuca hodnota = zapnute (predvolba Windows).
pub fn transparency_enabled() -> bool {
    read_dword(w!("EnableTransparency")).is_none_or(|v| v != 0)
}

fn read_dword(name: windows::core::PCWSTR) -> Option<u32> {
    let mut val: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: vystupny buffer `val` s velkostou `size`; kluc a hodnota su konstantne retazce.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    rc.is_ok().then_some(val)
}

/// Prehra systemovy zvuk upozornenia (schema zvukov Windows, `SystemNotification`) asynchronne; pri vypnutych zvukoch
/// alebo bez vystupu sa nic nestane (ziadny nahradny pip).
pub fn notify_sound() {
    use windows::Win32::Media::Audio::{
        PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT, SND_SYSTEM,
    };
    // SAFETY: konstantny alias, asynchronne prehratie bez modulu.
    let _ = unsafe {
        PlaySoundW(
            windows::core::w!("SystemNotification"),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM,
        )
    };
}
