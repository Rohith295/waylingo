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
    /// Stops are chosen within this straight-line distance of the anchor point when
    /// possible. Default 5 000 m.
    #[serde(default = "default_anchor_radius_m")]
    pub anchor_radius_m: u32,
    /// Results requested per provider search. Capped at the provider's maximum
    /// (20 for TomTom). Default 20.
    #[serde(default = "default_search_limit")]
    pub search_limit: u8,
    /// Route searched on each side of the anchor, metres. `None` means three times
    /// `anchor_radius_m`. Must be at least `anchor_radius_m`.
    #[serde(default)]
    pub search_span_m: Option<u32>,
    /// When true, never fall back to a place beyond `anchor_radius_m`: a stop with
    /// nothing in range is reported as not found. Default false.
    #[serde(default)]
    pub strict_radius: bool,
}

fn default_anchor_radius_m() -> u32 {
    5_000
}

fn default_search_limit() -> u8 {
    20
}

/// Bounds for developer-supplied settings.
pub const MIN_ANCHOR_RADIUS_M: u32 = 100;
pub const MAX_ANCHOR_RADIUS_M: u32 = 50_000;
pub const MAX_SEARCH_SPAN_M: u32 = 150_000;
pub const MAX_DETOUR_S: u32 = 3_600;

impl Defaults {
    /// Route searched on each side of the anchor, metres.
    #[must_use]
    pub fn search_span_m(&self) -> u32 {
        self.search_span_m
            .unwrap_or(self.anchor_radius_m.saturating_mul(3))
    }

    /// Check developer-supplied values.
    ///
    /// # Errors
    /// A message naming the first out-of-range setting.
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_ANCHOR_RADIUS_M..=MAX_ANCHOR_RADIUS_M).contains(&self.anchor_radius_m) {
            return Err(format!(
                "anchor_radius_m must be between {MIN_ANCHOR_RADIUS_M} and {MAX_ANCHOR_RADIUS_M}, got {}",
                self.anchor_radius_m
            ));
        }
        if self.search_limit == 0 {
            return Err("search_limit must be at least 1".into());
        }
        let span = self.search_span_m();
        if span < self.anchor_radius_m || span > MAX_SEARCH_SPAN_M {
            return Err(format!(
                "search_span_m must be between anchor_radius_m ({}) and {MAX_SEARCH_SPAN_M}, got {span}",
                self.anchor_radius_m
            ));
        }
        if self.max_detour_s == 0 || self.max_detour_s > MAX_DETOUR_S {
            return Err(format!(
                "max_detour_s must be between 1 and {MAX_DETOUR_S}, got {}",
                self.max_detour_s
            ));
        }
        if self.top_n == 0 || self.top_n > 5 {
            return Err(format!("top_n must be between 1 and 5, got {}", self.top_n));
        }
        if self.every_window_pct == 0 || self.every_window_pct > 50 {
            return Err(format!(
                "every_window_pct must be between 1 and 50, got {}",
                self.every_window_pct
            ));
        }
        Ok(())
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            max_detour_s: 600,
            every_window_pct: 15,
            // The route goes through the best place; the others are offered as
            // alternatives the end user can switch to.
            top_n: 3,
            anchor_radius_m: default_anchor_radius_m(),
            search_limit: default_search_limit(),
            search_span_m: None,
            strict_radius: false,
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
