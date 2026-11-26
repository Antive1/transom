//! Pristupnost klientskej plochy okna nastaveni (MSAA, `OBJID_CLIENT`): ovladace su kreslene v GDI, preto ich nazov, rolu,
//! stav, hodnotu a akciu zostavime z toho, co sa prave nakreslilo. Nazvy sa zachytavaju priamo pri kresleni (`tag`), takze
//! sa nemozu rozist s tym, co pouzivatel vidi; roly urcuje vycerpavajuci `role_of` (novy `Hit` bez roly sa neskompiluje).
use crate::{appearance_page, Hit, Ui};
use transom_i18n::t;
use transom_platform::menu_access::{
    MenuEntry, PageExtra, ROLE_CHECKBUTTON, ROLE_LISTITEM, ROLE_OUTLINEBUTTON, ROLE_PAGETAB,
    ROLE_PUSHBUTTON, ROLE_RADIOBUTTON, ROLE_SLIDER, ROLE_STATICTEXT, ROLE_TEXT, STATE_CHECKED,
    STATE_COLLAPSED, STATE_EXPANDED, STATE_OFFSCREEN, STATE_SELECTABLE, STATE_SELECTED,
};

/// Najviac statickych uzlov v snimke (zvysok sa zahodi: uzly nesmu nabobtnat).
const MAX_STATIC_NODES: usize = 60;

/// Co sa pri kresleni zachytilo o ovladaci (nazov, zaskrtnute / zvolene).
#[derive(Clone, Debug)]
pub(crate) struct AccMeta {
    pub hit: Hit,
    pub name: String,
    pub checked: Option<bool>,
}

/// Cielova akcia polozky snimky: ovladac stranky alebo vstupne pole.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    Hit(Hit),
    Edit(usize),
    /// Staticky text (nadpis, riadok zmeny, stav): len na citanie, bez akcie a bez fokusu.
    Static,
}

/// Zobrazene vstupne pole: index, obdlznik (x, y, sirka, vyska v klientskych px) a text.
pub(crate) type EditInfo = (usize, (i32, i32, i32, i32), String);

/// Snimka klientskej plochy: polozky, ich roly / stavy / hodnoty / akcie, cielove akcie a index polozky s fokusom.
pub(crate) type Snapshot = (Vec<MenuEntry>, Vec<PageExtra>, Vec<Target>, Option<usize>);

/// Rola ovladaca. Bez `_`: pridanie noveho `Hit` vynuti rozhodnutie.
pub(crate) fn role_of(h: &Hit) -> i32 {
    match h {
        Hit::Nav(_) => ROLE_PAGETAB,
        Hit::Cat(_) => ROLE_OUTLINEBUTTON,
        Hit::Chip(_) => ROLE_LISTITEM,
        Hit::Show(_)
        | Hit::Src(_)
        | Hit::ArchiveEnable
        | Hit::ArchivePause
        | Hit::Transparent
        | Hit::TrayIcon
        | Hit::NotchOpt(_)
        | Hit::ToolTile(_)
        | Hit::GlassPanels
        | Hit::GlassBar
        | Hit::GlassOpacity(_)
        | Hit::OledToggle(_)
        | Hit::DispToggle(_) => ROLE_CHECKBUTTON,
        Hit::TextMode(_) | Hit::TextPick => ROLE_RADIOBUTTON,
        Hit::Hour(_)
        | Hit::Zero(_)
        | Hit::Seconds(_)
        | Hit::Weekday(_)
        | Hit::Date(_)
        | Hit::First(_)
        | Hit::Interval(_)
        | Hit::Provider(_)
        | Hit::Mode(_)
        | Hit::ArchiveRetention(_)
        | Hit::PeriphCount(_)
        | Hit::PeriphInterval(_)
        | Hit::ExWork(_)
        | Hit::ExBreak(_)
        | Hit::ExLong(_)
        | Hit::ExRounds(_)
        | Hit::ExSysInterval(_)
        | Hit::ExMicKey(_)
        | Hit::Lang(_)
        | Hit::MarqueeOn(_)
        | Hit::UpdAuto(_)
        | Hit::UpdIcon(_)
        | Hit::Wel(..)
        | Hit::MarqueeDir(_)
        | Hit::MarqueeSpeed(_)
        | Hit::MarqueePaused(_)
        | Hit::ThemeColor
        | Hit::Swatch(_)
        | Hit::DispMode(_)
        | Hit::DateProvider(_)
        | Hit::DateKind(_)
        | Hit::DateRepeat(_)
        | Hit::WeatherUse(_)
        | Hit::WeatherDevice => ROLE_RADIOBUTTON,
        Hit::OpacityTrack | Hit::SoundTrack => ROLE_SLIDER,
        Hit::SoundToggle
        | Hit::AudioQuick(_)
        | Hit::ToolsToggle(_)
        | Hit::ExSound
        | Hit::ExSys(_)
        | Hit::ExMicShow => ROLE_CHECKBUTTON,
        Hit::TransferSection(_) => ROLE_CHECKBUTTON,
        Hit::ToolsCycle(_)
        | Hit::ToolsClear(_)
        | Hit::ToolsAddFolder(_)
        | Hit::ToolsRemoveFolder(..)
        | Hit::ToolsAddApp
        | Hit::ToolsRemoveApp(_)
        | Hit::ToolsHotkey
        | Hit::ToolsHotkeyOff
        | Hit::ToolsHotkeyDefault
        | Hit::IcsAdd
        | Hit::IcsRemove(_)
        | Hit::TransferExport
        | Hit::TransferImport
        | Hit::UpdCheck
        | Hit::WelDone
        | Hit::TransferApply
        | Hit::TransferCancel
        | Hit::ResetLayout
        | Hit::AddSep(_)
        | Hit::WeatherPick(_)
        | Hit::WeatherRemove(_)
        | Hit::WeatherGeoSettings
        | Hit::RemoveSep(_)
        | Hit::Preset(_)
        | Hit::DisableAll
        | Hit::DeleteData
        | Hit::CancelDelete
        | Hit::CopyStatusline
        | Hit::CcConnect
        | Hit::CcReplace
        | Hit::CcKeep
        | Hit::CcDisconnect
        | Hit::CcTest
        | Hit::ArchiveDelete
        | Hit::ArchiveCancel
        | Hit::Pick
        | Hit::OpacityStep(_)
        | Hit::SoundStep(_)
        | Hit::SoundPreview
        | Hit::ResetAppearance
        | Hit::DispRemove(_)
        | Hit::DispIdentify
        | Hit::DateEvery(_)
        | Hit::DateAdd
        | Hit::DateCancel
        | Hit::DateEdit(_)
        | Hit::DateCal
        | Hit::DateCalMonth(_)
        | Hit::DateCalDay(_)
        | Hit::DateRemove(_) => ROLE_PUSHBUTTON,
    }
}

