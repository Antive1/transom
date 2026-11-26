//! Je na monitore prave plnoobrazovkova aplikacia? Doplnok k `ABN_FULLSCREENAPP`: tá prichádza len pri **zmene** stavu, takže lista,
//! ktora vznikne (spustenie transom, znovu zobrazenie po "Skryt listu", pripojenie monitora), kym uz plnoobrazovkova aplikacia
//! bezi, by ostala viditelna nad hrou, az kym sa stav znova nezmeni. Preto sa stav pri vzniku listy zisti aj priamo.
//!
//! Rozhodnutie je zamerne konzervativne (zlyhanie = lista ostane viditelna, nikdy nie skryta navzdy): system musi hlasit
//! plnoobrazovkovu aplikaciu (`SHQueryUserNotificationState`: `QUNS_BUSY` alebo `QUNS_RUNNING_D3D_FULL_SCREEN`, rovnaky zdroj
//! pravdy, aky pouziva Prieskumnik) **a** popredne okno nie je shell ani transom, nie je minimalizovane a pokryva cely monitor listy.
use crate::monitor::Rect;
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowRect, IsIconic, IsWindowVisible,
};

/// Okno pokryva cely (neprazdny) monitor.
pub fn covers(win: Rect, mon: Rect) -> bool {
    mon.width() > 0
        && mon.height() > 0
        && win.left <= mon.left
        && win.top <= mon.top
        && win.right >= mon.right
        && win.bottom >= mon.bottom
}

/// Okna plochy, panela uloh a transom, ktore nikdy nie su "plnoobrazovkova aplikacia" (plocha pokryva monitor vzdy).
pub fn is_shell_class(c: &str) -> bool {
    matches!(
        c,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "XamlExplorerHostIslandWindow"
    ) || c.starts_with("Transom")
}

/// Popredne okno, ako ho pozna rozhodnutie.
pub struct Fg<'a> {
    pub rect: Rect,
    pub class: &'a str,
    pub visible: bool,
    pub iconic: bool,
}

/// Cisté rozhodnutie (testovatelne): `system_busy` = system hlasi plnoobrazovkovu aplikaciu.
pub fn decide(system_busy: bool, fg: Option<Fg<'_>>, mon: Rect) -> bool {
    if !system_busy {
        return false;
    }
    match fg {
        Some(f) => f.visible && !f.iconic && !is_shell_class(f.class) && covers(f.rect, mon),
        None => false,
    }
}

/// Prave teraz je na monitore s obdlznikom `mon` plnoobrazovkova aplikacia (popredie)?
pub fn app_on(mon: Rect) -> bool {
    // SAFETY: citanie stavu systemu a popredneho okna; ziadny zapis.
    unsafe {
        let busy = SHQueryUserNotificationState()
            .map(|s| s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN)
            .unwrap_or(false);
        if !busy {
            return false;
        }
        let h = GetForegroundWindow();
        if h.0.is_null() {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(h, &mut r).is_err() {
            return false;
        }
        let mut buf = [0u16; 128];
        let n = GetClassNameW(h, &mut buf).max(0) as usize;
        let class = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        decide(
            busy,
            Some(Fg {
                rect: r.into(),
                class: &class,
                visible: IsWindowVisible(h).as_bool(),
                iconic: IsIconic(h).as_bool(),
            }),
            mon,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M1: Rect = Rect {
        left: 0,
        top: 0,
        right: 2560,
        bottom: 1440,
    };
    const M2: Rect = Rect {
        left: -1080,
        top: -405,
        right: 0,
        bottom: 1515,
    };

    fn fg(rect: Rect, class: &str) -> Fg<'_> {
        Fg {
            rect,
            class,
            visible: true,
            iconic: false,
        }
    }

    #[test]
    fn a_window_covers_a_monitor_only_when_it_spans_all_of_it() {
        assert!(covers(M1, M1));
        assert!(covers(
            Rect {
                left: -10,
                top: -10,
                right: 3000,
                bottom: 1500
            },
            M1
        ));
        for short in [
            Rect {
                left: 0,
                top: 0,
                right: 2560,
                bottom: 1439,
            },
            Rect {
                left: 0,
                top: 28,
                right: 2560,
                bottom: 1440,
            },
            Rect {
                left: 1,
                top: 0,
                right: 2560,
                bottom: 1440,
            },
            Rect {
                left: 0,
                top: 0,
                right: 2559,
                bottom: 1440,
            },
        ] {
            assert!(!covers(short, M1), "{short:?}");
        }
        // prazdny monitor nie je pokryty nicim (obrana pred nezmyslom z enumeracie)
        assert!(!covers(
            M1,
            Rect {
                left: 5,
                top: 5,
                right: 5,
                bottom: 5
            }
        ));
    }

    #[test]
    fn only_a_visible_foreground_app_covering_the_bar_monitor_while_the_system_says_busy_hides_the_bar(
    ) {
        // hra cez cely hlavny monitor: lista na tomto monitore sa skryje, na druhom nie
        assert!(decide(true, Some(fg(M1, "UnrealWindow")), M1));
        assert!(!decide(true, Some(fg(M1, "UnrealWindow")), M2));
        // system nehlasi plnoobrazovkovu aplikaciu (napr. maximalizovane okno s titulkom, ktore monitor nepokryva presne, alebo
        // okno bez rezimu "busy"): lista ostava
        assert!(!decide(false, Some(fg(M1, "UnrealWindow")), M1));
        // plocha, panel uloh a vlastne okna transom nikdy nie su plnoobrazovkova aplikacia
        for c in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "TransomBar",
            "TransomPopover",
            "TransomCtl",
        ] {
            assert!(!decide(true, Some(fg(M1, c)), M1), "{c}");
        }
        // minimalizovane, neviditelne alebo ziadne popredie
        let mut f = fg(M1, "Game");
        f.iconic = true;
        assert!(!decide(true, Some(f), M1));
        let mut f = fg(M1, "Game");
        f.visible = false;
        assert!(!decide(true, Some(f), M1));
        assert!(!decide(true, None, M1));
        // okno mensie nez monitor (hra v okne): lista ostava
        assert!(!decide(
            true,
            Some(fg(
                Rect {
                    left: 100,
                    top: 100,
                    right: 1700,
                    bottom: 1000
                },
                "Game"
            )),
            M1
        ));
    }

    #[test]
    fn the_live_query_is_read_only_and_does_not_panic() {
        // skutocny stav tohto stroja (hodnota sa nepouziva): iba overenie, ze volanie funguje a nic nezapisuje
        let _ = app_on(M1);
        let _ = app_on(M2);
        let _ = app_on(Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        });
    }
}
