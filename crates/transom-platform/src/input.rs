//! Simulacia dokumentovanych skratiek Windows (`Win+S`, `Win+B`) cez `SendInput` a overenie ich ucinku.
//!
//! Poznatky (S4, PLAN §5):
//! - `SendInput` podlieha UIPI: ak ma popredie vyssiu integritu (okno spustene ako spravca), skratka sa
//!   **ticho** zahodi - navratova hodnota aj `GetLastError` vyzeraju ako uspech. Preto:
//!   1. pred odoslanim sa zisti popredne okno; ak je zvysene (a my nie), skratka sa neposle ([`Blocked`]);
//!   2. po odoslani sa jednorazovo (casovac hostitela, nie slucka) overi zmena popredia - vysledok sa nikdy
//!      nehlasi ako uspech len podla navratovej hodnoty.
//! - Drzane modifikatory by zmenili skratku (napr. drzany Shift: `Win+Shift+S` = vystrihnutie obrazovky), preto sa
//!   pri drzanom Shift/Ctrl/Alt/Win skratka neposle.
//! - Lista je `WS_EX_NOACTIVATE`, takze kliknutie nemeni popredie a skratka ide do kontextu povodneho okna.
use std::sync::mpsc;
use std::time::Duration;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenElevation,
    TokenIntegrityLevel, TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_A, VK_B, VK_CONTROL, VK_LWIN, VK_MENU, VK_N, VK_RETURN,
    VK_RWIN, VK_S, VK_SHIFT, VK_SPACE, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
};

/// Popredne okno v case dotazu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Foreground {
    pub hwnd: isize,
    pub class: String,
    /// Nazov suboru procesu (napr. `SearchHost.exe`); prazdny, ak proces nemozno otvorit.
    pub exe: String,
    /// `Some(true)` = zvyseny proces; `None` = nezistitelne (proces nemozno otvorit - casto prave zvyseny).
    pub elevated: Option<bool>,
    /// Integritna uroven procesu (rozhoduje UIPI); `None` = nezistitelna.
    pub integrity: Option<Integrity>,
}

/// Integritna uroven procesu (z RID posledneho prvku SID mandatory label). UIPI zahodi vstup z procesu s **nizsou**
/// urovnou do okna s vyssou.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Integrity {
    Untrusted,
    Low,
    Medium,
    MediumPlus,
    High,
    System,
    Protected,
}

impl Integrity {
    pub fn from_rid(rid: u32) -> Self {
        match rid {
            0..=0x0FFF => Integrity::Untrusted,
            0x1000..=0x1FFF => Integrity::Low,
            0x2000..=0x20FF => Integrity::Medium,
            0x2100..=0x2FFF => Integrity::MediumPlus,
            0x3000..=0x3FFF => Integrity::High,
            0x4000..=0x4FFF => Integrity::System,
            _ => Integrity::Protected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    /// `Win+S` - otvorit vyhladavanie.
    Search,
    /// `Win+B` - presunut fokus do oblasti oznameni.
    Tray,
    /// `Win+A` - rychle nastavenia Windows 11 (len klavesova skratka, nie dokumentovane API).
    QuickSettings,
    /// `Win+N` - centrum oznameni a kalendar (len klavesova skratka).
    Notifications,
    /// `Win+Ctrl+V` - strana vystupu zvuku v rychlych nastaveniach (skratka z dokumentacie podpory Microsoftu; vyber
    /// vystupu nemeni, len ho ukaze).
    OutputPicker,
    /// `Win+Space` - prepnutie jazyka / rozlozenia klavesnice (vyber Windows).
    InputSwitch,
    /// `Win+Ctrl+Vlavo` / `Win+Ctrl+Vpravo` - predchadzajuca / dalsia virtualna plocha.
    DesktopLeft,
    DesktopRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// Popredie bezi so zvysenymi pravami a my nie: UIPI by vstup ticho zahodil.
    ElevatedForeground,
    /// Pouzivatel drzi Shift/Ctrl/Alt/Win.
    ModifiersHeld,
    /// `SendInput` neprijal vsetky udalosti (napr. zamknuta plocha).
    Rejected,
}

/// Zvysenie a integritna uroven z tokenu procesu (`None` = token sa neda otvorit alebo precitat).
fn token_facts(process: HANDLE) -> (Option<bool>, Option<Integrity>) {
    // SAFETY: token sa otvara pre zadany proces a zatvori sa (CloseHandle); buffery maju velkost, ktoru vratil system.
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(process, TOKEN_QUERY, &mut tok).is_err() {
            return (None, None);
        }
        let mut te = TOKEN_ELEVATION::default();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            tok,
            TokenElevation,
            Some(&mut te as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        )
        .is_ok();
        let integrity = token_integrity(tok);
        let _ = CloseHandle(tok);
        (ok.then_some(te.TokenIsElevated != 0), integrity)
    }
}

/// Integritna uroven z tokenu (`TokenIntegrityLevel`): posledny prvok SID je RID urovne.
fn token_integrity(tok: HANDLE) -> Option<Integrity> {
    // SAFETY: `tok` je platny token otvoreny s `TOKEN_QUERY` (volajuci); buffer je zarovnany na 8 bajtov a dost velky.
    unsafe {
        let mut need = 0u32;
        // prve volanie zisti potrebnu velkost (ocakavana chyba ERROR_INSUFFICIENT_BUFFER)
        let _ = GetTokenInformation(tok, TokenIntegrityLevel, None, 0, &mut need);
        if need == 0 || need > 4096 {
            return None;
        }
        // `u64` kvoli zarovnaniu struktury s ukazovatelom
        let mut buf = vec![0u64; (need as usize).div_ceil(8)];
        GetTokenInformation(
            tok,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr() as *mut _),
            need,
            &mut need,
        )
        .ok()?;
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        Some(Integrity::from_rid(*GetSidSubAuthority(
            sid,
            u32::from(count) - 1,
        )))
    }
}

