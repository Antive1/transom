//! Male ohranicene pamate vysledkov ciste vypocitanych rastrov (znacka, hviezdicka, ikony, kruh, zmenseny obal).
//! Lista ich pri kazdej snimke ekvalizera a posuvu (az 60x za sekundu) kreslila znova z vektorov so supersamplingom;
//! vstupy sa pritom menia len zriedka (DPI, tema, percento, novy obal). Kazda pamat ma par poloziek (najdlhsie
//! nepouzita sa zahodi), je na vlakno (kreslenie je jednovlaknove) a vysledok je ten isty ako priamy vypocet.
use std::cell::RefCell;
use std::rc::Rc;

/// Pamat najviac `cap` poloziek: posledne pouzita je na konci, pri plnej sa zahodi prva.
pub(crate) struct Memo<K, V> {
    cap: usize,
    items: Vec<(K, Rc<V>)>,
}

impl<K: PartialEq, V> Memo<K, V> {
    pub(crate) const fn new(cap: usize) -> Self {
        Memo {
            cap,
            items: Vec::new(),
        }
    }

    /// Hodnota pre `key` (z pamate, inak `make` a ulozi sa).
    pub(crate) fn get(&mut self, key: K, make: impl FnOnce() -> V) -> Rc<V> {
        if let Some(i) = self.items.iter().position(|(k, _)| *k == key) {
            let e = self.items.remove(i);
            let v = e.1.clone();
            self.items.push(e);
            return v;
        }
        let v = Rc::new(make());
        if self.items.len() >= self.cap {
            self.items.remove(0);
        }
        self.items.push((key, v.clone()));
        v
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
}

/// `memo!(NAZOV, kapacita, K, V)`: pamat na vlakno a funkcia `NAZOV.with(|m| m.borrow_mut().get(...))`.
macro_rules! memo {
    ($name:ident, $cap:expr, $k:ty, $v:ty) => {
        thread_local! {
            static $name: RefCell<Memo<$k, $v>> = const { RefCell::new(Memo::new($cap)) };
        }
    };
}

memo!(MARKS, 6, (u32, bool), Vec<crate::brand::Rgba>);
memo!(SPARKS, 6, i32, Vec<u8>);
memo!(MEDIA_ICONS, 16, (crate::icons::MediaIcon, i32), Vec<u8>);
memo!(RINGS, 6, (i32, Option<u8>, bool), (Vec<u8>, Vec<u8>));
memo!(ROUND_RECTS, 8, (i32, i32, i32), Vec<u8>);
memo!(ART, 8, ArtKey, Vec<crate::painter::Color>);

/// Znacka transom `n x n` px (palety pre tmavy / svetly podklad), ako v kapsule ostrova.
pub(crate) fn transom_mark(n: u32, dark: bool) -> Rc<Vec<crate::brand::Rgba>> {
    MARKS.with(|m| {
        m.borrow_mut().get((n, dark), || {
            let palette = if dark {
                crate::brand::Palette::ON_DARK
            } else {
                crate::brand::Palette::ON_LIGHT
            };
            crate::brand::rasterize(
                &crate::brand::mark(crate::brand::Weight::for_px(n)),
                (0.0, 0.0),
                (32.0, 32.0),
                (n, n),
                &palette,
                3,
            )
        })
    })
}

/// [`crate::scene::spark`] z pamate.
pub(crate) fn spark(n: i32) -> Rc<Vec<u8>> {
    SPARKS.with(|m| m.borrow_mut().get(n, || crate::scene::spark(n)))
}

/// [`crate::icons::coverage`] z pamate.
pub(crate) fn media_icon(icon: crate::icons::MediaIcon, n: i32) -> Rc<Vec<u8>> {
    MEDIA_ICONS.with(|m| {
        m.borrow_mut()
            .get((icon, n), || crate::icons::coverage(icon, n))
    })
}

/// [`crate::icons::ring_coverage`] z pamate; `fill` v percentach (ako v `Part::Ring`).
pub(crate) fn ring(n: i32, fill: Option<u8>, ccw: bool) -> Rc<(Vec<u8>, Vec<u8>)> {
    RINGS.with(|m| {
        m.borrow_mut().get((n, fill, ccw), || {
            crate::icons::ring_coverage(n, fill.map(|f| f as f32 / 100.0), ccw)
        })
    })
}

/// [`crate::icons::round_rect_coverage`] s celociselnym polomerom z pamate.
pub(crate) fn round_rect(w: i32, h: i32, r: i32) -> Rc<Vec<u8>> {
    ROUND_RECTS.with(|m| {
        m.borrow_mut().get((w, h, r), || {
            crate::icons::round_rect_coverage(w, h, r as f32)
        })
    })
}

/// Identita zmenseneho obalu: obsah (`id` = hash obsahu a adresa obrazka, ktory pamat drzi slabo, takze adresa sa
/// nemoze pouzit znova pre iny obrazok), velkost a podklad.
pub(crate) struct ArtKey {
    id: u64,
    image: std::sync::Weak<transom_core::image::Image>,
    size: u32,
    bg: u32,
}

impl PartialEq for ArtKey {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && self.size == o.size && self.bg == o.bg && self.image.ptr_eq(&o.image)
    }
}

