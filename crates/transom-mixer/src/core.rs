//! Skutocny mixer cez Core Audio: relacie na **kazdom** aktivnom vystupe (`IAudioSessionManager2`), hlasitost a stlmenie
//! relacie (`ISimpleAudioVolume`), notifikacie novych relacii, zmien relacii a zariadeni.
//!
//! Vystup pre aplikaciu: Windows na to nema dokumentovane API. Pouziva sa to iste nedokumentovane rozhranie ako
//! EarTrumpet a nastavenia Windows (`Windows.Media.Internal.AudioPolicyConfig`, `SetPersistedDefaultAudioEndpoint`).
//! Ak sa ho nepodari ziskat (iny build Windows), `routing = false` a panel ponukne len nastavenia Windows. Nastavenie je
//! trvale ulozene v systeme (rovnako ako vo Windows nastaveniach) a "predvoleny" ho zrusi.
//!
//! Pravidla: callbacky su neblokujuce (len poslu udalost), registracie sa rusia na vlakne sluzby (nikdy v callbacku),
//! COM je MTA na vlastnom vlakne. ID zariadeni, cesty a nazvy procesov sa nelogu.
use crate::model::{pack_device_id, unpack_route, OutDevice, RawSession, Route};
use crate::service::{Backend, Reading};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use windows::core::{implement, interface, Interface, GUID, HRESULT, HSTRING, PCWSTR, PWSTR};
use windows::core::{IUnknown, IUnknown_Vtbl};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY};
use windows::Win32::Media::Audio::*;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::WinRT::RoGetActivationFactory;

/// Kontext nasich zapisov hlasitosti relacii.
pub const OWN_CONTEXT: GUID = GUID::from_u128(0x0f4e1b7c_3a62_4d8e_9b51_7c2a6e0d3f15);

const POLICY_CLASS: &str = "Windows.Media.Internal.AudioPolicyConfig";

/// Rozhranie politiky (IInspectable: 3 metody IUnknown + 3 IInspectable, 19 inych metod, potom 3 pouzite). Poradie podla
/// EarTrumpet (posledna metoda, zmazanie vsetkych nastaveni, sa zamerne nedeklaruje); od buildu 21390 ma ine IID nez starsie Windows 10, vtable je rovnaka.
macro_rules! policy_iface {
    ($name:ident, $iid:literal) => {
        #[interface($iid)]
        unsafe trait $name: IUnknown {
            fn inspectable0(&self) -> HRESULT;
            fn inspectable1(&self) -> HRESULT;
            fn inspectable2(&self) -> HRESULT;
            fn r0(&self) -> HRESULT;
            fn r1(&self) -> HRESULT;
            fn r2(&self) -> HRESULT;
            fn r3(&self) -> HRESULT;
            fn r4(&self) -> HRESULT;
            fn r5(&self) -> HRESULT;
            fn r6(&self) -> HRESULT;
            fn r7(&self) -> HRESULT;
            fn r8(&self) -> HRESULT;
            fn r9(&self) -> HRESULT;
            fn r10(&self) -> HRESULT;
            fn r11(&self) -> HRESULT;
            fn r12(&self) -> HRESULT;
            fn r13(&self) -> HRESULT;
            fn r14(&self) -> HRESULT;
            fn r15(&self) -> HRESULT;
            fn r16(&self) -> HRESULT;
            fn r17(&self) -> HRESULT;
            fn r18(&self) -> HRESULT;
            fn set_persisted(
                &self,
                pid: u32,
                flow: EDataFlow,
                role: ERole,
                id: *mut c_void,
            ) -> HRESULT;
            fn get_persisted(
                &self,
                pid: u32,
                flow: EDataFlow,
                role: ERole,
                id: *mut *mut c_void,
            ) -> HRESULT;
        }
    };
}
policy_iface!(IPolicyNew, "ab3d4648-e242-459f-b02f-541c70306324");
policy_iface!(IPolicyOld, "2a59116d-6c4f-45e0-a74f-707e3fef9258");

