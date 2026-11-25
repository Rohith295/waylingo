//! Turning a plan anchor ("15 min before arrival", "every 200 km", "halfway") into
//! a route-geometry anchor spec.

use waylingo_geo::{self as geo, AnchorSpec};
use waylingo_plan::{Amount, Anchor, Direction, Reference};
use waylingo_providers::RouteResult;

/// # Errors
/// A message when the anchor refers to a waypoint the route does not have.
pub(super) fn spec_for(a: &Anchor, base: &RouteResult) -> Result<AnchorSpec, String> {
    let geo_amount = |x: Amount| match x {
        Amount::DistanceM(v) => geo::Amount::DistanceM(v),
        Amount::DurationS(v) => geo::Amount::DurationS(v),
    };
    Ok(match *a {
        Anchor::Offset {
            from,
            direction,
            amount,
        } => {
            let from = match from {
                Reference::Origin => geo::Reference::Start,
                Reference::Destination => geo::Reference::End,
                Reference::Waypoint(i) => match base.leg_end_m.get(usize::from(i)) {
                    Some(m) => geo::Reference::AtDistance(*m),
                    None => return Err(format!("waypoint {i} is not on the route")),
                },
            };
            let direction = match direction {
                Direction::After => geo::Direction::After,
                Direction::Before => geo::Direction::Before,
            };
            AnchorSpec::Offset {
                from,
                direction,
                amount: geo_amount(amount),
            }
        }
        Anchor::Every {
            interval,
            window_pct,
        } => {
            // A mark is skipped when less than its tolerance is left before the end.
            let v = match interval {
                Amount::DistanceM(v) | Amount::DurationS(v) => v,
            };
            AnchorSpec::Every {
                interval: geo_amount(interval),
                min_tail: v * f64::from(window_pct) / 100.0,
            }
        }
        Anchor::Fraction { value } => AnchorSpec::Fraction(value),
    })
}
