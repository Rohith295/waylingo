//! The canonical plan: SI units, resolved placeholders, validated values.

use serde::{Deserialize, Serialize};

/// Version of the [`Plan`] format. Minor versions only add optional fields.
pub const SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub schema_version: String,
    pub origin: Location,
    /// Named in the query, or taken from the host's active route.
    pub destination: Location,
    pub waypoints: Vec<Location>,
    pub departure: Departure,
    pub vehicle: Vehicle,
    pub avoid: Vec<Avoid>,
    pub max_detour_s: u32,
    pub stop_requests: Vec<StopRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Location {
    /// Exact coordinates (current location, saved place, or active destination).
    Coords {
        lat: f64,
        lng: f64,
        label: Option<String>,
    },
    /// Text to geocode with the map provider.
    PlaceQuery { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Departure {
    /// RFC 3339 time from the host. `None` means "now" at the provider.
    pub time: Option<String>,
    /// IANA timezone from the host, if given.
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Vehicle {
    /// EV details, or `None` for a combustion car.
    pub ev: Option<EvFilter>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EvFilter {
    pub min_power_kw: Option<f64>,
    pub connector_types: Vec<Connector>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connector {
    Ccs1,
    Ccs2,
    Chademo,
    Type1,
    Type2,
    Nacs,
}

impl Connector {
    pub const ALL: [(&'static str, Self); 6] = [
        ("ccs1", Self::Ccs1),
        ("ccs2", Self::Ccs2),
        ("chademo", Self::Chademo),
        ("type1", Self::Type1),
        ("type2", Self::Type2),
        ("nacs", Self::Nacs),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Avoid {
    Tolls,
    Highways,
    Ferries,
}

impl Avoid {
    pub const ALL: [(&'static str, Self); 3] = [
        ("tolls", Self::Tolls),
        ("highways", Self::Highways),
        ("ferries", Self::Ferries),
    ];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StopRequest {
    pub id: String,
    pub what: What,
    pub anchor: Anchor,
    pub top_n: u8,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct What {
    pub category: Option<Category>,
    pub text: Option<String>,
    pub brand: Option<String>,
    pub ev: Option<EvFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    EvCharging,
    Fuel,
    Restaurant,
    FastFood,
    Coffee,
    RestArea,
    Hotel,
    Parking,
    Supermarket,
    Pharmacy,
}

impl Category {
    pub const ALL: [(&'static str, Self); 10] = [
        ("ev_charging", Self::EvCharging),
        ("fuel", Self::Fuel),
        ("restaurant", Self::Restaurant),
        ("fast_food", Self::FastFood),
        ("coffee", Self::Coffee),
        ("rest_area", Self::RestArea),
        ("hotel", Self::Hotel),
        ("parking", Self::Parking),
        ("supermarket", Self::Supermarket),
        ("pharmacy", Self::Pharmacy),
    ];
}

/// A distance in metres or a duration in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Amount {
    DistanceM(f64),
    DurationS(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reference {
    Origin,
    Destination,
    Waypoint(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    After,
    Before,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    Offset {
        from: Reference,
        direction: Direction,
        amount: Amount,
    },
    /// Marks at N, 2N, 3N… from the start, each searched ±`window_pct`% of N.
    Every {
        interval: Amount,
        window_pct: u8,
    },
    Fraction {
        value: f64,
    },
}
