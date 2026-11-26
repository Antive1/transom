//! Pocasie (volitelne, predvolene vypnute): mesto zadane v nastaveniach -> suradnice a predpoved z Open-Meteo
//! (open-meteo.com, bez kluca a uctu). Odosiela sa len nazov mesta a z neho zistene suradnice; nic sa nelogue okrem
//! uspechu / chyby. Stahuje sa na vlastnom vlakne kazdych 30 minut. Polozka lisaty ukaze teplotu a stav, vybezok
//! predpoved na 3 dni.
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use transom_i18n::t;
use transom_render::{Font, HAlign, Painter, Rect, TextStyle, Theme};

pub use transom_config::WeatherTarget;

const REFRESH: Duration = Duration::from_secs(30 * 60);
const MAX_BYTES: usize = 256 * 1024;

/// Den predpovede.
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub code: u32,
    pub max: f64,
    pub min: f64,
}

/// Snimka pocasia.
#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    pub place: String,
    pub temp: f64,
    pub code: u32,
    pub days: Vec<Day>,
}

/// Pada dazd / sneh (kod WMO)? `Some(true)` = sneh, `Some(false)` = dazd alebo mrholenie, `None` = nic nepada.
pub fn precip(code: u32) -> Option<bool> {
    match code {
        51..=67 | 80..=82 => Some(false),
        71..=77 | 85 | 86 => Some(true),
        _ => None,
    }
}

/// Ikona polozky na liste podla stavu: slnko, oblak, blesk, alebo oblak s padajucimi kvapkami / vlockami (snimka
/// `phase`).
pub fn glyph(code: u32, phase: u8) -> transom_core::item::Glyph {
    use transom_core::item::Glyph;
    match (code, precip(code)) {
        (_, Some(snow)) => Glyph::Precip { snow, phase },
        (0, _) => Glyph::WeatherClear,
        (95..=99, _) => Glyph::WeatherStorm,
        _ => Glyph::Weather,
    }
}

/// Kluc popisu stavu podla kodu WMO.
pub fn desc_key(code: u32) -> &'static str {
    match code {
        0 => "wx.clear",
        1..=3 => "wx.cloudy",
        45 | 48 => "wx.fog",
        51..=57 => "wx.drizzle",
        61..=67 | 80..=82 => "wx.rain",
        71..=77 | 85 | 86 => "wx.snow",
        95..=99 => "wx.storm",
        _ => "wx.cloudy",
    }
}

/// Znak ikony (Segoe Fluent Icons) podla kodu WMO.
pub fn icon(code: u32) -> char {
    match code {
        0 => '\u{E706}',
        95..=99 => '\u{E945}',
        _ => '\u{E753}',
    }
}

/// Teplota "12°" (zaokruhlena, zaporna so znamienkom minus).
pub fn temp(v: f64) -> String {
    format!("{}°", v.round() as i64).replace('-', "\u{2212}")
}

/// Rozbor odpovede predpovede.
pub fn parse_forecast(json: &str, place: &str) -> Option<Weather> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let cur = v.get("current")?;
    let d = v.get("daily")?;
    let arr = |k: &str| {
        d.get(k)
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
    };
    let (codes, maxs, mins) = (
        arr("weather_code"),
        arr("temperature_2m_max"),
        arr("temperature_2m_min"),
    );
    let days = codes
        .iter()
        .zip(maxs.iter())
        .zip(mins.iter())
        .filter_map(|((c, a), b)| {
            Some(Day {
                code: c.as_u64()? as u32,
                max: a.as_f64()?,
                min: b.as_f64()?,
            })
        })
        .take(3)
        .collect();
    Some(Weather {
        place: place.to_string(),
        temp: cur.get("temperature_2m")?.as_f64()?,
        code: cur.get("weather_code")?.as_u64()? as u32,
        days,
    })
}

fn forecast(lat: f64, lon: f64, place: &str) -> Result<Weather, String> {
    let fc = transom_platform::http::get(
        &format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}&current=temperature_2m,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min&forecast_days=3&timezone=auto"
        ),
        MAX_BYTES,
        15_000,
    )?;
    parse_forecast(&String::from_utf8_lossy(&fc), place)
        .ok_or_else(|| "neplatna odpoved".to_string())
}