/// Nazov tlacidla pre citacku: symboly "-" / "+" a farebne polia nemaju zrozumitelny text, volby v skupine nesu nazov skupiny.
pub(crate) fn acc_label(hit: &Hit, text: &str, group: Option<&str>) -> String {
    match hit {
        Hit::OpacityStep(d) if *d < 0 => t("acc.opacity_dec"),
        Hit::OpacityStep(_) => t("acc.opacity_inc"),
        Hit::SoundStep(d) if *d < 0 => t("acc.sound_dec"),
        Hit::SoundStep(_) => t("acc.sound_inc"),
        Hit::DateEvery(d) if *d < 0 => t("acc.every_dec"),
        Hit::DateEvery(_) => t("acc.every_inc"),
        Hit::DateCal => t("dat.calendar"),
        Hit::DateCalMonth(d) if *d < 0 => t("dat.cal_prev"),
        Hit::DateCalMonth(_) => t("dat.cal_next"),
        Hit::Swatch(i) => appearance_page::presets()
            .get(*i)
            .map(|c| transom_config::format_hex(*c))
            .unwrap_or_default(),
        // formular rucnych datumov: skupiny "Poskytovatel" a pod. sa inak opakuju s volbou "Poskytovatel" v zdrojoch AI
        Hit::DateProvider(_) | Hit::DateKind(_) | Hit::DateRepeat(_) => {
            format!("{}: {}: {text}", t("dat.title"), group.unwrap_or_default())
        }
        _ => match group {
            Some(g) if !g.is_empty() => format!("{g}: {text}"),
            _ => text.to_string(),
        },
    }
}

impl Ui {
    /// Zachyti nazov ovladaca, ktory sa prave kresli (volaju miesta, kde sa pridava hit-box).
    pub(crate) fn tag(&mut self, hit: &Hit, name: impl Into<String>, checked: Option<bool>) {
        self.acc.push(AccMeta {
            hit: hit.clone(),
            name: name.into(),
            checked,
        });
    }

