//! A result: the GeoJSON to draw, plus the same facts as typed fields, so an app can
//! show options and build choices without parsing JSON.

use waylingo::{Properties, Status, TripResult, UserChoice as CoreUserChoice};

use crate::choices::LocationRole;
use crate::trip::LatLng;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TripStatus {
    /// Every stop was found.
    Complete,
    /// The route was made, but some stops were not found (see `unresolved`).
    Partial,
    /// No usable result; see `error_code` and `error_message`.
    Failed,
}

/// A place found along the route. Map provider content: show it, never send it to
/// an LLM.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Place {
    /// Pass this in a `StopChoice` to choose the place.
    pub provider_place_id: String,
    pub name: String,
    pub brand: Option<String>,
    pub address: Option<String>,
    pub position: LatLng,
    /// Added travel time, seconds, when known.
    pub detour_s: Option<f64>,
    /// Added distance, metres, when known.
    pub detour_m: Option<f64>,
    /// Straight-line distance from the anchor point, metres.
    pub distance_from_anchor_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UserChoice {
    /// The pick is the place the end user chose.
    Chosen,
    /// The end user's earlier choice was not found again; the best match is used.
    Dropped,
}

/// A stop that was found.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Stop {
    pub stop_request_id: String,
    /// 0 for single stops; 0, 1, 2… for "every N" stops.
    pub occurrence: u32,
    pub pick: Place,
    pub alternatives: Vec<Place>,
    /// Estimated arrival, seconds from departure.
    pub eta_s: Option<f64>,
    /// Nothing was within the radius; the pick is the nearest place beyond it.
    pub beyond_radius: bool,
    pub user_choice: Option<UserChoice>,
}

/// A stop without a place.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UnresolvedStop {
    pub stop_request_id: String,
    pub occurrence: u32,
    /// `NO_CANDIDATES`, `SKIPPED_BY_USER`, `SKIPPED_BUDGET`, `UNSUPPORTED`,
    /// `PROVIDER_ERROR` or `NOT_SEARCHED`.
    pub reason: String,
    pub detail: Option<String>,
    /// For a skipped stop: the places it offered, to bring it back.
    pub options: Vec<Place>,
}

/// One match for a place the end user named.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Candidate {
    /// Pass this in a `LocationChoice` to use this match.
    pub id: String,
    pub label: String,
    pub position: LatLng,
    pub score: Option<f64>,
}

/// The origin, a waypoint, or the destination.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Location {
    pub role: LocationRole,
    /// For a waypoint: its position in the plan, from 0.
    pub index: Option<u32>,
    /// The place as the end user said it.
    pub text: String,
    pub chosen_id: Option<String>,
    pub position: Option<LatLng>,
    /// More than one means the text was ambiguous: offer "did you mean ...?".
    pub candidates: Vec<Candidate>,
}

/// The result of `plan` or `reroute`.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TripOutput {
    /// The full GeoJSON document. Draw it, and pass it to `reroute` after the end
    /// user chooses.
    pub geojson: String,
    pub status: TripStatus,
    /// For example `NOT_UNDERSTOOD` (ask the end user to rephrase), `NO_GEOCODE`,
    /// `PROVIDER_AUTH`, `LLM_UNAVAILABLE`.
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub stops: Vec<Stop>,
    pub unresolved: Vec<UnresolvedStop>,
    pub locations: Vec<Location>,
    /// Route length and time, when there is a route.
    pub route_m: Option<f64>,
    pub route_s: Option<f64>,
    pub warnings: Vec<String>,
    pub assumptions: Vec<String>,
    /// Must be shown with the map, as the map provider's terms require.
    pub attribution: String,
    pub llm_calls: u32,
    pub map_calls: u32,
}

fn ll(p: [f64; 2]) -> LatLng {
    LatLng {
        lat: p[1],
        lng: p[0],
    }
}

fn place(p: &waylingo::PlaceProps) -> Place {
    Place {
        provider_place_id: p.provider_place_id.clone(),
        name: p.name.clone(),
        brand: p.brand.clone(),
        address: p.address.clone(),
        position: ll(p.position),
        detour_s: p.detour_s,
        detour_m: p.detour_m,
        distance_from_anchor_m: p.distance_from_anchor_m,
    }
}

