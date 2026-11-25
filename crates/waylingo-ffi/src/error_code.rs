//! The error codes, as a typed enum in every language.

/// Why a result has no usable trip. Codes are only ever added; `Unknown` stands in
/// for a code from a newer version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
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
    Unknown,
}

impl From<waylingo::ErrorCode> for ErrorCode {
    fn from(c: waylingo::ErrorCode) -> Self {
        use waylingo::ErrorCode as C;
        match c {
            C::NotUnderstood => Self::NotUnderstood,
            C::NeedsDestination => Self::NeedsDestination,
            C::NeedsCurrentLocation => Self::NeedsCurrentLocation,
            C::InvalidPlan => Self::InvalidPlan,
            C::NoGeocode => Self::NoGeocode,
            C::NoRoute => Self::NoRoute,
            C::LlmAuth => Self::LlmAuth,
            C::LlmRateLimited => Self::LlmRateLimited,
            C::LlmQuotaExceeded => Self::LlmQuotaExceeded,
            C::LlmBadRequest => Self::LlmBadRequest,
            C::LlmBadResponse => Self::LlmBadResponse,
            C::LlmRefused => Self::LlmRefused,
            C::LlmUnavailable => Self::LlmUnavailable,
            C::LlmUnreachable => Self::LlmUnreachable,
            C::ProviderAuth => Self::ProviderAuth,
            C::ProviderQuotaExceeded => Self::ProviderQuotaExceeded,
            C::ProviderRateLimited => Self::ProviderRateLimited,
            C::ProviderBadRequest => Self::ProviderBadRequest,
            C::ProviderBadResponse => Self::ProviderBadResponse,
            C::ProviderUnavailable => Self::ProviderUnavailable,
            C::ProviderUnreachable => Self::ProviderUnreachable,
            C::Internal => Self::Internal,
            _ => Self::Unknown,
        }
    }
}

impl From<ErrorCode> for waylingo::ErrorCode {
    fn from(c: ErrorCode) -> Self {
        use waylingo::ErrorCode as C;
        match c {
            ErrorCode::NotUnderstood => C::NotUnderstood,
            ErrorCode::NeedsDestination => C::NeedsDestination,
            ErrorCode::NeedsCurrentLocation => C::NeedsCurrentLocation,
            ErrorCode::InvalidPlan => C::InvalidPlan,
            ErrorCode::NoGeocode => C::NoGeocode,
            ErrorCode::NoRoute => C::NoRoute,
            ErrorCode::LlmAuth => C::LlmAuth,
            ErrorCode::LlmRateLimited => C::LlmRateLimited,
            ErrorCode::LlmQuotaExceeded => C::LlmQuotaExceeded,
            ErrorCode::LlmBadRequest => C::LlmBadRequest,
            ErrorCode::LlmBadResponse => C::LlmBadResponse,
            ErrorCode::LlmRefused => C::LlmRefused,
            ErrorCode::LlmUnavailable => C::LlmUnavailable,
            ErrorCode::LlmUnreachable => C::LlmUnreachable,
            ErrorCode::ProviderAuth => C::ProviderAuth,
            ErrorCode::ProviderQuotaExceeded => C::ProviderQuotaExceeded,
            ErrorCode::ProviderRateLimited => C::ProviderRateLimited,
            ErrorCode::ProviderBadRequest => C::ProviderBadRequest,
            ErrorCode::ProviderBadResponse => C::ProviderBadResponse,
            ErrorCode::ProviderUnavailable => C::ProviderUnavailable,
            ErrorCode::ProviderUnreachable => C::ProviderUnreachable,
            ErrorCode::Internal => C::Internal,
            ErrorCode::Unknown => C::Unknown,
        }
    }
}

/// The same request may succeed later without any change: retry with backoff.
#[uniffi::export]
pub fn is_retryable(code: ErrorCode) -> bool {
    waylingo::ErrorCode::from(code).is_retryable()
}

/// The end user can fix it by saying more: rephrase, or name a place.
#[uniffi::export]
pub fn needs_user(code: ErrorCode) -> bool {
    waylingo::ErrorCode::from(code).needs_user()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_core_code_maps_to_itself() {
        for c in waylingo::ErrorCode::ALL {
            assert_eq!(waylingo::ErrorCode::from(ErrorCode::from(c)), c);
        }
    }
}
