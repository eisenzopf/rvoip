//! ICE protocol integration for SIP client
//! This module provides integration between SIP client and the ice-core crate
//! following the pjsip model of ICE integration.

// Submodules
mod candidate;
mod session;

// Re-export our own types
pub use self::candidate::SipIceCandidate;
pub use self::candidate::compute_priority;
pub use self::session::{IceSession, IceSessionState};

// Re-export the underlying ice-core types
pub use rvoip_ice_core::{
    IceAgent, IceAgentState, IceAgentEvent, IceCandidate, 
    CandidateType, TransportType, IceConfig, IceServerConfig, 
    IceRole, IceComponent, GatheringPolicy
}; 