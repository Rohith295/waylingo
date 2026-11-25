//! One trip request in, one result out.

use waylingo::{
    Avoid, Connector, Coords, Defaults, EvFilter, SavedPlace as CoreSavedPlace, TripContext,
    TripRequest, Vehicle,
};

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

impl From<LatLng> for Coords {
    fn from(p: LatLng) -> Self {
        Self {
            lat: p.lat,
            lng: p.lng,
        }
    }
}

/// A place the end user has named, such as "home". Only the name is shown to the
/// LLM; the position is used for routing.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SavedPlace {
    pub name: String,
    pub location: LatLng,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AvoidKind {
    Tolls,
    Highways,
    Ferries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectorKind {
    Ccs1,
    Ccs2,
    Chademo,
    Type1,
    Type2,
    Nacs,
}

/// An electric vehicle: chargers are searched with these limits unless the request
/// says otherwise.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EvSpec {
    #[uniffi(default = None)]
    pub min_power_kw: Option<f64>,
    #[uniffi(default = [])]
    pub connectors: Vec<ConnectorKind>,
}

/// What the end user said, and what the app knows about the trip.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TripInput {
    /// The request as the end user typed or said it.
    pub query: String,
    /// Where the end user is now. Needed unless the request names a start.
    #[uniffi(default = None)]
    pub current_location: Option<LatLng>,
    #[uniffi(default = None)]
    pub heading_deg: Option<f64>,
    /// For example "en-GB". Helps the LLM read units and place names.
    #[uniffi(default = None)]
    pub locale: Option<String>,
    #[uniffi(default = [])]
    pub saved_places: Vec<SavedPlace>,
    /// The destination of a route already being driven, when the request does not
    /// name one ("coffee in 20 minutes").
    #[uniffi(default = None)]
    pub active_destination: Option<LatLng>,
    #[uniffi(default = None)]
    pub ev: Option<EvSpec>,
    #[uniffi(default = [])]
    pub avoid: Vec<AvoidKind>,
    /// RFC 3339; unset means now.
    #[uniffi(default = None)]
    pub departure_time: Option<String>,
    /// IANA name, for example "Europe/Amsterdam".
    #[uniffi(default = None)]
    pub timezone: Option<String>,
    /// Echoed in the result, for your logs.
    #[uniffi(default = None)]
    pub request_id: Option<String>,
}

impl TripInput {
    pub(crate) fn request(self, defaults: Defaults) -> TripRequest {
        let ev = self.ev.map(|e| EvFilter {
            min_power_kw: e.min_power_kw,
            connector_types: e
                .connectors
                .into_iter()
                .map(|c| match c {
                    ConnectorKind::Ccs1 => Connector::Ccs1,
                    ConnectorKind::Ccs2 => Connector::Ccs2,
                    ConnectorKind::Chademo => Connector::Chademo,
                    ConnectorKind::Type1 => Connector::Type1,
                    ConnectorKind::Type2 => Connector::Type2,
                    ConnectorKind::Nacs => Connector::Nacs,
                })
                .collect(),
        });
        TripRequest {
            query: self.query,
            context: TripContext {
                current_location: self.current_location.map(Into::into),
                heading_deg: self.heading_deg,
                departure_time: self.departure_time,
                timezone: self.timezone,
                active_destination: self.active_destination.map(Into::into),
                vehicle: ev.map(|ev| Vehicle { ev: Some(ev) }),
                avoid: self
                    .avoid
                    .into_iter()
                    .map(|a| match a {
                        AvoidKind::Tolls => Avoid::Tolls,
                        AvoidKind::Highways => Avoid::Highways,
                        AvoidKind::Ferries => Avoid::Ferries,
                    })
                    .collect(),
                max_detour_s: None,
                saved_places: self
                    .saved_places
                    .into_iter()
                    .map(|p| CoreSavedPlace {
                        name: p.name,
                        location: p.location.into(),
                    })
                    .collect(),
                locale: self.locale,
                defaults,
            },
            request_id: self.request_id,
        }
    }
}
