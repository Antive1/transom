//! Interny model stavovych poloziek. Polozka je cisty datovy popis (bez closures a bez Win32): z coho sa sklada,
//! na ktorej strane lisaty je a ako skoro sa zahodi pri uzkej sirke. Aplikacia ho postavi zo stavu pri kazdom
//! prekresleni; renderer ho len nakresli a rozlozenie (`layout`) mu priradi miesto.
use crate::image::Image;
use std::sync::Arc;

/// Identita polozky (pre hit-testing a hover).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemId {
    Search,
    Media,
    /// Docasny oznam (napr. "Search sa neotvoril"), vlavo za mediami.
    Notice,
    /// Nastroje: polica, schranka, poznamky, snimky a hladanie (nahradilo tlacidlo oblasti oznameni).
    Tools,
    Battery,
    Clock,
    /// Prehlad vyuzitia AI (Claude Code, Codex) - otvara panel.
    Ai,
    /// Zvuk: systemovy vystup a rychle akcie - otvara panel.
    Audio,
    /// Baterie periferii (mysi, klavesnice, sluchadla, ovladace) - otvara panel.
    Peripherals,
    /// Subory Police priamo v liste (predvolene v strede; ak je v strede ina polozka, vedla nej). Klik skopiruje subor do
    /// schranky, tahanie ho prenesie inam (kopia), "+N" otvori panel Nastroje.
    Shelf,
    /// Rozlozenie klavesnice (jazyk) a Caps Lock.
    Keyboard,
    /// Casovac, pomodoro a stopky.
    Timer,
    /// Jas monitorov (DDC/CI).
    Brightness,
    /// Vytazenie systemu (CPU, RAM, GPU, siet).
    SysMon,
    /// Mikrofon a kamera: kto ich prave pouziva, stlmenie mikrofonu.
    Privacy,
    /// Samostatne tlacidla nastrojov (otvoria panel Nastroje rovno na danej casti).
    ToolShelf,
    ToolClipboard,
    ToolNotes,
    ToolShots,
    /// Vyrez obrazovky rovno do odlozenych suborov.
    ToolSnip,
    /// Kapatko na farbu.
    ToolPicker,
    /// Nazov aktivneho okna (aplikacia a titulok).
    Window,
    /// Virtualne plochy (aktualna z poctu).
    Desktops,
    /// Rezim sustredenia: otvori centrum oznameni Windows (Nerusit, Sustredenie).
    Focus,
    /// Pocasie (teplota a stav; predpoved vo vybezku).
    Weather,
    /// Dostupna aktualizacia: ikona stahovania uplne vpravo (nie je v rozlozeni, zobrazi sa sama).
    Update,
    /// Deliaca ciara medzi polozkami (pouzivatel ich moze pridat viac; cislo = id v konfiguracii `sep-N`).
    Separator(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    /// Stredova skupina: centrovana na liste, pri kolizii sa posunie do volneho miesta medzi lavou a pravou.
    Center,
    Right,
}

