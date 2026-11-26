//! Volitelny lokalny **archiv dennych suctov** (`usage-archive.json` v lokalnych datach transom), aby mesacna historia
//! prezila zmazanie surovych logov poskytovatelov (Claude Code maze prepisy po `cleanupPeriodDays`, predvolene 30 dni).
//!
//! **Co sa uklada:** len denny sucet podla poskytovatela a modelu - tokeny (vstup, vystup, citanie / zapis cache), pocet volani,
//! odhad ceny v USD (API-ekvivalent, nie faktura) + pocet volani bez ceny, **pôvod** (poskytovatel, den, model, typ zdroja,
//! datum cennika, priznak "neuplny den"). **Nikdy** prompty, odpovede, nazvy konverzacii, projekty, cesty ani surove logy.
//!
//! **Zlucovanie je idempotentne a opravuje minulost** (`merge`): pre kazdy den, ktory zdroj *teraz* cele cita (den po
//! najstarsom zazname a bez chyb citania), je cerstve citanie **autoritou** a nahradi archivovane riadky toho dna (opakovane
//! skeny nic nepripocitavaju; ak sa zaznamy neskor opravili / deduplikovali, archiv sa opravi). **Hranicny den** (najstarsi
//! zaznam; zdroj ho mohol uz casovo orezat) a dni pri chybach citania sa nikdy nezmensia: pri kazdom modeli ostava vacsi
//! sucet a den je oznaceny ako **neuplny**. Dni, ktore zdroj uz nema, ostavaju nedotknute (o to ide).
//!
//! **Chybajuce udaje sa nemenia na nulu:** archiv eviduje **pokrytie** (suvisle rozsahy dni, ktore sa uspesne naskenovali,
//! `[najstarsi zaznam, dnes]`); den mimo pokrytia je "bez historie", den v pokryti bez riadkov je skutocna nula.
//! **Retencia** (3 / 6 / 12 / 24 mesiacov alebo bez obmedzenia) maze starsie riadky aj pokrytie.
use crate::{DayTotals, Provider, SourceKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use transom_core::clock::{add_months, civil_from_days, days_from_civil, days_in_month, Ymd};

/// Verzia schemy suboru archivu.
pub const ARCHIVE_VERSION: u32 = 1;

/// Den ako `(rok, mesiac, den)` (rovnaky tvar ako `DayTotals::day`).
pub type Day = (i32, u32, u32);

fn ymd(d: Day) -> Ymd {
    Ymd::new(d.0, d.1, d.2)
}

fn tup(d: Ymd) -> Day {
    (d.y, d.m, d.d)
}

fn ord(d: Day) -> i64 {
    days_from_civil(ymd(d))
}

/// Jeden archivovany riadok: den x poskytovatel x model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub provider: Provider,
    pub model: String,
    pub day: Day,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub requests: u64,
    /// Odhad ceny (USD) z cennika platneho v case zapisu riadku.
    pub est_usd: f64,
    /// Volania bez ceny (neznamy model, nestandardna rychlost, dlhy kontext).
    pub unpriced: u64,
    /// Typ zdroja, z ktoreho riadok pochadza (lokalne prepisy Claude Code / relacie Codex).
    pub source: SourceKind,
    /// Datum cennika, z ktoreho je `est_usd` (napr. `2026-11-15`).
    pub price_date: String,
    /// Den mohol byt neuplny (hranicny den zdroja, chyby citania): sucet je dolny odhad.
    pub partial: bool,
}

impl Row {
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    fn from_day(d: &DayTotals, source: SourceKind, price_date: &str, partial: bool) -> Option<Row> {
        Some(Row {
            provider: d.provider?,
            model: d.model.clone(),
            day: d.day,
            input: d.input,
            output: d.output,
            cache_read: d.cache_read,
            cache_write: d.cache_write,
            requests: d.requests,
            est_usd: d.est_usd,
            unpriced: d.unpriced,
            source,
            price_date: price_date.to_string(),
            partial,
        })
    }

    /// Riadok ako `DayTotals` pre zvysok aplikacie (s pôvodom).
    pub fn to_day(&self, archived: bool) -> DayTotals {
        DayTotals {
            provider: Some(self.provider),
            model: self.model.clone(),
            day: self.day,
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            requests: self.requests,
            est_usd: self.est_usd,
            unpriced: self.unpriced,
            archived,
            partial: self.partial,
            price_date: self.price_date.clone(),
        }
    }
}

/// Suvisly rozsah dni, ktory sa uspesne naskenoval (vratane oboch koncov).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cover {
    pub provider: Provider,
    pub from: Day,
    pub to: Day,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Archive {
    pub version: u32,
    /// Zoradene podla (poskytovatel, model, den) - rovnaky stav = rovnaky subor (idempotentne).
    pub rows: Vec<Row>,
    pub cover: Vec<Cover>,
    /// Cas posledneho zapisu (Unix sekundy).
    pub updated_at: i64,
}

