//! SDP Integration
//!
//! This module provides integration with SDP (Session Description Protocol)
//! for negotiating media capabilities between endpoints.

use std::sync::Arc;
use std::collections::HashMap;
use std::net::IpAddr;
use tokio::sync::RwLock;

use crate::error::{Error, Result};
use crate::codec::{CodecType, CodecParameters, MediaType};
use crate::session::{MediaDirection, MediaSession};

use tracing::{debug, trace, warn};

use crate::codec::traits::{Codec, CodecCapability};

/// SDP Negotiator for handling media negotiation via SDP
///
/// This class translates between SDP media descriptions and
/// internal media capabilities.
#[derive(Debug, Clone)]
pub struct SdpNegotiator {
    /// Codec preferences in order of preference
    codec_preferences: Vec<String>,
    
    /// RTP port
    rtp_port: Option<u16>,
    
    /// Media direction
    direction: MediaDirection,
    
    /// Local IP address
    local_ip: Option<IpAddr>,
}

/// SDP media line
#[derive(Debug, Clone)]
pub struct SdpMedia {
    /// Media type (audio, video, etc.)
    pub media_type: MediaType,
    
    /// RTP port
    pub port: u16,
    
    /// Protocol ("RTP/AVP", "RTP/SAVP", etc.)
    pub protocol: String,
    
    /// RTCP port (if separate from RTP)
    pub rtcp_port: Option<u16>,
    
    /// Media direction
    pub direction: MediaDirection,
    
    /// Payload type to format mapping
    pub formats: HashMap<u8, SdpFormat>,
    
    /// RTCP attributes
    pub rtcp_attrs: Vec<String>,
    
    /// Candidates for ICE
    pub candidates: Vec<String>,
}

/// SDP format description
#[derive(Debug, Clone)]
pub struct SdpFormat {
    /// Format name (e.g., "PCMU", "opus")
    pub name: String,
    
    /// Payload type
    pub payload_type: u8,
    
    /// Clock rate in Hz
    pub clock_rate: u32,
    
    /// Number of channels
    pub channels: Option<u8>,
    
    /// Format parameters
    pub parameters: HashMap<String, String>,
}

/// SDP media description
#[derive(Debug, Clone)]
pub struct SdpMediaDescription {
    /// Remote address for media
    pub remote_address: String,
    /// Audio port
    pub audio_port: u16,
    /// Video port
    pub video_port: Option<u16>,
    /// Audio direction
    pub audio_direction: String,
    /// Video direction
    pub video_direction: Option<String>,
    /// ICE ufrag
    pub ice_ufrag: Option<String>,
    /// ICE pwd
    pub ice_pwd: Option<String>,
    /// DTLS fingerprint
    pub dtls_fingerprint: Option<String>,
    /// DTLS setup
    pub dtls_setup: Option<String>,
    /// RTCP-MUX used
    pub rtcp_mux: bool,
    /// RTCP-FB parameters
    pub rtcp_fb: Vec<String>,
    /// RTP header extensions
    pub rtp_hdrexts: Vec<(u8, String)>,
    /// Audio codec parameters
    pub audio_codec_params: Vec<(u8, String, String, Vec<String>)>,
    /// Video codec parameters
    pub video_codec_params: Vec<(u8, String, String, Vec<String>)>,
}

impl SdpNegotiator {
    /// Create a new SDP negotiator
    pub fn new(codec_preferences: Vec<String>) -> Self {
        Self {
            codec_preferences,
            rtp_port: None,
            direction: MediaDirection::SendRecv,
            local_ip: None,
        }
    }
    
    /// Set the local RTP port
    pub fn set_rtp_port(&mut self, port: u16) {
        self.rtp_port = Some(port);
    }
    
    /// Set the local IP address
    pub fn set_local_ip(&mut self, ip: IpAddr) {
        self.local_ip = Some(ip);
    }
    
    /// Set the media direction
    pub fn set_direction(&mut self, direction: MediaDirection) {
        self.direction = direction;
    }
    
    /// Generate an SDP offer
    pub fn generate_offer(&self) -> Result<String> {
        // In a real implementation, this would build a complete SDP offer
        // For now, we'll leave this as a placeholder
        Err(Error::NotImplemented("SDP offer generation".into()))
    }
    
    /// Process an SDP offer and generate an answer
    pub fn process_offer(&self, offer: &str) -> Result<String> {
        // In a real implementation, this would parse the offer and generate a response
        // For now, we'll leave this as a placeholder
        Err(Error::NotImplemented("SDP offer processing".into()))
    }
    