/// Stiahne pocasie pre ciel. Miesto so suradnicami = len predpoved; miesto zo starej konfiguracie sa najprv vyhlada
/// podla nazvu; poloha zariadenia sa zisti zo sluzby polohy Windows (nazov miesta ostane prazdny).
fn fetch(target: &WeatherTarget) -> Result<Weather, String> {
    match target {
        WeatherTarget::Place(p) => match (p.lat, p.lon) {
            (Some(lat), Some(lon)) => forecast(lat, lon, &p.name),
            _ => {
                // mesto zo starej konfiguracie: v jazyku aplikacie, potom v anglictine
                let app = transom_i18n::lang()
                    .code()
                    .split('-')
                    .next()
                    .unwrap_or("en");
                let mut found = None;
                for l in [app, "en"] {
                    let geo = transom_platform::http::get(
                        &transom_config::weather::search_url(&p.name, l, 1),
                        MAX_BYTES,
                        15_000,
                    )?;
                    found = transom_config::weather::parse_search(&String::from_utf8_lossy(&geo))
                        .into_iter()
                        .next();
                    if found.is_some() {
                        break;
                    }
                }
                let f = found.ok_or_else(|| "mesto sa nenaslo".to_string())?;
                forecast(f.lat.unwrap_or(0.0), f.lon.unwrap_or(0.0), &p.name)
            }
        },
        WeatherTarget::Device => {
            use transom_platform::geoloc::{position, GeoError};
            let (lat, lon) = position().map_err(|e| match e {
                GeoError::Denied => "poloha zariadenia nie je povolena".to_string(),
                GeoError::Unavailable => "poloha zariadenia nie je dostupna".to_string(),
            })?;
            forecast(lat, lon, "")
        }
    }
}

/// Nazov miesta na zobrazenie (poloha zariadenia = "Moja poloha").
pub fn place_name(w: &Weather) -> String {
    if w.place.is_empty() {
        t("wx.here")
    } else {
        w.place.clone()
    }
}

enum Msg {
    Target(WeatherTarget),
}

/// Sluzba pocasia (vlakno). Zastavenie = `drop`.
pub struct WeatherSync {
    tx: Sender<Msg>,
    pub data: Arc<Mutex<Option<Weather>>>,
    pub target: WeatherTarget,
}

