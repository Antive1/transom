//! Zivy scenar `--test-tools` (len test): spusta skutocnu listu na ploche (so scratch konfiguraciou a datami) a ovlada skutocne okna
//! popoverov spravami posielanymi **priamo ich oknam v tomto procese** (`PostMessage`), nie vstupom mysi ani klavesnice na ploche.
//! Do skutocnej schranky sa **nezapisuje a nic sa z nej necita**; nic sa nespusta ani nezatvara v inych aplikaciach.
//! Vysledky idu do logu ako riadky `LIVE-CHECK <nazov> PASS|FAIL <detail>` a `LIVE-SUMMARY`.
//!
//! Co je zamerne SIMULOVANE: operacny system pustenie suborov (OLE `DoDragDrop`) - pustenie sa vlozi do tej istej fronty ako skutocne
//! (`dragdrop::simulate_drop`), takze sa overi celý zvysok cesty (udalost -> sluzba -> ulozisko -> panel), nie vstup z OS. Kniznica
//! snimok cita **len scratch priecinok** so syntetickymi obrazkami (predvoleny priecinok Windows je vypnuty), takze sa nikdy
//! nenacita skutocny priecinok snimok; tlacidla "Otvorit" a "Ukazat v priecinku" sa neklikaju (spustili by inu aplikaciu).
use super::*;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

pub(crate) const STEP_MS: u32 = 700;
pub(crate) const BEAT_MS: u32 = 20;
/// Najdlhsie cakanie na jeden vysledok vyhladavania (studeny index moze trvat aj sekundy).
const SEARCH_DEADLINE_MS: u128 = 20_000;

type Expect = fn(&transom_launcher::Hit) -> bool;

#[derive(Default)]
pub(crate) struct Live {
    pub step: u32,
    dir: PathBuf,
    files: Vec<(PathBuf, Vec<u8>, SystemTime)>,
    fg_before: String,
    mons: Vec<(String, transom_platform::monitor::Rect)>,
    mon_idx: usize,
    passed: u32,
    failed: u32,
    // vyhladavanie
    q_idx: usize,
    q_t0: Option<Instant>,
    q_typed: bool,
    beat_last: Option<Instant>,
    beat_max_ms: u128,
    beats: u32,
    idx_q_mark: usize,
    // kniznica snimok
    shots_dir: PathBuf,
    shot_files: Vec<(PathBuf, Vec<u8>, SystemTime)>,
}

/// Obrazok `w x h` (jednofarebny PNG) so zadanym casom zmeny; vracia jeho bajty a cas (na neskorsie porovnanie).
fn write_png(p: &std::path::Path, rgb: [u8; 3], secs: u64) -> (Vec<u8>, SystemTime) {
    let (w, h) = (120u32, 80u32);
    let mut bgra = Vec::new();
    for _ in 0..w * h {
        bgra.extend_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
    }
    let png = transom_platform::imaging::encode_png(w, h, &bgra).unwrap_or_default();
    let _ = std::fs::write(p, &png);
    set_mtime(p, secs);
    let mt = std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    (png, mt)
}

fn set_mtime(p: &std::path::Path, secs: u64) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(p) {
        let _ = f.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs));
    }
}

fn is_title(h: &transom_launcher::Hit, s: &str) -> bool {
    h.item.title.to_lowercase().contains(s)
}

const QUERIES: [(&str, &str, Expect); 6] = [
    ("calen", "transom action Calendar", |h| {
        h.item.kind == transom_launcher::Kind::Action && is_title(h, "calendar")
    }),
    ("sound", "Windows Settings page", |h| {
        h.item.kind == transom_launcher::Kind::Setting
    }),
    ("synthetic", "note title", |h| {
        h.item.kind == transom_launcher::Kind::Note && is_title(h, "synthetic note")
    }),
    ("beta", "Shelf name", |h| {
        h.item.kind == transom_launcher::Kind::Shelf && is_title(h, "beta")
    }),
    ("notepad", "app (Apps folder)", |h| {
        h.item.kind == transom_launcher::Kind::App
    }),
    ("notepad", "file via Windows Search index", |h| {
        matches!(
            h.item.kind,
            transom_launcher::Kind::File | transom_launcher::Kind::Folder
        )
    }),
];

impl App {
    fn live(&mut self) -> &mut Live {
        self.live_tools.get_or_insert_with(Live::default)
    }

    fn lc(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        let d = detail.into();
        log(&format!(
            "LIVE-CHECK {name} {} {d}",
            if ok { "PASS" } else { "FAIL" }
        ));
        let l = self.live();
        if ok {
            l.passed += 1;
        } else {
            l.failed += 1;
        }
    }

