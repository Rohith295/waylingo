//! Trip plan types and the compiler that turns an LLM's loose output into a
//! canonical, validated plan.
//!
//! Tier 1: the LLM fills a [`SurfaceIntent`] (units and places as written).
//! Tier 2: [`compile`] converts units, fills placeholders from [`TripContext`],
//! applies defaults and validates. Errors come back as [`PlanError`]s with a JSON
//! Pointer, a reason and the allowed values, ready to send in a repair prompt.

mod compile;
mod context;
mod plan;
mod schema;
mod surface;
mod validate;

pub use compile::{PlanError, compile};
pub use context::{
    Coords, Defaults, MAX_ANCHOR_RADIUS_M, MAX_DETOUR_S, MAX_SEARCH_SPAN_M, MIN_ANCHOR_RADIUS_M,
    SavedPlace, TripContext,
};
pub use plan::{
    Amount, Anchor, Avoid, Category, Connector, Departure, Direction, EvFilter, Location, Plan,
    Reference, SCHEMA_VERSION, StopRequest, Vehicle, What,
};
pub use schema::{plan_hash, surface_intent_schema};
pub use surface::{Quantity, SurfaceAnchor, SurfaceIntent, SurfaceStop, SurfaceWhat};
pub use validate::{
    MAX_DISTANCE_M, MAX_DURATION_S, MAX_PLACE_CHARS, MAX_POWER_KW, MAX_STOPS, MAX_TOP_N,
    MAX_WAYPOINTS, MIN_EVERY_DISTANCE_M, MIN_EVERY_DURATION_S,
};
