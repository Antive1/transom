//! Zivy test jasu: `cargo run -p transom-platform --example ddcprobe` (len citanie); s `--write` znizi jas hlavneho
//! monitora o 1 %, precita ho a vrati povodnu hodnotu.
#[cfg(windows)]
fn main() {
    use std::sync::{Arc, Condvar, Mutex};
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let p2 = Arc::clone(&pair);
    let svc = transom_platform::brightness::Service::start(Arc::new(move || {
        *p2.0.lock().unwrap() = true;
        p2.1.notify_all();
    }))
    .unwrap();
    let (m, c) = &*pair;
    let _ = c.wait_timeout_while(m.lock().unwrap(), std::time::Duration::from_secs(10), |r| {
        !*r
    });
    for d in &svc.snapshot().displays {
        println!(
            "DISPLAY {:<30} primary={} level={:?}",
            d.name, d.primary, d.level
        );
    }
    if std::env::args().any(|a| a == "--write") {
        let snap = svc.snapshot();
        let Some(d) = snap.displays.iter().find(|d| d.level.is_some()) else {
            return;
        };
        let orig = d.level.unwrap();
        let wait = |gen: u64| {
            let t0 = std::time::Instant::now();
            while svc.snapshot().generation == gen && t0.elapsed().as_secs() < 5 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            t0.elapsed().as_millis()
        };
        let g = svc.snapshot().generation;
        svc.set(&d.key, orig.saturating_sub(1));
        let ms = wait(g);
        let s = svc.snapshot();
        println!(
            "WRITE {} -> read back {:?} in {ms} ms failed={:?}",
            orig - 1,
            s.displays[0].level,
            s.failed.is_some()
        );
        let g = s.generation;
        svc.set(&d.key, orig);
        wait(g);
        println!("RESTORED read back {:?}", svc.snapshot().displays[0].level);
    }
}
#[cfg(not(windows))]
fn main() {}
