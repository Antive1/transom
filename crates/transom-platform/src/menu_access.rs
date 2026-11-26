//! Pristupnost kontextoveho menu (popover): `MenuAccessible` - koren = `MENUPOPUP`, polozky = `MENUITEM` / `SEPARATOR`,
//! aktivna polozka ma stav fokus + hot-track a vracia ju `accFocus`. Rovnaky objekt sluzi aj pre vlastnu hlavicku okna
//! nastaveni (`Kind::TitleBar`: koren = `TITLEBAR`, polozky = `PUSHBUTTON`, `OBJID_TITLEBAR`). Rovnaka bezpecnost ako `access::BarAccessible`:
//! cita sa len kopia v globalnom `Mutex`, kazde child ID sa overi a ziadna metoda nepanikari (release je `panic = "abort"`).
//! `accDoDefaultAction` neurobi nic priamo, len posle popoveru spravu (`WM_TRANSOM_INVOKE`).
use crate::access::{
    child_of, default_action_name, var_empty, var_i4, STATE_UNAVAILABLE, WM_TRANSOM_INVOKE,
};
use windows::core::{implement, Error, Interface, Result, BSTR, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    E_FAIL, E_INVALIDARG, E_NOTIMPL, HWND, LPARAM, POINT, RECT, S_FALSE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::{
    IDispatch, IDispatch_Impl, ITypeInfo, DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CreateStdAccessibleObject, IAccessible, IAccessible_Impl, LresultFromObject, NAVDIR_FIRSTCHILD,
    NAVDIR_LASTCHILD, NAVDIR_NEXT, NAVDIR_PREVIOUS, ROLE_SYSTEM_GROUPING, ROLE_SYSTEM_MENUITEM,
    ROLE_SYSTEM_MENUPOPUP, ROLE_SYSTEM_PUSHBUTTON, ROLE_SYSTEM_SEPARATOR, ROLE_SYSTEM_TITLEBAR,
};
use windows::Win32::UI::Accessibility::{NotifyWinEvent, ROLE_SYSTEM_PANE};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, PostMessageW, OBJID_WINDOW,
};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_OBJECT_FOCUS, EVENT_OBJECT_REORDER, WM_APP};

/// Polozka menu pre citacky obrazovky (obdlznik v klientskych px popoveru).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub name: String,
    pub separator: bool,
    pub enabled: bool,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

type Snapshot = (Vec<MenuEntry>, Option<usize>);

/// Druh objektu: kontextove menu alebo vlastna hlavicka okna.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Menu,
    TitleBar,
    /// Popover so zoznamom tlacidiel (panel AI): koren = `GROUPING`, polozky = `PUSHBUTTON`.
    Panel,
    /// Klientska plocha okna nastaveni (`OBJID_CLIENT`): koren = `PANE`, polozky maju vlastnu rolu, stav, hodnotu a akciu
    /// (`PageExtra`): tlacidla, zaskrtavacie policka, volby, karty, posuvnik, vstupne polia.
    Page,
}

/// Rola, stav, hodnota a predvolena akcia polozky stranky (zarovnane s `MenuEntry` rovnakeho indexu).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageExtra {
    /// `ROLE_SYSTEM_*` (pozri konstanty `ROLE_*` nizsie).
    pub role: i32,
    /// `STATE_SYSTEM_*` bez fokusovatelnosti a fokusu (tie dopocita objekt z `enabled` a `focused`).
    pub state: i32,
    /// `accValue` (napr. "86 %" pri posuvniku, text pola).
    pub value: String,
    /// Lokalizovany nazov predvolenej akcie ("Stlacit", "Zaskrtnut", "Vybrat"); prazdny = ziadna akcia.
    pub action: String,
}

