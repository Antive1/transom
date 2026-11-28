//! Konfiguracia transom: typy, nacitanie, migracia a atomicky zapis. Zdielaju ju lista (`transom-app`) aj okno
//! nastaveni (`transom-settings`).
//!
//! Format: JSON v `%APPDATA%\transom\config.json` (odchylka od povodneho planu `config.toml`: JSON parser je aj tak
//! potrebny na udaje o vyuziti a nema dalsie zavislosti). Pole `version` urcuje schemu:
//! - neznamy kluc na najvyssej urovni a neznama polozka rozlozenia sa pri ulozeni **zachovaju** (buduce polozky
//!   nerozbiju starsiu verziu a naopak);
//! - chybajuca znama polozka sa doplni na predvolene miesto (nove polozky v buducich verziach);
//! - subor s **novsou** verziou alebo poskodeny subor sa nacita len na citanie (`read_only`) a neprepise sa;
//! - cesta sa da prepisat premennou `TRANSOM_CONFIG` (testy a izolovane behy nikdy nesiahnu na skutocny subor).
pub mod transfer;
pub mod weather;
pub use weather::{WeatherPlace, WeatherSource, WeatherTarget};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Verzia schemy. 2 (2026-11-20): `language`, `usage.bar_provider`, `usage.ring_mode`; viditelnost polozky sa uz
/// pri presune nemeni (skryta polozka ostava vo svojej skupine). 3 (2026-11-20): `media.marquee*`, `appearance`;
/// predvoleny interval obnovy AI 5 min (stara predvolba 60 sa pri migracii zmeni na 5), minimum 5 min.
/// 4 (2026-11-20): `appearance.tray_icon` (ikona transom v oblasti oznameni, predvolene zapnuta).
/// 5 (2026-11-20): `usage.dates` (rucne zadane datumy resetu a fakturacie pre kalendar; predvolene prazdne).
/// 6 (2026-11-20): `displays` (na ktorych monitoroch je lista: vsetky / primarny / vybrane; predvolene vsetky).
pub const CURRENT_VERSION: u32 = 7;

/// Najmensi povoleny automaticky interval obnovy udajov AI v minutach (0 = len na poziadanie).
pub const MIN_REFRESH_MINUTES: u32 = 5;
/// Najvacsi ulozeny interval obnovy (7 dni).
pub const MAX_REFRESH_MINUTES: u32 = 10_080;
/// Najmensia nepriehladnost pozadia listy v percentach (nizsie hodnoty len cez volbu "Priehladne").
pub const MIN_OPACITY_PERCENT: u32 = 10;

/// Zname polozky lisaty (id v konfiguracii) v predvolenom poradi.
pub const KNOWN_ITEMS: [&str; 24] = [
    "search",
    "media",
    "window",
    "desktops",
    "shelf",
    "ai",
    "audio",
    "tools",
    "tool_shelf",
    "tool_clipboard",
    "tool_notes",
    "tool_shots",
    "tool_snip",
    "tool_picker",
    "focus",
    "weather",
    "peripherals",
    "sysmon",
    "privacy",
    "brightness",
    "timer",
    "keyboard",
    "battery",
    "clock",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub id: String,
    #[serde(default)]
    pub group: Group,
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}

fn system_language() -> String {
    "system".into()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// Poradie zlava doprava v ramci skupiny.
    pub items: Vec<Slot>,
}

impl Default for Layout {
    fn default() -> Self {
        let slot = |id: &str, group, visible| Slot {
            id: id.into(),
            group,
            visible,
        };
        Layout {
            items: vec![
                slot("search", Group::Left, true),
                slot("media", Group::Left, true),
                // nazov aktivneho okna a virtualne plochy: predvolene skryte
                slot("window", Group::Left, false),
                slot("desktops", Group::Left, false),
                // subory Police v strede listy (zobrazia sa, len ked je na Polici aspon jeden subor)
                slot("shelf", Group::Center, true),
                slot("ai", Group::Right, false),
                slot("audio", Group::Right, false),
                slot("tools", Group::Right, true),
                // samostatne tlacidla nastrojov (otvoria panel rovno na danej casti): predvolene skryte
                slot("tool_shelf", Group::Right, false),
                slot("tool_clipboard", Group::Right, false),
                slot("tool_notes", Group::Right, false),
                slot("tool_shots", Group::Right, false),
                slot("tool_snip", Group::Right, false),
                slot("tool_picker", Group::Right, false),
                slot("focus", Group::Right, false),
                slot("weather", Group::Right, false),
                // zobrazi sa len, ked niektore zariadenie hlasi bateriu
                slot("peripherals", Group::Right, true),
                // vytazenie systemu, mikrofon/kamera, jas monitora, casovac, klavesnica: predvolene skryte
                slot("sysmon", Group::Right, false),
                slot("privacy", Group::Right, false),
                slot("brightness", Group::Right, false),
                slot("timer", Group::Right, false),
                slot("keyboard", Group::Right, false),
                slot("battery", Group::Right, true),
                slot("clock", Group::Right, true),
            ],
        }
    }
}

/// Najviac oddelovacov v liste (id `sep-1` az `sep-8`).
pub const MAX_SEPARATORS: u8 = 8;

/// Cislo oddelovaca z id `sep-N` (1..=`MAX_SEPARATORS`).
pub fn separator_no(id: &str) -> Option<u8> {
    let n: u8 = id.strip_prefix("sep-")?.parse().ok()?;
    (1..=MAX_SEPARATORS).contains(&n).then_some(n)
}

/// Platne id polozky rozlozenia: zname polozky a oddelovace.
pub fn is_layout_id(id: &str) -> bool {
    KNOWN_ITEMS.contains(&id) || separator_no(id).is_some()
}

impl Layout {
    /// Prida viditelny oddelovac na koniec skupiny `g` (najmensie volne cislo); `None` = uz je ich `MAX_SEPARATORS`.
    pub fn add_separator(&mut self, g: Group) -> Option<String> {
        let n = (1..=MAX_SEPARATORS)
            .find(|n| !self.items.iter().any(|s| separator_no(&s.id) == Some(*n)))?;
        let id = format!("sep-{n}");
        let at = self
            .items
            .iter()
            .rposition(|s| s.group == g)
            .map_or(self.items.len(), |i| i + 1);
        self.items.insert(
            at,
            Slot {
                id: id.clone(),
                group: g,
                visible: true,
            },
        );
        Some(id)
    }

    /// Presun polozky `id` do skupiny `g` tesne pred polozku `before` (ta musi byt v skupine `g`); `None` = na koniec
    /// skupiny. Pouziva uprava lisaty priamo na liste (pustenie medzi dve zobrazene polozky). Viditelnost sa nemeni.
    pub fn move_before(&mut self, id: &str, g: Group, before: Option<&str>) {
        if before == Some(id) {
            return;
        }
        let Some(pos) = self.items.iter().position(|s| s.id == id) else {
            return;
        };
        let mut slot = self.items.remove(pos);
        slot.group = g;
        let at = before
            .and_then(|b| self.items.iter().position(|s| s.id == b && s.group == g))
            .or_else(|| self.items.iter().rposition(|s| s.group == g).map(|i| i + 1))
            .unwrap_or(self.items.len());
        self.items.insert(at, slot);
    }

    /// Odstrani oddelovac `id` (ine polozky sa neodstranuju, len skryvaju).
    pub fn remove_separator(&mut self, id: &str) -> bool {
        if separator_no(id).is_none() {
            return false;
        }
        let before = self.items.len();
        self.items.retain(|s| s.id != id);
        self.items.len() != before
    }

    fn default_slot(id: &str) -> Option<Slot> {
        Layout::default().items.into_iter().find(|s| s.id == id)
    }

    /// Doplni chybajuce zname polozky (na predvolenu poziciu v ramci skupiny) a odstrani duplicity.
    pub fn normalize(&mut self) {
        // Stara polozka `tray` (tlacidlo Win+B) sa nahradila polozkou `tools`: premenuje sa na mieste, takze poloha, skupina
        // a viditelnost ostanu; ak su obe, ostane `tools`. Verzia konfiguracie sa kvoli tomu nemeni.
        if self.items.iter().any(|s| s.id == "tools") {
            self.items.retain(|s| s.id != "tray");
        } else if let Some(s) = self.items.iter_mut().find(|s| s.id == "tray") {
            s.id = "tools".into();
        }
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|s| seen.insert(s.id.clone()));
        for id in KNOWN_ITEMS {
            if !self.items.iter().any(|s| s.id == id) {
                if let Some(d) = Layout::default_slot(id) {
                    self.items.push(d);
                }
            }
        }
    }

    /// Viditelne polozky skupiny v poradi.
    pub fn visible_in(&self, g: Group) -> impl Iterator<Item = &Slot> {
        self.items.iter().filter(move |s| s.visible && s.group == g)
    }

    /// Presun polozky `id` do skupiny `g` na poziciu `index` medzi **vsetkymi** polozkami skupiny (aj skrytymi);
    /// viditelnost sa nemeni. Pouziva editor rozlozenia, kde skryte polozky ostavaju vo svojom stlpci.
    pub fn move_within(&mut self, id: &str, g: Group, index: usize) {
        let Some(pos) = self.items.iter().position(|s| s.id == id) else {
            return;
        };
        let mut slot = self.items.remove(pos);
        slot.group = g;
        let in_group: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, s)| s.group == g)
            .map(|(i, _)| i)
            .collect();
        let at = match in_group.get(index) {
            Some(i) => *i,
            None => in_group.last().map(|i| i + 1).unwrap_or(self.items.len()),
        };
        self.items.insert(at, slot);
    }

    /// Polozky skupiny v poradi (aj skryte).
    pub fn in_group(&self, g: Group) -> impl Iterator<Item = &Slot> {
        self.items.iter().filter(move |s| s.group == g)
    }

    /// Presun polozky `id` do skupiny `g` na poziciu `index` medzi viditelnymi polozkami tej skupiny a zobrazenie.
    pub fn move_to(&mut self, id: &str, g: Group, index: usize) {
        let Some(pos) = self.items.iter().position(|s| s.id == id) else {
            return;
        };
        let mut slot = self.items.remove(pos);
        slot.group = g;
        slot.visible = true;
        // index v ramci skupiny -> index v celom zozname
        let mut n = 0;
        let mut at = self.items.len();
        for (i, s) in self.items.iter().enumerate() {
            if s.visible && s.group == g {
                if n == index {
                    at = i;
                    break;
                }
                n += 1;
            }
        }
        if at == self.items.len() {
            // za poslednu polozku skupiny
            if let Some(last) = self.items.iter().rposition(|s| s.visible && s.group == g) {
                at = last + 1;
            }
        }
        self.items.insert(at, slot);
    }

    pub fn set_visible(&mut self, id: &str, visible: bool) {
        if let Some(s) = self.items.iter_mut().find(|s| s.id == id) {
            s.visible = visible;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HourMode {
    #[default]
    Locale,
    #[serde(rename = "12")]
    H12,
    #[serde(rename = "24")]
    H24,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tri {
    #[default]
    Locale,
    Yes,
    No,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateMode {
    #[default]
    None,
    /// Kratky / dlhy datum podla regionalnych nastaveni Windows.
    Short,
    Long,
    /// 25.11.2026
    Dmy,
    /// 25/11/2026
    DmySlash,
    /// 11/25/2026
    Mdy,
    /// 2026-11-25 (ISO 8601)
    Iso,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirstDay {
    #[default]
    Locale,
    Monday,
    Sunday,
    Saturday,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub hour: HourMode,
    pub leading_zero: Tri,
    pub seconds: bool,
    pub weekday: bool,
    /// Cely nazov dna ("streda") namiesto skratky ("st"); nazvy su v jazyku transom.
    pub weekday_long: bool,
    pub date: DateMode,
    pub first_day: FirstDay,
}

/// Zhovievave nacitanie cisla: akykolvek cislo sa nacita (a potom oreze), iny typ = `default`. Chybna hodnota
/// jedneho pola tak neprepne celu konfiguraciu na "len na citanie".
fn lenient_number<'de, D: serde::Deserializer<'de>>(d: D, default: u32) -> Result<u32, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match v.as_f64() {
        Some(f) if f.is_finite() => f.clamp(0.0, u32::MAX as f64) as u32,
        _ => default,
    })
}

fn lenient_bool<'de, D: serde::Deserializer<'de>>(d: D, default: bool) -> Result<bool, D::Error> {
    Ok(Value::deserialize(d)?.as_bool().unwrap_or(default))
}

fn de_refresh<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    lenient_number(d, MIN_REFRESH_MINUTES)
}

fn de_opacity<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    lenient_number(d, 100)
}

/// Predvolena a dovolene nepriehladnosti skla panelov (%).
pub const GLASS_OPACITY_DEFAULT: u32 = 88;
pub const GLASS_OPACITY_STEPS: [u32; 4] = [55, 72, 88, 98];

fn de_glass_opacity<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    Ok(lenient_number(d, GLASS_OPACITY_DEFAULT)?.clamp(40, 100))
}

/// Nazov mesta pre pocasie: orezany, bez riadiacich znakov, najviac 80 znakov (inak prazdny = vypnute).
pub fn normalize_city(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > 80 || t.chars().any(|c| c.is_control()) {
        return String::new();
    }
    t.to_string()
}

fn de_places<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<WeatherPlace>, D::Error> {
    let Value::Array(a) = Value::deserialize(d)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<WeatherPlace> = a
        .into_iter()
        .filter_map(|v| serde_json::from_value::<WeatherPlace>(v).ok())
        .filter(WeatherPlace::valid)
        .collect();
    out.truncate(weather::MAX_PLACES);
    Ok(out)
}

fn de_city<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(normalize_city(&de_string(d)?))
}

fn de_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        _ => String::new(),
    })
}

fn de_true<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    lenient_bool(d, true)
}

fn de_false<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    lenient_bool(d, false)
}

/// Smer posunu textu (laskavy retazec; neznama hodnota = `Ltr`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarqueeDirection {
    /// Citanie zlava doprava: ukaze sa zaciatok, potom sa okno posuva ku koncu (text sa pohybuje doprava->dolava).
    #[default]
    Ltr,
    /// Opacne: ukaze sa koniec a okno sa posuva k zaciatku (text sa pohybuje zlava doprava).
    Rtl,
}

impl MarqueeDirection {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "rtl" => MarqueeDirection::Rtl,
            _ => MarqueeDirection::Ltr,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            MarqueeDirection::Ltr => "ltr",
            MarqueeDirection::Rtl => "rtl",
        }
    }
}

/// Rychlost posunu (laskavy retazec; neznama hodnota = `Normal`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarqueeSpeed {
    Slow,
    #[default]
    Normal,
    Fast,
}

impl MarqueeSpeed {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "slow" => MarqueeSpeed::Slow,
            "fast" => MarqueeSpeed::Fast,
            _ => MarqueeSpeed::Normal,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            MarqueeSpeed::Slow => "slow",
            MarqueeSpeed::Normal => "normal",
            MarqueeSpeed::Fast => "fast",
        }
    }
    /// Rychlost v logickych pixeloch za sekundu.
    pub fn lp_per_s(self) -> f32 {
        match self {
            MarqueeSpeed::Slow => 20.0,
            MarqueeSpeed::Normal => 35.0,
            MarqueeSpeed::Fast => 60.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Media {
    /// Preferovana aplikacia (podretazec AUMID), prazdne = politika vyberu.
    pub prefer_app: String,
    /// Plynuly posun dlheho nazvu (inak staticky text so "...").
    #[serde(deserialize_with = "de_true")]
    pub marquee: bool,
    /// `ltr` (predvolene) alebo `rtl`; pozri [`MarqueeDirection`].
    #[serde(deserialize_with = "de_string")]
    pub marquee_direction: String,
    /// `slow`, `normal` (predvolene) alebo `fast`.
    #[serde(deserialize_with = "de_string")]
    pub marquee_speed: String,
    /// Posuvat text aj pri pozastavenom prehravani (predvolene nie).
    #[serde(deserialize_with = "de_false")]
    pub marquee_paused: bool,
}

impl Default for Media {
    fn default() -> Self {
        Media {
            prefer_app: String::new(),
            marquee: true,
            marquee_direction: "ltr".into(),
            marquee_speed: "normal".into(),
            marquee_paused: false,
        }
    }
}

impl Media {
    pub fn direction(&self) -> MarqueeDirection {
        MarqueeDirection::parse(&self.marquee_direction)
    }
    pub fn speed(&self) -> MarqueeSpeed {
        MarqueeSpeed::parse(&self.marquee_speed)
    }
}

/// Rychle akcie panela Zvuk (id v konfiguracii) v kanonickom poradi. Kazda len **otvara** natívny povrch Windows.
pub const QUICK_ACTIONS: [&str; 5] = [
    "quick_settings",
    "notifications",
    "search",
    "tray",
    "settings",
];

/// Panel Zvuk a rychle akcie: ktore rychle akcie sa zobrazia (poradie je kanonicke, nie poradie v poli). Chybajuci
/// kluc = vsetky; explicitne prazdne pole = riadok sa nezobrazi; neznamy alebo opakovany retazec sa zahodi.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioPanel {
    #[serde(deserialize_with = "de_quick_actions")]
    pub quick_actions: Vec<String>,
}

impl Default for AudioPanel {
    fn default() -> Self {
        AudioPanel {
            quick_actions: QUICK_ACTIONS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl AudioPanel {
    /// Zapnuta rychla akcia `id`?
    pub fn has(&self, id: &str) -> bool {
        self.quick_actions.iter().any(|a| a == id)
    }

    /// Zapne / vypne rychlu akciu (poradie ostava kanonicke).
    pub fn set(&mut self, id: &str, on: bool) {
        let mut v: Vec<String> = QUICK_ACTIONS
            .iter()
            .filter(|a| **a == id && on || **a != id && self.has(a))
            .map(|s| s.to_string())
            .collect();
        v.dedup();
        self.quick_actions = v;
    }
}

fn de_quick_actions<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => {
            let wanted: Vec<&str> = a.iter().filter_map(|v| v.as_str()).collect();
            QUICK_ACTIONS
                .iter()
                .filter(|k| wanted.contains(k))
                .map(|s| s.to_string())
                .collect()
        }
        _ => AudioPanel::default().quick_actions,
    })
}

/// Zdroje baterii periferii (id v konfiguracii) v kanonickom poradi.
pub const PERIPHERAL_SOURCES: [&str; 4] = ["bluetooth", "razer", "logitech", "nuphy"];
/// Kolko zariadeni ukazat v liste (0 = vsetky).
pub const PERIPHERAL_BAR_COUNTS: [u32; 4] = [1, 2, 3, 0];
/// Interval citania v sekundach.
pub const PERIPHERAL_INTERVALS: [u32; 4] = [30, 60, 120, 300];

/// Baterie periferii (polozka `peripherals` lisaty). Len prepinace zdrojov a cisla; zariadenia ani ich urovne sa
/// do konfiguracie nikdy neukladaju. Chybajuci kluc `sources` = vsetky zdroje; neznamy zdroj sa zahodi.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Peripherals {
    #[serde(deserialize_with = "de_peripheral_sources")]
    pub sources: Vec<String>,
    /// Najviac tolko zariadeni v liste (najnizsia uroven prva); 0 = vsetky.
    #[serde(deserialize_with = "de_peripheral_on_bar")]
    pub on_bar: u32,
    #[serde(deserialize_with = "de_peripheral_interval")]
    pub interval_secs: u32,
}

