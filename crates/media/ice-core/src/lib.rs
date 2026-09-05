//! Sans-io ICE (RFC 8445) for rvoip.
//!
//! This crate is deliberately free of sockets, timers, and async runtimes:
//! the [`agent::IceAgent`] is a pure state machine that is handed packets
//! and the current time, and is polled for transmissions, events, and its
//! next deadline. Everything an ICE agent can get wrong — role conflicts,
//! nomination races, retransmission storms, timestamp-ordered pathologies —
//! is therefore a deterministic scripted test rather than a timing-dependent
//! one. The io that drives an agent lives with the sockets, in `rtp-core`.
//!
//! The [`stun`] module is the RFC 8489 codec the agent speaks, complete for
//! ICE's needs: short-term-credential MESSAGE-INTEGRITY, FINGERPRINT, and
//! the ICE attributes (PRIORITY, USE-CANDIDATE, ICE-CONTROLLING/CONTROLLED).
//! [`candidate`] holds the candidate model and the RFC 8445 §5.1.2 priority
//! arithmetic. SDP encoding of candidates deliberately lives in `sip-core`
//! with the rest of SDP, not here.

#![warn(missing_docs)]

pub mod agent;
pub mod candidate;
pub mod stun;

pub use agent::{AgentConfig, Credentials, IceAgent, IceEvent, IceRole, IceState, Transmit};
pub use candidate::{Candidate, CandidateKind};
