use waylingo_plan::TripContext;

/// Longest query accepted, in characters.
pub const MAX_QUERY_CHARS: usize = 2_000;
/// Most saved places passed with one request; their names go into the prompt.
pub const MAX_SAVED_PLACES: usize = 50;
/// Longest saved-place name, in characters.
pub const MAX_SAVED_NAME_CHARS: usize = 60;
/// Longest locale tag, in characters (BCP 47 tags are far shorter).
pub const MAX_LOCALE_CHARS: usize = 35;

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
    #[error("more than {MAX_SAVED_PLACES} saved places")]
    TooManySavedPlaces,
    #[error("a saved place name is empty or longer than {MAX_SAVED_NAME_CHARS} characters")]
    BadSavedPlaceName,
    #[error("the locale is longer than {MAX_LOCALE_CHARS} characters")]
    LocaleTooLong,
    /// Latitude outside -90..90, longitude outside -180..180, or not a number.
    #[error("{0} has invalid coordinates")]
    InvalidCoordinates(&'static str),
}

impl ParseInput {
    /// Take the query text plus the only two context fields the LLM may see:
    /// the locale and the *names* of saved places.
    ///
    /// # Errors
    /// [`InputError`] for an empty or oversized query, oversized context, or
    /// coordinates out of range.
    pub fn new(query: &str, ctx: &TripContext) -> Result<Self, InputError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(InputError::EmptyQuery);
        }
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(InputError::QueryTooLong);
        }
        if ctx.saved_places.len() > MAX_SAVED_PLACES {
            return Err(InputError::TooManySavedPlaces);
        }
        for p in &ctx.saved_places {
            let n = p.name.trim().chars().count();
            if n == 0 || n > MAX_SAVED_NAME_CHARS {
                return Err(InputError::BadSavedPlaceName);
            }
            if !p.location.is_valid() {
                return Err(InputError::InvalidCoordinates("a saved place"));
            }
        }
        if ctx
            .locale
            .as_ref()
            .is_some_and(|l| l.chars().count() > MAX_LOCALE_CHARS)
        {
            return Err(InputError::LocaleTooLong);
        }
        if ctx.current_location.is_some_and(|c| !c.is_valid()) {
            return Err(InputError::InvalidCoordinates("current_location"));
        }
        if ctx.active_destination.is_some_and(|c| !c.is_valid()) {
            return Err(InputError::InvalidCoordinates("active_destination"));
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

    /// The user message sent to the LLM (locale, saved-place names, request text).
    /// Public so training data uses exactly the same format.
    #[must_use]
    pub fn user_message(&self) -> String {
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

    #[test]
    fn rejects_bad_context() {
        let bad = Coords {
            lat: 95.0,
            lng: f64::NAN,
        };
        let with = |f: &dyn Fn(&mut TripContext)| {
            let mut c = TripContext::default();
            f(&mut c);
            ParseInput::new("drive to Utrecht", &c).unwrap_err()
        };
        assert_eq!(
            with(&|c| c.current_location = Some(bad)),
            InputError::InvalidCoordinates("current_location")
        );
        assert_eq!(
            with(&|c| c.active_destination = Some(bad)),
            InputError::InvalidCoordinates("active_destination")
        );
        assert_eq!(
            with(&|c| c.locale = Some("x".repeat(40))),
            InputError::LocaleTooLong
        );
        let place = |name: &str| SavedPlace {
            name: name.into(),
            location: Coords { lat: 1.0, lng: 1.0 },
        };
        assert_eq!(
            with(&|c| c.saved_places = vec![place(" ")]),
            InputError::BadSavedPlaceName
        );
        assert_eq!(
            with(&|c| c.saved_places = (0..51).map(|i| place(&format!("p{i}"))).collect()),
            InputError::TooManySavedPlaces
        );
    }
}
