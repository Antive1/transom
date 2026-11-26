//! Skutocny zvukovy system cez dokumentovane rozhrania Core Audio (MMDevice API + EndpointVolume API):
//! `IMMDeviceEnumerator` (zoznam, predvoleny vystup), `IMMNotificationClient` (zmeny zariadeni),
//! `IAudioEndpointVolume` + `IAudioEndpointVolumeCallback` (hlasitost a stlmenie predvoleneho vystupu).
//!
//! Pravidla (z dokumentacie Microsoftu): callbacky su neblokujuce a len vlozia udalost do fronty; registracie sa
//! rusia **mimo** callbackov na vlakne sluzby pred uvolnenim rozhrani; COM je MTA na vlastnom vlakne. Predvoleny
//! vystup sa **nikdy nemeni** (nepouziva sa ziadne nedokumentovane rozhranie); hlasitost sa zapisuje len na predvolene
//! zariadenie a s vlastnym kontextom (`OWN_CONTEXT`), aby sa dala rozpoznat ozvena. Chyby sa vracaju ako hodnoty
//! (vydanie ma `panic = "abort"`), ID zariadenia sa nelogue ani nezobrazuje.
use crate::model::{percent_of, scalar_of, AudioError, Controls, DeviceState, RawDevice, Volume};
use crate::service::{Backend, Event, Refresh};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use windows::core::{implement, GUID, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eConsole, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE,
    DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};

/// Kontext udalosti nasich zapisov (ozvena sa v `OnNotify` rozpozna podla neho).
pub const OWN_CONTEXT: GUID = GUID::from_u128(0x5b0c2a7e_91d4_4f63_8a1e_3c7d2f9b6e04);

/// Odosielatel udalosti zdielany s callbackmi (callbacky bezia na cudzich vlaknach).
struct Notifier {
    tx: Mutex<Sender<Event>>,
}

impl Notifier {
    fn send(&self, e: Event) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(e);
        }
    }
}

#[implement(IMMNotificationClient)]
struct DeviceNotif {
    n: Arc<Notifier>,
}

#[allow(non_snake_case)]
impl IMMNotificationClient_Impl for DeviceNotif_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.n.send(Event::Devices);
        Ok(())
    }
    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.n.send(Event::Devices);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.n.send(Event::Devices);
        Ok(())
    }
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        // roly sa zlucuju: zobrazuje sa len predvoleny vystup pre bezne pouzitie (eConsole)
        if flow == eRender && role == eConsole {
            self.n.send(Event::Devices);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _id: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        // len zmena zobrazovaneho nazvu (kluc sa porovnava podla fmtid a pid)
        let k = PKEY_Device_FriendlyName;
        if key.fmtid == k.fmtid && key.pid == k.pid {
            self.n.send(Event::Devices);
        }
        Ok(())
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeNotif {
    n: Arc<Notifier>,
}

#[allow(non_snake_case)]
impl IAudioEndpointVolumeCallback_Impl for VolumeNotif_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if pnotify.is_null() {
            return Ok(());
        }
        // SAFETY: Windows zarucuje platnu strukturu pocas volania; kopirujeme len skalarne polia.
        let (ctx, vol, muted) = unsafe {
            let d = &*pnotify;
            (d.guidEventContext, d.fMasterVolume, d.bMuted.as_bool())
        };
        self.n.send(Event::Volume {
            percent: percent_of(vol),
            muted,
            own: ctx == OWN_CONTEXT,
        });
        Ok(())
    }
}

struct Bound {
    id: String,
    vol: IAudioEndpointVolume,
    cb: IAudioEndpointVolumeCallback,
}

pub struct CoreAudioBackend {
    com: bool,
    notifier: Arc<Notifier>,
    enumerator: Option<IMMDeviceEnumerator>,
    dev_cb: Option<IMMNotificationClient>,
    bound: Option<Bound>,
}

fn classify(e: &windows::core::Error) -> AudioError {
    let c = e.code().0 as u32;
    let msg = format!("{c:#010x}");
    match c {
        // E_NOTFOUND / ERROR_NOT_FOUND: ziadne zariadenie
        0x8007_0490 => AudioError::NoDevice,
        // AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING, RPC_E_DISCONNECTED, RPC_E_SERVER_DIED(_DNE), RPC_E_SERVERFAULT
        0x8889_0004 | 0x8889_0010 | 0x8001_0108 | 0x8001_0007 | 0x8001_0012 | 0x8001_0105 => {
            AudioError::Unavailable(msg)
        }
        _ => AudioError::Failed(msg),
    }
}

