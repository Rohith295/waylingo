//! Signing results, so a server can take a result back from a client and trust it.
//!
//! The signature is HMAC-SHA256 over the result serialized with `signature: null`
//! and the route line's coordinates removed, so a client may strip the (large) route
//! line before sending a result back. Serialization is deterministic (fixed field
//! order, rounded numbers, exact float round-trips), so the same document always
//! gives the same bytes.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::config::SigningKey;
use crate::result::{Geometry, Properties, TripResult};

fn mac(doc: &TripResult, key: &SigningKey) -> Hmac<Sha256> {
    let mut unsigned = TripResult {
        signature: None,
        ..doc.clone()
    };
    // The route line is not signed: a client may empty its coordinates or set its
    // geometry to null. The route feature itself must stay.
    for f in &mut unsigned.features {
        if matches!(f.properties, Properties::Route(_)) {
            f.geometry = Some(Geometry::LineString {
                coordinates: Vec::new(),
            });
        }
    }
    let mut m = Hmac::<Sha256>::new_from_slice(key.bytes()).expect("HMAC accepts any key length");
    m.update(&serde_json::to_vec(&unsigned).unwrap_or_default());
    m
}

/// The hex signature for `doc`.
pub(crate) fn sign(doc: &TripResult, key: &SigningKey) -> String {
    mac(doc, key)
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// True when `doc` carries a valid signature for `key` (compared in constant time).
pub(crate) fn verify(doc: &TripResult, key: &SigningKey) -> bool {
    let Some(hex) = &doc.signature else {
        return false;
    };
    let Some(bytes) = decode_hex(hex) else {
        return false;
    };
    mac(doc, key).verify_slice(&bytes).is_ok()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
