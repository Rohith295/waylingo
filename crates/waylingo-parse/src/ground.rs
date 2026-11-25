//! Snap place, brand and free-text strings back to the user's own words.
//!
//! The model is told to copy places exactly as the user wrote them, but small models
//! garble names they never saw in training ("Bordeaux" becomes "Bordean"). Every such
//! string must come from the request, so when one is not found verbatim, it is
//! replaced by the closest run of words in the request, if one is close enough.
//! Strings with no close match are left alone (the geocoder may still resolve them).

use waylingo_plan::SurfaceIntent;

/// Largest edit distance accepted, as a fraction of the longer string's length.
const MAX_RELATIVE_DISTANCE: f64 = 0.5;

/// Replace garbled copies of the user's words in `intent` with the words from
/// `query`. Saved-place references and "current_location" are not touched.
pub fn ground_in_query(intent: &mut SurfaceIntent, query: &str) {
    let words = word_spans(query);
    let fix = |s: &mut String| {
        if let Some(better) = ground_one(s, query, &words) {
            *s = better;
        }
    };
    for place in intent
        .origin
        .iter_mut()
        .chain(intent.destination.iter_mut())
        .chain(intent.waypoints.iter_mut())
    {
        if place.starts_with("saved:") || place == "current_location" {
            continue;
        }
        fix(place);
    }
    for stop in &mut intent.stops {
        for s in stop.what.brand.iter_mut().chain(stop.what.text.iter_mut()) {
            fix(s);
        }
    }
}

/// Byte spans of the words in `query`, without leading or trailing punctuation.
fn word_spans(query: &str) -> Vec<(usize, usize)> {
    let is_edge = |c: char| !c.is_alphanumeric();
    let mut spans = Vec::new();
    let mut start = None;
    for (i, c) in query
        .char_indices()
        .chain(std::iter::once((query.len(), ' ')))
    {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                let w = &query[s..i];
                let lead = w.len() - w.trim_start_matches(is_edge).len();
                let trimmed = w.trim_matches(is_edge);
                if !trimmed.is_empty() {
                    spans.push((s + lead, s + lead + trimmed.len()));
                }
                start = None;
            }
            _ => {}
        }
    }
    spans
}

/// The query text that `s` should be, or `None` to keep `s` as it is.
fn ground_one(s: &str, query: &str, words: &[(usize, usize)]) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if query.contains(t) {
        return (t != s).then(|| t.to_string());
    }
    let target: Vec<char> = t.to_lowercase().chars().collect();
    let first = target.iter().find(|c| c.is_alphanumeric())?;
    let n = t.split_whitespace().count().max(1);

    // Best candidate by edit distance relative to length; shorter wins ties.
    let mut best: Option<(f64, usize, &str)> = None;
    for len in n.saturating_sub(1).max(1)..=n + 1 {
        for w in words.windows(len) {
            let cand = &query[w[0].0..w[len - 1].1];
            let lower: Vec<char> = cand.to_lowercase().chars().collect();
            // Names rarely get their first letter wrong; house numbers do.
            if first.is_alphabetic() && lower.first() != Some(first) {
                continue;
            }
            let d = levenshtein(&target, &lower);
            let rel = d as f64 / lower.len().max(target.len()) as f64;
            let better = best.is_none_or(|(br, bl, _)| (rel, lower.len()) < (br, bl));
            if better {
                best = Some((rel, lower.len(), cand));
            }
        }
    }
    let (rel, _, cand) = best?;
    (rel <= MAX_RELATIVE_DISTANCE).then(|| cand.to_string())
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use waylingo_plan::{SurfaceAnchor, SurfaceStop, SurfaceWhat};

    fn place(query: &str, got: &str) -> String {
        let mut i = SurfaceIntent {
            destination: Some(got.into()),
            ..SurfaceIntent::default()
        };
        ground_in_query(&mut i, query);
        i.destination.unwrap()
    }

    #[test]
    fn fixes_garbled_names_seen_in_eval() {
        assert_eq!(
            place("Plan a trip to Bordeaux - I need breakfast", "Bordean"),
            "Bordeaux"
        );
        assert_eq!(
            place("Plan a route from Zagreb to Bergen", "Zagad"),
            "Zagreb"
        );
        assert_eq!(
            place("heading to bengaluru through rishikesh.", "b bengaluru"),
            "bengaluru"
        );
        assert_eq!(
            place("we go through rishikesh. get me", "risihesh"),
            "rishikesh"
        );
        assert_eq!(
            place("Drive to Gran Via, Madrid via Princes", "Granvia, Madrid"),
            "Gran Via, Madrid"
        );
        assert_eq!(
            place(
                "from tower bridge, london to helsinki",
                "tower bridges, london"
            ),
            "tower bridge, london"
        );
        assert_eq!(
            place(
                "to Atlanta via Terminal 3, New York:",
                "Terminal 5, New York"
            ),
            "Terminal 3, New York"
        );
    }

    #[test]
    fn keeps_exact_copies_and_trims_spaces() {
        assert_eq!(place("drive to Oslo now", "Oslo"), "Oslo");
        assert_eq!(
            place(
                "drive via Princes Street, Edinburgh",
                " Princes Street, Edinburgh"
            ),
            "Princes Street, Edinburgh"
        );
    }

    #[test]
    fn leaves_names_with_no_close_match() {
        assert_eq!(
            place("drive to SF tonight", "San Francisco"),
            "San Francisco"
        );
        assert_eq!(place("drive to Munich", "München"), "München");
        assert_eq!(place("take me to Pune", "Paris"), "Paris");
    }

    #[test]
    fn skips_saved_places_and_fixes_brands() {
        let mut i = SurfaceIntent {
            origin: Some("current_location".into()),
            destination: Some("saved:home".into()),
            stops: vec![SurfaceStop {
                id: None,
                what: SurfaceWhat {
                    brand: Some("Carrefuor".into()),
                    ..SurfaceWhat::default()
                },
                anchor: SurfaceAnchor::Fraction { value: 0.5 },
                top_n: None,
            }],
            ..SurfaceIntent::default()
        };
        ground_in_query(&mut i, "go home and stop at a Carrefour halfway");
        assert_eq!(i.origin.as_deref(), Some("current_location"));
        assert_eq!(i.destination.as_deref(), Some("saved:home"));
        assert_eq!(i.stops[0].what.brand.as_deref(), Some("Carrefour"));
    }
}
