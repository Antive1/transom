//! Napajanie: `GetSystemPowerStatus` a notifikacie o zmene (bez dotazovania v slucke).
//!
//! Riadiace okno hostitela sa registruje cez `RegisterPowerSettingNotification` na:
//! `GUID_ACDC_POWER_SOURCE` (siet/bateria), `GUID_BATTERY_PERCENTAGE_REMAINING` (percento) a
//! `GUID_POWER_SAVING_STATUS` (sporic baterie). Prichadzaju ako `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE`;
//! navyse system posiela `PBT_APMPOWERSTATUSCHANGE`. Hostitel z oboch urobi `BarEvent::PowerChanged` a aplikacia
//! si stav precita znova cez [`read`]. Pozn.: system posle jednu notifikaciu hned po registracii (aktualna hodnota).
//! GUID su z `windows` 0.62.2 (`Win32::System::SystemServices`), overene v zdrojoch crate 2026-11-20.
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, RegisterPowerSettingNotification, UnregisterPowerSettingNotification,
    HPOWERNOTIFY, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_POWER_SAVING_STATUS,
};
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE;

/// Surove hodnoty `SYSTEM_POWER_STATUS` (vyznam v `transom_core::battery::PowerRaw`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerStatus {
    pub ac_line: u8,
    pub battery_flag: u8,
    pub percent: u8,
    pub status_flag: u8,
}

/// Aktualny stav napajania; `None`, ak API zlyha.
pub fn read() -> Option<PowerStatus> {
    let mut s = SYSTEM_POWER_STATUS::default();
    // SAFETY: vystupna struktura je lokalna.
    unsafe { GetSystemPowerStatus(&mut s) }.ok()?;
    Some(PowerStatus {
        ac_line: s.ACLineStatus,
        battery_flag: s.BatteryFlag,
        percent: s.BatteryLifePercent,
        status_flag: s.SystemStatusFlag,
    })
}

/// Registracie notifikacii; pri `Drop` sa odregistruju.
pub(crate) struct PowerNotifications(Vec<HPOWERNOTIFY>);

impl PowerNotifications {
    pub(crate) fn register(hwnd: HWND) -> Self {
        let mut v = Vec::new();
        for g in [
            GUID_ACDC_POWER_SOURCE,
            GUID_BATTERY_PERCENTAGE_REMAINING,
            GUID_POWER_SAVING_STATUS,
        ] {
            // SAFETY: registracia na nase okno; handle sa odregistruje v Drop.
            if let Ok(h) = unsafe {
                RegisterPowerSettingNotification(HANDLE(hwnd.0), &g, DEVICE_NOTIFY_WINDOW_HANDLE)
            } {
                v.push(h);
            }
        }
        PowerNotifications(v)
    }
}

impl Drop for PowerNotifications {
    fn drop(&mut self) {
        for h in self.0.drain(..) {
            // SAFETY: handle z RegisterPowerSettingNotification, odregistruje sa raz (drain).
            let _ = unsafe { UnregisterPowerSettingNotification(h) };
        }
    }
}
