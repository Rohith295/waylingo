//! A blocking HTTP host for the Waylingo engine, for Rust programs.
//!
//! ```no_run
//! # use waylingo::*;
//! # fn demo(engine: &Engine, request: TripRequest) {
//! let keys = waylingo_reqwest::Keys { llm: "llm-key".into(), map: "map-key".into() };
//! let geojson = waylingo_reqwest::drive(engine, request, &keys).unwrap();
//! # }
//! ```
//!
//! Keys are added to each request only as its `auth` field says, at send time, and
//! are never logged. Errors never include URLs (a URL may carry a key).

use std::time::Duration;

use waylingo::{
    AuthPlacement, CredentialSlot, Engine, HttpRequest, HttpResponse, Step, TransportError,
    TripRequest, WaylingoError,
};

/// The two secrets a query needs.
pub struct Keys {
    pub llm: String,
    pub map: String,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys { llm: <redacted>, map: <redacted> }")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DriveError {
    #[error(transparent)]
    Engine(#[from] WaylingoError),
    #[error("could not create the HTTP client")]
    Client,
}

/// Gap between map requests: TomTom's default limit is 5 requests per second.
const MAP_SPACING: Duration = Duration::from_millis(220);

/// Run one query to completion and return the GeoJSON.
///
/// # Errors
/// Engine misuse or a failure to create the HTTP client. Provider and LLM failures
/// are reported inside the GeoJSON, not as errors.
pub fn drive(engine: &Engine, request: TripRequest, keys: &Keys) -> Result<String, DriveError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("waylingo/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| DriveError::Client)?;
    let (mut session, mut step) = engine.start(request)?;
    loop {
        match step {
            Step::Done { geojson } => return Ok(geojson),
            Step::NeedRequests { requests } => {
                let responses = requests.iter().map(|r| send(&client, r, keys)).collect();
                step = session.step(responses)?;
            }
        }
    }
}

fn send(client: &reqwest::blocking::Client, req: &HttpRequest, keys: &Keys) -> HttpResponse {
    let key = match req.credential {
        CredentialSlot::Llm => keys.llm.as_str(),
        CredentialSlot::Map => keys.map.as_str(),
    };
    let mut wait = Duration::from_millis(u64::from(req.not_before_ms));
    if req.credential == CredentialSlot::Map {
        wait += MAP_SPACING;
    }
    std::thread::sleep(wait);

    let url = match &req.auth {
        AuthPlacement::QueryParam { name } => {
            let sep = if req.url.contains('?') { '&' } else { '?' };
            format!("{}{sep}{name}={}", req.url, encode(key))
        }
        AuthPlacement::Header { .. } => req.url.clone(),
    };
    let mut b = match req.method.as_str() {
        "POST" => client.post(&url),
        _ => client.get(&url),
    }
    .timeout(Duration::from_millis(u64::from(req.timeout_ms.max(1_000))));
    for h in &req.headers {
        b = b.header(&h.name, &h.value);
    }
    if let AuthPlacement::Header { name, prefix } = &req.auth {
        b = b.header(name, format!("{prefix}{key}"));
    }
    if let Some(body) = &req.body {
        b = b.body(body.clone());
    }
    match b.send() {
        Ok(r) => {
            let status = r.status().as_u16();
            let headers = r
                .headers()
                .iter()
                .filter(|(n, _)| n.as_str().eq_ignore_ascii_case("retry-after"))
                .filter_map(|(n, v)| {
                    Some(waylingo::Header {
                        name: n.to_string(),
                        value: v.to_str().ok()?.to_string(),
                    })
                })
                .collect();
            let body = r.bytes().map(|b| b.to_vec()).unwrap_or_default();
            HttpResponse {
                id: req.id.clone(),
                status,
                headers,
                body,
                transport_error: None,
            }
        }
        Err(e) => HttpResponse {
            id: req.id.clone(),
            status: 0,
            headers: vec![],
            body: vec![],
            transport_error: Some(if e.is_timeout() {
                TransportError::Timeout
            } else {
                TransportError::ConnectionFailed
            }),
        },
    }
}

/// Percent-encode a key for a query string.
fn encode(s: &str) -> String {
    use std::fmt::Write;
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

    #[test]
    fn keys_debug_is_redacted() {
        let k = Keys {
            llm: "secret-llm".into(),
            map: "secret-map".into(),
        };
        let s = format!("{k:?}");
        assert!(!s.contains("secret"));
    }

    #[test]
    fn encodes_keys() {
        assert_eq!(encode("ab-c_9"), "ab-c_9");
        assert_eq!(encode("a+b/c"), "a%2Bb%2Fc");
    }
}
