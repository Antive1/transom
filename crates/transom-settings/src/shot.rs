//! Offscreen kontrola okna nastaveni (bez zmeny systemoveho DPI / jazyka / temy):
//!   transom-settings --shot <adresar> [--lang all|sk,en,..] [--dpi 96,144,192] [--theme dark,light]
//!                    [--page layout,media,...] [--png] [--full] [--ai-sample] [--font classic]
//! Pre kazdu kombinaciu vykresli stranku do pamatoveho DC (skutocny renderer), spusti audit textov (orezanie,
//! vyska, prekryv, kontrast voci ploche pod textom) na celej vyske obsahu a vypise jeden riadok; `--png` ulozi
//! snimky (stranka v okne s aktualnym posunom 0). Navratovy kod 1, ak sa nasla chyba.
use crate::{Page, Ui};
use std::fmt::Write as _;
use transom_i18n::Lang;
use transom_render::audit::{Audit, Issue};
use transom_render::offscreen::{png, Surface};
use transom_render::{GdiPainter, Size};

/// Najmensi klientsky rozmer okna (lp), s ktorym sa pocita: okno 840 x 720 minus ramik a titulok.
pub(crate) const CLIENT_W_LP: i32 = 824;
/// Klientska oblast po odstraneni systemovej hlavicky: vyska okna (720) bez dolneho ramu (8).
pub(crate) const CLIENT_H_LP: i32 = 712;

pub(crate) const PAGES: [(Page, &str); 21] = [
    (Page::Layout, "layout"),
    (Page::Displays, "displays"),
    (Page::Media, "media"),
    (Page::Appearance, "appearance"),
    (Page::Clock, "clock"),
    (Page::Ai, "ai"),
    (Page::Sound, "sound"),
    (Page::Audio, "audio"),
    (Page::Peripherals, "peripherals"),
    (Page::Tools, "tools"),
    (Page::Extras, "extras"),
    (Page::Language, "language"),
    (Page::Transfer, "transfer"),
    (Page::Updates, "updates"),
    (Page::Welcome, "welcome"),
    (Page::Clipboard, "clipboard"),
    (Page::Notes, "notes"),
    (Page::Shots, "shots"),
    (Page::Launcher, "launcher"),
    (Page::Notch, "notch"),
    (Page::ToolGroup, "toolgroup"),
];

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn describe(i: &Issue) -> String {
    match i {
        Issue::Clipped { text, need, have } => format!("orezane \"{text}\" ({need} > {have} px)"),
        Issue::TooTall { text, need, have } => format!("vysoke \"{text}\" ({need} > {have} px)"),
        Issue::Offscreen { text } => format!("mimo okna \"{text}\""),
        Issue::Overlap { a, b, area } => format!("prekryv \"{a}\" x \"{b}\" ({area} px2)"),
        Issue::Contrast { text, ratio, need } => {
            format!("kontrast \"{text}\" {ratio:.2} < {need}")
        }
    }
}

/// Texty, ktore sa zamerne skracuju alebo su vzorkou obsahu (cesta k suboru, ukazkovy nazov skladby).
fn allow_list() -> Vec<String> {
    vec!["\\".into(), "transom\\".into()]
}

