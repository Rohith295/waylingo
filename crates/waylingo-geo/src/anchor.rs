use crate::{GeoError, LatLng, MeasuredRoute};

/// A distance in metres or a duration in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Amount {
    DistanceM(f64),
    DurationS(f64),
}

impl Amount {
    fn value(self) -> f64 {
        match self {
            Self::DistanceM(v) | Self::DurationS(v) => v,
        }
    }
}

/// Where an offset is measured from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reference {
    Start,
    End,
    /// A point already on the route, given by its distance from the start
    /// (used for waypoints).
    AtDistance(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    After,
    Before,
}

/// What to place on the route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnchorSpec {
    /// "40 minutes into the drive", "15 minutes before we arrive".
    Offset {
        from: Reference,
        direction: Direction,
        amount: Amount,
    },
    /// "Halfway" is `Fraction(0.5)`.
    Fraction(f64),
    /// "Every 200 km": marks at N, 2N, 3N… from the start. A mark is skipped when
    /// less than `min_tail` (same unit as the interval) is left after it.
    Every { interval: Amount, min_tail: f64 },
}

/// A resolved anchor on the route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorPoint {
    pub point: LatLng,
    pub cum_m: f64,
    pub cum_s: f64,
    /// 0 for single anchors; 0, 1, 2… for `Every` marks.
    pub occurrence: usize,
}

/// Resolve an anchor to one or more points on the route.
///
/// # Errors
/// [`GeoError::InvalidAnchor`] for non-positive or non-finite amounts and fractions
/// outside `(0, 1)`; [`GeoError::OutsideRoute`] when an offset lands beyond either end.
/// An `Every` anchor whose interval is longer than the route yields an empty list.
pub fn resolve(route: &MeasuredRoute, spec: &AnchorSpec) -> Result<Vec<AnchorPoint>, GeoError> {
    match *spec {
        AnchorSpec::Offset {
            from,
            direction,
            amount,
        } => {
            check_positive(amount.value(), "offset")?;
            let one = offset(route, from, direction, amount)?;
            Ok(vec![one])
        }
        AnchorSpec::Fraction(f) => {
            if !(f.is_finite() && f > 0.0 && f < 1.0) {
                return Err(GeoError::InvalidAnchor(format!(
                    "fraction {f} must be between 0 and 1"
                )));
            }
            let p = at(route, Amount::DistanceM(route.total_m() * f), 0)?;
            Ok(vec![p])
        }
        AnchorSpec::Every { interval, min_tail } => {
            check_positive(interval.value(), "interval")?;
            let total = match interval {
                Amount::DistanceM(_) => route.total_m(),
                Amount::DurationS(_) => route.total_s(),
            };
            let n = interval.value();
            let mut out = Vec::new();
            let mut k = 1_u32;
            loop {
                let v = n * f64::from(k);
                if v >= total || total - v < min_tail.max(0.0) {
                    break;
                }
                let amount = match interval {
                    Amount::DistanceM(_) => Amount::DistanceM(v),
                    Amount::DurationS(_) => Amount::DurationS(v),
                };
                out.push(at(route, amount, out.len())?);
                k += 1;
            }
            Ok(out)
        }
    }
}

fn check_positive(v: f64, what: &str) -> Result<(), GeoError> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(GeoError::InvalidAnchor(format!(
            "{what} must be a positive number, got {v}"
        )))
    }
}

fn offset(
    route: &MeasuredRoute,
    from: Reference,
    dir: Direction,
    amount: Amount,
) -> Result<AnchorPoint, GeoError> {
    let base = match from {
        Reference::Start => at(route, Amount::DistanceM(0.0), 0)?,
        Reference::End => at(route, Amount::DistanceM(route.total_m()), 0)?,
        Reference::AtDistance(m) => at(route, Amount::DistanceM(m), 0)?,
    };
    let sign = if dir == Direction::After { 1.0 } else { -1.0 };
    let target = match amount {
        Amount::DistanceM(v) => Amount::DistanceM(base.cum_m + sign * v),
        Amount::DurationS(v) => Amount::DurationS(base.cum_s + sign * v),
    };
    at(route, target, 0)
}

