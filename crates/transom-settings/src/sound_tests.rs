//! Stranka "Zvuky rozhrania" cez skutocny stav `Ui` (klikanie cez `click`, klavesnica cez `slider_key`/`flush_pending`) s
//! izolovanou docasnou konfiguraciou a **falosnym zvukovym zariadenim**: skutocne zariadenie ani ine aplikacie sa
//! nepouzivaju a hlasitost systemu sa nikdy nemeni.
use super::*;
use crate::appearance_page::SliderKey;
use crate::archive_tests::Env;
use std::sync::{Arc, Mutex};
use transom_config::{load, save, update, UiSounds};
use transom_sound::{Backend, BackendError, Player, Tuning};

#[derive(Clone, Default)]
struct Fake {
    plays: Arc<Mutex<Vec<f32>>>,
}

struct FakeBackend(Fake);

impl Backend for FakeBackend {
    fn play(&mut self, _clip: &[i16], volume: f32) -> Result<(), BackendError> {
        self.0.plays.lock().unwrap().push(volume);
        Ok(())
    }
    fn release(&mut self) {}
}

fn fake_player(f: &Fake) -> Player {
    let f2 = f.clone();
    Player::with_backend(
        move || Box::new(FakeBackend(f2)),
        Tuning {
            min_interval: std::time::Duration::from_millis(1),
            ..Tuning::default()
        },
    )
}

fn config_path(e: &Env) -> std::path::PathBuf {
    e.root.join("config.json")
}

fn bytes(p: &std::path::Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_default()
}

