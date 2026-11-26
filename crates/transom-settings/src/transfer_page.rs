//! Stranka "Export a import": export nastaveni do suboru, import so nahladom zmien, vyberom sekcii a dvojkrokovym potvrdenim.
//! Stavovy automat (`Transfer`) a cela logika su bez okna a bez dialogu (`finish_dialog` dostane vybranu cestu), takze ich
//! testuju UI testy s docasnymi subormi. Kriticke pravidla: nic sa nezapise, kym pouzivatel nepotvrdi; pred potvrdenim sa
//! konfiguracia znova nacita z disku a nahlad sa porovna s tym, co pouzivatel videl; pred zapisom sa urobi zaloha `config.json`;
//! zapis ide cez `save_merged` (zamok, zlucenie so subeznymi zmenami, atomicky zapis).
use super::*;
use crate::transfer_dialog::{Kind, Pick};
use crate::transfer_io::{self, TransferError};
use std::path::Path;
use transom_config::transfer::{
    self, Change, ExportOpts, ImportDoc, ImportError, ImportPreview, Section,
};

/// Najviac zmien zobrazenych pod jednou sekciou.
const MAX_CHANGES_SHOWN: usize = 8;
/// Najviac ignorovanych poloziek v zozname.
const MAX_IGNORED_SHOWN: usize = 6;

#[derive(Default)]
pub(crate) enum Transfer {
    #[default]
    Idle,
    Preview(Box<TransferPreview>),
}

/// Otvoreny nahlad importu.
pub(crate) struct TransferPreview {
    pub doc: ImportDoc,
    /// Len nazov suboru (nie cela cesta).
    pub file: String,
    /// Vyber sekcii podla `Section::ALL`.
    pub selected: [bool; 6],
    /// Zmeny vybranych sekcii, ktore pouzivatel videl pri prvom kliku na "Importovat"; druhy klik ich znova porovna.
    pub confirm: Option<Vec<Change>>,
}

/// Co ma otvorit dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XferOp {
    Export,
    Import,
}

/// Nazov sekcie v jazyku aplikacie (existujuce kluce stranok nastaveni).
pub(crate) fn section_label(s: Section) -> String {
    t(match s {
        Section::Layout => "layout.title",
        Section::Clock => "clock.title",
        Section::Media => "med.title",
        Section::Appearance => "app.title",
        Section::Language => "lang.title",
        Section::Usage => "xfer.sec_usage",
    })
}

/// Citatelny popisok pola (existujuce kluce); neznamy nazov sa ukaze surovy.
pub(crate) fn field_label(s: Section, field: &str) -> String {
    let key = match (s, field) {
        (Section::Layout, _) => "layout.title",
        (Section::Language, _) => "lang.title",
        (Section::Clock, "hour") => "clock.hour",
        (Section::Clock, "leading_zero") => "clock.zero",
        (Section::Clock, "seconds") => "clock.seconds",
        (Section::Clock, "weekday") | (Section::Clock, "weekday_long") => "clock.weekday",
        (Section::Clock, "date") => "clock.date",
        (Section::Clock, "first_day") => "clock.first",
        (Section::Media, "marquee") => "med.enable",
        (Section::Media, "marquee_direction") => "med.direction",
        (Section::Media, "marquee_speed") => "med.speed",
        (Section::Media, "marquee_paused") => "med.paused",
        (Section::Appearance, "bg_color") => "app.title",
        (Section::Appearance, "bg_opacity") => "app.opacity",
        (Section::Appearance, "bg_transparent") => "app.transparent",
        (Section::Appearance, "tray_icon") => "app.tray_icon",
        (Section::Appearance, "glass_panels") => "app.glass_panels",
        (Section::Appearance, "glass_opacity") => "app.glass_opacity",
        (Section::Appearance, "glass_bar") => "app.glass_bar",
        (Section::Usage, "refresh_minutes") => "aip.interval",
        (Section::Usage, "bar_provider") => "aip.provider",
        (Section::Usage, "ring_mode") => "aip.mode",
        _ => return field.to_string(),
    };
    t(key)
}

