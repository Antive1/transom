//! Export a import nastaveni (prenos medzi strojmi / zaloha). Ciste funkcie bez suborov a okien: subor cita a zapisuje UI,
//! zapis ide cez existujuce `save_merged` (zamok, trojcestne zlucenie, atomicky zapis, poskodeny/novsi subor sa neprepise).
//!
//! **Co sa exportuje (biela listina, nie serializacia `Config`):** rozlozenie lisaty (len zname polozky), hodiny, posuvny text
//! medii, vzhlad (farba, nepriehladnost, priehladnost, ikona v oblasti oznameni), jazyk, interval obnovy AI, poskytovatel kruhu
//! a rezim kruhu.
//!
//! **Co sa nikdy neexportuje ani neimportuje:** zapnutie zdrojov AI (na inom stroji by spustilo citanie lokalnych prepisov),
//! archiv, rucne datumy fakturacie, vyber monitorov (ID su viazane na stroj), preferovana aplikacia medii (AUMID je viazany na
//! stroj), nezname kluce, ziadne cesty, poverenia, data o vyuziti, obsah schranky ani police.
//!
//! Import nic nezapise sam: `parse_import` subor overi, `preview` ukaze zmeny a az `apply_import` (po potvrdeni pouzivatelom)
//! vytvori novu konfiguraciu. Hodnoty idu cez rovnake tolerantne citanie ako `config.json` (`parse`), takze import nemoze
//! obist limity (napr. interval obnovy AI pod 5 minut alebo nepriehladnost pod minimum).
use crate::{parse, Config, CURRENT_VERSION};
use serde_json::{json, Map, Value};

/// Identifikator formatu exportu.
pub const FORMAT: &str = "transom-settings-export";
/// Verzia formatu exportu (nezavisla od verzie schemy `config.json`).
pub const FORMAT_VERSION: u64 = 1;
/// Najvacsi prijaty subor importu (bajty); UI ma velkost overit uz pred citanim suboru.
pub const MAX_IMPORT_BYTES: usize = 256 * 1024;

/// Sekcia exportu (a jednotka vyberu pri importe).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    Layout,
    Clock,
    Media,
    Appearance,
    Language,
    Usage,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Layout,
        Section::Clock,
        Section::Media,
        Section::Appearance,
        Section::Language,
        Section::Usage,
    ];

    /// Kluc sekcie v subore exportu.
    pub fn key(self) -> &'static str {
        match self {
            Section::Layout => "layout",
            Section::Clock => "clock",
            Section::Media => "media",
            Section::Appearance => "appearance",
            Section::Language => "language",
            Section::Usage => "usage",
        }
    }
}

/// Jazyky aplikacie (zhoda s `transom_i18n::Lang::ALL`; test v `transom-settings`), plus `system`. Import prijme len tieto kody.
pub const KNOWN_LANGUAGES: [&str; 12] = [
    "system", "sk", "en", "cs", "de", "fr", "es", "pl", "it", "pt", "ja", "zh-Hans",
];

/// Povolene polia sekcii, ktore nie su exportovane cele.
const MEDIA_FIELDS: [&str; 4] = [
    "marquee",
    "marquee_direction",
    "marquee_speed",
    "marquee_paused",
];
const APPEARANCE_FIELDS: [&str; 8] = [
    "bg_color",
    "bg_opacity",
    "bg_transparent",
    "tray_icon",
    "glass_panels",
    "glass_bar",
    "text_mode",
    "text_color",
];
const USAGE_FIELDS: [&str; 3] = ["refresh_minutes", "bar_provider", "ring_mode"];

/// Udaje do hlavicky exportu (vstrekovane, aby bol export deterministicky a testovatelny).
#[derive(Clone, Debug, Default)]
pub struct ExportOpts {
    pub app_version: String,
    pub exported_at: String,
}

/// Dovod odmietnutia suboru importu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// Subor je vacsi nez [`MAX_IMPORT_BYTES`].
    TooLarge,
    /// Nie je to platny JSON objekt alebo chyba povinna cast.
    Corrupt,
    /// Iny format (nie export nastaveni transom).
    Foreign,
    /// Novsia verzia formatu, nez pozna tato aplikacia.
    Newer(u64),
}

/// Overeny obsah suboru importu.
#[derive(Clone, Debug)]
pub struct ImportDoc {
    pub app_version: String,
    pub exported_at: String,
    /// Hodnoty sekcii po tolerantnom citani (sekcie, ktore v subore nie su alebo su neplatne, maju predvolene hodnoty).
    pub values: Config,
    /// Platne sekcie, ktore subor obsahuje (len tie sa daju importovat).
    pub present: Vec<Section>,
    /// Co sa zo suboru nepouzije (nezname sekcie a polia, neplatne sekcie, nezname polozky rozlozenia).
    pub ignored: Vec<String>,
}

