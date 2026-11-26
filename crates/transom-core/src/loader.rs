//! Obrazky nacitavane na pozadi (miniatury snimok obrazovky, ikony vysledkov hladania): cista logika bez Win32.
//!
//! * [`ImgCache`]: LRU cache **ohranicena v bajtoch** (nie len poctom poloziek) so zapamatanymi neuspechmi, aby sa chybny
//!   subor nedekodoval znova pri kazdom kresleni.
//! * [`Loader`]: pevny pocet pracovnych vlakien, ktore dekoduju obrazky mimo UI vlakna. UI vlakno **nikdy nic nedekoduje
//!   ani necaka**: `get` je len rychle nahliadnutie do cache a `want` nahradi cakajuci zoznam poziadaviek (vyhrava posledna
//!   mnozina, zastarane poziadavky, ktore sa este nezacali, sa zahodia, takze rychle pisanie ani posuvanie nevytvara ziadnu
//!   frontu). Dekodovanie, ktore uz bezi, sa dokonci a jeho vysledok sa po `clear` zahodi.
//!
//! Ak sa jedno vlakno zasekne vo volani systemu (napr. nedostupny sietovy disk), UI ostava citlive a ostatne vlakna pokracuju;
//! volajuci preto sietove cesty vobec neposiela (viz `transom-tools::lane::is_remote_path`).
use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// Dekodovany obrazok: **nepremultiplikovane** RGBA, riadky zhora nadol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<[u8; 4]>,
}

impl Bitmap {
    /// `None` pri nulovom rozmere alebo nespravnom pocte pixelov.
    pub fn new(w: u32, h: u32, rgba: Vec<[u8; 4]>) -> Option<Bitmap> {
        (w > 0 && h > 0 && rgba.len() as u64 == u64::from(w) * u64::from(h)).then_some(Bitmap {
            w,
            h,
            rgba,
        })
    }

    pub fn bytes(&self) -> usize {
        self.rgba.len() * 4
    }
}

/// Vysledok nahliadnutia do cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Ready(Arc<Bitmap>),
    /// Dekodovanie zlyhalo (neprecita sa znova, kym sa cache nevymaze).
    Failed,
    /// Caka vo fronte alebo prave bezi.
    Pending,
    /// Nic sa o tom nevie (nikto o to nepoziadal).
    Absent,
}

enum Slot {
    Ready(Arc<Bitmap>),
    Failed,
}

/// LRU cache ohranicena v bajtoch; najstarsie pouzite polozky sa vyhadzuju, kym sa nezmesti. Jedna polozka vacsia nez
/// rozpocet sa nikdy neuklada (rozpocet je pevny strop, nie odporucanie).
pub struct ImgCache<K> {
    budget: usize,
    max_failed: usize,
    bytes: usize,
    tick: u64,
    map: HashMap<K, (Slot, u64)>,
    failed: usize,
}

impl<K: Hash + Eq + Clone> ImgCache<K> {
    pub fn new(budget_bytes: usize, max_failed: usize) -> Self {
        ImgCache {
            budget: budget_bytes,
            max_failed,
            bytes: 0,
            tick: 0,
            map: HashMap::new(),
            failed: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
        self.failed = 0;
    }

    /// Nahliadnutie s oznacenim pouzitia (LRU).
    pub fn get(&mut self, k: &K) -> Option<Lookup> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|(s, used)| {
            *used = t;
            match s {
                Slot::Ready(b) => Lookup::Ready(b.clone()),
                Slot::Failed => Lookup::Failed,
            }
        })
    }

    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    /// Ulozi vysledok (`None` = zlyhanie). Obrazok vacsi nez rozpocet sa zapamata ako zlyhanie.
    pub fn insert(&mut self, k: K, img: Option<Arc<Bitmap>>) {
        self.tick += 1;
        self.remove(&k);
        match img {
            Some(b) if b.bytes() <= self.budget => {
                self.bytes += b.bytes();
                self.map.insert(k, (Slot::Ready(b), self.tick));
            }
            _ => {
                self.failed += 1;
                self.map.insert(k, (Slot::Failed, self.tick));
            }
        }
        self.evict();
    }

    fn remove(&mut self, k: &K) {
        if let Some((s, _)) = self.map.remove(k) {
            match s {
                Slot::Ready(b) => self.bytes -= b.bytes(),
                Slot::Failed => self.failed -= 1,
            }
        }
    }

    fn evict(&mut self) {
        while self.bytes > self.budget || self.failed > self.max_failed {
            let over_bytes = self.bytes > self.budget;
            let victim = self
                .map
                .iter()
                .filter(|(_, (s, _))| match s {
                    Slot::Ready(_) => over_bytes,
                    Slot::Failed => !over_bytes,
                })
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone());
            match victim {
                Some(v) => self.remove(&v),
                None => break,
            }
        }
    }

    /// Zabudne zapamatene zlyhania (obrazky ostanu): pri dalsom `want` sa skusia znova.
    pub fn forget_failed(&mut self) {
        let dead: Vec<K> = self
            .map
            .iter()
            .filter(|(_, (s, _))| matches!(s, Slot::Failed))
            .map(|(k, _)| k.clone())
            .collect();
        for k in dead {
            self.remove(&k);
        }
    }

    /// Ponecha len polozky, pre ktore `keep` plati (zneplatnenie napr. po zmene suboru alebo DPI).
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        let drop: Vec<K> = self.map.keys().filter(|k| !keep(k)).cloned().collect();
        for k in drop {
            self.remove(&k);
        }
    }
}

