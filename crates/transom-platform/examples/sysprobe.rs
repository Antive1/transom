//! Zivy test vytazenia: `cargo run -p transom-platform --example sysprobe` (5 merani po 1 s).
#[cfg(windows)]
fn main() {
    let mut s = transom_platform::sysmon::Sampler::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let v = s.sample(true, true);
        println!("{v:?} ({} ms)", t.elapsed().as_millis());
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
#[cfg(not(windows))]
fn main() {}
