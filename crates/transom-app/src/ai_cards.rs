//! Karty limitov v paneli AI: jedna karta na zapnuteho poskytovatela, v nej jeden blok na **kazde okno, ktore zdroj
//! naozaj hlasi** (5 h, 7 dni, 30 dni, ...). Okna roznej dlzky sa nikdy nezlucuju do jedneho percenta a nic sa
//! nedomysla: chybajuce okno sa nekresli, expirovane okno nema percento (len "reset presiel, caka sa na udaje").
//! Stav (zastarane / reset presiel / bez casu resetu) nesie text v odznaku, nie len farba.
//!
//! `build` je cista funkcia (testovatelna), `paint` len kresli. Vyska karty sa zmeria "nasucho" (`Dry`) a oba stlpce
//! v riadku maju rovnaku vysku.
use crate::ai::{countdown_text, snapshot_name, when, window_name, window_short, LIMIT_FRESH_S};
use transom_i18n::{t, tf};
use transom_render::painter::wrap_text;
use transom_render::{Color, Font, HAlign, Painter, Rect, Size, TextStyle, Theme};
use transom_usage::limits::{self, Freshness, LimitReading, ResetState, RingMode};
use transom_usage::{ContextSnapshot, PlanSnapshot, Provider};

/// Zostava najviac tolkoto percent = malo kapacity (aj text "!", nie len farba).
pub const LOW_REMAINING_PCT: u8 = 10;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub reading: LimitReading,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub provider: Provider,
    pub plan: String,
    /// E-mail prihlaseneho uctu (z lokalnych suborov nastroja), ak je znamy.
    pub account: String,
    pub rows: Vec<Row>,
    pub context: Option<ContextSnapshot>,
    /// Kratka veta, preco poskytovatel nema ziadne okno a ako to opravit (len ak nema ziadne okno).
    pub hint: Option<String>,
}

/// Karty zo snimkov planu: len okna, ktore zdroje naozaj hlasili; poskytovatelia v danom poradi.
pub fn build(
    plans: &[PlanSnapshot],
    context: &[ContextSnapshot],
    providers: &[Provider],
    now: i64,
    hint_for: &dyn Fn(Provider) -> Option<String>,
) -> Vec<Card> {
    providers
        .iter()
        .map(|&prov| {
            let rows: Vec<Row> = limits::provider_windows(plans, prov, now, LIMIT_FRESH_S)
                .into_iter()
                .map(|r| Row { reading: r })
                .collect();
            let plan = plans
                .iter()
                .filter(|x| x.provider == Some(prov))
                .max_by_key(|x| x.recorded_at)
                .map(|x| x.plan.clone())
                .unwrap_or_default();
            let context = context
                .iter()
                .filter(|c| c.provider == Some(prov))
                .max_by_key(|c| c.recorded_at)
                .cloned();
            let hint = if rows.is_empty() {
                hint_for(prov)
            } else {
                None
            };
            Card {
                account: String::new(),
                provider: prov,
                plan,
                rows,
                context,
                hint,
            }
        })
        .collect()
}

/// Painter, ktory nic nekresli, len meria (vyska karty pred kreslenim).
pub(crate) struct Dry<'a>(pub(crate) &'a mut dyn Painter);

impl Painter for Dry<'_> {
    fn size(&self) -> Size {
        self.0.size()
    }
    fn scale(&self) -> f32 {
        self.0.scale()
    }
    fn fill_rect(&mut self, _: Rect, _: Color) {}
    fn measure_text(&mut self, text: &str, style: &TextStyle) -> Size {
        self.0.measure_text(text, style)
    }
    fn draw_text(&mut self, _: &str, _: Rect, _: &TextStyle) {}
    fn blit(&mut self, _: Rect, _: &[Color]) {}
}

fn style(size: f32, bold: bool, color: Color, align: HAlign) -> TextStyle {
    TextStyle {
        font: Font::Ui,
        size_lp: size,
        bold,
        color,
        align,
    }
}

