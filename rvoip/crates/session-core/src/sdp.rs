use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use thiserror::Error;
use tracing;

/// Errors related to SDP operations
#[derive(Error, Debug)]
pub enum SdpError {
    #[error("Failed to parse SDP: {0}")]
    ParseError(String),
    
    #[error("Invalid SDP format: {0}")]
    FormatError(String),
    
    #[error("Missing required SDP field: {0}")]
    MissingField(String),
    
    #[error("Unsupported codec or media type: {0}")]
    UnsupportedMedia(String),
}

/// Media direction in an SDP
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

impl Default for MediaDirection {
    fn default() -> Self {
        Self::SendRecv
    }
}

impl FromStr for MediaDirection {
    type Err = SdpError;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sendrecv" => Ok(Self::SendRecv),
            "sendonly" => Ok(Self::SendOnly),
            "recvonly" => Ok(Self::RecvOnly),
            "inactive" => Ok(Self::Inactive),
            _ => Err(SdpError::ParseError(format!("Invalid media direction: {}", s))),
        }
    }
}

impl ToString for MediaDirection {
    fn to_string(&self) -> String {
        match self {
            Self::SendRecv => "sendrecv".to_string(),
            Self::SendOnly => "sendonly".to_string(),
            Self::RecvOnly => "recvonly".to_string(),
            Self::Inactive => "inactive".to_string(),
        }
    }
}

/// A media format defined in an SDP
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFormat {
    /// Payload type (e.g., 0 for PCMU, 8 for PCMA)
    pub payload_type: u8,
    
    /// Encoding name (e.g., "PCMU", "PCMA", "opus")
    pub encoding: String,
    
    /// Clock rate in Hz (e.g., 8000, 48000)
    pub clock_rate: u32,
    
    /// Number of channels (typically 1 for mono, 2 for stereo)
    pub channels: u8,
    
    /// Format-specific parameters
    pub parameters: HashMap<String, String>,
}

impl MediaFormat {
    /// Create a new G.711 μ-law format (PCMU)
    pub fn pcmu() -> Self {
        Self {
            payload_type: 0,
            encoding: "PCMU".to_string(),
            clock_rate: 8000,
            channels: 1,
            parameters: HashMap::new(),
        }
    }
    
    /// Create a new G.711 A-law format (PCMA)
    pub fn pcma() -> Self {
        Self {
            payload_type: 8,
            encoding: "PCMA".to_string(),
            clock_rate: 8000,
            channels: 1,
            parameters: HashMap::new(),
        }
    }
    
    /// Format the rtpmap attribute
    pub fn format_rtpmap(&self) -> String {
        if self.channels > 1 {
            format!("a=rtpmap:{} {}/{}/{}\r\n", 
                    self.payload_type, self.encoding, self.clock_rate, self.channels)
        } else {
            format!("a=rtpmap:{} {}/{}\r\n", 
                    self.payload_type, self.encoding, self.clock_rate)
        }
    }
    
    /// Format the fmtp attribute if there are parameters
    pub fn format_fmtp(&self) -> Option<String> {
        if self.parameters.is_empty() {
            return None;
        }
        
        let params = self.parameters.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(";");
            
        Some(format!("a=fmtp:{} {}\r\n", self.payload_type, params))
    }
}

/// A media description in an SDP
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDescription {
    /// Media type (e.g., "audio", "video")
    pub media_type: String,
    
    /// Port number
    pub port: u16,
    
    /// Protocol (e.g., "RTP/AVP", "RTP/SAVP")
    pub protocol: String,
    
    /// Available formats
    pub formats: Vec<MediaFormat>,
    
    /// Media direction
    pub direction: MediaDirection,
    
    /// Additional attributes
    pub attributes: HashMap<String, String>,
}

impl Default for MediaDescription {
    fn default() -> Self {
        Self {
            media_type: "audio".to_string(),
            port: 0,
            protocol: "RTP/AVP".to_string(),
            formats: Vec::new(),
            direction: MediaDirection::default(),
            attributes: HashMap::new(),
        }
    }
}