/// `true`, ak tento proces bezi so zvysenymi pravami.
pub fn self_elevated() -> bool {
    // SAFETY: pseudo-handle aktualneho procesu, netreba ho zatvarat.
    token_facts(unsafe { GetCurrentProcess() })
        .0
        .unwrap_or(false)
}

/// Integritna uroven tohto procesu.
pub fn self_integrity() -> Option<Integrity> {
    // SAFETY: pseudo-handle aktualneho procesu, netreba ho zatvarat.
    token_facts(unsafe { GetCurrentProcess() }).1
}

/// Popredne okno, jeho trieda, proces a zvysenie.
pub fn foreground() -> Foreground {
    // SAFETY: handle procesu z OpenProcess sa zatvori; buffery s danou dlzkou; okno moze medzitym zaniknut (chyba bez dereferencie).
    unsafe {
        let h = GetForegroundWindow();
        if h.0.is_null() {
            return Foreground::default();
        }
        let mut buf = [0u16; 256];
        let n = GetClassNameW(h, &mut buf);
        let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let (exe, (elevated, integrity)) =
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(p) => {
                    let mut b = [0u16; 260];
                    let mut len = b.len() as u32;
                    let exe = if QueryFullProcessImageNameW(
                        p,
                        PROCESS_NAME_FORMAT(0),
                        PWSTR(b.as_mut_ptr()),
                        &mut len,
                    )
                    .is_ok()
                    {
                        let full = String::from_utf16_lossy(&b[..len as usize]);
                        full.rsplit('\\').next().unwrap_or("").to_string()
                    } else {
                        String::new()
                    };
                    let e = token_facts(p);
                    let _ = CloseHandle(p);
                    (exe, e)
                }
                Err(_) => (String::new(), (None, None)),
            };
        Foreground {
            hwnd: h.0 as isize,
            class,
            exe,
            elevated,
            integrity,
        }
    }
}

fn modifiers_held() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        // SAFETY: GetAsyncKeyState nema predpoklady.
        .any(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0)
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), Blocked> {
    // SAFETY: rez INPUT s jeho dlzkou a velkostou struktury.
    let n = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if n as usize == inputs.len() {
        Ok(())
    } else {
        Err(Blocked::Rejected)
    }
}

/// Rozhodnutie pred odoslanim (cista funkcia, testovana bez `SendInput`). Poradie: drzane modifikatory > ziadne popredie
/// (prechod aktivacie, UAC) sa neblokuje > znama integrita popredia vyssia nez nasa (presne to, co UIPI zahodi) >
/// bez integrity sa pouzije priznak zvysenia > inak sa posle a ucinok overi `verify` v aplikacii (neznamy stav, napr.
/// proces inej identity, sa nesmie falosne blokovat).
pub(crate) fn decide(
    self_integrity: Option<Integrity>,
    self_elevated: bool,
    fg: &Foreground,
    mods_held: bool,
) -> Result<(), Blocked> {
    if mods_held {
        return Err(Blocked::ModifiersHeld);
    }
    if fg.hwnd == 0 {
        return Ok(());
    }
    match (fg.integrity, self_integrity) {
        (Some(f), Some(s)) if f > s => Err(Blocked::ElevatedForeground),
        (Some(_), Some(_)) => Ok(()),
        _ if fg.elevated == Some(true) && !self_elevated => Err(Blocked::ElevatedForeground),
        _ => Ok(()),
    }
}