enum Policy {
    New(IPolicyNew),
    Old(IPolicyOld),
}

const _: () = assert!(std::mem::size_of::<HSTRING>() == std::mem::size_of::<*mut c_void>());

impl Policy {
    fn get() -> Option<Policy> {
        let name = HSTRING::from(POLICY_CLASS);
        // SAFETY: aktivacia tovarne WinRT podla mena triedy a IID; pri zlyhani sa rozhranie nepouzije.
        unsafe {
            if let Ok(f) = RoGetActivationFactory::<IPolicyNew>(&name) {
                return Some(Policy::New(f));
            }
            RoGetActivationFactory::<IPolicyOld>(&name)
                .ok()
                .map(Policy::Old)
        }
    }

    fn set(&self, pid: u32, role: ERole, id: *mut c_void) -> HRESULT {
        // SAFETY: `id` je platny HSTRING (alebo null) pocas volania; vtable podla overeneho poradia.
        unsafe {
            match self {
                Policy::New(f) => f.set_persisted(pid, eRender, role, id),
                Policy::Old(f) => f.set_persisted(pid, eRender, role, id),
            }
        }
    }

    fn route(&self, pid: u32) -> Route {
        let mut out: *mut c_void = std::ptr::null_mut();
        // SAFETY: vystupny HSTRING patri nam; prevezme ho `HSTRING` (uvolni sa pri drop).
        unsafe {
            let hr = match self {
                Policy::New(f) => f.get_persisted(pid, eRender, eMultimedia, &mut out),
                Policy::Old(f) => f.get_persisted(pid, eRender, eMultimedia, &mut out),
            };
            if hr.is_err() || out.is_null() {
                return Route::Default;
            }
            let h: HSTRING = std::mem::transmute::<*mut c_void, HSTRING>(out);
            unpack_route(&h.to_string_lossy())
        }
    }

    /// Nastavi (alebo zrusi pri `None`) vystup procesu pre obe roly, ako nastavenia Windows.
    fn set_route(&self, pid: u32, device: Option<&str>) -> bool {
        let h = device.map(|d| HSTRING::from(pack_device_id(d)));
        let raw = h.as_ref().map_or(std::ptr::null_mut(), |h| {
            // SAFETY: HSTRING je jeden ukazovatel (overene vyssie); `h` zije az do konca funkcie.
            unsafe { std::mem::transmute_copy::<HSTRING, *mut c_void>(h) }
        });
        let a = self.set(pid, eMultimedia, raw);
        let b = self.set(pid, eConsole, raw);
        a.is_ok() && b.is_ok()
    }
}

struct Notifier(Mutex<Sender<()>>);

impl Notifier {
    fn ping(&self) {
        if let Ok(t) = self.0.lock() {
            let _ = t.send(());
        }
    }
}

#[implement(IMMNotificationClient)]
struct DeviceNotif(Arc<Notifier>);

#[allow(non_snake_case)]
impl IMMNotificationClient_Impl for DeviceNotif_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
    fn OnDefaultDeviceChanged(
        &self,
        _: EDataFlow,
        _: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        let k = PKEY_Device_FriendlyName;
        if key.fmtid == k.fmtid && key.pid == k.pid {
            self.0.ping();
        }
        Ok(())
    }
}

#[implement(IAudioSessionNotification)]
struct NewSessionNotif(Arc<Notifier>);

#[allow(non_snake_case)]
impl IAudioSessionNotification_Impl for NewSessionNotif_Impl {
    fn OnSessionCreated(
        &self,
        _: windows::core::Ref<IAudioSessionControl>,
    ) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
struct SessionNotif(Arc<Notifier>);

#[allow(non_snake_case)]
impl IAudioSessionEvents_Impl for SessionNotif_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnSimpleVolumeChanged(
        &self,
        _: f32,
        _: windows::core::BOOL,
        ctx: *const GUID,
    ) -> windows::core::Result<()> {
        // ozvena nasho zapisu: snimka uz je upravena, netreba citat znova
        // SAFETY: Windows odovzdava platny ukazovatel alebo null.
        if ctx.is_null() || unsafe { *ctx } != OWN_CONTEXT {
            self.0.ping();
        }
        Ok(())
    }
    fn OnChannelVolumeChanged(
        &self,
        _: u32,
        _: *const f32,
        _: u32,
        _: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnStateChanged(&self, _: AudioSessionState) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        self.0.ping();
        Ok(())
    }
}

