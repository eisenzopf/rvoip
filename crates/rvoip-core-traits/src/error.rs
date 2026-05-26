use thiserror::Error;

pub type Result<T> = std::result::Result<T, RvoipError>;

#[derive(Error, Debug)]
pub enum RvoipError {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error("no adapter registered for transport {0:?}")]
    NoAdapterForTransport(crate::connection::Transport),

    #[error("adapter for transport {0:?} already registered")]
    AdapterAlreadyRegistered(crate::connection::Transport),

    #[error("connection {0} not found")]
    ConnectionNotFound(crate::ids::ConnectionId),

    #[error("session {0} not found")]
    SessionNotFound(crate::ids::SessionId),

    #[error("conversation {0} not found")]
    ConversationNotFound(crate::ids::ConversationId),

    #[error("bridge {0} not found")]
    BridgeNotFound(crate::ids::BridgeId),

    #[error("admission rejected: {0}")]
    AdmissionRejected(&'static str),

    /// Lifecycle precondition violated — e.g. start_session on a Closed
    /// Conversation, join_session on an Ended Session, end_session on an
    /// already-Ended Session. The message identifies which transition
    /// was rejected so callers can map it to a user-facing error.
    #[error("invalid state: {0}")]
    InvalidState(&'static str),

    /// A codec name reached `codec_to_pt` that no RTP payload-type
    /// mapping is registered for. Surfaces as a clear "this codec can't
    /// be bridged" diagnostic instead of being masked as a generic
    /// transcoder error (carries the codec name for the operator).
    #[error("unsupported codec for bridge: {0}")]
    UnsupportedCodec(String),

    #[error("adapter error: {0}")]
    Adapter(String),

    #[error(transparent)]
    Other(#[from] anyhow_compat::AnyhowCompat),
}

pub mod anyhow_compat {
    use std::error::Error as StdError;
    use std::fmt;

    #[derive(Debug)]
    pub struct AnyhowCompat(pub Box<dyn StdError + Send + Sync + 'static>);

    impl fmt::Display for AnyhowCompat {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl StdError for AnyhowCompat {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            self.0.source()
        }
    }
}
