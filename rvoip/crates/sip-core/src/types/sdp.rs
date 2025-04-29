/// SDP (Session Description Protocol) module implements RFC 4566 for creating, parsing,
/// and manipulating SDP messages used in SIP (Session Initiation Protocol) communications.
/// 
/// This module provides a complete implementation for working with SDP, including:
/// - Parsing SDP text into structured data
/// - Representing SDP sessions, media descriptions, and attributes
/// - Serializing SDP data back to standard format
///
/// # References
/// - [RFC 4566: Session Description Protocol](https://tools.ietf.org/html/rfc4566)
/// - [RFC 5245: ICE](https://tools.ietf.org/html/rfc5245) (for ICE candidates)
/// - [RFC 5576: Source-Specific Media Attributes](https://tools.ietf.org/html/rfc5576) (for SSRC)
use std::collections::HashMap;
use std::fmt;
use bytes::Bytes;
use std::str::FromStr;
use crate::types::uri::Uri;
use crate::types::param::Param;
use serde::{Serialize, Deserialize};
use crate::error::{Error, Result};
use crate::sdp::parser::parse_sdp;

// Import attribute structs/enums from the correct location
use crate::sdp::attributes::MediaDirection; // Keep this
// Remove other potential imports from crate::sdp if they were added erroneously

// --- Placeholder Attribute Structs --- 
/// Represents an RTP Map attribute (a=rtpmap)
/// 
/// Maps RTP payload types to media encoding names, clock rates, and encoding parameters.
/// Format: `a=rtpmap:<payload type> <encoding name>/<clock rate>[/<encoding parameters>]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RtpMapAttribute {
    /// RTP payload type (numeric)
    pub payload_type: u8,
    /// Encoding name (e.g., "PCMU", "H264")
    pub encoding_name: String,
    /// Clock rate in Hertz
    pub clock_rate: u32,
    /// Optional encoding parameters (e.g., number of channels)
    pub encoding_params: Option<String>,
}

/// Represents a Format Parameters attribute (a=fmtp)
///
/// Provides additional parameters for a specified format.
/// Format: `a=fmtp:<format> <parameters>`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FmtpAttribute {
    /// Format identifier (typically the payload type from rtpmap)
    pub format: String,
    /// Format-specific parameters as a raw string
    pub parameters: String, 
}

/// Represents a parsed ICE Candidate attribute (RFC 5245 / 8445 / 8839).
///
/// Format: `a=candidate:<foundation> <component-id> <transport> <priority> 
/// <connection-address> <port> typ <candidate-type> 
/// [raddr <related-address>] [rport <related-port>] [...]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateAttribute {
    /// Unique identifier for this candidate
    pub foundation: String,
    /// Component ID (1 for RTP, 2 for RTCP)
    pub component_id: u32,
    /// Transport protocol (e.g., "UDP", "TCP")
    pub transport: String,
    /// Candidate priority
    pub priority: u32,
    /// Connection address (IP or FQDN)
    pub connection_address: String,
    /// Port number
    pub port: u16,
    /// Candidate type (e.g., "host", "srflx", "prflx", "relay")
    pub candidate_type: String,
    /// Related address for reflexive/relay candidates
    pub related_address: Option<String>,
    /// Related port for reflexive/relay candidates
    pub related_port: Option<u16>,
    /// Additional extension attributes as key-value pairs
    pub extensions: Vec<(String, Option<String>)>, 
}

/// Represents a parsed SSRC attribute (RFC 5576).
///
/// Format: `a=ssrc:<ssrc-id> <attribute>[:<value>]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SsrcAttribute {
    /// SSRC identifier
    pub ssrc_id: u32,
    /// Attribute name
    pub attribute: String,
    /// Optional attribute value
    pub value: Option<String>,
}