impl Default for Archive {
    fn default() -> Self {
        Archive {
            version: ARCHIVE_VERSION,
            rows: Vec::new(),
            cover: Vec::new(),
            updated_at: 0,
        }
    }
}

/// Subor archivu v lokalnych datach transom (`TRANSOM_DATA` ho presmeruje pre testy).
pub fn path() -> PathBuf {
    crate::cache::data_dir().join("usage-archive.json")
}

/// Vysledok nacitania.
#[derive(Debug, PartialEq)]
pub enum Loaded {
    Missing,
    Ok(Archive),
    /// Subor je poskodeny alebo ma novsiu schemu - **nikdy sa neprepise** (aby sa nestratili data).
    Unreadable,
}

pub fn load(path: &Path) -> Loaded {
    let Ok(bytes) = std::fs::read(path) else {
        return Loaded::Missing;
    };
    match serde_json::from_slice::<Archive>(&bytes) {
        Ok(a) if a.version <= ARCHIVE_VERSION => Loaded::Ok(a),
        _ => Loaded::Unreadable,
    }
}

/// Atomicky zapis (docasny subor + premenovanie).
pub fn save(path: &Path, a: &Archive) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    crate::cache::write_atomic(path, &serde_json::to_vec(a).map_err(std::io::Error::other)?)
}

/// Odtlacok suboru archivu (existencia, velkost, cas zmeny): zber si ho zapamata pred nacitanim a pred zapisom overi, ci sa
/// subor medzitym nezmenil zvonka (zmazanie alebo retencia z nastaveni).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    exists: bool,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

pub fn stamp(path: &Path) -> Stamp {
    match std::fs::metadata(path) {
        Ok(m) => Stamp {
            exists: true,
            len: m.len(),
            modified: m.modified().ok(),
        },
        Err(_) => Stamp {
            exists: false,
            len: 0,
            modified: None,
        },
    }
}

/// Zapise archiv, len ak sa subor od `before` nezmenil (zber, ktory bezal pocas "Zmazat archiv" alebo skratenia retencie,
/// by ho inak vzkriesil / vratil dlhsiu historiu). Vrati, ci sa zapisalo. Medzera medzi kontrolou a premenovanim (mikrosekundy)
/// ostava; zavrela by ju len medziprocesova generacia (dokumentovane v PLAN).
pub fn save_if_unchanged(path: &Path, a: &Archive, before: Stamp) -> std::io::Result<bool> {
    if stamp(path) != before {
        return Ok(false);
    }
    save(path, a)?;
    Ok(true)
}

/// Zmaze archiv; `true`, ak subor existoval. Surove logy poskytovatelov sa nedotknu.
pub fn delete(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Uplatni retenciu na subor archivu hned (napr. po skratenej retencii v nastaveniach); vrati pocet zmazanych riadkov.
/// Poskodeny / chybajuci subor sa nemeni.
pub fn prune_file(path: &Path, today: Ymd, retention_months: u32) -> std::io::Result<usize> {
    let Loaded::Ok(mut a) = load(path) else {
        return Ok(0);
    };
    let removed = prune(&mut a, today, retention_months);
    if removed > 0 {
        save(path, &a)?;
    }
    Ok(removed)
}

/// Zhrnutie archivu pre nastavenia (kolko a od kedy).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub rows: usize,
    pub days: usize,
    pub from: Option<Day>,
    pub to: Option<Day>,
    pub bytes: u64,
    pub unreadable: bool,
}

pub fn info(path: &Path) -> Info {
    let bytes = std::fs::metadata(path).map_or(0, |m| m.len());
    match load(path) {
        Loaded::Missing => Info::default(),
        Loaded::Unreadable => Info {
            bytes,
            unreadable: true,
            ..Default::default()
        },
        Loaded::Ok(a) => {
            let mut days: Vec<Day> = a.rows.iter().map(|r| r.day).collect();
            days.sort_unstable();
            days.dedup();
            Info {
                rows: a.rows.len(),
                days: days.len(),
                from: days.first().copied(),
                to: days.last().copied(),
                bytes,
                unreadable: false,
            }
        }
    }
}

/// Prvy den, ktory retencia este ponecha (`None` = bez obmedzenia).
pub fn cutoff(today: Ymd, retention_months: u32) -> Option<Day> {
    if retention_months == 0 {
        return None;
    }
    let (y, m) = add_months(today.y, today.m, -(retention_months as i32));
    Some((y, m, today.d.min(days_in_month(y, m))))
}

/// Retencia: zmaze riadky starsie ako hranica a orezi pokrytie. Vrati pocet zmazanych riadkov.
pub fn prune(a: &mut Archive, today: Ymd, retention_months: u32) -> usize {
    let Some(cut) = cutoff(today, retention_months) else {
        return 0;
    };
    let before = a.rows.len();
    a.rows.retain(|r| r.day >= cut);
    a.cover.retain(|c| c.to >= cut);
    for c in a.cover.iter_mut() {
        if c.from < cut {
            c.from = cut;
        }
    }
    before - a.rows.len()
}

