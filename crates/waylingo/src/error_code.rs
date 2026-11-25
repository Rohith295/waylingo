//! Why a trip failed: one fixed list of codes, so apps can handle every case.

use serde::{Deserialize, Serialize};
use waylingo_parse::FailureCode;
use waylingo_providers::ErrorKind;

/// Why a result has no usable trip. Serialized in `SCREAMING_SNAKE_CASE`
/// (`"NEEDS_DESTINATION"`). Codes are only ever added, never renamed or removed;
/// [`ErrorCode::Unknown`] stands in for a code from a newer version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum ErrorCode {
    // The request: ask the end user.
    /// Not a trip request. Ask the end user to rephrase.
    NotUnderstood,
    /// No destination was named and no active route was passed. Ask "where to?".
    NeedsDestination,
    /// The trip starts at the current location, which the app did not pass. Pass
    /// `current_location`, or ask the end user where to start.
    NeedsCurrentLocation,
    /// The LLM's plan was still invalid after the repair attempts. Ask the end user
    /// to rephrase; if it happens often, try a stronger model.
    InvalidPlan,
    /// No place was found for a location in the request. Show the text that was
    /// searched and ask for a more precise one.
    NoGeocode,
    /// The map provider found no route between the locations (for example, across
    /// water with no ferry).
    NoRoute,

    // The LLM endpoint: fix the configuration, or retry later.
    /// 401 or 403: the LLM key is wrong, expired, or lacks access to the model.
    LlmAuth,
    /// Still rate-limited (429) after the retries. Retry later or raise the limit.
    LlmRateLimited,
    /// The LLM account's quota or credit is used up. Add credit or raise the limit.
    LlmQuotaExceeded,
    /// Another 4xx, usually a wrong or retired model name or an unsupported
    /// parameter. Retrying will not help.
    LlmBadRequest,
    /// The endpoint answered in a format Waylingo cannot read: its API changed, or
    /// it is not OpenAI-compatible.
    LlmBadResponse,
    /// The model declined the request.
    LlmRefused,
    /// 5xx or timeouts after the retries. Retry later.
    LlmUnavailable,
    /// The LLM endpoint could not be reached (network, DNS or TLS). Check
    /// connectivity; on a phone, often no signal.
    LlmUnreachable,

    // The map provider: fix the configuration, or retry later.
    /// 401 or 403: the map key is wrong, expired, or lacks a needed product.
    ProviderAuth,
    /// The map plan's quota is used up. Only reported when the provider says so
    /// clearly; some providers report it as `ProviderAuth` or `ProviderRateLimited`.
    ProviderQuotaExceeded,
    /// Still rate-limited (429) after the retries. Retry later or lower
    /// `map_requests_per_second`.
    ProviderRateLimited,
    /// The provider rejected a request (another 4xx).
    ProviderBadRequest,
    /// The provider answered in a format Waylingo cannot read: its API changed.
    ProviderBadResponse,
    /// 5xx from the provider after the retries. Retry later.
    ProviderUnavailable,
    /// The provider could not be reached (network, DNS or TLS).
    ProviderUnreachable,

    /// A bug in Waylingo. Please report it with the `message`.
    Internal,
    /// A code this version does not know, from a result made by a newer version.
    #[serde(other)]
    Unknown,
}

impl ErrorCode {
    /// Every code, for documentation and exhaustive handling in tests.
    pub const ALL: [Self; 23] = [
        Self::NotUnderstood,
        Self::NeedsDestination,
        Self::NeedsCurrentLocation,
        Self::InvalidPlan,
        Self::NoGeocode,
        Self::NoRoute,
        Self::LlmAuth,
        Self::LlmRateLimited,
        Self::LlmQuotaExceeded,
        Self::LlmBadRequest,
        Self::LlmBadResponse,
        Self::LlmRefused,
        Self::LlmUnavailable,
        Self::LlmUnreachable,
        Self::ProviderAuth,
        Self::ProviderQuotaExceeded,
        Self::ProviderRateLimited,
        Self::ProviderBadRequest,
        Self::ProviderBadResponse,
        Self::ProviderUnavailable,
        Self::ProviderUnreachable,
        Self::Internal,
        Self::Unknown,
    ];

    /// The same request may succeed later without any change.
    #[must_use]
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::LlmRateLimited
                | Self::LlmUnavailable
                | Self::LlmUnreachable
                | Self::ProviderRateLimited
                | Self::ProviderUnavailable
                | Self::ProviderUnreachable
        )
    }

    /// The end user can fix it by saying more (rephrase, name a place). For
    /// `NeedsCurrentLocation` the app may instead pass the location itself.
    #[must_use]
    pub fn needs_user(self) -> bool {
        matches!(
            self,
            Self::NotUnderstood
                | Self::LlmRefused
                | Self::NeedsDestination
                | Self::NeedsCurrentLocation
                | Self::InvalidPlan
                | Self::NoGeocode
                | Self::NoRoute
        )
    }
}

