//! Spike skleneneho pozadia: `cargo run -p transom-platform --example glassprobe -- <dwm|accent> [x y]`.
//! Okno 420 x 260 na 6 s: pozadie zo systemu (DWM system backdrop / neoficialny accent acrylic) + vlastny obsah
//! s alfou po pixeloch (premultiplied BGRA cez BitBlt do okna s rozsirenym ramom). Nic netrva po skonceni.
#[cfg(windows)]
fn main() {
    imp::run();
}
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod imp {
    use windows::core::w;
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Dwm::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::Controls::MARGINS;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const W: i32 = 420;
    const H: i32 = 260;

    unsafe extern "system" fn proc_(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            match m {
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    let dc = BeginPaint(h, &mut ps);
                    paint(dc);
                    let _ = EndPaint(h, &ps);
                    LRESULT(0)
                }
                WM_TIMER => {
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(h, m, wp, lp),
            }
        }
    }

    /// Obsah: jemny tmavy nadych (alfa 35 %), svetly lesk navrchu (sklo), biely "text" pruh a nepriehladny stvorec.
    unsafe fn paint(dc: HDC) {
        unsafe {
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: W,
                    biHeight: -H,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let mem = CreateCompatibleDC(Some(dc));
            let bmp = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(mem, bmp.into());
            let px = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (W * H) as usize);
            for y in 0..H {
                for x in 0..W {
                    // podklad: tmavy nadych 35 %, lesk 0..30 % hore (premultiplied)
                    let sheen = if y < 60 && std::env::var("GP_NOSHEEN").is_err() {
                        0.30 * (1.0 - y as f32 / 60.0)
                    } else {
                        0.0
                    };
                    let ta: f32 = std::env::var("GP_TINT")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0.35);
                    let (mut r, mut g, mut b, mut a) = (0.08f32 * ta, 0.08 * ta, 0.10 * ta, ta);
                    // over: biela s alfou sheen
                    r = r * (1.0 - sheen) + sheen;
                    g = g * (1.0 - sheen) + sheen;
                    b = b * (1.0 - sheen) + sheen;
                    a = a * (1.0 - sheen) + sheen;
                    // okraj 1 px: svetla linka 40 %
                    if x == 0 || y == 0 || x == W - 1 || y == H - 1 {
                        let e = 0.4;
                        r = r * (1.0 - e) + e;
                        g = g * (1.0 - e) + e;
                        b = b * (1.0 - e) + e;
                        a = a * (1.0 - e) + e;
                    }
                    // "text": biely pruh, nepriehladny akcentovy stvorec
                    if (100..110).contains(&y) && (30..300).contains(&x) {
                        (r, g, b, a) = (1.0, 1.0, 1.0, 1.0);
                    }
                    if (150..200).contains(&y) && (30..80).contains(&x) {
                        (r, g, b, a) = (0.24, 0.84, 0.64, 1.0);
                    }
                    px[(y * W + x) as usize] = [
                        (b * 255.0) as u8,
                        (g * 255.0) as u8,
                        (r * 255.0) as u8,
                        (a * 255.0) as u8,
                    ];
                }
            }
            let _ = BitBlt(dc, 0, 0, W, H, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
        }
    }

    #[repr(C)]
    struct AccentPolicy {
        state: i32,
        flags: i32,
        color: u32,
        anim: i32,
    }
    #[repr(C)]
    struct WinCompAttr {
        attr: i32,
        data: *mut core::ffi::c_void,
        size: usize,
    }

    pub fn run() {
        let a: Vec<String> = std::env::args().collect();
        let mode = a.get(1).cloned().unwrap_or_else(|| "dwm".into());
        let x: i32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
        let y: i32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
        unsafe {
            let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            );
            let inst = GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc_),
                hInstance: inst.into(),
                lpszClassName: w!("transom-glassprobe"),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let h = CreateWindowExW(
                WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | if mode == "dwmactive" {
                        WINDOW_EX_STYLE(0)
                    } else {
                        WS_EX_NOACTIVATE
                    }
                    | if mode == "fade" {
                        WS_EX_LAYERED
                    } else {
                        WINDOW_EX_STYLE(0)
                    },
                w!("transom-glassprobe"),
                w!("glass"),
                WS_POPUP,
                x,
                y,
                W,
                H,
                None,
                None,
                Some(inst.into()),
                None,
            )
            .unwrap();
            let dark: i32 = 1;
            let _ =
                DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                h,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as _,
                4,
            );
            if mode == "dwm" || mode == "dwmactive" {
                let m = MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                println!("EXTEND {:?}", DwmExtendFrameIntoClientArea(h, &m));
                let t = DWMSBT_TRANSIENTWINDOW;
                println!(
                    "BACKDROP {:?}",
                    DwmSetWindowAttribute(h, DWMWA_SYSTEMBACKDROP_TYPE, &t as *const _ as _, 4)
                );
            } else {
                // "accent" aj "fade"
                let lib = LoadLibraryW(w!("user32.dll")).unwrap();
                let f = GetProcAddress(lib, windows::core::s!("SetWindowCompositionAttribute"));
                println!("SWCA {}", f.is_some());
                if let Some(f) = f {
                    let f: unsafe extern "system" fn(
                        HWND,
                        *mut WinCompAttr,
                    ) -> windows::core::BOOL = std::mem::transmute(f);
                    let mut p = AccentPolicy {
                        state: if mode == "blur3" { 3 } else { 4 },
                        flags: 2,
                        color: std::env::var("GP_COLOR")
                            .ok()
                            .and_then(|v| u32::from_str_radix(&v, 16).ok())
                            .unwrap_or(0x30_18_14_14),
                        anim: 0,
                    };
                    let mut d = WinCompAttr {
                        attr: 19,
                        data: &mut p as *mut _ as _,
                        size: std::mem::size_of::<AccentPolicy>(),
                    };
                    println!("ACCENT {:?}", f(h, &mut d));
                }
            }
            if mode == "fade" {
                // polovicna nepriehladnost celeho okna (vratane skla) - zaklad animacie roztmievania
                println!(
                    "LWA {:?}",
                    SetLayeredWindowAttributes(h, COLORREF(0), 110, LWA_ALPHA)
                );
            }
            if mode == "dwmactive" {
                let _ = ShowWindow(h, SW_SHOW);
                println!("FG {:?}", SetForegroundWindow(h));
            } else {
                let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            }
            SetTimer(Some(h), 1, 6000, None);
            if mode == "accent" || mode == "blur3" {
                // animacia velkosti 70 % -> 100 % za 1,5 s (stred na mieste): zachytenie, ci sklo drzi krok
                let (cx, cy) = (x + W / 2, y + H / 2);
                let t0 = std::time::Instant::now();
                let hw = h.0 as isize;
                std::thread::spawn(move || loop {
                    let t = t0.elapsed().as_secs_f32() / 1.5;
                    if t >= 1.0 {
                        break;
                    }
                    let k = 0.7 + 0.3 * t;
                    let (w, hh) = ((W as f32 * k) as i32, (H as f32 * k) as i32);
                    let _ = SetWindowPos(
                        HWND(hw as _),
                        None,
                        cx - w / 2,
                        cy - hh / 2,
                        w,
                        hh,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                    std::thread::sleep(std::time::Duration::from_millis(8));
                });
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