/// Enum representing a parsed SDP attribute.
///
/// SDP attributes provide detailed information about the session or media.
/// All attributes follow the format `a=<attribute>` or `a=<attribute>:<value>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParsedAttribute {
    /// RTP mapping attribute (a=rtpmap)
    RtpMap(RtpMapAttribute),
    /// Format parameters attribute (a=fmtp)
    Fmtp(FmtpAttribute),
    /// Media direction attribute (a=sendrecv, a=sendonly, a=recvonly, a=inactive)
    Direction(MediaDirection),
    /// Packetization time attribute (a=ptime)
    Ptime(u32),
    /// Maximum packetization time attribute (a=maxptime)
    MaxPtime(u32),
    /// ICE candidate attribute (a=candidate)
    Candidate(CandidateAttribute),
    /// SSRC attribute (a=ssrc)
    Ssrc(SsrcAttribute),
    /// ICE username fragment (a=ice-ufrag)
    IceUfrag(String),
    /// ICE password (a=ice-pwd)
    IcePwd(String),
    /// Fingerprint for DTLS-SRTP (a=fingerprint)
    Fingerprint(String, String), // (hash-function, fingerprint)
    /// Connection setup role for DTLS (a=setup)
    Setup(String), // active, passive, actpass, holdconn
    /// Media ID for grouping (a=mid)
    Mid(String),
    /// Group attribute for bundling (a=group)
    Group(String, Vec<String>), // (semantics, mids)
    /// RTP/RTCP multiplexing flag (a=rtcp-mux)
    RtcpMux,
    /// RTCP feedback mechanism (a=rtcp-fb)
    RtcpFb(String, String, Option<String>), // (payload-type, feedback-type, additional-params)
    /// RTP header extension mapping (a=extmap)
    ExtMap(u16, Option<String>, String, Option<String>), // (id, direction, uri, parameters)
    /// Media stream identifier (a=msid)
    Msid(String, Option<String>), // (stream-id, track-id)
    /// Bandwidth information (b=)
    Bandwidth(String, u32), // (bwtype, bandwidth)
    
    /// A simple flag attribute (e.g., a=msid-semantic)
    Flag(String),
    /// An attribute with a simple value that wasn't specifically parsed
    Value(String, String),
    /// Fallback for unparsed or unknown attributes (should be rare)
    Other(String, Option<String>),
}

/// Represents the Origin (o=) field in an SDP message.
///
/// Format: `o=<username> <sess-id> <sess-version> <nettype> <addrtype> <unicast-address>`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    /// Username of the originator (often "-")
    pub username: String,
    /// Session ID (unique identifier for this session)
    pub sess_id: String, // Often u64, but spec allows more flexibility
    /// Session version (increments when session is modified)
    pub sess_version: String, // Often u64
    /// Network type (typically "IN" for Internet)
    pub net_type: String,
    /// Address type ("IP4" or "IP6")
    pub addr_type: String,
    /// Unicast address (hostname or IP address)
    pub unicast_address: String,
}

/// Represents the Connection Data (c=) field in an SDP message.
///
/// Format: `c=<nettype> <addrtype> <connection-address>`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionData {
    /// Network type (typically "IN" for Internet)
    pub net_type: String,
    /// Address type ("IP4" or "IP6")
    pub addr_type: String,
    /// Connection address (IP address or FQDN, potentially with TTL/count)
    pub connection_address: String,
}

/// Represents a Time Description (t=) field in an SDP message.
///
/// Format: `t=<start-time> <stop-time>`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeDescription {
    /// Start time (NTP timestamp, 0 means session is permanent)
    pub start_time: String,
    /// Stop time (NTP timestamp, 0 means open-ended)
    pub stop_time: String,
}

/// Represents a complete SDP session with all its components.
///
/// An SDP session defines a multimedia session including connection information,
/// timing, and media descriptions for audio, video, and other streams.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] 
pub struct SdpSession {
    /// SDP protocol version (v=)
    pub version: String, 
    /// Session origin information (o=)
    pub origin: Origin,
    /// Session name (s=)
    pub session_name: String, 
    /// Optional connection information (c=)
    pub connection_info: Option<ConnectionData>,
    /// Time descriptions (t=)
    pub time_descriptions: Vec<TimeDescription>,
    /// Media descriptions (m=)
    pub media_descriptions: Vec<MediaDescription>,
    /// Session-level media direction
    pub direction: Option<MediaDirection>,
    /// Session-level attributes (a=)
    pub generic_attributes: Vec<ParsedAttribute>,
}