pub(crate) fn run(args: &[String], mut ui: Ui) -> i32 {
    let out_dir = arg(args, "--shot").unwrap_or_else(|| ".".into());
    // `--font classic`: kontrola zaloznoho pisma (Segoe UI) aj na stroji so Segoe UI Variable
    if arg(args, "--font").as_deref() == Some("classic") {
        transom_render::fonts::force(Some(transom_render::fonts::Family::Classic));
    }
    let _ = std::fs::create_dir_all(&out_dir);
    let save_png = args.iter().any(|a| a == "--png");
    let langs: Vec<Lang> = match arg(args, "--lang").as_deref() {
        None | Some("all") => Lang::ALL.to_vec(),
        Some(list) => list.split(',').filter_map(Lang::from_tag).collect(),
    };
    let dpis: Vec<u32> = arg(args, "--dpi")
        .map(|v| v.split(',').filter_map(|d| d.parse().ok()).collect())
        .unwrap_or_else(|| vec![96, 144, 192]);
    let themes: Vec<bool> = match arg(args, "--theme").as_deref() {
        Some("dark") => vec![true],
        Some("light") => vec![false],
        _ => vec![true, false],
    };
    // (stranka, nazov, potvrdzovaci stav "Zmazat data": druhy klik pyta potvrdenie - ma dlhsi text tlacidla)
    let mut pages: Vec<(Page, &str, bool)> = Vec::new();
    for (pg, n) in PAGES {
        if arg(args, "--page").is_none_or(|l| l.split(',').any(|x| x == n)) {
            pages.push((pg, n, false));
            if pg == Page::Ai {
                pages.push((pg, "ai-confirm", true));
                pages.push((pg, "ai-edit", false));
                pages.push((pg, "ai-cc-foreign", false));
                pages.push((pg, "ai-arch-on", false));
                pages.push((pg, "ai-arch-confirm", false));
                pages.push((pg, "ai-arch-bad", false));
                pages.push((pg, "ai-cc-nosnap", false));
                pages.push((pg, "ai-cc-ok", false));
            }
            if pg == Page::Displays {
                pages.push((pg, "displays-missing", false));
            }
            if pg == Page::Extras {
                pages.push((pg, "extras-weather", false));
            }
            if pg == Page::Appearance {
                pages.push((pg, "appearance-oled", false));
            }
            if pg == Page::Sound {
                pages.push((pg, "sound-on", false));
                pages.push((pg, "sound-max", false));
            }
            if pg == Page::Transfer {
                pages.push((pg, "transfer-preview", false));
                pages.push((pg, "transfer-confirm", false));
                pages.push((pg, "transfer-ro", false));
            }
        }
    }
    // ukazkove rucne datumy (zoznam v stranke AI sa kontroluje aj s dlhymi riadkami a vsetkymi tlacidlami)
    let md = |p: &str, k: &str, d: &str, r: &str| transom_config::ManualDate {
        provider: p.into(),
        kind: k.into(),
        date: d.into(),
        repeat: r.into(),
        every_days: 6,
    };
    ui.cfg.usage.dates = vec![
        md("claude", "reset", "2026-11-20", "days"),
        md("codex", "billing", "2026-01-31", "monthly"),
        md("claude", "billing", "2027-03-05", "none"),
    ];
    // ukazkove monitory (dlhy nazov, mierka 150 %, primarny) pre stranku Displays
    let mon = |id: &str, name: &str, l: i32, w: i32, h: i32, dpi: u32, prim: bool| {
        transom_platform::monitor::MonitorInfo {
            device: format!(r"\\.\DISPLAY{}", if prim { 1 } else { 2 }),
            id: id.into(),
            rect: transom_platform::monitor::Rect {
                left: l,
                top: 0,
                right: l + w,
                bottom: h,
            },
            dpi,
            primary: prim,
            name: name.into(),
        }
    };
    ui.monitors = vec![
        mon("A", "AOC 2402", 0, 2560, 1440, 96, true),
        mon(
            "B",
            "DELL U2720Q (ultra dlhy popis monitora pre kontrolu zalamovania)",
            -1080,
            1080,
            1920,
            144,
            false,
        ),
    ];
    let mut total = 0usize;
    let mut report = String::new();
    for &lang in &langs {
        transom_i18n::set_lang(lang);
        for &dpi in &dpis {
            let s = dpi as f32 / 96.0;
            let (cw, ch) = (
                (CLIENT_W_LP as f32 * s).round() as i32,
                (CLIENT_H_LP as f32 * s).round() as i32,
            );
            for &dark in &themes {
                ui.theme = crate::themed(dark, ui.cfg.appearance.glass_panels);
                ui.art = None;
                for &(page, name, confirm) in &pages {
                    ui.page = page;
                    ui.confirm_delete = confirm;
                    // export / import: nahlad s dlhymi hodnotami a ignorovanymi polozkami, potvrdenie, konfiguracia len na citanie
                    ui.transfer = transfer_fixture(name);
                    ui.read_only = name == "transfer-ro";
                    // zvuky rozhrania: vypnute (predvolene), zapnute s nizkou hlasitostou a zapnute na 100 %
                    ui.cfg.ui_sounds = transom_config::UiSounds {
                        enabled: name.starts_with("sound-"),
                        volume: if name == "sound-max" { 100 } else { 25 },
                    };
                    // ochrana OLED zapnuta (vsetky podvolby vratane skryvania) a vlastna farba textu
                    let oled = name == "appearance-oled";
                    ui.cfg.oled = transom_config::Oled {
                        enabled: oled,
                        auto_hide: oled,
                        ..Default::default()
                    };
                    (ui.cfg.appearance.text_mode, ui.cfg.appearance.text_color) = if oled {
                        ("custom".into(), "#FFCC00".into())
                    } else {
                        ("auto".into(), String::new())
                    };
                    // oddiel "Pripojit Claude Code": ukazkove stavy (dlhy cudzi prikaz, dlhy vysledok skusky)
                    ui.cc = cc_fixture(name);
                    (ui.cfg.usage.archive, ui.arc) = arc_fixture(name);
                    // ulozeny nepripojeny monitor (lista je na primarnom ako zaloha + vysvetlenie)
                    ui.cfg.displays = if name == "displays-missing" {
                        transom_config::Displays {
                            mode: "selected".into(),
                            selected: vec![transom_config::DisplayRef {
                                id: "gone".into(),
                                label: "LG 27UL850 · 3840 × 2160 · 150 %".into(),
                            }],
                        }
                    } else {
                        transom_config::Displays::default()
                    };
                    // rozpracovany formular: upravuje sa zaznam, interval dni a hlaska o neplatnom datume
                    ui.form = if name == "ai-edit" {
                        crate::DateForm {
                            provider: 1,
                            kind: 1,
                            repeat: 1,
                            every: 6,
                            editing: Some(0),
                            error: true,
                            cal: Some((2026, 11)),
                        }
                    } else {
                        crate::DateForm::default()
                    };
                    // pocasie: vysledky hladania (dlhe nazvy), ulozene miesta, zamietnuta poloha, plny zoznam
                    crate::weather_page::fixture(&mut ui, name == "extras-weather");
                    ui.scroll = 0;
                    ui.max_scroll = 0;
                    ui.status =
                        transom_i18n::tf("status.file", &["%APPDATA%\\transom\\config.json"]);
                    if name == "transfer-confirm" {
                        // dlhe chybove hlasenie (realne chyby nesu aj text systemu): stavovy riadok sa nesmie orezat
                        ui.status = transom_i18n::tf(
                            "xfer.import_failed",
                            &["The configuration file is locked by another program (os error 32)"],
                        );
                    }
                    // 1) okno v povodnej velkosti: zisti rozsah posuvania a (volitelne) ulozi snimku
                    let shot = Surface::new(cw, ch);
                    for _ in 0..2 {
                        let mut p = GdiPainter::new(shot.dc, Size { w: cw, h: ch }, dpi);
                        ui.paint(&mut p);
                    }
                    let max_scroll = ui.max_scroll;
                    // `--time`: cas jedneho vykreslenia stranky (posun do stredu ako pri posuvani kolieskom)
                    if args.iter().any(|a| a == "--time") {
                        ui.scroll = max_scroll / 2;
                        let t0 = std::time::Instant::now();
                        let n = 10;
                        for _ in 0..n {
                            let mut p = GdiPainter::new(shot.dc, Size { w: cw, h: ch }, dpi);
                            ui.paint(&mut p);
                        }
                        println!(
                            "TIME {name:<22} {dpi}  {:.1} ms",
                            t0.elapsed().as_secs_f64() * 1000.0 / n as f64
                        );
                        ui.scroll = 0;
                    }
                    if save_png {
                        let f = format!(
                            "{out_dir}\\settings-{}-{name}-{dpi}-{}.png",
                            lang.code(),
                            if dark { "dark" } else { "light" }
                        );
                        let _ = std::fs::write(&f, png(cw as u32, ch as u32, &shot.rgb()));
                    }
                    // `--scroll N` (lp): dalsia snimka na zadanej polohe posunu (napr. stredna cast dlhej stranky)
                    if let (true, Some(n)) = (
                        save_png,
                        arg(args, "--scroll").and_then(|v| v.parse::<f32>().ok()),
                    ) {
                        ui.scroll = ((n * s).round() as i32).min(max_scroll);
                        let mut p = GdiPainter::new(shot.dc, Size { w: cw, h: ch }, dpi);
                        ui.paint(&mut p);
                        let f = format!(
                            "{out_dir}\\settings-{}-{name}-{dpi}-{}-scroll.png",
                            lang.code(),
                            if dark { "dark" } else { "light" }
                        );
                        let _ = std::fs::write(&f, png(cw as u32, ch as u32, &shot.rgb()));
                        ui.scroll = 0;
                    }
                    // koniec posuvneho obsahu (spodna cast dlhych stran, napr. formular datumov)
                    if save_png && max_scroll > 0 {
                        ui.scroll = max_scroll;
                        let mut p = GdiPainter::new(shot.dc, Size { w: cw, h: ch }, dpi);
                        ui.paint(&mut p);
                        let f = format!(
                            "{out_dir}\\settings-{}-{name}-{dpi}-{}-end.png",
                            lang.code(),
                            if dark { "dark" } else { "light" }
                        );
                        let _ = std::fs::write(&f, png(cw as u32, ch as u32, &shot.rgb()));
                        ui.scroll = 0;
                    }
                    // 2) cely obsah na jednej vysokej ploche + audit
                    let th = ui.theme;
                    let tall_h = ch + max_scroll;
                    let tall = Surface::new(cw, tall_h);
                    let mut issues: Vec<Issue> = Vec::new();
                    {
                        let mut gp = GdiPainter::new(tall.dc, Size { w: cw, h: tall_h }, dpi);
                        let mut a = Audit::new(&mut gp, th.background);
                        a.allow_elide = allow_list();
                        ui.scroll = 0;
                        ui.paint(&mut a);
                        a.finish();
                        issues.append(&mut a.issues);
                    }
                    // `--full`: cela stranka na jednej snimke (vizualna kontrola celeho obsahu naraz)
                    if save_png && args.iter().any(|a| a == "--full") {
                        let f = format!(
                            "{out_dir}\\settings-{}-{name}-{dpi}-{}-full.png",
                            lang.code(),
                            if dark { "dark" } else { "light" }
                        );
                        let _ = std::fs::write(&f, png(cw as u32, tall_h as u32, &tall.rgb()));
                    }
                    // pri vyssej ploche sa rozsah posuvania zmenil na 0: vrat povodny stav
                    ui.max_scroll = max_scroll;
                    // rovnaky text sa moze hlasit viackrat (kazdy prvok raz)
                    issues.dedup_by(|a, b| describe(a) == describe(b));
                    let _ = writeln!(
                        report,
                        "{} {dpi:>3} {} {name:<10} scroll={max_scroll:<4} issues={}",
                        lang.code(),
                        if dark { "dark " } else { "light" },
                        issues.len()
                    );
                    for i in &issues {
                        let _ = writeln!(report, "    - {}", describe(i));
                    }
                    total += issues.len();
                }
            }
        }
    }
    print!("{report}");
    println!("SPOLU nalezov: {total}");
    let _ = std::fs::write(format!("{out_dir}\\settings-audit.txt"), &report);
    i32::from(total > 0)
}

