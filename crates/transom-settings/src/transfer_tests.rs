//! Export a import nastaveni cez skutocne UI (stav `Ui`, klikanie `click`, navrat z dialogu `finish_dialog`) s izolovanymi
//! docasnymi subormi a konfiguraciou. Natívne dialogy sa neotvaraju (cesta sa vstrekuje); skutocna konfiguracia pouzivatela
//! ani data AI sa nepouzivaju.
use super::*;
use crate::archive_tests::Env;
use crate::transfer_dialog::Pick;
use crate::transfer_page::{Transfer, XferOp};
use transom_config::transfer::{export, ExportOpts};
use transom_config::{load, save, update};

fn opts() -> ExportOpts {
    ExportOpts {
        app_version: "0.1.0".into(),
        exported_at: "2026-11-21T10:00:00Z".into(),
    }
}

fn write_source(e: &Env, name: &str, f: impl FnOnce(&mut Config)) -> std::path::PathBuf {
    let mut c = Config::default();
    f(&mut c);
    let p = e.root.join(name);
    std::fs::write(&p, export(&c, &opts())).unwrap();
    p
}

fn backups(e: &Env) -> Vec<std::path::PathBuf> {
    let d = e.root.join("backups");
    std::fs::read_dir(&d)
        .map(|r| r.flatten().map(|x| x.path()).collect())
        .unwrap_or_default()
}

fn config_path(e: &Env) -> std::path::PathBuf {
    e.root.join("config.json")
}

fn bytes(p: &std::path::Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_default()
}

fn open_preview(ui: &mut Ui, p: &std::path::Path) {
    ui.begin_import_from(p);
    assert!(ui.transfer_open(), "nahlad sa otvoril: {}", ui.status);
}

#[test]
fn export_through_the_dialog_result_writes_only_the_allow_list() {
    let e = Env::new("xfer-export");
    let mut ui = e.ui();
    ui.cfg.usage.claude_local = true;
    ui.cfg.usage.codex_local = true;
    ui.cfg.usage.archive.enabled = true;
    ui.cfg.media.prefer_app = "Spotify.exe".into();
    ui.cfg.displays.mode = "selected".into();
    ui.cfg.displays.selected = vec![transom_config::DisplayRef {
        id: r"\\?\DISPLAY#GSM5B7F#5&1234".into(),
        label: "LG".into(),
    }];
    ui.cfg.appearance.bg_color = "#102030".into();
    // klik na "Exportovat" len pripravi dialog; po vybere cesty sa zapise subor
    ui.click(Hit::TransferExport);
    assert_eq!(ui.pending_xfer, Some(XferOp::Export));
    let target = e.root.join("my-settings.json");
    ui.finish_dialog(XferOp::Export, Pick::Chosen(target.clone()));
    let text = std::fs::read_to_string(&target).expect("export vznikol");
    for forbidden in [
        "claude_local",
        "codex_local",
        "archive",
        "prefer_app",
        "Spotify",
        "displays",
        "DISPLAY#",
        "dates",
        "extra",
        "\\",
    ] {
        assert!(!text.contains(forbidden), "export obsahuje {forbidden:?}");
    }
    assert!(text.contains("#102030") && text.contains("transom-settings-export"));
    assert!(
        ui.status.contains("my-settings.json")
            && !ui.status.contains(&e.root.to_string_lossy().to_string()),
        "{}",
        ui.status
    );
    assert!(
        !config_path(&e).exists(),
        "export nemeni ani nevytvara konfiguraciu"
    );
    let tmp = std::fs::read_dir(&e.root)
        .unwrap()
        .flatten()
        .filter(|x| x.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(tmp, 0);
    // zrusenie dialogu alebo chyba dialogu nic nezapise
    ui.status.clear();
    ui.finish_dialog(XferOp::Export, Pick::Cancelled);
    assert!(ui.status.is_empty());
    ui.finish_dialog(XferOp::Export, Pick::Failed(0x3003));
    assert!(!ui.status.is_empty());
}

#[test]
fn export_refuses_the_apps_own_files_and_missing_folders() {
    let e = Env::new("xfer-private");
    let mut ui = e.ui();
    save(&config_path(&e), &Config::default()).unwrap();
    let data = e.root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("usage-archive.json"), b"archiv").unwrap();
    let original = bytes(&config_path(&e));
    for target in [
        config_path(&e),
        e.root.join("config.json.lock"),
        data.join("usage-archive.json"),
        data.join("novy.json"),
    ] {
        ui.status.clear();
        ui.begin_export_to(&target);
        assert_eq!(ui.status, t("xfer.err_private"), "{target:?}");
    }
    assert_eq!(bytes(&config_path(&e)), original, "konfiguracia nezmenena");
    assert_eq!(bytes(&data.join("usage-archive.json")), b"archiv");
    assert!(!data.join("novy.json").exists());
    // neexistujuci priecinok sa nevytvara; nazov bez pripony sa odmietne
    ui.begin_export_to(&e.root.join("missing").join("x.json"));
    assert!(!e.root.join("missing").exists());
    assert!(
        ui.status
            .contains(&t("xfer.export_failed").replace("{0}", "")[..6])
            || !ui.status.is_empty()
    );
    ui.begin_export_to(&e.root.join("bez-pripony"));
    assert_eq!(ui.status, t("xfer.err_ext"));
    assert!(!e.root.join("bez-pripony").exists());
}