impl Default for Peripherals {
    fn default() -> Self {
        Peripherals {
            sources: PERIPHERAL_SOURCES.iter().map(|s| s.to_string()).collect(),
            on_bar: 3,
            interval_secs: 60,
        }
    }
}

impl Peripherals {
    pub fn has(&self, id: &str) -> bool {
        self.sources.iter().any(|s| s == id)
    }

    /// Zapne / vypne zdroj (poradie ostava kanonicke).
    pub fn set(&mut self, id: &str, on: bool) {
        self.sources = PERIPHERAL_SOURCES
            .iter()
            .filter(|s| **s == id && on || **s != id && self.has(s))
            .map(|s| s.to_string())
            .collect();
    }
}

/// Najblizsia povolena hodnota (pri rovnakej vzdialenosti mensia).
fn nearest(v: u32, allowed: &[u32]) -> u32 {
    allowed
        .iter()
        .copied()
        .min_by_key(|a| (a.abs_diff(v), *a))
        .unwrap_or(v)
}

fn de_peripheral_sources<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(a) => {
            let wanted: Vec<&str> = a.iter().filter_map(|v| v.as_str()).collect();
            PERIPHERAL_SOURCES
                .iter()
                .filter(|k| wanted.contains(k))
                .map(|s| s.to_string())
                .collect()
        }
        _ => Peripherals::default().sources,
    })
}

fn de_peripheral_on_bar<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    // 0..=3 su platne; viac nez 3 = vsetky (0)
    let v = lenient_number(d, Peripherals::default().on_bar)?;
    Ok(if v > 3 { 0 } else { v })
}

fn de_peripheral_interval<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = lenient_number(d, Peripherals::default().interval_secs)?;
    Ok(nearest(v, &PERIPHERAL_INTERVALS))
}

/// Nastroje (Polica, Schranka, Poznamky, Snimky, spustac): iba prepinace a cisla (limity). Obsah (subory, text schranky, poznamky,
/// snimky, hladania) sa **nikdy** neuklada do `config.json`, ale do samostatnych verzovanych suborov v `%LOCALAPPDATA%\\transom\\tools`.
/// Automaticke zbieranie je vypnute, kym ho pouzivatel vyslovne nezapne (schranka), a nic z tejto sekcie sa neexportuje.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Tools {
    pub shelf: ShelfCfg,
    pub clipboard: ClipboardCfg,
    pub notes: NotesCfg,
    pub screenshots: ShotsCfg,
    pub launcher: LauncherCfg,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShelfCfg {
    /// Polica prijima len polozky, ktore pouzivatel vyslovne pusti na listu / vlozi; zapnuta je predvolene.
    pub enabled: bool,
    /// Pri pusteni vytvorit kopiu (predvolene len odkaz).
    pub copy_by_default: bool,
    pub max_items: u32,
    /// Najvacsi subor, ktory sa smie skopirovat (MB).
    pub max_copy_mb: u32,
    /// Spolu najviac MB kopii.
    pub max_total_mb: u32,
    /// 0 = bez automatickeho odstranovania (pripnute sa nikdy neodstranuju samy).
    pub retention_days: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClipboardCfg {
    /// **Predvolene vypnute.** Zachytava sa az po vyslovnom zapnuti a len nove polozky.
    pub enabled: bool,
    pub paused: bool,
    pub text: bool,
    pub images: bool,
    pub files: bool,
    pub max_items: u32,
    pub max_text_kb: u32,
    pub max_image_mb: u32,
    pub retention_days: u32,
    /// Nazvy .exe (bez cesty), z ktorych sa nezachytava (best-effort).
    pub excluded_apps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NotesCfg {
    pub enabled: bool,
    pub max_notes: u32,
}

/// Kniznica snimok obrazovky: **iba cita** hotove obrazky z priecinkov na disku (nic nekopiruje, nepremenuje ani nemaze).
/// Nastavenia obsahuju len prepinace, cesty priecinkov pouzivatela a limit zoznamu; samotny zoznam a miniatury zijú iba v pamati.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShotsCfg {
    pub enabled: bool,
    /// Zahrnut priecinok Snimky obrazovky z Windows (zisti sa pri kazdom otvoreni cez znamy priecinok `FOLDERID_Screenshots`,
    /// takze sleduje presmerovanie na OneDrive / iny disk; jeho cesta sa do konfiguracie nikdy neuklada).
    pub default_folder: bool,
    /// Dalsie priecinky pouzivatela (absolutne cesty, bez podpriecinkov).
    pub folders: Vec<String>,
    /// Najviac najnovsich snimok v zozname.
    pub max_items: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LauncherCfg {
    pub enabled: bool,
    /// Zdroje: aplikacie, subory a priecinky (index Windows Search), nastavenia Windows, akcie transom, poznamky, polica.
    pub apps: bool,
    pub files: bool,
    pub settings: bool,
    pub actions: bool,
    pub notes: bool,
    pub shelf: bool,
    pub max_results: u32,
    /// **Predvolene vypnute.** Ak sa zapne, posledne hladania sa ukladaju lokalne (nie do `config.json`).
    pub recent_searches: bool,
    /// Priecinky, ktore sa v zdroji suborov vynechaju (cesty; nikdy sa neexportuju).
    pub excluded_folders: Vec<String>,
    /// Priecinky, ktore sa prehladaju, ak index Windows Search nie je dostupny (plytko, s limitmi).
    pub fallback_folders: Vec<String>,
    /// Globalna klavesova skratka, ktora otvori vyhladavac (kanonicky zapis, napr. "Alt+Space"); prazdny = vypnuta.
    /// Zly zapis v subore = predvolena skratka (viz [`Hotkey::parse`]).
    pub hotkey: String,
    /// Skratka plati aj pocas aplikacie na celu obrazovku (hra, video). **Predvolene vypnute**: kym je na monitore listy
    /// aplikacia na celu obrazovku, skratka sa odregistruje a dostane ju ta aplikacia.
    pub hotkey_in_fullscreen: bool,
}

/// Predvolena skratka vyhladavaca.
pub const DEFAULT_LAUNCHER_HOTKEY: &str = "Alt+Space";

/// Globalna klavesova skratka: aspon jeden z modifikatorov Ctrl / Alt / Win (samotny Shift nestaci, inak by skratka
/// kradla bezne pisanie) a jedna klavesa z povolenej mnoziny (pismena, cislice, F1-F24, medzernik).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Virtualny kod klavesy (VK_*).
    pub vk: u32,
}

impl Hotkey {
    /// Nazov klavesy pre povoleny virtualny kod (inak `None`).
    pub fn key_name(vk: u32) -> Option<String> {
        match vk {
            0x20 => Some("Space".into()),
            0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map(|c| c.to_string()),
            0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
            _ => None,
        }
    }

    fn vk_of(name: &str) -> Option<u32> {
        let n = name.to_ascii_uppercase();
        if n == "SPACE" {
            return Some(0x20);
        }
        if n.len() == 1 {
            let c = n.as_bytes()[0];
            return (c.is_ascii_uppercase() || c.is_ascii_digit()).then_some(c as u32);
        }
        let f: u32 = n.strip_prefix('F')?.parse().ok()?;
        (1..=24).contains(&f).then_some(0x6F + f)
    }

    /// Skratka z modifikatorov a klavesy; `None`, ak nie je platna (chyba Ctrl/Alt/Win, nepovolena klavesa alebo bezna
    /// skratka Windows / aplikacii, ktoru by globalna registracia vsade "ukradla": Alt+F4, Ctrl+C/V/X/Z/Y/A/S).
    pub fn new(ctrl: bool, alt: bool, shift: bool, win: bool, vk: u32) -> Option<Hotkey> {
        let only_alt = alt && !ctrl && !shift && !win;
        let only_ctrl = ctrl && !alt && !shift && !win;
        let reserved = (only_alt && vk == 0x73)
            || (only_ctrl && matches!(vk, 0x41 | 0x43 | 0x53 | 0x56 | 0x58 | 0x59 | 0x5A));
        (!reserved && (ctrl || alt || win))
            .then_some(Hotkey {
                ctrl,
                alt,
                shift,
                win,
                vk,
            })
            .filter(|_| Hotkey::key_name(vk).is_some())
    }

    /// Precita zapis "Ctrl+Alt+K" (poradie a velkost pismen su lubovolne, medzery sa ignoruju).
    pub fn parse(s: &str) -> Option<Hotkey> {
        let (mut ctrl, mut alt, mut shift, mut win, mut vk) = (false, false, false, false, None);
        for part in s.split('+') {
            let p = part.trim();
            match p.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "win" | "windows" => win = true,
                _ => {
                    if vk.is_some() {
                        return None;
                    }
                    vk = Some(Hotkey::vk_of(p)?);
                }
            }
        }
        Hotkey::new(ctrl, alt, shift, win, vk?)
    }

    /// Kanonicky zapis (poradie Ctrl, Alt, Shift, Win, klavesa).
    pub fn label(&self) -> String {
        let mut out = Vec::new();
        for (on, n) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if on {
                out.push(n.to_string());
            }
        }
        out.push(Hotkey::key_name(self.vk).unwrap_or_default());
        out.join("+")
    }
}

impl LauncherCfg {
    /// Platna skratka vyhladavaca (`None` = vypnuta).
    pub fn hotkey(&self) -> Option<Hotkey> {
        Hotkey::parse(&self.hotkey)
    }
}

/// Tolerantne citanie skratky: chyba = predvolena; prazdny text / "off" = vypnuta; zly zapis = predvolena.
fn obj_hotkey(o: Option<&Map<String, Value>>, k: &str) -> String {
    match o.and_then(|o| o.get(k)).and_then(Value::as_str) {
        None => DEFAULT_LAUNCHER_HOTKEY.into(),
        Some(s) if s.trim().is_empty() || s.trim().eq_ignore_ascii_case("off") => String::new(),
        Some(s) => Hotkey::parse(s)
            .map(|h| h.label())
            .unwrap_or_else(|| DEFAULT_LAUNCHER_HOTKEY.into()),
    }
}

impl LauncherCfg {
    /// Zoznam priecinkov podla druhu ("excluded" alebo "fallback").
    pub fn folders(&self, kind: &str) -> &[String] {
        if kind == "excluded" {
            &self.excluded_folders
        } else {
            &self.fallback_folders
        }
    }

    pub fn folders_mut(&mut self, kind: &str) -> &mut Vec<String> {
        if kind == "excluded" {
            &mut self.excluded_folders
        } else {
            &mut self.fallback_folders
        }
    }
}

impl Default for Tools {
    fn default() -> Self {
        Tools::from_value(&Value::Null)
    }
}

fn obj_num(o: Option<&Map<String, Value>>, k: &str, default: u32, lo: u32, hi: u32) -> u32 {
    let v = o
        .and_then(|o| o.get(k))
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite())
        .map(|f| f.clamp(0.0, u32::MAX as f64) as u32)
        .unwrap_or(default);
    v.clamp(lo, hi)
}

fn obj_flag(o: Option<&Map<String, Value>>, k: &str, default: bool) -> bool {
    o.and_then(|o| o.get(k))
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn obj_list(
    o: Option<&Map<String, Value>>,
    k: &str,
    max_len: usize,
    max_chars: usize,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(Value::Array(a)) = o.and_then(|o| o.get(k)) {
        for v in a {
            let Some(s) = v.as_str() else { continue };
            let s = s.trim();
            if s.is_empty() || s.chars().count() > max_chars || s.chars().any(|c| c.is_control()) {
                continue;
            }
            if !out.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                out.push(s.to_string());
            }
            if out.len() >= max_len {
                break;
            }
        }
    }
    out
}

/// Najviac vylucenych aplikacii v konfiguracii (UI pridava najviac `transom-settings::tools_page::MAX_APPS`).
pub const TOOLS_MAX_EXCLUDED_APPS: usize = 50;
/// Najdlhsi nazov .exe (znaky).
pub const TOOLS_MAX_APP_NAME: usize = 64;

/// Nazov programu pre zoznam vylucenych aplikacii: len **nazov suboru** `.exe` (bez cesty). Akceptuje aj cestu (zoberie sa
/// posledna zlozka) – tak ho vrati vyber suboru. Odmietne prazdny nazov, nazov bez pripony `.exe`, znaky zakazane v nazvoch
/// suborov Windows, riadiace znaky a prilis dlhy nazov. Nazov sa neoveruje na disku (aplikacia nemusi byt nainstalovana).
/// Pouziva ho len UI pri **pridani**; nacitanie konfiguracie neodstranuje zaznamy (tichy zahod by oslabil sukromie).
pub fn normalize_exe_name(input: &str) -> Option<String> {
    let name = input.rsplit(['\\', '/']).next().unwrap_or("").trim();
    let n = name.chars().count();
    if n <= 4 || n > TOOLS_MAX_APP_NAME {
        return None;
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return None;
    }
    // `get` (nie `split_at`): posledne 4 bajty nemusia lezat na hranici znaku, ak nazov konci viacbajtovym znakom
    let ext = name.get(name.len() - 4..)?;
    let stem = &name[..name.len() - 4];
    if !ext.eq_ignore_ascii_case(".exe") || stem.trim_matches(['.', ' ']).is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Najvacsie povolene hodnoty (limity chrania disk aj pamat pred podvrhnutou konfiguraciou).
pub const TOOLS_MAX_ITEMS: u32 = 2000;
/// Najviac priecinkov snimok zadanych pouzivatelom a najdlhsia cesta (znaky).
pub const TOOLS_MAX_SHOT_FOLDERS: usize = 8;
pub const TOOLS_MAX_FOLDER_CHARS: usize = 1000;
pub const TOOLS_MAX_MB: u32 = 20_000;
pub const TOOLS_MAX_RETENTION_DAYS: u32 = 3650;

impl Tools {
    /// Tolerantne citanie: zly typ alebo mimo rozsahu = predvolena hodnota / orezanie, nikdy chyba celej konfiguracie.
    pub fn from_value(v: &Value) -> Tools {
        let root = v.as_object();
        let sub = |k: &str| root.and_then(|o| o.get(k)).and_then(Value::as_object);
        let (sh, cl, nt, ss, la) = (
            sub("shelf"),
            sub("clipboard"),
            sub("notes"),
            sub("screenshots"),
            sub("launcher"),
        );
        Tools {
            shelf: ShelfCfg {
                enabled: obj_flag(sh, "enabled", true),
                copy_by_default: obj_flag(sh, "copy_by_default", false),
                max_items: obj_num(sh, "max_items", 200, 1, TOOLS_MAX_ITEMS),
                max_copy_mb: obj_num(sh, "max_copy_mb", 256, 1, TOOLS_MAX_MB),
                max_total_mb: obj_num(sh, "max_total_mb", 2048, 1, TOOLS_MAX_MB),
                retention_days: obj_num(sh, "retention_days", 0, 0, TOOLS_MAX_RETENTION_DAYS),
            },
            clipboard: ClipboardCfg {
                enabled: obj_flag(cl, "enabled", false),
                paused: obj_flag(cl, "paused", false),
                text: obj_flag(cl, "text", true),
                images: obj_flag(cl, "images", true),
                files: obj_flag(cl, "files", true),
                max_items: obj_num(cl, "max_items", 100, 1, TOOLS_MAX_ITEMS),
                max_text_kb: obj_num(cl, "max_text_kb", 256, 1, 4096),
                max_image_mb: obj_num(cl, "max_image_mb", 8, 1, 64),
                retention_days: obj_num(cl, "retention_days", 7, 0, TOOLS_MAX_RETENTION_DAYS),
                excluded_apps: obj_list(
                    cl,
                    "excluded_apps",
                    TOOLS_MAX_EXCLUDED_APPS,
                    TOOLS_MAX_APP_NAME,
                ),
            },
            notes: NotesCfg {
                enabled: obj_flag(nt, "enabled", true),
                max_notes: obj_num(nt, "max_notes", 500, 1, TOOLS_MAX_ITEMS),
            },
            screenshots: ShotsCfg {
                enabled: obj_flag(ss, "enabled", true),
                default_folder: obj_flag(ss, "default_folder", true),
                folders: obj_list(
                    ss,
                    "folders",
                    TOOLS_MAX_SHOT_FOLDERS,
                    TOOLS_MAX_FOLDER_CHARS,
                ),
                max_items: obj_num(ss, "max_items", 300, 20, TOOLS_MAX_ITEMS),
            },
            launcher: LauncherCfg {
                enabled: obj_flag(la, "enabled", true),
                apps: obj_flag(la, "apps", true),
                files: obj_flag(la, "files", true),
                settings: obj_flag(la, "settings", true),
                actions: obj_flag(la, "actions", true),
                notes: obj_flag(la, "notes", true),
                shelf: obj_flag(la, "shelf", true),
                max_results: obj_num(la, "max_results", 8, 3, 20),
                recent_searches: obj_flag(la, "recent_searches", false),
                excluded_folders: obj_list(la, "excluded_folders", 50, 260),
                fallback_folders: obj_list(la, "fallback_folders", 20, 260),
                hotkey: obj_hotkey(la, "hotkey"),
                hotkey_in_fullscreen: obj_flag(la, "hotkey_in_fullscreen", false),
            },
        }
    }
}

fn de_tools<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Tools, D::Error> {
    Ok(Tools::from_value(&Value::deserialize(d)?))
}

/// Najvyssia hlasitost zvukov rozhrania (percenta; 100 = plna hlasitost relacie transom, nie systemu).
pub const MAX_SOUND_VOLUME: u32 = 100;

/// Predvolena hlasitost zvukov rozhrania: nizka (pri zapnuti aj po prvom spusteni).
pub const DEFAULT_SOUND_VOLUME: u32 = 25;

/// Zvuky rozhrania: kratky tichy zvuk po umyselnom a uspesnom kliknuti na ovladac listy alebo polozku menu.
/// Zapnute su len na vyslovnu volbu (`enabled` = false po instalacii aj po aktualizacii). `volume` je hlasitost
/// **relacie transom** (percenta; zmena sa nedotkne systemu ani inych aplikacii), mimo rozsahu sa oreze na 0..=100.
/// `enabled = false` (stlmenie) a `volume = 0` znamenaju, ze sa nespusti ziadny zvukovy prud.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSounds {
    #[serde(deserialize_with = "de_false")]
    pub enabled: bool,
    #[serde(deserialize_with = "de_sound_volume")]
    pub volume: u32,
}

impl Default for UiSounds {
    fn default() -> Self {
        UiSounds {
            enabled: false,
            volume: DEFAULT_SOUND_VOLUME,
        }
    }
}

impl UiSounds {
    /// Hlasitost, ktora sa skutocne pouzije: 0 pri stlmeni, inak orezana na 0..=100.
    pub fn effective_volume(&self) -> u32 {
        if self.enabled {
            self.volume.min(MAX_SOUND_VOLUME)
        } else {
            0
        }
    }
}

fn de_sound_volume<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    Ok(lenient_number(d, DEFAULT_SOUND_VOLUME)?.min(MAX_SOUND_VOLUME))
}

