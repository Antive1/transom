//! Pristupnost (MSAA `IAccessible`) pre klientsku oblast listy: kazda oblast z `regions` je jednoduchy prvok
//! (child ID 1..=n) s nazvom, rolou (tlacidlo / staticky text), stavom `UNAVAILABLE` pri zakazanom tlacidle
//! a predvolenou akciou. UIA klienti (Narrator, Inspect) ich vidia cez MSAA proxy.
//!
//! Bezpecnost: objekt cita len kopiu oblasti z globalneho `Mutex` (nikdy stav hostitela ani handler), kazde child ID
//! sa overi a ziadna metoda nepanikari (release je `panic = "abort"`). `accDoDefaultAction` akciu nevykona
//! priamo, ale posle spravu liste (`PostMessage`), ktora ju spracuje ako klik.
use crate::regions;
use windows::core::{implement, Error, Interface, Result, BSTR, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    E_FAIL, E_INVALIDARG, E_NOTIMPL, HWND, LPARAM, POINT, RECT, S_FALSE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::{
    IDispatch, IDispatch_Impl, ITypeInfo, DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO,
};
use windows::Win32::System::Variant::{VARIANT, VT_EMPTY, VT_I4};
use windows::Win32::UI::Accessibility::{
    CreateStdAccessibleObject, IAccessible, IAccessible_Impl, NAVDIR_FIRSTCHILD, NAVDIR_LASTCHILD,
    NAVDIR_NEXT, NAVDIR_PREVIOUS, ROLE_SYSTEM_PUSHBUTTON, ROLE_SYSTEM_STATICTEXT,
    ROLE_SYSTEM_TOOLBAR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, PostMessageW, OBJID_WINDOW,
};

/// Nazov predvolenej akcie tlacidiel pre citacky obrazovky (v jazyku aplikacie; nastavuje aplikacia).
static DEFAULT_ACTION: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Nastavi nazov predvolenej akcie (napr. "Stlačiť") - vola sa pri zmene jazyka aplikacie.
pub fn set_default_action_name(name: &str) {
    if let Ok(mut g) = DEFAULT_ACTION.lock() {
        *g = name.to_string();
    }
}

pub(crate) fn default_action_name() -> String {
    DEFAULT_ACTION
        .lock()
        .ok()
        .map(|g| g.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Press".into())
}

/// `STATE_SYSTEM_UNAVAILABLE` (oleacc.h).
pub(crate) const STATE_UNAVAILABLE: i32 = 0x1;
/// Sprava liste: vykonat akciu oblasti `wParam` (0-based) - z `accDoDefaultAction`.
pub const WM_TRANSOM_INVOKE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x32;

pub(crate) fn var_i4(v: i32) -> VARIANT {
    let mut x = VARIANT::default();
    // SAFETY: VARIANT je inicializovany (Default); zapisujeme typ a hodnotu do jeho vlastnej unie.
    unsafe {
        let inner = &mut *x.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = v;
    }
    x
}

pub(crate) fn var_empty() -> VARIANT {
    let mut x = VARIANT::default();
    // SAFETY: VARIANT je inicializovany (Default); zapisujeme len typ do jeho vlastnej unie.
    unsafe {
        (*x.Anonymous.Anonymous).vt = VT_EMPTY;
    }
    x
}

/// Child ID z VARIANT (len `VT_I4`); `None` pri inom type.
pub(crate) fn child_of(v: &VARIANT) -> Option<i32> {
    // SAFETY: citanie unie VARIANT: typ sa overi pred citanim hodnoty `lVal`.
    unsafe {
        let inner = &*v.Anonymous.Anonymous;
        (inner.vt == VT_I4).then_some(inner.Anonymous.lVal)
    }
}

#[implement(IAccessible)]
pub(crate) struct BarAccessible {
    hwnd: isize,
}

impl BarAccessible {
    pub(crate) fn create(hwnd: HWND) -> IAccessible {
        BarAccessible {
            hwnd: hwnd.0 as isize,
        }
        .into()
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    /// Overi child ID: `Ok(None)` = samotna lista, `Ok(Some(i))` = oblast `i` (0-based).
    fn resolve(&self, v: &VARIANT) -> Result<(Option<usize>, Vec<regions::Region>)> {
        let regs = regions::get(self.hwnd);
        match child_of(v) {
            Some(0) => Ok((None, regs)),
            Some(id) if id > 0 && (id as usize) <= regs.len() => Ok((Some(id as usize - 1), regs)),
            _ => Err(E_INVALIDARG.into()),
        }
    }
}

impl IDispatch_Impl for BarAccessible_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }
    fn GetTypeInfo(&self, _: u32, _: u32) -> Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }
    fn GetIDsOfNames(
        &self,
        _: *const GUID,
        _: *const PCWSTR,
        _: u32,
        _: u32,
        _: *mut i32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn Invoke(
        &self,
        _: i32,
        _: *const GUID,
        _: u32,
        _: DISPATCH_FLAGS,
        _: *const DISPPARAMS,
        _: *mut VARIANT,
        _: *mut EXCEPINFO,
        _: *mut u32,
    ) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

impl IAccessible_Impl for BarAccessible_Impl {
    fn accParent(&self) -> Result<IDispatch> {
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: vystupny ukazovatel `p` je lokalny; pri uspechu nesie odkaz IDispatch, ktory prevezme `from_raw` (bez dalsieho AddRef).
        unsafe {
            CreateStdAccessibleObject(self.hwnd(), OBJID_WINDOW.0, &IDispatch::IID, &mut p)?;
            if p.is_null() {
                return Err(E_FAIL.into());
            }
            Ok(IDispatch::from_raw(p))
        }
    }
    fn accChildCount(&self) -> Result<i32> {
        Ok(regions::get(self.hwnd).len() as i32)
    }
    fn get_accChild(&self, v: &VARIANT) -> Result<IDispatch> {
        self.resolve(v)?;
        // jednoduche prvky nemaju vlastny objekt: S_FALSE -> klient pouzije child ID
        Err(Error::from_hresult(S_FALSE))
    }
    fn get_accName(&self, v: &VARIANT) -> Result<BSTR> {
        let (i, regs) = self.resolve(v)?;
        Ok(match i {
            None => BSTR::from("transom"),
            Some(i) => BSTR::from(regs[i].name.as_str()),
        })
    }
    fn get_accValue(&self, v: &VARIANT) -> Result<BSTR> {
        let (i, regs) = self.resolve(v)?;
        Ok(i.map(|i| BSTR::from(regs[i].value.as_str()))
            .unwrap_or_default())
    }
    fn get_accDescription(&self, v: &VARIANT) -> Result<BSTR> {
        self.resolve(v)?;
        Ok(BSTR::new())
    }
    fn get_accRole(&self, v: &VARIANT) -> Result<VARIANT> {
        let (i, regs) = self.resolve(v)?;
        let role = match i {
            None => ROLE_SYSTEM_TOOLBAR,
            Some(i) if regs[i].button => ROLE_SYSTEM_PUSHBUTTON,
            Some(_) => ROLE_SYSTEM_STATICTEXT,
        };
        Ok(var_i4(role as i32))
    }
    fn get_accState(&self, v: &VARIANT) -> Result<VARIANT> {
        let (i, regs) = self.resolve(v)?;
        let st = match i {
            Some(i) if !regs[i].enabled => STATE_UNAVAILABLE,
            _ => 0,
        };
        Ok(var_i4(st))
    }
    fn get_accHelp(&self, v: &VARIANT) -> Result<BSTR> {
        self.resolve(v)?;
        Ok(BSTR::new())
    }
    fn get_accHelpTopic(&self, _: *mut BSTR, _: &VARIANT) -> Result<i32> {
        Err(E_NOTIMPL.into())
    }
    fn get_accKeyboardShortcut(&self, v: &VARIANT) -> Result<BSTR> {
        self.resolve(v)?;
        Ok(BSTR::new())
    }
    fn accFocus(&self) -> Result<VARIANT> {
        Ok(var_empty())
    }
    fn accSelection(&self) -> Result<VARIANT> {
        Ok(var_empty())
    }
    fn get_accDefaultAction(&self, v: &VARIANT) -> Result<BSTR> {
        let (i, regs) = self.resolve(v)?;
        Ok(match i {
            Some(i) if regs[i].button && regs[i].enabled => {
                BSTR::from(default_action_name().as_str())
            }
            _ => BSTR::new(),
        })
    }
    fn accSelect(&self, _: i32, _: &VARIANT) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn accLocation(
        &self,
        l: *mut i32,
        t: *mut i32,
        w: *mut i32,
        h: *mut i32,
        v: &VARIANT,
    ) -> Result<()> {
        if l.is_null() || t.is_null() || w.is_null() || h.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let (i, regs) = self.resolve(v)?;
        // SAFETY: Win32 volania s handle nasho okna a lokalnymi strukturami; index `i` vratil `resolve`, je v rozsahu `regs`.
        let (x0, y0, x1, y1) = unsafe {
            match i {
                None => {
                    let mut r = RECT::default();
                    GetWindowRect(self.hwnd(), &mut r)?;
                    (r.left, r.top, r.right, r.bottom)
                }
                Some(i) => {
                    let mut rc = RECT::default();
                    GetClientRect(self.hwnd(), &mut rc)?;
                    let mut a = POINT { x: regs[i].x, y: 0 };
                    let mut b = POINT {
                        x: regs[i].x + regs[i].w,
                        y: rc.bottom,
                    };
                    let _ = ClientToScreen(self.hwnd(), &mut a);
                    let _ = ClientToScreen(self.hwnd(), &mut b);
                    (a.x, a.y, b.x, b.y)
                }
            }
        };
        // SAFETY: vystupne ukazovatele overene vyssie (nie su null); klient MSAA ich poskytuje na zapis.
        unsafe {
            *l = x0;
            *t = y0;
            *w = x1 - x0;
            *h = y1 - y0;
        }
        Ok(())
    }
    fn accNavigate(&self, dir: i32, start: &VARIANT) -> Result<VARIANT> {
        let (i, regs) = self.resolve(start)?;
        let n = regs.len() as i32;
        let target = match (dir as u32, i) {
            (NAVDIR_FIRSTCHILD, None) if n > 0 => Some(1),
            (NAVDIR_LASTCHILD, None) if n > 0 => Some(n),
            (NAVDIR_NEXT, Some(i)) if (i as i32) + 1 < n => Some(i as i32 + 2),
            (NAVDIR_PREVIOUS, Some(i)) if i > 0 => Some(i as i32),
            _ => None,
        };
        Ok(target.map(var_i4).unwrap_or_else(var_empty))
    }
    fn accHitTest(&self, x: i32, y: i32) -> Result<VARIANT> {
        let mut p = POINT { x, y };
        let mut rc = RECT::default();
        // SAFETY: Win32 volania s handle nasho okna a lokalnymi strukturami.
        unsafe {
            let _ = ScreenToClient(self.hwnd(), &mut p);
            GetClientRect(self.hwnd(), &mut rc)?;
        }
        if p.x < rc.left || p.x >= rc.right || p.y < rc.top || p.y >= rc.bottom {
            return Ok(var_empty());
        }
        let regs = regions::get(self.hwnd);
        Ok(
            match regs.iter().position(|r| p.x >= r.x && p.x < r.x + r.w) {
                Some(i) => var_i4(i as i32 + 1),
                None => var_i4(0),
            },
        )
    }
    fn accDoDefaultAction(&self, v: &VARIANT) -> Result<()> {
        let (i, regs) = self.resolve(v)?;
        match i {
            // SAFETY: PostMessageW len s cislami (index), ziadny ukazovatel neprechadza spravou.
            Some(i) if regs[i].button && regs[i].enabled => unsafe {
                PostMessageW(Some(self.hwnd()), WM_TRANSOM_INVOKE, WPARAM(i), LPARAM(0))
            },
            _ => Err(Error::from_hresult(HRESULT(E_FAIL.0))),
        }
    }
    fn put_accName(&self, _: &VARIANT, _: &BSTR) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn put_accValue(&self, _: &VARIANT, _: &BSTR) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_roundtrip_and_invalid_child() {
        assert_eq!(child_of(&var_i4(3)), Some(3));
        assert_eq!(child_of(&var_empty()), None);
        let a = BarAccessible { hwnd: 0x1234_5678 };
        // neznama lista: 0 oblasti -> child 1 je neplatny, child 0 (samotna lista) platny
        assert!(a.resolve(&var_i4(1)).is_err());
        assert!(a.resolve(&var_i4(-5)).is_err());
        assert!(a.resolve(&var_empty()).is_err());
        assert!(matches!(a.resolve(&var_i4(0)), Ok((None, _))));
    }
}