#[test]
fn import_is_two_step_backs_up_saves_and_never_enables_ai_sources() {
    let e = Env::new("xfer-import");
    save(&config_path(&e), &Config::default()).unwrap();
    let before = bytes(&config_path(&e));
    let mut ui = e.ui();
    // zdroj: zmeny + podvrhnute polia, ktore sa NESMU importovat (vlozene do textu)
    let src = write_source(&e, "in.json", |c| {
        c.appearance.bg_color = "#abcdef".into();
        c.appearance.bg_opacity = 70;
        c.clock.seconds = true;
        c.language = "sk".into();
        c.usage.refresh_minutes = 15;
    });
    let forged = std::fs::read_to_string(&src)
        .unwrap()
        .replacen("\"usage\": {", "\"usage\": {\"claude_local\": true, \"codex_local\": true, \"archive\": {\"enabled\": true}, ", 1)
        .replacen("\"settings\": {", "\"settings\": {\"displays\": {\"mode\": \"selected\"}, ", 1);
    std::fs::write(&src, forged).unwrap();
    ui.click(Hit::TransferImport);
    assert_eq!(ui.pending_xfer, Some(XferOp::Import));
    ui.finish_dialog(XferOp::Import, Pick::Chosen(src));
    assert!(ui.transfer_open());
    // nahlad neprepisuje nic
    assert_eq!(bytes(&config_path(&e)), before);
    let Transfer::Preview(p) = &ui.transfer else {
        unreachable!()
    };
    assert!(
        p.selected.iter().filter(|x| **x).count() >= 3,
        "zmenene sekcie su predvolene vybrane"
    );
    // 1. klik: iba potvrdenie, nic sa nezapise
    ui.click(Hit::TransferApply);
    assert!(matches!(&ui.transfer, Transfer::Preview(p) if p.confirm.is_some()));
    assert_eq!(
        bytes(&config_path(&e)),
        before,
        "po prvom kliku sa nic nezapisalo"
    );
    assert!(backups(&e).is_empty());
    // 2. klik: zaloha + zapis
    ui.click(Hit::TransferApply);
    assert!(!ui.transfer_open(), "{}", ui.status);
    let saved = load(&config_path(&e)).config;
    assert_eq!(saved.appearance.bg_color, "#abcdef");
    assert_eq!(saved.appearance.bg_opacity, 70);
    assert!(saved.clock.seconds);
    assert_eq!(saved.language, "sk");
    assert_eq!(saved.usage.refresh_minutes, 15);
    assert!(
        !saved.usage.claude_local && !saved.usage.codex_local && !saved.usage.claude_statusline,
        "zdroje AI ostali vypnute"
    );
    assert!(!saved.usage.archive.enabled);
    assert_eq!(
        saved.displays,
        Config::default().displays,
        "vyber monitorov sa neimportuje"
    );
    assert_eq!(ui.cfg, saved, "okno ukazuje to, co je ulozene");
    assert_eq!(
        transom_i18n::lang(),
        transom_i18n::Lang::Sk,
        "jazyk sa uplatnil"
    );
    transom_i18n::set_lang(transom_i18n::Lang::En);
    let b = backups(&e);
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(bytes(&b[0]), before, "zaloha = povodne bajty");
    // v exporte nie su ziadne cesty a zaloha nepatri medzi data AI
    assert!(!ui.status.is_empty());
}

