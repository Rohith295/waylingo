//! HTTP request and response descriptions shared by the Waylingo engine and its host.
//!
//! The engine never performs network I/O. It emits [`HttpRequest`] values, the host
//! sends them (adding credentials as described by [`AuthPlacement`]), and passes the
//! results back as [`HttpResponse`] values. No type in this crate holds a secret.

use serde::{Deserialize, Serialize};

/// One HTTP header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// Which credential the host must attach to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSlot {
    /// The LLM endpoint's key.
    Llm,
    /// The configured map provider's key or token.
    Map,
}

/// Where the host must put the credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AuthPlacement {
    /// Append `?name=<key>` (or `&name=<key>`) to the URL.
    QueryParam { name: String },
    /// Add a header `name: <prefix><key>`, for example `Authorization: Bearer <key>`.
    Header { name: String, prefix: String },
}

/// A request the host must send. `url` never contains a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    /// Opaque id; echo it back in [`HttpResponse::id`].
    pub id: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<Header>,
    pub body: Option<Vec<u8>>,
    pub credential: CredentialSlot,
    pub auth: AuthPlacement,
    /// Hint: abandon the request after this many milliseconds.
    pub timeout_ms: u32,
    /// Hint: do not send before this many milliseconds have passed (retry backoff).
    pub not_before_ms: u32,
}

/// Why a request produced no HTTP response at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportError {
    Timeout,
    ConnectionFailed,
    Cancelled,
}

/// The host's answer to one [`HttpRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub id: String,
    /// HTTP status, or 0 when `transport_error` is set.
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
    pub transport_error: Option<TransportError>,
}

impl HttpResponse {
    /// True for responses worth retrying: 429, any 5xx, or a transport failure other than cancellation.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self.transport_error {
            Some(TransportError::Cancelled) => false,
            Some(_) => true,
            None => self.status == 429 || (500..600).contains(&self.status),
        }
    }

    /// Value of the first header with this name (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(status: u16, err: Option<TransportError>) -> HttpResponse {
        HttpResponse {
            id: "r1".into(),
            status,
            headers: vec![Header {
                name: "Retry-After".into(),
                value: "2".into(),
            }],
            body: vec![],
            transport_error: err,
        }
    }

    #[test]
    fn retryable_statuses() {
        assert!(resp(429, None).is_retryable());
        assert!(resp(503, None).is_retryable());
        assert!(!resp(400, None).is_retryable());
        assert!(!resp(200, None).is_retryable());
        assert!(resp(0, Some(TransportError::Timeout)).is_retryable());
        assert!(!resp(0, Some(TransportError::Cancelled)).is_retryable());
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        assert_eq!(resp(429, None).header("retry-after"), Some("2"));
        assert_eq!(resp(429, None).header("x-missing"), None);
    }
}
