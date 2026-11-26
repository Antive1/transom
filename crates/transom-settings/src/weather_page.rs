//! Karta "Pocasie" na stranke Doplnky: vyhladavanie miesta pocas pisania (nazov cez Open-Meteo, PSC cez
//! Zippopotam.us), zoznam ulozenych miest a volba, ktore sa ukazuje v liste (alebo poloha zariadenia).
//! Hladanie bezi na vlastnom vlakne (okno dostane `WM_APP_WX`); stav je v `Ui`, takze testy a snimky obrazovky ho
//! nastavia bez siete. Hladany text ani miesta sa nelogguju.
use super::*;
use transom_config::weather::{self as wx, WeatherPlace, MAX_PLACES};
use transom_config::WeatherSource;

/// Najmenej znakov na hladanie.
pub(crate) const MIN_QUERY: usize = 2;
/// Najviac vysledkov.
pub(crate) const MAX_RESULTS: usize = 6;

/// Stav hladania.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum WxState {
    #[default]
    Idle,
    Searching,
    Done,
    Failed,
}

/// Stav povolenia polohy zariadenia.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum GeoState {
    #[default]
    Unknown,
    Checking,
    Allowed,
    Denied,
    Unavailable,
}

/// Stav karty pocasia.
#[derive(Default)]
pub(crate) struct WxUi {
    pub query: String,
    /// Cislo posledneho dotazu (starsie odpovede sa zahodia).
    pub gen: u64,
    pub state: WxState,
    pub results: Vec<WeatherPlace>,
    pub geo: GeoState,
    /// Overit povolenie polohy po kliknuti (okno spusti vlakno).
    pub geo_check: bool,
    /// Vymazat text v poli (po pridani miesta).
    pub clear_field: bool,
    pub full: bool,
    /// Text sa zmenil: okno ma (znova) spustit odlozene hladanie.
    pub restart: bool,
}

/// Hladanie (sietove, na pracovnom vlakne): PSC v krajine regionu / z predpony a nazov v jazyku aplikacie aj
/// v anglictine. `Err` len ked zlyhali vsetky poziadavky.
pub(crate) fn search(query: &str, lang: &str, country: &str) -> Result<Vec<WeatherPlace>, ()> {
    const MAX: usize = 256 * 1024;
    let get = |url: &str| transom_platform::http::get(url, MAX, 10_000);
    let mut lists = Vec::new();
    let mut ok = false;
    if let Some((cc, code)) = wx::postal_query(query, country) {
        // nezname PSC vrati 404 = ziadny vysledok, nie chyba
        ok = true;
        if let Ok(b) = get(&wx::postal_url(&cc, &code)) {
            lists.push(wx::parse_postal(&String::from_utf8_lossy(&b)));
        }
    }
    let mut langs = vec![lang.to_string()];
    if lang != "en" {
        langs.push("en".into());
    }
    for l in langs {
        if let Ok(b) = get(&wx::search_url(query, &l, MAX_RESULTS as u32)) {
            ok = true;
            lists.push(wx::parse_search(&String::from_utf8_lossy(&b)));
        }
    }
    if ok {
        Ok(wx::merge(lists, MAX_RESULTS))
    } else {
        Err(())
    }
}

impl Ui {
    /// Text v poli hladania sa zmenil. Vrati `true`, ked sa ma (znova) spustit odlozene hladanie.
    pub(crate) fn wx_query_changed(&mut self, text: &str) -> bool {
        let q = text.trim().to_string();
        if q == self.wx.query {
            return false;
        }
        self.wx.query = q;
        self.wx.gen += 1;
        self.wx.full = false;
        if self.wx.query.chars().count() < MIN_QUERY {
            self.wx.state = WxState::Idle;
            self.wx.results.clear();
            return false;
        }
        self.wx.state = WxState::Searching;
        true
    }

