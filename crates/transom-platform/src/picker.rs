//! Kapatko na farbu: po spusteni nasledujuce lave kliknutie kdekolvek na obrazovke neprejde do aplikacie pod mysou,
//! ale precita farbu bodu na obrazovke (`BarEvent::ColorPicked`). Prave tlacidlo, Esc alebo 30 s necinnosti vyber
//! zrusia (`ColorPicked(None)`).
//!
//! Pocas vyberu:
//! - **kurzor je kapatko**: cez celu virtualnu obrazovku lezi takmer neviditelne okno (vrstvene, krytie 1/255), ktore
//!   vlastni kurzor. Neaktivuje sa (aktivacia by zmenila farby okna, ktore sa prave meria) a je vylucene zo snimania
//!   obrazovky (`WDA_EXCLUDEFROMCAPTURE`), takze nameranu farbu nijako neovplyvni;
//! - **lupa** pri kurzore ukazuje zvacsene okolie (11 x 11 bodov), vybrany bod v strede a jeho HEX; tiez je vylucena
//!   zo snimania;
//! - nizkourovnovy hacik mysi (`WH_MOUSE_LL`, kliknutie a pohyb) bezi len pocas vyberu; Esc je len pocas vyberu
//!   zaregistrovany ako globalna skratka (`RegisterHotKey`) - klavesnica sa nesleduje (ziadny klavesnicovy hacik).
use std::cell::Cell;
use std::time::Instant;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, GetDC, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyWindow,
    GetCursorPos, GetSystemMetrics, RegisterClassW, SetCursor, SetLayeredWindowAttributes,
    SetWindowDisplayAffinity, SetWindowPos, SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx,
    HCURSOR, HHOOK, HICON, HWND_TOPMOST, ICONINFO, LWA_ALPHA, MA_NOACTIVATE, MSLLHOOKSTRUCT,
    SM_CXCURSOR, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

thread_local! {
    static HOOK: Cell<isize> = const { Cell::new(0) };
    /// Zaciatok vyberu (pre casovy limit) a ci sa ma pohltit este uvolnenie tlacidla.
    static STATE: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
    static OVERLAY: Cell<isize> = const { Cell::new(0) };
    static LOUPE: Cell<isize> = const { Cell::new(0) };
    /// Kurzor kapatka (vytvori sa raz, plati do konca procesu).
    static CURSOR: Cell<isize> = const { Cell::new(0) };
    static CLASSES: Cell<bool> = const { Cell::new(false) };
    /// Posledna poloha mysi (px obrazovky) - stred lupy.
    static AT: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
}

const TIMEOUT_S: u64 = 30;
/// Id skratky Esc (len pocas vyberu, na okne prekrytia).
const ESC_HOTKEY: i32 = 0xB0E5;
/// Lupa: pocet bodov na stranu (neparny, stred je vybrany bod), velkost bodu a vyska pasu s HEX (lp).
const LOUPE_N: i32 = 11;
const LOUPE_CELL_LP: i32 = 10;
const LOUPE_BAND_LP: i32 = 26;
/// Odstup lupy od kurzora (lp).
const LOUPE_GAP_LP: i32 = 22;

/// Spusti vyber farby (volat na vlakne listy). Vrati `false`, ak sa hacik nepodarilo nastavit.
pub fn start() -> bool {
    if HOOK.with(|h| h.get()) != 0 {
        return true;
    }
    // SAFETY: haciky na vlakne so sluckou sprav; odstrania sa pri kliknuti, zruseni alebo vyprsani.
    let h = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(proc), None, 0) };
    let Ok(h) = h else { return false };
    HOOK.with(|c| c.set(h.0 as isize));
    STATE.with(|s| s.set(Some((Instant::now(), false))));
    show_windows();
    true
}

/// Je vyber prave spusteny?
pub fn active() -> bool {
    HOOK.with(|h| h.get()) != 0
}

/// Zrusi vyber (Esc z aplikacie).
pub fn cancel() {
    if active() {
        stop();
        STATE.with(|s| s.set(None));
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
    let o = OVERLAY.with(|c| c.get());
    if o != 0 {
        // SAFETY: skratka registrovana na nase prekrytie v `show_windows`.
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey(
                Some(HWND(o as *mut _)),
                ESC_HOTKEY,
            );
        }
    }
    for win in [&OVERLAY, &LOUPE] {
        let h = win.with(|c| c.replace(0));
        if h != 0 {
            // SAFETY: okno vytvorene v `show_windows` na tomto vlakne.
            unsafe {
                let _ = DestroyWindow(HWND(h as *mut _));
            }
        }
    }
}