impl MediaDescription {
    /// Create a new audio media description
    pub fn new_audio(port: u16) -> Self {
        Self {
            media_type: "audio".to_string(),
            port,
            protocol: "RTP/AVP".to_string(),
            formats: Vec::new(),
            direction: MediaDirection::default(),
            attributes: HashMap::new(),
        }
    }
    
    /// Add a media format
    pub fn add_format(&mut self, format: MediaFormat) {
        self.formats.push(format);
    }
    
    /// Add a standard G.711 μ-law format
    pub fn add_pcmu(&mut self) {
        self.formats.push(MediaFormat::pcmu());
    }
    
    /// Add a standard G.711 A-law format
    pub fn add_pcma(&mut self) {
        self.formats.push(MediaFormat::pcma());
    }
    
    /// Get payload types as a space-separated string
    fn format_payload_types(&self) -> String {
        self.formats.iter()
            .map(|f| f.payload_type.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Session Description Protocol representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescription {
    /// Protocol version
    pub version: u8,
    
    /// Origin information
    pub origin: SessionOrigin,
    
    /// Session name
    pub session_name: String,
    
    /// Connection information
    pub connection: Option<ConnectionInfo>,
    
    /// Time description
    pub timing: Vec<(u64, u64)>,
    
    /// Media descriptions
    pub media: Vec<MediaDescription>,
    
    /// Session-level attributes
    pub attributes: HashMap<String, String>,
}

/// Session origin information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOrigin {
    /// Username
    pub username: String,
    
    /// Session ID
    pub session_id: u64,
    
    /// Session version
    pub session_version: u64,
    
    /// Network type (typically "IN")
    pub network_type: String,
    
    /// Address type (typically "IP4" or "IP6")
    pub address_type: String,
    
    /// Unicast address
    pub unicast_address: IpAddr,
}

impl Default for SessionOrigin {
    fn default() -> Self {
        Self {
            username: "-".to_string(),
            session_id: rand::random::<u32>() as u64,
            session_version: rand::random::<u32>() as u64,
            network_type: "IN".to_string(),
            address_type: "IP4".to_string(),
            unicast_address: IpAddr::from([127, 0, 0, 1]),
        }
    }
}

/// Connection information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    /// Network type (typically "IN")
    pub network_type: String,
    
    /// Address type (typically "IP4" or "IP6")
    pub address_type: String,
    
    /// Connection address
    pub connection_address: IpAddr,
}

impl Default for ConnectionInfo {
    fn default() -> Self {
        Self {
            network_type: "IN".to_string(),
            address_type: "IP4".to_string(),
            connection_address: IpAddr::from([127, 0, 0, 1]),
        }
    }
}

impl Default for SessionDescription {
    fn default() -> Self {
        Self {
            version: 0,
            origin: SessionOrigin::default(),
            session_name: "Session".to_string(),
            connection: Some(ConnectionInfo::default()),
            timing: vec![(0, 0)],
            media: Vec::new(),
            attributes: HashMap::new(),
        }
    }
}

impl SessionDescription {
    /// Create a new SessionDescription with default values
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Create a new SessionDescription for a basic audio call
    pub fn new_audio_call(username: &str, local_ip: IpAddr, rtp_port: u16) -> Self {
        let mut sdp = Self::default();
        
        // Set origin
        sdp.origin.username = username.to_string();
        sdp.origin.unicast_address = local_ip;
        
        // Set session name
        sdp.session_name = "Audio Call".to_string();
        
        // Set connection info
        sdp.connection = Some(ConnectionInfo {
            network_type: "IN".to_string(),
            address_type: if local_ip.is_ipv4() { "IP4" } else { "IP6" }.to_string(),
            connection_address: local_ip,
        });
        
        // Create an audio media description
        let mut media = MediaDescription::new_audio(rtp_port);
        media.add_pcmu();
        media.add_pcma();
        media.direction = MediaDirection::SendRecv;
        
        // Add the media description
        sdp.media.push(media);
        
        sdp
    }
    
