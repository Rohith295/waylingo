//! Map provider adapters for Waylingo.
//!
//! Each adapter is pure: it builds [`waylingo_http::HttpRequest`]s and parses
//! [`waylingo_http::HttpResponse`]s. It never sends anything itself and never holds a
//! key; requests carry [`waylingo_http::CredentialSlot::Map`] and the host adds the
//! key. Results from this crate are provider data and must never be passed to the
//! LLM crate. This crate does not depend on it, and `scripts/check_deps.sh` keeps
//! it that way.

mod category;
mod encode;
mod error;
mod tomtom;
mod types;

pub use error::{ErrorKind, ProviderError};
pub use tomtom::TomTom;
pub use types::{
    Candidate, Capabilities, DetourStrategy, GeocodeCandidate, ProviderAdapter, ProviderKind,
    RouteQuery, RouteResult, SearchQuery,
};
