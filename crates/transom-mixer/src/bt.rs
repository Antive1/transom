//! Pripojenie a odpojenie sparovanych Bluetooth zvukovych zariadeni (sluchadla, reproduktory) jednym klikom.
//!
//! Rovnaky postup ako nastavenia Windows a nastroj ToothTray (BSD-2, len ako popis postupu, bez prevzateho kodu):
//! vystup (`IMMDevice`, vsetky stavy) -> topologia zariadenia (`IDeviceTopology`, konektory) -> spojeny KS filter
//! Bluetooth ovladaca (ID zacina `\\?\bth`) -> `IKsControl::KsProperty` s dokumentovanou sadou `KSPROPSETID_BtAudio`
//! (`KSPROPERTY_ONESHOT_RECONNECT` / `KSPROPERTY_ONESHOT_DISCONNECT`). Bez administratorskych prav.
//!
//! Funguje len pre zvukove Bluetooth zariadenia (A2DP / Hands-Free), ktore su sparovane. Pripojene = niektory vystup
//! zariadenia je aktivny; vysledok sa vzdy potvrdi az novym citanim stavu.
use crate::core::take_pwstr;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::core::Interface;
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_ContainerId, PKEY_Device_FriendlyName,
};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::KernelStreaming::{
    IKsControl, KSPROPSETID_BtAudio, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0,
    KSPROPERTY_ONESHOT_DISCONNECT, KSPROPERTY_ONESHOT_RECONNECT, KSPROPERTY_TYPE_GET,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

/// Sparovane Bluetooth zvukove zariadenie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtDevice {
    /// ID kontajnera zariadenia (spaja vsetky jeho vystupy).
    pub key: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BtSnap {
    pub ready: bool,
    pub devices: Vec<BtDevice>,
    /// Prave sa pripaja / odpaja (kluc).
    pub busy: Option<String>,
    /// Posledna poziadavka zlyhala alebo stav sa nezmenil (kluc).
    pub failed: Option<String>,
    pub generation: u64,
}

struct Found {
    dev: BtDevice,
    controls: Vec<IKsControl>,
}

/// Odstrani z mena koncovku rozhrania ("Sluchadla (JBL Charge 5)" -> "JBL Charge 5").
pub fn short_name(endpoint: &str) -> String {
    let s = endpoint.trim();
    match (s.find('('), s.rfind(')')) {
        (Some(a), Some(b)) if b > a + 1 && b == s.len() - 1 => s[a + 1..b].trim().to_string(),
        _ => s.to_string(),
    }
}

fn prop_string(
    store: &windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore,
    key: &windows::Win32::Foundation::PROPERTYKEY,
) -> String {
    // SAFETY: citanie vlastnosti zariadenia.
    unsafe {
        store
            .GetValue(key)
            .ok()
            .and_then(|v| windows::core::BSTR::try_from(&v).ok())
            .map(|b| b.to_string())
            .unwrap_or_default()
    }
}

fn container_of(store: &windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore) -> String {
    // SAFETY: citanie GUID kontajnera (VT_CLSID).
    unsafe {
        let Ok(v) = store.GetValue(&PKEY_Device_ContainerId) else {
            return String::new();
        };
        let raw = &v.Anonymous.Anonymous;
        if raw.vt.0 == 72 && !raw.Anonymous.puuid.is_null() {
            format!("{:?}", *(raw.Anonymous.puuid as *const windows::core::GUID))
        } else {
            String::new()
        }
    }
}

fn scan(en: &IMMDeviceEnumerator) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    // SAFETY: enumeracia vystupov vo vsetkych stavoch, topologia a aktivacia KS filtra; len citanie.
    unsafe {
        let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE(DEVICE_STATEMASK_ALL)) else {
            return out;
        };
        for i in 0..col.GetCount().unwrap_or(0) {
            let Ok(dev) = col.Item(i) else { continue };
            let state = dev.GetState().unwrap_or(DEVICE_STATE(0));
            let Ok(store) = dev.OpenPropertyStore(STGM_READ) else {
                continue;
            };
            let Ok(topo) = dev.Activate::<IDeviceTopology>(CLSCTX_ALL, None) else {
                continue;
            };
            let mut controls = Vec::new();
            for c in 0..topo.GetConnectorCount().unwrap_or(0) {
                let Ok(con) = topo.GetConnector(c) else {
                    continue;
                };
                let Ok(other) = con.GetConnectedTo() else {
                    continue;
                };
                let Ok(part) = other.cast::<IPart>() else {
                    continue;
                };
                let Ok(ot) = part.GetTopologyObject() else {
                    continue;
                };
                let id = take_pwstr(ot.GetDeviceId());
                if !id.to_lowercase().contains(r"\\?\bth") {
                    continue;
                }
                let Ok(od) = en.GetDevice(&windows::core::HSTRING::from(id.as_str())) else {
                    continue;
                };
                if let Ok(ks) = od.Activate::<IKsControl>(CLSCTX_ALL, None) {
                    controls.push(ks);
                }
            }
            if controls.is_empty() {
                continue;
            }
            let key = container_of(&store);
            let name = short_name(&prop_string(&store, &PKEY_Device_FriendlyName));
            let connected = state == DEVICE_STATE_ACTIVE;
            match out.iter_mut().find(|f| f.dev.key == key && !key.is_empty()) {
                Some(f) => {
                    f.dev.connected |= connected;
                    f.controls.extend(controls);
                }
                None => out.push(Found {
                    dev: BtDevice {
                        key: if key.is_empty() { name.clone() } else { key },
                        name,
                        connected,
                    },
                    controls,
                }),
            }
        }
    }
    out
}