pub const ROLE_PANE: i32 = 0x10;
pub const ROLE_LISTITEM: i32 = 0x22;
pub const ROLE_STATICTEXT: i32 = 0x29;
pub const ROLE_TEXT: i32 = 0x2A;
pub const ROLE_PUSHBUTTON: i32 = 0x2B;
pub const ROLE_CHECKBUTTON: i32 = 0x2C;
pub const ROLE_RADIOBUTTON: i32 = 0x2D;
pub const ROLE_SLIDER: i32 = 0x33;
pub const ROLE_PAGETAB: i32 = 0x25;
/// Rozbalovacie tlacidlo stromu (kategoria v bocnom paneli nastaveni).
pub const ROLE_OUTLINEBUTTON: i32 = 0x40;
pub const STATE_EXPANDED: i32 = 0x200;
pub const STATE_COLLAPSED: i32 = 0x400;
pub const STATE_SELECTED: i32 = 0x2;
pub const STATE_READONLY: i32 = 0x40;
pub const STATE_PRESSED: i32 = 0x8;
pub const STATE_CHECKED: i32 = 0x10;
pub const STATE_OFFSCREEN: i32 = 0x0001_0000;
pub const STATE_SELECTABLE: i32 = 0x0020_0000;

/// Sprava oknu: klient zavolal `accDoDefaultAction` na polozke stranky (`wParam` = index polozky, 0-based).
pub const WM_TRANSOM_INVOKE_PAGE: u32 = WM_APP + 0x37;

/// Snimky (polozky, aktivna polozka) - cita ich `MenuAccessible` z lubovolneho vlakna.
static MENU: std::sync::Mutex<Snapshot> = std::sync::Mutex::new((Vec::new(), None));
static TITLE: std::sync::Mutex<Snapshot> = std::sync::Mutex::new((Vec::new(), None));
static PANEL: std::sync::Mutex<Snapshot> = std::sync::Mutex::new((Vec::new(), None));
static PAGE: std::sync::Mutex<Snapshot> = std::sync::Mutex::new((Vec::new(), None));
static PAGE_EXTRA: std::sync::Mutex<Vec<PageExtra>> = std::sync::Mutex::new(Vec::new());
static PAGE_NAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// Role, stavy a hodnoty prvkov panela v popoveri (zarovnane s `PANEL`; nezarovnane = ignoruju sa).
static PANEL_EXTRA: std::sync::Mutex<Vec<PageExtra>> = std::sync::Mutex::new(Vec::new());
static PANEL_NAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn slot(kind: Kind) -> &'static std::sync::Mutex<Snapshot> {
    match kind {
        Kind::Menu => &MENU,
        Kind::TitleBar => &TITLE,
        Kind::Panel => &PANEL,
        Kind::Page => &PAGE,
    }
}

/// Role, stavy, hodnoty a akcie prvkov otvoreneho panela (popover) a nazov korena; volat **po**
/// `popup::set_panel_accessibility` (ta ich vynuluje). Pocet musi sediet s poctom poloziek snimky.
pub fn set_panel_extras(name: String, extras: Vec<PageExtra>) {
    if let Ok(mut g) = PANEL_EXTRA.lock() {
        *g = extras;
    }
    if let Ok(mut g) = PANEL_NAME.lock() {
        *g = name;
    }
}

pub(crate) fn clear_panel_extras() {
    set_panel_extras(String::new(), Vec::new());
}

/// Extra prvku panela, ak su extras zarovnane s polozkami snimky.
fn panel_extra(i: usize) -> Option<PageExtra> {
    let n = PANEL.lock().map(|g| g.0.len()).unwrap_or(0);
    let g = PANEL_EXTRA.lock().ok()?;
    (g.len() == n && n > 0).then(|| g.get(i).cloned()).flatten()
}

fn page_extra(i: usize) -> PageExtra {
    PAGE_EXTRA
        .lock()
        .ok()
        .and_then(|g| g.get(i).cloned())
        .unwrap_or_default()
}