/// Vzhlad pozadia listy. Vlastna farba prepisuje **len pozadie** (text a ikony zostavaju podla temy);
/// nepriehladnost mimo 100 % znamena vrstvene okno (viac v `docs/PLAN.md` §8.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// Vlastna farba `#RRGGBB`; prazdne alebo neplatne = farba temy.
    #[serde(deserialize_with = "de_string")]
    pub bg_color: String,
    /// Nepriehladnost v percentach (cislo mimo rozsahu sa oreze na 10..=100).
    #[serde(deserialize_with = "de_opacity")]
    pub bg_opacity: u32,
    /// Uplne priehladne pozadie (nepriehladnost sa ignoruje, ale ostava ulozena).
    #[serde(deserialize_with = "de_false")]
    pub bg_transparent: bool,
    /// Ikona transom (znacka) v oblasti oznameni: klik = nastavenia, pravy klik = ponuka. Predvolene zapnuta.
    #[serde(deserialize_with = "de_true")]
    pub tray_icon: bool,
    /// Cierne sklo pod panelmi, menu a prvkami nastaveni (cez okno jemne vidno obsah pod nim). Predvolene zapnute.
    #[serde(deserialize_with = "de_true")]
    pub glass_panels: bool,
    /// Cierne sklo aj pod samotnou listou (inak lista podla temy Windows alebo vlastnej farby). Predvolene vypnute.
    #[serde(deserialize_with = "de_false")]
    pub glass_bar: bool,
    /// Nepriehladnost skla panelov v percentach (cim menej, tym viac vidno pod panel); predvolene 88.
    #[serde(deserialize_with = "de_glass_opacity")]
    pub glass_opacity: u32,
    /// Farba textu a ikon lisaty: `auto` (podla temy; pri vlastnom aspon z polovice nepriehladnom pozadi sa prepne na
    /// citatelnejsiu), `light`, `dark` alebo `custom` (`text_color`). Neznama hodnota = `auto`.
    #[serde(deserialize_with = "de_text_mode")]
    pub text_mode: String,
    /// Vlastna farba textu `#RRGGBB` (plati pri `text_mode` = `custom`; neplatna = `auto`).
    #[serde(deserialize_with = "de_string")]
    pub text_color: String,
}

/// Povolene rezimy farby textu lisaty.
pub const TEXT_MODES: [&str; 4] = ["auto", "light", "dark", "custom"];

fn de_text_mode<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let s = de_string(d)?.trim().to_ascii_lowercase();
    Ok(if TEXT_MODES.contains(&s.as_str()) {
        s
    } else {
        "auto".into()
    })
}

/// Farba textu lisaty po vyhodnoteni nastavenia.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextChoice {
    Auto,
    Light,
    Dark,
    Custom((u8, u8, u8)),
}

fn val_num(v: &Value, k: &str, def: u32, lo: u32, hi: u32) -> u32 {
    match v.get(k).and_then(Value::as_f64) {
        Some(f) if f.is_finite() => (f.round().clamp(0.0, u32::MAX as f64) as u32).clamp(lo, hi),
        _ => def,
    }
}

fn val_bool(v: &Value, k: &str, def: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(def)
}

fn val_str(v: &Value, k: &str, def: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .map(|s| s.chars().take(200).collect())
        .unwrap_or_else(|| def.to_string())
}

/// Casovac a pomodoro (polozka Casovac). Dlzky v minutach, orezane na rozumny rozsah.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TimerCfg {
    pub work_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    /// Pracovne kola pred dlhou prestavkou.
    pub rounds: u32,
    /// Systemovy zvuk upozornenia na konci.
    pub sound: bool,
}

impl Default for TimerCfg {
    fn default() -> Self {
        TimerCfg {
            work_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            rounds: 4,
            sound: true,
        }
    }
}

impl<'de> Deserialize<'de> for TimerCfg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let z = TimerCfg::default();
        Ok(TimerCfg {
            work_min: val_num(&v, "work_min", z.work_min, 1, 180),
            short_break_min: val_num(&v, "short_break_min", z.short_break_min, 1, 60),
            long_break_min: val_num(&v, "long_break_min", z.long_break_min, 1, 120),
            rounds: val_num(&v, "rounds", z.rounds, 1, 12),
            sound: val_bool(&v, "sound", z.sound),
        })
    }
}

/// Vytazenie systemu (polozka): ktore hodnoty ukazat v liste a ako casto citat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SysMonCfg {
    pub cpu: bool,
    pub ram: bool,
    pub gpu: bool,
    pub net: bool,
    /// Interval citania v sekundach (1-10).
    pub interval_secs: u32,
}

impl Default for SysMonCfg {
    fn default() -> Self {
        SysMonCfg {
            cpu: true,
            ram: true,
            gpu: false,
            net: false,
            interval_secs: 2,
        }
    }
}

impl<'de> Deserialize<'de> for SysMonCfg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let z = SysMonCfg::default();
        Ok(SysMonCfg {
            cpu: val_bool(&v, "cpu", z.cpu),
            ram: val_bool(&v, "ram", z.ram),
            gpu: val_bool(&v, "gpu", z.gpu),
            net: val_bool(&v, "net", z.net),
            interval_secs: val_num(&v, "interval_secs", z.interval_secs, 1, 10),
        })
    }
}

/// Pevne mena zvukovych profilov (kluc textu `prof.<slot>`).
pub const AUDIO_PROFILE_SLOTS: [&str; 6] = ["games", "call", "music", "movie", "work", "night"];

/// Ulozene nastavenie jednej aplikacie v zvukovom profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileApp {
    /// Cesta k .exe malymi pismenami, alebo "system" pre systemove zvuky.
    pub exe: String,
    pub volume: u8,
    #[serde(default)]
    pub muted: bool,
    /// ID vystupu (MMDevice) alebo `None` = predvoleny vystup.
    #[serde(default)]
    pub device: Option<String>,
}

/// Zvukovy profil: hlasitost, stlmenie a vystup aplikacii. Neexportuje sa (obsahuje cesty a ID zariadeni).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioProfile {
    pub slot: String,
    #[serde(default)]
    pub apps: Vec<ProfileApp>,
}

fn de_profiles<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<AudioProfile>, D::Error> {
    let v = Value::deserialize(d)?;
    let mut out: Vec<AudioProfile> = Vec::new();
    for p in v.as_array().into_iter().flatten() {
        let Ok(mut p) = serde_json::from_value::<AudioProfile>(p.clone()) else {
            continue;
        };
        if !AUDIO_PROFILE_SLOTS.contains(&p.slot.as_str()) || out.iter().any(|o| o.slot == p.slot) {
            continue;
        }
        p.apps.truncate(64);
        for a in &mut p.apps {
            a.volume = a.volume.min(100);
        }
        out.push(p);
    }
    Ok(out)
}

/// Najviac pripojenych kalendarov (ICS).
pub const MAX_ICS: usize = 8;

/// Adresa kalendara ICS: `webcal://` -> `https://`, len https bez medzier; inak `None`.
pub fn normalize_ics_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 2048 || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let rest = if lower.starts_with("webcal://") {
        &s[9..]
    } else if lower.starts_with("https://") {
        &s[8..]
    } else {
        return None;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!("https://{rest}"))
}

fn de_ics<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Value::deserialize(d)?;
    let mut out: Vec<String> = Vec::new();
    for u in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
        if let Some(n) = normalize_ics_url(u) {
            if !out.contains(&n) && out.len() < MAX_ICS {
                out.push(n);
            }
        }
    }
    Ok(out)
}

/// Predvolena skratka stlmenia mikrofonu.
pub const DEFAULT_MIC_HOTKEY: &str = "Ctrl+Alt+M";

/// Aktualizacie z vydani na GitHube.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Updates {
    /// Po starte a potom kazdych niekolko hodin sa pozriet na nove vydanie (jedna poziadavka na api.github.com).
    #[serde(default = "yes")]
    pub auto_check: bool,
    /// Ikona stahovania uplne vpravo na liste, kym je dostupna novsia verzia (klik otvori panel aktualizacie).
    #[serde(default = "yes")]
    pub show_icon: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Updates {
            auto_check: true,
            show_icon: true,
        }
    }
}

/// Mikrofon a kamera (polozka): globalna skratka stlmenia mikrofonu.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrivacyCfg {
    /// Skratka stlmenia mikrofonu ("" alebo "off" = vypnuta, neplatna = predvolena).
    pub mic_hotkey: String,
    /// Polozka sa ukazuje aj vtedy, ked mikrofon ani kameru nic nepouziva (inak len pocas pouzivania a pri stlmeni).
    pub always_show: bool,
}

impl Default for PrivacyCfg {
    fn default() -> Self {
        PrivacyCfg {
            mic_hotkey: DEFAULT_MIC_HOTKEY.into(),
            always_show: true,
        }
    }
}

impl PrivacyCfg {
    /// Platna skratka (`None` = vypnuta).
    pub fn hotkey(&self) -> Option<Hotkey> {
        let s = self.mic_hotkey.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("off") {
            return None;
        }
        Hotkey::parse(s).or_else(|| Hotkey::parse(DEFAULT_MIC_HOTKEY))
    }
}

impl<'de> Deserialize<'de> for PrivacyCfg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let z = PrivacyCfg::default();
        Ok(PrivacyCfg {
            mic_hotkey: val_str(&v, "mic_hotkey", &z.mic_hotkey),
            always_show: val_bool(&v, "always_show", z.always_show),
        })
    }
}

/// Vybezok (notch) pod listou: co sa v nom smie ukazat. Kazde spravanie je samostatna volba.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotchCfg {
    /// Odlozene subory po prejdeni mysou nad vycnelkom v strede listy.
    #[serde(deserialize_with = "de_true")]
    pub stash: bool,
    /// Prehravana hudba (obal, priebeh, ovladanie) po prejdeni mysou nad mediami.
    #[serde(deserialize_with = "de_true")]
    pub media: bool,
    /// Koniec casovaca / pomodora.
    #[serde(deserialize_with = "de_true")]
    pub timer: bool,
    /// Najblizsia udalost z kalendara (kratko pred zaciatkom).
    #[serde(deserialize_with = "de_true")]
    pub event: bool,
    /// Nahlad posledne skopirovanych veci po prejdeni mysou nad tlacidlom Schranka (len so zapnutou historiou).
    #[serde(deserialize_with = "de_true")]
    pub clipboard: bool,
    /// Predpoved pocasia po prejdeni mysou nad polozkou Pocasie.
    #[serde(deserialize_with = "de_true")]
    pub weather: bool,
    /// Ostrov v strede: prehravana hudba (nazov novej skladby, potom obal a ekvalizer).
    #[serde(deserialize_with = "de_true")]
    pub island_music: bool,
    /// Ostrov v strede: pracuje Claude Code (logo a limit zo statusline).
    #[serde(deserialize_with = "de_true")]
    pub island_ai: bool,
    /// Nova snimka obrazovky v priecinku Snimky obrazovky.
    #[serde(deserialize_with = "de_true")]
    pub screenshot: bool,
    /// Skopirovane veci ulozene do historie schranky.
    #[serde(deserialize_with = "de_true")]
    pub clip_saved: bool,
}

impl Default for NotchCfg {
    fn default() -> Self {
        NotchCfg {
            stash: true,
            media: true,
            timer: true,
            event: true,
            clipboard: true,
            weather: true,
            island_music: true,
            island_ai: true,
            screenshot: true,
            clip_saved: true,
        }
    }
}

/// Ochrana OLED displejov pred vypalenim statickeho obsahu lisaty. **Predvolene vypnuta** (`enabled`); podvolby platia,
/// len ked je zapnuta. Znizuje riziko, nezarucuje, ze k vypaleniu nedojde.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Oled {
    #[serde(deserialize_with = "de_false")]
    pub enabled: bool,
    /// Obsah sa raz za minutu posunie o 1-2 px (po malom kruhu).
    #[serde(deserialize_with = "de_true")]
    pub pixel_shift: bool,
    /// Text a ikony s nizsim jasom (kontrast nikdy pod 4,5 : 1).
    #[serde(deserialize_with = "de_true")]
    pub dim_text: bool,
    /// Cisto cierne nepriehladne pozadie (prebije vlastnu farbu aj priehladnost, kym je ochrana zapnuta).
    #[serde(deserialize_with = "de_true")]
    pub black_bg: bool,
    /// Bez spodneho okraja a oddelovacov (tenke ciary svietia stale na tom istom mieste).
    #[serde(deserialize_with = "de_true")]
    pub no_lines: bool,
    /// Lista sa skryje (uvolni miesto) a vysunie sa pri mysi na hornom okraji monitora.
    #[serde(deserialize_with = "de_false")]
    pub auto_hide: bool,
}

impl Default for Oled {
    fn default() -> Self {
        Oled {
            enabled: false,
            pixel_shift: true,
            dim_text: true,
            black_bg: true,
            no_lines: true,
            auto_hide: false,
        }
    }
}

impl Oled {
    /// Podvolba plati (ochrana je zapnuta a podvolba tiez).
    pub fn pixel_shift(&self) -> bool {
        self.enabled && self.pixel_shift
    }
    pub fn dim_text(&self) -> bool {
        self.enabled && self.dim_text
    }
    pub fn black_bg(&self) -> bool {
        self.enabled && self.black_bg
    }
    pub fn no_lines(&self) -> bool {
        self.enabled && self.no_lines
    }
    pub fn auto_hide(&self) -> bool {
        self.enabled && self.auto_hide
    }
}

/// Posun obsahu (dx, dy) v logickych px pre fazu `phase` (napr. minuta): kruh 8 poloh do +-2 px vodorovne, +-1 px zvisle.
pub fn pixel_shift_lp(phase: u64) -> (i32, i32) {
    const ORBIT: [(i32, i32); 8] = [
        (0, 0),
        (1, 0),
        (2, 1),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-2, 0),
        (-1, 0),
    ];
    ORBIT[(phase % ORBIT.len() as u64) as usize]
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            bg_color: String::new(),
            bg_opacity: 100,
            bg_transparent: false,
            tray_icon: true,
            glass_panels: true,
            glass_bar: false,
            glass_opacity: GLASS_OPACITY_DEFAULT,
            text_mode: "auto".into(),
            text_color: String::new(),
        }
    }
}

/// `#RRGGBB`, `RRGGBB`, `#RGB` alebo `RGB` (velkost pismen nehra, okolite medzery sa ignoruju).
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let t = s.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    if !t.is_ascii() || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = |i: usize, n: usize| u8::from_str_radix(&t[i..i + n], 16).ok();
    match t.len() {
        6 => Some((v(0, 2)?, v(2, 2)?, v(4, 2)?)),
        3 => {
            let d = |i| v(i, 1).map(|x| x * 17);
            Some((d(0)?, d(1)?, d(2)?))
        }
        _ => None,
    }
}

/// Tri kanaly zadane ako text (kazdy 0..=255, bez znamienka a bez desatinnych miest).
pub fn parse_rgb(r: &str, g: &str, b: &str) -> Option<(u8, u8, u8)> {
    let c = |s: &str| {
        let t = s.trim();
        if t.is_empty() || t.len() > 3 || !t.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        t.parse::<u16>().ok().filter(|v| *v <= 255).map(|v| v as u8)
    };
    Some((c(r)?, c(g)?, c(b)?))
}

/// Relativna luminancia podla WCAG 2.x (sRGB -> linearne).
pub fn luminance(c: (u8, u8, u8)) -> f32 {
    let f = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.039_28 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
}

/// Pomer kontrastu dvoch farieb (1.0 az 21.0; WCAG: text aspon 4.5, velky text/ikony aspon 3).
pub fn contrast_ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Farba `top` s alfou `alpha` (0..=255) nad nepriehladnou farbou `under`.
pub fn blend(top: (u8, u8, u8), under: (u8, u8, u8), alpha: u8) -> (u8, u8, u8) {
    let a = alpha as f32 / 255.0;
    let m = |t: u8, u: u8| (t as f32 * a + u as f32 * (1.0 - a)).round() as u8;
    (m(top.0, under.0), m(top.1, under.1), m(top.2, under.2))
}

pub fn format_hex(c: (u8, u8, u8)) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

impl Appearance {
    /// Vlastna farba pozadia, ak je platna.
    pub fn color(&self) -> Option<(u8, u8, u8)> {
        parse_hex(&self.bg_color)
    }

    /// Nepriehladnost v percentach po orezani (10..=100).
    pub fn opacity_percent(&self) -> u32 {
        self.bg_opacity.clamp(MIN_OPACITY_PERCENT, 100)
    }

    /// Alfa pozadia 0..=255: 0 pri "priehladne", inak z nepriehladnosti.
    pub fn alpha(&self) -> u8 {
        if self.bg_transparent {
            0
        } else {
            ((self.opacity_percent() * 255 + 50) / 100) as u8
        }
    }

    /// Je pozadie plne nepriehladne (stara cesta bez vrstveneho okna, s ClearType)?
    pub fn is_opaque(&self) -> bool {
        self.alpha() == 255
    }

    /// Farba textu po vyhodnoteni (`custom` s neplatnou farbou = `Auto`).
    pub fn text(&self) -> TextChoice {
        match self.text_mode.as_str() {
            "light" => TextChoice::Light,
            "dark" => TextChoice::Dark,
            "custom" => parse_hex(&self.text_color).map_or(TextChoice::Auto, TextChoice::Custom),
            _ => TextChoice::Auto,
        }
    }

    /// Skutocne pozadie lisaty: (vlastna farba, alfa). Ochrana OLED s ciernym pozadim ho nahradi nepriehladnou ciernou.
    pub fn effective(&self, oled: &Oled) -> (Option<(u8, u8, u8)>, u8) {
        if oled.black_bg() {
            (Some((0, 0, 0)), 255)
        } else {
            (self.color(), self.alpha())
        }
    }
}

/// Zdroje udajov o vyuziti AI. Vsetky **predvolene vypnute**; bez zapnutia sa nic necita.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// Lokalne prepisy Claude Code (`~/.claude/projects`, nezdokumentovany format).
    pub claude_local: bool,
    /// Most statusline Claude Code (zdokumentovane rozhranie; zapisuje `transom-statusline`).
    pub claude_statusline: bool,
    /// Lokalne relacie Codex (`~/.codex/sessions`, nezdokumentovany format).
    pub codex_local: bool,
    /// Lokalne relacie Grok CLI (`~/.grok/sessions`, nezdokumentovany format).
    pub grok_local: bool,
    /// Automaticka obnova v minutach; 0 = len na poziadanie, inak najmenej [`MIN_REFRESH_MINUTES`]
    /// (pozri [`Usage::refresh_interval_minutes`]).
    #[serde(deserialize_with = "de_refresh")]
    pub refresh_minutes: u32,
    /// Poskytovatel kruhu v liste: `auto`, `claude`, `codex` (tolerantne - neznama hodnota = auto).
    pub bar_provider: String,
    /// Co ukazuje kruh: `used` (spotrebovane) alebo `remaining` (zostavajuce); neznama hodnota = used.
    pub ring_mode: String,
    /// Rucne zadane datumy resetu limitu a fakturacie (kalendar). Zo zdrojov sa fakturacny datum zistit neda
    /// (ziadne zdokumentovane rozhranie), takze ho zadava pouzivatel. Neplatne polozky sa pri citani vynechaju.
    #[serde(deserialize_with = "de_dates")]
    pub dates: Vec<ManualDate>,
    /// Volitelny lokalny archiv dennych suctov (v7): **predvolene vypnuty**, nikdy sa nezapne sam.
    pub archive: Archive,
}

/// Povolene volby retencie archivu v mesiacoch (0 = bez obmedzenia).
pub const ARCHIVE_RETENTIONS: [u32; 5] = [0, 3, 6, 12, 24];
/// Predvolena retencia po zapnuti archivu (mesiace).
pub const ARCHIVE_DEFAULT_RETENTION: u32 = 12;