    /// Parse an SDP string
    pub fn parse(sdp_str: &str) -> Result<Self, SdpError> {
        let mut sdp = Self::default();
        let mut current_media: Option<MediaDescription> = None;
        
        for line in sdp_str.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            
            // SDP lines are in the format type=value
            if line.len() < 2 || !line.contains('=') {
                return Err(SdpError::ParseError(format!("Invalid SDP line: {}", line)));
            }
            
            let type_char = line.chars().next().unwrap();
            let value = &line[2..];
            
            match type_char {
                'v' => {
                    // Version
                    sdp.version = value.parse()
                        .map_err(|_| SdpError::ParseError(format!("Invalid version: {}", value)))?;
                },
                'o' => {
                    // Origin
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    if parts.len() != 6 {
                        return Err(SdpError::ParseError(format!("Invalid origin: {}", value)));
                    }
                    
                    sdp.origin = SessionOrigin {
                        username: parts[0].to_string(),
                        session_id: parts[1].parse()
                            .map_err(|_| SdpError::ParseError(format!("Invalid session ID: {}", parts[1])))?,
                        session_version: parts[2].parse()
                            .map_err(|_| SdpError::ParseError(format!("Invalid session version: {}", parts[2])))?,
                        network_type: parts[3].to_string(),
                        address_type: parts[4].to_string(),
                        unicast_address: parts[5].parse()
                            .map_err(|_| SdpError::ParseError(format!("Invalid address: {}", parts[5])))?,
                    };
                },
                's' => {
                    // Session name
                    sdp.session_name = value.to_string();
                },
                'c' => {
                    // Connection information
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    if parts.len() != 3 {
                        return Err(SdpError::ParseError(format!("Invalid connection info: {}", value)));
                    }
                    
                    let connection = ConnectionInfo {
                        network_type: parts[0].to_string(),
                        address_type: parts[1].to_string(),
                        connection_address: parts[2].parse()
                            .map_err(|_| SdpError::ParseError(format!("Invalid connection address: {}", parts[2])))?,
                    };
                    
                    // If we're parsing a media section, add to the current media
                    if let Some(ref mut _media) = current_media {
                        // Media-level connection information would go here if needed
                    } else {
                        // Session-level connection
                        sdp.connection = Some(connection);
                    }
                },
                't' => {
                    // Timing
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    if parts.len() != 2 {
                        return Err(SdpError::ParseError(format!("Invalid timing: {}", value)));
                    }
                    
                    let start_time = parts[0].parse()
                        .map_err(|_| SdpError::ParseError(format!("Invalid start time: {}", parts[0])))?;
                    let end_time = parts[1].parse()
                        .map_err(|_| SdpError::ParseError(format!("Invalid end time: {}", parts[1])))?;
                    
                    sdp.timing.push((start_time, end_time));
                },
                'm' => {
                    // Media description
                    // If we already have a current media, add it to the SDP
                    if let Some(media) = current_media.take() {
                        sdp.media.push(media);
                    }
                    
                    // Parse the new media line
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    if parts.len() < 4 {
                        return Err(SdpError::ParseError(format!("Invalid media description: {}", value)));
                    }
                    
                    let media_type = parts[0].to_string();
                    let port = parts[1].parse()
                        .map_err(|_| SdpError::ParseError(format!("Invalid port: {}", parts[1])))?;
                    let protocol = parts[2].to_string();
                    
                    // Create new media description
                    let mut media = MediaDescription {
                        media_type,
                        port,
                        protocol,
                        formats: Vec::new(),
                        direction: MediaDirection::default(),
                        attributes: HashMap::new(),
                    };
                    
                    // Parse formats (payload types)
                    for pt in &parts[3..] {
                        let payload_type = pt.parse()
                            .map_err(|_| SdpError::ParseError(format!("Invalid payload type: {}", pt)))?;
                        
                        // We'll fill in more details when we see rtpmap attributes
                        media.formats.push(MediaFormat {
                            payload_type,
                            encoding: String::new(),
                            clock_rate: 0,
                            channels: 1,
                            parameters: HashMap::new(),
                        });
                    }
                    
                    current_media = Some(media);
                },
                'a' => {
                    // Attribute
                    let attr_parts: Vec<&str> = value.splitn(2, ':').collect();
                    let attr_name = attr_parts[0];
                    let attr_value = attr_parts.get(1).map(|&v| v).unwrap_or("");
                    
                    if let Some(ref mut _media) = current_media {
                        // Media-level attribute
                        match attr_name {
                            "rtpmap" => {
                                // Parse rtpmap
                                let rtpmap_parts: Vec<&str> = attr_value.splitn(2, ' ').collect();
                                if rtpmap_parts.len() != 2 {
                                    return Err(SdpError::ParseError(format!("Invalid rtpmap: {}", attr_value)));
                                }
                                
                                let payload_type: u8 = rtpmap_parts[0].parse()
                                    .map_err(|_| SdpError::ParseError(format!("Invalid payload type: {}", rtpmap_parts[0])))?;
                                
                                let codec_parts: Vec<&str> = rtpmap_parts[1].split('/').collect();
                                if codec_parts.len() < 2 {
                                    return Err(SdpError::ParseError(format!("Invalid codec format: {}", rtpmap_parts[1])));
                                }
                                
                                let encoding = codec_parts[0].to_string();
                                let clock_rate: u32 = codec_parts[1].parse()
                                    .map_err(|_| SdpError::ParseError(format!("Invalid clock rate: {}", codec_parts[1])))?;
                                
                                let channels = if codec_parts.len() > 2 {
                                    codec_parts[2].parse()
                                        .map_err(|_| SdpError::ParseError(format!("Invalid channels: {}", codec_parts[2])))?
                                } else {
                                    1
                                };
                                
                                // Find the format with this payload type and update it
                                for format in &mut _media.formats {
                                    if format.payload_type == payload_type {
                                        format.encoding = encoding;
                                        format.clock_rate = clock_rate;
                                        format.channels = channels;
                                        break;
                                    }
                                }
                            },
                            "fmtp" => {
                                // Parse fmtp
                                let fmtp_parts: Vec<&str> = attr_value.splitn(2, ' ').collect();
                                if fmtp_parts.len() != 2 {
                                    return Err(SdpError::ParseError(format!("Invalid fmtp: {}", attr_value)));
                                }
                                
                                let payload_type: u8 = fmtp_parts[0].parse()
                                    .map_err(|_| SdpError::ParseError(format!("Invalid payload type: {}", fmtp_parts[0])))?;
                                
                                // Parse parameters
                                let mut parameters = HashMap::new();
                                for param in fmtp_parts[1].split(';') {
                                    let param_parts: Vec<&str> = param.splitn(2, '=').collect();
                                    if param_parts.len() == 2 {
                                        parameters.insert(param_parts[0].to_string(), param_parts[1].to_string());
                                    }
                                }
                                
                                // Find the format with this payload type and update it
                                for format in &mut _media.formats {
                                    if format.payload_type == payload_type {
                                        format.parameters = parameters;
                                        break;
                                    }
                                }
                            },
                            "sendrecv" | "sendonly" | "recvonly" | "inactive" => {
                                // Parse direction
                                _media.direction = MediaDirection::from_str(attr_name)?;
                            },
                            _ => {
                                // Other attributes
                                _media.attributes.insert(attr_name.to_string(), attr_value.to_string());
                            }
                        }
                    } else {
                        // Session-level attribute
                        sdp.attributes.insert(attr_name.to_string(), attr_value.to_string());
                    }
                },
                _ => {
                    // Ignore other lines for now
                }
            }
        }
        
