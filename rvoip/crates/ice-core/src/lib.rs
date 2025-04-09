//! ICE (Interactive Connectivity Establishment) implementation for NAT traversal.
//!
//! This crate provides ICE functionality as defined in RFC 8445, including STUN and TURN
//! support for NAT traversal in VoIP applications.

pub mod error;
pub mod agent;
pub mod candidate;
pub mod config;

pub use agent::{IceAgent, IceAgentState, IceAgentEvent};
pub use candidate::{IceCandidate, CandidateType};
pub use config::{IceConfig, IceServerConfig};
pub use error::{Error, Result};

/// Re-export of common types and functions
pub mod prelude {
    pub use super::{
        IceAgent, IceAgentState, IceAgentEvent, IceCandidate, CandidateType,
        IceConfig, IceServerConfig, Error, Result,
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
} 