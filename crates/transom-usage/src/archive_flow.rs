//! Archiv dennych suctov end-to-end (`collect`): **syntetické** logy (ziadne skutocne data) v docasnom adresari.
use super::*;
use std::path::{Path, PathBuf};

fn utc(ts: i64) -> Ymd {
    transom_core::clock::civil_from_days(ts.div_euclid(86_400))
}

struct Env {
    root: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("transom-flow-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("claude/projects/p")).unwrap();
        Env { root }
    }

    /// Kazdy riadok = jedna odpoved Claude Code: (casova znacka, id odpovede, tokeny vstupu).
    fn log(&self, file: &str, lines: &[(&str, &str, u64)]) {
        let text: String = lines
            .iter()
            .map(|(ts, id, inp)| {
                format!(
                    "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"requestId\":\"r-{id}\",\"message\":{{\"id\":\"m-{id}\",\"model\":\"claude-opus-5-5\",\"usage\":{{\"input_tokens\":{inp},\"output_tokens\":10}}}}}}\n"
                )
            })
            .collect();
        std::fs::write(self.root.join("claude/projects/p").join(file), text).unwrap();
    }

    fn rm(&self, file: &str) {
        std::fs::remove_file(self.root.join("claude/projects/p").join(file)).unwrap();
    }

    fn archive_path(&self) -> PathBuf {
        self.root.join("usage-archive.json")
    }

