//! Poradie fokusu klavesnice v okne nastaveni (cista logika, bez WinAPI): Tab / Shift+Tab prechadza ovladace aktualnej stranky
//! (v poradi kreslenia: navigacia, potom obsah), viditelne vstupne polia a nakoniec tlacidla hlavicky (Minimalizovat,
//! Maximalizovat, Zavriet). Za poslednym prvkom fokus opusti kruh (nic nema fokus) a dalsi Tab zacne od zaciatku - rovnako ako
//! doterajsi cyklus hlavicky.
use crate::appearance_page::SliderKey;
use crate::chrome::{self, Btn};
use crate::dnd::Dir;
use crate::Hit;

/// Jeden prvok kruhu fokusu.
#[derive(Clone, Debug, PartialEq)]
pub enum Slot {
    /// Ovladac kreslenej stranky (identita je `Hit`, nie poloha: prezije posun a prekreslenie).
    Page(Hit),
    /// Vstupne pole (index v `Ui.edits`).
    Edit(usize),
    Chrome(Btn),
}

/// Kruh fokusu: ovladace stranky (bez duplicit, v poradi; vsetky su dosiahnutelne klavesnicou: polozky rozlozenia sa
/// presuvaju Enter + sipky, posuvnik nepriehladnosti sipkami), viditelne vstupne polia a tlacidla hlavicky.
pub fn ring(page_hits: &[Hit], visible_edits: &[usize]) -> Vec<Slot> {
    let mut out: Vec<Slot> = Vec::new();
    for h in page_hits {
        let s = Slot::Page(h.clone());
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.extend(visible_edits.iter().map(|&i| Slot::Edit(i)));
    out.extend(chrome::BTNS.iter().map(|&b| Slot::Chrome(b)));
    out
}

/// Dalsi prvok po `cur` (`back` = opacne). Z "nic" dopredu na prvy, dozadu na posledny; za poslednym (pred prvym) fokus kruh
/// opusti (`None`). Prvok, ktory v kruhu uz nie je (napr. po zmene stranky), sa berie ako "nic".
pub fn step(ring: &[Slot], cur: Option<&Slot>, back: bool) -> Option<Slot> {
    let pos = cur.and_then(|c| ring.iter().position(|s| s == c));
    let next = match (pos, back) {
        (None, false) => Some(0),
        (None, true) => ring.len().checked_sub(1),
        (Some(i), false) => (i + 1 < ring.len()).then_some(i + 1),
        (Some(i), true) => i.checked_sub(1),
    };
    next.and_then(|i| ring.get(i)).cloned()
}

/// Co ma okno urobit s klavesou (`vk`), ak ju dostalo hlavne okno (`in_edit = false`) alebo vstupne pole (`in_edit = true`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Tab / Shift+Tab: posun v kruhu (`back`).
    Move { back: bool },
    /// Enter / medzernik: vykonaj prvok s fokusom (v poli: potvrd hodnotu a opusti pole).
    Activate,
    /// Sipky vlavo / vpravo na tlacidlach hlavicky.
    ChromeSide { right: bool },
    /// Esc bez rozpracovanej upravy: zrus fokus.
    Clear,
    /// Esc pri rozpracovanej uprave (pole: vrat povodnu hodnotu; presun polozky: vrat povodne rozlozenie).
    Cancel,
    /// Enter / medzernik na polozke rozlozenia: chytit (zacat presun).
    Grab,
    /// Enter / medzernik pocas presunu: pustit (potvrdit).
    Drop,
    /// Sipka / Home / End pocas presunu polozky.
    Nudge(Dir),
    /// Klavesa na posuvniku nepriehladnosti (`fine` = Shift: krok 1 %).
    Slider { key: SliderKey, fine: bool },
    /// Esc pri otvorenom nahlade importu: zatvorit nahlad (rovnako ako "Zrusit").
    ClosePreview,
    /// Nic: klavesu ma dostat povodny prijemca (pole pise, okno posuva).
    Pass,
}

/// Co ma fokus na kreslenej stranke (rozhoduje o vyznam sipok).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFocus {
    Other,
    /// Polozka rozlozenia; `moving` = prave sa presuva.
    Chip {
        moving: bool,
    },
    /// Posuvnik nepriehladnosti.
    Slider,
    /// Otvoreny nahlad importu (fokus je na inom ovladaci): Esc ho zatvori.
    Preview,
}