struct DeviceEntry {
    mgr: IAudioSessionManager2,
    notif: IAudioSessionNotification,
}

struct SessionEntry {
    ctl: IAudioSessionControl,
    events: IAudioSessionEvents,
    vol: ISimpleAudioVolume,
    pid: u32,
    system: bool,
}

pub struct CoreBackend {
    com: bool,
    n: Arc<Notifier>,
    en: Option<IMMDeviceEnumerator>,
    dev_notif: Option<IMMNotificationClient>,
    devices: HashMap<String, DeviceEntry>,
    /// Relacie podla (ID zariadenia, ID instancie relacie).
    sessions: HashMap<(String, String), SessionEntry>,
    policy: Option<Policy>,
    /// Popis .exe podla cesty (citanie verzie suboru je pomalsie, nemeni sa).
    desc: HashMap<String, Option<String>>,
}

pub(crate) fn take_pwstr(p: windows::core::Result<PWSTR>) -> String {
    match p {
        Ok(p) if !p.is_null() => {
            // SAFETY: retazec alokoval COM; po skopirovani sa uvolni.
            let s = unsafe { p.to_string() }.unwrap_or_default();
            // SAFETY: `p` alokoval COM, retazec je uz skopirovany; uvolni sa raz.
            unsafe { CoTaskMemFree(Some(p.0 as _)) };
            s
        }
        _ => String::new(),
    }
}

pub(crate) fn exe_of(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: bezne volania Win32; handle sa zatvori, buffer ma zadanu velkost.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n)
            .is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// Popis suboru (FileDescription) z verzie .exe.