fn at(route: &MeasuredRoute, amount: Amount, occurrence: usize) -> Result<AnchorPoint, GeoError> {
    let (pos, total) = match amount {
        Amount::DistanceM(v) => (route.position_at_distance(v), route.total_m()),
        Amount::DurationS(v) => (route.position_at_time(v), route.total_s()),
    };
    let p = pos.ok_or(GeoError::OutsideRoute {
        value: amount.value(),
        total,
    })?;
    Ok(AnchorPoint {
        point: p.point,
        cum_m: p.cum_m,
        cum_s: p.cum_s,
        occurrence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Checkpoint;
    use crate::measured::tests::line;

    /// 650 km over 651 points, 6.5 h, with a slow last 50 km (city traffic).
    fn trip() -> MeasuredRoute {
        let cps = [
            Checkpoint {
                index: 0,
                cum_m: 0.0,
                cum_s: 0.0,
            },
            Checkpoint {
                index: 600,
                cum_m: 600_000.0,
                cum_s: 19_800.0,
            },
            Checkpoint {
                index: 650,
                cum_m: 650_000.0,
                cum_s: 23_400.0,
            },
        ];
        MeasuredRoute::new(line(651), &cps).unwrap()
    }

    #[test]
    fn minutes_after_start() {
        let a = resolve(
            &trip(),
            &AnchorSpec::Offset {
                from: Reference::Start,
                direction: Direction::After,
                amount: Amount::DurationS(40.0 * 60.0),
            },
        )
        .unwrap();
        assert_eq!(a.len(), 1);
        assert!((a[0].cum_s - 2_400.0).abs() < 1e-6);
        // Highway part averages 600 km / 19 800 s.
        assert!((a[0].cum_m - 2_400.0 * 600_000.0 / 19_800.0).abs() < 1.0);
    }

    #[test]
    fn minutes_before_end_uses_slow_city_section() {
        let a = resolve(
            &trip(),
            &AnchorSpec::Offset {
                from: Reference::End,
                direction: Direction::Before,
                amount: Amount::DurationS(15.0 * 60.0),
            },
        )
        .unwrap();
        // Last 50 km take 3 600 s, so 900 s before the end is 12.5 km before it.
        assert!((a[0].cum_m - 637_500.0).abs() < 1.0, "{:?}", a[0]);
        assert!((a[0].cum_s - 22_500.0).abs() < 1e-6);
    }

    #[test]
    fn distance_after_waypoint() {
        let a = resolve(
            &trip(),
            &AnchorSpec::Offset {
                from: Reference::AtDistance(100_000.0),
                direction: Direction::After,
                amount: Amount::DistanceM(10_000.0),
            },
        )
        .unwrap();
        assert!((a[0].cum_m - 110_000.0).abs() < 1e-6);
    }

    #[test]
    fn offsets_outside_route_are_errors() {
        let r = trip();
        let err = resolve(
            &r,
            &AnchorSpec::Offset {
                from: Reference::Start,
                direction: Direction::Before,
                amount: Amount::DistanceM(1.0),
            },
        );
        assert!(matches!(err, Err(GeoError::OutsideRoute { .. })));
        let err = resolve(
            &r,
            &AnchorSpec::Offset {
                from: Reference::Start,
                direction: Direction::After,
                amount: Amount::DurationS(99_999.0),
            },
        );
        assert!(matches!(err, Err(GeoError::OutsideRoute { .. })));
    }

    #[test]
    fn halfway() {
        let a = resolve(&trip(), &AnchorSpec::Fraction(0.5)).unwrap();
        assert!((a[0].cum_m - 325_000.0).abs() < 1e-6);
        assert!(resolve(&trip(), &AnchorSpec::Fraction(1.0)).is_err());
        assert!(resolve(&trip(), &AnchorSpec::Fraction(0.0)).is_err());
    }

    #[test]
    fn every_200_km_keeps_600_km_mark() {
        // Window ±15% of 200 km = 30 km half-width; 50 km are left after 600 km.
        let a = resolve(
            &trip(),
            &AnchorSpec::Every {
                interval: Amount::DistanceM(200_000.0),
                min_tail: 30_000.0,
            },
        )
        .unwrap();
        let marks: Vec<f64> = a.iter().map(|p| p.cum_m).collect();
        assert_eq!(marks.len(), 3, "{marks:?}");
        assert!((marks[2] - 600_000.0).abs() < 1e-6);
        assert_eq!(
            a.iter().map(|p| p.occurrence).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn every_skips_mark_too_close_to_end() {
        let a = resolve(
            &trip(),
            &AnchorSpec::Every {
                interval: Amount::DistanceM(200_000.0),
                min_tail: 60_000.0,
            },
        )
        .unwrap();
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn every_two_hours_by_time() {
        let a = resolve(
            &trip(),
            &AnchorSpec::Every {
                interval: Amount::DurationS(7_200.0),
                min_tail: 0.0,
            },
        )
        .unwrap();
        // 6.5 h trip: marks at 2 h, 4 h, 6 h.
        assert_eq!(a.len(), 3);
        assert!((a[2].cum_s - 21_600.0).abs() < 1e-6);
    }

    #[test]
    fn every_longer_than_route_is_empty() {
        let a = resolve(
            &trip(),
            &AnchorSpec::Every {
                interval: Amount::DistanceM(1_000_000.0),
                min_tail: 0.0,
            },
        )
        .unwrap();
        assert!(a.is_empty());
    }

    #[test]
    fn non_positive_amounts_rejected() {
        let r = trip();
        assert!(
            resolve(
                &r,
                &AnchorSpec::Every {
                    interval: Amount::DistanceM(0.0),
                    min_tail: 0.0
                }
            )
            .is_err()
        );
        assert!(
            resolve(
                &r,
                &AnchorSpec::Offset {
                    from: Reference::Start,
                    direction: Direction::After,
                    amount: Amount::DurationS(f64::NAN),
                },
            )
            .is_err()
        );
    }
}
