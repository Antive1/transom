//! Jedno kolo citania vsetkych zdrojov na tomto PC (len citanie): `cargo run -p transom-peripherals --example probe`.
//! Vypise kazde citanie a cas zdroja; pri `--rounds N` zopakuje N kol (s 5 s pauzou) cez `Tracker`.
#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};
    use transom_peripherals::{Source, Tracker};
    let rounds: usize = std::env::args()
        .skip_while(|a| a != "--rounds")
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut providers: Vec<Box<dyn transom_peripherals::Provider>> =
        transom_peripherals::system_providers(&Source::ALL);
    let mut tracker = Tracker::new(Duration::from_secs(180));
    for round in 0..rounds {
        let mut all = Vec::new();
        for p in providers.iter_mut() {
            let t = Instant::now();
            let r = p.poll();
            println!(
                "{:<10} {:>5} ms  {} citani",
                p.source().id(),
                t.elapsed().as_millis(),
                r.len()
            );
            for x in &r {
                println!(
                    "    {:<28} {:?} {:?} {:?} charging={} [{}]",
                    x.name, x.kind, x.presence, x.percent, x.charging, x.key
                );
            }
            all.extend(r);
        }
        tracker.update(all, Instant::now());
        println!("--- kolo {} -> zobrazene:", round + 1);
        for d in tracker.devices(Instant::now()) {
            println!(
                "    {:<28} {:>4}  charging={} stale={}",
                d.name,
                transom_peripherals::short_label(&d),
                d.charging,
                d.stale
            );
        }
        if round + 1 < rounds {
            std::thread::sleep(Duration::from_secs(5));
        }
    }
}

#[cfg(not(windows))]
fn main() {}