    /// Prisli vysledky hladania (starsie cislo dotazu sa ignoruje).
    pub(crate) fn wx_results_arrived(&mut self, gen: u64, r: Result<Vec<WeatherPlace>, ()>) {
        if gen != self.wx.gen || self.wx.state != WxState::Searching {
            return;
        }
        match r {
            Ok(v) => {
                self.wx.results = v;
                self.wx.state = WxState::Done;
            }
            Err(()) => {
                self.wx.results.clear();
                self.wx.state = WxState::Failed;
            }
        }
    }

    /// Prida vysledok `i` do miest a zvoli ho pre listu. Vrati, ci sa konfiguracia zmenila.
    pub(crate) fn wx_pick(&mut self, i: usize) -> bool {
        let Some(p) = self.wx.results.get(i).cloned() else {
            return false;
        };
        let places = &mut self.cfg.weather_places;
        let idx = match places.iter().position(|o| o.same(&p)) {
            Some(k) => k,
            None if places.len() >= MAX_PLACES => {
                self.wx.full = true;
                return false;
            }
            None => {
                places.push(p);
                places.len() - 1
            }
        };
        self.cfg.weather_source = WeatherSource::Place(idx);
        self.wx.query.clear();
        self.wx.results.clear();
        self.wx.state = WxState::Idle;
        self.wx.gen += 1;
        self.wx.full = false;
        self.wx.clear_field = true;
        true
    }

    /// Odstrani miesto `i` (zvoleny zdroj sa posunie, aby ukazoval na to iste miesto).
    pub(crate) fn wx_remove(&mut self, i: usize) -> bool {
        if i >= self.cfg.weather_places.len() {
            return false;
        }
        self.cfg.weather_places.remove(i);
        self.cfg.weather_source = match self.cfg.weather_source {
            WeatherSource::Place(k) if k == i => WeatherSource::None,
            WeatherSource::Place(k) if k > i => WeatherSource::Place(k - 1),
            s => s,
        };
        self.wx.full = false;
        true
    }

    /// Zvoli zdroj pre listu (poloha zariadenia najprv overi povolenie).
    pub(crate) fn wx_select(&mut self, src: WeatherSource) -> bool {
        if src == WeatherSource::Device && self.wx.geo != GeoState::Allowed {
            self.wx.geo = GeoState::Checking;
            self.wx.geo_check = true;
        }
        if self.cfg.weather_source == src {
            return false;
        }
        self.cfg.weather_source = src;
        true
    }