        // Add the last media section if there is one
        if let Some(media) = current_media {
            sdp.media.push(media);
        }
        
        Ok(sdp)
    }
    
    /// Format SDP to string
    pub fn to_string(&self) -> String {
        let mut sdp = String::new();
        
        // Version
        sdp.push_str(&format!("v={}\r\n", self.version));
        
        // Origin
        sdp.push_str(&format!("o={} {} {} {} {} {}\r\n",
            self.origin.username,
            self.origin.session_id,
            self.origin.session_version,
            self.origin.network_type,
            self.origin.address_type,
            self.origin.unicast_address
        ));
        
        // Session name
        sdp.push_str(&format!("s={}\r\n", self.session_name));
        
        // Connection information
        if let Some(conn) = &self.connection {
            sdp.push_str(&format!("c={} {} {}\r\n",
                conn.network_type,
                conn.address_type,
                conn.connection_address
            ));
        }
        
        // Timing
        for (start, end) in &self.timing {
            sdp.push_str(&format!("t={} {}\r\n", start, end));
        }
        
        // Session-level attributes
        for (name, value) in &self.attributes {
            if value.is_empty() {
                sdp.push_str(&format!("a={}\r\n", name));
            } else {
                sdp.push_str(&format!("a={}:{}\r\n", name, value));
            }
        }
        
        // Media descriptions
        for media in &self.media {
            // Media line
            sdp.push_str(&format!("m={} {} {} {}\r\n",
                media.media_type,
                media.port,
                media.protocol,
                media.format_payload_types()
            ));
            
            // Media attributes
            // Add rtpmap attributes
            for format in &media.formats {
                sdp.push_str(&format.format_rtpmap());
                
                // Add fmtp if present
                if let Some(fmtp) = format.format_fmtp() {
                    sdp.push_str(&fmtp);
                }
            }
            
            // Add direction attribute
            sdp.push_str(&format!("a={}\r\n", media.direction.to_string()));
            
            // Add other media attributes
            for (name, value) in &media.attributes {
                if value.is_empty() {
                    sdp.push_str(&format!("a={}\r\n", name));
                } else {
                    sdp.push_str(&format!("a={}:{}\r\n", name, value));
                }
            }
        }
        
        sdp
    }
    
