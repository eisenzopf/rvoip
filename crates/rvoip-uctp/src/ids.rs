//! ID types + constructors used in UCTP envelopes.
//!
//! `EnvelopeId` is UCTP-specific. The other ID types are re-exported
//! from `rvoip_core::ids` so cross-crate consumers can name them as
//! `rvoip_uctp::SessionId`, etc., without depending on rvoip-core
//! directly.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

pub use rvoip_core::ids::{
    ConnectionId, ConversationId, IdentityId, MessageId, ParticipantId, SessionId, StreamId,
};

/// Globally unique envelope identifier. Format: `env_<simple-uuid>`.
///
/// CONVERSATION_PROTOCOL.md §3.1 marks the format advisory — receivers
/// must accept any string. Senders SHOULD use this shape so logs and
/// trace IDs are uniformly searchable.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct EnvelopeId(pub String);

impl EnvelopeId {
    pub fn new() -> Self {
        Self(format!("env_{}", Uuid::new_v4().simple()))
    }

    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvelopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Default for EnvelopeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Aliases for ID types referenced by name in the design doc's
/// `lib.rs` re-export sketch (§3.2). UCTP doesn't fork these — they're
/// just shorter spellings for the rvoip-core types.
pub type UctpSessionId = SessionId;
pub type UctpConnId = ConnectionId;

// --- Convenience constructors (mirror rvoip-core's `Default::default()`
// pattern; they're short enough to be ergonomic in handler code) ---

pub fn new_envelope_id() -> EnvelopeId {
    EnvelopeId::new()
}

pub fn new_conversation_id() -> ConversationId {
    ConversationId::new()
}

pub fn new_session_id() -> SessionId {
    SessionId::new()
}

pub fn new_connection_id() -> ConnectionId {
    ConnectionId::new()
}

pub fn new_stream_id() -> StreamId {
    StreamId::new()
}