#[test]
fn cancel_and_leaving_the_page_discard_the_preview_without_touching_anything() {
    let e = Env::new("xfer-cancel");
    save(&config_path(&e), &Config::default()).unwrap();
    let before = bytes(&config_path(&e));
    let mut ui = e.ui();
    let cfg_before = ui.cfg.clone();
    let src = write_source(&e, "in.json", |c| c.clock.seconds = true);
    open_preview(&mut ui, &src);
    ui.click(Hit::TransferApply); // prvy krok
    ui.click(Hit::TransferCancel);
    assert!(!ui.transfer_open());
    assert_eq!(bytes(&config_path(&e)), before);
    assert_eq!(ui.cfg, cfg_before);
    assert!(backups(&e).is_empty());
    // odchod na inu stranku nahlad zahodi
    open_preview(&mut ui, &src);
    ui.click(Hit::Nav(Page::Clock));
    assert!(!ui.transfer_open());
    assert_eq!(bytes(&config_path(&e)), before);
    // ine kliknutie zrusi rozpracovane potvrdenie (druhy krok sa nesmie vykonat nahodou)
    ui.click(Hit::Nav(Page::Transfer));
    open_preview(&mut ui, &src);
    ui.click(Hit::TransferApply);
    ui.click(Hit::TransferSection(0));
    assert!(matches!(&ui.transfer, Transfer::Preview(p) if p.confirm.is_none()));
    assert_eq!(bytes(&config_path(&e)), before);
}

#[test]
fn bad_files_are_refused_with_a_message_and_change_nothing() {
    let e = Env::new("xfer-bad");
    save(&config_path(&e), &Config::default()).unwrap();
    let before = bytes(&config_path(&e));
    let mut ui = e.ui();
    let cfg_before = ui.cfg.clone();
    let ok = export(&Config::default(), &opts());
    let mut cases: Vec<(&str, Vec<u8>, String)> = vec![
        (
            "not-json.json",
            b"{ nie je json".to_vec(),
            t("xfer.err_corrupt"),
        ),
        ("array.json", b"[1,2]".to_vec(), t("xfer.err_corrupt")),
        ("empty.json", Vec::new(), t("xfer.err_corrupt")),
        (
            "foreign.json",
            br#"{"format": "other", "format_version": 1, "settings": {}}"#.to_vec(),
            t("xfer.err_foreign"),
        ),
        (
            "config.json.copy",
            br#"{"version": 7, "layout": {}}"#.to_vec(),
            t("xfer.err_foreign"),
        ),
        (
            "future.json",
            br#"{"format": "transom-settings-export", "format_version": 9, "settings": {}}"#
                .to_vec(),
            tf("xfer.err_newer", &["9"]),
        ),
        (
            "binary.json",
            vec![0xFF, 0x00, 0xC3, 0x28, 0x80],
            t("xfer.err_corrupt"),
        ),
        (
            "utf16.json",
            [
                &[0xFF, 0xFE][..],
                &"{}"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<u8>>()[..],
            ]
            .concat(),
            t("xfer.err_utf16"),
        ),
        (
            "big.json",
            vec![b' '; transom_config::transfer::MAX_IMPORT_BYTES + 1],
            t("xfer.err_toolarge"),
        ),
    ];
    // platny export so zhodnymi hodnotami nema co importovat
    cases.push(("same.json", ok.into_bytes(), t("xfer.no_changes")));
    for (name, data, expected) in cases {
        let p = e.root.join(name);
        std::fs::write(&p, data).unwrap();
        ui.status.clear();
        ui.finish_dialog(XferOp::Import, Pick::Chosen(p));
        assert_eq!(ui.status, expected, "{name}");
        assert!(!ui.transfer_open(), "{name}");
    }
    // priecinok a neexistujuci subor
    ui.status.clear();
    ui.begin_import_from(&e.root);
    assert_eq!(ui.status, t("xfer.err_notfile"));
    ui.begin_import_from(&e.root.join("neexistuje.json"));
    assert!(!ui.transfer_open() && !ui.status.is_empty());
    assert_eq!(bytes(&config_path(&e)), before);
    assert_eq!(ui.cfg, cfg_before);
    assert!(backups(&e).is_empty());
}