/// Body obrazovky v obdlzniku (BGRA, riadok po riadku); okna vylucene zo snimania (kapatko, lupa) v nich nie su.
fn capture(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u32>> {
    // SAFETY: docasne DC a DIB sa uvolnia na konci; pixely sa citaju len v hraniciach DIB.
    unsafe {
        let sdc = GetDC(None);
        if sdc.is_invalid() {
            return None;
        }
        let mdc = CreateCompatibleDC(Some(sdc));
        let bi = dib_info(w, h);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let out = match CreateDIBSection(Some(mdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) if !bits.is_null() => {
                let old = SelectObject(mdc, bmp.into());
                let ok = BitBlt(mdc, 0, 0, w, h, Some(sdc), x, y, SRCCOPY).is_ok();
                let v = ok.then(|| {
                    std::slice::from_raw_parts(bits as *const u32, (w * h) as usize).to_vec()
                });
                SelectObject(mdc, old);
                let _ = DeleteObject(bmp.into());
                v
            }
            _ => None,
        };
        let _ = DeleteDC(mdc);
        ReleaseDC(None, sdc);
        out
    }
}

fn dib_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
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
    }
}

fn rgb_of(c: u32) -> (u8, u8, u8) {
    (
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    )
}

/// Farba bodu obrazovky.
fn pixel(x: i32, y: i32) -> Option<(u8, u8, u8)> {
    capture(x, y, 1, 1)
        .and_then(|v| v.first().copied())
        .map(rgb_of)
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wp.0 as u32;
        let timed_out = STATE.with(|s| {
            s.get()
                .is_some_and(|(t0, _)| t0.elapsed().as_secs() >= TIMEOUT_S)
        });
        // SAFETY: pre WH_MOUSE_LL je lParam ukazovatel na MSLLHOOKSTRUCT.
        let pt = unsafe { (*(lp.0 as *const MSLLHOOKSTRUCT)).pt };
        if timed_out && msg != WM_LBUTTONUP {
            stop();
            STATE.with(|s| s.set(None));
            crate::dragdrop::post_color(None);
        } else {
            match msg {
                WM_MOUSEMOVE => move_loupe(pt.x, pt.y),
                WM_LBUTTONDOWN => {
                    let c = pixel(pt.x, pt.y);
                    STATE.with(|s| s.set(Some((Instant::now(), true))));
                    hide_windows();
                    crate::dragdrop::post_color(c);
                    return LRESULT(1);
                }
                WM_RBUTTONDOWN => {
                    STATE.with(|s| s.set(Some((Instant::now(), true))));
                    hide_windows();
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

/// Mierka (DPI / 96) monitora v bode.
fn scale_at(x: i32, y: i32) -> f32 {
    let mut dx = 96u32;
    let mut dy = 96u32;
    // SAFETY: vystupne hodnoty su lokalne.
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    }
    dx.max(96) as f32 / 96.0
}

fn loupe_size(s: f32) -> (i32, i32, i32) {
    let cell = ((LOUPE_CELL_LP as f32 * s).round() as i32).max(6);
    let side = cell * LOUPE_N;
    (cell, side, side + (LOUPE_BAND_LP as f32 * s).round() as i32)
}

/// Lupa vedla kurzora (vpravo dole, pri okraji monitora na opacnej strane) a jej prekreslenie.
fn move_loupe(x: i32, y: i32) {
    AT.with(|a| a.set((x, y)));
    let h = LOUPE.with(|c| c.get());
    if h == 0 {
        return;
    }
    let s = scale_at(x, y);
    let (_, w, ht) = loupe_size(s);
    let gap = (LOUPE_GAP_LP as f32 * s).round() as i32;
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: vystupna struktura je lokalna; okno lupy patri tomuto vlaknu.
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(m, &mut mi);
        let r = mi.rcMonitor;
        let lx = if x + gap + w > r.right {
            x - gap - w
        } else {
            x + gap
        };
        let ly = if y + gap + ht > r.bottom {
            y - gap - ht
        } else {
            y + gap
        };
        let hw = HWND(h as *mut _);
        let _ = SetWindowPos(hw, Some(HWND_TOPMOST), lx, ly, w, ht, SWP_NOACTIVATE);
        let _ = InvalidateRect(Some(hw), None, false);
    }
}

fn register_classes() {
    if CLASSES.with(|c| c.replace(true)) {
        return;
    }
    // SAFETY: registracia tried s konstantnymi retazcami a nasimi wndproc.
    unsafe {
        let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
            .map(|m| m.into())
            .unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(overlay_proc),
            hInstance: hinst,
            lpszClassName: w!("TransomPickerOverlay"),
            hCursor: HCURSOR(cursor() as *mut _),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(loupe_proc),
            hInstance: hinst,
            lpszClassName: w!("TransomPickerLoupe"),
            hCursor: HCURSOR(cursor() as *mut _),
            ..Default::default()
        });
    }
}