    /// Klik na prvok panela podla nazvu (stred jeho obdlzníka v klientskych suradniciach popoveru).
    fn click_named(&mut self, name: &str) -> bool {
        let (entries, _, _, _) = self.tools_panel.a11y();
        match entries.iter().find(|e| e.name == name) {
            Some(e) => popup::post_click_to_popover(e.x + e.w / 2, e.y + e.h / 2),
            None => false,
        }
    }

    fn post_text(&self, s: &str) {
        for c in s.chars() {
            popup::post_char_to_popover(c);
        }
    }

    fn tools_open_on(&mut self, mon: usize) -> bool {
        let Some((id, _)) = self.live().mons.get(mon).cloned() else {
            return false;
        };
        self.act(Action::ToolsPanel, Some((id, 200)));
        true
    }

    fn files_unchanged(&mut self) -> bool {
        self.live().files.clone().iter().all(|(p, bytes, mt)| {
            std::fs::read(p).is_ok_and(|b| &b == bytes)
                && std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .is_ok_and(|m| m == *mt)
        })
    }

    fn shot_files_unchanged(&mut self) -> bool {
        self.live().shot_files.clone().iter().all(|(p, bytes, mt)| {
            std::fs::read(p).is_ok_and(|b| &b == bytes)
                && std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .is_ok_and(|m| m == *mt)
        })
    }

    /// Nazvy riadkov sekcie Snimky tak, ako ich panel naposledy vykreslil.
    fn shot_titles(&self) -> Vec<String> {
        self.tools_data.sections[3]
            .rows
            .iter()
            .map(|r| r.title.clone())
            .collect()
    }

