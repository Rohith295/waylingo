use serde::{Deserialize, Serialize};

use crate::{Avoid, Vehicle};

/// A coordinate supplied by the host.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Coords {
    pub lat: f64,
    pub lng: f64,
}

impl Coords {
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.lat.is_finite()
            && self.lng.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lng)
    }
}

/// A host-supplied named place, e.g. "home".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedPlace {
    pub name: String,
    pub location: Coords,
}

/// Engine defaults applied when the query and context leave a value open.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Defaults {
    pub max_detour_s: u32,
    pub every_window_pct: u8,
    pub top_n: u8,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            max_detour_s: 600,
            every_window_pct: 15,
            top_n: 1,
        }
    }
}

/// Everything the host knows about the request besides the query text.
/// None of this is ever sent to the LLM except `saved_places` names and `locale`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TripContext {
    pub current_location: Option<Coords>,
    pub heading_deg: Option<f64>,
    pub departure_time: Option<String>,
    pub timezone: Option<String>,
    /// Destination of the route the user is already driving, if any.
    pub active_destination: Option<Coords>,
    pub vehicle: Option<Vehicle>,
    pub avoid: Vec<Avoid>,
    pub max_detour_s: Option<u32>,
    pub saved_places: Vec<SavedPlace>,
    pub locale: Option<String>,
    pub defaults: Defaults,
}
