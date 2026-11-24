use crate::{GeoError, LatLng, haversine_m};

/// A point where the provider tells us the cumulative distance and time.
/// HERE spans, Google steps and TomTom instructions all reduce to these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Checkpoint {
    /// Index into the route's coordinate list.
    pub index: usize,
    /// Cumulative driving distance from the start, metres.
    pub cum_m: f64,
    /// Cumulative driving time from the start, seconds.
    pub cum_s: f64,
}

/// A position on a route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePosition {
    pub point: LatLng,
    pub cum_m: f64,
    pub cum_s: f64,
    /// Index of the route segment `[segment, segment + 1]` that contains the position.
    pub segment: usize,
}

/// A route line carrying cumulative distance and time at every coordinate.
///
/// Distances and times come from the provider, not from geometry: between two
/// checkpoints, the provider's distance and time are spread in proportion to the
/// geometric (haversine) length of each segment.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredRoute {
    coords: Vec<LatLng>,
    cum_m: Vec<f64>,
    cum_s: Vec<f64>,
}

impl MeasuredRoute {
    /// Build from coordinates plus provider checkpoints.
    ///
    /// Checkpoints must be sorted by index, start at index 0 with zero distance and
    /// time, end at the last coordinate, and never decrease.
    ///
    /// # Errors
    /// Returns an error for fewer than two points, invalid coordinates, or invalid checkpoints.
    pub fn new(coords: Vec<LatLng>, checkpoints: &[Checkpoint]) -> Result<Self, GeoError> {
        if coords.len() < 2 {
            return Err(GeoError::TooFewPoints);
        }
        if let Some(i) = coords.iter().position(|p| !p.is_valid()) {
            return Err(GeoError::InvalidCoordinate(i));
        }
        validate_checkpoints(checkpoints, coords.len())?;

        let mut geo = Vec::with_capacity(coords.len());
        geo.push(0.0);
        for w in coords.windows(2) {
            let last = *geo.last().unwrap_or(&0.0);
            geo.push(last + haversine_m(w[0], w[1]));
        }

        let mut cum_m = vec![0.0; coords.len()];
        let mut cum_s = vec![0.0; coords.len()];
        for pair in checkpoints.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let span_geo = geo[b.index] - geo[a.index];
            let steps = (b.index - a.index) as f64;
            for i in a.index..=b.index {
                let f = if span_geo > 0.0 {
                    (geo[i] - geo[a.index]) / span_geo
                } else {
                    (i - a.index) as f64 / steps
                };
                cum_m[i] = a.cum_m + f * (b.cum_m - a.cum_m);
                cum_s[i] = a.cum_s + f * (b.cum_s - a.cum_s);
            }
        }
        Ok(Self {
            coords,
            cum_m,
            cum_s,
        })
    }

    /// Build from coordinates and the provider's route totals only.
    ///
    /// # Errors
    /// Same as [`MeasuredRoute::new`].
    pub fn from_totals(coords: Vec<LatLng>, total_m: f64, total_s: f64) -> Result<Self, GeoError> {
        let last = coords.len().saturating_sub(1);
        let cps = [
            Checkpoint {
                index: 0,
                cum_m: 0.0,
                cum_s: 0.0,
            },
            Checkpoint {
                index: last,
                cum_m: total_m,
                cum_s: total_s,
            },
        ];
        Self::new(coords, &cps)
    }

    #[must_use]
    pub fn coords(&self) -> &[LatLng] {
        &self.coords
    }
    #[must_use]
    pub fn cum_m(&self) -> &[f64] {
        &self.cum_m
    }
    #[must_use]
    pub fn cum_s(&self) -> &[f64] {
        &self.cum_s
    }
    #[must_use]
    pub fn total_m(&self) -> f64 {
        *self.cum_m.last().unwrap_or(&0.0)
    }
    #[must_use]
    pub fn total_s(&self) -> f64 {
        *self.cum_s.last().unwrap_or(&0.0)
    }

    /// The position `m` metres along the route, or `None` outside `[0, total_m]`.
    #[must_use]
    pub fn position_at_distance(&self, m: f64) -> Option<RoutePosition> {
        self.position_on(&self.cum_m, m)
    }

    /// The position reached after `s` seconds of driving, or `None` outside `[0, total_s]`.
    #[must_use]
    pub fn position_at_time(&self, s: f64) -> Option<RoutePosition> {
        self.position_on(&self.cum_s, s)
    }

    fn position_on(&self, axis: &[f64], v: f64) -> Option<RoutePosition> {
        let total = *axis.last()?;
        if !v.is_finite() || v < 0.0 || v > total {
            return None;
        }
        // First index whose value is >= v; the segment ends there.
        let end = axis.partition_point(|&x| x < v).max(1).min(axis.len() - 1);
        let start = end - 1;
        let span = axis[end] - axis[start];
        let t = if span > 0.0 {
            (v - axis[start]) / span
        } else {
            0.0
        };
        let lerp = |a: &[f64]| a[start] + t * (a[end] - a[start]);
        Some(RoutePosition {
            point: self.coords[start].lerp(self.coords[end], t),
            cum_m: lerp(&self.cum_m),
            cum_s: lerp(&self.cum_s),
            segment: start,
        })
    }

    /// The route line between two distances, with interpolated end points.
    /// Bounds are clamped to the route.
    #[must_use]
    pub fn slice(&self, start_m: f64, end_m: f64) -> Vec<LatLng> {
        let total = self.total_m();
        let (a, b) = (start_m.clamp(0.0, total), end_m.clamp(0.0, total));
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let (Some(pa), Some(pb)) = (self.position_at_distance(a), self.position_at_distance(b))
        else {
            return Vec::new();
        };
        let mut out = vec![pa.point];
        for i in (pa.segment + 1)..=pb.segment {
            if self.cum_m[i] > a && self.cum_m[i] < b {
                out.push(self.coords[i]);
            }
        }
        out.push(pb.point);
        out
    }
}

