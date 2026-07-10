//! Gap plan §4.2 v1 punch list — shared adapter helpers.
//!
//! Today this module hosts [`renegotiate_via_envelope`], used by the
//! QUIC / WebTransport / WebSocket adapters' `renegotiate_media`
//! impls. The function encapsulates the envelope round-trip:
//!
//! 1. Build a `connection.update` envelope with
//!    `action == "renegotiate-media"` + the new codec preferences.
//! 2. Send-and-wait on the peer's reply via
//!    [`crate::substrate::send_and_wait`].
//! 3. Parse the reply:
//!    - `connection.update` with a non-empty `codec_preferences` →
//!      success; return the matched [`NegotiatedCodecs`].
//!    - `error 488` → `AdmissionRejected("incompatible-capabilities")`.
//!    - Anything else → `Adapter("unexpected reply …")`.
//!
//! The SIP adapter does NOT use this helper — re-INVITE goes through
//! the SIP dialog state machine instead.

use std::sync::Arc;
use std::time::Duration;

use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::error::{Result as CoreResult, RvoipError};
use rvoip_core::identity::{AuthenticatedPrincipal, PrincipalOwnershipKey};
use rvoip_core::ids::ConnectionId;
use rvoip_core::DataMessage;
use tokio::sync::mpsc;

use crate::envelope::UctpEnvelope;
use crate::payloads::control::Error as ErrorPayload;
use crate::substrate::{send_and_wait, Pending};
use crate::types::MessageType;

/// Default time the adapter waits for the peer's reply.
pub const DEFAULT_RENEGOTIATE_TIMEOUT: Duration = Duration::from_secs(5);

