//! ICE (Interactive Connectivity Establishment) implementation per RFC 8445.
//!
//! Provides ICE agents for NAT traversal and connectivity establishment
//! between VoIP endpoints.
//!
//! # Architecture
//!
//! - [`types`]: Core types (candidates, pairs, credentials, states).
//! - [`gather`]: Candidate gathering (host + server-reflexive via STUN).
//! - [`checklist`]: Candidate pair formation, sorting, pruning, and check building.
//! - [`agent`]: The **legacy** self-built ICE agent (kept as fallback).
//! - [`adapter`]: Production-grade adapter wrapping `webrtc-ice` 0.17 (recommended).
//!
//! # Recommended usage
//!
//! Use [`IceAgentAdapter`] for new code. It delegates to the battle-tested
//! `webrtc-ice` crate with full RFC 8445 compliance (aggressive nomination,
//! ICE restart, peer-reflexive candidates, consent freshness, etc.).
//!
//! ```rust,no_run
//! use rvoip_rtp_core::ice::{IceAgentAdapter, IceRole};
//!
//! # async fn example() -> Result<(), rvoip_rtp_core::Error> {
//! let mut agent = IceAgentAdapter::new(IceRole::Controlling);
//! let creds = agent.local_credentials();
//! println!("ice-ufrag: {}", creds.ufrag);
//!
//! let local_addr = "0.0.0.0:0".parse().unwrap();
//! let stun_servers = vec!["74.125.250.129:19302".parse().unwrap()];
//! let candidates = agent.gather_candidates(local_addr, &stun_servers).await?;
//!
//! agent.set_remote_credentials("remote_ufrag".into(), "remote_password_22chars".into());
//! agent.start_checks().await?;
//! # Ok(())
//! # }
//! ```

pub mod types;
#[allow(deprecated)]
pub mod gather;
#[allow(deprecated)]
pub mod checklist;
#[allow(deprecated)]
pub mod agent;
pub mod adapter;

// Re-export key types at module level for convenience.
pub use types::{
    CandidateType, IceRole, IceConnectionState, ComponentId,
    IceCandidate, IceCandidatePair, CandidatePairState,
    IceCredentials,
};

/// The legacy self-built ICE agent. Kept as fallback.
///
/// Prefer [`IceAgentAdapter`] for new code — it wraps the production-grade
/// `webrtc-ice` crate with full RFC 8445 compliance.
#[deprecated(since = "0.1.27", note = "Use IceAgentAdapter instead for full RFC 8445 compliance")]
pub use agent::IceAgent;

/// Production-grade ICE agent adapter backed by `webrtc-ice`.
pub use adapter::IceAgentAdapter;

pub use gather::{compute_priority, generate_foundation, gather_relay_candidates};
