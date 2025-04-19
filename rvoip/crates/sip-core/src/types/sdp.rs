use std::collections::HashMap;
use std::fmt;
use crate::sdp::parser::parse_sdp; // Use the parser
use bytes::Bytes;
use std::str::FromStr;

// Import attribute structs/enums
// Assuming MediaDirection is defined in sdp/attributes.rs for now
use crate::sdp::attributes::MediaDirection; 


// --- Placeholder Attribute Structs --- 
#[derive(Debug, Clone, PartialEq)]
pub struct RtpMapAttribute {
    pub payload_type: u8,
    pub encoding_name: String,
    pub clock_rate: u32,
    pub encoding_params: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FmtpAttribute {
    pub format: String,
    pub parameters: String, 
}

/// Represents a parsed ICE Candidate attribute (RFC 5245 / 8445 / 8839).
/// Structure: foundation component-id transport priority conn-addr port type [related-addr related-port] *(extensions)
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateAttribute {
    pub foundation: String,
    pub component_id: u32,
    pub transport: String, // e.g., "UDP", "TCP"
    pub priority: u32,
    pub connection_address: String, // IP address or FQDN
    pub port: u16,
    pub candidate_type: String, // e.g., "host", "srflx", "prflx", "relay"
    pub related_address: Option<String>,
    pub related_port: Option<u16>,
    // Store extensions as key-value pairs for flexibility
    pub extensions: Vec<(String, Option<String>)>, 
}

/// Represents a parsed SSRC attribute (RFC 5576).
/// Structure: ssrc-id attribute[:value]
#[derive(Debug, Clone, PartialEq, Eq)] // Eq because value is Option<String>
pub struct SsrcAttribute {
    pub ssrc_id: u32,
    pub attribute: String,
    pub value: Option<String>,
}

/// Enum representing a parsed SDP attribute.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedAttribute {
    RtpMap(RtpMapAttribute),
    Fmtp(FmtpAttribute),
    Direction(MediaDirection),
    Ptime(u32),
    Candidate(CandidateAttribute),
    Ssrc(SsrcAttribute),
    
    /// A simple flag attribute (e.g., a=msid-semantic)
    Flag(String),
    /// An attribute with a simple value that wasn't specifically parsed
    Value(String, String),
    /// Fallback for unparsed or unknown attributes (should be rare)
    Other(String, Option<String>),
}

/// Represents the Origin (o=) field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub username: String,
    pub sess_id: String, // Often u64, but spec allows more flexibility
    pub sess_version: String, // Often u64
    pub net_type: String, // IN
    pub addr_type: String, // IP4 / IP6
    pub unicast_address: String, // Hostname or IP address
}

/// Represents the Connection Data (c=) field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionData {
    pub net_type: String, // IN
    pub addr_type: String, // IP4 / IP6
    pub connection_address: String, // IP address or FQDN, potentially with TTL/count
}

/// Represents a Time Description (t=) field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeDescription {
    pub start_time: String, // u64 NTP timestamp
    pub stop_time: String, // u64 NTP timestamp
}

/// Represents a parsed SDP Session.
#[derive(Debug, Clone, PartialEq)] 
pub struct SdpSession {
    pub version: String, 
    pub origin: Origin, // Changed from String
    pub session_name: String, 
    pub connection_info: Option<ConnectionData>, // Changed from Option<String>
    pub time_descriptions: Vec<TimeDescription>, // Changed from Vec<String>
    pub media_descriptions: Vec<MediaDescription>,
    pub direction: Option<MediaDirection>,
    pub generic_attributes: Vec<ParsedAttribute>,
}

impl SdpSession {
    /// Creates a new SdpSession with mandatory origin and session name.
    /// Version defaults to 0, TimeDescription defaults to t=0 0.
    pub fn new(origin: Origin, session_name: impl Into<String>) -> Self {
        Self {
            version: "0".to_string(),
            origin,
            session_name: session_name.into(),
            connection_info: None,
            time_descriptions: vec![TimeDescription { start_time: "0".to_string(), stop_time: "0".to_string()}],
            media_descriptions: Vec::new(),
            direction: None,
            generic_attributes: Vec::new(),
        }
    }

