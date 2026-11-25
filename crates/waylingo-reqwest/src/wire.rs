//! Turning engine requests into HTTP and HTTP back into engine responses. Shared by
//! the blocking and async hosts. Keys are added here, at send time, and nothing in
//! this module logs or returns a URL (a URL may carry a key).

use std::fmt::Write as _;
use std::time::Duration;

use waylingo::{AuthPlacement, CredentialSlot, Header, HttpRequest, HttpResponse, TransportError};

use crate::Keys;

/// One request, ready to send.
pub(crate) struct Prepared {
    pub post: bool,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

/// Add the right key in the right place. An empty LLM key (a local server that
/// needs none) adds nothing.
pub(crate) fn prepare(req: &HttpRequest, keys: &Keys) -> Prepared {
    let key = match req.credential {
        CredentialSlot::Llm => keys.llm.as_str(),
        CredentialSlot::Map => keys.map.as_str(),
    };
    let mut url = req.url.clone();
    let mut headers: Vec<(String, String)> = req
        .headers
        .iter()
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect();
    if !key.is_empty() {
        match &req.auth {
            AuthPlacement::QueryParam { name } => {
                let sep = if url.contains('?') { '&' } else { '?' };
                url = format!("{url}{sep}{name}={}", encode(key));
            }
            AuthPlacement::Header { name, prefix } => {
                headers.push((name.clone(), format!("{prefix}{key}")));
            }
        }
    }
    Prepared {
        post: req.method == "POST",
        url,
        headers,
        body: req.body.clone(),
        timeout: Duration::from_millis(u64::from(req.timeout_ms.max(1_000))),
    }
}

/// An engine response from an HTTP status, the `Retry-After` header and the body.
pub(crate) fn response(
    id: &str,
    status: u16,
    retry_after: Option<String>,
    body: Vec<u8>,
) -> HttpResponse {
    HttpResponse {
        id: id.to_string(),
        status,
        headers: retry_after
            .map(|v| Header {
                name: "Retry-After".into(),
                value: v,
            })
            .into_iter()
            .collect(),
        body,
        transport_error: None,
    }
}

/// An engine response for a request that got no HTTP answer.
pub(crate) fn transport_failure(id: &str, timed_out: bool) -> HttpResponse {
    HttpResponse {
        id: id.to_string(),
        status: 0,
        headers: vec![],
        body: vec![],
        transport_error: Some(if timed_out {
            TransportError::Timeout
        } else {
            TransportError::ConnectionFailed
        }),
    }
}

/// Percent-encode a key for a query string.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(credential: CredentialSlot, auth: AuthPlacement) -> HttpRequest {
        HttpRequest {
            id: "r".into(),
            method: "GET".into(),
            url: "https://x.example/a?b=1".into(),
            headers: vec![],
            body: None,
            credential,
            auth,
            timeout_ms: 0,
            not_before_ms: 0,
        }
    }

    #[test]
    fn keys_go_where_the_request_says() {
        let keys = Keys {
            llm: "l+1".into(),
            map: "m/2".into(),
        };
        let q = prepare(
            &req(
                CredentialSlot::Map,
                AuthPlacement::QueryParam { name: "key".into() },
            ),
            &keys,
        );
        assert_eq!(q.url, "https://x.example/a?b=1&key=m%2F2");
        let h = prepare(
            &req(
                CredentialSlot::Llm,
                AuthPlacement::Header {
                    name: "Authorization".into(),
                    prefix: "Bearer ".into(),
                },
            ),
            &keys,
        );
        assert_eq!(
            h.headers,
            vec![("Authorization".into(), "Bearer l+1".into())]
        );
        assert_eq!(
            h.timeout,
            Duration::from_millis(1_000),
            "at least one second"
        );
    }

    #[test]
    fn an_empty_llm_key_adds_no_header() {
        let keys = Keys {
            llm: String::new(),
            map: "m".into(),
        };
        let h = prepare(
            &req(
                CredentialSlot::Llm,
                AuthPlacement::Header {
                    name: "Authorization".into(),
                    prefix: "Bearer ".into(),
                },
            ),
            &keys,
        );
        assert!(h.headers.is_empty());
    }
}