/// Kontroly pred odoslanim (UIPI, modifikatory); vrati popredie pred odoslanim.
fn precheck() -> Result<Foreground, Blocked> {
    let fg = foreground();
    decide(self_integrity(), self_elevated(), &fg, modifiers_held())?;
    Ok(fg)
}

/// Odosle `Win+S` / `Win+B` (LWin dole, kláves dole/hore, LWin hore - jedno volanie `SendInput`).
/// `Ok` znamena len "odoslane"; ucinok treba overit cez [`foreground`] o chvilu neskor.
pub fn send_shortcut(s: Shortcut) -> Result<Foreground, Blocked> {
    let fg = precheck()?;
    let k = match s {
        Shortcut::Search => VK_S,
        Shortcut::Tray => VK_B,
        Shortcut::QuickSettings => VK_A,
        Shortcut::Notifications => VK_N,
        Shortcut::OutputPicker => VK_V,
        Shortcut::InputSwitch => VK_SPACE,
        Shortcut::DesktopLeft => windows::Win32::UI::Input::KeyboardAndMouse::VK_LEFT,
        Shortcut::DesktopRight => windows::Win32::UI::Input::KeyboardAndMouse::VK_RIGHT,
    };
    if matches!(
        s,
        Shortcut::OutputPicker | Shortcut::DesktopLeft | Shortcut::DesktopRight
    ) {
        send(&[
            key(VK_LWIN, false),
            key(VK_CONTROL, false),
            key(k, false),
            key(k, true),
            key(VK_CONTROL, true),
            key(VK_LWIN, true),
        ])?;
        return Ok(fg);
    }
    send(&[
        key(VK_LWIN, false),
        key(k, false),
        key(k, true),
        key(VK_LWIN, true),
    ])?;
    Ok(fg)
}

/// Odosle Enter (na otvorenie flyoutu skrytych ikon po overenom `Win+B`).
pub fn send_enter() -> Result<(), Blocked> {
    precheck()?;
    send(&[key(VK_RETURN, false), key(VK_RETURN, true)])
}

/// Prvok s fokusom podla UI Automation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Focused {
    pub name: String,
    pub automation_id: String,
    pub class_name: String,
}

/// Zisti prvok s klavesovym fokusom cez UI Automation. Bezi na kratkom MTA vlakne (UI vlakno nepumpuje spravy
/// a nevstupuje znova do wndproc); pri prekroceni `timeout` vrati `None`.
pub fn focused_element(timeout: Duration) -> Option<Focused> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("transom-uia".into())
        .spawn(move || {
            // SAFETY: COM pre toto vlakno; ak sa inicializoval, na konci sa CoUninitialize.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            let r = (|| -> windows::core::Result<Focused> {
                let uia: IUIAutomation =
                    // SAFETY: COM je inicializovany na tomto vlakne (MTA).
                    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
                // SAFETY: `uia` je platny objekt UI Automation.
                let el = unsafe { uia.GetFocusedElement()? };
                // SAFETY: `el` je platny prvok UI Automation; vlastnosti sa kopiruju do vlastnych retazcov.
                unsafe {
                    Ok(Focused {
                        name: el.CurrentName().map(|b| b.to_string()).unwrap_or_default(),
                        automation_id: el
                            .CurrentAutomationId()
                            .map(|b| b.to_string())
                            .unwrap_or_default(),
                        class_name: el
                            .CurrentClassName()
                            .map(|b| b.to_string())
                            .unwrap_or_default(),
                    })
                }
            })();
            let _ = tx.send(r.ok());
            if com {
                // SAFETY: par k uspesnemu CoInitializeEx na tom istom vlakne.
                unsafe { CoUninitialize() };
            }
        })
        .ok()?;
    rx.recv_timeout(timeout).ok().flatten()
}