impl From<FailureCode> for ErrorCode {
    fn from(c: FailureCode) -> Self {
        match c {
            FailureCode::InvalidPlan => Self::InvalidPlan,
            FailureCode::LlmAuth => Self::LlmAuth,
            FailureCode::LlmRateLimited => Self::LlmRateLimited,
            FailureCode::LlmQuotaExceeded => Self::LlmQuotaExceeded,
            FailureCode::LlmUnreachable => Self::LlmUnreachable,
            FailureCode::Internal => Self::Internal,
            FailureCode::LlmBadRequest => Self::LlmBadRequest,
            FailureCode::LlmBadResponse => Self::LlmBadResponse,
            FailureCode::LlmUnavailable => Self::LlmUnavailable,
            FailureCode::LlmRefused => Self::LlmRefused,
            FailureCode::NotUnderstood => Self::NotUnderstood,
            FailureCode::NeedsCurrentLocation => Self::NeedsCurrentLocation,
            FailureCode::NeedsDestination => Self::NeedsDestination,
        }
    }
}

impl From<ErrorKind> for ErrorCode {
    fn from(k: ErrorKind) -> Self {
        match k {
            ErrorKind::Auth => Self::ProviderAuth,
            ErrorKind::QuotaExceeded => Self::ProviderQuotaExceeded,
            ErrorKind::RateLimited => Self::ProviderRateLimited,
            ErrorKind::NoRoute => Self::NoRoute,
            ErrorKind::BadRequest => Self::ProviderBadRequest,
            ErrorKind::Upstream => Self::ProviderUnavailable,
            ErrorKind::Transport => Self::ProviderUnreachable,
            ErrorKind::BadResponse => Self::ProviderBadResponse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire names are a public contract: changing one breaks apps.
    #[test]
    fn wire_names_are_stable() {
        let names: Vec<String> = ErrorCode::ALL
            .iter()
            .map(|c| {
                serde_json::to_value(c)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            [
                "NOT_UNDERSTOOD",
                "NEEDS_DESTINATION",
                "NEEDS_CURRENT_LOCATION",
                "INVALID_PLAN",
                "NO_GEOCODE",
                "NO_ROUTE",
                "LLM_AUTH",
                "LLM_RATE_LIMITED",
                "LLM_QUOTA_EXCEEDED",
                "LLM_BAD_REQUEST",
                "LLM_BAD_RESPONSE",
                "LLM_REFUSED",
                "LLM_UNAVAILABLE",
                "LLM_UNREACHABLE",
                "PROVIDER_AUTH",
                "PROVIDER_QUOTA_EXCEEDED",
                "PROVIDER_RATE_LIMITED",
                "PROVIDER_BAD_REQUEST",
                "PROVIDER_BAD_RESPONSE",
                "PROVIDER_UNAVAILABLE",
                "PROVIDER_UNREACHABLE",
                "INTERNAL",
                "UNKNOWN",
            ]
        );
    }

    /// A new variant must be added to `ALL` too, or bindings would see `Unknown`.
    /// The match has no wildcard, so a new variant fails to compile here.
    #[test]
    fn all_lists_every_variant() {
        let index = |c: ErrorCode| -> usize {
            match c {
                ErrorCode::NotUnderstood => 0,
                ErrorCode::NeedsDestination => 1,
                ErrorCode::NeedsCurrentLocation => 2,
                ErrorCode::InvalidPlan => 3,
                ErrorCode::NoGeocode => 4,
                ErrorCode::NoRoute => 5,
                ErrorCode::LlmAuth => 6,
                ErrorCode::LlmRateLimited => 7,
                ErrorCode::LlmQuotaExceeded => 8,
                ErrorCode::LlmBadRequest => 9,
                ErrorCode::LlmBadResponse => 10,
                ErrorCode::LlmRefused => 11,
                ErrorCode::LlmUnavailable => 12,
                ErrorCode::LlmUnreachable => 13,
                ErrorCode::ProviderAuth => 14,
                ErrorCode::ProviderQuotaExceeded => 15,
                ErrorCode::ProviderRateLimited => 16,
                ErrorCode::ProviderBadRequest => 17,
                ErrorCode::ProviderBadResponse => 18,
                ErrorCode::ProviderUnavailable => 19,
                ErrorCode::ProviderUnreachable => 20,
                ErrorCode::Internal => 21,
                ErrorCode::Unknown => 22,
            }
        };
        for (i, c) in ErrorCode::ALL.iter().enumerate() {
            assert_eq!(index(*c), i, "{c:?} is out of place in ALL");
        }
    }

    #[test]
    fn retry_and_ask_user_do_not_overlap() {
        for c in ErrorCode::ALL {
            assert!(!(c.is_retryable() && c.needs_user()), "{c:?}");
        }
    }

    #[test]
    fn unknown_codes_from_newer_versions_are_read() {
        let c: ErrorCode = serde_json::from_str(r#""SOMETHING_NEW""#).unwrap();
        assert_eq!(c, ErrorCode::Unknown);
    }
}
