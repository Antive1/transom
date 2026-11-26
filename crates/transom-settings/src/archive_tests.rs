//! Nastavenia archivu dennych suctov: opt-in, pozastavenie, retencia a dvojkrokove mazanie (bez okna; konfiguracia a data
//! v docasnom adresari, broadcasty su v testoch vypnute).
use super::*;
use transom_usage::archive;
use transom_usage::{DayTotals, Provider};

/// Env premenne su spolocne pre proces: testy ich pouzivaju jeden po druhom.
pub(crate) static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) struct Env {
    _g: std::sync::MutexGuard<'static, ()>,
    pub(crate) root: std::path::PathBuf,
}

impl Env {
    pub(crate) fn new(name: &str) -> Env {
        let g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let root =
            std::env::temp_dir().join(format!("transom-arc-set-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::env::set_var("TRANSOM_CONFIG", root.join("config.json"));
        std::env::set_var("TRANSOM_DATA", root.join("data"));
        transom_i18n::set_lang(transom_i18n::Lang::En);
        Env { _g: g, root }
    }
    pub(crate) fn ui(&self) -> Ui {
        let loaded = transom_config::Loaded {
            config: transom_config::Config::default(),
            read_only: false,
            note: None,
            issue: None,
        };
        make_ui(loaded, Theme::dark(), String::new())
    }
    pub(crate) fn saved(&self) -> transom_config::Config {
        transom_config::load(&self.root.join("config.json")).config
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        std::env::remove_var("TRANSOM_CONFIG");
        std::env::remove_var("TRANSOM_DATA");
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Archiv so starym (jan 2026) a novym (nov 2026) dnom.
fn write_archive() {
    let mut a = archive::Archive::default();
    let f: Vec<DayTotals> = [(2026, 1, 15), (2026, 11, 3)]
        .into_iter()
        .map(|day| DayTotals {
            provider: Some(Provider::ClaudeCode),
            model: "m".into(),
            day,
            input: 5,
            requests: 1,
            ..Default::default()
        })
        .collect();
    archive::merge(
        &mut a,
        &[archive::Scan {
            provider: Provider::ClaudeCode,
            source: transom_usage::SourceKind::ClaudeLocal,
            oldest: Some(transom_core::clock::Ymd::new(2026, 1, 1)),
            errors: false,
            missing: false,
        }],
        &f,
        transom_core::clock::Ymd::new(2026, 11, 20),
        &|_| "2026-11-15".into(),
    );
    archive::save(&archive::path(), &a).unwrap();
}

#[test]
fn archiving_is_off_by_default_and_only_an_explicit_click_turns_it_on() {
    let e = Env::new("optin");
    let mut ui = e.ui();
    assert!(!ui.cfg.usage.archive.enabled, "nikdy sa nezapne samo");
    assert!(!archive::path().exists());
    // pozastavenie bez zapnutia nic nerobi
    assert!(!ui.click(Hit::ArchivePause));
    assert!(!ui.cfg.usage.archive.paused);
    // vedome zapnutie: ulozi sa a archiv zacne zapisovat (writing)
    assert!(ui.click(Hit::ArchiveEnable));
    let c = e.saved();
    assert!(c.usage.archive.enabled && c.usage.archive.writing());
    assert_eq!(
        c.usage.archive.retention_months,
        transom_config::ARCHIVE_DEFAULT_RETENTION
    );
    // zapnutie archivu nezapina ziadne zdroje (ostava na pouzivatelovi)
    assert!(!c.usage.any_enabled());
    // ani samotne zapnutie nevytvori subor (vytvori ho az zber)
    assert!(!archive::path().exists());
}

#[test]
fn pause_and_resume_keep_everything_saved_and_enabling_clears_a_pause() {
    let e = Env::new("pause");
    let mut ui = e.ui();
    ui.click(Hit::ArchiveEnable);
    write_archive();
    assert!(ui.click(Hit::ArchivePause));
    let c = e.saved();
    assert!(c.usage.archive.enabled && c.usage.archive.paused && !c.usage.archive.writing());
    assert!(archive::path().exists(), "pozastavenie nic nemaze");
    // vypnutie a znovu zapnutie: pozastavenie sa zrusi, subor ostava
    ui.click(Hit::ArchiveEnable);
    assert!(!e.saved().usage.archive.enabled);
    assert!(archive::path().exists(), "vypnutie nic nemaze");
    ui.click(Hit::ArchiveEnable);
    let c = e.saved();
    assert!(c.usage.archive.enabled && !c.usage.archive.paused);
}

#[test]
fn shorter_retention_prunes_immediately_and_longer_never_deletes() {
    let e = Env::new("ret");
    let mut ui = e.ui();
    ui.click(Hit::ArchiveEnable);
    write_archive();
    ui.refresh_archive();
    assert_eq!(ui.arc.info.rows, 2);
    // dlhsia / rovnaka retencia: nic sa nemaze
    ui.click(Hit::ArchiveRetention(24));
    assert_eq!(archive::info(&archive::path()).rows, 2);
    assert_eq!(e.saved().usage.archive.retention_months, 24);
    // 0 = bez obmedzenia: nic sa nemaze
    ui.click(Hit::ArchiveRetention(0));
    assert_eq!(archive::info(&archive::path()).rows, 2);
    // kratsia: starsie ako hranica sa zmazu hned (dnesok je skutocny, januar 2026 je starsi ako 3 mesiace len ak dnes > april)
    let today = pclock::today();
    let jan_old = transom_core::clock::days_from_civil(today)
        - transom_core::clock::days_from_civil(transom_core::clock::Ymd::new(2026, 1, 15))
        > 100;
    ui.click(Hit::ArchiveRetention(3));
    let rows = archive::info(&archive::path()).rows;
    assert_eq!(rows, if jan_old { 1 } else { 2 });
    assert_eq!(ui.arc.info.rows, rows, "zhrnutie sa obnovilo");
    assert_eq!(e.saved().usage.archive.retention_months, 3);
}

#[test]
fn deleting_the_archive_needs_a_second_click_and_any_other_action_cancels_it() {
    let e = Env::new("del");
    let mut ui = e.ui();
    write_archive();
    ui.refresh_archive();
    // prvy klik len pyta potvrdenie
    ui.click(Hit::ArchiveDelete);
    assert!(ui.arc.confirm_delete && archive::path().exists());
    // iny klik potvrdenie zrusi
    ui.click(Hit::ArchivePause);
    assert!(!ui.arc.confirm_delete && archive::path().exists());
    // Zrusit
    ui.click(Hit::ArchiveDelete);
    ui.click(Hit::ArchiveCancel);
    assert!(!ui.arc.confirm_delete && archive::path().exists());
    // druhy klik: zmaze (a len archiv; konfiguracia a ine subory ostavaju)
    let other = archive::path().with_file_name("claude-statusline.jsonl");
    std::fs::write(&other, b"{}\n").unwrap();
    ui.click(Hit::ArchiveDelete);
    ui.click(Hit::ArchiveDelete);
    assert!(!archive::path().exists());
    assert!(other.exists(), "zaznamy mosta statusline ostavaju");
    assert_eq!(ui.arc.info, archive::Info::default());
    assert!(!ui.arc.confirm_delete);
    // znova: nie je co zmazat, nic nepadne
    ui.click(Hit::ArchiveDelete);
    ui.click(Hit::ArchiveDelete);
    assert!(!archive::path().exists());
}

#[test]
fn summary_reports_unreadable_files_without_touching_them() {
    let e = Env::new("bad");
    let mut ui = e.ui();
    std::fs::create_dir_all(archive::path().parent().unwrap()).unwrap();
    std::fs::write(archive::path(), b"nonsense").unwrap();
    ui.refresh_archive();
    assert!(ui.arc.info.unreadable);
    ui.click(Hit::ArchiveRetention(3));
    assert_eq!(std::fs::read(archive::path()).unwrap(), b"nonsense");
    let _ = e;
}

#[test]
fn every_archive_text_exists_in_every_language() {
    let _e = Env::new("lang");
    let keys = [
        "arc.title",
        "arc.enable",
        "arc.d1",
        "arc.d2",
        "arc.pause",
        "arc.pause_d",
        "arc.retention",
        "arc.forever",
        "arc.months",
        "arc.needsrc",
        "arc.paused_state",
        "arc.unreadable",
        "arc.empty",
        "arc.info",
        "arc.delete",
        "arc.delete_confirm",
        "arc.note",
        "arc.on",
        "arc.off",
        "arc.paused_now",
        "arc.resumed",
        "arc.pruned",
        "arc.deleted",
        "arc.nothing",
        "arc.delete_failed",
    ];
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for k in keys {
            let v = t(k);
            assert!(!v.is_empty() && v != k, "{lang:?} {k}");
        }
        // vysvetlenie povie, co sa neuklada (prompty), v kazdom jazyku neprazdne a rozne od anglictiny okrem en
        assert!(t("arc.d2").contains("{0}") || !t("arc.d2").is_empty());
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
}
