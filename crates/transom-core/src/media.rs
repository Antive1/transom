//! Media: data GSMTC session (bez WinRT typov) a politika, ktoru session lista zobrazi (PLAN §6, rozhodnute po S5).
//!
//! Vyber (v tomto poradi):
//! 1. session, ktoru si pouzivatel zvolil prepnutim v liste (`user_pick`), kym existuje a ma metadata - explicitny
//!    klik ma prednost aj pred konfiguraciou (inak by prepinac pri nakonfigurovanej aplikacii nic nerobil);
//! 2. aplikacia nakonfigurovana pouzivatelom (`prefer_app`, podretazec `SourceAppUserModelId` bez ohladu na velkost),
//!    ak ma session s platnymi metadatami;
//! 3. hrajuca session s platnymi metadatami: ak predchadzajuca vybrana stale hra, ostane (stabilita); inak ta,
//!    ktora zacala hrat naposledy;
//! 4. ak nic nehra: predchadzajuca vybrana, ak stale existuje (po pauze polozka nezmizne); inak jedina/naposledy
//!    hrajuca session s metadatami.
//!
//! Systemova "aktualna session" sa nepouziva (v S5 sa prepla na sumovu session Riot Client). Session bez nazvu sa
//! neponukaju. Ak zostava viac vierohodnych kandidatov (viac hrajucich, alebo ziadna nehra a je ich viac), lista
//! zobrazi indikator `1/2` a kliknutim sa prepina.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayState {
    #[default]
    Closed,
    Opened,
    Changing,
    Stopped,
    Playing,
    Paused,
}

/// Priznaky `GlobalSystemMediaTransportControlsSessionPlaybackControls`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub play: bool,
    pub pause: bool,
    pub toggle: bool,
    pub next: bool,
    pub prev: bool,
    pub seek: bool,
}

/// Surova casova os (jednotky 100 ns ako `TimeSpan`/`DateTime` vo WinRT). `last_updated` je UTC FILETIME (0 = neznamy).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timeline {
    pub start: i64,
    pub end: i64,
    pub position: i64,
    pub last_updated: i64,
}

/// Platna casova os v sekundach (pozicia relativne k zaciatku).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidTimeline {
    pub position_s: f64,
    pub duration_s: f64,
}

const TICKS: f64 = 10_000_000.0;
/// Tolerancia pre poziciu tesne za koncom (zaokruhlenie prehravaca).
const END_SLACK: i64 = 2 * 10_000_000;

impl Timeline {
    /// Platna casova os, alebo `None`. Neplatna je: `end <= start` (napr. Spotify tesne po starte `pos=0/end=0`),
    /// pozicia mimo `<start, end + 2 s>`, alebo nikdy neaktualizovana (`last_updated == 0`). Lista vtedy nezobrazi
    /// ziadnu poziciu ani neumozni seek - nikdy nie falosnu nulu.
    /// Ako [`Timeline::valid`], ale pozicia dopocitana k casu `now_ft` (UTC FILETIME), kym skladba hra: prehravac
    /// hlasi poziciu len pri zmene (zaciatok, presun), medzitym ide hodiny.
    pub fn valid_at(&self, now_ft: i64, playing: bool) -> Option<ValidTimeline> {
        let mut t = *self;
        if playing && self.last_updated > 0 && now_ft > self.last_updated {
            t.position = (self.position + (now_ft - self.last_updated)).min(self.end);
        }
        t.valid()
    }

    pub fn valid(&self) -> Option<ValidTimeline> {
        if self.end <= self.start || self.last_updated <= 0 {
            return None;
        }
        if self.position < self.start || self.position > self.end + END_SLACK {
            return None;
        }
        let duration = self.end - self.start;
        Some(ValidTimeline {
            position_s: (self.position - self.start).min(duration) as f64 / TICKS,
            duration_s: duration as f64 / TICKS,
        })
    }
}

/// Teraz ako UTC FILETIME (100 ns od 1601), v jednotkach casovej osi.
pub fn now_filetime() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_nanos() / 100) as i64)
        .unwrap_or(0)
        + 116_444_736_000_000_000
}

