//! Choosing stops from search candidates.

use std::collections::HashSet;

use waylingo_geo::{AnchorPoint, LatLng, MeasuredRoute, Window, haversine_m};
use waylingo_providers::Candidate;

/// Distance along the route of the route point nearest to `p`, looking only inside
/// `window` when given (on a route that passes the same area twice, the place
/// belongs to the stretch that was searched). Falls back to the whole route when the
/// window holds no route point.
pub(crate) fn nearest_cum_m(route: &MeasuredRoute, p: LatLng, window: Option<Window>) -> f64 {
    let inside = |m: f64| window.is_none_or(|w| (w.start_m..=w.end_m).contains(&m));
    let nearest = |only_window: bool| {
        route
            .coords()
            .iter()
            .zip(route.cum_m())
            .filter(|(_, m)| !only_window || inside(**m))
            .map(|(c, m)| (haversine_m(*c, p), *m))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, m)| m)
    };
    nearest(true).or_else(|| nearest(false)).unwrap_or(0.0)
}

/// A place with where it sits relative to the route and the anchor.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    pub c: Candidate,
    /// Distance along the base route to the point nearest the place, metres.
    pub along_m: f64,
    /// Straight-line distance from the anchor point, metres.
    pub distance_m: f64,
}

pub(crate) struct Choice {
    pub pick: Placed,
    /// True when nothing was found within the radius and the pick is the nearest
    /// place beyond it.
    pub beyond_radius: bool,
    /// Further options, best first.
    pub alternatives: Vec<Placed>,
    /// Set when the end user's choice shaped this stop.
    pub user_choice: Option<crate::result::UserChoice>,
}

/// How one stop is chosen.
#[derive(Clone, Copy)]
pub(crate) struct Rules {
    pub radius_m: f64,
    pub strict: bool,
    pub max_detour_s: u32,
    pub top_n: u8,
    /// The stretch of route that was searched for this stop.
    pub window: Option<Window>,
}

/// Places within `radius_m` of the anchor point come first, ranked by detour time
/// (unknown last). If none is within the radius, the nearest places beyond it are
/// returned and flagged, unless `strict` is set. Places already used by another stop,
/// and detours over the limit, are dropped.
pub(crate) fn choose(
    candidates: &[Candidate],
    route: &MeasuredRoute,
    anchor: &AnchorPoint,
    rules: &Rules,
    taken: &HashSet<String>,
) -> Option<Choice> {
    let Rules {
        radius_m,
        strict,
        max_detour_s,
        top_n,
        window,
    } = *rules;
    struct Ranked<'a> {
        within: bool,
        detour: f64,
        dist: f64,
        cum: f64,
        c: &'a Candidate,
    }
    let mut ranked: Vec<Ranked<'_>> = candidates
        .iter()
        .filter(|c| !taken.contains(&c.provider_place_id))
        .filter(|c| c.detour_s.is_none_or(|d| d <= f64::from(max_detour_s)))
        .map(|c| {
            let dist = haversine_m(c.position, anchor.point);
            Ranked {
                within: dist <= radius_m,
                detour: c.detour_s.unwrap_or(f64::MAX),
                dist,
                cum: nearest_cum_m(route, c.position, window),
                c,
            }
        })
        .filter(|r| !strict || r.within)
        .collect();
    ranked.sort_by(|a, b| {
        b.within.cmp(&a.within).then_with(|| {
            if a.within {
                a.detour
                    .total_cmp(&b.detour)
                    .then(a.dist.total_cmp(&b.dist))
            } else {
                a.dist.total_cmp(&b.dist)
            }
        })
    });
    let any_within = ranked.first().is_some_and(|r| r.within);
    // Alternatives stay on the same side of the radius as the pick.
    let mut it = ranked
        .into_iter()
        .filter(|r| r.within == any_within)
        .take(usize::from(top_n.max(1)));
    let placed = |r: Ranked<'_>| Placed {
        c: r.c.clone(),
        along_m: r.cum,
        distance_m: r.dist,
    };
    let first = placed(it.next()?);
    Some(Choice {
        pick: first,
        beyond_radius: !any_within,
        alternatives: it.map(placed).collect(),
        user_choice: None,
    })
}