    /// Snimka pre citacku obrazovky po kresleni: polozky (nazov, obdlznik), roly / stavy / hodnoty / akcie, cielova akcia kazdej
    /// polozky a index polozky s fokusom klavesnice. `edits` = obdlzniky (v klientskych px) zobrazenych vstupnych poli
    /// a ich texty; `edit_focus` = pole, ktore ma skutocny fokus.
    pub(crate) fn a11y_snapshot(&self, edits: &[EditInfo], edit_focus: Option<usize>) -> Snapshot {
        let (mut entries, mut extras, mut targets) = (Vec::new(), Vec::new(), Vec::new());
        let vp = self.viewport;
        let mut seen: Vec<&Hit> = Vec::new();
        for (full, hit) in &self.all_hits {
            if seen.contains(&hit) {
                continue;
            }
            seen.push(hit);
            let Some(meta) = self.acc.iter().find(|m| &m.hit == hit) else {
                continue;
            };
            let shown = self.hits.iter().find(|(_, h)| h == hit).map(|(r, _)| *r);
            // polozka mimo viditelneho okna obsahu: poloha bez orezania a stav OFFSCREEN (navigacia je pevna)
            let (r, off) = match (shown, hit) {
                (Some(r), _) => (r, false),
                (None, _) if vp.h > 0 => (*full, true),
                (None, _) => (*full, false),
            };
            let role = role_of(hit);
            let on = meta.checked == Some(true);
            let mut state = if off { STATE_OFFSCREEN } else { 0 };
            match role {
                ROLE_CHECKBUTTON | ROLE_RADIOBUTTON if on => state |= STATE_CHECKED,
                ROLE_PAGETAB => {
                    state |= STATE_SELECTABLE;
                    if on {
                        state |= STATE_SELECTED;
                    }
                }
                ROLE_OUTLINEBUTTON => {
                    state |= if on { STATE_EXPANDED } else { STATE_COLLAPSED };
                }
                _ => {}
            }
            let action = match role {
                ROLE_CHECKBUTTON => t(if on { "acc.uncheck" } else { "acc.check" }),
                ROLE_RADIOBUTTON | ROLE_PAGETAB | ROLE_LISTITEM => t("acc.select"),
                ROLE_PUSHBUTTON => t("acc.press"),
                ROLE_OUTLINEBUTTON => t(if on { "acc.collapse" } else { "acc.expand" }),
                _ => String::new(), // posuvnik sa meni sipkami a tlacidlami +/-
            };
            let value = if *hit == Hit::OpacityTrack {
                format!("{} %", self.cfg.appearance.opacity_percent())
            } else if *hit == Hit::SoundTrack {
                format!("{} %", self.cfg.ui_sounds.volume)
            } else {
                String::new()
            };
            // zvolit polozku rozlozenia ma zmysel len ako presun (Enter na kruhu fokusu); cez akciu sa nespusta
            let action = if matches!(hit, Hit::Chip(_)) {
                String::new()
            } else {
                action
            };
            entries.push(MenuEntry {
                name: meta.name.clone(),
                separator: false,
                enabled: true,
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            });
            extras.push(PageExtra {
                role,
                state,
                value,
                action,
            });
            targets.push(Target::Hit(hit.clone()));
        }
        for (i, (x, y, w, h), text) in edits.iter().map(|(i, r, t)| (*i, *r, t.clone())) {
            let name = crate::edit_names::names()
                .get(i)
                .cloned()
                .unwrap_or_default();
            entries.push(MenuEntry {
                name,
                separator: false,
                enabled: true,
                x,
                y,
                w,
                h,
            });
            extras.push(PageExtra {
                role: ROLE_TEXT,
                state: 0,
                value: text,
                action: t("acc.focus"),
            });
            targets.push(Target::Edit(i));
        }
        // staticke texty (nadpis nahladu, zmeny, ignorovane, stavovy riadok): len na citanie; nazvy zo suboru su vycistene
        let mut statics: Vec<String> = self
            .acc_statics
            .iter()
            .map(|s| crate::transfer_page::a11y_text(s))
            .collect();
        if !self.status.trim().is_empty() {
            statics.push(crate::transfer_page::a11y_text(&self.status));
        }
        statics.truncate(MAX_STATIC_NODES);
        let vp_full = vp;
        for text in statics {
            entries.push(MenuEntry {
                name: text,
                separator: false,
                enabled: true,
                x: vp_full.x,
                y: vp_full.y,
                w: vp_full.w.max(1),
                h: vp_full.h.max(1),
            });
            extras.push(PageExtra {
                role: ROLE_STATICTEXT,
                state: 0,
                value: String::new(),
                action: String::new(),
            });
            targets.push(Target::Static);
        }
        let focus = match (edit_focus, &self.kfocus) {
            (Some(e), _) => targets.iter().position(|t| *t == Target::Edit(e)),
            (None, Some(h)) => targets.iter().position(|t| *t == Target::Hit(h.clone())),
            _ => None,
        };
        (entries, extras, targets, focus)
    }

    /// Nazov korena klientskej plochy (stranka v okne nastaveni).
    pub(crate) fn a11y_page_name(&self) -> String {
        let page = crate::PAGE_ORDER
            .iter()
            .find(|(p, _)| *p == self.page)
            .map(|(_, key)| t(&format!("nav.{key}")))
            .unwrap_or_default();
        format!("{} – {page}", t("set.title"))
    }
}