/// Smerovanie klaves (cista funkcia). `focused` = ma nejaky prvok kruhu fokus; `chrome_focused` = je to tlacidlo hlavicky;
/// `repeat` = automaticke opakovanie klavesu (Enter / medzernik sa pri nom na polozke rozlozenia ignoruje, aby ju drzanie
/// klavesu nechytilo a hned nepustilo).
pub fn route_key_ctx(
    vk: u32,
    shift: bool,
    in_edit: bool,
    focused: bool,
    chrome_focused: bool,
    page: PageFocus,
    repeat: bool,
) -> KeyAction {
    // Tab vzdy posuva fokus (pocas presunu ho najprv potvrdi volajuci)
    if vk == 0x09 {
        return KeyAction::Move { back: shift };
    }
    if in_edit {
        return match vk {
            // v poli: Enter potvrdi a opusti pole, Esc vrati povodnu hodnotu; medzernik a sipky su pisanie
            0x0D => KeyAction::Activate,
            0x1B => KeyAction::Cancel,
            _ => KeyAction::Pass,
        };
    }
    let once = |a: KeyAction| if repeat { KeyAction::Pass } else { a };
    match (page, vk) {
        (PageFocus::Chip { moving: true }, 0x0D | 0x20) => once(KeyAction::Drop),
        (PageFocus::Chip { moving: true }, 0x1B) => KeyAction::Cancel,
        (PageFocus::Chip { moving: true }, 0x25) => KeyAction::Nudge(Dir::Left),
        (PageFocus::Chip { moving: true }, 0x27) => KeyAction::Nudge(Dir::Right),
        (PageFocus::Chip { moving: true }, 0x26) => KeyAction::Nudge(Dir::Up),
        (PageFocus::Chip { moving: true }, 0x28) => KeyAction::Nudge(Dir::Down),
        (PageFocus::Chip { moving: true }, 0x24) => KeyAction::Nudge(Dir::Home),
        (PageFocus::Chip { moving: true }, 0x23) => KeyAction::Nudge(Dir::End),
        // pocas presunu ostatne klavesy (aj posuv) nerobia nic
        (PageFocus::Chip { moving: true }, _) => KeyAction::Pass,
        (PageFocus::Chip { moving: false }, 0x0D | 0x20) => once(KeyAction::Grab),
        (PageFocus::Slider, 0x25) => KeyAction::Slider {
            key: SliderKey::Dec,
            fine: shift,
        },
        (PageFocus::Slider, 0x27) => KeyAction::Slider {
            key: SliderKey::Inc,
            fine: shift,
        },
        (PageFocus::Slider, 0x22) => KeyAction::Slider {
            key: SliderKey::PageDec,
            fine: false,
        },
        (PageFocus::Slider, 0x21) => KeyAction::Slider {
            key: SliderKey::PageInc,
            fine: false,
        },
        (PageFocus::Slider, 0x24) => KeyAction::Slider {
            key: SliderKey::Min,
            fine: false,
        },
        (PageFocus::Slider, 0x23) => KeyAction::Slider {
            key: SliderKey::Max,
            fine: false,
        },
        // otvoreny nahlad: Esc ho zatvori (aj bez fokusu na ovladaci)
        (PageFocus::Preview, 0x1B) => KeyAction::ClosePreview,
        _ => match vk {
            // drzanie Enter / medzernika nesmie preskocit druhy krok dvojkrokoveho potvrdenia (zmazanie, import)
            0x0D | 0x20 if focused => once(KeyAction::Activate),
            0x1B if focused => KeyAction::Clear,
            0x25 | 0x27 if chrome_focused => KeyAction::ChromeSide { right: vk == 0x27 },
            _ => KeyAction::Pass,
        },
    }
}

