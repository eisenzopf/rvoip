use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use rvoip_session_core::sdp::{
    SessionDescription, MediaDescription, MediaFormat, MediaDirection, 
    ConnectionInfo, extract_rtp_port_from_sdp
};
use rvoip_media_core::codec::{CodecType, CodecParams};
use rvoip_rtp_core::session::RtpSessionConfig;

use crate::error::{Error, Result};
use crate::media::{MediaSession, MediaType};
use crate::config::CallConfig;

use anyhow::format_err;
use bytes::{Bytes, BytesMut};
use rand::rngs::ThreadRng;
use rand::Rng;
use rand::RngCore;

/// SDP handler for SIP calls
pub struct SdpHandler {
    /// Local IP address
    local_ip: IpAddr,
    
    /// RTP port range start
    rtp_port_range_start: u16,
    
    /// RTP port range end
    rtp_port_range_end: u16,
    
    /// Next port to use (cycles through the range)
    next_port: Arc<Mutex<u16>>,
    
    /// Call configuration
    call_config: CallConfig,
    
    /// Local SDP
    local_sdp: Arc<RwLock<Option<SessionDescription>>>,
    
    /// Remote SDP
    remote_sdp: Arc<RwLock<Option<SessionDescription>>>,
}

impl SdpHandler {
    /// Create a new SDP handler
    pub fn new(
        local_ip: IpAddr,
        rtp_port_range_start: u16,
        rtp_port_range_end: u16,
        call_config: CallConfig,
        local_sdp: Arc<RwLock<Option<SessionDescription>>>,
        remote_sdp: Arc<RwLock<Option<SessionDescription>>>,
    ) -> Self {
        Self {
            local_ip,
            rtp_port_range_start,
            rtp_port_range_end,
            next_port: Arc::new(Mutex::new(rtp_port_range_start)),
            call_config,
            local_sdp,
            remote_sdp,
        }
    }
    
    /// Initialize local SDP for an outgoing call
    pub async fn init_local_sdp(&self, username: &str) -> Result<SessionDescription> {
        // Allocate an RTP port for audio
        let rtp_port = self.allocate_rtp_port().await?;
        
        // Create a new SDP
        let sdp = SessionDescription::new_audio_call(username, self.local_ip, rtp_port);
        
        // Store the local SDP
        *self.local_sdp.write().await = Some(sdp.clone());
        
        Ok(sdp)
    }
    
    /// Process a remote SDP and create a media session
    pub async fn process_remote_sdp(&self, sdp: &SessionDescription) -> Result<Option<MediaSession>> {
        // For now, we only support audio
        let local_audio = sdp.media.iter()
            .find(|m| m.media_type == "audio");
        
        let remote_audio = sdp.media.iter()
            .find(|m| m.media_type == "audio");
        
        if let (Some(local_audio), Some(remote_audio)) = (local_audio, remote_audio) {
            debug!("Found matching audio media in local and remote SDP");
            
            // Extract remote RTP/RTCP address
            let remote_ip = if let Some(media_connection) = remote_audio.attributes.get("connection") {
                // Try to extract IP from media-level connection attribute
                media_connection.parse()
                    .map_err(|_| Error::SdpParsing("Invalid connection address in media attribute".into()))?
            } else if let Some(c) = &sdp.connection {
                c.connection_address
            } else {
                return Err(Error::SdpParsing("No connection information found".into()));
            };
            
            // Extract port
            let remote_port = remote_audio.port;
            
            // Create remote RTP address
            let remote_rtp_addr = SocketAddr::new(remote_ip, remote_port);
            
            // Create local RTP address
            let local_port = self.allocate_rtp_port().await?;
            let local_rtp_addr = SocketAddr::new(self.local_ip, local_port);
            
            debug!("Using local RTP addr {} and remote RTP addr {}", local_rtp_addr, remote_rtp_addr);
            
            // Find matching codecs
            let mut common_codecs = Vec::new();
            
            for local_format in &local_audio.formats {
                for remote_format in &remote_audio.formats {
                    if local_format.encoding.to_uppercase() == remote_format.encoding.to_uppercase() {
                        // Codec match found
                        let codec_name = local_format.encoding.to_uppercase();
                        let codec_type = SdpHandler::parse_codec_type(codec_name.as_str())
                            .ok_or_else(|| Error::SdpParsing(format!("Unsupported codec: {}", codec_name)))?;
                        
                        // Add codec to common codecs
                        common_codecs.push((codec_type, local_format.payload_type, remote_format.payload_type));
                        break;
                    }
                }
            }
            
            if common_codecs.is_empty() {
                return Err(Error::SdpParsing("No matching codecs found".into()));
            }
            
            // Check for ICE attributes in remote SDP
            let has_ice = sdp.attributes.contains_key("ice-ufrag") || 
                          remote_audio.attributes.contains_key("ice-ufrag");
            
            // Extract ICE candidates from remote SDP if ICE is in use
            let remote_candidates = if has_ice {
                self.extract_ice_candidates(sdp)
            } else {
                Vec::new()
            };
            
            // Create media session with ICE if needed
            let media_session = MediaSession::new(
                MediaType::Audio,
                local_rtp_addr,
                remote_rtp_addr,
                common_codecs[0].0.into(), // Convert media-core CodecType to our ConfigCodecType
                self.call_config.enable_rtcp(),
                has_ice && self.call_config.enable_ice(),
            ).await?;
            
            // If we have ICE candidates and ICE is enabled, add them to the ICE session
            if has_ice && self.call_config.enable_ice() && !remote_candidates.is_empty() {
                if let Some(ice_session) = media_session.ice_session() {
                    // Add each remote candidate to our ICE session
                    for candidate in remote_candidates {
                        ice_session.add_remote_candidate(candidate).await?;
                    }
                }
            }
            
            return Ok(Some(media_session));
        }
        
        // No compatible media found
        Ok(None)
    }
    
