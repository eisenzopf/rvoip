//! ICE (Interactive Connectivity Establishment) implementation for NAT traversal.
//!
//! This crate provides ICE functionality as defined in RFC 8445, including STUN
//! support for NAT traversal in VoIP applications. This is a custom implementation
//! that follows the ICE standard and doesn't rely on the webrtc-ice crate.

// Error handling
pub mod error;

// Core STUN protocol implementation
pub mod stun;

// ICE agent
pub mod agent;

// ICE candidates
pub mod candidate;

// Configuration
pub mod config;

// Public exports
pub use agent::{IceAgent, IceAgentState, IceAgentEvent};
pub use candidate::{IceCandidate, CandidateType, TransportType, Candidate};
pub use config::{
    IceConfig, IceServerConfig, IceRole, IceComponent, 
    GatheringPolicy, IceConfigBuilder
};
pub use error::{Error, Result};
pub use stun::{StunMessage, StunAttribute, StunMessageType, StunAttributeType};

/// Re-export of common types and functions
pub mod prelude {
    pub use super::{
        IceAgent, IceAgentState, IceAgentEvent, 
        IceCandidate, CandidateType, TransportType,
        IceConfig, IceServerConfig, IceRole, IceComponent, GatheringPolicy,
        Error, Result, 
        StunMessage, StunAttribute
    };
}

/// ICE protocol constants
pub mod constants {
    /// STUN magic cookie value (RFC 5389)
    pub const STUN_MAGIC_COOKIE: u32 = 0x2112A442;
    
    /// Default port for STUN servers
    pub const DEFAULT_STUN_PORT: u16 = 3478;
    
    /// Default port for TURNS servers
    pub const DEFAULT_TURNS_PORT: u16 = 5349;
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn simple_stun_message() {
        // Create a binding request
        let msg = StunMessage::binding_request();
        
        // Encode to bytes
        let encoded = msg.encode();
        
        // Decode from bytes
        let decoded = StunMessage::decode(&encoded).expect("Failed to decode STUN message");
        
        // Check message type
        assert_eq!(decoded.msg_type, StunMessageType::BindingRequest);
    }
    
    #[tokio::test]
    async fn parse_candidate() {
        let sdp = "candidate:0 1 UDP 2130706431 192.168.1.1 8000 typ host";
        let candidate = IceCandidate::from_sdp_string(sdp).expect("Failed to parse candidate");
        
        assert_eq!(candidate.candidate_type, CandidateType::Host);
        assert_eq!(candidate.component, 1);
        assert_eq!(candidate.transport, TransportType::Udp);
        assert_eq!(candidate.port, 8000);
    }
} 