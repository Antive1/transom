//! Temy: svetla, tmava a "podla systemu". Cista data a logika, bez Win32.
use crate::brand::{colors, Rgba};
use crate::painter::Color;

/// Volba pouzivatela (konfiguracia).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    /// Podla systemoveho nastavenia (Windows: tmavy/svetly rezim panela).
    #[default]
    System,
}

impl ThemeMode {
    /// Z textu konfiguracie/argumentu (`light`, `dark`, `system`, bez ohladu na velkost pismen).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => Some(ThemeMode::Light),
            "dark" => Some(ThemeMode::Dark),
            "system" => Some(ThemeMode::System),
            _ => None,
        }
    }

    /// Zvoli konkretnu temu; `system_dark` sa pouzije len pri `System`.
    pub fn resolve(self, system_dark: bool) -> Theme {
        match self {
            ThemeMode::Light => Theme::light(),
            ThemeMode::Dark => Theme::dark(),
            ThemeMode::System => {
                if system_dark {
                    Theme::dark()
                } else {
                    Theme::light()
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub dark: bool,
    pub background: Color,
    pub foreground: Color,
    /// Menej vyrazny text (napoveda, indikator).
    pub muted: Color,
    /// Pozadie casti pod mysou (len klikatelne casti).
    pub hover: Color,
    /// Nizka bateria.
    pub low: Color,
    /// Nabijajuca sa bateria (zelena).
    pub good: Color,
    /// Pozadie stlacenej casti.
    pub pressed: Color,
    /// Ikona zakazaneho tlacidla (nepodporovany prikaz).
    pub disabled: Color,
    /// Neutralny zastupny symbol obalu.
    pub placeholder: Color,
    /// Akcent (dnesok v kalendari, aktivne volby).
    pub accent: Color,
    /// Text na akcente.
    pub on_accent: Color,
    /// Text v farbe akcentu na pozadi.
    pub accent_text: Color,
    /// Jemny okraj (vlas) pre nastavenia a popovery; na lište sa okraje a oddelovace pocitaju z textu nad skutocnym
    /// pozadim (pozri `scene`), aby fungovali s vlastnou farbou aj pri priehladnosti.
    pub edge: Color,
    /// Zdvihnuta plocha karty (sekcia nastaveni, karta v popoveri) nad `background`.
    pub surface: Color,
    /// Vypln ovladaca (bezne tlacidlo, nevybrana volba, policko) v pokoji / pod mysou / stlaceneho.
    pub control: Color,
    pub control_hover: Color,
    pub control_pressed: Color,
    /// Okraj ovladaca (zretelnejsi nez `edge`).
    pub control_edge: Color,
    /// Akcentova vypln pod mysou / stlacena (hlavne tlacidlo, zapnute policko).
    pub accent_hover: Color,
    pub accent_pressed: Color,
    /// Vypln vybranej volby (akcentovy nadych; vzdy spolu s akcentovym okrajom a polotucnym textom).
    pub selected: Color,
    /// Nebezpecna akcia: text a okraj, nadych pod mysou, plna vypln potvrdenia (aj pod mysou) a text na nej.
    pub danger: Color,
    pub danger_tint: Color,
    pub danger_fill: Color,
    pub danger_hover: Color,
    pub on_danger: Color,
}

impl Theme {
    pub const fn dark() -> Self {
        Theme {
            dark: true,
            background: c(colors::INK),
            foreground: Color::hex(0xE8ECF1),
            muted: Color::hex(0x98A3B0),
            hover: Color::hex(0x2A313B),
            low: Color::hex(0xF2766E),
            good: Color::hex(0x6CCB5F),
            pressed: Color::hex(0x20262E),
            disabled: Color::hex(0x5A626D),
            placeholder: Color::hex(0x232933),
            accent: Color::hex(0x0070E0),
            on_accent: Color::hex(0xFFFFFF),
            accent_text: Color::hex(0x5EADFF),
            edge: Color::hex(0x2E353F),
            surface: Color::hex(0x1E232A),
            control: Color::hex(0x2A313B),
            control_hover: Color::hex(0x2F3640),
            control_pressed: Color::hex(0x262C35),
            control_edge: Color::hex(0x3A424D),
            accent_hover: Color::hex(0x0064CC),
            accent_pressed: Color::hex(0x0058B4),
            selected: Color::hex(0x10223A),
            danger: Color::hex(0xF2766E),
            danger_tint: Color::hex(0x382B30),
            danger_fill: Color::hex(0xC42B1C),
            danger_hover: Color::hex(0xA8251A),
            on_danger: Color::hex(0xFFFFFF),
        }
    }

    pub const fn light() -> Self {
        Theme {
            dark: false,
            background: c(colors::MIST),
            foreground: Color::hex(0x14181E),
            muted: Color::hex(0x505B69),
            hover: Color::hex(0xDFE4EA),
            low: Color::hex(0xA8281F),
            good: Color::hex(0x107C10),
            pressed: Color::hex(0xCDD3DB),
            disabled: Color::hex(0xA6AEB8),
            placeholder: Color::hex(0xE3E7EC),
            accent: Color::hex(0x0066D6),
            on_accent: Color::hex(0xFFFFFF),
            accent_text: Color::hex(0x0052AD),
            edge: Color::hex(0xD3D9E0),
            surface: Color::hex(0xFFFFFF),
            control: Color::hex(0xFAFBFC),
            control_hover: Color::hex(0xF0F3F6),
            control_pressed: Color::hex(0xE6EAEE),
            control_edge: Color::hex(0xC9D0D8),
            accent_hover: Color::hex(0x0059BD),
            accent_pressed: Color::hex(0x004FA8),
            selected: Color::hex(0xE1EDFC),
            danger: Color::hex(0xA8281F),
            danger_tint: Color::hex(0xF9EDED),
            danger_fill: Color::hex(0xC42B1C),
            danger_hover: Color::hex(0xA8251A),
            on_danger: Color::hex(0xFFFFFF),
        }
    }
}

impl Theme {
    /// Tema jedneho prechodu skleneneho panelu nad podkladom `backdrop` (cierny alebo biely, pozri
    /// `transom_platform::popup`). Plochy (karta, ovladac, vyber, okraje) su jemne biele / cierne nadychy so skutocnou
    /// alfou: tu su predmiesane nad `backdrop` a alfa sa obnovi z rozdielu dvoch prechodov. Plochy vnorene do karty
    /// (ovladac na karte) maju vyssiu alfu nez karta, takze v nej nikdy nevytvoria "dieru". Akcent, nebezpecna vypln
    /// a text ostavaju nepriehladne.
    pub fn glass_pass(self, backdrop: Color) -> Theme {
        const W: Color = Color::rgb(255, 255, 255);
        const K: Color = Color::rgb(0, 0, 0);
        let o = |top: Color, a: u8| crate::icons::mix(backdrop, top, a);
        let (lift, sink) = (W, K);
        let mut t = self;
        t.background = backdrop;
        if self.dark {
            t.surface = o(lift, 20);
            t.control = o(lift, 30);
            t.control_hover = o(lift, 44);
            t.control_pressed = o(lift, 22);
            t.hover = o(lift, 30);
            t.pressed = o(lift, 20);
            t.placeholder = o(lift, 24);
            // svetly lem kariet a ovladacov (ako dlazdice skla)
            t.edge = o(lift, 44);
            t.control_edge = o(lift, 60);
            // pod sklom moze byt aj biela stranka (najhorsi pripad ~#1D1D1F + karta): svetlejsi tlmeny, cerveny
            // a akcentovy text, aby mali aj na karte a riadku pod mysou kontrast >= 4,5 : 1
            t.muted = Color::hex(0xB4BDC8);
            t.low = Color::hex(0xFF8F86);
            t.good = Color::hex(0x7DD870);
            t.accent_text = Color::hex(0x7CBCFF);
        } else {
            t.surface = o(lift, 110);
            t.control = o(lift, 150);
            t.control_hover = o(lift, 190);
            t.control_pressed = o(sink, 22);
            t.hover = o(sink, 16);
            t.pressed = o(sink, 26);
            t.placeholder = o(sink, 14);
            t.edge = o(sink, 22);
            t.control_edge = o(sink, 34);
            // pod svetlym sklom moze byt aj cierna stranka: tmavsi tlmeny, cerveny a akcentovy text
            t.muted = Color::hex(0x3D4652);
            t.low = Color::hex(0x8E1F17);
            t.good = Color::hex(0x0B5A0B);
            t.accent_text = Color::hex(0x00448F);
        }
        t.selected = o(self.accent, if self.dark { 70 } else { 50 });
        t.danger_tint = o(self.danger, if self.dark { 56 } else { 36 });
        t
    }

    /// Tema lišty s vlastnou farbou pozadia `bg`. Text a ikony idu podla temy; ak by vsak text temy mal na `bg` slaby
    /// kontrast (< 4,5 : 1) a text opacnej temy by bol citatelnejsi, pouziju sa farby opacnej temy (citatelnost ma prednost
    /// pred zhodou s temou). Pozadie je vzdy `bg`.
    pub fn on_background(self, bg: Color) -> Theme {
        let other = if self.dark {
            Theme::light()
        } else {
            Theme::dark()
        };
        let mine = crate::audit::ratio(self.foreground, bg);
        let theirs = crate::audit::ratio(other.foreground, bg);
        let mut t = if mine < 4.5 && theirs > mine {
            other
        } else {
            self
        };
        t.background = bg;
        // tlmeny text (napr. "Claude · 5 h · resets ...") a akcent (kruh) musia byt citatelne tiez: ak nie su, posunu sa
        // k farbe hlavneho textu, kym nedosiahnu 4,5 : 1 (akcent 3 : 1) - alebo kym nie su rovnake ako hlavny text
        t.muted = toward(t.muted, t.foreground, bg, 4.5);
        t.low = toward(t.low, t.foreground, bg, 4.5);
        t.good = toward(t.good, t.foreground, bg, 3.0);
        t.accent = toward(t.accent, t.foreground, bg, 3.0);
        t
    }

    /// Tema lišty: bez vlastnej farby tema; s vlastnou farbou `on_background`, ak je lista aspon z polovice nepriehladna
    /// (`alpha` 0-255). Pod polovicou je pod listou hlavne plocha, ktoru nepozname - text ostava podla temy (ako plocha
    /// a panel uloh Windows) a farba sa pouzije len ako pozadie.
    pub fn for_bar(self, custom: Option<Color>, alpha: u8) -> Theme {
        match custom {
            Some(bg) if alpha >= 128 => self.on_background(bg),
            Some(bg) => Theme {
                background: bg,
                ..self
            },
            None => self,
        }
    }
}

/// Farba textu lisaty zvolena pouzivatelom (`Auto` = podla temy a pozadia, pozri [`Theme::for_bar`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarText {
    Auto,
    Light,
    Dark,
    Custom(Color),
}

impl Theme {
    /// Tema s pevnou farbou textu nad `self.background`. Odvodene farby (tlmeny text, zakazane ikony, nizka bateria,
    /// akcent) sa pocitaju z noveho textu a pozadia: tlmeny text a nizka bateria maju aspon 4,5 : 1 (alebo su rovnake ako
    /// text), akcent 3 : 1. Samotny text sa **neopravuje** - zly kontrast vlastnej farby len oznami nastavenie.
    pub fn with_text(self, text: BarText) -> Theme {
        let fg = match text {
            BarText::Auto => return self,
            BarText::Light => Theme::dark().foreground,
            BarText::Dark => Theme::light().foreground,
            BarText::Custom(c) => c,
        };
        let bg = self.background;
        // svetly text = tmava tema (zakladne farby stavov z nej), tmavy text = svetla
        let light_text = crate::audit::ratio(fg, Color::rgb(0, 0, 0))
            >= crate::audit::ratio(fg, Color::rgb(255, 255, 255));
        let mut t = if light_text {
            Theme::dark()
        } else {
            Theme::light()
        };
        t.background = bg;
        t.foreground = fg;
        t.muted = toward(crate::scene::mix(fg, bg, 0.65), fg, bg, 4.5);
        t.disabled = crate::scene::mix(fg, bg, 0.4);
        t.placeholder = crate::scene::mix(fg, bg, 0.25);
        t.low = toward(t.low, fg, bg, 4.5);
        t.good = toward(t.good, fg, bg, 3.0);
        t.accent = toward(t.accent, fg, bg, 3.0);
        t
    }

    /// Stlmeny text a ikony (ochrana OLED): jas textu sa znizi priblizne na 75 %, ale kontrast textu a tlmeneho textu voci
    /// pozadiu nikdy neklesne pod 4,5 : 1 (ak bol uz predtym nizsi, ostane povodny).
    pub fn dimmed(self) -> Theme {
        let bg = self.background;
        let dim = |c: Color, need: f32| toward(crate::scene::mix(c, bg, 0.75), c, bg, need);
        Theme {
            foreground: dim(self.foreground, 4.5),
            muted: dim(self.muted, 4.5),
            low: dim(self.low, 4.5),
            good: dim(self.good, 3.0),
            accent: dim(self.accent, 3.0),
            disabled: crate::scene::mix(self.disabled, bg, 0.75),
            ..self
        }
    }
}

/// Farba `c` posunuta k `target`, kym nema voci `bg` aspon `need` (najviac az na `target`).
fn toward(c: Color, target: Color, bg: Color, need: f32) -> Color {
    for k in 0..=10 {
        let m = crate::scene::mix(target, c, k as f32 / 10.0);
        if crate::audit::ratio(m, bg) >= need {
            return m;
        }
    }
    target
}

const fn c(v: Rgba) -> Color {
    Color::rgb(v.r, v.g, v.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_text_colour_is_kept_and_its_derived_colours_stay_readable() {
        use crate::audit::ratio;
        let black = Color::rgb(0, 0, 0);
        for base in [Theme::dark(), Theme::light()] {
            for bg in [
                black,
                Color::rgb(0x20, 0x40, 0x80),
                Color::rgb(0xF0, 0xF0, 0xF0),
            ] {
                let on = base.for_bar(Some(bg), 255);
                assert_eq!(on.with_text(BarText::Auto), on, "auto = bez zmeny");
                for text in [
                    BarText::Light,
                    BarText::Dark,
                    BarText::Custom(Color::rgb(0xFF, 0xCC, 0x00)),
                ] {
                    let t = on.with_text(text);
                    assert_eq!(t.background, bg);
                    let fg_ratio = ratio(t.foreground, bg);
                    for (name, c, need) in [
                        ("muted", t.muted, 4.5_f32),
                        ("low", t.low, 4.5),
                        ("accent", t.accent, 3.0),
                    ] {
                        assert!(
                            ratio(c, bg) >= need.min(fg_ratio) - 0.01,
                            "{name} {c:?} na {bg:?} pri texte {:?}",
                            t.foreground
                        );
                    }
                }
            }
        }
        let t = Theme::dark()
            .for_bar(Some(black), 255)
            .with_text(BarText::Custom(Color::rgb(1, 2, 3)));
        assert_eq!(
            t.foreground,
            Color::rgb(1, 2, 3),
            "zla vlastna farba sa neopravuje"
        );
    }

    #[test]
    fn dimming_lowers_text_brightness_but_never_below_4_5_to_1() {
        use crate::audit::ratio;
        let black = Color::rgb(0, 0, 0);
        for t in [
            Theme::dark().for_bar(Some(black), 255),
            Theme::light(),
            Theme::dark(),
            Theme::dark()
                .for_bar(Some(black), 255)
                .with_text(BarText::Custom(Color::rgb(0x60, 0x60, 0x60))),
        ] {
            let d = t.dimmed();
            let bg = t.background;
            assert_eq!(d.background, bg);
            let before = ratio(t.foreground, bg);
            let after = ratio(d.foreground, bg);
            assert!(after <= before + 0.01, "stlmenie nezvysi kontrast");
            assert!(after >= 4.5_f32.min(before) - 0.01, "{after} pod 4,5");
            assert!(ratio(d.muted, bg) >= 4.5_f32.min(ratio(t.muted, bg)) - 0.01);
        }
        // biely text na ciernom sa naozaj stlmi
        let t = Theme::dark()
            .for_bar(Some(black), 255)
            .with_text(BarText::Light);
        assert!(t.dimmed().foreground.r < t.foreground.r);
    }

    #[test]
    fn parse_accepts_known_values_case_insensitively() {
        assert_eq!(ThemeMode::parse("Dark"), Some(ThemeMode::Dark));
        assert_eq!(ThemeMode::parse(" light "), Some(ThemeMode::Light));
        assert_eq!(ThemeMode::parse("SYSTEM"), Some(ThemeMode::System));
        assert_eq!(ThemeMode::parse("auto"), None);
    }

    #[test]
    fn explicit_modes_ignore_system() {
        assert_eq!(ThemeMode::Light.resolve(true), Theme::light());
        assert_eq!(ThemeMode::Dark.resolve(false), Theme::dark());
    }

    #[test]
    fn system_follows_flag() {
        assert_eq!(ThemeMode::System.resolve(true), Theme::dark());
        assert_eq!(ThemeMode::System.resolve(false), Theme::light());
    }

    /// WCAG relativna luminancia a pomer kontrastu.
    fn lum(c: Color) -> f32 {
        let f = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
    }

    fn ratio(a: Color, b: Color) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Kazdy text a kazda ikona musi mat dost kontrastu na plochach, na ktorych sa v UI naozaj kresli:
    /// text >= 4,5 : 1, ovladacie prvky a ikony (akcent, okraje volieb) >= 3 : 1.
    #[test]
    fn text_and_controls_meet_wcag_contrast_in_both_themes() {
        for (name, t) in [("dark", Theme::dark()), ("light", Theme::light())] {
            for (bgn, bg) in [
                ("background", t.background),
                ("placeholder", t.placeholder),
                ("hover", t.hover),
                ("surface", t.surface),
                ("control", t.control),
                ("control_hover", t.control_hover),
                ("control_pressed", t.control_pressed),
                ("selected", t.selected),
                ("pressed", t.pressed),
            ] {
                for (fgn, fg) in [
                    ("foreground", t.foreground),
                    ("muted", t.muted),
                    ("accent_text", t.accent_text),
                ] {
                    let r = ratio(fg, bg);
                    assert!(r >= 4.5, "{name}: {fgn} na {bgn} = {r:.2}");
                }
            }
            for (bgn, bg) in [
                ("background", t.background),
                ("placeholder", t.placeholder),
                ("surface", t.surface),
                ("hover", t.hover),
                ("pressed", t.pressed),
            ] {
                let r = ratio(t.low, bg);
                assert!(r >= 4.5, "{name}: low na {bgn} = {r:.2}");
            }
            let r = ratio(t.on_accent, t.accent);
            assert!(r >= 4.5, "{name}: text na akcente = {r:.2}");
            for (bgn, bg) in [("background", t.background), ("placeholder", t.placeholder)] {
                let r = ratio(t.accent, bg);
                assert!(r >= 3.0, "{name}: akcent na {bgn} = {r:.2}");
            }
            // zakazana ikona je zamerne tlmena, ale stale viditelna (>= 1,6 : 1; WCAG ju z poziadavky vynechava)
            assert!(ratio(t.disabled, t.background) >= 1.6, "{name}: disabled");
            // okraj a hover/pressed sa od pozadia odlisuju
            assert!(ratio(t.edge, t.background) >= 1.1, "{name}: edge");
            assert!(ratio(t.hover, t.background) >= 1.1, "{name}: hover");
            // karta je od pozadia odlisena (jemne) a jej okraj aj riadok pod mysou na nej su viditelne
            assert!(ratio(t.surface, t.background) >= 1.05, "{name}: surface");
            assert!(ratio(t.edge, t.surface) >= 1.2, "{name}: edge na karte");
            assert!(ratio(t.hover, t.surface) >= 1.1, "{name}: hover na karte");
            assert!(
                ratio(t.control_edge, t.surface) >= 1.4,
                "{name}: okraj ovladaca"
            );
            // nebezpecna akcia: text na pozadi, karte a nadychu; biely text na plnej cervenej
            for (bgn, bg) in [
                ("background", t.background),
                ("surface", t.surface),
                ("control", t.control),
                ("danger_tint", t.danger_tint),
            ] {
                let r = ratio(t.danger, bg);
                assert!(r >= 4.5, "{name}: danger na {bgn} = {r:.2}");
            }
            for bg in [t.danger_fill, t.danger_hover] {
                assert!(ratio(t.on_danger, bg) >= 4.5, "{name}: on_danger");
            }
            for bg in [t.accent, t.accent_hover, t.accent_pressed] {
                assert!(ratio(t.on_accent, bg) >= 4.5, "{name}: on_accent");
            }
            // fokus (accent_text) a okraj vybranej volby (accent) su viditelne na kazdej ploche
            for bg in [t.background, t.surface, t.placeholder] {
                assert!(ratio(t.accent_text, bg) >= 3.0, "{name}: fokus");
            }
            assert!(
                ratio(t.accent, t.selected) >= 3.0,
                "{name}: okraj vybranej volby"
            );
        }
    }

    /// Vlastna farba lišty: text ostava podla temy, kym je citatelny; inak sa prepne na opacnu temu. Kazda predvolba
    /// v oboch temach tak ma text aspon 4,5 : 1.
    #[test]
    fn a_custom_bar_color_keeps_the_theme_text_while_readable_and_switches_otherwise() {
        let navy = Color::hex(0x0F1B2D);
        let cream = Color::hex(0xF3E9D2);
        assert_eq!(
            Theme::dark().on_background(navy).foreground,
            Theme::dark().foreground
        );
        assert_eq!(
            Theme::light().on_background(navy).foreground,
            Theme::dark().foreground
        );
        assert_eq!(
            Theme::light().on_background(cream).foreground,
            Theme::light().foreground
        );
        assert_eq!(
            Theme::dark().on_background(cream).foreground,
            Theme::light().foreground
        );
        // cela siva skala: hlavny, tlmeny aj chybovy text su citatelne (alebo aspon taky citatelny ako hlavny text, ak na
        // strednej sivej nie je 4,5 : 1 dosiahnutelne ziadnou farbou textu tem), akcent >= 3 : 1 alebo rovny textu
        for v in (0..=255u16).step_by(5) {
            let g = Color::rgb(v as u8, v as u8, v as u8);
            for t in [Theme::dark(), Theme::light()] {
                let o = t.on_background(g);
                let fg = ratio(o.foreground, g);
                for (n, c) in [("muted", o.muted), ("low", o.low)] {
                    assert!(ratio(c, g) >= 4.5f32.min(fg) - 0.01, "{n} na {g:?}");
                }
                assert!(
                    ratio(o.accent, g) >= 3.0f32.min(fg) - 0.01,
                    "accent na {g:?}"
                );
            }
        }
        // priehladna lista pod 50 %: text podla temy, farba je len pozadie
        let half = Theme::light().for_bar(Some(navy), 100);
        assert_eq!(half.foreground, Theme::light().foreground);
        assert_eq!(half.background, navy);
        assert_eq!(
            Theme::light().for_bar(Some(navy), 255).foreground,
            Theme::dark().foreground
        );
        assert_eq!(Theme::dark().for_bar(None, 255), Theme::dark());
        for bg in [navy, cream, Color::rgb(0, 0, 0), Color::rgb(255, 255, 255)] {
            for t in [Theme::dark(), Theme::light()] {
                let o = t.on_background(bg);
                assert_eq!(o.background, bg);
                assert!(ratio(o.foreground, bg) >= 4.5, "{bg:?}");
                assert!(ratio(o.muted, bg) >= 3.0, "{bg:?} muted");
            }
        }
    }

    #[test]
    fn dark_background_comes_from_the_brand_and_accent_is_system_blue() {
        assert_eq!(Theme::dark().background, Color::hex(0x171B21));
        assert_eq!(Theme::dark().accent, Color::hex(0x0070E0));
        assert_eq!(Theme::light().accent, Color::hex(0x0066D6));
    }

    #[test]
    fn themes_have_distinct_contrasting_colors() {
        for t in [Theme::light(), Theme::dark()] {
            assert_ne!(t.background, t.foreground);
            assert_ne!(t.background, t.hover);
            assert_ne!(t.muted, t.foreground);
            assert_ne!(t.pressed, t.hover);
            assert_ne!(t.disabled, t.foreground);
            assert_ne!(t.placeholder, t.background);
        }
        assert!(Theme::dark().dark && !Theme::light().dark);
    }
}