impl SdpSession {
    /// Creates a new SdpSession with mandatory origin and session name.
    ///
    /// Version defaults to 0, TimeDescription defaults to t=0 0 (permanent session).
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{SdpSession, Origin};
    /// let origin = Origin {
    ///     username: "-".to_string(),
    ///     sess_id: "1234567890".to_string(),
    ///     sess_version: "2".to_string(),
    ///     net_type: "IN".to_string(),
    ///     addr_type: "IP4".to_string(),
    ///     unicast_address: "192.168.1.100".to_string(),
    /// };
    ///
    /// let session = SdpSession::new(origin, "SIP Call");
    /// ```
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

    /// Adds a media description to the session.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{SdpSession, Origin, MediaDescription};
    /// # let origin = Origin {
    /// #    username: "-".to_string(),
    /// #    sess_id: "1234567890".to_string(),
    /// #    sess_version: "2".to_string(),
    /// #    net_type: "IN".to_string(),
    /// #    addr_type: "IP4".to_string(),
    /// #    unicast_address: "192.168.1.100".to_string(),
    /// # };
    /// # let mut session = SdpSession::new(origin, "SIP Call");
    /// let audio_media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string(), "8".to_string()]);
    /// session.add_media(audio_media);
    /// ```
    pub fn add_media(&mut self, media: MediaDescription) {
        self.media_descriptions.push(media);
    }
    
    /// Sets session-level connection data.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{SdpSession, Origin, ConnectionData};
    /// # let origin = Origin {
    /// #    username: "-".to_string(),
    /// #    sess_id: "1234567890".to_string(),
    /// #    sess_version: "2".to_string(),
    /// #    net_type: "IN".to_string(),
    /// #    addr_type: "IP4".to_string(),
    /// #    unicast_address: "192.168.1.100".to_string(),
    /// # };
    /// # let session = SdpSession::new(origin, "SIP Call");
    /// let conn = ConnectionData {
    ///     net_type: "IN".to_string(),
    ///     addr_type: "IP4".to_string(),
    ///     connection_address: "192.168.1.100".to_string(),
    /// };
    /// let session = session.with_connection_data(conn);
    /// ```
    pub fn with_connection_data(mut self, conn: ConnectionData) -> Self {
        self.connection_info = Some(conn);
        self
    }
    
    /// Adds a session-level attribute.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{SdpSession, Origin, ParsedAttribute};
    /// # let origin = Origin {
    /// #    username: "-".to_string(),
    /// #    sess_id: "1234567890".to_string(),
    /// #    sess_version: "2".to_string(),
    /// #    net_type: "IN".to_string(),
    /// #    addr_type: "IP4".to_string(),
    /// #    unicast_address: "192.168.1.100".to_string(),
    /// # };
    /// # let session = SdpSession::new(origin, "SIP Call");
    /// let session = session.with_attribute(ParsedAttribute::Flag("ice-lite".to_string()));
    /// ```
     pub fn with_attribute(mut self, attr: ParsedAttribute) -> Self {
        // TODO: Handle setting dedicated fields vs adding to generic?
        self.generic_attributes.push(attr);
        self
    }

    /// Gets the session-level media direction attribute, if set.
    pub fn get_direction(&self) -> Option<MediaDirection> {
        self.direction
    }

