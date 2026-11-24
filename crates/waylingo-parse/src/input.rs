use waylingo_plan::TripContext;

/// Longest query accepted, in characters.
pub const MAX_QUERY_CHARS: usize = 2_000;

/// Everything the LLM is allowed to see about a request. There is deliberately no
/// field for coordinates, routes or provider results, and no way to add one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseInput {
    query: String,
    locale: Option<String>,
    saved_place_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("the query is empty")]
    EmptyQuery,
    #[error("the query is longer than {MAX_QUERY_CHARS} characters")]
    QueryTooLong,
}

impl ParseInput {
    /// Take the query text plus the only two context fields the LLM may see:
    /// the locale and the *names* of saved places.
    ///
    /// # Errors
    /// [`InputError`] for an empty or oversized query.
    pub fn new(query: &str, ctx: &TripContext) -> Result<Self, InputError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(InputError::EmptyQuery);
        }
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(InputError::QueryTooLong);
        }
        Ok(Self {
            query: query.to_string(),
            locale: ctx.locale.clone(),
            saved_place_names: ctx.saved_places.iter().map(|p| p.name.clone()).collect(),
        })
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn user_message(&self) -> String {
        let mut m = String::new();
        if let Some(l) = &self.locale {
            m.push_str("Locale: ");
            m.push_str(l);
            m.push('\n');
        }
        if self.saved_place_names.is_empty() {
            m.push_str("Saved places: none\n");
        } else {
            let names: Vec<String> = self
                .saved_place_names
                .iter()
                .map(|n| format!("saved:{n}"))
                .collect();
            m.push_str("Saved places: ");
            m.push_str(&names.join(", "));
            m.push('\n');
        }
        m.push_str("Request: ");
        m.push_str(&self.query);
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waylingo_plan::{Coords, SavedPlace};

    #[test]
    fn keeps_names_drops_coordinates() {
        let ctx = TripContext {
            current_location: Some(Coords {
                lat: 52.3702,
                lng: 4.8952,
            }),
            saved_places: vec![SavedPlace {
                name: "home".into(),
                location: Coords {
                    lat: 52.11,
                    lng: 5.12,
                },
            }],
            locale: Some("en-GB".into()),
            ..TripContext::default()
        };
        let i = ParseInput::new("  coffee in 20 minutes ", &ctx).unwrap();
        let m = i.user_message();
        assert!(m.contains("saved:home"));
        assert!(m.contains("Locale: en-GB"));
        assert!(m.ends_with("Request: coffee in 20 minutes"));
        for leaked in ["52.37", "4.89", "52.11", "5.12"] {
            assert!(!m.contains(leaked), "coordinate {leaked} leaked");
        }
    }

    #[test]
    fn rejects_empty_and_long() {
        let ctx = TripContext::default();
        assert_eq!(ParseInput::new("   ", &ctx), Err(InputError::EmptyQuery));
        assert_eq!(
            ParseInput::new(&"a".repeat(2_001), &ctx),
            Err(InputError::QueryTooLong)
        );
    }
}