/// Co sa stane po kliknuti na cast polozky. Vykonava aplikacia, nie model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Windows Search (Win+S): uz len ako volitelna rychla akcia, hlavne hladanie je vstavany vyhladavac.
    Search,
    /// Otvorit vstavany vyhladavac transom.
    Launcher,
    Tray,
    MediaPrev,
    MediaToggle,
    MediaNext,
    /// Prepnut na dalsiu vierohodnu media session (ak ich je viac).
    MediaCycle,
    /// Skryt napovedu (napr. Spotify bez session) do konca behu Spotify.
    DismissHint,
    /// Aktivovat / otvorit aplikaciu zvolenej media session (klik na obal).
    MediaOpenSource,
    /// Otvorit/zavriet kalendar (klik na hodiny).
    Calendar,
    /// Otvorit/zavriet panel vyuzitia AI.
    AiPanel,
    /// Otvorit/zavriet panel Zvuk a rychle akcie.
    AudioPanel,
    /// Otvorit/zavriet panel Nastroje.
    ToolsPanel,
    /// Otvorit/zavriet panel Baterie periferii.
    PeripheralsPanel,
    /// Subor Police v liste (ktory - urci aplikacia podla indexu casti z posledneho kreslenia).
    ShelfFile,
    /// Ostrov v strede listy: otvorit, co k aktivite patri (ovladanie hudby, panel AI, priecinok, casovac...).
    Island,
    /// Prepnut rozlozenie klavesnice (vyber jazyka Windows).
    KeyboardSwitch,
    /// Otvorit/zavriet panel Casovac.
    TimerPanel,
    /// Otvorit/zavriet panel Jas.
    BrightnessPanel,
    /// Otvorit Spravcu uloh.
    SysMonOpen,
    /// Otvorit/zavriet panel Mikrofon a kamera.
    PrivacyPanel,
    /// Otvorit panel Nastroje rovno na Polici / Schranke / Poznamkach / Snimkach.
    ToolsShelf,
    ToolsClipboard,
    ToolsNotes,
    ToolsShots,
    /// Spustit vyrez obrazovky Windows a snimku pridat do odlozenych suborov.
    SnipToStash,
    /// Kapatko: dalsie kliknutie na obrazovke precita farbu a HEX skopiruje do schranky.
    ColorPick,
    /// Prepnut na dalsiu virtualnu plochu (za poslednou prva).
    DesktopNext,
    /// Otvorit centrum oznameni Windows (prepinace Nerusit a Sustredenie; zvonka ich zapnut neda).
    FocusPanel,
    /// Ukazat predpoved pocasia vo vybezku.
    WeatherOpen,
    /// Otvorit/zavriet panel dostupnej aktualizacie (co je nove, Aktualizovat teraz).
    UpdatePanel,
}

/// Ikona; konkretny znak/font urcuje renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Search,
    /// Nastroje (mriezka).
    Tools,
    Prev,
    Play,
    Pause,
    Next,
    /// Reproduktor: hlasitost systemoveho vystupu 0 / nizka / stredna / vysoka a stlmeny (stlmenie ma vlastny tvar).
    Volume0,
    Volume1,
    Volume2,
    Volume3,
    VolumeMuted,
    /// Druh periferie (polozka Baterie periferii).
    Mouse,
    Keyboard,
    Headset,
    Controller,
    Device,
    /// Subor / priecinok (polozka Police v liste).
    File,
    Folder,
    /// Casovac (stopky).
    Timer,
    /// Jas (slnko).
    Brightness,
    /// Mikrofon, stlmeny mikrofon, kamera.
    Mic,
    MicOff,
    Camera,
    /// Caps Lock zapnuty.
    CapsLock,
    /// Samostatne tlacidla nastrojov: Polica, Schranka, Poznamky, Snimky.
    Shelf,
    Clipboard,
    Note,
    Screenshot,
    /// Vyrez obrazovky (nožnice).
    Snip,
    /// Kapatko na farbu (paleta).
    Picker,
    /// Virtualne plochy (zobrazenie uloh).
    Desktops,
    /// Rezim sustredenia (mesiac).
    Focus,
    /// Pocasie (oblak).
    Weather,
    /// Pocasie: jasno (slnko).
    WeatherClear,
    /// Pocasie: burka (blesk).
    WeatherStorm,
    /// Pocasie: dazd / sneh - oblak, pod nim padaju kvapky / vlocky (snimka `phase` z
    /// `transom_render::scene::PRECIP_FRAMES`).
    Precip {
        snow: bool,
        phase: u8,
    },
    /// Stahovanie (dostupna aktualizacia).
    Download,
}

/// Pruzny text: pri nedostatku miesta sa skrati (elipsa) najviac na `min_lp`; ak sa nezmesti ani to, cast sa skryje.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flex {
    pub min_lp: i32,
    pub max_lp: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Normal,
    /// Menej vyrazny text (napoveda, indikator vyberu).
    Muted,
}

/// Znacka v kapsule ostrova.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Logo transom.
    Transom,
    /// Znak ikony (Segoe Fluent Icons).
    Icon(char),
    /// Oranzova hviezdicka (pracuje AI).
    Spark,
}

/// Logo AI v ostrove (vlastna kresba, nie ochranne znamky).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiLogo {
    /// Oranzova hviezdicka.
    Claude,
    /// Terminalova vyzva ">_".
    Codex,
}

/// Posuv dlheho textu v ostrove: cas od zaciatku a rychlost / smer ako v polozke Music.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scroll {
    pub ms: u32,
    /// Rychlost v lp/s.
    pub speed_lp: u16,
    pub rtl: bool,
}

