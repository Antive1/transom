//! Rozlozenie poloziek na plnosirkovom pase. Iba horizontalne: kazda cast zabera celu vysku lisaty.
//!
//! Pravidla:
//! - Lava skupina (Search, media, oznam) zacina pri lavom okraji; prava skupina (Tray, bateria, hodiny) konci pri
//!   pravom okraji. Stred zostava volny; medzi skupinami je vzdy aspon `center_gap_lp`.
//! - **Stredova skupina lezi presne na polovici lisaty** (jej stred = sirka / 2, +-1 px), nezavisle od sirky lavej a pravej
//!   skupiny; nikdy sa neposuva do zvysneho miesta medzi nimi. Ak by zasiahla do susednej skupiny, skracuje sa/skryva
//!   obsah (najprv pruzny nazov skladby, potom polozky podla `drop_rank`), nie jej poloha.
//! - Pruzne texty (nazov skladby, oznam) dostanu najprv minimum, zvysne miesto sa rozdeli zlava doprava az po ich
//!   prirodzenu sirku (ohranicenu `Flex::max_lp`), takze sa na sirokych monitoroch nerozťahuju.
//! - **Polica** (`ItemId::Shelf`) je "pripojena" polozka stredovej skupiny: ak je v strede aj ina polozka (napr. hodiny),
//!   na polovici lisaty lezi ta (kotva) a subory Police su vedla nej (vlavo alebo vpravo podla poradia v nastaveniach);
//!   ak je Polica v strede sama, lezi na polovici ona.
//! - Pri nedostatku miesta: (1) skryju sa pruzne texty (od polozky s najvyssim `drop_rank`), (2) zahadzuju sa cele
//!   polozky podla `drop_rank`; hodiny (`drop_rank` 0) zostanu vzdy, v krajnom pripade orezane na sirku lisaty.
use crate::item::{ItemId, Part, Side, StatusItem};

/// Rozmery v logickych px (lp); prepocet na px cez `scale`.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub edge_pad_lp: i32,
    pub item_gap_lp: i32,
    pub part_gap_lp: i32,
    pub button_lp: i32,
    /// Vnutorny okraj textu vlavo aj vpravo (aby bol hover obdlznik vzdusny).
    pub text_pad_lp: i32,
    pub center_gap_lp: i32,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            edge_pad_lp: 4,
            item_gap_lp: 8,
            part_gap_lp: 0,
            button_lp: 28,
            text_pad_lp: 6,
            center_gap_lp: 24,
        }
    }
}

/// Umiestnena cast polozky: `x`, `w` v px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub item: ItemId,
    pub part: usize,
    pub x: i32,
    pub w: i32,
}

impl Placed {
    pub fn contains(&self, x: i32) -> bool {
        x >= self.x && x < self.x + self.w
    }
}

fn px(lp: i32, scale: f32) -> i32 {
    (lp as f32 * scale).round() as i32
}

struct PartW {
    natural: i32,
    min: i32,
    flex: bool,
    /// Moze sa skryt v kroku (1): pruzny text alebo obal.
    optional: bool,
    shown: bool,
}

/// Sirka oddelovaca (cast `Part::Divider`) v lp: tenka ciara so vzduchom na oboch stranach.
pub const DIVIDER_LP: i32 = 9;

/// Vypocita rozlozenie. `measure` vrati sirku obsahu casti v px (text bez okrajov, bateria cela; pre ikony sa nevola).
/// Poradie poloziek v `items` urcuje poradie zlava doprava v ramci strany.
///
/// Oddelovace (`ItemId::Separator`) oddeluju len skutocne zobrazene polozky: oddelovac na zaciatku alebo konci skupiny
/// a druhy z dvoch po sebe iducich (napr. ked sa polozka medzi nimi zahodila pre nedostatok miesta) sa skryje a
/// rozlozenie sa prepocita.
pub fn layout(
    items: &[StatusItem],
    width: i32,
    scale: f32,
    m: &Metrics,
    measure: &mut dyn FnMut(&Part) -> i32,
) -> Vec<Placed> {
    let mut hidden = vec![false; items.len()];
    loop {
        let placed = layout_once(items, &hidden, width, scale, m, measure);
        let stray = stray_separators(items, &placed);
        if stray.is_empty() {
            return placed;
        }
        for i in stray {
            hidden[i] = true;
        }
    }
}

/// Oddelovace, ktore po rozlozeni nic neoddeluju: prvy / posledny v skupine alebo hned za inym oddelovacom.
fn stray_separators(items: &[StatusItem], placed: &[Placed]) -> Vec<usize> {
    let idx = |id: ItemId| items.iter().position(|i| i.id == id);
    let mut out = Vec::new();
    for side in [Side::Left, Side::Center, Side::Right] {
        let mut seq: Vec<usize> = Vec::new();
        for p in placed {
            let Some(i) = idx(p.item) else { continue };
            if items[i].side == side && seq.last() != Some(&i) {
                seq.push(i);
            }
        }
        let is_sep = |i: usize| matches!(items[i].id, ItemId::Separator(_));
        for (k, &i) in seq.iter().enumerate() {
            if !is_sep(i) {
                continue;
            }
            let first = k == 0;
            let last = k + 1 == seq.len();
            let doubled = k > 0 && is_sep(seq[k - 1]);
            if first || last || doubled {
                out.push(i);
            }
        }
    }
    out
}

