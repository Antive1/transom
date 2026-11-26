//! Tenka Win32 vrstva nad HID (len Windows): zoznam rozhrani, feature reporty a zapis/citanie s casovym limitom.
//!
//! Vsetky volania su blokujuce s kratkym limitom a bezia len na vlakne sluzby. Rozhrania sa otvaraju so zdielanim
//! citania aj zapisu, takze sa nebija s aplikaciami vyrobcov (Synapse, G HUB, NuPhyIO).
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid,
    HidD_GetInputReport, HidD_GetPreparsedData, HidD_GetProductString, HidD_SetFeature,
    HidD_SetNumInputBuffers, HidP_Feature, HidP_GetCaps, HidP_GetUsageValue, HidP_GetValueCaps,
    HidP_Input, HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_REPORT_TYPE, HIDP_VALUE_CAPS,
    PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

const HIDP_STATUS_SUCCESS: i32 = 0x0011_0000;

/// Popis jedneho HID rozhrania (kolekcie).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HidInfo {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
    pub product: String,
}

impl HidInfo {
    /// Cislo rozhrania USB z cesty (`&mi_01`), ak ho cesta obsahuje.
    pub fn interface(&self) -> Option<u8> {
        let p = self.path.to_ascii_lowercase();
        let i = p.find("&mi_")?;
        u8::from_str_radix(p.get(i + 4..i + 6)?, 16).ok()
    }
}

struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: handle vlastnime a zatvara sa prave raz
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_raw(path: &str, access: u32, flags: FILE_FLAGS_AND_ATTRIBUTES) -> Option<Owned> {
    let w = wide(path);
    // SAFETY: cesta je platny retazec ukonceny nulou; handle prevezme `Owned`
    let h = unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .ok()?;
    Some(Owned(h))
}

/// Vlastnosti rozhrania z handle (aj bez prava citat/zapisovat).
fn describe(path: &str, h: HANDLE) -> Option<HidInfo> {
    let mut attrs = HIDD_ATTRIBUTES {
        Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32,
        ..Default::default()
    };
    // SAFETY: platny handle HID zariadenia a struktura so spravnou velkostou
    if !unsafe { HidD_GetAttributes(h, &mut attrs) } {
        return None;
    }
    let mut pre = PHIDP_PREPARSED_DATA::default();
    // SAFETY: vystupny ukazovatel na lokalnu premennu; uvolni sa nizsie
    if !unsafe { HidD_GetPreparsedData(h, &mut pre) } {
        return None;
    }
    let mut caps = HIDP_CAPS::default();
    // SAFETY: `pre` je platny az do HidD_FreePreparsedData
    let ok = unsafe { HidP_GetCaps(pre, &mut caps) }.0 == HIDP_STATUS_SUCCESS;
    // SAFETY: uvolnenie dat z HidD_GetPreparsedData
    unsafe {
        let _ = HidD_FreePreparsedData(pre);
    }
    if !ok {
        return None;
    }
    let mut buf = [0u16; 128];
    // SAFETY: buffer s danou velkostou v bajtoch
    let product = if unsafe {
        HidD_GetProductString(
            h,
            buf.as_mut_ptr().cast(),
            std::mem::size_of_val(&buf) as u32,
        )
    } {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n]).trim().to_string()
    } else {
        String::new()
    };
    Some(HidInfo {
        path: path.to_string(),
        vid: attrs.VendorID,
        pid: attrs.ProductID,
        usage_page: caps.UsagePage,
        usage: caps.Usage,
        input_len: caps.InputReportByteLength,
        output_len: caps.OutputReportByteLength,
        feature_len: caps.FeatureReportByteLength,
        product,
    })
}

/// Vsetky pritomne HID rozhrania danych vyrobcov (`vids`). Cesty sa filtruju este pred otvorenim, takze sa nikdy
/// neotvara zariadenie ineho vyrobcu.
pub fn enumerate(vids: &[u16]) -> Vec<HidInfo> {
    enumerate_where(Some(vids))
}

