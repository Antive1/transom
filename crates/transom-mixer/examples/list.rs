//! Zivy test mixera iba na citanie: `cargo run -p transom-mixer --example list`.
//! Vypise aplikacie (meno, hlasitost, stlmenie, ci hra, vystup) a ci je vyber vystupu dostupny. Nic nemeni.
#[cfg(windows)]
fn main() {
    use std::sync::{Arc, Condvar, Mutex};
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let p2 = Arc::clone(&pair);
    let svc = transom_mixer::start_system(
        Arc::new(move || {
            *p2.0.lock().unwrap() = true;
            p2.1.notify_all();
        }),
        "System sounds".into(),
    )
    .expect("start");
    let (m, c) = &*pair;
    let _ = c.wait_timeout_while(m.lock().unwrap(), std::time::Duration::from_secs(5), |r| {
        !*r
    });
    let s = svc.snapshot();
    println!(
        "READY {} ROUTING {} DEVICES {}",
        s.ready,
        s.routing,
        s.devices.len()
    );
    for a in &s.apps {
        let route = match &a.route {
            transom_mixer::Route::Default => "default".to_string(),
            transom_mixer::Route::Device(id) => s
                .devices
                .iter()
                .find(|d| &d.id == id)
                .map_or("missing".into(), |d| d.name.clone()),
        };
        println!(
            "APP {:<30} vol={:>3} muted={} active={} system={} pids={} out={route}",
            a.name,
            a.volume,
            a.muted,
            a.active,
            a.system,
            a.pids.len()
        );
    }
}

#[cfg(not(windows))]
fn main() {}
