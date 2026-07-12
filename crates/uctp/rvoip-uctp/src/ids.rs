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

/// Maximum UTF-8 byte length accepted for a wire envelope identifier.
/// Envelope IDs are ASCII by grammar, so this is also the character bound.
pub const MAX_ENVELOPE_ID_BYTES: usize = 128;

/// Validate the bounded wire grammar for an envelope or correlation ID.
///
/// IDs may use any non-empty URI-unreserved ASCII string. The generated
/// `env_<simple-uuid>` form is canonical, while ULIDs and application-defined
/// identifiers remain interoperable as required by the protocol.
pub fn validate_envelope_id(value: &str) -> Result<(), &'static str> {
    if value.len() > MAX_ENVELOPE_ID_BYTES {
        return Err("envelope id exceeds 128 bytes");
    }
    if value.is_empty() {
        return Err("envelope id is empty");
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
    {
        return Err("envelope id contains invalid characters");
    }
    Ok(())
}

/// Globally unique envelope identifier. Format: `env_<simple-uuid>`.
///
/// The canonical UUID suffix is generated locally. Receivers accept the
/// bounded URI-unreserved grammar documented by [`validate_envelope_id`] so
/// replay keys and correlation lookups cannot retain attacker-controlled
/// strings of arbitrary size.
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

    /// Validate this ID for use on the UCTP wire.
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_envelope_id(&self.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_id_accepts_generated_ulid_and_readable_forms() {
        assert!(EnvelopeId::new().validate().is_ok());
        assert!(validate_envelope_id("01HXYZ0123456789ABCDEFGHJK").is_ok());
        assert!(validate_envelope_id("env_request-1.retry_2~a").is_ok());
        assert!(validate_envelope_id("request_1").is_ok());
    }

    #[test]
    fn envelope_id_rejects_empty_and_unsafe_characters() {
        for invalid in ["", "env_has space", "env_line\nbreak"] {
            assert!(
                validate_envelope_id(invalid).is_err(),
                "unexpectedly accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn envelope_id_rejects_values_over_byte_limit() {
        let oversized = format!("env_{}", "a".repeat(MAX_ENVELOPE_ID_BYTES - 3));
        assert_eq!(oversized.len(), MAX_ENVELOPE_ID_BYTES + 1);
        assert!(validate_envelope_id(&oversized).is_err());
        let boundary = format!("env_{}", "a".repeat(MAX_ENVELOPE_ID_BYTES - 4));
        assert_eq!(boundary.len(), MAX_ENVELOPE_ID_BYTES);
        assert!(validate_envelope_id(&boundary).is_ok());
    }
}
