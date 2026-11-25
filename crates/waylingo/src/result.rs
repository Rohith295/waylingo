//! The result document: an RFC 7946 GeoJSON `FeatureCollection` with Waylingo's
//! members. It is typed so that a result can be read back and continued with
//! [`crate::Engine::resume`] after the end user picks a different place.
//!
//! Positions are `[lng, lat]`, as GeoJSON requires. Everything a continuation needs
//! is in the document itself; Waylingo keeps no state between calls.

use serde::{Deserialize, Serialize};
use waylingo_plan::{Defaults, Plan};
use waylingo_providers::ProviderKind;

/// Version of the result document. A major change means [`crate::Engine::resume`]
/// rejects results of the older major version.
pub const RESULT_SCHEMA_VERSION: &str = "1.0.0";

/// One trip result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TripResult {
    /// Always `"FeatureCollection"`.
    #[serde(rename = "type")]
    pub type_: String,
    pub result_schema: String,
    /// Version of the [`Plan`] format (the format the LLM step produces).
    pub plan_schema: String,
    pub request_id: Option<String>,
    pub status: Status,
    pub error: Option<ResultError>,
    pub provider: ProviderKind,
    /// Must be shown with the results, as the provider's terms require.
    pub attribution: String,
    pub plan: Option<Plan>,
    pub plan_hash: Option<String>,
    pub assumptions: Vec<String>,
    pub warnings: Vec<String>,
    /// Calls made by this call only; a resume starts counting from zero.
    pub usage: Usage,
    pub search: SearchEcho,
    pub features: Vec<Feature>,
    /// HMAC-SHA256 of the document (with this field `null`), hex. Present when the
    /// engine has a signing key; [`crate::Engine::resume`] then accepts only results
    /// it signed. Servers that take results back from clients should set a key.
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Every stop was found.
    Complete,
    /// The route was made but some stops were not found (see `unresolved_stop`).
    Partial,
    /// No usable result; see `error`.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultError {
    pub code: crate::ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub llm_calls: u32,
    pub map_calls: u32,
}

/// The search settings in effect, so results can be interpreted and continued.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SearchEcho {
    pub anchor_radius_m: u32,
    /// Route searched on each side of an anchor, metres.
    pub search_span_m: u32,
    /// Results asked per search, after the provider's cap.
    pub search_limit: u8,
    /// Results the developer asked for.
    pub search_limit_requested: u8,
    pub strict_radius: bool,
    pub max_detour_s: u32,
    pub every_window_pct: u8,
    pub top_n: u8,
}

impl SearchEcho {
    pub(crate) fn new(d: &Defaults, provider_max: u8) -> Self {
        Self {
            anchor_radius_m: d.anchor_radius_m,
            search_span_m: d.search_span_m(),
            search_limit: d.search_limit.min(provider_max),
            search_limit_requested: d.search_limit,
            strict_radius: d.strict_radius,
            max_detour_s: d.max_detour_s,
            every_window_pct: d.every_window_pct,
            top_n: d.top_n,
        }
    }

