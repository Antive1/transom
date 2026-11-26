//! Stranka "Doplnky": nastavenia novych poloziek lisaty (Casovac, Vytazenie systemu, Mikrofon a kamera) a pripojene
//! kalendare (ICS). Odkaz kalendara je tajny udaj: na stranke sa ukazuje len pocitac (napr. calendar.google.com) a
//! pridava sa zo schranky (Nastavenia nemaju textove pole), nikdy sa nevypisuje ani neloguje cely.
use super::*;

pub(crate) const WORK: [u32; 7] = [15, 20, 25, 30, 45, 50, 60];
pub(crate) const BREAKS: [u32; 4] = [3, 5, 10, 15];
pub(crate) const LONG: [u32; 4] = [10, 15, 20, 30];
pub(crate) const ROUNDS: [u32; 5] = [2, 3, 4, 5, 6];
pub(crate) const SYS: [(&str, &str); 4] = [
    ("cpu", "ex.sys_cpu"),
    ("ram", "ex.sys_ram"),
    ("gpu", "ex.sys_gpu"),
    ("net", "ex.sys_net"),
];
pub(crate) const SYS_INTERVALS: [u32; 4] = [1, 2, 5, 10];
/// Ponukane skratky stlmenia mikrofonu ("off" = vypnuta).
pub(crate) const MIC_KEYS: [&str; 4] = ["Ctrl+Alt+M", "Ctrl+Shift+M", "Ctrl+Alt+F12", "off"];

pub(crate) fn sys_get(c: &transom_config::SysMonCfg, id: &str) -> bool {
    match id {
        "cpu" => c.cpu,
        "ram" => c.ram,
        "gpu" => c.gpu,
        "net" => c.net,
        _ => false,
    }
}

pub(crate) fn sys_toggle(c: &mut transom_config::SysMonCfg, id: &str) {
    let v = !sys_get(c, id);
    match id {
        "cpu" => c.cpu = v,
        "ram" => c.ram = v,
        "gpu" => c.gpu = v,
        "net" => c.net = v,
        _ => {}
    }
}