/// Jedna zmena v nahlade importu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub section: Section,
    pub field: String,
    pub from: String,
    pub to: String,
}

/// Nahlad importu: co by sa zmenilo a co sa ignoruje. Bez zmien = nie je co importovat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportPreview {
    pub changes: Vec<Change>,
    pub ignored: Vec<String>,
}

fn known(id: &str) -> bool {
    crate::is_layout_id(id)
}

/// Hodnota sekcie v tvare suboru exportu (len povolene polia).
fn section_value(cfg: &Config, s: Section) -> Value {
    match s {
        Section::Layout => json!({
            "items": cfg
                .layout
                .items
                .iter()
                .filter(|i| known(&i.id))
                .map(|i| serde_json::to_value(i).unwrap_or(Value::Null))
                .collect::<Vec<_>>()
        }),
        Section::Clock => serde_json::to_value(cfg.clock).unwrap_or(Value::Null),
        Section::Media => json!({
            "marquee": cfg.media.marquee,
            "marquee_direction": cfg.media.marquee_direction,
            "marquee_speed": cfg.media.marquee_speed,
            "marquee_paused": cfg.media.marquee_paused,
        }),
        Section::Appearance => json!({
            "bg_color": cfg.appearance.bg_color,
            "bg_opacity": cfg.appearance.bg_opacity,
            "bg_transparent": cfg.appearance.bg_transparent,
            "tray_icon": cfg.appearance.tray_icon,
            "glass_panels": cfg.appearance.glass_panels,
            "glass_bar": cfg.appearance.glass_bar,
            "text_mode": cfg.appearance.text_mode,
            "text_color": cfg.appearance.text_color,
        }),
        Section::Language => Value::String(cfg.language.clone()),
        Section::Usage => json!({
            "refresh_minutes": cfg.usage.refresh_minutes,
            "bar_provider": cfg.usage.bar_provider,
            "ring_mode": cfg.usage.ring_mode,
        }),
    }
}

