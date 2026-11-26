//! Sledovanie priecinkov cez dokumentovane `ReadDirectoryChangesW` (asynchronne, `FILE_FLAG_OVERLAPPED`): jedno vlakno na priecinok.
//! Sluzi len ako **spustac** ("nieco sa zmenilo, vymenuj znova"); nazvy zmenenych suborov sa vobec nespracuvaju ani nelogu.
//!
//! * Uchytka priecinka sa otvara so zdielanim READ|WRITE|DELETE, takze pouzivatel moze priecinok aj subory v Prieskumnikovi
//!   bez prekazky premenovat, presunut alebo zmazat; po zahodeni `Watch` sa uchytky zatvoria.
//! * Buffer je 32 KB (pre siet dokumentacia dovoluje najviac 64 KB). Nulovy pocet prenesenych bajtov (preplneny buffer) a
//!   `ERROR_NOTIFY_ENUM_DIR` znamenaju "treba znovu vymenovat" (`Change::Overflow`); kazda ina chyba znamena nedostupny
//!   priecinok (`Change::Unavailable`) a opakovany pokus s narastajucim odstupom, nikdy nie tesna slucka.
//! * Chybajuci priecinok: caka sa na vytvorenie v **bezprostrednom** rodicovi (nikdy sa nestupa vyssie k profilu ani disku) a
//!   aspon kazdych `backoff_max` ms sa skusi znova (len kym je sledovanie aktivne).
use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{
    CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Pribudol, zmizol alebo sa zmenil subor.
    Changed,
    /// Buffer pretiekol alebo system nezaznamenal vsetky zmeny: treba znovu vymenovat.
    Overflow,
    /// Sledovany priecinok zmizol alebo je nedostupny (po jeho obnove pride `Changed`).
    Unavailable,
}

/// Casovanie a velkost bufra (testy ich skracuju).
#[derive(Clone, Copy, Debug)]
pub struct WatchCfg {
    pub buffer_bytes: usize,
    pub backoff_start_ms: u32,
    pub backoff_max_ms: u32,
}

impl Default for WatchCfg {
    fn default() -> Self {
        WatchCfg {
            buffer_bytes: 32 * 1024,
            backoff_start_ms: 2_000,
            backoff_max_ms: 30_000,
        }
    }
}

const ERROR_NOTIFY_ENUM_DIR: u32 = 1022;

/// Vysledok `GetOverlappedResult`: `Ok(prenesene bajty)` alebo `Err(kod Win32)`.
pub fn classify(r: Result<u32, u32>) -> Change {
    match r {
        Ok(0) => Change::Overflow,
        Ok(_) => Change::Changed,
        Err(ERROR_NOTIFY_ENUM_DIR) => Change::Overflow,
        Err(_) => Change::Unavailable,
    }
}

fn win32_code(e: &windows::core::Error) -> u32 {
    (e.code().0 as u32) & 0xFFFF
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn h(v: isize) -> HANDLE {
    HANDLE(v as *mut c_void)
}

fn open_dir(dir: &str) -> Option<HANDLE> {
    let w = wide(dir);
    // SAFETY: retazec ukonceny nulou zije cez volanie; uchytku zatvara volajuci (`CloseHandle`).
    unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED | FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .ok()
    }
}

const FILTER: FILE_NOTIFY_CHANGE = FILE_NOTIFY_CHANGE(
    FILE_NOTIFY_CHANGE_FILE_NAME.0
        | FILE_NOTIFY_CHANGE_LAST_WRITE.0
        | FILE_NOTIFY_CHANGE_SIZE.0
        | FILE_NOTIFY_CHANGE_DIR_NAME.0,
);

enum Ended {
    Stopped,
    Lost,
}

