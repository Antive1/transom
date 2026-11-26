//! Cisty model mixera (bez Win32): relacie zo vsetkych vystupov sa zoskupia do aplikacii, pomenuju a zoradia.
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Kluc skupiny systemovych zvukov.
pub const SYSTEM_KEY: &str = "\u{1}system";

/// Jedna zvukova relacia, ako ju precital backend.
#[derive(Clone, Debug, PartialEq)]
pub struct RawSession {
    pub pid: u32,
    /// Relacia systemovych zvukov Windows.
    pub system: bool,
    /// Plna cesta k .exe (ak sa proces dal otvorit).
    pub exe: Option<String>,
    /// Popis suboru z verzie .exe (napr. "Google Chrome").
    pub description: Option<String>,
    /// Nazov, ktory si relacia nastavila sama (casto prazdny alebo odkaz na zdroj "@...").
    pub display: String,
    /// Hlasitost relacie 0.0..=1.0.
    pub volume: f32,
    pub muted: bool,
    /// Relacia prave hra.
    pub active: bool,
}

/// Aktivny vystup (ID z MMDevice API sa pouziva len na prepinanie, nikdy sa nezobrazuje ani nelogue).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Kam aplikacia posiela zvuk.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Route {
    /// Predvoleny vystup Windows.
    #[default]
    Default,
    /// Konkretny vystup (ID zariadenia).
    Device(String),
}

/// Aplikacia v mixeri (vsetky jej relacie, aj na roznych vystupoch a z viacerych procesov s tym istym .exe).
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    /// Stabilny kluc (hash cesty .exe, systemove zvuky, alebo PID).
    pub key: u64,
    pub name: String,
    pub exe: Option<String>,
    pub system: bool,
    pub pids: Vec<u32>,
    /// Hlasitost 0..=100 (pri roznych hodnotach relacii hlasitost prvej hrajucej).
    pub volume: u8,
    /// Vsetky relacie su stlmene.
    pub muted: bool,
    pub active: bool,
    pub route: Route,
}

/// Snimka mixera pre UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Prve nacitanie prebehlo.
    pub ready: bool,
    pub apps: Vec<App>,
    pub devices: Vec<OutDevice>,
    /// Vystup pre aplikaciu sa da menit (nedokumentovane rozhranie Windows je dostupne).
    pub routing: bool,
    /// Posledna zmena vystupu zlyhala (kluc aplikacie).
    pub route_failed: Option<u64>,
    pub generation: u64,
}

impl Snapshot {
    pub fn app(&self, key: u64) -> Option<&App> {
        self.apps.iter().find(|a| a.key == key)
    }
}