/// [`transom_core::image::compose_square`] obalu z pamate (ako nepriehladne farby na `blit`).
pub(crate) fn art(
    a: &transom_core::item::ArtImage,
    size: u32,
    bg: u32,
) -> Rc<Vec<crate::painter::Color>> {
    let key = ArtKey {
        id: a.id,
        image: std::sync::Arc::downgrade(&a.image),
        size,
        bg,
    };
    ART.with(|m| {
        m.borrow_mut().get(key, || {
            transom_core::image::compose_square(&a.image, size, bg)
                .into_iter()
                .map(crate::painter::Color::hex)
                .collect()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_at_most_cap_entries_and_drops_the_least_recently_used() {
        let mut m: Memo<u32, u32> = Memo::new(2);
        let mut made = 0;
        let mut get = |m: &mut Memo<u32, u32>, k| {
            *m.get(k, || {
                made += 1;
                k * 10
            })
        };
        assert_eq!(get(&mut m, 1), 10);
        assert_eq!(get(&mut m, 2), 20);
        assert_eq!(get(&mut m, 1), 10, "z pamate");
        assert_eq!(get(&mut m, 3), 30, "zahodi 2 (najdlhsie nepouzita)");
        assert_eq!(m.len(), 2);
        assert_eq!(get(&mut m, 1), 10);
        assert_eq!(get(&mut m, 2), 20, "znova vypocitana");
        drop(get);
        assert_eq!(made, 4);
    }

    #[test]
    fn cached_rasters_equal_a_fresh_computation() {
        for n in [12, 16, 24, 32] {
            assert_eq!(*spark(n), crate::scene::spark(n));
            assert_eq!(*spark(n), crate::scene::spark(n), "aj z pamate");
            for dark in [true, false] {
                let pal = if dark {
                    crate::brand::Palette::ON_DARK
                } else {
                    crate::brand::Palette::ON_LIGHT
                };
                let fresh = crate::brand::rasterize(
                    &crate::brand::mark(crate::brand::Weight::for_px(n as u32)),
                    (0.0, 0.0),
                    (32.0, 32.0),
                    (n as u32, n as u32),
                    &pal,
                    3,
                );
                assert_eq!(*transom_mark(n as u32, dark), fresh);
            }
            for fill in [None, Some(0), Some(37), Some(100)] {
                for ccw in [false, true] {
                    let fresh = crate::icons::ring_coverage(n, fill.map(|f| f as f32 / 100.0), ccw);
                    assert_eq!(*ring(n, fill, ccw), fresh);
                }
            }
            for icon in [
                crate::icons::MediaIcon::Prev,
                crate::icons::MediaIcon::Play,
                crate::icons::MediaIcon::Pause,
                crate::icons::MediaIcon::Next,
            ] {
                assert_eq!(*media_icon(icon, n), crate::icons::coverage(icon, n));
            }
            assert_eq!(
                *round_rect(n * 3, n, 6),
                crate::icons::round_rect_coverage(n * 3, n, 6.0)
            );
        }
    }

    #[test]
    fn a_different_image_with_the_same_id_is_never_served_from_the_cache() {
        use std::sync::Arc;
        use transom_core::image::Image;
        use transom_core::item::ArtImage;
        let a = ArtImage {
            id: 1,
            image: Arc::new(Image::solid(8, 8, 0xFF0000)),
        };
        let b = ArtImage {
            id: 1,
            image: Arc::new(Image::solid(8, 8, 0x00FF00)),
        };
        let red = crate::painter::Color::rgb(255, 0, 0);
        let green = crate::painter::Color::rgb(0, 255, 0);
        assert!(art(&a, 4, 0).iter().all(|c| *c == red));
        assert!(art(&b, 4, 0).iter().all(|c| *c == green));
        assert!(art(&a, 4, 0).iter().all(|c| *c == red));
        // iny podklad = iny vysledok (polopriehladny obal)
        let half = ArtImage {
            id: 2,
            image: Arc::new(Image {
                w: 2,
                h: 2,
                px: vec![0x8000_0000; 4],
            }),
        };
        assert_ne!(*art(&half, 2, 0xFFFFFF), *art(&half, 2, 0));
        assert_eq!(
            *art(&half, 2, 0x123456),
            transom_core::image::compose_square(&half.image, 2, 0x123456)
                .into_iter()
                .map(crate::painter::Color::hex)
                .collect::<Vec<_>>()
        );
    }
}
