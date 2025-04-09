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
    
    /// Process remote SDP from a received message
    pub async fn process_remote_sdp(&self, sdp_data: &[u8]) -> Result<Option<MediaSession>> {
        // Parse the SDP
        let sdp_str = std::str::from_utf8(sdp_data)
            .map_err(|e| Error::SdpParsing(e.to_string()))?;
            
        let sdp = SessionDescription::parse(sdp_str)
            .map_err(|e| Error::SdpParsing(e.to_string()))?;
            
        info!("Received remote SDP with {} media sections", sdp.media.len());
        
        // Store the remote SDP
        *self.remote_sdp.write().await = Some(sdp.clone());
        
        // Get our local SDP
        let local_sdp = self.local_sdp.read().await.clone();
        
        // If we have both local and remote SDP, we can set up media
        if let Some(local_sdp) = local_sdp {
            // Extract media information from both SDPs and create the media session
            return self.create_media_session(&local_sdp, &sdp).await;
        }
        
        // If we don't have local SDP yet, we'll create media sessions later
        Ok(None)
    }
    
    /// Create media sessions based on local and remote SDP
    pub async fn create_media_session(
        &self,
        local_sdp: &SessionDescription,
        remote_sdp: &SessionDescription,
    ) -> Result<Option<MediaSession>> {
        // For now, we only support audio
        let local_audio = local_sdp.media.iter()
            .find(|m| m.media_type == "audio");
            
        let remote_audio = remote_sdp.media.iter()
            .find(|m| m.media_type == "audio");
            
        // Check if we have both audio media
        if let (Some(local_audio), Some(remote_audio)) = (local_audio, remote_audio) {
            // Extract remote address
            let remote_addr = if let Some(conn) = &remote_sdp.connection {
                conn.connection_address
            } else if let Some(media_conn) = remote_audio.connection.as_ref() {
                media_conn.connection_address
            } else {
                return Err(Error::SdpParsing("Remote SDP missing connection information".into()));
            };
            
            // Extract remote port
            let remote_port = remote_audio.port;
            
            // Complete remote address
            let remote_rtp_addr = SocketAddr::new(remote_addr, remote_port);
            
            // Extract local port
            let local_port = local_audio.port;
            
            // Find matching codecs
            let mut common_codecs = Vec::new();
            
            for local_format in &local_audio.formats {
                for remote_format in &remote_audio.formats {
                    if local_format.encoding.to_uppercase() == remote_format.encoding.to_uppercase() {
                        // Codec match found
                        let codec_type = match local_format.encoding.to_uppercase().as_str() {
                            "PCMU" => CodecType::Pcmu,
                            "PCMA" => CodecType::Pcma,
                            // Add other codec types as needed
                            _ => continue, // Skip unsupported codecs
                        };
                        
                        // Add codec to common codecs
                        common_codecs.push((codec_type, local_format.payload_type, remote_format.payload_type));
                        break;
                    }
                }
            }
            
            if common_codecs.is_empty() {
                return Err(Error::SdpParsing("No matching codecs found".into()));
            }
            
            // Create RTP session config
            let rtp_config = RtpSessionConfig {
                local_port,
                remote_addr: remote_rtp_addr,
                payload_type: common_codecs[0].1, // Use first matching codec
                // Add other configuration as needed
                ..Default::default()
            };
            
            // Create codec parameters
            let codec_params = CodecParams {
                codec_type: common_codecs[0].0, // Use first matching codec
                // Add other parameters as needed
                ..Default::default()
            };
            
            // Create media session
            let media_session = MediaSession::new(
                MediaType::Audio,
                rtp_config,
                codec_params,
            );
            
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
    pub fn extract_sdp_from_message(body: &[u8], content_type: Option<&str>) -> Option<&[u8]> {
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