pub(crate) fn description_of(path: &str) -> Option<String> {
    let wide = HSTRING::from(path);
    // SAFETY: bloky verzie maju velkost od systemu; ukazovatele z VerQueryValueW ukazuju do `data`, ktore zije.
    unsafe {
        let size = GetFileVersionInfoSizeW(&wide, None);
        if size == 0 || size > 4 * 1024 * 1024 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(&wide, None, size, data.as_mut_ptr() as _).ok()?;
        let mut langs: Vec<String> = Vec::new();
        let mut p: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if VerQueryValueW(
            data.as_ptr() as _,
            &HSTRING::from(r"\VarFileInfo\Translation"),
            &mut p,
            &mut len,
        )
        .as_bool()
            && !p.is_null()
        {
            let pairs = std::slice::from_raw_parts(p as *const u16, (len / 2) as usize);
            for c in pairs.chunks(2).filter(|c| c.len() == 2) {
                langs.push(format!("{:04x}{:04x}", c[0], c[1]));
            }
        }
        langs.push("040904b0".into());
        for l in langs {
            let q = HSTRING::from(format!(r"\StringFileInfo\{l}\FileDescription"));
            let mut p: *mut c_void = std::ptr::null_mut();
            let mut len = 0u32;
            if VerQueryValueW(data.as_ptr() as _, &q, &mut p, &mut len).as_bool()
                && !p.is_null()
                && len > 1
            {
                let s = std::slice::from_raw_parts(p as *const u16, len as usize);
                let s = String::from_utf16_lossy(s);
                let s = s.trim_end_matches('\0').trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        None
    }
}

fn friendly_name(dev: &IMMDevice) -> String {
    // SAFETY: citanie vlastnosti zariadenia.
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

impl CoreBackend {
    pub fn new(tx: Sender<()>) -> CoreBackend {
        // SAFETY: COM MTA na vlakne sluzby (parovy CoUninitialize v `shutdown`).
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let n = Arc::new(Notifier(Mutex::new(tx)));
        // SAFETY: vytvorenie enumeratora a registracia notifikacii zariadeni.
        let en: Option<IMMDeviceEnumerator> =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok();
        let dev_notif: Option<IMMNotificationClient> = en.as_ref().and_then(|en| {
            let c: IMMNotificationClient = DeviceNotif(Arc::clone(&n)).into();
            // SAFETY: `c` je platny COM objekt, odregistruje sa pri ukonceni sluzby.
            unsafe { en.RegisterEndpointNotificationCallback(&c) }
                .ok()
                .map(|_| c)
        });
        CoreBackend {
            com,
            n,
            en,
            dev_notif,
            devices: HashMap::new(),
            sessions: HashMap::new(),
            policy: Policy::get(),
            desc: HashMap::new(),
        }
    }

    /// Aktivne vystupy; spravcovia relacii sa vytvoria pre nove a zrusia pre zmiznute.
    fn sync_devices(&mut self) -> Vec<OutDevice> {
        let mut out = Vec::new();
        let Some(en) = &self.en else { return out };
        // SAFETY: citanie zoznamu aktivnych vystupov a aktivacia spravcu relacii.
        unsafe {
            let def = en
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .ok()
                .map(|d| take_pwstr(d.GetId()));
            let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
                return out;
            };
            let mut seen = Vec::new();
            for i in 0..col.GetCount().unwrap_or(0) {
                let Ok(dev) = col.Item(i) else { continue };
                let id = take_pwstr(dev.GetId());
                if id.is_empty() {
                    continue;
                }
                if !self.devices.contains_key(&id) {
                    if let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) {
                        let notif: IAudioSessionNotification =
                            NewSessionNotif(Arc::clone(&self.n)).into();
                        if mgr.RegisterSessionNotification(&notif).is_ok() {
                            self.devices.insert(id.clone(), DeviceEntry { mgr, notif });
                        }
                    }
                }
                out.push(OutDevice {
                    name: friendly_name(&dev),
                    is_default: def.as_deref() == Some(id.as_str()),
                    id: id.clone(),
                });
                seen.push(id);
            }
            let gone: Vec<String> = self
                .devices
                .keys()
                .filter(|k| !seen.contains(k))
                .cloned()
                .collect();
            for id in gone {
                if let Some(d) = self.devices.remove(&id) {
                    let _ = d.mgr.UnregisterSessionNotification(&d.notif);
                }
                self.drop_sessions(|(dev, _)| dev == &id);
            }
        }
        out
    }

    fn drop_sessions(&mut self, which: impl Fn(&(String, String)) -> bool) {
        let keys: Vec<(String, String)> =
            self.sessions.keys().filter(|k| which(k)).cloned().collect();
        for k in keys {
            if let Some(s) = self.sessions.remove(&k) {
                // SAFETY: zrusenie registracie mimo callbacku.
                let _ = unsafe { s.ctl.UnregisterAudioSessionNotification(&s.events) };
            }
        }
    }

    fn sessions_for<'a>(
        &'a self,
        pids: &'a [u32],
        system: bool,
    ) -> impl Iterator<Item = &'a SessionEntry> + 'a {
        self.sessions.values().filter(move |s| {
            if system {
                s.system
            } else {
                !s.system && pids.contains(&s.pid)
            }
        })
    }
}