/// Citatelny text hodnoty (existujuce kluce); neznama hodnota ostane taka, aka je (v nahlade je uz bezpecne skratena).
pub(crate) fn value_text(field: &str, raw: &str) -> String {
    match (field, raw) {
        (_, "") => t("xfer.v_default"),
        (_, "true") => t("opt.yes"),
        (_, "false") => t("opt.no"),
        ("hour" | "leading_zero" | "first_day", "locale") => t("opt.system"),
        ("hour", "12") => t("opt.12h"),
        ("hour", "24") => t("opt.24h"),
        ("leading_zero", "yes") => t("opt.zero_yes"),
        ("leading_zero", "no") => t("opt.zero_no"),
        ("date", "none") => t("opt.date_none"),
        ("date", "short") => t("opt.date_short"),
        ("date", "long") => t("opt.date_long"),
        ("marquee_direction", "ltr") => t("med.dir_ltr"),
        ("marquee_direction", "rtl") => t("med.dir_rtl"),
        ("marquee_speed", "slow") => t("med.slow"),
        ("marquee_speed", "normal") => t("med.normal"),
        ("marquee_speed", "fast") => t("med.fast"),
        ("ring_mode", "used") => t("opt.used"),
        ("ring_mode", "remaining") => t("opt.left"),
        ("bar_provider", "auto") => t("opt.auto"),
        ("bar_provider", "claude") => "Claude".into(),
        ("bar_provider", "codex") => "Codex".into(),
        ("refresh_minutes", "0") => t("opt.on_demand"),
        ("refresh_minutes", n) => tf("opt.minutes", &[n]),
        _ => raw.to_string(),
    }
}

/// Polozky rozlozenia z textu zhrnutia (`id` alebo `(id)` pre skryte) na nazvy v jazyku aplikacie.
fn layout_items_text(s: &str) -> String {
    s.split(", ")
        .filter(|x| !x.is_empty())
        .map(
            |tok| match tok.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
                Some(id) => format!("({})", label(id)),
                None => label(tok),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}

/// Skupina rozlozenia z textu zhrnutia: `left: a, b` -> (kluc, polozky).
fn split_group(part: &str) -> (&str, &str) {
    part.split_once(": ").unwrap_or((part, ""))
}

fn group_name(g: &str) -> String {
    match g {
        "left" => t("xfer.g_left"),
        "center" => t("xfer.g_center"),
        "right" => t("xfer.g_right"),
        other => other.to_string(),
    }
}

/// Riadky jednej zmeny do nahladu (rozlozenie sa rozlozi na skupiny, ktore sa lisia).
pub(crate) fn change_lines(c: &Change) -> Vec<String> {
    if c.section == Section::Layout && c.field == "items" {
        let (from, to): (Vec<&str>, Vec<&str>) =
            (c.from.split(" | ").collect(), c.to.split(" | ").collect());
        let mut out = Vec::new();
        for (a, b) in from.iter().zip(&to) {
            if a != b {
                let (g, fa) = split_group(a);
                let (_, fb) = split_group(b);
                out.push(format!(
                    "{}: {} → {}",
                    group_name(g),
                    layout_items_text(fa),
                    layout_items_text(fb)
                ));
            }
        }
        return out;
    }
    let label = if c.field.is_empty() {
        section_label(c.section)
    } else {
        field_label(c.section, &c.field)
    };
    vec![format!(
        "{label}: {} → {}",
        value_text(&c.field, &c.from),
        value_text(&c.field, &c.to)
    )]
}

/// Ignorovana polozka v citatelnom tvare (neplatna sekcia = vysvetlenie, ostatne surovy technicky nazov).
pub(crate) fn ignored_text(raw: &str) -> String {
    if let Some(sec) = raw.strip_suffix(" (invalid)") {
        if let Some(s) = Section::ALL.iter().find(|s| s.key() == sec) {
            return tf("xfer.invalid", &[&section_label(*s)]);
        }
    }
    raw.to_string()
}

/// Dnesne UTC (rok, mesiac, den, hodina, minuta, sekunda) bez zavislosti na casovom pasme.
fn utc_now() -> (i32, u32, u32, u32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let ymd = transom_core::clock::civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400) as u32;
    (ymd.y, ymd.m, ymd.d, rem / 3600, rem % 3600 / 60, rem % 60)
}

pub(crate) fn export_opts() -> ExportOpts {
    let (y, m, d, h, mi, s) = utc_now();
    ExportOpts {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
    }
}