fn layout_once(
    items: &[StatusItem],
    hidden: &[bool],
    width: i32,
    scale: f32,
    m: &Metrics,
    measure: &mut dyn FnMut(&Part) -> i32,
) -> Vec<Placed> {
    let width = width.max(0);
    let pad = px(m.text_pad_lp, scale);
    let mut parts: Vec<Vec<PartW>> = items
        .iter()
        .map(|it| {
            it.parts
                .iter()
                .map(|p| match p {
                    Part::Ring { .. } => {
                        let w = px(m.button_lp, scale) + measure(p).max(0);
                        PartW {
                            natural: w,
                            min: w,
                            flex: false,
                            optional: false,
                            shown: true,
                        }
                    }
                    Part::Icon { .. } | Part::Art { .. } => {
                        let w = px(m.button_lp, scale);
                        PartW {
                            natural: w,
                            min: w,
                            flex: false,
                            optional: matches!(p, Part::Art { .. }),
                            shown: true,
                        }
                    }
                    Part::Text { flex, .. } => {
                        let content = measure(p).max(0) + 2 * pad;
                        match flex {
                            Some(f) => {
                                let natural = content.min(px(f.max_lp, scale)).max(0);
                                PartW {
                                    natural,
                                    min: natural.min(px(f.min_lp, scale)),
                                    flex: true,
                                    optional: true,
                                    shown: true,
                                }
                            }
                            None => PartW {
                                natural: content,
                                min: content,
                                flex: false,
                                optional: false,
                                shown: true,
                            },
                        }
                    }
                    Part::Divider => {
                        let w = px(DIVIDER_LP, scale);
                        PartW {
                            natural: w,
                            min: w,
                            flex: false,
                            optional: false,
                            shown: true,
                        }
                    }
                    Part::Battery { .. }
                    | Part::Device { .. }
                    | Part::File { .. }
                    | Part::Island { .. } => {
                        let w = measure(p).max(0) + 2 * pad;
                        PartW {
                            natural: w,
                            min: w,
                            flex: false,
                            optional: false,
                            shown: true,
                        }
                    }
                })
                .collect()
        })
        .collect();
    let mut visible: Vec<bool> = items
        .iter()
        .zip(hidden)
        .map(|(it, h)| !it.parts.is_empty() && !h)
        .collect();

    let item_gap = px(m.item_gap_lp, scale);
    let part_gap = px(m.part_gap_lp, scale);
    let edge = px(m.edge_pad_lp, scale);
    let center = px(m.center_gap_lp, scale);

    // sirka polozky pri minimach pruznych textov; None = polozka nema ziadnu zobrazenu cast
    // (bez alokacii: rozlozenie bezi pri kazdej snimke posuvu a ekvalizera)
    let item_min = |ps: &[PartW]| -> Option<i32> {
        let (n, sum) = ps
            .iter()
            .filter(|p| p.shown)
            .fold((0, 0), |(n, sum), p| (n + 1, sum + p.min));
        (n > 0).then(|| sum + part_gap * (n - 1))
    };
    // sirka skupiny pri minimach (0 = prazdna)
    let side_w = |parts: &[Vec<PartW>], visible: &[bool], side: Side| -> i32 {
        let (n, sum) = items
            .iter()
            .enumerate()
            .filter(|(i, it)| it.side == side && visible[*i])
            .filter_map(|(i, _)| item_min(&parts[i]))
            .fold((0, 0), |(n, sum), w| (n + 1, sum + w));
        if n == 0 {
            0
        } else {
            sum + item_gap * (n - 1)
        }
    };
    // Stredova skupina pri danych sirkach poloziek (polozka -> sirka, None = nezobrazena): (sirka skupiny, zaciatok a koniec
    // kotvy v skupine). Kotva = polozky okrem Police; bez nich je kotvou cela skupina.
    let center_span = |item_w: &dyn Fn(usize) -> Option<i32>| -> (i32, i32, i32) {
        let (mut x, mut first) = (0, true);
        let (mut a0, mut a1) = (None, 0);
        for (i, it) in items.iter().enumerate() {
            if it.side != Side::Center {
                continue;
            }
            let Some(w) = item_w(i) else { continue };
            if !first {
                x += item_gap;
            }
            first = false;
            if it.id != ItemId::Shelf {
                a0.get_or_insert(x);
                a1 = x + w;
            }
            x += w;
        }
        match a0 {
            Some(a0) => (x, a0, a1),
            None => (x, 0, x),
        }
    };
    // Zmesti sa lista? Bez stredu: lava + prava skupina s medzerou. So stredom: stredova skupina lezi **presne na
    // polovici lisaty** (jej stred = sirka / 2) a nesmie zasiahnut do lavej ani pravej skupiny (s `center` medzerou);
    // stred sa nikdy neposuva do "zvysneho miesta" medzi skupinami - namiesto toho sa skracuje/skryva obsah.
    let fits = |parts: &[Vec<PartW>], visible: &[bool]| -> bool {
        let (l, c, r) = (
            side_w(parts, visible, Side::Left),
            side_w(parts, visible, Side::Center),
            side_w(parts, visible, Side::Right),
        );
        if c == 0 {
            return 2 * edge + center + l + r <= width;
        }
        let (_, a0, a1) = center_span(&|i| {
            (visible[i] && items[i].side == Side::Center)
                .then(|| item_min(&parts[i]))
                .flatten()
        });
        let x0 = (width - (a0 + a1)) / 2;
        let left_bound = if l > 0 { edge + l + center } else { edge };
        let right_bound = if r > 0 {
            width - edge - r - center
        } else {
            width - edge
        };
        x0 >= left_bound && x0 + c <= right_bound
    };

    // poradie poloziek od najskor zahodenej
    let mut by_rank: Vec<usize> = (0..items.len()).collect();
    by_rank.sort_by_key(|&i| std::cmp::Reverse(items[i].drop_rank));

    // (1) skryt volitelne casti (obal, pruzne texty) v poradi casti
    'flex: while !fits(&parts, &visible) {
        for &i in &by_rank {
            if !visible[i] || items[i].drop_rank == 0 {
                continue;
            }
            if let Some(p) = parts[i].iter_mut().find(|p| p.optional && p.shown) {
                p.shown = false;
                continue 'flex;
            }
        }
        break;
    }
    // (2) zahadzovat polozky
    for &i in &by_rank {
        if fits(&parts, &visible) {
            break;
        }
        if items[i].drop_rank > 0 {
            visible[i] = false;
        }
    }
    for i in 0..items.len() {
        if visible[i] && item_min(&parts[i]).is_none() {
            visible[i] = false;
        }
    }

    // rozdelenie zvysku pruznym textom
    let mut widths: Vec<Vec<i32>> = parts
        .iter()
        .map(|ps| ps.iter().map(|p| if p.shown { p.min } else { 0 }).collect())
        .collect();
    let (l_min, c_min, r_min) = (
        side_w(&parts, &visible, Side::Left),
        side_w(&parts, &visible, Side::Center),
        side_w(&parts, &visible, Side::Right),
    );
    // zvysok jednej skupiny: pruzne casti zlava doprava az po prirodzenu sirku, najviac `room` px; vrati pridane px
    let grow = |widths: &mut Vec<Vec<i32>>, side: Side, mut room: i32| -> i32 {
        let mut added = 0;
        for (i, it) in items.iter().enumerate() {
            if it.side != side || !visible[i] {
                continue;
            }
            for (j, p) in parts[i].iter().enumerate() {
                if p.flex && p.shown && room > 0 {
                    let add = (p.natural - p.min).min(room);
                    widths[i][j] += add;
                    room -= add;
                    added += add;
                }
            }
        }
        added
    };
    let centered = c_min > 0 && fits(&parts, &visible);
    if centered {
        // stred ostava na polovici: kazda skupina smie rast len po hranicu, ktoru si stred (aj s rastom) nevyzaduje
        let gap_l = if l_min > 0 { center } else { 0 };
        let gap_r = if r_min > 0 { center } else { 0 };
        // kotva (bez Police) je na polovici: zaciatok skupiny pri minimach
        let (_, a0, a1) = center_span(&|i| {
            (visible[i] && items[i].side == Side::Center)
                .then(|| item_min(&parts[i]))
                .flatten()
        });
        let (s_l, s_r) = (a0, c_min - a1);
        let room_l = (width - (a0 + a1)) / 2 - gap_l - edge - l_min;
        let add_l = grow(&mut widths, Side::Left, room_l.max(0));
        let left_bound = edge + l_min + add_l + gap_l;
        let right_bound = width - edge - r_min - gap_r;
        // kotva rastie symetricky okolo polovice; Polica (pevne sirky) vedla nej rast obmedzuje zo svojej strany
        let a_max = (width - 2 * (left_bound + s_l)).min(2 * (right_bound - s_r) - width);
        let add_c = grow(&mut widths, Side::Center, (a_max - (a1 - a0)).max(0));
        let c_now = c_min + add_c;
        let x0 = (width - (2 * s_l + (a1 - a0) + add_c)) / 2;
        let room_r = (width - edge - gap_r - (x0 + c_now)) - r_min;
        grow(&mut widths, Side::Right, room_r.max(0));
    } else {
        let total = 2 * edge + center + l_min + c_min + r_min + if c_min > 0 { center } else { 0 };
        let mut extra = (width - total).max(0);
        for side in [Side::Left, Side::Center, Side::Right] {
            extra -= grow(&mut widths, side, extra);
        }
    }

    let mut out = Vec::new();
    let place_group = |out: &mut Vec<Placed>, side: Side, start: i32| -> i32 {
        let mut x = start;
        let mut first = true;
        for (i, it) in items.iter().enumerate() {
            if it.side != side || !visible[i] {
                continue;
            }
            if !first {
                x += item_gap;
            }
            first = false;
            let mut first_part = true;
            for (j, p) in parts[i].iter().enumerate() {
                if !p.shown {
                    continue;
                }
                if !first_part {
                    x += part_gap;
                }
                first_part = false;
                out.push(Placed {
                    item: it.id,
                    part: j,
                    x,
                    w: widths[i][j],
                });
                x += widths[i][j];
            }
        }
        x - start
    };
    place_group(&mut out, Side::Left, edge);
    // prava skupina: najprv zmerat (umiestnenim od 0), potom posunut k pravemu okraju
    let n_left = out.len();
    // ak sa prava skupina nezmesti (zostali len hodiny), zarovnat ju dolava a orezat sprava
    let left_end = out.last().map(|p| p.x + p.w).unwrap_or(0);
    let right_w = place_group(&mut out, Side::Right, 0);
    let shift = (width - edge - right_w).max(left_end);
    for p in out.iter_mut().skip(n_left) {
        p.x += shift;
    }
    // stredova skupina: presne v strede lisaty (jej stred = sirka / 2, s presnostou na 1 px); len v krajnom pripade,
    // ked sa nezmesti ani po skrateni a zahodeni poloziek, sa posunie k lavej skupine (a pripadne orezie sprava)
    let right_start = out
        .iter()
        .skip(n_left)
        .map(|p| p.x)
        .min()
        .unwrap_or(width - edge);
    let n_before_center = out.len();
    let center_w = place_group(&mut out, Side::Center, 0);
    if center_w > 0 {
        let lo = if n_left > 0 { left_end + center } else { edge };
        let hi = if out.len() > n_left && n_before_center > n_left {
            right_start - center - center_w
        } else {
            width - edge - center_w
        };
        // kotva (polozky okrem Police) na polovici lisaty; Polica vedla nej
        let placed_c = &out[n_before_center..];
        let anchor: Vec<&Placed> = placed_c
            .iter()
            .filter(|p| p.item != ItemId::Shelf)
            .collect();
        let (a0, a1) = match (anchor.first(), anchor.last()) {
            (Some(f), Some(l)) => (f.x, l.x + l.w),
            _ => (0, center_w),
        };
        let ideal = (width - (a0 + a1)) / 2;
        let x0 = if centered {
            ideal
        } else if hi >= lo {
            ideal.clamp(lo, hi)
        } else if n_left > 0 {
            left_end
        } else {
            0
        };
        for p in out.iter_mut().skip(n_before_center) {
            p.x += x0;
        }
        // zoradit podla x (hit-testing a invarianty predpokladaju poradie zlava doprava)
        out.sort_by_key(|p| p.x);
    }
    // krajny pripad (len hodiny a uzsie nez lista): orezat do <0, width>
    for p in out.iter_mut() {
        if p.x < 0 {
            p.w += p.x;
            p.x = 0;
        }
        if p.x + p.w > width {
            p.w = (width - p.x).max(0);
        }
        p.w = p.w.max(0);
    }
    out.retain(|p| p.w > 0 || width == 0);
    out
}