/// Zalomeny odsek; vrati y pod posledne nakreslenym riadkom.
#[allow(clippy::too_many_arguments)]
fn para(
    p: &mut dyn Painter,
    text: &str,
    x: i32,
    mut y: i32,
    w: i32,
    size: f32,
    bold: bool,
    color: Color,
) -> i32 {
    let line_h = (size * 1.5 * p.scale()).round() as i32;
    let st = style(size, bold, color, HAlign::Left);
    for l in wrap_text(p, text, &st, w) {
        p.draw_text(&l, Rect::new(x, y, w, line_h), &st);
        y += line_h;
    }
    y
}

/// Text odznaku STALE: nesie cas posledneho zaznamu ("STALE · 26/11/18 11:56") - zastarana hodnota sa nikdy nevydava za
/// aktualnu. Ostatne odznaky su len slovo.
fn badge_text(r: &LimitReading) -> String {
    format!("{} · {}", t("ai.badge.stale"), when(r.recorded_at))
}

/// Odznak (zaoblena znacka s textom); vrati jeho sirku. Vzdy obrys + slovo.
fn badge(p: &mut dyn Painter, th: &Theme, text: &str, x: i32, y: i32) -> i32 {
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    let st = style(10.0, true, th.foreground, HAlign::Center);
    let w = p.measure_text(text, &st).w + px(12.0);
    let r = Rect::new(x, y, w, px(17.0));
    p.stroke_round_rect(r, px(8.0), px(1.0).max(1), th.muted);
    p.draw_text(text, r, &st);
    w
}

/// Obrysova znacka s textom (napr. plan "Pro/Max/Ultra"), zarovnana **vpravo** na `right`; vrati jej sirku.
fn chip_right(p: &mut dyn Painter, th: &Theme, text: &str, right: i32, y: i32) -> i32 {
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    let st = style(10.0, true, th.muted, HAlign::Center);
    let w = p.measure_text(text, &st).w + px(14.0);
    let r = Rect::new(right - w, y, w, px(17.0));
    p.stroke_round_rect(r, px(8.0), px(1.0).max(1), th.muted);
    p.draw_text(text, r, &st);
    w
}

/// Nazov okna v bezom pohlade: "Session · 5 h", "Weekly · 7 d", "Monthly · 30 d"; ine dlzky podla minut ("90-minute window").
pub fn window_label(minutes: u32) -> String {
    match minutes {
        limits::FIVE_HOURS => tf("ai.win.session", &[&window_short(minutes)]),
        limits::WEEK => tf("ai.win.weekly", &[&window_short(minutes)]),
        43_200 => tf("ai.win.monthly", &[&window_short(minutes)]),
        m => window_name(m),
    }
}

/// Texty hodnoty riadku limitu podla volby Zostava / Spotrebovane (cista funkcia - testovatelna).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowText {
    /// Velke cislo vpravo: "62 %", s "~" (zastarane) a "! " (malo zostava; aj v rezime Spotrebovane); "—" po resete.
    pub big: String,
    /// Slovo za cislom: "left" / "used".
    pub word: String,
    /// Male opacne cislo pod pruhom ("38 % left" pri Used, "62 % used" pri Left); `None` pri expirovanom okne.
    pub other: Option<String>,
    /// Zvolene percento (0-100) pre pruh; `None` pri expirovanom okne (nikdy nie 0 %).
    pub shown: Option<u8>,
}

pub fn row_text(r: &LimitReading, mode: RingMode) -> RowText {
    let expired = r.freshness == Freshness::Expired;
    let stale = r.freshness == Freshness::LastRecorded;
    let rem = r.shown(RingMode::Remaining);
    let shown = r.shown(mode);
    let low = !expired && rem <= LOW_REMAINING_PCT;
    let word = t(match mode {
        RingMode::Remaining => "ai.row.left",
        RingMode::Used => "ai.row.used",
    });
    if expired {
        return RowText {
            big: "—".to_string(),
            word,
            other: None,
            shown: None,
        };
    }
    RowText {
        big: format!(
            "{}{}{} %",
            if low { "! " } else { "" },
            if stale { "~" } else { "" },
            shown
        ),
        word,
        other: Some(match mode {
            RingMode::Remaining => tf("ring.used", &[&r.shown(RingMode::Used).to_string()]),
            RingMode::Used => tf("ring.left", &[&rem.to_string()]),
        }),
        shown: Some(shown),
    }
}

