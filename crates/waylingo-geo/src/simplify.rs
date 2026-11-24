use crate::{EARTH_RADIUS_M, LatLng};

/// Points closer than this to the simplified line are always dropped.
const MIN_TOLERANCE_M: f64 = 0.5;

/// Simplify a line to at most `max_points` points with Douglas–Peucker, keeping
/// the first and last point. Points within [`MIN_TOLERANCE_M`] of the line are
/// always dropped; beyond that, the tolerance is found by bisection so the result
/// keeps as much detail as fits. `max_points` below 2 is treated as 2.
#[must_use]
pub fn simplify_to_max_points(points: &[LatLng], max_points: usize) -> Vec<LatLng> {
    let max_points = max_points.max(2);
    if points.len() <= 2 {
        return points.to_vec();
    }
    let fine = douglas_peucker(points, MIN_TOLERANCE_M);
    if fine.len() <= max_points {
        return fine;
    }
    let (mut lo, mut hi) = (MIN_TOLERANCE_M, MIN_TOLERANCE_M * 2.0);
    while douglas_peucker(points, hi).len() > max_points {
        hi *= 2.0;
        if hi > 1e8 {
            break;
        }
    }
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if douglas_peucker(points, mid).len() > max_points {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let kept = douglas_peucker(points, hi);
    if kept.len() <= max_points {
        kept
    } else {
        vec![points[0], points[points.len() - 1]]
    }
}

/// Douglas–Peucker with tolerance in metres, iterative to avoid deep recursion.
fn douglas_peucker(points: &[LatLng], tolerance_m: f64) -> Vec<LatLng> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0_usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut idx, mut max_d) = (a, -1.0_f64);
        for (i, p) in points.iter().enumerate().take(b).skip(a + 1) {
            let d = cross_track_m(*p, points[a], points[b]);
            if d > max_d {
                max_d = d;
                idx = i;
            }
        }
        if max_d > tolerance_m {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| *p)
        .collect()
}

/// Distance in metres from `p` to segment `a`–`b`, using a local equirectangular
/// projection (accurate for the short spans inside one search window).
fn cross_track_m(p: LatLng, a: LatLng, b: LatLng) -> f64 {
    let lat0 = a.lat.to_radians();
    let proj = |q: LatLng| {
        (
            (q.lng - a.lng).to_radians() * lat0.cos() * EARTH_RADIUS_M,
            (q.lat - a.lat).to_radians() * EARTH_RADIUS_M,
        )
    };
    let (px, py) = proj(p);
    let (bx, by) = proj(b);
    let len2 = bx * bx + by * by;
    let t = if len2 > 0.0 {
        ((px * bx + py * by) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dy) = (px - t * bx, py - t * by);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zigzag(n: usize) -> Vec<LatLng> {
        (0..n)
            .map(|i| LatLng::new(if i % 2 == 0 { 0.0 } else { 0.001 }, i as f64 * 0.001))
            .collect()
    }

    #[test]
    fn short_zigzag_unchanged() {
        // Every vertex deviates ~111 m, so nothing is dropped.
        let l = zigzag(5);
        assert_eq!(simplify_to_max_points(&l, 10), l);
    }

    #[test]
    fn respects_max_and_keeps_ends() {
        let l = zigzag(1_000);
        let s = simplify_to_max_points(&l, 250);
        assert!(s.len() <= 250 && s.len() >= 2, "{}", s.len());
        assert_eq!(s[0], l[0]);
        assert_eq!(*s.last().unwrap(), *l.last().unwrap());
    }

    #[test]
    fn straight_line_collapses_to_ends() {
        let l: Vec<LatLng> = (0..500)
            .map(|i| LatLng::new(0.0, i as f64 * 0.001))
            .collect();
        assert_eq!(simplify_to_max_points(&l, 250).len(), 2);
    }

    #[test]
    fn tiny_max_is_two() {
        assert_eq!(simplify_to_max_points(&zigzag(50), 0).len(), 2);
    }
}
