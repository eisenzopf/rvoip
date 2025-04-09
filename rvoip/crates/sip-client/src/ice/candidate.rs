use std::net::{SocketAddr, IpAddr};
use rvoip_ice_core::{IceCandidate, CandidateType, TransportType};

/// SIP-specific extensions to IceCandidate
pub trait SipIceCandidate {
    /// Convert candidate to SDP format as used in SIP messages
    fn to_sdp_line(&self) -> String;
    
    /// Create a host candidate from a socket address
    fn create_host(addr: SocketAddr, component: u32) -> IceCandidate;
    
    /// Get the socket address from the candidate
    fn socket_addr(&self) -> SocketAddr;
}

impl SipIceCandidate for IceCandidate {
    fn to_sdp_line(&self) -> String {
        format!("a=candidate:{} {} {} {} {} {} typ {}{}{}",
            self.foundation,
            self.component,
            self.transport.to_string().to_lowercase(),
            self.priority,
            self.ip,
            self.port,
            self.candidate_type.to_string().to_lowercase(),
            if self.related_address.is_some() { " raddr " } else { "" },
            self.related_address.map_or(String::new(), |addr| {
                format!("{} rport {}", addr, self.related_port.unwrap_or(0))
            })
        )
    }
    
    fn create_host(addr: SocketAddr, component: u32) -> Self {
        let transport = if addr.is_ipv4() {
            TransportType::Udp
        } else {
            TransportType::Udp6
        };
        
        Self {
            foundation: "1".to_string(),  // Simple foundation for host candidates
            component,
            transport,
            priority: compute_priority(CandidateType::Host, 0, component),
            ip: addr.ip(),
            port: addr.port(),
            candidate_type: CandidateType::Host,
            related_address: None,
            related_port: None,
            network_type: None,
            generation: None,
        }
    }
    
    fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }
}

/// Compute priority for a candidate as per RFC 8445
/// type_preference: 126 for host, 110 for srflx, 0 for relay
/// local_preference: typically encoding interface preference
/// component_id: 1 for RTP, 2 for RTCP
pub fn compute_priority(
    candidate_type: CandidateType,
    local_preference: u32,
    component_id: u32
) -> u32 {
    let type_preference = match candidate_type {
        CandidateType::Host => 126,
        CandidateType::Srflx => 100,
        CandidateType::Prflx => 110,
        CandidateType::Relay => 0,
    };
    
    // Per RFC 8445: priority = (2^24)*(type_preference) + 
    //               (2^8)*(local_preference) + 
    //               (2^0)*(256 - component_id)
    (type_preference << 24) | (local_preference << 8) | (256 - component_id)
} 