//! Stranka "Nastroje": zapnutie / vypnutie Police, historie schranky, poznamok a snimok, obmedzenia a vymazanie ulozenych dat.
//!
//! Sukromie: nic sa nezbiera samo; historia schranky je predvolene **vypnuta** a zapina sa tu. Stranka vysvetli, co sa uklada, kde
//! a ze to nie je sifrovane. Vymazanie je dvojkrokove a vykona ho bezici transom na zaklade znacky v priecinku nastrojov (zdielane
//! ulozisko ma jedneho pisara). Nastroje sa nikdy nevyexportuju (viz `transom-config::transfer`).
use super::*;
use transom_config::Tools;

/// Volby so zaskrtavacim polickom: (id, kluc textu).
pub(crate) const TOGGLES: [(&str, &str); 10] = [
    ("shelf.enabled", "tls.shelf_on"),
    ("shelf.copy", "tls.shelf_copy"),
    ("clip.enabled", "tls.clip_on"),
    ("clip.paused", "tls.clip_pause"),
    ("clip.text", "tls.clip_text"),
    ("clip.images", "tls.clip_images"),
    ("clip.files", "tls.clip_files"),
    ("notes.enabled", "tls.notes_on"),
    ("shots.enabled", "tls.shots_on"),
    ("shots.default", "tls.shots_default"),
];

/// Vymazatelne ulozista: (id v priecinku, kluc nazvu sekcie). Snimky tu nie su: kniznica snimok iba cita priecinky pouzivatela,
/// takze nema co vymazat (a tlacidlo "Vymazat snimky" by sa dalo mylne chapat ako mazanie jeho obrazkov).
pub(crate) const CLEARS: [(&str, &str); 3] = [
    ("shelf", "tools.sec_shelf"),
    ("clipboard", "tools.sec_clipboard"),
    ("notes", "tools.sec_notes"),
];

/// Volby vyhladavaca: (id, kluc textu).
pub(crate) const LAUNCHER_TOGGLES: [(&str, &str); 8] = [
    ("lau.enabled", "tls.lau_on"),
    ("lau.apps", "tls.lau_apps"),
    ("lau.files", "tls.lau_files"),
    ("lau.settings", "tls.lau_settings"),
    ("lau.actions", "tls.lau_actions"),
    ("lau.notes", "tls.lau_notes"),
    ("lau.shelf", "tls.lau_shelf"),
    ("lau.recent", "tls.lau_recent"),
];

const MAX_RESULTS: [u32; 4] = [5, 8, 12, 20];
/// Najviac priecinkov v kazdom zozname (obrana proti obrovskemu konfiguracnemu suboru).
pub(crate) const MAX_FOLDERS: usize = 8;
/// Najviac aplikacii, ktore UI pridava do zoznamu vylucenych (konfiguracia znesie viac, viz `TOOLS_MAX_EXCLUDED_APPS`).
pub(crate) const MAX_APPS: usize = 20;

const KEEP_DAYS: [u32; 6] = [1, 3, 7, 14, 30, 0];
const MAX_ITEMS: [u32; 4] = [50, 100, 200, 500];

fn px_of(p: &dyn Painter, v: i32) -> i32 {
    (v as f32 * p.scale()).round() as i32
}

pub(crate) fn get(t: &Tools, id: &str) -> bool {
    match id {
        "shelf.enabled" => t.shelf.enabled,
        "shelf.copy" => t.shelf.copy_by_default,
        "clip.enabled" => t.clipboard.enabled,
        "clip.paused" => t.clipboard.paused,
        "clip.text" => t.clipboard.text,
        "clip.images" => t.clipboard.images,
        "clip.files" => t.clipboard.files,
        "notes.enabled" => t.notes.enabled,
        "shots.enabled" => t.screenshots.enabled,
        "shots.default" => t.screenshots.default_folder,
        "lau.enabled" => t.launcher.enabled,
        "lau.apps" => t.launcher.apps,
        "lau.files" => t.launcher.files,
        "lau.settings" => t.launcher.settings,
        "lau.actions" => t.launcher.actions,
        "lau.notes" => t.launcher.notes,
        "lau.shelf" => t.launcher.shelf,
        "lau.recent" => t.launcher.recent_searches,
        "lau.hotkey_fs" => t.launcher.hotkey_in_fullscreen,
        _ => false,
    }
}

pub(crate) fn set(t: &mut Tools, id: &str, v: bool) {
    match id {
        "shelf.enabled" => t.shelf.enabled = v,
        "shelf.copy" => t.shelf.copy_by_default = v,
        "clip.enabled" => t.clipboard.enabled = v,
        "clip.paused" => t.clipboard.paused = v,
        "clip.text" => t.clipboard.text = v,
        "clip.images" => t.clipboard.images = v,
        "clip.files" => t.clipboard.files = v,
        "notes.enabled" => t.notes.enabled = v,
        "shots.enabled" => t.screenshots.enabled = v,
        "shots.default" => t.screenshots.default_folder = v,
        "lau.enabled" => t.launcher.enabled = v,
        "lau.apps" => t.launcher.apps = v,
        "lau.files" => t.launcher.files = v,
        "lau.settings" => t.launcher.settings = v,
        "lau.actions" => t.launcher.actions = v,
        "lau.notes" => t.launcher.notes = v,
        "lau.shelf" => t.launcher.shelf = v,
        "lau.recent" => t.launcher.recent_searches = v,
        "lau.hotkey_fs" => t.launcher.hotkey_in_fullscreen = v,
        _ => {}
    }
}

