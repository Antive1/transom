//! Miesta pre pocasie: ulozene miesta (nazov, oblast, suradnice), zvoleny zdroj pre listu (miesto alebo poloha
//! zariadenia) a spolocny rozbor odpovedi vyhladavania (Open-Meteo podla nazvu, Zippopotam.us podla PSC).
//! Sietove volania tu nie su; robi ich aplikacia a nastavenia.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Najviac ulozenych miest.
pub const MAX_PLACES: usize = 8;

/// Ulozene miesto. `lat` / `lon` chybaju len pri mieste prenesenom zo starej konfiguracie (len nazov mesta):
/// aplikacia ho vtedy vyhlada podla nazvu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WeatherPlace {
    pub name: String,
    /// Oblast a krajina ("Bratislavský, Slovensko"); moze byt prazdna.
    pub region: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

impl WeatherPlace {
    /// Platne miesto: neprazdny rozumne dlhy nazov bez riadiacich znakov a suradnice v rozsahu (alebo ziadne).
    pub fn valid(&self) -> bool {
        let text_ok =
            |s: &str, max: usize| s.chars().count() <= max && !s.chars().any(char::is_control);
        let coords_ok = match (self.lat, self.lon) {
            (Some(a), Some(o)) => {
                a.is_finite()
                    && o.is_finite()
                    && (-90.0..=90.0).contains(&a)
                    && (-180.0..=180.0).contains(&o)
            }
            (None, None) => true,
            _ => false,
        };
        !self.name.trim().is_empty()
            && text_ok(&self.name, 80)
            && text_ok(&self.region, 120)
            && coords_ok
    }

    /// Popis pre zoznam: "Nazov, oblast" (bez oblasti len nazov).
    pub fn label(&self) -> String {
        if self.region.is_empty() {
            self.name.clone()
        } else {
            format!("{}, {}", self.name, self.region)
        }
    }

    /// To iste miesto (rovnaky nazov a suradnice do ~1 km): pridanie ho nezdvoji.
    pub fn same(&self, o: &WeatherPlace) -> bool {
        let near = match (self.lat, self.lon, o.lat, o.lon) {
            (Some(a), Some(b), Some(c), Some(d)) => (a - c).abs() < 0.01 && (b - d).abs() < 0.01,
            (None, None, None, None) => true,
            _ => false,
        };
        self.name.eq_ignore_ascii_case(&o.name) && near
    }
}

/// Zdroj pocasia v liste: ulozene miesto (index) alebo poloha zariadenia.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WeatherSource {
    #[default]
    None,
    Place(usize),
    Device,
}

impl WeatherSource {
    pub fn code(self) -> String {
        match self {
            WeatherSource::None => String::new(),
            WeatherSource::Place(i) => i.to_string(),
            WeatherSource::Device => "device".into(),
        }
    }

    pub fn parse(s: &str) -> WeatherSource {
        match s.trim() {
            "device" => WeatherSource::Device,
            v => v
                .parse::<usize>()
                .ok()
                .filter(|i| *i < MAX_PLACES)
                .map_or(WeatherSource::None, WeatherSource::Place),
        }
    }
}

impl Serialize for WeatherSource {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.code())
    }
}

impl<'de> Deserialize<'de> for WeatherSource {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(s) => WeatherSource::parse(&s),
            Value::Number(n) => n.as_u64().map_or(WeatherSource::None, |i| {
                WeatherSource::parse(&i.to_string())
            }),
            _ => WeatherSource::None,
        })
    }
}

/// Ciel stahovania pre aplikaciu.
#[derive(Clone, Debug, PartialEq)]
pub enum WeatherTarget {
    Place(WeatherPlace),
    Device,
}

/// Percentove kodovanie hodnoty do adresy (medzera = "+").
pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Adresa vyhladavania podla nazvu (Open-Meteo); `lang` = kod jazyka vysledkov ("sk", "en").
pub fn search_url(query: &str, lang: &str, count: u32) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count={count}&language={}&format=json",
        enc(query.trim()),
        enc(lang)
    )
}