/// Ktora cast polozky je pod suradnicou `x` (px).
pub fn hit_test(placed: &[Placed], x: i32) -> Option<(ItemId, usize)> {
    placed
        .iter()
        .find(|p| p.contains(x))
        .map(|p| (p.item, p.part))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Action, Flex, Glyph, Tone};

    fn icon(a: Action) -> Part {
        Part::Icon {
            glyph: Glyph::Search,
            action: Some(a),
            enabled: true,
            name: String::new(),
        }
    }
    fn flex_text(t: &str) -> Part {
        Part::Text {
            text: t.into(),
            flex: Some(Flex {
                min_lp: 60,
                max_lp: 400,
            }),
            tone: Tone::Normal,
            action: None,
        }
    }

    /// Typicka lista: Search, media (3 tlacidla + nazov), oznam; Tray, bateria, hodiny.
    fn bar() -> Vec<StatusItem> {
        vec![
            StatusItem::new(ItemId::Search, Side::Left, vec![icon(Action::Launcher)]),
            StatusItem::new(
                ItemId::Media,
                Side::Left,
                vec![
                    Part::Art {
                        art: None,
                        action: None,
                    },
                    icon(Action::MediaPrev),
                    icon(Action::MediaToggle),
                    icon(Action::MediaNext),
                    flex_text("Artist – A very long song title that goes on"),
                ],
            ),
            StatusItem::new(ItemId::Tools, Side::Right, vec![icon(Action::ToolsPanel)]),
            StatusItem::new(
                ItemId::Battery,
                Side::Right,
                vec![
                    Part::Battery {
                        percent: 50,
                        charging: false,
                    },
                    Part::text("50 %"),
                ],
            ),
            StatusItem::new(ItemId::Clock, Side::Right, vec![Part::text("14:05")]),
        ]
    }

    /// 7 px na znak; bateria 24 px.
    fn measure(p: &Part) -> i32 {
        match p {
            Part::Text { text, .. } => 7 * text.chars().count() as i32,
            Part::Battery { .. } => 24,
            Part::Device { label, .. } => 40 + 7 * label.chars().count() as i32,
            Part::File { label, .. } => 20 + 7 * label.chars().count() as i32,
            Part::Island { label, .. } => 40 + 7 * label.chars().count() as i32,
            Part::Icon { .. } | Part::Art { .. } | Part::Ring { .. } | Part::Divider => 0,
        }
    }

    fn run(items: &[StatusItem], width: i32, scale: f32) -> Vec<Placed> {
        layout(items, width, scale, &Metrics::default(), &mut measure)
    }

    fn ids(p: &[Placed]) -> Vec<ItemId> {
        let mut v: Vec<ItemId> = p.iter().map(|x| x.item).collect();
        v.dedup();
        v
    }

    /// Spolocne invarianty pre kazdu sirku.
    fn check_invariants(items: &[StatusItem], p: &[Placed], width: i32) {
        for x in p {
            assert!(x.w >= 0, "zaporna sirka {x:?}");
            assert!(
                x.x >= 0 && x.x + x.w <= width,
                "mimo lisaty {x:?} sirka={width}"
            );
        }
        for w in p.windows(2) {
            assert!(w[0].x + w[0].w <= w[1].x, "prekryv {:?} {:?}", w[0], w[1]);
        }
        let side_of = |id: ItemId| items.iter().find(|i| i.id == id).unwrap().side;
        let left_end = p
            .iter()
            .filter(|x| side_of(x.item) == Side::Left)
            .map(|x| x.x + x.w)
            .max();
        let right_start = p
            .iter()
            .filter(|x| side_of(x.item) == Side::Right)
            .map(|x| x.x)
            .min();
        if let (Some(l), Some(r)) = (left_end, right_start) {
            assert!(l <= r, "lava skupina zasahuje do pravej: {l} > {r}");
        }
        // hodiny (drop_rank 0) zostavaju vzdy, ak su medzi polozkami (pouzivatel ich moze skryt v nastaveniach)
        if items.iter().any(|i| i.id == ItemId::Clock) {
            assert!(
                p.iter().any(|x| x.item == ItemId::Clock) || width == 0,
                "hodiny chybaju pri sirke {width}"
            );
        }
        // stredova skupina lezi medzi lavou a pravou
        let center: Vec<&Placed> = p
            .iter()
            .filter(|x| side_of(x.item) == Side::Center)
            .collect();
        if let (Some(c0), Some(c1)) = (center.first(), center.last()) {
            if let Some(l) = left_end {
                assert!(l <= c0.x, "stred zasahuje do lavej skupiny");
            }
            if let Some(r) = right_start {
                assert!(c1.x + c1.w <= r, "stred zasahuje do pravej skupiny");
            }
        }
    }

    #[test]
    fn wide_bar_keeps_everything_and_groups_at_edges() {
        let items = bar();
        let p = run(&items, 2560, 1.0);
        check_invariants(&items, &p, 2560);
        assert_eq!(
            ids(&p),
            vec![
                ItemId::Search,
                ItemId::Media,
                ItemId::Tools,
                ItemId::Battery,
                ItemId::Clock
            ]
        );
        assert_eq!(p[0].x, 4, "lava skupina zacina pri okraji");
        let last = p.last().unwrap();
        assert_eq!(last.x + last.w, 2560 - 4, "hodiny konci pri pravom okraji");
        // nazov ma prirodzenu sirku (nerozťahuje sa)
        let title = p
            .iter()
            .find(|x| x.item == ItemId::Media && x.part == 4)
            .unwrap();
        assert_eq!(title.w, 7 * 44 + 12);
    }

    #[test]
    fn flex_text_is_capped_at_max() {
        let mut items = bar();
        items[1].parts[4] = flex_text(&"x".repeat(200));
        let p = run(&items, 3840, 1.0);
        let title = p
            .iter()
            .find(|x| x.item == ItemId::Media && x.part == 4)
            .unwrap();
        assert_eq!(title.w, 400);
    }

    #[test]
    fn narrow_bar_shrinks_title_before_dropping_items() {
        let items = bar();
        // pevne casti: vlavo 4 tlacidla (112) + medzera; vpravo 28 + bateria (36+...) + hodiny
        let p = run(&items, 420, 1.0);
        check_invariants(&items, &p, 420);
        assert_eq!(ids(&p).len(), 5, "vsetky polozky zostali: {p:?}");
        let title = p
            .iter()
            .find(|x| x.item == ItemId::Media && x.part == 4)
            .unwrap();
        assert!(
            title.w >= 60 && title.w < 7 * 44 + 12,
            "nazov skrateny: {}",
            title.w
        );
    }

    #[test]
    fn very_narrow_bar_hides_title_then_drops_by_rank() {
        let items = bar();
        let mut prev = usize::MAX;
        for width in [400, 300, 250, 200, 150, 100, 80, 50, 30, 10, 1, 0] {
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
            let n = ids(&p).len();
            assert!(n <= prev, "pri uzsej sirke pribudla polozka ({width})");
            prev = n;
        }
        // pri 200 px: nazov skryty, media alebo dalsie zahodene, hodiny ostavaju
        let p = run(&items, 200, 1.0);
        assert!(!p.iter().any(|x| x.item == ItemId::Media && x.part == 4));
        assert!(p.iter().any(|x| x.item == ItemId::Clock));
        // ziadna sirka nesmie zahodit hodiny skor nez ostatne
        let p = run(&items, 60, 1.0);
        assert_eq!(ids(&p), vec![ItemId::Clock]);
    }

    #[test]
    fn drop_order_media_then_tray_then_search_then_battery() {
        let items = bar();
        let order: Vec<Vec<ItemId>> = (0..=600)
            .rev()
            .step_by(5)
            .map(|w| ids(&run(&items, w, 1.0)))
            .collect();
        let first_missing = |id: ItemId| {
            order
                .iter()
                .position(|v| !v.contains(&id))
                .unwrap_or(usize::MAX)
        };
        assert!(first_missing(ItemId::Media) <= first_missing(ItemId::Tools));
        assert!(first_missing(ItemId::Tools) <= first_missing(ItemId::Search));
        assert!(first_missing(ItemId::Search) <= first_missing(ItemId::Battery));
        assert!(first_missing(ItemId::Battery) < first_missing(ItemId::Clock));
    }

    #[test]
    fn art_hides_before_title_and_controls_stay() {
        let items = bar();
        let has =
            |p: &[Placed], part: usize| p.iter().any(|x| x.item == ItemId::Media && x.part == part);
        // dost miesta: obal aj nazov
        let p = run(&items, 2560, 1.0);
        assert!(has(&p, 0) && has(&p, 4));
        // najdi sirku, kde zmizne obal: nazov tam este musi byt (aspon na minime)
        let w_art = (100..2560)
            .rev()
            .find(|&w| !has(&run(&items, w, 1.0), 0))
            .unwrap();
        let p = run(&items, w_art, 1.0);
        check_invariants(&items, &p, w_art);
        assert!(has(&p, 4), "pri sirke {w_art} zmizol obal skor nez nazov");
        assert!(has(&p, 1) && has(&p, 2) && has(&p, 3), "tlacidla zostavaju");
        // este uzsie: nazov zmizne, tlacidla zostanu, kym sa nezahodi cela polozka
        let w_title = (100..w_art)
            .rev()
            .find(|&w| !has(&run(&items, w, 1.0), 4))
            .unwrap();
        let p = run(&items, w_title, 1.0);
        assert!(has(&p, 2), "prehrat/pauza ostava pri sirke {w_title}");
    }

    /// Lista s hodinami v strede (casta volba v nastaveniach).
    fn bar_center_clock() -> Vec<StatusItem> {
        let mut v = bar();
        for it in v.iter_mut() {
            if it.id == ItemId::Clock {
                it.side = Side::Center;
            }
        }
        v
    }

    #[test]
    fn center_group_is_centered_when_there_is_room() {
        let items = bar_center_clock();
        let p = run(&items, 2560, 1.0);
        check_invariants(&items, &p, 2560);
        let c = p.iter().find(|x| x.item == ItemId::Clock).unwrap();
        let mid = c.x + c.w / 2;
        assert!((mid - 1280).abs() <= 1, "stred hodin {mid}");
    }

    fn shelf(n: usize) -> StatusItem {
        let parts = (0..n)
            .map(|i| Part::File {
                glyph: Glyph::File,
                muted: false,
                label: format!("subor-{i}.pdf"),
                name: String::new(),
                action: Some(Action::ShelfFile),
            })
            .collect();
        StatusItem::new(ItemId::Shelf, Side::Center, parts)
    }

    fn span(p: &[Placed], id: ItemId) -> Option<(i32, i32)> {
        let v: Vec<&Placed> = p.iter().filter(|x| x.item == id).collect();
        Some((v.first()?.x, v.last()?.x + v.last()?.w))
    }

    #[test]
    fn a_lone_shelf_sits_at_the_midpoint_of_the_bar() {
        let mut items = bar();
        items.push(shelf(3));
        for width in [1200, 1920, 2560] {
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
            let (a, b) = span(&p, ItemId::Shelf).unwrap();
            assert!(
                ((a + b) / 2 - width / 2).abs() <= 1,
                "sirka {width}: {a}..{b}"
            );
        }
    }

    #[test]
    fn with_another_center_item_the_shelf_attaches_beside_it_and_the_item_stays_centered() {
        // hodiny v strede, Polica za nimi (vpravo) / pred nimi (vlavo) podla poradia
        for shelf_first in [false, true] {
            let mut items = bar_center_clock();
            let clock = items.iter().position(|i| i.id == ItemId::Clock).unwrap();
            let at = if shelf_first { clock } else { clock + 1 };
            items.insert(at, shelf(2));
            for width in [1400, 1920, 2560] {
                let p = run(&items, width, 1.0);
                check_invariants(&items, &p, width);
                let (c0, c1) = span(&p, ItemId::Clock).unwrap();
                assert!(
                    ((c0 + c1) / 2 - width / 2).abs() <= 1,
                    "hodiny nie su v strede pri {width}"
                );
                let (s0, s1) = span(&p, ItemId::Shelf).expect("Polica sa zmesti");
                if shelf_first {
                    assert!(s1 <= c0, "Polica vlavo od hodin");
                } else {
                    assert!(s0 >= c1, "Polica vpravo od hodin");
                }
            }
        }
    }

    #[test]
    fn on_a_narrow_bar_the_shelf_goes_before_the_clock_and_nothing_overlaps() {
        let mut items = bar_center_clock();
        items.push(shelf(4));
        let mut dropped_somewhere = false;
        for width in (160..=2600).rev().step_by(17) {
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
            match span(&p, ItemId::Shelf) {
                Some(_) => {
                    let (c0, c1) = span(&p, ItemId::Clock).unwrap();
                    assert!(((c0 + c1) / 2 - width / 2).abs() <= 1, "sirka {width}");
                }
                None => dropped_somewhere = true,
            }
        }
        assert!(dropped_somewhere, "pri uzkej liste sa Polica skryje");
    }

    #[test]
    fn a_flexible_center_title_grows_around_the_midpoint_without_running_into_the_shelf() {
        let mut items = bar_media_center();
        items.push(shelf(3));
        for width in (300..=2600).rev().step_by(23) {
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
            let all_media = p
                .iter()
                .filter(|x| x.item == ItemId::Media && x.part >= 1)
                .count()
                == 4;
            if all_media && span(&p, ItemId::Shelf).is_some() {
                let m = center_mid(&p).unwrap();
                assert!((m - width / 2).abs() <= 1, "sirka {width}: stred medii {m}");
            }
        }
    }

    /// Media (tlacidla + pruzny nazov) v strede, Search vlavo, Tray/bateria/hodiny vpravo.
    fn bar_media_center() -> Vec<StatusItem> {
        let mut v = bar();
        for it in v.iter_mut() {
            if it.id == ItemId::Media {
                it.side = Side::Center;
            }
        }
        v
    }

    fn center_mid(p: &[Placed]) -> Option<i32> {
        let c: Vec<&Placed> = p.iter().filter(|x| x.item == ItemId::Media).collect();
        let (a, b) = (c.first()?, c.last()?);
        Some((a.x + b.x + b.w) / 2)
    }

    #[test]
    fn center_group_stays_at_the_exact_midpoint_while_sides_change_width() {
        let width = 1600;
        // roznorode sirky lavej aj pravej strany (dlhy oznam vlavo, dlhe hodiny/datum vpravo): stred sa nehybe
        let mut mids = Vec::new();
        for (left_extra, clock) in [
            (0usize, "14:05"),
            (30, "14:05"),
            (0, "streda 20. novembra 14:05:33"),
            (60, "x"),
            (10, "1"),
        ] {
            let mut items = bar_media_center();
            if left_extra > 0 {
                items.insert(
                    0,
                    StatusItem::new(
                        ItemId::Notice,
                        Side::Left,
                        vec![flex_text(&"n".repeat(left_extra))],
                    ),
                );
            }
            for it in items.iter_mut() {
                if it.id == ItemId::Clock {
                    it.parts = vec![Part::text(clock)];
                }
            }
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
            mids.push(center_mid(&p).unwrap());
        }
        for m in &mids {
            assert!(
                (m - width / 2).abs() <= 1,
                "stred {m} != {} ({mids:?})",
                width / 2
            );
        }
    }

    #[test]
    fn center_title_shrinks_first_and_the_center_never_drifts_or_overlaps() {
        let items = bar_media_center();
        let natural = 7 * 44 + 12;
        let mut shrunk_somewhere = false;
        for width in (200..=2600).rev().step_by(13) {
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
            let controls = p
                .iter()
                .filter(|x| x.item == ItemId::Media && x.part >= 1)
                .count();
            if controls == 4 {
                // media (tlacidla aj nazov) je zobrazene: presne v strede
                let m = center_mid(&p).unwrap();
                assert!((m - width / 2).abs() <= 1, "sirka {width}: stred {m}");
            }
            if let Some(t) = p.iter().find(|x| x.item == ItemId::Media && x.part == 4) {
                assert!(t.w <= natural);
                shrunk_somewhere |= t.w < natural;
                // nazov sa nikdy nezuzi pod minimum (inak sa skryje cely)
                assert!(t.w >= 60, "sirka {width}: nazov {}", t.w);
            }
        }
        assert!(shrunk_somewhere, "pri uzkych listach sa nazov skracuje");
        // nazov je skryty skor, nez by sa stred posunul: pri sirke, ked sa nazov uz nezmesti, stred stale sedi
        let mut hidden_title_centered = false;
        for width in (300..=1200).rev().step_by(5) {
            let p = run(&items, width, 1.0);
            let has_title = p.iter().any(|x| x.item == ItemId::Media && x.part == 4);
            let has_controls = p.iter().any(|x| x.item == ItemId::Media && x.part == 3);
            if !has_title && has_controls {
                let m = center_mid(&p).unwrap();
                assert!((m - width / 2).abs() <= 1);
                hidden_title_centered = true;
            }
        }
        assert!(
            hidden_title_centered,
            "existuje sirka so skrytym nazvom a vycentrovanymi tlacidlami"
        );
    }

    #[test]
    fn center_midpoint_holds_at_other_dpi_scales() {
        let items = bar_media_center();
        for scale in [1.0f32, 1.5, 2.0] {
            for width in [1080, 1920, 2560] {
                let w = (width as f32 * scale) as i32;
                let p = layout(&items, w, scale, &Metrics::default(), &mut measure);
                check_invariants(&items, &p, w);
                if p.iter().any(|x| x.item == ItemId::Media && x.part == 3) {
                    let m = center_mid(&p).unwrap();
                    assert!((m - w / 2).abs() <= 1, "{scale} {w}: {m}");
                }
            }
        }
    }

    #[test]
    fn center_group_stays_centered_instead_of_taking_the_leftover_space() {
        // lava skupina je ovela sirsia nez prava; stred nesmie "odist" do zvysneho miesta
        let mut items = bar_center_clock();
        items.insert(
            0,
            StatusItem::new(ItemId::Notice, Side::Left, vec![flex_text(&"n".repeat(40))]),
        );
        let p = run(&items, 1400, 1.0);
        check_invariants(&items, &p, 1400);
        let c = p.iter().find(|x| x.item == ItemId::Clock).unwrap();
        assert!(((c.x + c.w / 2) - 700).abs() <= 1);
    }

    #[test]
    fn center_group_shifts_instead_of_overlapping() {
        let items = bar_center_clock();
        // lava skupina (Search + media ~ 450 px) presahuje polovicu pri sirke 900 -> stred sa posunie doprava
        let p = run(&items, 900, 1.0);
        check_invariants(&items, &p, 900);
        let c = p.iter().find(|x| x.item == ItemId::Clock).unwrap();
        let left_end = p
            .iter()
            .filter(|x| matches!(x.item, ItemId::Search | ItemId::Media))
            .map(|x| x.x + x.w)
            .max()
            .unwrap();
        assert!(c.x >= left_end + 24, "aspon center_gap medzi skupinami");
    }

    #[test]
    fn center_group_survives_narrow_widths_by_drop_rank() {
        let items = bar_center_clock();
        for width in (0..=1200).rev().step_by(7) {
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
        }
        // velmi uzko: zostanu len hodiny (drop_rank 0), aj v strede
        assert_eq!(ids(&run(&items, 70, 1.0)), vec![ItemId::Clock]);
    }

    #[test]
    fn only_center_items() {
        let items = vec![StatusItem::new(
            ItemId::Clock,
            Side::Center,
            vec![Part::text("14:05")],
        )];
        let p = run(&items, 1000, 1.0);
        check_invariants(&items, &p, 1000);
        assert!((p[0].x + p[0].w / 2 - 500).abs() <= 1);
    }

    #[test]
    fn hidden_clock_is_allowed() {
        let items: Vec<StatusItem> = bar()
            .into_iter()
            .filter(|i| i.id != ItemId::Clock)
            .collect();
        for width in [2560, 400, 100, 0] {
            let p = run(&items, width, 1.0);
            check_invariants(&items, &p, width);
        }
    }

    #[test]
    fn clock_alone_is_clipped_not_dropped() {
        let items = vec![StatusItem::new(
            ItemId::Clock,
            Side::Right,
            vec![Part::text("14:05")],
        )];
        let p = run(&items, 20, 1.0);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].x, p[0].w), (0, 20));
    }

    #[test]
    fn scale_multiplies_fixed_sizes() {
        let items = vec![StatusItem::new(
            ItemId::Search,
            Side::Left,
            vec![icon(Action::Search)],
        )];
        let p = run(&items, 1000, 1.5);
        assert_eq!((p[0].x, p[0].w), (6, 42));
    }

    #[test]
    fn empty_items_are_skipped_and_hit_test_works() {
        let mut items = bar();
        items.push(StatusItem::new(ItemId::Notice, Side::Left, vec![]));
        let p = run(&items, 2560, 1.0);
        assert!(!p.iter().any(|x| x.item == ItemId::Notice));
        let search = p[0];
        assert_eq!(hit_test(&p, search.x + 1), Some((ItemId::Search, 0)));
        assert_eq!(hit_test(&p, 1280), None, "stred je inertny");
    }

    /// Jednoduchy deterministicky generator (bez zavislosti): xorshift64*.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    const ALL_IDS: [ItemId; 23] = [
        ItemId::Search,
        ItemId::Media,
        ItemId::Notice,
        ItemId::Tools,
        ItemId::Battery,
        ItemId::Clock,
        ItemId::Ai,
        ItemId::Audio,
        ItemId::Peripherals,
        ItemId::Shelf,
        ItemId::Keyboard,
        ItemId::Timer,
        ItemId::Brightness,
        ItemId::SysMon,
        ItemId::Privacy,
        ItemId::ToolShelf,
        ItemId::ToolClipboard,
        ItemId::ToolNotes,
        ItemId::ToolShots,
        ItemId::Separator(1),
        ItemId::Separator(2),
        ItemId::Separator(3),
        ItemId::Search,
    ];

    /// Nahodna cast: ikona, kruh s textom, pevny / pruzny (aj velmi dlhy) text, bateria, periferia, subor, obal.
    fn random_part(r: &mut Rng) -> Part {
        let long = "Ž".repeat(1 + r.below(120) as usize);
        match r.below(8) {
            0 => icon(Action::ToolsPanel),
            1 => Part::Ring {
                fill: Some(r.below(101) as u8),
                ccw: false,
                muted: false,
                alert: false,
                label: format!("{} %", r.below(101)),
                name: String::new(),
                action: None,
            },
            2 => Part::text(&long),
            3 => flex_text(&long),
            4 => Part::Battery {
                percent: 50,
                charging: false,
            },
            5 => Part::Device {
                glyph: Glyph::Search,
                percent: Some(40),
                charging: false,
                muted: false,
                low: false,
                label: long,
                name: String::new(),
                action: None,
            },
            6 => Part::File {
                glyph: Glyph::Search,
                muted: false,
                label: long,
                name: String::new(),
                action: None,
            },
            _ => Part::Art {
                art: None,
                action: None,
            },
        }
    }

    /// Kazda polozka na nahodnej strane (aj vsetky na jednej), nahodne casti, sirky 0..6000 px, mierky 1-3:
    /// nic sa neprekryva, vsetko je v liste, hodiny ostanu, casti polozky idu v poradi a `hit_test` sedi s rozlozenim.
    #[test]
    fn random_bars_keep_every_invariant_even_with_everything_on_one_side() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..4000 {
            let one_side = match case % 4 {
                0 => Some(Side::Left),
                1 => Some(Side::Center),
                2 => Some(Side::Right),
                _ => None,
            };
            let n = 1 + r.below(ALL_IDS.len() as u64 - 1) as usize;
            let mut items: Vec<StatusItem> = Vec::new();
            // nahodne poradie (oddelovace aj vedla seba, na krajoch, medzi polozkami)
            let mut pool: Vec<ItemId> = ALL_IDS[..22].to_vec();
            for i in (1..pool.len()).rev() {
                pool.swap(i, r.below(i as u64 + 1) as usize);
            }
            for &id in pool.iter().take(n.max(4)) {
                let side = one_side.unwrap_or(match r.below(3) {
                    0 => Side::Left,
                    1 => Side::Center,
                    _ => Side::Right,
                });
                // hodiny su v skutocnosti vzdy text (obal alebo pruzny text by sa smel skryt)
                let parts = if id == ItemId::Clock {
                    vec![Part::text("Wednesday, November 25, 2026 10:51")]
                } else if matches!(id, ItemId::Separator(_)) {
                    vec![Part::Divider]
                } else {
                    (0..1 + r.below(5)).map(|_| random_part(&mut r)).collect()
                };
                items.push(StatusItem::new(id, side, parts));
            }
            let width = match r.below(4) {
                // uzsia lista nez ~2 okraje nema miesto ani na orezane hodiny (realny monitor ma aspon stovky px)
                0 => 120 + r.below(400) as i32,
                1 => 400 + r.below(1200) as i32,
                _ => 1600 + r.below(4400) as i32,
            };
            let scale = [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0][r.below(7) as usize];
            let p = run(&items, width, scale);
            check_invariants(&items, &p, width);
            // casti jednej polozky su spolu a v poradi; polozky stranou v poradi z nastaveni
            for w in p.windows(2) {
                if w[0].item == w[1].item {
                    assert!(
                        w[0].part < w[1].part,
                        "casti mimo poradia {w:?} (pripad {case})"
                    );
                }
            }
            let order: Vec<ItemId> = items.iter().map(|i| i.id).collect();
            let placed = ids(&p);
            let mut pos = placed
                .iter()
                .map(|id| order.iter().position(|o| o == id).unwrap());
            let mut last = None;
            for (id, i) in placed.iter().zip(pos.by_ref()) {
                let side = items[i].side;
                if let Some((ls, li)) = last {
                    if ls == side {
                        assert!(li < i, "{id:?} mimo poradia v skupine (pripad {case})");
                    }
                }
                last = Some((side, i));
            }
            // oddelovac vzdy medzi dvoma zobrazenymi polozkami tej istej skupiny, nikdy na kraji ani dva po sebe
            for side in [Side::Left, Side::Center, Side::Right] {
                let seq: Vec<ItemId> = placed
                    .iter()
                    .copied()
                    .filter(|id| items.iter().any(|i| i.id == *id && i.side == side))
                    .collect();
                for (k, id) in seq.iter().enumerate() {
                    if matches!(id, ItemId::Separator(_)) {
                        assert!(
                            k > 0 && k + 1 < seq.len(),
                            "oddelovac na kraji {seq:?} (pripad {case})"
                        );
                        assert!(
                            !matches!(seq[k + 1], ItemId::Separator(_)),
                            "dva oddelovace po sebe {seq:?} (pripad {case})"
                        );
                    }
                }
            }
            // kazda polozka je umiestnena najviac raz (jeden suvisly usek)
            let mut seen = placed.clone();
            seen.sort_by_key(|id| format!("{id:?}"));
            let before = seen.len();
            seen.dedup();
            assert_eq!(
                before,
                seen.len(),
                "polozka rozdelena na dve miesta (pripad {case})"
            );
            for x in &p {
                if x.w > 0 {
                    assert_eq!(
                        hit_test(&p, x.x + x.w / 2),
                        Some((x.item, x.part)),
                        "hit_test (pripad {case})"
                    );
                }
            }
        }
    }
}