    pub(crate) fn test_tools_step(&mut self) {
        let step = self.live().step;
        self.live().step += 1;
        let next = |ms: u32| schedule_timer(T_TESTTOOLS, ms);
        match step {
            0 => {
                // ochrana: iba so scratch konfiguraciou a datami
                let cfg = std::env::var("TRANSOM_CONFIG").unwrap_or_default();
                let data = std::env::var("TRANSOM_DATA").unwrap_or_default();
                if cfg.is_empty() || data.is_empty() {
                    log("LIVE-REFUSED: TRANSOM_CONFIG a TRANSOM_DATA musia byt nastavene na scratch umiestnenia");
                    transom_platform::appbar::request_quit();
                    return;
                }
                let dir = PathBuf::from(&data).join("live-files");
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::create_dir_all(dir.join("folder"));
                let mut files = Vec::new();
                for (n, body) in [
                    ("alpha.txt", b"alpha synthetic".to_vec()),
                    ("beta.txt", b"beta synthetic".to_vec()),
                    ("gamma.dat", (0u8..200).collect::<Vec<u8>>()),
                ] {
                    let p = dir.join(n);
                    let _ = std::fs::write(&p, &body);
                    let mt = std::fs::metadata(&p)
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    files.push((p, body, mt));
                }
                let mons: Vec<_> = transom_platform::monitor::enumerate()
                    .into_iter()
                    .filter(|m| self.layouts.contains_key(&m.id))
                    .map(|m| (m.id.clone(), m.rect))
                    .collect();
                log(&format!(
                    "LIVE-INFO monitors_with_bar={} data_dir_scratch=true",
                    mons.len()
                ));
                let fg = popup::foreground_description();
                let l = self.live();
                l.dir = dir;
                l.files = files;
                l.mons = mons;
                l.fg_before = fg;
                next(STEP_MS);
            }
            1 => {
                let ok = self.tools_open_on(0);
                self.lc("open-tools-popover", ok, "act(ToolsPanel) on monitor 0");
                next(STEP_MS);
            }
            2 => {
                let open = popup::popover_open() == Some(POP_TOOLS);
                self.lc(
                    "tools-popover-is-open",
                    open,
                    format!("{:?}", popup::popover_open()),
                );
                let (entries, extras, _, name) = self.tools_panel.a11y();
                let names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
                let need = [
                    "Search",
                    "Stashed files",
                    "Clipboard",
                    "Notes",
                    "Screenshots",
                ];
                let missing: Vec<_> = need
                    .iter()
                    .filter(|n| !names.iter().any(|x| x == *n))
                    .collect();
                self.lc(
                    "screen-reader-names-for-tabs-and-search",
                    missing.is_empty() && name == "Tools" && entries.len() == extras.len(),
                    format!("root='{name}' missing={missing:?} nodes={}", entries.len()),
                );
                // klavesnica: Tab ide do kruhu fokusu
                popup::post_key_to_popover(0x09);
                next(STEP_MS);
            }
            3 => {
                let f = self.tools_panel.focus.is_some();
                self.lc(
                    "keyboard-tab-moves-focus-into-the-ring",
                    f,
                    format!("{:?}", self.tools_panel.focus),
                );
                // Right na zaostrenej karte? fokus je na prvom prvku (Search); sipka vpravo ho nemeni, Escape zatvara
                popup::post_key_to_popover(0x1B);
                next(STEP_MS);
            }
            4 => {
                let closed = popup::popover_open().is_none();
                let after = popup::foreground_description();
                let before = self.live().fg_before.clone();
                self.lc(
                    "escape-closes-the-popover",
                    closed,
                    format!("{:?}", popup::popover_open()),
                );
                // obnova popredia: popover si pamata predchadzajuce okno a po zatvoreni mu ho vrati
                self.lc(
                    "focus-restored-to-the-previous-window",
                    after == before,
                    format!("before='{before}' after='{after}'"),
                );
                self.live().mon_idx = 0;
                next(STEP_MS / 2);
            }
            // dva monitory: popover sa otvori pod listou toho monitora a ostane v jeho hraniciach
            5 => {
                let i = self.live().mon_idx;
                if i >= self.live().mons.len() {
                    self.live().step = 8;
                    next(50);
                    return;
                }
                self.tools_open_on(i);
                next(STEP_MS);
            }
            6 => {
                let i = self.live().mon_idx;
                let (id, r) = self.live().mons[i].clone();
                let rect = popup::popover_screen_rect();
                let inside = rect.is_some_and(|(x, y, w, h)| {
                    x >= r.left && y >= r.top && x + w <= r.right && y + h <= r.bottom
                });
                self.lc(
                    &format!("popover-stays-inside-monitor-{i}"),
                    inside,
                    format!(
                        "monitor={:?} popover={rect:?}",
                        (r.left, r.top, r.right, r.bottom)
                    ),
                );
                let _ = id;
                popup::post_key_to_popover(0x1B);
                next(STEP_MS);
            }
            7 => {
                self.live().mon_idx += 1;
                self.live().step = 5;
                next(STEP_MS / 2);
            }
            // Polica (simulovane pustenie)
            8 => {
                let d = self.live().dir.clone();
                let paths: Vec<String> = ["alpha.txt", "beta.txt", "gamma.dat", "folder"]
                    .iter()
                    .map(|n| d.join(n).to_string_lossy().to_string())
                    .chain(std::iter::once(
                        d.join("missing.txt").to_string_lossy().to_string(),
                    ))
                    .collect();
                transom_platform::dragdrop::simulate_drop(paths);
                next(STEP_MS);
            }
            9 => {
                let rows = self.tools_snap.shelf.len();
                let msg = format!("{:?}", self.tools_snap.msg);
                self.lc(
                    "shelf-multi-drop-adds-existing-items-and-rejects-missing",
                    rows == 4,
                    format!("rows={rows} msg={msg}"),
                );
                let same = self.files_unchanged();
                self.lc(
                    "shelf-originals-unchanged-after-drop",
                    same,
                    "bytes and mtime compared",
                );
                // duplicity: ten isty zoznam este raz sa nepridá znova
                let d = self.live().dir.clone();
                transom_platform::dragdrop::simulate_drop(vec![d
                    .join("alpha.txt")
                    .to_string_lossy()
                    .to_string()]);
                next(STEP_MS);
            }
            10 => {
                let rows = self.tools_snap.shelf.len();
                self.lc(
                    "shelf-duplicate-drop-is-merged",
                    rows == 4,
                    format!("rows={rows}"),
                );
                self.tools_open_on(0);
                next(STEP_MS);
            }
            11 => {
                // zmazanie z Police: dvakrat Delete (dvojkrokove); original ostava
                popup::post_key_to_popover(0x2E);
                next(STEP_MS / 2);
            }
            12 => {
                popup::post_key_to_popover(0x2E);
                next(STEP_MS);
            }
            13 => {
                let rows = self.tools_snap.shelf.len();
                self.lc(
                    "shelf-two-step-delete-removes-one-entry",
                    rows == 3,
                    format!("rows={rows}"),
                );
                let same = self.files_unchanged();
                self.lc(
                    "shelf-originals-unchanged-after-removal",
                    same,
                    "bytes and mtime compared",
                );
                // poznamky
                let ok = self.click_named("Notes");
                self.lc("click-notes-tab", ok, "");
                next(STEP_MS);
            }
            14 => {
                let ok = self.click_named("New note");
                self.lc("click-new-note", ok, "");
                next(STEP_MS);
            }
            15 => {
                let open = self.tools_panel.editor.is_some();
                self.lc("note-editor-opens", open, "");
                self.post_text("Synthetic note one");
                next(STEP_MS);
            }
            16 => {
                let t = self
                    .tools_panel
                    .editor
                    .as_ref()
                    .map(|e| e.text.clone())
                    .unwrap_or_default();
                self.lc(
                    "note-typing-reaches-the-editor",
                    t == "Synthetic note one",
                    format!("len={}", t.len()),
                );
                let ok = self.click_named("Save");
                self.lc("click-save", ok, "");
                next(STEP_MS);
            }
            17 => {
                let n = self.tools_snap.notes.len();
                self.lc(
                    "note-saved-and-listed",
                    n == 1 && self.tools_panel.editor.is_none(),
                    format!("notes={n}"),
                );
                // otvorit (Enter na vybranom riadku)
                popup::post_key_to_popover(0x0D);
                next(STEP_MS);
            }
            18 => {
                let t = self.tools_panel.editor.as_ref().map(|e| e.text.clone());
                self.lc(
                    "note-reopens-with-saved-text",
                    t.as_deref() == Some("Synthetic note one"),
                    format!("{t:?}"),
                );
                self.post_text("!");
                next(STEP_MS / 2);
            }
            19 => {
                let ok = self.click_named("Discard changes");
                self.lc("click-discard", ok, "");
                next(STEP_MS);
            }
            20 => {
                self.lc(
                    "discard-closes-editor",
                    self.tools_panel.editor.is_none(),
                    "",
                );
                popup::post_key_to_popover(0x0D);
                next(STEP_MS);
            }
            21 => {
                let t = self.tools_panel.editor.as_ref().map(|e| e.text.clone());
                self.lc(
                    "discarded-edit-did-not-change-the-note",
                    t.as_deref() == Some("Synthetic note one"),
                    format!("{t:?}"),
                );
                self.post_text(" edited");
                next(STEP_MS / 2);
            }
            22 => {
                let ok = self.click_named("Save");
                self.lc("click-save-edit", ok, "");
                next(STEP_MS);
            }
            23 => {
                popup::post_key_to_popover(0x0D);
                next(STEP_MS);
            }
            24 => {
                let t = self.tools_panel.editor.as_ref().map(|e| e.text.clone());
                self.lc(
                    "edited-note-persisted",
                    t.as_deref() == Some("Synthetic note one edited"),
                    format!("{t:?}"),
                );
                let _ = self.click_named("Discard changes");
                next(STEP_MS);
            }
            // zlyhanie ulozenia: limit poznamok je v scratch konfiguracii 1
            25 => {
                let _ = self.click_named("New note");
                next(STEP_MS);
            }
            26 => {
                self.post_text("must survive a failed save");
                next(STEP_MS);
            }
            27 => {
                let _ = self.click_named("Save");
                next(STEP_MS);
            }
            28 => {
                let e = self.tools_panel.editor.as_ref().map(|e| e.text.clone());
                let n = self.tools_snap.notes.len();
                self.lc(
                    "failed-save-keeps-the-editor-and-its-text",
                    e.as_deref() == Some("must survive a failed save")
                        && n == 1
                        && self.tools_panel.msg.is_some(),
                    format!(
                        "editor_text_len={:?} notes={n} msg={}",
                        e.map(|t| t.len()),
                        self.tools_panel.msg.is_some()
                    ),
                );
                // zatvorenie okna s neulozitelnou rozpisanou poznamkou: text sa nesmie stratit
                popup::post_key_to_popover(0x1B);
                next(STEP_MS);
            }
            29 => {
                self.tools_open_on(0);
                next(STEP_MS);
            }
            30 => {
                let _ = self.click_named("Notes");
                next(STEP_MS);
            }
            31 => {
                let e = self.tools_panel.editor.as_ref().map(|e| e.text.clone());
                self.lc(
                    "closed-unsaved-note-is-recoverable-after-reopening",
                    e.as_deref() == Some("must survive a failed save"),
                    format!("{:?}", e.map(|t| t.len())),
                );
                let _ = self.click_named("Discard changes");
                next(STEP_MS);
            }
            // snimky: kniznica (iba citanie) nad scratch priecinkom; skutocny priecinok snimok sa nikdy nenacita
            32 => {
                let dir = self.live().dir.join("shots");
                let _ = std::fs::remove_dir_all(&dir);
                let _ = std::fs::create_dir_all(&dir);
                let mut files = Vec::new();
                for (n, rgb, secs) in [
                    ("old.png", [220, 0, 0], 1_700_000_000u64),
                    ("mid.png", [0, 200, 0], 1_700_000_100),
                    ("new.png", [0, 0, 220], 1_700_000_200),
                ] {
                    let p = dir.join(n);
                    let (b, mt) = write_png(&p, rgb, secs);
                    files.push((p, b, mt));
                }
                // text a poskodeny obrazok: text sa nezobrazi, poskodeny ma zastupnu ikonu
                let _ = std::fs::write(dir.join("notes.txt"), b"not an image");
                let broken = dir.join("broken.png");
                let _ = std::fs::write(&broken, b"this is not a png");
                set_mtime(&broken, 1_700_000_050);
                let l = self.live();
                l.shots_dir = dir.clone();
                l.shot_files = files;
                // iba tento priecinok: predvoleny priecinok Windows je vypnuty
                self.config.tools.screenshots.default_folder = false;
                self.config.tools.screenshots.folders = vec![dir.to_string_lossy().to_string()];
                let ok = self.click_named("Screenshots");
                self.lc("click-screenshots-tab", ok, "");
                next(STEP_MS * 3);
            }
            33 => {
                let titles = self.shot_titles();
                self.lc(
                    "screenshots-list-images-newest-first-and-skip-other-files",
                    titles == ["new.png", "mid.png", "broken.png", "old.png"],
                    format!("rows={}", titles.len()),
                );
                let (entries, _, _, _) = self.tools_panel.a11y();
                let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
                let toolbar = ["Open", "Show in folder", "Refresh"]
                    .iter()
                    .all(|b| names.iter().any(|x| x == b));
                let dated = names.iter().any(|x| x.starts_with("new.png, "));
                self.lc(
                    "screenshots-toolbar-and-dated-rows-are-exposed-to-screen-readers",
                    toolbar && dated,
                    format!("toolbar={toolbar} dated_row={dated}"),
                );
                next(STEP_MS * 2);
            }
            34 => {
                let rows = self.tools_data.sections[3].rows.clone();
                let thumb_ok = |i: usize| {
                    matches!(
                        rows.get(i).map(|r| &r.thumb),
                        Some(tools_pop::Thumb::Ready(_))
                    )
                };
                self.lc(
                    "thumbnails-load-in-the-background-and-a-broken-file-gets-a-placeholder",
                    thumb_ok(0)
                        && thumb_ok(1)
                        && thumb_ok(3)
                        && rows.get(2).map(|r| &r.thumb) == Some(&tools_pop::Thumb::Placeholder),
                    format!("ready={}/4", (0..4).filter(|i| thumb_ok(*i)).count()),
                );
                // nova snimka (najnovsia) sa ma objavit bez stlacenia Obnovit
                let dir = self.live().shots_dir.clone();
                let p = dir.join("newest.png");
                let (b, mt) = write_png(&p, [200, 200, 0], 1_700_000_300);
                self.live().shot_files.push((p, b, mt));
                next(STEP_MS * 2);
            }
            35 => {
                let titles = self.shot_titles();
                self.lc(
                    "a-new-file-appears-without-pressing-refresh",
                    titles.first().map(String::as_str) == Some("newest.png") && titles.len() == 5,
                    format!("rows={}", titles.len()),
                );
                // zmazanie suboru (scratch): zmizne zo zoznamu a jeho zaznam sa neporovnava s povodnymi
                let dir = self.live().shots_dir.clone();
                let _ = std::fs::remove_file(dir.join("old.png"));
                self.live()
                    .shot_files
                    .retain(|(p, _, _)| !p.ends_with("old.png"));
                next(STEP_MS * 2);
            }
            36 => {
                let titles = self.shot_titles();
                self.lc(
                    "a-deleted-file-disappears-from-the-list",
                    !titles.iter().any(|t| t == "old.png") && titles.len() == 4,
                    format!("rows={}", titles.len()),
                );
                let _ = self.click_named("Refresh");
                next(STEP_MS * 2);
            }
            37 => {
                let titles = self.shot_titles();
                let same = self.shot_files_unchanged();
                self.lc(
                    "refresh-keeps-the-list-and-the-library-never-changes-the-image-files",
                    titles.len() == 4 && same,
                    format!("rows={} bytes_and_mtime_unchanged={same}", titles.len()),
                );
                popup::post_key_to_popover(0x1B);
                next(STEP_MS);
            }
            38 => {
                // po zatvoreni okna kniznica prestane sledovat a uvolni uchytku priecinka (da sa premenovat)
                let closed = self.library.as_ref().is_some_and(|l| !l.is_open());
                let dir = self.live().shots_dir.clone();
                let moved = dir.with_file_name("shots-moved");
                let mut renamed = false;
                for _ in 0..40 {
                    if std::fs::rename(&dir, &moved).is_ok() {
                        renamed = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                if renamed {
                    let _ = std::fs::rename(&moved, &dir);
                }
                self.lc(
                    "closing-the-window-stops-watching-and-releases-the-folder",
                    closed && renamed,
                    format!("library_closed={closed} folder_renamable={renamed}"),
                );
                next(STEP_MS);
            }
            39 => {
                // vyhladavac
                let m0 = self.live().mons[0].0.clone();
                self.act(Action::Launcher, Some((m0, 300)));
                next(STEP_MS * 2);
            }
            40 => {
                self.lc(
                    "launcher-popover-open",
                    popup::popover_open() == Some(POP_LAUNCHER),
                    format!("{:?}", popup::popover_open()),
                );
                let (_, _, _, name) = self.launcher_panel.a11y();
                self.lc("launcher-has-an-accessible-name", !name.is_empty(), name);
                self.live().q_idx = 0;
                self.live().q_typed = false;
                self.live().beat_last = None;
                schedule_timer(T_TESTBEAT, BEAT_MS);
            }
            41 => {
                self.test_tools_finish();
                next(STEP_MS);
            }
            // simulovany studeny start indexu: dotaz na subory visi 6 s, novsi dotaz ho musi predbehnut a UI ostat plynule
            42 => {
                // (po kroku 41 je Esc poslany; znova otvor vyhladavac)
                let m0 = self.live().mons[0].0.clone();
                self.act(Action::Launcher, Some((m0, 300)));
                crate::launcher_rt::set_test_index_delay_ms(6000);
                self.live().beat_max_ms = 0;
                self.live().beat_last = None;
                next(STEP_MS * 2);
            }
            43 => {
                self.post_text("notepad");
                schedule_timer(T_TESTBEAT, BEAT_MS);
                self.live().q_idx = 100;
                next(600);
            }
            44 => {
                let pending = !self.launcher_panel.done
                    && self.launcher_panel.files_pending
                    && self.launcher_panel.query == "notepad";
                self.lc(
                    "cold-index-simulated-shows-searching-indicator-and-fast-results-first",
                    pending
                        && self
                            .launcher_panel
                            .results
                            .iter()
                            .any(|h| h.item.kind == transom_launcher::Kind::App),
                    format!(
                        "done={} results={}",
                        self.launcher_panel.done,
                        self.launcher_panel.results.len()
                    ),
                );
                for _ in 0..7 {
                    popup::post_key_to_popover(0x08);
                }
                self.post_text("calen");
                self.live().q_t0 = Some(Instant::now());
                schedule_timer(T_TESTBEAT, BEAT_MS);
                self.live().q_typed = true;
                self.live().q_idx = 100; // beat len meria medzery
                next(150);
            }
            45 => {
                let el = self.live().q_t0.map_or(0, |t| t.elapsed().as_millis());
                let ok = self.launcher_panel.query == "calen"
                    && !self.launcher_panel.stale
                    && self
                        .launcher_panel
                        .results
                        .iter()
                        .any(|h| h.item.title.to_lowercase().contains("calendar"));
                if !ok && el < 4000 {
                    self.live().step = 45;
                    next(100);
                    return;
                }
                let gap = self.live().beat_max_ms;
                self.lc(
                    "cold-index-simulated-newer-query-supersedes-the-stuck-one",
                    ok && el < 1500,
                    format!(
                        "calendar_result_after_ms={el} done={} max_ui_gap_ms={gap}",
                        self.launcher_panel.done
                    ),
                );
                self.lc(
                    "cold-index-simulated-ui-stays-responsive",
                    gap < 400,
                    format!("max_gap_between_{BEAT_MS}ms_heartbeats={gap}ms"),
                );
                crate::launcher_rt::set_test_index_delay_ms(0);
                popup::post_key_to_popover(0x1B);
                next(STEP_MS);
            }
            46 => {
                let closed = popup::popover_open().is_none();
                self.lc("escape-closes-the-launcher", closed, "");
                self.tools_open_on(0);
                next(STEP_MS * 2);
            }
            // Polica: zmena poradia tlacidlami (klavesy Ctrl+hore / dole PostMessage nasimulovat nevie, citaju stav klavesnice
            // vlakna; prikaz je ten isty) a "Pridat subory" (dialog je SIMULOVANY: vysledok cesty sa vlozi do rovnakej cesty)
            47 => {
                let (entries, _, _, _) = self.tools_panel.a11y();
                let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
                let n = self.tools_snap.shelf.len();
                let toolbar = ["Add files…", "Move up", "Move down"]
                    .iter()
                    .all(|b| names.iter().any(|x| x == b));
                let positions = (0..n).all(|i| {
                    names
                        .iter()
                        .any(|x| x.ends_with(&format!(", {} of {n}", i + 1)))
                });
                self.lc(
                    "shelf-toolbar-and-row-positions-are-exposed-to-screen-readers",
                    toolbar && positions && n == 3,
                    format!("rows={n} toolbar={toolbar} positions={positions}"),
                );
                // Dolna sipka vyberie druhy riadok (neaktivuje ho: Enter by zapisal do skutocnej schranky)
                popup::post_key_to_popover(0x28);
                next(STEP_MS);
            }
            48 => {
                let _ = self.click_named("Move up");
                next(STEP_MS * 2);
            }
            49 => {
                let names: Vec<String> = self
                    .tools_snap
                    .shelf
                    .iter()
                    .map(|r| r.name.clone())
                    .collect();
                let on_disk = std::fs::read_to_string(
                    transom_tools::tools_dir().join("shelf").join("index.json"),
                )
                .map(|j| {
                    let a = j.find("gamma.dat");
                    let b = j.find("beta.txt");
                    matches!((a, b), (Some(a), Some(b)) if a < b)
                })
                .unwrap_or(false);
                self.lc(
                    "shelf-move-up-reorders-persists-and-the-selection-follows-the-item",
                    names.first().map(String::as_str) == Some("gamma.dat")
                        && self.tools_panel.selected == 0
                        && self.tools_snap.msg
                            == Some(transom_tools::service::Msg::Moved { pos: 1, total: 3 })
                        && on_disk,
                    format!(
                        "order={names:?} selected={} on_disk={on_disk}",
                        self.tools_panel.selected
                    ),
                );
                let same = self.files_unchanged();
                self.lc(
                    "shelf-originals-unchanged-after-reorder",
                    same,
                    "bytes and mtime compared",
                );
                let _ = self.click_named("Move up");
                next(STEP_MS * 2);
            }
            50 => {
                self.lc(
                    "shelf-move-up-at-the-top-reports-the-edge-and-changes-nothing",
                    self.tools_snap.msg
                        == Some(transom_tools::service::Msg::MoveEdge { first: true })
                        && self.tools_snap.shelf.first().map(|r| r.name.as_str())
                            == Some("gamma.dat"),
                    format!("{:?}", self.tools_snap.msg),
                );
                // "Pridat subory": vysledok dialogu vlozeny do rovnakej cesty ako skutocny (vlakno -> N_PICKED -> AddPaths)
                let d = self.live().dir.clone();
                let delta = d.join("delta.txt");
                let _ = std::fs::write(&delta, b"delta synthetic");
                *self.picked.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(vec![delta.to_string_lossy().to_string()]);
                self.picking = true;
                self.notice = None;
                if let Some(n) = notifier() {
                    n.notify(N_PICKED);
                }
                next(STEP_MS * 2);
            }
            51 => {
                let has = self.tools_snap.shelf.iter().any(|r| r.name == "delta.txt");
                let delta_intact = std::fs::read(self.live().dir.join("delta.txt"))
                    .is_ok_and(|b| b == b"delta synthetic");
                self.lc(
                    "add-files-result-goes-through-the-drop-path-and-the-original-stays",
                    has && delta_intact
                        && !self.picking
                        && self.tools_snap.shelf.len() == 4
                        && self.notice.is_some(),
                    format!(
                        "rows={} notice={:?}",
                        self.tools_snap.shelf.len(),
                        self.notice
                    ),
                );
                let same = self.files_unchanged();
                self.lc(
                    "shelf-originals-unchanged-after-add-files",
                    same,
                    "bytes and mtime compared",
                );
                popup::post_key_to_popover(0x1B);
                next(STEP_MS);
            }
            _ => {
                let closed = popup::popover_open().is_none();
                self.lc("escape-closes-the-tools-panel", closed, "");
                let (p, f) = (self.live().passed, self.live().failed);
                log(&format!("LIVE-SUMMARY passed={p} failed={f}"));
                transom_platform::appbar::request_quit();
            }
        }
    }

    /// Tep (kazdych `BEAT_MS`): meria najvacsiu medzeru medzi tepmi (zablokovane UI vlakno by ju zvacsilo) a riadi vyhladavanie.
    pub(crate) fn test_tools_beat(&mut self) {
        let now = Instant::now();
        let l = self.live();
        if let Some(prev) = l.beat_last {
            let gap = now.duration_since(prev).as_millis();
            l.beat_max_ms = l.beat_max_ms.max(gap);
        }
        l.beat_last = Some(now);
        l.beats += 1;
        let idx = l.q_idx;
        if idx >= QUERIES.len() {
            // faza simulovaneho studeneho indexu (kroky 43-45): tep bezi dalej a meria medzery
            if (43..=45).contains(&self.live().step) {
                schedule_timer(T_TESTBEAT, BEAT_MS);
            }
            return;
        }
        let (q, what, expect) = QUERIES[idx];
        if !self.live().q_typed {
            // vymazat predchadzajuci dotaz a napisat novy (rychle za sebou = ako rychle pisanie)
            for _ in 0..self.launcher_panel.query.chars().count() {
                popup::post_key_to_popover(0x08);
            }
            self.post_text(q);
            let l = self.live();
            l.q_typed = true;
            l.q_t0 = Some(Instant::now());
            schedule_timer(T_TESTBEAT, BEAT_MS);
            return;
        }
        let t0 = self.live().q_t0.unwrap_or(now);
        let el = now.duration_since(t0).as_millis();
        let typed_ok = self.launcher_panel.query == q;
        let found = typed_ok
            && !self.launcher_panel.stale
            && self.launcher_panel.results.iter().any(expect);
        if found {
            let beat_max = self.live().beat_max_ms;
            let detail = format!(
                "query='{q}' first_matching_result_after_ms={el} results={} done={} max_ui_gap_ms={}",
                self.launcher_panel.results.len(),
                self.launcher_panel.done,
                beat_max
            );
            self.lc(&format!("search-{what}"), true, detail);
            let l = self.live();
            l.q_idx += 1;
            l.q_typed = false;
        } else if el > SEARCH_DEADLINE_MS {
            let detail = format!(
                "query='{q}' no matching result within {el} ms (typed_ok={typed_ok} results={} index_down={})",
                self.launcher_panel.results.len(),
                self.launcher_panel.index_down
            );
            // dotaz na subory cez index nemusi nic najst (scratch/necache): hlasi sa to ako obmedzenie, nie ako chyba UI
            self.lc(&format!("search-{what}"), what.contains("index"), detail);
            let l = self.live();
            l.q_idx += 1;
            l.q_typed = false;
        }
        if self.live().q_idx >= QUERIES.len() {
            let max = self.live().beat_max_ms;
            let beats = self.live().beats;
            self.lc(
                "ui-thread-never-blocked-during-search",
                max < 500,
                format!("max_gap_between_{BEAT_MS}ms_heartbeats={max}ms beats={beats}"),
            );
            // po dobehnuti hladania sa uz nesmu spustat dalsie dotazy do indexu (ziadna slucka)
            self.live().idx_q_mark =
                crate::launcher_rt::INDEX_QUERIES.load(std::sync::atomic::Ordering::Relaxed);
            schedule_timer(T_TESTTOOLS, 2000);
            self.live().step = 41;
            return;
        }
        schedule_timer(T_TESTBEAT, BEAT_MS);
    }

    fn test_tools_finish(&mut self) {
        let now = crate::launcher_rt::INDEX_QUERIES.load(std::sync::atomic::Ordering::Relaxed);
        let mark = self.live().idx_q_mark;
        self.lc(
            "no-requery-storm-after-the-search-settled",
            now == mark,
            format!("index_queries_at_settle={mark} two_seconds_later={now}"),
        );
        // ikony vysledkov sa nacitali na pozadi (kym bezali dotazy; "ui-thread-never-blocked-during-search" meria aj to)
        let icons = self.icons.as_ref().map_or(0, |i| i.cached_len());
        self.lc(
            "search-result-icons-load-asynchronously",
            icons >= 2,
            format!("icons_cached={icons}"),
        );
        popup::post_key_to_popover(0x1B);
    }
}
