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

// Extension traits for IceConfig
pub trait IceConfigExt {
    /// Add multiple STUN servers at once
    fn with_stun_servers(self, servers: Vec<String>) -> Self;
    
    /// Set the gathering policy
    fn with_gathering_policy(self, policy: GatheringPolicy) -> Self;
}

impl IceConfigExt for IceConfig {
    fn with_stun_servers(mut self, servers: Vec<String>) -> Self {
        // Clear existing servers
        self.servers.clear();
        
        // Add each STUN server
        for url in servers {
            self.servers.push(IceServerConfig {
                url,
                username: None,
                credential: None,
            });
        }
        
        self
    }
    
    fn with_gathering_policy(mut self, policy: GatheringPolicy) -> Self {
        self.gathering_policy = policy;
        self
    }
} 