fn show_windows() {
    register_classes();
    // SAFETY: okna tohto vlakna; znicia sa v `stop`.
    unsafe {
        let (vx, vy, vw, vh) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        );
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED;
        if let Ok(o) = CreateWindowExW(
            ex,
            w!("TransomPickerOverlay"),
            w!(""),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            None,
            None,
            None,
            None,
        ) {
            // krytie 1/255: nevidno ho, ale vlastni kurzor; zo snimania obrazovky je vylucene
            let _ = SetLayeredWindowAttributes(o, COLORREF(0), 1, LWA_ALPHA);
            let _ = SetWindowDisplayAffinity(o, WDA_EXCLUDEFROMCAPTURE);
            // Esc zrusi vyber (len kym trva; inu aplikaciu, ktora Esc drzi ako skratku, to neprepise - registracia zlyha)
            use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_NOREPEAT};
            let _ = RegisterHotKey(Some(o), ESC_HOTKEY, MOD_NOREPEAT, 0x1B);
            let _ = ShowWindow(o, SW_SHOWNOACTIVATE);
            OVERLAY.with(|c| c.set(o.0 as isize));
        }
        // lupa: mys cez nu prechadza (WS_EX_TRANSPARENT), kurzor patri prekrytiu pod nou
        if let Ok(l) = CreateWindowExW(
            ex | WS_EX_TRANSPARENT,
            w!("TransomPickerLoupe"),
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        ) {
            let _ = SetLayeredWindowAttributes(l, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetWindowDisplayAffinity(l, WDA_EXCLUDEFROMCAPTURE);
            // zaoblene rohy ako ostatne plavajuce prvky Windows 11 (DWMWCP_ROUND)
            let round: u32 = 2;
            let _ = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                l,
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(33),
                &round as *const u32 as *const _,
                4,
            );
            LOUPE.with(|c| c.set(l.0 as isize));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            move_loupe(pt.x, pt.y);
            let _ = ShowWindow(l, SW_SHOWNOACTIVATE);
        }
        // kurzor sa zmeni hned, nie az po prvom pohybe
        SetCursor(Some(HCURSOR(cursor() as *mut _)));
    }
}

/// Po kliknuti: okna hned zmiznu (vyber sa skonci az uvolnenim tlacidla).
fn hide_windows() {
    for win in [&OVERLAY, &LOUPE] {
        let h = win.with(|c| c.get());
        if h != 0 {
            // SAFETY: okno tohto vlakna.
            unsafe {
                let _ = ShowWindow(
                    HWND(h as *mut _),
                    windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                );
            }
        }
    }
}