/// Export nastaveni do textu (JSON). Obsahuje len bielu listinu sekcii a poli.
pub fn export(cfg: &Config, opts: &ExportOpts) -> String {
    let mut settings = Map::new();
    for s in Section::ALL {
        settings.insert(s.key().to_string(), section_value(cfg, s));
    }
    let doc = json!({
        "format": FORMAT,
        "format_version": FORMAT_VERSION,
        "app_version": opts.app_version,
        "exported_at": opts.exported_at,
        "settings": Value::Object(settings),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// Kratky bezpecny popis hodnoty zo suboru (do nahladu): bez riadiacich znakov, najviac 60 znakov.
fn short(s: &str) -> String {
    // bez riadiacich a obojsmernych / nulovej sirky znakov (podvrhnuty text nesmie menit smer ani skryvat obsah)
    let clean: String = s
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect();
    if clean.chars().count() > 60 {
        format!("{}…", clean.chars().take(60).collect::<String>())
    } else {
        clean
    }
}

/// Ponecha v objekte sekcie len povolene polia; ostatne zaznamena ako ignorovane.
fn keep_fields(v: &Value, section: &str, allowed: &[&str], ignored: &mut Vec<String>) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = Map::new();
            for (k, val) in o {
                if allowed.contains(&k.as_str()) {
                    out.insert(k.clone(), val.clone());
                } else {
                    ignored.push(format!("{section}.{}", short(k)));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Odfiltruje z rozlozenia nezname polozky (z inej / buducej verzie alebo podvrhnute).
fn keep_known_items(v: &Value, ignored: &mut Vec<String>) -> Value {
    let mut v = v.clone();
    if let Some(items) = v.get_mut("items").and_then(Value::as_array_mut) {
        // export zo starsej verzie: tlacidlo `tray` (Win+B) je teraz `tools`
        for it in items.iter_mut() {
            if it.get("id").and_then(Value::as_str) == Some("tray") {
                it["id"] = Value::String("tools".into());
            }
        }
        items.retain(|it| {
            let id = it.get("id").and_then(Value::as_str).unwrap_or("");
            let ok = known(id);
            if !ok {
                ignored.push(format!("layout.items: {}", short(id)));
            }
            ok
        });
    }
    v
}

/// Overi subor importu. Nic nezapisuje; vysledok treba ukazat cez [`preview`] a potvrdit.
pub fn parse_import(text: &str) -> Result<ImportDoc, ImportError> {
    if text.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge);
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let root: Value = serde_json::from_str(text).map_err(|_| ImportError::Corrupt)?;
    let obj = root.as_object().ok_or(ImportError::Corrupt)?;
    if !matches!(obj.get("format").and_then(Value::as_str), Some(FORMAT)) {
        return Err(ImportError::Foreign);
    }
    let version = obj
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or(ImportError::Corrupt)?;
    if version > FORMAT_VERSION {
        return Err(ImportError::Newer(version));
    }
    let settings = obj
        .get("settings")
        .and_then(Value::as_object)
        .ok_or(ImportError::Corrupt)?;
    let text_of = |k: &str| short(obj.get(k).and_then(Value::as_str).unwrap_or(""));
    let mut doc = ImportDoc {
        app_version: text_of("app_version"),
        exported_at: text_of("exported_at"),
        values: Config::default(),
        present: Vec::new(),
        ignored: Vec::new(),
    };
    for key in settings.keys() {
        if !Section::ALL.iter().any(|s| s.key() == key) {
            doc.ignored.push(short(key));
        }
    }
    for s in Section::ALL {
        let Some(raw) = settings.get(s.key()) else {
            continue;
        };
        let mut ignored = Vec::new();
        let mut v = match s {
            Section::Layout => keep_known_items(raw, &mut ignored),
            Section::Media => keep_fields(raw, "media", &MEDIA_FIELDS, &mut ignored),
            Section::Appearance => keep_fields(raw, "appearance", &APPEARANCE_FIELDS, &mut ignored),
            Section::Usage => keep_fields(raw, "usage", &USAGE_FIELDS, &mut ignored),
            Section::Clock | Section::Language => raw.clone(),
        };
        if s == Section::Usage {
            // povolene hodnoty; neznama = predvolena (rovnako ako pri citani konfiguracie)
            if let Some(o) = v.as_object_mut() {
                for (k, allowed, default) in [
                    ("bar_provider", &["auto", "claude", "codex"][..], "auto"),
                    ("ring_mode", &["used", "remaining"][..], "used"),
                ] {
                    if let Some(x) = o.get(k) {
                        if !x.as_str().is_some_and(|x| allowed.contains(&x)) {
                            o.insert(k.to_string(), Value::String(default.to_string()));
                        }
                    }
                }
            }
        }
        // rovnake tolerantne citanie a obmedzenia ako pri config.json; neplatna sekcia sa preskoci (ostatne platia)
        let mut one = Map::new();
        one.insert("version".into(), json!(CURRENT_VERSION));
        one.insert(s.key().into(), v);
        let loaded = parse(&Value::Object(one).to_string());
        if loaded.issue.is_some() {
            doc.ignored.push(format!("{} (invalid)", s.key()));
            continue;
        }
        let c = loaded.config;
        match s {
            Section::Layout => doc.values.layout = c.layout,
            Section::Clock => doc.values.clock = c.clock,
            Section::Media => doc.values.media = c.media,
            Section::Appearance => doc.values.appearance = c.appearance,
            Section::Language => doc.values.language = c.language,
            Section::Usage => doc.values.usage = c.usage,
        }
        doc.present.push(s);
        doc.ignored.extend(ignored);
    }
    Ok(doc)
}

/// Citatelny text hodnoty do nahladu.
fn show(v: &Value) -> String {
    match v {
        Value::String(s) => short(s),
        other => short(&other.to_string()),
    }
}

/// Rozlozenie ako text: skupiny a polozky v poradi, skryte v zatvorkach.
fn layout_summary(cfg: &Config) -> String {
    ["left", "center", "right"]
        .iter()
        .map(|g| {
            let items: Vec<String> = cfg
                .layout
                .items
                .iter()
                .filter(|i| known(&i.id))
                .filter(|i| {
                    serde_json::to_value(i.group)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .as_deref()
                        == Some(*g)
                })
                .map(|i| {
                    if i.visible {
                        i.id.clone()
                    } else {
                        format!("({})", i.id)
                    }
                })
                .collect();
            format!("{g}: {}", items.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Nahlad: ktore polia platnych sekcii sa zmenia oproti aktualnej konfiguracii.
pub fn preview(cfg: &Config, doc: &ImportDoc) -> ImportPreview {
    let mut changes = Vec::new();
    for &s in &doc.present {
        if s == Section::Layout {
            let (a, b) = (layout_summary(cfg), layout_summary(&doc.values));
            if a != b {
                changes.push(Change {
                    section: s,
                    field: "items".into(),
                    from: a,
                    to: b,
                });
            }
            continue;
        }
        let (a, b) = (section_value(cfg, s), section_value(&doc.values, s));
        match (&a, &b) {
            (Value::Object(x), Value::Object(y)) => {
                let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
                keys.sort();
                keys.dedup();
                for k in keys {
                    let (va, vb) = (
                        x.get(k).unwrap_or(&Value::Null),
                        y.get(k).unwrap_or(&Value::Null),
                    );
                    if va != vb {
                        changes.push(Change {
                            section: s,
                            field: k.clone(),
                            from: show(va),
                            to: show(vb),
                        });
                    }
                }
            }
            _ if a != b => changes.push(Change {
                section: s,
                field: String::new(),
                from: show(&a),
                to: show(&b),
            }),
            _ => {}
        }
    }
    ImportPreview {
        changes,
        ignored: doc.ignored.clone(),
    }
}

/// Nova konfiguracia po importe zvolenych sekcii (az po potvrdeni nahladu). Meni **len** povolene polia; zdroje AI, archiv,
/// rucne datumy, vyber monitorov, preferovana aplikacia medii a nezname kluce ostanu z aktualnej konfiguracie. Nezname
/// polozky rozlozenia z aktualnej konfiguracie (z novsej verzie) sa zachovaju.
pub fn apply_import(cfg: &Config, doc: &ImportDoc, sections: &[Section]) -> Config {
    let mut c = cfg.clone();
    let v = &doc.values;
    for s in sections.iter().filter(|s| doc.present.contains(s)) {
        match s {
            Section::Layout => {
                let mut items: Vec<_> = v
                    .layout
                    .items
                    .iter()
                    .filter(|i| known(&i.id))
                    .cloned()
                    .collect();
                items.extend(cfg.layout.items.iter().filter(|i| !known(&i.id)).cloned());
                c.layout.items = items;
            }
            Section::Clock => c.clock = v.clock,
            Section::Media => {
                c.media.marquee = v.media.marquee;
                // volne retazce zo suboru sa normalizuju na zname kody (neznama hodnota = predvolena)
                c.media.marquee_direction = v.media.direction().code().to_string();
                c.media.marquee_speed = v.media.speed().code().to_string();
                c.media.marquee_paused = v.media.marquee_paused;
            }
            Section::Appearance => {
                // hodnoty sa ulozia uz obmedzene (neplatna farba = farba temy, nepriehladnost v povolenom rozsahu)
                c.appearance.bg_color = if crate::parse_hex(&v.appearance.bg_color).is_some() {
                    v.appearance.bg_color.clone()
                } else {
                    String::new()
                };
                c.appearance.bg_opacity = v.appearance.opacity_percent();
                c.appearance.bg_transparent = v.appearance.bg_transparent;
                c.appearance.tray_icon = v.appearance.tray_icon;
                c.appearance.glass_panels = v.appearance.glass_panels;
                c.appearance.glass_bar = v.appearance.glass_bar;
                // rezim uz je overeny pri citani; vlastna farba len platna (inak prazdna = auto)
                c.appearance.text_mode = v.appearance.text_mode.clone();
                c.appearance.text_color = if crate::parse_hex(&v.appearance.text_color).is_some() {
                    v.appearance.text_color.clone()
                } else {
                    String::new()
                };
            }
            Section::Language => {
                // len zname kody jazyka (inak by sa do konfiguracie dostal lubovolne dlhy retazec)
                if KNOWN_LANGUAGES.contains(&v.language.as_str()) {
                    c.language = v.language.clone();
                }
            }
            Section::Usage => {
                c.usage.refresh_minutes = v.usage.refresh_minutes;
                c.usage.bar_provider = v.usage.bar_provider.clone();
                c.usage.ring_mode = v.usage.ring_mode.clone();
            }
        }
    }
    // importovane rozlozenie moze mat polozky v strede, ktory patri odlozenym suborom
    c.enforce_center();
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backup_file, load, save, save_merged, update, Group, MIN_OPACITY_PERCENT,
        MIN_REFRESH_MINUTES,
    };

    fn opts() -> ExportOpts {
        ExportOpts {
            app_version: "0.1.0".into(),
            exported_at: "2026-11-21T10:00:00Z".into(),
        }
    }

    /// Konfiguracia so vsetkym, co sa exportovat NESMIE.
    fn private_config() -> Config {
        let mut c = Config::default();
        c.usage.claude_local = true;
        c.usage.claude_statusline = true;
        c.usage.codex_local = true;
        c.usage.archive.enabled = true;
        c.media.prefer_app = "Spotify.exe".into();
        c.displays.mode = "selected".into();
        c.displays.selected = vec![crate::DisplayRef {
            id: r"\\?\DISPLAY#GSM5B7F#5&1234&0&UID4353#{e6f07b5f}".into(),
            label: "LG".into(),
        }];
        c.extra
            .insert("secret_future_key".into(), json!({"token": "abc"}));
        c.layout.items.push(crate::Slot {
            id: "weather-from-future".into(),
            group: Group::Left,
            visible: true,
        });
        // nesukromne, ale zmenene hodnoty
        c.appearance.bg_color = "#102030".into();
        c.appearance.bg_opacity = 70;
        c.language = "de".into();
        c.clock.seconds = true;
        c
    }

    #[test]
    fn audio_panel_and_ui_sound_settings_are_neither_exported_nor_imported() {
        let mut c = Config::default();
        c.audio_panel.set("tray", false);
        c.ui_sounds.enabled = true;
        c.ui_sounds.volume = 77;
        // polozka lisaty `audio` je sucastou rozlozenia (export ju ma); panel a zvuky nie
        c.tools.clipboard.enabled = true;
        c.tools.clipboard.excluded_apps = vec!["secret-app.exe".into()];
        c.tools.launcher.excluded_folders = vec!["C:\\\\Private".into()];
        c.weather_places = vec![crate::WeatherPlace {
            name: "Secretville".into(),
            region: String::new(),
            lat: Some(1.0),
            lon: Some(2.0),
        }];
        c.weather_source = crate::WeatherSource::Device;
        let text = export(&c, &opts());
        for forbidden in [
            "audio_panel",
            "quick_actions",
            "ui_sounds",
            "volume",
            "\"tools\":",
            // nastavenia schranky (polozka rozlozenia `tool_clipboard` je len tlacidlo v liste)
            "\"clipboard\":",
            "\"enabled\": true",
            "excluded_apps",
            "secret-app",
            "Private",
            "launcher",
            "\"notes\":",
            "screenshots",
            // miesta pocasia a zdroj (poloha) sa neexportuju
            "weather_places",
            "weather_source",
            "Secretville",
        ] {
            assert!(!text.contains(forbidden), "export obsahuje {forbidden:?}");
        }
        // zalozeny subor s tymito sekciami: nic sa neimportuje, oba su ignorovane
        let forged = text.replacen(
            "\"settings\": {",
            "\"settings\": {\"audio_panel\": {\"quick_actions\": []}, \"tools\": {\"clipboard\": {\"enabled\": true}}, \"ui_sounds\": {\"enabled\": true, \"volume\": 100}, ",
            1,
        );
        let doc = parse_import(&forged).expect("platny subor");
        let before = Config::default();
        let pv = preview(&before, &doc);
        assert!(
            pv.ignored.iter().any(|i| i.contains("audio_panel")),
            "{:?}",
            pv.ignored
        );
        assert!(
            pv.ignored.iter().any(|i| i.contains("ui_sounds")),
            "{:?}",
            pv.ignored
        );
        let after = apply_import(&before, &doc, &Section::ALL);
        assert!(
            pv.ignored.iter().any(|i| i.contains("tools")),
            "{:?}",
            pv.ignored
        );
        assert_eq!(after.audio_panel, before.audio_panel);
        assert_eq!(after.ui_sounds, before.ui_sounds);
        assert!(
            !after.tools.clipboard.enabled,
            "import nikdy nezapne schranku"
        );
    }

    #[test]
    fn the_export_contains_only_the_allow_list_and_nothing_private_or_machine_bound() {
        let mut c = private_config();
        c.usage.dates = vec![];
        let text = export(&c, &opts());
        for forbidden in [
            "claude_local",
            "claude_statusline",
            "codex_local",
            "archive",
            "dates",
            "selected",
            "displays",
            "prefer_app",
            "Spotify",
            "secret_future_key",
            "token",
            "weather-from-future",
            "\\\\",
        ] {
            assert!(
                !text.contains(forbidden),
                "export obsahuje {forbidden:?}:\n{text}"
            );
        }
        assert!(!text.contains('\\'), "ziadna spatna lomka (cesty)");
        for expected in [
            "\"format\": \"transom-settings-export\"",
            "#102030",
            "\"language\": \"de\"",
            "bg_opacity",
        ] {
            assert!(text.contains(expected), "chyba {expected:?}");
        }
    }

    #[test]
    fn export_and_import_round_trip_every_section() {
        for c in [Config::default(), private_config()] {
            let doc = parse_import(&export(&c, &opts())).expect("vlastny export je platny");
            assert_eq!(doc.present, Section::ALL.to_vec());
            assert!(doc.ignored.is_empty(), "{:?}", doc.ignored);
            let applied = apply_import(&Config::default(), &doc, &Section::ALL);
            for s in Section::ALL {
                assert_eq!(section_value(&applied, s), section_value(&c, s), "{s:?}");
            }
            assert_eq!(doc.app_version, "0.1.0");
        }
    }

    #[test]
    fn separators_round_trip_through_export_and_import() {
        let mut c = Config::default();
        c.layout.add_separator(crate::Group::Left);
        c.layout.add_separator(crate::Group::Right);
        let doc = parse_import(&export(&c, &opts())).expect("vlastny export je platny");
        assert!(doc.ignored.is_empty(), "{:?}", doc.ignored);
        let applied = apply_import(&Config::default(), &doc, &[Section::Layout]);
        assert_eq!(applied.layout, c.layout);
    }

    #[test]
    fn glass_choices_round_trip_through_export_and_import() {
        let mut c = Config::default();
        c.appearance.glass_panels = false;
        c.appearance.glass_bar = true;
        let doc = parse_import(&export(&c, &opts())).expect("vlastny export je platny");
        let applied = apply_import(&Config::default(), &doc, &[Section::Appearance]);
        assert!(!applied.appearance.glass_panels && applied.appearance.glass_bar);
    }

    #[test]
    fn damaged_foreign_newer_or_oversized_files_are_refused() {
        assert_eq!(
            parse_import("{ nie je json").unwrap_err(),
            ImportError::Corrupt
        );
        assert_eq!(parse_import("[1, 2]").unwrap_err(), ImportError::Corrupt);
        assert_eq!(
            parse_import(r#"{"format": "other-app", "format_version": 1, "settings": {}}"#)
                .unwrap_err(),
            ImportError::Foreign
        );
        assert_eq!(
            parse_import(r#"{"version": 7, "layout": {}}"#).unwrap_err(),
            ImportError::Foreign,
            "config.json nie je export"
        );
        assert_eq!(
            parse_import(
                r#"{"format": "transom-settings-export", "format_version": 2, "settings": {}}"#
            )
            .unwrap_err(),
            ImportError::Newer(2)
        );
        assert_eq!(
            parse_import(r#"{"format": "transom-settings-export", "settings": {}}"#).unwrap_err(),
            ImportError::Corrupt
        );
        assert_eq!(
            parse_import(r#"{"format": "transom-settings-export", "format_version": 1}"#)
                .unwrap_err(),
            ImportError::Corrupt
        );
        let huge = format!(
            r#"{{"format": "transom-settings-export", "format_version": 1, "settings": {{"language": "{}"}}}}"#,
            "x".repeat(MAX_IMPORT_BYTES)
        );
        assert_eq!(parse_import(&huge).unwrap_err(), ImportError::TooLarge);
        // BOM na zaciatku je v poriadku
        let ok = format!("\u{feff}{}", export(&Config::default(), &opts()));
        assert!(parse_import(&ok).is_ok());
    }

    #[test]
    fn a_forged_file_cannot_enable_ai_sources_or_the_archive_or_bypass_limits() {
        let forged = r##"{
            "format": "transom-settings-export", "format_version": 1,
            "settings": {
                "usage": {"refresh_minutes": 1, "bar_provider": "evil", "ring_mode": "remaining",
                          "claude_local": true, "codex_local": true, "archive": {"enabled": true},
                          "dates": [{"provider": "claude", "kind": "billing", "date": "2026-01-01"}]},
                "appearance": {"bg_opacity": 1, "bg_color": "#ffffff", "extra_path": "C:\\Windows"},
                "media": {"prefer_app": "Evil.exe", "marquee": false},
                "layout": {"items": [{"id": "clock", "group": "left", "visible": true},
                                     {"id": "C:\\evil", "group": "left", "visible": true}]},
                "displays": {"mode": "selected", "selected": [{"id": "x", "label": "y"}]},
                "profiles": []
            }
        }"##;
        let doc = parse_import(forged).expect("tolerantne precitany");
        let mut current = Config::default();
        current.usage.dates.clear();
        let out = apply_import(&current, &doc, &Section::ALL);
        assert!(!out.usage.claude_local && !out.usage.codex_local && !out.usage.claude_statusline);
        assert!(!out.usage.archive.enabled);
        assert!(out.usage.dates.is_empty(), "rucne datumy sa neimportuju");
        assert_eq!(
            out.displays, current.displays,
            "vyber monitorov sa neimportuje"
        );
        assert_eq!(out.media.prefer_app, current.media.prefer_app);
        assert!(!out.media.marquee, "povolene pole sa prevzalo");
        assert!(
            out.usage.refresh_minutes == 0 || out.usage.refresh_minutes >= MIN_REFRESH_MINUTES,
            "interval obnovy: {}",
            out.usage.refresh_minutes
        );
        assert!(
            out.appearance.bg_opacity >= MIN_OPACITY_PERCENT,
            "nepriehladnost {}",
            out.appearance.bg_opacity
        );
        assert_eq!(
            out.usage.bar_provider, "auto",
            "neznamy poskytovatel = predvoleny"
        );
        assert_eq!(out.usage.ring_mode, "remaining");
        assert!(
            !out.layout.items.iter().any(|i| i.id.contains('\\')),
            "neznama polozka sa neimportuje"
        );
        assert!(out
            .layout
            .items
            .iter()
            .any(|i| i.id == "clock" && i.group == Group::Left));
        // nahlad vysvetli, co sa ignoruje
        let p = preview(&current, &doc);
        for expected in [
            "usage.claude_local",
            "usage.codex_local",
            "usage.archive",
            "usage.dates",
            "media.prefer_app",
            "appearance.extra_path",
            "displays",
            "profiles",
        ] {
            assert!(
                p.ignored.iter().any(|i| i == expected),
                "{expected} v {:?}",
                p.ignored
            );
        }
        assert!(p.ignored.iter().any(|i| i.starts_with("layout.items: C:")));
    }

    #[test]
    fn an_invalid_section_is_skipped_and_the_others_still_apply() {
        let text = r##"{"format": "transom-settings-export", "format_version": 1,
            "settings": {"clock": {"hour": "25"}, "language": "sk", "appearance": {"bg_color": "#00ff00"}}}"##;
        let doc = parse_import(text).unwrap();
        assert!(!doc.present.contains(&Section::Clock));
        assert!(
            doc.present.contains(&Section::Language) && doc.present.contains(&Section::Appearance)
        );
        assert!(doc.ignored.iter().any(|i| i == "clock (invalid)"));
        let out = apply_import(&Config::default(), &doc, &Section::ALL);
        assert_eq!(
            out.clock,
            Config::default().clock,
            "neplatna sekcia sa neaplikuje"
        );
        assert_eq!(out.language, "sk");
        assert_eq!(out.appearance.bg_color, "#00ff00");
    }

    #[test]
    fn the_preview_lists_exactly_what_changes_and_an_identical_file_changes_nothing() {
        let cur = Config::default();
        let same = parse_import(&export(&cur, &opts())).unwrap();
        assert!(preview(&cur, &same).changes.is_empty(), "nic na import");
        let mut other = cur.clone();
        other.appearance.bg_opacity = 60;
        other.language = "ja".into();
        other.layout.set_visible("ai", true);
        let doc = parse_import(&export(&other, &opts())).unwrap();
        let p = preview(&cur, &doc);
        let fields: Vec<(Section, &str)> = p
            .changes
            .iter()
            .map(|c| (c.section, c.field.as_str()))
            .collect();
        assert!(fields.contains(&(Section::Appearance, "bg_opacity")));
        assert!(fields.contains(&(Section::Language, "")));
        assert!(fields.contains(&(Section::Layout, "items")));
        assert_eq!(p.changes.len(), 3, "{:?}", p.changes);
        let opacity = p.changes.iter().find(|c| c.field == "bg_opacity").unwrap();
        assert_eq!((opacity.from.as_str(), opacity.to.as_str()), ("100", "60"));
        // vyber sekcii: importuje sa len zvolena
        let only_lang = apply_import(&cur, &doc, &[Section::Language]);
        assert_eq!(only_lang.language, "ja");
        assert_eq!(only_lang.appearance, cur.appearance);
        assert_eq!(only_lang.layout, cur.layout);
    }

    #[test]
    fn importing_keeps_unknown_future_items_of_the_current_layout() {
        let mut cur = Config::default();
        cur.layout.items.push(crate::Slot {
            id: "weather".into(),
            group: Group::Right,
            visible: true,
        });
        let doc = parse_import(&export(&Config::default(), &opts())).unwrap();
        let out = apply_import(&cur, &doc, &[Section::Layout]);
        assert!(
            out.layout.items.iter().any(|i| i.id == "weather"),
            "polozka z novsej verzie ostala"
        );
    }

    #[test]
    fn an_import_saved_with_save_merged_keeps_a_concurrent_bar_change_and_never_writes_a_read_only_file(
    ) {
        let d = std::env::temp_dir().join(format!("transom-transfer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("config.json");
        save(&p, &Config::default()).unwrap();
        // okno nastaveni nacita konfiguraciu a pripravi import
        let base = load(&p).config;
        let mut src = Config::default();
        src.appearance.bg_color = "#abcdef".into();
        let doc = parse_import(&export(&src, &opts())).unwrap();
        let ours = apply_import(&base, &doc, &Section::ALL);
        // medzitym ina cast (lista / iny proces) zmeni pole mimo importu
        update(&p, |c| c.displays.mode = "primary".into()).unwrap();
        let merged = save_merged(&p, &base, &ours).unwrap();
        assert_eq!(merged.appearance.bg_color, "#abcdef", "import prevzaty");
        assert_eq!(merged.displays.mode, "primary", "suběžna zmena zachovana");
        assert_eq!(load(&p).config, merged);
        // poskodeny subor: import sa nezapise a subor ostane bajt po bajte
        std::fs::write(&p, "{ poskodene").unwrap();
        assert!(save_merged(&p, &base, &ours).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ poskodene");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn imported_language_and_marquee_strings_are_normalized_to_known_codes_and_never_stored_raw() {
        let long = "x".repeat(100_000);
        let forged = format!(
            r#"{{"format": "transom-settings-export", "format_version": 1, "settings": {{
                "language": "{long}",
                "media": {{"marquee_direction": "{long}", "marquee_speed": "warp-9"}}}}}}"#
        );
        let doc = parse_import(&forged).expect("tolerantne precitane");
        let cur = Config::default();
        let out = apply_import(&cur, &doc, &Section::ALL);
        assert_eq!(out.language, cur.language, "neznamy jazyk sa neulozi");
        assert_eq!(out.media.marquee_direction, "ltr");
        assert_eq!(out.media.marquee_speed, "normal");
        assert!(
            serde_json::to_string(&out).unwrap().len() < 20_000,
            "ziaden obrovsky retazec v konfiguracii"
        );
        // znamy jazyk prejde
        let ok = parse_import(r#"{"format": "transom-settings-export", "format_version": 1, "settings": {"language": "zh-Hans"}}"#).unwrap();
        assert_eq!(apply_import(&cur, &ok, &Section::ALL).language, "zh-Hans");
        for code in KNOWN_LANGUAGES {
            assert!(!code.is_empty() && code.len() <= 8);
        }
    }

    #[test]
    fn preview_text_from_a_file_has_no_control_bidi_or_zero_width_characters() {
        let text = "a\u{202E}b\u{200B}c\u{2066}d\u{FEFF}e\u{0007}f";
        assert_eq!(short(text), "abcdef");
        let forged = r#"{"format": "transom-settings-export", "format_version": 1, "app_version": "1.0\u202E.9",
            "exported_at": "x\u200By", "settings": {"unknown\u202Esection": {}}}"#;
        let doc = parse_import(forged).unwrap();
        assert_eq!(doc.app_version, "1.0.9");
        assert_eq!(doc.exported_at, "xy");
        assert!(
            doc.ignored.iter().all(|i| !i.contains('\u{202E}')),
            "{:?}",
            doc.ignored
        );
    }

    #[test]
    fn backups_are_raw_copies_numbered_never_overwritten_and_pruned_only_among_themselves() {
        let d = std::env::temp_dir().join(format!("transom-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("config.json");
        assert_eq!(
            backup_file(&p, "20261121-100000", 5).unwrap(),
            None,
            "bez configu nie je co zalohovat"
        );
        assert!(!d.join("backups").exists());
        let original = "{ \"version\": 7, \"weird_key\": [1, 2,3] }\n";
        std::fs::write(&p, original).unwrap();
        let b1 = backup_file(&p, "20261121-100000", 5).unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(&b1).unwrap(),
            original,
            "surove bajty"
        );
        let b2 = backup_file(&p, "20261121-100000", 5).unwrap().unwrap();
        assert_ne!(b1, b2, "rovnaky stamp neprepise zalohu");
        assert!(b2.to_string_lossy().ends_with("-1.json"));
        // cudzie subory a priecinky sa nikdy nemazu ani nepocitaju
        let dir = d.join("backups");
        std::fs::write(dir.join("notes.txt"), "moje").unwrap();
        std::fs::write(dir.join("config-final.json"), "moje").unwrap();
        std::fs::create_dir_all(dir.join("config-20200101-000000.json")).unwrap();
        for i in 0..8 {
            backup_file(&p, &format!("20261121-1{i:05}"), 5).unwrap();
        }
        let mut mine: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().unwrap().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("config-2"))
            .collect();
        mine.sort();
        assert_eq!(mine.len(), 5, "{mine:?}");
        assert!(
            mine.last().unwrap().contains("100007"),
            "ostali najnovsie: {mine:?}"
        );
        assert!(dir.join("notes.txt").exists() && dir.join("config-final.json").exists());
        assert!(dir.join("config-20200101-000000.json").is_dir());
        for n in [
            "config-20261121-100000.json",
            "config-20261121-100000-12.json",
        ] {
            assert!(crate::is_backup_name(n), "{n}");
        }
        for n in [
            "config-final.json",
            "config-20261121-100000x.json",
            "config-2026-100000.json",
            "config-20261121-100000-.json",
            "xconfig-20261121-100000.json",
        ] {
            assert!(!crate::is_backup_name(n), "{n}");
        }
        // zaloha pri chybe (na mieste priecinka je subor) zlyha a config ostane
        let d2 = std::env::temp_dir().join(format!("transom-backup2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d2);
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(d2.join("config.json"), original).unwrap();
        std::fs::write(d2.join("backups"), "subor").unwrap();
        assert!(backup_file(&d2.join("config.json"), "20261121-100000", 5).is_err());
        assert_eq!(
            std::fs::read_to_string(d2.join("config.json")).unwrap(),
            original
        );
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d2);
    }

    #[test]
    fn write_atomic_replaces_files_leaves_no_temp_and_does_not_create_directories() {
        let d = std::env::temp_dir().join(format!("transom-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("export.json");
        crate::write_atomic(&p, b"one").unwrap();
        crate::write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let leftovers = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        assert!(crate::write_atomic(&d.join("missing").join("x.json"), b"x").is_err());
        assert!(!d.join("missing").exists(), "priecinok sa nevytvara");
        let _ = std::fs::remove_dir_all(&d);
    }
}