/// Jedna asynchronna slucka nad otvorenou uchytkou: caka na udalost alebo zastavenie.
fn pump(dir: HANDLE, buf_bytes: usize, notify: &dyn Fn(Change), stop: HANDLE) -> Ended {
    // SAFETY: udalost a buffer zijú cez cele cakanie; pred opustenim sa cakajuci vstup zrusi a pocka sa na jeho dokoncenie.
    unsafe {
        let Ok(ev) = CreateEventW(None, true, false, PCWSTR::null()) else {
            return Ended::Lost;
        };
        // buffer zarovnany na DWORD (u32)
        let mut buf = vec![0u32; (buf_bytes / 4).max(64)];
        let mut ov = OVERLAPPED {
            hEvent: ev,
            ..Default::default()
        };
        let out = loop {
            let _ = ResetEvent(ev);
            if ReadDirectoryChangesW(
                dir,
                buf.as_mut_ptr() as *mut c_void,
                (buf.len() * 4) as u32,
                false,
                FILTER,
                None,
                Some(&mut ov),
                None,
            )
            .is_err()
            {
                break Ended::Lost;
            }
            let w = WaitForMultipleObjects(&[ev, stop], false, u32::MAX);
            if w.0 != WAIT_OBJECT_0.0 {
                // zastavenie: zrusit cakajuci vstup a pockat na dokoncenie (buffer musi zit, kym system pise)
                let _ = CancelIoEx(dir, Some(&ov));
                let mut n = 0u32;
                let _ = GetOverlappedResult(dir, &ov, &mut n, true);
                break Ended::Stopped;
            }
            let mut n = 0u32;
            let r = GetOverlappedResult(dir, &ov, &mut n, false)
                .map(|()| n)
                .map_err(|e| win32_code(&e));
            match classify(r) {
                Change::Unavailable => break Ended::Lost,
                c => notify(c),
            }
        };
        let _ = CloseHandle(ev);
        out
    }
}

/// Pocka na zmenu v rodicovi (vznik priecinka) najviac `ms` ms; vracia `true`, ak sa ma sledovanie ukoncit.
fn wait_for_dir(parent: Option<&str>, ms: u32, stop: HANDLE) -> bool {
    // SAFETY: ako v `pump`; uchytka rodica sa vzdy zatvori.
    unsafe {
        let parent_handle = parent.and_then(open_dir);
        let Some(ph) = parent_handle else {
            return WaitForSingleObject(stop, ms) == WAIT_OBJECT_0;
        };
        let Ok(ev) = CreateEventW(None, true, false, PCWSTR::null()) else {
            let _ = CloseHandle(ph);
            return WaitForSingleObject(stop, ms) == WAIT_OBJECT_0;
        };
        let mut buf = vec![0u32; 64];
        let mut ov = OVERLAPPED {
            hEvent: ev,
            ..Default::default()
        };
        let stopped;
        let armed = ReadDirectoryChangesW(
            ph,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 4) as u32,
            false,
            FILE_NOTIFY_CHANGE_DIR_NAME,
            None,
            Some(&mut ov),
            None,
        )
        .is_ok();
        if armed {
            let w = WaitForMultipleObjects(&[ev, stop], false, ms);
            stopped = w.0 == WAIT_OBJECT_0.0 + 1;
            // cakajuci vstup sa vzdy zrusi a pocka sa na jeho dokoncenie, kym buffer zije
            let _ = CancelIoEx(ph, Some(&ov));
            let mut n = 0u32;
            let _ = GetOverlappedResult(ph, &ov, &mut n, true);
        } else {
            stopped = WaitForSingleObject(stop, ms) == WAIT_OBJECT_0;
        }
        let _ = CloseHandle(ev);
        let _ = CloseHandle(ph);
        stopped
    }
}

fn watch_dir(dir: String, cfg: WatchCfg, notify: Arc<dyn Fn(Change) + Send + Sync>, stop: isize) {
    let stop = h(stop);
    let parent = Path::new(&dir)
        .parent()
        .and_then(|p| p.to_str())
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let mut backoff = cfg.backoff_start_ms;
    let mut unavailable = false;
    loop {
        // SAFETY: cakanie na udalost zastavenia.
        if unsafe { WaitForSingleObject(stop, 0) } == WAIT_OBJECT_0 {
            return;
        }
        match open_dir(&dir) {
            Some(handle) => {
                if unavailable {
                    // priecinok sa vratil: zoznam sa musi nacitat znova
                    notify(Change::Changed);
                    unavailable = false;
                }
                backoff = cfg.backoff_start_ms;
                let ended = pump(handle, cfg.buffer_bytes, notify.as_ref(), stop);
                // SAFETY: uchytku sme otvorili my a nikto ju nepouziva.
                unsafe {
                    let _ = CloseHandle(handle);
                }
                if matches!(ended, Ended::Stopped) {
                    return;
                }
                // uchytka sa stratila (priecinok zmazany / odpojeny): kratka pauza pred dalsim pokusom
                if !unavailable {
                    notify(Change::Unavailable);
                    unavailable = true;
                }
                // SAFETY: cakanie na udalost zastavenia.
                if unsafe { WaitForSingleObject(stop, cfg.backoff_start_ms.min(500)) }
                    == WAIT_OBJECT_0
                {
                    return;
                }
            }
            None => {
                if !unavailable {
                    notify(Change::Unavailable);
                    unavailable = true;
                }
                if wait_for_dir(parent.as_deref(), backoff, stop) {
                    return;
                }
                backoff = (backoff.saturating_mul(2)).min(cfg.backoff_max_ms);
            }
        }
    }
}

