//! Errors raised to Python, Swift and Kotlin. Map provider and LLM failures are not
//! errors: they come back in the result (`status`, `error_code`), because the result
//! still says what was understood and what was found.

use waylingo::WaylingoError as EngineError;
use waylingo_reqwest::DriveError;

/// Each variant carries a `detail` (not `message`, which would clash with the
/// built-in `message` of Kotlin and Java exceptions).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WaylingoError {
    /// The configuration is unusable (for example, a missing model name).
    #[error("invalid configuration: {detail}")]
    InvalidConfig { detail: String },
    /// The request is unusable (empty or too long, settings out of range).
    #[error("invalid request: {detail}")]
    InvalidRequest { detail: String },
    /// The previous result cannot be continued (not a Waylingo result, another
    /// version or provider, not signed by this engine, no plan).
    #[error("cannot continue this result: {detail}")]
    IncompatibleResult { detail: String },
    /// A choice names a stop, place or candidate that the result did not offer.
    #[error("unknown choice: {detail}")]
    UnknownChoice { detail: String },
    /// The query did not finish within the configured time.
    #[error("timed out: {detail}")]
    Timeout { detail: String },
    /// A bug or an unexpected failure inside Waylingo.
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

impl From<DriveError> for WaylingoError {
    fn from(e: DriveError) -> Self {
        let detail = e.to_string();
        match e {
            DriveError::Engine(EngineError::InvalidRequest(m)) => {
                Self::InvalidRequest { detail: m }
            }
            DriveError::Engine(EngineError::IncompatibleResult(m)) => {
                Self::IncompatibleResult { detail: m }
            }
            DriveError::Engine(EngineError::UnknownChoice(m)) => Self::UnknownChoice { detail: m },
            DriveError::Timeout => Self::Timeout { detail },
            DriveError::Client | DriveError::Engine(_) => Self::Internal { detail },
        }
    }
}