unsafe extern "system" fn overlay_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // WM_HOTKEY: Esc pocas vyberu
        0x0312 if wp.0 as i32 == ESC_HOTKEY => {
            cancel();
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_SETCURSOR => {
            // SAFETY: kurzor vytvoreny v `cursor`.
            unsafe { SetCursor(Some(HCURSOR(cursor() as *mut _))) };
            LRESULT(1)
        }
        // SAFETY: predvolene spracovanie nasho okna.
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

unsafe extern "system" fn loupe_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // HTTRANSPARENT: mys patri oknu pod lupou (prekrytiu s kurzorom kapatka)
        WM_NCHITTEST => LRESULT(-1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: kreslenie vlastneho okna medzi BeginPaint a EndPaint.
            unsafe {
                let dc = BeginPaint(h, &mut ps);
                paint_loupe(h, dc);
                let _ = EndPaint(h, &ps);
            }
            LRESULT(0)
        }
        // SAFETY: predvolene spracovanie nasho okna.
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

/// Lupa: zvacsene okolie bodu pod kurzorom, ramik vybraneho bodu a pas s jeho farbou a HEX.
fn paint_loupe(h: HWND, dc: HDC) {
    let (x, y) = AT.with(|a| a.get());
    let s = scale_at(x, y);
    let (cell, side, total) = loupe_size(s);
    let half = LOUPE_N / 2;
    let src = capture(x - half, y - half, LOUPE_N, LOUPE_N).unwrap_or_default();
    let center = src
        .get((half * LOUPE_N + half) as usize)
        .copied()
        .map(rgb_of)
        .unwrap_or((0, 0, 0));
    let mut buf = vec![0u32; (side * total) as usize];
    for py in 0..side {
        for px in 0..side {
            let c = src
                .get(((py / cell) * LOUPE_N + px / cell) as usize)
                .copied()
                .unwrap_or(0);
            buf[(py * side + px) as usize] = c & 0x00FF_FFFF;
        }
    }
    // ramik vybraneho bodu: biely s ciernym lemom (viditelny na kazdej farbe)
    let (x0, y0) = (half * cell, half * cell);
    let b = (s.round() as i32).max(1);
    let mut frame = |r: RECT, c: u32| {
        for yy in r.top.max(0)..r.bottom.min(side) {
            for xx in r.left.max(0)..r.right.min(side) {
                let inner =
                    xx >= r.left + b && xx < r.right - b && yy >= r.top + b && yy < r.bottom - b;
                if !inner {
                    buf[(yy * side + xx) as usize] = c;
                }
            }
        }
    };
    frame(
        RECT {
            left: x0 - b,
            top: y0 - b,
            right: x0 + cell + b,
            bottom: y0 + cell + b,
        },
        0x000000,
    );
    frame(
        RECT {
            left: x0,
            top: y0,
            right: x0 + cell,
            bottom: y0 + cell,
        },
        0xFFFFFF,
    );
    // pas s farbou bodu
    let col = (center.0 as u32) << 16 | (center.1 as u32) << 8 | center.2 as u32;
    for v in buf.iter_mut().skip((side * side) as usize) {
        *v = col;
    }
    // SAFETY: pamatove DC a DIB sa uvolnia na konci; zapis do DIB v jeho hraniciach.
    unsafe {
        let mdc = CreateCompatibleDC(Some(dc));
        let bi = dib_info(side, total);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        if let Ok(bmp) = CreateDIBSection(Some(mdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                std::ptr::copy_nonoverlapping(buf.as_ptr(), bits as *mut u32, buf.len());
                let old = SelectObject(mdc, bmp.into());
                // HEX na pase: cierny alebo biely text podla jasu farby
                let lum =
                    0.2126 * center.0 as f32 + 0.7152 * center.1 as f32 + 0.0722 * center.2 as f32;
                let fg = if lum > 150.0 { 0x000000 } else { 0xFFFFFF };
                let font = CreateFontW(
                    -((13.0 * s).round() as i32),
                    0,
                    0,
                    0,
                    600,
                    0,
                    0,
                    0,
                    windows::Win32::Graphics::Gdi::DEFAULT_CHARSET,
                    windows::Win32::Graphics::Gdi::OUT_DEFAULT_PRECIS,
                    windows::Win32::Graphics::Gdi::CLIP_DEFAULT_PRECIS,
                    windows::Win32::Graphics::Gdi::CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                );
                let old_font = SelectObject(mdc, HGDIOBJ(font.0));
                SetBkMode(mdc, TRANSPARENT);
                SetTextColor(mdc, COLORREF(fg));
                let mut text: Vec<u16> = hex(center).encode_utf16().collect();
                let mut r = RECT {
                    left: 0,
                    top: side,
                    right: side,
                    bottom: total,
                };
                DrawTextW(
                    mdc,
                    &mut text,
                    &mut r,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                SelectObject(mdc, old_font);
                let _ = DeleteObject(HGDIOBJ(font.0));
                let mut rc = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(h, &mut rc);
                let _ = BitBlt(
                    dc,
                    0,
                    0,
                    side.min(rc.right),
                    total.min(rc.bottom),
                    Some(mdc),
                    0,
                    0,
                    SRCCOPY,
                );
                SelectObject(mdc, old);
            }
            let _ = DeleteObject(bmp.into());
        }
        let _ = DeleteDC(mdc);
    }
}

/// Kurzor kapatka (vytvoreny raz): pipeta sikmo zlava dole doprava hore, hrot (aktivny bod) vlavo dole; cierna s
/// bielym lemom, aby bola viditelna na kazdom pozadi.
fn cursor() -> isize {
    let c = CURSOR.with(|c| c.get());
    if c != 0 {
        return c;
    }
    // SAFETY: systemova metrika bez vedlajsich ucinkov.
    let n = unsafe { GetSystemMetrics(SM_CXCURSOR) }.clamp(32, 128);
    let px = eyedropper_pixels(n);
    let (hx, hy) = eyedropper_hotspot(n);
    // SAFETY: bitmapy sa po vytvoreni kurzora uvolnia (CreateIconIndirect si ich skopiruje).
    let made = unsafe {
        let sdc = GetDC(None);
        let bi = dib_info(n, n);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(Some(sdc), &bi, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, sdc);
        let mask = windows::Win32::Graphics::Gdi::CreateBitmap(n, n, 1, 1, None);
        match color {
            Ok(color) if !bits.is_null() && !mask.is_invalid() => {
                std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
                let info = ICONINFO {
                    fIcon: false.into(),
                    xHotspot: hx,
                    yHotspot: hy,
                    hbmMask: mask,
                    hbmColor: color,
                };
                let icon: Option<HICON> = CreateIconIndirect(&info).ok();
                let _ = DeleteObject(color.into());
                let _ = DeleteObject(mask.into());
                icon.map_or(0, |i| i.0 as isize)
            }
            _ => 0,
        }
    };
    let made = if made == 0 {
        // nahrada: systemovy kriz
        // SAFETY: systemovy kurzor sa neuvolnuje.
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                None,
                windows::Win32::UI::WindowsAndMessaging::IDC_CROSS,
            )
        }
        .map_or(0, |c| c.0 as isize)
    } else {
        made
    };
    CURSOR.with(|c| c.set(made));
    made
}