    /// Process an SDP answer
    pub fn process_answer(&self, answer: &str) -> Result<Vec<SdpMedia>> {
        // In a real implementation, this would parse the SDP answer and extract
        // the negotiated media parameters
        // For now, we'll leave this as a placeholder
        Err(Error::NotImplemented("SDP answer processing".into()))
    }
    
    /// Convert codec parameters to SDP format
    pub fn codec_to_sdp(&self, codec_type: CodecType, params: &CodecParameters) -> Result<SdpFormat> {
        let (name, payload_type, clock_rate) = match codec_type {
            CodecType::Pcmu => ("PCMU", 0, 8000),
            CodecType::Pcma => ("PCMA", 8, 8000),
            CodecType::G722 => ("G722", 9, 8000),
            CodecType::Opus => ("opus", 111, 48000),
            CodecType::Ilbc => ("iLBC", 102, 8000),
            CodecType::G729 => ("G729", 18, 8000),
        };
        
        let mut format_params = HashMap::new();
        
        // Add common parameters
        if let Some(bitrate) = params.bitrate {
            format_params.insert("maxaveragebitrate".to_string(), bitrate.to_string());
        }
        
        // Add codec-specific parameters
        match codec_type {
            CodecType::Opus => {
                format_params.insert("minptime".to_string(), "10".to_string());
                format_params.insert("useinbandfec".to_string(), 
                                   if params.fec_enabled { "1" } else { "0" }.to_string());
                format_params.insert("usedtx".to_string(),
                                   if params.dtx_enabled { "1" } else { "0" }.to_string());
            },
            CodecType::Ilbc => {
                // Add iLBC-specific parameters
                if let Some(frame_ms) = params.frame_duration_ms {
                    format_params.insert("mode".to_string(), frame_ms.to_string());
                }
            },
            _ => {}
        }
        
        Ok(SdpFormat {
            name: name.to_string(),
            payload_type: payload_type,
            clock_rate: clock_rate,
            channels: Some(params.channels),
            parameters: format_params,
        })
    }
    
    /// Convert SDP format to codec parameters
    pub fn sdp_to_codec(&self, format: &SdpFormat) -> Result<(CodecType, CodecParameters)> {
        let (codec_type, media_type, channels) = match format.name.to_uppercase().as_str() {
            "PCMU" => (CodecType::Pcmu, MediaType::Audio, 1),
            "PCMA" => (CodecType::Pcma, MediaType::Audio, 1),
            "G722" => (CodecType::G722, MediaType::Audio, 1),
            "OPUS" => (CodecType::Opus, MediaType::Audio, 2),
            "ILBC" => (CodecType::Ilbc, MediaType::Audio, 1),
            "G729" => (CodecType::G729, MediaType::Audio, 1),
            _ => return Err(Error::UnsupportedCodec(format.name.clone())),
        };
        
        let mut params = CodecParameters::audio(
            format.clock_rate,
            format.channels.unwrap_or(channels)
        );
        
        // Process common parameters
        if let Some(bitrate) = format.parameters.get("maxaveragebitrate") {
            if let Ok(br) = bitrate.parse::<u32>() {
                params = params.with_bitrate(br);
            }
        }
        
        // Process codec-specific parameters
        match codec_type {
            CodecType::Opus => {
                // Parse opus parameters
                if let Some(fec) = format.parameters.get("useinbandfec") {
                    params = params.with_fec(fec == "1");
                }
                
                if let Some(dtx) = format.parameters.get("usedtx") {
                    params = params.with_dtx(dtx == "1");
                }
            },
            CodecType::Ilbc => {
                // Parse iLBC-specific mode parameter
                if let Some(mode) = format.parameters.get("mode") {
                    if let Ok(frame_ms) = mode.parse::<u32>() {
                        params = params.with_frame_duration(frame_ms);
                    }
                }
            },
            _ => {}
        }
        
        Ok((codec_type, params))
    }
}