/// Volitelny archiv dennych suctov vyuzitia AI (`usage-archive.json` v lokalnych datach transom). Uklada sa **len**
/// denny sucet podla poskytovatela a modelu (tokeny, pocet volani, odhad ceny + datum cennika, priznak neuplneho dna) -
/// nikdy prompty, odpovede, nazvy konverzacii, projekty ani surove logy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Archive {
    /// Archivovanie zapnute pouzivatelom (predvolene nie).
    pub enabled: bool,
    /// Pozastavene: nic sa nezapisuje, uz ulozene data ostavaju (a zobrazuju sa) do zmazania.
    pub paused: bool,
    /// Retencia v mesiacoch (`ARCHIVE_RETENTIONS`; 0 = bez obmedzenia); neznama hodnota = predvolena.
    #[serde(deserialize_with = "de_retention")]
    pub retention_months: u32,
}

impl Default for Archive {
    fn default() -> Self {
        Archive {
            enabled: false,
            paused: false,
            retention_months: ARCHIVE_DEFAULT_RETENTION,
        }
    }
}

impl Archive {
    /// Zapisuje sa teraz? (zapnute a nie pozastavene)
    pub fn writing(&self) -> bool {
        self.enabled && !self.paused
    }
}

/// Tolerantne: neznama alebo nezmyselna hodnota = predvolena retencia (subor nikdy nie je kvoli tomu len na citanie).
fn de_retention<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_u64()
        .map(|n| n.min(u64::from(u32::MAX)) as u32)
        .filter(|n| ARCHIVE_RETENTIONS.contains(n))
        .unwrap_or(ARCHIVE_DEFAULT_RETENTION))
}

/// Najvyssi pocet rucne zadanych datumov.
pub const MAX_MANUAL_DATES: usize = 12;

/// Rucne zadany datum pre kalendar: reset limitu alebo fakturacia/obnovenie predplatneho jedneho poskytovatela.
/// Retazce (nie enum), aby neznama hodnota z buducej verzie neznehodnotila cely subor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualDate {
    /// `claude` alebo `codex`.
    pub provider: String,
    /// `reset` alebo `billing`.
    pub kind: String,
    /// Kotva: znamy datum `YYYY-MM-DD` (miestny kalendarny den).
    pub date: String,
    /// `none` (jednorazovo), `days` (kazdych `every_days` dni) alebo `monthly` (rovnaky den v mesiaci).
    pub repeat: String,
    /// Interval pre `days` (1-366; predvolene 6).
    pub every_days: u32,
}

impl ManualDate {
    /// Platne hodnoty vsetkych poli (datum vratane dna v mesiaci)?
    pub fn is_valid(&self) -> bool {
        ["claude", "codex"].contains(&self.provider.as_str())
            && ["reset", "billing"].contains(&self.kind.as_str())
            && ["none", "days", "monthly"].contains(&self.repeat.as_str())
            && (1..=366).contains(&self.every_days)
            && valid_iso_date(&self.date)
    }
}

/// `YYYY-MM-DD` s platnym dnom (rok 2000-2100).
fn valid_iso_date(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    if p.len() != 3 || p[0].len() != 4 || p[1].len() != 2 || p[2].len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (
        p[0].parse::<i32>(),
        p[1].parse::<u32>(),
        p[2].parse::<u32>(),
    ) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (2000..=2100).contains(&y) && (1..=dim).contains(&d)
}

/// Tolerantne nacitanie zoznamu: neplatne polozky (zly typ, nezname hodnoty, neexistujuci den) sa vynechaju; chybajuci
/// `every_days` = 6; najviac `MAX_MANUAL_DATES`. Nikdy to nespravi konfiguraciu "len na citanie".
fn de_dates<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ManualDate>, D::Error> {
    let Value::Array(items) = Value::deserialize(d)? else {
        return Ok(Vec::new());
    };
    Ok(items
        .into_iter()
        .filter_map(|mut v| {
            if let Value::Object(o) = &mut v {
                o.entry("every_days").or_insert(Value::from(6));
                o.entry("repeat").or_insert(Value::from("none"));
            }
            serde_json::from_value::<ManualDate>(v).ok()
        })
        .filter(ManualDate::is_valid)
        .take(MAX_MANUAL_DATES)
        .collect())
}

impl Default for Usage {
    fn default() -> Self {
        Usage {
            claude_local: false,
            claude_statusline: false,
            codex_local: false,
            grok_local: false,
            refresh_minutes: MIN_REFRESH_MINUTES,
            bar_provider: "auto".into(),
            ring_mode: "used".into(),
            dates: Vec::new(),
            archive: Archive::default(),
        }
    }
}

impl Usage {
    pub fn any_enabled(&self) -> bool {
        self.claude_local || self.claude_statusline || self.codex_local || self.grok_local
    }

    /// Skutocny automaticky interval: `None` = len na poziadanie, inak `MIN_REFRESH_MINUTES..=MAX_REFRESH_MINUTES`
    /// (hodnoty 1-4 sa zvysia na minimum, nikdy sa nezbiera castejsie).
    pub fn refresh_interval_minutes(&self) -> Option<u32> {
        match self.refresh_minutes {
            0 => None,
            m => Some(m.clamp(MIN_REFRESH_MINUTES, MAX_REFRESH_MINUTES)),
        }
    }
}

/// Odkaz na vybrany monitor: stabilne ID (device interface path) a posledny znamy popis (na zobrazenie, ked monitor
/// nie je pripojeny - ID samotne nic nepovie).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayRef {
    pub id: String,
    #[serde(default)]
    pub label: String,
}

/// Na ktorych monitoroch ma byt lista. `mode`: `all` (predvolene), `primary` (vzdy primarny monitor, aj ked sa zmeni) alebo
/// `selected` (len monitory v `selected`; ulozene ID sa nikdy ticho nepresuvaju na iny monitor).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Displays {
    #[serde(deserialize_with = "de_display_mode")]
    pub mode: String,
    #[serde(deserialize_with = "de_display_refs")]
    pub selected: Vec<DisplayRef>,
}

impl Default for Displays {
    fn default() -> Self {
        Displays {
            mode: "all".into(),
            selected: Vec::new(),
        }
    }
}

/// Najvyssi pocet ulozenych monitorov.
pub const MAX_DISPLAY_REFS: usize = 16;

/// Vlastne nastavenia jedneho monitora (stabilne ID ako v `displays.selected`; nikdy sa ticho neprenesu na iny
/// monitor). `None` = ta cast sa berie zo spolocnych nastaveni.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorOverride {
    pub id: String,
    /// Posledny znamy popis (na zobrazenie, ked monitor nie je pripojeny).
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Appearance>,
}

/// Tolerantne: neplatne zaznamy (zly typ, prazdne ID) a duplicity sa vynechaju, najviac `MAX_DISPLAY_REFS`.
fn de_monitors<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<MonitorOverride>, D::Error> {
    let Value::Array(items) = Value::deserialize(d)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<MonitorOverride> = Vec::new();
    for v in items {
        if let Ok(mut m) = serde_json::from_value::<MonitorOverride>(v) {
            if !m.id.trim().is_empty() && !out.iter().any(|o| o.id.eq_ignore_ascii_case(&m.id)) {
                if let Some(l) = &mut m.layout {
                    l.normalize();
                }
                out.push(m);
            }
        }
        if out.len() >= MAX_DISPLAY_REFS {
            break;
        }
    }
    Ok(out)
}

fn de_display_mode<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) if ["all", "primary", "selected"].contains(&s.as_str()) => s,
        _ => "all".into(),
    })
}

/// Tolerantne: neplatne polozky (zly typ, prazdne ID) sa vynechaju, duplicity (bez ohladu na velkost pismen) tiez.
fn de_display_refs<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<DisplayRef>, D::Error> {
    let Value::Array(items) = Value::deserialize(d)? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<DisplayRef> = Vec::new();
    for v in items {
        if let Ok(r) = serde_json::from_value::<DisplayRef>(v) {
            if !r.id.trim().is_empty() && !out.iter().any(|o| o.id.eq_ignore_ascii_case(&r.id)) {
                out.push(r);
            }
        }
        if out.len() >= MAX_DISPLAY_REFS {
            break;
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Chyba = 0 (subor bez verzie -> migracia).
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub clock: Clock,
    #[serde(default)]
    pub media: Media,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub appearance: Appearance,
    /// Zvuky rozhrania (predvolene vypnute).
    #[serde(default)]
    pub ui_sounds: UiSounds,
    /// Panel Zvuk a rychle akcie (polozka `audio` lisaty).
    #[serde(default)]
    pub audio_panel: AudioPanel,
    /// Baterie periferii (polozka `peripherals` lisaty): zdroje, pocet v liste, interval.
    #[serde(default)]
    pub peripherals: Peripherals,
    /// Nastroje: len prepinace a limity (obsah je v samostatnych suboroch).
    #[serde(default, deserialize_with = "de_tools")]
    pub tools: Tools,
    /// Ochrana OLED (predvolene vypnuta).
    #[serde(default)]
    pub oled: Oled,
    /// Casovac a pomodoro.
    #[serde(default)]
    pub timer: TimerCfg,
    /// Vytazenie systemu.
    #[serde(default)]
    pub sysmon: SysMonCfg,
    /// Mikrofon a kamera.
    #[serde(default)]
    pub privacy: PrivacyCfg,
    /// Vybezok pod listou a co v nom ukazovat.
    #[serde(default)]
    pub notch: NotchCfg,
    /// Skupina dlazdic v paneli Nastroje (id v poradi): co si pouzivatel chce mat po ruke. Polozky mozu byt aj samostatne
    /// v liste (Rozlozenie) - skupina je len dalsi pristup.
    #[serde(default = "default_tool_group")]
    pub tool_group: Vec<String>,
    /// Zvukove profily (panel Zvuk).
    #[serde(default, deserialize_with = "de_profiles")]
    pub audio_profiles: Vec<AudioProfile>,
    /// Pripojene kalendare: sukromne odkazy ICS (tajny udaj - nelogue sa, neexportuje sa).
    #[serde(default, deserialize_with = "de_ics")]
    pub ics_calendars: Vec<String>,
    /// Pocasie: stara volba (len nazov mesta). Pri nacitani sa prenesie do `weather_places` a uz sa neuklada.
    #[serde(default, deserialize_with = "de_city", skip_serializing)]
    pub weather_city: String,
    /// Pocasie: ulozene miesta (najviac `weather::MAX_PLACES`). Neexportuju sa.
    #[serde(default, deserialize_with = "de_places")]
    pub weather_places: Vec<WeatherPlace>,
    /// Pocasie v liste: zvolene miesto alebo poloha zariadenia (nic = vypnute, nic sa neodosiela). Neexportuje sa.
    #[serde(default)]
    pub weather_source: WeatherSource,
    /// Na ktorych monitoroch je lista.
    #[serde(default)]
    pub displays: Displays,
    /// Vlastne nastavenia jednotlivych monitorov (polozky a vzhlad lisaty); monitor bez zaznamu pouziva spolocne.
    /// Neexportuju sa (ID monitora plati len na tomto PC).
    #[serde(default, deserialize_with = "de_monitors")]
    pub monitors: Vec<MonitorOverride>,
    /// Aktualizacie: lista sa pozera na nove vydania na GitHube (predvolene zapnute, da sa vypnut). Neexportuje sa.
    #[serde(default)]
    pub updates: Updates,
    /// Jazyk aplikacie: `system` (podla Windows) alebo kod jazyka (`sk`, `en`, `cs`, `de`, `fr`, `es`, `pl`, `it`,
    /// `pt`, `ja`, `zh-Hans`). Retazec, aby neznama hodnota nespravila konfiguraciu len na citanie.
    #[serde(default = "system_language")]
    pub language: String,
    /// Nezname kluce (buduce verzie) - zachovaju sa pri ulozeni.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Vsetky dlazdice skupiny Nastroje v predvolenom poradi.
pub const TOOL_TILES: [&str; 10] = [
    "weather",
    "files",
    "clipboard",
    "notes",
    "shots",
    "timer",
    "brightness",
    "snip",
    "picker",
    "focus",
];

fn default_tool_group() -> Vec<String> {
    TOOL_TILES.iter().map(|s| s.to_string()).collect()
}

impl Config {
    /// Stred lisaty patri ostrovu (polozka `shelf` je zobrazena): ine polozky tam nemozu byt.
    pub fn center_locked(&self) -> bool {
        self.layout
            .items
            .iter()
            .any(|s| s.id == "shelf" && s.visible)
    }

    /// Pravidlo stredu: so zapnutymi odlozenymi subormi je v strede len ich polozka (zobrazena); ostatne polozky zo
    /// stredu sa presunu na zaciatok pravej skupiny (poradie ostane), oddelovace zo stredu sa odstrania. Vypnute:
    /// polozka odlozenych suborov sa skryje a stred je volny pre ine polozky. Pouziva sa po nacitani, importe a
    /// kazdej zmene rozlozenia.
    /// Pocasie: prenesie stare mesto do zoznamu miest a opravi zdroj mimo zoznamu.
    pub fn normalize_weather(&mut self) {
        let city = std::mem::take(&mut self.weather_city);
        if !city.is_empty() && self.weather_places.is_empty() {
            self.weather_places.push(WeatherPlace {
                name: city,
                ..Default::default()
            });
            if self.weather_source == WeatherSource::None {
                self.weather_source = WeatherSource::Place(0);
            }
        }
        self.weather_places.retain(WeatherPlace::valid);
        self.weather_places.truncate(weather::MAX_PLACES);
        if let WeatherSource::Place(i) = self.weather_source {
            if i >= self.weather_places.len() {
                self.weather_source = if self.weather_places.is_empty() {
                    WeatherSource::None
                } else {
                    WeatherSource::Place(0)
                };
            }
        }
    }

    /// Co ma aplikacia stahovat (nic = pocasie vypnute).
    pub fn weather_target(&self) -> Option<WeatherTarget> {
        match self.weather_source {
            WeatherSource::None => None,
            WeatherSource::Device => Some(WeatherTarget::Device),
            WeatherSource::Place(i) => self
                .weather_places
                .get(i)
                .cloned()
                .map(WeatherTarget::Place),
        }
    }

    pub fn enforce_center(&mut self) {
        self.layout.enforce_center();
        for m in &mut self.monitors {
            if let Some(l) = &mut m.layout {
                l.enforce_center();
            }
        }
    }

    /// Rozlozenie lisaty na monitore `id`: vlastne, ak ho monitor ma, inak spolocne.
    pub fn layout_for(&self, id: &str) -> &Layout {
        self.monitor(id)
            .and_then(|m| m.layout.as_ref())
            .unwrap_or(&self.layout)
    }

    /// Vzhlad lisaty na monitore `id`: vlastny, ak ho monitor ma, inak spolocny.
    pub fn appearance_for(&self, id: &str) -> &Appearance {
        self.monitor(id)
            .and_then(|m| m.appearance.as_ref())
            .unwrap_or(&self.appearance)
    }

    /// Vlastne nastavenia monitora `id` (porovnanie ID bez ohladu na velkost pismen).
    pub fn monitor(&self, id: &str) -> Option<&MonitorOverride> {
        self.monitors.iter().find(|m| m.id.eq_ignore_ascii_case(id))
    }

    /// Rozlozenie, ktore sa upravuje pre monitor `id` (vlastne, ak ho monitor ma, inak spolocne).
    pub fn layout_for_mut(&mut self, id: &str) -> &mut Layout {
        match self
            .monitors
            .iter()
            .position(|m| m.id.eq_ignore_ascii_case(id) && m.layout.is_some())
        {
            Some(i) => self.monitors[i].layout.as_mut().expect("layout"),
            None => &mut self.layout,
        }
    }

    /// Je polozka `id` zobrazena aspon na jednom monitore (spolocne rozlozenie alebo vlastne niektoreho monitora)?
    pub fn visible_anywhere(&self, id: &str) -> bool {
        std::iter::once(&self.layout)
            .chain(self.monitors.iter().filter_map(|m| m.layout.as_ref()))
            .any(|l| l.items.iter().any(|s| s.id == id && s.visible))
    }

    pub fn monitor_mut(&mut self, id: &str) -> Option<&mut MonitorOverride> {
        self.monitors
            .iter_mut()
            .find(|m| m.id.eq_ignore_ascii_case(id))
    }
}

impl Layout {
    /// Pravidlo stredu pre toto rozlozenie (pozri `Config::enforce_center`).
    pub fn enforce_center(&mut self) {
        if !self.items.iter().any(|s| s.id == "shelf" && s.visible) {
            return;
        }
        let items = &mut self.items;
        items.retain(|s| !(s.group == Group::Center && separator_no(&s.id).is_some()));
        let moved: Vec<Slot> = items
            .iter()
            .filter(|s| s.group == Group::Center && s.id != "shelf")
            .cloned()
            .collect();
        if !moved.is_empty() {
            items.retain(|s| s.group != Group::Center || s.id == "shelf");
            let at = items
                .iter()
                .position(|s| s.group == Group::Right)
                .unwrap_or(items.len());
            for (k, mut m) in moved.into_iter().enumerate() {
                m.group = Group::Right;
                items.insert(at + k, m);
            }
        }
        if let Some(s) = items.iter_mut().find(|s| s.id == "shelf") {
            s.group = Group::Center;
            s.visible = true;
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CURRENT_VERSION,
            layout: Layout::default(),
            clock: Clock::default(),
            media: Media::default(),
            usage: Usage::default(),
            appearance: Appearance::default(),
            ui_sounds: UiSounds::default(),
            audio_panel: AudioPanel::default(),
            peripherals: Peripherals::default(),
            tools: Tools::default(),
            oled: Oled::default(),
            timer: TimerCfg::default(),
            sysmon: SysMonCfg::default(),
            privacy: PrivacyCfg::default(),
            notch: NotchCfg::default(),
            tool_group: default_tool_group(),
            audio_profiles: Vec::new(),
            ics_calendars: Vec::new(),
            weather_city: String::new(),
            weather_places: Vec::new(),
            weather_source: WeatherSource::None,
            displays: Displays::default(),
            monitors: Vec::new(),
            updates: Updates::default(),
            language: system_language(),
            extra: Map::new(),
        }
    }
}

/// Vysledok nacitania.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub config: Config,
    /// Subor sa nesmie prepisat (novsia verzia alebo poskodeny obsah).
    pub read_only: bool,
    /// Vysvetlenie do logu (technicke, neprekladane).
    pub note: Option<String>,
    /// Dovod, preco je subor len na citanie - pre preklad v UI.
    pub issue: Option<Issue>,
}

/// Dovod obmedzenia konfiguracie (UI ho prelozi).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    /// Poskodeny alebo neplatny obsah.
    Corrupt,
    /// Novsia verzia schemy, nez pozna tato aplikacia.
    Newer(u32),
    /// Subor sa neda precitat (prava, zamok).
    Unreadable,
}

impl Loaded {
    /// Konfiguracia, ktoru ma bezici proces po **opatovnom** nacitani (zmena v nastaveniach alebo inym procesom).
    /// Poskodeny alebo docasne necitatelny subor (napr. rozpisany inym procesom) NESMIE vratit listu na predvolene
    /// hodnoty (monitory, rozlozenie, zdroje AI): dovtedy platna konfiguracia ostava. Novsia verzia schemy sa
    /// nacita tolerantne (len na citanie), takze sa preberie.
    #[must_use]
    pub fn effective(self, previous: &Config) -> Config {
        match self.issue {
            Some(Issue::Corrupt | Issue::Unreadable) => previous.clone(),
            Some(Issue::Newer(_)) | None => self.config,
        }
    }
}

/// Cesta ku konfiguracii: `TRANSOM_CONFIG`, inak `%APPDATA%\transom\config.json`.
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TRANSOM_CONFIG") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("transom").join("config.json")
}

