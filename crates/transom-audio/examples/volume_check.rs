//! Zivy test systemovej hlasitosti a notifikacii Core Audio (len na vyslovne spustenie: `-- --live`).
//!
//! `cargo run -p transom-audio --example volume_check -- --live`
//!
//! Co robi: precita povodnu hlasitost a stlmenie predvoleneho vystupu, zmeni hlasitost o 2 % (nadol; nahor len pri
//! hlasitosti <= 50 %), na chvilu stlmi a odstlmi a overi, ze zmeny prisli **ako notifikacie** Core Audio (snimka
//! sluzby sa zmenila bez citania nazaj). **Povodne hodnoty sa vzdy obnovia** v cistiacej ceste (`Drop` strazca, aj pri
//! panike v predvolenom profile `dev`; pri `--release` je `panic = "abort"` a `Drop` sa po panike nevykona); test sa odmietne spustit, ak je vystup uz stlmeny alebo ma hlasitost 0. Predvoleny vystup sa NEMENI.
//! Nazvy a ID zariadeni sa nevypisuju (len pocty a stavy). Ctrl+C pocas ~3 s behu nie je osetreny: ak by test bol
//! prerusny, povodnu hlasitost vypise na zaciatku, aby ju bolo mozne obnovit rucne.
#[cfg(windows)]
fn main() {
    std::process::exit(live::run());
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod live {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use transom_audio::{Availability, Service, Volume};

    fn wait(svc: &Service, mut ok: impl FnMut(&transom_audio::Snapshot) -> bool, ms: u64) -> bool {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            if ok(&svc.snapshot()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        ok(&svc.snapshot())
    }

    /// Obnovi povodnu hlasitost a stlmenie vzdy (aj pri panike): cistiaca cesta `finally`.
    struct Restore<'a> {
        svc: &'a Service,
        orig: Volume,
    }

    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            self.svc.set_mute(self.orig.muted);
            self.svc.set_volume(self.orig.percent as u32);
            let want = self.orig;
            let ok = wait(self.svc, |s| s.volume == Some(want), 3000);
            println!(
                "RESTORE percent={} muted={} confirmed={ok}",
                self.orig.percent, self.orig.muted
            );
        }
    }

    pub fn run() -> i32 {
        if !std::env::args().any(|a| a == "--live") {
            println!("Zivy test meni systemovu hlasitost o 2 % a na chvilu stlmi vystup (potom vsetko obnovi). Spustite s `-- --live`.");
            return 0;
        }
        let svc = match transom_audio::start_system(Arc::new(|| {})) {
            Ok(s) => s,
            Err(e) => {
                println!("FAIL sluzbu sa nepodarilo spustit: {e}");
                return 2;
            }
        };
        if !wait(&svc, |s| s.availability != Availability::Starting, 5000) {
            println!("FAIL sluzba sa nenacitala");
            return 2;
        }
        let s = svc.snapshot();
        println!(
            "STATE availability={:?} devices={} default_present={} controls={:?}",
            s.availability,
            s.devices.len(),
            s.default_device().is_some(),
            s.controls
        );
        for d in &s.devices {
            println!("DEVICE state={:?} default={}", d.state, d.is_default);
        }
        let Some(orig) = s.volume else {
            println!("SKIP ziadny pouzitelny vystup alebo hlasitost nie je dostupna");
            return 0;
        };
        println!("ORIGINAL percent={} muted={}", orig.percent, orig.muted);
        if orig.muted || orig.percent == 0 || !s.controls.volume {
            println!(
                "SKIP vystup je stlmeny, na 0 % alebo bez regulacie hlasitosti: test sa nespusta"
            );
            return 0;
        }
        let mut ok = true;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = Restore { svc: &svc, orig };
            let target = if orig.percent >= 3 {
                orig.percent - 2
            } else if orig.percent <= 50 {
                orig.percent + 2
            } else {
                orig.percent - 1
            };
            let gen0 = svc.snapshot().generation;
            svc.set_volume(target as u32);
            let got = wait(&svc, |s| s.volume.map(|v| v.percent) == Some(target), 3000);
            println!(
                "VOLUME_SET target={target} notified={got} generation_changed={}",
                svc.snapshot().generation != gen0
            );
            if !got {
                println!("FAIL zmena hlasitosti neprisla ako notifikacia");
                return false;
            }
            if s.controls.mute {
                svc.set_mute(true);
                let m = wait(&svc, |s| s.volume.is_some_and(|v| v.muted), 3000);
                println!("MUTE_ON notified={m}");
                std::thread::sleep(Duration::from_millis(250));
                svc.set_mute(false);
                let u = wait(&svc, |s| s.volume.is_some_and(|v| !v.muted), 3000);
                println!("MUTE_OFF notified={u}");
                if !(m && u) {
                    println!("FAIL stlmenie neprislo ako notifikacia");
                    return false;
                }
            }
            true
        }));
        match result {
            Ok(r) => ok &= r,
            Err(_) => {
                println!("FAIL panika v tele testu (povodne hodnoty sa obnovili)");
                ok = false;
            }
        }
        // nezavisla kontrola po obnove
        let end = svc.snapshot();
        let restored = end.volume == Some(orig);
        println!(
            "FINAL percent={:?} restored={restored}",
            end.volume.map(|v| v.percent)
        );
        ok &= restored;
        println!("RESULT {}", if ok { "OK" } else { "FAIL" });
        if ok {
            0
        } else {
            1
        }
    }
}
