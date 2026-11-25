//! Limits every [`Plan`] must meet, wherever it came from: the compiler checks them
//! last, and a plan read back from a previous result is checked again before use.
//! They bound the work a plan can cause (anchors, searches, routes), so a tampered
//! or over-ambitious plan cannot exhaust a server.

use std::collections::HashSet;

use crate::compile::PlanError;
use crate::{Amount, Anchor, Location, Plan, Reference, SCHEMA_VERSION};

/// Most stop requests in one plan.
pub const MAX_STOPS: usize = 10;
/// Most waypoints in one plan.
pub const MAX_WAYPOINTS: usize = 10;
/// Most results per stop.
pub const MAX_TOP_N: u8 = 5;
/// Longest interval or offset: 5 000 km or 72 h.
pub const MAX_DISTANCE_M: f64 = 5_000_000.0;
pub const MAX_DURATION_S: f64 = 72.0 * 3600.0;
/// Shortest "every N": 1 km or 5 minutes. Shorter intervals are not stop plans.
pub const MIN_EVERY_DISTANCE_M: f64 = 1_000.0;
pub const MIN_EVERY_DURATION_S: f64 = 300.0;
/// Longest place text.
pub const MAX_PLACE_CHARS: usize = 200;
/// Highest charger power accepted, kW.
pub const MAX_POWER_KW: f64 = 1_000.0;
/// Largest detour limit, seconds.
const MAX_DETOUR_S: u32 = 3_600;