/// Jedna session (snimka z media vlakna).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionInfo {
    /// Jedinecny kluc v ramci snimky: AUMID, pri duplicitach s priponou `#2`, `#3`...
    pub key: String,
    /// `SourceAppUserModelId` (napr. `Spotify.exe`, `Brave`).
    pub app_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub state: PlayState,
    pub controls: Controls,
    pub timeline: Timeline,
    /// Poradove cislo posledneho prechodu do `Playing` (vyssie = neskor); 0 = este nehrala.
    pub play_seq: u64,
    /// Identita dekodovaneho obalu (hash obsahu) patriaceho **aktualnej** skladbe; `None` = zastupny symbol.
    pub art_id: Option<u64>,
}

impl SessionInfo {
    /// Identita media (skladby/videa): session + nazov + interpret + album. Obal sa viaze na tuto identitu.
    pub fn media_key(&self) -> String {
        format!(
            "{}\u{1}{}\u{1}{}\u{1}{}",
            self.key,
            self.title.trim(),
            self.artist.trim(),
            self.album.trim()
        )
    }

    /// Platne metadata = neprazdny nazov a session nie je zatvorena.
    pub fn has_metadata(&self) -> bool {
        !self.title.trim().is_empty() && self.state != PlayState::Closed
    }

    /// Text do lisaty: `Nazov – Interpret`, alebo len nazov.
    pub fn display_text(&self) -> String {
        let t = self.title.trim();
        let a = self.artist.trim();
        if a.is_empty() {
            t.to_string()
        } else {
            format!("{t} – {a}")
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Kluc zvolenej session.
    pub chosen: Option<String>,
    /// Vierohodni kandidati v stabilnom poradi (obsahuje aj `chosen`); viac ako 1 = ponuknut prepinanie.
    pub candidates: Vec<String>,
}

impl Selection {
    /// Pozicia zvolenej medzi kandidatmi (1-based) a ich pocet, ak su aspon 2.
    pub fn indicator(&self) -> Option<(usize, usize)> {
        let c = self.chosen.as_ref()?;
        if self.candidates.len() < 2 {
            return None;
        }
        let i = self.candidates.iter().position(|k| k == c)?;
        Some((i + 1, self.candidates.len()))
    }

    /// Kluc dalsieho kandidata po zvolenej (cyklicky) - pre prepnutie klikom.
    pub fn next_candidate(&self) -> Option<String> {
        if self.candidates.len() < 2 {
            return None;
        }
        let i = self
            .chosen
            .as_ref()
            .and_then(|c| self.candidates.iter().position(|k| k == c))
            .map(|i| (i + 1) % self.candidates.len())
            .unwrap_or(0);
        Some(self.candidates[i].clone())
    }
}

fn find<'a>(sessions: &'a [SessionInfo], key: Option<&str>) -> Option<&'a SessionInfo> {
    let key = key?;
    sessions.iter().find(|s| s.key == key && s.has_metadata())
}

/// Vyber session podla politiky v hlavicke modulu.
pub fn select(
    sessions: &[SessionInfo],
    prefer_app: Option<&str>,
    user_pick: Option<&str>,
    previous: Option<&str>,
) -> Selection {
    let valid: Vec<&SessionInfo> = sessions.iter().filter(|s| s.has_metadata()).collect();
    let playing: Vec<&SessionInfo> = valid
        .iter()
        .copied()
        .filter(|s| s.state == PlayState::Playing)
        .collect();
    // vierohodni: hrajuci, ak nejaki su; inak vsetci s metadatami
    let pool = if playing.is_empty() { &valid } else { &playing };
    let mut candidates: Vec<String> = pool.iter().map(|s| s.key.clone()).collect();

    let latest = |v: &[&SessionInfo]| -> Option<String> {
        v.iter().max_by_key(|s| s.play_seq).map(|s| s.key.clone())
    };

    let preferred = prefer_app
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .and_then(|p| {
            let m: Vec<&SessionInfo> = valid
                .iter()
                .copied()
                .filter(|s| s.app_id.to_lowercase().contains(&p))
                .collect();
            // pri viacerych zhodach: hrajuca a naposledy spustena
            let playing_m: Vec<&SessionInfo> = m
                .iter()
                .copied()
                .filter(|s| s.state == PlayState::Playing)
                .collect();
            latest(if playing_m.is_empty() { &m } else { &playing_m })
        });

    let chosen = find(sessions, user_pick)
        .map(|s| s.key.clone())
        .or_else(|| preferred.clone())
        .or_else(|| {
            if playing.is_empty() {
                None
            } else if let Some(p) =
                find(sessions, previous).filter(|s| s.state == PlayState::Playing)
            {
                Some(p.key.clone())
            } else {
                latest(&playing)
            }
        })
        .or_else(|| find(sessions, previous).map(|s| s.key.clone()))
        .or_else(|| latest(&valid));

    // zvolena (napr. po pauze) aj nakonfigurovana musia byt medzi kandidatmi, aby indikator sedel a dalo sa
    // prepnut spat na nakonfigurovanu aplikaciu
    for c in [&chosen, &preferred].into_iter().flatten() {
        if !candidates.contains(c) {
            candidates.insert(0, c.clone());
        }
    }
    Selection { chosen, candidates }
}