/// Ukazkovy stav archivu dennych suctov (nastavenie + zhrnutie suboru) pre audit rozlozenia (bez citania suborov).
fn arc_fixture(name: &str) -> (transom_config::Archive, crate::ArchiveUi) {
    use transom_usage::archive::Info;
    let info = Info {
        rows: 1840,
        days: 412,
        from: Some((2025, 10, 3)),
        to: Some((2026, 11, 20)),
        bytes: 482_000,
        unreadable: false,
    };
    let on = transom_config::Archive {
        enabled: true,
        paused: false,
        retention_months: 12,
    };
    match name {
        "ai-arch-on" => (
            on,
            crate::ArchiveUi {
                info,
                confirm_delete: false,
            },
        ),
        // pozastavene, bez lokalneho zdroja (varovanie), druhy krok mazania
        "ai-arch-confirm" => (
            transom_config::Archive {
                paused: true,
                retention_months: 0,
                ..on
            },
            crate::ArchiveUi {
                info,
                confirm_delete: true,
            },
        ),
        "ai-arch-bad" => (
            on,
            crate::ArchiveUi {
                info: Info {
                    unreadable: true,
                    bytes: 12,
                    ..Default::default()
                },
                confirm_delete: false,
            },
        ),
        _ => Default::default(),
    }
}

