//! Ostrov v strede listy: kapsula v strede (v pokoji logo a nazov transom, pocas aktivity jej ikona a nazov), obsah
//! aktivity vlavo a vpravo od nej priamo v liste. Hra hudba -> pri novej skladbe nazov vlavo a interpret vpravo, potom
//! obal vlavo a ekvalizer vpravo. Pracuje AI (Claude Code) -> okno limitu vlavo a hodnota vpravo (tyzdenny limit, ked
//! sa blizi k hranici). Kratke oznamenia (snimka obrazovky, ulozena schranka, koniec casovaca, udalost) ikonou a
//! nazvom v kapsule; klik pocas oznamenia otvori, co k nemu patri. Tu je cisty stav (co ukazat a kedy).
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Ako dlho trva oznamenie (a kym ho klik otvori).
pub const NOTICE: Duration = Duration::from_secs(3);
/// Nazov a interpret novej skladby.
pub const PEEK: Duration = Duration::from_secs(8);
/// Hudba po pozastaveni este chvilu (prechod medzi skladbami nebliká).
pub const MUSIC_GRACE: Duration = Duration::from_secs(3);

/// Co sa stane po kliknuti na oznamenie.
#[derive(Clone, Debug, PartialEq)]
pub enum NoticeAct {
    /// Priecinok so snimkami (subor oznaceny).
    Shot(PathBuf),
    /// Nahlad schranky.
    Clipboard,
    /// Panel casovaca.
    Timer,
    /// Kalendar.
    Calendar,
    /// Odlozene subory.
    Stash,
    /// Limity AI (karta AI v menu).
    Ai,
}

/// Kratke oznamenie (ikona a nazov v strede, pod nimi jeden riadok).
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// Znak ikony (Segoe Fluent Icons).
    pub icon: char,
    pub title: String,
    pub sub: String,
    pub act: Option<NoticeAct>,
}

/// Limit AI v ostrove (z posledneho zaznamu statusline).
#[derive(Clone, Debug, PartialEq)]
pub struct AiLive {
    /// "Claude" alebo "Codex".
    pub name: String,
    /// "5 h" alebo "Tyzden".
    pub window: String,
    /// "34 % využité" / "66 % zostáva" (podla rezimu kruhu v liste); `None` = zdroj limit nehlasi.
    pub value: Option<String>,
    /// Malo kapacity (tyzdenny limit pri hranici): farba upozornenia.
    pub alert: bool,
    /// Stav agenta (farba bodky ako na klavesnici Codex Agent Keys).
    pub state: AiState,
}

/// Stav agenta (ako klavesnica Codex Agent Keys): premysla, potrebuje povolenie / odpoved, hotovo a neprecitane,
/// chyba, necinny.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiState {
    Thinking,
    Waiting,
    Unread,
    Error,
    Idle,
}

impl AiState {
    /// Prave pracuje alebo potrebuje pouzivatela: ostrov ukaze logo aj limit. Inak len logo s bodkou.
    pub fn busy(self) -> bool {
        matches!(self, AiState::Thinking | AiState::Waiting)
    }
}

/// Hudba v ostrove.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicLive {
    /// Nazov aplikacie (Spotify, Brave...).
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

/// Co ostrov prave ukazuje.
#[derive(Clone, Debug, PartialEq)]
pub enum Compact {
    Notice(Notice),
    Ai(AiLive),
    /// Nova skladba: nazov vlavo, interpret vpravo.
    Peek(MusicLive),
    /// Obal vlavo, ekvalizer vpravo.
    Music(MusicLive),
    /// Hra hudba a zaroven pracuje AI: hudba (obal, nazov, interpret) vlavo, logo AI a limit vpravo.
    MusicAi(MusicLive, AiLive),
}

#[derive(Default)]
pub struct Island {
    notice: Option<(Notice, Instant)>,
    /// Skladba, ktorej nazov sa uz ukazal (media_key), a dokedy sa ukazuje.
    track: Option<String>,
    peek_until: Option<Instant>,
    /// Posledne hranie (pozastavena hudba este chvilu ostava).
    played_at: Option<Instant>,
}

impl Island {
    /// Nove oznamenie (nahradi predosle).
    pub fn notify(&mut self, n: Notice, now: Instant) {
        self.notice = Some((n, now));
    }

    /// Oznamenie este bezi?
    pub fn notice(&self, now: Instant) -> Option<&Notice> {
        self.notice
            .as_ref()
            .filter(|(_, t)| now.duration_since(*t) < NOTICE)
            .map(|(n, _)| n)
    }

    /// Oznamenie sa pouzilo (klik) alebo zrusilo.
    pub fn clear_notice(&mut self) {
        self.notice = None;
    }

    /// Stav prehravania: zapamata novu skladbu (nazov sa ukaze) a cas posledneho hrania. `key` = identita skladby.
    pub fn music(&mut self, key: Option<&str>, playing: bool, now: Instant) {
        if playing {
            self.played_at = Some(now);
            if let Some(k) = key {
                if self.track.as_deref() != Some(k) {
                    self.track = Some(k.to_string());
                    self.peek_until = Some(now + PEEK);
                }
            }
        }
    }