/// Rozparsuje text konfiguracie (bez suboru) - jadro `load`, testovatelne.
pub fn parse(text: &str) -> Loaded {
    // UTF-8 BOM (Poznamkovy blok / PowerShell 5.1 ho pridavaju) nie je poskodenie
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return Loaded {
                config: Config::default(),
                read_only: true,
                issue: Some(Issue::Corrupt),
                note: Some(format!(
                "konfiguracia je poskodena ({e}); pouzivam predvolene hodnoty a subor neprepisem"
            )),
            }
        }
    };
    let version = value.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
    let parsed: Result<Config, _> = serde_json::from_value(value.clone());
    let mut cfg = match parsed {
        Ok(c) => c,
        Err(e) => {
            return Loaded {
                config: Config::default(),
                read_only: true,
                issue: Some(Issue::Corrupt),
                note: Some(format!(
                    "konfiguraciu sa nepodarilo nacitat ({e}); subor neprepisem"
                )),
            }
        }
    };
    let mut note = None;
    let read_only = version > CURRENT_VERSION;
    if read_only {
        note = Some(format!(
            "konfiguracia ma novsiu verziu {version} (tato aplikacia pozna {CURRENT_VERSION}); nacitane len na citanie"
        ));
    } else if version < CURRENT_VERSION {
        // migracia: verzia 0 = subor bez cisla verzie (rucne napisany) - rovnaka schema, len doplnit
        if version < 3 && cfg.usage.refresh_minutes == 60 {
            // stara predvolba (v1/v2 bola 60 min a nedala sa rozlisit od vlastnej volby) -> nova predvolba
            cfg.usage.refresh_minutes = MIN_REFRESH_MINUTES;
        }
        cfg.version = CURRENT_VERSION;
        note = Some(format!(
            "konfiguracia migrovana z verzie {version} na {CURRENT_VERSION}"
        ));
    }
    cfg.layout.normalize();
    cfg.enforce_center();
    cfg.normalize_weather();
    // vzdy: nezbierat castejsie nez raz za 5 min (aj pri rucne upravenom subore)
    cfg.usage.refresh_minutes = cfg.usage.refresh_interval_minutes().unwrap_or(0);
    Loaded {
        config: cfg,
        read_only,
        note,
        issue: read_only.then_some(Issue::Newer(version)),
    }
}

pub fn load(path: &Path) -> Loaded {
    match std::fs::read_to_string(path) {
        Ok(t) => parse(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded {
            config: Config::default(),
            read_only: false,
            note: None,
            issue: None,
        },
        Err(e) => Loaded {
            config: Config::default(),
            read_only: true,
            issue: Some(Issue::Unreadable),
            note: Some(format!("konfiguraciu nemozno citat: {e}")),
        },
    }
}

/// Atomicky zapis: docasny subor v tom istom adresari a premenovanie (na Windows nahradi existujuci subor).
///
/// Docasny subor ma **unikatny nazov** (proces + pocitadlo): lista a okno nastaveni zapisuju do rovnakeho adresara a spolocny
/// `.tmp` by si navzajom prepisoval alebo premenovaval. Obsah sa pred premenovanim vyprazdni na disk (`sync_all`).
/// Samotny zapis **nezlucuje**: na zmenu jedneho pola pouzi [`update`], na ulozenie nastaveni [`save_merged`].
pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    write_atomic(path, text.as_bytes())
}

/// Atomicky zapis bajtov (unikatny docasny subor v tom istom priecinku, `sync_all`, premenovanie; pri chybe sa docasny subor
/// zmaze). Adresar sa **nevytvara** (export do neexistujuceho priecinka zlyha). Pouziva `save` aj export nastaveni.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = unique_tmp(path);
    let written = (|| {
        // `create_new`: na mieste docasneho suboru sa nikdy nenasleduje odkaz ani neprepise cudzi subor; zvysok po padu
        // procesu s rovnakym cislom sa zmaze (len samotna polozka, nie ciel odkazu) a skusi sa znova
        let open = || {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
        };
        let mut f = match open() {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&tmp)?;
                open()?
            }
            r => r?,
        };
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Najviac tolko zaloh `config.json` sa ponecha (pozri [`backup_file`]).
pub const BACKUPS_KEPT: usize = 5;

/// Je to nazov zalohy `config-YYYYMMDD-HHMMSS[-N].json`? (len take subory sa orezavaju; cudzie sa nikdy nemazu)
pub(crate) fn is_backup_name(n: &str) -> bool {
    let Some(rest) = n
        .strip_prefix("config-")
        .and_then(|r| r.strip_suffix(".json"))
    else {
        return false;
    };
    // cudzi subor s viacbajtovym znakom v nazve nesmie rozdelit znak (panika by zhodila zalohovanie)
    if !rest.is_char_boundary(rest.len().min(15)) {
        return false;
    }
    let (date, tail) = rest.split_at(rest.len().min(15));
    let b = date.as_bytes();
    let stamp_ok = b.len() == 15
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..].iter().all(u8::is_ascii_digit);
    let tail_ok = tail.is_empty()
        || tail
            .strip_prefix('-')
            .is_some_and(|t| !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()));
    stamp_ok && tail_ok
}

/// Zaloha surovych bajtov `config.json` do `backups\config-<stamp>.json` vedla neho (`stamp` = `YYYYMMDD-HHMMSS`); nikdy neprepise
/// existujucu zalohu (pripona `-1`, `-2`, ...). Ponecha najviac `keep` najnovsich zaloh; mazu sa len subory presne tvaru zaloh.
/// Chybajuci `config.json` = `Ok(None)` (nie je co zalohovat).
pub fn backup_file(path: &Path, stamp: &str, keep: usize) -> std::io::Result<Option<PathBuf>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let dir = path
        .parent()
        .map(|p| p.join("backups"))
        .unwrap_or_else(|| PathBuf::from("backups"));
    std::fs::create_dir_all(&dir)?;
    let mut target = dir.join(format!("config-{stamp}.json"));
    let mut n = 0;
    while target.exists() {
        n += 1;
        target = dir.join(format!("config-{stamp}-{n}.json"));
    }
    write_atomic(&target, &bytes)?;
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_backup_name(n))
        .collect();
    names.sort();
    while names.len() > keep {
        let old = names.remove(0);
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(Some(target))
}

/// Nazov docasneho suboru unikatny pre proces aj volanie.
fn unique_tmp(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("json.{}.{n}.tmp", std::process::id()))
}

/// Vykona `f` pod zamkom nad konfiguraciou (`config.json.lock`, medziprocesovy zamok suboru), aby citanie-uprava-zapis
/// dvoch procesov (lista, nastavenia) nezahubilo zmenu toho druheho. Ak zamok nie je do ~2 s k dispozicii (zasekly proces,
/// nepodporovany suborovy system), pokracuje sa bez neho: zapis ostava atomicky, len bez ochrany proti stratenej zmene.
fn with_lock<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("json.lock"))
        .ok();
    let locked = lock.as_ref().is_some_and(|l| {
        for _ in 0..200 {
            if l.try_lock().is_ok() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    });
    let r = f();
    if locked {
        if let Some(l) = &lock {
            let _ = l.unlock();
        }
    }
    r
}

fn read_only_error() -> std::io::Error {
    std::io::Error::other("konfiguracia je len na citanie (poskodena, necitatelna alebo novsej verzie): nezapisuje sa")
}

/// Zmeni cast konfiguracie: pod zamkom ju **znova nacita z disku**, aplikuje `f` a atomicky zapise. Poskodeny, necitatelny
/// alebo novsi subor sa nezapise (chyba). Chybajuci subor zacina od predvolenych hodnot. Vrati zapisanu konfiguraciu.
pub fn update(path: &Path, f: impl FnOnce(&mut Config)) -> std::io::Result<Config> {
    with_lock(path, || {
        let loaded = load(path);
        if loaded.read_only {
            return Err(read_only_error());
        }
        let mut cfg = loaded.config;
        f(&mut cfg);
        save(path, &cfg)?;
        Ok(cfg)
    })
}

/// Ulozi nastavenia **trojcestnym zlucenim**: `base` je stav, z ktoreho okno nastaveni vychadzalo (po poslednom nacitani
/// alebo ulozeni), `ours` stav po zmenach pouzivatela. Pod zamkom sa nacita aktualny subor (`theirs`) a prenesie sa don
/// len to, v com sa `ours` lisi od `base`; ostatne polia (zmeny inej instancie, lista, rucna uprava, nezname kluce)
/// ostanu. Poskodeny alebo novsi subor sa neprepise (chyba). Vrati zlucenu konfiguraciu.
pub fn save_merged(path: &Path, base: &Config, ours: &Config) -> std::io::Result<Config> {
    with_lock(path, || {
        let merged = if path.exists() {
            let loaded = load(path);
            if loaded.read_only {
                return Err(read_only_error());
            }
            let (b, o, t) = (
                serde_json::to_value(base).map_err(std::io::Error::other)?,
                serde_json::to_value(ours).map_err(std::io::Error::other)?,
                serde_json::to_value(&loaded.config).map_err(std::io::Error::other)?,
            );
            let m = merge3(&b, &o, t);
            parse(&serde_json::to_string(&m).map_err(std::io::Error::other)?).config
        } else {
            ours.clone()
        };
        save(path, &merged)?;
        Ok(merged)
    })
}

/// Trojcestne zlucenie JSON hodnot: z `ours` sa prevezme len to, co sa oproti `base` zmenilo (objekty po klucoch, polia a
/// skalary celé), zvysok ostava z `theirs`.
fn merge3(base: &Value, ours: &Value, theirs: Value) -> Value {
    if base == ours {
        return theirs; // nic sme nezmenili
    }
    match (base, ours, theirs) {
        (Value::Object(b), Value::Object(o), Value::Object(mut t)) => {
            for (k, ov) in o {
                match b.get(k) {
                    Some(bv) if bv == ov => {} // kluc sme nemenili: ostava ich hodnota
                    Some(bv) => {
                        let tv = t.remove(k).unwrap_or(Value::Null);
                        t.insert(k.clone(), merge3(bv, ov, tv));
                    }
                    None => {
                        t.insert(k.clone(), ov.clone()); // nas novy kluc
                    }
                }
            }
            for k in b.keys().filter(|k| !o.contains_key(*k)) {
                t.remove(k); // kluc sme odstranili
            }
            Value::Object(t)
        }
        (_, ours, _) => ours.clone(),
    }
}

/// Meno registrovanej spravy (`RegisterWindowMessage`), ktorou nastavenia oznamia liste zmenu konfiguracie.
pub const CHANGED_MESSAGE: &str = "transom-config-changed-v1";

/// Sprava liste: nastavenia zmazali lokalne data o vyuziti AI (lista zahodi aj data v pamati).
pub const USAGE_DELETED_MESSAGE: &str = "transom-usage-deleted-v1";

/// Sprava liste: znova zobrazit skrytu listu (posiela ju druhe spustenie `transom-app`).
pub const RESTORE_MESSAGE: &str = "transom-restore-v1";

/// Nastavenia ziadaju listu, aby hned skontrolovala aktualizacie.
pub const UPDATE_CHECK_MESSAGE: &str = "transom-update-check-v1";

/// Nastavenia ziadaju listu, aby nainstalovala dostupnu aktualizaciu (ako "Aktualizovat teraz" v jej paneli).
pub const UPDATE_INSTALL_MESSAGE: &str = "transom-update-install-v1";

/// Lista oznamuje nastaveniam, ze sa zmenila dostupna aktualizacia (`read_update_state`).
pub const UPDATE_STATE_MESSAGE: &str = "transom-update-state-v1";

/// Subor s verziou dostupnej aktualizacie, ktoru nasla lista (pre stranku Aktualizacie v nastaveniach):
/// `TRANSOM_DATA` alebo `%LOCALAPPDATA%\transom`, `update-available.txt`.
fn update_state_path() -> PathBuf {
    let dir = std::env::var_os("TRANSOM_DATA")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|b| PathBuf::from(b).join("transom")))
        .unwrap_or_else(|| std::env::temp_dir().join("transom"));
    dir.join("update-available.txt")
}