    /// Generate a response SDP for an incoming call
    pub async fn generate_response_sdp(&self, remote_sdp: &[u8], username: &str) -> Result<SessionDescription> {
        // Parse remote SDP
        let sdp_str = std::str::from_utf8(remote_sdp)
            .map_err(|e| Error::SdpParsing(e.to_string()))?;
            
        let remote = SessionDescription::parse(sdp_str)
            .map_err(|e| Error::SdpParsing(e.to_string()))?;
            
        // Allocate an RTP port for audio
        let rtp_port = self.allocate_rtp_port().await?;
        
        // Create the local SDP
        let mut local = SessionDescription::new_audio_call(username, self.local_ip, rtp_port);
        
        // Analyze remote SDP to match codecs
        let mut has_added_formats = false;
        
        if let Some(remote_audio) = remote.media.iter().find(|m| m.media_type == "audio") {
            if let Some(local_audio) = local.media.iter_mut().find(|m| m.media_type == "audio") {
                // Clear default formats first
                local_audio.formats.clear();
                
                // Add supported formats that match remote
                for remote_format in &remote_audio.formats {
                    match remote_format.encoding.to_uppercase().as_str() {
                        "PCMU" => {
                            local_audio.add_pcmu();
                            has_added_formats = true;
                        },
                        "PCMA" => {
                            local_audio.add_pcma();
                            has_added_formats = true;
                        },
                        // Add more codec types as needed
                        _ => {
                            // Unsupported codec, skip
                        }
                    }
                }
            }
        }
        
        // If no matching formats were found, add default ones
        if !has_added_formats {
            if let Some(local_audio) = local.media.iter_mut().find(|m| m.media_type == "audio") {
                local_audio.add_pcmu();
                local_audio.add_pcma();
            }
        }
        
        // Store the local SDP
        *self.local_sdp.write().await = Some(local.clone());
        
        // Store the remote SDP
        let remote_parsed = match SessionDescription::parse(sdp_str) {
            Ok(sdp) => Some(sdp),
            Err(e) => {
                warn!("Failed to parse remote SDP: {}", e);
                None
            }
        };
        *self.remote_sdp.write().await = remote_parsed;
        
        Ok(local)
    }
    