    /// Extract the RTP port for a specific media type
    pub fn get_rtp_port(&self, media_type: &str) -> Option<u16> {
        self.media.iter()
            .find(|m| m.media_type == media_type)
            .map(|m| m.port)
    }
    
    /// Extract audio RTP port
    pub fn get_audio_port(&self) -> Option<u16> {
        self.get_rtp_port("audio")
    }
}

/// Helper function to extract RTP port from SDP bytes
pub fn extract_rtp_port_from_sdp(sdp: &[u8]) -> Option<u16> {
    use tracing::{debug, warn, trace};
    
    trace!("Extracting RTP port from SDP bytes, length: {}", sdp.len());
    
    // First, convert the SDP bytes to string
    let sdp_str = match std::str::from_utf8(sdp) {
        Ok(s) => s,
        Err(e) => {
            warn!("Invalid UTF-8 in SDP: {}", e);
            return None;
        }
    };
    
    debug!("Parsing SDP:\n{}", sdp_str);
    
    // Try the structured approach first
    match SessionDescription::parse(sdp_str) {
        Ok(session) => {
            let port = session.get_audio_port();
            if let Some(p) = port {
                debug!("Successfully extracted audio RTP port {} using structured parsing", p);
                return Some(p);
            } else {
                warn!("No audio media found in SDP using structured parsing");
            }
        },
        Err(e) => {
            warn!("Failed to parse SDP using structured approach: {}", e);
        }
    }
    
    // Fall back to manual parsing if structured parsing fails
    debug!("Falling back to manual parsing to extract RTP port");
    for (i, line) in sdp_str.lines().enumerate() {
        trace!("SDP line {}: {}", i, line);
        
        if line.starts_with("m=audio ") {
            debug!("Found audio media line: {}", line);
            // Format is "m=audio <port> RTP/AVP..."
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                match parts[1].parse::<u16>() {
                    Ok(port) => {
                        debug!("Manually extracted audio RTP port: {}", port);
                        return Some(port);
                    },
                    Err(e) => {
                        warn!("Failed to parse port number '{}': {}", parts[1], e);
                    }
                }
            } else {
                warn!("Invalid audio media line format, expected at least 3 parts: {}", line);
            }
        }
    }
    
    warn!("Could not extract RTP port from SDP");
    None
}

