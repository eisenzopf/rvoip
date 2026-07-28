//! Events that the [`super::coordinator::UctpCoordinator`] emits to its
//! consumer (the adapter crate). The adapter translates these into
//! `rvoip_core::AdapterEvent`s.

use rvoip_core::identity::IdentityAssurance;
use std::fmt;
use tokio::sync::oneshot;

use crate::ids::{ConnectionId, SessionId, StreamId};

use super::connection::AcceptedStream;

/// One coordinator event. Adapter crates map this to
/// `rvoip_core::AdapterEvent` per design doc §4.4.
#[non_exhaustive]
pub enum UctpSessionEvent {
    /// Peer sent `auth.session` — we are authenticated.
    Authenticated {
        identity_id: String,
        participant_id: String,
        assurance: IdentityAssurance,
    },

    /// Inbound `session.invite` arrived.
    InboundInvite {
        /// Peer-selected Conversation ID retained as part of the exact wire
        /// route. Session-scoped replies must never infer or substitute it.
        cid: Option<String>,
        sid: SessionId,
        from: String,
        to: Vec<String>,
        medium: String,
        intent: String,
        capabilities_offer: serde_json::Value,
    },

    /// Session reached `Active` (per §7.3 boundary rule).
    SessionConnected { sid: SessionId },

    /// Session moved to `Ended`.
    SessionEnded { sid: SessionId, reason: String },

    /// A new Connection was created in a Session.
    ConnectionOpened {
        sid: SessionId,
        connid: ConnectionId,
        chosen_codec: Option<String>,
    },

    /// Request an all-or-nothing substrate binding for the negotiated media
    /// Streams before the coordinator announces them with `stream.opened`.
    ///
    /// QUIC and WebTransport enable this path because their datagram handle is
    /// peer-global. The adapter must create the concrete MediaStreams, bind
    /// their real wire Stream IDs in the peer media router, then reply with one
    /// local ID per input Stream in the same order. On error it must roll back
    /// every partial binding before replying.
    BindMediaStreams {
        sid: SessionId,
        connid: ConnectionId,
        streams: Vec<AcceptedStream>,
        reply: oneshot::Sender<Result<Vec<u16>, crate::errors::UctpError>>,
    },

    /// A Connection moved to `Connected` (after `connection.ready`).
    ConnectionConnected {
        sid: SessionId,
        connid: ConnectionId,
    },

    /// A Connection ended.
    ConnectionEnded {
        sid: SessionId,
        connid: ConnectionId,
        reason: String,
    },

    /// A media datagram arrived on a known Stream.
    MediaFrame {
        connid: ConnectionId,
        strm_id: StreamId,
        seq: u32,
        payload: bytes::Bytes,
    },

    /// Capability negotiation failed with code 488.
    NegotiationFailed { sid: SessionId, reason: String },

    /// Peer sent `dtmf.send` (CONVERSATION_PROTOCOL.md §7.5). Adapters
    /// translate this into `AdapterEvent::Dtmf` so the orchestrator
    /// surfaces it as `Event::DtmfReceived`. Plan C2.
    Dtmf {
        connid: ConnectionId,
        digits: String,
        duration_ms: u32,
        /// `"rfc4733"` or `"info"`. Echoed from the inbound payload's
        /// `method` field so downstream consumers can distinguish how
        /// the DTMF was carried (RTP events vs SIP INFO bridging).
        method: String,
    },

    DataMessage {
        connid: ConnectionId,
        message: rvoip_core::DataMessage,
    },

    /// Peer sent `connection.quality` (CONVERSATION_PROTOCOL.md §10.3) —
    /// one event per Stream entry in the envelope (the coordinator
    /// emits N events for N-stream reports). Adapters translate to
    /// `AdapterEvent::Quality` so the orchestrator publishes
    /// `Event::MediaQuality`. Plan C2.
    Quality {
        connid: ConnectionId,
        /// Wire-level Stream id the snapshot is for. Carried through
        /// so consumers that need per-stream attribution can join
        /// back against the publisher registry.
        strm_id: String,
        snapshot: rvoip_core::stream::QualitySnapshot,
        /// Round-trip estimate in milliseconds (separate from
        /// `QualitySnapshot` which doesn't carry RTT today).
        rtt_ms: u32,
        bitrate_bps: u32,
    },

