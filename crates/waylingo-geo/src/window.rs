use crate::{AnchorPoint, MeasuredRoute};

/// A stretch of route around an anchor, by distance from the start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub start_m: f64,
    pub end_m: f64,
}

impl Window {
    /// A window `half_m` metres either side of the anchor, clamped to the route.
    #[must_use]
    pub fn by_distance(route: &MeasuredRoute, anchor: &AnchorPoint, half_m: f64) -> Self {
        let total = route.total_m();
        let h = half_m.max(0.0);
        Self {
            start_m: (anchor.cum_m - h).clamp(0.0, total),
            end_m: (anchor.cum_m + h).clamp(0.0, total),
        }
    }

    /// A window `half_s` seconds of driving either side of the anchor, clamped to the route.
    #[must_use]
    pub fn by_time(route: &MeasuredRoute, anchor: &AnchorPoint, half_s: f64) -> Self {
        let total_s = route.total_s();
        let h = half_s.max(0.0);
        let to_m = |s: f64| {
            route
                .position_at_time(s.clamp(0.0, total_s))
                .map_or(anchor.cum_m, |p| p.cum_m)
        };
        Self {
            start_m: to_m(anchor.cum_s - h),
            end_m: to_m(anchor.cum_s + h),
        }
    }

    #[must_use]
    pub fn contains(&self, cum_m: f64) -> bool {
        cum_m >= self.start_m && cum_m <= self.end_m
    }

    #[must_use]
    pub fn length_m(&self) -> f64 {
        self.end_m - self.start_m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measured::tests::line;
    use crate::{AnchorSpec, Checkpoint, resolve};

    fn route() -> MeasuredRoute {
        // 10 km: first 5 km in 120 s, last 5 km in 480 s.
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
        MeasuredRoute::new(line(11), &cps).unwrap()
    }

    #[test]
    fn distance_window_clamps() {
        let r = route();
        let a = resolve(&r, &AnchorSpec::Fraction(0.1)).unwrap()[0];
        let w = Window::by_distance(&r, &a, 3_000.0);
        assert!((w.start_m - 0.0).abs() < 1e-9);
        assert!((w.end_m - 4_000.0).abs() < 1e-6);
        assert!(w.contains(2_000.0));
        assert!(!w.contains(4_500.0));
    }

    #[test]
    fn time_window_is_asymmetric_in_distance() {
        let r = route();
        let a = resolve(&r, &AnchorSpec::Fraction(0.5)).unwrap()[0]; // 5 km, 120 s
        let w = Window::by_time(&r, &a, 60.0);
        // 60 s back on the fast part is 2.5 km; 60 s forward on the slow part is 625 m.
        assert!((w.start_m - 2_500.0).abs() < 1e-6, "{w:?}");
        assert!((w.end_m - 5_625.0).abs() < 1e-6, "{w:?}");
    }
}