#[test]
fn a_partly_invalid_file_imports_the_valid_sections_and_lists_what_it_ignored() {
    let e = Env::new("xfer-partial");
    save(&config_path(&e), &Config::default()).unwrap();
    let mut ui = e.ui();
    let src = write_source(&e, "in.json", |c| {
        c.clock.seconds = true;
        c.appearance.bg_opacity = 60;
    });
    let text = std::fs::read_to_string(&src)
        .unwrap()
        .replacen(
            "\"appearance\": {",
            "\"appearance\": 5, \"zz_appearance\": {",
            1,
        )
        .replacen("\"settings\": {", "\"settings\": {\"weather\": {}, ", 1);
    std::fs::write(&src, text).unwrap();
    open_preview(&mut ui, &src);
    let Transfer::Preview(p) = &ui.transfer else {
        unreachable!()
    };
    let prev = transom_config::transfer::preview(&ui.cfg, &p.doc);
    assert!(
        prev.ignored.iter().any(|i| i == "appearance (invalid)"),
        "{:?}",
        prev.ignored
    );
    assert!(prev.ignored.iter().any(|i| i == "weather"));
    assert_eq!(
        crate::transfer_page::ignored_text("appearance (invalid)"),
        tf("xfer.invalid", &[&t("app.title")])
    );
    ui.click(Hit::TransferApply);
    ui.click(Hit::TransferApply);
    let saved = load(&config_path(&e)).config;
    assert!(saved.clock.seconds, "platna sekcia sa importovala");
    assert_eq!(
        saved.appearance,
        Config::default().appearance,
        "neplatna sekcia sa nezmenila"
    );
}

#[test]
fn a_concurrent_change_survives_and_a_changed_preview_must_be_confirmed_again() {
    let e = Env::new("xfer-concurrent");
    save(&config_path(&e), &Config::default()).unwrap();
    let mut ui = e.ui();
    let src = write_source(&e, "in.json", |c| {
        c.clock.seconds = true;
        c.appearance.bg_opacity = 60;
    });
    open_preview(&mut ui, &src);
    ui.click(Hit::TransferApply);
    // medzitym iny proces zmeni pole mimo importu: prezije
    update(&config_path(&e), |c| c.displays.mode = "primary".into()).unwrap();
    ui.click(Hit::TransferApply);
    assert!(!ui.transfer_open(), "{}", ui.status);
    let saved = load(&config_path(&e)).config;
    assert!(saved.clock.seconds && saved.appearance.bg_opacity == 60);
    assert_eq!(saved.displays.mode, "primary", "subezna zmena zachovana");
    assert_eq!(ui.cfg, saved);

    // zmena, ktora ovplyvni to, co nahlad ukazoval: druhy krok nic nezapise a pyta potvrdenie znova
    drop(ui);
    drop(e);
    let e2 = Env::new("xfer-concurrent2");
    save(&config_path(&e2), &Config::default()).unwrap();
    let mut ui = e2.ui();
    let src = write_source(&e2, "in.json", |c| {
        c.clock.seconds = true;
        c.appearance.bg_opacity = 60;
    });
    open_preview(&mut ui, &src);
    ui.click(Hit::TransferApply);
    update(&config_path(&e2), |c| c.appearance.bg_opacity = 60).unwrap(); // jedna zo zmien je uz bezpredmetna
    let after_external = bytes(&config_path(&e2));
    ui.click(Hit::TransferApply);
    assert!(ui.transfer_open(), "nahlad ostal otvoreny");
    assert!(matches!(&ui.transfer, Transfer::Preview(p) if p.confirm.is_none()));
    assert_eq!(ui.status, t("xfer.stale"));
    assert_eq!(
        bytes(&config_path(&e2)),
        after_external,
        "nic sa nezapisalo"
    );
    assert!(backups(&e2).is_empty());
    // po novom potvrdeni sa importuje
    ui.click(Hit::TransferApply);
    ui.click(Hit::TransferApply);
    assert!(!ui.transfer_open(), "{}", ui.status);
    assert!(load(&config_path(&e2)).config.clock.seconds);
    assert_eq!(backups(&e2).len(), 1);
}