/// Rozbor vysledkov Open-Meteo (len platne miesta, s oblastou "kraj, krajina").
pub fn parse_search(json: &str) -> Vec<WeatherPlace> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.get("results").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|r| {
            let s = |k: &str| {
                r.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            let region: Vec<String> = [s("admin1"), s("country")]
                .into_iter()
                .filter(|x| !x.is_empty())
                .fold(Vec::new(), |mut acc, x| {
                    if !acc.contains(&x) && x != s("name") {
                        acc.push(x);
                    }
                    acc
                });
            let p = WeatherPlace {
                name: s("name"),
                region: region.join(", "),
                lat: r.get("latitude").and_then(Value::as_f64),
                lon: r.get("longitude").and_then(Value::as_f64),
            };
            (p.valid() && p.lat.is_some()).then_some(p)
        })
        .collect()
}

/// Dotaz vyzera ako PSC: (krajina, PSC v tvare sluzby). Krajina z predpony ("AT 1010", "SK-949 01") alebo `default_cc`
/// (region Windows). Slovenske a ceske PSC sa upravia na "NNN NN".
pub fn postal_query(q: &str, default_cc: &str) -> Option<(String, String)> {
    let q = q.trim();
    let (cc, rest) = match q.get(..2) {
        Some(p) if p.chars().all(|c| c.is_ascii_alphabetic()) => (
            p.to_ascii_uppercase(),
            q[2..].trim_start_matches(['-', ' ']).trim(),
        ),
        _ => (default_cc.trim().to_ascii_uppercase(), q),
    };
    if cc.len() != 2 || !cc.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let digits: String = rest.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    if digits.len() < 3 || digits.len() > 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !rest
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-'))
    {
        return None;
    }
    let code = match cc.as_str() {
        "SK" | "CZ" if digits.len() == 5 => format!("{} {}", &digits[..3], &digits[3..]),
        "PL" if digits.len() == 5 => format!("{}-{}", &digits[..2], &digits[2..]),
        _ => digits,
    };
    Some((cc, code))
}

/// Adresa vyhladavania PSC (Zippopotam.us, bez uctu).
pub fn postal_url(cc: &str, code: &str) -> String {
    format!(
        "https://api.zippopotam.us/{}/{}",
        enc(&cc.to_ascii_lowercase()),
        enc(code).replace('+', "%20")
    )
}

/// Rozbor odpovede Zippopotam.us: obce s danym PSC (oblast = "PSC, krajina").
pub fn parse_postal(json: &str) -> Vec<WeatherPlace> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let code = v
        .get("post code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let country = v
        .get("country")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let Some(arr) = v.get("places").and_then(Value::as_array) else {
        return Vec::new();
    };
    let num = |r: &Value, k: &str| {
        r.get(k).and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
        })
    };
    arr.iter()
        .filter_map(|r| {
            let p = WeatherPlace {
                name: r
                    .get("place name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                region: [code.as_str(), country.as_str()]
                    .iter()
                    .filter(|x| !x.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", "),
                lat: num(r, "latitude"),
                lon: num(r, "longitude"),
            };
            (p.valid() && p.lat.is_some()).then_some(p)
        })
        .collect()
}

