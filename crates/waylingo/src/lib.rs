//! Waylingo: natural-language stop planning along routes.
//!
//! Give the engine a query such as "drive to Berlin, a fast charger every 200 km and a
//! McDonald's 15 minutes before we arrive" plus context (current location and so on).
//! It returns one GeoJSON `FeatureCollection` with the route and the chosen stops.
//!
//! The engine performs no I/O. It hands the host [`Step::NeedRequests`] with HTTP
//! requests to send (the host adds the keys) and continues when the host passes the
//! responses to [`Session::step`]. `waylingo-reqwest` is a ready-made host for Rust.
//!
//! Map provider data never reaches the LLM: parsing finishes before the first map
//! request, and the LLM only ever sees the query text, the locale and saved-place names.

mod choices;
mod config;
mod engine;
mod output;
mod result;
mod select;
mod signing;

pub use choices::{Choices, LocationChoice, LocationRole, StopChoice, StopPick};
pub use config::{EngineConfig, Limits, ProviderChoice, SigningKey};
pub use engine::{Engine, Session, Step, TripRequest, WaylingoError};
pub use result::{
    AnchorEcho, Feature, Geometry, LocationCandidate, LocationProps, PlaceProps, Properties,
    RESULT_SCHEMA_VERSION, ResultError, RouteProps, SearchEcho, Status, StopProps, TripResult,
    UnresolvedProps, UnresolvedReason, Usage, UserChoice, WindowEcho,
};
pub use waylingo_http::{
    AuthPlacement, CredentialSlot, Header, HttpRequest, HttpResponse, TransportError,
};
pub use waylingo_parse::{LlmConfig, LlmMode, PromptStyle};
pub use waylingo_plan::{
    Avoid, Connector, Coords, Defaults, EvFilter, SavedPlace, TripContext, Vehicle,
};