/// Jeden blok okna (technicke detaily su v "Podrobnostiach"). Zobrazenie nasleduje volbu **Zostava / Spotrebovane**
/// (`mode`, ta ista ako kruh v liste; prepina sa v paneli):
/// - riadok A: nazov okna a odpocet (hned za nazvom), vpravo **zvolene %** velkym pismom + slovo ("left" / "used");
/// - pruh = zvolena cast (rovnaky vyznam ako cislo);
/// - riadok B: vlavo stav slovom (STALE + cas / RESET PASSED) a pripadne odpocet, vpravo male opacne cislo
///   ("X % used" pri "Left", "X % left" pri "Used").
///
/// Upozornenie na malo kapacity ("!" a farba) vychadza vzdy zo **zostavajucej** casti (<= 10 %), v oboch rezimoch.
/// Chybajuca hodnota sa nikdy nekresli ako 0 %: expirovane okno ma "—" a prazdnu drahu.
#[allow(clippy::too_many_arguments)]
fn draw_row(
    p: &mut dyn Painter,
    th: &Theme,
    row: &Row,
    x: i32,
    y0: i32,
    w: i32,
    now: i64,
    mode: RingMode,
) -> i32 {
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    let r = &row.reading;
    let expired = r.freshness == Freshness::Expired;
    let stale = r.freshness == Freshness::LastRecorded;
    let rem = r.shown(RingMode::Remaining);
    let low = !expired && rem <= LOW_REMAINING_PCT;
    let label = window_label(r.minutes);
    let (lst, bst, wst, mst) = (
        style(12.0, true, th.foreground, HAlign::Left),
        style(18.0, true, th.foreground, HAlign::Right),
        style(10.0, false, th.muted, HAlign::Left),
        style(11.0, false, th.muted, HAlign::Left),
    );
    // hodnota: zvolena (velke) + slovo; "~" = zastarane, "!" = malo
    let rt = row_text(r, mode);
    let (big, left_word) = (rt.big.clone(), rt.word.clone());
    let (bw, ww) = (
        p.measure_text(&big, &bst).w,
        p.measure_text(&left_word, &wst).w,
    );
    let gap = px(8.0);
    let vw = bw + px(4.0) + ww;
    let rs = r.reset(now);
    let full = countdown_text(rs);
    let short = match rs {
        ResetState::In { secs, .. } => {
            Some(tf("reset.in_short", &[&crate::ai::format_duration(secs)]))
        }
        _ => None,
    };
    let lw = p.measure_text(&label, &lst).w;
    let line_h = p.measure_text(&big, &bst).h.max(px(22.0));
    let mut y = y0;
    p.draw_text(&label, Rect::new(x, y, lw + px(4.0), line_h), &lst);
    // odpocet vedla nazvu: plny text, inak kratky ("↻ 2 h"), inak prejde na riadok B; po resete / bez casu resetu ide na B
    let mut countdown_on_b: Option<String> = None;
    let beside = matches!(rs, ResetState::In { .. });
    if beside {
        let fw = p.measure_text(&full, &mst).w;
        let sw = short
            .as_ref()
            .map(|t| p.measure_text(t, &mst).w)
            .unwrap_or(i32::MAX);
        let room = w - lw - vw - 2 * gap;
        let pick = if fw <= room {
            Some(full.clone())
        } else if sw <= room {
            short.clone()
        } else {
            None
        };
        match pick {
            Some(t) => p.draw_text(
                &t,
                Rect::new(x + lw + gap, y + px(3.0), room.max(0), line_h),
                &mst,
            ),
            None => countdown_on_b = Some(full.clone()),
        }
    }
    // hodnota vpravo: velke cislo, za nim "left"
    p.draw_text(&big, Rect::new(x, y, w - ww - px(4.0), line_h), &bst);
    p.draw_text(
        &left_word,
        Rect::new(x + w - ww, y + px(8.0), ww + px(2.0), line_h),
        &wst,
    );
    y += line_h + px(3.0);
    // pruh (zostavajuca cast): hrubsi, zaoblene konce; expirovane okno = prazdna draha
    let bh = (px(10.0) / 2) * 2;
    p.fill_round_rect(Rect::new(x, y, w, bh), bh / 2, th.control_edge);
    if let Some(sh) = rt.shown.filter(|v| *v > 0) {
        let fw = ((w as f32 * f32::from(sh) / 100.0).round() as i32).max(bh);
        let color = if low {
            th.low
        } else if stale {
            th.muted
        } else {
            th.accent
        };
        p.fill_round_rect(Rect::new(x, y, fw.min(w), bh), bh / 2, color);
    }
    y += bh + px(3.0);
    // riadok B: stav slovom + odpocet (ak nebol vedla nazvu) vlavo, "X % used" vpravo (len ak ma okno hodnotu)
    let used = rt.other.clone();
    let ust = style(11.0, false, th.muted, HAlign::Right);
    let uw = used
        .as_ref()
        .map(|u| p.measure_text(u, &ust).w)
        .unwrap_or(0);
    let chip = if expired {
        Some(t("ai.badge.passed"))
    } else if stale {
        Some(badge_text(r))
    } else {
        None
    };
    let after = match (rs, expired) {
        (ResetState::Unavailable, _) => Some(t("reset.none")),
        (_, true) => Some(t("ai.row.waiting")),
        _ => countdown_on_b,
    };
    if chip.is_some() || after.is_some() || used.is_some() {
        let line_b = px(18.0);
        let mut bx = x;
        if let Some(c) = &chip {
            bx += badge(p, th, c, bx, y) + px(6.0);
        }
        let mut b_h = line_b;
        if let Some(a) = &after {
            let aw = (w - (bx - x) - uw - px(8.0)).max(px(40.0));
            let lines = wrap_text(p, a, &mst, aw);
            for (k, l) in lines.iter().enumerate() {
                p.draw_text(
                    l,
                    Rect::new(bx, y + px(1.0) + k as i32 * px(15.0), aw, px(15.0)),
                    &mst,
                );
            }
            b_h = b_h.max(lines.len() as i32 * px(15.0) + px(2.0));
        }
        if let Some(u) = &used {
            p.draw_text(u, Rect::new(x, y + px(1.0), w, line_b), &ust);
        }
        y += b_h;
    }
    y - y0
}

