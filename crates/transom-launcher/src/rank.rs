//! Radenie vysledkov: cista funkcia bez I/O. Skladanie skore: presna zhoda > predpona nazvu > predpona slova > podretazec >
//! postupnost znakov (fuzzy) > zhoda v podrobnosti. Vsetky tokeny dotazu musia sediet (AND). Diakritika sa pri porovnani
//! zahodi (`c` == `č`), takze "ziadost" najde "Žiadosť".
use crate::{Item, Kind};

/// Znak bez diakritiky a malymi pismenami (iba bezne latinske znaky; ostatne sa len zmensia).
pub fn fold(c: char) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    match l {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ą' => 'a',
        'č' | 'ć' | 'ç' => 'c',
        'ď' | 'đ' => 'd',
        'é' | 'è' | 'ê' | 'ë' | 'ě' | 'ē' | 'ę' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
        'ĺ' | 'ľ' | 'ł' => 'l',
        'ň' | 'ń' | 'ñ' => 'n',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ő' | 'ō' => 'o',
        'ŕ' | 'ř' => 'r',
        'š' | 'ś' => 's',
        'ť' => 't',
        'ú' | 'ù' | 'û' | 'ü' | 'ů' | 'ű' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'ž' | 'ź' | 'ż' => 'z',
        'ß' => 's',
        x => x,
    }
}

pub fn norm(s: &str) -> String {
    s.chars().map(fold).collect()
}

/// Tokeny dotazu (bez diakritiky, oddelene bielymi znakmi; najviac 8, kazdy najviac 64 znakov).
pub fn tokens(q: &str) -> Vec<String> {
    norm(q)
        .split_whitespace()
        .take(8)
        .map(|t| t.chars().take(64).collect())
        .collect()
}

fn is_word_start(chars: &[char], i: usize) -> bool {
    i == 0 || !chars[i - 1].is_alphanumeric()
}

/// Skore jedneho tokenu voci nazvu a podrobnosti (`None` = nezhoduje sa).
fn token_score(tok: &str, title: &[char], detail: &[char]) -> Option<i32> {
    let t: Vec<char> = tok.chars().collect();
    if t.is_empty() {
        return Some(0);
    }
    let find = |hay: &[char]| -> Option<usize> {
        if t.len() > hay.len() {
            return None;
        }
        (0..=hay.len() - t.len()).find(|&i| hay[i..i + t.len()] == t[..])
    };
    if title == &t[..] {
        return Some(1000);
    }
    if let Some(i) = find(title) {
        // predpona nazvu > predpona slova > podretazec
        return Some(if i == 0 {
            800 - (title.len() - t.len()).min(200) as i32
        } else if is_word_start(title, i) {
            600 - (i as i32).min(100)
        } else {
            400 - (i as i32).min(100)
        });
    }
    // postupnost znakov v nazve (fuzzy): vsetky znaky v poradi; menej medzier = viac
    let mut pos = 0usize;
    let mut gaps = 0i32;
    let mut ok = true;
    for &c in &t {
        match title[pos..].iter().position(|x| *x == c) {
            Some(off) => {
                gaps += off as i32;
                pos += off + 1;
            }
            None => {
                ok = false;
                break;
            }
        }
    }
    if ok && t.len() >= 2 {
        return Some((250 - gaps * 8).max(40));
    }
    find(detail).map(|_| 100)
}

/// Skore polozky pre dotaz (`None` = nezhoduje sa); prazdny dotaz vracia 0 pre vsetko.
pub fn score(query: &str, title: &str, detail: &str) -> Option<i32> {
    let toks = tokens(query);
    if toks.is_empty() {
        return Some(0);
    }
    let tc: Vec<char> = norm(title).chars().collect();
    let dc: Vec<char> = norm(detail).chars().collect();
    let mut total = 0;
    for t in &toks {
        total += token_score(t, &tc, &dc)?;
    }
    Some(total / toks.len() as i32)
}

fn kind_bonus(k: Kind) -> i32 {
    match k {
        Kind::App => 30,
        Kind::Action => 20,
        Kind::Setting => 10,
        Kind::Note | Kind::Shelf => 5,
        Kind::Folder => 3,
        Kind::File => 0,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub item: Item,
    pub score: i32,
}

/// Vyberie najlepsich `max` vysledkov: zhody podla skore (zostupne, pri rovnosti kratsi nazov a potom abecedne), duplicity
/// (rovnaky druh a cil) sa zlucia. `recents` su nazvy nedavnych volieb (iba ak ich pouzivatel zapol): dostanu bonus.
pub fn rank(query: &str, items: &[Item], max: usize, recents: &[String]) -> Vec<Hit> {
    let mut hits: Vec<Hit> = items
        .iter()
        .filter_map(|it| {
            let s = score(query, &it.title, &it.detail)?;
            let recent = if recents.contains(&it.title) { 150 } else { 0 };
            Some(Hit {
                item: it.clone(),
                score: s + kind_bonus(it.kind) + recent,
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(
                a.item
                    .title
                    .chars()
                    .count()
                    .cmp(&b.item.title.chars().count()),
            )
            .then_with(|| norm(&a.item.title).cmp(&norm(&b.item.title)))
            .then_with(|| a.item.detail.cmp(&b.item.detail))
    });
    let mut seen: Vec<(Kind, String)> = Vec::new();
    let mut out = Vec::new();
    for h in hits {
        if out.len() >= max {
            break;
        }
        let key = (h.item.kind, h.item.target.key());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(h);
    }
    out
}
