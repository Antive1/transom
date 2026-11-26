//! Cisty (bez WinAPI) stavovy automat ikony v oblasti oznameni: rozhoduje, ktore prikazy shellu (`NIM_ADD`, `NIM_MODIFY`,
//! `NIM_DELETE`) treba poslat. Vdaka tomu sa restart Explorera (`TaskbarCreated`) da overit deterministickym testom s falosnym
//! shellom, bez restartu skutocneho Explorera.
//!
//! Invarianty:
//! - po `TaskbarCreated` sa ikona (ak je ziadana) prida **presne raz**; dalsia `TaskbarCreated` bez novej straty ikonu nepridava
//!   znova na existujucu (shell by `NIM_ADD` odmietol a vznikol by "duch" bez nasho stavu);
//! - ziadana ikona sa nikdy nezabudne: zlyhane pridanie ju necha ziadanu a zopakuje sa pri `Retry`, dalsom `Set` alebo `TaskbarCreated`;
//! - `NIM_ADD`, ktory zlyhal (ikona uz existuje alebo shell vratil `ERROR_TIMEOUT` hoci ikonu pridal), sa overi jednym `NIM_MODIFY`;
//! - `NIM_MODIFY`, ktory zlyhal, lebo ikona zanikla, sa zmeni na `NIM_ADD`;
//! - automat nepozna listy: viditelnost ani pocet listov na neho nemaju vplyv.

/// Co ma ikona ukazovat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraySpec {
    /// Tooltip (v jazyku aplikacie; najviac 127 znakov).
    pub tip: String,
    /// Panel uloh je svetly (vyber variantu ikony).
    pub light_taskbar: bool,
}

/// Prikaz pre shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Add(TraySpec),
    Modify(TraySpec),
    Delete,
}

/// Udalost pre automat (prve styri su vonkajsie, zvysok su vysledky prikazov).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ev {
    /// Ziadana ikona (`None` = ziadna).
    Set(Option<TraySpec>),
    /// Explorer sa restartoval: jeho ikony zanikli.
    TaskbarCreated,
    /// Opakovanie neuspesneho pridania (casovac).
    Retry,
    Added(bool),
    Modified(bool),
}

/// Co posledny prikaz robil (na spracovanie jeho vysledku).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    None,
    Add,
    /// Bezny `NIM_MODIFY` (ikona by mala existovat).
    Modify,
    /// `NIM_MODIFY` po neuspesnom `NIM_ADD`: zistuje, ci ikona uz nahodou existuje.
    Probe,
}

/// Pocet automatickych opakovani po neuspesnom pridani (`Retry`); obnovi sa novym `Set`/`TaskbarCreated`.
pub const RETRIES: u8 = 5;

#[derive(Debug)]
pub struct Model {
    /// Ziadany stav.
    want: Option<TraySpec>,
    /// Co (podla nas) teraz zobrazuje shell.
    shown: Option<TraySpec>,
    pending: Pending,
    retries_left: u8,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    pub const fn new() -> Self {
        Self {
            want: None,
            shown: None,
            pending: Pending::None,
            retries_left: RETRIES,
        }
    }

    /// Je ikona (podla poslednych vysledkov) v oblasti oznameni.
    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    /// Ikona je ziadana, ale nie je pridana a este su k dispozicii opakovania (pre casovac).
    pub fn wants_retry(&self) -> bool {
        self.want.is_some() && self.shown.is_none() && self.retries_left > 0
    }