fn validate_checkpoints(cps: &[Checkpoint], n: usize) -> Result<(), GeoError> {
    let bad = |m: &str| Err(GeoError::InvalidCheckpoints(m.to_string()));
    let (Some(first), Some(last)) = (cps.first(), cps.last()) else {
        return bad("no checkpoints");
    };
    if first.index != 0 || first.cum_m != 0.0 || first.cum_s != 0.0 {
        return bad("first checkpoint must be index 0 with zero distance and time");
    }
    if last.index != n - 1 {
        return bad("last checkpoint must be at the last coordinate");
    }
    if last.cum_m <= 0.0 {
        return bad("route length must be positive");
    }
    for c in cps {
        if !c.cum_m.is_finite() || !c.cum_s.is_finite() || c.cum_m < 0.0 || c.cum_s < 0.0 {
            return bad("checkpoint values must be finite and non-negative");
        }
    }
    for w in cps.windows(2) {
        if w[1].index <= w[0].index {
            return bad("checkpoint indexes must strictly increase");
        }
        if w[1].cum_m < w[0].cum_m || w[1].cum_s < w[0].cum_s {
            return bad("checkpoint distance and time must not decrease");
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A straight east-west line along the equator: 0.01° of longitude ≈ 1113 m.
    pub(crate) fn line(n: usize) -> Vec<LatLng> {
        (0..n).map(|i| LatLng::new(0.0, i as f64 * 0.01)).collect()
    }

    #[test]
    fn totals_are_scaled_to_provider_values() {
        let r = MeasuredRoute::from_totals(line(11), 10_000.0, 600.0).unwrap();
        assert!((r.total_m() - 10_000.0).abs() < 1e-9);
        assert!((r.total_s() - 600.0).abs() < 1e-9);
        // Even segments: the midpoint is at index 5.
        assert!((r.cum_m()[5] - 5_000.0).abs() < 1e-6);
    }

    #[test]
    fn checkpoints_give_uneven_speed() {
        // First half: 5 km in 120 s (highway). Second half: 5 km in 480 s (city).
        let cps = [
            Checkpoint {
                index: 0,
                cum_m: 0.0,
                cum_s: 0.0,
            },
            Checkpoint {
                index: 5,
                cum_m: 5_000.0,
                cum_s: 120.0,
            },
            Checkpoint {
                index: 10,
                cum_m: 10_000.0,
                cum_s: 600.0,
            },
        ];
        let r = MeasuredRoute::new(line(11), &cps).unwrap();
        // 120 s in is the halfway point, not 20% of the way.
        let p = r.position_at_time(120.0).unwrap();
        assert!((p.cum_m - 5_000.0).abs() < 1e-6, "{p:?}");
        // 360 s in is 3/4 of the route by distance.
        let p = r.position_at_time(360.0).unwrap();
        assert!((p.cum_m - 7_500.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn positions_interpolate_and_bound() {
        let r = MeasuredRoute::from_totals(line(3), 2_000.0, 100.0).unwrap();
        let p = r.position_at_distance(500.0).unwrap();
        assert!((p.point.lng - 0.005).abs() < 1e-9);
        assert_eq!(p.segment, 0);
        assert!(r.position_at_distance(-1.0).is_none());
        assert!(r.position_at_distance(2_000.1).is_none());
        assert!(r.position_at_distance(2_000.0).is_some());
        assert!(r.position_at_distance(0.0).is_some());
    }

    #[test]
    fn slice_has_interpolated_ends() {
        let r = MeasuredRoute::from_totals(line(5), 4_000.0, 400.0).unwrap();
        let s = r.slice(500.0, 2_500.0);
        // 0.5 km point, interior points at 1 km and 2 km, then 2.5 km point.
        assert_eq!(s.len(), 4);
        assert!((s[0].lng - 0.005).abs() < 1e-9);
        assert!((s[3].lng - 0.025).abs() < 1e-9);
        // Reversed and out-of-range bounds are handled.
        assert_eq!(r.slice(2_500.0, 500.0), s);
        assert_eq!(r.slice(-100.0, 99_999.0).len(), 5);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            MeasuredRoute::from_totals(line(1), 1.0, 1.0),
            Err(GeoError::TooFewPoints)
        );
        let mut c = line(3);
        c[1].lat = 200.0;
        assert_eq!(
            MeasuredRoute::from_totals(c, 1.0, 1.0),
            Err(GeoError::InvalidCoordinate(1))
        );
        let cps = [
            Checkpoint {
                index: 0,
                cum_m: 0.0,
                cum_s: 0.0,
            },
            Checkpoint {
                index: 1,
                cum_m: 500.0,
                cum_s: 50.0,
            },
            Checkpoint {
                index: 2,
                cum_m: 400.0,
                cum_s: 60.0,
            },
        ];
        assert!(MeasuredRoute::new(line(3), &cps).is_err());
    }

    #[test]
    fn duplicate_points_do_not_divide_by_zero() {
        let mut c = line(3);
        c[1] = c[0];
        let r = MeasuredRoute::from_totals(c, 1_000.0, 60.0).unwrap();
        assert!(r.cum_m().iter().all(|v| v.is_finite()));
        assert!(r.position_at_time(30.0).is_some());
    }
}
