//! Most medzi sluzbou nastrojov (`transom_tools::service`) a panelom Nastroje: cisty prevod snimky na riadky a prikazov panela na
//! prikazy sluzby. Bez okien a bez I/O, takze sa vsetko testuje offscreen. Obsah riadkov (nazvy, text) sa nikdy nelogue.
use crate::tools_pop::{Row, Section, SectionView, Thumb, ToolsCmd, ToolsData};
use transom_i18n::{t, tf};
use transom_tools::clipboard::{ClipKind, Payload};
use transom_tools::service::{Cmd, Msg, Snapshot};
use transom_tools::shelf::{RejectReason, State};

/// Velkost v ludskej podobe (binarne jednotky, jedno desatinne miesto od MB vyssie).
pub fn size_text(b: u64) -> String {
    const KB: f64 = 1024.0;
    let f = b as f64;
    if b < 1024 {
        format!("{b} B")
    } else if f < KB * KB {
        format!("{:.0} KB", f / KB)
    } else if f < KB * KB * KB {
        format!("{:.1} MB", f / (KB * KB))
    } else {
        format!("{:.1} GB", f / (KB * KB * KB))
    }
}

/// Politika citania schranky z konfiguracie historie: vypnute druhy sa nenacitaju a obrazok sa obmedzi uz podla hlavicky.
pub(crate) fn read_policy(
    c: &transom_config::ClipboardCfg,
) -> transom_platform::clipboard::ReadPolicy {
    transom_platform::clipboard::ReadPolicy {
        text: c.text,
        images: c.images,
        files: c.files,
        max_side: transom_tools::clipboard::MAX_IMAGE_SIDE,
    }
}

pub(crate) fn msg_text(m: &Msg) -> Option<String> {
    Some(match m {
        Msg::Added {
            added,
            duplicates,
            rejected,
            ..
        } => {
            let mut s = if *duplicates > 0 {
                tf(
                    "tools.msg_added_dup",
                    &[&added.to_string(), &duplicates.to_string()],
                )
            } else {
                tf("tools.msg_added", &[&added.to_string()])
            };
            if *rejected > 0 {
                s.push(' ');
                s.push_str(&tf("tools.msg_rejected", &[&rejected.to_string()]));
            }
            s
        }
        Msg::Rejected(RejectReason::ReadOnly) => t("tools.unusable"),
        Msg::Rejected(RejectReason::Network) => t("tools.net_copy"),
        Msg::Moved { pos, total } => tf("tools.moved", &[&pos.to_string(), &total.to_string()]),
        Msg::MoveEdge { first: true } => t("tools.move_first"),
        Msg::MoveEdge { first: false } => t("tools.move_last"),
        Msg::Rejected(_) => tf("tools.msg_rejected", &["1"]),
        Msg::Cleared(n) => tf("tools.msg_cleared", &[&n.to_string()]),
        Msg::StorageUnusable => t("tools.unusable"),
        Msg::NoteError(_) => t("tools.note_err"),
    })
}

/// Zobrazenie vsetkych styroch sekcii z poslednej snimky sluzby.
pub fn view(s: &Snapshot) -> ToolsData {
    let mut d = ToolsData::default();
    let unusable = s.unusable.then(|| t("tools.unusable"));
    d.sections[0] = SectionView {
        rows: s
            .shelf
            .iter()
            .map(|r| Row {
                id: r.id.clone(),
                title: r.name.clone(),
                subtitle: format!(
                    "{} · {}{}",
                    if r.copy {
                        t("tools.stored_copy")
                    } else {
                        t("tools.reference")
                    },
                    r.location,
                    // sietova cesta sa nesonduje: nie je "chybajuca", len nekontrolovana
                    if r.state == State::Unchecked && r.remote {
                        format!(" · {}", t("tools.net_unchecked"))
                    } else {
                        String::new()
                    }
                ),
                value: if r.dir {
                    String::new()
                } else {
                    size_text(r.size)
                },
                pinned: r.pinned,
                unavailable: matches!(r.state, State::Missing | State::Offline),
                thumb: Thumb::None,
            })
            .collect(),
        enabled: true,
        empty: None,
        status: s
            .msg
            .as_ref()
            .filter(|m| {
                matches!(
                    m,
                    Msg::Added { .. }
                        | Msg::Rejected(_)
                        | Msg::Cleared(_)
                        | Msg::StorageUnusable
                        | Msg::Moved { .. }
                        | Msg::MoveEdge { .. }
                )
            })
            .and_then(msg_text)
            .or_else(|| unusable.clone()),
    };
    d.sections[1] = SectionView {
        rows: s
            .clipboard
            .iter()
            .map(|r| Row {
                id: r.id.clone(),
                title: match r.kind {
                    ClipKind::Text => r.preview.clone(),
                    ClipKind::Image => r.preview.clone(),
                    ClipKind::Files => r.preview.clone(),
                },
                subtitle: r.app.clone(),
                value: size_text(r.size),
                pinned: r.pinned,
                unavailable: false,
                thumb: Thumb::None,
            })
            .collect(),
        enabled: s.clip_enabled,
        empty: None,
        status: (s.clip_enabled && s.clip_paused).then(|| t("tools.paused")),
    };
    d.sections[2] = SectionView {
        rows: s
            .notes
            .iter()
            .map(|n| Row {
                id: n.id.clone(),
                title: n.title.clone(),
                subtitle: String::new(),
                value: String::new(),
                pinned: n.pinned,
                unavailable: false,
                thumb: Thumb::None,
            })
            .collect(),
        enabled: true,
        empty: None,
        status: s
            .msg
            .as_ref()
            .filter(|m| matches!(m, Msg::NoteError(_)))
            .and_then(msg_text)
            .or_else(|| unusable.clone()),
    };
    // sekcia Snimky (kniznica) sa sklada pri kazdom kresleni z `library_rt`, nie zo snimky sluzby
    d.sections[3] = SectionView::default();
    d
}

