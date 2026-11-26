//! Mikrofon a kamera (polozka Mikrofon a kamera): kto ich prave pouziva a stlmenie mikrofonu.
//!
//! * Mikrofon v pouziti: aktivne relacie na **vstupnych** zariadeniach (dokumentovane Core Audio, `IAudioSessionManager2`).
//! * Kamera (a doplnkovo mikrofon) v pouziti: historia pristupu, ktoru vedie Windows v nastaveniach sukromia
//!   (`HKCU\...\CapabilityAccessManager\ConsentStore\<webcam|microphone>`, hodnoty `LastUsedTimeStart/Stop`). Toto je
//!   **nedokumentovany** zdroj (Windows ho pouziva pre ikonu kamery a mikrofonu v oblasti oznameni); transom ho len cita.
//! * Stlmenie mikrofonu: `IAudioEndpointVolume::SetMute` na predvolenom vstupe pre **obe** roly (konzola aj komunikacia,
//!   ak su to rozne zariadenia). Stav sa vzdy precita spat, nikdy sa neodhaduje z prepinaca.
//!
//! Hlavnu hlasitost ani predvoleny vystup tento subor nemeni.
use crate::core::{description_of, exe_of, take_pwstr};
use crate::model::exe_stem;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
};

/// Kontext nasich zapisov stlmenia.
const OWN_CONTEXT: windows::core::GUID =
    windows::core::GUID::from_u128(0x6a1d3e4f_0b2c_4e7a_9d18_2f5c7b9e0a31);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivSnap {
    pub ready: bool,
    /// Je nejaky aktivny vstup (mikrofon)?
    pub mic_present: bool,
    /// Predvoleny mikrofon je stlmeny (`None` = nevieme, napr. ziadny mikrofon).
    pub mic_muted: Option<bool>,
    /// Aplikacie, ktore prave pouzivaju mikrofon / kameru (mena, bez duplicit).
    pub mic_apps: Vec<String>,
    pub cam_apps: Vec<String>,
    pub generation: u64,
}

const CONSENT: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: kluc otvoreny cez RegOpenKeyExW.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn open(parent: HKEY, sub: &str) -> Option<Key> {
    let mut k = HKEY::default();
    // SAFETY: len citanie; kluc sa zatvori v Drop.
    unsafe { RegOpenKeyExW(parent, &HSTRING::from(sub), Some(0), KEY_READ, &mut k) }
        .ok()
        .ok()
        .map(|_| Key(k))
}

fn subkeys(k: &Key) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..4096u32 {
        let mut buf = [0u16; 512];
        let mut n = buf.len() as u32;
        // SAFETY: buffer s dlzkou; koniec zoznamu vrati chybu.
        let r = unsafe {
            RegEnumKeyExW(
                k.0,
                i,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut n,
                None,
                None,
                None,
                None,
            )
        };
        if r.is_err() {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..n as usize]));
    }
    out
}

fn qword(k: &Key, name: &str) -> Option<u64> {
    let mut v = 0u64;
    let mut n = 8u32;
    // SAFETY: citanie 8-bajtovej hodnoty do lokalnej premennej.
    unsafe {
        RegQueryValueExW(
            k.0,
            &HSTRING::from(name),
            None,
            None,
            Some(&mut v as *mut u64 as *mut u8),
            Some(&mut n),
        )
    }
    .ok()
    .ok()
    .filter(|_| n == 8)
    .map(|_| v)
}

fn in_use(k: &Key) -> bool {
    let start = qword(k, "LastUsedTimeStart").unwrap_or(0);
    let stop = qword(k, "LastUsedTimeStop");
    start > 0 && stop == Some(0)
}

/// Meno balika "Vydavatel.Aplikacia_hash" -> "Aplikacia".
pub fn package_name(pfn: &str) -> String {
    let base = pfn.split('_').next().unwrap_or(pfn);
    base.rsplit('.').next().unwrap_or(base).to_string()
}

/// Cesta z kluca NonPackaged ("C:#Program Files#app.exe") -> meno aplikacie.
pub fn nonpackaged_name(key: &str) -> String {
    let path = key.replace('#', "\\");
    description_of(&path).unwrap_or_else(|| exe_stem(&path))
}