impl SdpMediaDescription {
    /// Get audio codecs from SDP
    pub fn get_audio_codecs(&self) -> Vec<CodecCapability> {
        let mut codecs = Vec::new();
        
        for (pt, name, fmtp, _) in &self.audio_codec_params {
            let mut cap = CodecCapability {
                id: name.clone(),
                name: name.clone(),
                parameters: bytes::Bytes::from(fmtp.clone()),
                mime_type: format!("audio/{}", name),
                clock_rate: 8000, // Default, may be overridden
                payload_type: Some(*pt),
                media_type: MediaType::Audio,
                features: Default::default(),
                bandwidth: (8, 64, 128), // Default bandwidth
            };
            
            // Set clock rate based on codec
            match name.as_str() {
                "PCMU" | "PCMA" => cap.clock_rate = 8000,
                "G722" => cap.clock_rate = 16000,
                "opus" => cap.clock_rate = 48000,
                "telephone-event" => cap.clock_rate = 8000,
                _ => {}
            }
            
            codecs.push(cap);
        }
        
        codecs
    }
    
    /// Get video codecs from SDP
    pub fn get_video_codecs(&self) -> Vec<CodecCapability> {
        let mut codecs = Vec::new();
        
        for (pt, name, fmtp, _) in &self.video_codec_params {
            let mut cap = CodecCapability {
                id: name.clone(),
                name: name.clone(),
                parameters: bytes::Bytes::from(fmtp.clone()),
                mime_type: format!("video/{}", name),
                clock_rate: 90000, // Default for video
                payload_type: Some(*pt),
                media_type: MediaType::Video,
                features: Default::default(),
                bandwidth: (50, 500, 2000), // Default bandwidth
            };
            
            codecs.push(cap);
        }
        
        codecs
    }
}

/// SDP handler for media negotiation
pub struct SdpHandler;

impl SdpHandler {
    /// Parse SDP into media description
    pub fn parse_sdp(sdp: &str) -> Result<SdpMediaDescription> {
        let mut remote_address = "0.0.0.0".to_string();
        let mut audio_port = 0;
        let mut video_port = None;
        let mut audio_direction = "sendrecv".to_string();
        let mut video_direction = None;
        let mut ice_ufrag = None;
        let mut ice_pwd = None;
        let mut dtls_fingerprint = None;
        let mut dtls_setup = None;
        let mut rtcp_mux = false;
        let mut rtcp_fb = Vec::new();
        let mut rtp_hdrexts = Vec::new();
        let mut audio_codec_params = Vec::new();
        let mut video_codec_params = Vec::new();
        
        // Parse SDP
        let mut current_media = None;
        
        for line in sdp.lines() {
            if line.is_empty() {
                continue;
            }
            
            // Split into type and value
            let (line_type, value) = if line.len() > 2 && line.chars().nth(1) == Some('=') {
                (line.chars().next().unwrap(), &line[2..])
            } else {
                continue;
            };
            
            match line_type {
                'o' => {
                    // Origin contains address
                    let parts: Vec<&str> = value.split(' ').collect();
                    if parts.len() >= 6 {
                        remote_address = parts[5].to_string();
                    }
                },
                'c' => {
                    // Connection contains address
                    let parts: Vec<&str> = value.split(' ').collect();
                    if parts.len() >= 3 {
                        remote_address = parts[2].to_string();
                    }
                },
                'm' => {
                    // Media line
                    let parts: Vec<&str> = value.split(' ').collect();
                    if parts.len() >= 3 {
                        match parts[0] {
                            "audio" => {
                                current_media = Some("audio");
                                audio_port = parts[1].parse().unwrap_or(0);
                            },
                            "video" => {
                                current_media = Some("video");
                                video_port = Some(parts[1].parse().unwrap_or(0));
                            },
                            _ => {
                                current_media = None;
                            }
                        }
                    }
                },
                'a' => {
                    // Attribute
                    if value.starts_with("rtpmap:") {
                        // RTP mapping
                        let rtpmap_parts: Vec<&str> = value[7..].split(' ').collect();
                        if rtpmap_parts.len() >= 2 {
                            let pt = rtpmap_parts[0].parse().unwrap_or(0);
                            let codec_parts: Vec<&str> = rtpmap_parts[1].split('/').collect();
                            if codec_parts.len() >= 2 {
                                let codec_name = codec_parts[0].to_string();
                                
                                match current_media {
                                    Some("audio") => {
                                        audio_codec_params.push((pt, codec_name, String::new(), Vec::new()));
                                    },
                                    Some("video") => {
                                        video_codec_params.push((pt, codec_name, String::new(), Vec::new()));
                                    },
                                    _ => {}
                                }
                            }
                        }
                    } else if value.starts_with("fmtp:") {
                        // Format parameters
                        let fmtp_parts: Vec<&str> = value[5..].splitn(2, ' ').collect();
                        if fmtp_parts.len() >= 2 {
                            let pt = fmtp_parts[0].parse().unwrap_or(0);
                            let fmtp = fmtp_parts[1].to_string();
                            
                            match current_media {
                                Some("audio") => {
                                    for param in &mut audio_codec_params {
                                        if param.0 == pt {
                                            param.2 = fmtp.clone();
                                            break;
                                        }
                                    }
                                },
                                Some("video") => {
                                    for param in &mut video_codec_params {
                                        if param.0 == pt {
                                            param.2 = fmtp.clone();
                                            break;
                                        }
                                    }
                                },
                                _ => {}
                            }
                        }
                    } else if value.starts_with("rtcp-fb:") {
                        // RTCP feedback
                        rtcp_fb.push(value[8..].to_string());
                    } else if value.starts_with("extmap:") {
                        // RTP extension
                        let extmap_parts: Vec<&str> = value[7..].splitn(2, ' ').collect();
                        if extmap_parts.len() >= 2 {
                            let id = extmap_parts[0].parse().unwrap_or(0);
                            let ext = extmap_parts[1].to_string();
                            rtp_hdrexts.push((id, ext));
                        }
                    } else if value.starts_with("ice-ufrag:") {
                        // ICE username fragment
                        ice_ufrag = Some(value[10..].to_string());
                    } else if value.starts_with("ice-pwd:") {
                        // ICE password
                        ice_pwd = Some(value[8..].to_string());
                    } else if value.starts_with("fingerprint:") {
                        // DTLS fingerprint
                        dtls_fingerprint = Some(value[12..].to_string());
                    } else if value.starts_with("setup:") {
                        // DTLS setup
                        dtls_setup = Some(value[6..].to_string());
                    } else if value == "rtcp-mux" {
                        // RTCP multiplexing
                        rtcp_mux = true;
                    } else if value == "sendrecv" || value == "sendonly" || value == "recvonly" || value == "inactive" {
                        // Media direction
                        match current_media {
                            Some("audio") => {
                                audio_direction = value.to_string();
                            },
                            Some("video") => {
                                video_direction = Some(value.to_string());
                            },
                            _ => {}
                        }
                    }
                },
                _ => {}
            }
        }
        
        // Create media description
        let description = SdpMediaDescription {
            remote_address,
            audio_port,
            video_port,
            audio_direction,
            video_direction,
            ice_ufrag,
            ice_pwd,
            dtls_fingerprint,
            dtls_setup,
            rtcp_mux,
            rtcp_fb,
            rtp_hdrexts,
            audio_codec_params,
            video_codec_params,
        };
        
        Ok(description)
    }
    