/// Create a default audio SDP for simple call scenarios
pub fn default_audio(host_str: String, port: u16) -> SessionDescription {
    // Parse host string into IpAddr, defaulting to 127.0.0.1 if it fails
    let host = host_str.parse().unwrap_or_else(|_| IpAddr::from([127, 0, 0, 1]));
    
    // Create default session description
    let mut sdp = SessionDescription::default();
    
    // Update session name
    sdp.session_name = "RVOIP SIP Call".to_string();
    
    // Update origin
    sdp.origin.username = "rvoip".to_string();
    sdp.origin.unicast_address = host;
    
    // Update connection
    if let Some(connection) = &mut sdp.connection {
        connection.connection_address = host;
    }
    
    // Create audio media description
    let mut audio = MediaDescription::new_audio(port);
    audio.add_pcmu();
    audio.add_pcma();
    audio.direction = MediaDirection::SendRecv;
    
    // Add the media description
    sdp.media.push(audio);
    
    // Add session attributes
    sdp.attributes.insert("tool".to_string(), "rvoip".to_string());
    
    sdp
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_sdp() {
        let sdp_str = "v=0\r\n\
                      o=alice 123456 789012 IN IP4 192.168.1.2\r\n\
                      s=Call\r\n\
                      c=IN IP4 192.168.1.2\r\n\
                      t=0 0\r\n\
                      m=audio 10000 RTP/AVP 0 8\r\n\
                      a=rtpmap:0 PCMU/8000\r\n\
                      a=rtpmap:8 PCMA/8000\r\n\
                      a=sendrecv\r\n";
        
        let sdp = SessionDescription::parse(sdp_str).unwrap();
        
        assert_eq!(sdp.version, 0);
        assert_eq!(sdp.origin.username, "alice");
        assert_eq!(sdp.session_name, "Call");
        assert_eq!(sdp.media.len(), 1);
        assert_eq!(sdp.media[0].media_type, "audio");
        assert_eq!(sdp.media[0].port, 10000);
        assert_eq!(sdp.media[0].formats.len(), 2);
        assert_eq!(sdp.media[0].formats[0].payload_type, 0);
        assert_eq!(sdp.media[0].formats[0].encoding, "PCMU");
        assert_eq!(sdp.media[0].direction, MediaDirection::SendRecv);
    }
    
    #[test]
    fn test_format_sdp() {
        let mut sdp = SessionDescription::new_audio_call(
            "alice", 
            "192.168.1.2".parse().unwrap(), 
            10000
        );
        
        // Add another format (PCMA)
        if let Some(media) = sdp.media.first_mut() {
            media.add_pcma();
        }
        
        let sdp_str = sdp.to_string();
        
        // Check that essential elements are present
        assert!(sdp_str.contains("v=0"));
        assert!(sdp_str.contains("o=alice"));
        assert!(sdp_str.contains("c=IN IP4 192.168.1.2"));
        assert!(sdp_str.contains("m=audio 10000 RTP/AVP"));
        assert!(sdp_str.contains("a=rtpmap:0 PCMU/8000"));
        assert!(sdp_str.contains("a=rtpmap:8 PCMA/8000"));
        assert!(sdp_str.contains("a=sendrecv"));
        
        // Parse it back and verify
        let parsed_sdp = SessionDescription::parse(&sdp_str).unwrap();
        assert_eq!(parsed_sdp.get_audio_port(), Some(10000));
    }
    
    #[test]
    fn test_extract_rtp_port() {
        let sdp_str = "v=0\r\n\
                      o=alice 123456 789012 IN IP4 192.168.1.2\r\n\
                      s=Call\r\n\
                      c=IN IP4 192.168.1.2\r\n\
                      t=0 0\r\n\
                      m=audio 12345 RTP/AVP 0\r\n\
                      a=rtpmap:0 PCMU/8000\r\n\
                      a=sendrecv\r\n";
        
        let port = extract_rtp_port_from_sdp(sdp_str.as_bytes());
        assert_eq!(port, Some(12345));
    }
} 