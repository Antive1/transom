//! Zvuky rozhrania transom: nepovinny, tichy a kratky zvuk po umyselnom a uspesnom kliknuti na ovladac listy,
//! polozku menu alebo ovladac popoveru.
//!
//! * `policy`: kedy sa smie hrat (len umyselne a uspesne akcie; nie hover, vypnute ovladace, zlyhania, aktualizacie
//!   na pozadi, animacie, automatizacia) a prevod hlasitosti.
//! * `synth`: vlastny vygenerovany zvuk (ziadne externe subory).
//! * `engine`: pracovne vlakno s obmedzenou frontou; UI vlakno nikdy necaka. Nedostupne zariadenie sa ticho
//!   preskoci a po case skusi znova.
//! * `wasapi` (Windows): vlastna zvukova relacia transom s hlasitostou relacie; hlavna hlasitost systemu a ine
//!   aplikacie sa nikdy nemenia.
pub mod engine;
pub mod policy;
pub mod synth;
#[cfg(windows)]
pub mod wasapi;

pub use engine::{Backend, BackendError, Player, Stats, Tuning};
pub use policy::{is_deliberate, session_volume, should_play, Origin, Outcome};

/// Nastavenie spustenia systemoveho prehravaca.
#[derive(Clone, Copy, Debug)]
pub struct StartOptions {
    /// Automaticky test alebo simulacia: nikdy sa nevytvori vlakno ani zvukove zariadenie.
    pub automation: bool,
    /// Zapis do logu (zmena stavu zariadenia).
    pub log: fn(&str),
}

/// Premenna prostredia, ktora zvuky natrvalo vypne (testy, skripty): `TRANSOM_NO_SOUND=1`.
pub const NO_SOUND_ENV: &str = "TRANSOM_NO_SOUND";

/// Je zvuk vypnuty premennou prostredia?
pub fn disabled_by_env() -> bool {
    std::env::var_os(NO_SOUND_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Spusti prehravac so skutocnym zvukovym zariadenim (az pri prvom zvuku sa otvori relacia). Pri automatizacii
/// alebo `TRANSOM_NO_SOUND` vrati mlcky prehravac bez vlakna.
#[cfg(windows)]
pub fn start_system(opts: StartOptions) -> Player {
    if opts.automation || disabled_by_env() {
        return Player::silent();
    }
    Player::with_backend(
        || Box::new(wasapi::WasapiBackend::new()),
        Tuning {
            log: Some(opts.log),
            ..Tuning::default()
        },
    )
}

#[cfg(not(windows))]
pub fn start_system(_opts: StartOptions) -> Player {
    Player::silent()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zdrojove kody tohto crate nesmu siahat na hlavnu hlasitost zariadenia ani na vyber vystupu.
    #[test]
    fn the_crate_never_touches_the_master_volume_or_other_apps() {
        for (name, src) in [
            ("wasapi.rs", include_str!("wasapi.rs")),
            ("engine.rs", include_str!("engine.rs")),
        ] {
            // zakazane identifikatory sa skladaju, aby tento test nenasiel sam seba
            for bad in [
                ["IAudioEndpoint", "Volume"].concat(),
                ["SetMasterVolume", "Level"].concat(),
                ["IPolicy", "Config"].concat(),
                ["SetDefault", "Endpoint"].concat(),
                ["SetMute"].concat(),
            ] {
                // kod bez komentarov (komentare tieto nazvy vysvetluju)
                let code: String = src
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(!code.contains(&bad), "{name} pouziva {bad}");
            }
        }
    }

    /// Zvuky rozhrania (relacia transom) a systemovy vystup (hlavna hlasitost) su oddelene crate-y: ziadna zavislost.
    #[test]
    fn the_ui_sound_crate_does_not_depend_on_the_system_output_crate() {
        let manifest = include_str!("../Cargo.toml");
        assert!(!manifest.contains(&["transom", "-audio"].concat()));
    }

    #[test]
    fn automation_and_the_environment_switch_give_a_silent_player() {
        fn nolog(_: &str) {}
        let p = start_system(StartOptions {
            automation: true,
            log: nolog,
        });
        assert!(p.is_silent());
    }
}