/// Zoznam priecinkov podla druhu: "shots" = priecinky kniznice snimok, inak zoznamy vyhladavaca ("excluded", "fallback").
pub(crate) fn folders_mut<'a>(t: &'a mut Tools, kind: &str) -> &'a mut Vec<String> {
    if kind == "shots" {
        &mut t.screenshots.folders
    } else {
        t.launcher.folders_mut(kind)
    }
}

/// Priecinok, ktory kniznica snimok prijme: absolutna cesta (`C:\...` alebo `\\server\zdielanie\...`), nie zariadenie ani
/// koren disku (kniznica cita len jeden priecinok bez podpriecinkov, nikdy cely disk).
pub(crate) fn shots_folder_ok(p: &str) -> bool {
    let p = p.trim();
    if p.is_empty()
        || p.chars().count() > transom_config::TOOLS_MAX_FOLDER_CHARS
        || p.chars().any(|c| c.is_control())
    {
        return false;
    }
    let n = p.replace('/', "\\");
    let l = n.to_ascii_lowercase();
    if l.starts_with("\\\\.\\") || l.starts_with("\\\\?\\") || l.starts_with("\\??\\") {
        return false;
    }
    let b = n.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
    if drive {
        // koren disku (`C:\`) nie
        return !n[3..].trim_matches('\\').is_empty();
    }
    if let Some(rest) = n.strip_prefix("\\\\") {
        let mut parts = rest.split('\\');
        let (host, share) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        return !host.is_empty()
            && !share.is_empty()
            && !matches!(share.to_ascii_lowercase().as_str(), "pipe" | "mailslot");
    }
    false
}

/// Dalsia hodnota v kruhu (nezname hodnoty prejdu na prvu).
fn next(list: &[u32], cur: u32) -> u32 {
    let i = list
        .iter()
        .position(|v| *v == cur)
        .map_or(0, |i| (i + 1) % list.len());
    list[i]
}

pub(crate) fn cycle(t: &mut Tools, id: &str) {
    match id {
        "clip.keep" => t.clipboard.retention_days = next(&KEEP_DAYS, t.clipboard.retention_days),
        "clip.max" => t.clipboard.max_items = next(&MAX_ITEMS, t.clipboard.max_items),
        "lau.max" => t.launcher.max_results = next(&MAX_RESULTS, t.launcher.max_results),
        _ => {}
    }
}