    /// Extract SDP content from a SIP message body
    pub fn extract_sdp_from_message<'a>(body: &'a [u8], content_type: Option<&str>) -> Option<&'a [u8]> {
        // If content type is application/sdp, the entire body is SDP
        if let Some(ct) = content_type {
            if ct.to_lowercase().contains("application/sdp") {
                return Some(body);
            }
        }
        
        // Otherwise, try to detect SDP content based on first line
        if body.len() > 3 {
            let start = &body[0..3];
            if start == b"v=0" {
                return Some(body);
            }
        }
        
        None
    }
    
    /// Allocate an RTP port
    async fn allocate_rtp_port(&self) -> Result<u16> {
        let mut next_port = self.next_port.lock().await;
        
        // Get current port
        let port = *next_port;
        
        // Update to next port (ensuring it's even)
        *next_port = if *next_port + 2 > self.rtp_port_range_end {
            // Wrap around to start of range
            self.rtp_port_range_start + (*next_port + 2) % self.rtp_port_range_end
        } else {
            *next_port + 2
        };
        
        // Ensure port is even (as per RTP standards)
        let port = if port % 2 == 0 { port } else { port + 1 };
        
        Ok(port)
    }
    
    /// Add ICE candidates to local SDP
    pub async fn add_ice_candidates(&self, local_sdp: &mut SessionDescription, candidates: Vec<crate::ice::IceCandidate>) -> Result<()> {
        // Add ICE attributes to session level
        local_sdp.attributes.insert("ice-ufrag".to_string(), generate_ice_ufrag());
        local_sdp.attributes.insert("ice-pwd".to_string(), generate_ice_pwd());
        
        // Add fingerprint for DTLS (using SHA-256)
        // In a real implementation, this would be generated from the certificate
        local_sdp.attributes.insert("fingerprint".to_string(), "sha-256 AA:BB:CC:DD:EE:FF:11:22:33:44:55:66:77:88:99:00:AA:BB:CC:DD:EE:FF:11:22:33:44:55:66:77:88:99:00".to_string());
        
        // Add setup attribute (actpass for offer, active for answer)
        local_sdp.attributes.insert("setup".to_string(), "actpass".to_string());
        
        // Add candidates to media level
        for media in &mut local_sdp.media {
            // Add RTP/SAVPF protocol for SRTP with DTLS
            media.protocol = "UDP/TLS/RTP/SAVPF".to_string();
            
            // Add ICE candidates using SipIceCandidate trait
            for candidate in &candidates {
                // Get SDP line using our SipIceCandidate trait
                let candidate_line = crate::ice::SipIceCandidate::to_sdp_line(candidate);
                
                // Extract the part after "a=candidate:"
                if let Some(stripped) = candidate_line.strip_prefix("a=candidate:") {
                    media.attributes.insert("candidate".to_string(), stripped.to_string());
                } else {
                    // Default fallback
                    media.attributes.insert("candidate".to_string(), candidate_line);
                }
            }
            
            // Add end-of-candidates attribute
            media.attributes.insert("end-of-candidates".to_string(), "".to_string());
            
            // Add rtcp-mux attribute for RTP/RTCP multiplexing
            media.attributes.insert("rtcp-mux".to_string(), "".to_string());
        }
        
        Ok(())
    }
    
    /// Extract ICE candidates from remote SDP
    pub fn extract_ice_candidates(&self, remote_sdp: &SessionDescription) -> Vec<crate::ice::IceCandidate> {
        let mut candidates = Vec::new();
        
        // Process each media section
        for media in &remote_sdp.media {
            // Look for candidate attributes
            for (attr_name, attr_value) in &media.attributes {
                if attr_name.starts_with("candidate:") || attr_name == "candidate" {
                    if let Some(candidate) = self.parse_ice_candidate(attr_value) {
                        candidates.push(candidate);
                    }
                }
            }
        }
        
        candidates
    }
    
    /// Parse an ICE candidate from SDP attribute
    fn parse_ice_candidate(&self, candidate_str: &str) -> Option<crate::ice::IceCandidate> {
        // Format: foundation component transport priority ip port typ candidate-type [raddr related-addr] [rport related-port]
        let parts: Vec<&str> = candidate_str.split_whitespace().collect();
        
        if parts.len() < 8 {
            // Not enough parts
            return None;
        }
        
        // Parse foundation
        let foundation = parts[0].to_string();
        
        // Parse component
        let component = parts[1].parse::<u32>().ok()?;
        
        // Parse transport
        let transport_str = parts[2].to_lowercase();
        // Convert transport string to TransportType enum
        let transport = match transport_str.as_str() {
            "udp" => crate::ice::TransportType::Udp,
            "tcp-active" => crate::ice::TransportType::TcpActive,
            "tcp-passive" => crate::ice::TransportType::TcpPassive,
            _ => crate::ice::TransportType::Udp, // Default
        };
        
        // Parse priority
        let priority = parts[3].parse::<u32>().ok()?;
        
        // Parse IP address
        let ip = parts[4].parse().ok()?;
        
        // Parse port
        let port = parts[5].parse::<u16>().ok()?;
        
        // Check for "typ"
        if parts[6] != "typ" {
            return None;
        }
        
        // Parse candidate type
        let candidate_type_str = parts[7].to_lowercase();
        // Convert candidate type string to CandidateType enum
        let candidate_type = match candidate_type_str.as_str() {
            "host" => crate::ice::CandidateType::Host,
            "srflx" => crate::ice::CandidateType::ServerReflexive,
            "prflx" => crate::ice::CandidateType::PeerReflexive,
            "relay" => crate::ice::CandidateType::Relay,
            _ => crate::ice::CandidateType::Host, // Default
        };
        
        // Parse related address and port if present
        let mut related_address = None;
        let mut related_port = None;
        
        if parts.len() > 9 && parts[8] == "raddr" {
            related_address = parts[9].parse().ok();
            
            if parts.len() > 11 && parts[10] == "rport" {
                related_port = parts[11].parse().ok();
            }
        }
        
        // Create candidate using new IceCandidate struct
        Some(crate::ice::IceCandidate {
            foundation,
            component,
            transport,
            priority,
            ip,
            port,
            candidate_type,
            related_address,
            related_port,
        })
    }
    
    /// Extract DTLS setup role from SDP
    pub fn extract_dtls_setup(&self, sdp: &SessionDescription) -> Option<&'static str> {
        // Look for setup attribute at session level
        if let Some(setup) = sdp.attributes.get("setup") {
            return Some(match setup.as_str() {
                "active" => "passive",
                "passive" => "active",
                "actpass" => "active",
                _ => "active",
            });
        }
        
        // Look for setup attribute at media level
        for media in &sdp.media {
            if let Some(setup) = media.attributes.get("setup") {
                return Some(match setup.as_str() {
                    "active" => "passive",
                    "passive" => "active",
                    "actpass" => "active",
                    _ => "active",
                });
            }
        }
        
        // Default to active
        Some("active")
    }
    
    /// Extract DTLS fingerprint from SDP
    pub fn extract_dtls_fingerprint(&self, sdp: &SessionDescription) -> Option<String> {
        // Look for fingerprint attribute at session level
        if let Some(fingerprint) = sdp.attributes.get("fingerprint") {
            return Some(fingerprint.clone());
        }
        
        // Look for fingerprint attribute at media level
        for media in &sdp.media {
            if let Some(fingerprint) = media.attributes.get("fingerprint") {
                return Some(fingerprint.clone());
            }
        }
        
        None
    }
    
    fn parse_codec_type(codec_name: &str) -> Option<rvoip_media_core::codec::CodecType> {
        // Match codec name to our internal codec type
        match codec_name.to_uppercase().as_str() {
            "PCMU" => Some(rvoip_media_core::codec::CodecType::Pcmu),
            "PCMA" => Some(rvoip_media_core::codec::CodecType::Pcma),
            "G722" => Some(rvoip_media_core::codec::CodecType::G729), // Use G729 as fallback
            "G729" => Some(rvoip_media_core::codec::CodecType::G729),
            "OPUS" => Some(rvoip_media_core::codec::CodecType::Opus),
            _ => None,
        }
    }
}

