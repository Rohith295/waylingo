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
            message: format!("HTTP {s}{}", error_detail(&resp.body)),
            retryable: resp.is_retryable(),
        })
    }
}

/// The provider's own error text (HERE `title`/`cause`, TomTom `errorText`,
/// `detailedError.message`, or a generic `message`/`error`), cut to 300 characters.
/// Words that look like keys or long tokens are masked.
fn error_detail(body: &[u8]) -> String {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let parts: Vec<&str> = [
        v["title"].as_str(),
        v["cause"].as_str(),
        v["errorText"].as_str(),
        v["detailedError"]["message"].as_str(),
        v["error"]["message"].as_str(),
        v["message"].as_str(),
        v["error"].as_str(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        return String::new();
    }
    let masked: Vec<String> = parts
        .join(" | ")
        .split_whitespace()
        .map(|w| {
            if w.len() > 40 || w.to_ascii_lowercase().contains("apikey=") || w.contains("key=") {
                "<redacted>".to_string()
            } else {
                w.to_string()
            }
        })
        .collect();
    let s: String = masked.join(" ").chars().take(300).collect();
    format!(" ({s})")
}