    /// Prepinac (kruzok) s popisom; vrati y pod riadkom.
    fn wx_radio(&mut self, p: &mut dyn Painter, r: Rect, label: &str, on: bool, hit: Hit) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        if self.hover.as_ref() == Some(&hit) {
            p.fill_round_rect(r, px(design::radius::CONTROL), th.hover);
        }
        let d = px(18);
        let c = Rect::new(r.x + px(5), r.y + (r.h - d) / 2, d, d);
        p.fill_round_rect(c, d / 2, if on { th.accent } else { th.control_edge });
        let b = px(2).max(1);
        let inner = Rect::new(c.x + b, c.y + b, d - 2 * b, d - 2 * b);
        p.fill_round_rect(inner, inner.w / 2, if on { th.accent } else { th.surface });
        if on {
            let k = px(7);
            p.fill_round_rect(
                Rect::new(c.x + (d - k) / 2, c.y + (d - k) / 2, k, k),
                k / 2,
                th.on_accent,
            );
        }
        let st = Ty::Body.style(th.foreground, HAlign::Left);
        let tw = r.w - px(38);
        let text = fit_text(p, label, &st, tw);
        p.draw_text(&text, Rect::new(r.x + px(34), r.y, tw, r.h), &st);
        self.tag(&hit, label.to_string(), Some(on));
        self.hits.push((r, hit));
    }

    /// Male tlacidlo s ikonou (odstranit / pridat).
    fn wx_icon_button(
        &mut self,
        p: &mut dyn Painter,
        r: Rect,
        glyph: char,
        name: String,
        hit: Hit,
    ) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let bg = if self.hover.as_ref() == Some(&hit) {
            th.control_hover
        } else {
            th.control
        };
        p.fill_round_rect(r, px(design::radius::CONTROL), bg);
        p.draw_text(
            &glyph.to_string(),
            r,
            &TextStyle {
                font: Font::Icons,
                size_lp: 12.0,
                bold: false,
                color: th.foreground,
                align: HAlign::Center,
            },
        );
        self.tag(&hit, name, None);
        self.hits.push((r, hit));
    }

    /// Karta pocasia. Vrati y pod nou.
    pub(crate) fn paint_weather_card(
        &mut self,
        p: &mut dyn Painter,
        x0: i32,
        y: i32,
        w: i32,
    ) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("item.weather"));
        y = self.notes(p, x0, y, w, &["ex.wx_help"]) + px(4);

        // pole hladania (nativne pole lezi presne na tomto mieste; pod nim popis pre snimky bez okna)
        let fw = w.min(px(380));
        let field = Rect::new(x0, y, fw, px(28));
        p.fill_round_rect(field, px(design::radius::CONTROL), th.control_edge);
        let b = px(1).max(1);
        p.fill_round_rect(
            Rect::new(field.x + b, field.y + b, field.w - 2 * b, field.h - 2 * b),
            px(design::radius::CONTROL),
            th.placeholder,
        );
        let ph = Ty::Body.style(th.muted, HAlign::Left);
        let shown = if self.wx.query.is_empty() {
            t("ex.wx_search")
        } else {
            self.wx.query.clone()
        };
        // s natívnym polom kresli text obraz pola (`paint_field_images`); bez neho (snimky) tu
        if self.edits[5] == 0 {
            let text = fit_text(p, &shown, &ph, field.w - px(16));
            p.draw_text(
                &text,
                Rect::new(field.x + px(8), field.y, field.w - px(16), field.h),
                &ph,
            );
        }
        self.wx_rect = Rect::new(field.x + b, field.y + b, field.w - 2 * b, field.h - 2 * b);
        self.wx_scroll = self.scroll;
        y += field.h + px(6);

        let status = match self.wx.state {
            WxState::Searching => Some(t("ex.wx_searching")),
            WxState::Done if self.wx.results.is_empty() => Some(t("ex.wx_nothing")),
            WxState::Failed => Some(t("ex.wx_neterr")),
            _ => None,
        };
        if let Some(m) = status {
            y = self.paragraph(p, x0, y, w, &m, 12.0, th.muted);
        }
        if self.wx.full {
            y = self.paragraph(p, x0, y, w, &t("ex.wx_full"), 12.0, th.foreground);
        }
        let results = self.wx.results.clone();
        for (i, r) in results.iter().enumerate() {
            let row = Rect::new(x0, y, w, px(30));
            let hit = Hit::WeatherPick(i);
            if self.hover.as_ref() == Some(&hit) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            p.draw_text(
                "\u{E710}",
                Rect::new(row.x + px(6), row.y, px(20), row.h),
                &TextStyle {
                    font: Font::Icons,
                    size_lp: 12.0,
                    bold: false,
                    color: th.accent_text,
                    align: HAlign::Center,
                },
            );
            let st = Ty::Body.style(th.foreground, HAlign::Left);
            let tw = row.w - px(40);
            let label = fit_text(p, &r.label(), &st, tw);
            p.draw_text(&label, Rect::new(row.x + px(34), row.y, tw, row.h), &st);
            self.tag(&hit, tf("ex.wx_add", &[&r.label()]), None);
            self.hits.push((row, hit));
            y += row.h + px(2);
        }
        y += px(8);

        // ulozene miesta: jedno sa ukazuje v liste
        y = self.section(p, x0, y, w, &t("ex.wx_places"));
        let src = self.cfg.weather_source;
        self.acc_group = Some(t("ex.wx_places"));
        self.wx_radio(
            p,
            Rect::new(x0, y, w, px(30)),
            &t("ex.wx_off"),
            src == WeatherSource::None,
            Hit::WeatherUse(None),
        );
        y += px(32);
        self.wx_radio(
            p,
            Rect::new(x0, y, w, px(30)),
            &t("ex.wx_device"),
            src == WeatherSource::Device,
            Hit::WeatherDevice,
        );
        y += px(32);
        let geo_msg = match self.wx.geo {
            GeoState::Checking => Some("ex.wx_checking"),
            GeoState::Denied => Some("ex.wx_denied"),
            GeoState::Unavailable => Some("ex.wx_geo_na"),
            _ => None,
        };
        y = self.notes(p, x0 + px(34), y, w - px(34), &["ex.wx_device_note"]);
        if let Some(k) = geo_msg {
            y = self.paragraph(p, x0 + px(34), y, w - px(34), &t(k), 12.0, th.foreground);
            if self.wx.geo == GeoState::Denied {
                y = self.flow_buttons(
                    p,
                    x0 + px(34),
                    y,
                    w - px(34),
                    vec![(t("ex.wx_geo_settings"), Hit::WeatherGeoSettings, false)],
                );
            }
        }
        y += px(4);
        let places = self.cfg.weather_places.clone();
        for (i, pl) in places.iter().enumerate() {
            let rb = px(30);
            self.wx_radio(
                p,
                Rect::new(x0, y, w - rb - px(8), px(30)),
                &pl.label(),
                src == WeatherSource::Place(i),
                Hit::WeatherUse(Some(i)),
            );
            self.wx_icon_button(
                p,
                Rect::new(x0 + w - rb, y, rb, px(30)),
                '\u{E711}',
                tf("ex.wx_remove", &[&pl.label()]),
                Hit::WeatherRemove(i),
            );
            y += px(32);
        }
        self.acc_group = None;
        if places.is_empty() {
            y = self.notes(p, x0, y, w, &["ex.wx_none"]);
        }
        y = self.notes(p, x0, y + px(4), w, &["ex.wx_privacy"]);
        self.card_end(s, x0, w, y)
    }
}

