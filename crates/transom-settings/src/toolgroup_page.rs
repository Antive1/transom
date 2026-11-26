//! Stranka "Skupina v Nastrojoch": ktore dlazdice ukaze panel Nastroje (`Config::tool_group`). Polozky mozu byt aj
//! samostatne v liste - to sa nastavuje v Rozlozeni.
use super::*;

/// Je dlazdica v skupine?
pub(crate) fn get(c: &transom_config::Config, id: &str) -> bool {
    c.tool_group.iter().any(|g| g == id)
}

/// Zapne / vypne dlazdicu; poradie ostava podla `TOOL_TILES`.
pub(crate) fn toggle(c: &mut transom_config::Config, id: &str) {
    let on = !get(c, id);
    let mut set: Vec<String> = c.tool_group.clone();
    set.retain(|g| g != id);
    if on {
        set.push(id.to_string());
    }
    c.tool_group = transom_config::TOOL_TILES
        .iter()
        .filter(|t| set.iter().any(|g| g == *t))
        .map(|t| t.to_string())
        .collect();
}

impl Ui {
    pub(crate) fn paint_toolgroup_page(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = self.title(p, x0, y, w, "tls.group");
        y = self.notes(p, x0, y, w, &["tls.group_note"]) + px(12);
        y = self.card_begin(s, y);
        for id in transom_config::TOOL_TILES {
            let key = format!("tile.{id}");
            let on = get(&self.cfg, id);
            let row = Rect::new(x0, y, w, px(28));
            let hit = Hit::ToolTile(id);
            if self.hover.as_ref() == Some(&hit) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            self.checkbox(
                p,
                Rect::new(row.x + px(4), row.y + px(4), px(20), px(20)),
                on,
            );
            p.draw_text(
                &t(&key),
                Rect::new(row.x + px(34), row.y, row.w - px(34), row.h),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            self.tag(&hit, t(&key), Some(on));
            self.hits.push((row, hit));
            y += px(32);
        }
        self.card_end(s, x0, w, y - px(4));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_tests::Env;

    #[test]
    fn every_tile_toggles_is_saved_and_keeps_the_order() {
        let e = Env::new("tool-group");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::ToolGroup));
        assert_eq!(e.saved().tool_group.len(), transom_config::TOOL_TILES.len());
        ui.click(Hit::ToolTile("notes"));
        assert!(!get(&e.saved(), "notes"));
        ui.click(Hit::ToolTile("notes"));
        let g = e.saved().tool_group;
        assert_eq!(
            g,
            transom_config::TOOL_TILES.map(String::from).to_vec(),
            "poradie ostava"
        );
    }
}