impl Ui {
    pub(crate) fn paint_extras_page(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let lw = px(crate::CHOICE_LABEL_LP);
        let mut y = self.title(p, x0, y, w, "ex.title");
        y = self.notes(p, x0, y, w, &["ex.help"]);
        y += px(12);

        // casovac
        let tc = self.cfg.timer.clone();
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("item.timer"));
        let mins = |v: u32| tf("timer.min", &[&v.to_string()]);
        let opts = |list: &[u32], cur: u32, f: fn(u32) -> Hit| -> Vec<Choice> {
            list.iter().map(|&v| (mins(v), f(v), cur == v)).collect()
        };
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("ex.work"),
            opts(&WORK, tc.work_min, Hit::ExWork),
        );
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("ex.break"),
            opts(&BREAKS, tc.short_break_min, Hit::ExBreak),
        );
        y += self.choice_row(
            p,
            Rect::new(x0, y, w, 0),
            lw,
            &t("ex.long"),
            opts(&LONG, tc.long_break_min, Hit::ExLong),
        );
        let rounds: Vec<Choice> = ROUNDS
            .iter()
            .map(|&v| (v.to_string(), Hit::ExRounds(v), tc.rounds == v))
            .collect();
        y += self.choice_row(p, Rect::new(x0, y, w, 0), lw, &t("ex.rounds"), rounds);
        y = self.check_row(p, x0, y, w, Hit::ExSound, "timer.sound", tc.sound, false);
        y = self.card_end(s, x0, w, y);

        // vytazenie systemu
        let sc = self.cfg.sysmon.clone();
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("item.sysmon"));
        for (id, key) in SYS {
            y = self.check_row(p, x0, y, w, Hit::ExSys(id), key, sys_get(&sc, id), false);
        }
        let iv: Vec<Choice> = SYS_INTERVALS
            .iter()
            .map(|&v| {
                (
                    tf("pp.secs", &[&v.to_string()]),
                    Hit::ExSysInterval(v),
                    sc.interval_secs == v,
                )
            })
            .collect();
        y += self.choice_row(p, Rect::new(x0, y, w, 0), lw, &t("pp.interval"), iv);
        y = self.notes(p, x0, y, w, &["ex.sys_note"]);
        y = self.card_end(s, x0, w, y);

        // mikrofon a kamera
        let pc = self.cfg.privacy.clone();
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("item.privacy"));
        let cur = pc
            .hotkey()
            .map(|h| h.label())
            .unwrap_or_else(|| "off".into());
        let keys: Vec<Choice> = MIC_KEYS
            .iter()
            .map(|&k| {
                (
                    if k == "off" {
                        t("ex.key_off")
                    } else {
                        k.to_string()
                    },
                    Hit::ExMicKey(k),
                    cur == k,
                )
            })
            .collect();
        y += self.choice_row(p, Rect::new(x0, y, w, 0), lw, &t("ex.mic_key"), keys);
        y = self.check_row(
            p,
            x0,
            y,
            w,
            Hit::ExMicShow,
            "ex.mic_show",
            pc.always_show,
            false,
        );
        y = self.notes(p, x0, y, w, &["ex.mic_note"]);
        y = self.card_end(s, x0, w, y);

        // kalendare ICS
        y = self.card_begin(s, y);
        y = self.section(p, x0, y, w, &t("ex.ics_h"));
        y = self.notes(
            p,
            x0,
            y,
            w,
            &["ex.ics_help", "ex.ics_google", "ex.ics_outlook"],
        );
        y += px(4);
        let urls = self.cfg.ics_calendars.clone();
        if urls.is_empty() {
            y = self.notes(p, x0, y, w, &["ex.ics_none"]);
        }
        let mut items: Vec<(String, Hit, bool)> = Vec::new();
        for (i, u) in urls.iter().enumerate() {
            let host = transom_platform::http::host_of(u);
            items.push((tf("ex.ics_remove", &[&host]), Hit::IcsRemove(i), false));
        }
        if !items.is_empty() {
            y = self.flow_buttons(p, x0, y, w, items);
        }
        y = self.flow_buttons(p, x0, y, w, vec![(t("ex.ics_add"), Hit::IcsAdd, false)]);
        if let Some(m) = self.extras_msg.clone() {
            let col = self.theme.foreground;
            y = self.paragraph(p, x0, y, w, &m, 12.0, col);
        }
        y = self.notes(p, x0, y, w, &["ex.ics_privacy"]);
        y = self.card_end(s, x0, w, y);

        self.paint_weather_card(p, x0, y, w);
    }

    /// Prida kalendar zo schranky (len platny https / webcal odkaz, najviac `MAX_ICS`).
    pub(crate) fn ics_add_from_clipboard(&mut self) -> bool {
        let text = transom_platform::clipboard::read_text().unwrap_or_default();
        let Some(url) = transom_config::normalize_ics_url(&text) else {
            self.extras_msg = Some(t("ex.ics_bad"));
            return false;
        };
        if self.cfg.ics_calendars.contains(&url) {
            self.extras_msg = Some(t("ex.ics_dup"));
            return false;
        }
        if self.cfg.ics_calendars.len() >= transom_config::MAX_ICS {
            self.extras_msg = Some(t("ex.ics_full"));
            return false;
        }
        let host = transom_platform::http::host_of(&url);
        self.cfg.ics_calendars.push(url);
        self.extras_msg = Some(tf("ex.ics_added", &[&host]));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_tests::Env;
    use transom_config::load;

    #[test]
    fn timer_system_monitor_and_microphone_settings_persist() {
        let e = Env::new("extras-page");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Extras));
        ui.click(Hit::ExWork(50));
        ui.click(Hit::ExBreak(10));
        ui.click(Hit::ExLong(30));
        ui.click(Hit::ExRounds(2));
        ui.click(Hit::ExSound);
        ui.click(Hit::ExSys("gpu"));
        ui.click(Hit::ExSys("cpu"));
        ui.click(Hit::ExSysInterval(5));
        ui.click(Hit::ExMicKey("off"));
        ui.click(Hit::ExMicShow);
        let c = load(&e.root.join("config.json")).config;
        assert_eq!(
            (
                c.timer.work_min,
                c.timer.short_break_min,
                c.timer.long_break_min,
                c.timer.rounds,
                c.timer.sound
            ),
            (50, 10, 30, 2, false)
        );
        assert_eq!(
            (
                c.sysmon.cpu,
                c.sysmon.ram,
                c.sysmon.gpu,
                c.sysmon.net,
                c.sysmon.interval_secs
            ),
            (false, true, true, false, 5)
        );
        assert!(c.privacy.hotkey().is_none() && !c.privacy.always_show);
        ui.click(Hit::ExMicKey("Ctrl+Shift+M"));
        let c = load(&e.root.join("config.json")).config;
        assert_eq!(
            c.privacy.hotkey().map(|h| h.label()).as_deref(),
            Some("Ctrl+Shift+M")
        );
    }

    #[test]
    fn calendars_are_added_from_the_clipboard_shown_by_host_and_removed() {
        let e = Env::new("extras-ics");
        let mut ui = e.ui();
        ui.page = Page::Extras;
        ui.cfg.ics_calendars = vec![
            "https://calendar.google.com/calendar/ical/x/private-abc/basic.ics".into(),
            "https://outlook.office365.com/owa/calendar/y/reachcalendar.ics".into(),
        ];
        ui.save();
        crate::a11y_tests::paint(&mut ui, 96);
        let (entries, _, targets, _) = ui.a11y_snapshot(&[], None);
        let i = targets
            .iter()
            .position(|t| *t == crate::page_a11y::Target::Hit(Hit::IcsRemove(1)))
            .unwrap();
        assert_eq!(entries[i].name, "Remove outlook.office365.com");
        // tajna cast odkazu sa nikde nezobrazuje
        assert!(entries.iter().all(|e| !e.name.contains("private-abc")));
        ui.click(Hit::IcsRemove(0));
        let c = load(&e.root.join("config.json")).config;
        assert_eq!(c.ics_calendars.len(), 1);
        assert!(c.ics_calendars[0].contains("outlook"));
    }
}
