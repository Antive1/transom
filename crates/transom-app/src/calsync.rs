//! Pripojene kalendare (ICS): stiahnutie na vlastnom vlakne pri starte, kazdych 30 minut a na poziadanie (otvorenie
//! kalendara, ak su data starsie nez 10 minut). Pri chybe ostane posledny uspesne nacitany kalendar a chyba sa ukaze
//! bez adresy (odkaz je tajny udaj, nikdy sa nelogue ani nezobrazuje cely).
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use transom_core::ics::{self, Calendar};

const REFRESH: Duration = Duration::from_secs(30 * 60);
const STALE_ON_OPEN: Duration = Duration::from_secs(10 * 60);
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Stav jedneho kalendara.
#[derive(Clone, Debug, Default)]
pub struct Feed {
    pub url: String,
    /// Posledny uspesne nacitany kalendar.
    pub cal: Option<Arc<Calendar>>,
    /// Posledna chyba (kratky popis bez adresy).
    pub error: Option<String>,
}

enum Msg {
    Urls(Vec<String>),
    Refresh,
}

pub struct IcsSync {
    tx: Sender<Msg>,
    feeds: Arc<Mutex<Vec<Feed>>>,
    last: Arc<Mutex<Option<Instant>>>,
    urls: Vec<String>,
}

fn fetch(url: &str) -> Result<Calendar, String> {
    let bytes = transom_platform::http::get(url, MAX_BYTES, 20_000)?;
    let text = String::from_utf8_lossy(&bytes);
    if !text.contains("BEGIN:VCALENDAR") {
        return Err("nie je to kalendar ICS".into());
    }
    Ok(ics::parse(&text))
}

impl IcsSync {
    pub fn start(urls: Vec<String>, notify: Arc<dyn Fn() + Send + Sync>) -> Option<IcsSync> {
        let (tx, rx) = channel::<Msg>();
        let feeds: Arc<Mutex<Vec<Feed>>> = Arc::new(Mutex::new(Vec::new()));
        let last = Arc::new(Mutex::new(None));
        let (f2, l2) = (Arc::clone(&feeds), Arc::clone(&last));
        let first = urls.clone();
        std::thread::Builder::new()
            .name("transom-ics".into())
            .spawn(move || {
                let mut urls = first;
                loop {
                    // stiahnut vsetky (pomaly zdroj nezdrzi ostatne az tak, lebo kazdy ma casovy limit)
                    for u in urls.clone() {
                        let r = fetch(&u);
                        crate::log(&format!(
                            "kalendar ICS ({}): {}",
                            transom_platform::http::host_of(&u),
                            match &r {
                                Ok(c) => format!("{} udalosti", c.event_count()),
                                Err(e) => e.clone(),
                            }
                        ));
                        if let Ok(mut g) = f2.lock() {
                            let i = match g.iter().position(|f| f.url == u) {
                                Some(i) => i,
                                None => {
                                    g.push(Feed {
                                        url: u.clone(),
                                        ..Feed::default()
                                    });
                                    g.len() - 1
                                }
                            };
                            match r {
                                Ok(c) => {
                                    g[i].cal = Some(Arc::new(c));
                                    g[i].error = None;
                                }
                                Err(e) => g[i].error = Some(e),
                            }
                        }
                        notify();
                    }
                    if let Ok(mut l) = l2.lock() {
                        *l = Some(Instant::now());
                    }
                    match rx.recv_timeout(REFRESH) {
                        Ok(Msg::Urls(u)) => {
                            urls = u;
                            if let Ok(mut g) = f2.lock() {
                                g.retain(|f| urls.contains(&f.url));
                            }
                            notify();
                        }
                        Ok(Msg::Refresh) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    // zlucit dalsie cakajuce spravy
                    while let Ok(m) = rx.try_recv() {
                        if let Msg::Urls(u) = m {
                            urls = u;
                        }
                    }
                }
            })
            .ok()?;
        Some(IcsSync {
            tx,
            feeds,
            last,
            urls,
        })
    }

    pub fn feeds(&self) -> Vec<Feed> {
        self.feeds.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn calendars(&self) -> Vec<Arc<Calendar>> {
        self.feeds().into_iter().filter_map(|f| f.cal).collect()
    }

    /// Nove adresy z nastaveni (stiahnu sa hned).
    pub fn set_urls(&mut self, urls: Vec<String>) {
        if urls != self.urls {
            self.urls = urls.clone();
            let _ = self.tx.send(Msg::Urls(urls));
        }
    }

    /// Otvorenie kalendara: obnovit, ak su data starsie nez 10 minut.
    pub fn on_open(&self) {
        let old = self
            .last
            .lock()
            .map(|l| l.is_none_or(|t| t.elapsed() > STALE_ON_OPEN))
            .unwrap_or(false);
        if old {
            let _ = self.tx.send(Msg::Refresh);
        }
    }
}
