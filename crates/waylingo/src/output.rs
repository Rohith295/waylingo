//! Building the result document. Numbers are rounded and fields have a fixed order,
//! so the same inputs always give byte-identical output.

use waylingo_geo::LatLng;
use waylingo_plan::{Plan, SCHEMA_VERSION, plan_hash};
use waylingo_providers::{GeocodeCandidate, ProviderKind, RouteResult};

use crate::config::SigningKey;
use crate::engine::{Outcome, Slot};
use crate::result::{
    AnchorEcho, Feature, Geometry, LocationCandidate, LocationProps, PlaceProps, Properties,
    RESULT_SCHEMA_VERSION, ResultError, RouteProps, SearchEcho, Status, StopProps, TripResult,
    UnresolvedProps, UnresolvedReason, Usage, WindowEcho,
};
use crate::select::Placed;
use crate::signing;

pub(crate) struct Failure {
    pub code: crate::ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Origin,
    Waypoint(u32),
    Destination,
}

/// Origin, a waypoint or the destination, as resolved.
pub(crate) struct LocationOut {
    pub role: Role,
    pub text: String,
    pub chosen_id: Option<String>,
    pub position: Option<LatLng>,
    pub along_m: Option<f64>,
    pub candidates: Vec<GeocodeCandidate>,
}

pub(crate) struct Inputs<'a> {
    pub request_id: Option<&'a str>,
    pub provider: ProviderKind,
    pub attribution: &'a str,
    pub plan: Option<&'a Plan>,
    pub base: Option<&'a RouteResult>,
    pub final_route: Option<&'a RouteResult>,
    pub locations: Vec<LocationOut>,
    pub slots: &'a [Slot],
    pub stop_legs: &'a [(usize, usize)],
    pub assumptions: &'a [String],
    pub warnings: &'a [String],
    pub failure: Option<&'a Failure>,
    pub llm_calls: u32,
    pub map_calls: u32,
    pub search: SearchEcho,
    pub signing_key: Option<&'a SigningKey>,
}

fn r6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn r0(v: f64) -> f64 {
    v.round()
}

fn pos(p: LatLng) -> [f64; 2] {
    [r6(p.lng), r6(p.lat)]
}

fn place(p: &Placed) -> PlaceProps {
    PlaceProps {
        provider_place_id: p.c.provider_place_id.clone(),
        name: p.c.name.clone(),
        brand: p.c.brand.clone(),
        address: p.c.address.clone(),
        position: pos(p.c.position),
        detour_s: p.c.detour_s.map(r0),
        detour_m: p.c.detour_m.map(r0),
        along_m: r0(p.along_m),
        distance_from_anchor_m: r0(p.distance_m),
    }
}

fn eta(i: &Inputs<'_>, slot_index: usize, legs_ok: bool, s: &Slot) -> Option<f64> {
    let from_final = legs_ok
        .then(|| {
            let f = i.final_route?;
            let k = i.stop_legs.iter().find(|x| x.0 == slot_index)?.1;
            f.route
                .position_at_distance(*f.leg_end_m.get(k)?)
                .map(|p| p.cum_s)
        })
        .flatten();
    from_final.or_else(|| s.anchor.map(|a| a.cum_s)).map(r0)
}

fn feature(geometry: Option<Geometry>, properties: Properties) -> Feature {
    Feature {
        type_: "Feature".into(),
        geometry,
        properties,
    }
}