/// Posle poziadavku vsetkym KS filtrom zariadenia; vrati HRESULT kazdeho volania (0 = prijate).
pub fn oneshot_codes(controls: &[IKsControl], connect: bool) -> Vec<i32> {
    let prop = ks_prop(connect);
    controls
        .iter()
        .map(|c| {
            let mut n = 0u32;
            // SAFETY: vlastnost bez dat (vstup = KSIDENTIFIER, vystup prazdny).
            match unsafe {
                c.KsProperty(
                    &prop,
                    std::mem::size_of::<KSIDENTIFIER>() as u32,
                    std::ptr::null_mut(),
                    0,
                    &mut n,
                )
            } {
                Ok(()) => 0,
                Err(e) => e.code().0,
            }
        })
        .collect()
}

fn ks_prop(connect: bool) -> KSIDENTIFIER {
    KSIDENTIFIER {
        Anonymous: KSIDENTIFIER_0 {
            Anonymous: KSIDENTIFIER_0_0 {
                Set: KSPROPSETID_BtAudio,
                Id: if connect {
                    KSPROPERTY_ONESHOT_RECONNECT.0 as u32
                } else {
                    KSPROPERTY_ONESHOT_DISCONNECT.0 as u32
                },
                Flags: KSPROPERTY_TYPE_GET,
            },
        },
    }
}

/// Diagnostika (priklad `bt`): HRESULT poziadavky pre zariadenie s menom `name`.
pub fn probe(name: &str, connect: bool) -> Option<Vec<i32>> {
    // SAFETY: COM MTA na volajucom vlakne len pre tento test.
    let _com = Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok());
    // SAFETY: enumerator zariadeni.
    let en: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    let f = scan(&en).into_iter().find(|f| f.dev.name == name)?;
    Some(oneshot_codes(&f.controls, connect))
}

fn oneshot(controls: &[IKsControl], connect: bool) -> bool {
    let prop = KSIDENTIFIER {
        Anonymous: KSIDENTIFIER_0 {
            Anonymous: KSIDENTIFIER_0_0 {
                Set: KSPROPSETID_BtAudio,
                Id: if connect {
                    KSPROPERTY_ONESHOT_RECONNECT.0 as u32
                } else {
                    KSPROPERTY_ONESHOT_DISCONNECT.0 as u32
                },
                Flags: KSPROPERTY_TYPE_GET,
            },
        },
    };
    let mut ok = false;
    for c in controls {
        let mut n = 0u32;
        // SAFETY: vlastnost bez dat (vstup = KSIDENTIFIER, vystup prazdny).
        ok |= unsafe {
            c.KsProperty(
                &prop,
                std::mem::size_of::<KSIDENTIFIER>() as u32,
                std::ptr::null_mut(),
                0,
                &mut n,
            )
        }
        .is_ok();
    }
    ok
}