/// Polozky klientskej plochy okna nastaveni (po kazdom prekresleni): `entries` a `extras` su zarovnane (rovnaka dlzka),
/// `focused` = polozka s fokusom klavesnice, `name` = nazov korena. Pri zmene fokusu sa posle `EVENT_OBJECT_FOCUS`, pri zmene
/// zostavy poloziek (inej stranky) `EVENT_OBJECT_REORDER`.
pub fn set_page(
    hwnd: isize,
    name: String,
    entries: Vec<MenuEntry>,
    extras: Vec<PageExtra>,
    focused: Option<usize>,
) {
    let n = entries.len().min(extras.len());
    let (mut entries, mut extras) = (entries, extras);
    entries.truncate(n);
    extras.truncate(n);
    let (old_names, old_focus) = {
        let g = PAGE.lock().map(|g| g.clone()).unwrap_or_default();
        (g.0.iter().map(|e| e.name.clone()).collect::<Vec<_>>(), g.1)
    };
    let reorder = old_names != entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    set_snapshot_of(Kind::Page, entries, focused);
    if let Ok(mut g) = PAGE_EXTRA.lock() {
        *g = extras;
    }
    if let Ok(mut g) = PAGE_NAME.lock() {
        *g = name;
    }
    if hwnd != 0 {
        let hw = HWND(hwnd as *mut _);
        // SAFETY: oznamenie udalosti pre platne okno tohto procesu; nic sa nemeni.
        unsafe {
            if reorder {
                NotifyWinEvent(EVENT_OBJECT_REORDER, hw, OBJID_CLIENT_ID, 0);
            }
            if focused != old_focus || reorder {
                if let Some(f) = focused {
                    NotifyWinEvent(EVENT_OBJECT_FOCUS, hw, OBJID_CLIENT_ID, f as i32 + 1);
                }
            }
        }
    }
}

const OBJID_CLIENT_ID: i32 = -4;

/// Tlacidlo hlavicky okna (`OBJID_TITLEBAR`) dostalo fokus klavesnice: oznami sa citackam obrazovky.
pub fn notify_titlebar_focus(hwnd: isize, button: usize) {
    const OBJID_TITLEBAR_ID: i32 = -2;
    // SAFETY: oznamenie udalosti pre platne okno tohto procesu; nic sa nemeni.
    unsafe {
        NotifyWinEvent(
            EVENT_OBJECT_FOCUS,
            HWND(hwnd as *mut _),
            OBJID_TITLEBAR_ID,
            button as i32 + 1,
        );
    }
}

/// Odpoved na `WM_GETOBJECT` s `OBJID_CLIENT` pre okno `hwnd` (`None` = nie je to nasa poziadavka).
pub fn page_object(hwnd: isize, wp: usize, lp: isize) -> Option<isize> {
    if lp as i32 != OBJID_CLIENT_ID {
        return None;
    }
    let acc = MenuAccessible::create_kind(HWND(hwnd as *mut _), Kind::Page);
    // SAFETY: `acc` je platny IAccessible; LresultFromObject si spravi vlastnu referenciu.
    Some(unsafe { LresultFromObject(&IAccessible::IID, WPARAM(wp), &acc) }.0)
}

pub(crate) fn set_snapshot(entries: Vec<MenuEntry>, active: Option<usize>) {
    set_snapshot_of(Kind::Menu, entries, active);
}

pub(crate) fn snapshot() -> Snapshot {
    snapshot_of(Kind::Menu)
}

pub(crate) fn set_snapshot_of(kind: Kind, entries: Vec<MenuEntry>, active: Option<usize>) {
    if let Ok(mut g) = slot(kind).lock() {
        *g = (entries, active);
    }
}

pub(crate) fn snapshot_of(kind: Kind) -> Snapshot {
    slot(kind).lock().map(|g| g.clone()).unwrap_or_default()
}

/// Tlacidla vlastnej hlavicky okna (nazov, obdlznik v klientskych px, povolene) + tlacidlo s fokusom klavesnice; vola
/// okno nastaveni po kazdom prekresleni (citacky obrazovky ich citaju z `OBJID_TITLEBAR`).
pub fn set_titlebar(entries: Vec<MenuEntry>, focused: Option<usize>) {
    set_snapshot_of(Kind::TitleBar, entries, focused);
}

/// Odpoved na `WM_GETOBJECT` s `OBJID_TITLEBAR` pre okno `hwnd` (`wp`/`lp` z spravy; `None` = nie je to nasa poziadavka).
pub fn titlebar_object(hwnd: isize, wp: usize, lp: isize) -> Option<isize> {
    const OBJID_TITLEBAR: i32 = -2;
    if lp as i32 != OBJID_TITLEBAR {
        return None;
    }
    let acc = MenuAccessible::create_kind(HWND(hwnd as *mut _), Kind::TitleBar);
    // SAFETY: `acc` je platny IAccessible; LresultFromObject si spravi vlastnu referenciu.
    Some(unsafe { LresultFromObject(&IAccessible::IID, WPARAM(wp), &acc) }.0)
}

const STATE_FOCUSED: i32 = 0x4;
const STATE_HOTTRACKED: i32 = 0x80;
const STATE_FOCUSABLE: i32 = 0x0010_0000;