/// Vlozi rozsah do pokrytia a zluci prekryvajuce sa / susediace rozsahy toho isteho poskytovatela.
fn add_cover(cover: &mut Vec<Cover>, add: Cover) {
    cover.push(add);
    cover.sort_by_key(|c| (c.provider, ord(c.from)));
    let mut out: Vec<Cover> = Vec::new();
    for c in cover.drain(..) {
        match out.last_mut() {
            Some(l) if l.provider == c.provider && ord(c.from) <= ord(l.to) + 1 => {
                if ord(c.to) > ord(l.to) {
                    l.to = c.to;
                }
            }
            _ => out.push(c),
        }
    }
    *cover = out;
}

/// Vysledok skenu jedneho zdroja pre zlucovanie.
#[derive(Clone, Debug)]
pub struct Scan {
    pub provider: Provider,
    pub source: SourceKind,
    /// Prvy lokalny den so zaznamom (`None` = zdroj nema ziadny zaznam).
    pub oldest: Option<Ymd>,
    /// Zdroj sa nepodarilo cele precitat (chyby citania) - sucty mozu byt nizsie.
    pub errors: bool,
    /// Adresar zdroja neexistuje / zdroj nie je dostupny - archiv sa nemeni.
    pub missing: bool,
}

/// Co zlucenie zmenilo (pre testy a log).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub added: usize,
    pub replaced: usize,
    pub kept: usize,
}

fn key(r: &Row) -> (Provider, &str, Day) {
    (r.provider, r.model.as_str(), r.day)
}

/// Zluci cerstve denne sucty (`fresh`, z `by_day`) do archivu. `today` = dnesny lokalny den. Vrati pre kazdy riadok archivu,
/// ci jeho hodnota pochadza z tohto skenu (`live`), v poradi `a.rows` po zlucovani.
pub fn merge(
    a: &mut Archive,
    scans: &[Scan],
    fresh: &[DayTotals],
    today: Ymd,
    price_date: &dyn Fn(Provider) -> String,
) -> (Stats, Vec<bool>) {
    let mut stats = Stats::default();
    // riadky, ktorych hodnota je z tohto skenu: (poskytovatel, model, den)
    let mut live: std::collections::HashSet<(Provider, String, Day)> =
        std::collections::HashSet::new();
    for sc in scans.iter().filter(|s| !s.missing) {
        let Some(oldest) = sc.oldest else {
            continue; // zdroj bez zaznamov: nic sa nevie, archiv sa nemeni (ani pokrytie)
        };
        let oldest_t = tup(oldest);
        let pd = price_date(sc.provider);
        // cerstve riadky tohto poskytovatela podla dna
        let mut days: Vec<Day> = fresh
            .iter()
            .filter(|d| d.provider == Some(sc.provider))
            .map(|d| d.day)
            .collect();
        days.sort_unstable();
        days.dedup();
        for day in days {
            let rows: Vec<Row> = fresh
                .iter()
                .filter(|d| d.provider == Some(sc.provider) && d.day == day)
                .filter_map(|d| {
                    // hranicny den (najstarsi zaznam) a dni pri chybach: mozno neuplne
                    Row::from_day(d, sc.source, &pd, day == oldest_t || sc.errors)
                })
                .collect();
            let authoritative = day > oldest_t && !sc.errors;
            if authoritative {
                // cerstve citanie je autorita: nahradi vsetky archivovane riadky dna (oprava dedupliakcie / aktualizacie)
                let had = a
                    .rows
                    .iter()
                    .filter(|r| r.provider == sc.provider && r.day == day)
                    .count();
                a.rows
                    .retain(|r| !(r.provider == sc.provider && r.day == day));
                for r in rows {
                    live.insert((r.provider, r.model.clone(), r.day));
                    a.rows.push(r);
                }
                if had > 0 {
                    stats.replaced += 1;
                } else {
                    stats.added += 1;
                }
            } else {
                // hranicny den / chyby: nikdy nezmensit (pri modeli ostava vacsi sucet; rovnost = cerstve)
                for r in rows {
                    match a.rows.iter_mut().find(|x| key(x) == key(&r)) {
                        Some(x) if x.tokens() + x.requests > r.tokens() + r.requests => {
                            stats.kept += 1; // archivovany je vacsi (cast dna uz zdroj nema): ostava aj s jeho priznakom
                        }
                        Some(x) => {
                            live.insert((r.provider, r.model.clone(), r.day));
                            // rovnaky alebo novsi sucet: cerstvy; neuplny zostava, ak uz bol (nikdy sa neoznaci za uplny)
                            let was_partial = x.partial;
                            *x = r;
                            x.partial |= was_partial;
                            stats.replaced += 1;
                        }
                        None => {
                            live.insert((r.provider, r.model.clone(), r.day));
                            a.rows.push(r);
                            stats.added += 1;
                        }
                    }
                }
            }
        }
        // pokrytie: [najstarsi zaznam, dnes] bolo uspesne naskenovane
        if today >= oldest {
            add_cover(
                &mut a.cover,
                Cover {
                    provider: sc.provider,
                    from: oldest_t,
                    to: tup(today),
                },
            );
        }
    }
    a.rows
        .sort_by(|x, y| (x.provider, &x.model, x.day).cmp(&(y.provider, &y.model, y.day)));
    let flags = a
        .rows
        .iter()
        .map(|r| live.contains(&(r.provider, r.model.clone(), r.day)))
        .collect();
    (stats, flags)
}

