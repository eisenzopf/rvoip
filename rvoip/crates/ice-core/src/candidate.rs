use std::net::{SocketAddr, IpAddr};
use serde::{Serialize, Deserialize};

/// ICE candidate types (RFC 8445)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CandidateType {
    /// Host candidate
    Host,
    
    /// Server reflexive candidate (STUN)
    ServerReflexive,
    
    /// Peer reflexive candidate
    PeerReflexive,
    
    /// Relay candidate (TURN)
    Relay,
}

impl CandidateType {
    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::ServerReflexive => "srflx",
            Self::PeerReflexive => "prflx",
            Self::Relay => "relay",
        }
    }
}

impl std::fmt::Display for CandidateType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// ICE candidate
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceCandidate {
    /// Foundation
    pub foundation: String,
    
    /// Component ID (1 for RTP, 2 for RTCP)
    pub component: u32,
    
    /// Network protocol
    pub protocol: String,
    
    /// Priority
    pub priority: u32,
    
    /// IP address
    pub ip: IpAddr,
    
    /// Port
    pub port: u16,
    
    /// Candidate type
    pub candidate_type: CandidateType,
    
    /// Related address (for reflexive/relay candidates)
    pub related_address: Option<IpAddr>,
    
    /// Related port (for reflexive/relay candidates)
    pub related_port: Option<u16>,
    
    /// TCP type (active, passive, so) - only for TCP candidates
    pub tcp_type: Option<String>,
}

impl IceCandidate {
    /// Create a new host candidate
    pub fn new_host(
        component: u32,
        protocol: &str,
        addr: SocketAddr,
    ) -> Self {
        Self {
            foundation: format!("{:x}", rand::random::<u32>()),
            component,
            protocol: protocol.to_uppercase(),
            // Host candidates have highest priority, per RFC 8445
            priority: compute_priority(CandidateType::Host, component as u8, 0),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::Host,
            related_address: None,
            related_port: None,
            tcp_type: None,
        }
    }
    
    /// Create a new server reflexive candidate
    pub fn new_srflx(
        component: u32,
        protocol: &str,
        addr: SocketAddr,
        related_addr: SocketAddr,
    ) -> Self {
        Self {
            foundation: format!("{:x}", rand::random::<u32>()),
            component,
            protocol: protocol.to_uppercase(),
            // Server reflexive priority, per RFC 8445
            priority: compute_priority(CandidateType::ServerReflexive, component as u8, 0),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::ServerReflexive,
            related_address: Some(related_addr.ip()),
            related_port: Some(related_addr.port()),
            tcp_type: None,
        }
    }
    
    /// Create a new relay candidate
    pub fn new_relay(
        component: u32,
        protocol: &str,
        addr: SocketAddr,
        related_addr: SocketAddr,
    ) -> Self {
        Self {
            foundation: format!("{:x}", rand::random::<u32>()),
            component,
            protocol: protocol.to_uppercase(),
            // Relay candidates have lowest priority, per RFC 8445
            priority: compute_priority(CandidateType::Relay, component as u8, 0),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::Relay,
            related_address: Some(related_addr.ip()),
            related_port: Some(related_addr.port()),
            tcp_type: None,
        }
    }
    
    /// Get the full address
    pub fn address(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }
    
    /// Get the related address if present
    pub fn related_address_full(&self) -> Option<SocketAddr> {
        match (self.related_address, self.related_port) {
            (Some(ip), Some(port)) => Some(SocketAddr::new(ip, port)),
            _ => None,
        }
    }
    
    /// Format the candidate as per SDP syntax (RFC 8839)
    pub fn to_sdp_string(&self) -> String {
        let mut sdp = format!(
            "{} {} {} {} {} {} typ {}",
            self.foundation,
            self.component,
            self.protocol.to_lowercase(),
            self.priority,
            self.ip,
            self.port,
            self.candidate_type
        );
        
        // Add related address information if present
        if let (Some(raddr), Some(rport)) = (self.related_address, self.related_port) {
            sdp.push_str(&format!(" raddr {} rport {}", raddr, rport));
        }
        
        // Add TCP type if present
        if let Some(tcp_type) = &self.tcp_type {
            sdp.push_str(&format!(" tcptype {}", tcp_type));
        }
        
        sdp
    }
}

/// Compute candidate priority (as per RFC 8445)
///
/// - type_preference: 0-126, higher values are more preferred
/// - local_preference: 0-65535, higher values are more preferred
/// - component_id: 1-256, 1 = RTP, 2 = RTCP
fn compute_priority(candidate_type: CandidateType, component_id: u8, local_preference: u16) -> u32 {
    // The type preference gets the most significant bits
    let type_preference = match candidate_type {
        CandidateType::Host => 126,            // Highest preference for host
        CandidateType::ServerReflexive => 100, // Server reflexive next
        CandidateType::PeerReflexive => 110,   // Peer reflexive are discovered during connectivity checks
        CandidateType::Relay => 0,             // Lowest preference for relay
    };
    
    // Priority formula: (2^24) * type_pref + (2^8) * local_pref + (256 - component_id)
    (type_preference as u32) << 24 |
    (local_preference as u32) << 8 |
    (256 - component_id as u32)
} 