//! Najblizsia udalost z kalendara pre oznamenie ostrova.

/// Najblizsia casova udalost, ktora zacne do `lead_s` sekund od `now` (a este nezacala): (nazov, zaciatok).
pub fn next_event(
    occ: &[transom_core::ics::Occurrence],
    now: i64,
    lead_s: i64,
) -> Option<(String, i64)> {
    occ.iter()
        .filter_map(|o| match o.start {
            transom_core::ics::When::At(a) if a > now && a - now <= lead_s => {
                Some((o.summary.clone(), a))
            }
            _ => None,
        })
        .min_by_key(|(_, a)| *a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::ics::{Occurrence, When};

    #[test]
    fn the_next_event_is_the_closest_timed_one_inside_the_lead_window() {
        let occ = vec![
            Occurrence {
                summary: "Later".into(),
                start: When::At(1_000 + 3_000),
                duration: 60,
            },
            Occurrence {
                summary: "Soon".into(),
                start: When::At(1_000 + 300),
                duration: 60,
            },
            Occurrence {
                summary: "Started".into(),
                start: When::At(900),
                duration: 600,
            },
            Occurrence {
                summary: "All day".into(),
                start: When::Day(transom_core::clock::Ymd {
                    y: 2026,
                    m: 11,
                    d: 25,
                }),
                duration: 86_400,
            },
        ];
        assert_eq!(next_event(&occ, 1_000, 600), Some(("Soon".into(), 1_300)));
        assert_eq!(next_event(&occ, 1_000, 200), None);
    }
}