impl Plan {
    /// Check every limit. Pointers refer to the plan's own fields (stop `i` is the
    /// `i`-th entry of `stop_requests`, which matches the request's `stops`).
    ///
    /// # Errors
    /// Every problem found.
    pub fn validate(&self) -> Result<(), Vec<PlanError>> {
        let mut errs = Vec::new();
        let mut bad = |ptr: String, reason: &str| errs.push(PlanError::new(ptr, reason));

        if self.schema_version != SCHEMA_VERSION {
            bad("/schema_version".into(), "unsupported plan schema version");
        }
        if self.stop_requests.len() > MAX_STOPS {
            bad(
                "/stops".into(),
                &format!("at most {MAX_STOPS} stops are supported"),
            );
        }
        if self.waypoints.len() > MAX_WAYPOINTS {
            bad(
                "/waypoints".into(),
                &format!("at most {MAX_WAYPOINTS} waypoints are supported"),
            );
        }
        let places = std::iter::once(("/origin".to_string(), &self.origin))
            .chain(
                self.waypoints
                    .iter()
                    .enumerate()
                    .map(|(i, w)| (format!("/waypoints/{i}"), w)),
            )
            .chain(std::iter::once((
                "/destination".to_string(),
                &self.destination,
            )));
        for (ptr, l) in places {
            match l {
                Location::Coords { lat, lng, label } => {
                    if label
                        .as_deref()
                        .is_some_and(|l| l.chars().count() > MAX_PLACE_CHARS)
                    {
                        bad(ptr.clone(), "label is too long");
                    }
                    let ok = lat.is_finite()
                        && lng.is_finite()
                        && (-90.0..=90.0).contains(lat)
                        && (-180.0..=180.0).contains(lng);
                    if !ok {
                        bad(ptr, "coordinates are out of range");
                    }
                }
                Location::PlaceQuery { text } => {
                    let n = text.trim().chars().count();
                    if n == 0 || n > MAX_PLACE_CHARS {
                        bad(
                            ptr,
                            &format!("place must be 1 to {MAX_PLACE_CHARS} characters"),
                        );
                    }
                }
            }
        }
        if self.departure.time.as_deref().is_some_and(|t| t.len() > 64)
            || self
                .departure
                .timezone
                .as_deref()
                .is_some_and(|t| t.len() > 64)
        {
            bad(
                "/departure".into(),
                "time and timezone must be at most 64 characters",
            );
        }
        if self.max_detour_s == 0 || self.max_detour_s > MAX_DETOUR_S {
            bad(
                "/max_detour".into(),
                &format!("must be between 1 s and {MAX_DETOUR_S} s"),
            );
        }

        let mut ids = HashSet::new();
        for (i, s) in self.stop_requests.iter().enumerate() {
            let base = format!("/stops/{i}");
            if !ids.insert(s.id.as_str()) {
                bad(format!("{base}/id"), "stop ids must be unique");
            }
            if !(1..=MAX_TOP_N).contains(&s.top_n) {
                bad(
                    format!("{base}/top_n"),
                    &format!("must be between 1 and {MAX_TOP_N}"),
                );
            }
            if let Some(kw) = s.what.ev.as_ref().and_then(|e| e.min_power_kw)
                && !(kw.is_finite() && kw > 0.0 && kw <= MAX_POWER_KW)
            {
                bad(
                    format!("{base}/what/min_power_kw"),
                    &format!("must be between 0 and {MAX_POWER_KW} kW"),
                );
            }
            let texts = [
                ("what/text", s.what.text.as_deref()),
                ("what/brand", s.what.brand.as_deref()),
                ("id", Some(s.id.as_str())),
            ];
            for (field, t) in texts {
                if t.is_some_and(|t| t.chars().count() > MAX_PLACE_CHARS) {
                    bad(
                        format!("{base}/{field}"),
                        &format!("must be at most {MAX_PLACE_CHARS} characters"),
                    );
                }
            }
            let ptr = format!("{base}/anchor");
            match s.anchor {
                Anchor::Offset { from, amount, .. } => {
                    if let Some(reason) = amount_problem(amount, None) {
                        bad(format!("{ptr}/amount"), &reason);
                    }
                    if let Reference::Waypoint(k) = from
                        && usize::from(k) >= self.waypoints.len()
                    {
                        bad(
                            format!("{ptr}/from"),
                            "refers to a waypoint that does not exist",
                        );
                    }
                }
                Anchor::Every {
                    interval,
                    window_pct,
                } => {
                    let floor = match interval {
                        Amount::DistanceM(_) => MIN_EVERY_DISTANCE_M,
                        Amount::DurationS(_) => MIN_EVERY_DURATION_S,
                    };
                    if let Some(reason) = amount_problem(interval, Some(floor)) {
                        bad(format!("{ptr}/amount"), &reason);
                    }
                    if window_pct == 0 || window_pct > 50 {
                        bad(format!("{ptr}/window_pct"), "must be between 1 and 50");
                    }
                }
                Anchor::Fraction { value } => {
                    if !(value.is_finite() && value > 0.0 && value < 1.0) {
                        bad(format!("{ptr}/value"), "must be between 0 and 1");
                    }
                }
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

/// Why an amount is unusable, if it is.
pub(crate) fn amount_problem(a: Amount, floor: Option<f64>) -> Option<String> {
    let (v, max) = match a {
        Amount::DistanceM(v) => (v, MAX_DISTANCE_M),
        Amount::DurationS(v) => (v, MAX_DURATION_S),
    };
    if !(v.is_finite() && v > 0.0) {
        return Some("must be a positive number".into());
    }
    if v > max {
        return Some("is longer than any supported trip (5000 km or 72 h)".into());
    }
    if let Some(f) = floor
        && v < f
    {
        return Some("is too short: \"every\" needs at least 1 km or 5 minutes".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::{SurfaceIntent, TripContext, compile};

    fn plan_of(json: &str) -> Result<crate::Plan, Vec<crate::PlanError>> {
        let s: SurfaceIntent = serde_json::from_str(json).unwrap();
        let ctx = TripContext {
            current_location: Some(crate::Coords {
                lat: 52.37,
                lng: 4.89,
            }),
            ..TripContext::default()
        };
        compile(&s, &ctx)
    }

    fn every(value: f64, unit: &str) -> String {
        format!(
            r#"{{"destination":"Berlin","stops":[{{"what":{{"category":"fuel"}},"anchor":{{"kind":"every","amount":{{"value":{value},"unit":"{unit}"}}}}}}]}}"#
        )
    }

    #[test]
    fn tiny_intervals_are_rejected() {
        for (v, u) in [(1.0, "m"), (0.5, "km"), (1.0, "min")] {
            let errs = plan_of(&every(v, u)).unwrap_err();
            assert!(
                errs.iter().any(|e| e.reason.contains("too short")),
                "{v} {u}: {errs:?}"
            );
        }
        assert!(plan_of(&every(200.0, "km")).is_ok());
        assert!(plan_of(&every(5.0, "min")).is_ok());
    }

    #[test]
    fn a_tampered_plan_fails_validation() {
        let mut p = plan_of(&every(200.0, "km")).unwrap();
        p.validate().unwrap();
        p.stop_requests[0].anchor = crate::Anchor::Every {
            interval: crate::Amount::DistanceM(0.01),
            window_pct: 15,
        };
        assert!(p.validate().is_err());

        let mut p = plan_of(&every(200.0, "km")).unwrap();
        p.stop_requests[0].anchor = crate::Anchor::Offset {
            from: crate::Reference::Waypoint(0),
            direction: crate::Direction::After,
            amount: crate::Amount::DistanceM(1_000.0),
        };
        assert!(p.validate().is_err(), "no waypoint 0 in this plan");

        let mut p = plan_of(&every(200.0, "km")).unwrap();
        p.origin = crate::Location::Coords {
            lat: 95.0,
            lng: 0.0,
            label: None,
        };
        assert!(p.validate().is_err());
    }
}
