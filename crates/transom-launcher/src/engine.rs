//! Postupne hladanie na pozadi: jedno vlakno, dotazy sa zluca (debounce), zastarany dotaz sa zrusi hned a vysledky sa publikuju
//! po kazdom zdroji (rychle zdroje najprv, subory neskor). UI vlakno nikdy necaka; vlakno nic nerobi, kym nepride dotaz
//! (ziadne dotazovanie ani casovac naprazdno).
use crate::rank::{rank, Hit};
use crate::sources::Source;
use crate::Item;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

/// Zrusenie jedneho dotazu: zastarava, ked pride novsi.
#[derive(Clone)]
pub struct Cancel {
    latest: Arc<AtomicU64>,
    mine: u64,
}

impl Cancel {
    pub fn is_cancelled(&self) -> bool {
        self.latest.load(Ordering::Relaxed) != self.mine
    }

    /// Nezrusitelny token (testy a jednorazove volania).
    pub fn never() -> Cancel {
        Cancel {
            latest: Arc::new(AtomicU64::new(0)),
            mine: 0,
        }
    }
}

/// Doba, pocas ktorej sa cakaju dalsie znaky, nez sa zacne hladat.
pub const DEBOUNCE_MS: u64 = 60;

/// Posledne vysledky publikovane pre dotaz `generation` (`done` = vsetky zdroje dobehli).
pub struct Update {
    pub generation: u64,
    pub hits: Vec<Hit>,
    pub done: bool,
}

/// Jeden zdroj na vlastnom vlakne (chyba zdroja = prazdny zoznam). Izolacia **paniky** plati len v zostaveniach s odvijanim
/// (testy, dev): release ma `panic = "abort"` a panika zdroja zhodi proces. Zdroje preto nesmu panikovat (docs/PLAN.md 8.24).
fn run_source(s: &Arc<dyn Source>, query: &str, cancel: &Cancel) -> Vec<Item> {
    let (s2, q, c) = (Arc::clone(s), query.to_string(), cancel.clone());
    std::thread::scope(|sc| sc.spawn(move || s2.search(&q, &c)).join()).unwrap_or_default()
}

pub struct Engine {
    latest: Arc<AtomicU64>,
    tx: Sender<(u64, String)>,
}

impl Engine {
    pub fn start(
        sources: Vec<Arc<dyn Source>>,
        max_results: usize,
        recents: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
        publish: Arc<dyn Fn(Update) + Send + Sync>,
    ) -> Engine {
        let latest = Arc::new(AtomicU64::new(0));
        let (tx, rx) = channel::<(u64, String)>();
        let l2 = Arc::clone(&latest);
        let _ = std::thread::Builder::new()
            .name("transom-launcher".into())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    // zluc dotazy: pocka kratko a vezme posledny
                    loop {
                        match rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS)) {
                            Ok(next) => job = next,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    let (gen, query) = job;
                    if l2.load(Ordering::Relaxed) != gen {
                        continue;
                    }
                    let cancel = Cancel {
                        latest: Arc::clone(&l2),
                        mine: gen,
                    };
                    let recent = recents();
                    let fast: Vec<Arc<dyn Source>> =
                        sources.iter().filter(|s| !s.slow()).cloned().collect();
                    let slow: Vec<Arc<dyn Source>> =
                        sources.iter().filter(|s| s.slow()).cloned().collect();
                    let mut items: Vec<Item> = Vec::new();
                    let nf = fast.len();
                    for (i, s) in fast.into_iter().enumerate() {
                        if cancel.is_cancelled() {
                            break;
                        }
                        items.extend(run_source(&s, &query, &cancel));
                        if cancel.is_cancelled() {
                            break;
                        }
                        publish(Update {
                            generation: gen,
                            hits: rank(&query, &items, max_results, &recent),
                            done: slow.is_empty() && i + 1 == nf,
                        });
                    }
                    if cancel.is_cancelled() {
                        continue;
                    }
                    if nf == 0 && slow.is_empty() {
                        publish(Update {
                            generation: gen,
                            hits: Vec::new(),
                            done: true,
                        });
                    }
                    if !slow.is_empty() {
                        // pomale zdroje (index suborov) bezia mimo slucky: studeny `Execute` nemoze zdrzat novsi dotaz
                        let publish = Arc::clone(&publish);
                        let query = query.clone();
                        let _ = std::thread::Builder::new()
                            .name("transom-launcher-slow".into())
                            .spawn(move || {
                                let n = slow.len();
                                for (i, s) in slow.into_iter().enumerate() {
                                    if cancel.is_cancelled() {
                                        return;
                                    }
                                    items.extend(run_source(&s, &query, &cancel));
                                    if cancel.is_cancelled() {
                                        return;
                                    }
                                    publish(Update {
                                        generation: gen,
                                        hits: rank(&query, &items, max_results, &recent),
                                        done: i + 1 == n,
                                    });
                                }
                            });
                    }
                }
            });
        Engine { latest, tx }
    }

    /// Novy dotaz: zrusi predchadzajuci a vrati jeho cislo.
    pub fn query(&self, text: &str) -> u64 {
        let g = self.latest.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send((g, text.to_string()));
        g
    }

    /// Zrusi rozbehnute hladanie bez noveho dotazu (zatvorenie okna).
    pub fn cancel(&self) {
        self.latest.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // zahodeny beh (zmena nastaveni) uz nesmie publikovat vysledky
        self.latest.fetch_add(1, Ordering::Relaxed);
    }
}