impl WeatherSync {
    pub fn start(
        target: WeatherTarget,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Option<WeatherSync> {
        let (tx, rx) = channel::<Msg>();
        let data: Arc<Mutex<Option<Weather>>> = Arc::new(Mutex::new(None));
        let d2 = Arc::clone(&data);
        let first = target.clone();
        std::thread::Builder::new()
            .name("transom-weather".into())
            .spawn(move || {
                let mut target = first;
                loop {
                    match fetch(&target) {
                        Ok(w) => {
                            crate::log("pocasie: nacitane");
                            if let Ok(mut g) = d2.lock() {
                                *g = Some(w);
                            }
                            notify();
                        }
                        Err(e) => crate::log(&format!("pocasie: chyba ({e})")),
                    }
                    match rx.recv_timeout(REFRESH) {
                        Ok(Msg::Target(c)) => target = c,
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .ok()?;
        Some(WeatherSync { tx, data, target })
    }

    /// Iny ciel (miesto / poloha): stare data sa zahodia a hned sa stiahne nove pocasie.
    pub fn set_target(&mut self, c: WeatherTarget) {
        if c != self.target {
            self.target = c.clone();
            if let Ok(mut g) = self.data.lock() {
                *g = None;
            }
            let _ = self.tx.send(Msg::Target(c));
        }
    }

    pub fn snapshot(&self) -> Option<Weather> {
        self.data.lock().ok().and_then(|g| g.clone())
    }
}

/// Vyska vybezku s predpovedou (lp).
pub const NOTCH_H_LP: i32 = 150;

/// Vybezok s predpovedou: miesto a teplota, stav a tri dni.
pub fn paint(p: &mut dyn Painter, th: &Theme, w: &Weather) {
    paint_with(p, th, w, false);
}

/// Plocha sipky spat (pri otvoreni z Nastrojov), v px.
pub fn back_rect(scale: f32) -> Rect {
    let px = |v: i32| (v as f32 * scale).round() as i32;
    Rect::new(px(8), px(8), px(30), px(28))
}

/// Ako [`paint`], volitelne so sipkou spat pred nazvom miesta.
pub fn paint_with(p: &mut dyn Painter, th: &Theme, w: &Weather, back: bool) {
    let s = p.scale();
    let px = |v: i32| (v as f32 * s).round() as i32;
    let sz = p.size();
    p.fill_rect(Rect::new(0, 0, sz.w, sz.h), th.edge);
    let b = px(1).max(1);
    p.fill_rect(Rect::new(b, 0, sz.w - 2 * b, sz.h - b), th.background);
    let pad = px(16);
    let st = |size: f32, bold: bool, color, align| TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    };
    let full = sz.w - 2 * pad;
    let place = crate::audio_pop::fit(
        p,
        &place_name(w),
        &st(14.0, true, th.foreground, HAlign::Left),
        full - px(80),
    );
    let shift = if back {
        p.draw_text(
            "\u{E76B}",
            back_rect(s),
            &TextStyle {
                font: Font::Icons,
                size_lp: 12.0,
                bold: false,
                color: th.foreground,
                align: HAlign::Center,
            },
        );
        px(26)
    } else {
        0
    };
    p.draw_text(
        &place,
        Rect::new(pad + shift, px(10), full - px(80) - shift, px(24)),
        &st(14.0, true, th.foreground, HAlign::Left),
    );
    p.draw_text(
        &temp(w.temp),
        Rect::new(pad, px(8), full, px(28)),
        &st(20.0, true, th.foreground, HAlign::Right),
    );
    p.draw_text(
        &t(desc_key(w.code)),
        Rect::new(pad, px(34), full, px(18)),
        &st(12.0, false, th.muted, HAlign::Left),
    );
    let names = ["wx.today", "wx.tomorrow", "wx.after"];
    let cw = full / 3;
    for (i, d) in w.days.iter().take(3).enumerate() {
        let x = pad + i as i32 * cw;
        let r = Rect::new(x + px(2), px(62), cw - px(4), px(78));
        p.fill_round_rect(r, px(10), th.control);
        p.draw_text(
            &t(names[i]),
            Rect::new(r.x, r.y + px(4), r.w, px(18)),
            &st(11.0, false, th.muted, HAlign::Center),
        );
        p.draw_text(
            &icon(d.code).to_string(),
            Rect::new(r.x, r.y + px(22), r.w, px(24)),
            &TextStyle {
                font: Font::Icons,
                size_lp: 16.0,
                bold: false,
                color: th.accent_text,
                align: HAlign::Center,
            },
        );
        p.draw_text(
            &format!("{} / {}", temp(d.max), temp(d.min)),
            Rect::new(r.x, r.y + px(50), r.w, px(20)),
            &st(12.0, true, th.foreground, HAlign::Center),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forecast_answers_are_parsed() {
        let fc = r#"{"current":{"temperature_2m":-3.4,"weather_code":71},
            "daily":{"weather_code":[71,3,0],"temperature_2m_max":[1.0,4.2,6.0],"temperature_2m_min":[-5.0,-2.0,0.4]}}"#;
        let w = parse_forecast(fc, "Bratislava").unwrap();
        assert_eq!((w.code, w.days.len()), (71, 3));
        assert_eq!(temp(w.temp), "\u{2212}3°");
        assert_eq!(desc_key(w.code), "wx.snow");
        assert_eq!(parse_forecast("not json", "x"), None);
    }

    #[test]
    fn the_forecast_notch_paints_without_clipped_text() {
        let _g = crate::test_lang::guard();
        let w = Weather {
            place: "Bratislava".into(),
            temp: 12.4,
            code: 2,
            days: vec![
                Day {
                    code: 2,
                    max: 14.0,
                    min: 6.0,
                },
                Day {
                    code: 61,
                    max: 10.0,
                    min: 4.0,
                },
                Day {
                    code: 0,
                    max: 16.0,
                    min: 7.0,
                },
            ],
        };
        let (ww, h) = (transom_platform::dragdrop::NOTCH_W_LP, NOTCH_H_LP);
        let surf = transom_render::offscreen::Surface::new(ww, h);
        let mut g = transom_render::GdiPainter::new(surf.dc, transom_render::Size { w: ww, h }, 96);
        let mut a = transom_render::audit::Audit::new(&mut g, Theme::dark().background);
        paint(&mut a, &Theme::dark(), &w);
        a.finish();
        assert!(a.issues.is_empty(), "{:?}", a.issues);
    }

    /// Skutocne stiahnutie predpovede (spustit rucne s `--ignored`).
    #[test]
    #[ignore = "siet"]
    fn live_forecast_for_saved_and_legacy_places() {
        let p = transom_config::WeatherPlace {
            name: "Wien".into(),
            region: String::new(),
            lat: Some(48.21),
            lon: Some(16.37),
        };
        let w = fetch(&WeatherTarget::Place(p.clone())).unwrap();
        assert_eq!((w.place.as_str(), w.days.len()), ("Wien", 3));
        assert!(w.temp > -60.0 && w.temp < 60.0);
        // stare mesto bez suradnic: najde sa podla nazvu (tu jednoznacny nazov)
        let legacy = transom_config::WeatherPlace {
            name: "Bratislava".into(),
            lat: None,
            lon: None,
            ..p
        };
        assert_eq!(fetch(&WeatherTarget::Place(legacy)).unwrap().days.len(), 3);
    }
}
