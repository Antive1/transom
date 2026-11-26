//! Vypise pripojene monitory presne tak, ako ich vidi transom (nazov, ID, rozlisenie, mierka, primarny): len cita, nic nemeni.
//!   cargo run -p transom-platform --example monitors
#[cfg(windows)]
fn main() {
    transom_platform::init_dpi_awareness();
    for m in transom_platform::monitor::select(
        transom_platform::monitor::enumerate(),
        &transom_platform::monitor::MonitorSelection::All,
    ) {
        println!(
            "{} | name='{}' | {}x{} @({},{}) | dpi={} ({} %) | primary={} | id={}",
            m.device,
            m.name,
            m.rect.width(),
            m.rect.height(),
            m.rect.left,
            m.rect.top,
            m.dpi,
            (m.dpi * 100 + 48) / 96,
            m.primary,
            m.id
        );
    }
}

#[cfg(not(windows))]
fn main() {}