    /// Spracuje udalost a vrati prikazy, ktore treba vykonat (vysledok kazdeho vratit ako `Added`/`Modified`).
    pub fn on(&mut self, ev: Ev) -> Vec<Cmd> {
        match ev {
            Ev::Set(None) => {
                self.want = None;
                self.pending = Pending::None;
                if self.shown.take().is_some() {
                    vec![Cmd::Delete]
                } else {
                    vec![]
                }
            }
            Ev::Set(Some(s)) => {
                self.want = Some(s.clone());
                self.retries_left = RETRIES;
                match &self.shown {
                    Some(cur) if *cur == s => vec![],
                    Some(_) => self.send(Cmd::Modify(s)),
                    None => self.send(Cmd::Add(s)),
                }
            }
            Ev::TaskbarCreated => {
                // Explorer zanikol aj s ikonou: stary stav neplati
                self.shown = None;
                self.pending = Pending::None;
                self.retries_left = RETRIES;
                match self.want.clone() {
                    Some(s) => self.send(Cmd::Add(s)),
                    None => vec![],
                }
            }
            Ev::Retry => match (self.wants_retry(), self.want.clone()) {
                (true, Some(s)) => {
                    self.retries_left -= 1;
                    self.send(Cmd::Add(s))
                }
                _ => vec![],
            },
            Ev::Added(ok) => {
                let spec = self.want.clone();
                match (self.pending, ok, spec) {
                    (Pending::Add, true, Some(s)) => {
                        self.shown = Some(s);
                        self.pending = Pending::None;
                        vec![]
                    }
                    // pridanie zlyhalo: ikona mozno uz existuje (duplikat) alebo ju shell pridal napriek `ERROR_TIMEOUT`
                    (Pending::Add, false, Some(s)) => {
                        self.pending = Pending::Probe;
                        vec![Cmd::Modify(s)]
                    }
                    _ => {
                        self.pending = Pending::None;
                        vec![]
                    }
                }
            }
            Ev::Modified(ok) => {
                let spec = self.want.clone();
                match (self.pending, ok, spec) {
                    (Pending::Modify | Pending::Probe, true, Some(s)) => {
                        self.shown = Some(s);
                        self.pending = Pending::None;
                        vec![]
                    }
                    // ikona zanikla (napr. Explorer sa restartoval bez nasej spravy): pridat znova
                    (Pending::Modify, false, Some(s)) => {
                        self.shown = None;
                        self.send(Cmd::Add(s))
                    }
                    // overenie po neuspesnom pridani zlyhalo: ikona nie je, zopakuje sa casovacom / dalsou udalostou
                    (Pending::Probe, false, _) => {
                        self.shown = None;
                        self.pending = Pending::None;
                        vec![]
                    }
                    _ => {
                        self.pending = Pending::None;
                        vec![]
                    }
                }
            }
        }
    }

    fn send(&mut self, cmd: Cmd) -> Vec<Cmd> {
        self.pending = match cmd {
            Cmd::Add(_) => Pending::Add,
            Cmd::Modify(_) => Pending::Modify,
            Cmd::Delete => Pending::None,
        };
        vec![cmd]
    }
}

/// Rozhranie k shellu (v testoch falosne).
pub trait Shell {
    fn add(&mut self, spec: &TraySpec) -> bool;
    fn modify(&mut self, spec: &TraySpec) -> bool;
    fn delete(&mut self);
}