/// Zapise (alebo pri `None` zmaze) verziu dostupnej aktualizacie.
pub fn write_update_state(version: Option<&str>) {
    let p = update_state_path();
    match version {
        Some(v) => {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = write_atomic(&p, v.as_bytes());
        }
        None => {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Verzia dostupnej aktualizacie, ktoru naposledy nasla lista (len kratky platny retazec verzie).
pub fn read_update_state() -> Option<String> {
    let s = std::fs::read_to_string(update_state_path()).ok()?;
    let v = s.trim();
    (!v.is_empty()
        && v.len() <= 32
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    .then(|| v.to_string())
}

/// Sprava oknu nastaveni: prepni na stranku (wParam = poradie stranky); posiela ju druha instancia nastaveni (`--page`).
pub const SETTINGS_PAGE_MESSAGE: &str = "transom-settings-page-v1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_sounds_are_off_by_default_tolerant_and_clamped() {
        let d = Config::default().ui_sounds;
        assert!(!d.enabled && d.volume == DEFAULT_SOUND_VOLUME && d.effective_volume() == 0);
        // chyba sekcie, zly typ a mimo rozsah: nikdy chyba, nikdy hlasnejsie nez 100
        assert_eq!(parse("{}").config.ui_sounds, d);
        let c = parse(r#"{"version": 7, "ui_sounds": {"enabled": "ano", "volume": "hlasno"}}"#);
        assert!(
            !c.read_only && c.config.ui_sounds == d,
            "{:?}",
            c.config.ui_sounds
        );
        for (v, want) in [
            ("-5", 0),
            ("0", 0),
            ("42", 42),
            ("100", 100),
            ("250", 100),
            ("1e30", 100),
        ] {
            let l = parse(&format!(
                r#"{{"version": 7, "ui_sounds": {{"enabled": true, "volume": {v}}}}}"#
            ));
            assert_eq!(l.config.ui_sounds.volume, want, "{v}");
            assert_eq!(l.config.ui_sounds.effective_volume(), want, "{v}");
        }
        // stlmenie = nulova ucinna hlasitost bez ohladu na ulozenu
        let m = UiSounds {
            enabled: false,
            volume: 80,
        };
        assert_eq!(m.effective_volume(), 0);
    }

    #[test]
    fn the_update_icon_is_on_by_default_also_for_files_saved_before_it_existed() {
        let d = Config::default().updates;
        assert!(d.auto_check && d.show_icon);
        let c = parse(r#"{"version": 7, "updates": {"auto_check": false}}"#).config;
        assert!(!c.updates.auto_check && c.updates.show_icon);
        let c = parse(r#"{"version": 7, "updates": {"show_icon": false}}"#).config;
        assert!(c.updates.auto_check && !c.updates.show_icon);
    }

    #[test]
    fn ui_sounds_round_trip_and_survive_a_concurrent_change_in_save_merged() {
        let dir = std::env::temp_dir().join(format!("transom-uisounds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        save(&p, &Config::default()).unwrap();
        let base = load(&p).config;
        // iny proces zmeni hodiny, my zvuk: obe zmeny prezijuju
        update(&p, |c| c.clock.seconds = true).unwrap();
        let mut ours = base.clone();
        ours.ui_sounds = UiSounds {
            enabled: true,
            volume: 40,
        };
        let merged = save_merged(&p, &base, &ours).unwrap();
        let back = load(&p).config;
        assert_eq!(merged, back);
        assert!(back.clock.seconds && back.ui_sounds.enabled && back.ui_sounds.volume == 40);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_audio_item_is_known_hidden_by_default_and_old_files_get_it_hidden() {
        let l = Layout::default();
        let s = l
            .items
            .iter()
            .find(|s| s.id == "audio")
            .expect("polozka audio");
        assert!(!s.visible && s.group == Group::Right);
        // starsi subor bez polozky: doplni sa skryta, nic ine sa nezmeni
        let p = parse(
            r#"{"version": 7, "layout": {"items": [{"id": "clock", "group": "right", "visible": true}]}}"#,
        );
        let a = p
            .config
            .layout
            .items
            .iter()
            .find(|s| s.id == "audio")
            .unwrap();
        assert!(!a.visible);
        assert!(
            p.config
                .layout
                .items
                .iter()
                .find(|s| s.id == "clock")
                .unwrap()
                .visible
        );
    }

    #[test]
    fn quick_actions_default_to_all_and_parse_leniently() {
        let all: Vec<String> = QUICK_ACTIONS.iter().map(|s| s.to_string()).collect();
        assert_eq!(Config::default().audio_panel.quick_actions, all);
        assert_eq!(parse("{}").config.audio_panel.quick_actions, all);
        // zly typ = predvolene; prazdne pole = skryty riadok
        assert_eq!(
            parse(r#"{"version":7,"audio_panel":{"quick_actions":"vsetko"}}"#)
                .config
                .audio_panel
                .quick_actions,
            all
        );
        assert!(parse(r#"{"version":7,"audio_panel":{"quick_actions":[]}}"#)
            .config
            .audio_panel
            .quick_actions
            .is_empty());
        // neznama, opakovana a nestringova polozka sa zahodi, poradie je kanonicke
        let v = parse(
            r#"{"version":7,"audio_panel":{"quick_actions":["settings","format_c","search","search",7,null,"quick_settings"]}}"#,
        )
        .config
        .audio_panel
        .quick_actions;
        assert_eq!(v, vec!["quick_settings", "search", "settings"]);
        // zapnutie / vypnutie z nastaveni
        let mut p = AudioPanel::default();
        p.set("tray", false);
        p.set("notifications", false);
        assert!(!p.has("tray") && !p.has("notifications") && p.has("search"));
        p.set("tray", true);
        assert_eq!(
            p.quick_actions,
            vec!["quick_settings", "search", "tray", "settings"]
        );
        p.set("nieco_ine", true);
        assert_eq!(p.quick_actions.len(), 4, "neznama akcia sa nepridava");
    }

    #[test]
    fn peripherals_default_on_and_parse_leniently() {
        let d = Peripherals::default();
        assert_eq!(d.sources, PERIPHERAL_SOURCES.to_vec());
        assert_eq!((d.on_bar, d.interval_secs), (3, 60));
        assert_eq!(parse("{}").config.peripherals, d);
        // polozka je v predvolenom rozlozeni viditelna (ukaze sa len so zariadenim) a stara konfiguracia ju doplni
        assert!(Layout::default()
            .items
            .iter()
            .any(|s| s.id == "peripherals" && s.visible));
        let old = parse(r#"{"version":7,"layout":{"items":[{"id":"clock","group":"right"}]}}"#);
        assert!(old
            .config
            .layout
            .items
            .iter()
            .any(|s| s.id == "peripherals"));
        // zle typy a hodnoty mimo volieb: nikdy "len na citanie"
        let l = parse(
            r#"{"version":7,"peripherals":{"sources":["razer","x","razer",5,"bluetooth"],"on_bar":99,"interval_secs":"rychlo"}}"#,
        );
        assert!(!l.read_only);
        assert_eq!(l.config.peripherals.sources, vec!["bluetooth", "razer"]);
        assert_eq!(l.config.peripherals.on_bar, 0, "viac nez 3 = vsetky");
        assert_eq!(l.config.peripherals.interval_secs, 60);
        let l = parse(
            r#"{"version":7,"peripherals":{"sources":"vsetko","on_bar":2,"interval_secs":100}}"#,
        );
        assert_eq!(l.config.peripherals.sources, d.sources);
        assert_eq!(l.config.peripherals.on_bar, 2);
        assert_eq!(l.config.peripherals.interval_secs, 120, "najblizsia volba");
        // prazdne pole = ziadny zdroj (polozka nic necita)
        assert!(parse(r#"{"version":7,"peripherals":{"sources":[]}}"#)
            .config
            .peripherals
            .sources
            .is_empty());
        let mut p = Peripherals::default();
        p.set("logitech", false);
        p.set("x", true);
        assert_eq!(p.sources, vec!["bluetooth", "razer", "nuphy"]);
        p.set("logitech", true);
        assert_eq!(p.sources, PERIPHERAL_SOURCES.to_vec());
    }

    #[test]
    fn the_old_tray_item_becomes_tools_in_place_and_keeps_its_position_group_and_visibility() {
        let p = parse(
            r#"{"version": 7, "layout": {"items": [
                {"id": "tray", "group": "left", "visible": false},
                {"id": "clock", "group": "right", "visible": true}]}}"#,
        );
        let ids: Vec<&str> = p
            .config
            .layout
            .items
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids[0], "tools", "premenovane na mieste: {ids:?}");
        assert!(!ids.contains(&"tray"));
        let t = &p.config.layout.items[0];
        assert!(!t.visible && t.group == Group::Left);
        // oba naraz: ostane len `tools` a jeho nastavenie
        let p = parse(
            r#"{"version": 7, "layout": {"items": [
                {"id": "tools", "group": "right", "visible": true},
                {"id": "tray", "group": "left", "visible": false}]}}"#,
        );
        let tools: Vec<_> = p
            .config
            .layout
            .items
            .iter()
            .filter(|s| s.id == "tools")
            .collect();
        assert_eq!(tools.len(), 1);
        assert!(tools[0].visible && tools[0].group == Group::Right);
        assert!(!p.config.layout.items.iter().any(|s| s.id == "tray"));
        // novy subor (len `tools`) a predvolene rozlozenie: ziadne `tray` v zozname znamych poloziek
        assert!(KNOWN_ITEMS.contains(&"tools") && !KNOWN_ITEMS.contains(&"tray"));
        assert!(Layout::default()
            .items
            .iter()
            .any(|s| s.id == "tools" && s.visible));
    }

    #[test]
    fn text_colour_and_oled_protection_default_off_and_parse_leniently() {
        let d = Config::default();
        assert_eq!(d.appearance.text(), TextChoice::Auto);
        assert!(!d.oled.enabled && !d.oled.pixel_shift() && !d.oled.auto_hide());
        assert_eq!(d.appearance.effective(&d.oled), (None, 255));
        let l = parse(
            r##"{"appearance": {"text_mode": "CUSTOM", "text_color": "#ffcc00", "bg_transparent": true},
                 "oled": {"enabled": true, "pixel_shift": "ano", "auto_hide": true}}"##,
        );
        assert!(!l.read_only, "{:?}", l.issue);
        let c = l.config;
        assert_eq!(c.appearance.text(), TextChoice::Custom((0xFF, 0xCC, 0x00)));
        assert!(
            c.oled.pixel_shift(),
            "zly typ = predvolena hodnota podvolby"
        );
        assert!(c.oled.auto_hide() && c.oled.black_bg());
        assert_eq!(
            c.appearance.effective(&c.oled),
            (Some((0, 0, 0)), 255),
            "cierne pozadie prebije priehladnost"
        );
        let l = parse(r##"{"appearance": {"text_mode": "neon", "text_color": "#ffcc00"}}"##);
        assert_eq!(l.config.appearance.text_mode, "auto");
        let l = parse(r##"{"appearance": {"text_mode": "custom", "text_color": "zle"}}"##);
        assert_eq!(
            l.config.appearance.text(),
            TextChoice::Auto,
            "neplatna farba = auto"
        );
        // posun: 8 poloh v kruhu, nikdy viac nez 2 px vodorovne a 1 px zvisle
        let all: Vec<_> = (0..16).map(pixel_shift_lp).collect();
        assert!(all.iter().all(|(x, y)| x.abs() <= 2 && (0..=1).contains(y)));
        assert_eq!(all[0], all[8]);
        assert_eq!(
            all[..8]
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            8
        );
    }

    #[test]
    fn launcher_hotkey_defaults_to_alt_space_parses_leniently_and_can_be_turned_off() {
        let d = Config::default().tools.launcher;
        assert_eq!(d.hotkey, "Alt+Space");
        assert!(!d.hotkey_in_fullscreen, "na celej obrazovke predvolene nie");
        let alt_space = Hotkey {
            ctrl: false,
            alt: true,
            shift: false,
            win: false,
            vk: 0x20,
        };
        assert_eq!(d.hotkey(), Some(alt_space));
        let hk = |v: &str| {
            let json = format!(r#"{{"tools": {{"launcher": {{"hotkey": {v}}}}}}}"#);
            parse(&json).config.tools.launcher.hotkey
        };
        assert_eq!(
            hk(r#"" shift + ctrl+k ""#),
            "Ctrl+Shift+K",
            "kanonicky zapis"
        );
        assert_eq!(hk(r#""win+alt+f13""#), "Alt+Win+F13");
        assert_eq!(hk(r#""""#), "", "prazdne = vypnute");
        assert_eq!(hk(r#""OFF""#), "");
        for bad in [
            r#""Shift+K""#,
            r#""K""#,
            r#""Ctrl+Alt""#,
            r#""Ctrl+Esc""#,
            r#""Ctrl+K+L""#,
            r#""Ctrl+F25""#,
            "5",
            "null",
        ] {
            assert_eq!(hk(bad), "Alt+Space", "zly zapis {bad} = predvolena skratka");
        }
        assert!(
            Hotkey::new(false, false, true, false, 0x41).is_none(),
            "samotny Shift nestaci"
        );
        assert!(
            Hotkey::new(true, false, false, false, 0x1B).is_none(),
            "Esc nie je povolena"
        );
        assert!(
            Hotkey::new(false, true, false, false, 0x73).is_none(),
            "Alt+F4 nie"
        );
        for vk in [0x41, 0x43, 0x53, 0x56, 0x58, 0x59, 0x5A] {
            assert!(
                Hotkey::new(true, false, false, false, vk).is_none(),
                "Ctrl+{vk:#x} nie"
            );
        }
        assert!(
            Hotkey::new(true, false, true, false, 0x56).is_some(),
            "Ctrl+Shift+V ano"
        );
        for vk in (0x30..=0x39)
            .chain(0x41..=0x5A)
            .chain(0x70..=0x87)
            .chain([0x20])
        {
            let h = Hotkey::new(true, true, false, false, vk).unwrap();
            assert_eq!(Hotkey::parse(&h.label()), Some(h), "zapis sa vrati presne");
            let h = Hotkey::new(false, false, true, true, vk).unwrap();
            assert_eq!(Hotkey::parse(&h.label()), Some(h), "zapis sa vrati presne");
        }
    }

    #[test]
    fn tools_defaults_collect_nothing_automatically_and_parse_leniently_with_clamping() {
        let d = Config::default().tools;
        // jedine automaticke zbieranie (schranka) je vypnute; posledne hladania su vypnute
        assert!(!d.clipboard.enabled && !d.launcher.recent_searches);
        assert!(
            d.shelf.enabled && !d.shelf.copy_by_default,
            "Polica: odkazy, kopia len na volbu"
        );
        assert_eq!(parse("{}").config.tools, d);
        // zly typ, zaporne, obrovske a nestringove hodnoty: predvolene alebo orezane, subor ostava zapisatelny
        let l = parse(
            r#"{"version": 7, "tools": {"clipboard": {"enabled": "ano", "max_items": -5, "max_text_kb": 1e30,
               "retention_days": 0, "excluded_apps": ["KeePass.exe", "keepass.EXE", 5, "", "a\u0007b", "x.exe"]},
               "shelf": "nic", "launcher": {"max_results": 9999, "excluded_folders": ["C:\\Secret", null]}}}"#,
        );
        assert!(!l.read_only, "{:?}", l.issue);
        let c = &l.config.tools;
        assert!(!c.clipboard.enabled, "zly typ nikdy nezapne zbieranie");
        assert_eq!(c.clipboard.max_items, 1);
        assert_eq!(c.clipboard.max_text_kb, 4096);
        assert_eq!(
            c.clipboard.retention_days, 0,
            "0 = ponechat, kym ich pouzivatel nezmaze"
        );
        assert_eq!(c.clipboard.excluded_apps, vec!["KeePass.exe", "x.exe"]);
        assert_eq!(c.shelf, d.shelf);
        assert_eq!(c.launcher.max_results, 20);
        assert_eq!(c.launcher.excluded_folders, vec!["C:\\Secret"]);
        // snimky: kniznica je predvolene zapnuta s priecinkom Windows, bez vlastnych priecinkov; stare kluce (`max_mb`,
        // `retention_days` z predchadzajuceho ulozenia) sa ticho ignoruju a nic z nich nevznikne
        assert!(
            d.screenshots.enabled
                && d.screenshots.default_folder
                && d.screenshots.folders.is_empty()
        );
        assert_eq!(d.screenshots.max_items, 300);
        let old = parse(
            r#"{"tools":{"screenshots":{"max_items":100,"max_mb":500,"retention_days":3,"folders":["D:\\Shots","d:\\shots",7,"","a\u0007b"]}}}"#,
        )
        .config;
        assert_eq!(old.tools.screenshots.max_items, 100);
        assert_eq!(old.tools.screenshots.folders, vec!["D:\\Shots"]);
        let many: Vec<String> = (0..30).map(|i| format!("D:\\S{i}")).collect();
        let lots = parse(&format!(
            r#"{{"tools":{{"screenshots":{{"folders":{},"max_items":99999,"default_folder":"nie"}}}}}}"#,
            serde_json::to_string(&many).unwrap()
        ))
        .config;
        assert_eq!(lots.tools.screenshots.folders.len(), TOOLS_MAX_SHOT_FOLDERS);
        assert_eq!(lots.tools.screenshots.max_items, TOOLS_MAX_ITEMS);
        assert!(
            lots.tools.screenshots.default_folder,
            "zly typ ostane na predvolenej hodnote"
        );
        // round trip a presiahnutie save_merged: cesty a prepinace ostanu, nic ine sa nezmeni
        let mut x = Config::default();
        x.tools.clipboard.enabled = true;
        x.tools.clipboard.excluded_apps = vec!["a.exe".into()];
        let back = parse(&serde_json::to_string(&x).unwrap()).config;
        assert_eq!(back.tools, x.tools);
    }

    #[test]
    fn exe_names_for_the_exclusion_list_are_file_names_only_and_hostile_input_is_rejected() {
        // cesta z vyberu suboru sa zredukuje na nazov
        assert_eq!(
            normalize_exe_name(r"C:\Program Files\KeePass\KeePass.exe").as_deref(),
            Some("KeePass.exe")
        );
        assert_eq!(
            normalize_exe_name("  c:/x/Bitwarden.EXE ").as_deref(),
            Some("Bitwarden.EXE")
        );
        assert_eq!(
            normalize_exe_name("1Password.exe").as_deref(),
            Some("1Password.exe")
        );
        // nie je to .exe, prazdne, iba pripona, iba bodky/medzery, riadiace a zakazane znaky, cesta konciaca lomitkom
        for bad in [
            "",
            "   ",
            ".exe",
            "..exe",
            " .exe",
            "notepad",
            "notepad.exe.txt",
            "notepad.com",
            r"C:\dir\",
            "a\u{7}b.exe",
            "a|b.exe",
            "a*.exe",
            "a?.exe",
            "con:.exe",
            "\"x\".exe",
            "<x>.exe",
        ] {
            assert_eq!(normalize_exe_name(bad), None, "{bad:?}");
        }
        // dlzka: 64 znakov este ano, 65 uz nie; viacbajtovy znak na konci nikdy nespanikuje
        let ok = format!("{}.exe", "a".repeat(60));
        assert_eq!(normalize_exe_name(&ok).as_deref(), Some(ok.as_str()));
        assert_eq!(normalize_exe_name(&format!("{}.exe", "a".repeat(61))), None);
        assert_eq!(normalize_exe_name("aaaaaaé"), None);
        assert_eq!(normalize_exe_name("é.exe").as_deref(), Some("é.exe"));
        // nacitanie konfiguracie nic nezahodi len preto, ze nema .exe (tichy zahod by oslabil sukromie)
        let c = parse(r#"{"tools":{"clipboard":{"excluded_apps":["legacy-name","KeePass.exe"]}}}"#)
            .config;
        assert_eq!(
            c.tools.clipboard.excluded_apps,
            vec!["legacy-name", "KeePass.exe"]
        );
    }

    #[test]
    fn default_layout_has_all_known_items_once() {
        let l = Layout::default();
        for id in KNOWN_ITEMS {
            assert_eq!(l.items.iter().filter(|s| s.id == id).count(), 1, "{id}");
        }
        assert!(
            !l.items.iter().find(|s| s.id == "ai").unwrap().visible,
            "AI predvolene skryte"
        );
    }

    #[test]
    fn roundtrip_preserves_unknown_items_and_keys() {
        let text = r#"{
            "version": 1,
            "layout": {"items": [
                {"id": "clock", "group": "left", "visible": true},
                {"id": "weather", "group": "right", "visible": true}
            ]},
            "future_section": {"x": 1}
        }"#;
        let l = parse(text);
        assert!(!l.read_only);
        let c = &l.config;
        assert_eq!(c.layout.items[0].id, "clock");
        assert_eq!(c.layout.items[0].group, Group::Left);
        assert!(
            c.layout.items.iter().any(|s| s.id == "weather"),
            "neznama polozka zachovana"
        );
        // chybajuce zname polozky doplnene
        for id in KNOWN_ITEMS {
            assert!(c.layout.items.iter().any(|s| s.id == id), "{id}");
        }
        let out = serde_json::to_string(c).unwrap();
        assert!(out.contains("future_section") && out.contains("weather"));
        assert_eq!(parse(&out).config, *c, "stabilny roundtrip");
    }

    #[test]
    fn migration_from_unversioned_file() {
        let l = parse(r#"{"clock": {"seconds": true}}"#);
        assert!(!l.read_only);
        assert_eq!(l.config.version, CURRENT_VERSION);
        assert!(l.config.clock.seconds);
        assert_eq!(l.config.layout, Layout::default());
        assert!(l.note.unwrap().contains("migrovana"));
    }

    #[test]
    fn newer_or_corrupt_file_is_read_only() {
        let l = parse(r#"{"version": 99, "clock": {"seconds": true}}"#);
        assert!(l.read_only && l.config.clock.seconds);
        let l = parse("{ toto nie je json");
        assert!(l.read_only);
        assert_eq!(l.config, Config::default());
        let l = parse(r#"{"version": 2, "clock": {"hour": "25"}}"#);
        assert!(l.read_only, "neplatna hodnota -> neprepisovat");
    }

    #[test]
    fn duplicate_items_are_dropped() {
        let l = parse(
            r#"{"version":1,"layout":{"items":[{"id":"clock","group":"left"},{"id":"clock","group":"right"}]}}"#,
        );
        let clocks: Vec<_> = l
            .config
            .layout
            .items
            .iter()
            .filter(|s| s.id == "clock")
            .collect();
        assert_eq!(clocks.len(), 1);
        assert_eq!(clocks[0].group, Group::Left, "prva vyhrava");
    }

    #[test]
    fn hex_and_rgb_parsing_is_strict() {
        assert_eq!(parse_hex("#1E2228"), Some((0x1E, 0x22, 0x28)));
        assert_eq!(parse_hex("  1e2228 "), Some((0x1E, 0x22, 0x28)));
        assert_eq!(parse_hex("#fff"), Some((255, 255, 255)));
        assert_eq!(parse_hex("#0a5"), Some((0, 0xAA, 0x55)));
        for bad in [
            "", "#", "#12", "#1234", "#12345", "#1234567", "#GGGGGG", "#12 456", "##123456",
            "0x123456",
        ] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
        assert_eq!(parse_rgb("0", "128", "255"), Some((0, 128, 255)));
        assert_eq!(parse_rgb(" 7 ", "007", "10"), Some((7, 7, 10)));
        for bad in [
            ("256", "0", "0"),
            ("-1", "0", "0"),
            ("", "0", "0"),
            ("1.5", "0", "0"),
            ("1000", "0", "0"),
            ("a", "0", "0"),
            ("+5", "0", "0"),
        ] {
            assert_eq!(parse_rgb(bad.0, bad.1, bad.2), None, "{bad:?}");
        }
        assert_eq!(format_hex((0, 171, 255)), "#00ABFF");
        assert_eq!(parse_hex(&format_hex((1, 2, 3))), Some((1, 2, 3)));
    }

    #[test]
    fn contrast_and_blend_follow_wcag() {
        assert!((contrast_ratio((0, 0, 0), (255, 255, 255)) - 21.0).abs() < 0.01);
        assert!((contrast_ratio((255, 255, 255), (255, 255, 255)) - 1.0).abs() < 1e-4);
        // symetricke
        let (a, b) = ((0xE6, 0xE9, 0xEE), (0x1E, 0x22, 0x28));
        assert_eq!(contrast_ratio(a, b), contrast_ratio(b, a));
        assert!(
            contrast_ratio(a, b) > 12.0,
            "text temy na jeho pozadi ma vysoky kontrast"
        );
        // biely text na bielom pozadi je necitatelny; svetly text na svetlom tiez
        assert!(contrast_ratio((0xE6, 0xE9, 0xEE), (0xF3, 0xF5, 0xF8)) < 1.3);
        assert_eq!(blend((10, 20, 30), (200, 200, 200), 255), (10, 20, 30));
        assert_eq!(blend((10, 20, 30), (200, 200, 200), 0), (200, 200, 200));
        assert_eq!(blend((0, 0, 0), (200, 100, 50), 128), (100, 50, 25));
    }

    #[test]
    fn opacity_limits_and_transparent_mode() {
        let mut a = Appearance::default();
        assert!(a.is_opaque() && a.alpha() == 255 && a.color().is_none());
        a.bg_opacity = 50;
        assert_eq!((a.alpha(), a.is_opaque()), (128, false));
        a.bg_opacity = 0;
        assert_eq!(
            a.opacity_percent(),
            MIN_OPACITY_PERCENT,
            "bez 'priehladne' sa neklesne pod minimum"
        );
        assert_eq!(a.alpha(), 26);
        a.bg_opacity = 5000;
        assert_eq!((a.opacity_percent(), a.alpha()), (100, 255));
        a.bg_transparent = true;
        assert_eq!(
            a.alpha(),
            0,
            "priehladne = alfa 0, ulozena nepriehladnost sa nestraca"
        );
        assert_eq!(a.bg_opacity, 5000);
        a.bg_color = "#102030".into();
        assert_eq!(a.color(), Some((0x10, 0x20, 0x30)));
        a.bg_color = "nezmysel".into();
        assert_eq!(a.color(), None, "neplatna farba = farba temy");
    }

    #[test]
    fn lenient_new_fields_never_make_config_read_only() {
        let l = parse(
            r#"{"version": 3,
            "appearance": {"bg_color": 42, "bg_opacity": 300, "bg_transparent": "ano"},
            "media": {"marquee": "x", "marquee_direction": "sideways", "marquee_speed": 7, "marquee_paused": null},
            "usage": {"refresh_minutes": "soon"}}"#,
        );
        assert!(!l.read_only, "{:?}", l.note);
        let c = l.config;
        assert_eq!(c.appearance.opacity_percent(), 100);
        assert!(!c.appearance.bg_transparent);
        assert!(c.media.marquee && !c.media.marquee_paused);
        assert_eq!(c.media.direction(), MarqueeDirection::Ltr);
        assert_eq!(c.media.speed(), MarqueeSpeed::Normal);
        assert_eq!(c.usage.refresh_minutes, MIN_REFRESH_MINUTES);
        let l = parse(
            r#"{"version": 3, "appearance": {"bg_opacity": -40}, "usage": {"refresh_minutes": 9e12}}"#,
        );
        assert!(!l.read_only);
        assert_eq!(l.config.appearance.opacity_percent(), MIN_OPACITY_PERCENT);
        assert_eq!(l.config.usage.refresh_minutes, MAX_REFRESH_MINUTES);
    }

    #[test]
    fn refresh_interval_bounds() {
        let mut u = Usage::default();
        assert_eq!(u.refresh_minutes, 5, "predvolene 5 min");
        assert_eq!(u.refresh_interval_minutes(), Some(5));
        for (stored, effective) in [
            (0, None),
            (1, Some(5)),
            (4, Some(5)),
            (5, Some(5)),
            (6, Some(6)),
            (240, Some(240)),
            (u32::MAX, Some(MAX_REFRESH_MINUTES)),
        ] {
            u.refresh_minutes = stored;
            assert_eq!(u.refresh_interval_minutes(), effective, "{stored}");
        }
        // cez subor: 1..4 sa zvysi na 5, 0 ostane "na poziadanie"
        for (json, want) in [("1", 5), ("4", 5), ("0", 0), ("30", 30)] {
            let l = parse(&format!(
                r#"{{"version": 3, "usage": {{"refresh_minutes": {json}}}}}"#
            ));
            assert_eq!(l.config.usage.refresh_minutes, want, "{json}");
        }
    }

    #[test]
    fn glass_defaults_to_panels_on_and_bar_off_and_bad_values_fall_back_to_defaults() {
        let c = parse(r#"{"version": 7}"#).config;
        assert!(c.appearance.glass_panels && !c.appearance.glass_bar);
        let c =
            parse(r#"{"version": 7, "appearance": {"glass_panels": false, "glass_bar": true}}"#)
                .config;
        assert!(!c.appearance.glass_panels && c.appearance.glass_bar);
        // neplatna hodnota = predvolena, nikdy "len na citanie"
        let l = parse(r#"{"version": 7, "appearance": {"glass_panels": "ano", "glass_bar": 7}}"#);
        assert!(!l.read_only);
        assert!(l.config.appearance.glass_panels && !l.config.appearance.glass_bar);
    }

    #[test]
    fn migration_to_v4_adds_the_tray_icon_default_and_keeps_everything_else() {
        let l = parse(
            r##"{"version": 3, "language": "de", "appearance": {"bg_color": "#102030", "bg_opacity": 60},
                "usage": {"refresh_minutes": 15}}"##,
        );
        assert!(!l.read_only && l.note.as_deref().is_some_and(|n| n.contains("migrovana")));
        let c = l.config;
        assert_eq!(c.version, CURRENT_VERSION);
        assert!(c.appearance.tray_icon, "predvolene zapnuta");
        assert_eq!(
            (c.appearance.bg_color.as_str(), c.appearance.bg_opacity),
            ("#102030", 60)
        );
        assert_eq!((c.language.as_str(), c.usage.refresh_minutes), ("de", 15));
        // vedome vypnuta ikona sa zachova; neplatna hodnota = predvolene (zapnuta), nie "len na citanie"
        assert!(
            !parse(r#"{"version": 4, "appearance": {"tray_icon": false}}"#)
                .config
                .appearance
                .tray_icon
        );
        let l = parse(r#"{"version": 4, "appearance": {"tray_icon": "nie"}}"#);
        assert!(!l.read_only && l.config.appearance.tray_icon);
    }

    #[test]
    fn migration_to_v3_sets_new_defaults_and_old_default_interval() {
        // v2 s povodnou predvolbou 60 -> nova predvolba 5; vlastne hodnoty (30, 240) ostavaju
        for (old, want) in [(60, 5), (30, 30), (240, 240), (0, 0)] {
            let l = parse(&format!(
                r#"{{"version": 2, "usage": {{"refresh_minutes": {old}}}, "language": "sk"}}"#
            ));
            assert!(!l.read_only && l.note.unwrap().contains("migrovana"));
            let c = l.config;
            assert_eq!(
                (c.version, c.usage.refresh_minutes),
                (CURRENT_VERSION, want),
                "{old}"
            );
            assert_eq!(c.language, "sk");
            assert!(c.media.marquee && c.media.direction() == MarqueeDirection::Ltr);
            assert!(c.appearance.is_opaque() && c.appearance.color().is_none());
        }
        // v3 s 60 = vedomy vyber, nemeni sa
        let l = parse(r#"{"version": 3, "usage": {"refresh_minutes": 60}}"#);
        assert_eq!(l.config.usage.refresh_minutes, 60);
        // novsia verzia ostava len na citanie
        assert!(parse(&format!(r#"{{"version": {}}}"#, CURRENT_VERSION + 1)).read_only);
        // ulozenie a nacitanie zachova nove polia
        let mut c = Config::default();
        c.appearance.bg_color = "#102030".into();
        c.appearance.bg_opacity = 40;
        c.media.marquee_direction = "rtl".into();
        let back = parse(&serde_json::to_string(&c).unwrap()).config;
        assert_eq!(back, c);
    }

    #[test]
    fn migration_from_v1_keeps_settings_and_adds_defaults() {
        let v1 = r#"{"version": 1,
            "layout": {"items": [{"id": "clock", "group": "center", "visible": true},
                                 {"id": "tools", "group": "right", "visible": false}]},
            "clock": {"seconds": true},
            "usage": {"codex_local": true, "refresh_minutes": 30}}"#;
        let l = parse(v1);
        assert!(!l.read_only);
        assert_eq!(l.config.version, CURRENT_VERSION);
        assert!(l.note.unwrap().contains("migrovana"));
        let c = &l.config;
        assert_eq!(c.language, "system");
        assert_eq!(
            (c.usage.bar_provider.as_str(), c.usage.ring_mode.as_str()),
            ("auto", "used")
        );
        assert!(c.usage.codex_local && c.usage.refresh_minutes == 30 && c.clock.seconds);
        assert!(
            !c.layout
                .items
                .iter()
                .find(|s| s.id == "tools")
                .unwrap()
                .visible
        );
    }

    #[test]
    fn unknown_new_values_do_not_make_config_read_only() {
        let l = parse(
            r#"{"version": 2, "language": "klingon", "usage": {"ring_mode": "?", "bar_provider": 5}}"#,
        );
        // bar_provider ako cislo = typova chyba -> len na citanie (chrani subor); retazce su tolerantne
        assert!(l.read_only);
        let l = parse(
            r#"{"version": 2, "language": "klingon", "usage": {"ring_mode": "?", "bar_provider": "x"}}"#,
        );
        assert!(!l.read_only);
        assert_eq!(
            l.config.language, "klingon",
            "riesi sa az pri zobrazeni (zaloha na anglictinu)"
        );
    }

    #[test]
    fn separators_are_added_up_to_the_limit_survive_a_reload_and_only_they_can_be_removed() {
        let mut l = Layout::default();
        assert_eq!(l.add_separator(Group::Left).as_deref(), Some("sep-1"));
        assert_eq!(l.add_separator(Group::Right).as_deref(), Some("sep-2"));
        // na konci skupiny a viditelny
        let left: Vec<_> = l.in_group(Group::Left).map(|s| s.id.as_str()).collect();
        assert_eq!(left.last(), Some(&"sep-1"));
        for _ in 2..MAX_SEPARATORS {
            assert!(l.add_separator(Group::Right).is_some());
        }
        assert_eq!(
            l.add_separator(Group::Left),
            None,
            "najviac {MAX_SEPARATORS}"
        );
        // uvolnene cislo sa pouzije znova
        assert!(l.remove_separator("sep-2"));
        assert_eq!(l.add_separator(Group::Left).as_deref(), Some("sep-2"));
        // bezne polozky sa takto neodstranuju
        assert!(!l.remove_separator("clock") && !l.remove_separator("sep-99"));
        assert!(l.items.iter().any(|s| s.id == "clock"));
        // zapis a nacitanie: oddelovace ostanu, normalize ich neodstrani ani nezdvoji
        let c = Config {
            layout: l.clone(),
            ..Default::default()
        };
        let back = parse(&serde_json::to_string(&c).unwrap()).config;
        assert_eq!(back.layout, l);
        assert!(is_layout_id("sep-8") && !is_layout_id("sep-0") && !is_layout_id("sep-9"));
    }

    #[test]
    fn with_stashed_files_on_the_centre_holds_only_them_and_off_it_is_free() {
        let mut c = Config::default();
        assert!(c.center_locked());
        c.layout.move_to("clock", Group::Center, 0);
        c.layout.move_to("battery", Group::Center, 1);
        c.layout.add_separator(Group::Center);
        c.enforce_center();
        let center: Vec<_> = c
            .layout
            .in_group(Group::Center)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(center, vec!["shelf"]);
        // presunute na zaciatok pravej skupiny v povodnom poradi, nic sa nestratilo (okrem oddelovaca zo stredu)
        let right: Vec<_> = c
            .layout
            .in_group(Group::Right)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(&right[..2], &["clock", "battery"]);
        assert!(!c.layout.items.iter().any(|s| s.id.starts_with("sep-")));
        assert_eq!(c.layout.items.len(), Layout::default().items.len());
        // ostrov skryty: stred je volny (aj s vypnutymi odlozenymi subormi ostrov ostava, ak je zobrazeny)
        let shelf = |c: &mut Config, v: bool| {
            if let Some(s) = c.layout.items.iter_mut().find(|s| s.id == "shelf") {
                s.visible = v;
            }
        };
        shelf(&mut c, false);
        c.enforce_center();
        assert!(!c.center_locked());
        c.layout.move_to("clock", Group::Center, 0);
        c.enforce_center();
        assert!(c.layout.in_group(Group::Center).any(|s| s.id == "clock"));
        c.tools.shelf.enabled = false;
        // znova zobrazeny: stred sa uprace
        shelf(&mut c, true);
        c.enforce_center();
        assert!(c.center_locked(), "ostrov nezavisi od odlozenych suborov");
        let center: Vec<_> = c
            .layout
            .in_group(Group::Center)
            .filter(|s| s.visible)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(center, vec!["shelf"]);
        // nacitanie suboru uplatni pravidlo tiez
        let l = parse(
            r#"{"version": 7, "layout": {"items": [{"id": "clock", "group": "center", "visible": true}]}}"#,
        );
        assert!(!l
            .config
            .layout
            .in_group(Group::Center)
            .any(|s| s.id == "clock"));
    }

    #[test]
    fn move_before_places_an_item_in_front_of_another_or_at_the_end_of_a_group() {
        let mut l = Layout::default();
        l.move_before("clock", Group::Left, Some("media"));
        let left: Vec<_> = l.in_group(Group::Left).map(|s| s.id.as_str()).collect();
        assert_eq!(left, vec!["search", "clock", "media", "window", "desktops"]);
        l.move_before("search", Group::Center, None);
        let center: Vec<_> = l.in_group(Group::Center).map(|s| s.id.as_str()).collect();
        assert_eq!(center, vec!["shelf", "search"]);
        // ciel v inej skupine sa ignoruje (koniec skupiny), sam pred sebou = bez zmeny
        let before = l.clone();
        l.move_before("media", Group::Left, Some("media"));
        assert_eq!(l, before);
        l.move_before("media", Group::Right, Some("search"));
        assert_eq!(
            l.in_group(Group::Right).last().map(|s| s.id.as_str()),
            Some("media")
        );
        // viditelnost sa nemeni a ziadna polozka sa nestrati
        assert_eq!(l.items.len(), Layout::default().items.len());
    }

    #[test]
    fn an_older_layout_without_the_tool_buttons_gets_them_hidden_next_to_tools() {
        let c = parse(
            r#"{"version": 7, "layout": {"items": [{"id": "tools", "group": "left", "visible": true},
                {"id": "clock", "group": "right", "visible": true}]}}"#,
        )
        .config;
        for id in ["tool_shelf", "tool_clipboard", "tool_notes", "tool_shots"] {
            let s = c.layout.items.iter().find(|s| s.id == id).expect(id);
            assert!(!s.visible, "{id} predvolene skryte");
        }
    }

    #[test]
    fn move_within_keeps_visibility_and_counts_hidden() {
        let mut l = Layout::default();
        l.set_visible("tools", false);
        // pravo: ai(skryte), tray(skryte), battery, clock -> hodiny na index 0 vratane skrytych
        l.move_within("clock", Group::Right, 0);
        let right: Vec<_> = l.in_group(Group::Right).map(|s| s.id.as_str()).collect();
        assert_eq!(
            right,
            vec![
                "clock",
                "ai",
                "audio",
                "tools",
                "tool_shelf",
                "tool_clipboard",
                "tool_notes",
                "tool_shots",
                "tool_snip",
                "tool_picker",
                "focus",
                "weather",
                "peripherals",
                "sysmon",
                "privacy",
                "brightness",
                "timer",
                "keyboard",
                "battery"
            ]
        );
        // presun skrytej polozky ju nezobrazi
        l.move_within("tools", Group::Left, 99);
        let t = l.items.iter().find(|s| s.id == "tools").unwrap();
        assert!(!t.visible && t.group == Group::Left);
        assert_eq!(l.in_group(Group::Left).last().unwrap().id, "tools");
        // do stredu (predvolene je tam len Polica): index za koncom = na koniec skupiny
        l.move_within("search", Group::Center, 3);
        assert_eq!(
            l.in_group(Group::Center)
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["shelf", "search"]
        );
    }

    #[test]
    fn move_and_hide() {
        let mut l = Layout::default();
        l.move_to("clock", Group::Center, 0);
        assert_eq!(
            l.visible_in(Group::Center)
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["clock", "shelf"],
            "hodiny v strede, Polica vedla nich"
        );
        l.move_to("search", Group::Right, 0);
        assert_eq!(
            l.visible_in(Group::Right)
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["search", "tools", "peripherals", "battery"]
        );
        l.move_to("search", Group::Right, 99);
        assert_eq!(l.visible_in(Group::Right).last().unwrap().id, "search");
        l.set_visible("tools", false);
        assert!(!l.visible_in(Group::Right).any(|s| s.id == "tools"));
        // presun skrytej polozky ju zobrazi
        l.move_to("ai", Group::Left, 1);
        assert_eq!(
            l.visible_in(Group::Left)
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["media", "ai"]
        );
    }

    #[test]
    fn save_and_load_file_atomically() {
        let dir = std::env::temp_dir().join(format!("transom-config-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let mut c = Config::default();
        c.clock.seconds = true;
        c.layout.move_to("clock", Group::Left, 0);
        save(&path, &c).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        let l = load(&path);
        assert_eq!(l.config, c);
        assert!(!l.read_only);
        // chybajuci subor = predvolene, zapisovatelne
        let l = load(&dir.join("nie.json"));
        assert_eq!(l.config, Config::default());
        assert!(!l.read_only);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn utf8_bom_is_not_treated_as_corruption() {
        let l = parse("\u{feff}{\"version\": 4, \"language\": \"sk\"}");
        assert!(!l.read_only && l.issue.is_none(), "{l:?}");
        assert_eq!(l.config.language, "sk");
    }

    #[test]
    fn migration_to_v5_adds_empty_dates_and_dates_are_validated_leniently() {
        let l = parse(r#"{"version": 4, "usage": {"refresh_minutes": 15}}"#);
        assert!(!l.read_only && l.config.version == CURRENT_VERSION);
        assert!(l.config.usage.dates.is_empty());
        assert_eq!(l.config.usage.refresh_minutes, 15);
        let l = parse(
            r#"{"version": 5, "usage": {"dates": [
                {"provider": "claude", "kind": "reset", "date": "2026-11-20", "repeat": "days", "every_days": 6},
                {"provider": "codex", "kind": "billing", "date": "2027-01-31", "repeat": "monthly", "every_days": 6},
                {"provider": "codex", "kind": "billing", "date": "2027-02-30", "repeat": "none", "every_days": 6},
                {"provider": "gemini", "kind": "reset", "date": "2026-11-20", "repeat": "none", "every_days": 6},
                {"provider": "claude", "kind": "reset", "date": "2026-12-01"},
                {"provider": "claude", "kind": "reset", "date": "2026-12-01", "repeat": "days", "every_days": 0},
                "nezmysel", 42
            ]}}"#,
        );
        assert!(
            !l.read_only,
            "neplatne polozky nerobia subor len na citanie"
        );
        let d = &l.config.usage.dates;
        assert_eq!(d.len(), 3, "{d:?}");
        assert_eq!(
            (
                d[0].provider.as_str(),
                d[0].every_days,
                d[0].repeat.as_str()
            ),
            ("claude", 6, "days")
        );
        assert_eq!(d[1].date, "2027-01-31");
        assert_eq!(d[2].repeat, "none", "chybajuce opakovanie = jednorazovo");
        // zly typ celeho zoznamu = prazdny zoznam
        assert!(parse(r#"{"version": 5, "usage": {"dates": "x"}}"#)
            .config
            .usage
            .dates
            .is_empty());
        // pocet je obmedzeny
        let many = format!(
            r#"{{"version": 5, "usage": {{"dates": [{}]}}}}"#,
            vec![r#"{"provider":"claude","kind":"reset","date":"2026-11-20","repeat":"none","every_days":6}"#; 40].join(",")
        );
        assert_eq!(parse(&many).config.usage.dates.len(), MAX_MANUAL_DATES);
        // round trip
        let mut c = Config::default();
        c.usage.dates = d.clone();
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(parse(&text).config.usage.dates, c.usage.dates);
    }

    #[test]
    fn migration_to_v7_never_enables_the_archive_and_keeps_the_rest() {
        // v6 bez sekcie archive: archiv je vypnuty, zdroje a ostatne nastavenia ostavaju
        let l = parse(
            r#"{"version": 6, "language": "sk", "usage": {"claude_local": true, "ring_mode": "remaining", "refresh_minutes": 15}}"#,
        );
        assert!(!l.read_only && l.config.version == 7);
        let u = &l.config.usage;
        assert!(u.claude_local && u.ring_mode == "remaining" && u.refresh_minutes == 15);
        assert!(!u.archive.enabled && !u.archive.paused && !u.archive.writing());
        assert_eq!(u.archive.retention_months, ARCHIVE_DEFAULT_RETENTION);
        // aj uplne stary a prazdny subor: archiv vypnuty
        for src in [r#"{"version": 1}"#, "{}", r#"{"version": 6, "usage": {}}"#] {
            let l = parse(src);
            assert!(!l.read_only, "{src}");
            assert!(!l.config.usage.archive.enabled, "{src}");
        }
        // ulozenie a nacitanie zachova volby
        let mut c = Config::default();
        c.usage.archive = Archive {
            enabled: true,
            paused: true,
            retention_months: 6,
        };
        let back = parse(&serde_json::to_string(&c).unwrap());
        assert_eq!(back.config.usage.archive, c.usage.archive);
        assert!(
            !back.config.usage.archive.writing(),
            "pozastavene = nezapisuje"
        );
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("transom-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn two_writers_changing_different_fields_both_survive_and_leave_no_temp_files() {
        let d = tmpdir("two-writers");
        let p = d.join("config.json");
        save(&p, &Config::default()).unwrap();
        // "lista" meni len ring_mode, "nastavenia" meni len clock.seconds; kazdy v cykle 40x (znova nacita, zmeni, zapise)
        std::thread::scope(|s| {
            let p = &p;
            s.spawn(move || {
                for i in 0..40 {
                    let v = if i % 2 == 0 { "used" } else { "remaining" };
                    update(p, |c| c.usage.ring_mode = v.into()).unwrap();
                }
            });
            s.spawn(move || {
                let mut base = load(p).config;
                for i in 0..40 {
                    let mut ours = base.clone();
                    ours.clock.seconds = i % 2 == 0;
                    ours.clock.weekday = i % 3 == 0;
                    base = save_merged(p, &base, &ours).unwrap();
                }
            });
        });
        let end = load(&p);
        assert!(!end.read_only && end.issue.is_none(), "subor sa da nacitat");
        assert_eq!(
            end.config.usage.ring_mode, "remaining",
            "posledna zmena listy (i = 39)"
        );
        assert!(
            !end.config.clock.seconds,
            "posledna zmena nastaveni (i = 39)"
        );
        assert!(
            end.config.clock.weekday,
            "posledna zmena nastaveni (39 % 3 == 0)"
        );
        assert!(
            leftovers(&d).is_empty(),
            "ziadny zostavajuci .tmp: {:?}",
            leftovers(&d)
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn update_and_save_merged_never_overwrite_a_corrupt_or_newer_file() {
        let d = tmpdir("never-overwrite");
        let p = d.join("config.json");
        for text in [
            "{ nie je json".to_string(),
            format!(
                r#"{{"version": {}, "future": {{"x": 1}}}}"#,
                CURRENT_VERSION + 1
            ),
        ] {
            std::fs::write(&p, &text).unwrap();
            assert!(update(&p, |c| c.usage.ring_mode = "used".into()).is_err());
            let base = Config::default();
            let mut ours = base.clone();
            ours.clock.seconds = !ours.clock.seconds;
            assert!(save_merged(&p, &base, &ours).is_err());
            assert_eq!(
                std::fs::read_to_string(&p).unwrap(),
                text,
                "bajty nezmenene"
            );
        }
        assert!(leftovers(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_migrated_weather_city_never_comes_back_after_its_place_is_removed() {
        let d = tmpdir("wx-migrate");
        let p = d.join("config.json");
        std::fs::write(&p, r#"{"version":1,"weather_city":"Trnava"}"#).unwrap();
        let base = load(&p).config;
        assert_eq!(base.weather_places.len(), 1);
        let mut ours = base.clone();
        ours.weather_places.clear();
        ours.normalize_weather();
        save_merged(&p, &base, &ours).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            !text.contains("weather_city") && !text.contains("Trnava"),
            "{text}"
        );
        let again = load(&p).config;
        assert!(again.weather_places.is_empty());
        assert_eq!(
            again.weather_target(),
            None,
            "pocasie sa samo znova nezapne"
        );
    }

    #[test]
    fn save_merged_keeps_their_changes_unknown_keys_and_applies_only_ours() {
        let d = tmpdir("merge");
        let p = d.join("config.json");
        let base = Config::default();
        // iny proces (alebo rucna uprava) medzitym zmenil ring_mode a pridal neznamy kluc
        let mut theirs = base.clone();
        theirs.usage.ring_mode = "used".into();
        theirs
            .extra
            .insert("future_section".into(), serde_json::json!({"x": 1}));
        save(&p, &theirs).unwrap();
        // nastavenia zmenili len hodinovy format a jazyk
        let mut ours = base.clone();
        ours.clock.seconds = !base.clock.seconds;
        ours.language = "de".into();
        let merged = save_merged(&p, &base, &ours).unwrap();
        assert_eq!(merged.usage.ring_mode, "used", "cudzia zmena ostala");
        assert_eq!(merged.clock.seconds, ours.clock.seconds);
        assert_eq!(merged.language, "de");
        assert_eq!(
            merged.extra["future_section"],
            serde_json::json!({"x": 1}),
            "neznamy kluc zachovany"
        );
        assert_eq!(load(&p).config, merged);
        // chybajuci subor: zapise sa `ours`
        let q = d.join("fresh").join("config.json");
        assert_eq!(save_merged(&q, &base, &ours).unwrap(), ours);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn backup_names_with_multibyte_characters_are_foreign_files_not_panics() {
        for n in [
            "config-\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}.json",
            "config-2026112\u{17e}-100000.json",
            "config-20261121-10000\u{20ac}.json",
            "config-\u{20ac}.json",
            "config-.json",
            "config-20261121-100000-\u{17e}.json",
        ] {
            assert!(!is_backup_name(n), "{n}");
        }
        assert!(is_backup_name("config-20261121-100000.json"));
        assert!(is_backup_name("config-20261121-100000-3.json"));
        // zaloha v priecinku s takym cudzim suborom prebehne a cudzi subor ostane
        let dir = tmpdir("backup-mb");
        let cfg = dir.join("config.json");
        std::fs::write(&cfg, "{}").unwrap();
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        let foreign = dir
            .join("backups")
            .join("config-\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}\u{17e}.json");
        std::fs::write(&foreign, "x").unwrap();
        assert!(backup_file(&cfg, "20261121-100000", 1).unwrap().is_some());
        assert!(foreign.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn three_way_merge_takes_only_our_changes_table() {
        use serde_json::json;
        // (base, ours, theirs, vysledok)
        let cases = [
            // nic sme nezmenili: ich subor cely, aj ich nove kluce
            (
                json!({"a": 1}),
                json!({"a": 1}),
                json!({"a": 2, "z": 0}),
                json!({"a": 2, "z": 0}),
            ),
            // zmenili sme a, oni b
            (
                json!({"a": 1, "b": 1}),
                json!({"a": 5, "b": 1}),
                json!({"a": 1, "b": 7}),
                json!({"a": 5, "b": 7}),
            ),
            // ta ista hodnota zmenena oboma: vyhra nasa (posledne ulozenie)
            (
                json!({"a": 1}),
                json!({"a": 2}),
                json!({"a": 3}),
                json!({"a": 2}),
            ),
            // vnorene objekty po klucoch
            (
                json!({"o": {"x": 1, "y": 1}}),
                json!({"o": {"x": 2, "y": 1}}),
                json!({"o": {"x": 1, "y": 9, "n": true}}),
                json!({"o": {"x": 2, "y": 9, "n": true}}),
            ),
            // polia sa nezlucuju po prvkoch: nase cele
            (
                json!({"l": [1, 2]}),
                json!({"l": [1, 2, 3]}),
                json!({"l": [9]}),
                json!({"l": [1, 2, 3]}),
            ),
            // nas novy kluc, nas odstraneny kluc
            (
                json!({"old": 1, "k": 0}),
                json!({"k": 0, "new": 2}),
                json!({"old": 1, "k": 0, "their": 3}),
                json!({"k": 0, "new": 2, "their": 3}),
            ),
            // oni kluc odstranili, my sme ho zmenili: vrati sa nasa hodnota
            (
                json!({"a": {"x": 1}}),
                json!({"a": {"x": 2}}),
                json!({}),
                json!({"a": {"x": 2}}),
            ),
            // zmena typu
            (
                json!({"a": 1}),
                json!({"a": {"x": 1}}),
                json!({"a": 1}),
                json!({"a": {"x": 1}}),
            ),
        ];
        for (b, o, t, want) in cases {
            assert_eq!(
                merge3(&b, &o, t.clone()),
                want,
                "base {b} ours {o} theirs {t}"
            );
        }
    }

    #[test]
    fn schema_version_edge_cases() {
        // (subor, len na citanie, verzia po nacitani)
        let cases = [
            (r#"{}"#, false, CURRENT_VERSION),
            (r#"{"version": 0}"#, false, CURRENT_VERSION),
            (r#"{"version": 1}"#, false, CURRENT_VERSION),
            (
                &*format!(r#"{{"version": {CURRENT_VERSION}}}"#),
                false,
                CURRENT_VERSION,
            ),
            (
                &*format!(r#"{{"version": {}}}"#, CURRENT_VERSION + 1),
                true,
                CURRENT_VERSION + 1,
            ),
            // zly typ verzie = typova chyba celej konfiguracie: subor sa chrani (v pamati predvolby)
            (r#"{"version": "7"}"#, true, CURRENT_VERSION),
            (r#"{"version": -1}"#, true, CURRENT_VERSION),
            (r#"{"version": 7.5}"#, true, CURRENT_VERSION),
            (r#"{"version": 4294967296}"#, true, CURRENT_VERSION),
        ];
        for (text, ro, version) in cases {
            let l = parse(text);
            assert_eq!(l.read_only, ro, "{text}");
            assert_eq!(l.config.version, version, "{text}");
        }
        // novsia verzia: neznamy kluc ostava, zname sa citaju
        let l = parse(&format!(
            r#"{{"version": {}, "clock": {{"seconds": true}}, "future": {{"x": 1}}}}"#,
            CURRENT_VERSION + 1
        ));
        assert_eq!(l.issue, Some(Issue::Newer(CURRENT_VERSION + 1)));
        assert!(l.config.clock.seconds);
        assert!(l.config.extra.contains_key("future"));
        // zmena v novsej verzii sa nikdy nezapise
        let dir = tmpdir("newer-version");
        let path = dir.join("config.json");
        let text = format!(r#"{{"version": {}, "future": 1}}"#, CURRENT_VERSION + 1);
        std::fs::write(&path, &text).unwrap();
        assert!(update(&path, |c| c.clock.seconds = true).is_err());
        assert!(save_merged(&path, &Config::default(), &Config::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn temp_file_names_are_unique_per_call() {
        let p = Path::new("C:/x/config.json");
        let (a, b) = (unique_tmp(p), unique_tmp(p));
        assert_ne!(a, b);
        assert!(a.to_string_lossy().ends_with(".tmp") && a.parent() == p.parent());
    }

    #[test]
    fn a_corrupt_or_unreadable_reload_keeps_the_running_configuration() {
        let mut running = Config::default();
        running.usage.claude_local = true;
        running.clock.seconds = !running.clock.seconds;
        // poskodeny / useknuty JSON (napr. subor rozpisany inym procesom)
        for text in [
            "",
            "{",
            r#"{"version": 7, "usage": {"claude_l"#,
            "nie je json",
        ] {
            let l = parse(text);
            assert_eq!(l.issue, Some(Issue::Corrupt), "{text:?}");
            assert_eq!(l.effective(&running), running, "{text:?}");
        }
        let unreadable = Loaded {
            config: Config::default(),
            read_only: true,
            note: None,
            issue: Some(Issue::Unreadable),
        };
        assert_eq!(unreadable.effective(&running), running);
        // chybajuci subor = platne predvolene hodnoty (pouzivatel subor zmazal) a platny subor sa prevezme
        assert_eq!(
            load(Path::new("Z:/transom-neexistuje/config.json")).effective(&running),
            Config::default()
        );
        let mut next = running.clone();
        next.usage.codex_local = true;
        let text = serde_json::to_string(&next).unwrap();
        assert_eq!(parse(&text).effective(&running), next);
        // novsia verzia sa nacita tolerantne (len na citanie) a prevezme sa
        let newer = parse(&format!(
            r#"{{"version": {}, "usage": {{"codex_local": true}}}}"#,
            CURRENT_VERSION + 1
        ));
        assert_eq!(newer.issue, Some(Issue::Newer(CURRENT_VERSION + 1)));
        assert!(newer.clone().effective(&running).usage.codex_local);
    }

    #[test]
    fn archive_settings_are_read_leniently() {
        // neznama retencia / zly typ = predvolena; subor nie je len na citanie
        for src in [
            r#"{"version": 7, "usage": {"archive": {"enabled": true, "retention_months": 5}}}"#,
            r#"{"version": 7, "usage": {"archive": {"enabled": true, "retention_months": "x"}}}"#,
            r#"{"version": 7, "usage": {"archive": {"enabled": true, "retention_months": -3}}}"#,
        ] {
            let l = parse(src);
            assert!(!l.read_only, "{src}");
            assert!(l.config.usage.archive.enabled, "{src}");
            assert_eq!(
                l.config.usage.archive.retention_months, ARCHIVE_DEFAULT_RETENTION,
                "{src}"
            );
        }
        for m in ARCHIVE_RETENTIONS {
            let l = parse(&format!(
                r#"{{"version": 7, "usage": {{"archive": {{"retention_months": {m}}}}}}}"#
            ));
            assert_eq!(l.config.usage.archive.retention_months, m);
        }
        // archiv sam o sebe nezapina zdroje ani nic ine
        let l = parse(r#"{"version": 7, "usage": {"archive": {"enabled": true}}}"#);
        assert!(!l.config.usage.any_enabled());
    }

    #[test]
    fn migration_to_v6_defaults_to_all_displays_and_validates_leniently() {
        let l = parse(r#"{"version": 5, "language": "sk"}"#);
        assert!(!l.read_only && l.config.version == CURRENT_VERSION);
        assert_eq!(l.config.displays, Displays::default());
        assert_eq!(l.config.displays.mode, "all");
        let l = parse(
            r##"{"version": 6, "displays": {"mode": "selected", "selected": [
                {"id": "DISPLAY#A#1", "label": "AOC 2402 1080x1920"},
                {"id": "display#a#1", "label": "duplikat"},
                {"id": "  "}, {"id": "b"}, 7, {"label": "bez id"}]}}"##,
        );
        assert!(!l.read_only);
        let d = &l.config.displays;
        assert_eq!(d.mode, "selected");
        assert_eq!(d.selected.len(), 2, "{d:?}");
        assert_eq!(d.selected[1].label, "", "chybajuci popis = prazdny");
        // neznamy rezim = all (subor nie je len na citanie)
        let l = parse(r#"{"version": 6, "displays": {"mode": "nezmysel", "selected": "x"}}"#);
        assert!(!l.read_only);
        assert_eq!(
            (
                l.config.displays.mode.as_str(),
                l.config.displays.selected.len()
            ),
            ("all", 0)
        );
        // round trip
        let c = Config {
            displays: Displays {
                mode: "selected".into(),
                selected: vec![DisplayRef {
                    id: "x".into(),
                    label: "L".into(),
                }],
            },
            ..Config::default()
        };
        assert_eq!(
            parse(&serde_json::to_string(&c).unwrap()).config.displays,
            c.displays
        );
    }
}

#[cfg(test)]
mod extras_cfg_tests {
    use super::*;

    #[test]
    fn weather_city_migrates_to_a_place_and_bad_places_or_sources_are_fixed() {
        let c = parse(r#"{"weather_city":"Trnava"}"#).config;
        assert_eq!(c.weather_places.len(), 1);
        assert_eq!(c.weather_places[0].name, "Trnava");
        assert_eq!(c.weather_source, WeatherSource::Place(0));
        assert!(c.weather_city.is_empty());
        let out = serde_json::to_string(&c).unwrap();
        assert!(!out.contains("weather_city"), "stare pole sa uz neuklada");
        let c2 = parse(&out).config;
        assert_eq!(c2.weather_places, c.weather_places);
        assert_eq!(c2.weather_source, WeatherSource::Place(0));

        let c = parse(r#"{"weather_places":[{"name":"A","lat":1,"lon":2},{"name":"B","lat":999,"lon":0},"x"],"weather_source":"5"}"#).config;
        assert_eq!(c.weather_places.len(), 1, "neplatne miesta vypadnu");
        assert_eq!(
            c.weather_source,
            WeatherSource::Place(0),
            "zdroj mimo zoznamu"
        );
        let c = parse(r#"{"weather_source":"device"}"#).config;
        assert_eq!(c.weather_target(), Some(WeatherTarget::Device));
        assert_eq!(
            Config::default().weather_target(),
            None,
            "predvolene vypnute"
        );
    }

    #[test]
    fn new_item_settings_are_lenient_clamped_and_hidden_by_default() {
        let c = parse(r#"{"timer":{"work_min":0,"rounds":"x","sound":false},"sysmon":{"interval_secs":99,"gpu":true},"privacy":{"mic_hotkey":"Ctrl+V"}}"#).config;
        assert_eq!(
            (c.timer.work_min, c.timer.rounds, c.timer.sound),
            (1, 4, false)
        );
        assert_eq!(
            (c.sysmon.interval_secs, c.sysmon.gpu, c.sysmon.cpu),
            (10, true, true)
        );
        // neplatna skratka (Ctrl+V) = predvolena; "off" = vypnuta
        assert_eq!(
            c.privacy.hotkey().map(|h| h.label()),
            Some("Ctrl+Alt+M".into())
        );
        let off = PrivacyCfg {
            mic_hotkey: "off".into(),
            ..PrivacyCfg::default()
        };
        assert!(off.hotkey().is_none());
        let l = Layout::default();
        for id in ["keyboard", "timer", "brightness", "sysmon", "privacy"] {
            assert!(
                !l.items.iter().find(|s| s.id == id).unwrap().visible,
                "{id}"
            );
        }
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;

    #[test]
    fn a_monitor_uses_its_own_layout_and_appearance_or_the_common_ones() {
        let mut c = Config::default();
        let mut own = c.layout.clone();
        own.set_visible("weather", true);
        let look = Appearance {
            bg_color: "#FF0000".into(),
            ..Default::default()
        };
        c.monitors.push(MonitorOverride {
            id: "MON#A".into(),
            label: "A".into(),
            layout: Some(own.clone()),
            appearance: None,
        });
        c.monitors.push(MonitorOverride {
            id: "MON#B".into(),
            label: "B".into(),
            layout: None,
            appearance: Some(look.clone()),
        });
        assert_eq!(c.layout_for("mon#a"), &own, "ID bez ohladu na velkost");
        assert_eq!(c.appearance_for("MON#A"), &c.appearance);
        assert_eq!(c.layout_for("MON#B"), &c.layout);
        assert_eq!(c.appearance_for("MON#B"), &look);
        assert_eq!(c.layout_for("iny"), &c.layout);
        // zapis a nacitanie zachova zaznamy
        let back = parse(&serde_json::to_string(&c).unwrap()).config;
        assert_eq!(back.monitors, c.monitors);
    }

    #[test]
    fn broken_or_duplicate_monitor_entries_are_skipped() {
        let json = r#"{"version":7,"monitors":[{"id":""},{"id":"X","layout":{"items":[]}},{"id":"x"},3,{"id":"Y","appearance":{"bg_opacity":5}}]}"#;
        let c = parse(json).config;
        let ids: Vec<&str> = c.monitors.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["X", "Y"]);
        // prazdne rozlozenie sa doplni o vsetky zname polozky (ako spolocne)
        assert!(!c.monitors[0].layout.as_ref().unwrap().items.is_empty());
        assert_eq!(
            c.monitors[1].appearance.as_ref().unwrap().opacity_percent(),
            MIN_OPACITY_PERCENT
        );
    }
}
