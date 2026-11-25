use serde::{Deserialize, Serialize};
use waylingo_geo::{LatLng, MeasuredRoute};
use waylingo_http::{HttpRequest, HttpResponse};
use waylingo_plan::{Avoid, What};

use crate::ProviderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Here,
    Google,
    #[serde(rename = "tomtom")]
    TomTom,
}

/// How an adapter gets the detour time for each candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetourStrategy {
    /// Search results already carry detour time (TomTom).
    Native,
    /// Computed from per-candidate routing summaries in the search response (Google).
    FromRoutingSummaries,
    /// Needs a separate matrix call per stop request (HERE).
    Matrix,
}

/// What an adapter can do. Published as the support matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub detour: DetourStrategy,
    pub ev_power_filter: bool,
    pub ev_connector_filter: bool,
    pub brand_filter: bool,
    pub avoid_tolls: bool,
    pub avoid_highways: bool,
    pub avoid_ferries: bool,
    /// Most route points accepted in one along-route search window.
    pub max_search_points: usize,
    /// Most results one search call returns.
    pub max_search_results: u8,
}

/// One geocoding match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeocodeCandidate {
    /// Stable id: the provider's place id, or one derived from the position when the
    /// provider gives none. Never a list index.
    pub id: String,
    pub label: String,
    pub position: LatLng,
    /// Provider relevance score, higher is better, if given.
    pub score: Option<f64>,
}

/// A route request. `points` holds origin, waypoints in order, and destination.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteQuery {
    pub points: Vec<LatLng>,
    pub avoid: Vec<Avoid>,
    /// RFC 3339; `None` means now.
    pub departure_time: Option<String>,
    /// True when the vehicle is electric.
    pub electric: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteResult {
    pub route: MeasuredRoute,
    /// Cumulative distance at the end of each leg, metres. The last equals the total.
    pub leg_end_m: Vec<f64>,
    /// Provider-reported traffic delay, seconds, if given.
    pub traffic_delay_s: Option<f64>,
}

/// An along-route search for one stop request in one window.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchQuery {
    /// The window's slice of the route, already simplified to the adapter's limit.
    pub window_line: Vec<LatLng>,
    pub what: What,
    pub max_detour_s: u32,
    pub limit: u8,
    /// How far from the route line places may be, metres (providers that search a
    /// corridor, such as HERE, use it; TomTom uses `max_detour_s` instead).
    pub corridor_m: u32,
}

/// A place found along the route. This is provider content: it goes into the
/// GeoJSON output only, never to an LLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub provider_place_id: String,
    pub name: String,
    pub position: LatLng,
    pub address: Option<String>,
    pub brand: Option<String>,
    /// Added travel time, seconds, when the provider reports it.
    pub detour_s: Option<f64>,
    /// Added distance, metres, when the provider reports it.
    pub detour_m: Option<f64>,
}

/// A map provider. All methods are pure; `id` values are chosen by the engine and
/// echoed back in the matching response.
pub trait ProviderAdapter {
    fn kind(&self) -> ProviderKind;
    fn capabilities(&self) -> Capabilities;
    /// Attribution text that must be shown with this provider's results.
    fn attribution(&self) -> &'static str;

    /// Requests to make once per session before searching (for example, category
    /// lists). Empty when none are needed.
    fn setup_requests(&self, id_prefix: &str) -> Vec<HttpRequest>;
    /// Feed one setup response back.
    ///
    /// # Errors
    /// The provider reported an error or the response was unusable.
    fn apply_setup(&mut self, resp: &HttpResponse) -> Result<(), ProviderError>;

    fn geocode_request(&self, id: String, query: &str, bias: Option<LatLng>) -> HttpRequest;
    /// # Errors
    /// The provider reported an error or the response was unusable.
    fn parse_geocode(&self, resp: &HttpResponse) -> Result<Vec<GeocodeCandidate>, ProviderError>;

    fn route_request(&self, id: String, q: &RouteQuery) -> HttpRequest;
    /// # Errors
    /// The provider reported an error, found no route, or the response was unusable.
    fn parse_route(&self, resp: &HttpResponse) -> Result<RouteResult, ProviderError>;

    fn search_request(&self, id: String, q: &SearchQuery) -> HttpRequest;
    /// # Errors
    /// The provider reported an error or the response was unusable.
    fn parse_search(&self, resp: &HttpResponse) -> Result<Vec<Candidate>, ProviderError>;

    /// A travel-time matrix request, for adapters whose detour strategy is
    /// [`DetourStrategy::Matrix`]. `None` when unsupported.
    fn matrix_request(
        &self,
        _id: String,
        _origins: &[LatLng],
        _destinations: &[LatLng],
    ) -> Option<HttpRequest> {
        None
    }
    /// Travel times in seconds, row-major (`origins x destinations`); `None` where a
    /// pair could not be routed.
    ///
    /// # Errors
    /// The provider reported an error or the response was unusable.
    fn parse_matrix(
        &self,
        _resp: &HttpResponse,
        _origins: usize,
        _destinations: usize,
    ) -> Result<Vec<Option<f64>>, ProviderError> {
        Err(ProviderError {
            kind: crate::ErrorKind::BadRequest,
            message: "matrix routing is not supported by this provider".into(),
            retryable: false,
        })
    }
}
