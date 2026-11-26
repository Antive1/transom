//! Diagnostika DDC/CI (len citanie): pocet fyzickych monitorov a vysledok citania VCP 0x10 s chybovym kodom.
#[cfg(windows)]
fn main() {
    use windows::Win32::Devices::Display::*;
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let hm = MonitorFromPoint(POINT { x: 10, y: 10 }, MONITOR_DEFAULTTOPRIMARY);
        let mut n = 0u32;
        let r = GetNumberOfPhysicalMonitorsFromHMONITOR(hm, &mut n);
        println!("PHYS n={n} ok={}", r.is_ok());
        let mut arr = vec![PHYSICAL_MONITOR::default(); n as usize];
        println!("GET {:?}", GetPhysicalMonitorsFromHMONITOR(hm, &mut arr));
        for h in &arr {
            for _ in 0..6 {
                let (mut c, mut m) = (0u32, 0u32);
                let ok = GetVCPFeatureAndVCPFeatureReply(
                    h.hPhysicalMonitor,
                    0x10,
                    None,
                    &mut c,
                    Some(&mut m),
                );
                println!("VCP ok={ok} cur={c} max={m} err={:?}", GetLastError());
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        }
        let _ = DestroyPhysicalMonitors(&arr);
    }
}
#[cfg(not(windows))]
fn main() {}