fn backup_stamp() -> String {
    let (y, m, d, h, mi, s) = utc_now();
    format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// Hlaska k chybe suboru.
pub(crate) fn error_text(e: &TransferError) -> String {
    match e {
        TransferError::Io(m) => tf("xfer.read_failed", &[m]),
        TransferError::NotAFile => t("xfer.err_notfile"),
        TransferError::TooLarge => t("xfer.err_toolarge"),
        TransferError::Utf16 => t("xfer.err_utf16"),
        TransferError::NotUtf8 | TransferError::Format(ImportError::Corrupt) => {
            t("xfer.err_corrupt")
        }
        TransferError::Format(ImportError::TooLarge) => t("xfer.err_toolarge"),
        TransferError::Format(ImportError::Foreign) => t("xfer.err_foreign"),
        TransferError::Format(ImportError::Newer(v)) => tf("xfer.err_newer", &[&v.to_string()]),
        TransferError::PrivatePath => t("xfer.err_private"),
        TransferError::NeedExtension => t("xfer.err_ext"),
    }
}

/// Zmeny len zvolenych sekcii.
fn changes_of(prev: &ImportPreview, secs: &[Section]) -> Vec<Change> {
    prev.changes
        .iter()
        .filter(|c| secs.contains(&c.section))
        .cloned()
        .collect()
}

/// Odstrani z textu pre citacku obrazovky riadiace a obojsmerne znaky a skrati ho.
pub(crate) fn a11y_text(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect();
    if clean.chars().count() > 120 {
        format!("{}…", clean.chars().take(120).collect::<String>())
    } else {
        clean
    }
}

impl Ui {
    /// Export a import nie su dostupne: konfiguracia je len na citanie (poskodena / novsia) alebo bezi dialog.
    fn xfer_blocked(&self) -> bool {
        self.read_only || self.transfer_busy
    }

    pub(crate) fn transfer_open(&self) -> bool {
        matches!(self.transfer, Transfer::Preview(_))
    }

    /// Nahlad voci AKTUALNEJ konfiguracii (pocita sa pri kazdom kresleni a pred potvrdenim).
    fn transfer_preview_now(&self) -> Option<ImportPreview> {
        match &self.transfer {
            Transfer::Preview(p) => Some(transfer::preview(&self.cfg, &p.doc)),
            Transfer::Idle => None,
        }
    }

    /// Sekcie, ktore su vybrane, v subore platne a maju aspon jednu zmenu.
    pub(crate) fn transfer_selected(&self) -> Vec<Section> {
        let (Transfer::Preview(p), Some(prev)) = (&self.transfer, self.transfer_preview_now())
        else {
            return Vec::new();
        };
        Section::ALL
            .iter()
            .enumerate()
            .filter(|(i, s)| {
                p.selected[*i]
                    && p.doc.present.contains(s)
                    && prev.changes.iter().any(|c| c.section == **s)
            })
            .map(|(_, s)| *s)
            .collect()
    }

    /// Po vybere v dialogu: export zapise subor, import otvori nahlad. Chyby sa len oznamia; konfiguracia sa nemeni.
    pub(crate) fn finish_dialog(&mut self, op: XferOp, pick: Pick) {
        match pick {
            Pick::Cancelled => {}
            Pick::Failed(code) => self.status = tf("xfer.read_failed", &[&format!("0x{code:04X}")]),
            Pick::Chosen(path) => match op {
                XferOp::Export => self.begin_export_to(&path),
                XferOp::Import => self.begin_import_from(&path),
            },
        }
    }

    pub(crate) fn begin_export_to(&mut self, path: &Path) {
        if self.read_only {
            self.status = t("xfer.readonly");
            return;
        }
        self.flush_pending();
        match transfer_io::export_to(path, &self.cfg, &export_opts()) {
            Ok(()) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.status = tf("xfer.exported", &[&a11y_text(&name)]);
            }
            Err(TransferError::Io(m)) => self.status = tf("xfer.export_failed", &[&m]),
            Err(e) => self.status = error_text(&e),
        }
    }

    pub(crate) fn begin_import_from(&mut self, path: &Path) {
        if self.read_only {
            self.status = t("xfer.readonly");
            return;
        }
        self.flush_pending();
        match transfer_io::import_from(path) {
            Err(e) => self.status = error_text(&e),
            Ok(doc) => {
                let file = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let prev = transfer::preview(&self.cfg, &doc);
                if prev.changes.is_empty() && prev.ignored.is_empty() {
                    self.status = t("xfer.no_changes");
                    return;
                }
                let mut selected = [false; 6];
                for (i, s) in Section::ALL.iter().enumerate() {
                    selected[i] = prev.changes.iter().any(|c| c.section == *s);
                }
                self.transfer = Transfer::Preview(Box::new(TransferPreview {
                    doc,
                    file: a11y_text(&file),
                    selected,
                    confirm: None,
                }));
                self.status.clear();
                // po otvoreni nahladu sa klavesnicovy fokus presunie na prvu zmenenu sekciu
                if self.kfocus.is_some() {
                    if let Some(i) = selected.iter().position(|x| *x) {
                        self.kfocus = Some(Hit::TransferSection(i as u8));
                        self.reveal = self.kfocus.clone();
                    }
                }
            }
        }
    }

    pub(crate) fn cancel_transfer(&mut self) {
        let was_open = self.transfer_open();
        self.transfer = Transfer::Idle;
        if was_open && self.kfocus.is_some() {
            self.kfocus = Some(Hit::TransferImport);
            self.reveal = self.kfocus.clone();
        }
    }

    pub(crate) fn transfer_toggle(&mut self, i: u8) {
        let sections = Section::ALL;
        let Some(s) = sections.get(i as usize).copied() else {
            return;
        };
        let has_changes = self
            .transfer_preview_now()
            .is_some_and(|p| p.changes.iter().any(|c| c.section == s));
        if let Transfer::Preview(p) = &mut self.transfer {
            if p.doc.present.contains(&s) && has_changes {
                p.selected[i as usize] = !p.selected[i as usize];
            }
            p.confirm = None;
        }
    }

    /// Klik na "Importovat vybrane": prvy krok zaznamena, co sa zmeni; druhy krok (po znovu-overeni) zapise.
    pub(crate) fn transfer_apply(&mut self) {
        let secs = self.transfer_selected();
        if secs.is_empty() || self.read_only {
            return;
        }
        let Some(prev) = self.transfer_preview_now() else {
            return;
        };
        let now = changes_of(&prev, &secs);
        let first_step = matches!(&self.transfer, Transfer::Preview(p) if p.confirm.is_none());
        if first_step {
            if let Transfer::Preview(p) = &mut self.transfer {
                p.confirm = Some(now);
            }
            return;
        }
        self.confirm_import(&secs);
    }

    /// Druhy krok importu. Konfiguracia sa znova nacita z disku; ak sa nahlad od prveho kroku zmenil (alebo subor je
    /// poskodeny / novsi), nezapise sa nic. Zaloha musi uspiet. Zapis ide cez `save_merged`.
    fn confirm_import(&mut self, secs: &[Section]) {
        let Transfer::Preview(p) = &self.transfer else {
            return;
        };
        let (doc, seen) = (p.doc.clone(), p.confirm.clone().unwrap_or_default());
        let path = transom_config::default_path();
        let fresh = transom_config::load(&path);
        if fresh.issue.is_some() {
            self.status = tf("xfer.import_failed", &[&t("xfer.readonly")]);
            return;
        }
        // aktualna konfiguracia z disku (zmeny z lisaty / z inej instancie)
        self.cfg = fresh.config.clone();
        self.base = fresh.config;
        let prev = transfer::preview(&self.cfg, &doc);
        let now = changes_of(&prev, secs);
        if now.is_empty() {
            self.transfer = Transfer::Idle;
            self.status = t("xfer.no_changes");
            return;
        }
        if now != seen {
            if let Transfer::Preview(p) = &mut self.transfer {
                p.confirm = None;
            }
            self.status = t("xfer.stale");
            return;
        }
        let out = transfer::apply_import(&self.cfg, &doc, secs);
        if let Err(e) =
            transom_config::backup_file(&path, &backup_stamp(), transom_config::BACKUPS_KEPT)
        {
            self.status = tf("xfer.backup_failed", &[&e.to_string()]);
            return;
        }
        match transom_config::save_merged(&path, &self.base, &out) {
            Ok(merged) => {
                self.cfg = merged.clone();
                self.base = merged;
                apply_lang(&self.cfg, &self.windows_lang);
                self.bad = [false; 4];
                self.fill_edits(None);
                notify_bar();
                self.transfer = Transfer::Idle;
                let sep = ", ";
                let names: Vec<String> = secs.iter().map(|s| section_label(*s)).collect();
                self.status = tf("xfer.imported", &[&names.join(sep)]);
                if self.kfocus.is_some() {
                    self.kfocus = Some(Hit::TransferImport);
                    self.reveal = self.kfocus.clone();
                }
            }
            Err(e) => {
                self.status = tf("xfer.import_failed", &[&e.to_string()]);
            }
        }
    }

    /// Texty dialogu v jazyku aplikacie.
    pub(crate) fn dialog_spec(&self, op: XferOp) -> crate::transfer_dialog::Spec {
        let filter = crate::transfer_dialog::filter_wide(&[
            (&format!("{} (*.json)", t("xfer.dlg_json")), "*.json"),
            (&format!("{} (*.*)", t("xfer.dlg_all")), "*.*"),
        ]);
        let (y, m, d, ..) = utc_now();
        match op {
            XferOp::Export => crate::transfer_dialog::Spec {
                title: t("xfer.dlg_export"),
                filter,
                initial: crate::transfer_dialog::default_export_name(y, m, d),
                def_ext: crate::transfer_dialog::wide0("json"),
            },
            XferOp::Import => crate::transfer_dialog::Spec {
                title: t("xfer.dlg_import"),
                filter,
                initial: String::new(),
                def_ext: crate::transfer_dialog::wide0("json"),
            },
        }
    }

    pub(crate) fn paint_transfer(&mut self, p: &mut dyn Painter, x0: i32, y: i32, w: i32) {
        let th = self.theme;
        let s = p.scale();
        let px = |v: i32| (v as f32 * s).round() as i32;
        self.acc_statics.clear();
        let mut y = self.title(p, x0, y, w, "xfer.title");
        y = self.paragraph(p, x0, y, w, &t("xfer.intro"), 12.0, th.muted) + px(12);
        y = self.card_begin(s, y);
        y = self.subsection(p, x0, y, w, &t("xfer.includes_h"));
        y = self.paragraph(p, x0, y, w, &t("xfer.includes"), 12.0, th.muted) + px(8);
        y = self.subsection(p, x0, y, w, &t("xfer.excludes_h"));
        y = self.paragraph(p, x0, y, w, &t("xfer.excludes"), 12.0, th.muted) + px(8);
        y = self.paragraph(p, x0, y, w, &t("xfer.backup_note"), 12.0, th.muted) + px(12);
        if self.read_only {
            // export a import sa vtedy nekresli ako ovladace (pri poskodenom subore by `cfg` boli len predvolene hodnoty)
            y = self.infobar(p, x0, y, w, &t("xfer.readonly"));
            self.card_end(s, x0, w, y - px(8));
            return;
        }
        let busy = self.transfer_busy;
        if !busy {
            y = self.flow_buttons(
                p,
                x0,
                y,
                w,
                vec![
                    (t("xfer.export"), Hit::TransferExport, false),
                    (t("xfer.import"), Hit::TransferImport, false),
                ],
            );
        } else {
            y += px(design::CONTROL_H_LP + 10);
        }
        y = self.card_end(s, x0, w, y - px(10));
        let (Transfer::Preview(pv), Some(prev)) = (&self.transfer, self.transfer_preview_now())
        else {
            return;
        };
        let (doc, file, selected, confirming) = (
            pv.doc.clone(),
            pv.file.clone(),
            pv.selected,
            pv.confirm.is_some(),
        );
        y = self.card_begin(s, y);
        let heading = tf("xfer.preview_h", &[&file]);
        y = self.section(p, x0, y, w, &heading);
        self.acc_statics.push(heading);
        let header = if doc.app_version.is_empty() && doc.exported_at.is_empty() {
            t("xfer.header_unknown")
        } else {
            tf("xfer.header", &[&doc.app_version, &doc.exported_at])
        };
        y = self.paragraph(p, x0, y, w, &header, 12.0, th.muted) + px(10);
        let mut absent = Vec::new();
        for (i, sec) in Section::ALL.iter().enumerate() {
            if !doc.present.contains(sec) {
                absent.push(section_label(*sec));
                continue;
            }
            let changes: Vec<&Change> = prev.changes.iter().filter(|c| c.section == *sec).collect();
            let row = Rect::new(x0, y, w, px(24));
            if changes.is_empty() {
                // bez zmien: sede policko bez ovladaca
                self.checkbox(
                    p,
                    Rect::new(row.x + px(2), row.y + px(2), px(20), px(20)),
                    false,
                );
                let txt = format!("{} – {}", section_label(*sec), t("xfer.same"));
                p.draw_text(
                    &txt,
                    Rect::new(row.x + px(32), row.y, row.w - px(32), row.h),
                    &Ty::Body.style(th.muted, HAlign::Left),
                );
                self.acc_statics.push(txt);
                y += px(28);
                continue;
            }
            let on = selected[i];
            let hit = Hit::TransferSection(i as u8);
            if self.hover.as_ref() == Some(&hit) {
                p.fill_round_rect(row, px(design::radius::CONTROL), th.hover);
            }
            self.checkbox(
                p,
                Rect::new(row.x + px(2), row.y + px(2), px(20), px(20)),
                on,
            );
            let txt = format!(
                "{} – {}",
                section_label(*sec),
                tf("xfer.changes", &[&changes.len().to_string()])
            );
            p.draw_text(
                &txt,
                Rect::new(row.x + px(32), row.y, row.w - px(32), row.h),
                &Ty::Body.style(th.foreground, HAlign::Left),
            );
            self.tag(&hit, txt.clone(), Some(on));
            self.hits.push((row, hit));
            y += px(26);
            let mut shown = 0usize;
            let lines: Vec<String> = changes.iter().flat_map(|c| change_lines(c)).collect();
            for l in lines.iter().take(MAX_CHANGES_SHOWN) {
                y = self.paragraph(p, x0 + px(34), y, w - px(34), l, 12.0, th.muted);
                self.acc_statics.push(l.clone());
                shown += 1;
            }
            if lines.len() > shown {
                let more = tf("xfer.more", &[&(lines.len() - shown).to_string()]);
                y = self.paragraph(p, x0 + px(34), y, w - px(34), &more, 12.0, th.muted);
            }
            y += px(8);
        }
        if !absent.is_empty() {
            let txt = tf("xfer.absent", &[&absent.join(", ")]);
            y = self.paragraph(p, x0, y, w, &txt, 12.0, th.muted) + px(6);
            self.acc_statics.push(txt);
        }
        if !prev.ignored.is_empty() {
            y = self.subsection(p, x0, y, w, &t("xfer.ignored_h"));
            self.acc_statics.push(t("xfer.ignored_h"));
            for item in prev.ignored.iter().take(MAX_IGNORED_SHOWN) {
                let txt = ignored_text(item);
                y = self.paragraph(p, x0 + px(14), y, w - px(14), &txt, 12.0, th.muted);
                self.acc_statics.push(txt);
            }
            if prev.ignored.len() > MAX_IGNORED_SHOWN {
                let more = tf(
                    "xfer.more",
                    &[&(prev.ignored.len() - MAX_IGNORED_SHOWN).to_string()],
                );
                y = self.paragraph(p, x0 + px(14), y, w - px(14), &more, 12.0, th.muted);
            }
            y += px(6);
        }
        if prev.changes.is_empty() {
            y = self.paragraph(p, x0, y, w, &t("xfer.no_changes"), 12.0, th.muted) + px(6);
        }
        let any = !self.transfer_selected().is_empty();
        y += px(4);
        let mut items = Vec::new();
        if any {
            let label = if confirming {
                t("xfer.apply_confirm")
            } else {
                t("xfer.apply")
            };
            items.push((label, Hit::TransferApply, confirming));
        } else if !prev.changes.is_empty() {
            y = self.paragraph(p, x0, y, w, &t("xfer.pick_hint"), 12.0, th.muted) + px(8);
        }
        items.push((t("aip.cancel"), Hit::TransferCancel, false));
        y = self.flow_buttons(p, x0, y, w, items);
        self.card_end(s, x0, w, y - px(10));
    }
}