    /// Generate an SDP offer
    pub fn generate_offer(
        local_port: u16,
        audio_codecs: &[CodecCapability], 
        video_codecs: &[CodecCapability]
    ) -> Result<String> {
        let mut sdp = String::new();
        
        // Add session-level fields
        sdp.push_str("v=0\r\n");
        sdp.push_str(&format!("o=rvoip {} {} IN IP4 0.0.0.0\r\n", 
                             rand::random::<u32>(), 
                             std::time::SystemTime::now()
                                 .duration_since(std::time::UNIX_EPOCH)
                                 .unwrap()
                                 .as_secs()));
        sdp.push_str("s=rvoip media session\r\n");
        sdp.push_str("t=0 0\r\n");
        
        // Add audio media section
        if !audio_codecs.is_empty() {
            sdp.push_str(&format!("m=audio {} RTP/AVP", local_port));
            
            // Add payload types
            for codec in audio_codecs {
                if let Some(pt) = codec.payload_type {
                    sdp.push_str(&format!(" {}", pt));
                }
            }
            sdp.push_str("\r\n");
            
            // Add connection info
            sdp.push_str("c=IN IP4 0.0.0.0\r\n");
            
            // Add rtpmap and fmtp attributes
            for codec in audio_codecs {
                if let Some(pt) = codec.payload_type {
                    sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", 
                                         pt, 
                                         codec.name, 
                                         codec.clock_rate));
                    
                    // Add fmtp if present
                    if !codec.parameters.is_empty() {
                        sdp.push_str(&format!("a=fmtp:{} {}\r\n", 
                                             pt, 
                                             String::from_utf8_lossy(&codec.parameters)));
                    }
                }
            }
            
            // Add direction
            sdp.push_str("a=sendrecv\r\n");
            
