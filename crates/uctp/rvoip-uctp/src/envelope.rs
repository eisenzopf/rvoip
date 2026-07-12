//! `UctpEnvelope<T>` — the wire-level envelope all UCTP messages carry.
//!
//! Per CONVERSATION_PROTOCOL.md §3.1, every envelope on the wire has the
//! same outer shape with a typed payload. Two-layer typing: decode first
//! to `UctpEnvelope<serde_json::Value>` (forward-compat — unknown payload
//! fields are tolerated) and then call [`UctpEnvelope::decode_payload`]
//! to typed structs in [`crate::payloads`] on demand.

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::compatibility::UCTP_ENVELOPE_VERSION;
use crate::errors::UctpError;
use crate::types::MessageType;

/// UCTP envelope with a generic payload `T` (defaults to
/// `serde_json::Value` for two-layer decoding — see module docs).
///
/// Field order matches CONVERSATION_PROTOCOL.md §3.1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UctpEnvelope<T = serde_json::Value> {
    /// Protocol version. v0 = 1.
    pub v: u8,

    /// Dotted message type. Wire field name is `type`.
    #[serde(rename = "type")]
    pub msg_type: MessageType,

    /// Globally-unique envelope ID (format: `env_<simple-uuid>`).
    pub id: String,

    /// Sender's clock; receivers do not trust it for ordering.
    pub ts: DateTime<Utc>,

    /// Conversation ID. `None` only for connection-level envelopes (auth, keepalive).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cid: Option<String>,

    /// Session ID. `None` when not scoped to a Session.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sid: Option<String>,

    /// Connection ID. `None` when not scoped to a Connection within a Session.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub connid: Option<String>,

    /// Envelope ID being responded to (request/response correlation).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub in_reply_to: Option<String>,

    /// Type-specific body. May be `{}` for envelopes that carry only routing fields.
    pub payload: T,

    /// Optional inline RFC 9421 signature per CONVERSATION_PROTOCOL.md
    /// §5.5.1. When present, deployments configured with a
    /// `Sig9421Verifier` will canonicalize the envelope (excluding
    /// this field), verify the signature, and reject the envelope if
    /// verification fails. Unsigned envelopes pass through unchanged
    /// unless the deployment's policy marks the envelope's
    /// `msg_type` as requiring a signature.
    ///
    /// `#[serde(default)]` means existing wire formats without a
    /// `signature` field continue to deserialize cleanly — no
    /// wire-compatibility break (gap plan §5.2 v1 punch list).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub signature: Option<rvoip_auth_core::sig9421::EnvelopeSignature>,
}

impl<T> UctpEnvelope<T> {
    /// Build a new envelope with sensible defaults. Caller fills in
    /// routing fields (`cid` / `sid` / `connid` / `in_reply_to`) via
    /// the chainable setters below.
    pub fn new(msg_type: MessageType, payload: T) -> Self {
        Self {
            v: UCTP_ENVELOPE_VERSION,
            msg_type,
            id: crate::ids::new_envelope_id().to_string(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: None,
            payload,
            signature: None,
        }
    }

    /// Attach an inline RFC 9421 signature to this envelope. Used by
    /// signing producers; the corresponding verify happens via
    /// `Sig9421Verifier` on the receiving coordinator's dispatch gate.
    pub fn with_signature(
        mut self,
        signature: rvoip_auth_core::sig9421::EnvelopeSignature,
    ) -> Self {
        self.signature = Some(signature);
        self
    }

    pub fn with_cid(mut self, cid: impl Into<String>) -> Self {
        self.cid = Some(cid.into());
        self
    }

    pub fn with_sid(mut self, sid: impl Into<String>) -> Self {
        self.sid = Some(sid.into());
        self
    }

    pub fn with_connid(mut self, connid: impl Into<String>) -> Self {
        self.connid = Some(connid.into());
        self
    }

    pub fn with_in_reply_to(mut self, id: impl Into<String>) -> Self {
        self.in_reply_to = Some(id.into());
        self
    }
}

impl UctpEnvelope<serde_json::Value> {
    /// Decode the `payload` field into a strongly-typed payload struct
    /// from [`crate::payloads`]. Returns [`UctpError::Decode`] if the
    /// payload doesn't match `P`'s expected shape.
    pub fn decode_payload<P: DeserializeOwned>(&self) -> Result<P, UctpError> {
        serde_json::from_value(self.payload.clone()).map_err(UctpError::from)
    }
}

impl<T: Serialize> UctpEnvelope<T> {
    /// Re-encode this envelope as `UctpEnvelope<serde_json::Value>` so
    /// it can be matched against unknown-payload code paths.
    pub fn into_value(self) -> Result<UctpEnvelope<serde_json::Value>, UctpError> {
        let payload = serde_json::to_value(self.payload)?;
        Ok(UctpEnvelope {
            v: self.v,
            msg_type: self.msg_type,
            id: self.id,
            ts: self.ts,
            cid: self.cid,
            sid: self.sid,
            connid: self.connid,
            in_reply_to: self.in_reply_to,
            payload,
            signature: self.signature,
        })
    }
}