/// Aktivny bod kurzora kapatka (hrot) pri velkosti `n`.
fn eyedropper_hotspot(n: i32) -> (u32, u32) {
    let k = n as f32 / 32.0;
    ((2.0 * k).round() as u32, (29.0 * k).round() as u32)
}

/// Body kurzora kapatka (BGRA, premiesane alfou), velkost `n` x `n`. Tvar v suradniciach 32 x 32: hrot, sklenena
/// trubicka, objimka a gumena banka; cierna vypln s bielym lemom.
fn eyedropper_pixels(n: i32) -> Vec<u32> {
    // (zaciatok, koniec, polomer) useciek v suradniciach 32 x 32
    type Seg = ((f32, f32), (f32, f32), f32);
    const SEGS: [Seg; 4] = [
        ((2.5, 29.5), (8.5, 23.5), 1.1),
        ((8.5, 23.5), (18.5, 13.5), 2.7),
        ((17.0, 15.0), (21.0, 11.0), 4.2),
        ((21.5, 10.5), (26.5, 5.5), 3.6),
    ];
    let k = n as f32 / 32.0;
    let dist = |x: f32, y: f32, a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        let (px, py) = (a.0 + t * dx - x, a.1 + t * dy - y);
        (px * px + py * py).sqrt()
    };
    let outline = 1.4f32;
    let mut out = vec![0u32; (n * n) as usize];
    for py in 0..n {
        for px in 0..n {
            // stred bodu v suradniciach 32 x 32
            let (x, y) = ((px as f32 + 0.5) / k, (py as f32 + 0.5) / k);
            // znamienkova vzdialenost od tvaru (zaporna vnutri)
            let d = SEGS
                .iter()
                .map(|(a, b, r)| dist(x, y, *a, *b) - r)
                .fold(f32::MAX, f32::min);
            let px_d = d * k; // v bodoch kurzora
            let fill = (0.5 - px_d).clamp(0.0, 1.0);
            let shape = (0.5 - (px_d - outline * k)).clamp(0.0, 1.0);
            if shape <= 0.0 {
                continue;
            }
            // biely lem pod ciernou vyplnou (premiesane alfou)
            let a = shape;
            let white = a - fill; // podiel bielej
            let v = (white.max(0.0) * 255.0).round() as u32;
            let al = (a * 255.0).round() as u32;
            out[(py * n + px) as usize] = al << 24 | v << 16 | v << 8 | v;
        }
    }
    out
}