/// Sledovanie priecinkov; zahodenie ho ukonci (vlakna sa zastavia a uchytky zatvoria na pozadi, UI vlakno necaka).
pub struct Watch {
    stop: isize,
    threads: Vec<JoinHandle<()>>,
}

impl Watch {
    pub fn start(dirs: &[String], notify: Arc<dyn Fn(Change) + Send + Sync>) -> Watch {
        Watch::start_with(dirs, WatchCfg::default(), notify)
    }

    pub fn start_with(
        dirs: &[String],
        cfg: WatchCfg,
        notify: Arc<dyn Fn(Change) + Send + Sync>,
    ) -> Watch {
        // SAFETY: rucne resetovana udalost zastavenia, zatvara ju vlakno uvolnujuce `Watch`.
        let stop = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map(|e| e.0 as isize)
            .unwrap_or(0);
        let mut threads = Vec::new();
        if stop != 0 {
            for d in dirs.iter().take(16) {
                let (d, n) = (d.clone(), notify.clone());
                if let Ok(t) = std::thread::Builder::new()
                    .name("transom-dirwatch".into())
                    .spawn(move || watch_dir(d, cfg, n, stop))
                {
                    threads.push(t);
                }
            }
        }
        Watch { stop, threads }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.stop == 0 {
            return;
        }
        // SAFETY: signalizacia udalosti, ktoru sme vytvorili.
        unsafe {
            let _ = SetEvent(h(self.stop));
        }
        let (stop, threads) = (self.stop, std::mem::take(&mut self.threads));
        // prepojenie na pozadi: vlakno moze cakat na pomaly sietovy priecinok a UI vlakno nesmie
        let _ = std::thread::Builder::new()
            .name("transom-dirwatch-reaper".into())
            .spawn(move || {
                for t in threads {
                    let _ = t.join();
                }
                // SAFETY: vsetky vlakna skoncili, nikto uz udalost nepouziva.
                unsafe {
                    let _ = CloseHandle(h(stop));
                }
            });
    }
}

