//! Ikony vysledkov vyhladavaca: vyber zdroja ikony pre kazdy vysledok a asynchronne nacitanie cez dokumentovane API shellu
//! (`transom_platform::shellicon`). Kreslenie nikdy necaka: `IconSource::icon` je rychle nahliadnutie do cache a vzdy vrati hned
//! pouzitelnu ikonu (hotovu systemovu, alebo vstavanu vektorovu zastupnu), takze pisanie ani vyber vysledku sa nezdrzia.
//!
//! Zdroje podla druhu vysledku:
//! * aplikacia: `shell:AppsFolder\...` cez shell; odkaz `.lnk` a `.exe` z lokalnej cesty cez ikonu ciela;
//! * subor: ikona **typu** podla pripony (jedna polozka cache na priponu, bez pristupu k suboru); len `.exe`, `.ico`, `.url`, `.lnk` a
//!   podobne, ktore nesu vlastnu ikonu, sa citaju zo suboru - a len ak je lokalny a plne dostupny;
//! * priecinok: standardna ikona priecinka;
//! * nastavenia, akcie transom, poznamky, Polica: vstavane vektorove ikony (`transom_render::resulticons`).
//!
//! Sietove cesty (UNC, namapovane sietove disky), cloudove (len online) a chybajuce subory sa nikdy nepytaju z disku: ikona sa
//! odvodi z pripony. Nahlady obrazkov sa vo vysledkoch zamerne nepouzivaju (nacitanie obsahu suborov nie je lacne); cesty
//! a nazvy sa nelogu.
use std::any::Any;
use std::sync::{Arc, Mutex};
use transom_core::loader::{Bitmap, Decode, Loader, LoaderCfg, Lookup};
use transom_launcher::{Item, Kind, Target};
use transom_platform::shellicon::{self, ext_of, IconSpec};
use transom_render::resulticons::ResultIcon;

/// Najviac ikon v pamati (bajty RGBA).
const BUDGET: usize = 6 * 1024 * 1024;
const MAX_QUEUE: usize = 32;
const APPS_PREFIX: &str = "shell:AppsFolder\\";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IconKey {
    pub spec: IconSpec,
    pub px: u16,
}

/// Ikona na nakreslenie: hotove pixely (RGBA, presna velkost) alebo vstavana zastupna.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    Pixels(Arc<Bitmap>),
    Builtin(ResultIcon),
}

/// Zdroj ikon pre kreslenie vysledkov. `begin` / `end` obaluju jedno kreslenie: poziadavky z neho sa zlucia a nahradia zoznam
/// cakajucich nacitani (zastarane, uz nevidene riadky sa nenacitavaju).
pub trait IconSource {
    fn begin(&self) {}
    fn icon(&self, item: &Item, px: u32) -> Icon;
    fn end(&self) {}
}

/// Vstavana ikona pre druh vysledku (zastupna, kym sa systemova nenacita alebo ak sa nepodari).
pub fn builtin_for(kind: Kind) -> ResultIcon {
    match kind {
        Kind::App => ResultIcon::App,
        Kind::File => ResultIcon::File,
        Kind::Folder => ResultIcon::Folder,
        Kind::Setting => ResultIcon::Setting,
        Kind::Action => ResultIcon::Action,
        Kind::Note => ResultIcon::Note,
        Kind::Shelf => ResultIcon::Shelf,
    }
}

/// Pripony, ktore nesu vlastnu ikonu (cita sa zo suboru).
fn carries_own_icon(ext: &str) -> bool {
    matches!(
        ext,
        ".exe" | ".ico" | ".scr" | ".cpl" | ".url" | ".appref-ms"
    )
}

fn sane_path(p: &str) -> bool {
    p.chars().count() <= 1000 && transom_tools::names::check_path(p).is_ok()
}

/// Odkial sa ikona vysledku berie (`None` = vstavana ikona). Nic sa neotvara ani nestatuje (to robi az pracovne vlakno).
pub fn spec_for(item: &Item) -> Option<IconSpec> {
    match item.kind {
        Kind::Setting | Kind::Action | Kind::Note | Kind::Shelf => None,
        Kind::Folder => Some(IconSpec::Folder),
        Kind::App | Kind::File => {
            let Target::Path(p) = &item.target else {
                return None;
            };
            if let Some(id) = p.strip_prefix(APPS_PREFIX) {
                let ok = !id.is_empty()
                    && id.chars().count() <= 1000
                    && !id.chars().any(|c| c.is_control());
                return ok.then(|| IconSpec::Apps(p.clone()));
            }
            if !sane_path(p) {
                return None;
            }
            let ext = ext_of(p);
            Some(if ext == ".lnk" {
                IconSpec::Shortcut(p.clone())
            } else if carries_own_icon(&ext) {
                IconSpec::Item(p.clone())
            } else {
                IconSpec::Ext(ext)
            })
        }
    }
}