/// Zdroj a cas zaznamu okna (technicky detail do "Podrobnosti").
pub fn meta_text(r: &LimitReading) -> String {
    tf(
        "ai.limit.meta_short",
        &[&snapshot_name(r.source), &when(r.recorded_at)],
    )
}

/// Karta poskytovatela (jeden stlpec na celu sirku). `bg_h` = vyska pozadia; `None` = len meranie.
#[allow(clippy::too_many_arguments)]
fn draw_card(
    p: &mut dyn Painter,
    th: &Theme,
    card: &Card,
    x: i32,
    y0: i32,
    w: i32,
    now: i64,
    mode: RingMode,
    bg_h: Option<i32>,
) -> i32 {
    let s = p.scale();
    let px = |v: f32| (v * s).round() as i32;
    let pad = px(10.0);
    if let Some(h) = bg_h {
        transom_render::design::card(p, th, Rect::new(x, y0, w, h));
    }
    let (ix, iw) = (x + pad, w - 2 * pad);
    let mut y = y0 + pad;
    // hlavicka: poskytovatel vlavo, plan vpravo (len ak je znamy)
    let hst = style(13.0, true, th.foreground, HAlign::Left);
    let name = crate::ai::provider_short(card.provider);
    p.draw_text(name, Rect::new(ix, y, iw, px(20.0)), &hst);
    let chip_w = if card.plan.is_empty() {
        0
    } else {
        chip_right(p, th, &card.plan, ix + iw, y + px(2.0)) + px(8.0)
    };
    // ucet (pri viacerych uctoch je jasne, ku ktoremu patria limity): v hlavicke medzi nazvom a planom, skrateny
    if !card.account.is_empty() {
        let ast = style(11.0, false, th.muted, HAlign::Left);
        let ax = ix + p.measure_text(name, &hst).w + px(8.0);
        let aw = ix + iw - chip_w - ax;
        if aw > px(40.0) {
            let text = crate::audio_pop::fit(p, &card.account, &ast, aw);
            p.draw_text(&text, Rect::new(ax, y + px(1.0), aw, px(20.0)), &ast);
        }
    }
    y += px(26.0);
    // Grok: limity kreditu su len na serveri - kompaktna karta (ucet a plan); tokeny su v historii
    let local_only = card.provider == Provider::Grok;
    if card.rows.is_empty() && !local_only {
        // nedostupne: nikdy "0 %"; kratky nadpis + jedna veta, ako to opravit (bez tajomstiev a ciest)
        p.draw_text(
            &t("ai.unavail.title"),
            Rect::new(ix, y, iw, px(18.0)),
            &style(12.0, true, th.muted, HAlign::Left),
        );
        y += px(20.0);
        if let Some(h) = &card.hint {
            y = para(p, h, ix, y, iw, 11.0, false, th.muted);
        }
    }
    for (i, row) in card.rows.iter().enumerate() {
        if i > 0 {
            y += px(6.0);
        }
        y += draw_row(p, th, row, ix, y, iw, now, mode);
    }
    y + pad - y0
}