struct Com(bool);
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: parter k CoInitializeEx.
            unsafe { CoUninitialize() };
        }
    }
}

enum Msg {
    Refresh,
    Set(String, bool),
}

/// Sluzba Bluetooth zvuku: zoznam pri starte a obnoveni (bez pravidelneho citania), pripojenie/odpojenie s overenim.
pub struct BtService {
    tx: Sender<Msg>,
    snap: Arc<Mutex<Arc<BtSnap>>>,
}

impl BtService {
    pub fn start(notify: Arc<dyn Fn() + Send + Sync>) -> Result<BtService, String> {
        let (tx, rx) = channel::<Msg>();
        let snap = Arc::new(Mutex::new(Arc::new(BtSnap::default())));
        let s2 = Arc::clone(&snap);
        std::thread::Builder::new()
            .name("transom-bt-audio".into())
            .spawn(move || {
                // SAFETY: COM MTA na tomto vlakne.
                let _com = Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok());
                // SAFETY: enumerator zariadeni.
                let Ok(en) = (unsafe {
                    CoCreateInstance::<_, IMMDeviceEnumerator>(
                        &MMDeviceEnumerator,
                        None,
                        CLSCTX_ALL,
                    )
                }) else {
                    return;
                };
                let mut gen = 0u64;
                let mut publish =
                    |devices: Vec<BtDevice>, busy: Option<String>, failed: Option<String>| {
                        gen += 1;
                        if let Ok(mut g) = s2.lock() {
                            *g = Arc::new(BtSnap {
                                ready: true,
                                devices,
                                busy,
                                failed,
                                generation: gen,
                            });
                        }
                        notify();
                    };
                let mut failed: Option<String> = None;
                let mut found = scan(&en);
                publish(found.iter().map(|f| f.dev.clone()).collect(), None, None);
                while let Ok(m) = rx.recv() {
                    match m {
                        Msg::Refresh => {}
                        Msg::Set(key, connect) => {
                            let Some(f) = found.iter().find(|f| f.dev.key == key) else {
                                continue;
                            };
                            publish(
                                found.iter().map(|f| f.dev.clone()).collect(),
                                Some(key.clone()),
                                None,
                            );
                            let sent = oneshot(&f.controls, connect);
                            // overenie: stav vystupu sa zmeni az po chvili (pripojenie trva aj niekolko sekund)
                            let mut ok = false;
                            if sent {
                                for _ in 0..40 {
                                    match rx.recv_timeout(Duration::from_millis(250)) {
                                        Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                                        Err(RecvTimeoutError::Disconnected) => return,
                                    }
                                    found = scan(&en);
                                    if found
                                        .iter()
                                        .any(|f| f.dev.key == key && f.dev.connected == connect)
                                    {
                                        ok = true;
                                        break;
                                    }
                                }
                            }
                            failed = (!ok).then_some(key);
                        }
                    }
                    found = scan(&en);
                    publish(
                        found.iter().map(|f| f.dev.clone()).collect(),
                        None,
                        failed.clone(),
                    );
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(BtService { tx, snap })
    }

    pub fn snapshot(&self) -> Arc<BtSnap> {
        self.snap.lock().map(|g| Arc::clone(&g)).unwrap_or_default()
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    /// Pripoji (`true`) alebo odpoji zariadenie; vysledok potvrdi nove citanie (najviac ~10 s).
    pub fn set(&self, key: &str, connect: bool) {
        let _ = self.tx.send(Msg::Set(key.to_string(), connect));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_names_are_shortened_to_the_device() {
        assert_eq!(short_name("Speakers (JBL Charge 5)"), "JBL Charge 5");
        assert_eq!(
            short_name("Headset (WH-1000XM4 Hands-Free)"),
            "WH-1000XM4 Hands-Free"
        );
        assert_eq!(short_name("JBL"), "JBL");
        assert_eq!(short_name("A (B) C"), "A (B) C");
    }
}
