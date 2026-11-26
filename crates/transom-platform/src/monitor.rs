//! Monitory: enumeracia so stabilnym ID (device interface path), vyska listy v px, vyber monitorov.
//!
//! Poznatok S2: `\\.\DISPLAYn` NIE JE stabilne (po odpojeni sa monitory prečíslujú), preto sa monitor identifikuje
//! cez device interface path (`EnumDisplayDevicesW` s `EDD_GET_DEVICE_INTERFACE_NAME`). Prezitie restartu Windows
//! nie je overene.
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

impl From<Rect> for RECT {
    fn from(r: Rect) -> Self {
        RECT {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorInfo {
    /// `\\.\DISPLAYn` - len na zobrazenie/logy, nie na identifikaciu.
    pub device: String,
    /// Device interface path - stabilny identifikator v ramci relacie.
    pub id: String,
    /// `rcMonitor` (cely monitor, nie pracovna plocha).
    pub rect: Rect,
    /// Efektivne DPI monitora (96 = 100 %).
    pub dpi: u32,
    pub primary: bool,
    /// Pekny nazov monitora (napr. "AOC 2402"), ak ho system pozna; inak prazdny.
    pub name: String,
}

/// Na ktorych monitoroch ma byt lista. `Selected` nesie stabilne ID (device interface path, bez ohladu na velkost pismen).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MonitorSelection {
    #[default]
    All,
    Primary,
    Selected(Vec<String>),
}

/// Vyska listy v px z logickych pixelov (lp) a DPI.
pub fn bar_height_px(height_lp: u32, dpi: u32) -> i32 {
    ((height_lp as f64) * (dpi as f64) / 96.0).round() as i32
}

/// Obdlznik listy: hore na monitore, cez celu sirku `rcMonitor` (nie pracovnej plochy; S1, S2).
pub fn bar_rect(monitor: Rect, height_px: i32) -> Rect {
    Rect {
        left: monitor.left,
        top: monitor.top,
        right: monitor.right,
        bottom: monitor.top + height_px,
    }
}

/// Zoradi (primarny prvy, potom zlava doprava) a aplikuje vyber.
pub fn select(mons: Vec<MonitorInfo>, sel: &MonitorSelection) -> Vec<MonitorInfo> {
    select_with_fallback(mons, sel).0
}

/// Ako [`select`], plus `fallback` = **vyber `Selected` nezodpoveda ziadnemu pripojenemu monitoru**: lista sa vtedy
/// zobrazi na primarnom (aby pouzivatel nezostal bez listy) a aplikacia to **viditelne oznami**; ulozeny vyber sa nemeni
/// (po pripojeni monitora sa lista vrati). Ak je pripojena len cast vybranych, ukaze sa na nich (bez fallbacku).
pub fn select_with_fallback(
    mut mons: Vec<MonitorInfo>,
    sel: &MonitorSelection,
) -> (Vec<MonitorInfo>, bool) {
    mons.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    match sel {
        MonitorSelection::All => (mons, false),
        MonitorSelection::Primary => {
            mons.truncate(1);
            (mons, false)
        }
        MonitorSelection::Selected(ids) => {
            let chosen: Vec<MonitorInfo> = mons
                .iter()
                .filter(|m| ids.iter().any(|i| i.eq_ignore_ascii_case(&m.id)))
                .cloned()
                .collect();
            if chosen.is_empty() {
                mons.truncate(1);
                (mons, true)
            } else {
                (chosen, false)
            }
        }
    }
}

/// Ktore z ulozenych ID teraz nie su pripojene (pre stranku Displays: "nepripojene").
pub fn missing_ids(mons: &[MonitorInfo], ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|i| !mons.iter().any(|m| m.id.eq_ignore_ascii_case(i)))
        .cloned()
        .collect()
}

/// Rozdiel medzi existujucimi listami a pozadovanymi monitormi (ID).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcilePlan {
    pub remove: Vec<String>,
    pub keep: Vec<String>,
    pub add: Vec<String>,
}

pub fn plan_reconcile(existing: &[String], wanted: &[String]) -> ReconcilePlan {
    ReconcilePlan {
        remove: existing
            .iter()
            .filter(|e| !wanted.contains(e))
            .cloned()
            .collect(),
        keep: wanted
            .iter()
            .filter(|w| existing.contains(w))
            .cloned()
            .collect(),
        add: wanted
            .iter()
            .filter(|w| !existing.contains(w))
            .cloned()
            .collect(),
    }
}

