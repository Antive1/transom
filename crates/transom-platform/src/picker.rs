//! Kapatko na farbu: po spusteni nasledujuce lave kliknutie kdekolvek na obrazovke neprejde do aplikacie pod mysou,
//! ale precita farbu bodu na obrazovke (`BarEvent::ColorPicked`). Prave tlacidlo, Esc alebo 20 s necinnosti vyber
//! zrusia (`ColorPicked(None)`). Nizkourovnovy hacik mysi (`WH_MOUSE_LL`) bezi len pocas vyberu.
use std::cell::Cell;
use std::time::Instant;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, MSLLHOOKSTRUCT, WH_MOUSE_LL,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

thread_local! {
    static HOOK: Cell<isize> = const { Cell::new(0) };
    /// Zaciatok vyberu (pre casovy limit) a ci sa ma pohltit este uvolnenie tlacidla.
    static STATE: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
}

const TIMEOUT_S: u64 = 20;

/// Spusti vyber farby (volat na vlakne listy). Vrati `false`, ak sa hacik nepodarilo nastavit.
pub fn start() -> bool {
    if HOOK.with(|h| h.get()) != 0 {
        return true;
    }
    // SAFETY: hacik na vlakne so sluckou sprav; odstrani sa pri prvom kliknuti alebo zruseni.
    let h = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(proc), None, 0) };
    match h {
        Ok(h) => {
            HOOK.with(|c| c.set(h.0 as isize));
            STATE.with(|s| s.set(Some((Instant::now(), false))));
            true
        }
        Err(_) => false,
    }
}

/// Je vyber prave spusteny?
pub fn active() -> bool {
    HOOK.with(|h| h.get()) != 0
}

/// Zrusi vyber (Esc z aplikacie).
pub fn cancel() {
    if active() {
        stop();
        crate::dragdrop::post_color(None);
    }
}

fn stop() {
    let h = HOOK.with(|c| c.replace(0));
    if h != 0 {
        // SAFETY: hacik nastaveny v `start`.
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(h as *mut _));
        }
    }
}

fn pixel(x: i32, y: i32) -> Option<(u8, u8, u8)> {
    use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC, CLR_INVALID};
    // SAFETY: DC obrazovky sa hned vrati.
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return None;
        }
        let c = GetPixel(dc, x, y);
        ReleaseDC(None, dc);
        (c.0 != CLR_INVALID).then_some((
            (c.0 & 0xFF) as u8,
            ((c.0 >> 8) & 0xFF) as u8,
            ((c.0 >> 16) & 0xFF) as u8,
        ))
    }
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wp.0 as u32;
        let timed_out = STATE.with(|s| {
            s.get()
                .is_some_and(|(t0, _)| t0.elapsed().as_secs() >= TIMEOUT_S)
        });
        if timed_out && msg != WM_LBUTTONUP {
            stop();
            STATE.with(|s| s.set(None));
            crate::dragdrop::post_color(None);
        } else {
            match msg {
                WM_LBUTTONDOWN => {
                    // SAFETY: pre WH_MOUSE_LL je lParam ukazovatel na MSLLHOOKSTRUCT.
                    let pt = unsafe { (*(lp.0 as *const MSLLHOOKSTRUCT)).pt };
                    let c = pixel(pt.x, pt.y);
                    STATE.with(|s| s.set(Some((Instant::now(), true))));
                    crate::dragdrop::post_color(c);
                    return LRESULT(1);
                }
                WM_RBUTTONDOWN => {
                    STATE.with(|s| s.set(Some((Instant::now(), true))));
                    crate::dragdrop::post_color(None);
                    return LRESULT(1);
                }
                WM_LBUTTONUP | WM_RBUTTONUP => {
                    // uvolnenie patri k pohltenemu stlaceniu: pohltit aj ho a vyber ukoncit
                    if STATE.with(|s| s.get().is_some_and(|(_, swallow)| swallow)) {
                        STATE.with(|s| s.set(None));
                        stop();
                        return LRESULT(1);
                    }
                }
                _ => {}
            }
        }
    }
    // SAFETY: odovzdanie dalsiemu hacikovi.
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// HEX zapis farby (`#RRGGBB`).
pub fn hex(c: (u8, u8, u8)) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_is_upper_case_with_a_hash() {
        assert_eq!(super::hex((0, 112, 224)), "#0070E0");
        assert_eq!(super::hex((255, 255, 255)), "#FFFFFF");
    }
}
