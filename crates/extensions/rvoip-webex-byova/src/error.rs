use rvoip_core::error::RvoipError;

pub type Result<T> = std::result::Result<T, WebexError>;

/// Adapter errors. They never carry Webex payloads, tokens, or transcripts.
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum WebexError {
    #[error("invalid Webex BYOVA configuration")]
    Configuration,
    #[error("Webex BYOVA authentication failed")]
    Unauthorized,
    #[error("Webex BYOVA signing keys are unavailable or expired")]
    KeysUnavailable,
    #[error("invalid Webex BYOVA request")]
    Protocol,
    #[error("unsupported Webex BYOVA operation")]
    Unsupported,
    #[error("operation is invalid in the current Webex BYOVA state")]
    State,
    #[error("utterance shorter than the qualified minimum chunk")]
    ShortAudio,
    #[error("Webex BYOVA payload or queue limit exceeded")]
    Capacity,
    #[error("Webex BYOVA operation timed out")]
    Timeout,
    #[error("Webex BYOVA connection closed")]
    Closed,
    #[error("Webex BYOVA conversation is already active")]
    DuplicateConversation,
    #[error("Webex BYOVA admission was rejected")]
    Rejected,
}

impl From<WebexError> for RvoipError {
    fn from(error: WebexError) -> Self {
        Self::Adapter(error.to_string())
    }
}

impl From<crate::protocol::ProtocolError> for WebexError {
    fn from(error: crate::protocol::ProtocolError) -> Self {
        match error {
            crate::protocol::ProtocolError::ShortAudio => Self::ShortAudio,
            crate::protocol::ProtocolError::Limit => Self::Capacity,
            crate::protocol::ProtocolError::StaleReply | crate::protocol::ProtocolError::State => {
                Self::State
            }
            _ => Self::Protocol,
        }
    }
}