fn is_unavailable(e: &AudioError) -> bool {
    matches!(e, AudioError::Unavailable(_))
}

impl CoreAudioBackend {
    /// Vola sa na vlakne sluzby: inicializuje COM (MTA) a pripravi enumerator s registraciou notifikacii zariadeni.
    pub fn new(tx: Sender<Event>) -> CoreAudioBackend {
        // SAFETY: inicializacia COM pre toto (nove) vlakno; parter `CoUninitialize` je v `Drop`.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let mut b = CoreAudioBackend {
            com,
            notifier: Arc::new(Notifier { tx: Mutex::new(tx) }),
            enumerator: None,
            dev_cb: None,
            bound: None,
        };
        let _ = b.ensure_enumerator();
        b
    }

    fn ensure_enumerator(&mut self) -> Result<&IMMDeviceEnumerator, AudioError> {
        if self.enumerator.is_none() {
            // SAFETY: COM volania na vlakne s inicializovanym COM.
            unsafe {
                let en: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| {
                        match classify(&e) {
                            AudioError::Failed(m) => AudioError::Unavailable(m),
                            other => other,
                        }
                    })?;
                let cb: IMMNotificationClient = DeviceNotif {
                    n: Arc::clone(&self.notifier),
                }
                .into();
                en.RegisterEndpointNotificationCallback(&cb)
                    .map_err(|e| classify(&e))?;
                self.dev_cb = Some(cb);
                self.enumerator = Some(en);
            }
        }
        self.enumerator
            .as_ref()
            .ok_or_else(|| AudioError::Unavailable("enumerator".into()))
    }

    fn unbind(&mut self) {
        if let Some(b) = self.bound.take() {
            // SAFETY: odregistrovanie pred uvolnenim rozhrania, mimo callbacku (vlakno sluzby).
            unsafe {
                let _ = b.vol.UnregisterControlChangeNotify(&b.cb);
            }
        }
    }

    /// Zahodi vsetko (po strate sluzby): dalsie nacitanie vytvori enumerator a registracie znova.
    fn reset(&mut self) {
        self.unbind();
        if let (Some(en), Some(cb)) = (self.enumerator.take(), self.dev_cb.take()) {
            // SAFETY: odregistrovanie klienta pred uvolnenim; chyby pri strate sluzby sa ignoruju.
            unsafe {
                let _ = en.UnregisterEndpointNotificationCallback(&cb);
            }
        }
    }

    fn id_of(dev: &IMMDevice) -> Option<String> {
        // SAFETY: GetId vracia retazec vlastneny COM; po skopirovani sa uvolni.
        unsafe {
            let p = dev.GetId().ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as _));
            s
        }
    }

    fn name_of(dev: &IMMDevice) -> String {
        // SAFETY: citanie vlastnosti zariadenia; PROPVARIANT sa uvolni v Drop.
        unsafe {
            let Ok(store) = dev.OpenPropertyStore(STGM_READ) else {
                return String::new();
            };
            let Ok(v) = store.GetValue(&PKEY_Device_FriendlyName) else {
                return String::new();
            };
            windows::core::BSTR::try_from(&v)
                .map(|b| b.to_string())
                .unwrap_or_default()
        }
    }

    fn bind(&mut self, dev: &IMMDevice, id: String) -> Result<(), AudioError> {
        // SAFETY: aktivacia rozhrania hlasitosti pre platne zariadenie; parametre aktivacie musia byt NULL.
        unsafe {
            let vol: IAudioEndpointVolume =
                dev.Activate(CLSCTX_ALL, None).map_err(|e| classify(&e))?;
            let cb: IAudioEndpointVolumeCallback = VolumeNotif {
                n: Arc::clone(&self.notifier),
            }
            .into();
            vol.RegisterControlChangeNotify(&cb)
                .map_err(|e| classify(&e))?;
            self.bound = Some(Bound { id, vol, cb });
        }
        Ok(())
    }

    fn read_volume(&self) -> (Option<Volume>, Controls) {
        let Some(b) = &self.bound else {
            return (None, Controls::default());
        };
        // SAFETY: citanie stavu platneho rozhrania.
        unsafe {
            let v = b.vol.GetMasterVolumeLevelScalar();
            let m = b.vol.GetMute();
            let controls = Controls {
                volume: v.is_ok(),
                mute: m.is_ok(),
            };
            let vol = if v.is_ok() || m.is_ok() {
                Some(Volume {
                    percent: v.map(percent_of).unwrap_or(0),
                    muted: m.map(|b| b.as_bool()).unwrap_or(false),
                })
            } else {
                None
            };
            (vol, controls)
        }
    }

    fn refresh_inner(&mut self) -> Result<Refresh, AudioError> {
        self.ensure_enumerator()?;
        let en = self
            .enumerator
            .clone()
            .ok_or_else(|| AudioError::Unavailable("enumerator".into()))?;
        // SAFETY: citanie zoznamu zariadeni; vsetky rozhrania zijú len pocas tohto volania.
        unsafe {
            let default = en.GetDefaultAudioEndpoint(eRender, eConsole);
            let default_id = match &default {
                Ok(d) => Self::id_of(d),
                Err(e) => match classify(e) {
                    AudioError::NoDevice => None,
                    other => return Err(other),
                },
            };
            let mask = DEVICE_STATE(
                DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0 | DEVICE_STATE_UNPLUGGED.0,
            );
            let coll = en
                .EnumAudioEndpoints(eRender, mask)
                .map_err(|e| classify(&e))?;
            let n = coll.GetCount().map_err(|e| classify(&e))?;
            let mut devices = Vec::new();
            for i in 0..n {
                let Ok(d) = coll.Item(i) else { continue };
                let Some(id) = Self::id_of(&d) else { continue };
                let state = match d.GetState() {
                    Ok(s) if s.0 == DEVICE_STATE_ACTIVE.0 => DeviceState::Active,
                    Ok(s) if s.0 == DEVICE_STATE_DISABLED.0 => DeviceState::Disabled,
                    Ok(_) => DeviceState::Unplugged,
                    Err(_) => continue,
                };
                devices.push(RawDevice {
                    is_default: default_id.as_deref() == Some(id.as_str()),
                    name: Self::name_of(&d),
                    id,
                    state,
                });
            }
            // naviazat hlasitost na (nove) predvolene zariadenie
            match (&default, &default_id) {
                (Ok(d), Some(id)) => {
                    if self.bound.as_ref().map(|b| b.id.as_str()) != Some(id.as_str()) {
                        self.unbind();
                        self.bind(d, id.clone())?;
                    }
                }
                _ => self.unbind(),
            }
            let (volume, controls) = self.read_volume();
            Ok(Refresh {
                devices,
                volume,
                controls,
            })
        }
    }
}