fn reason_code(r: waylingo::UnresolvedReason) -> String {
    serde_json::to_value(r)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

impl TripOutput {
    pub(crate) fn from_geojson(geojson: String) -> Self {
        let doc = match TripResult::parse(&geojson) {
            Ok(doc) => doc,
            // The engine wrote this document, so it parses; say so rather than hide
            // a broken result.
            Err(e) => {
                return Self {
                    geojson,
                    status: TripStatus::Failed,
                    error_code: Some("INTERNAL".into()),
                    error_message: Some(e),
                    stops: vec![],
                    unresolved: vec![],
                    locations: vec![],
                    route_m: None,
                    route_s: None,
                    warnings: vec![],
                    assumptions: vec![],
                    attribution: String::new(),
                    llm_calls: 0,
                    map_calls: 0,
                };
            }
        };
        let mut out = Self {
            status: match doc.status {
                Status::Complete => TripStatus::Complete,
                Status::Partial => TripStatus::Partial,
                Status::Failed => TripStatus::Failed,
            },
            error_code: doc.error.as_ref().map(|e| e.code.clone()),
            error_message: doc.error.as_ref().map(|e| e.message.clone()),
            stops: vec![],
            unresolved: vec![],
            locations: vec![],
            route_m: None,
            route_s: None,
            warnings: doc.warnings.clone(),
            assumptions: doc.assumptions.clone(),
            attribution: doc.attribution.clone(),
            llm_calls: doc.usage.llm_calls,
            map_calls: doc.usage.map_calls,
            geojson: String::new(),
        };
        for f in &doc.features {
            let position = match &f.geometry {
                Some(waylingo::Geometry::Point { coordinates }) => Some(ll(*coordinates)),
                _ => None,
            };
            match &f.properties {
                Properties::Route(r) => {
                    out.route_m = Some(r.total_m);
                    out.route_s = Some(r.total_s);
                }
                Properties::Origin(p) | Properties::Waypoint(p) | Properties::Destination(p) => {
                    let role = match &f.properties {
                        Properties::Origin(_) => LocationRole::Origin,
                        Properties::Waypoint(_) => LocationRole::Waypoint,
                        _ => LocationRole::Destination,
                    };
                    out.locations.push(Location {
                        role,
                        index: p.index,
                        text: p.text.clone(),
                        chosen_id: p.chosen_id.clone(),
                        position,
                        candidates: p
                            .candidates
                            .iter()
                            .map(|c| Candidate {
                                id: c.id.clone(),
                                label: c.label.clone(),
                                position: ll(c.position),
                                score: c.score,
                            })
                            .collect(),
                    });
                }
                Properties::Stop(p) => out.stops.push(Stop {
                    stop_request_id: p.stop_request_id.clone(),
                    occurrence: p.occurrence,
                    pick: place(&p.pick),
                    alternatives: p.alternatives.iter().map(place).collect(),
                    eta_s: p.eta_s,
                    beyond_radius: p.beyond_radius,
                    user_choice: p.user_choice.map(|c| match c {
                        CoreUserChoice::Chosen => UserChoice::Chosen,
                        CoreUserChoice::Dropped => UserChoice::Dropped,
                    }),
                }),
                Properties::UnresolvedStop(p) => out.unresolved.push(UnresolvedStop {
                    stop_request_id: p.stop_request_id.clone(),
                    occurrence: p.occurrence,
                    reason: reason_code(p.result),
                    detail: p.detail.clone(),
                    options: p.options.iter().map(place).collect(),
                }),
            }
        }
        out.geojson = geojson;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_that_is_not_a_result_is_reported_as_failed() {
        let out = TripOutput::from_geojson("{}".into());
        assert_eq!(out.status, TripStatus::Failed);
        assert_eq!(out.error_code.as_deref(), Some("INTERNAL"));
    }

    #[test]
    fn reason_codes_match_the_geojson() {
        assert_eq!(
            reason_code(waylingo::UnresolvedReason::SkippedByUser),
            "SKIPPED_BY_USER"
        );
    }
}