impl Ui {
    /// Riadok so zaskrtavacim polickom (cely riadok je cielom kliknutia); `indent` = zavisla volba (odsadena pod hlavnou).
    /// Dlhy popisok sa zalomi. Vrati y pod riadkom.
    #[allow(clippy::too_many_arguments)]
    fn tools_toggle(
        &mut self,
        p: &mut dyn Painter,
        x0: i32,
        y: i32,
        w: i32,
        id: &'static str,
        key: &str,
        indent: bool,
    ) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let dx = if indent { px(32) } else { 0 };
        let on = get(&self.cfg.tools, id);
        let st = Ty::Body.style(th.foreground, HAlign::Left);
        let lh = px(Ty::Body.line_lp());
        let lines = Self::wrap_lines(p, &t(key), &st, w - dx - px(40));
        let h = (lines.len() as i32 * lh + px(8)).max(px(28));
        let row = Rect::new(x0 + dx, y, w - dx, h);
        let hit = Hit::ToolsToggle(id);
        if self.hover.as_ref() == Some(&hit) {
            p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
        }
        self.checkbox(
            p,
            Rect::new(row.x + px(4), row.y + px(4), px(20), px(20)),
            on,
        );
        for (k, l) in lines.iter().enumerate() {
            p.draw_text(
                l,
                Rect::new(
                    row.x + px(34),
                    row.y + px(4) + k as i32 * lh,
                    row.w - px(38),
                    lh,
                ),
                &st,
            );
        }
        self.tag(&hit, t(key), Some(on));
        self.hits.push((row, hit));
        y + h + px(4)
    }

    /// Podstranka nastrojov (Polica, Schranka, Poznamky, Snimky, Vyhladavac): len volby a mazanie daneho nastroja.
    pub(crate) fn paint_tools_page(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let (sec, title) = match self.page {
            Page::Clipboard => ("clip", "tools.sec_clipboard"),
            Page::Notes => ("notes", "tools.sec_notes"),
            Page::Shots => ("shots", "tools.sec_shots"),
            Page::Launcher => ("lau", "lau.title"),
            _ => ("shelf", "tools.sec_shelf"),
        };
        let mut y = self.title(p, x0, y, w, title);
        if sec == "lau" {
            self.paint_launcher_settings(p, x0, y, w);
            return;
        }
        y = self.notes(p, x0, y, w, &["tls.help1", "tls.help2"]) + px(12);
        // poradie ovladacov (a kruhu fokusu) je rovnake ako v `TOGGLES`; volby nastroja v jednej karte
        y = self.card_begin(s, y);
        if sec == "clip" {
            y = self.notes(p, x0, y, w, &["tls.clip_note"]) + px(8);
        }
        for (id, key) in TOGGLES {
            if id.split('.').next() != Some(sec) {
                continue;
            }
            // zavisle volby su odsadene pod hlavnym prepinacom nastroja
            let indent = !id.ends_with(".enabled");
            y = self.tools_toggle(p, x0, y, w, id, key, indent);
        }
        y = self.tools_section_end(p, x0, y, w, sec);
        // vymazanie ulozenych dat nastroja (snimky nemaju co mazat): nebezpecne tlacidlo, druhy krok = potvrdenie
        let clear_id = match sec {
            "shelf" => Some("shelf"),
            "clip" => Some("clipboard"),
            "notes" => Some("notes"),
            _ => None,
        };
        if let Some(&(id, key)) = clear_id.and_then(|c| CLEARS.iter().find(|(i, _)| *i == c)) {
            y = self.card_begin(s, y);
            let armed = self.tools_confirm == Some(id);
            let name = t(key);
            let label = if armed {
                tf("tls.clear_confirm", &[&name])
            } else {
                tf("tls.clear", &[&name])
            };
            y = self.flow_buttons(p, x0, y, w, vec![(label, Hit::ToolsClear(id), armed)]);
            y = self.notes(p, x0, y, w, &["tls.clear_note"]);
            self.card_end(s, x0, w, y);
        }
    }

    /// Koniec karty nastroja: doplnky sekcie (obmedzenia a vylucene aplikacie schranky, priecinky snimok) a uzavretie.
    fn tools_section_end(
        &mut self,
        p: &mut dyn Painter,
        x0: i32,
        y: i32,
        w: i32,
        sec: &str,
    ) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let mut y = y;
        if sec == "clip" {
            // obmedzenia historie: dva cyklicke tlacidla (sirka podla textu)
            let days = self.cfg.tools.clipboard.retention_days;
            let keep = if days == 0 {
                t("tls.forever")
            } else {
                tf("tls.days", &[&days.to_string()])
            };
            let l1 = format!("{}: {keep}", t("tls.clip_keep"));
            let l2 = tf(
                "tls.clip_max",
                &[&self.cfg.tools.clipboard.max_items.to_string()],
            );
            y = self.flow_buttons(
                p,
                x0 + px(32),
                y + px(4),
                w - px(32),
                vec![
                    (l1, Hit::ToolsCycle("clip.keep"), false),
                    (l2, Hit::ToolsCycle("clip.max"), false),
                ],
            );
            y = self.paint_excluded_apps(p, x0, y + px(4), w);
        }
        if sec == "shots" {
            y = self.paint_shot_folders(p, x0, y, w);
        }
        self.card_end(s, x0, w, y - px(4))
    }

    /// Zoznam (priecinky, aplikacie): podnadpis, poznamka, riadky "Odstranit ..." (text vlavo) a "Pridat ..." (sirka podla
    /// textu). Tlacidla nesu nazov zoznamu (inak by "Pridat" a "Odstranit X" splyvali s ostatnymi zoznamami).
    #[allow(clippy::too_many_arguments)]
    fn tools_list(
        &mut self,
        p: &mut dyn Painter,
        x0: i32,
        y: i32,
        w: i32,
        head: &str,
        note: Option<&str>,
        rows: Vec<(String, Hit)>,
        add: Option<(String, Hit)>,
    ) -> i32 {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        let ch = px(design::CONTROL_H_LP);
        let mut y = self.subsection(p, x0, y + px(6), w, &t(head));
        if let Some(n) = note {
            y = self.notes(p, x0, y, w, &[n]) + px(6);
        }
        self.acc_group = Some(t(head));
        if rows.is_empty() {
            p.draw_text(
                &t("tls.lau_none"),
                Rect::new(x0, y, w, px(22)),
                &Ty::Secondary.style(th.muted, HAlign::Left),
            );
            y += px(28);
        }
        self.btn_left = true;
        for (name, hit) in rows {
            self.button(p, Rect::new(x0, y, w, ch), &name, hit, false);
            y += ch + px(6);
        }
        self.btn_left = false;
        if let Some((label, hit)) = add {
            let bw = design::button_width(
                p,
                &design::Button::new(&label, Kind::Secondary).icon('\u{E710}'),
            );
            self.button(
                p,
                Rect::new(x0, y, bw.max(px(140)).min(w), ch),
                &label,
                hit,
                false,
            );
            y += ch + px(8);
        }
        self.acc_group = None;
        y
    }

    /// Priecinky kniznice snimok (okrem predvoleneho priecinka Windows): pridavaju sa vyberom priecinka, odstranuju tlacidlom.
    /// Kniznica z nich iba cita obrazky (bez podpriecinkov) a nic v nich nemeni.
    fn paint_shot_folders(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        let list = self.cfg.tools.screenshots.folders.clone();
        let rows = list
            .iter()
            .enumerate()
            .map(|(i, f)| {
                (
                    tf("tls.lau_remove", &[f]),
                    Hit::ToolsRemoveFolder("shots", i),
                )
            })
            .collect();
        let add =
            (list.len() < MAX_FOLDERS).then(|| (t("tls.lau_add"), Hit::ToolsAddFolder("shots")));
        self.tools_list(
            p,
            x0,
            y,
            w,
            "tls.shots_folders_h",
            Some("tls.shots_note"),
            rows,
            add,
        )
    }

    /// Aplikacie, z ktorych sa schranka nikdy nezachytava (podla nazvu .exe; best-effort). Zoznam sa upravuje len tu.
    fn paint_excluded_apps(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        let list = self.cfg.tools.clipboard.excluded_apps.clone();
        let rows = list
            .iter()
            .enumerate()
            .map(|(i, a)| (tf("tls.lau_remove", &[a]), Hit::ToolsRemoveApp(i)))
            .collect();
        let add = (list.len() < MAX_APPS).then(|| (t("tls.apps_add"), Hit::ToolsAddApp));
        let mut y = self.tools_list(p, x0, y, w, "tls.apps_h", Some("tls.apps_note"), rows, add);
        if let Some(k) = self.apps_msg {
            y = self.notes(p, x0, y, w, &[k]) + px_of(p, 4);
        }
        y
    }

    fn paint_launcher_settings(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        // nazov je uz v hlavicke podstranky
        let mut y = self.card_begin(s, y);
        y = self.notes(p, x0, y, w, &["tls.lau_note"]) + px(8);
        for (id, key) in LAUNCHER_TOGGLES {
            let indent = id != "lau.enabled";
            y = self.tools_toggle(p, x0, y, w, id, key, indent);
        }
        let label = tf(
            "tls.lau_max",
            &[&self.cfg.tools.launcher.max_results.to_string()],
        );
        y = self.flow_buttons(
            p,
            x0 + px(32),
            y + px(4),
            w - px(32),
            vec![(label, Hit::ToolsCycle("lau.max"), false)],
        );
        y = self.paint_hotkey(p, x0 + px(32), y, w - px(32));
        for (kind, head) in [("excluded", "tls.lau_excl_h"), ("fallback", "tls.lau_fb_h")] {
            let list = self.cfg.tools.launcher.folders(kind).to_vec();
            let rows = list
                .iter()
                .enumerate()
                .map(|(i, f)| (tf("tls.lau_remove", &[f]), Hit::ToolsRemoveFolder(kind, i)))
                .collect();
            let add =
                (list.len() < MAX_FOLDERS).then(|| (t("tls.lau_add"), Hit::ToolsAddFolder(kind)));
            y = self.tools_list(p, x0, y, w, head, None, rows, add);
        }
        self.card_end(s, x0, w, y - px(8))
    }

    /// Skratka vyhladavaca: tlacidlo so sucasnou skratkou (klik = zachytit novu), vypnut / obnovit predvolenu, vysvetlenie.
    fn paint_hotkey(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) -> i32 {
        let cur = self.cfg.tools.launcher.hotkey.clone();
        let shown = if cur.is_empty() {
            t("tls.lau_hotkey_none")
        } else {
            cur.clone()
        };
        let main = if self.hotkey_capture {
            t("tls.lau_hotkey_capture")
        } else {
            tf("tls.lau_hotkey", &[&shown])
        };
        let mut items = vec![(main, Hit::ToolsHotkey, self.hotkey_capture)];
        if !cur.is_empty() {
            items.push((t("tls.lau_hotkey_off"), Hit::ToolsHotkeyOff, false));
        }
        if cur != transom_config::DEFAULT_LAUNCHER_HOTKEY {
            items.push((
                tf(
                    "tls.lau_hotkey_default",
                    &[transom_config::DEFAULT_LAUNCHER_HOTKEY],
                ),
                Hit::ToolsHotkeyDefault,
                false,
            ));
        }
        let gap = px_of(p, 4);
        let y = self.flow_buttons(p, x0, y + gap, w, items);
        let y = self.tools_toggle(
            p,
            x0,
            y + gap,
            w,
            "lau.hotkey_fs",
            "tls.lau_hotkey_fs",
            false,
        );
        let mut keys = vec!["tls.lau_hotkey_note"];
        if let Some(k) = self.hotkey_msg {
            keys.push(k);
        }
        self.notes(p, x0, y, w, &keys) + px_of(p, 4)
    }

    /// Klavesa pocas zachytavania skratky. Vrati `true`, ak sa ulozila nova skratka (volajuci ulozi konfiguraciu).
    /// Samotne modifikatory sa cakaju, Esc bez modifikatorov zrusi; neplatna kombinacia ostane v zachytavani s vysvetlenim.
    pub(crate) fn hotkey_capture_key(
        &mut self,
        vk: u32,
        ctrl: bool,
        alt: bool,
        shift: bool,
        win: bool,
    ) -> bool {
        if !self.hotkey_capture || matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5) {
            return false;
        }
        if vk == 0x1B && !(ctrl || alt || shift || win) {
            self.hotkey_capture = false;
            self.hotkey_msg = None;
            return false;
        }
        match transom_config::Hotkey::new(ctrl, alt, shift, win, vk) {
            Some(h) => {
                self.cfg.tools.launcher.hotkey = h.label();
                self.hotkey_capture = false;
                self.hotkey_msg = None;
                true
            }
            None => {
                self.hotkey_msg = Some("tls.lau_hotkey_bad");
                false
            }
        }
    }

    /// Klik na volbu / cyklus / vymazanie. Vrati, ci sa zmenila konfiguracia (vymazanie konfiguraciu nemeni).
    pub(crate) fn tools_click(&mut self, hit: &Hit) -> bool {
        match hit {
            Hit::ToolsToggle(id) => {
                self.tools_confirm = None;
                let v = get(&self.cfg.tools, id);
                set(&mut self.cfg.tools, id, !v);
                true
            }
            Hit::ToolsCycle(id) => {
                self.tools_confirm = None;
                cycle(&mut self.cfg.tools, id);
                true
            }
            Hit::ToolsAddFolder(kind) => {
                self.tools_confirm = None;
                self.pending_folder = Some(kind);
                false
            }
            Hit::ToolsRemoveFolder(kind, i) => {
                self.tools_confirm = None;
                let list = folders_mut(&mut self.cfg.tools, kind);
                if *i < list.len() {
                    list.remove(*i);
                }
                true
            }
            Hit::ToolsHotkey => {
                self.tools_confirm = None;
                self.hotkey_capture = !self.hotkey_capture;
                self.hotkey_msg = None;
                false
            }
            Hit::ToolsHotkeyOff | Hit::ToolsHotkeyDefault => {
                self.tools_confirm = None;
                self.hotkey_msg = None;
                self.cfg.tools.launcher.hotkey = if matches!(hit, Hit::ToolsHotkeyOff) {
                    String::new()
                } else {
                    transom_config::DEFAULT_LAUNCHER_HOTKEY.into()
                };
                true
            }
            Hit::ToolsAddApp => {
                self.tools_confirm = None;
                self.apps_msg = None;
                self.pending_app = true;
                false
            }
            Hit::ToolsRemoveApp(i) => {
                self.tools_confirm = None;
                self.apps_msg = None;
                let list = &mut self.cfg.tools.clipboard.excluded_apps;
                if *i < list.len() {
                    list.remove(*i);
                }
                true
            }
            Hit::ToolsClear(id) => {
                if self.tools_confirm == Some(*id) {
                    self.tools_confirm = None;
                    // zapis znacky do priecinka nastrojov; chyba (napr. zamknuty disk) sa len zaznamena, nic sa nemaze priamo
                    let _ = transom_tools::store::request_clear(&transom_tools::tools_dir(), id);
                    notify_message(transom_config::CHANGED_MESSAGE);
                } else {
                    self.tools_confirm = Some(id);
                }
                false
            }
            _ => false,
        }
    }
}

