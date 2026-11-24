//! Route geometry and anchor maths for Waylingo.
//!
//! Everything here is pure computation on coordinates and the time/distance data a
//! map provider reports along a route. No network, no provider-specific JSON.

mod anchor;
mod error;
mod latlng;
mod measured;
pub mod polyline;
mod simplify;
mod window;

pub use anchor::{Amount, AnchorPoint, AnchorSpec, Direction, Reference, resolve};
pub use error::GeoError;
pub use latlng::{EARTH_RADIUS_M, LatLng, haversine_m};
pub use measured::{Checkpoint, MeasuredRoute, RoutePosition};
pub use simplify::simplify_to_max_points;
pub use window::Window;