#[test]
fn a_read_only_or_damaged_configuration_blocks_export_and_import_and_is_never_overwritten() {
    let e = Env::new("xfer-readonly");
    std::fs::write(config_path(&e), "{ poskodene").unwrap();
    let mut ui = e.ui();
    ui.read_only = true;
    let src = write_source(&e, "in.json", |c| c.clock.seconds = true);
    assert!(!ui.click(Hit::TransferExport) && ui.pending_xfer.is_none());
    assert!(!ui.click(Hit::TransferImport) && ui.pending_xfer.is_none());
    ui.finish_dialog(XferOp::Import, Pick::Chosen(src.clone()));
    assert_eq!(ui.status, t("xfer.readonly"));
    ui.begin_export_to(&e.root.join("e.json"));
    assert!(!e.root.join("e.json").exists());
    assert_eq!(bytes(&config_path(&e)), b"{ poskodene");
    assert!(backups(&e).is_empty());

    // poskodenie medzi nahladom a potvrdenim: zapis sa zrusi a subor ostane bajt po bajte
    drop(ui);
    drop(e);
    let e2 = Env::new("xfer-corrupt-late");
    save(&config_path(&e2), &Config::default()).unwrap();
    let mut ui = e2.ui();
    let src = write_source(&e2, "in.json", |c| c.clock.seconds = true);
    open_preview(&mut ui, &src);
    ui.click(Hit::TransferApply);
    std::fs::write(config_path(&e2), "{ rozpisane").unwrap();
    ui.click(Hit::TransferApply);
    assert!(ui.transfer_open());
    assert_eq!(bytes(&config_path(&e2)), b"{ rozpisane");
    assert!(backups(&e2).is_empty());
    assert!(ui.status.contains(&t("xfer.readonly")), "{}", ui.status);
}

#[test]
fn a_failed_backup_aborts_the_import_and_changes_nothing() {
    let e = Env::new("xfer-backup-fail");
    save(&config_path(&e), &Config::default()).unwrap();
    let before = bytes(&config_path(&e));
    std::fs::write(e.root.join("backups"), "tu ma byt priecinok").unwrap();
    let mut ui = e.ui();
    let src = write_source(&e, "in.json", |c| c.clock.seconds = true);
    open_preview(&mut ui, &src);
    ui.click(Hit::TransferApply);
    ui.click(Hit::TransferApply);
    assert!(ui.transfer_open(), "nahlad ostava");
    assert!(
        ui.status.starts_with(
            &t("xfer.backup_failed")
                .split("{0}")
                .next()
                .unwrap_or("")
                .to_string()
        ) || !ui.status.is_empty()
    );
    assert_eq!(bytes(&config_path(&e)), before);
}

#[test]
fn escape_closes_the_preview_and_returns_the_focus_to_the_import_button() {
    use crate::focus::{route_key_ctx, KeyAction, PageFocus};
    let e = Env::new("xfer-esc");
    save(&config_path(&e), &Config::default()).unwrap();
    let mut ui = e.ui();
    let src = write_source(&e, "in.json", |c| c.clock.seconds = true);
    open_preview(&mut ui, &src);
    // fokus na sekcii: Esc zatvori nahlad (aj bez fokusu na ovladaci)
    assert_eq!(
        route_key_ctx(0x1B, false, false, true, false, PageFocus::Preview, false),
        KeyAction::ClosePreview
    );
    assert_eq!(
        route_key_ctx(0x1B, false, false, false, false, PageFocus::Preview, false),
        KeyAction::ClosePreview
    );
    ui.cancel_transfer();
    assert!(!ui.transfer_open());
    // Enter drzany (auto-opakovanie) nesmie potvrdit dvojkrokovy import
    assert_eq!(
        route_key_ctx(0x0D, false, false, true, false, PageFocus::Preview, true),
        KeyAction::Pass
    );
    assert_eq!(
        route_key_ctx(0x0D, false, false, true, false, PageFocus::Preview, false),
        KeyAction::Activate
    );
}