/// Zdokumentovany vyber priecinka (`SHBrowseForFolderW`); vracia cestu alebo `None` pri zruseni.
fn pick_folder(owner: HWND) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS,
        BROWSEINFOW,
    };
    let mut name = [0u16; 260];
    let bi = BROWSEINFOW {
        hwndOwner: owner,
        pszDisplayName: PWSTR(name.as_mut_ptr()),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    // SAFETY: modalny dialog vyberu priecinka; PIDL sa uvolni; cesta sa cita z buffera s dlzkou MAX_PATH.
    unsafe {
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let mut buf = [0u16; 260];
        let ok = SHGetPathFromIDListW(pidl, &mut buf).as_bool();
        CoTaskMemFree(Some(pidl as *const _));
        if !ok {
            return None;
        }
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        let s = String::from_utf16(&buf[..n]).ok()?;
        (!s.is_empty()).then_some(s)
    }
}

/// Spusti cakajuci vyber priecinka (mimo vypozicky stavu) a ulozi vysledok do konfiguracie.
pub(crate) fn run_pending_folder(hwnd: HWND) {
    let Some(kind) = with_ui(|ui| ui.pending_folder.take()).flatten() else {
        return;
    };
    let Some(path) = pick_folder(hwnd) else {
        return;
    };
    with_ui(|ui| {
        ui.add_folder(kind, path);
    });
}