/// Nakresli karty pod seba (kazda na celu sirku). Vrati y pod nimi.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    p: &mut dyn Painter,
    th: &Theme,
    cards: &[Card],
    x: i32,
    mut y: i32,
    w: i32,
    now: i64,
    mode: RingMode,
) -> i32 {
    let gap = (8.0 * p.scale()).round() as i32;
    for c in cards {
        let h = draw_card(&mut Dry(p), th, c, 0, 0, w, now, mode, None);
        draw_card(p, th, c, x, y, w, now, mode, Some(h));
        y += h + gap;
    }
    y
}

/// Vyska kariet (bez kreslenia) - pre test, ze sekcia limitov sa zmesti aj v najhorsom pripade.
#[cfg(test)]
pub fn height(
    p: &mut dyn Painter,
    th: &Theme,
    cards: &[Card],
    w: i32,
    now: i64,
    mode: RingMode,
) -> i32 {
    paint(&mut Dry(p), th, cards, 0, 0, w, now, mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_usage::{LimitWindow, SnapshotSource};

    const NOW: i64 = 2_000_000_000;

    fn snap(p: Provider, at: i64, plan: &str, ws: &[(u32, f64, Option<i64>)]) -> PlanSnapshot {
        PlanSnapshot {
            provider: Some(p),
            plan: plan.into(),
            windows: ws
                .iter()
                .map(|&(minutes, used_percent, resets_at)| LimitWindow {
                    minutes,
                    used_percent,
                    resets_at,
                })
                .collect(),
            recorded_at: at,
            source: match p {
                Provider::ClaudeCode => SnapshotSource::ClaudeStatusline,
                Provider::Codex | Provider::Grok => SnapshotSource::CodexSession,
            },
        }
    }

    const BOTH: [Provider; 2] = [Provider::ClaudeCode, Provider::Codex];

    /// Prve okno karty zo snimku s jednym oknom (`used` %, cas resetu, cas zaznamu).
    fn one_reading(used: f64, reset: Option<i64>, at: i64) -> LimitReading {
        let plans = vec![snap(Provider::ClaudeCode, at, "", &[(300, used, reset)])];
        let cards = build(&plans, &[], &[Provider::ClaudeCode], NOW, &|_| None);
        cards[0].rows[0].reading.clone()
    }

    #[test]
    fn left_and_used_show_the_same_quota_from_two_sides_and_keep_the_other_number() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let r = one_reading(62.0, Some(NOW + 8_000), NOW - 60);
        let used = row_text(&r, RingMode::Used);
        assert_eq!(
            (
                used.big.as_str(),
                used.word.as_str(),
                used.other.as_deref(),
                used.shown
            ),
            ("62 %", "used", Some("38 % left"), Some(62))
        );
        let left = row_text(&r, RingMode::Remaining);
        assert_eq!(
            (
                left.big.as_str(),
                left.word.as_str(),
                left.other.as_deref(),
                left.shown
            ),
            ("38 %", "left", Some("62 % used"), Some(38))
        );
    }

    #[test]
    fn low_alert_is_based_on_remaining_in_both_modes_and_stale_is_marked() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        // 95 % spotrebovane = 5 % zostava: "!" v oboch rezimoch (nie len farba)
        let low = one_reading(95.0, Some(NOW + 8_000), NOW - 60);
        assert_eq!(row_text(&low, RingMode::Used).big, "! 95 %");
        assert_eq!(row_text(&low, RingMode::Remaining).big, "! 5 %");
        // 50 %: ziadne upozornenie
        let ok = one_reading(50.0, Some(NOW + 8_000), NOW - 60);
        assert_eq!(row_text(&ok, RingMode::Used).big, "50 %");
        // zastarana hodnota (zaznam starsi ako 30 min, reset este nenastal): "~"
        let stale = one_reading(40.0, Some(NOW + 8_000), NOW - 7_200);
        assert_eq!(row_text(&stale, RingMode::Used).big, "~40 %");
        assert_eq!(row_text(&stale, RingMode::Remaining).big, "~60 %");
    }

    #[test]
    fn expired_window_has_no_percentage_in_either_mode() {
        let _lang = crate::test_lang::guard();
        transom_i18n::set_lang(transom_i18n::Lang::En);
        let r = one_reading(70.0, Some(NOW - 10), NOW - 60);
        for mode in [RingMode::Used, RingMode::Remaining] {
            let t = row_text(&r, mode);
            assert_eq!(t.big, "—", "nikdy 0 % ani vymyslene cislo");
            assert_eq!((t.other, t.shown), (None, None));
        }
    }

    #[test]
    fn row_words_exist_in_every_language_and_differ() {
        let _lang = crate::test_lang::guard();
        for lang in transom_i18n::Lang::ALL {
            transom_i18n::set_lang(lang);
            let r = one_reading(62.0, Some(NOW + 8_000), NOW - 60);
            let (u, l) = (
                row_text(&r, RingMode::Used),
                row_text(&r, RingMode::Remaining),
            );
            assert!(
                !u.word.is_empty() && !l.word.is_empty() && u.word != l.word,
                "{lang:?}"
            );
            assert!(
                !u.word.starts_with("ai.") && !l.word.starts_with("ai."),
                "{lang:?}: chybajuci preklad"
            );
        }
        transom_i18n::set_lang(transom_i18n::Lang::En);
    }

    #[test]
    fn each_provider_shows_only_the_windows_it_really_reports() {
        let _lang = crate::test_lang::guard();
        let plans = vec![
            snap(
                Provider::ClaudeCode,
                NOW - 60,
                "Pro/Max/Ultra",
                &[
                    (300, 62.0, Some(NOW + 8_040)),
                    (10_080, 31.0, Some(NOW + 300_000)),
                ],
            ),
            // Codex "go": len 30 dni (ziadne 5 h ani tyzden)
            snap(
                Provider::Codex,
                NOW - 60,
                "go",
                &[(43_200, 31.0, Some(NOW + 600_000))],
            ),
        ];
        let cards = build(&plans, &[], &BOTH, NOW, &|_| None);
        let mins = |c: &Card| c.rows.iter().map(|r| r.reading.minutes).collect::<Vec<_>>();
        assert_eq!(mins(&cards[0]), [300, 10_080]);
        assert_eq!(mins(&cards[1]), [43_200]);
        assert_eq!(
            (cards[0].plan.as_str(), cards[1].plan.as_str()),
            ("Pro/Max/Ultra", "go")
        );
        // poskytovatel bez udajov: ziadne okna (nic sa nevymysla), karta ostane (povie "bez limitu")
        let only_codex = build(&plans[1..], &[], &BOTH, NOW, &|_| None);
        assert!(only_codex[0].rows.is_empty());
        assert_eq!(only_codex[1].rows.len(), 1);
    }

    #[test]
    fn context_is_attached_to_its_provider_and_the_latest_wins() {
        let _lang = crate::test_lang::guard();
        let ctx = |at, pct| ContextSnapshot {
            provider: Some(Provider::ClaudeCode),
            used_percent: pct,
            window_tokens: 200_000,
            recorded_at: at,
        };
        let cards = build(
            &[],
            &[ctx(10, 5.0), ctx(30, 42.0), ctx(20, 9.0)],
            &BOTH,
            NOW,
            &|_| None,
        );
        assert_eq!(
            cards[0].context.as_ref().map(|c| c.used_percent),
            Some(42.0)
        );
        assert!(
            cards[1].context.is_none(),
            "Codex kontext nehlasi: nic sa nevymysla"
        );
    }
}
