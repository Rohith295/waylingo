//! Errors raised to Python, Swift and Kotlin. Map provider and LLM failures are not
//! errors: they come back in the result (`status`, `error_code`), because the result
//! still says what was understood and what was found.

use waylingo::WaylingoError as EngineError;
use waylingo_reqwest::DriveError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WaylingoError {
    /// The configuration is unusable (for example, a missing model name).
    #[error("invalid configuration: {message}")]
    InvalidConfig { message: String },
    /// The request is unusable (empty or too long, settings out of range).
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    /// The previous result cannot be continued (not a Waylingo result, another
    /// version or provider, not signed by this engine, no plan).
    #[error("cannot continue this result: {message}")]
    IncompatibleResult { message: String },
    /// A choice names a stop, place or candidate that the result did not offer.
    #[error("unknown choice: {message}")]
    UnknownChoice { message: String },
    /// The query did not finish within the configured time.
    #[error("timed out: {message}")]
    Timeout { message: String },
    /// A bug or an unexpected failure inside Waylingo.
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl From<DriveError> for WaylingoError {
    fn from(e: DriveError) -> Self {
        let message = e.to_string();
        match e {
            DriveError::Engine(EngineError::InvalidRequest(m)) => {
                Self::InvalidRequest { message: m }
            }
            DriveError::Engine(EngineError::IncompatibleResult(m)) => {
                Self::IncompatibleResult { message: m }
            }
            DriveError::Engine(EngineError::UnknownChoice(m)) => Self::UnknownChoice { message: m },
            DriveError::Timeout => Self::Timeout { message },
            DriveError::Client | DriveError::Engine(_) => Self::Internal { message },
        }
    }
}
