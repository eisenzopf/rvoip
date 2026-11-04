use rvoip_core::error::RvoipError;

pub type Result<T> = std::result::Result<T, A2aError>;

/// Errors intentionally contain no peer payloads, credentials, or transcripts.
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum A2aError {
    #[error("invalid A2A configuration")]
    Configuration,
    #[error("invalid A2A request")]
    Protocol,
    #[error("unsupported A2A operation")]
    Unsupported,
    #[error("operation is invalid in the current A2A state")]
    State,
    #[error("A2A payload or queue limit exceeded")]
    Capacity,
    #[error("A2A operation timed out")]
    Timeout,
    #[error("A2A connection closed")]
    Closed,
    #[error("invalid or incomplete turn trace")]
    Trace,
    #[error("A2A context is already active")]
    DuplicateContext,
}

impl From<A2aError> for RvoipError {
    fn from(error: A2aError) -> Self {
        Self::Adapter(error.to_string())
    }
}
