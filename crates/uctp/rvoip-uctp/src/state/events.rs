//! Events that the [`super::coordinator::UctpCoordinator`] emits to its
//! consumer (the adapter crate). The adapter translates these into
//! `rvoip_core::AdapterEvent`s.

use rvoip_core::identity::IdentityAssurance;

use crate::ids::{ConnectionId, SessionId, StreamId};

/// One coordinator event. Adapter crates map this to
/// `rvoip_core::AdapterEvent` per design doc §4.4.
#[derive(Debug)]
pub enum UctpSessionEvent {
    /// Peer sent `auth.session` — we are authenticated.
    Authenticated {
        identity_id: String,
        participant_id: String,
        assurance: IdentityAssurance,
    },

    /// Inbound `session.invite` arrived.
    InboundInvite {
        sid: SessionId,
        from: String,
        to: Vec<String>,
        medium: String,
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