/// Strana ostrova (vlavo / vpravo od kapsuly).
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    None,
    /// Text (`muted` = sedy, `strong` = tucny; `low` = farba upozornenia).
    Text {
        text: String,
        muted: bool,
        strong: bool,
        low: bool,
        /// Dlhy text sa posuva ako polozka Music (`None` = skrati sa "...").
        scroll: Option<Scroll>,
    },
    /// Obal albumu (`None` = zastupny symbol).
    Art(Option<ArtImage>),
    /// Obal a vedla neho text (nazov skladby).
    Cover {
        art: Option<ArtImage>,
        text: String,
        /// Dlhy text sa posuva ako polozka Music (`None` = skrati sa "...").
        scroll: Option<Scroll>,
    },
    /// Logo AI a vedla neho text (limit). `low` = malo kapacity (farba upozornenia).
    Logo {
        logo: AiLogo,
        text: String,
        low: bool,
        /// Stavova bodka pri logu (farba; `None` = bez nej): pracuje / caka na pouzivatela.
        dot: Option<(u8, u8, u8)>,
    },
    /// Ekvalizer: vysky stlpcov 0..=255 (zo zvuku alebo animacie), farba (napr. z obalu; `None` = akcent).
    Eq {
        levels: [u8; 5],
        color: Option<(u8, u8, u8)>,
        playing: bool,
    },
}

/// Dekodovany obal s identitou; porovnava sa len podla `id` (lacne porovnanie poloziek pri kazdom pohybe mysi).
#[derive(Clone, Debug)]
pub struct ArtImage {
    pub id: u64,
    pub image: Arc<Image>,
}

impl PartialEq for ArtImage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// Stvorcove tlacidlo s ikonou (sirka = `Metrics::button_lp`). Zakazane tlacidlo (`enabled == false`) sa kresli
    /// stlmene, nema hover, stlacenie ani akciu, ale ma tooltip a pristupny nazov.
    Icon {
        glyph: Glyph,
        action: Option<Action>,
        enabled: bool,
        /// Pristupny nazov a tooltip (napr. "Dalsia skladba").
        name: String,
    },
    /// Obal albumu / nahlad videa (stvorec, sirka = `Metrics::button_lp`); `None` = neutralny zastupny symbol.
    /// Kruhovy indikator (limit planu AI), sirka = `Metrics::button_lp`. `fill` = vyplnena cast obvodu v percentach
    /// (`None` = prazdny kruh: hodnota nedostupna alebo neplatna), `ccw` = vypln proti smeru hodinovych ruciciek
    /// (zostavajuca cast - netýka sa len farby), `muted` = posledna zaznamenana, mozno neaktualna hodnota.
    /// `label` = text vedla kruhu (napr. "62 % vyuzite" - vyznam nie je len vo farbe), `name` = pristupny nazov
    /// a tooltip. Sirka = kruh + text.
    Ring {
        fill: Option<u8>,
        ccw: bool,
        muted: bool,
        /// Zostava malo kapacity (alebo je limit vycerpany): arc farbou `low` a text tucne - **nie len farbou**,
        /// text nesie aj znacku ("!"), ktoru dodava zostavovatel `label`.
        alert: bool,
        label: String,
        name: String,
        action: Option<Action>,
    },
    /// `action` = co urobi klik (aktivacia zdrojovej aplikacie).
    Art {
        art: Option<ArtImage>,
        action: Option<Action>,
    },
    Text {
        text: String,
        /// `None` = pevna sirka podla textu.
        flex: Option<Flex>,
        tone: Tone,
        action: Option<Action>,
    },
    /// Symbol baterie (kresleny obdlznikmi); `percent` 0..=100.
    Battery { percent: u8, charging: bool },
    /// Bateria periferie: maly symbol zariadenia, symbol baterie a text urovne; cela cast je jedno tlacidlo.
    /// `percent == None` = zariadenie uroven nehlasi (prazdna bateria, text "–"); `muted` = posledna hodnota spiaceho
    /// zariadenia; `low` = nizka uroven (vypln farbou `low` - stav nesie aj `name`, nie len farba).
    Device {
        glyph: Glyph,
        percent: Option<u8>,
        charging: bool,
        muted: bool,
        low: bool,
        label: String,
        /// Pristupny nazov a tooltip ("Razer DeathAdder V4 Pro: 64 %, nabija sa").
        name: String,
        action: Option<Action>,
    },
    /// Deliaca ciara (polozka Oddelovac): uzka cast s tenkou zvislou ciarou, bez akcie.
    Divider,
    /// Ostrov v strede listy: kapsula so znackou a nazvom (v pokoji transom, pocas aktivity jej ikona a nazov), vlavo
    /// a vpravo od nej obsah aktivity v rovnako sirokych stranach (kapsula ostava v strede). `badge` = pocet
    /// odlozenych suborov (prazdne = nic). Cela cast je jedno tlacidlo.
    Island {
        left: Seg,
        mark: Mark,
        label: String,
        badge: String,
        right: Seg,
        /// Pristupny nazov a tooltip.
        name: String,
        action: Option<Action>,
    },
    /// Subor v liste: maly symbol (subor / priecinok) a skrateny nazov; cela cast je jedno tlacidlo. `muted` = subor nie je
    /// dostupny (chyba, siet, cloud) - stav nesie aj `name`, nie len farba.
    File {
        glyph: Glyph,
        muted: bool,
        label: String,
        /// Pristupny nazov a tooltip (cely nazov suboru a stav).
        name: String,
        action: Option<Action>,
    },
}