/// Dekoder jednej poziadavky; `init` sa vola raz na kazdom pracovnom vlakne (napr. inicializacia COM) a jeho vysledok sa
/// zahodi na konci vlakna.
pub trait Decode<K>: Send + Sync + 'static {
    fn init(&self) -> Box<dyn Any> {
        Box::new(())
    }
    fn decode(&self, key: &K) -> Option<Bitmap>;
}

#[derive(Clone, Copy, Debug)]
pub struct LoaderCfg {
    pub name: &'static str,
    pub threads: usize,
    pub budget_bytes: usize,
    /// Najviac cakajucich poziadaviek (dlhsi zoznam sa orezava od konca = od najmenej dolezitych).
    pub max_queue: usize,
    pub max_failed: usize,
}

struct State<K> {
    queue: VecDeque<K>,
    inflight: HashSet<K>,
    cache: ImgCache<K>,
    stop: bool,
    /// Zvysuje sa pri `clear`: vysledok poziadavky, ktora zacala skor, sa zahodi.
    epoch: u64,
}

type Shared<K> = Arc<(Mutex<State<K>>, Condvar)>;

fn lock<K>(s: &Shared<K>) -> MutexGuard<'_, State<K>> {
    s.0.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Loader<K: Hash + Eq + Clone + Send + 'static> {
    st: Shared<K>,
    max_queue: usize,
}

impl<K: Hash + Eq + Clone + Send + 'static> Loader<K> {
    /// Spusti pracovne vlakna. `wake` sa vola po kazdom dokonceni (z pracovneho vlakna); volajuci ho ma zlucovat
    /// (napr. jedna sprava na prekreslenie).
    pub fn start(
        cfg: LoaderCfg,
        dec: Arc<dyn Decode<K>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let st: Shared<K> = Arc::new((
            Mutex::new(State {
                queue: VecDeque::new(),
                inflight: HashSet::new(),
                cache: ImgCache::new(cfg.budget_bytes, cfg.max_failed),
                stop: false,
                epoch: 0,
            }),
            Condvar::new(),
        ));
        for i in 0..cfg.threads.max(1) {
            let (st2, dec2, wake2) = (st.clone(), dec.clone(), wake.clone());
            let _ = std::thread::Builder::new()
                .name(format!("{}-{i}", cfg.name))
                .spawn(move || worker(st2, dec2, wake2));
        }
        Loader {
            st,
            max_queue: cfg.max_queue,
        }
    }

    /// Rychle nahliadnutie (nikdy nedekoduje ani necaka na dekoder).
    pub fn get(&self, k: &K) -> Lookup {
        let mut s = lock(&self.st);
        if let Some(l) = s.cache.get(k) {
            return l;
        }
        if s.inflight.contains(k) || s.queue.contains(k) {
            Lookup::Pending
        } else {
            Lookup::Absent
        }
    }

    /// Nahradi cakajuci zoznam: `keys` v poradi dolezitosti (viditelne riadky prve), najviac `max_queue`. Co je uz v cache alebo
    /// bezi, sa nepridava; co bolo cakajuce a v zozname nie je, sa zahodi.
    pub fn want(&self, keys: &[K]) {
        let mut s = lock(&self.st);
        let mut q: VecDeque<K> = VecDeque::new();
        for k in keys {
            if q.len() >= self.max_queue {
                break;
            }
            if s.cache.contains(k) || s.inflight.contains(k) || q.contains(k) {
                continue;
            }
            q.push_back(k.clone());
        }
        let changed = q != s.queue;
        s.queue = q;
        drop(s);
        if changed {
            self.st.1.notify_all();
        }
    }

    /// Zahodi cache aj cakajuce poziadavky (zneplatnenie: zmena DPI, nastaveni). Bezace dekodovania sa dokoncia, ale ich
    /// vysledok sa zahodi.
    pub fn clear(&self) {
        let mut s = lock(&self.st);
        s.cache.clear();
        s.queue.clear();
        s.epoch += 1;
    }

    /// Zabudne zapamatene zlyhania (napr. pri novom otvoreni okna): pri dalsej poziadavke sa skusia znova.
    pub fn forget_failed(&self) {
        lock(&self.st).cache.forget_failed();
    }

    /// Zahodi z cache polozky, pre ktore `keep` neplati (napr. zmeneny subor).
    pub fn retain(&self, keep: impl FnMut(&K) -> bool) {
        lock(&self.st).cache.retain(keep);
    }

    pub fn cached_bytes(&self) -> usize {
        lock(&self.st).cache.bytes()
    }

    pub fn cached_len(&self) -> usize {
        lock(&self.st).cache.len()
    }

    /// Cakajuce + bezace poziadavky (testy a diagnostika).
    pub fn pending_len(&self) -> usize {
        let s = lock(&self.st);
        s.queue.len() + s.inflight.len()
    }
}