/// Aplikacie, ktore podla historie sukromia Windows prave pouzivaju schopnost (`webcam`, `microphone`).
pub fn consent_in_use(cap: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(root) = open(HKEY_CURRENT_USER, &format!(r"{CONSENT}\{cap}")) else {
        return out;
    };
    for sk in subkeys(&root) {
        if sk == "NonPackaged" {
            if let Some(np) = open(root.0, "NonPackaged") {
                for app in subkeys(&np) {
                    if open(np.0, &app).is_some_and(|k| in_use(&k)) {
                        out.push(nonpackaged_name(&app));
                    }
                }
            }
        } else if open(root.0, &sk).is_some_and(|k| in_use(&k)) {
            out.push(package_name(&sk));
        }
    }
    out
}

fn dedup(mut v: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    v.retain(|s| {
        let l = s.to_lowercase();
        if seen.contains(&l) {
            false
        } else {
            seen.push(l);
            true
        }
    });
    v
}

/// Na vlakne s COM (MTA).
struct Reader {
    en: Option<IMMDeviceEnumerator>,
}

impl Reader {
    fn new() -> Reader {
        // SAFETY: vytvorenie enumeratora.
        Reader {
            // SAFETY: COM je inicializovany na vlakne citania (volajuci).
            en: unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok(),
        }
    }

    fn default_mics(&self) -> Vec<IAudioEndpointVolume> {
        let mut out = Vec::new();
        let Some(en) = &self.en else { return out };
        let mut ids: Vec<String> = Vec::new();
        for role in [eConsole, eCommunications] {
            // SAFETY: citanie predvoleneho vstupu a aktivacia jeho hlasitosti.
            unsafe {
                let Ok(d) = en.GetDefaultAudioEndpoint(eCapture, role) else {
                    continue;
                };
                let id = take_pwstr(d.GetId());
                if ids.contains(&id) {
                    continue;
                }
                if let Ok(v) = d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    ids.push(id);
                    out.push(v);
                }
            }
        }
        out
    }

    fn read(&self) -> PrivSnap {
        let mut s = PrivSnap {
            ready: true,
            ..PrivSnap::default()
        };
        let mics = self.default_mics();
        s.mic_present = !mics.is_empty();
        if !mics.is_empty() {
            // stlmeny = vsetky predvolene vstupy stlmene
            // SAFETY: citanie stavu stlmenia.
            let muted: Vec<bool> = mics
                .iter()
                .filter_map(|v| unsafe { v.GetMute() }.ok().map(|b| b.as_bool()))
                .collect();
            s.mic_muted = (!muted.is_empty()).then(|| muted.iter().all(|m| *m));
        }
        let mut apps = Vec::new();
        if let Some(en) = &self.en {
            // SAFETY: enumeracia relacii na aktivnych vstupoch, len citanie.
            unsafe {
                if let Ok(col) = en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) {
                    for i in 0..col.GetCount().unwrap_or(0) {
                        let Ok(dev) = col.Item(i) else { continue };
                        let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)
                        else {
                            continue;
                        };
                        let Ok(list) = mgr.GetSessionEnumerator() else {
                            continue;
                        };
                        for j in 0..list.GetCount().unwrap_or(0) {
                            let Ok(ctl) = list.GetSession(j) else {
                                continue;
                            };
                            if ctl.GetState().ok() != Some(AudioSessionStateActive) {
                                continue;
                            }
                            let Ok(c2) = ctl.cast::<IAudioSessionControl2>() else {
                                continue;
                            };
                            let pid = c2.GetProcessId().unwrap_or(0);
                            let name = match exe_of(pid) {
                                Some(e) => description_of(&e).unwrap_or_else(|| exe_stem(&e)),
                                None => format!("PID {pid}"),
                            };
                            apps.push(name);
                        }
                    }
                }
            }
        }
        apps.extend(consent_in_use("microphone"));
        s.mic_apps = dedup(apps);
        s.cam_apps = dedup(consent_in_use("webcam"));
        s
    }

    /// Stlmi / zapne vsetky predvolene vstupy; vrati stav podla spatneho citania.
    fn set_mute(&self, m: bool) -> Option<bool> {
        let mics = self.default_mics();
        for v in &mics {
            // SAFETY: zapis stlmenia s nasim kontextom.
            let _ = unsafe { v.SetMute(m, &OWN_CONTEXT) };
        }
        // SAFETY: spatne citanie.
        let back: Vec<bool> = mics
            .iter()
            .filter_map(|v| unsafe { v.GetMute() }.ok().map(|b| b.as_bool()))
            .collect();
        (!back.is_empty()).then(|| back.iter().all(|x| *x))
    }
}