/// HEX zapis farby (`#RRGGBB`).
pub fn hex(c: (u8, u8, u8)) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_upper_case_with_a_hash() {
        assert_eq!(hex((0, 112, 224)), "#0070E0");
        assert_eq!(hex((255, 255, 255)), "#FFFFFF");
    }

    /// Na skutocnej obrazovke (`--ignored`): s prekrytim a lupou nad oknom znamej farby sa nameria presne ta farba
    /// (prekrytie s krytim 1/255 ani lupa sa do snimania nedostanu).
    #[test]
    #[ignore]
    fn live_sampling_is_exact_with_the_overlay_and_loupe_up() {
        use windows::Win32::Graphics::Gdi::CreateSolidBrush;
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };
        let pump = |ms: u64| {
            let t0 = Instant::now();
            while t0.elapsed().as_millis() < ms as u128 {
                let mut m = MSG::default();
                // SAFETY: slucka sprav vlakna testu.
                unsafe {
                    while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&m);
                        DispatchMessageW(&m);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        for (i, color) in [(0x12u8, 0x34u8, 0x56u8), (250, 250, 250), (1, 1, 1)]
            .iter()
            .enumerate()
        {
            let cls: Vec<u16> = format!("TransomPickTest{i}")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: testovacie okno tohto vlakna.
            let win = unsafe {
                let brush = CreateSolidBrush(COLORREF(
                    color.0 as u32 | (color.1 as u32) << 8 | (color.2 as u32) << 16,
                ));
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(overlay_proc),
                    lpszClassName: windows::core::PCWSTR(cls.as_ptr()),
                    hbrBackground: brush,
                    ..Default::default()
                });
                let h = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    windows::core::PCWSTR(cls.as_ptr()),
                    w!(""),
                    WS_POPUP,
                    300,
                    300,
                    300,
                    300,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
                h
            };
            pump(300);
            assert_eq!(pixel(450, 450), Some(*color), "bez prekrytia");
            show_windows();
            move_loupe(430, 430);
            pump(400);
            let got = pixel(450, 450);
            // aj priamo pod lupou
            let (lx, ly) = {
                let mut r = RECT::default();
                // SAFETY: citanie rozmerov vlastneho okna.
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(
                        HWND(LOUPE.with(|c| c.get()) as *mut _),
                        &mut r,
                    );
                }
                ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
            };
            let under = pixel(lx, ly);
            stop();
            // SAFETY: testovacie okno.
            unsafe {
                let _ = DestroyWindow(win);
            }
            pump(100);
            assert_eq!(got, Some(*color), "s prekrytim");
            assert_eq!(under, Some(*color), "pod lupou ({lx},{ly})");
        }
    }

    #[test]
    fn the_eyedropper_cursor_has_an_opaque_tip_at_the_hotspot_and_a_transparent_background() {
        for n in [32, 48, 64] {
            let px = eyedropper_pixels(n);
            let (hx, hy) = eyedropper_hotspot(n);
            let a = |x: u32, y: u32| px[(y * n as u32 + x) as usize] >> 24;
            assert!(a(hx, hy) > 128, "hrot je viditelny ({n})");
            assert_eq!(
                a(n as u32 - 1, n as u32 - 1),
                0,
                "pravy dolny roh je priehladny"
            );
            assert_eq!(a(0, 0), 0, "lavy horny roh je priehladny");
            // premiesane: farba nikdy neprekroci alfu
            assert!(px.iter().all(|c| ((c >> 16) & 0xFF) <= c >> 24));
        }
    }

    /// Kurzor kapatka ako surove BGRA (sirka u32 LE + body) do `TRANSOM_GALLERY` (rucna kontrola).
    #[test]
    #[ignore]
    fn gallery_eyedropper_cursor() {
        let Some(dir) = std::env::var_os("TRANSOM_GALLERY") else {
            return;
        };
        for n in [32, 48] {
            let px = eyedropper_pixels(n);
            let mut b = (n as u32).to_le_bytes().to_vec();
            for c in px {
                b.extend_from_slice(&c.to_le_bytes());
            }
            std::fs::write(
                std::path::Path::new(&dir).join(format!("eyedropper-{n}.bgra")),
                b,
            )
            .unwrap();
        }
    }
}