    /// Adds a media description.
    pub fn add_media(&mut self, media: MediaDescription) {
        self.media_descriptions.push(media);
    }
    
    /// Builder method to set session-level connection data.
    pub fn with_connection_data(mut self, conn: ConnectionData) -> Self {
        self.connection_info = Some(conn);
        self
    }
    
    /// Builder method to add a session-level attribute.
     pub fn with_attribute(mut self, attr: ParsedAttribute) -> Self {
        // TODO: Handle setting dedicated fields vs adding to generic?
        self.generic_attributes.push(attr);
        self
    }

    /// Gets the session-level direction attribute, if set.
    pub fn get_direction(&self) -> Option<MediaDirection> {
        self.direction
    }

    /// Finds all session-level rtpmap attributes.
    pub fn rtpmaps(&self) -> impl Iterator<Item = &RtpMapAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::RtpMap(rtpmap) => Some(rtpmap),
            _ => None,
        })
    }

    /// Finds the first session-level rtpmap attribute for a given payload type.
    pub fn get_rtpmap(&self, payload_type: u8) -> Option<&RtpMapAttribute> {
        self.rtpmaps().find(|r| r.payload_type == payload_type)
    }
    
    /// Finds all session-level fmtp attributes.
    pub fn fmtps(&self) -> impl Iterator<Item = &FmtpAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Fmtp(fmtp) => Some(fmtp),
            _ => None,
        })
    }

     /// Finds the first session-level fmtp attribute for a given format.
    pub fn get_fmtp(&self, format: &str) -> Option<&FmtpAttribute> {
        self.fmtps().find(|f| f.format == format)
    }
    
    /// Gets the value of a generic session-level attribute by key.
    pub fn get_generic_attribute_value(&self, key: &str) -> Option<Option<&str>> {
        self.generic_attributes.iter().find_map(|a| match a {
            ParsedAttribute::Value(k, v) if k.eq_ignore_ascii_case(key) => Some(Some(v.as_str())),
            ParsedAttribute::Flag(k) if k.eq_ignore_ascii_case(key) => Some(None),
            ParsedAttribute::Other(k, v) if k.eq_ignore_ascii_case(key) => Some(v.as_deref()),
             // Add checks for dedicated fields if applicable at session level
             ParsedAttribute::Direction(_) if key.eq_ignore_ascii_case(self.direction.map(|d| d.to_string()).as_deref().unwrap_or("")) => Some(None),
            _ => None
        })
    }
}

/// Represents an SDP Media Description section (m=...)
#[derive(Debug, Clone, PartialEq)] 
pub struct MediaDescription {
    pub media: String, 
    pub port: u16,
    pub protocol: String, 
    pub formats: Vec<String>, 
    pub connection_info: Option<ConnectionData>, // Changed from Option<String>

    // --- Media-level Attributes ---
    // Dedicated fields for common single-value attributes
    pub ptime: Option<u32>,
    pub direction: Option<MediaDirection>,
    // Add others like: pub rtcp_port: Option<u16>, pub mid: Option<String>, etc.
    
    // Vector for repeatable or less common/generic attributes
    pub generic_attributes: Vec<ParsedAttribute>,
}

impl MediaDescription {
    /// Creates a new MediaDescription.
    pub fn new(
        media: impl Into<String>, 
        port: u16, 
        protocol: impl Into<String>, 
        formats: Vec<String>
    ) -> Self {
        Self {
            media: media.into(),
            port,
            protocol: protocol.into(),
            formats,
            connection_info: None,
            ptime: None,
            direction: None,
            generic_attributes: Vec::new(),
        }
    }

     /// Builder method to set media-level connection data.
    pub fn with_connection_data(mut self, conn: ConnectionData) -> Self {
        self.connection_info = Some(conn);
        self
    }
    
    /// Builder method to add a media-level attribute.
     pub fn with_attribute(mut self, attr: ParsedAttribute) -> Self {
        // TODO: Handle setting dedicated fields vs adding to generic?
         match attr {
            ParsedAttribute::Ptime(v) => { self.ptime = Some(v); }
            ParsedAttribute::Direction(d) => { self.direction = Some(d); }
            _ => self.generic_attributes.push(attr),
        }
        self
    }