const ATTR_CLOUD: u32 = 0x1000 | 0x4_0000 | 0x40_0000;

/// Na pracovnom vlakne: co sa naozaj nacita. Sietove, chybajuce, nedostupne, cloudove a nie-subory sa znizia na ikonu podla
/// pripony (bez ich citania); ostatne ostanu.
pub fn effective(spec: &IconSpec) -> IconSpec {
    let (IconSpec::Item(p) | IconSpec::Shortcut(p)) = spec else {
        return spec.clone();
    };
    let is_link = matches!(spec, IconSpec::Shortcut(_));
    let fallback = || {
        let e = ext_of(p);
        IconSpec::Ext(if e.is_empty() && is_link {
            ".lnk".into()
        } else {
            e
        })
    };
    if transom_tools::lane::is_remote_path(p) {
        return fallback();
    }
    match std::fs::symlink_metadata(p) {
        Ok(md) if md.is_file() && !is_cloud(&md) => spec.clone(),
        _ => fallback(),
    }
}

fn is_cloud(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & ATTR_CLOUD != 0
}

struct ComGuard(bool);

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: parter k uspesnemu `CoInitializeEx` na tomto vlakne.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

struct IconDecoder;

impl Decode<IconKey> for IconDecoder {
    fn init(&self) -> Box<dyn Any> {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        // SAFETY: COM (STA) pre pracovne vlakno, kym zije; shell s nim pracuje najlepsie.
        Box::new(ComGuard(
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok(),
        ))
    }

    fn decode(&self, k: &IconKey) -> Option<Bitmap> {
        let px = shellicon::load(&effective(&k.spec), u32::from(k.px))?;
        Bitmap::new(px.w, px.h, px.rgba)
    }
}

pub struct IconRt {
    loader: Loader<IconKey>,
    frame: Mutex<Vec<IconKey>>,
}

impl IconRt {
    /// `wake` sa zavola z pracovneho vlakna po kazdej nacitanej ikone (volajuci ho zluci na jedno prekreslenie).
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>) -> IconRt {
        IconRt {
            loader: Loader::start(
                LoaderCfg {
                    name: "transom-icons",
                    // volania shellu sa aj tak radia za sebou (viz `shellicon::load`), viac vlakien by nepomohlo
                    threads: 1,
                    budget_bytes: BUDGET,
                    max_queue: MAX_QUEUE,
                    max_failed: 1024,
                },
                Arc::new(IconDecoder),
                wake,
            ),
            frame: Mutex::new(Vec::new()),
        }
    }

    /// Nove otvorenie okna: skus znova to, co predtym zlyhalo (zlyhanie mohlo byt docasne).
    pub fn on_open(&self) {
        self.loader.forget_failed();
    }

    pub fn cached_len(&self) -> usize {
        self.loader.cached_len()
    }
}

