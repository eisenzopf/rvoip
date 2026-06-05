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
use rvoip_core::ids::ConnectionId;
use tokio::sync::mpsc;

use crate::envelope::UctpEnvelope;
use crate::payloads::control::Error as ErrorPayload;
use crate::substrate::{send_and_wait, Pending};
use crate::types::MessageType;

/// Default time the adapter waits for the peer's reply.
pub const DEFAULT_RENEGOTIATE_TIMEOUT: Duration = Duration::from_secs(5);

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