    /// Gets the media-level direction attribute, if set.
    pub fn get_direction(&self) -> Option<MediaDirection> {
        self.direction
    }
    
    /// Gets the media-level ptime attribute, if set.
    pub fn get_ptime(&self) -> Option<u32> {
        self.ptime
    }

    /// Finds all media-level rtpmap attributes.
    pub fn rtpmaps(&self) -> impl Iterator<Item = &RtpMapAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::RtpMap(rtpmap) => Some(rtpmap),
            _ => None,
        })
    }
    
    /// Finds the first media-level rtpmap attribute for a given payload type.
    pub fn get_rtpmap(&self, payload_type: u8) -> Option<&RtpMapAttribute> {
        self.rtpmaps().find(|r| r.payload_type == payload_type)
    }

    /// Finds all media-level fmtp attributes.
    pub fn fmtps(&self) -> impl Iterator<Item = &FmtpAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Fmtp(fmtp) => Some(fmtp),
            _ => None,
        })
    }

     /// Finds the first media-level fmtp attribute for a given format.
    pub fn get_fmtp(&self, format: &str) -> Option<&FmtpAttribute> {
        self.fmtps().find(|f| f.format == format)
    }
    
    /// Finds all media-level candidate attributes.
    pub fn candidates(&self) -> impl Iterator<Item = &CandidateAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Candidate(candidate) => Some(candidate),
            _ => None,
        })
    }
    
    /// Finds all media-level ssrc attributes.
    pub fn ssrcs(&self) -> impl Iterator<Item = &SsrcAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Ssrc(ssrc) => Some(ssrc),
            _ => None,
        })
    }
    
     /// Gets the value of a generic media-level attribute by key.
    pub fn get_generic_attribute_value(&self, key: &str) -> Option<Option<&str>> {
        self.generic_attributes.iter().find_map(|a| match a {
            ParsedAttribute::Value(k, v) if k.eq_ignore_ascii_case(key) => Some(Some(v.as_str())),
            ParsedAttribute::Flag(k) if k.eq_ignore_ascii_case(key) => Some(None),
            ParsedAttribute::Other(k, v) if k.eq_ignore_ascii_case(key) => Some(v.as_deref()),
             // Add checks for dedicated fields
             ParsedAttribute::Ptime(v) if key.eq_ignore_ascii_case("ptime") => Some(Some(Box::leak(v.to_string().into_boxed_str()))), // Leak! Needs better way
             ParsedAttribute::Direction(_) if key.eq_ignore_ascii_case(self.direction.map(|d| d.to_string()).as_deref().unwrap_or("")) => Some(None),
            _ => None
        })
    }
}

impl FromStr for SdpSession {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Convert string to Bytes and parse
        parse_sdp(&Bytes::from(s.as_bytes()))
    }
}


