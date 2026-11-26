//! Ciste rozhodnutie drag & drop v editore rozlozenia: kam pustit polozku (skupina a poradie) podla polohy mysi.
//! Kazda skupina ukazuje vsetky svoje polozky (viditelne aj skryte); viditelnost meni len "Zobrazit v liste".
use transom_config::{Group, Layout};

/// Obdlznik stlpca (skupiny) a jeho poloziek (v poradi zhora nadol).
#[derive(Clone, Debug)]
pub struct ColGeom {
    pub group: Group,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// (id, y stredu polozky)
    pub chips: Vec<(String, i32)>,
}

/// Ciel pustenia: skupina a index medzi polozkami skupiny (bez tahanej polozky).
pub fn drop_target(cols: &[ColGeom], dragged: &str, x: i32, y: i32) -> Option<(Group, usize)> {
    let c = cols
        .iter()
        .find(|c| x >= c.x && x < c.x + c.w && y >= c.y && y < c.y + c.h)?;
    let idx = c
        .chips
        .iter()
        .filter(|(id, _)| id != dragged)
        .take_while(|(_, cy)| *cy < y)
        .count();
    Some((c.group, idx))
}

/// Vykona pustenie v konfiguracii: presun v ramci/medzi skupinami, viditelnost sa nemeni. `target.1` je index medzi
/// **zobrazenymi** (znamymi) polozkami cielovej skupiny bez presuvanej; nezname polozky (z buducej verzie) ostanu v subore
/// a poradie znamych sa nenarusi (preklad na index medzi vsetkymi polozkami skupiny).
pub fn apply_drop(layout: &mut Layout, id: &str, target: (Group, usize)) {
    let (g, k) = target;
    let others: Vec<&str> = layout
        .in_group(g)
        .filter(|s| s.id != id)
        .map(|s| s.id.as_str())
        .collect();
    let known: Vec<usize> = others
        .iter()
        .enumerate()
        .filter(|(_, i)| transom_config::is_layout_id(i))
        .map(|(pos, _)| pos)
        .collect();
    let at = match known.get(k) {
        Some(&pos) => pos,
        // za poslednu znamu polozku (nezname na konci skupiny ostanu za nou)
        None => known.last().map_or(others.len(), |&pos| pos + 1),
    };
    layout.move_within(id, g, at);
}

/// Smer klavesnicoveho presunu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
}

/// Skupina a poradie (medzi zobrazenymi polozkami) polozky `id`.
pub fn position_of(layout: &Layout, id: &str) -> Option<(Group, usize)> {
    let slot = layout.items.iter().find(|s| s.id == id)?;
    let idx = column_items(layout, slot.group)
        .iter()
        .position(|x| x == id)?;
    Some((slot.group, idx))
}

fn neighbour(g: Group, right: bool) -> Option<Group> {
    match (g, right) {
        (Group::Left, true) => Some(Group::Center),
        (Group::Center, true) => Some(Group::Right),
        (Group::Right, false) => Some(Group::Center),
        (Group::Center, false) => Some(Group::Left),
        _ => None,
    }
}

/// Cil klavesnicoveho presunu v tvare ako [`drop_target`] (index medzi zobrazenymi polozkami cielovej skupiny bez presuvanej).
/// `None` = polozka je uz na okraji alebo nie je zobrazena (nezname id).
pub fn move_target(layout: &Layout, id: &str, d: Dir) -> Option<(Group, usize)> {
    let (g, i) = position_of(layout, id)?;
    let n = column_items(layout, g).len();
    match d {
        Dir::Up => i.checked_sub(1).map(|j| (g, j)),
        Dir::Down => (i + 1 < n).then_some((g, i + 1)),
        Dir::Home => (i > 0).then_some((g, 0)),
        Dir::End => n
            .checked_sub(1)
            .filter(|&last| i < last)
            .map(|last| (g, last)),
        Dir::Left | Dir::Right => {
            let tg = neighbour(g, d == Dir::Right)?;
            // do susedneho stlpca na rovnaku poziciu (najviac na koniec)
            Some((tg, i.min(column_items(layout, tg).len())))
        }
    }
}

/// Presunie polozku klavesnicou; vrati jej novu skupinu a poradie (`None` = nic sa nezmenilo).
#[cfg(test)]
pub fn move_by_key(layout: &mut Layout, id: &str, d: Dir) -> Option<(Group, usize)> {
    let t = move_target(layout, id, d)?;
    apply_drop(layout, id, t);
    position_of(layout, id)
}

