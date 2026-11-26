//! Zivy test Bluetooth zvuku: `cargo run -p transom-mixer --example bt` vypise sparovane zariadenia (nic nemeni).
//! `-- --connect "<meno>"` / `--disconnect "<meno>"` zariadenie pripoji / odpoji a pocka na potvrdenie.
#[cfg(windows)]
fn main() {
    use std::sync::Arc;
    let svc = transom_mixer::bt::BtService::start(Arc::new(|| {})).unwrap();
    let wait = |f: &dyn Fn(&transom_mixer::bt::BtSnap) -> bool| {
        for _ in 0..200 {
            if f(&svc.snapshot()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    };
    wait(&|s| s.ready);
    for d in &svc.snapshot().devices {
        println!("BT {:<30} connected={}", d.name, d.connected);
    }
    let a: Vec<String> = std::env::args().collect();
    if a.len() >= 3 && a[1] == "--probe" {
        let t = std::time::Instant::now();
        println!(
            "PROBE hresults={:?} ({} ms)",
            transom_mixer::bt::probe(&a[2], true),
            t.elapsed().as_millis()
        );
        return;
    }
    if a.len() >= 3 {
        let connect = a[1] == "--connect";
        let snap = svc.snapshot();
        let Some(d) = snap.devices.iter().find(|d| d.name == a[2]) else {
            return;
        };
        let g = snap.generation;
        svc.set(&d.key, connect);
        wait(&|s| s.generation > g + 1);
        let s = svc.snapshot();
        println!(
            "RESULT failed={} now={:?}",
            s.failed.is_some(),
            s.devices
                .iter()
                .find(|x| x.key == d.key)
                .map(|x| x.connected)
        );
    }
}
#[cfg(not(windows))]
fn main() {}
