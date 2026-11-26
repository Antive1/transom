//! Zivy test kalendara ICS: `cargo run -p transom-platform --example icsprobe -- <https odkaz>` (vypise pocet udalosti a
//! vyskyty v najblizsich 60 dnoch). Odkaz sa nevypisuje.
#[cfg(windows)]
fn main() {
    use transom_core::clock::{civil_from_days, days_from_civil};
    let url = std::env::args().nth(1).expect("odkaz");
    let t0 = std::time::Instant::now();
    let bytes = match transom_platform::http::get(&url, 8 << 20, 20_000) {
        Ok(b) => b,
        Err(e) => return println!("CHYBA {e}"),
    };
    let cal = transom_core::ics::parse(&String::from_utf8_lossy(&bytes));
    let today = transom_platform::clock::today();
    let end = civil_from_days(days_from_civil(today) + 60);
    let l2u = |l: transom_core::ics::Local| transom_platform::clock::utc_of_local(l.day, l.secs);
    let occ = cal.occurrences(
        transom_platform::clock::utc_of_local(today, 0),
        transom_platform::clock::utc_of_local(end, 0),
        today,
        end,
        &l2u,
    );
    println!(
        "BYTES {} EVENTS {} NEXT60 {} ({} ms)",
        bytes.len(),
        cal.event_count(),
        occ.len(),
        t0.elapsed().as_millis()
    );
    for o in occ.iter().take(5) {
        println!("  {:?} {}", o.start, o.summary);
    }
}
#[cfg(not(windows))]
fn main() {}