/// Pocet dni medzi dvoma dnami (pre testy).
pub fn days_between(a: Day, b: Day) -> i64 {
    ord(b) - ord(a)
}

/// Den z poradoveho cisla (pre testy).
pub fn day_of(n: i64) -> Day {
    tup(civil_from_days(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CL: Provider = Provider::ClaudeCode;
    const CX: Provider = Provider::Codex;

    fn d(y: i32, m: u32, dd: u32) -> Ymd {
        Ymd::new(y, m, dd)
    }
    fn pd(_: Provider) -> String {
        "2026-11-15".into()
    }
    fn fresh(p: Provider, model: &str, day: Day, tokens: u64, usd: f64) -> DayTotals {
        DayTotals {
            provider: Some(p),
            model: model.into(),
            day,
            input: tokens,
            requests: 1,
            est_usd: usd,
            ..Default::default()
        }
    }
    fn scan(p: Provider, oldest: Option<Ymd>) -> Scan {
        Scan {
            provider: p,
            source: if p == CL {
                SourceKind::ClaudeLocal
            } else {
                SourceKind::CodexLocal
            },
            oldest,
            errors: false,
            missing: false,
        }
    }
    fn row<'a>(a: &'a Archive, p: Provider, model: &str, day: Day) -> Option<&'a Row> {
        a.rows
            .iter()
            .find(|r| r.provider == p && r.model == model && r.day == day)
    }

    #[test]
    fn daily_rows_are_aggregated_with_provenance_and_the_first_day_is_partial() {
        let mut a = Archive::default();
        let f = vec![
            fresh(CL, "m1", (2026, 11, 2), 100, 1.0), // hranicny den
            fresh(CL, "m1", (2026, 11, 3), 200, 2.0),
            fresh(CL, "m2", (2026, 11, 3), 50, 0.5),
            fresh(CX, "g", (2026, 11, 3), 70, 0.7),
        ];
        let sc = [
            scan(CL, Some(d(2026, 11, 2))),
            scan(CX, Some(d(2026, 11, 3))),
        ];
        let (st, _) = merge(&mut a, &sc, &f, d(2026, 11, 20), &pd);
        assert_eq!(a.rows.len(), 4);
        assert_eq!(
            st.added, 3,
            "2. 11. (Claude), 3. 11. (Claude, Codex) - po dnoch"
        );
        let first = row(&a, CL, "m1", (2026, 11, 2)).unwrap();
        assert!(first.partial, "hranicny den je neuplny");
        let r = row(&a, CL, "m1", (2026, 11, 3)).unwrap();
        assert!(!r.partial);
        assert_eq!(
            (r.provider, r.model.as_str(), r.day),
            (CL, "m1", (2026, 11, 3))
        );
        assert_eq!(r.source, SourceKind::ClaudeLocal);
        assert_eq!(r.price_date, "2026-11-15");
        assert_eq!((r.input, r.est_usd, r.requests), (200, 2.0, 1));
        assert_eq!(
            row(&a, CX, "g", (2026, 11, 3)).unwrap().source,
            SourceKind::CodexLocal
        );
        // pokrytie: [najstarsi zaznam, dnes] za kazdeho poskytovatela
        assert!(a.cover.contains(&Cover {
            provider: CL,
            from: (2026, 11, 2),
            to: (2026, 11, 20)
        }));
        assert!(a.cover.contains(&Cover {
            provider: CX,
            from: (2026, 11, 3),
            to: (2026, 11, 20)
        }));
    }

    #[test]
    fn repeated_scans_are_idempotent_and_never_double_count() {
        let f = vec![
            fresh(CL, "m1", (2026, 11, 2), 100, 1.0),
            fresh(CL, "m1", (2026, 11, 3), 200, 2.0),
            fresh(CL, "m1", (2026, 11, 20), 300, 3.0),
        ];
        let sc = [scan(CL, Some(d(2026, 11, 2)))];
        let mut a = Archive::default();
        merge(&mut a, &sc, &f, d(2026, 11, 20), &pd);
        let once = a.clone();
        for _ in 0..5 {
            merge(&mut a, &sc, &f, d(2026, 11, 20), &pd);
        }
        assert_eq!(a, once, "opakovany sken nic nepripocita");
        let total: u64 = a.rows.iter().map(Row::tokens).sum();
        assert_eq!(total, 600);
        // aj dnesok, ktory sa v case meni: stale nahrada, nikdy nepripocita
        let f2 = vec![
            fresh(CL, "m1", (2026, 11, 2), 100, 1.0),
            fresh(CL, "m1", (2026, 11, 3), 200, 2.0),
            fresh(CL, "m1", (2026, 11, 20), 450, 4.5),
        ];
        merge(&mut a, &sc, &f2, d(2026, 11, 20), &pd);
        merge(&mut a, &sc, &f2, d(2026, 11, 20), &pd);
        assert_eq!(row(&a, CL, "m1", (2026, 11, 20)).unwrap().input, 450);
        assert_eq!(a.rows.iter().map(Row::tokens).sum::<u64>(), 750);
    }

    #[test]
    fn full_days_are_corrected_when_source_records_change_or_are_deduplicated() {
        let sc = [scan(CL, Some(d(2026, 11, 2)))];
        let mut a = Archive::default();
        // povodne sa den 5. 11. precital s dvojitym zapoctom a modelom "dup"
        merge(
            &mut a,
            &sc,
            &[
                fresh(CL, "m1", (2026, 11, 2), 10, 0.1),
                fresh(CL, "m1", (2026, 11, 5), 1000, 10.0),
                fresh(CL, "dup", (2026, 11, 5), 500, 5.0),
            ],
            d(2026, 11, 20),
            &pd,
        );
        assert_eq!(a.rows.iter().filter(|r| r.day == (2026, 11, 5)).count(), 2);
        // neskor zdroj deduplikoval: nizsi sucet a model "dup" zanikol -> archiv sa opravi (nie max)
        let (st, _) = merge(
            &mut a,
            &sc,
            &[
                fresh(CL, "m1", (2026, 11, 2), 10, 0.1),
                fresh(CL, "m1", (2026, 11, 5), 700, 7.0),
            ],
            d(2026, 11, 20),
            &pd,
        );
        assert_eq!(
            st.replaced, 2,
            "hranicny 2. 11. (rovnaky sucet) a opraveny 5. 11."
        );
        assert_eq!(row(&a, CL, "m1", (2026, 11, 5)).unwrap().input, 700);
        assert!(row(&a, CL, "dup", (2026, 11, 5)).is_none());
        assert!((row(&a, CL, "m1", (2026, 11, 5)).unwrap().est_usd - 7.0).abs() < 1e-9);
        // vyssia hodnota (zaznamy pribudli) tiez nahradi
        merge(
            &mut a,
            &sc,
            &[
                fresh(CL, "m1", (2026, 11, 2), 10, 0.1),
                fresh(CL, "m1", (2026, 11, 5), 900, 9.0),
            ],
            d(2026, 11, 20),
            &pd,
        );
        assert_eq!(row(&a, CL, "m1", (2026, 11, 5)).unwrap().input, 900);
        // cena sa prepocita novym cennikom len pre dni, ktore zdroj este cita
        let new_prices = |_: Provider| "2026-12-01".to_string();
        merge(
            &mut a,
            &sc,
            &[
                fresh(CL, "m1", (2026, 11, 2), 10, 0.1),
                fresh(CL, "m1", (2026, 11, 5), 900, 9.9),
            ],
            d(2026, 11, 20),
            &new_prices,
        );
        let r = row(&a, CL, "m1", (2026, 11, 5)).unwrap();
        assert_eq!((r.price_date.as_str(), r.est_usd), ("2026-12-01", 9.9));
    }

    #[test]
    fn days_the_source_no_longer_has_are_kept_and_boundary_days_never_shrink() {
        let mut a = Archive::default();
        let all = vec![
            fresh(CL, "m1", (2026, 11, 1), 100, 1.0),
            fresh(CL, "m1", (2026, 11, 2), 200, 2.0),
            fresh(CL, "m1", (2026, 11, 3), 300, 3.0),
            fresh(CL, "m1", (2026, 11, 4), 400, 4.0),
        ];
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 11, 1)))],
            &all,
            d(2026, 11, 4),
            &pd,
        );
        assert!(row(&a, CL, "m1", (2026, 11, 3)).is_some());
        // Claude Code zmazal prepisy starsie ako 3. 11. a z dna 3. 11. ostala len cast (150 z 300)
        let left = vec![
            fresh(CL, "m1", (2026, 11, 3), 150, 1.5),
            fresh(CL, "m1", (2026, 11, 4), 400, 4.0),
        ];
        let (st, flags) = merge(
            &mut a,
            &[scan(CL, Some(d(2026, 11, 3)))],
            &left,
            d(2026, 11, 10),
            &pd,
        );
        // 1. a 2. 11. ostali (zdroj ich uz nema)
        assert_eq!(row(&a, CL, "m1", (2026, 11, 1)).unwrap().input, 100);
        assert_eq!(row(&a, CL, "m1", (2026, 11, 2)).unwrap().input, 200);
        // hranicny den 3. 11.: vacsi archivovany sucet ostava (neznizi sa)
        assert_eq!(
            row(&a, CL, "m1", (2026, 11, 3)).unwrap().input,
            300,
            "den nikdy nezmensit"
        );
        assert_eq!(st.kept, 1);
        // 4. 11. je cele citane: cerstve; 1.-3. nie su "live"
        let live: Vec<Day> = a
            .rows
            .iter()
            .zip(&flags)
            .filter(|(_, f)| **f)
            .map(|(r, _)| r.day)
            .collect();
        assert_eq!(live, vec![(2026, 11, 4)]);
        // pokrytie: povodne [1, 4] + nove [3, 10] = [1, 10] suvisle
        assert_eq!(
            a.cover,
            vec![Cover {
                provider: CL,
                from: (2026, 11, 1),
                to: (2026, 11, 10)
            }]
        );
    }

    #[test]
    fn boundary_day_takes_a_larger_fresh_value_but_stays_partial() {
        let mut a = Archive::default();
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 11, 3)))],
            &[fresh(CL, "m1", (2026, 11, 3), 100, 1.0)],
            d(2026, 11, 3),
            &pd,
        );
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 11, 3)))],
            &[fresh(CL, "m1", (2026, 11, 3), 130, 1.3)],
            d(2026, 11, 3),
            &pd,
        );
        let r = row(&a, CL, "m1", (2026, 11, 3)).unwrap();
        assert_eq!(r.input, 130);
        assert!(r.partial, "hranicny den sa nikdy neoznaci za uplny");
    }

    #[test]
    fn read_errors_never_shrink_and_mark_rows_partial() {
        let mut a = Archive::default();
        let sc = [scan(CL, Some(d(2026, 11, 1)))];
        merge(
            &mut a,
            &sc,
            &[
                fresh(CL, "m1", (2026, 11, 1), 1, 0.0),
                fresh(CL, "m1", (2026, 11, 5), 500, 5.0),
            ],
            d(2026, 11, 10),
            &pd,
        );
        assert!(!row(&a, CL, "m1", (2026, 11, 5)).unwrap().partial);
        // sken s chybami citania vrati mensi sucet: archiv ostane, den je neuplny
        let bad = [Scan {
            errors: true,
            ..scan(CL, Some(d(2026, 11, 1)))
        }];
        merge(
            &mut a,
            &bad,
            &[
                fresh(CL, "m1", (2026, 11, 1), 1, 0.0),
                fresh(CL, "m1", (2026, 11, 5), 200, 2.0),
            ],
            d(2026, 11, 10),
            &pd,
        );
        assert_eq!(row(&a, CL, "m1", (2026, 11, 5)).unwrap().input, 500);
        // vyssi sucet pri chybach sa prevezme, ale ako neuplny
        merge(
            &mut a,
            &bad,
            &[fresh(CL, "m1", (2026, 11, 5), 600, 6.0)],
            d(2026, 11, 10),
            &pd,
        );
        let r = row(&a, CL, "m1", (2026, 11, 5)).unwrap();
        assert_eq!(r.input, 600);
        assert!(r.partial);
    }

    #[test]
    fn missing_or_empty_sources_change_nothing_and_add_no_coverage() {
        let mut a = Archive::default();
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 11, 1)))],
            &[fresh(CL, "m1", (2026, 11, 2), 5, 0.0)],
            d(2026, 11, 3),
            &pd,
        );
        let before = a.clone();
        // adresar zdroja zmizol
        let gone = [Scan {
            missing: true,
            ..scan(CL, None)
        }];
        merge(&mut a, &gone, &[], d(2026, 12, 25), &pd);
        assert_eq!(
            a, before,
            "chybajuci zdroj archiv nemeni (ani nerozsiruje pokrytie)"
        );
        // zdroj bez zaznamov: nevieme nic => ziadne pokrytie
        merge(&mut a, &[scan(CL, None)], &[], d(2026, 12, 25), &pd);
        assert_eq!(a, before);
        // den medzi skenmi mimo pokrytia ostane "bez historie": [1,3] a [20,25] nie su suvisle
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 12, 20)))],
            &[fresh(CL, "m1", (2026, 12, 22), 9, 0.0)],
            d(2026, 12, 25),
            &pd,
        );
        assert_eq!(
            a.cover,
            vec![
                Cover {
                    provider: CL,
                    from: (2026, 11, 1),
                    to: (2026, 11, 3)
                },
                Cover {
                    provider: CL,
                    from: (2026, 12, 20),
                    to: (2026, 12, 25)
                }
            ],
            "diera medzi pokrytiami sa nezmeni na nulu"
        );
    }

    #[test]
    fn cover_ranges_merge_when_overlapping_or_adjacent_per_provider() {
        let mut c = Vec::new();
        let mk = |p, f: Day, t: Day| Cover {
            provider: p,
            from: f,
            to: t,
        };
        add_cover(&mut c, mk(CL, (2026, 11, 1), (2026, 11, 5)));
        add_cover(&mut c, mk(CL, (2026, 11, 6), (2026, 11, 9))); // susedny
        add_cover(&mut c, mk(CX, (2026, 11, 3), (2026, 11, 4)));
        add_cover(&mut c, mk(CL, (2026, 11, 2), (2026, 11, 3))); // vnutri
        add_cover(&mut c, mk(CL, (2026, 11, 20), (2026, 11, 21)));
        assert_eq!(
            c,
            vec![
                mk(CL, (2026, 11, 1), (2026, 11, 9)),
                mk(CL, (2026, 11, 20), (2026, 11, 21)),
                mk(CX, (2026, 11, 3), (2026, 11, 4)),
            ]
        );
    }

    #[test]
    fn retention_removes_old_rows_and_trims_coverage() {
        let mut a = Archive::default();
        let f: Vec<DayTotals> = [(2026, 1, 15), (2026, 5, 20), (2026, 8, 20), (2026, 11, 20)]
            .into_iter()
            .map(|day| fresh(CL, "m", day, 10, 0.1))
            .collect();
        // zdroj cita od 1. 1. (hranicny den 15. 1. nie je najstarsi => vsetky dni su cele)
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 1, 1)))],
            &f,
            d(2026, 11, 20),
            &pd,
        );
        assert_eq!(a.rows.len(), 4);
        assert_eq!(
            prune(&mut a.clone(), d(2026, 11, 20), 0),
            0,
            "0 = bez obmedzenia"
        );
        // 3 mesiace: hranica 20. 8. (vratane)
        assert_eq!(cutoff(d(2026, 11, 20), 3), Some((2026, 8, 20)));
        let removed = prune(&mut a, d(2026, 11, 20), 3);
        assert_eq!(removed, 2);
        assert!(a.rows.iter().all(|r| r.day >= (2026, 8, 20)));
        assert_eq!(
            a.cover[0].from,
            (2026, 8, 20),
            "pokrytie sa orezalo na hranicu"
        );
        // retencia pocita pomocou posledneho dna kratkeho mesiaca: 31. 5. - 3 mesiace -> 28. 2.
        assert_eq!(cutoff(d(2026, 5, 31), 3), Some((2026, 2, 28)));
        assert_eq!(cutoff(d(2024, 5, 31), 3), Some((2024, 2, 29)));
        assert_eq!(cutoff(d(2026, 2, 28), 12), Some((2025, 2, 28)));
        // cele pokrytie pred hranicou zmizne
        let mut b = Archive {
            cover: vec![Cover {
                provider: CL,
                from: (2025, 1, 1),
                to: (2025, 2, 1),
            }],
            ..Default::default()
        };
        prune(&mut b, d(2026, 11, 20), 6);
        assert!(b.cover.is_empty());
    }

    #[test]
    fn a_collection_that_started_before_delete_or_prune_never_resurrects_the_file() {
        let dir = std::env::temp_dir().join(format!("transom-archive-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("usage-archive.json");
        let rows = |n: u32| {
            let mut a = Archive::default();
            let fresh_rows: Vec<_> = (1..=n)
                .map(|day| fresh(CL, "claude-opus-5-5", (2026, 11, day), 100, 1.0))
                .collect();
            merge(
                &mut a,
                &[scan(CL, Some(d(2026, 11, 1)))],
                &fresh_rows,
                d(2026, 11, 28),
                &pd,
            );
            a
        };
        // 1) zber nacita archiv (odtlacok), medzitym "Zmazat archiv" -> zapis sa preskoci, subor nevznikne
        save(&p, &rows(3)).unwrap();
        let before = stamp(&p);
        assert!(
            save_if_unchanged(&p, &rows(4), before).unwrap(),
            "nezmeneny subor sa zapise"
        );
        let before = stamp(&p);
        assert!(delete(&p).unwrap());
        assert!(!save_if_unchanged(&p, &rows(5), before).unwrap());
        assert!(!p.exists(), "zmazany archiv ostal zmazany");
        // 2) skratena retencia z nastaveni (subor sa prepise) pocas zberu -> zber nevrati dlhsiu historiu
        save(&p, &rows(6)).unwrap();
        let before = stamp(&p);
        assert_eq!(
            prune_file(&p, d(2026, 11, 28), 1).unwrap(),
            0,
            "nic na zmazanie: subor sa nemeni"
        );
        assert!(save_if_unchanged(&p, &rows(7), before).unwrap());
        let before = stamp(&p);
        save(&p, &rows(2)).unwrap(); // iny proces zapisal kratsi archiv
        assert!(!save_if_unchanged(&p, &rows(7), before).unwrap());
        assert_eq!(info(&p).rows, 2, "kratsi archiv ostal");
        // 3) subor, ktory este neexistoval a ostal neexistovat, sa zapise (prvy zber)
        let q = dir.join("new").join("usage-archive.json");
        assert!(save_if_unchanged(&q, &rows(1), stamp(&q)).unwrap());
        // ziadne zostatky docasnych suborov
        let tmp = |d: &Path| {
            std::fs::read_dir(d)
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".tmp")
                })
                .count()
        };
        assert_eq!((tmp(&dir), tmp(q.parent().unwrap())), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_delete_and_info_roundtrip_without_any_content() {
        let dir = std::env::temp_dir().join(format!("transom-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("usage-archive.json");
        assert_eq!(load(&p), Loaded::Missing);
        assert_eq!(info(&p), Info::default());
        let mut a = Archive::default();
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 11, 1)))],
            &[
                fresh(CL, "claude-opus-5-5", (2026, 11, 2), 123, 1.5),
                fresh(CL, "claude-opus-5-5", (2026, 11, 3), 77, 0.5),
            ],
            d(2026, 11, 20),
            &pd,
        );
        save(&p, &a).unwrap();
        assert_eq!(load(&p), Loaded::Ok(a.clone()));
        let i = info(&p);
        assert_eq!(
            (i.rows, i.days, i.from, i.to),
            (2, 2, Some((2026, 11, 2)), Some((2026, 11, 3)))
        );
        assert!(i.bytes > 0 && !i.unreadable);
        // sukromie: subor obsahuje len pole schemy (ziadne cesty, texty konverzacii, projekty)
        let text = std::fs::read_to_string(&p).unwrap();
        for bad in [
            "prompt", "message", "content", "cwd", "project", "title", "\\\\", "session",
        ] {
            assert!(!text.contains(bad), "subor obsahuje '{bad}'");
        }
        // poskodeny / novsi subor = Unreadable (nikdy sa neprepise)
        std::fs::write(&p, b"{ nonsense").unwrap();
        assert_eq!(load(&p), Loaded::Unreadable);
        assert!(info(&p).unreadable);
        std::fs::write(
            &p,
            br#"{"version": 99, "rows": [], "cover": [], "updated_at": 0}"#,
        )
        .unwrap();
        assert_eq!(load(&p), Loaded::Unreadable);
        // zmazanie: raz zmaze, druhy raz nic; ine subory ostavaju
        let other = dir.join("ine.txt");
        std::fs::write(&other, b"x").unwrap();
        assert!(delete(&p).unwrap());
        assert!(!delete(&p).unwrap());
        assert!(other.exists() && load(&p) == Loaded::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_file_applies_a_shorter_retention_immediately_and_ignores_bad_files() {
        let dir =
            std::env::temp_dir().join(format!("transom-archive-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("a.json");
        let mut a = Archive::default();
        let f: Vec<DayTotals> = [(2026, 2, 3), (2026, 10, 3), (2026, 11, 3)]
            .into_iter()
            .map(|day| fresh(CL, "m", day, 5, 0.0))
            .collect();
        merge(
            &mut a,
            &[scan(CL, Some(d(2026, 1, 1)))],
            &f,
            d(2026, 11, 20),
            &pd,
        );
        save(&p, &a).unwrap();
        assert_eq!(
            prune_file(&p, d(2026, 11, 20), 0).unwrap(),
            0,
            "bez obmedzenia"
        );
        assert_eq!(prune_file(&p, d(2026, 11, 20), 6).unwrap(), 1);
        assert_eq!(info(&p).rows, 2);
        assert_eq!(
            prune_file(&p, d(2026, 11, 20), 6).unwrap(),
            0,
            "druhy raz nic"
        );
        std::fs::write(&p, b"nonsense").unwrap();
        assert_eq!(prune_file(&p, d(2026, 11, 20), 1).unwrap(), 0);
        assert_eq!(
            std::fs::read(&p).unwrap(),
            b"nonsense",
            "poskodeny subor sa nemeni"
        );
        assert_eq!(
            prune_file(&dir.join("none.json"), d(2026, 11, 20), 1).unwrap(),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rows_are_sorted_so_equal_state_is_an_equal_file() {
        let sc = [
            scan(CL, Some(d(2026, 11, 1))),
            scan(CX, Some(d(2026, 11, 1))),
        ];
        let f1 = vec![
            fresh(CX, "g", (2026, 11, 5), 1, 0.0),
            fresh(CL, "b", (2026, 11, 5), 2, 0.0),
            fresh(CL, "a", (2026, 11, 5), 3, 0.0),
        ];
        let mut f2 = f1.clone();
        f2.reverse();
        let (mut a, mut b) = (Archive::default(), Archive::default());
        merge(&mut a, &sc, &f1, d(2026, 11, 20), &pd);
        merge(&mut b, &sc, &f2, d(2026, 11, 20), &pd);
        assert_eq!(a, b);
        assert_eq!(days_between((2026, 11, 1), (2026, 11, 5)), 4);
        assert_eq!(day_of(days_from_civil(d(2026, 11, 5))), (2026, 11, 5));
    }
}