/// Je lave tlacidlo mysi prave stlacene (fyzicky stav, bez ohladu na okno)? Pouziva sa na zaciatok tahania z listy, ked
/// kurzor opusti listu so stlacenym tlacidlom (lista nema zachytenie mysi).
pub fn left_button_down() -> bool {
    // VK_LBUTTON = 1; najvyssi bit = stlacene
    // SAFETY: GetAsyncKeyState nema predpoklady.
    let state = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(1) };
    state < 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(hwnd: isize, elevated: Option<bool>, integrity: Option<Integrity>) -> Foreground {
        Foreground {
            hwnd,
            class: "X".into(),
            exe: "x.exe".into(),
            elevated,
            integrity,
        }
    }

    use Integrity::*;

    #[test]
    fn rid_boundaries_map_to_integrity_levels_in_order() {
        for (rid, il) in [
            (0, Untrusted),
            (0x1000, Low),
            (0x1FFF, Low),
            (0x2000, Medium),
            (0x20FF, Medium),
            (0x2100, MediumPlus),
            (0x3000, High),
            (0x4000, System),
            (0x5000, Protected),
            (u32::MAX, Protected),
        ] {
            assert_eq!(Integrity::from_rid(rid), il, "{rid:#x}");
        }
        assert!(
            Untrusted < Low
                && Low < Medium
                && Medium < MediumPlus
                && MediumPlus < High
                && High < System
        );
    }

    #[test]
    fn modifiers_always_win_and_no_foreground_is_never_blocked() {
        let high = fg(7, Some(true), Some(High));
        assert_eq!(
            decide(Some(Medium), false, &high, true),
            Err(Blocked::ModifiersHeld)
        );
        assert_eq!(
            decide(Some(Medium), false, &fg(0, None, None), false),
            Ok(()),
            "prechod aktivacie / UAC: ziadne okno"
        );
        assert_eq!(
            decide(Some(Medium), false, &fg(0, Some(true), Some(System)), false),
            Ok(())
        );
    }

    #[test]
    fn a_foreground_of_higher_integrity_is_blocked_because_uipi_would_drop_the_input_silently() {
        for (own, theirs) in [
            (Medium, High),
            (Medium, System),
            (Low, Medium),
            (Medium, MediumPlus),
            (MediumPlus, High),
        ] {
            assert_eq!(
                decide(Some(own), false, &fg(7, Some(true), Some(theirs)), false),
                Err(Blocked::ElevatedForeground),
                "{own:?} -> {theirs:?}"
            );
        }
        // aj bez priznaku zvysenia rozhoduje integrita (napr. sluzba SYSTEM s necitatelnym priznakom)
        assert_eq!(
            decide(Some(Medium), false, &fg(7, None, Some(System)), false),
            Err(Blocked::ElevatedForeground)
        );
    }

    #[test]
    fn equal_or_lower_integrity_foreground_is_allowed() {
        for (own, theirs) in [
            (Medium, Medium),
            (Medium, Low),
            (Medium, Untrusted),
            (High, High),
            (High, Medium),
            (System, High),
        ] {
            assert_eq!(
                decide(Some(own), true, &fg(7, Some(false), Some(theirs)), false),
                Ok(()),
                "{own:?} -> {theirs:?}"
            );
        }
        // my sami zvyseni: popredie High je v poriadku
        assert_eq!(
            decide(Some(High), true, &fg(7, Some(true), Some(High)), false),
            Ok(())
        );
    }

    #[test]
    fn without_integrity_the_elevation_flag_decides_and_an_unknown_state_is_sent_and_verified_later(
    ) {
        // dnesne spravanie zostava: zvysene popredie a my nie -> blok
        assert_eq!(
            decide(None, false, &fg(7, Some(true), None), false),
            Err(Blocked::ElevatedForeground)
        );
        assert_eq!(
            decide(Some(Medium), false, &fg(7, Some(true), None), false),
            Err(Blocked::ElevatedForeground)
        );
        // nezvysene / nezistitelne (napr. proces inej identity): odosle sa a ucinok overi `verify` v aplikacii
        assert_eq!(
            decide(Some(Medium), false, &fg(7, Some(false), None), false),
            Ok(())
        );
        assert_eq!(
            decide(Some(Medium), false, &fg(7, None, None), false),
            Ok(())
        );
        assert_eq!(
            decide(None, false, &fg(7, None, Some(High)), false),
            Ok(()),
            "nasa uroven nie je znama: nevieme porovnat"
        );
        // my zvyseni a popredie zvysene: bez falosneho bloku
        assert_eq!(decide(None, true, &fg(7, Some(true), None), false), Ok(()));
    }

    #[test]
    fn the_own_token_reports_a_consistent_integrity_without_sending_any_input() {
        // citanie vlastneho tokenu: nic sa neposiela ani nemeni
        let il = self_integrity().expect("vlastny token je citatelny");
        assert!(il >= Low, "{il:?}");
        if self_elevated() {
            assert!(
                il >= High,
                "zvyseny proces ma integritu High alebo vyssiu: {il:?}"
            );
        } else {
            assert!(
                il <= MediumPlus,
                "nezvyseny proces ma najviac Medium+: {il:?}"
            );
        }
        // foreground() nepadne ani bez popredneho okna; nic neposiela
        let _ = foreground();
    }
}