/// Smerovanie bez kontextu stranky (hlavicka, bezne ovladace).
#[cfg(test)]
pub fn route_key(
    vk: u32,
    shift: bool,
    in_edit: bool,
    focused: bool,
    chrome_focused: bool,
) -> KeyAction {
    route_key_ctx(
        vk,
        shift,
        in_edit,
        focused,
        chrome_focused,
        PageFocus::Other,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Vec<Hit> {
        vec![
            Hit::Nav(crate::Page::Layout),
            Hit::Nav(crate::Page::Clock),
            Hit::Show("clock".into()),
            Hit::Chip("clock".into()),
            Hit::ResetLayout,
            Hit::OpacityTrack,
            Hit::Show("clock".into()), // duplicita sa vynecha
            Hit::Pick,
        ]
    }

    #[test]
    fn the_ring_has_every_page_control_once_then_visible_edits_then_the_header_buttons() {
        let r = ring(&page(), &[1, 3]);
        assert_eq!(
            r,
            vec![
                Slot::Page(Hit::Nav(crate::Page::Layout)),
                Slot::Page(Hit::Nav(crate::Page::Clock)),
                Slot::Page(Hit::Show("clock".into())),
                Slot::Page(Hit::Chip("clock".into())),
                Slot::Page(Hit::ResetLayout),
                Slot::Page(Hit::OpacityTrack),
                Slot::Page(Hit::Pick),
                Slot::Edit(1),
                Slot::Edit(3),
                Slot::Chrome(Btn::Min),
                Slot::Chrome(Btn::Max),
                Slot::Chrome(Btn::Close),
            ]
        );
    }

    #[test]
    fn tab_visits_every_slot_once_leaves_the_ring_and_starts_again() {
        let r = ring(&page(), &[0]);
        let mut seen = Vec::new();
        let mut cur: Option<Slot> = None;
        loop {
            cur = step(&r, cur.as_ref(), false);
            match &cur {
                Some(s) => seen.push(s.clone()),
                None => break,
            }
            assert!(seen.len() <= r.len(), "nekonecny cyklus");
        }
        assert_eq!(seen, r, "kazdy prvok presne raz, v poradi");
        assert_eq!(
            step(&r, None, false).as_ref(),
            r.first(),
            "dalsi Tab zacne od zaciatku"
        );
    }

    #[test]
    fn a_page_without_controls_keeps_the_old_header_only_cycle() {
        let r = ring(&[], &[]);
        let mut seen = vec![];
        let mut cur: Option<Slot> = None;
        for _ in 0..4 {
            cur = step(&r, cur.as_ref(), false);
            seen.push(cur.clone());
        }
        assert_eq!(
            seen,
            [
                Some(Slot::Chrome(Btn::Min)),
                Some(Slot::Chrome(Btn::Max)),
                Some(Slot::Chrome(Btn::Close)),
                None
            ]
        );
        let mut cur: Option<Slot> = None;
        let mut seen = vec![];
        for _ in 0..4 {
            cur = step(&r, cur.as_ref(), true);
            seen.push(cur.clone());
        }
        assert_eq!(
            seen,
            [
                Some(Slot::Chrome(Btn::Close)),
                Some(Slot::Chrome(Btn::Max)),
                Some(Slot::Chrome(Btn::Min)),
                None
            ]
        );
    }

    #[test]
    fn shift_tab_is_the_exact_reverse() {
        let r = ring(&page(), &[0, 2]);
        let mut back = Vec::new();
        let mut cur: Option<Slot> = None;
        while let Some(s) = step(&r, cur.as_ref(), true) {
            back.push(s.clone());
            cur = Some(s);
            assert!(back.len() <= r.len());
        }
        back.reverse();
        assert_eq!(back, r);
        let a = step(&r, None, false);
        let b = step(&r, a.as_ref(), false);
        assert_eq!(step(&r, b.as_ref(), true), a);
    }

    #[test]
    fn a_focused_slot_that_vanished_counts_as_no_focus_and_an_empty_ring_never_panics() {
        let r = ring(&[Hit::ResetLayout], &[]);
        let gone = Slot::Page(Hit::DeleteData);
        assert_eq!(step(&r, Some(&gone), false), r.first().cloned());
        assert_eq!(step(&r, Some(&gone), true), r.last().cloned());
        assert_eq!(step(&[], None, false), None);
        assert_eq!(step(&[], Some(&gone), true), None);
    }

    #[test]
    fn key_routing_never_lets_a_key_typed_in_a_field_operate_the_header() {
        // ROOT CAUSE povodnej chyby: Enter / sipky / Esc z vstupneho pola sa viazali na tlacidla hlavicky (aj Zavriet)
        for vk in [0x20u32, 0x25, 0x27, 0x08, 0x41, 0x30] {
            assert_eq!(
                route_key(vk, false, true, true, true),
                KeyAction::Pass,
                "vk {vk:#x} v poli"
            );
        }
        assert_eq!(
            route_key(0x0D, false, true, true, true),
            KeyAction::Activate,
            "Enter potvrdi pole"
        );
        assert_eq!(
            route_key(0x1B, false, true, false, false),
            KeyAction::Cancel,
            "Esc v poli vrati povodnu hodnotu (a neulozi zamietnutu)"
        );
        assert_eq!(
            route_key(0x09, false, true, false, false),
            KeyAction::Move { back: false },
            "Tab opusti pole"
        );
        assert_eq!(
            route_key(0x09, true, true, false, false),
            KeyAction::Move { back: true }
        );
    }

    #[test]
    fn key_routing_in_the_main_window() {
        assert_eq!(
            route_key(0x09, false, false, false, false),
            KeyAction::Move { back: false }
        );
        assert_eq!(
            route_key(0x09, true, false, true, false),
            KeyAction::Move { back: true }
        );
        assert_eq!(
            route_key(0x0D, false, false, true, false),
            KeyAction::Activate
        );
        assert_eq!(
            route_key(0x20, false, false, true, true),
            KeyAction::Activate
        );
        assert_eq!(route_key(0x1B, false, false, true, false), KeyAction::Clear);
        assert_eq!(route_key(0x0D, false, false, false, false), KeyAction::Pass);
        assert_eq!(route_key(0x20, false, false, false, false), KeyAction::Pass);
        assert_eq!(route_key(0x1B, false, false, false, false), KeyAction::Pass);
        assert_eq!(
            route_key(0x25, false, false, true, true),
            KeyAction::ChromeSide { right: false }
        );
        assert_eq!(
            route_key(0x27, false, false, true, true),
            KeyAction::ChromeSide { right: true }
        );
        assert_eq!(route_key(0x25, false, false, true, false), KeyAction::Pass);
        assert_eq!(route_key(0x26, false, false, true, true), KeyAction::Pass);
    }

    fn ctx(vk: u32, shift: bool, page: PageFocus, repeat: bool) -> KeyAction {
        route_key_ctx(vk, shift, false, true, false, page, repeat)
    }

    #[test]
    fn a_layout_item_is_grabbed_moved_with_arrows_dropped_or_cancelled() {
        let idle = PageFocus::Chip { moving: false };
        let moving = PageFocus::Chip { moving: true };
        assert_eq!(ctx(0x0D, false, idle, false), KeyAction::Grab);
        assert_eq!(ctx(0x20, false, idle, false), KeyAction::Grab);
        assert_eq!(
            ctx(0x0D, false, idle, true),
            KeyAction::Pass,
            "auto-opakovanie nechyti"
        );
        for (vk, d) in [
            (0x25, Dir::Left),
            (0x27, Dir::Right),
            (0x26, Dir::Up),
            (0x28, Dir::Down),
            (0x24, Dir::Home),
            (0x23, Dir::End),
        ] {
            assert_eq!(
                ctx(vk, false, moving, false),
                KeyAction::Nudge(d),
                "{vk:#x}"
            );
            assert_eq!(
                ctx(vk, false, moving, true),
                KeyAction::Nudge(d),
                "opakovanie sipky je v poriadku"
            );
        }
        assert_eq!(ctx(0x0D, false, moving, false), KeyAction::Drop);
        assert_eq!(ctx(0x20, false, moving, false), KeyAction::Drop);
        assert_eq!(
            ctx(0x0D, false, moving, true),
            KeyAction::Pass,
            "opakovanie hned nepusti"
        );
        assert_eq!(ctx(0x1B, false, moving, false), KeyAction::Cancel);
        assert_eq!(ctx(0x41, false, moving, false), KeyAction::Pass);
        assert_eq!(
            ctx(0x09, false, moving, false),
            KeyAction::Move { back: false }
        );
        // mimo presunu sipky na polozke patria oknu (posuv)
        assert_eq!(ctx(0x28, false, idle, false), KeyAction::Pass);
    }

    #[test]
    fn the_opacity_slider_takes_left_right_page_home_end_and_leaves_up_down_to_scrolling() {
        let s = PageFocus::Slider;
        assert_eq!(
            ctx(0x25, false, s, false),
            KeyAction::Slider {
                key: SliderKey::Dec,
                fine: false
            }
        );
        assert_eq!(
            ctx(0x27, true, s, false),
            KeyAction::Slider {
                key: SliderKey::Inc,
                fine: true
            }
        );
        assert_eq!(
            ctx(0x21, true, s, false),
            KeyAction::Slider {
                key: SliderKey::PageInc,
                fine: false
            }
        );
        assert_eq!(
            ctx(0x22, false, s, false),
            KeyAction::Slider {
                key: SliderKey::PageDec,
                fine: false
            }
        );
        assert_eq!(
            ctx(0x24, false, s, false),
            KeyAction::Slider {
                key: SliderKey::Min,
                fine: false
            }
        );
        assert_eq!(
            ctx(0x23, false, s, false),
            KeyAction::Slider {
                key: SliderKey::Max,
                fine: false
            }
        );
        assert_eq!(ctx(0x26, false, s, false), KeyAction::Pass);
        assert_eq!(ctx(0x28, false, s, false), KeyAction::Pass);
        assert_eq!(
            route_key_ctx(0x25, false, true, true, false, s, false),
            KeyAction::Pass,
            "v poli ma prednost pole"
        );
    }
}