/// Principal-owned association between an adapter's core Connection ID and
/// the peer-selected UCTP wire `connid`.
///
/// A route is created from an authenticated `session.invite`, then bound once
/// when that same peer emits `connection.offer`. Binding compares issuer,
/// tenant, and subject and refuses both ownership switches and wire-ID
/// rebinding. All UCTP substrates use this type so their authorization and
/// outbound-ID behavior cannot drift.
#[derive(Clone, Debug)]
pub struct AuthenticatedConnectionBinding {
    owner: PrincipalOwnershipKey,
    wire_connection_id: Arc<parking_lot::RwLock<Option<ConnectionId>>>,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum ConnectionBindingError {
    #[error("route has not established an authenticated wire connection ID")]
    NotBound,
    #[error("authenticated principal is expired")]
    PrincipalExpired,
    #[error("wire connection owner does not match route owner")]
    OwnerMismatch,
    #[error("route is already bound to wire connection {existing}; cannot bind {attempted}")]
    AlreadyBound {
        existing: ConnectionId,
        attempted: ConnectionId,
    },
}

impl AuthenticatedConnectionBinding {
    pub fn new(principal: &AuthenticatedPrincipal) -> Self {
        Self {
            owner: principal.ownership_key(),
            wire_connection_id: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    pub fn owner(&self) -> &PrincipalOwnershipKey {
        &self.owner
    }

    pub fn is_owned_by(&self, principal: &AuthenticatedPrincipal) -> bool {
        self.owner == principal.ownership_key()
    }

    pub fn bind_wire_connection(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_connection_id: ConnectionId,
    ) -> Result<(), ConnectionBindingError> {
        if principal.is_expired() {
            return Err(ConnectionBindingError::PrincipalExpired);
        }
        if !self.is_owned_by(principal) {
            return Err(ConnectionBindingError::OwnerMismatch);
        }
        let mut bound = self.wire_connection_id.write();
        match bound.as_ref() {
            None => {
                *bound = Some(wire_connection_id);
                Ok(())
            }
            Some(existing) if existing == &wire_connection_id => Ok(()),
            Some(existing) => Err(ConnectionBindingError::AlreadyBound {
                existing: existing.clone(),
                attempted: wire_connection_id,
            }),
        }
    }

    pub fn wire_connection_id(&self) -> Option<ConnectionId> {
        self.wire_connection_id.read().clone()
    }

    pub fn outbound_connection_id(&self) -> Result<ConnectionId, ConnectionBindingError> {
        self.wire_connection_id()
            .ok_or(ConnectionBindingError::NotBound)
    }
}

pub fn require_bound_wire_connection(
    binding: &AuthenticatedConnectionBinding,
) -> CoreResult<ConnectionId> {
    binding.outbound_connection_id().map_err(|error| {
        RvoipError::Adapter(format!("UCTP connection route is not ready: {error}"))
    })
}

pub async fn send_data_message_via_envelope(
    out_tx: &mpsc::Sender<UctpEnvelope>,
    sid: &str,
    conn: &ConnectionId,
    message: &DataMessage,
) -> CoreResult<()> {
    // `DataMessage` deliberately has no sender field. The peer must derive
    // sender/ownership from the authenticated Connection route and ignore the
    // legacy UCTP payload's non-authoritative `from` string.
    const ROUTE_DERIVED_SENDER: &str = "system:rvoip";
    let payload = crate::payloads::message::MessageSend::from_data_message(
        message,
        ROUTE_DERIVED_SENDER,
        serde_json::json!("all"),
    )
    .map_err(|error| match error {
        crate::payloads::message::MessagePayloadError::UnsupportedReliability => {
            RvoipError::NotImplemented("UCTP message.send supports reliable ordered delivery only")
        }
        other => RvoipError::Adapter(format!("invalid data message: {other}")),
    })?;
    let env = UctpEnvelope::new(
        MessageType::MessageSend,
        serde_json::to_value(payload)
            .map_err(|error| RvoipError::Adapter(format!("encode message.send: {error}")))?,
    )
    .with_sid(sid.to_string())
    .with_connid(conn.to_string());
    out_tx
        .send(env)
        .await
        .map_err(|_| RvoipError::Adapter("peer signaling channel closed".into()))
}

/// Send a `connection.update` envelope with the new capabilities and
/// await the peer's reply. See module docs for the parse rules.
pub async fn renegotiate_via_envelope(
    out_tx: &mpsc::Sender<UctpEnvelope>,
    pending: &Arc<Pending>,
    sid: &str,
    conn: &ConnectionId,
    capabilities: &CapabilityDescriptor,
    timeout: Duration,
) -> CoreResult<NegotiatedCodecs> {
    if capabilities.audio_codecs.is_empty() {
        return Err(RvoipError::UnsupportedCodec(
            "renegotiate_media: empty audio_codecs in new capabilities".into(),
        ));
    }
    let prefs: Vec<String> = capabilities
        .audio_codecs
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let payload = serde_json::json!({
        "action": "renegotiate-media",
        "codec_preferences": prefs,
    });
    let env = UctpEnvelope::new(MessageType::ConnectionUpdate, payload)
        .with_sid(sid.to_string())
        .with_connid(conn.to_string());

    let reply = send_and_wait(out_tx, pending.as_ref(), env, timeout)
        .await
        .map_err(|_| {
            RvoipError::Adapter(
                "renegotiate_media: timeout or substrate closed before peer replied".into(),
            )
        })?;

    match reply.msg_type {
        MessageType::ConnectionUpdate => parse_update_reply(reply, capabilities),
        MessageType::Error => {
            let payload: ErrorPayload = reply.decode_payload().map_err(|e| {
                RvoipError::Adapter(format!(
                    "renegotiate_media: malformed error reply payload: {e}"
                ))
            })?;
            if payload.code == 488 {
                Err(RvoipError::AdmissionRejected(
                    "renegotiate_media: peer rejected with 488 incompatible-capabilities",
                ))
            } else {
                Err(RvoipError::Adapter(format!(
                    "renegotiate_media: peer error {} {}",
                    payload.code, payload.reason
                )))
            }
        }
        other => Err(RvoipError::Adapter(format!(
            "renegotiate_media: unexpected reply type {other}"
        ))),
    }
}

fn parse_update_reply(
    reply: UctpEnvelope,
    capabilities: &CapabilityDescriptor,
) -> CoreResult<NegotiatedCodecs> {
    let payload: crate::payloads::connection::ConnectionUpdate =
        reply.decode_payload().map_err(|e| {
            RvoipError::Adapter(format!(
                "renegotiate_media: malformed connection.update reply: {e}"
            ))
        })?;
    let chosen_name = payload
        .codec_preferences
        .into_iter()
        .next()
        .ok_or_else(|| {
            RvoipError::Adapter(
                "renegotiate_media: peer replied with empty codec_preferences".into(),
            )
        })?;
    // Look up the full CodecInfo (clock rate, channels) from our
    // capabilities — the wire reply only carries the name. If the
    // peer's chosen name doesn't appear in our request set,
    // synthesize a minimal CodecInfo so the caller still sees the
    // negotiated outcome.
    let chosen = capabilities
        .audio_codecs
        .iter()
        .find(|c| c.name == chosen_name)
        .cloned()
        .unwrap_or_else(|| rvoip_core::capability::CodecInfo {
            name: chosen_name,
            clock_rate_hz: 0,
            channels: 0,
            fmtp: None,
        });
    Ok(NegotiatedCodecs {
        audio: Some(chosen),
        video: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_core::identity::{AuthenticationMethod, IdentityAssurance};

    fn principal(issuer: &str, tenant: &str, subject: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: subject.into(),
            tenant: Some(tenant.into()),
            scopes: vec!["calls:read".into()],
            issuer: Some(issuer.into()),
            expires_at: None,
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::Anonymous,
        }
    }

    #[test]
    fn connection_binding_is_idempotent_but_owner_and_wire_id_are_immutable() {
        let owner = principal("issuer-a", "tenant-a", "alice");
        let binding = AuthenticatedConnectionBinding::new(&owner);
        let wire = ConnectionId::from_string("conn_wire_a");

        assert_eq!(
            binding.outbound_connection_id(),
            Err(ConnectionBindingError::NotBound)
        );
        assert_eq!(binding.bind_wire_connection(&owner, wire.clone()), Ok(()));
        assert_eq!(binding.bind_wire_connection(&owner, wire.clone()), Ok(()));
        assert_eq!(binding.outbound_connection_id(), Ok(wire));

        let other_owner = principal("issuer-a", "tenant-b", "alice");
        assert_eq!(
            binding.bind_wire_connection(&other_owner, ConnectionId::from_string("conn_wire_b")),
            Err(ConnectionBindingError::OwnerMismatch)
        );
        assert!(matches!(
            binding.bind_wire_connection(&owner, ConnectionId::from_string("conn_wire_b")),
            Err(ConnectionBindingError::AlreadyBound { .. })
        ));
    }
}
