//! Testovaci MSAA klient (samostatny proces): vypise pristupne prvky listy transom a volitelne vyvola jeden z nich.
//!   cargo run -p transom-platform --example msaa_probe [-- --invoke N] [--title transom-bar-0]
//! Overuje nazvy, role, stav (UNAVAILABLE) a `accDoDefaultAction` tak, ako ich vidi cudzi klient (citacka obrazovky).
#[cfg(windows)]
fn main() {
    use windows::core::{w, Interface, BSTR, PCWSTR};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::Variant::{VARIANT, VT_I4};
    use windows::Win32::UI::Accessibility::{AccessibleObjectFromWindow, IAccessible};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, OBJID_CLIENT};

    fn var(v: i32) -> VARIANT {
        let mut x = VARIANT::default();
        unsafe {
            let i = &mut *x.Anonymous.Anonymous;
            i.vt = VT_I4;
            i.Anonymous.lVal = v;
        }
        x
    }
    fn as_i4(v: &VARIANT) -> Option<i32> {
        unsafe {
            let i = &*v.Anonymous.Anonymous;
            (i.vt == VT_I4).then_some(i.Anonymous.lVal)
        }
    }

    let args: Vec<String> = std::env::args().collect();
    let get = |n: &str| {
        args.iter()
            .position(|a| a == n)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let title = get("--title").unwrap_or_else(|| "transom-bar-0".into());
    let invoke: Option<i32> = get("--invoke").and_then(|v| v.parse().ok());
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let t: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let Ok(h) = FindWindowW(w!("TransomBar"), PCWSTR(t.as_ptr())) else {
            println!("okno {title} nenajdene");
            return;
        };
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        if let Err(e) =
            AccessibleObjectFromWindow(h, OBJID_CLIENT.0 as u32, &IAccessible::IID, &mut p)
        {
            println!("AccessibleObjectFromWindow zlyhal: {e}");
            return;
        }
        let acc = IAccessible::from_raw(p);
        let n = acc.accChildCount().unwrap_or(-1);
        let name0 = acc.get_accName(&var(0)).unwrap_or_default();
        let role0 = acc.get_accRole(&var(0)).ok().and_then(|v| as_i4(&v));
        println!("lista: name={name0} role={role0:?} deti={n}");
        for id in 1..=n {
            let name: BSTR = acc.get_accName(&var(id)).unwrap_or_default();
            let role = acc.get_accRole(&var(id)).ok().and_then(|v| as_i4(&v));
            let state = acc.get_accState(&var(id)).ok().and_then(|v| as_i4(&v));
            let act = acc.get_accDefaultAction(&var(id)).unwrap_or_default();
            let (mut l, mut tp, mut wd, mut ht) = (0, 0, 0, 0);
            let _ = acc.accLocation(&mut l, &mut tp, &mut wd, &mut ht, &var(id));
            println!(
                "  #{id}: role={role:?} state={state:?} akcia='{act}' rect=({l},{tp},{wd}x{ht}) name='{name}'"
            );
        }
        // neplatne child ID musi vratit chybu, nie pad
        println!(
            "neplatne ID 999: {:?}",
            acc.get_accName(&var(999)).map(|b| b.to_string())
        );
        // natívny UIA klient (ako Narrator): vidi prvky cez MSAA->UIA most?
        {
            use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
            use windows::Win32::UI::Accessibility::{
                CUIAutomation, IUIAutomation, TreeScope_Children,
            };
            if let Ok(uia) =
                CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            {
                if let Ok(el) = uia.ElementFromHandle(h) {
                    let cond = uia.CreateTrueCondition().ok();
                    let kids = cond.and_then(|c| el.FindAll(TreeScope_Children, &c).ok());
                    let n = kids.as_ref().and_then(|k| k.Length().ok()).unwrap_or(-1);
                    println!(
                        "UIA: okno name='{}' deti={n}",
                        el.CurrentName().unwrap_or_default()
                    );
                    if let Some(k) = kids {
                        for i in 0..n {
                            if let Ok(e) = k.GetElement(i) {
                                println!(
                                    "  UIA #{i}: name='{}' typ={} enabled={:?}",
                                    e.CurrentName().unwrap_or_default(),
                                    e.CurrentControlType().map(|t| t.0).unwrap_or(0),
                                    e.CurrentIsEnabled().map(|b| b.as_bool())
                                );
                            }
                        }
                    }
                }
            }
        }
        if let Some(i) = invoke {
            println!(
                "accDoDefaultAction(#{i}): {:?}",
                acc.accDoDefaultAction(&var(i))
            );
        }
    }
}

#[cfg(not(windows))]
fn main() {}