impl<K: Hash + Eq + Clone + Send + 'static> Drop for Loader<K> {
    /// Vlakna sa neprepajaju (mohli by cakat na pomaly system); ukoncia sa po dokonceni rozpracovaneho dekodovania.
    fn drop(&mut self) {
        lock(&self.st).stop = true;
        self.st.1.notify_all();
    }
}

fn worker<K: Hash + Eq + Clone + Send + 'static>(
    st: Shared<K>,
    dec: Arc<dyn Decode<K>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let _guard = dec.init();
    loop {
        let (key, epoch) = {
            let mut s = lock(&st);
            loop {
                if s.stop {
                    return;
                }
                if let Some(k) = s.queue.pop_front() {
                    s.inflight.insert(k.clone());
                    break (k, s.epoch);
                }
                s = st.1.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };
        let out = dec.decode(&key).map(Arc::new);
        {
            let mut s = lock(&st);
            s.inflight.remove(&key);
            if s.epoch == epoch {
                s.cache.insert(key, out);
            }
        }
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn bmp(side: u32) -> Bitmap {
        Bitmap::new(side, side, vec![[1, 2, 3, 255]; (side * side) as usize]).unwrap()
    }

    fn wait(mut f: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn the_cache_is_bounded_in_bytes_evicts_the_least_recently_used_and_remembers_failures() {
        // rozpocet na dva obrazky 4x4 (64 B kazdy)
        let mut c: ImgCache<u32> = ImgCache::new(130, 2);
        c.insert(1, Some(Arc::new(bmp(4))));
        c.insert(2, Some(Arc::new(bmp(4))));
        assert_eq!((c.len(), c.bytes()), (2, 128));
        // pouzitie 1 ho ochrani: vyhodi sa 2
        assert!(matches!(c.get(&1), Some(Lookup::Ready(_))));
        c.insert(3, Some(Arc::new(bmp(4))));
        assert!(c.contains(&1) && c.contains(&3) && !c.contains(&2));
        assert!(c.bytes() <= 130, "rozpocet je pevny strop");
        // obrazok vacsi nez cely rozpocet sa neulozi, zapamata sa ako zlyhanie (bez bajtov)
        c.insert(9, Some(Arc::new(bmp(8))));
        assert_eq!(c.get(&9), Some(Lookup::Failed));
        assert!(c.bytes() <= 130);
        // zlyhania su ohranicene poctom a vyhadzuju sa pred obrazkami
        c.insert(10, None);
        c.insert(11, None);
        c.insert(12, None);
        assert!(
            c.map
                .values()
                .filter(|(s, _)| matches!(s, Slot::Failed))
                .count()
                <= 2
        );
        assert!(c.contains(&1) && c.contains(&3), "obrazky zostali");
        // prepis ten isty kluc nezdvojuje uctovanie
        c.insert(1, Some(Arc::new(bmp(4))));
        assert_eq!(c.bytes(), 128);
        c.retain(|k| *k == 1);
        assert_eq!((c.len(), c.bytes()), (1, 64));
        c.insert(5, None);
        c.forget_failed();
        assert!(
            !c.contains(&5) && c.contains(&1),
            "zlyhanie sa zabudlo, obrazok ostal"
        );
        c.clear();
        assert_eq!((c.len(), c.bytes()), (0, 0));
    }

    struct Fake {
        calls: AtomicUsize,
        order: Mutex<Vec<u32>>,
        gate: Mutex<Option<mpsc::Receiver<()>>>,
    }

    impl Decode<u32> for Fake {
        fn decode(&self, k: &u32) -> Option<Bitmap> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.order.lock().unwrap().push(*k);
            if *k == 1000 {
                // zablokovany dekoder (napr. zaseknute volanie systemu)
                if let Some(rx) = self.gate.lock().unwrap().take() {
                    let _ = rx.recv_timeout(Duration::from_secs(10));
                }
            }
            (!(*k).is_multiple_of(7)).then(|| bmp(2))
        }
    }

    fn fake() -> (Arc<Fake>, mpsc::Sender<()>) {
        let (tx, rx) = mpsc::channel();
        (
            Arc::new(Fake {
                calls: AtomicUsize::new(0),
                order: Mutex::new(Vec::new()),
                gate: Mutex::new(Some(rx)),
            }),
            tx,
        )
    }

    fn cfg(threads: usize) -> LoaderCfg {
        LoaderCfg {
            name: "test-loader",
            threads,
            budget_bytes: 1 << 20,
            max_queue: 64,
            max_failed: 64,
        }
    }

    #[test]
    fn decoding_happens_off_the_calling_thread_results_wake_the_ui_and_failures_are_not_retried() {
        let (f, _tx) = fake();
        let woke = Arc::new(AtomicUsize::new(0));
        let w = woke.clone();
        let l = Loader::start(
            cfg(1),
            f.clone(),
            Arc::new(move || {
                w.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert_eq!(l.get(&1), Lookup::Absent);
        l.want(&[1, 7, 2]);
        assert!(wait(|| matches!(l.get(&2), Lookup::Ready(_))));
        assert!(matches!(l.get(&1), Lookup::Ready(_)));
        assert_eq!(l.get(&7), Lookup::Failed, "chyba sa zapamata");
        assert_eq!(
            woke.load(Ordering::SeqCst),
            3,
            "po kazdom dokonceni sa UI zobudi"
        );
        // znovu poziadat o to iste nedekoduje nic
        let before = f.calls.load(Ordering::SeqCst);
        l.want(&[1, 7, 2]);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(f.calls.load(Ordering::SeqCst), before);
        // poradie dolezitosti sa dodrzi
        assert_eq!(*f.order.lock().unwrap(), vec![1, 7, 2]);
    }

    #[test]
    fn a_blocked_decoder_never_blocks_the_caller_and_the_latest_wanted_set_wins() {
        let (f, tx) = fake();
        let l = Loader::start(cfg(1), f.clone(), Arc::new(|| {}));
        l.want(&[1000, 1, 2, 3]);
        assert!(
            wait(|| f.calls.load(Ordering::SeqCst) == 1),
            "zacal dekodovat 1000"
        );
        // dekoder je zablokovany: get / want / clear su okamzite
        let t = Instant::now();
        assert_eq!(l.get(&1000), Lookup::Pending);
        assert_eq!(l.get(&1), Lookup::Pending);
        l.want(&[3, 4]); // 1 a 2 zastarali
        assert_eq!(
            l.get(&1),
            Lookup::Absent,
            "zastarana poziadavka sa zahodila"
        );
        assert!(t.elapsed() < Duration::from_millis(500));
        tx.send(()).unwrap();
        assert!(wait(|| matches!(l.get(&4), Lookup::Ready(_))));
        assert!(matches!(l.get(&3), Lookup::Ready(_)));
        let seen = f.order.lock().unwrap().clone();
        assert!(!seen.contains(&1) && !seen.contains(&2), "{seen:?}");
    }

    #[test]
    fn clear_discards_results_of_requests_that_started_before_it_and_the_queue_is_capped() {
        let (f, tx) = fake();
        let l = Loader::start(cfg(1), f.clone(), Arc::new(|| {}));
        l.want(&[1000]);
        assert!(wait(|| f.calls.load(Ordering::SeqCst) == 1));
        l.clear();
        tx.send(()).unwrap();
        assert!(wait(|| l.pending_len() == 0));
        assert_eq!(
            l.get(&1000),
            Lookup::Absent,
            "zastaraly vysledok sa neulozil"
        );
        // fronta je ohranicena a berie sa od najdolezitejsich
        let (g, _tx2) = fake();
        let l2 = Loader::start(
            LoaderCfg {
                max_queue: 10,
                ..cfg(1)
            },
            g.clone(),
            Arc::new(|| {}),
        );
        let keys: Vec<u32> = (1..=500).filter(|k| k % 7 != 0 && *k != 1000).collect();
        l2.want(&keys);
        assert!(wait(|| l2.pending_len() == 0));
        assert_eq!(g.calls.load(Ordering::SeqCst), 10);
        assert_eq!(*g.order.lock().unwrap(), keys[..10].to_vec());
    }
}