impl Backend for CoreBackend {
    fn read(&mut self) -> Reading {
        let devices = self.sync_devices();
        let mut raw = Vec::new();
        let mut live: Vec<(String, String)> = Vec::new();
        let ids: Vec<String> = self.devices.keys().cloned().collect();
        for dev_id in ids {
            let mgr = self.devices[&dev_id].mgr.clone();
            // SAFETY: enumeracia relacii (zaroven zapne notifikacie novych relacii) a citanie ich vlastnosti.
            unsafe {
                let Ok(list) = mgr.GetSessionEnumerator() else {
                    continue;
                };
                for i in 0..list.GetCount().unwrap_or(0) {
                    let Ok(ctl) = list.GetSession(i) else {
                        continue;
                    };
                    let Ok(c2) = ctl.cast::<IAudioSessionControl2>() else {
                        continue;
                    };
                    let state = ctl.GetState().unwrap_or(AudioSessionStateExpired);
                    if state == AudioSessionStateExpired {
                        continue;
                    }
                    let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() else {
                        continue;
                    };
                    let system = c2.IsSystemSoundsSession() == windows::Win32::Foundation::S_OK;
                    let pid = if system {
                        0
                    } else {
                        c2.GetProcessId().unwrap_or(0)
                    };
                    let inst = take_pwstr(c2.GetSessionInstanceIdentifier());
                    let key = (dev_id.clone(), inst);
                    if !self.sessions.contains_key(&key) {
                        let events: IAudioSessionEvents = SessionNotif(Arc::clone(&self.n)).into();
                        if ctl.RegisterAudioSessionNotification(&events).is_ok() {
                            self.sessions.insert(
                                key.clone(),
                                SessionEntry {
                                    ctl: ctl.clone(),
                                    events,
                                    vol: vol.clone(),
                                    pid,
                                    system,
                                },
                            );
                        }
                    }
                    live.push(key);
                    // chraneny proces (cestu nedostaneme) sa ukaze s nazvom relacie alebo PID
                    let exe = exe_of(pid);
                    let description = match &exe {
                        Some(e) => self
                            .desc
                            .entry(e.to_lowercase())
                            .or_insert_with(|| description_of(e))
                            .clone(),
                        None => None,
                    };
                    raw.push(RawSession {
                        pid,
                        system,
                        exe,
                        description,
                        display: take_pwstr(ctl.GetDisplayName()),
                        volume: vol.GetMasterVolume().unwrap_or(1.0),
                        muted: vol.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                        active: state == AudioSessionStateActive,
                    });
                }
            }
        }
        self.drop_sessions(|k| !live.contains(k));
        let mut routes = HashMap::new();
        if let Some(p) = &self.policy {
            let mut pids: Vec<u32> = raw.iter().map(|s| s.pid).collect();
            pids.sort_unstable();
            pids.dedup();
            for pid in pids {
                let r = p.route(pid);
                if r != Route::Default {
                    routes.insert(pid, r);
                }
            }
        }
        Reading {
            sessions: raw,
            devices,
            routes,
            routing: self.policy.is_some(),
        }
    }

    fn set_volume(&mut self, pids: &[u32], system: bool, v: f32) {
        for s in self.sessions_for(pids, system) {
            // SAFETY: zapis hlasitosti relacie s nasim kontextom.
            let _ = unsafe { s.vol.SetMasterVolume(v.clamp(0.0, 1.0), &OWN_CONTEXT) };
        }
    }

    fn set_mute(&mut self, pids: &[u32], system: bool, m: bool) {
        for s in self.sessions_for(pids, system) {
            // SAFETY: zapis stlmenia relacie s nasim kontextom.
            let _ = unsafe { s.vol.SetMute(m, &OWN_CONTEXT) };
        }
    }

    fn set_route(&mut self, pids: &[u32], device: Option<&str>) -> bool {
        let Some(p) = &self.policy else { return false };
        let mut ok = true;
        for pid in pids {
            ok &= p.set_route(*pid, device);
        }
        ok
    }

    fn shutdown(&mut self) {
        self.drop_sessions(|_| true);
        // SAFETY: zrusenie registracii mimo callbackov, potom uvolnenie rozhrani a COM.
        unsafe {
            for (_, d) in self.devices.drain() {
                let _ = d.mgr.UnregisterSessionNotification(&d.notif);
            }
            if let (Some(en), Some(c)) = (&self.en, &self.dev_notif) {
                let _ = en.UnregisterEndpointNotificationCallback(c);
            }
        }
        self.dev_notif = None;
        self.en = None;
        self.policy = None;
        if std::mem::take(&mut self.com) {
            // SAFETY: parter k uspesnemu CoInitializeEx.
            unsafe { CoUninitialize() };
        }
    }
}
