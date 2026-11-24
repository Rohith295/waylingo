use serde::{Deserialize, Serialize};

/// Mean Earth radius in metres (IUGG).
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// A WGS84 coordinate. Note the field order: latitude first.
/// GeoJSON output uses `[lng, lat]`; conversion happens only at export.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

impl LatLng {
    #[must_use]
    pub const fn new(lat: f64, lng: f64) -> Self {
        Self { lat, lng }
    }

    /// True if both values are finite and within WGS84 bounds.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.lat.is_finite()
            && self.lng.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lng)
    }

    /// Linear interpolation in degrees. Accurate enough between adjacent route
    /// points, which are at most a few hundred metres apart.
    #[must_use]
    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            lat: self.lat + (other.lat - self.lat) * t,
            lng: self.lng + (other.lng - self.lng) * t,
        }
    }
}

/// Great-circle distance in metres.
#[must_use]
pub fn haversine_m(a: LatLng, b: LatLng) -> f64 {
    let (lat1, lat2) = (a.lat.to_radians(), b.lat.to_radians());
    let dlat = lat2 - lat1;
    let dlng = (b.lng - a.lng).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amsterdam_berlin_straight_line() {
        let d = haversine_m(LatLng::new(52.3702, 4.8952), LatLng::new(52.5200, 13.4050));
        // Known great-circle distance is about 577 km.
        assert!((d - 577_000.0).abs() < 3_000.0, "{d}");
    }

    #[test]
    fn zero_distance() {
        let p = LatLng::new(10.0, 20.0);
        assert!(haversine_m(p, p).abs() < 1e-9);
    }

    #[test]
    fn validity() {
        assert!(LatLng::new(52.0, 4.0).is_valid());
        assert!(!LatLng::new(91.0, 4.0).is_valid());
        assert!(!LatLng::new(f64::NAN, 4.0).is_valid());
    }
}