/// Prikaz pre tlacidlo prehrat/pauza.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleCmd {
    Toggle,
    Play,
    Pause,
}

/// Stav troch tlacidiel (vzdy viditelne; nepodporovany prikaz = zakazane tlacidlo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub prev_enabled: bool,
    /// `true` = ikona pauzy (session hra), `false` = ikona prehrat (pozastavene, zastavene, otvorene...).
    pub shows_pause: bool,
    pub toggle_enabled: bool,
    pub next_enabled: bool,
}

pub fn buttons(s: &SessionInfo) -> Buttons {
    Buttons {
        prev_enabled: s.controls.prev,
        shows_pause: s.state == PlayState::Playing,
        toggle_enabled: toggle_command(s).is_some(),
        next_enabled: s.controls.next,
    }
}

/// Stav na zobrazenie: prechodny `Changing` (napr. pocas prepnutia skladby) nezmeni ikonu - ostane predchadzajuci
/// znamy stav, aby stredne tlacidlo neblikalo Play/Pause.
pub fn stable_state(previous: Option<PlayState>, now: PlayState) -> PlayState {
    match (now, previous) {
        (PlayState::Changing, Some(p)) if p != PlayState::Closed => p,
        _ => now,
    }
}

/// Vysledok citania nahladu pri poslednej obnove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetched {
    /// Nahlad sa precital a dekodoval (identita obsahu).
    Image(u64),
    /// Session nahlad nema (alebo sa ho nepodarilo precitat/dekodovat).
    Missing,
    /// Nahlad sa pri tejto obnove necital (napr. zmenil sa len stav prehravania).
    NotRead,
}

/// Ktory obal patri aktualnej skladbe. `previous` = (identita media, obal) z predchadzajucej snimky.
/// - novo precitany obal plati pre aktualnu identitu;
/// - ak sa necital: povodny obal ostane len pri **rovnakej** identite media, inak sa zahodi (zastaraly obal
///   inej skladby sa nikdy nezobrazi pod novym nazvom);
/// - chybajuci nahlad -> `None` (zastupny symbol).
pub fn resolve_art(
    previous: Option<(&str, Option<u64>)>,
    media_key: &str,
    fetched: Fetched,
) -> Option<u64> {
    match fetched {
        Fetched::Image(id) => Some(id),
        Fetched::Missing => None,
        Fetched::NotRead => previous
            .filter(|(k, _)| *k == media_key)
            .and_then(|(_, a)| a),
    }
}

/// Prikaz pre tlacidlo prehrat/pauza podla stavu a priznakov; `None` = session ho nepovoluje.
pub fn toggle_command(s: &SessionInfo) -> Option<ToggleCmd> {
    let c = s.controls;
    if s.state == PlayState::Playing {
        if c.pause {
            Some(ToggleCmd::Pause)
        } else if c.toggle {
            Some(ToggleCmd::Toggle)
        } else {
            None
        }
    } else if c.play {
        Some(ToggleCmd::Play)
    } else if c.toggle {
        Some(ToggleCmd::Toggle)
    } else {
        None
    }
}