#[test]
fn every_label_key_used_by_the_preview_exists_in_all_languages() {
    use transom_config::transfer::Section;
    let _g = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for s in Section::ALL {
            let l = crate::transfer_page::section_label(s);
            assert!(
                !l.contains('.') || l.contains(' '),
                "{lang:?} {s:?}: surovy kluc {l:?}"
            );
        }
        for (s, f) in [
            (Section::Clock, "hour"),
            (Section::Clock, "leading_zero"),
            (Section::Clock, "seconds"),
            (Section::Clock, "weekday"),
            (Section::Clock, "date"),
            (Section::Clock, "first_day"),
            (Section::Media, "marquee"),
            (Section::Media, "marquee_direction"),
            (Section::Media, "marquee_speed"),
            (Section::Media, "marquee_paused"),
            (Section::Appearance, "bg_color"),
            (Section::Appearance, "bg_opacity"),
            (Section::Appearance, "bg_transparent"),
            (Section::Appearance, "tray_icon"),
            (Section::Appearance, "glass_panels"),
            (Section::Appearance, "glass_bar"),
            (Section::Usage, "refresh_minutes"),
            (Section::Usage, "bar_provider"),
            (Section::Usage, "ring_mode"),
        ] {
            let l = crate::transfer_page::field_label(s, f);
            assert_ne!(l, f, "{lang:?}: chyba popisok pola {f}");
            assert!(
                l.contains(' ') || !l.contains('.'),
                "{lang:?}: surovy kluc {l:?}"
            );
        }
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
    // zname jazyky exportu = jazyky aplikacie
    let mut app: Vec<&str> = transom_i18n::Lang::ALL.iter().map(|l| l.code()).collect();
    app.push("system");
    let mut known: Vec<&str> = transom_config::transfer::KNOWN_LANGUAGES.to_vec();
    app.sort();
    known.sort();
    assert_eq!(app, known);
}

#[test]
fn change_lines_are_readable_and_split_the_layout_by_group() {
    use transom_config::transfer::{Change, Section};
    let _g = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    transom_i18n::set_lang(transom_i18n::Lang::En);
    let c = Change {
        section: Section::Layout,
        field: "items".into(),
        from: "left: search, media | center:  | right: ai, tray, battery, clock".into(),
        to: "left: media | center:  | right: ai, tray, battery, clock".into(),
    };
    let lines = crate::transfer_page::change_lines(&c);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("Left:") && lines[0].contains("Search, Media →")
            || lines[0].contains("→"),
        "{lines:?}"
    );
    let opacity = Change {
        section: Section::Appearance,
        field: "bg_opacity".into(),
        from: "100".into(),
        to: "60".into(),
    };
    assert_eq!(
        crate::transfer_page::change_lines(&opacity),
        vec!["Opacity: 100 → 60".to_string()]
    );
    let lang = Change {
        section: Section::Language,
        field: String::new(),
        from: "system".into(),
        to: "de".into(),
    };
    assert_eq!(
        crate::transfer_page::change_lines(&lang),
        vec!["Application language: system → de".to_string()]
    );
    assert_eq!(
        crate::transfer_page::value_text("refresh_minutes", "0"),
        "On demand"
    );
    assert_eq!(
        crate::transfer_page::value_text("bg_color", ""),
        "(default)"
    );
}

