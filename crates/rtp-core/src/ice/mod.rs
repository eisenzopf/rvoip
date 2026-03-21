//! ICE (Interactive Connectivity Establishment) implementation per RFC 8445.
//!
//! Provides a full ICE agent for NAT traversal and connectivity establishment
//! between VoIP endpoints. Builds on the STUN module for binding requests
//! and server-reflexive candidate gathering.
//!
//! # Architecture
//!
//! - [`types`]: Core types (candidates, pairs, credentials, states).
//! - [`gather`]: Candidate gathering (host + server-reflexive via STUN).
//! - [`checklist`]: Candidate pair formation, sorting, pruning, and check building.
//! - [`agent`]: The ICE agent state machine that orchestrates the full process.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use rvoip_rtp_core::ice::{IceAgent, IceRole, IceCandidate, CandidateType, ComponentId};
//!
//! # async fn example() -> Result<(), rvoip_rtp_core::Error> {
//! // Create a controlling ICE agent
//! let mut agent = IceAgent::new(IceRole::Controlling);
//!
//! // Get local credentials for SDP offer
//! let creds = agent.local_credentials();
//! println!("ice-ufrag: {}", creds.ufrag);
//! println!("ice-pwd: {}", creds.pwd);
//!
//! // Gather candidates
//! let local_addr = "0.0.0.0:0".parse().unwrap();
//! let stun_servers = vec!["74.125.250.129:19302".parse().unwrap()];
//! let candidates = agent.gather_candidates(local_addr, &stun_servers).await?;
//!
//! // After receiving remote SDP answer, set remote credentials
//! agent.set_remote_credentials("remote_ufrag".into(), "remote_password_22chars".into());
//!
//! // Add remote candidates from SDP
//! // agent.add_remote_candidate(remote_candidate);
//!
//! // Start connectivity checks
//! agent.start_checks()?;
//!
//! // Check pairs until connected
//! while let Some(idx) = agent.next_check() {
//!     let (request, remote_addr) = agent.check_pair(idx)?;
//!     // Send request to remote_addr via UDP socket
//!     // Handle response with agent.handle_stun_response(...)
//! }
//!
//! // Get selected pair for media transport
//! if let Some(pair) = agent.selected_pair() {
//!     println!("Selected: {} <-> {}", pair.local.address, pair.remote.address);
//! }
//! # Ok(())
//! # }
//! ```

pub mod types;
pub mod gather;
pub mod checklist;
pub mod agent;

// Re-export key types at module level for convenience.
pub use types::{
    CandidateType, IceRole, IceConnectionState, ComponentId,
    IceCandidate, IceCandidatePair, CandidatePairState,
    IceCredentials,
};
pub use agent::IceAgent;
pub use gather::{compute_priority, generate_foundation, gather_relay_candidates};