/// Zjednodusene sledovanie na spolocny pouzitie: ukonci sa zahodenim vysledku.
pub fn watch_boxed(
    dirs: &[String],
    notify: Arc<dyn Fn(Change) + Send + Sync>,
) -> Box<dyn std::any::Any + Send> {
    Box::new(Watch::start(dirs, notify))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn scratch(name: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "transom-dirwatch-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fast() -> WatchCfg {
        WatchCfg {
            buffer_bytes: 32 * 1024,
            backoff_start_ms: 40,
            backoff_max_ms: 200,
        }
    }

    fn collect() -> (Arc<dyn Fn(Change) + Send + Sync>, mpsc::Receiver<Change>) {
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        (
            Arc::new(move |c| {
                let _ = tx.lock().unwrap().send(c);
            }),
            rx,
        )
    }

    fn wait_for(rx: &mpsc::Receiver<Change>, want: Change, secs: u64) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(secs) {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(c) if c == want => return true,
                _ => {}
            }
        }
        false
    }

    #[test]
    fn results_map_to_changes_zero_bytes_and_enum_dir_mean_rescan_other_errors_mean_unavailable() {
        assert_eq!(classify(Ok(48)), Change::Changed);
        assert_eq!(
            classify(Ok(0)),
            Change::Overflow,
            "preplneny buffer: success s nulou bajtov"
        );
        assert_eq!(
            classify(Err(1022)),
            Change::Overflow,
            "ERROR_NOTIFY_ENUM_DIR"
        );
        assert_eq!(
            classify(Err(5)),
            Change::Unavailable,
            "access denied po zmazani priecinka"
        );
        assert_eq!(classify(Err(995)), Change::Unavailable);
    }

    #[test]
    fn a_new_file_triggers_a_change_and_the_watched_folder_can_be_renamed_while_watched() {
        let d = scratch("change");
        let (n, rx) = collect();
        let w = Watch::start_with(&[d.to_str().unwrap().to_string()], fast(), n);
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(d.join("a.png"), b"x").unwrap();
        assert!(wait_for(&rx, Change::Changed, 5), "zmena sa ohlasila");
        // uchytka ma zdielanie DELETE: priecinok sa da premenovat aj pocas sledovania (pouzivatel nie je blokovany)
        let moved = d.with_file_name(format!(
            "{}-moved",
            d.file_name().unwrap().to_string_lossy()
        ));
        std::fs::rename(&d, &moved).expect("premenovanie sledovaneho priecinka");
        // uchytka nasleduje samotny priecinok, nie jeho nazov; premenovanie sledovaneho priecinka RDCW nehlasi (dokumentacia: "does
        // not report changes to the specified directory itself"), preto kniznica ponuka rucne obnovenie a po otvoreni nacita znova
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(moved.join("b.png"), b"y").unwrap();
        assert!(
            wait_for(&rx, Change::Changed, 5),
            "sledovanie vydrzi premenovanie bez panik a slucky"
        );
        drop(w);
        let _ = std::fs::remove_dir_all(&moved);
    }

    #[test]
    fn a_vanished_folder_reports_once_does_not_spin_and_its_return_triggers_a_rescan() {
        let root = scratch("gone");
        let d = root.join("Shots");
        std::fs::create_dir_all(&d).unwrap();
        let (n, rx) = collect();
        let w = Watch::start_with(&[d.to_str().unwrap().to_string()], fast(), n);
        std::thread::sleep(Duration::from_millis(200));
        std::fs::remove_dir_all(&d).unwrap();
        assert!(wait_for(&rx, Change::Unavailable, 5));
        // pocas nedostupnosti nepridu dalsie udalosti (ziadna tesna slucka ani opakovane hlasenia)
        std::thread::sleep(Duration::from_millis(700));
        let extra: Vec<Change> = rx.try_iter().collect();
        assert!(extra.len() <= 1, "{extra:?}");
        // priecinok sa vrati: sledovanie ho znova otvori a ohlasi, ze treba znovu vymenovat
        std::fs::create_dir_all(&d).unwrap();
        assert!(
            wait_for(&rx, Change::Changed, 5),
            "navrat priecinka vedie k znovunacitaniu"
        );
        std::fs::write(d.join("b.png"), b"y").unwrap();
        assert!(
            wait_for(&rx, Change::Changed, 5),
            "a sledovanie znova funguje"
        );
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_never_existed_is_reported_without_creating_it_and_appears_when_made() {
        let root = scratch("missing");
        let d = root.join("later");
        let (n, rx) = collect();
        let w = Watch::start_with(&[d.to_str().unwrap().to_string()], fast(), n);
        assert!(wait_for(&rx, Change::Unavailable, 5));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!d.exists(), "sledovanie priecinok nikdy nevytvara");
        std::fs::create_dir_all(&d).unwrap();
        assert!(wait_for(&rx, Change::Changed, 5));
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dropping_the_watch_releases_the_folder_handle_so_it_can_be_removed() {
        let d = scratch("release");
        let (n, rx) = collect();
        let w = Watch::start_with(&[d.to_str().unwrap().to_string()], fast(), n);
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(d.join("a.png"), b"x").unwrap();
        assert!(wait_for(&rx, Change::Changed, 5));
        drop(w);
        let t = Instant::now();
        let mut removed = false;
        while t.elapsed() < Duration::from_secs(5) {
            if std::fs::remove_dir_all(&d).is_ok() && !d.exists() {
                removed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            removed,
            "uchytka priecinka sa po zahodeni sledovania uvolnila"
        );
        // po zahodeni uz nechodia ziadne udalosti
    }
}