impl IconSource for IconRt {
    fn begin(&self) {
        self.frame.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn icon(&self, item: &Item, px: u32) -> Icon {
        let fallback = Icon::Builtin(builtin_for(item.kind));
        let Some(spec) = spec_for(item) else {
            return fallback;
        };
        let key = IconKey {
            spec,
            px: px.clamp(1, shellicon::MAX_PX) as u16,
        };
        match self.loader.get(&key) {
            Lookup::Ready(b) => Icon::Pixels(b),
            Lookup::Failed => fallback,
            Lookup::Pending | Lookup::Absent => {
                let mut f = self.frame.lock().unwrap_or_else(|e| e.into_inner());
                if !f.contains(&key) {
                    f.push(key);
                }
                fallback
            }
        }
    }

    fn end(&self) {
        let keys = std::mem::take(&mut *self.frame.lock().unwrap_or_else(|e| e.into_inner()));
        self.loader.want(&keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn item(kind: Kind, title: &str, target: Target) -> Item {
        Item::new(title, "", kind, target)
    }

    fn path(kind: Kind, p: &str) -> Item {
        item(kind, "x", Target::Path(p.into()))
    }

    #[test]
    fn every_kind_gets_a_sensible_source_and_hostile_or_remote_paths_never_reach_the_disk() {
        // aplikacie zo zlozky Aplikacie a z odkazov
        assert_eq!(
            spec_for(&path(
                Kind::App,
                r"shell:AppsFolder\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"
            )),
            Some(IconSpec::Apps(
                r"shell:AppsFolder\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App".into()
            ))
        );
        assert_eq!(
            spec_for(&path(Kind::App, r"C:\Menu\Notepad.lnk")),
            Some(IconSpec::Shortcut(r"C:\Menu\Notepad.lnk".into()))
        );
        assert_eq!(
            spec_for(&path(Kind::App, r"C:\Tools\tool.EXE")),
            Some(IconSpec::Item(r"C:\Tools\tool.EXE".into()))
        );
        // subory: ikona podla pripony (jedna polozka cache na priponu), vlastnu ikonu nesu len vybrane pripony
        assert_eq!(
            spec_for(&path(Kind::File, r"C:\docs\Report.DOCX")),
            Some(IconSpec::Ext(".docx".into()))
        );
        assert_eq!(
            spec_for(&path(Kind::File, r"C:\docs\noext")),
            Some(IconSpec::Ext(String::new()))
        );
        assert_eq!(
            spec_for(&path(Kind::File, r"C:\docs\app.exe")),
            Some(IconSpec::Item(r"C:\docs\app.exe".into()))
        );
        assert_eq!(
            spec_for(&path(Kind::Folder, r"C:\docs")),
            Some(IconSpec::Folder)
        );
        // vstavane ikony: nastavenia, akcie, poznamky, Polica; nikdy zdroj zo systemu
        for (k, t) in [
            (Kind::Setting, Target::Uri("ms-settings:display")),
            (Kind::Action, Target::Action("calendar")),
            (Kind::Note, Target::Note("n1".into())),
            (Kind::Shelf, Target::Shelf("s1".into())),
        ] {
            assert_eq!(spec_for(&item(k, "x", t)), None, "{k:?}");
        }
        // nebezpecne alebo nezmyselne cesty: ziadny zdroj (vstavana ikona), nikdy zariadenie
        for bad in [
            "",
            "relative.exe",
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\x",
            "C:\\a\u{7}b.exe",
            r"shell:AppsFolder\",
            &format!("C:\\{}.exe", "x".repeat(1100)),
        ] {
            assert_eq!(spec_for(&path(Kind::File, bad)), None, "{bad:?}");
            assert_eq!(spec_for(&path(Kind::App, bad)), None, "{bad:?}");
        }
        assert_eq!(
            spec_for(&path(Kind::Folder, "nonsense")),
            Some(IconSpec::Folder),
            "priecinok ma ikonu bez ohladu na cestu (nic sa neotvara)"
        );
        // pracovne vlakno: siet, chybajuci subor a priecinok pod menom suboru sa znizia na ikonu podla pripony
        let remote = IconSpec::Item(r"\\nas\share\tool.exe".into());
        assert_eq!(effective(&remote), IconSpec::Ext(".exe".into()));
        let missing = IconSpec::Shortcut(r"C:\transom-no-such-dir\a.lnk".into());
        assert_eq!(effective(&missing), IconSpec::Ext(".lnk".into()));
        let dir = std::env::temp_dir();
        assert_eq!(
            effective(&IconSpec::Item(dir.to_string_lossy().to_string())),
            IconSpec::Ext(String::new()),
            "priecinok nie je subor"
        );
        assert_eq!(effective(&IconSpec::Folder), IconSpec::Folder);
    }

    fn rt() -> (IconRt, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        (
            IconRt::start(Arc::new(move || {
                w.fetch_add(1, Ordering::SeqCst);
            })),
            wakes,
        )
    }

    fn settle(rt: &IconRt, items: &[Item], px: u32) -> Vec<Icon> {
        let t = Instant::now();
        loop {
            rt.begin();
            let v: Vec<Icon> = items.iter().map(|i| rt.icon(i, px)).collect();
            rt.end();
            let loading = v
                .iter()
                .zip(items)
                .any(|(ic, it)| matches!(ic, Icon::Builtin(_)) && spec_for(it).is_some());
            if !loading || t.elapsed() > Duration::from_secs(15) {
                return v;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ikony pre zastupcov vsetkych druhov na skutocnom systeme: odkaz, spustitelny subor, bezne typy, priecinok, chybajuci subor,
    /// poskodeny a zamknuty subor. Pracuje len v scratch priecinku a so systemovymi subormi na citanie.
    #[test]
    fn real_icons_load_asynchronously_for_shortcuts_executables_file_types_folders_missing_and_locked_files(
    ) {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
        // SAFETY: COM pre testovacie vlakno (vytvorenie odkazu).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let d = std::env::temp_dir().join(format!("transom-icon-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Folder")).unwrap();
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let notepad = format!(r"{sys}\System32\notepad.exe");
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let lnk = d.join("Notepad.lnk");
        // SAFETY: vytvorenie odkazu v scratch priecinku.
        unsafe {
            let l: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            l.SetPath(PCWSTR(wide(&notepad).as_ptr())).unwrap();
            let f: IPersistFile = l.cast().unwrap();
            f.Save(PCWSTR(wide(lnk.to_str().unwrap()).as_ptr()), true)
                .unwrap();
        }
        std::fs::write(d.join("note.txt"), b"hello").unwrap();
        std::fs::write(d.join("pic.png"), b"x").unwrap();
        std::fs::write(
            d.join("junk.exe"),
            (0..3000u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>(),
        )
        .unwrap();
        std::fs::write(d.join("locked.docx"), b"x").unwrap();
        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(d.join("locked.docx"))
            .unwrap();
        let p = |n: &str| d.join(n).to_string_lossy().to_string();
        let items = vec![
            path(Kind::App, lnk.to_str().unwrap()),
            path(Kind::App, &notepad),
            path(Kind::File, &p("note.txt")),
            path(Kind::File, &p("pic.png")),
            path(Kind::Folder, &p("Folder")),
            path(Kind::File, r"C:\transom-no-such-dir\gone.pdf"),
            path(Kind::App, r"C:\transom-no-such-dir\gone.exe"),
            path(Kind::File, &p("junk.exe")),
            path(Kind::File, &p("locked.docx")),
            path(Kind::File, r"\\transom-no-such-server\share\x.zip"),
        ];
        let (rt, wakes) = rt();
        // prve kreslenie vrati hned vstavane ikony (nic sa necaka), potom prichadzaju systemove
        let t0 = Instant::now();
        rt.begin();
        let first: Vec<Icon> = items.iter().map(|i| rt.icon(i, 24)).collect();
        rt.end();
        assert!(
            t0.elapsed() < Duration::from_millis(100),
            "kreslenie nic nenacitava"
        );
        assert!(first.iter().all(|i| matches!(i, Icon::Builtin(_))));
        let icons = settle(&rt, &items, 24);
        for (i, ic) in icons.iter().enumerate() {
            match ic {
                Icon::Pixels(b) => {
                    assert_eq!((b.w, b.h), (24, 24), "polozka {i}: presna velkost");
                    let ink = b.rgba.iter().filter(|c| c[3] > 32).count();
                    assert!(ink > 30, "polozka {i}: ikona ma viditelny obsah ({ink})");
                }
                Icon::Builtin(_) => panic!("polozka {i}: systemova ikona sa nenacitala"),
            }
        }
        assert!(wakes.load(Ordering::SeqCst) >= 1);
        // druhe kreslenie: vsetko z cache, ziadne nove nacitanie
        let n = rt.cached_len();
        let again = settle(&rt, &items, 24);
        assert_eq!(again, icons);
        assert_eq!(rt.cached_len(), n);
        // ina velkost (iny DPI) je samostatna polozka a nacita sa ostro v svojej velkosti
        let big = settle(&rt, &items[..3], 36);
        assert!(big
            .iter()
            .all(|i| matches!(i, Icon::Pixels(b) if b.w == 36 && b.h == 36)));
        // vstavane ikony pre druhy bez systemovej ikony
        let note = item(Kind::Note, "n", Target::Note("1".into()));
        assert_eq!(rt.icon(&note, 24), Icon::Builtin(ResultIcon::Note));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_hung_icon_call_never_stalls_drawing_and_results_that_scroll_away_are_dropped() {
        // zastaraju poziadavky, ktore sa este nezacali: `end` nahradi cakajuci zoznam poslednou mnozinou viditelnych riadkov
        let (rt, _) = rt();
        let many: Vec<Item> = (0..60)
            .map(|i| path(Kind::File, &format!(r"C:\transom-no-such-dir\f{i}.ext{i}")))
            .collect();
        rt.begin();
        for it in &many {
            rt.icon(it, 24);
        }
        rt.end();
        // dalsie kreslenie uz vidi len tri riadky: poziadavky na ostatne sa zahodia (fronta je ohranicena a nenarasta)
        rt.begin();
        for it in &many[..3] {
            rt.icon(it, 24);
        }
        rt.end();
        let t = Instant::now();
        while rt.loader.pending_len() > 0 && t.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(rt.cached_len() <= MAX_QUEUE + 3, "{}", rt.cached_len());
    }
}
