//! Zivy test nezavislosti hlasitosti zvukov transom od hlavnej hlasitosti systemu a od inych aplikacii.
//!
//! `cargo run -p transom-sound --example session_check`
//!
//! Co robi (a co NIE):
//! * Pocita len s CITANIM hlavnej hlasitosti a stlmenia predvoleneho vystupu (`Get*`) a hlasitosti, stlmenia a stavu
//!   vsetkych relacii v miesadle. Nic z toho nikdy NEMENI (ziadne `Set*` mimo relacie tohto procesu).
//! * Prehra kratky zvuk transom pri hlasitostiach 10, 30 a 60 % cez skutocny engine (`transom_sound::Player`),
//!   zmeria hlasitost a spicku vlastnej relacie a po kazdom kroku porovna hlavnu hlasitost a relacie ostatnych
//!   aplikacii (Spotify a pod.) s povodnym stavom.
//! * Zvuky su kratke (70 ms) a ticho: zvuk je viditelne mensi nez systemove upozornenia. Ak je hlavna hlasitost
//!   nulova alebo stlmena, nic nebude pocut, ale relacia a jej hlasitost sa overia rovnako.
//!
//! Vystup su riadky `KEY value`; navratovy kod 0 = vsetky invarianty platia.
#[cfg(windows)]
fn main() {
    std::process::exit(check::run());
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod check {
    use std::time::{Duration, Instant};
    use transom_config::UiSounds;
    use transom_sound::{Origin, Outcome, StartOptions};
    use windows::core::{Interface, PWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioSessionControl, IAudioSessionControl2, IAudioSessionManager2,
        IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    #[derive(Clone, Debug, PartialEq)]
    struct Sess {
        pid: u32,
        exe: String,
        id: String,
        name: String,
        state: i32,
        volume: f32,
        mute: bool,
    }

    fn log(m: &str) {
        println!("LOG {m}");
    }

    fn exe_of(pid: u32) -> String {
        if pid == 0 {
            return "(system)".into();
        }
        // SAFETY: bezne volania Win32; handle sa zatvori, buffer ma zadanu velkost.
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return "(?)".into();
            };
            let mut buf = [0u16; 520];
            let mut n = buf.len() as u32;
            let ok =
                QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n)
                    .is_ok();
            let _ = CloseHandle(h);
            if !ok {
                return "(?)".into();
            }
            let s = String::from_utf16_lossy(&buf[..n as usize]);
            s.rsplit('\\').next().unwrap_or("").to_string()
        }
    }

    fn pwstr(p: windows::core::Result<PWSTR>) -> String {
        match p {
            Ok(p) if !p.is_null() => {
                // SAFETY: retazec vlastni COM (CoTaskMemAlloc); po skopirovani sa uvolni.
                let s = unsafe { p.to_string() }.unwrap_or_default();
                unsafe { CoTaskMemFree(Some(p.0 as _)) };
                s
            }
            _ => String::new(),
        }
    }

    fn sessions(mgr: &IAudioSessionManager2) -> Vec<(Sess, IAudioSessionControl)> {
        let mut out = Vec::new();
        // SAFETY: len citanie vlastnosti relacii.
        unsafe {
            let Ok(en) = mgr.GetSessionEnumerator() else {
                return out;
            };
            for i in 0..en.GetCount().unwrap_or(0) {
                let Ok(ctl) = en.GetSession(i) else { continue };
                let Ok(c2) = ctl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() else {
                    continue;
                };
                let pid = c2.GetProcessId().unwrap_or(0);
                out.push((
                    Sess {
                        pid,
                        exe: exe_of(pid),
                        id: pwstr(c2.GetSessionInstanceIdentifier()),
                        name: pwstr(ctl.GetDisplayName()),
                        state: ctl.GetState().map(|s| s.0).unwrap_or(-1),
                        volume: vol.GetMasterVolume().unwrap_or(-1.0),
                        mute: vol.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                    },
                    ctl,
                ));
            }
        }
        out
    }

    fn peak(ctl: &IAudioSessionControl) -> f32 {
        // SAFETY: citanie spicky relacie.
        unsafe {
            ctl.cast::<IAudioMeterInformation>()
                .and_then(|m| m.GetPeakValue())
                .unwrap_or(-1.0)
        }
    }

    pub fn run() -> i32 {
        // SAFETY: COM pre hlavne vlakno; len citanie stavu zvukoveho systemu (viz hlavicka suboru).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(en) =
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            else {
                println!("FAIL ziadny zvukovy system");
                return 2;
            };
            let Ok(dev) = en.GetDefaultAudioEndpoint(eRender, eConsole) else {
                println!("FAIL ziadne predvolene vystupne zariadenie");
                return 2;
            };
            run_on(&dev)
        }
    }

    unsafe fn run_on(dev: &IMMDevice) -> i32 {
        let master: IAudioEndpointVolume = match unsafe { dev.Activate(CLSCTX_ALL, None) } {
            Ok(m) => m,
            Err(e) => {
                println!("FAIL endpoint volume: {e}");
                return 2;
            }
        };
        let read_master = || unsafe {
            (
                master.GetMasterVolumeLevelScalar().unwrap_or(-1.0),
                master.GetMute().map(|b| b.as_bool()).unwrap_or(false),
            )
        };
        let mgr: IAudioSessionManager2 = match unsafe { dev.Activate(CLSCTX_ALL, None) } {
            Ok(m) => m,
            Err(e) => {
                println!("FAIL session manager: {e}");
                return 2;
            }
        };
        let me = std::process::id();
        let before = sessions(&mgr);
        let master0 = read_master();
        println!("MASTER_BEFORE scalar={:.4} mute={}", master0.0, master0.1);
        for (s, _) in &before {
            println!(
                "SESSION_BEFORE pid={} exe={} state={} volume={:.4} mute={} name='{}'",
                s.pid, s.exe, s.state, s.volume, s.mute, s.name
            );
        }
        let others0: Vec<Sess> = before
            .iter()
            .filter(|(s, _)| s.pid != me)
            .map(|(s, _)| s.clone())
            .collect();

        let player = transom_sound::start_system(StartOptions {
            automation: false,
            log,
        });
        if player.is_silent() {
            println!("FAIL prehravac je mlcky (TRANSOM_NO_SOUND?)");
            return 2;
        }
        let mut ok = true;
        let mut peaks: Vec<(u32, f32, f32)> = Vec::new();
        for percent in [10u32, 30, 60] {
            player.set_settings(&UiSounds {
                enabled: true,
                volume: percent,
            });
            std::thread::sleep(Duration::from_millis(80)); // odstup medzi zvukmi
            if !player.cue(Origin::Pointer, Outcome::Done) {
                println!("FAIL cue odmietnuty pri {percent} %");
                ok = false;
                continue;
            }
            // pocas prehravania cita spicku a hlasitost vlastnej relacie
            let mut top = 0.0_f32;
            let mut vol = -1.0_f32;
            let mut name = String::new();
            let end = Instant::now() + Duration::from_millis(400);
            while Instant::now() < end {
                for (s, ctl) in sessions(&mgr) {
                    if s.pid == me {
                        top = top.max(peak(&ctl));
                        vol = s.volume;
                        name = s.name.clone();
                    }
                }
                std::thread::sleep(Duration::from_millis(3));
            }
            println!("OWN percent={percent} session_volume={vol:.4} peak={top:.4} name='{name}'");
            peaks.push((percent, vol, top));
            if (vol - percent as f32 / 100.0).abs() > 0.011 {
                println!("FAIL hlasitost relacie {vol} != {}", percent as f32 / 100.0);
                ok = false;
            }
            let m = read_master();
            if m != master0 {
                println!("FAIL hlavna hlasitost sa zmenila: {m:?} oproti {master0:?}");
                ok = false;
            }
            let now = sessions(&mgr);
            let others: Vec<Sess> = now
                .iter()
                .filter(|(s, _)| s.pid != me)
                .map(|(s, _)| s.clone())
                .collect();
            for o in &others0 {
                match others.iter().find(|n| n.id == o.id) {
                    Some(n) if (n.volume - o.volume).abs() < 1e-4 && n.mute == o.mute => {}
                    Some(n) => {
                        println!(
                            "FAIL relacia {} ({}) sa zmenila: {:?} -> {:?}",
                            o.exe, o.pid, o, n
                        );
                        ok = false;
                    }
                    None => println!("NOTE relacia {} ({}) medzitym zanikla", o.exe, o.pid),
                }
            }
        }
        // spicka relacie je informativna: Windows ju meria PRED hlasitostou relacie a hlavnou hlasitostou, takze pri
        // roznych hlasitostiach vychadza rovnaka (zdroj sa nemeni, meni sa len hlasitost relacie)
        if peaks.len() == 3 {
            println!(
                "PEAK_INFO source_peaks={:.4},{:.4},{:.4} (pred hlasitostou relacie; nie je test)",
                peaks[0].2, peaks[1].2, peaks[2].2
            );
        }
        // stlmenie: ziadny dalsi zvuk, ziadna nova relacia
        let played = player.stats().played;
        player.set_settings(&UiSounds {
            enabled: false,
            volume: 30,
        });
        std::thread::sleep(Duration::from_millis(80));
        let accepted = player.cue(Origin::Pointer, Outcome::Done);
        std::thread::sleep(Duration::from_millis(300));
        println!(
            "MUTE accepted={accepted} played_before={played} played_after={}",
            player.stats().played
        );
        if accepted || player.stats().played != played {
            println!("FAIL pri stlmeni sa zvuk prehral");
            ok = false;
        }
        // po necinnosti (4 s) sa prud uvolni: relacia transom uz nie je aktivna (stav 0 = neaktivna) a miesadlo ju
        // po case odstrani
        std::thread::sleep(Duration::from_millis(5500));
        let own: Vec<i32> = sessions(&mgr)
            .iter()
            .filter(|(s, _)| s.pid == me)
            .map(|(s, _)| s.state)
            .collect();
        println!(
            "OWN_SESSION_AFTER_IDLE count={} states={:?} (0 = neaktivna, 1 = aktivna)",
            own.len(),
            own
        );
        if own.contains(&1) {
            ok = false;
            println!("FAIL relacia transom je po necinnosti stale aktivna");
        }
        let m = read_master();
        println!("MASTER_AFTER scalar={:.4} mute={}", m.0, m.1);
        if m != master0 {
            ok = false;
            println!("FAIL hlavna hlasitost sa zmenila");
        }
        println!("STATS {:?}", player.stats());
        println!("RESULT {}", if ok { "OK" } else { "FAIL" });
        if ok {
            0
        } else {
            1
        }
    }
}