/// Rozpracovany klavesnicovy presun: stav pred zaciatkom (na zrusenie Esc).
#[derive(Clone, Debug)]
pub struct MoveSession {
    pub id: String,
    before: Layout,
}

impl MoveSession {
    pub fn begin(layout: &Layout, id: &str) -> Self {
        Self {
            id: id.to_string(),
            before: layout.clone(),
        }
    }

    /// Obnovi presne povodne rozlozenie.
    pub fn cancel(self, layout: &mut Layout) {
        *layout = self.before;
    }

    /// Zmenilo sa rozlozenie oproti zaciatku (ma sa ulozit)?
    pub fn changed(&self, layout: &Layout) -> bool {
        self.before != *layout
    }
}

/// Polozky skupiny v poradi konfiguracie (aj skryte; nezname id sa nezobrazuju, ale zostanu v subore).
pub fn column_items(layout: &Layout, g: Group) -> Vec<String> {
    layout
        .in_group(g)
        .filter(|s| transom_config::is_layout_id(&s.id))
        .map(|s| s.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Predvolene rozlozenie bez poloziek pridanych neskor do lavej skupiny (aktivne okno, plochy): testy pocitaju
    /// s povodnym poctom chipov v stlpci.
    fn base() -> Layout {
        let mut l = Layout::default();
        l.items
            .retain(|s| !matches!(s.id.as_str(), "window" | "desktops"));
        l
    }

    fn geom(l: &Layout) -> Vec<ColGeom> {
        [Group::Left, Group::Center, Group::Right]
            .iter()
            .enumerate()
            .map(|(i, g)| ColGeom {
                group: *g,
                x: i as i32 * 100,
                y: 0,
                w: 100,
                h: 300,
                chips: column_items(l, *g)
                    .into_iter()
                    .enumerate()
                    .map(|(j, id)| (id, 20 + j as i32 * 40))
                    .collect(),
            })
            .collect()
    }

    fn visible(l: &Layout, id: &str) -> bool {
        l.items.iter().any(|s| s.id == id && s.visible)
    }

    #[test]
    fn drop_into_center_and_reorder() {
        let mut l = base();
        let g = geom(&l);
        let t = drop_target(&g, "clock", 150, 10).unwrap();
        assert_eq!(t, (Group::Center, 0));
        apply_drop(&mut l, "clock", t);
        // v strede uz predvolene je Polica: hodiny idu pred nu
        assert_eq!(column_items(&l, Group::Center), vec!["clock", "shelf"]);
        // search na koniec lavej skupiny (pod media)
        let g = geom(&l);
        let t = drop_target(&g, "search", 50, 290).unwrap();
        apply_drop(&mut l, "search", t);
        assert_eq!(column_items(&l, Group::Left), vec!["media", "search"]);
    }

    #[test]
    fn moving_keeps_visibility_and_hidden_items_stay_in_their_group() {
        let mut l = base();
        l.set_visible("tools", false);
        // skryta polozka zostava vo svojej skupine (zaskrtavacie policko ju vrati na to iste miesto)
        assert!(column_items(&l, Group::Right).contains(&"tools".to_string()));
        let g = geom(&l);
        let t = drop_target(&g, "tools", 50, 1).unwrap();
        apply_drop(&mut l, "tools", t);
        assert_eq!(column_items(&l, Group::Left)[0], "tools");
        assert!(!visible(&l, "tools"), "presun skrytu polozku nezobrazi");
        // viditelna polozka po presune zostane viditelna
        apply_drop(&mut l, "clock", (Group::Center, 0));
        assert!(visible(&l, "clock"));
        // mimo stlpcov = ziadna zmena
        assert_eq!(drop_target(&g, "tools", 999, 999), None);
    }

    fn with_unknown(l: &mut Layout) {
        // neznama polozka z buducej verzie na zaciatku lavej skupiny
        l.items.insert(
            0,
            transom_config::Slot {
                id: "future_widget".into(),
                group: Group::Left,
                visible: true,
            },
        );
    }

    #[test]
    fn a_drop_keeps_the_order_of_known_items_when_an_unknown_one_is_in_the_group() {
        // regresia: index "medzi zobrazenymi" sa predtym bral ako index medzi vsetkymi polozkami skupiny
        let mut l = base();
        with_unknown(&mut l);
        let all_left = |l: &Layout| {
            l.in_group(Group::Left)
                .map(|s| s.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(all_left(&l), ["future_widget", "search", "media"]);
        // search na koniec zobrazenych (za media)
        apply_drop(&mut l, "search", (Group::Left, 1));
        assert_eq!(column_items(&l, Group::Left), ["media", "search"]);
        assert!(
            all_left(&l).contains(&"future_widget".to_string()),
            "neznama polozka zachovana"
        );
        // naspat na zaciatok
        apply_drop(&mut l, "search", (Group::Left, 0));
        assert_eq!(column_items(&l, Group::Left), ["search", "media"]);
    }

    #[test]
    fn keyboard_moves_within_a_column_between_columns_and_stops_at_the_edges() {
        let mut l = base();
        let left = column_items(&l, Group::Left);
        assert!(left.len() >= 2, "{left:?}");
        let (first, second) = (left[0].clone(), left[1].clone());
        // okraje: nahor z prveho a doprava z posledneho stlpca nic nerobi
        assert_eq!(move_target(&l, &first, Dir::Up), None);
        assert_eq!(move_target(&l, &first, Dir::Home), None);
        assert_eq!(move_target(&l, &first, Dir::Left), None, "uplne vlavo");
        assert_eq!(
            move_by_key(&mut l, &first, Dir::Down),
            Some((Group::Left, 1))
        );
        assert_eq!(
            column_items(&l, Group::Left)[..2],
            [second.clone(), first.clone()]
        );
        assert_eq!(
            move_by_key(&mut l, &first, Dir::Home),
            Some((Group::Left, 0))
        );
        let n = column_items(&l, Group::Left).len();
        assert_eq!(
            move_by_key(&mut l, &first, Dir::End),
            Some((Group::Left, n - 1))
        );
        // medzi stlpcami: poloha sa zachova (tu 1 - posledna v lavom stlpci), najviac na koniec stlpca; v strede je Polica
        assert_eq!(
            move_by_key(&mut l, &first, Dir::Right),
            Some((Group::Center, 1))
        );
        assert_eq!(position_of(&l, &first), Some((Group::Center, 1)));
        assert_eq!(
            move_by_key(&mut l, &first, Dir::Right),
            Some((Group::Right, 1))
        );
        assert_eq!(move_target(&l, &first, Dir::Right), None, "uplne vpravo");
        assert_eq!(
            move_by_key(&mut l, &first, Dir::Left),
            Some((Group::Center, 1))
        );
    }

    #[test]
    fn keyboard_moves_never_change_visibility_unknown_ids_or_other_items() {
        let mut l = base();
        with_unknown(&mut l);
        l.set_visible("media", false);
        let before: std::collections::HashMap<String, bool> =
            l.items.iter().map(|s| (s.id.clone(), s.visible)).collect();
        // skryta polozka sa hybe a ostane skryta; neznama polozka ostane v subore
        for d in [
            Dir::Down,
            Dir::Right,
            Dir::Home,
            Dir::Left,
            Dir::End,
            Dir::Up,
        ] {
            move_by_key(&mut l, "media", d);
        }
        let after: std::collections::HashMap<String, bool> =
            l.items.iter().map(|s| (s.id.clone(), s.visible)).collect();
        assert_eq!(before, after);
        assert_eq!(l.items.len(), before.len());
        // nezname id sa klavesnicou nehybe
        assert_eq!(move_target(&l, "future_widget", Dir::Down), None);
        assert_eq!(move_target(&l, "neexistuje", Dir::Up), None);
    }

    #[test]
    fn a_cancelled_move_restores_the_exact_layout_and_changed_reports_the_difference() {
        let mut l = base();
        let ses = MoveSession::begin(&l, "clock");
        assert!(!ses.changed(&l));
        move_by_key(&mut l, "clock", Dir::Left);
        move_by_key(&mut l, "clock", Dir::Home);
        assert!(ses.changed(&l));
        let original = base();
        ses.clone().cancel(&mut l);
        assert_eq!(l, original, "Esc vrati presne povodny stav");
        // presun tam a spat nie je zmena
        let ses = MoveSession::begin(&l, "clock");
        move_by_key(&mut l, "clock", Dir::Left);
        move_by_key(&mut l, "clock", Dir::Right);
        move_by_key(&mut l, "clock", Dir::End);
        assert_eq!(ses.changed(&l), l != original);
    }

    #[test]
    fn unknown_ids_are_not_editable_but_kept() {
        let mut l = base();
        l.items.push(transom_config::Slot {
            id: "future_widget".into(),
            group: Group::Left,
            visible: true,
        });
        assert!(!column_items(&l, Group::Left).contains(&"future_widget".to_string()));
        apply_drop(&mut l, "clock", (Group::Left, 0));
        assert!(
            l.items.iter().any(|s| s.id == "future_widget"),
            "neznama polozka zachovana"
        );
    }
}