            // Add RTCP mux
            sdp.push_str("a=rtcp-mux\r\n");
        }
        
        // Add video media section
        if !video_codecs.is_empty() {
            sdp.push_str(&format!("m=video {} RTP/AVP", local_port + 2));
            
            // Add payload types
            for codec in video_codecs {
                if let Some(pt) = codec.payload_type {
                    sdp.push_str(&format!(" {}", pt));
                }
            }
            sdp.push_str("\r\n");
            
            // Add connection info
            sdp.push_str("c=IN IP4 0.0.0.0\r\n");
            
            // Add rtpmap and fmtp attributes
            for codec in video_codecs {
                if let Some(pt) = codec.payload_type {
                    sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", 
                                         pt, 
                                         codec.name, 
                                         codec.clock_rate));
                    
                    // Add fmtp if present
                    if !codec.parameters.is_empty() {
                        sdp.push_str(&format!("a=fmtp:{} {}\r\n", 
                                             pt, 
                                             String::from_utf8_lossy(&codec.parameters)));
                    }
                }
            }
            
            // Add direction
            sdp.push_str("a=sendrecv\r\n");
            
            // Add RTCP mux
            sdp.push_str("a=rtcp-mux\r\n");
        }
        
        Ok(sdp)
    }
    
    /// Generate an SDP answer
    pub fn generate_answer(
        offer: &SdpMediaDescription,
        local_port: u16,
        selected_audio_codec: Option<&Box<dyn Codec>>,
        selected_video_codec: Option<&Box<dyn Codec>>,
    ) -> Result<String> {
        let mut sdp = String::new();
        
        // Add session-level fields
        sdp.push_str("v=0\r\n");
        sdp.push_str(&format!("o=rvoip {} {} IN IP4 0.0.0.0\r\n", 
                             rand::random::<u32>(), 
                             std::time::SystemTime::now()
                                 .duration_since(std::time::UNIX_EPOCH)
                                 .unwrap()
                                 .as_secs()));
        sdp.push_str("s=rvoip media session\r\n");
        sdp.push_str("t=0 0\r\n");
        
        // Add audio media section
        if let Some(codec) = selected_audio_codec {
            let pt = codec.capability().payload_type.unwrap_or(96);
            
            sdp.push_str(&format!("m=audio {} RTP/AVP {}\r\n", local_port, pt));
            
            // Add connection info
            sdp.push_str("c=IN IP4 0.0.0.0\r\n");
            
            // Add rtpmap
            sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", 
                                 pt, 
                                 codec.capability().name, 
                                 codec.capability().clock_rate));
            
            // Add fmtp if present
            if !codec.capability().parameters.is_empty() {
                sdp.push_str(&format!("a=fmtp:{} {}\r\n", 
                                     pt, 
                                     String::from_utf8_lossy(&codec.capability().parameters)));
            }
            
            // Add direction (mirror the offer)
            sdp.push_str(&format!("a={}\r\n", offer.audio_direction));
            
            // Add RTCP mux if in offer
            if offer.rtcp_mux {
                sdp.push_str("a=rtcp-mux\r\n");
            }
        } else {
            // Reject audio if no codec
            sdp.push_str("m=audio 0 RTP/AVP 0\r\n");
        }
        
        // Add video media section
        if let Some(codec) = selected_video_codec {
            let pt = codec.capability().payload_type.unwrap_or(96);
            
            sdp.push_str(&format!("m=video {} RTP/AVP {}\r\n", local_port + 2, pt));
            
            // Add connection info
            sdp.push_str("c=IN IP4 0.0.0.0\r\n");
            
            // Add rtpmap
            sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", 
                                 pt, 
                                 codec.capability().name, 
                                 codec.capability().clock_rate));
            
            // Add fmtp if present
            if !codec.capability().parameters.is_empty() {
                sdp.push_str(&format!("a=fmtp:{} {}\r\n", 
                                     pt, 
                                     String::from_utf8_lossy(&codec.capability().parameters)));
            }
            
            // Add direction (mirror the offer)
            if let Some(direction) = &offer.video_direction {
                sdp.push_str(&format!("a={}\r\n", direction));
            } else {
                sdp.push_str("a=sendrecv\r\n");
            }
            
            // Add RTCP mux if in offer
            if offer.rtcp_mux {
                sdp.push_str("a=rtcp-mux\r\n");
            }
        } else if offer.video_port.is_some() {
            // Reject video if no codec but offer included video
            sdp.push_str("m=video 0 RTP/AVP 0\r\n");
        }
        
        Ok(sdp)
    }
} 