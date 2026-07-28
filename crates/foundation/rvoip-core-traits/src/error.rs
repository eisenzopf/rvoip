use std::fmt;

pub type Result<T> = std::result::Result<T, RvoipError>;

pub enum RvoipError {
    NotImplemented(&'static str),
    NoAdapterForTransport(crate::connection::Transport),
    AdapterAlreadyRegistered(crate::connection::Transport),
    ConnectionNotFound(crate::ids::ConnectionId),
    SessionNotFound(crate::ids::SessionId),
    ConversationNotFound(crate::ids::ConversationId),
    BridgeNotFound(crate::ids::BridgeId),
    AdmissionRejected(&'static str),

    /// Lifecycle precondition violated — e.g. start_session on a Closed
    /// Conversation, join_session on an Ended Session, end_session on an
    /// already-Ended Session. The message identifies which transition
    /// was rejected so callers can map it to a user-facing error.
    InvalidState(&'static str),

    /// A codec name reached `codec_to_pt` that no RTP payload-type
    /// mapping is registered for. Surfaces as a clear "this codec can't
    /// be bridged" diagnostic instead of being masked as a generic
    /// transcoder error (carries the codec name for the operator).
    UnsupportedCodec(String),
    Adapter(String),
    Other(anyhow_compat::AnyhowCompat),
}

impl RvoipError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::NotImplemented(_) => "not-implemented",
            Self::NoAdapterForTransport(_) => "adapter-missing",
            Self::AdapterAlreadyRegistered(_) => "adapter-duplicate",
            Self::ConnectionNotFound(_) => "connection-not-found",
            Self::SessionNotFound(_) => "session-not-found",
            Self::ConversationNotFound(_) => "conversation-not-found",
            Self::BridgeNotFound(_) => "bridge-not-found",
            Self::AdmissionRejected(_) => "admission-rejected",
            Self::InvalidState(_) => "invalid-state",
            Self::UnsupportedCodec(_) => "unsupported-codec",
            Self::Adapter(_) => "adapter",
            Self::Other(_) => "other",
        }
    }
}

impl fmt::Display for RvoipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "rvoip operation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for RvoipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RvoipError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for RvoipError {}

impl From<anyhow_compat::AnyhowCompat> for RvoipError {
    fn from(error: anyhow_compat::AnyhowCompat) -> Self {
        Self::Other(error)
    }
}

pub mod anyhow_compat {
    use std::error::Error as StdError;
    use std::fmt;

    pub struct AnyhowCompat(pub Box<dyn StdError + Send + Sync + 'static>);

    impl fmt::Display for AnyhowCompat {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("erased error")
        }
    }

    impl fmt::Debug for AnyhowCompat {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("AnyhowCompat([redacted])")
        }
    }

    impl StdError for AnyhowCompat {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            None
        }
    }
}