struct Com(bool);

impl Com {
    fn init() -> Com {
        // SAFETY: COM MTA na tomto vlakne (parovy CoUninitialize v Drop).
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

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
    Mute(bool),
    Toggle,
}

/// Sluzba: cita stav v intervale (kym zije) a vykonava stlmenie.
pub struct PrivService {
    tx: Sender<Msg>,
    snap: Arc<Mutex<Arc<PrivSnap>>>,
}

impl PrivService {
    pub fn start(
        notify: Arc<dyn Fn() + Send + Sync>,
        interval: Duration,
    ) -> Result<PrivService, String> {
        let (tx, rx) = channel::<Msg>();
        let snap = Arc::new(Mutex::new(Arc::new(PrivSnap::default())));
        let s2 = Arc::clone(&snap);
        std::thread::Builder::new()
            .name("transom-privacy".into())
            .spawn(move || {
                let _com = Com::init();
                let r = Reader::new();
                let mut gen = 0u64;
                let mut msg: Option<Msg> = Some(Msg::Refresh);
                loop {
                    match msg.take() {
                        Some(Msg::Mute(m)) => {
                            let _ = r.set_mute(m);
                        }
                        Some(Msg::Toggle) => {
                            let cur = r.read().mic_muted.unwrap_or(false);
                            let _ = r.set_mute(!cur);
                        }
                        Some(Msg::Refresh) | None => {}
                    }
                    let mut s = r.read();
                    let changed = s2.lock().map(|g| {
                        let mut a = (**g).clone();
                        a.generation = 0;
                        a != s
                    });
                    if changed.unwrap_or(true) {
                        gen += 1;
                        s.generation = gen;
                        if let Ok(mut g) = s2.lock() {
                            *g = Arc::new(s);
                        }
                        notify();
                    }
                    match rx.recv_timeout(interval) {
                        Ok(m) => msg = Some(m),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(PrivService { tx, snap })
    }

    pub fn snapshot(&self) -> Arc<PrivSnap> {
        self.snap.lock().map(|g| Arc::clone(&g)).unwrap_or_default()
    }

    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    pub fn set_mute(&self, m: bool) {
        let _ = self.tx.send(Msg::Mute(m));
    }

    pub fn toggle_mute(&self) {
        let _ = self.tx.send(Msg::Toggle);
    }
}

/// Jednorazove prepnutie stlmenia (ked sluzba nebezi): na vlastnom vlakne, vysledok (stav po zmene) do `done`.
pub fn toggle_mute_once(done: impl FnOnce(Option<bool>) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("transom-mic-toggle".into())
        .spawn(move || {
            let _com = Com::init();
            let r = Reader::new();
            let cur = r.read().mic_muted;
            done(cur.and_then(|c| r.set_mute(!c)));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_and_path_keys_become_readable_names() {
        assert_eq!(
            package_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"),
            "WindowsCamera"
        );
        assert_eq!(
            package_name("AD2F1837.HPPrinterControl_v10z8vjag6ke6"),
            "HPPrinterControl"
        );
        assert_eq!(package_name("noseparator"), "noseparator");
        // neexistujuca cesta: meno .exe
        assert_eq!(nonpackaged_name(r"Z:#Nie#Je#Moja Hra.exe"), "Moja Hra");
        assert_eq!(
            dedup(vec!["Zoom".into(), "zoom".into(), "Teams".into()]),
            vec!["Zoom", "Teams"]
        );
    }

    #[test]
    fn reading_the_real_privacy_store_never_fails() {
        // len citanie: na kazdom PC vrati zoznam (mozno prazdny), nikdy nespadne
        let _ = consent_in_use("webcam");
        let _ = consent_in_use("microphone");
        let _ = consent_in_use("neexistuje");
    }
}