    /// Finds all session-level rtpmap attributes.
    ///
    /// Returns an iterator over references to all RtpMapAttribute values.
    pub fn rtpmaps(&self) -> impl Iterator<Item = &RtpMapAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::RtpMap(rtpmap) => Some(rtpmap),
            _ => None,
        })
    }

    /// Finds the first session-level rtpmap attribute for a given payload type.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{SdpSession, Origin};
    /// # let session = SdpSession::new(
    /// #    Origin {
    /// #        username: "-".to_string(),
    /// #        sess_id: "1234567890".to_string(),
    /// #        sess_version: "2".to_string(),
    /// #        net_type: "IN".to_string(),
    /// #        addr_type: "IP4".to_string(),
    /// #        unicast_address: "192.168.1.100".to_string(),
    /// #    }, 
    /// #    "SIP Call"
    /// # );
    /// // Get the rtpmap for PCMU (payload type 0)
    /// if let Some(rtpmap) = session.get_rtpmap(0) {
    ///     println!("Encoding: {}, Clock rate: {}", rtpmap.encoding_name, rtpmap.clock_rate);
    /// }
    /// ```
    pub fn get_rtpmap(&self, payload_type: u8) -> Option<&RtpMapAttribute> {
        self.rtpmaps().find(|r| r.payload_type == payload_type)
    }
    
    /// Finds all session-level fmtp attributes.
    ///
    /// Returns an iterator over references to all FmtpAttribute values.
    pub fn fmtps(&self) -> impl Iterator<Item = &FmtpAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Fmtp(fmtp) => Some(fmtp),
            _ => None,
        })
    }

    /// Finds the first session-level fmtp attribute for a given format.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{SdpSession, Origin};
    /// # let session = SdpSession::new(
    /// #    Origin {
    /// #        username: "-".to_string(),
    /// #        sess_id: "1234567890".to_string(),
    /// #        sess_version: "2".to_string(),
    /// #        net_type: "IN".to_string(),
    /// #        addr_type: "IP4".to_string(),
    /// #        unicast_address: "192.168.1.100".to_string(),
    /// #    }, 
    /// #    "SIP Call"
    /// # );
    /// // Get fmtp parameters for payload type 96
    /// if let Some(fmtp) = session.get_fmtp("96") {
    ///     println!("Parameters: {}", fmtp.parameters);
    /// }
    /// ```
    pub fn get_fmtp(&self, format: &str) -> Option<&FmtpAttribute> {
        self.fmtps().find(|f| f.format == format)
    }
    
    /// Gets the value of a generic session-level attribute by key.
    ///
    /// Returns:
    /// - `Some(Some(value))` for attributes with values
    /// - `Some(None)` for flag attributes
    /// - `None` if the attribute doesn't exist
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{SdpSession, Origin, ParsedAttribute};
    /// # let mut session = SdpSession::new(
    /// #    Origin {
    /// #        username: "-".to_string(),
    /// #        sess_id: "1234567890".to_string(),
    /// #        sess_version: "2".to_string(),
    /// #        net_type: "IN".to_string(),
    /// #        addr_type: "IP4".to_string(),
    /// #        unicast_address: "192.168.1.100".to_string(),
    /// #    }, 
    /// #    "SIP Call"
    /// # );
    /// # session = session.with_attribute(ParsedAttribute::Value("group".to_string(), "BUNDLE audio video".to_string()));
    /// if let Some(Some(value)) = session.get_generic_attribute_value("group") {
    ///     println!("Group attribute: {}", value);
    /// }
    /// ```
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
///
/// A media description defines a single media stream (audio, video, etc.)
/// with its transport information and attributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] 
pub struct MediaDescription {
    /// Media type (e.g., "audio", "video", "application")
    pub media: String, 
    /// Transport port number
    pub port: u16,
    /// Transport protocol (e.g., "RTP/AVP", "RTP/SAVP", "UDP/TLS/RTP/SAVP")
    pub protocol: String, 
    /// Media format descriptions (payload types or format identifiers)
    pub formats: Vec<String>, 
    /// Media-specific connection information (overrides session-level)
    pub connection_info: Option<ConnectionData>,

    // --- Media-level Attributes ---
    /// Packetization time in milliseconds (a=ptime)
    pub ptime: Option<u32>,
    /// Media direction attribute
    pub direction: Option<MediaDirection>,
    // Add others like: pub rtcp_port: Option<u16>, pub mid: Option<String>, etc.
    
    /// Other media-level attributes
    pub generic_attributes: Vec<ParsedAttribute>,
}

impl MediaDescription {
    /// Creates a new MediaDescription.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::MediaDescription;
    /// // Create an audio media section for RTP/AVP with PCMU (0) and PCMA (8)
    /// let audio = MediaDescription::new(
    ///     "audio", 
    ///     49170, 
    ///     "RTP/AVP", 
    ///     vec!["0".to_string(), "8".to_string()]
    /// );
    ///
    /// // Create a video media section for RTP/AVP with H264 (96)
    /// let video = MediaDescription::new(
    ///     "video",
    ///     49174,
    ///     "RTP/AVP",
    ///     vec!["96".to_string()]
    /// );
    /// ```
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

