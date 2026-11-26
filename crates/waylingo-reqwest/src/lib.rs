//! HTTP hosts for the Waylingo engine, built on reqwest.
//!
//! * [`drive`] / [`drive_resume`] / [`drive_parse`]: blocking, one request at a time. For scripts.
//! * [`AsyncHost`]: async, parallel within a batch, map calls paced across every
//!   query on the host, with an overall deadline. For apps and servers.
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

mod asynchronous;
mod blocking;
mod wire;

use std::time::Duration;

use waylingo::WaylingoError;

pub use asynchronous::AsyncHost;
pub use blocking::{drive, drive_parse, drive_resume};

/// The two secrets a query needs. Leave `llm` empty for a local server that needs
/// no key.
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
    #[error("the query did not finish before the deadline")]
    Timeout,
}

/// Gap between map requests in the blocking host, and the async host's default:
/// TomTom's default limit is 5 requests per second.
pub const MAP_SPACING: Duration = Duration::from_millis(220);

pub(crate) const USER_AGENT: &str = concat!("waylingo/", env!("CARGO_PKG_VERSION"));

/// Root certificates to trust, when built with `bundled-roots`.
#[cfg(feature = "bundled-roots")]
fn bundled_roots() -> Vec<reqwest::Certificate> {
    let roots: Vec<reqwest::Certificate> = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|c| reqwest::Certificate::from_der(c.as_ref()).ok())
        .collect();
    // Without roots every HTTPS request would fail with no hint why.
    debug_assert!(
        !roots.is_empty(),
        "no bundled root certificate could be read"
    );
    roots
}

/// TLS settings shared by both hosts.
pub(crate) fn tls(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    #[cfg(feature = "bundled-roots")]
    let b = b.tls_certs_only(bundled_roots());
    b
}

#[cfg(feature = "bundled-roots")]
pub(crate) fn tls_blocking(
    b: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    b.tls_certs_only(bundled_roots())
}

#[cfg(not(feature = "bundled-roots"))]
pub(crate) fn tls_blocking(
    b: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run with `--features bundled-roots -- --ignored`: proves the bundled roots
    /// complete a real TLS handshake (any HTTP status will do).
    #[cfg(feature = "bundled-roots")]
    #[test]
    #[ignore = "network"]
    fn bundled_roots_reach_https() {
        let client = tls_blocking(reqwest::blocking::Client::builder())
            .build()
            .unwrap();
        let r = client.get("https://api.tomtom.com/").send();
        assert!(
            r.is_ok(),
            "TLS failed: {:?}",
            r.err().map(|e| e.to_string())
        );
    }

    #[test]
    fn keys_debug_is_redacted() {
        let k = Keys {
            llm: "secret-llm".into(),
            map: "secret-map".into(),
        };
        let s = format!("{k:?}");
        assert!(!s.contains("secret"));
    }
}