    /// The developer settings this echo came from.
    pub(crate) fn defaults(&self) -> Defaults {
        Defaults {
            max_detour_s: self.max_detour_s,
            every_window_pct: self.every_window_pct,
            top_n: self.top_n,
            anchor_radius_m: self.anchor_radius_m,
            search_limit: self.search_limit_requested,
            search_span_m: Some(self.search_span_m),
            strict_radius: self.strict_radius,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    /// Always `"Feature"`.
    #[serde(rename = "type")]
    pub type_: String,
    pub geometry: Option<Geometry>,
    pub properties: Properties,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Geometry {
    LineString { coordinates: Vec<[f64; 2]> },
    Point { coordinates: [f64; 2] },
}

/// What a feature is, in `properties.kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Properties {
    /// The route line: through the chosen stops when `includes_stops` is true.
    Route(RouteProps),
    Origin(LocationProps),
    Waypoint(LocationProps),
    Destination(LocationProps),
    /// A stop that was found. The Point is the chosen place.
    Stop(StopProps),
    /// A stop that was not found, skipped, or could not be searched. No geometry.
    UnresolvedStop(UnresolvedProps),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RouteProps {
    pub total_m: f64,
    pub total_s: f64,
    pub includes_stops: bool,
}

/// Origin, a waypoint, or the destination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationProps {
    /// For waypoints: position in the plan's `waypoints`, from 0.
    pub index: Option<u32>,
    /// What was looked up (the place as the user said it), or the label of given
    /// coordinates such as "current location".
    pub text: String,
    /// The candidate in use, or `None` for given coordinates.
    pub chosen_id: Option<String>,
    /// Distance along the base route to this point, metres.
    pub along_m: Option<f64>,
    /// Places the provider matched, best first. Offer these when the text was
    /// ambiguous, then continue with the chosen id.
    pub candidates: Vec<LocationCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocationCandidate {
    pub id: String,
    pub label: String,
    pub position: [f64; 2],
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopProps {
    /// The plan's stop request this belongs to.
    pub stop_request_id: String,
    /// 0 for single stops; 0, 1, 2… for "every N" marks.
    pub occurrence: u32,
    pub anchor: Option<AnchorEcho>,
    pub window: Option<WindowEcho>,
    /// The chosen place.
    pub pick: PlaceProps,
    /// Estimated arrival at the stop, seconds from departure.
    pub eta_s: Option<f64>,
    /// True when nothing matched within the anchor radius and the pick is the
    /// nearest place beyond it.
    pub beyond_radius: bool,
    /// Further options for this stop, best first.
    pub alternatives: Vec<PlaceProps>,
    /// Set when the end user's choice shaped this stop.
    pub user_choice: Option<UserChoice>,
}

/// A place found along the route. Provider content: output only, never sent to an LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceProps {
    pub provider_place_id: String,
    pub name: String,
    pub brand: Option<String>,
    pub address: Option<String>,
    pub position: [f64; 2],
    /// Added travel time, seconds, when known.
    pub detour_s: Option<f64>,
    /// Added distance, metres, when known.
    pub detour_m: Option<f64>,
    /// Distance along the base route to the point nearest the place, metres. Used
    /// to order stops.
    pub along_m: f64,
    /// Straight-line distance from the anchor point, metres.
    pub distance_from_anchor_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserChoice {
    /// The pick is the place the end user chose.
    Chosen,
    /// The end user's earlier choice is no longer available (for example, after the
    /// destination changed); the best match was used instead.
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedProps {
    pub stop_request_id: String,
    pub occurrence: u32,
    pub anchor: Option<AnchorEcho>,
    pub window: Option<WindowEcho>,
    pub result: UnresolvedReason,
    /// For `PROVIDER_ERROR`: what went wrong, as for a failed trip (for example
    /// `PROVIDER_RATE_LIMITED` to retry, `PROVIDER_AUTH` to fix the key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<crate::ErrorCode>,
    pub detail: Option<String>,
    /// For a stop the end user skipped: the places that were offered, so it can be
    /// brought back with a choice. Empty otherwise.
    pub options: Vec<PlaceProps>,
}

/// Why a stop has no place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnresolvedReason {
    /// Nothing matched within the detour limit (or every match serves another stop).
    NoCandidates,
    /// The end user left this stop out.
    SkippedByUser,
    /// The map call budget ran out before this stop was searched.
    SkippedBudget,
    /// The anchor cannot be placed on this route (for example, the route is shorter
    /// than the interval).
    Unsupported,
    /// The provider failed for this search.
    ProviderError,
    /// The search did not run.
    NotSearched,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AnchorEcho {
    pub position: [f64; 2],
    pub cum_m: f64,
    pub cum_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowEcho {
    pub start_m: f64,
    pub end_m: f64,
}

impl TripResult {
    /// Read a result produced by Waylingo.
    ///
    /// # Errors
    /// The text is not a Waylingo result document.
    pub fn parse(geojson: &str) -> Result<Self, String> {
        let doc: Self = serde_json::from_str(geojson).map_err(|e| e.to_string())?;
        if doc.type_ != "FeatureCollection" {
            return Err(format!(
                "type is {:?}, not \"FeatureCollection\"",
                doc.type_
            ));
        }
        Ok(doc)
    }
}
