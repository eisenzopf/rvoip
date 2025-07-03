//! Types for SDP negotiation

use std::net::SocketAddr;

/// Role in SDP negotiation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdpRole {
    /// User Agent Client - generates offer, receives answer
    Uac,
    /// User Agent Server - receives offer, generates answer
    Uas,
}

/// Complete negotiated media configuration
/// This contains everything needed to establish media flow
#[derive(Debug, Clone)]
pub struct NegotiatedMediaConfig {
    /// The negotiated codec both parties will use
    pub codec: String,
    
    /// Local RTP endpoint
    pub local_addr: SocketAddr,
    
    /// Remote RTP endpoint
    pub remote_addr: SocketAddr,
    
    /// Local SDP (our offer or answer)
    pub local_sdp: String,
    
    /// Remote SDP (their offer or answer)
    pub remote_sdp: String,
    
    /// Our role in the negotiation
    pub role: SdpRole,
    
    /// Negotiated ptime (packetization time)
    pub ptime: Option<u8>,
    
    /// Whether DTMF is supported
    pub dtmf_enabled: bool,
} 