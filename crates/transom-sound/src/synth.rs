//! Vlastny zvuk: kratke jemne "klik" vygenerovane vzorcom, ziadne externe subory ani licencie tretich stran.
//!
//! Povod a licencia: zvuk vznika vypoctom z dvoch sinusoviek (1318,5 Hz a 1976 Hz, t. j. e6 a b6 v pomere kvinty)
//! s plynulym nabehom (2,5 ms), exponencialnym doznievanim a plynulym dobehom do ticha (10 ms). Je to povodne dielo
//! projektu transom (licencia MIT, ako zvysok kodu); nic sa nesampluje ani nestahuje. Vrchol je obmedzeny na `PEAK`
//! plnej amplitudy, takze ani pri hlasitosti 100 % relacie nie je ostry; predvolena hlasitost je nizka.
//!
//! Format: 16-bitove PCM, stereo (L = R), `RATE` Hz, prekladane.
use std::f32::consts::PI;
use std::sync::OnceLock;

pub const RATE: u32 = 44_100;
pub const CHANNELS: usize = 2;
/// Najvyssia amplituda vzorky voci plnej skale (0.5 = -6 dBFS).
pub const PEAK: f32 = 0.5;
/// Dlzka zvuku v ms (kratky: pod `MAX_MS`).
pub const DURATION_MS: u32 = 70;
/// Horna hranica dlzky, ktoru testy vyzaduju (zvuk musi byt strucny).
pub const MAX_MS: u32 = 120;

const ATTACK_MS: f32 = 2.5;
const RELEASE_MS: f32 = 10.0;
const DECAY_S: f32 = 0.018;
const F1: f32 = 1318.5;
const F2: f32 = 1976.0;

/// Jediny zvuk rozhrania (vygeneruje sa raz).
pub fn click() -> &'static [i16] {
    static CLIP: OnceLock<Vec<i16>> = OnceLock::new();
    CLIP.get_or_init(generate)
}

/// Dlzka zvuku v milisekundach.
pub fn duration_ms(clip: &[i16]) -> u32 {
    (clip.len() / CHANNELS) as u32 * 1000 / RATE
}

fn generate() -> Vec<i16> {
    let frames = (RATE * DURATION_MS / 1000) as usize;
    let attack = ATTACK_MS / 1000.0 * RATE as f32;
    let release = RELEASE_MS / 1000.0 * RATE as f32;
    let raw: Vec<f32> = (0..frames)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let tone = 0.7 * (2.0 * PI * F1 * t).sin() + 0.3 * (2.0 * PI * F2 * t).sin();
            let fade_in = if (i as f32) < attack {
                0.5 - 0.5 * (PI * i as f32 / attack).cos()
            } else {
                1.0
            };
            let left = (frames - 1 - i) as f32;
            let fade_out = if left < release {
                0.5 - 0.5 * (PI * left / release).cos()
            } else {
                1.0
            };
            tone * fade_in * fade_out * (-t / DECAY_S).exp()
        })
        .collect();
    let max = raw.iter().fold(0.0_f32, |m, v| m.max(v.abs())).max(1e-6);
    let scale = PEAK * i16::MAX as f32 / max;
    let mut out = Vec::with_capacity(frames * CHANNELS);
    for v in raw {
        let s = (v * scale)
            .round()
            .clamp(-(i16::MAX as f32), i16::MAX as f32) as i16;
        out.push(s);
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sound_is_short_quiet_and_not_silent() {
        let c = click();
        assert_eq!(c.len() % CHANNELS, 0);
        let ms = duration_ms(c);
        assert!((20..=MAX_MS).contains(&ms), "{ms} ms");
        let peak = c.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(peak > 2000, "nie je ticho: {peak}");
        assert!(
            peak as f32 <= PEAK * i16::MAX as f32 + 1.0,
            "vrchol {peak} presahuje limit"
        );
    }

    #[test]
    fn it_starts_and_ends_at_zero_without_dc_offset_so_it_does_not_pop() {
        let c = click();
        assert_eq!(c[0], 0);
        assert!(c[c.len() - 2].unsigned_abs() < 40 && c[c.len() - 1].unsigned_abs() < 40);
        let mean = c.iter().map(|s| *s as f64).sum::<f64>() / c.len() as f64;
        assert!(mean.abs() < 400.0, "stredna hodnota {mean}");
        // plynuly priebeh: najvacsi skok medzi susednymi vzorkami je maly voci vrcholu
        let left: Vec<i32> = c.chunks(CHANNELS).map(|f| f[0] as i32).collect();
        let max_step = left
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .max()
            .unwrap_or(0);
        assert!(max_step < 9000, "skok {max_step}");
    }

    #[test]
    fn both_channels_are_identical_and_the_clip_is_deterministic() {
        let a = click();
        assert!(a.chunks(CHANNELS).all(|f| f[0] == f[1]));
        assert_eq!(a, generate().as_slice());
    }
}
