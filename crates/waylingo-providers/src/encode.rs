//! Small URL helpers so the crate needs no HTTP dependency.

use std::fmt::Write;

use waylingo_geo::LatLng;

/// Percent-encode everything except RFC 3986 unreserved characters.
pub(crate) fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Build `base?k=v&k=v` with encoded names and values. Pairs with empty values are
/// skipped. Names are encoded too: HERE rejects raw brackets such as
/// `evStation[minPower]` with HTTP 400 (found against the live API).
pub(crate) fn url(base: &str, params: &[(&str, String)]) -> String {
    let q: Vec<String> = params
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", pct(k), pct(v)))
        .collect();
    if q.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", q.join("&"))
    }
}

/// A stable id for a place the provider gave no id for: its position, rounded to
/// about 10 cm.
pub(crate) fn position_id(p: LatLng) -> String {
    format!("pos:{:.6},{:.6}", p.lat, p.lng)
}

/// `lat,lng` with 6 decimals (about 0.1 m).
pub(crate) fn ll(p: LatLng) -> String {
    format!("{:.6},{:.6}", p.lat, p.lng)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_reserved() {
        assert_eq!(pct("McDonald's & Co/1"), "McDonald%27s%20%26%20Co%2F1");
        assert_eq!(pct("Düsseldorf"), "D%C3%BCsseldorf");
    }

    #[test]
    fn builds_query() {
        assert_eq!(
            url("https://x/y", &[("a", "1 2".into()), ("b", String::new())]),
            "https://x/y?a=1%202"
        );
        assert_eq!(url("https://x/y", &[]), "https://x/y");
        assert_eq!(
            url("https://x/y", &[("avoid[features]", "tollRoad".into())]),
            "https://x/y?avoid%5Bfeatures%5D=tollRoad"
        );
    }
}