/// Spracuje udalost, vykona vsetky prikazy cez `shell` a vrati, ci je ikona po nich v oblasti oznameni.
/// Reťaz je kratka (najviac Modify -> Add -> Probe), takze sa nikdy nezacykli.
pub fn drive<S: Shell>(model: &mut Model, shell: &mut S, ev: Ev) -> bool {
    let mut queue = model.on(ev);
    let mut guard = 0;
    while let Some(cmd) = (!queue.is_empty()).then(|| queue.remove(0)) {
        guard += 1;
        if guard > 8 {
            break; // poistka: invarianty to nedovolia, ale slucka v shell prikazoch by bola horsia nez neuspech
        }
        let next = match cmd {
            Cmd::Add(s) => model.on(Ev::Added(shell.add(&s))),
            Cmd::Modify(s) => model.on(Ev::Modified(shell.modify(&s))),
            Cmd::Delete => {
                shell.delete();
                vec![]
            }
        };
        queue.extend(next);
    }
    model.is_shown()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn spec(tip: &str, light: bool) -> TraySpec {
        TraySpec {
            tip: tip.into(),
            light_taskbar: light,
        }
    }

    /// Falosny shell: drzi ikony ako skutocny (duplicitny `NIM_ADD` zlyha, `NIM_MODIFY` bez ikony zlyha, restart ikony zmaze).
    #[derive(Default)]
    struct Fake {
        /// ikona v oblasti oznameni (jedna identita hwnd+uID)
        icon: Option<TraySpec>,
        log: Vec<String>,
        /// shell este nema oblast oznameni (start pred panelom uloh): `add` zlyha bez pridania
        not_ready: bool,
        /// prvy `add` ikonu prida, ale vrati `false` (ERROR_TIMEOUT)
        add_times_out_once: bool,
        adds: u32,
        deletes: u32,
        /// maximalny pocet sucasne zobrazenych ikon, aky sa kedy pozoroval (duplikaty)
        max_icons: u32,
        counters: HashMap<&'static str, u32>,
    }

    impl Fake {
        fn explorer_restart(&mut self) {
            self.icon = None;
            self.log.push("explorer restart".into());
        }
        fn note(&mut self) {
            self.max_icons = self.max_icons.max(u32::from(self.icon.is_some()));
        }
    }

    impl Shell for Fake {
        fn add(&mut self, s: &TraySpec) -> bool {
            *self.counters.entry("add").or_default() += 1;
            self.log.push(format!("add {}", s.tip));
            if self.not_ready {
                return false;
            }
            if self.icon.is_some() {
                return false; // duplicit: shell NIM_ADD odmietne
            }
            self.icon = Some(s.clone());
            self.adds += 1;
            self.note();
            if self.add_times_out_once {
                self.add_times_out_once = false;
                return false;
            }
            true
        }
        fn modify(&mut self, s: &TraySpec) -> bool {
            self.log.push(format!("modify {}", s.tip));
            match &mut self.icon {
                Some(i) => {
                    *i = s.clone();
                    true
                }
                None => false,
            }
        }
        fn delete(&mut self) {
            self.log.push("delete".into());
            self.icon = None;
            self.deletes += 1;
        }
    }

    #[test]
    fn explorer_restart_readds_the_icon_exactly_once_each_time() {
        let (mut m, mut sh) = (Model::new(), Fake::default());
        let s = spec("transom", false);
        assert!(drive(&mut m, &mut sh, Ev::Set(Some(s.clone()))));
        for n in 1..=5 {
            sh.explorer_restart();
            assert!(sh.icon.is_none());
            assert!(drive(&mut m, &mut sh, Ev::TaskbarCreated), "restart {n}");
            assert_eq!(sh.icon.as_ref(), Some(&s));
            assert_eq!(sh.adds, n + 1, "presne jedno pridanie na restart");
        }
        assert_eq!(sh.max_icons, 1, "nikdy dve ikony");
        assert_eq!(
            sh.counters["add"], 6,
            "ziadny zbytocny NIM_ADD na existujucu ikonu"
        );
    }

    #[test]
    fn the_taskbar_created_broadcast_reaching_the_model_twice_never_creates_a_duplicate() {
        // pri viacerych oknach/listoch moze sprava dojst viackrat: druhe spracovanie ikonu neprida na existujucu
        let (mut m, mut sh) = (Model::new(), Fake::default());
        let s = spec("transom", true);
        drive(&mut m, &mut sh, Ev::Set(Some(s.clone())));
        sh.explorer_restart();
        for _ in 0..3 {
            assert!(drive(&mut m, &mut sh, Ev::TaskbarCreated));
        }
        assert_eq!(sh.adds, 2, "povodne pridanie + jedno po restarte");
        assert_eq!(sh.max_icons, 1);
        assert_eq!(sh.icon.as_ref(), Some(&s));
        assert!(m.is_shown());
    }

    #[test]
    fn a_hidden_bar_keeps_the_icon_through_a_restart_and_the_model_knows_nothing_about_bars() {
        // skryta lista = ikona je jedina cesta spat: `want` zostava, TaskbarCreated ju prida bez ohladu na listy
        let (mut m, mut sh) = (Model::new(), Fake::default());
        let s = spec("transom (hidden)", false);
        drive(&mut m, &mut sh, Ev::Set(Some(s.clone())));
        sh.explorer_restart();
        assert!(drive(&mut m, &mut sh, Ev::TaskbarCreated));
        assert_eq!(sh.icon, Some(s));
        // jedine prikazy shellu boli add, (restart), add: ziadne modify/delete okolo viditelnosti
        assert_eq!(
            sh.log,
            [
                "add transom (hidden)",
                "explorer restart",
                "add transom (hidden)"
            ]
        );
    }

    #[test]
    fn a_failed_add_keeps_the_wish_and_the_retry_succeeds_once_the_shell_is_ready() {
        let (mut m, mut sh) = (
            Model::new(),
            Fake {
                not_ready: true,
                ..Default::default()
            },
        );
        let s = spec("transom", false);
        assert!(
            !drive(&mut m, &mut sh, Ev::Set(Some(s.clone()))),
            "shell nie je pripraveny"
        );
        assert!(m.wants_retry(), "ikona ostava ziadana");
        assert!(!drive(&mut m, &mut sh, Ev::Retry));
        sh.not_ready = false;
        assert!(drive(&mut m, &mut sh, Ev::Retry));
        assert_eq!(sh.icon, Some(s));
        assert!(!m.wants_retry());
        assert_eq!(sh.max_icons, 1);
        // retry bez ziadanej ikony alebo pri uz pridanej nic nerobi
        let before = sh.log.len();
        assert!(drive(&mut m, &mut sh, Ev::Retry));
        assert_eq!(sh.log.len(), before);
    }

    #[test]
    fn a_failed_add_during_taskbar_created_is_retried_by_the_next_event() {
        let (mut m, mut sh) = (Model::new(), Fake::default());
        let s = spec("transom", false);
        drive(&mut m, &mut sh, Ev::Set(Some(s.clone())));
        sh.explorer_restart();
        sh.not_ready = true; // novy Explorer este nema oblast oznameni
        assert!(!drive(&mut m, &mut sh, Ev::TaskbarCreated));
        assert!(m.wants_retry());
        sh.not_ready = false;
        assert!(drive(&mut m, &mut sh, Ev::Retry));
        assert_eq!(sh.icon, Some(s));
    }

    #[test]
    fn retries_are_bounded_and_renewed_by_a_new_request() {
        let (mut m, mut sh) = (
            Model::new(),
            Fake {
                not_ready: true,
                ..Default::default()
            },
        );
        let s = spec("transom", false);
        drive(&mut m, &mut sh, Ev::Set(Some(s.clone())));
        let mut tries = 0;
        while m.wants_retry() {
            drive(&mut m, &mut sh, Ev::Retry);
            tries += 1;
            assert!(tries <= RETRIES as usize + 1, "nekonecne opakovanie");
        }
        assert_eq!(tries, RETRIES as usize);
        // dalsi Set s rovnakym specom (napr. zmena nastavenia) znova otvori opakovania
        drive(&mut m, &mut sh, Ev::Set(Some(s)));
        assert!(m.wants_retry());
    }

    #[test]
    fn an_add_that_reports_failure_but_created_the_icon_is_recognised_by_the_probe() {
        let (mut m, mut sh) = (
            Model::new(),
            Fake {
                add_times_out_once: true,
                ..Default::default()
            },
        );
        let s = spec("transom", false);
        assert!(
            drive(&mut m, &mut sh, Ev::Set(Some(s.clone()))),
            "ERROR_TIMEOUT: ikona existuje"
        );
        assert_eq!(sh.adds, 1, "nepridava sa druha");
        assert_eq!(sh.log, ["add transom", "modify transom"]);
        assert_eq!(sh.max_icons, 1);
    }

    #[test]
    fn modify_on_a_vanished_icon_becomes_an_add() {
        let (mut m, mut sh) = (Model::new(), Fake::default());
        drive(&mut m, &mut sh, Ev::Set(Some(spec("a", false))));
        sh.explorer_restart(); // sprava TaskbarCreated este nedosla
        assert!(
            drive(&mut m, &mut sh, Ev::Set(Some(spec("b", true)))),
            "self-heal"
        );
        assert_eq!(sh.icon, Some(spec("b", true)));
        assert_eq!(sh.max_icons, 1);
    }

    #[test]
    fn theme_or_tooltip_change_modifies_and_the_same_spec_does_nothing() {
        let (mut m, mut sh) = (Model::new(), Fake::default());
        let a = spec("transom", false);
        drive(&mut m, &mut sh, Ev::Set(Some(a.clone())));
        let n = sh.log.len();
        drive(&mut m, &mut sh, Ev::Set(Some(a.clone())));
        assert_eq!(
            sh.log.len(),
            n,
            "rovnaka specifikacia = ziadne volanie shellu"
        );
        let b = TraySpec {
            light_taskbar: true,
            ..a
        };
        assert!(drive(&mut m, &mut sh, Ev::Set(Some(b.clone()))));
        assert_eq!(sh.adds, 1, "zmena temy je modify, nie add");
        assert_eq!(sh.icon, Some(b));
    }

    #[test]
    fn removal_deletes_exactly_once_and_a_later_restart_adds_nothing() {
        let (mut m, mut sh) = (Model::new(), Fake::default());
        drive(&mut m, &mut sh, Ev::Set(Some(spec("transom", false))));
        assert!(!drive(&mut m, &mut sh, Ev::Set(None)));
        assert!(
            !drive(&mut m, &mut sh, Ev::Set(None)),
            "druhe odstranenie je no-op"
        );
        assert_eq!(sh.deletes, 1);
        sh.explorer_restart();
        assert!(!drive(&mut m, &mut sh, Ev::TaskbarCreated));
        assert!(
            sh.icon.is_none(),
            "bez ziadanej ikony sa po restarte nic nepridava"
        );
        assert!(!m.wants_retry());
    }

    #[test]
    fn an_unwanted_icon_never_triggers_any_shell_call() {
        let (mut m, mut sh) = (Model::new(), Fake::default());
        drive(&mut m, &mut sh, Ev::TaskbarCreated);
        drive(&mut m, &mut sh, Ev::Retry);
        drive(&mut m, &mut sh, Ev::Set(None));
        assert!(sh.log.is_empty());
    }
}
