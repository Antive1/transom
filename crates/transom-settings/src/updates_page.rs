//! Stranka "Aktualizacie": pravidelna kontrola novych vydani (zapnut/vypnut), ikona dostupnej aktualizacie na liste,
//! aktualna verzia a kontrola hned (poziada listu; vysledok oznami lista, nastavenia samy nic nestahuju).
use super::*;

impl Ui {
    pub(crate) fn paint_updates(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        self.acc_statics.clear();
        let mut y = self.title(p, x0, y, w, "upd.page_title");
        y = self.paragraph(p, x0, y, w, &t("upd.intro"), 12.0, th.muted) + px(12);
        y = self.card_begin(s, y);
        let on = self.cfg.updates.auto_check;
        let lw = px(crate::CHOICE_LABEL_LP);
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("upd.auto"),
            vec![
                (t("opt.yes"), Hit::UpdAuto(true), on),
                (t("opt.no"), Hit::UpdAuto(false), !on),
            ],
        );
        let icon = self.cfg.updates.show_icon;
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("upd.show_icon"),
            vec![
                (t("opt.yes"), Hit::UpdIcon(true), icon),
                (t("opt.no"), Hit::UpdIcon(false), !icon),
            ],
        );
        y = self.paragraph(
            p,
            x0,
            y + px(4),
            w,
            &tf("upd.version", &[env!("CARGO_PKG_VERSION")]),
            12.0,
            th.muted,
        ) + px(10);
        y = self.flow_buttons(
            p,
            x0,
            y,
            w,
            vec![(t("upd.check_now"), Hit::UpdCheck, false)],
        );
        y = self.paragraph(p, x0, y, w, &t("upd.privacy"), 12.0, th.muted) + px(4);
        self.card_end(s, x0, w, y);
    }
}