pub(crate) fn build(i: &Inputs<'_>) -> String {
    let mut features = Vec::new();
    let mut warnings: Vec<String> = i.warnings.to_vec();

    if let Some(r) = i.final_route.or(i.base) {
        features.push(feature(
            Some(Geometry::LineString {
                coordinates: r.route.coords().iter().map(|p| pos(*p)).collect(),
            }),
            Properties::Route(RouteProps {
                total_m: r0(r.route.total_m()),
                total_s: r0(r.route.total_s()),
                includes_stops: i.final_route.is_some(),
            }),
        ));
    }

    for l in &i.locations {
        let props = LocationProps {
            index: match l.role {
                Role::Waypoint(k) => Some(k),
                _ => None,
            },
            text: l.text.clone(),
            chosen_id: l.chosen_id.clone(),
            along_m: l.along_m.map(r0),
            candidates: l
                .candidates
                .iter()
                .map(|c| LocationCandidate {
                    id: c.id.clone(),
                    label: c.label.clone(),
                    position: pos(c.position),
                    score: c.score.map(|v| (v * 1e3).round() / 1e3),
                })
                .collect(),
        };
        let geometry = l.position.map(|p| Geometry::Point {
            coordinates: pos(p),
        });
        features.push(feature(
            geometry,
            match l.role {
                Role::Origin => Properties::Origin(props),
                Role::Waypoint(_) => Properties::Waypoint(props),
                Role::Destination => Properties::Destination(props),
            },
        ));
    }

    // Arrival times come from the final route when its legs line up with the stops.
    let legs_ok = i.final_route.is_some_and(|f| {
        let max_k = i.stop_legs.iter().map(|x| x.1).max().unwrap_or(0);
        f.leg_end_m.len() > max_k
    });
    if i.final_route.is_some() && !legs_ok && !i.stop_legs.is_empty() {
        warnings.push("arrival times are estimated from the base route".into());
    }

    let mut unresolved = 0;
    let mut beyond = 0;
    for (si, s) in i.slots.iter().enumerate() {
        let stop_request_id = i
            .plan
            .map_or_else(String::new, |p| p.stop_requests[s.stop].id.clone());
        let occurrence = u32::try_from(s.occurrence).unwrap_or(u32::MAX);
        let anchor = s.anchor.map(|a| AnchorEcho {
            position: pos(a.point),
            cum_m: r0(a.cum_m),
            cum_s: r0(a.cum_s),
        });
        let window = s.window.map(|w| WindowEcho {
            start_m: r0(w.start_m),
            end_m: r0(w.end_m),
        });
        if let Outcome::Found(c) = &s.outcome {
            beyond += usize::from(c.beyond_radius);
            features.push(feature(
                Some(Geometry::Point {
                    coordinates: pos(c.pick.c.position),
                }),
                Properties::Stop(StopProps {
                    stop_request_id,
                    occurrence,
                    anchor,
                    window,
                    pick: place(&c.pick),
                    eta_s: eta(i, si, legs_ok, s),
                    beyond_radius: c.beyond_radius,
                    alternatives: c.alternatives.iter().map(place).collect(),
                    user_choice: c.user_choice,
                }),
            ));
            continue;
        }
        unresolved += 1;
        let mut error_code = None;
        let (result, detail, options) = match &s.outcome {
            Outcome::SkippedBudget => (UnresolvedReason::SkippedBudget, None, vec![]),
            Outcome::SkippedByUser { options } => (
                UnresolvedReason::SkippedByUser,
                None,
                options.iter().map(place).collect(),
            ),
            Outcome::Unsupported(m) => (UnresolvedReason::Unsupported, Some(m.clone()), vec![]),
            Outcome::ProviderError(e) => {
                error_code = Some(e.kind.into());
                (
                    UnresolvedReason::ProviderError,
                    Some(e.message.clone()),
                    vec![],
                )
            }
            Outcome::Waiting => (UnresolvedReason::NotSearched, None, vec![]),
            Outcome::NoCandidates {
                returned,
                within_detour,
            } => (
                UnresolvedReason::NoCandidates,
                Some(format!(
                    "provider returned {returned} result(s), {within_detour} within the detour limit"
                )),
                vec![],
            ),
            Outcome::Carried {
                result,
                detail,
                error_code: code,
            } => {
                error_code = *code;
                (*result, detail.clone(), vec![])
            }
            Outcome::Candidates(_) | Outcome::Found(_) => {
                (UnresolvedReason::NoCandidates, None, vec![])
            }
        };
        features.push(feature(
            None,
            Properties::UnresolvedStop(UnresolvedProps {
                stop_request_id,
                occurrence,
                anchor,
                window,
                result,
                error_code,
                detail,
                options,
            }),
        ));
    }

    if beyond > 0 {
        warnings.push(format!(
            "{beyond} stop(s) had no match within the anchor radius; the nearest match beyond it was used (see beyond_radius)"
        ));
    }
    let status = if i.failure.is_some() {
        Status::Failed
    } else if unresolved > 0 {
        Status::Partial
    } else {
        Status::Complete
    };
    let doc = TripResult {
        type_: "FeatureCollection".into(),
        result_schema: RESULT_SCHEMA_VERSION.into(),
        plan_schema: SCHEMA_VERSION.into(),
        request_id: i.request_id.map(String::from),
        status,
        error: i.failure.map(|f| ResultError {
            code: f.code,
            message: f.message.clone(),
        }),
        provider: i.provider,
        attribution: i.attribution.into(),
        plan: i.plan.cloned(),
        plan_hash: i.plan.map(plan_hash),
        assumptions: i.assumptions.to_vec(),
        warnings,
        usage: Usage {
            llm_calls: i.llm_calls,
            map_calls: i.map_calls,
        },
        search: i.search,
        features,
        signature: None,
    };
    let doc = match i.signing_key {
        Some(key) => TripResult {
            signature: Some(signing::sign(&doc, key)),
            ..doc
        },
        None => doc,
    };
    // Every field is a plain value, so serializing cannot fail.
    serde_json::to_string(&doc).unwrap_or_default()
}