fn wait_plays(f: &Fake, n: usize) -> bool {
    for _ in 0..400 {
        if f.plays.lock().unwrap().len() >= n {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

fn quiet_for(f: &Fake, n: usize) -> bool {
    std::thread::sleep(std::time::Duration::from_millis(120));
    f.plays.lock().unwrap().len() == n
}

#[test]
fn sounds_are_off_by_default_and_the_toggle_is_saved_to_the_config_only() {
    let e = Env::new("snd-toggle");
    let mut ui = e.ui();
    assert!(!ui.cfg.ui_sounds.enabled);
    assert_eq!(
        ui.cfg.ui_sounds.volume,
        transom_config::DEFAULT_SOUND_VOLUME
    );
    ui.click(Hit::Nav(Page::Sound));
    ui.click(Hit::SoundToggle);
    let saved = load(&config_path(&e)).config;
    assert!(saved.ui_sounds.enabled && ui.cfg.ui_sounds.enabled);
    assert_eq!(saved.ui_sounds.volume, 25);
    // druhy klik = stlmenie
    ui.click(Hit::SoundToggle);
    assert!(!load(&config_path(&e)).config.ui_sounds.enabled);
    // prepinac nezmenil nic ine (ziadne zdroje AI, zobrazenia ani vzhlad)
    let d = transom_config::Config::default();
    let s = load(&config_path(&e)).config;
    assert_eq!(
        (s.usage, s.appearance, s.layout, s.displays),
        (d.usage, d.appearance, d.layout, d.displays)
    );
}

#[test]
fn volume_steps_stay_within_zero_and_one_hundred_and_persist() {
    let e = Env::new("snd-volume");
    let mut ui = e.ui();
    for _ in 0..30 {
        ui.click(Hit::SoundStep(5));
    }
    assert_eq!(ui.cfg.ui_sounds.volume, 100);
    assert_eq!(load(&config_path(&e)).config.ui_sounds.volume, 100);
    for _ in 0..40 {
        ui.click(Hit::SoundStep(-5));
    }
    assert_eq!(ui.cfg.ui_sounds.volume, 0);
    assert_eq!(load(&config_path(&e)).config.ui_sounds.volume, 0);
    ui.click(Hit::SoundStep(5));
    assert_eq!(load(&config_path(&e)).config.ui_sounds.volume, 5);
}

#[test]
fn the_preview_plays_only_when_enabled_at_the_stored_volume_and_never_touches_the_config() {
    let e = Env::new("snd-preview");
    let f = Fake::default();
    let mut ui = e.ui();
    ui.sound = fake_player(&f);
    save(&config_path(&e), &transom_config::Config::default()).unwrap();
    let before = bytes(&config_path(&e));
    // vypnute: tlacidlo nic nehra a vysvetli to
    ui.click(Hit::SoundPreview);
    assert_eq!(ui.status, t("snd.muted_hint"));
    assert!(quiet_for(&f, 0));
    // zapnute pri 25 %: ukazka s hlasitostou relacie 0.25
    ui.cfg.ui_sounds = UiSounds {
        enabled: true,
        volume: 25,
    };
    ui.click(Hit::SoundPreview);
    assert!(wait_plays(&f, 1));
    assert!((f.plays.lock().unwrap()[0] - 0.25).abs() < 1e-6);
    assert_eq!(ui.status, tf("snd.previewed", &["25"]));
    assert_eq!(bytes(&config_path(&e)), before, "ukazka nic neulozila");
    // nulova hlasitost nehra
    ui.cfg.ui_sounds.volume = 0;
    std::thread::sleep(std::time::Duration::from_millis(10));
    ui.click(Hit::SoundPreview);
    assert!(quiet_for(&f, 1));
    assert_eq!(ui.status, t("snd.muted_hint"));
}

#[test]
fn the_step_buttons_and_keyboard_play_one_preview_after_saving_and_never_while_muted() {
    let e = Env::new("snd-keys");
    let f = Fake::default();
    let mut ui = e.ui();
    ui.sound = fake_player(&f);
    ui.cfg.ui_sounds.enabled = true;
    // tlacidlo +: ulozi a prehra jednu ukazku s novou hlasitostou
    ui.click(Hit::SoundStep(5));
    assert!(wait_plays(&f, 1));
    assert!((f.plays.lock().unwrap()[0] - 0.30).abs() < 1e-6);
    assert_eq!(load(&config_path(&e)).config.ui_sounds.volume, 30);
    // klavesnica: viac klaves, jedna ukazka po ulozeni (flush)
    std::thread::sleep(std::time::Duration::from_millis(10));
    ui.kfocus = Some(Hit::SoundTrack);
    for _ in 0..3 {
        assert!(ui.slider_key(SliderKey::Inc, false));
    }
    assert_eq!(ui.cfg.ui_sounds.volume, 45);
    assert!(quiet_for(&f, 1), "ukazka sa neprehrava pocas pisania");
    ui.flush_pending();
    assert!(wait_plays(&f, 2));
    assert!((f.plays.lock().unwrap()[1] - 0.45).abs() < 1e-6);
    assert_eq!(load(&config_path(&e)).config.ui_sounds.volume, 45);
    // Home = 0 %, End = 100 %, uz na kraji sa nic nemeni
    assert!(ui.slider_key(SliderKey::Max, false));
    assert!(!ui.slider_key(SliderKey::Inc, false));
    assert_eq!(ui.cfg.ui_sounds.volume, 100);
    assert!(ui.slider_key(SliderKey::Min, false));
    assert!(!ui.slider_key(SliderKey::Dec, false));
    ui.flush_pending();
    assert!(quiet_for(&f, 2), "0 % nehra");
    // stlmene: ani kroky, ani klavesy nehraju
    ui.cfg.ui_sounds.enabled = false;
    ui.click(Hit::SoundStep(5));
    assert!(ui.slider_key(SliderKey::Inc, false));
    ui.flush_pending();
    assert!(quiet_for(&f, 2));
}

#[test]
fn the_opacity_slider_is_unaffected_by_the_sound_slider_keys() {
    let e = Env::new("snd-opacity");
    let mut ui = e.ui();
    ui.kfocus = Some(Hit::OpacityTrack);
    assert!(ui.slider_key(SliderKey::Dec, false));
    assert_eq!(ui.cfg.appearance.bg_opacity, 95);
    assert_eq!(ui.cfg.ui_sounds.volume, 25, "zvuk sa nezmenil");
}

#[test]
fn a_concurrent_change_survives_and_a_read_only_configuration_is_never_written() {
    let e = Env::new("snd-concurrent");
    save(&config_path(&e), &transom_config::Config::default()).unwrap();
    let mut ui = e.ui();
    // iny proces zmeni hodiny medzi nacitanim a ulozenim
    update(&config_path(&e), |c| c.clock.seconds = true).unwrap();
    ui.click(Hit::SoundToggle);
    let s = load(&config_path(&e)).config;
    assert!(s.clock.seconds && s.ui_sounds.enabled, "obe zmeny prezili");
    // poskodena konfiguracia: len na citanie, subor ostane bajt po bajte
    drop(ui);
    drop(e);
    let e = Env::new("snd-readonly");
    std::fs::write(config_path(&e), "{ poskodene").unwrap();
    let mut ui = e.ui();
    ui.read_only = true;
    ui.click(Hit::SoundToggle);
    ui.click(Hit::SoundStep(5));
    assert_eq!(bytes(&config_path(&e)), b"{ poskodene");
    assert_eq!(ui.status, t("status.readonly"));
}

#[test]
fn the_page_exposes_a_checkbox_a_slider_named_buttons_and_a_preview_for_screen_readers() {
    use crate::page_a11y::Target;
    use transom_platform::menu_access::{ROLE_CHECKBUTTON, ROLE_PUSHBUTTON, ROLE_SLIDER};
    let _g = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for enabled in [false, true] {
            let loaded = transom_config::Loaded {
                config: transom_config::Config::default(),
                read_only: false,
                note: None,
                issue: None,
            };
            let mut ui = make_ui(loaded, Theme::dark(), String::new());
            ui.page = Page::Sound;
            ui.cfg.ui_sounds.enabled = enabled;
            crate::a11y_tests::paint(&mut ui, 96);
            let (entries, extras, targets, _) = ui.a11y_snapshot(&[], None);
            let find = |h: Hit| {
                targets
                    .iter()
                    .position(|t| *t == Target::Hit(h.clone()))
                    .unwrap_or_else(|| panic!("{lang:?}: {h:?} chyba"))
            };
            let tg = find(Hit::SoundToggle);
            assert_eq!(extras[tg].role, ROLE_CHECKBUTTON);
            assert_eq!(entries[tg].name, t("snd.enable"));
            assert_eq!(
                extras[tg].state & transom_platform::menu_access::STATE_CHECKED != 0,
                enabled,
                "{lang:?}"
            );
            let tr = find(Hit::SoundTrack);
            assert_eq!(extras[tr].role, ROLE_SLIDER);
            assert_eq!(extras[tr].value, "25 %");
            assert_eq!(entries[tr].name, t("snd.volume"));
            assert_eq!(entries[find(Hit::SoundStep(-5))].name, t("acc.sound_dec"));
            assert_eq!(entries[find(Hit::SoundStep(5))].name, t("acc.sound_inc"));
            let pv = find(Hit::SoundPreview);
            assert_eq!(extras[pv].role, ROLE_PUSHBUTTON);
            assert_eq!(entries[pv].name, t("snd.preview"));
            // nazvy su jedinecne a neprazdne
            let names: Vec<&String> = [tg, tr, pv].iter().map(|i| &entries[*i].name).collect();
            assert!(names.iter().all(|n| !n.is_empty()));
            assert_ne!(names[0], names[1]);
        }
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
}

#[test]
fn the_keyboard_ring_reaches_every_sound_control_in_drawing_order() {
    use crate::focus::{ring, Slot};
    let _g = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let loaded = transom_config::Loaded {
        config: transom_config::Config::default(),
        read_only: false,
        note: None,
        issue: None,
    };
    let mut ui = make_ui(loaded, Theme::dark(), String::new());
    ui.page = Page::Sound;
    crate::a11y_tests::paint(&mut ui, 96);
    let hits: Vec<Hit> = ui.all_hits.iter().map(|(_, h)| h.clone()).collect();
    let r = ring(&hits, &[]);
    let pos = |h: Hit| {
        r.iter()
            .position(|s| *s == Slot::Page(h.clone()))
            .unwrap_or_else(|| panic!("{h:?} nie je v kruhu"))
    };
    assert!(pos(Hit::SoundToggle) < pos(Hit::SoundTrack));
    assert!(pos(Hit::SoundTrack) < pos(Hit::SoundStep(-5)));
    assert!(pos(Hit::SoundStep(-5)) < pos(Hit::SoundStep(5)));
    assert!(pos(Hit::SoundStep(5)) < pos(Hit::SoundPreview));
}

#[test]
fn every_new_string_exists_in_all_languages_with_the_same_placeholders() {
    let _g = crate::archive_tests::ENV
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for lang in transom_i18n::Lang::ALL {
        transom_i18n::set_lang(lang);
        for k in [
            "nav.sound",
            "snd.title",
            "snd.help1",
            "snd.help2",
            "snd.enable",
            "snd.volume",
            "snd.volume_value",
            "snd.preview",
            "snd.previewed",
            "snd.muted_hint",
            "snd.note_default",
            "acc.sound_dec",
            "acc.sound_inc",
        ] {
            let v = t(k);
            assert!(!v.is_empty() && v != k, "{lang:?}: chyba {k}");
        }
        assert_eq!(tf("snd.volume_value", &["40"]), "40 %");
        assert!(tf("snd.previewed", &["40"]).contains("40"), "{lang:?}");
    }
    transom_i18n::set_lang(transom_i18n::Lang::En);
}
