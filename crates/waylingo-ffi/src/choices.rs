//! The end user's choices after seeing a result, for `reroute`.

use waylingo::{
    Choices as CoreChoices, LocationChoice as CoreLocationChoice, LocationRole as CoreLocationRole,
    StopChoice as CoreStopChoice, StopPick as CoreStopPick,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LocationRole {
    Origin,
    Waypoint,
    Destination,
}

/// Use another candidate for an ambiguous place ("did you mean ...?"): the id is
/// one of the location feature's `candidates[].id`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LocationChoice {
    pub role: LocationRole,
    /// For a waypoint: its position in the plan's waypoints, from 0.
    #[uniffi(default = None)]
    pub index: Option<u32>,
    pub candidate_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StopPick {
    /// The pick, an alternative, or (for a skipped stop) one of its options.
    Place { provider_place_id: String },
    /// Leave this stop out.
    Skip,
}

/// A choice for one stop, identified as in the result's stop features.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StopChoice {
    pub stop_request_id: String,
    pub occurrence: u32,
    pub pick: StopPick,
}

/// Anything not mentioned keeps its value from the previous result.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct Choices {
    #[uniffi(default = [])]
    pub locations: Vec<LocationChoice>,
    #[uniffi(default = [])]
    pub stops: Vec<StopChoice>,
}

impl From<Choices> for CoreChoices {
    fn from(c: Choices) -> Self {
        Self {
            locations: c
                .locations
                .into_iter()
                .map(|l| CoreLocationChoice {
                    role: match l.role {
                        LocationRole::Origin => CoreLocationRole::Origin,
                        LocationRole::Waypoint => CoreLocationRole::Waypoint,
                        LocationRole::Destination => CoreLocationRole::Destination,
                    },
                    index: l.index,
                    candidate_id: l.candidate_id,
                })
                .collect(),
            stops: c
                .stops
                .into_iter()
                .map(|s| CoreStopChoice {
                    stop_request_id: s.stop_request_id,
                    occurrence: s.occurrence,
                    pick: match s.pick {
                        StopPick::Place { provider_place_id } => {
                            CoreStopPick::Place { provider_place_id }
                        }
                        StopPick::Skip => CoreStopPick::Skip,
                    },
                })
                .collect(),
        }
    }
}