/// Zobrazit napovedu pre Spotify: bezi proces Spotify, ale GSMTC pre neho nema ziadnu session.
pub fn spotify_hint_needed(spotify_running: bool, sessions: &[SessionInfo]) -> bool {
    spotify_running
        && !sessions
            .iter()
            .any(|s| s.app_id.to_lowercase().contains("spotify"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(key: &str, title: &str, state: PlayState, seq: u64) -> SessionInfo {
        SessionInfo {
            key: key.into(),
            app_id: key.trim_end_matches("#2").into(),
            title: title.into(),
            state,
            play_seq: seq,
            ..Default::default()
        }
    }

    #[test]
    fn noise_session_without_title_is_ignored() {
        // S5: Riot Client (Paused, bez nazvu) a hrajuci Brave
        let v = vec![
            s("Riot Client", "", PlayState::Paused, 0),
            s("Brave", "Video", PlayState::Playing, 1),
        ];
        let sel = select(&v, None, None, None);
        assert_eq!(sel.chosen.as_deref(), Some("Brave"));
        assert_eq!(sel.candidates, vec!["Brave"]);
        assert_eq!(sel.indicator(), None);
        // len sumova session -> nic
        let sel = select(&v[..1], None, None, None);
        assert_eq!(sel, Selection::default());
    }

    #[test]
    fn configured_app_wins_even_when_paused() {
        let v = vec![
            s("Spotify.exe", "Song", PlayState::Paused, 1),
            s("Brave", "Video", PlayState::Playing, 2),
        ];
        let sel = select(&v, Some("spotify"), None, None);
        assert_eq!(sel.chosen.as_deref(), Some("Spotify.exe"));
        // zvolena je medzi kandidatmi, aby sa dalo prepnut na hrajuci Brave
        assert_eq!(sel.indicator(), Some((1, 2)));
        // nakonfigurovana bez session -> bezna politika
        let sel = select(&v[1..], Some("spotify"), None, None);
        assert_eq!(sel.chosen.as_deref(), Some("Brave"));
    }

    #[test]
    fn cycling_works_even_with_configured_app() {
        let v = vec![
            s("Spotify.exe", "Song", PlayState::Paused, 1),
            s("Brave", "Video", PlayState::Playing, 2),
        ];
        let sel = select(&v, Some("spotify"), None, None);
        assert_eq!(sel.chosen.as_deref(), Some("Spotify.exe"));
        // klik na indikator: dalsi kandidat musi byt naozaj zvoleny
        let pick = sel.next_candidate();
        assert_eq!(pick.as_deref(), Some("Brave"));
        let sel2 = select(&v, Some("spotify"), pick.as_deref(), sel.chosen.as_deref());
        assert_eq!(sel2.chosen.as_deref(), Some("Brave"));
        assert_eq!(sel2.indicator(), Some((2, 2)));
        // a spat na nakonfigurovanu
        let back = sel2.next_candidate();
        let sel3 = select(&v, Some("spotify"), back.as_deref(), sel2.chosen.as_deref());
        assert_eq!(sel3.chosen.as_deref(), Some("Spotify.exe"));
    }

    #[test]
    fn latest_started_playing_wins_and_stays_stable() {
        let v = vec![
            s("A", "a", PlayState::Playing, 5),
            s("B", "b", PlayState::Playing, 9),
        ];
        assert_eq!(select(&v, None, None, None).chosen.as_deref(), Some("B"));
        // predchadzajuca stale hra -> nemeni sa (ziadne preskakovanie)
        let sel = select(&v, None, None, Some("A"));
        assert_eq!(sel.chosen.as_deref(), Some("A"));
        assert_eq!(sel.indicator(), Some((1, 2)));
        assert_eq!(sel.next_candidate().as_deref(), Some("B"));
    }

    #[test]
    fn paused_previous_stays_visible() {
        let v = vec![
            s("Spotify.exe", "Song", PlayState::Paused, 3),
            s("Brave", "Video", PlayState::Paused, 1),
        ];
        let sel = select(&v, None, None, Some("Spotify.exe"));
        assert_eq!(sel.chosen.as_deref(), Some("Spotify.exe"));
        assert_eq!(sel.candidates.len(), 2, "obe pozastavene su vierohodne");
        // bez predchadzajucej: naposledy hrajuca
        assert_eq!(
            select(&v, None, None, None).chosen.as_deref(),
            Some("Spotify.exe")
        );
    }

    #[test]
    fn playing_beats_paused_previous() {
        let v = vec![
            s("Spotify.exe", "Song", PlayState::Paused, 3),
            s("Brave", "Video", PlayState::Playing, 4),
        ];
        let sel = select(&v, None, None, Some("Spotify.exe"));
        assert_eq!(sel.chosen.as_deref(), Some("Brave"));
    }

    #[test]
    fn user_pick_wins_until_session_disappears() {
        let v = vec![
            s("A", "a", PlayState::Playing, 5),
            s("B", "b", PlayState::Playing, 9),
        ];
        assert_eq!(
            select(&v, None, Some("A"), Some("B")).chosen.as_deref(),
            Some("A")
        );
        // zvolena zmizla -> spat na politiku
        assert_eq!(
            select(&v[1..], None, Some("A"), None).chosen.as_deref(),
            Some("B")
        );
    }

    #[test]
    fn closed_session_disappears_cleanly() {
        let v = vec![s("Brave", "Video", PlayState::Closed, 1)];
        assert_eq!(select(&v, None, None, Some("Brave")), Selection::default());
    }

    #[test]
    fn invalid_timelines_are_rejected() {
        // Spotify tesne po starte: pos=0/end=0
        let t = Timeline {
            start: 0,
            end: 0,
            position: 0,
            last_updated: 1,
        };
        assert_eq!(t.valid(), None);
        // nikdy neaktualizovana
        let t = Timeline {
            start: 0,
            end: 100 * 10_000_000,
            position: 0,
            last_updated: 0,
        };
        assert_eq!(t.valid(), None);
        // pozicia pred zaciatkom / daleko za koncom
        let t = Timeline {
            start: 10,
            end: 100,
            position: 5,
            last_updated: 1,
        };
        assert_eq!(t.valid(), None);
        let t = Timeline {
            start: 0,
            end: 10_000_000,
            position: 10 * 10_000_000,
            last_updated: 1,
        };
        assert_eq!(t.valid(), None);
        // zaporne trvanie
        let t = Timeline {
            start: 50,
            end: 10,
            position: 20,
            last_updated: 1,
        };
        assert_eq!(t.valid(), None);
    }

    #[test]
    fn valid_timeline_is_relative_to_start_and_clamped() {
        let t = Timeline {
            start: 10 * 10_000_000,
            end: 190 * 10_000_000,
            position: 70 * 10_000_000,
            last_updated: 1,
        };
        let v = t.valid().unwrap();
        assert_eq!((v.position_s, v.duration_s), (60.0, 180.0));
        // tesne za koncom (v tolerancii) -> orezane na koniec
        let t = Timeline {
            start: 0,
            end: 100 * 10_000_000,
            position: 101 * 10_000_000,
            last_updated: 1,
        };
        assert_eq!(t.valid().unwrap().position_s, 100.0);
    }

    #[test]
    fn buttons_follow_controls() {
        let mut x = s("A", "a", PlayState::Playing, 1);
        // nic nepodporuje: vsetky tri viditelne, ale zakazane; hra -> ikona pauzy
        assert_eq!(
            buttons(&x),
            Buttons {
                prev_enabled: false,
                shows_pause: true,
                toggle_enabled: false,
                next_enabled: false
            }
        );
        x.controls = Controls {
            pause: true,
            next: true,
            ..Default::default()
        };
        assert_eq!(
            buttons(&x),
            Buttons {
                prev_enabled: false,
                shows_pause: true,
                toggle_enabled: true,
                next_enabled: true
            }
        );
        assert_eq!(toggle_command(&x), Some(ToggleCmd::Pause));
        x.state = PlayState::Paused;
        // pozastavene, play nepovolene, toggle nie -> zakazane tlacidlo s ikonou Play
        assert_eq!(toggle_command(&x), None);
        assert!(!buttons(&x).toggle_enabled && !buttons(&x).shows_pause);
        x.controls.toggle = true;
        assert_eq!(toggle_command(&x), Some(ToggleCmd::Toggle));
        assert!(!buttons(&x).shows_pause);
        x.controls.play = true;
        assert_eq!(toggle_command(&x), Some(ToggleCmd::Play));
    }

    #[test]
    fn play_pause_icon_follows_state() {
        let mut x = s("A", "a", PlayState::Playing, 1);
        x.controls.toggle = true;
        assert!(buttons(&x).shows_pause, "hra -> Pause");
        for st in [PlayState::Paused, PlayState::Stopped, PlayState::Opened] {
            x.state = st;
            assert!(!buttons(&x).shows_pause, "{st:?} -> Play");
        }
    }

    #[test]
    fn changing_state_keeps_previous_icon() {
        assert_eq!(
            stable_state(Some(PlayState::Playing), PlayState::Changing),
            PlayState::Playing
        );
        assert_eq!(
            stable_state(Some(PlayState::Paused), PlayState::Changing),
            PlayState::Paused
        );
        assert_eq!(stable_state(None, PlayState::Changing), PlayState::Changing);
        assert_eq!(
            stable_state(Some(PlayState::Playing), PlayState::Paused),
            PlayState::Paused
        );
    }

    #[test]
    fn art_is_bound_to_media_identity() {
        let mut a = s("Spotify.exe", "Song A", PlayState::Playing, 1);
        a.artist = "X".into();
        let mut b = a.clone();
        b.title = "Song B".into();
        let (ka, kb) = (a.media_key(), b.media_key());
        assert_ne!(ka, kb);
        // novy obal sa prevezme
        assert_eq!(resolve_art(None, &ka, Fetched::Image(7)), Some(7));
        // ta ista skladba, nahlad sa necital (len zmena stavu) -> obal ostava
        assert_eq!(
            resolve_art(Some((&ka, Some(7))), &ka, Fetched::NotRead),
            Some(7)
        );
        // zmena skladby bez noveho nahladu -> stary obal sa zahodi
        assert_eq!(
            resolve_art(Some((&ka, Some(7))), &kb, Fetched::NotRead),
            None
        );
        // chybajuci nahlad -> zastupny symbol aj pri rovnakej skladbe
        assert_eq!(
            resolve_art(Some((&ka, Some(7))), &ka, Fetched::Missing),
            None
        );
        // iny album = ina identita (rovnaky nazov)
        let mut c = a.clone();
        c.album = "Live".into();
        assert_ne!(c.media_key(), ka);
        // ina session s rovnakymi metadatami = ina identita
        let mut d = a.clone();
        d.key = "Brave".into();
        assert_ne!(d.media_key(), ka);
    }

    #[test]
    fn display_text_and_spotify_hint() {
        let mut x = s("Spotify.exe", " Song ", PlayState::Playing, 1);
        assert_eq!(x.display_text(), "Song");
        x.artist = "Artist".into();
        assert_eq!(x.display_text(), "Song – Artist");
        assert!(spotify_hint_needed(true, &[]));
        assert!(!spotify_hint_needed(true, &[x.clone()]));
        assert!(!spotify_hint_needed(false, &[]));
        let brave = s("Brave", "Video", PlayState::Playing, 1);
        assert!(spotify_hint_needed(true, &[brave]));
    }

    #[test]
    fn duplicate_app_sessions_have_distinct_keys() {
        let v = vec![
            s("Brave", "One", PlayState::Playing, 1),
            s("Brave#2", "Two", PlayState::Playing, 2),
        ];
        let sel = select(&v, Some("brave"), None, None);
        assert_eq!(sel.chosen.as_deref(), Some("Brave#2"));
        assert_eq!(sel.candidates.len(), 2);
    }

    #[test]
    fn a_playing_timeline_advances_with_the_clock_and_stops_at_the_end() {
        let t = Timeline {
            start: 0,
            end: 100 * 10_000_000,
            position: 10 * 10_000_000,
            last_updated: 1_000_000_000,
        };
        let later = 1_000_000_000 + 5 * 10_000_000;
        assert_eq!(t.valid_at(later, true).unwrap().position_s, 15.0);
        assert_eq!(t.valid_at(later, false).unwrap().position_s, 10.0);
        let much_later = 1_000_000_000 + 500 * 10_000_000;
        assert_eq!(t.valid_at(much_later, true).unwrap().position_s, 100.0);
    }
}
