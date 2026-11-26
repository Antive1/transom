//! Pristupne nazvy nativnych vstupnych poli (HEX, R, G, B, datum): bez nazvu ich citacka obrazovky ohlasi len ako "pole".
//! Nazov sa nastavi cez `IAccPropServices` (dokumentovane API, bez podtriedenia okna) a po zmene jazyka sa obnovi.
use transom_i18n::t;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{CAccPropServices, IAccPropServices, PROPID_ACC_NAME};

/// Nazvy poli v poradi `Ui.edits` (HEX, R, G, B, datum, hladanie miesta) v jazyku aplikacie.
pub fn names() -> [String; 6] {
    let rgb = t("app.rgb");
    [
        t("app.hex"),
        format!("{rgb}: R"),
        format!("{rgb}: G"),
        format!("{rgb}: B"),
        t("dat.date"),
        t("ex.wx_search"),
    ]
}

/// Nastavi pristupny nazov pola (chyba sa ticho ignoruje: pole ostane funkcne, len bez nazvu).
pub fn set(edit: HWND, name: &str) {
    const OBJID_CLIENT: u32 = 0xFFFF_FFFC; // -4
    const CHILDID_SELF: u32 = 0;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: COM na vlakne okna (jednovlaknovy apartman, opakovana inicializacia je povolena); `wide` zije pocas volania.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(svc) =
            CoCreateInstance::<_, IAccPropServices>(&CAccPropServices, None, CLSCTX_INPROC_SERVER)
        else {
            return;
        };
        let _ = svc.SetHwndPropStr(
            edit,
            OBJID_CLIENT,
            CHILDID_SELF,
            PROPID_ACC_NAME,
            PCWSTR(wide.as_ptr()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_i18n::Lang;

    #[test]
    fn every_field_has_a_distinct_translated_accessible_name_in_every_language() {
        let _env = crate::archive_tests::ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for lang in Lang::ALL {
            transom_i18n::set_lang(lang);
            let n = names();
            for (i, a) in n.iter().enumerate() {
                assert!(!a.trim().is_empty(), "{lang:?} #{i}");
                assert!(
                    !a.starts_with("app.") && !a.starts_with("dat."),
                    "{lang:?}: neprelozeny kluc {a:?}"
                );
                for b in &n[i + 1..] {
                    assert_ne!(a, b, "{lang:?}");
                }
            }
        }
        transom_i18n::set_lang(Lang::En);
    }

    /// Skutocne pole EDIT v skrytom okne (nic sa nezobrazi): nazov nastaveny cez `set` ma vratit MSAA `accName`.
    #[test]
    fn the_name_is_reported_by_msaa_for_a_real_edit_control() {
        const OBJID_CLIENT: u32 = 0xFFFF_FFFC; // -4
        use windows::core::{w, Interface};
        use windows::Win32::System::Variant::VARIANT;
        use windows::Win32::UI::Accessibility::{AccessibleObjectFromWindow, IAccessible};
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_CHILD, WS_OVERLAPPED,
        };
        let _env = crate::archive_tests::ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: okna sa vytvaraju na vlakne UI s platnym rodicom; retazce ziju pocas volani.
        unsafe {
            let parent = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_OVERLAPPED,
                0,
                0,
                100,
                60,
                None,
                None,
                None,
                None,
            )
            .expect("skryte okno");
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!("ff00aa"),
                WS_CHILD,
                0,
                0,
                80,
                20,
                Some(parent),
                None,
                None,
                None,
            )
            .expect("pole");
            set(edit, "Probe name HEX");
            let mut acc: *mut core::ffi::c_void = std::ptr::null_mut();
            AccessibleObjectFromWindow(edit, OBJID_CLIENT, &IAccessible::IID, &mut acc)
                .expect("MSAA objekt pola");
            let acc: IAccessible = std::mem::transmute(acc);
            let name = acc.get_accName(&VARIANT::from(0i32)).map(|b| b.to_string());
            let _ = DestroyWindow(parent);
            assert_eq!(name.as_deref(), Ok("Probe name HEX"));
        }
    }
}