/// A utility function to convert media direction from SDP to a boolean
/// indicating whether the local side can send audio
pub fn media_direction_to_can_send(direction: MediaDirection) -> bool {
    matches!(direction, MediaDirection::SendRecv | MediaDirection::SendOnly)
}

/// A utility function to convert media direction from SDP to a boolean
/// indicating whether the local side can receive audio
pub fn media_direction_to_can_receive(direction: MediaDirection) -> bool {
    matches!(direction, MediaDirection::SendRecv | MediaDirection::RecvOnly)
}

/// Generate random ICE username fragment
fn generate_ice_ufrag() -> String {
    let mut rng = rand::thread_rng();
    
    // Generate 4-6 random bytes and convert to base64
    let size = 4 + (rng.next_u32() % 3) as usize;
    let mut bytes = vec![0u8; size];
    rand::RngCore::fill_bytes(&mut rng, &mut bytes);
    
    // Convert to base64 and remove padding
    let ufrag = base64::encode(&bytes).replace("=", "");
    
    // Truncate to 4-6 characters
    ufrag[0..size].to_string()
}

/// Generate random ICE password
fn generate_ice_pwd() -> String {
    let mut rng = rand::thread_rng();
    
    // Generate 22-24 random bytes and convert to base64
    let size = 22 + (rng.next_u32() % 3) as usize;
    let mut bytes = vec![0u8; size];
    rand::RngCore::fill_bytes(&mut rng, &mut bytes);
    
    // Convert to base64 and remove padding
    let pwd = base64::encode(&bytes).replace("=", "");
    
    // Truncate to 22-24 characters
    pwd[0..size].to_string()
} 