impl fmt::Display for ParsedAttribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParsedAttribute::RtpMap(rtpmap) => {
                write!(f, "a=rtpmap:{} {}/{}", rtpmap.payload_type, rtpmap.encoding_name, rtpmap.clock_rate)?;
                if let Some(params) = &rtpmap.encoding_params {
                    write!(f, "/{}", params)?;
                }
                Ok(())
            }
            ParsedAttribute::Fmtp(fmtp) => write!(f, "a=fmtp:{} {}", fmtp.format, fmtp.parameters),
            ParsedAttribute::Direction(dir) => {
                let dir_str = match dir {
                    MediaDirection::SendRecv => "sendrecv",
                    MediaDirection::SendOnly => "sendonly",
                    MediaDirection::RecvOnly => "recvonly",
                    MediaDirection::Inactive => "inactive",
                };
                write!(f, "a={}", dir_str)
            }
            ParsedAttribute::Ptime(time) => write!(f, "a=ptime:{}", time),
            ParsedAttribute::Flag(key) => write!(f, "a={}", key),
            ParsedAttribute::Value(key, value) => write!(f, "a={}:{}", key, value),
            ParsedAttribute::Other(key, Some(value)) => write!(f, "a={}:{}", key, value), // Fallback with colon
            ParsedAttribute::Other(key, None) => write!(f, "a={}", key), // Fallback flag
            ParsedAttribute::Candidate(candidate) => {
                write!(f, "a=candidate:{} {} {} {} {} {} typ {}", 
                    candidate.foundation, candidate.component_id, candidate.transport, 
                    candidate.priority, candidate.connection_address, candidate.port, 
                    candidate.candidate_type)?;
                if let Some(raddr) = &candidate.related_address {
                    write!(f, " raddr {}", raddr)?;
                }
                if let Some(rport) = candidate.related_port {
                     write!(f, " rport {}", rport)?;
                }
                for (ext_key, ext_value) in &candidate.extensions {
                    write!(f, " {}", ext_key)?;
                    if let Some(value) = ext_value {
                        write!(f, " {}", value)?;
                    }
                }
                Ok(())
            }
            ParsedAttribute::Ssrc(ssrc) => {
                write!(f, "a=ssrc:{} {}", ssrc.ssrc_id, ssrc.attribute)?;
                if let Some(val) = &ssrc.value {
                    write!(f, ":{}", val)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {} {} {} {}", 
            self.username, self.sess_id, self.sess_version, 
            self.net_type, self.addr_type, self.unicast_address)
    }
}

impl fmt::Display for ConnectionData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
         write!(f, "{} {} {}", self.net_type, self.addr_type, self.connection_address)
    }
}

impl fmt::Display for TimeDescription {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
         write!(f, "{} {}", self.start_time, self.stop_time)
    }
}

impl fmt::Display for MediaDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // m=
        write!(f, "m={} {} {} {}\r\n", 
            self.media, self.port, self.protocol, self.formats.join(" "))?;
        
        // Optional c=
        if let Some(conn) = &self.connection_info {
            write!(f, "c={}\r\n", conn)?;
        }

        // Dedicated Attributes for Media
        if let Some(ptime) = self.ptime {
            write!(f, "a=ptime:{}\r\n", ptime)?;
        }
         if let Some(direction) = self.direction {
            let dir_str = match direction {
                MediaDirection::SendRecv => "sendrecv",
                MediaDirection::SendOnly => "sendonly",
                MediaDirection::RecvOnly => "recvonly",
                MediaDirection::Inactive => "inactive",
            };
            write!(f, "a={}\r\n", dir_str)?;
        }

        // Other attributes for this media
        for attr in &self.generic_attributes {
            // Avoid re-printing attributes handled by dedicated fields
            if matches!(attr, ParsedAttribute::Ptime(_) | ParsedAttribute::Direction(_)) {
                continue;
            }
            write!(f, "{}\\r\\n", attr)?; 
        }
        Ok(())
    }
}

impl fmt::Display for SdpSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Mandatory lines
        write!(f, "v={}\r\n", self.version)?;
        write!(f, "o={}\r\n", self.origin)?;
        write!(f, "s={}\r\n", self.session_name)?;
        
        // Optional session c=
        if let Some(conn) = &self.connection_info {
            write!(f, "c={}\r\n", conn)?;
        }
        
        // t=
        for time in &self.time_descriptions {
             write!(f, "t={}\r\n", time)?;
        }
        
        // Dedicated Session Attributes
         if let Some(direction) = self.direction {
             let dir_str = match direction {
                MediaDirection::SendRecv => "sendrecv",
                MediaDirection::SendOnly => "sendonly",
                MediaDirection::RecvOnly => "recvonly",
                MediaDirection::Inactive => "inactive",
            };
            write!(f, "a={}\r\n", dir_str)?;
        }
        // Add other dedicated session attributes here...

        // Other session-level attributes
        for attr in &self.generic_attributes {
             // Avoid re-printing attributes handled by dedicated fields
             if matches!(attr, ParsedAttribute::Direction(_)) {
                 continue;
             }
            write!(f, "{}\\r\\n", attr)?;
        }

        // Media descriptions
        for media in &self.media_descriptions {
            write!(f, "{}", media)?;
        }

        Ok(())
    }
} 