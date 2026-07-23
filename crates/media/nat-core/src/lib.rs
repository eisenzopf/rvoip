//! ICE (RFC 8445) NAT traversal for the RVOIP media plane.
//!
//! Wraps a real ICE agent (`webrtc-ice`, the same webrtc-rs "production"
//! 0.17.x lineage already used elsewhere in this workspace for DTLS and
//! SRTP interop testing — not the pre-production `rtc` crate the
//! `rvoip-webrtc` crate depends on) behind a small, SDP-agnostic API.
//!
//! This crate has no notion of SDP or of any particular RTP transport —
//! see [`agent::IceAgent`] for what it actually does and doesn't own.

pub mod candidate;
pub mod error;

#[cfg(feature = "ice")]
pub mod agent;

#[cfg(feature = "ice")]
pub mod bridge;

pub use candidate::{CandidateKind, IceCandidate};
pub use error::{Error, Result};

#[cfg(feature = "ice")]
pub use agent::{IceAgent, IceRole};

#[cfg(feature = "ice")]
pub use bridge::{SharedIceMux, SharedIceSocket};
