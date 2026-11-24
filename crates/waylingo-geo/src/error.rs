use thiserror::Error;

/// Errors from route construction, decoding and anchor resolution.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GeoError {
    #[error("route needs at least two coordinates")]
    TooFewPoints,
    #[error("invalid coordinate at index {0}")]
    InvalidCoordinate(usize),
    #[error("invalid checkpoints: {0}")]
    InvalidCheckpoints(String),
    #[error("polyline decode failed: {0}")]
    Decode(String),
    #[error("anchor falls outside the route ({value} of {total})")]
    OutsideRoute { value: f64, total: f64 },
    #[error("invalid anchor: {0}")]
    InvalidAnchor(String),
}
