//! Interaktivne oblasti listy: jeden zoznam {obdlznik, nazov, povolene} na listu, z ktoreho sa plnia
//! **tooltipy** (Win32 tooltip control) aj **pristupne nazvy** (MSAA, modul `access`). Hit-testing robi aplikacia
//! nad tym istym rozlozenim, takze vsetky tri zostavaju zosuladene.
//!
//! - Tooltipy sa aktualizuju len pri zmene zoznamu (prekreslenie kvoli hoveru tooltip neresetuje).
//! - `TTS_ALWAYSTIP`: lista je neaktivne okno (`WS_EX_NOACTIVATE`), bez tohto stylu by sa tooltip nezobrazil.
//!   Tooltip okno sa nikdy neaktivuje, takze fokus ostava v aplikacii, ktora hra media.
//! - Zoznam je v globalnom `Mutex` (nie thread-local): MSAA klienti mozu volat z ineho vlakna.
use std::collections::HashMap;
use std::sync::{Mutex, Once};
use windows::core::PWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, TOOLTIPS_CLASSW, TTF_SUBCLASS,
    TTM_ADDTOOLW, TTM_DELTOOLW, TTM_SETMAXTIPWIDTH, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetClientRect, SendMessageW, CW_USEDEFAULT, WINDOW_STYLE, WS_EX_TOPMOST,
    WS_POPUP,
};

/// Jedna interaktivna oblast (horizontalne `x .. x + w`, cela vyska lisaty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub w: i32,
    /// Pristupny nazov a text tooltipu.
    pub name: String,
    /// Tlacidlo (MSAA rola push button) alebo len text (staticky text).
    pub button: bool,
    /// Zakazane tlacidlo: tooltip a nazov ano, akcia nie (MSAA stav `UNAVAILABLE`).
    pub enabled: bool,
    /// Aktualna hodnota pre citacku obrazovky (`accValue`): cas hodin, percento kruhu AI, stav baterie; prazdna = ziadna.
    pub value: String,
    /// Bez bubliny s popisom (napr. odlozene subory: po prejdeni mysou sa namiesto nej rozbali vybezok); nazov pre
    /// citacku obrazovky ostava.
    pub no_tip: bool,
}

struct BarRegions {
    regions: Vec<Region>,
    tooltip: isize,
}

static BARS: Mutex<Option<HashMap<isize, BarRegions>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut HashMap<isize, BarRegions>) -> R) -> R {
    let mut g = BARS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

/// Kopia oblasti listy (pre MSAA).
pub(crate) fn get(hwnd: isize) -> Vec<Region> {
    with(|m| m.get(&hwnd).map(|b| b.regions.clone()).unwrap_or_default())
}

/// Zabudnut listu (pri `WM_DESTROY`; tooltip okno vlastni lista, znici sa s nou).
pub(crate) fn forget(hwnd: isize) {
    with(|m| m.remove(&hwnd));
}

fn create_tooltip(owner: HWND) -> isize {
    static INIT: Once = Once::new();
    // SAFETY: jednorazova inicializacia spolocnych ovladacich prvkov so spravnou velkostou struktury.
    INIT.call_once(|| unsafe {
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        });
    });
    // SAFETY: okno tooltipu vlastni nase okno (zanikne s nim); parametre su konstanty.
    unsafe {
        match CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            None,
            WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(owner),
            None,
            None,
            None,
        ) {
            Ok(t) => {
                SendMessageW(t, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(480)));
                t.0 as isize
            }
            Err(_) => 0,
        }
    }
}

/// `cbSize` bez `lpReserved` (V2): funguje s comctl32 v5 aj v6 (lista nema manifest pre v6).
const TOOLINFO_V2_SIZE: u32 =
    (std::mem::size_of::<TTTOOLINFOW>() - std::mem::size_of::<*mut core::ffi::c_void>()) as u32;

/// Nastavi oblasti listy. Tooltipy sa prestavaju len pri zmene. Volat na vlakne hostitela.
/// Vrati `true`, ak sa zoznam zmenil.
pub fn set(hwnd: HWND, regions: Vec<Region>) -> bool {
    let raw = hwnd.0 as isize;
    // 1) kratka vypozicka: zistit zmenu a stare pocty, bez Win32 volani pod zamkom
    let (changed, old_len, tip) = with(|m| {
        let e = m.entry(raw).or_insert(BarRegions {
            regions: Vec::new(),
            tooltip: 0,
        });
        let changed = e.regions != regions;
        (changed, e.regions.len(), e.tooltip)
    });
    if !changed {
        return false;
    }
    let tip = if tip == 0 { create_tooltip(hwnd) } else { tip };
    if tip != 0 {
        let tip_hwnd = HWND(tip as *mut _);
        let mut rc = RECT::default();
        // SAFETY: vystupny RECT je lokalny.
        let _ = unsafe { GetClientRect(hwnd, &mut rc) };
        // SAFETY: spravy tooltipu s ukazovatelmi na lokalne TTTOOLINFOW, ktore ziju pocas synchronneho SendMessageW.
        unsafe {
            for id in 0..old_len {
                let ti = TTTOOLINFOW {
                    cbSize: TOOLINFO_V2_SIZE,
                    hwnd,
                    uId: id,
                    ..Default::default()
                };
                SendMessageW(
                    tip_hwnd,
                    TTM_DELTOOLW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&ti as *const _ as isize)),
                );
            }
            for (id, r) in regions.iter().enumerate().filter(|(_, r)| !r.no_tip) {
                let mut text: Vec<u16> = r.name.encode_utf16().chain(std::iter::once(0)).collect();
                let ti = TTTOOLINFOW {
                    cbSize: TOOLINFO_V2_SIZE,
                    uFlags: TTF_SUBCLASS,
                    hwnd,
                    uId: id,
                    rect: RECT {
                        left: r.x,
                        top: 0,
                        right: r.x + r.w,
                        bottom: rc.bottom,
                    },
                    lpszText: PWSTR(text.as_mut_ptr()),
                    ..Default::default()
                };
                // tooltip si text skopiruje
                SendMessageW(
                    tip_hwnd,
                    TTM_ADDTOOLW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&ti as *const _ as isize)),
                );
            }
        }
    }
    with(|m| {
        if let Some(e) = m.get_mut(&raw) {
            e.regions = regions;
            e.tooltip = tip;
        }
    });
    true
}