fn u16str(b: &[u16]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

/// `DISPLAY_DEVICE_ACTIVE` zo `StateFlags`.
const DD_ACTIVE: u32 = 0x1;

/// Z monitorov pod jednym adaptérom (stavove priznaky, interface path) vyberie aktivny.
/// Poznatok (po S2, zistene pri odpojeni monitora): pod adapterom moze byt viac zaznamov a index 0 nemusi byt
/// aktivny - vtedy by lista dostala ID celkom ineho (len pripojeneho) monitora.
fn pick_active_id(entries: &[(u32, String)]) -> Option<String> {
    entries
        .iter()
        .find(|(flags, _)| flags & DD_ACTIVE != 0)
        .map(|(_, id)| id.clone())
}

fn interface_id(device: &str) -> String {
    let wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
    let mut entries: Vec<(u32, String)> = Vec::new();
    for i in 0..16u32 {
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // 0x1 = EDD_GET_DEVICE_INTERFACE_NAME
        // SAFETY: retazec ukonceny nulou a vystupna struktura s cb zijuce pocas volania.
        if !unsafe { EnumDisplayDevicesW(PCWSTR(wide.as_ptr()), i, &mut dd, 1) }.as_bool() {
            break;
        }
        entries.push((dd.StateFlags.0, u16str(&dd.DeviceID)));
    }
    // Zalozny identifikator: lepsie nez nic, ale nestabilny.
    pick_active_id(&entries).unwrap_or_else(|| device.to_string())
}

unsafe extern "system" fn enum_cb(hmon: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` je ukazovatel na `Vec<MonitorInfo>` z `enumerate`, ktory zije pocas celeho EnumDisplayMonitors (synchronne).
    let v = unsafe { &mut *(lp.0 as *mut Vec<MonitorInfo>) };
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: MONITORINFOEXW s nastavenym cbSize; handle monitora z enumeracie.
    if unsafe { GetMonitorInfoW(hmon, &mut mi as *mut _ as *mut MONITORINFO) }.as_bool() {
        let (mut dx, mut dy) = (96u32, 96u32);
        // SAFETY: vystupne cisla su lokalne premenne.
        let _ = unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
        let device = u16str(&mi.szDevice);
        v.push(MonitorInfo {
            id: interface_id(&device),
            device,
            rect: mi.monitorInfo.rcMonitor.into(),
            dpi: dx,
            primary: mi.monitorInfo.dwFlags & 1 != 0,
            name: String::new(),
        });
    }
    TRUE
}

/// Pekne nazvy aktivnych monitorov z `QueryDisplayConfig`: (device path malymi pismenami, nazov). Prazdne pri chybe.
fn friendly_names() -> Vec<(String, String)> {
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
    };
    let (mut np, mut nm) = (0u32, 0u32);
    // SAFETY: standardne dvojkrokove volanie (velkosti, potom data); bufre su dost velke podla prveho volania.
    unsafe {
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).0 != 0 {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        )
        .0 != 0
        {
            return Vec::new();
        }
        let mut out = Vec::new();
        for p in paths.iter().take(np as usize) {
            let mut t = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            t.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            t.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            t.header.adapterId = p.targetInfo.adapterId;
            t.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut t.header) == 0 {
                out.push((
                    u16str(&t.monitorDevicePath).to_lowercase(),
                    u16str(&t.monitorFriendlyDeviceName),
                ));
            }
        }
        out
    }
}

/// Vsetky aktualne monitory (bez zoradenia).
pub fn enumerate() -> Vec<MonitorInfo> {
    let mut v: Vec<MonitorInfo> = Vec::new();
    // SAFETY: EnumDisplayMonitors vola `enum_cb` synchronne; `v` zije az do navratu.
    let _ = unsafe {
        EnumDisplayMonitors(None, None, Some(enum_cb), LPARAM(&mut v as *mut _ as isize))
    };
    let names = friendly_names();
    for m in v.iter_mut() {
        if let Some((_, n)) = names.iter().find(|(p, _)| *p == m.id.to_lowercase()) {
            m.name = n.clone();
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(id: &str, left: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            device: String::new(),
            id: id.into(),
            rect: Rect {
                left,
                top: 0,
                right: left + 1000,
                bottom: 800,
            },
            dpi: 96,
            primary,
            name: String::new(),
        }
    }

    #[test]
    fn height_scales_with_dpi_and_rounds() {
        assert_eq!(bar_height_px(28, 96), 28);
        assert_eq!(bar_height_px(28, 144), 42); // 150 %
        assert_eq!(bar_height_px(28, 192), 56); // 200 %
        assert_eq!(bar_height_px(28, 120), 35); // 125 %
        assert_eq!(bar_height_px(28, 168), 49); // 175 %
        assert_eq!(bar_height_px(25, 144), 38); // 37,5 -> 38
    }

    #[test]
    fn rect_spans_full_monitor_width_from_rc_monitor() {
        let m = Rect {
            left: -1080,
            top: -405,
            right: 0,
            bottom: 1515,
        };
        assert_eq!(
            bar_rect(m, 42),
            Rect {
                left: -1080,
                top: -405,
                right: 0,
                bottom: -363
            }
        );
    }

    #[test]
    fn select_orders_primary_first_then_left_to_right() {
        let r = select(
            vec![
                mon("b", 1000, false),
                mon("a", 0, true),
                mon("c", -1000, false),
            ],
            &MonitorSelection::All,
        );
        assert_eq!(
            r.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["a", "c", "b"]
        );
    }

    #[test]
    fn select_primary_only() {
        let r = select(
            vec![mon("b", 1000, false), mon("a", 0, true)],
            &MonitorSelection::Primary,
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "a");
    }

    #[test]
    fn reconcile_adds_removes_keeps() {
        let ex = vec!["a".to_string(), "b".to_string()];
        let wa = vec!["b".to_string(), "c".to_string()];
        let p = plan_reconcile(&ex, &wa);
        assert_eq!(
            p,
            ReconcilePlan {
                remove: vec!["a".into()],
                keep: vec!["b".into()],
                add: vec!["c".into()]
            }
        );
    }

    #[test]
    fn picks_active_monitor_not_first_attached() {
        // namerane na stroji: mon0 flags=0x2 (len pripojeny), mon1 flags=0x3 (aktivny)
        let e = vec![(0x2, "GSM".to_string()), (0x3, "AOC".to_string())];
        assert_eq!(pick_active_id(&e).as_deref(), Some("AOC"));
        assert_eq!(pick_active_id(&[(0x2, "GSM".into())]), None);
        assert_eq!(pick_active_id(&[]), None);
    }

    #[test]
    fn reconcile_empty_cases() {
        assert_eq!(plan_reconcile(&[], &[]), ReconcilePlan::default());
        let wa = vec!["x".to_string()];
        assert_eq!(plan_reconcile(&[], &wa).add, wa);
        assert_eq!(plan_reconcile(&wa, &[]).remove, wa);
    }

    fn three() -> Vec<MonitorInfo> {
        vec![
            mon("B", 1000, false),
            mon("A", 0, true),
            mon("C", -1000, false),
        ]
    }

    #[test]
    fn selected_monitors_keep_only_those_ids_case_insensitively() {
        let sel = MonitorSelection::Selected(vec!["c".into(), "B".into()]);
        let (m, fb) = select_with_fallback(three(), &sel);
        assert!(!fb);
        assert_eq!(
            m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["C", "B"]
        );
        // jediny vybrany (nie primarny)
        let (m, fb) = select_with_fallback(three(), &MonitorSelection::Selected(vec!["B".into()]));
        assert_eq!((m.len(), m[0].id.as_str(), fb), (1, "B", false));
    }

    #[test]
    fn missing_selection_falls_back_to_primary_with_a_flag_and_never_rewrites_the_selection() {
        let sel = MonitorSelection::Selected(vec!["gone".into(), "other".into()]);
        let (m, fb) = select_with_fallback(three(), &sel);
        assert!(fb, "ziadny vybrany monitor nie je pripojeny");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].id, "A", "zobrazi sa na primarnom, nie na lubovolnom");
        // cast je pripojena: ziadny fallback, len pripojene
        let sel = MonitorSelection::Selected(vec!["gone".into(), "C".into()]);
        let (m, fb) = select_with_fallback(three(), &sel);
        assert_eq!((m.len(), fb), (1, false));
        assert_eq!(
            missing_ids(&three(), &["gone".into(), "c".into()]),
            vec!["gone".to_string()]
        );
        // vyber sa pocas toho nemeni (ziadny mutable pristup)
        assert_eq!(
            sel,
            MonitorSelection::Selected(vec!["gone".into(), "C".into()])
        );
        // ziadny monitor (napr. vsetky odpojene): prazdny vysledok, fallback
        let (m, fb) = select_with_fallback(vec![], &sel);
        assert!(m.is_empty() && fb);
        // povodne volby
        assert_eq!(select(three(), &MonitorSelection::All).len(), 3);
        assert_eq!(select(three(), &MonitorSelection::Primary)[0].id, "A");
    }

    #[test]
    fn hot_plug_dpi_flip_and_return_sequence_keeps_one_bar_per_wanted_monitor() {
        // izolovana simulacia (bez realnej zmeny displejov): A primarny 100 %, B vpravo 100 %
        let ids = |v: &[MonitorInfo]| v.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        let mut b = mon("B", 1000, false);
        let all = |v: Vec<MonitorInfo>| select(v, &MonitorSelection::All);
        let mut bars = ids(&all(vec![mon("A", 0, true), b.clone()]));
        assert_eq!(bars, ["A", "B"]);
        // 1) B sa odpoji: jedna lista ostane, B sa odstrani (nic sa nepridava)
        let wanted = ids(&all(vec![mon("A", 0, true)]));
        let plan = plan_reconcile(&bars, &wanted);
        assert_eq!(
            (plan.remove.as_slice(), plan.keep.as_slice(), plan.add.len()),
            (&["B".to_string()][..], &["A".to_string()][..], 0)
        );
        bars = wanted;
        // 2) B sa vrati s INYM DPI (200 %): lista sa prida a vyska sa prepocita z noveho DPI
        b.dpi = 192;
        let wanted = ids(&all(vec![mon("A", 0, true), b.clone()]));
        let plan = plan_reconcile(&bars, &wanted);
        assert_eq!(
            (plan.remove.len(), plan.add.as_slice()),
            (0, &["B".to_string()][..])
        );
        bars = wanted;
        let h_a = bar_height_px(28, 96);
        let h_b = bar_height_px(28, b.dpi);
        assert_eq!(
            (h_a, h_b),
            (28, 56),
            "zmiesane DPI: kazda lista ma vlastnu vysku"
        );
        assert_eq!(bar_rect(b.rect, h_b).height(), 56);
        // 3) zmena primarneho monitora (B sa stane primarnym): poradie sa zmeni, ziadna lista nezanikne ani sa nezdvoji
        let (mut a2, mut b2) = (mon("A", 0, false), b.clone());
        b2.primary = true;
        a2.primary = false;
        let wanted = ids(&all(vec![a2, b2]));
        assert_eq!(wanted, ["B", "A"]);
        let plan = plan_reconcile(&bars, &wanted);
        assert!(plan.remove.is_empty() && plan.add.is_empty() && plan.keep.len() == 2);
        // 4) vybrany monitor B zmizne a ostane len A: fallback na primarny (s oznamom), vyber sa nemeni; po navrate sa lista vrati
        let sel = MonitorSelection::Selected(vec!["b".into()]);
        let (m, fb) = select_with_fallback(vec![mon("A", 0, true)], &sel);
        assert!((ids(&m), fb) == (vec!["A".to_string()], true));
        let (m, fb) = select_with_fallback(vec![mon("A", 0, true), mon("B", 1000, false)], &sel);
        assert!(
            (ids(&m), fb) == (vec!["B".to_string()], false),
            "ID sa porovnava bez ohladu na velkost pismen"
        );
        // 5) ziadny monitor (napr. spanok/odpojenie vsetkeho): ziadne listy, nepada
        let wanted = ids(&all(vec![]));
        let plan = plan_reconcile(&bars, &wanted);
        assert_eq!(
            (plan.remove.len(), plan.keep.len(), plan.add.len()),
            (2, 0, 0)
        );
    }

    #[test]
    fn primary_change_moves_the_primary_only_selection_but_not_a_selected_id() {
        // primarny sa zmenil z A na C: "Primary" nasleduje primarny, "Selected(A)" ostava na A
        let mut v = three();
        for m in v.iter_mut() {
            m.primary = m.id == "C";
        }
        assert_eq!(select(v.clone(), &MonitorSelection::Primary)[0].id, "C");
        assert_eq!(
            select(v, &MonitorSelection::Selected(vec!["A".into()]))[0].id,
            "A"
        );
    }
}