impl Part {
    /// Akcia po kliknuti; zakazane tlacidlo nema ziadnu.
    pub fn action(&self) -> Option<Action> {
        match self {
            Part::Icon {
                action, enabled, ..
            } => action.filter(|_| *enabled),
            Part::Text { action, .. }
            | Part::Art { action, .. }
            | Part::Ring { action, .. }
            | Part::Device { action, .. }
            | Part::File { action, .. }
            | Part::Island { action, .. } => *action,
            Part::Battery { .. } | Part::Divider => None,
        }
    }

    /// Tlacidlo s ikonou (povolene alebo zakazane).
    pub fn button(glyph: Glyph, action: Action, enabled: bool, name: impl Into<String>) -> Self {
        Part::Icon {
            glyph,
            action: Some(action),
            enabled,
            name: name.into(),
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Part::Text {
            text: text.into(),
            flex: None,
            tone: Tone::Normal,
            action: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatusItem {
    pub id: ItemId,
    pub side: Side,
    /// Poradie zahadzovania pri uzkej sirke: vyssie cislo sa zahodi skor; 0 = nikdy (hodiny).
    pub drop_rank: u8,
    pub parts: Vec<Part>,
}

/// Poradie zahadzovania (vyssie skor). Pruzne texty sa skracuju/skryvaju este pred zahodenim celej polozky.
pub const fn default_drop_rank(id: ItemId) -> u8 {
    match id {
        ItemId::Notice => 6,
        ItemId::Media => 5,
        ItemId::Tools => 4,
        ItemId::Search => 3,
        ItemId::Ai => 5,
        ItemId::Audio => 4,
        ItemId::Peripherals => 4,
        ItemId::Shelf => 5,
        ItemId::Keyboard => 4,
        ItemId::Timer => 3,
        ItemId::Brightness => 4,
        ItemId::SysMon => 5,
        ItemId::Privacy => 2,
        ItemId::ToolShelf
        | ItemId::ToolClipboard
        | ItemId::ToolNotes
        | ItemId::ToolShots
        | ItemId::ToolSnip
        | ItemId::ToolPicker => 4,
        ItemId::Window => 6,
        ItemId::Desktops => 3,
        ItemId::Focus => 4,
        ItemId::Weather => 5,
        // oddelovac nema vlastny obsah: pri nedostatku miesta ide prvy (a sam zmizne, ked nema susedov)
        ItemId::Separator(_) => 7,
        ItemId::Battery => 2,
        // aktualizacia ostava co najdlhsie (hned po hodinach)
        ItemId::Update => 1,
        ItemId::Clock => 0,
    }
}

impl StatusItem {
    pub fn new(id: ItemId, side: Side, parts: Vec<Part>) -> Self {
        StatusItem {
            id,
            side,
            drop_rank: default_drop_rank(id),
            parts,
        }
    }
}