    /// Co ukazat (prednost: oznamenie, AI, nova skladba, hudba). `None` = nic.
    pub fn pick(
        &self,
        now: Instant,
        music: Option<MusicLive>,
        ai: Option<AiLive>,
    ) -> Option<Compact> {
        if let Some(n) = self.notice(now) {
            return Some(Compact::Notice(n.clone()));
        }
        let m = music.filter(|m| {
            m.playing
                || self
                    .played_at
                    .is_some_and(|t| now.duration_since(t) < MUSIC_GRACE)
        });
        // hudba je hlavna; pracujuca AI sa ukaze vedla nej, necinna (len logo s bodkou) len bez hudby
        let Some(m) = m else {
            return ai.map(Compact::Ai);
        };
        if let Some(a) = ai.filter(|a| a.state.busy()) {
            return Some(Compact::MusicAi(m, a));
        }
        if m.playing && self.peek_until.is_some_and(|t| now < t) {
            return Some(Compact::Peek(m));
        }
        Some(Compact::Music(m))
    }

    /// Najblizsi okamih, ked sa moze zmenit vyber (koniec oznamenia, nazvu skladby, dobehu hudby).
    pub fn next_change(&self, now: Instant) -> Option<Instant> {
        [
            self.notice.as_ref().map(|(_, t)| *t + NOTICE),
            self.peek_until,
            self.played_at.map(|t| t + MUSIC_GRACE),
        ]
        .into_iter()
        .flatten()
        .filter(|t| *t > now)
        .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(playing: bool) -> MusicLive {
        MusicLive {
            app: "Spotify".into(),
            title: "T".into(),
            artist: "A".into(),
            playing,
        }
    }

    fn ai() -> AiLive {
        AiLive {
            name: "Claude".into(),
            state: AiState::Thinking,
            window: "5 h".into(),
            value: Some("34 %".into()),
            alert: false,
        }
    }

    #[test]
    fn an_idle_agent_stays_as_its_logo_alone_and_never_beside_music() {
        let now = Instant::now();
        let isl = Island::default();
        let idle = AiLive {
            state: AiState::Idle,
            ..ai()
        };
        // bez hudby: necinny agent ostava (len logo s bodkou)
        assert_eq!(
            isl.pick(now, None, Some(idle.clone())),
            Some(Compact::Ai(idle.clone()))
        );
        // s hudbou: hudba sama; pracujuci agent vedla nej
        assert!(matches!(
            isl.pick(now, Some(song(true)), Some(idle)),
            Some(Compact::Music(_))
        ));
        assert!(matches!(
            isl.pick(now, Some(song(true)), Some(ai())),
            Some(Compact::MusicAi(..))
        ));
        for st in [AiState::Thinking, AiState::Waiting] {
            assert!(st.busy());
        }
        for st in [AiState::Unread, AiState::Error, AiState::Idle] {
            assert!(!st.busy());
        }
    }

    #[test]
    fn nothing_when_idle_and_priorities_notice_ai_peek_music() {
        let t0 = Instant::now();
        let mut i = Island::default();
        assert_eq!(i.pick(t0, None, None), None);
        assert_eq!(
            i.pick(t0, Some(song(false)), None),
            None,
            "pozastavena, nikdy nehrala"
        );
        i.music(Some("k1"), true, t0);
        assert!(matches!(
            i.pick(t0, Some(song(true)), None),
            Some(Compact::Peek(_))
        ));
        let later = t0 + PEEK + Duration::from_millis(1);
        i.music(Some("k1"), true, later);
        assert!(matches!(
            i.pick(later, Some(song(true)), None),
            Some(Compact::Music(_))
        ));
        assert!(matches!(
            i.pick(later, Some(song(true)), Some(ai())),
            Some(Compact::MusicAi(..))
        ));
        assert!(matches!(
            i.pick(later, None, Some(ai())),
            Some(Compact::Ai(_))
        ));
        i.notify(
            Notice {
                icon: 'x',
                title: "n".into(),
                sub: String::new(),
                act: Some(NoticeAct::Timer),
            },
            later,
        );
        assert!(matches!(
            i.pick(later, Some(song(true)), Some(ai())),
            Some(Compact::Notice(_))
        ));
        let gone = later + NOTICE;
        assert!(i.notice(gone).is_none(), "oznamenie trva 3 s");
        assert!(matches!(
            i.pick(gone, Some(song(true)), Some(ai())),
            Some(Compact::MusicAi(..))
        ));
    }

    #[test]
    fn a_new_track_peeks_once_and_paused_music_lingers_briefly() {
        let t0 = Instant::now();
        let mut i = Island::default();
        i.music(Some("k1"), true, t0);
        let t1 = t0 + PEEK;
        // ta ista skladba znova (dalsia aktualizacia relacie): nazov sa neukaze znova
        i.music(Some("k1"), true, t1);
        assert!(matches!(
            i.pick(t1, Some(song(true)), None),
            Some(Compact::Music(_))
        ));
        // nova skladba
        i.music(Some("k2"), true, t1);
        assert!(matches!(
            i.pick(t1, Some(song(true)), None),
            Some(Compact::Peek(_))
        ));
        // pozastavene: hudba este chvilu (bez nazvu), potom nic
        let p = t1 + Duration::from_secs(1);
        assert!(matches!(
            i.pick(p, Some(song(false)), None),
            Some(Compact::Music(_))
        ));
        assert_eq!(i.pick(t1 + MUSIC_GRACE, Some(song(false)), None), None);
        assert_eq!(i.next_change(t1), Some(t1 + MUSIC_GRACE));
    }
}
