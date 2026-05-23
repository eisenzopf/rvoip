//! # rvoip-uctp
//!
//! UCTP (Universal Conversation Transport Protocol) — substrate-agnostic
//! application protocol that speaks the voip-3 nouns directly on the wire
//! over QUIC / WebTransport / WebSocket.
//!
//! This crate owns the protocol itself (envelopes, type catalog, state
//! machine, capability negotiation) plus the substrate-agnostic helpers
//! consumed by per-substrate adapter crates ([`rvoip-quic`],
//! [`rvoip-webtransport`]). It does **not** own any accept/dial loops —
//! those live in the adapter crates.
//!
//! See `UCTP_IMPLEMENTATION_PLAN.md` (this directory) for the v0 design
//! and `crates/rvoip-core/CONVERSATION_PROTOCOL.md` for the wire spec.
//!
//! ## Public surface
//!
//! Adapter crates and downstream callers should reach into the
//! re-exports below; deep paths (`rvoip_uctp::state::coordinator::*`,
//! `rvoip_uctp::substrate::quinn::*`) are stable but not the intended
//! entry points.

pub mod envelope;
pub mod errors;
pub mod ids;
pub mod types;

pub mod payloads;

pub mod state;

pub mod substrate;

// --- Re-exports — public surface per design doc §3.2 ---

pub use crate::envelope::UctpEnvelope;
pub use crate::errors::{Result, SubstrateError, UctpError};
pub use crate::ids::{ConnectionId, EnvelopeId, SessionId};
pub use crate::state::{
    default_v0_descriptor, UctpConnectionState, UctpCoordinator, UctpSessionEvent,
    UctpSessionState, ENVELOPE_CHANNEL_CAP, SIGNALING_SEND_TIMEOUT,
};
pub use crate::types::MessageType;
