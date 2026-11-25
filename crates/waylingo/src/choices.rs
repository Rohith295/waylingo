//! What the end user chose, for [`crate::Engine::resume`].
//!
//! Choices refer to ids that are in the previous result: a location candidate id, or
//! a stop's `provider_place_id`. Earlier choices recorded in that result (picked or
//! skipped stops) are kept unless a new choice replaces them.
//!
//! With a signing key, Waylingo routes only through places it found itself. Without
//! one, stop positions are taken from the result as given, so a server that takes
//! results back from clients should set [`crate::EngineConfig::signing_key`].
//!
//! A stop skipped before a destination change has no options afterwards (it was not
//! searched along the new route); to bring it back, resume the earlier result.

use serde::{Deserialize, Serialize};

/// The end user's choices after seeing a result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choices {
    pub locations: Vec<LocationChoice>,
    pub stops: Vec<StopChoice>,
}

/// Which of the plan's places a [`LocationChoice`] is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationRole {
    Origin,
    Waypoint,
    Destination,
}

/// Use another candidate for an ambiguous place ("did you mean ...?").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationChoice {
    pub role: LocationRole,
    /// For [`LocationRole::Waypoint`]: its position in the plan's `waypoints`, from 0.
    /// Ignored otherwise.
    pub index: Option<u32>,
    /// One of the feature's `candidates[].id`.
    pub candidate_id: String,
}

/// What the end user wants for one stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopPick {
    /// The pick, one of the `alternatives`, or (for a skipped stop) one of its
    /// `options`, by `provider_place_id`.
    Place { provider_place_id: String },
    /// Leave this stop out of the route.
    Skip,
}

/// A choice for one stop, identified as in the result's stop features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopChoice {
    pub stop_request_id: String,
    pub occurrence: u32,
    pub pick: StopPick,
}
