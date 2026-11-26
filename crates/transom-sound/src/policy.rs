//! Cista logika zvukov rozhrania: kedy sa smie hrat a ako sa hlasitost prevedie na hlasitost relacie.
//! Bez Win32 a bez zvukoveho zariadenia, preto sa da cela testovat.
use transom_config::{UiSounds, MAX_SOUND_VOLUME};

/// Odkial pochadza podnet. Hraju len umyselne podnety (`is_deliberate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Klik mysou na ovladaci prvok listy.
    Pointer,
    /// Vyber polozky kontextoveho menu (mys alebo Enter).
    Menu,
    /// Klik na ovladaci prvok popoveru (kalendar, panel AI).
    Popover,
    /// Aktivacia cez pristupnost (MSAA/UIA `DoDefaultAction`).
    Assistive,
    /// Najazd kurzora (nikdy).
    Hover,
    /// Stlacenie tlacidla bez uvolnenia (nikdy; akcia sa spusti az uvolnenim).
    Press,
    /// Aktualizacia na pozadi: hodiny, media, AI, bateria (nikdy).
    Background,
    /// Snimka animacie, napr. posuvny text (nikdy).
    Animation,
    /// Automaticky test alebo simulovany vstup (nikdy).
    Automation,
}

/// Vysledok akcie, na ktoru pouzivatel klikol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Akcia sa vykonala (alebo sa uspesne odoslala).
    Done,
    /// Ovladac je vypnuty alebo akcia nie je dostupna.
    Disabled,
    /// Akcia zlyhala (zablokovana skratka, ziadna session, chyba).
    Failed,
}

/// Je podnet umyselny ukon pouzivatela?
pub fn is_deliberate(o: Origin) -> bool {
    matches!(
        o,
        Origin::Pointer | Origin::Menu | Origin::Popover | Origin::Assistive
    )
}

/// Ma sa po tomto podnete prehrat zvuk? Len pri zapnutych zvukoch s nenulovou hlasitostou, umyselnom podnete a
/// uspesnej akcii.
pub fn should_play(s: &UiSounds, origin: Origin, outcome: Outcome) -> bool {
    s.effective_volume() > 0 && is_deliberate(origin) && outcome == Outcome::Done
}

/// Hlasitost relacie 0.0..=1.0 z percent (mimo rozsahu sa oreze). Linearne, aby sa posuvnik v nastaveniach
/// zhodoval s posuvnikom relacie transom v miesadle hlasitosti Windows.
pub fn session_volume(percent: u32) -> f32 {
    percent.min(MAX_SOUND_VOLUME) as f32 / MAX_SOUND_VOLUME as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Origin; 9] = [
        Origin::Pointer,
        Origin::Menu,
        Origin::Popover,
        Origin::Assistive,
        Origin::Hover,
        Origin::Press,
        Origin::Background,
        Origin::Animation,
        Origin::Automation,
    ];

    fn on(volume: u32) -> UiSounds {
        UiSounds {
            enabled: true,
            volume,
        }
    }

    #[test]
    fn only_deliberate_successful_actions_play() {
        for o in ALL {
            for out in [Outcome::Done, Outcome::Disabled, Outcome::Failed] {
                let want = matches!(
                    o,
                    Origin::Pointer | Origin::Menu | Origin::Popover | Origin::Assistive
                ) && out == Outcome::Done;
                assert_eq!(should_play(&on(30), o, out), want, "{o:?} {out:?}");
            }
        }
    }

    #[test]
    fn hover_press_updates_animation_and_automation_never_play() {
        for o in [
            Origin::Hover,
            Origin::Press,
            Origin::Background,
            Origin::Animation,
            Origin::Automation,
        ] {
            assert!(!is_deliberate(o), "{o:?}");
            assert!(!should_play(&on(100), o, Outcome::Done), "{o:?}");
        }
    }

    #[test]
    fn mute_and_zero_volume_silence_everything() {
        let muted = UiSounds {
            enabled: false,
            volume: 80,
        };
        let zero = on(0);
        for o in ALL {
            assert!(!should_play(&muted, o, Outcome::Done), "{o:?}");
            assert!(!should_play(&zero, o, Outcome::Done), "{o:?}");
        }
    }

    #[test]
    fn session_volume_is_linear_and_always_within_bounds() {
        assert_eq!(session_volume(0), 0.0);
        assert_eq!(session_volume(100), 1.0);
        assert!((session_volume(25) - 0.25).abs() < 1e-6);
        for p in [101, 250, u32::MAX] {
            assert_eq!(session_volume(p), 1.0, "{p}");
        }
        let mut last = -1.0;
        for p in 0..=100 {
            let v = session_volume(p);
            assert!((0.0..=1.0).contains(&v) && v > last, "{p}");
            last = v;
        }
    }
}
