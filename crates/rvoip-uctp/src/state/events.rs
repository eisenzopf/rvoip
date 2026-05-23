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
}