    /// P12.6 — peer sent `identity.step-up-response` answering a
    /// previous `identity.step-up-request` we issued via
    /// [`super::coordinator::UctpCoordinator::send_step_up_request`].
    /// Adapters translate this into `AdapterEvent::StepUpResponse` so
    /// the orchestrator surfaces it as
    /// `Event::IdentityStepUpResponseReceived`. See
    /// CONVERSATION_PROTOCOL.md §5.8.
    StepUpResponse {
        connid: Option<ConnectionId>,
        method: String,
        credential: String,
    },
}

impl fmt::Debug for UctpSessionEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authenticated { .. } => formatter.write_str("Authenticated"),
            Self::InboundInvite { to, medium, .. } => formatter
                .debug_struct("InboundInvite")
                .field("recipient_count", &to.len())
                .field("medium_bytes", &medium.len())
                .finish(),
            Self::SessionConnected { .. } => formatter.write_str("SessionConnected"),
            Self::SessionEnded { reason, .. } => formatter
                .debug_struct("SessionEnded")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::ConnectionOpened { chosen_codec, .. } => formatter
                .debug_struct("ConnectionOpened")
                .field("codec_present", &chosen_codec.is_some())
                .field("codec_bytes", &chosen_codec.as_ref().map_or(0, String::len))
                .finish(),
            Self::BindMediaStreams { streams, .. } => formatter
                .debug_struct("BindMediaStreams")
                .field("stream_count", &streams.len())
                .finish(),
            Self::ConnectionConnected { .. } => formatter.write_str("ConnectionConnected"),
            Self::ConnectionEnded { reason, .. } => formatter
                .debug_struct("ConnectionEnded")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::MediaFrame { seq, payload, .. } => formatter
                .debug_struct("MediaFrame")
                .field("sequence", seq)
                .field("payload_bytes", &payload.len())
                .finish(),
            Self::NegotiationFailed { reason, .. } => formatter
                .debug_struct("NegotiationFailed")
                .field("reason_bytes", &reason.len())
                .finish(),
            Self::Dtmf {
                digits,
                duration_ms,
                method,
                ..
            } => formatter
                .debug_struct("Dtmf")
                .field("digit_count", &digits.chars().count())
                .field("duration_ms", duration_ms)
                .field("method_bytes", &method.len())
                .finish(),
            Self::DataMessage { message, .. } => formatter
                .debug_struct("DataMessage")
                .field("body_bytes", &message.bytes.len())
                .finish(),
            Self::Quality {
                strm_id,
                rtt_ms,
                bitrate_bps,
                ..
            } => formatter
                .debug_struct("Quality")
                .field("stream_id_present", &!strm_id.is_empty())
                .field("stream_id_bytes", &strm_id.len())
                .field("rtt_ms", rtt_ms)
                .field("bitrate_bps", bitrate_bps)
                .finish(),
            Self::StepUpResponse {
                connid,
                method,
                credential,
            } => formatter
                .debug_struct("StepUpResponse")
                .field("connection_present", &connid.is_some())
                .field("method_present", &!method.is_empty())
                .field("method_bytes", &method.len())
                .field("credential_present", &!credential.is_empty())
                .field("credential_bytes", &credential.len())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_up_event_debug_redacts_live_credential() {
        const CANARY: &str = "uctp-state-credential-canary\r\nAuthorization: exposed";
        let event = UctpSessionEvent::StepUpResponse {
            connid: None,
            method: "bearer".into(),
            credential: CANARY.into(),
        };
        let rendered = format!("{event:?}");
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
        match event {
            UctpSessionEvent::StepUpResponse { credential, .. } => {
                assert_eq!(credential, CANARY)
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