/// Ukazkovy nahlad importu (cez skutocny export + `parse_import`): vela zmien, dlhy nazov suboru, dlhe ignorovane polozky,
/// sekcia, ktora v subore nie je, a stav po prvom kliku na "Importovat vybrane".
pub(crate) fn transfer_fixture(name: &str) -> crate::transfer_page::Transfer {
    use crate::transfer_page::{Transfer, TransferPreview};
    use transom_config::transfer::{export, parse_import, ExportOpts};
    if !matches!(name, "transfer-preview" | "transfer-confirm") {
        return Transfer::Idle;
    }
    let mut c = transom_config::Config::default();
    c.appearance.bg_color = "#102030".into();
    c.appearance.bg_opacity = 60;
    c.appearance.bg_transparent = true;
    c.clock.seconds = true;
    c.clock.weekday = true;
    c.media.marquee = false;
    c.media.marquee_speed = "fast".into();
    c.usage.refresh_minutes = 15;
    c.usage.ring_mode = "remaining".into();
    c.layout.set_visible("ai", true);
    c.layout
        .move_within("search", transom_config::Group::Right, 0);
    c.layout
        .move_within("battery", transom_config::Group::Left, 0);
    c.layout
        .move_within("clock", transom_config::Group::Center, 0);
    let opts = ExportOpts {
        app_version: "0.1.0".into(),
        exported_at: "2026-11-21T10:00:00Z".into(),
    };
    let mut text = export(&c, &opts)
        // jazyk chyba v subore (riadok "Nie je v subore") a prida sa dlha ignorovana polozka aj zakazane polia
        .replacen("\"language\":", "\"language_x\":", 1)
        .replacen("\"settings\": {", "\"settings\": {\"displays\": {}, \"profiles\": [], \"weather_from_a_future_version_without_spaces\": 1, \"a\": 1, \"b\": 1, \"c\": 1, \"d\": 1, \"e\": 1, \"f\": 1, \"g\": 1,", 1)
        .replacen("\"usage\": {", "\"usage\": {\"claude_local\": true, ", 1);
    text = text.replacen(
        "\"clock\": {",
        "\"clock\": {\"unknown_clock_field\": 3, ",
        1,
    );
    let doc = parse_import(&text).expect("fixture je platny export");
    Transfer::Preview(Box::new(TransferPreview {
        doc,
        file: "transom-settings-with-a-very-long-file-name-without-any-spaces-2026-11-21.json"
            .into(),
        selected: [true; 6],
        confirm: (name == "transfer-confirm").then(Vec::new),
    }))
}