/// Zluci vysledky (bez duplicit, najviac `max`).
pub fn merge(lists: Vec<Vec<WeatherPlace>>, max: usize) -> Vec<WeatherPlace> {
    let mut out: Vec<WeatherPlace> = Vec::new();
    for p in lists.into_iter().flatten() {
        if out.len() < max && !out.iter().any(|o| o.same(&p)) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // skutocne odpovede sluzieb (skratene)
    const OM: &str = r#"{"results":[
        {"id":2950159,"name":"Berlín","latitude":52.52437,"longitude":13.41053,"country_code":"DE","admin1":"Berlín","country":"Nemecko","postcodes":["10967"]},
        {"id":3161732,"name":"Bergen","latitude":60.39299,"longitude":5.32415,"admin1":"Vestland","country":"Nórsko"},
        {"id":1,"name":"","latitude":1.0,"longitude":1.0},
        {"id":2,"name":"Bez suradnic"}
    ],"generationtime_ms":0.5}"#;
    const ZIP: &str = r#"{"country": "Austria", "country abbreviation": "AT", "post code": "1010", "places": [{"place name": "Wien", "longitude": "16.3705", "latitude": "48.2077", "state": "", "state abbreviation": ""}]}"#;

    #[test]
    fn open_meteo_results_become_places_with_region() {
        let r = parse_search(OM);
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].label(),
            "Berlín, Nemecko",
            "kraj rovnaky ako nazov sa neopakuje"
        );
        assert_eq!(r[1].label(), "Bergen, Vestland, Nórsko");
        assert!(parse_search("{}").is_empty() && parse_search("nie json").is_empty());
    }

    #[test]
    fn postal_codes_are_recognised_with_country_from_prefix_or_region() {
        assert_eq!(
            postal_query("949 01", "SK"),
            Some(("SK".into(), "949 01".into()))
        );
        assert_eq!(
            postal_query("94901", "sk"),
            Some(("SK".into(), "949 01".into()))
        );
        assert_eq!(
            postal_query("AT 1010", "SK"),
            Some(("AT".into(), "1010".into()))
        );
        assert_eq!(
            postal_query("de-10115", "SK"),
            Some(("DE".into(), "10115".into()))
        );
        assert_eq!(
            postal_query("00001", "PL"),
            Some(("PL".into(), "00-001".into()))
        );
        assert_eq!(postal_query("Bratislava", "SK"), None);
        assert_eq!(postal_query("12", "SK"), None);
        assert_eq!(
            postal_query("949 01", ""),
            None,
            "bez krajiny sa PSC nehlada"
        );
        assert_eq!(
            postal_url("SK", "949 01"),
            "https://api.zippopotam.us/sk/949%2001"
        );
        let z = parse_postal(ZIP);
        assert_eq!(
            (z.len(), z[0].label()),
            (1, "Wien, 1010, Austria".to_string())
        );
        assert_eq!(z[0].lat, Some(48.2077));
        assert!(parse_postal("{}").is_empty());
    }

    #[test]
    fn sources_round_trip_and_bad_values_are_none() {
        for s in [
            WeatherSource::Device,
            WeatherSource::Place(3),
            WeatherSource::None,
        ] {
            assert_eq!(WeatherSource::parse(&s.code()), s);
        }
        assert_eq!(WeatherSource::parse("99"), WeatherSource::None);
        assert_eq!(WeatherSource::parse("x"), WeatherSource::None);
    }

    #[test]
    fn invalid_places_are_rejected_and_duplicates_merged() {
        let ok = WeatherPlace {
            name: "A".into(),
            region: String::new(),
            lat: Some(1.0),
            lon: Some(2.0),
        };
        assert!(ok.valid());
        for bad in [
            WeatherPlace {
                lat: Some(f64::NAN),
                ..ok.clone()
            },
            WeatherPlace {
                lat: Some(91.0),
                ..ok.clone()
            },
            WeatherPlace {
                lon: None,
                ..ok.clone()
            },
            WeatherPlace {
                name: "a\nb".into(),
                ..ok.clone()
            },
            WeatherPlace {
                name: " ".into(),
                ..ok.clone()
            },
        ] {
            assert!(!bad.valid(), "{bad:?}");
        }
        let near = WeatherPlace {
            lat: Some(1.001),
            ..ok.clone()
        };
        assert_eq!(
            merge(vec![vec![ok.clone()], vec![near, ok.clone()]], 5).len(),
            1
        );
        assert!(search_url("Nové Zámky", "sk", 6)
            .contains("name=Nov%C3%A9+Z%C3%A1mky&count=6&language=sk"));
    }
}
