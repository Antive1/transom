//! Cennik pre **odhad** ceny ako pri API (USD za 1 milion tokenov). Nie je to faktura: pri predplatnom
//! (Claude Pro/Max, ChatGPT plany) sa za tokeny neplati, a aj pri API rozhoduje faktura.
//!
//! Zdroje a datumy (zobrazuju sa vedla odhadu):
//! - Anthropic: tabulka modelov v Claude API skill (cache platform.claude.com, stav 2026-11-03): Opus 5.5 $4/$20,
//!   Sonnet 5.5 $2/$10, Fable 5.2 $8/$40, Haiku 5.5 $0,10/$0,50 (prompt <= 100K); citanie z cache Opus 5.5 a Fable 5.2 $0,20, Sonnet 5.5 $0,10;
//!   zapis do cache 1,25x (5 min) a 2x (1 h) ceny vstupu. Rychlost `fast` a iny `service_tier` sa neoceňuju.
//! - OpenAI: developers.openai.com/api/docs/pricing, nacitane 2026-11-20 (stranka bez datumu verzie): GPT-6 Luna
//!   Standard, kratky kontext (<= 272K vstupnych tokenov): vstup $0,10, cachovany vstup $0,01, vystup $0,50.
//!   Volanie s dlhsim vstupom sa neoceni (ina tarifa).
use crate::{Event, Provider};

pub const ANTHROPIC_PRICES_DATE: &str = "2026-11-03";
pub const OPENAI_PRICES_DATE: &str = "2026-11-20";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    /// Maximalny vstup jedneho volania, pre ktory cena plati (None = bez limitu).
    pub max_input: Option<u64>,
}

const fn anthropic(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        max_input: None,
    }
}

/// Cena modelu, ak je v tabulke.
pub fn price(provider: Option<Provider>, model: &str) -> Option<Price> {
    let m = model.trim().to_ascii_lowercase();
    match (provider?, m.as_str()) {
        (Provider::ClaudeCode, "claude-opus-5-5") => Some(anthropic(4.0, 20.0, 0.20)),
        (Provider::ClaudeCode, "claude-sonnet-5-5") => Some(anthropic(2.0, 10.0, 0.10)),
        (Provider::ClaudeCode, "claude-fable-5-2") => Some(anthropic(8.0, 40.0, 0.20)),
        // Haiku 5.5: cena plati pre prompt <= 100K tokenov (nad tym $0,50/$2,50 - neocenuje sa);
        // cache podla vseobecneho pravidla (citanie ~0,1x, zapis 1,25x/2x)
        (Provider::ClaudeCode, "claude-haiku-5-5") => Some(Price {
            max_input: Some(100_000),
            ..anthropic(0.10, 0.50, 0.01)
        }),
        (Provider::Codex, "gpt-6-luna") => Some(Price {
            input: 0.10,
            output: 0.50,
            cache_read: 0.01,
            // OpenAI nefakturuje zapis do cache zvlast; tokeny zapisu sa ocenia ako vstup
            cache_write_5m: 0.10,
            cache_write_1h: 0.10,
            max_input: Some(272_000),
        }),
        _ => None,
    }
}

/// Datum cennika pre poskytovatela (zobrazuje sa pri odhade).
pub fn prices_date(p: Option<Provider>) -> &'static str {
    match p {
        Some(Provider::Codex) => OPENAI_PRICES_DATE,
        _ => ANTHROPIC_PRICES_DATE,
    }
}

/// Odhad ceny jedneho volania v USD; `None` = neda sa ocenit (neznamy model, nestandardna rychlost/uroven,
/// vstup nad hranicou tarify). Nikdy nevrati 0 namiesto "nevieme".
pub fn estimate(e: &Event) -> Option<f64> {
    if e.nonstandard || e.over_price_limit {
        return None;
    }
    let p = price(e.provider, &e.model)?;
    let total_in = e.input + e.cache_read + e.cache_write_5m + e.cache_write_1h;
    if p.max_input.is_some_and(|m| total_in > m) {
        return None;
    }
    let per = |n: u64, usd: f64| n as f64 * usd / 1e6;
    Some(
        per(e.input, p.input)
            + per(e.output, p.output)
            + per(e.cache_read, p.cache_read)
            + per(e.cache_write_5m, p.cache_write_5m)
            + per(e.cache_write_1h, p.cache_write_1h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(provider: Provider, model: &str) -> Event {
        Event {
            provider: Some(provider),
            model: model.into(),
            ..Default::default()
        }
    }

    #[test]
    fn claude_cache_tiers_are_priced_separately() {
        let mut x = e(Provider::ClaudeCode, "claude-opus-5-5");
        x.input = 1_000_000;
        x.output = 1_000_000;
        x.cache_read = 1_000_000;
        x.cache_write_5m = 1_000_000;
        x.cache_write_1h = 1_000_000;
        // 4 + 20 + 0,20 + 5 + 8
        assert!((estimate(&x).unwrap() - 37.2).abs() < 1e-9);
        let mut s = e(Provider::ClaudeCode, "claude-sonnet-5-5");
        s.cache_read = 1_000_000;
        assert!((estimate(&s).unwrap() - 0.10).abs() < 1e-9);
        let mut f = e(Provider::ClaudeCode, "claude-fable-5-2");
        f.cache_write_1h = 1_000_000;
        assert!((estimate(&f).unwrap() - 16.0).abs() < 1e-9);
    }

    #[test]
    fn codex_luna_and_long_context() {
        let mut x = e(Provider::Codex, "gpt-6-luna");
        x.input = 100_000;
        x.cache_read = 100_000;
        x.output = 10_000;
        // 0,1*0,10 + 0,1*0,01 + 0,01*0,50
        assert!((estimate(&x).unwrap() - 0.016).abs() < 1e-9);
        x.input = 300_000;
        assert_eq!(estimate(&x), None, "dlhy kontext = ina tarifa, neocenit");
    }

    #[test]
    fn haiku_short_prompt_only() {
        let mut h = e(Provider::ClaudeCode, "claude-haiku-5-5");
        h.input = 50_000;
        h.output = 1_000_000;
        assert!((estimate(&h).unwrap() - 0.505).abs() < 1e-9);
        h.cache_read = 60_000;
        assert_eq!(estimate(&h), None, "prompt nad 100K = ina tarifa");
    }

    #[test]
    fn unknown_or_nonstandard_is_none_not_zero() {
        assert_eq!(estimate(&e(Provider::ClaudeCode, "<synthetic>")), None);
        assert_eq!(
            estimate(&e(Provider::Codex, "claude-opus-5-5")),
            None,
            "model patri inemu poskytovatelovi"
        );
        let mut fast = e(Provider::ClaudeCode, "claude-opus-5-5");
        fast.nonstandard = true;
        assert_eq!(estimate(&fast), None);
        assert_eq!(estimate(&Event::default()), None);
        assert_eq!(prices_date(Some(Provider::Codex)), "2026-11-20");
    }
}