/// Snimka schranky z platformy -> vstup rozhodovacej logiky ulozisk (iba presun poloziek).
pub fn to_tools_snapshot(
    s: transom_platform::clipboard::ClipSnapshot,
) -> transom_tools::clipboard::Snapshot {
    transom_tools::clipboard::Snapshot {
        formats: s.formats,
        can_include_history: s.can_include_history,
        can_upload: s.can_upload,
        owner_exe: s.owner_exe,
        own_write: s.own_write,
        text: s.text,
        image_png: s.image_png,
        image_dims: s.image_dims,
        files: s.files,
    }
}

/// Prikaz panela -> prikaz sluzby (nic nevykonava; `None` = nic robit netreba alebo ide o inu cestu).
pub fn plan(cmd: ToolsCmd, s: &Snapshot) -> Option<Cmd> {
    match cmd {
        // editor poznamky a tahanie ma na starosti aplikacia
        ToolsCmd::Search
        | ToolsCmd::DragOut(..)
        | ToolsCmd::ShelfAdd
        | ToolsCmd::NoteNew
        | ToolsCmd::EditorSave
        | ToolsCmd::EditorCopy
        | ToolsCmd::EditorDiscard
        // snimky su iba na citanie a ovlada ich aplikacia (otvorenie, priecinok, obnovenie)
        | ToolsCmd::ShowInFolder(..)
        | ToolsCmd::Refresh
        // dlazdice skupiny otvaraju ine panely / akcie (aplikacia)
        | ToolsCmd::Tile(_)
        | ToolsCmd::Home => None,
        ToolsCmd::Move(Section::Shelf, i, d) => Some(Cmd::ShelfMove {
            id: s.shelf.get(i)?.id.clone(),
            delta: d,
        }),
        ToolsCmd::Move(..) => None,
        ToolsCmd::Activate(sec, i) => match sec {
            Section::Shelf => {
                let r = s.shelf.get(i)?;
                // kopirovanie cesty do schranky nic nepristupuje k suboru: funguje aj pre sietovu cestu, ktora sa nekontroluje
                matches!(r.state, State::Available | State::Unchecked)
                    .then(|| Cmd::Put(Payload::Files(vec![r.path.clone()])))
            }
            Section::Clipboard => Some(Cmd::ClipRestore(s.clipboard.get(i)?.id.clone())),
            Section::Notes => Some(Cmd::NoteOpen(s.notes.get(i)?.id.clone())),
            Section::Shots => None,
        },
        ToolsCmd::TogglePin(sec, i) => match sec {
            Section::Shelf => Some(Cmd::ShelfPin(s.shelf.get(i)?.id.clone())),
            Section::Clipboard => Some(Cmd::ClipPin(s.clipboard.get(i)?.id.clone())),
            Section::Notes => Some(Cmd::NotePin(s.notes.get(i)?.id.clone())),
            Section::Shots => None,
        },
        ToolsCmd::Delete(sec, i) => match sec {
            Section::Shelf => Some(Cmd::ShelfRemove(s.shelf.get(i)?.id.clone())),
            Section::Clipboard => Some(Cmd::ClipDelete(s.clipboard.get(i)?.id.clone())),
            Section::Notes => Some(Cmd::NoteDelete(s.notes.get(i)?.id.clone())),
            // snimky su cudzie subory: kniznica nic nemaze
            Section::Shots => None,
        },
    }
}

/// Je riadok Police sietova polozka (netaha sa von)?
pub fn is_remote_row(sec: Section, i: usize, s: &Snapshot) -> bool {
    sec == Section::Shelf && s.shelf.get(i).is_some_and(|r| r.remote)
}