impl Backend for CoreAudioBackend {
    fn refresh(&mut self) -> Result<Refresh, AudioError> {
        let r = self.refresh_inner();
        if let Err(e) = &r {
            if is_unavailable(e) {
                self.reset();
            }
        }
        r
    }

    fn set_volume(&mut self, percent: u8) -> Result<(), AudioError> {
        let Some(b) = &self.bound else {
            return Err(AudioError::NoDevice);
        };
        // SAFETY: zapis hlasitosti na platne rozhranie s vlastnym kontextom; rozsah 0.0..=1.0.
        unsafe {
            b.vol
                .SetMasterVolumeLevelScalar(scalar_of(percent as u32), &OWN_CONTEXT)
                .map_err(|e| match classify(&e) {
                    AudioError::Failed(_) => AudioError::Unsupported,
                    other => other,
                })
        }
    }

    fn set_mute(&mut self, muted: bool) -> Result<(), AudioError> {
        let Some(b) = &self.bound else {
            return Err(AudioError::NoDevice);
        };
        // SAFETY: zapis stlmenia na platne rozhranie s vlastnym kontextom.
        unsafe {
            b.vol
                .SetMute(muted, &OWN_CONTEXT)
                .map_err(|e| match classify(&e) {
                    AudioError::Failed(_) => AudioError::Unsupported,
                    other => other,
                })
        }
    }

    fn shutdown(&mut self) {
        self.reset();
    }
}

impl Drop for CoreAudioBackend {
    fn drop(&mut self) {
        self.reset();
        if self.com {
            // SAFETY: parter k uspesnemu `CoInitializeEx` na tomto vlakne; rozhrania su uz uvolnene.
            unsafe { CoUninitialize() };
        }
    }
}
