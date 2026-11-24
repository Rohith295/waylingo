use serde::{Deserialize, Serialize};
use waylingo_http::HttpResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorKind {
    Auth,
    QuotaExceeded,
    RateLimited,
    NoRoute,
    BadRequest,
    Upstream,
    Transport,
    BadResponse,
}

/// A provider failure. The message never contains a key: URLs are not included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct ProviderError {
    pub kind: ErrorKind,
    pub message: String,
    pub retryable: bool,
}

impl ProviderError {
    pub(crate) fn bad_response(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::BadResponse,
            message: message.into(),
            retryable: false,
        }
    }

    /// Classify a non-2xx response or transport failure. Returns `None` for 2xx.
    pub(crate) fn from_status(resp: &HttpResponse) -> Option<Self> {
        if let Some(t) = resp.transport_error {
            return Some(Self {
                kind: ErrorKind::Transport,
                message: format!("{t:?}"),
                retryable: resp.is_retryable(),
            });
        }
        let s = resp.status;
        let kind = match s {
            200..=299 => return None,
            401 | 403 => ErrorKind::Auth,
            429 => ErrorKind::RateLimited,
            402 => ErrorKind::QuotaExceeded,
            500..=599 => ErrorKind::Upstream,
            _ => ErrorKind::BadRequest,
        };
        Some(Self {
            kind,
            message: format!("HTTP {s}"),
            retryable: resp.is_retryable(),
        })
    }
}