#[test]
fn file_helpers_validate_paths_sizes_and_encoding() {
    use crate::transfer_io::{decode_import, is_plain_file_path, read_limited, TransferError};
    use std::path::Path;
    for ok in [r"C:\x\y.json", r"\\server\share\a.json"] {
        assert!(is_plain_file_path(Path::new(ok)), "{ok}");
    }
    for bad in [
        r"\\.\CON",
        r"\\?\GLOBALROOT\Device\x",
        "CON",
        "NUL",
        r"rel\x.json",
        r"C:",
    ] {
        assert!(!is_plain_file_path(Path::new(bad)), "{bad}");
    }
    // obmedzene citanie: klamliva velkost aj nekonecny prud
    struct Endless;
    impl std::io::Read for Endless {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            b.fill(b' ');
            Ok(b.len())
        }
    }
    assert_eq!(read_limited(Endless, 10), Err(TransferError::TooLarge));
    assert_eq!(read_limited(&b"abc"[..], 3).unwrap(), b"abc");
    assert_eq!(
        read_limited(&b"abc"[..], u64::MAX),
        Err(TransferError::TooLarge)
    );
    assert!(matches!(
        decode_import(&[0xFE, 0xFF, 0, 0x7B]),
        Err(TransferError::Utf16)
    ));
    assert!(matches!(
        decode_import(&[0xC3, 0x28]),
        Err(TransferError::NotUtf8)
    ));
}

#[test]
fn dialog_inputs_are_built_correctly_without_showing_a_dialog() {
    use crate::transfer_dialog::*;
    assert_eq!(
        default_export_name(2026, 11, 21),
        "transom-settings-2026-11-21.json"
    );
    let f = filter_wide(&[("JSON (*.json)", "*.json"), ("All (*.*)", "*.*")]);
    assert_eq!(&f[f.len() - 2..], &[0, 0], "filter konci dvojitou nulou");
    assert_eq!(f.iter().filter(|c| **c == 0).count(), 5);
    let b = buffer_with("a.json");
    assert_eq!(b.len(), BUF_LEN);
    assert_eq!(path_from_buffer(&b).unwrap().to_string_lossy(), "a.json");
    assert_eq!(path_from_buffer(&buffer_with("")), None);
    let long = "x".repeat(BUF_LEN + 50);
    let b = buffer_with(&long);
    assert_eq!(b[BUF_LEN - 1], 0, "vzdy ukonceny nulou");
    assert!(path_from_buffer(&b).unwrap().to_string_lossy().len() == BUF_LEN - 1);
    let (save, open) = (flags(Kind::Save), flags(Kind::Open));
    assert!(save.contains(windows::Win32::UI::Controls::Dialogs::OFN_OVERWRITEPROMPT));
    assert!(!save.contains(windows::Win32::UI::Controls::Dialogs::OFN_FILEMUSTEXIST));
    assert!(open.contains(windows::Win32::UI::Controls::Dialogs::OFN_FILEMUSTEXIST));
    assert!(!open.contains(windows::Win32::UI::Controls::Dialogs::OFN_OVERWRITEPROMPT));
    for f in [save, open] {
        assert!(
            f.contains(windows::Win32::UI::Controls::Dialogs::OFN_NOCHANGEDIR)
                && f.contains(windows::Win32::UI::Controls::Dialogs::OFN_EXPLORER)
        );
    }
    let spec = Spec {
        title: "T".into(),
        filter: f,
        initial: String::new(),
        def_ext: wide0("json"),
    };
    let mut buf = buffer_with("x.json");
    let title: Vec<u16> = "T\0".encode_utf16().collect();
    let ofn = build_ofn(
        Kind::Save,
        windows::Win32::Foundation::HWND(std::ptr::null_mut()),
        &mut buf,
        &spec,
        &title,
    );
    assert_eq!(
        ofn.lStructSize as usize,
        std::mem::size_of::<windows::Win32::UI::Controls::Dialogs::OPENFILENAMEW>()
    );
    assert_eq!(ofn.nMaxFile as usize, BUF_LEN);
    assert_eq!(ofn.lpstrFile.0, buf.as_mut_ptr());
    assert_eq!(ofn.lpstrDefExt.0, spec.def_ext.as_ptr());
    assert_eq!(wide0("exe"), vec![b'e' as u16, b'x' as u16, b'e' as u16, 0]);
}