    /// Sets media-level connection data.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{MediaDescription, ConnectionData};
    /// # let media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
    /// let conn = ConnectionData {
    ///     net_type: "IN".to_string(),
    ///     addr_type: "IP4".to_string(),
    ///     connection_address: "192.168.1.100".to_string(),
    /// };
    /// let media = media.with_connection_data(conn);
    /// ```
    pub fn with_connection_data(mut self, conn: ConnectionData) -> Self {
        self.connection_info = Some(conn);
        self
    }
    
    /// Adds a media-level attribute.
    ///
    /// Certain attribute types (ptime, direction) will be stored in dedicated fields.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{MediaDescription, ParsedAttribute, RtpMapAttribute};
    /// # let media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
    /// // Add an rtpmap attribute
    /// let rtpmap = RtpMapAttribute {
    ///     payload_type: 0,
    ///     encoding_name: "PCMU".to_string(),
    ///     clock_rate: 8000,
    ///     encoding_params: None,
    /// };
    /// let media = media.with_attribute(ParsedAttribute::RtpMap(rtpmap));
    /// ```
    pub fn with_attribute(mut self, attr: ParsedAttribute) -> Self {
        // Handle setting dedicated fields vs adding to generic collection
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
    ///
    /// Returns an iterator over references to all RtpMapAttribute values.
    pub fn rtpmaps(&self) -> impl Iterator<Item = &RtpMapAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::RtpMap(rtpmap) => Some(rtpmap),
            _ => None,
        })
    }
    
    /// Finds the first media-level rtpmap attribute for a given payload type.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::MediaDescription;
    /// # let media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
    /// // Get the rtpmap for payload type 0 (PCMU)
    /// if let Some(rtpmap) = media.get_rtpmap(0) {
    ///     println!("Encoding: {}, Clock rate: {}", rtpmap.encoding_name, rtpmap.clock_rate);
    /// }
    /// ```
    pub fn get_rtpmap(&self, payload_type: u8) -> Option<&RtpMapAttribute> {
        self.rtpmaps().find(|r| r.payload_type == payload_type)
    }

    /// Finds all media-level fmtp attributes.
    ///
    /// Returns an iterator over references to all FmtpAttribute values.
    pub fn fmtps(&self) -> impl Iterator<Item = &FmtpAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Fmtp(fmtp) => Some(fmtp),
            _ => None,
        })
    }

    /// Finds the first media-level fmtp attribute for a given format.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::MediaDescription;
    /// # let media = MediaDescription::new("video", 49174, "RTP/AVP", vec!["96".to_string()]);
    /// // Get format parameters for payload type 96 (often H.264)
    /// if let Some(fmtp) = media.get_fmtp("96") {
    ///     println!("H.264 parameters: {}", fmtp.parameters);
    /// }
    /// ```
    pub fn get_fmtp(&self, format: &str) -> Option<&FmtpAttribute> {
        self.fmtps().find(|f| f.format == format)
    }
    
    /// Finds all media-level ICE candidate attributes.
    ///
    /// Returns an iterator over references to all CandidateAttribute values.
    pub fn candidates(&self) -> impl Iterator<Item = &CandidateAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Candidate(candidate) => Some(candidate),
            _ => None,
        })
    }
    
    /// Finds all media-level SSRC attributes.
    ///
    /// Returns an iterator over references to all SsrcAttribute values.
    pub fn ssrcs(&self) -> impl Iterator<Item = &SsrcAttribute> {
        self.generic_attributes.iter().filter_map(|a| match a {
            ParsedAttribute::Ssrc(ssrc) => Some(ssrc),
            _ => None,
        })
    }
    
    /// Gets the value of a generic media-level attribute by key.
    ///
    /// Returns:
    /// - `Some(Some(value))` for attributes with values
    /// - `Some(None)` for flag attributes
    /// - `None` if the attribute doesn't exist
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip_core::types::sdp::{MediaDescription, ParsedAttribute};
    /// # let mut media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
    /// # media = media.with_attribute(ParsedAttribute::Value("mid".to_string(), "audio".to_string()));
    /// if let Some(Some(value)) = media.get_generic_attribute_value("mid") {
    ///     println!("Media ID: {}", value);
    /// }
    /// ```
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

    /// Parses an SDP string into an SdpSession.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::str::FromStr;
    /// # use rvoip_sip_core::types::sdp::SdpSession;
    /// let sdp_str = "v=0\r\n\
    ///     o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r\n\
    ///     s=SDP Seminar\r\n\
    ///     t=0 0\r\n\
    ///     m=audio 49170 RTP/AVP 0\r\n\
    ///     a=rtpmap:0 PCMU/8000\r\n";
    ///
    /// match SdpSession::from_str(sdp_str) {
    ///     Ok(session) => println!("Parsed SDP with {} media sections", session.media_descriptions.len()),
    ///     Err(e) => println!("Failed to parse SDP: {}", e),
    /// }
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        // Convert string to owned Bytes and parse
        parse_sdp(&Bytes::copy_from_slice(s.as_bytes()))
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
            ParsedAttribute::MaxPtime(time) => write!(f, "a=maxptime:{}", time),
            ParsedAttribute::Candidate(candidate) => {
                write!(f, "a=candidate:{} {} {} {} {} {} typ {}", 
                    candidate.foundation, candidate.component_id, candidate.transport, 
                    candidate.priority, candidate.connection_address, candidate.port,
                    candidate.candidate_type)?;
                
                if let Some(addr) = &candidate.related_address {
                    write!(f, " raddr {}", addr)?;
                }
                
                if let Some(port) = candidate.related_port {
                    write!(f, " rport {}", port)?;
                }
                
                for (key, value) in &candidate.extensions {
                    if let Some(val) = value {
                        write!(f, " {} {}", key, val)?;
                    } else {
                        write!(f, " {}", key)?;
                    }
                }
                
                Ok(())
            }
            ParsedAttribute::Ssrc(ssrc) => {
                write!(f, "a=ssrc:{} {}", ssrc.ssrc_id, ssrc.attribute)?;
                if let Some(value) = &ssrc.value {
                    write!(f, ":{}", value)?;
                }
                Ok(())
            }
            ParsedAttribute::IceUfrag(ufrag) => write!(f, "a=ice-ufrag:{}", ufrag),
            ParsedAttribute::IcePwd(pwd) => write!(f, "a=ice-pwd:{}", pwd),
            ParsedAttribute::Fingerprint(hash_func, fingerprint) => write!(f, "a=fingerprint:{} {}", hash_func, fingerprint),
            ParsedAttribute::Setup(role) => write!(f, "a=setup:{}", role),
            ParsedAttribute::Mid(mid) => write!(f, "a=mid:{}", mid),
            ParsedAttribute::Group(semantics, mids) => {
                write!(f, "a=group:{}", semantics)?;
                for mid in mids {
                    write!(f, " {}", mid)?;
                }
                Ok(())
            }
            ParsedAttribute::RtcpMux => write!(f, "a=rtcp-mux"),
            ParsedAttribute::RtcpFb(payload_type, fb_type, params) => {
                write!(f, "a=rtcp-fb:{} {}", payload_type, fb_type)?;
                if let Some(p) = params {
                    write!(f, " {}", p)?;
                }
                Ok(())
            }
            ParsedAttribute::ExtMap(id, direction, uri, params) => {
                write!(f, "a=extmap:{}", id)?;
                if let Some(dir) = direction {
                    write!(f, "/{}", dir)?;
                }
                write!(f, " {}", uri)?;
                if let Some(p) = params {
                    write!(f, " {}", p)?;
                }
                Ok(())
            }
            ParsedAttribute::Msid(stream_id, track_id) => {
                write!(f, "a=msid:{}", stream_id)?;
                if let Some(track) = track_id {
                    write!(f, " {}", track)?;
                }
                Ok(())
            }
            ParsedAttribute::Bandwidth(bwtype, bandwidth) => write!(f, "b={}:{}", bwtype, bandwidth),
            ParsedAttribute::Flag(key) => write!(f, "a={}", key),
            ParsedAttribute::Value(key, value) => write!(f, "a={}:{}", key, value),
            ParsedAttribute::Other(key, Some(value)) => write!(f, "a={}:{}", key, value), // Fallback with colon
            ParsedAttribute::Other(key, None) => write!(f, "a={}", key), // Fallback flag
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
            write!(f, "{}\r\n", attr)?; 
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
            write!(f, "{}\r\n", attr)?;
        }

        // Media descriptions
        for media in &self.media_descriptions {
            write!(f, "{}", media)?;
        }

        Ok(())
    }
} 