//! Decoders for the polyline formats used by the supported providers.
//!
//! * Google: Encoded Polyline Algorithm, precision 5.
//! * HERE: Flexible Polyline (via the `flexpolyline` crate); a third dimension, if present, is dropped.
//! * TomTom returns plain coordinate objects, so it needs no decoder here.

use crate::{GeoError, LatLng};

/// Decode a Google encoded polyline (precision 5).
///
/// # Errors
/// Returns [`GeoError::Decode`] on malformed input.
pub fn decode_google(encoded: &str) -> Result<Vec<LatLng>, GeoError> {
    let bytes = encoded.as_bytes();
    let mut i = 0;
    let (mut lat, mut lng) = (0_i64, 0_i64);
    let mut out = Vec::new();
    while i < bytes.len() {
        lat += next_value(bytes, &mut i)?;
        lng += next_value(bytes, &mut i)?;
        out.push(LatLng::new(lat as f64 / 1e5, lng as f64 / 1e5));
    }
    Ok(out)
}

fn next_value(bytes: &[u8], i: &mut usize) -> Result<i64, GeoError> {
    let mut result: i64 = 0;
    let mut shift = 0;
    loop {
        let Some(&b) = bytes.get(*i) else {
            return Err(GeoError::Decode("truncated Google polyline".into()));
        };
        *i += 1;
        if !(63..=126).contains(&b) {
            return Err(GeoError::Decode(format!(
                "invalid character {:?}",
                b as char
            )));
        }
        let chunk = i64::from(b - 63);
        if shift > 60 {
            return Err(GeoError::Decode("value too long".into()));
        }
        result |= (chunk & 0x1f) << shift;
        shift += 5;
        if chunk < 0x20 {
            break;
        }
    }
    Ok(if result & 1 == 1 {
        !(result >> 1)
    } else {
        result >> 1
    })
}

/// Decode a HERE Flexible Polyline.
///
/// # Errors
/// Returns [`GeoError::Decode`] on malformed input.
pub fn decode_here(encoded: &str) -> Result<Vec<LatLng>, GeoError> {
    let decoded = flexpolyline::Polyline::decode(encoded)
        .map_err(|e| GeoError::Decode(format!("flexible polyline: {e:?}")))?;
    Ok(match decoded {
        flexpolyline::Polyline::Data2d { coordinates, .. } => coordinates
            .into_iter()
            .map(|(lat, lng)| LatLng::new(lat, lng))
            .collect(),
        flexpolyline::Polyline::Data3d { coordinates, .. } => coordinates
            .into_iter()
            .map(|(lat, lng, _)| LatLng::new(lat, lng))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: LatLng, lat: f64, lng: f64, eps: f64) -> bool {
        (a.lat - lat).abs() < eps && (a.lng - lng).abs() < eps
    }

    #[test]
    fn google_reference_example() {
        // Example from Google's "Encoded Polyline Algorithm Format" documentation.
        let pts = decode_google("_p~iF~ps|U_ulLnnqC_mqNvxq`@").unwrap();
        assert_eq!(pts.len(), 3);
        assert!(close(pts[0], 38.5, -120.2, 1e-9));
        assert!(close(pts[1], 40.7, -120.95, 1e-9));
        assert!(close(pts[2], 43.252, -126.453, 1e-9));
    }

    #[test]
    fn google_rejects_garbage() {
        assert!(decode_google("_p~iF~ps|U_").is_err());
        assert!(decode_google("\u{1}").is_err());
        assert_eq!(decode_google("").unwrap(), vec![]);
    }

    #[test]
    fn here_reference_example() {
        // Test vector from the heremaps/flexible-polyline spec (precision 5).
        let pts = decode_here("BFoz5xJ67i1B1B7PzIhaxL7Y").unwrap();
        assert_eq!(pts.len(), 4);
        assert!(close(pts[0], 50.102_28, 8.698_21, 1e-5));
        assert!(close(pts[3], 50.098_78, 8.687_52, 1e-5));
    }

    #[test]
    fn here_3d_drops_third_dimension() {
        let pts = decode_here("BVoz5xJ67i1BU1B7PUzIhaUxL7YU").unwrap();
        assert_eq!(pts.len(), 4);
        assert!(close(pts[0], 50.102_28, 8.698_21, 1e-5));
    }

    #[test]
    fn here_rejects_garbage() {
        assert!(decode_here("!!!").is_err());
    }
}