#[implement(IAccessible)]
pub(crate) struct MenuAccessible {
    hwnd: isize,
    kind: Kind,
}

impl MenuAccessible {
    pub(crate) fn create(hwnd: HWND) -> IAccessible {
        Self::create_kind(hwnd, Kind::Menu)
    }

    pub(crate) fn create_kind(hwnd: HWND, kind: Kind) -> IAccessible {
        MenuAccessible {
            hwnd: hwnd.0 as isize,
            kind,
        }
        .into()
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    /// Overi child ID: `None` = samotne menu, `Some(i)` = polozka `i` (0-based).
    fn resolve(&self, v: &VARIANT) -> Result<(Option<usize>, Snapshot)> {
        let snap = snapshot_of(self.kind);
        match child_of(v) {
            Some(0) => Ok((None, snap)),
            Some(id) if id > 0 && (id as usize) <= snap.0.len() => {
                Ok((Some(id as usize - 1), snap))
            }
            _ => Err(E_INVALIDARG.into()),
        }
    }
}

impl IDispatch_Impl for MenuAccessible_Impl {
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

impl IAccessible_Impl for MenuAccessible_Impl {
    fn accParent(&self) -> Result<IDispatch> {
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: vystupny ukazovatel `p` je lokalny; pri uspechu nesie odkaz IDispatch, ktory prevezme `from_raw`.
        unsafe {
            CreateStdAccessibleObject(self.hwnd(), OBJID_WINDOW.0, &IDispatch::IID, &mut p)?;
            if p.is_null() {
                return Err(E_FAIL.into());
            }
            Ok(IDispatch::from_raw(p))
        }
    }
    fn accChildCount(&self) -> Result<i32> {
        Ok(snapshot_of(self.kind).0.len() as i32)
    }
    fn get_accChild(&self, v: &VARIANT) -> Result<IDispatch> {
        self.resolve(v)?;
        // jednoduche prvky nemaju vlastny objekt: S_FALSE -> klient pouzije child ID
        Err(Error::from_hresult(S_FALSE))
    }
    fn get_accName(&self, v: &VARIANT) -> Result<BSTR> {
        let (i, (regs, _)) = self.resolve(v)?;
        Ok(match i {
            None => BSTR::from(match self.kind {
                Kind::TitleBar => String::new(),
                Kind::Page => PAGE_NAME.lock().map(|g| g.clone()).unwrap_or_default(),
                Kind::Panel => PANEL_NAME
                    .lock()
                    .map(|g| g.clone())
                    .ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "transom".to_string()),
                _ => "transom".to_string(),
            }),
            Some(i) => BSTR::from(regs[i].name.as_str()),
        })
    }
    fn get_accValue(&self, v: &VARIANT) -> Result<BSTR> {
        let (i, _) = self.resolve(v)?;
        Ok(match (self.kind, i) {
            (Kind::Page, Some(i)) => BSTR::from(page_extra(i).value),
            (Kind::Panel, Some(i)) => {
                BSTR::from(panel_extra(i).map(|e| e.value).unwrap_or_default())
            }
            _ => BSTR::new(),
        })
    }
    fn get_accDescription(&self, v: &VARIANT) -> Result<BSTR> {
        self.resolve(v)?;
        Ok(BSTR::new())
    }
    fn get_accRole(&self, v: &VARIANT) -> Result<VARIANT> {
        let (i, (regs, _)) = self.resolve(v)?;
        if self.kind == Kind::Page {
            return Ok(var_i4(match i {
                None => ROLE_SYSTEM_PANE as i32,
                Some(i) => page_extra(i).role,
            }));
        }
        let role = match (self.kind, i) {
            (Kind::Menu, None) => ROLE_SYSTEM_MENUPOPUP,
            (Kind::TitleBar, None) => ROLE_SYSTEM_TITLEBAR,
            (Kind::Panel, None) => ROLE_SYSTEM_GROUPING,
            (Kind::Panel, Some(i)) if panel_extra(i).is_some() => {
                panel_extra(i).map(|e| e.role).unwrap_or(ROLE_PUSHBUTTON) as _
            }
            (Kind::Menu, Some(i)) if regs[i].separator => ROLE_SYSTEM_SEPARATOR,
            (Kind::Menu, Some(_)) => ROLE_SYSTEM_MENUITEM,
            (Kind::TitleBar | Kind::Panel, Some(_)) => ROLE_SYSTEM_PUSHBUTTON,
            (Kind::Page, _) => ROLE_SYSTEM_PANE,
        };
        Ok(var_i4(role as i32))
    }
    fn get_accState(&self, v: &VARIANT) -> Result<VARIANT> {
        let (i, (regs, active)) = self.resolve(v)?;
        let st = match i {
            Some(i) if !regs[i].separator => {
                let mut s = if regs[i].enabled {
                    STATE_FOCUSABLE
                } else {
                    STATE_UNAVAILABLE
                };
                if self.kind == Kind::Page {
                    let extra = page_extra(i);
                    s |= extra.state;
                    if extra.role == ROLE_STATICTEXT {
                        // staticky text: len na citanie, nie je ovladac s fokusom
                        s = (s & !STATE_FOCUSABLE) | STATE_READONLY;
                    }
                }
                if self.kind == Kind::Panel {
                    if let Some(extra) = panel_extra(i) {
                        s |= extra.state;
                        if extra.role == ROLE_STATICTEXT || extra.role == ROLE_LISTITEM {
                            // text a zariadenia: len na citanie, nie su ovladace s fokusom
                            s = (s & !STATE_FOCUSABLE) | STATE_READONLY;
                        }
                    }
                }
                if active == Some(i) {
                    s |= STATE_FOCUSED | STATE_HOTTRACKED;
                }
                s
            }
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
        let (regs, active) = snapshot_of(self.kind);
        Ok(match active.filter(|a| *a < regs.len()) {
            Some(a) => var_i4(a as i32 + 1),
            None => var_i4(0),
        })
    }
    fn accSelection(&self) -> Result<VARIANT> {
        self.accFocus()
    }
    fn get_accDefaultAction(&self, v: &VARIANT) -> Result<BSTR> {
        let (i, (regs, _)) = self.resolve(v)?;
        Ok(match i {
            Some(i) if self.kind == Kind::Page => {
                if regs[i].enabled {
                    BSTR::from(page_extra(i).action)
                } else {
                    BSTR::new()
                }
            }
            Some(i) if self.kind == Kind::Panel && panel_extra(i).is_some() => {
                if regs[i].enabled {
                    BSTR::from(panel_extra(i).map(|e| e.action).unwrap_or_default())
                } else {
                    BSTR::new()
                }
            }
            Some(i) if !regs[i].separator && regs[i].enabled => {
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
        let (i, (regs, _)) = self.resolve(v)?;
        // SAFETY: Win32 volania s handle nasho okna a lokalnymi strukturami; index `i` vratil `resolve`.
        let (x0, y0, x1, y1) = unsafe {
            match i {
                None => {
                    let mut r = RECT::default();
                    GetWindowRect(self.hwnd(), &mut r)?;
                    (r.left, r.top, r.right, r.bottom)
                }
                Some(i) => {
                    let e = &regs[i];
                    let mut a = POINT { x: e.x, y: e.y };
                    let mut b = POINT {
                        x: e.x + e.w,
                        y: e.y + e.h,
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
        let (i, (regs, _)) = self.resolve(start)?;
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
        let (regs, _) = snapshot_of(self.kind);
        Ok(
            match regs
                .iter()
                .position(|e| p.x >= e.x && p.x < e.x + e.w && p.y >= e.y && p.y < e.y + e.h)
            {
                Some(i) => var_i4(i as i32 + 1),
                None => var_i4(0),
            },
        )
    }
    fn accDoDefaultAction(&self, v: &VARIANT) -> Result<()> {
        let (i, (regs, _)) = self.resolve(v)?;
        match i {
            Some(i)
                if self.kind == Kind::Page
                    && regs[i].enabled
                    && !page_extra(i).action.is_empty() =>
            // SAFETY: PostMessageW len s cislami (index).
            unsafe {
                PostMessageW(
                    Some(self.hwnd()),
                    WM_TRANSOM_INVOKE_PAGE,
                    WPARAM(i),
                    LPARAM(0),
                )
            },
            // prvok panela bez akcie (posuvnik, text, zariadenie): nie je co vykonat
            Some(i)
                if self.kind == Kind::Panel
                    && panel_extra(i).is_some_and(|e| e.action.is_empty()) =>
            {
                Err(Error::from_hresult(HRESULT(E_FAIL.0)))
            }
            // SAFETY: PostMessageW len s cislami (index).
            Some(i) if self.kind != Kind::Page && !regs[i].separator && regs[i].enabled => unsafe {
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

    /// Testy menia globalny snimok stranky: bezia po jednom.
    static PAGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolves_children_from_the_snapshot_and_rejects_invalid_ids() {
        let e = |n: &str, sep: bool| MenuEntry {
            name: n.into(),
            separator: sep,
            enabled: true,
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        set_snapshot(
            vec![e("Settings", false), e("", true), e("Quit", false)],
            Some(2),
        );
        let a = MenuAccessible {
            hwnd: 0x1234_5678,
            kind: Kind::Menu,
        };
        assert!(matches!(a.resolve(&var_i4(0)), Ok((None, _))));
        assert!(matches!(a.resolve(&var_i4(3)), Ok((Some(2), _))));
        assert!(a.resolve(&var_i4(4)).is_err());
        assert!(a.resolve(&var_i4(-1)).is_err());
        assert!(a.resolve(&var_empty()).is_err());
        set_snapshot(vec![], None);
        assert!(a.resolve(&var_i4(1)).is_err());
    }

    /// Panel v popoveri s vlastnymi rolami (posuvnik, zaskrtavacie policko, zoznam, tlacidlo, text): rola, stav, hodnota,
    /// akcia a nazov korena; prvok bez akcie (posuvnik) sa nema dat vyvolat. AI panel (bez extras) ostava tlacidlami.
    #[test]
    fn the_panel_object_uses_extras_when_aligned_and_falls_back_to_buttons_otherwise() {
        let _g = PAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
        };
        // SAFETY: testovacie okno na vlakne testu, znici sa na konci.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_OVERLAPPED,
                0,
                0,
                300,
                200,
                None,
                None,
                None,
                None,
            )
            .expect("skryte okno")
        };
        let entry = |name: &str, y: i32, enabled: bool| MenuEntry {
            name: name.into(),
            separator: false,
            enabled,
            x: 0,
            y,
            w: 100,
            h: 20,
        };
        let extra = |role: i32, state: i32, value: &str, action: &str| PageExtra {
            role,
            state,
            value: value.into(),
            action: action.into(),
        };
        set_snapshot_of(
            Kind::Panel,
            vec![
                entry("Volume", 0, true),
                entry("Mute", 30, true),
                entry("Speakers", 60, true),
                entry("Quick", 90, true),
            ],
            Some(1),
        );
        set_panel_extras(
            "Audio panel".into(),
            vec![
                extra(ROLE_SLIDER, 0, "40 %", ""),
                extra(ROLE_CHECKBUTTON, STATE_CHECKED, "", "Check"),
                extra(ROLE_LISTITEM, STATE_SELECTED, "", ""),
                extra(ROLE_PUSHBUTTON, 0, "", "Press"),
            ],
        );
        let acc: IAccessible = MenuAccessible::create_kind(hwnd, Kind::Panel);
        // SAFETY: volania platneho IAccessible v teste.
        unsafe {
            let root = var_i4(0);
            assert_eq!(acc.get_accName(&root).unwrap().to_string(), "Audio panel");
            let role = |i: i32| var_of_i4(&acc.get_accRole(&var_i4(i)).unwrap());
            let state = |i: i32| var_of_i4(&acc.get_accState(&var_i4(i)).unwrap());
            assert_eq!(role(1), ROLE_SLIDER);
            assert_eq!(acc.get_accValue(&var_i4(1)).unwrap().to_string(), "40 %");
            assert_eq!(role(2), ROLE_CHECKBUTTON);
            assert!(state(2) & STATE_CHECKED != 0 && state(2) & STATE_FOCUSED != 0);
            assert_eq!(
                acc.get_accDefaultAction(&var_i4(2)).unwrap().to_string(),
                "Check"
            );
            // zariadenie: len na citanie, bez fokusu, bez akcie
            assert_eq!(role(3), ROLE_LISTITEM);
            assert!(
                state(3) & STATE_READONLY != 0
                    && state(3) & STATE_FOCUSABLE == 0
                    && state(3) & STATE_SELECTED != 0
            );
            assert_eq!(
                acc.get_accDefaultAction(&var_i4(3)).unwrap().to_string(),
                ""
            );
            // posuvnik a zariadenie sa nedaju vyvolat; tlacidlo ano
            assert!(acc.accDoDefaultAction(&var_i4(1)).is_err());
            assert!(acc.accDoDefaultAction(&var_i4(3)).is_err());
            assert!(acc.accDoDefaultAction(&var_i4(4)).is_ok());
            // nezarovnane extras (iny pocet) = povodne spravanie tlacidiel
            set_panel_extras("x".into(), vec![extra(ROLE_SLIDER, 0, "", "")]);
            assert_eq!(role(1), ROLE_PUSHBUTTON);
            let _ = DestroyWindow(hwnd);
        }
        clear_panel_extras();
        set_snapshot_of(Kind::Panel, vec![], None);
    }

    /// Objekt `Page` na skutocnom (skrytom) okne: nazov, rola, stav, hodnota, akcia, poloha, hit-test a vyvolanie akcie cez COM
    /// rozhranie (tak, ako ho vola citacka obrazovky); nic sa nezobrazi.
    #[test]
    fn the_page_object_reports_roles_states_values_bounds_and_invokes_through_a_message() {
        let _g = PAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, PeekMessageW, MSG, PM_REMOVE, WINDOW_EX_STYLE,
            WS_OVERLAPPED,
        };
        // SAFETY: testovacie okno na vlakne testu, znici sa na konci.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_OVERLAPPED,
                0,
                0,
                300,
                200,
                None,
                None,
                None,
                None,
            )
            .expect("skryte okno")
        };
        let entry = |name: &str, x: i32, y: i32, enabled: bool| MenuEntry {
            name: name.into(),
            separator: false,
            enabled,
            x,
            y,
            w: 100,
            h: 20,
        };
        let extra = |role: i32, state: i32, value: &str, action: &str| PageExtra {
            role,
            state,
            value: value.into(),
            action: action.into(),
        };
        set_page(
            0, // bez oznamenia udalosti (okno nie je zobrazene)
            "Settings".into(),
            vec![
                entry("Layout", 0, 0, true),
                entry("Show clock", 0, 30, true),
                entry("Opacity", 0, 60, true),
                entry("Locked", 0, 90, false),
            ],
            vec![
                extra(
                    ROLE_PAGETAB,
                    STATE_SELECTED | STATE_SELECTABLE,
                    "",
                    "Select",
                ),
                extra(ROLE_CHECKBUTTON, STATE_CHECKED, "", "Uncheck"),
                extra(ROLE_SLIDER, 0, "86 %", ""),
                extra(ROLE_PUSHBUTTON, 0, "", "Press"),
            ],
            Some(1),
        );
        let acc = MenuAccessible::create_kind(hwnd, Kind::Page);
        let acc_i: IAccessible = acc;
        // SAFETY: volania platneho IAccessible v teste.
        unsafe {
            assert_eq!(acc_i.accChildCount().unwrap(), 4);
            assert_eq!(
                acc_i.get_accName(&var_i4(0)).unwrap().to_string(),
                "Settings"
            );
            assert_eq!(
                var_of_i4(&acc_i.get_accRole(&var_i4(0)).unwrap()),
                ROLE_SYSTEM_PANE as i32
            );
            // nazov, rola, stav, hodnota, akcia kazdej polozky
            assert_eq!(
                acc_i.get_accName(&var_i4(2)).unwrap().to_string(),
                "Show clock"
            );
            assert_eq!(
                var_of_i4(&acc_i.get_accRole(&var_i4(2)).unwrap()),
                ROLE_CHECKBUTTON
            );
            let st = var_of_i4(&acc_i.get_accState(&var_i4(2)).unwrap());
            assert!(
                st & STATE_CHECKED != 0 && st & STATE_FOCUSABLE != 0 && st & STATE_FOCUSED != 0,
                "{st:#x}"
            );
            let st1 = var_of_i4(&acc_i.get_accState(&var_i4(1)).unwrap());
            assert!(st1 & STATE_SELECTED != 0 && st1 & STATE_FOCUSED == 0);
            assert_eq!(acc_i.get_accValue(&var_i4(3)).unwrap().to_string(), "86 %");
            assert_eq!(
                var_of_i4(&acc_i.get_accRole(&var_i4(3)).unwrap()),
                ROLE_SLIDER
            );
            assert_eq!(
                acc_i.get_accDefaultAction(&var_i4(2)).unwrap().to_string(),
                "Uncheck"
            );
            assert_eq!(
                acc_i.get_accDefaultAction(&var_i4(3)).unwrap().to_string(),
                "",
                "posuvnik nema akciu"
            );
            // zakazana polozka: UNAVAILABLE, bez akcie
            let st4 = var_of_i4(&acc_i.get_accState(&var_i4(4)).unwrap());
            assert!(st4 & STATE_UNAVAILABLE != 0 && st4 & STATE_FOCUSABLE == 0);
            assert_eq!(
                acc_i.get_accDefaultAction(&var_i4(4)).unwrap().to_string(),
                ""
            );
            // fokus a poloha
            assert_eq!(var_of_i4(&acc_i.accFocus().unwrap()), 2);
            let (mut l, mut t, mut wd, mut h) = (0, 0, 0, 0);
            acc_i
                .accLocation(&mut l, &mut t, &mut wd, &mut h, &var_i4(2))
                .unwrap();
            assert_eq!((wd, h), (100, 20));
            // neplatne child ID sa odmietne, nepanikari
            assert!(acc_i.get_accName(&var_i4(5)).is_err());
            assert!(acc_i.get_accName(&var_i4(-3)).is_err());
            // vyvolanie: spravu dostane okno (index polozky), zakazana polozka a polozka bez akcie nic neposlu
            acc_i.accDoDefaultAction(&var_i4(2)).unwrap();
            assert!(acc_i.accDoDefaultAction(&var_i4(3)).is_err());
            assert!(acc_i.accDoDefaultAction(&var_i4(4)).is_err());
            let mut msg = MSG::default();
            assert!(PeekMessageW(
                &mut msg,
                Some(hwnd),
                WM_TRANSOM_INVOKE_PAGE,
                WM_TRANSOM_INVOKE_PAGE,
                PM_REMOVE
            )
            .as_bool());
            assert_eq!(msg.wParam.0, 1, "index polozky (0-based)");
            assert!(!PeekMessageW(
                &mut msg,
                Some(hwnd),
                WM_TRANSOM_INVOKE_PAGE,
                WM_TRANSOM_INVOKE_PAGE,
                PM_REMOVE
            )
            .as_bool());
            let _ = DestroyWindow(hwnd);
        }
        set_page(0, String::new(), vec![], vec![], None);
    }

    /// Staticky text stranky: len na citanie, bez fokusu a bez akcie (nadpis nahladu, riadok zmeny, hlaska).
    #[test]
    fn static_text_is_read_only_not_focusable_and_has_no_default_action() {
        let _g = PAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
        };
        // SAFETY: testovacie okno na vlakne testu, znici sa na konci.
        let hwnd = unsafe {
            CreateWindowExW(
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
            .expect("skryte okno")
        };
        set_page(
            0,
            "Settings".into(),
            vec![MenuEntry {
                name: "Preview: a.json".into(),
                separator: false,
                enabled: true,
                x: 0,
                y: 0,
                w: 50,
                h: 10,
            }],
            vec![PageExtra {
                role: ROLE_STATICTEXT,
                state: 0,
                value: String::new(),
                action: String::new(),
            }],
            None,
        );
        let acc: IAccessible = MenuAccessible::create_kind(hwnd, Kind::Page);
        // SAFETY: volania platneho IAccessible v teste.
        unsafe {
            let st = var_of_i4(&acc.get_accState(&var_i4(1)).unwrap());
            assert!(
                st & STATE_READONLY != 0 && st & STATE_FOCUSABLE == 0,
                "{st:#x}"
            );
            assert_eq!(
                var_of_i4(&acc.get_accRole(&var_i4(1)).unwrap()),
                ROLE_STATICTEXT
            );
            assert_eq!(
                acc.get_accDefaultAction(&var_i4(1)).unwrap().to_string(),
                ""
            );
            assert!(acc.accDoDefaultAction(&var_i4(1)).is_err());
            let _ = DestroyWindow(hwnd);
        }
        set_page(0, String::new(), vec![], vec![], None);
    }

    fn var_of_i4(v: &VARIANT) -> i32 {
        i32::try_from(v).unwrap_or(i32::MIN)
    }
}