/// Ukazkovy stav pripojenia Claude Code pre audit rozlozenia (bez citania suborov).
fn cc_fixture(name: &str) -> crate::cc::Cc {
    use transom_usage::claude_setup::Connection as C;
    let long_test = "x".repeat(0)
        + "Bridge OK: Sample · 12 % 5h. Snapshots recorded: 42, but none carries plan limits (only claude.ai Pro, Max and Ultra report them).";
    match name {
        "ai-cc-foreign" => crate::cc::Cc {
            conn: Some(C::Foreign(
                "C:/Users/someone/.claude/very-long-statusline-script-name.sh --with --flags"
                    .into(),
            )),
            can_rollback: true,
            confirm_replace: true,
            ..Default::default()
        },
        "ai-cc-nosnap" => crate::cc::Cc {
            conn: Some(C::ConfiguredNoSnapshot),
            can_rollback: true,
            test: Some(long_test),
            redirected: true,
            ..Default::default()
        },
        "ai-cc-ok" => crate::cc::Cc {
            conn: Some(C::Connected { limits: true }),
            records: 12,
            can_rollback: true,
            ..Default::default()
        },
        _ => crate::cc::Cc::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regresna brana vzhladu: ziadne orezanie, prekryv ani slaby kontrast v kluchovych jazykoch (najdlhsie preklady,
    /// japoncina bez medzier) pri 100 a 200 % v oboch temach. Cela matica (11 jazykov x 3 DPI) ide cez `--shot`.
    #[test]
    fn settings_audit_is_clean_for_key_languages() {
        // meni globalny jazyk a pouziva spolocne prostredie: bezi po jednom s ostatnymi testami nastaveni
        let _env = crate::archive_tests::ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // vlastny priecinok procesu: testy inej kopie (iny pracovny strom, CI matica) nesmu citat cudzi report
        let dir = std::env::temp_dir().join(format!("transom-shot-test-{}", std::process::id()));
        let args: Vec<String> = [
            "x",
            "--shot",
            dir.to_str().unwrap(),
            "--lang",
            "de,ja",
            "--dpi",
            "96,192",
            "--theme",
            "dark",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let new_ui = || {
            let loaded = transom_config::Loaded {
                config: transom_config::Config::default(),
                read_only: false,
                note: None,
                issue: None,
            };
            crate::make_ui(
                loaded,
                transom_render::ThemeMode::System.resolve(true),
                String::new(),
            )
        };
        let code = run(&args, new_ui());
        let report = std::fs::read_to_string(dir.join("settings-audit.txt")).unwrap_or_default();
        assert_eq!(code, 0, "{report}");
        // bez animacii vo Windows (napr. runner GitHubu) sa nahlad Media kresli inak (staticky, skrateny nazov)
        crate::media_page::TEST_ANIMATIONS.with(|a| a.set(false));
        let mut media: Vec<String> = args.clone();
        media.extend(["--page".to_string(), "media".to_string()]);
        let code = run(&media, new_ui());
        crate::media_page::TEST_ANIMATIONS.with(|a| a.set(true));
        let report = std::fs::read_to_string(dir.join("settings-audit.txt")).unwrap_or_default();
        assert_eq!(code, 0, "bez animacii: {report}");
        transom_i18n::set_lang(Lang::En);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
