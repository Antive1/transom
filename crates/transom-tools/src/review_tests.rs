//! Regresne testy z nezavislej kontroly (sukromie zaloh, vytlacanie Police, vylucene aplikacie, rury, retencia).
use crate::clipboard::{History, Outcome, Skip, Snapshot};
use crate::names::{check_path, PathReject};
use crate::notes::Notes;
use crate::shelf::{FsProbe, Shelf};
use crate::store::temp_dir;

fn cfg() -> transom_config::Tools {
    transom_config::Config::default().tools
}

#[test]
fn deleting_or_clearing_notes_leaves_no_backup_with_the_old_content() {
    let d = temp_dir("rv-bak");
    let mut n = Notes::open(d.clone(), cfg().notes);
    let a = n.create("tajna poznamka", 1).unwrap();
    n.create("druha", 2).unwrap();
    assert!(d.join("index.json.bak").exists());
    n.delete(&a).unwrap();
    assert!(
        !d.join("index.json.bak").exists(),
        ".bak by obsahovala zmazanu poznamku"
    );
    n.clear(false).unwrap();
    let main = std::fs::read_to_string(d.join("index.json")).unwrap();
    assert!(!main.contains("tajna") && !d.join("index.json.bak").exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_rejected_drop_never_evicts_existing_shelf_items() {
    let d = temp_dir("rv-shelf");
    let src = temp_dir("rv-shelf-src");
    let mut c = cfg().shelf;
    c.max_items = 2;
    let mut s = Shelf::open(d.clone(), c);
    for n in ["a.txt", "b.txt"] {
        std::fs::write(src.join(n), b"x").unwrap();
    }
    let paths: Vec<String> = ["a.txt", "b.txt"]
        .iter()
        .map(|n| src.join(n).to_string_lossy().to_string())
        .collect();
    assert_eq!(s.add_paths(&paths, false, &FsProbe, 1).added, 2);
    // priecinok s zapnutou kopiou sa odmietne a nic nevytlaci
    std::fs::create_dir_all(src.join("dir")).unwrap();
    let r = s.add_paths(
        &[src.join("dir").to_string_lossy().to_string()],
        true,
        &FsProbe,
        2,
    );
    assert_eq!((r.added, r.rejected.len()), (0, 1));
    assert_eq!(s.rows(&FsProbe).len(), 2, "polozky ostali");
    // prijata polozka vytlaci najstarsiu
    std::fs::write(src.join("c.txt"), b"x").unwrap();
    assert_eq!(
        s.add_paths(
            &[src.join("c.txt").to_string_lossy().to_string()],
            false,
            &FsProbe,
            3
        )
        .added,
        1
    );
    assert_eq!(s.rows(&FsProbe).len(), 2);
    let _ = std::fs::remove_dir_all(d);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn with_excluded_apps_an_unknown_owner_is_not_captured_and_zero_retention_keeps_entries() {
    let d = temp_dir("rv-clip");
    let mut c = cfg().clipboard;
    c.enabled = true;
    c.excluded_apps = vec!["keepass.exe".into()];
    c.retention_days = 0;
    let mut h = History::open(d.clone(), c.clone());
    let snap = |app: Option<&str>| Snapshot {
        text: Some("heslo".into()),
        owner_exe: app.map(str::to_string),
        ..Default::default()
    };
    assert_eq!(
        h.capture(&snap(None), 1),
        Outcome::Skipped(Skip::ExcludedApp),
        "neznamy vlastnik = fail-closed"
    );
    assert!(matches!(
        h.capture(&snap(Some("notepad.exe")), 1),
        Outcome::Stored(_)
    ));
    assert_eq!(h.enforce(10_000_000), 0, "retencia 0 = ponechat");
    assert_eq!(h.entries().len(), 1);
    // bez vylucenych aplikacii sa neznamy vlastnik zachytava
    c.excluded_apps.clear();
    let mut h2 = History::open(temp_dir("rv-clip2"), c);
    assert!(matches!(h2.capture(&snap(None), 1), Outcome::Stored(_)));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn named_pipes_behind_a_unc_host_are_refused_but_normal_shares_are_not() {
    assert_eq!(
        check_path(r"\\localhost\pipe\evil"),
        Err(PathReject::Device)
    );
    assert_eq!(
        check_path(r"\\LOCALHOST\PIPE\evil"),
        Err(PathReject::Device)
    );
    assert!(check_path(r"\\server\share\doc.txt").is_ok());
}

#[test]
fn unc_named_pipes_are_refused_in_every_spelling() {
    for p in [
        r"\\?\UNC\localhost\pipe\x",
        r"\\localhost/pipe/x",
        "//localhost/pipe/x",
        r"\\?\unc\Localhost\PIPE\x",
        r"\\srv\mailslot\x",
    ] {
        assert_eq!(check_path(p), Err(PathReject::Device), "{p}");
    }
    assert!(check_path(r"\\?\UNC\server\share\file.txt").is_ok());
}

#[test]
fn identical_file_lists_are_deduplicated_in_the_clipboard_history() {
    let d = temp_dir("rv-files");
    let src = temp_dir("rv-files-src");
    let f = src.join("a.txt");
    std::fs::write(&f, b"x").unwrap();
    let mut c = cfg().clipboard;
    c.enabled = true;
    let mut h = History::open(d.clone(), c);
    let snap = Snapshot {
        files: Some(vec![f.to_string_lossy().to_string()]),
        ..Default::default()
    };
    assert!(matches!(h.capture(&snap, 1), Outcome::Stored(_)));
    assert!(matches!(h.capture(&snap, 2), Outcome::Duplicate(_)));
    assert_eq!(h.entries().len(), 1);
    let _ = std::fs::remove_dir_all(d);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn editing_a_note_or_enforcing_retention_leaves_no_old_content_in_the_backup() {
    let d = temp_dir("rv-bak2");
    let mut n = Notes::open(d.clone(), cfg().notes);
    let a = n.create("heslo: tajne123", 1).unwrap();
    n.update(&a, "heslo odstranene", 2).unwrap();
    let bak = std::fs::read_to_string(d.join("index.json.bak")).unwrap_or_default();
    assert!(
        !bak.contains("tajne123"),
        "zaloha nesmie niest povodny text"
    );
    assert!(!std::fs::read_to_string(d.join("index.json"))
        .unwrap()
        .contains("tajne123"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn shelf_retention_is_persisted_orphan_copies_are_swept_and_pinned_rows_come_first() {
    let d = temp_dir("rv-shelf2");
    let src = temp_dir("rv-shelf2-src");
    let mut c = cfg().shelf;
    c.retention_days = 1;
    let mut s = Shelf::open(d.clone(), c.clone());
    let mut paths = Vec::new();
    for n in ["old.txt", "new.txt", "pinned.txt"] {
        std::fs::write(src.join(n), b"data").unwrap();
        paths.push(src.join(n).to_string_lossy().to_string());
    }
    s.add_paths(&paths[..1], true, &FsProbe, 0);
    s.add_paths(&paths[1..], true, &FsProbe, 3 * 86_400);
    let pin_id = s
        .items()
        .iter()
        .find(|i| i.name == "pinned.txt")
        .unwrap()
        .id
        .clone();
    s.toggle_pin(&pin_id);
    s.save().unwrap();
    assert_eq!(s.rows(&FsProbe)[0].name, "pinned.txt", "pripnute ako prve");
    // retencia zmaze stary subor aj zaznam a ulozi sa (a zmaze sa i zaloha so starym nazvom)
    assert_eq!(s.enforce(3 * 86_400 + 10), 1);
    s.save().unwrap();
    assert!(!std::fs::read_to_string(d.join("index.json"))
        .unwrap()
        .contains("old.txt"));
    assert!(!d.join("index.json.bak").exists());
    // osirotena kopia (adresar s nasim id bez zaznamu) sa zmaze pri otvoreni, cudzi adresar nie
    std::fs::create_dir_all(d.join("files").join("0123456789abcdef")).unwrap();
    std::fs::write(d.join("files").join("0123456789abcdef").join("x.bin"), b"o").unwrap();
    std::fs::create_dir_all(d.join("files").join("moje-subory")).unwrap();
    let s2 = Shelf::open(d.clone(), c);
    assert!(!d.join("files").join("0123456789abcdef").exists());
    assert!(d.join("files").join("moje-subory").exists());
    assert_eq!(s2.len(), 2);
    let _ = std::fs::remove_dir_all(d);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn clearing_disabled_notes_works() {
    // vypnute poznamky sa daju vymazat
    let dn = temp_dir("rv-notes-off");
    let mut n = Notes::open(dn.clone(), cfg().notes);
    n.create("a", 1).unwrap();
    let mut off = cfg().notes;
    off.enabled = false;
    let mut n2 = Notes::open(dn.clone(), off);
    assert_eq!(n2.clear(false), Ok(1));
    let _ = std::fs::remove_dir_all(dn);
}