/// Vyber programu (`GetOpenFileNameW`, filter `*.exe`); vracia cestu alebo chybu dialogu. Zrusenie = `Ok(None)`.
fn pick_exe(owner: HWND) -> Result<Option<String>, u32> {
    use crate::transfer_dialog::{filter_wide, run_dialog, wide0, Kind, Pick, Spec};
    let spec = Spec {
        title: t("tls.apps_pick"),
        filter: filter_wide(&[
            (&format!("{} (*.exe)", t("tls.apps_filter")), "*.exe"),
            (&format!("{} (*.*)", t("xfer.dlg_all")), "*.*"),
        ]),
        initial: String::new(),
        def_ext: wide0("exe"),
    };
    match run_dialog(Kind::Open, owner, &spec) {
        Pick::Chosen(p) => Ok(Some(p.to_string_lossy().into_owned())),
        Pick::Cancelled => Ok(None),
        Pick::Failed(code) => Err(code),
    }
}

/// Spusti cakajuci vyber aplikacie (mimo vypozicky stavu) a ulozi vysledok do konfiguracie.
pub(crate) fn run_pending_app(hwnd: HWND) {
    if !with_ui(|ui| std::mem::take(&mut ui.pending_app)).unwrap_or(false) {
        return;
    }
    let picked = pick_exe(hwnd);
    with_ui(|ui| match picked {
        Ok(Some(path)) => ui.add_app(&path),
        Ok(None) => {}
        Err(_) => ui.apps_msg = Some("tls.apps_fail"),
    });
}