/// Vsetky pritomne HID rozhrania (pre standardnu uroven baterie). Otvaraju sa bez prava citat/zapisovat - len na
/// zistenie vlastnosti.
pub fn enumerate_all() -> Vec<HidInfo> {
    enumerate_where(None)
}

fn enumerate_where(vids: Option<&[u16]>) -> Vec<HidInfo> {
    let mut out = Vec::new();
    let vids_f = vids.unwrap_or(&[]);
    let needles: Vec<String> = vids_f.iter().map(|v| format!("vid_{v:04x}")).collect();
    // SAFETY: GUID triedy HID; zoznam sa uvolni na konci
    let guid = unsafe { HidD_GetHidGuid() };
    // SAFETY: HidD_GetHidGuid nema predpoklady.
    let Ok(set) = (unsafe {
        SetupDiGetClassDevsW(
            Some(&guid),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }) else {
        return out;
    };
    for index in 0..4096u32 {
        let mut item = SP_DEVICE_INTERFACE_DATA {
            cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: platny zoznam a struktura so spravnou velkostou
        if unsafe { SetupDiEnumDeviceInterfaces(set, None, &guid, index, &mut item) }.is_err() {
            break;
        }
        let mut needed = 0u32;
        // SAFETY: prvy dotaz len na potrebnu velkost (ocakavana chyba "maly buffer")
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(set, &item, None, 0, Some(&mut needed), None)
        };
        if !(8..=65_536).contains(&needed) {
            continue;
        }
        // zarovnany buffer (u32) pre strukturu s premenlivou dlzkou
        let mut storage = vec![0u32; (needed as usize).div_ceil(4)];
        let detail = storage
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        // SAFETY: buffer ma aspon `needed` bajtov; cbSize je velkost pevnej casti struktury
        unsafe {
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        }
        // SAFETY: rovnaky zoznam a polozka, buffer s potrebnou velkostou
        if unsafe { SetupDiGetDeviceInterfaceDetailW(set, &item, Some(detail), needed, None, None) }
            .is_err()
        {
            continue;
        }
        // SAFETY: DevicePath je retazec ukonceny nulou vnutri bufferu
        let path = unsafe {
            let p = std::ptr::addr_of!((*detail).DevicePath).cast::<u16>();
            let max = (needed as usize - 4) / 2;
            let n = (0..max).position(|i| *p.add(i) == 0).unwrap_or(max);
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        };
        let lower = path.to_ascii_lowercase();
        if vids.is_some() && !needles.iter().any(|n| lower.contains(n.as_str())) {
            continue;
        }
        if let Some(h) = open_raw(&path, 0, FILE_FLAGS_AND_ATTRIBUTES(0)) {
            if let Some(info) = describe(&path, h.0) {
                if vids.is_none_or(|v| v.contains(&info.vid)) {
                    out.push(info);
                }
            }
        }
    }
    // SAFETY: zoznam z SetupDiGetClassDevsW
    unsafe {
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    out
}

/// Kde rozhranie hlasi standardnu uroven baterie (z popisu reportov).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryCap {
    /// Feature (true) alebo input report.
    pub feature: bool,
    pub report_id: u8,
    pub page: u16,
    pub usage: u16,
    pub link: u16,
    pub logical_min: i32,
    pub logical_max: i32,
    pub report_len: u16,
}

/// Popis reportov rozhrania (predspracovane data) pocas volania `f`.
fn with_preparsed<T>(
    path: &str,
    f: impl FnOnce(HANDLE, PHIDP_PREPARSED_DATA) -> Option<T>,
) -> Option<T> {
    let h = open_raw(path, 0, FILE_FLAGS_AND_ATTRIBUTES(0))?;
    let mut pre = PHIDP_PREPARSED_DATA::default();
    // SAFETY: platny handle; data sa uvolnia nizsie
    if !unsafe { HidD_GetPreparsedData(h.0, &mut pre) } {
        return None;
    }
    let out = f(h.0, pre);
    // SAFETY: uvolnenie dat z HidD_GetPreparsedData
    unsafe {
        let _ = HidD_FreePreparsedData(pre);
    }
    out
}

/// Standardna hodnota baterie rozhrania (Battery Strength / Absolute State Of Charge), ak ju popis obsahuje.
pub fn battery_cap(path: &str) -> Option<BatteryCap> {
    with_preparsed(path, |_, pre| {
        let mut caps = HIDP_CAPS::default();
        // SAFETY: `pre` platne pocas volania
        if unsafe { HidP_GetCaps(pre, &mut caps) }.0 != HIDP_STATUS_SUCCESS {
            return None;
        }
        let kinds: [(HIDP_REPORT_TYPE, u16, u16, bool); 2] = [
            (
                HidP_Feature,
                caps.NumberFeatureValueCaps,
                caps.FeatureReportByteLength,
                true,
            ),
            (
                HidP_Input,
                caps.NumberInputValueCaps,
                caps.InputReportByteLength,
                false,
            ),
        ];
        for (ty, n, len, feature) in kinds {
            if n == 0 || len == 0 {
                continue;
            }
            let mut vc = vec![HIDP_VALUE_CAPS::default(); usize::from(n)];
            let mut count = n;
            // SAFETY: buffer pre `n` poloziek, `count` vstup/vystup
            if unsafe { HidP_GetValueCaps(ty, vc.as_mut_ptr(), &mut count, pre) }.0
                != HIDP_STATUS_SUCCESS
            {
                continue;
            }
            for c in vc.iter().take(usize::from(count)) {
                // SAFETY: citame tu cast unie, ktoru urcuje IsRange
                let (lo, hi) = unsafe {
                    if c.IsRange {
                        (c.Anonymous.Range.UsageMin, c.Anonymous.Range.UsageMax)
                    } else {
                        (c.Anonymous.NotRange.Usage, c.Anonymous.NotRange.Usage)
                    }
                };
                let hit = (lo..=hi).find(|u| crate::generic::is_battery_usage(c.UsagePage, *u));
                if let Some(usage) = hit {
                    return Some(BatteryCap {
                        feature,
                        report_id: c.ReportID,
                        page: c.UsagePage,
                        usage,
                        link: c.LinkCollection,
                        logical_min: c.LogicalMin,
                        logical_max: c.LogicalMax,
                        report_len: len,
                    });
                }
            }
        }
        None
    })
}

/// Surova hodnota baterie podla `cap` (feature alebo input report vyziadany od zariadenia).
pub fn read_battery(path: &str, cap: &BatteryCap) -> Option<i64> {
    with_preparsed(path, |h, pre| {
        let mut buf = vec![0u8; usize::from(cap.report_len)];
        buf[0] = cap.report_id;
        // SAFETY: zapisovatelny buffer s danou dlzkou, platny handle
        let ok = unsafe {
            if cap.feature {
                HidD_GetFeature(h, buf.as_mut_ptr().cast(), buf.len() as u32)
            } else {
                HidD_GetInputReport(h, buf.as_mut_ptr().cast(), buf.len() as u32)
            }
        };
        if !ok {
            return None;
        }
        let ty = if cap.feature {
            HidP_Feature
        } else {
            HidP_Input
        };
        let mut v = 0u32;
        // SAFETY: report od zariadenia a jeho popis
        let st = unsafe {
            HidP_GetUsageValue(ty, cap.page, Some(cap.link), cap.usage, &mut v, pre, &buf)
        };
        (st.0 == HIDP_STATUS_SUCCESS).then_some(i64::from(v))
    })
}

/// Handle len na feature reporty (bez prava citat/zapisovat: mys a klavesnica sa inak otvorit nedaju).
pub struct FeatureDevice(Owned);

impl FeatureDevice {
    pub fn open(path: &str) -> Option<FeatureDevice> {
        open_raw(path, 0, FILE_FLAGS_AND_ATTRIBUTES(0)).map(FeatureDevice)
    }

    /// `buf[0]` je cislo reportu.
    pub fn set_feature(&self, buf: &[u8]) -> bool {
        // SAFETY: buffer s danou dlzkou, platny handle
        unsafe { HidD_SetFeature(self.0 .0, buf.as_ptr().cast(), buf.len() as u32) }
    }

    /// `buf[0]` musi obsahovat cislo reportu.
    pub fn get_feature(&self, buf: &mut [u8]) -> bool {
        // SAFETY: zapisovatelny buffer s danou dlzkou, platny handle
        unsafe { HidD_GetFeature(self.0 .0, buf.as_mut_ptr().cast(), buf.len() as u32) }
    }
}

/// Handle na vystupne a vstupne reporty (prekryvane I/O s casovym limitom).
pub struct IoDevice {
    h: Owned,
    event: Owned,
}

impl IoDevice {
    pub fn open(path: &str, write: bool) -> Option<IoDevice> {
        let access = if write {
            GENERIC_READ.0 | GENERIC_WRITE.0
        } else {
            GENERIC_READ.0
        };
        let h = open_raw(path, access, FILE_FLAG_OVERLAPPED)?;
        // SAFETY: rucne nulovana udalost, vlastni ju `Owned`
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?;
        Some(IoDevice {
            h,
            event: Owned(event),
        })
    }

    fn finish(&self, ov: &mut OVERLAPPED, started: bool, timeout: Duration) -> Option<usize> {
        let mut n = 0u32;
        if !started {
            // SAFETY: citanie poslednej chyby hned po volani
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return None;
            }
            // SAFETY: cakanie na vlastnu udalost
            let w = unsafe { WaitForSingleObject(self.event.0, timeout.as_millis() as u32) };
            if w != WAIT_OBJECT_0 {
                // SAFETY: zrusenie a docakanie nasej operacie, aby buffer nebol pouzity po navrate
                unsafe {
                    let _ = CancelIoEx(self.h.0, Some(ov));
                    let _ = GetOverlappedResult(self.h.0, ov, &mut n, true);
                }
                return None;
            }
        }
        // SAFETY: operacia skoncila (alebo skoncila hned), vysledok je platny
        unsafe { GetOverlappedResult(self.h.0, ov, &mut n, false) }.ok()?;
        Some(n as usize)
    }

    /// Kolko vstupnych reportov drzi Windows pre tento handle (starsie sa pri preteceni zahodia).
    pub fn set_input_buffers(&self, n: u32) {
        // SAFETY: platny handle HID zariadenia
        unsafe {
            let _ = HidD_SetNumInputBuffers(self.h.0, n);
        }
    }

    /// Zapise vystupny report (`buf[0]` = cislo reportu).
    pub fn write(&self, buf: &[u8], timeout: Duration) -> bool {
        let mut ov = OVERLAPPED {
            hEvent: self.event.0,
            ..Default::default()
        };
        // SAFETY: udalost a OVERLAPPED ziju az do konca `finish`
        unsafe {
            let _ = ResetEvent(self.event.0);
        }
        // SAFETY: buffer zije az do skoncenia operacie (finish ju docaka alebo zrusi)
        let started = unsafe { WriteFile(self.h.0, Some(buf), None, Some(&mut ov)) }.is_ok();
        self.finish(&mut ov, started, timeout) == Some(buf.len())
    }

    /// Precita jeden vstupny report; `None` = nic neprislo v limite.
    pub fn read(&self, buf: &mut [u8], timeout: Duration) -> Option<usize> {
        let mut ov = OVERLAPPED {
            hEvent: self.event.0,
            ..Default::default()
        };
        // SAFETY: ako pri zapise
        unsafe {
            let _ = ResetEvent(self.event.0);
        }
        // SAFETY: buffer zije az do skoncenia operacie (finish ju docaka alebo zrusi)
        let started = unsafe { ReadFile(self.h.0, Some(buf), None, Some(&mut ov)) }.is_ok();
        self.finish(&mut ov, started, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_number_from_path() {
        let mut i = HidInfo {
            path: r"\\?\hid#vid_19f5&pid_8f01&mi_01#c&15b18173&0&0000#{4d1e55b2}".into(),
            ..Default::default()
        };
        assert_eq!(i.interface(), Some(1));
        i.path = r"\\?\hid#vid_046d&pid_c548&mi_02&col02#x".into();
        assert_eq!(i.interface(), Some(2));
        i.path = r"\\?\hid#vid_1532&pid_00bf#x".into();
        assert_eq!(i.interface(), None);
    }
}