/// Cesty na tahanie von pre riadok Police (snimky tahaju cesty z kniznice v aplikacii; len dostupne polozky).
pub fn drag_paths(sec: Section, i: usize, s: &Snapshot) -> Vec<String> {
    match sec {
        Section::Shelf => s
            .shelf
            .get(i)
            .filter(|r| r.state == State::Available && !r.remote)
            .map(|r| vec![r.path.clone()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_tools::service::{ClipRow, NoteRow, ShelfRow};

    #[test]
    fn the_clipboard_read_policy_follows_the_configuration_and_never_widens_it() {
        let mut c = transom_config::Config::default().tools.clipboard;
        c.text = false;
        c.images = false;
        c.files = true;
        let p = read_policy(&c);
        assert!(!p.text && !p.images && p.files);
        assert_eq!(p.max_side, transom_tools::clipboard::MAX_IMAGE_SIDE);
        c.images = true;
        assert!(read_policy(&c).images);
    }

    #[test]
    fn moving_a_shelf_row_maps_to_the_item_id_and_the_result_is_announced() {
        let _g = crate::test_lang::guard();
        let sn = snap();
        // riadky nesu id polozky (selekcia a potvrdenie mazania sa riadia id, nie indexom)
        let v = view(&sn);
        assert_eq!(v.sections[0].rows[0].id, "s1");
        assert_eq!(v.sections[0].rows[1].id, "s2");
        match plan(ToolsCmd::Move(Section::Shelf, 1, -1), &sn) {
            Some(Cmd::ShelfMove { id, delta }) => assert_eq!((id.as_str(), delta), ("s2", -1)),
            _ => panic!("ocakavalo sa ShelfMove"),
        }
        assert!(plan(ToolsCmd::Move(Section::Shelf, 99, 1), &sn).is_none());
        assert!(plan(ToolsCmd::Move(Section::Notes, 0, 1), &sn).is_none());
        assert!(
            plan(ToolsCmd::ShelfAdd, &sn).is_none(),
            "dialog riesi aplikacia"
        );
        assert_eq!(
            msg_text(&Msg::Moved { pos: 3, total: 7 }),
            Some("Moved to position 3 of 7.".to_string())
        );
        assert_eq!(
            msg_text(&Msg::MoveEdge { first: true }),
            Some(t("tools.move_first"))
        );
        assert_eq!(
            msg_text(&Msg::MoveEdge { first: false }),
            Some(t("tools.move_last"))
        );
        // hlaska o presune sa ukaze v stavovom riadku Police
        let mut with_msg = snap();
        with_msg.msg = Some(Msg::Moved { pos: 1, total: 2 });
        assert!(view(&with_msg).sections[0].status.is_some());
    }

    #[test]
    fn a_network_item_is_shown_as_not_checked_can_copy_its_path_but_is_never_dragged_out() {
        let _g = crate::test_lang::guard();
        let mut sn = Snapshot::default();
        sn.shelf.push(ShelfRow {
            id: "n1".into(),
            name: "report.xlsx".into(),
            location: r"\\srv\share".into(),
            size: 0,
            copy: false,
            dir: false,
            pinned: false,
            state: State::Unchecked,
            remote: true,
            path: r"\\srv\share\report.xlsx".into(),
        });
        let v = view(&sn);
        let row = &v.sections[0].rows[0];
        assert!(!row.unavailable, "nekontrolovana nie je chybajuca");
        assert!(
            row.subtitle.contains(&t("tools.net_unchecked")),
            "{}",
            row.subtitle
        );
        // kopirovanie cesty do schranky nic nepristupuje k suboru
        assert!(matches!(
            plan(ToolsCmd::Activate(Section::Shelf, 0), &sn),
            Some(Cmd::Put(Payload::Files(_)))
        ));
        // tahanie von by volalo shell (SHParseDisplayName) a mohlo by zablokovat UI: nic sa netaha a pouzivatel dostane vysvetlenie
        assert!(drag_paths(Section::Shelf, 0, &sn).is_empty());
        assert!(is_remote_row(Section::Shelf, 0, &sn));
        assert!(!is_remote_row(Section::Shelf, 1, &sn) && !is_remote_row(Section::Notes, 0, &sn));
        assert_eq!(
            msg_text(&Msg::Rejected(RejectReason::Network)),
            Some(t("tools.net_copy"))
        );
        assert!(!t("tools.net_drag").starts_with("tools."));
    }

    fn snap() -> Snapshot {
        Snapshot {
            shelf: vec![
                ShelfRow {
                    id: "s1".into(),
                    name: "a.txt".into(),
                    location: r"C:\x".into(),
                    size: 2048,
                    copy: false,
                    dir: false,
                    pinned: true,
                    state: State::Available,
                    remote: false,
                    path: r"C:\x\a.txt".into(),
                },
                ShelfRow {
                    id: "s2".into(),
                    name: "gone.txt".into(),
                    location: r"C:\y".into(),
                    size: 0,
                    copy: true,
                    dir: false,
                    pinned: false,
                    state: State::Missing,
                    remote: false,
                    path: r"C:\y\gone.txt".into(),
                },
            ],
            clipboard: vec![ClipRow {
                id: "c1".into(),
                kind: ClipKind::Text,
                preview: "ahoj".into(),
                app: "notepad.exe".into(),
                size: 4,
                pinned: false,
                added: 1,
            }],
            notes: vec![NoteRow {
                id: "n1".into(),
                title: "Nakup".into(),
                pinned: false,
                modified: 1,
            }],
            clip_enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn sizes_are_human_readable_and_rows_mirror_the_snapshot() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(1023), "1023 B");
        assert_eq!(size_text(2048), "2 KB");
        assert_eq!(size_text(1_500_000), "1.4 MB");
        assert_eq!(size_text(3 * 1024 * 1024 * 1024), "3.0 GB");
        let _g = crate::test_lang::guard();
        let v = view(&snap());
        assert_eq!(v.sections[0].rows.len(), 2);
        assert!(v.sections[0].rows[0].pinned && !v.sections[0].rows[0].unavailable);
        assert!(
            v.sections[0].rows[1].unavailable,
            "chybajuci zdroj sa len ukaze"
        );
        assert_eq!(v.sections[1].rows[0].title, "ahoj");
        assert!(v.sections[1].enabled);
        assert_eq!(v.sections[2].rows[0].title, "Nakup");
        assert!(
            v.sections[3].rows.is_empty(),
            "snimky sa skladaju z kniznice pri kresleni, nie zo snimky sluzby"
        );
    }

    #[test]
    fn paused_clipboard_unusable_storage_and_messages_are_explained() {
        let _g = crate::test_lang::guard();
        let mut s = snap();
        s.clip_paused = true;
        s.unusable = true;
        s.msg = Some(Msg::Added {
            added: 2,
            duplicates: 1,
            rejected: 1,
            truncated: false,
        });
        let v = view(&s);
        assert!(v.sections[1].status.is_some(), "pozastavenie je viditelne");
        assert!(
            v.sections[2].status.is_some(),
            "poskodene ulozisko sa povie"
        );
        let shelf = v.sections[0].status.clone().unwrap();
        assert!(shelf.contains('2') && shelf.contains('1'), "{shelf}");
        s.clip_enabled = false;
        assert!(
            view(&s).sections[1].status.is_none(),
            "vypnuta schranka nehlasi pozastavenie"
        );
    }

    #[test]
    fn commands_map_to_the_right_store_item_and_out_of_range_or_missing_ones_do_nothing() {
        let s = snap();
        assert!(
            matches!(plan(ToolsCmd::Activate(Section::Shelf, 0), &s), Some(Cmd::Put(Payload::Files(f))) if f == vec![r"C:\x\a.txt"])
        );
        assert!(
            plan(ToolsCmd::Activate(Section::Shelf, 1), &s).is_none(),
            "chybajuci subor sa nekopiruje"
        );
        assert!(plan(ToolsCmd::Activate(Section::Shelf, 9), &s).is_none());
        assert!(
            matches!(plan(ToolsCmd::Activate(Section::Clipboard, 0), &s), Some(Cmd::ClipRestore(id)) if id == "c1")
        );
        assert!(
            matches!(plan(ToolsCmd::Activate(Section::Notes, 0), &s), Some(Cmd::NoteOpen(id)) if id == "n1")
        );
        assert!(
            matches!(plan(ToolsCmd::TogglePin(Section::Shelf, 0), &s), Some(Cmd::ShelfPin(id)) if id == "s1")
        );
        assert!(plan(ToolsCmd::TogglePin(Section::Shots, 0), &s).is_none());
        // snimky su cudzie subory: nic sa nepripina, nemaze ani nekopiruje cez sluzbu
        assert!(plan(ToolsCmd::Delete(Section::Shots, 0), &s).is_none());
        assert!(plan(ToolsCmd::Activate(Section::Shots, 0), &s).is_none());
        assert!(plan(ToolsCmd::Delete(Section::Notes, 5), &s).is_none());
        assert!(plan(ToolsCmd::Search, &s).is_none());
        assert_eq!(
            drag_paths(Section::Shelf, 0, &s),
            vec![r"C:\x\a.txt".to_string()]
        );
        assert!(
            drag_paths(Section::Shelf, 1, &s).is_empty(),
            "chybajuca polozka sa neda tahat"
        );
        assert!(drag_paths(Section::Clipboard, 0, &s).is_empty());
        assert!(drag_paths(Section::Shots, 0, &s).is_empty());
    }
}