impl Ui {
    /// Prida aplikaciu (len nazov `.exe`, nie cesta) do zoznamu vylucenych: bez duplicit (bez ohladu na velkost pismen),
    /// najviac `MAX_APPS`; neplatny vstup sa nezapise a zobrazi sa dovod. Uklada konfiguraciu.
    pub(crate) fn add_app(&mut self, input: &str) {
        self.apps_msg = None;
        let Some(name) = transom_config::normalize_exe_name(input) else {
            self.apps_msg = Some("tls.apps_bad");
            return;
        };
        let list = &mut self.cfg.tools.clipboard.excluded_apps;
        if list.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
            self.apps_msg = Some("tls.apps_dup");
        } else if list.len() >= MAX_APPS {
            self.apps_msg = Some("tls.apps_full");
        } else {
            list.push(name);
            self.save();
        }
    }

    /// Prida priecinok do zoznamu (bez duplicit, najviac `MAX_FOLDERS`) a ulozi konfiguraciu.
    pub(crate) fn add_folder(&mut self, kind: &str, path: String) {
        // priecinky snimok sa overuju (absolutna cesta, nie zariadenie ani koren disku); zoznamy vyhladavaca ostavaju volne
        if kind == "shots" && !shots_folder_ok(&path) {
            return;
        }
        let list = folders_mut(&mut self.cfg.tools, kind);
        let dup = list.iter().any(|f| f.eq_ignore_ascii_case(&path));
        if !dup && list.len() < MAX_FOLDERS && path.len() <= 1000 {
            list.push(path);
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_tests::Env;
    use transom_config::load;

    #[test]
    fn privacy_defaults_are_off_and_every_toggle_round_trips_through_the_saved_file() {
        let e = Env::new("tls-toggle");
        let mut ui = e.ui();
        assert!(
            !get(&ui.cfg.tools, "clip.enabled"),
            "historia schranky je predvolene vypnuta"
        );
        ui.click(Hit::Nav(Page::Tools));
        for (id, _) in TOGGLES {
            let before = get(&ui.cfg.tools, id);
            ui.click(Hit::ToolsToggle(id));
            let saved = load(&e.root.join("config.json")).config;
            assert_eq!(get(&saved.tools, id), !before, "{id}");
        }
        ui.click(Hit::ToolsCycle("clip.keep"));
        ui.click(Hit::ToolsCycle("clip.max"));
        let saved = load(&e.root.join("config.json")).config;
        assert_eq!(saved.tools.clipboard.retention_days, 14);
        assert_eq!(saved.tools.clipboard.max_items, 200);
    }

    #[test]
    fn clearing_needs_two_clicks_writes_only_a_marker_and_any_other_click_disarms() {
        let e = Env::new("tls-clear");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Tools));
        let dir = transom_tools::tools_dir();
        let marker = dir.join("clear-clipboard.req");
        ui.click(Hit::ToolsClear("clipboard"));
        assert!(
            ui.tools_confirm.is_some() && !marker.exists(),
            "prvy klik len zbrojí"
        );
        ui.click(Hit::ToolsToggle("notes.enabled"));
        assert!(ui.tools_confirm.is_none(), "ina akcia zrusi");
        ui.click(Hit::ToolsClear("clipboard"));
        ui.click(Hit::ToolsClear("clipboard"));
        assert!(marker.exists(), "druhy klik zapisal znacku");
        assert!(ui.tools_confirm.is_none());
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn launcher_options_are_saved_and_folder_lists_are_unique_limited_and_removable() {
        let e = Env::new("tls-launcher");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Tools));
        assert!(
            !get(&ui.cfg.tools, "lau.recent"),
            "nedavne volby su predvolene vypnute"
        );
        ui.click(Hit::ToolsToggle("lau.files"));
        ui.click(Hit::ToolsToggle("lau.recent"));
        ui.click(Hit::ToolsCycle("lau.max"));
        let saved = load(&e.root.join("config.json")).config;
        assert!(!saved.tools.launcher.files && saved.tools.launcher.recent_searches);
        assert_eq!(saved.tools.launcher.max_results, 12);
        ui.add_folder("excluded", r"C:\Private".into());
        ui.add_folder("excluded", r"c:\private".into());
        for i in 0..20 {
            ui.add_folder("fallback", format!("C:\\F{i}"));
        }
        let saved = load(&e.root.join("config.json")).config;
        assert_eq!(
            saved.tools.launcher.excluded_folders,
            vec![r"C:\Private".to_string()]
        );
        assert_eq!(saved.tools.launcher.fallback_folders.len(), MAX_FOLDERS);
        ui.click(Hit::ToolsRemoveFolder("excluded", 0));
        ui.click(Hit::ToolsRemoveFolder("excluded", 7));
        let saved = load(&e.root.join("config.json")).config;
        assert!(saved.tools.launcher.excluded_folders.is_empty());
    }

    #[test]
    fn the_launcher_hotkey_is_captured_validated_cancelled_turned_off_and_restored() {
        let e = Env::new("tls-hotkey");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Tools));
        let saved = || {
            load(&e.root.join("config.json"))
                .config
                .tools
                .launcher
                .hotkey
        };
        assert_eq!(
            ui.cfg.tools.launcher.hotkey, "Alt+Space",
            "predvolena skratka"
        );
        // bez zachytavania klavesy nic nemenia
        assert!(!ui.hotkey_capture_key(0x4B, true, false, false, false));
        ui.click(Hit::ToolsHotkey);
        assert!(ui.hotkey_capture);
        // samotne modifikatory sa cakaju, neplatna kombinacia ostane v zachytavani s vysvetlenim
        assert!(!ui.hotkey_capture_key(0x11, true, false, false, false));
        assert!(!ui.hotkey_capture_key(0x4B, false, false, true, false));
        assert_eq!(ui.hotkey_msg, Some("tls.lau_hotkey_bad"));
        assert!(ui.hotkey_capture);
        assert!(ui.hotkey_capture_key(0x4B, true, false, true, false));
        ui.save();
        assert!(!ui.hotkey_capture && ui.hotkey_msg.is_none());
        assert_eq!(saved(), "Ctrl+Shift+K");
        // Esc zrusi zachytavanie bez zmeny; ina volba ho tiez ukonci
        ui.click(Hit::ToolsHotkey);
        assert!(!ui.hotkey_capture_key(0x1B, false, false, false, false));
        assert!(!ui.hotkey_capture);
        ui.click(Hit::ToolsHotkey);
        ui.click(Hit::ToolsToggle("lau.files"));
        assert!(!ui.hotkey_capture);
        assert_eq!(saved(), "Ctrl+Shift+K");
        ui.click(Hit::ToolsHotkeyOff);
        assert_eq!(saved(), "", "vypnuta skratka");
        assert!(ui.cfg.tools.launcher.hotkey().is_none());
        ui.click(Hit::ToolsHotkeyDefault);
        assert_eq!(saved(), "Alt+Space");
        // na celej obrazovke predvolene nie; volba sa ulozi
        assert!(!ui.cfg.tools.launcher.hotkey_in_fullscreen);
        ui.click(Hit::ToolsToggle("lau.hotkey_fs"));
        let c = load(&e.root.join("config.json")).config.tools.launcher;
        assert!(c.hotkey_in_fullscreen);
    }

    #[test]
    fn excluded_apps_are_stored_as_file_names_only_without_duplicates_up_to_the_limit_and_removable(
    ) {
        let e = Env::new("tls-apps");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Tools));
        assert!(ui.cfg.tools.clipboard.excluded_apps.is_empty());
        // cesta z vyberu suboru sa ulozi ako nazov, nikdy nie cesta (sukromie: cesta by prezradila meno uctu / adresare)
        ui.add_app(r"C:\Users\someone\AppData\Local\KeePassXC\KeePassXC.exe");
        assert_eq!(
            e.saved().tools.clipboard.excluded_apps,
            vec!["KeePassXC.exe"]
        );
        assert!(ui.apps_msg.is_none());
        // duplicita bez ohladu na velkost pismen sa nezapise a pouzivatel dostane dovod
        ui.add_app("keepassxc.EXE");
        assert_eq!(e.saved().tools.clipboard.excluded_apps.len(), 1);
        assert_eq!(ui.apps_msg, Some("tls.apps_dup"));
        // neplatne vstupy sa nezapisu
        for bad in [
            "",
            "notepad",
            "x.exe.txt",
            "a|b.exe",
            "con?.exe",
            r"C:\dir\",
            ".exe",
        ] {
            ui.add_app(bad);
            assert_eq!(ui.apps_msg, Some("tls.apps_bad"), "{bad:?}");
        }
        assert_eq!(ui.cfg.tools.clipboard.excluded_apps.len(), 1);
        // limit UI
        for i in 0..MAX_APPS + 5 {
            ui.add_app(&format!("app{i}.exe"));
        }
        assert_eq!(ui.cfg.tools.clipboard.excluded_apps.len(), MAX_APPS);
        assert_eq!(ui.apps_msg, Some("tls.apps_full"));
        assert_eq!(e.saved().tools.clipboard.excluded_apps.len(), MAX_APPS);
        // odstranenie: zly index nic neurobi, spravny odstrani prave tu polozku a ulozi
        ui.click(Hit::ToolsRemoveApp(MAX_APPS + 3));
        assert_eq!(ui.cfg.tools.clipboard.excluded_apps.len(), MAX_APPS);
        ui.click(Hit::ToolsRemoveApp(0));
        let saved = e.saved().tools.clipboard.excluded_apps;
        assert_eq!(saved.len(), MAX_APPS - 1);
        assert!(!saved.contains(&"KeePassXC.exe".to_string()));
        assert!(ui.apps_msg.is_none(), "dalsia volba chybu zmaze");
        // pridanie z klavesnice / mysi len zaradi vyber suboru; nic sa nezapise, kym pouzivatel nevyberie subor
        ui.click(Hit::ToolsAddApp);
        assert!(ui.pending_app);
        assert_eq!(e.saved().tools.clipboard.excluded_apps, saved);
    }

    #[test]
    fn the_exclusion_list_is_drawn_with_named_remove_and_add_controls_in_every_language() {
        use crate::a11y_tests::paint;
        let e = Env::new("tls-apps-ui");
        let mut ui = e.ui();
        ui.cfg.tools.clipboard.excluded_apps = vec!["KeePass.exe".into(), "Bitwarden.exe".into()];
        ui.page = Page::Clipboard;
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            ui.scroll = 0;
            paint(&mut ui, 96);
            let (entries, _, targets, _) = ui.a11y_snapshot(&[], None);
            let name_of = |h: &Hit| -> String {
                let i = targets
                    .iter()
                    .position(|t| matches!(t, crate::page_a11y::Target::Hit(x) if x == h))
                    .unwrap_or_else(|| panic!("{lang:?}: chyba ovladac {h:?}"));
                entries[i].name.clone()
            };
            let head = t("tls.apps_h");
            for (i, app) in ["KeePass.exe", "Bitwarden.exe"].iter().enumerate() {
                let n = name_of(&Hit::ToolsRemoveApp(i));
                assert!(
                    n.contains(app) && n.starts_with(&head),
                    "{lang:?}: {n:?} nesie nazov skupiny a aplikacie"
                );
            }
            let add = name_of(&Hit::ToolsAddApp);
            assert!(add.starts_with(&head) && add != head, "{lang:?}: {add:?}");
            // odlisne od "Pridat" v zozname vyhladavaca (ten je na vlastnej podstranke)
            assert!(!add.starts_with(&t("tls.lau_excl_h")), "{lang:?}: {add:?}");
            for k in [
                "tls.apps_h",
                "tls.apps_note",
                "tls.apps_add",
                "tls.apps_pick",
            ] {
                assert!(!t(k).starts_with("tls."), "{lang:?}: neprelozeny {k}");
            }
        }
        // plny zoznam: tlacidlo Pridat zmizne (UI nepridava nad limit)
        ui.cfg.tools.clipboard.excluded_apps = (0..MAX_APPS).map(|i| format!("a{i}.exe")).collect();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        paint(&mut ui, 96);
        assert!(!ui.all_hits.iter().any(|(_, h)| *h == Hit::ToolsAddApp));
    }

    #[test]
    fn screenshot_folders_are_validated_unique_limited_removable_and_saved_without_a_clear_button()
    {
        let e = Env::new("tls-shots-folders");
        let mut ui = e.ui();
        ui.click(Hit::Nav(Page::Tools));
        // predvolene: kniznica zapnuta s priecinkom Windows a bez vlastnych priecinkov
        assert!(get(&ui.cfg.tools, "shots.enabled") && get(&ui.cfg.tools, "shots.default"));
        assert!(ui.cfg.tools.screenshots.folders.is_empty());
        // neplatne cesty sa neprijmu (relativna, zariadenie, koren disku, riadiace znaky, prazdna, pipe)
        for bad in [
            "",
            "   ",
            "Pictures",
            r"..\Pictures",
            r"C:\",
            "C:",
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\x",
            r"\\server",
            r"\\server\pipe\x",
            "D:\\a\u{7}b",
        ] {
            ui.add_folder("shots", bad.to_string());
        }
        assert!(
            ui.cfg.tools.screenshots.folders.is_empty(),
            "{:?}",
            ui.cfg.tools.screenshots.folders
        );
        ui.add_folder("shots", r"D:\Screens".into());
        ui.add_folder("shots", r"d:\screens".into());
        ui.add_folder("shots", r"\\nas\share\shots".into());
        assert_eq!(
            e.saved().tools.screenshots.folders,
            vec![r"D:\Screens", r"\\nas\share\shots"]
        );
        for i in 0..MAX_FOLDERS + 4 {
            ui.add_folder("shots", format!("E:\\S{i}"));
        }
        assert_eq!(ui.cfg.tools.screenshots.folders.len(), MAX_FOLDERS);
        // zoznamy vyhladavaca su nezavisle
        assert!(ui.cfg.tools.launcher.excluded_folders.is_empty());
        // odstranenie: zly index nic nerobi
        ui.click(Hit::ToolsRemoveFolder("shots", 99));
        assert_eq!(ui.cfg.tools.screenshots.folders.len(), MAX_FOLDERS);
        ui.click(Hit::ToolsRemoveFolder("shots", 0));
        assert_eq!(e.saved().tools.screenshots.folders[0], r"\\nas\share\shots");
        // vypnutie predvoleneho priecinka sa ulozi
        ui.click(Hit::ToolsToggle("shots.default"));
        assert!(!e.saved().tools.screenshots.default_folder);
        // pridanie len zaradi vyber priecinka; nic sa nezmeni, kym pouzivatel nevyberie
        let before = e.saved().tools.screenshots.folders;
        ui.click(Hit::ToolsAddFolder("shots"));
        assert_eq!(ui.pending_folder, Some("shots"));
        assert_eq!(e.saved().tools.screenshots.folders, before);
        // kniznica iba cita: medzi vymazatelnymi uloziskami nie su snimky
        assert!(CLEARS.iter().all(|(id, _)| *id != "screenshots"));
    }

    #[test]
    fn the_screenshot_folder_controls_are_drawn_with_named_buttons_in_every_language() {
        use crate::a11y_tests::paint;
        let e = Env::new("tls-shots-ui");
        let mut ui = e.ui();
        ui.cfg.tools.screenshots.folders = vec![r"D:\Screens".into(), r"E:\More".into()];
        ui.page = Page::Shots;
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            ui.scroll = 0;
            paint(&mut ui, 96);
            let (entries, _, targets, _) = ui.a11y_snapshot(&[], None);
            let name_of = |h: &Hit| -> String {
                let i = targets
                    .iter()
                    .position(|t| matches!(t, crate::page_a11y::Target::Hit(x) if x == h))
                    .unwrap_or_else(|| panic!("{lang:?}: chyba ovladac {h:?}"));
                entries[i].name.clone()
            };
            let head = t("tls.shots_folders_h");
            for (i, f) in [r"D:\Screens", r"E:\More"].iter().enumerate() {
                let n = name_of(&Hit::ToolsRemoveFolder("shots", i));
                assert!(n.contains(f) && n.starts_with(&head), "{lang:?}: {n:?}");
            }
            let add = name_of(&Hit::ToolsAddFolder("shots"));
            assert!(add.starts_with(&head) && add != head, "{lang:?}: {add:?}");
            // odlisne od "Pridat" v zozname vyhladavaca (ten je na vlastnej podstranke)
            assert!(!add.starts_with(&t("tls.lau_excl_h")), "{lang:?}: {add:?}");
            let on = name_of(&Hit::ToolsToggle("shots.default"));
            assert!(!on.starts_with("tls."), "{lang:?}: {on:?}");
            for k in [
                "tls.shots_on",
                "tls.shots_default",
                "tls.shots_folders_h",
                "tls.shots_note",
            ] {
                assert!(!t(k).starts_with("tls."), "{lang:?}: neprelozeny {k}");
            }
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }

    #[test]
    fn cycling_wraps_and_unknown_values_restart_the_cycle() {
        assert_eq!(next(&KEEP_DAYS, 30), 0);
        assert_eq!(next(&KEEP_DAYS, 0), 1);
        assert_eq!(
            next(&KEEP_DAYS, 99),
            1,
            "nezname -> prvy prvok (cyklus zacne od zaciatku)"
        );
        assert_eq!(next(&MAX_ITEMS, 500), 50);
    }
}