pub fn hash_key(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Kluc skupiny relacie.
pub fn group_key(s: &RawSession) -> String {
    if s.system {
        SYSTEM_KEY.to_string()
    } else if let Some(e) = &s.exe {
        e.to_lowercase()
    } else {
        format!("pid:{}", s.pid)
    }
}

/// Meno suboru bez priecinka a pripony.
pub fn exe_stem(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match file.rfind('.') {
        Some(i) if i > 0 => file[..i].to_string(),
        _ => file.to_string(),
    }
}

/// Meno aplikacie: popis z verzie .exe, vlastny nazov relacie (nie odkaz na zdroj), meno .exe, inak PID.
pub fn app_name(s: &RawSession, system_name: &str) -> String {
    if s.system {
        return system_name.to_string();
    }
    let clean = |v: &str| {
        let v = v.trim();
        (!v.is_empty() && !v.starts_with('@')).then(|| v.chars().take(60).collect::<String>())
    };
    if let Some(d) = s.description.as_deref().and_then(clean) {
        return d;
    }
    if let Some(d) = clean(&s.display) {
        return d;
    }
    if let Some(e) = &s.exe {
        let stem = exe_stem(e);
        if !stem.is_empty() {
            return stem;
        }
    }
    format!("PID {}", s.pid)
}

/// Zoskupi relacie do aplikacii a zoradi ich: hrajuce, potom podla mena; systemove zvuky vzdy na konci.
pub fn group(sessions: &[RawSession], routes: &HashMap<u32, Route>, system_name: &str) -> Vec<App> {
    let mut order: Vec<String> = Vec::new();
    let mut by: HashMap<String, Vec<&RawSession>> = HashMap::new();
    for s in sessions {
        let k = group_key(s);
        if !by.contains_key(&k) {
            order.push(k.clone());
        }
        by.entry(k).or_default().push(s);
    }
    let mut apps: Vec<App> = order
        .into_iter()
        .map(|k| {
            let list = &by[&k];
            let lead = list.iter().find(|s| s.active).unwrap_or(&list[0]);
            let mut pids: Vec<u32> = list.iter().map(|s| s.pid).collect();
            pids.sort_unstable();
            pids.dedup();
            let route_pid = if lead.system { 0 } else { pids[0] };
            App {
                key: hash_key(&k),
                // meno z relacie s popisom .exe (ak ho ma niektora), inak z hlavnej
                name: app_name(
                    list.iter()
                        .find(|s| s.description.is_some())
                        .unwrap_or(lead),
                    system_name,
                ),
                exe: if lead.system { None } else { lead.exe.clone() },
                system: lead.system,
                volume: (lead.volume.clamp(0.0, 1.0) * 100.0).round() as u8,
                muted: list.iter().all(|s| s.muted),
                active: list.iter().any(|s| s.active),
                route: routes.get(&route_pid).cloned().unwrap_or_default(),
                pids,
            }
        })
        .collect();
    apps.sort_by(|a, b| {
        a.system
            .cmp(&b.system)
            .then(b.active.cmp(&a.active))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    apps
}

/// Ciselne ID vystupu Windows -> retazec pre rozhranie politiky (a spat).
pub const POLICY_PREFIX: &str = r"\\?\SWD#MMDEVAPI#";
pub const POLICY_RENDER_SUFFIX: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";

pub fn pack_device_id(id: &str) -> String {
    format!("{POLICY_PREFIX}{id}{POLICY_RENDER_SUFFIX}")
}

/// Odpoved rozhrania politiky -> trasa (prazdna = predvoleny vystup).
pub fn unpack_route(raw: &str) -> Route {
    let s = raw.trim();
    let s = s.strip_prefix(POLICY_PREFIX).unwrap_or(s);
    let s = s.strip_suffix(POLICY_RENDER_SUFFIX).unwrap_or(s);
    if s.is_empty() {
        Route::Default
    } else {
        Route::Device(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sess(pid: u32, exe: Option<&str>, vol: f32, muted: bool, active: bool) -> RawSession {
        RawSession {
            pid,
            system: false,
            exe: exe.map(str::to_string),
            description: None,
            display: String::new(),
            volume: vol,
            muted,
            active,
        }
    }

    #[test]
    fn sessions_of_one_exe_become_one_app_and_system_sounds_go_last() {
        let mut sys = sess(0, None, 0.5, false, false);
        sys.system = true;
        let mut chrome1 = sess(10, Some(r"C:\P\Google\chrome.exe"), 0.8, false, false);
        chrome1.description = Some("Google Chrome".into());
        let chrome2 = sess(11, Some(r"C:\p\google\CHROME.exe"), 0.4, false, true);
        let spotify = sess(20, Some(r"C:\Spotify\Spotify.exe"), 1.0, true, false);
        let unknown = sess(30, None, 1.0, false, false);
        let mut routes = HashMap::new();
        routes.insert(20, Route::Device("{x}".into()));
        let apps = group(
            &[sys, chrome1, chrome2, spotify, unknown],
            &routes,
            "System sounds",
        );
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        // hrajuci Chrome prvy, potom podla mena, systemove zvuky na konci
        assert_eq!(
            names,
            ["Google Chrome", "PID 30", "Spotify", "System sounds"]
        );
        let chrome = &apps[0];
        assert_eq!(chrome.pids, [10, 11]);
        // hlasitost hrajucej relacie
        assert_eq!(chrome.volume, 40);
        assert!(chrome.active && !chrome.muted);
        assert_eq!(apps[2].route, Route::Device("{x}".into()));
        assert!(apps[2].muted);
        assert!(apps[3].system && apps[3].exe.is_none());
        // kluce su stabilne a rozne
        let again = group(
            &[sess(11, Some(r"c:\P\GOOGLE\chrome.exe"), 1.0, false, false)],
            &HashMap::new(),
            "",
        );
        assert_eq!(again[0].key, chrome.key);
        assert_ne!(apps[1].key, apps[2].key);
    }

    #[test]
    fn names_prefer_the_file_description_and_skip_resource_references() {
        let mut s = sess(5, Some(r"C:\x\app.v2.exe"), 1.0, false, false);
        assert_eq!(app_name(&s, ""), "app.v2");
        s.display = "@%SystemRoot%\\System32\\AudioSrv.Dll,-202".into();
        assert_eq!(app_name(&s, ""), "app.v2");
        s.display = "  Moja hra ".into();
        assert_eq!(app_name(&s, ""), "Moja hra");
        s.description = Some("Pekna aplikacia".into());
        assert_eq!(app_name(&s, ""), "Pekna aplikacia");
        s.system = true;
        assert_eq!(app_name(&s, "Systemove zvuky"), "Systemove zvuky");
    }

    #[test]
    fn device_ids_round_trip_through_the_policy_format() {
        let id = "{0.0.0.00000000}.{abc}";
        assert_eq!(unpack_route(&pack_device_id(id)), Route::Device(id.into()));
        assert_eq!(unpack_route(""), Route::Default);
        assert_eq!(unpack_route(id), Route::Device(id.into()));
    }
}