/// Ukazkovy stav karty pre snimky (`--shot`, stranka `extras-weather`); inak prazdny stav.
pub(crate) fn fixture(ui: &mut Ui, on: bool) {
    let pl = |n: &str, r: &str, lat: f64| WeatherPlace {
        name: n.into(),
        region: r.into(),
        lat: Some(lat),
        lon: Some(17.1),
    };
    if on {
        ui.wx = WxUi {
            query: "Nové Mesto".into(),
            state: WxState::Done,
            results: vec![
                pl("Nové Mesto nad Váhom", "Trenčiansky kraj, Slovensko", 48.75),
                pl(
                    "Nové Mesto na Moravě",
                    "Kraj Vysočina, Česká republika",
                    49.56,
                ),
                pl(
                    "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch",
                    "Wales, United Kingdom of Great Britain and Northern Ireland",
                    53.2,
                ),
            ],
            geo: GeoState::Denied,
            full: true,
            ..Default::default()
        };
        ui.cfg.weather_places = vec![
            pl("Bratislava", "Bratislavský kraj, Slovensko", 48.15),
            pl("Wien", "1010, Austria", 48.21),
        ];
        ui.cfg.weather_source = WeatherSource::Place(1);
    } else {
        ui.wx = WxUi::default();
        ui.cfg.weather_places.clear();
        ui.cfg.weather_source = WeatherSource::None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_tests::Env;

    fn place(n: &str, lat: f64) -> WeatherPlace {
        WeatherPlace {
            name: n.into(),
            region: "Kraj, Krajina".into(),
            lat: Some(lat),
            lon: Some(17.0),
        }
    }

    #[test]
    fn typing_searches_only_from_two_characters_and_stale_answers_are_dropped() {
        let e = Env::new("wx-search");
        let mut ui = e.ui();
        assert!(!ui.wx_query_changed("B"));
        assert_eq!(ui.wx.state, WxState::Idle);
        assert!(ui.wx_query_changed("Br"));
        let old = ui.wx.gen;
        assert!(ui.wx_query_changed("Bra"));
        ui.wx_results_arrived(old, Ok(vec![place("Stare", 1.0)]));
        assert!(
            ui.wx.results.is_empty(),
            "odpoved na starsi dotaz sa zahodi"
        );
        ui.wx_results_arrived(ui.wx.gen, Ok(vec![place("Bratislava", 48.1)]));
        assert_eq!(ui.wx.results.len(), 1);
        assert!(!ui.wx_query_changed("Bra "), "rovnaky dotaz nehlada znova");
        assert!(ui.wx_query_changed("Brx"));
        ui.wx_results_arrived(ui.wx.gen, Err(()));
        assert_eq!(ui.wx.state, WxState::Failed);
    }

    #[test]
    fn picking_adds_once_selects_and_removing_keeps_the_selection_on_the_same_place() {
        let e = Env::new("wx-places");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Extras));
        for (k, n) in ["A", "B", "C"].iter().enumerate() {
            ui.wx_query_changed(&format!("{n}{n}"));
            ui.wx_results_arrived(ui.wx.gen, Ok(vec![place(n, k as f64)]));
            ui.click(Hit::WeatherPick(0));
        }
        let c = e.saved();
        assert_eq!(c.weather_places.len(), 3);
        assert_eq!(c.weather_source, WeatherSource::Place(2));
        assert!(ui.wx.clear_field && ui.wx.results.is_empty());
        // to iste miesto znova = len vyber, bez duplikatu
        ui.wx_query_changed("AA");
        ui.wx_results_arrived(ui.wx.gen, Ok(vec![place("A", 0.0)]));
        ui.click(Hit::WeatherPick(0));
        assert_eq!(e.saved().weather_places.len(), 3);
        assert_eq!(e.saved().weather_source, WeatherSource::Place(0));
        // zvolit C, odstranit A: vyber ostava na C
        ui.click(Hit::WeatherUse(Some(2)));
        ui.click(Hit::WeatherRemove(0));
        let c = e.saved();
        assert_eq!(c.weather_places[c.weather_places.len() - 1].name, "C");
        assert_eq!(c.weather_source, WeatherSource::Place(1));
        // odstranit zvolene = pocasie vypnute
        ui.click(Hit::WeatherRemove(1));
        assert_eq!(e.saved().weather_source, WeatherSource::None);
        assert_eq!(e.saved().weather_target(), None);
    }

    #[test]
    fn the_list_is_capped_and_device_location_asks_for_permission_first() {
        let e = Env::new("wx-cap");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Extras));
        for k in 0..MAX_PLACES + 1 {
            ui.wx_query_changed(&format!("q{k}"));
            ui.wx_results_arrived(ui.wx.gen, Ok(vec![place(&format!("P{k}"), k as f64)]));
            ui.click(Hit::WeatherPick(0));
        }
        assert_eq!(e.saved().weather_places.len(), MAX_PLACES);
        assert!(ui.wx.full, "plny zoznam ohlasi spravu");
        ui.click(Hit::WeatherDevice);
        assert_eq!(ui.wx.geo, GeoState::Checking);
        assert!(ui.wx.geo_check);
        assert_eq!(e.saved().weather_source, WeatherSource::Device);
    }

    /// Skutocne hladanie cez siet (spustit rucne: `cargo test -p transom-settings live_search -- --ignored`).
    #[test]
    #[ignore = "siet"]
    fn live_search_finds_names_in_two_languages_and_postal_codes() {
        let vienna = |r: &[WeatherPlace]| {
            r.iter().any(|p| {
                (p.lat.unwrap_or(0.0) - 48.2).abs() < 0.1
                    && (p.lon.unwrap_or(0.0) - 16.37).abs() < 0.1
            })
        };
        // slovensky nazov v slovencine, anglicky v anglictine (zaloha)
        assert!(vienna(&search("Viede", "sk", "SK").unwrap()));
        assert!(vienna(&search("Vienna", "sk", "SK").unwrap()));
        // PSC: slovenske s krajinou regionu, rakuske s predponou
        let r = search("949 01", "sk", "SK").unwrap();
        assert!(r.iter().any(|p| p.region.starts_with("949 01")), "{r:?}");
        assert!(vienna(&search("AT 1010", "sk", "SK").unwrap()));
    }
}