/// Spusti dialog pre cakajucu operaciu (volat mimo vypozicky stavu); po navrate (ak okno este existuje) spracuje vysledok.
pub(crate) fn run_pending(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    let Some((op, spec)) = with_ui(|ui| {
        let op = ui.pending_xfer.take()?;
        if ui.xfer_blocked() {
            return None;
        }
        ui.transfer_busy = true;
        Some((op, ui.dialog_spec(op)))
    })
    .flatten() else {
        return;
    };
    let kind = if op == XferOp::Export {
        Kind::Save
    } else {
        Kind::Open
    };
    let pick = crate::transfer_dialog::run_dialog(kind, hwnd, &spec);
    // SAFETY: cisto zistenie, ci okno este existuje (pocas dialogu ho mohol pouzivatel zatvorit).
    let alive = unsafe { IsWindow(Some(hwnd)) }.as_bool();
    with_ui(|ui| {
        ui.transfer_busy = false;
        if alive {
            ui.finish_dialog(op, pick);
        }
    });
    if alive {
        // modalny dialog vzal fokus: vrati sa oknu nastaveni (virtualny fokus kruhu ostava na tlacidle, ktore dialog spustilo)
        // SAFETY: platne okno tohto vlakna.
        unsafe {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(hwnd));
        }
    }
}
