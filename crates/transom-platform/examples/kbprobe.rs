//! Zivy test polozky Klavesnica: `cargo run -p transom-platform --example kbprobe` (precita stav 3x po 0,5 s).
#[cfg(windows)]
fn main() {
    for _ in 0..1 {
        let s = transom_platform::keyboard::state();
        println!("LANG {} NAME {} CAPS {}", s.lang, s.name, s.caps);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
#[cfg(not(windows))]
fn main() {}
