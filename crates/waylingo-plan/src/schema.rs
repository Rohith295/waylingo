use sha2::{Digest, Sha256};

use crate::{Plan, SurfaceIntent};

/// JSON Schema of [`SurfaceIntent`], sent to the LLM for structured output.
#[must_use]
pub fn surface_intent_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(SurfaceIntent)).unwrap_or(serde_json::Value::Null)
}

/// SHA-256 of the plan's JSON form, lowercase hex. Field order is fixed by the
/// type definitions, so equal plans always hash equally.
#[must_use]
pub fn plan_hash(plan: &Plan) -> String {
    let bytes = serde_json::to_vec(plan).unwrap_or_default();
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Coords, Quantity, SurfaceAnchor, SurfaceStop, SurfaceWhat, TripContext, compile};

    #[test]
    fn schema_mentions_anchor_kinds() {
        let s = surface_intent_schema().to_string();
        for k in ["offset", "every", "fraction", "destination", "stops"] {
            assert!(s.contains(k), "schema missing {k}");
        }
    }

    #[test]
    fn hash_is_stable_and_sensitive() {
        let ctx = TripContext {
            current_location: Some(Coords { lat: 1.0, lng: 2.0 }),
            ..TripContext::default()
        };
        let mut s = SurfaceIntent {
            destination: Some("Berlin".into()),
            stops: vec![SurfaceStop {
                id: None,
                what: SurfaceWhat {
                    category: Some("coffee".into()),
                    ..Default::default()
                },
                anchor: SurfaceAnchor::Every {
                    amount: Quantity {
                        value: 2.0,
                        unit: "h".into(),
                    },
                },
                top_n: None,
            }],
            ..SurfaceIntent::default()
        };
        let a = plan_hash(&compile(&s, &ctx).unwrap());
        let b = plan_hash(&compile(&s, &ctx).unwrap());
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        s.destination = Some("Hamburg".into());
        assert_ne!(a, plan_hash(&compile(&s, &ctx).unwrap()));
    }
}