    fn collect(&self, write: bool, retention: u32, now: &str) -> Collected {
        let src = Sources {
            claude_dir: Some(self.root.join("claude")),
            archive: Some(ArchiveSource {
                path: self.archive_path(),
                write,
                retention_months: retention,
            }),
            ..Default::default()
        };
        collect(&src, parse_iso_utc(now).unwrap(), &utc)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn tokens(c: &Collected, day: (i32, u32, u32)) -> u64 {
    c.days
        .iter()
        .filter(|d| d.day == day)
        .map(|d| d.input + d.output)
        .sum()
}

fn file_bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

#[test]
fn archive_keeps_days_after_the_provider_logs_are_deleted_and_marks_them_archived() {
    let e = Env::new("keep");
    // dva subory: starsi (2. a 3. 11.) a novsi (10. a 11. 11.)
    e.log(
        "old.jsonl",
        &[
            ("2026-11-02T10:00:00Z", "a", 100),
            ("2026-11-03T10:00:00Z", "b", 200),
        ],
    );
    e.log(
        "new.jsonl",
        &[
            ("2026-11-10T10:00:00Z", "c", 300),
            ("2026-11-11T10:00:00Z", "d", 400),
        ],
    );
    let c1 = e.collect(true, 0, "2026-11-11T12:00:00Z");
    assert_eq!(c1.archive, ArchiveStatus::Writing);
    assert_eq!(
        (tokens(&c1, (2026, 11, 3)), tokens(&c1, (2026, 11, 11))),
        (210, 410)
    );
    assert!(
        c1.days.iter().all(|d| !d.archived),
        "vsetko je citatelne: nic nie je archivovane"
    );
    // 2. 11. je hranicny den (najstarsi zaznam): neuplny
    assert!(
        c1.days
            .iter()
            .find(|d| d.day == (2026, 11, 2))
            .unwrap()
            .partial
    );
    assert!(
        !c1.days
            .iter()
            .find(|d| d.day == (2026, 11, 3))
            .unwrap()
            .partial
    );
    // Claude Code zmazal stary subor: zdroj uz nema 2. a 3. 11., archiv ich drzi
    e.rm("old.jsonl");
    let c2 = e.collect(true, 0, "2026-11-12T12:00:00Z");
    assert_eq!(
        (tokens(&c2, (2026, 11, 2)), tokens(&c2, (2026, 11, 3))),
        (110, 210),
        "sucty prezili zmazanie logov"
    );
    for day in [(2026, 11, 2), (2026, 11, 3)] {
        let d = c2.days.iter().find(|d| d.day == day).unwrap();
        assert!(d.archived, "{day:?} je z archivu, nie z citatelnych logov");
        assert!(!d.price_date.is_empty(), "povod: datum cennika");
    }
    assert!(c2
        .days
        .iter()
        .filter(|d| d.day >= (2026, 11, 10))
        .all(|d| !d.archived));
    // pokrytie: 2. 11. - 12. 11. suvisle (aj dni bez zaznamov su skutocne nuly)
    assert_eq!(
        c2.cover,
        vec![archive::Cover {
            provider: Provider::ClaudeCode,
            from: (2026, 11, 2),
            to: (2026, 11, 12)
        }]
    );
}

#[test]
fn repeated_collects_write_the_same_file_and_never_double_count() {
    let e = Env::new("idem");
    e.log(
        "a.jsonl",
        &[
            ("2026-11-02T10:00:00Z", "a", 100),
            ("2026-11-05T10:00:00Z", "b", 200),
        ],
    );
    let c1 = e.collect(true, 0, "2026-11-06T12:00:00Z");
    let f1 = file_bytes(&e.archive_path());
    for _ in 0..4 {
        let c = e.collect(true, 0, "2026-11-06T12:00:00Z");
        assert_eq!(c.days, c1.days);
    }
    assert_eq!(
        file_bytes(&e.archive_path()),
        f1,
        "nezmeneny obsah = subor sa neprepisuje"
    );
    assert_eq!(tokens(&c1, (2026, 11, 5)), 210);
}

#[test]
fn source_corrections_replace_archived_totals_for_fully_readable_days() {
    let e = Env::new("fix");
    // 5. 11. sa najprv cita s dvoma odpovedami; neskor je jedna z nich odstranena zo suboru
    e.log(
        "a.jsonl",
        &[
            ("2026-11-02T10:00:00Z", "z", 1),
            ("2026-11-05T10:00:00Z", "b", 200),
            ("2026-11-05T11:00:00Z", "c", 300),
        ],
    );
    let c1 = e.collect(true, 0, "2026-11-06T12:00:00Z");
    assert_eq!(tokens(&c1, (2026, 11, 5)), 520);
    e.log(
        "a.jsonl",
        &[
            ("2026-11-02T10:00:00Z", "z", 1),
            ("2026-11-05T10:00:00Z", "b", 200),
        ],
    );
    let c2 = e.collect(true, 0, "2026-11-06T12:00:00Z");
    assert_eq!(
        tokens(&c2, (2026, 11, 5)),
        210,
        "archiv sa opravil po oprave zdroja"
    );
}

#[test]
fn paused_archive_is_read_but_never_written() {
    let e = Env::new("pause");
    e.log(
        "a.jsonl",
        &[
            ("2026-11-02T10:00:00Z", "a", 100),
            ("2026-11-03T10:00:00Z", "b", 200),
        ],
    );
    e.collect(true, 0, "2026-11-04T12:00:00Z");
    let before = file_bytes(&e.archive_path());
    // novy den s datami pocas pozastavenia
    e.log(
        "a.jsonl",
        &[
            ("2026-11-02T10:00:00Z", "a", 100),
            ("2026-11-03T10:00:00Z", "b", 200),
            ("2026-11-04T10:00:00Z", "c", 999),
        ],
    );
    let c = e.collect(false, 0, "2026-11-04T12:00:00Z");
    assert_eq!(c.archive, ArchiveStatus::ReadOnly);
    assert_eq!(
        file_bytes(&e.archive_path()),
        before,
        "pozastavene: nic sa nezapisalo"
    );
    assert_eq!(
        tokens(&c, (2026, 11, 4)),
        1009,
        "aktualne citatelne data sa stale zobrazuju"
    );
    // po zmazani logov dni z archivu ostavaju zobrazene aj pocas pozastavenia
    e.rm("a.jsonl");
    let c = e.collect(false, 0, "2026-11-20T12:00:00Z");
    assert_eq!(tokens(&c, (2026, 11, 3)), 210);
    assert!(c.days.iter().all(|d| d.archived));
    assert_eq!(
        tokens(&c, (2026, 11, 4)),
        0,
        "den, ktory sa nikdy neulozil, sa nevymysla"
    );
}

#[test]
fn disabled_archive_without_a_file_changes_nothing_and_deletion_removes_archived_days() {
    let e = Env::new("off");
    e.log(
        "a.jsonl",
        &[
            ("2026-11-02T10:00:00Z", "a", 100),
            ("2026-11-03T10:00:00Z", "b", 200),
        ],
    );
    let c = e.collect(false, 0, "2026-11-04T12:00:00Z");
    assert_eq!(c.archive, ArchiveStatus::Off);
    assert!(
        !e.archive_path().exists(),
        "bez zapnutia sa archiv nikdy nevytvori"
    );
    assert!(c.cover.is_empty() && c.days.iter().all(|d| !d.archived));
    // zapnute -> archiv; logy zmazane; potom zmazanie archivu: dni zmiznu a nevrati sa ani nula
    e.collect(true, 0, "2026-11-04T12:00:00Z");
    e.rm("a.jsonl");
    assert_eq!(
        tokens(&e.collect(false, 0, "2026-11-05T12:00:00Z"), (2026, 11, 3)),
        210
    );
    assert!(archive::delete(&e.archive_path()).unwrap());
    let gone = e.collect(false, 0, "2026-11-05T12:00:00Z");
    assert_eq!(gone.archive, ArchiveStatus::Off);
    assert!(gone.days.is_empty() && gone.cover.is_empty());
}

#[test]
fn retention_drops_old_archived_days_when_writing() {
    let e = Env::new("ret");
    e.log(
        "a.jsonl",
        &[
            ("2026-01-10T10:00:00Z", "old", 100),
            ("2026-01-11T10:00:00Z", "o2", 100),
            ("2026-11-02T10:00:00Z", "new", 200),
        ],
    );
    let c = e.collect(true, 0, "2026-11-03T12:00:00Z");
    assert_eq!(tokens(&c, (2026, 1, 11)), 110);
    // 3 mesiace: januar sa zmaze z archivu aj z pohladu
    let c = e.collect(true, 3, "2026-11-03T12:00:00Z");
    assert_eq!(tokens(&c, (2026, 1, 11)), 0);
    assert_eq!(tokens(&c, (2026, 11, 2)), 210);
    let a = match archive::load(&e.archive_path()) {
        archive::Loaded::Ok(a) => a,
        other => panic!("{other:?}"),
    };
    assert!(a.rows.iter().all(|r| r.day >= (2026, 8, 3)));
}

#[test]
fn unreadable_archive_is_never_overwritten_and_not_used() {
    let e = Env::new("bad");
    e.log("a.jsonl", &[("2026-11-02T10:00:00Z", "a", 100)]);
    std::fs::write(e.archive_path(), b"{ nonsense").unwrap();
    let c = e.collect(true, 0, "2026-11-03T12:00:00Z");
    assert_eq!(c.archive, ArchiveStatus::Unreadable);
    assert_eq!(std::fs::read(e.archive_path()).unwrap(), b"{ nonsense");
    assert_eq!(
        tokens(&c, (2026, 11, 2)),
        110,
        "citatelne data sa zobrazuju normalne"
    );
}

#[test]
fn day_without_any_source_data_is_not_turned_into_zero_outside_the_coverage() {
    let e = Env::new("gap");
    e.log("a.jsonl", &[("2026-11-02T10:00:00Z", "a", 100)]);
    e.collect(true, 0, "2026-11-03T12:00:00Z");
    e.rm("a.jsonl");
    // dlho nebezala ziadna obnova; nova relacia az 20. 12. (zdroj cita od 20. 12.)
    e.log("b.jsonl", &[("2026-12-20T10:00:00Z", "b", 50)]);
    let c = e.collect(true, 0, "2026-12-21T12:00:00Z");
    assert_eq!(c.cover.len(), 2, "dve oddelene pokrytia: {:?}", c.cover);
    assert_eq!(c.cover[0].to, (2026, 11, 3));
    assert_eq!(c.cover[1].from, (2026, 12, 20));
}
