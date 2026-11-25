//! The loose structure the LLM fills. Enum-like fields are plain strings so the
//! compiler can reply with "allowed values" instead of a JSON parse failure.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the user asked for, as close to their words as possible.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceIntent {
    /// Set to "not_understood" when the text is not a trip request or the wanted
    /// stop or place cannot be told; leave everything else empty then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Start place. Omit or use "current_location" for where the user is now.
    /// Use "saved:<name>" for a saved place such as "saved:home". Otherwise, the
    /// place name exactly as the user said it, e.g. "Amsterdam".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Destination in the same forms as origin. Omit only if the user is already
    /// driving to a destination and did not name a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    /// Places the user wants to pass through, in order, same forms as origin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<String>,
    /// Things to avoid. Allowed: "tolls", "highways", "ferries".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub avoid: Vec<String>,
    /// Largest detour the user accepts for a stop, if they said one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_detour: Option<Quantity>,
    /// The stops the user asked for.
    #[serde(default)]
    pub stops: Vec<SurfaceStop>,
}

/// A number with the unit the user used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub value: f64,
    /// Distance: "m", "km", "mi". Time: "s", "min", "h".
    pub unit: String,
}

/// One requested stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStop {
    /// Short id such as "charger" or "food". Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub what: SurfaceWhat,
    pub anchor: SurfaceAnchor,
    /// How many options to return for this stop (1 to 5). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u8>,
}

/// What kind of place.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceWhat {
    /// One of: "ev_charging", "fuel", "restaurant", "fast_food", "coffee",
    /// "rest_area", "hotel", "parking", "supermarket", "pharmacy".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Free text when no category fits, e.g. "bakery".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A name the user gave (chain, brand or specific place), exactly as written, e.g.
    /// "McDonald's". Do not guess a category for it; the map search finds the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// For chargers: minimum power in kW, if the user said one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_power_kw: Option<f64>,
    /// For chargers: connector types. Allowed: "ccs1", "ccs2", "chademo", "type1", "type2", "nacs".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_types: Vec<String>,
}

/// Where on the route the stop should be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SurfaceAnchor {
    /// "40 minutes into the drive" is from "origin", direction "after".
    /// "15 minutes before we arrive" is from "destination", direction "before".
    Offset {
        /// "origin", "destination", or "waypoint:<n>" (0-based).
        from: String,
        /// "after" or "before".
        direction: String,
        amount: Quantity,
    },
    /// "Every 200 km" or "every 2 hours", counted from the start of the trip.
    Every { amount: Quantity },
    /// A share of the trip: "halfway" is 0.5.
    Fraction { value: f64 },
}
