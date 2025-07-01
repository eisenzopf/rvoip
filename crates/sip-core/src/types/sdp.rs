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
#[cfg(feature = "sdp")]
use crate::sdp::parser::parse_sdp;
#[cfg(feature = "sdp")]
use crate::sdp::attributes;
#[cfg(feature = "sdp")]
use crate::sdp::attributes::MediaDirection;
#[cfg(feature = "sdp")]
use crate::sdp::attributes::rid::{RidAttribute, RidDirection};

// Provide local definitions of types used from sdp module when the feature is not enabled
#[cfg(not(feature = "sdp"))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MediaDirection {
    /// Both send and receive
    SendRecv,
    /// Send only
    SendOnly,
    /// Receive only
    RecvOnly,
    /// Neither send nor receive
    Inactive,
}

#[cfg(not(feature = "sdp"))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RidDirection {
    /// Send direction
    Send,
    /// Receive direction
    Recv,
}

#[cfg(not(feature = "sdp"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RidAttribute {
    /// The RID identifier
    pub id: String,
    /// The direction (send/recv)
    pub direction: RidDirection,
    /// Format restrictions
    pub formats: Option<Vec<String>>,
    /// Payload type restrictions
    pub pt: Option<Vec<String>>,
    /// Additional parameters
    pub params: HashMap<String, String>,
}

// Now all references to these types should work regardless of whether the sdp feature is enabled

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

/// A parsed attribute, identified by its type
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum ParsedAttribute {
    /// RTP format parameters, corresponds to a=rtpmap:<payload type> <encoding name>/<clock rate>[/<encoding parameters>]
    RtpMap(RtpMapAttribute),
    /// Format parameters, corresponds to a=fmtp:<format> <format specific parameters>
    Fmtp(FmtpAttribute),
    /// Media direction attributes: a=sendrecv, a=sendonly, a=recvonly, a=inactive
    Direction(MediaDirection),
    /// Packetization time, corresponds to a=ptime:<packet time>
    Ptime(u64),
    /// Maximum packetization time, corresponds to a=maxptime:<maximum packet time>
    MaxPtime(u64),
    /// ICE candidate, corresponds to a=candidate:<foundation> <component-id> <transport> <priority> <connection-address> <port> typ <cand-type> [raddr <rel-addr>] [rport <rel-port>] [tcptype <tcp-type>] [generation <generation>]
    Candidate(CandidateAttribute),
    /// SSRC attribute, corresponds to a=ssrc:<ssrc-id> <attribute>[:value]
    Ssrc(SsrcAttribute),
    /// ICE username fragment, corresponds to a=ice-ufrag:<username>
    IceUfrag(String),
    /// ICE password, corresponds to a=ice-pwd:<password>
    IcePwd(String),
    /// DTLS fingerprint, corresponds to a=fingerprint:<hash-function> <fingerprint>
    Fingerprint(String, String),
    /// DTLS setup role, corresponds to a=setup:<role>
    Setup(String),
    /// Media identification, corresponds to a=mid:<identification-tag>
    Mid(String),
    /// Media grouping, corresponds to a=group:<semantics> <id> <id> ...
    Group(String, Vec<String>),
    /// RTCP multiplexing, corresponds to a=rtcp-mux
    RtcpMux,
    /// RTCP feedback, corresponds to a=rtcp-fb:<payload type> <feedback type> [<feedback parameter>]
    RtcpFb(String, String, Option<String>),
    /// Header extension, corresponds to a=extmap:<id>[/<direction>] <URI> [<params>]
    ExtMap(u8, Option<String>, String, Option<String>),
    /// Media stream identification, corresponds to a=msid:<stream id> [<track id>]
    Msid(String, Option<String>),
    /// Bandwidth information, corresponds to b=<bwtype>:<bandwidth>
    Bandwidth(String, u64),
    /// Restriction identifier, corresponds to a=rid:<id> <direction> [pt=<formats>] [;<key=value>*]
    Rid(RidAttribute),
    /// Simulcast attribute, corresponds to a=simulcast:<send list> <recv list>
    Simulcast(Vec<String>, Vec<String>),
    /// ICE options, corresponds to a=ice-options:<option-tag> [<option-tag>]*
    IceOptions(Vec<String>),
    /// End of candidates, corresponds to a=end-of-candidates
    EndOfCandidates,
    /// SCTP port, corresponds to a=sctp-port:<port>
    SctpPort(u16),
    /// Max message size, corresponds to a=max-message-size:<size>
    MaxMessageSize(u64),
    /// SCTP map, corresponds to a=sctpmap:<number> <app> <max-num-of-streams>
    SctpMap(u16, String, u16),
    /// Flag attribute, corresponds to a=<flag>
    Flag(String),
    /// Value attribute, corresponds to a=<n>:<value>
    Value(String, String),
    /// Other attribute, corresponds to a=<n>[:<value>]
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
    /// Connection address (IP address or FQDN)
    pub connection_address: String,
    /// Time-to-live for multicast (IPv4 only)
    pub ttl: Option<u8>,
    /// Number of addresses in multicast group
    pub multicast_count: Option<u32>,
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
    /// Associated repeat times (r= lines)
    pub repeat_times: Vec<RepeatTime>,
}

/// Represents a Repeat Time (r=) field in an SDP message.
///
/// Format: `r=<repeat-interval> <active-duration> <list-of-offsets-from-start-time>`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatTime {
    /// Repeat interval in seconds
    pub repeat_interval: u64,
    /// Active duration in seconds
    pub active_duration: u64,
    /// Offsets from start time in seconds
    pub offsets: Vec<u64>,
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
    /// Session information/description (i=)
    pub session_info: Option<String>,
    /// Session URI (u=)
    pub uri: Option<String>,
    /// Contact email address (e=)
    pub email: Option<String>,
    /// Contact phone number (p=)
    pub phone: Option<String>,
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
            session_info: None,
            uri: None,
            email: None,
            phone: None,
            connection_info: None,
            time_descriptions: Vec::new(),
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
    ///     ttl: None,                // Optional TTL for multicast (IPv4 only)
    ///     multicast_count: None,    // Optional number of addresses in multicast group
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

    /// Get the media direction (if set)
    pub fn get_direction(&self) -> Option<MediaDirection> {
        self.direction.clone()
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
    ///     ttl: None,                // Optional TTL for multicast (IPv4 only)
    ///     multicast_count: None,    // Optional number of addresses in multicast group
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
            ParsedAttribute::Ptime(v) => { self.ptime = Some(v as u32); }
            ParsedAttribute::Direction(d) => { self.direction = Some(d); }
            _ => self.generic_attributes.push(attr),
        }
        self
    }

    /// Get the media direction (if set)
    pub fn get_direction(&self) -> Option<MediaDirection> {
        self.direction.clone()
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
    type Err = Error;

    #[cfg(feature = "sdp")]
    fn from_str(s: &str) -> Result<Self> {
        use bytes::Bytes;
        // Use the actual parser when the sdp feature is enabled
        parse_sdp(&Bytes::copy_from_slice(s.as_bytes()))
    }
    
    #[cfg(not(feature = "sdp"))]
    fn from_str(s: &str) -> Result<Self> {
        // Create an empty session when the feature is not enabled
        Err(Error::SdpError("SDP parsing requires the 'sdp' feature".to_string()))
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
            ParsedAttribute::Fmtp(fmtp) => {
                write!(f, "a=fmtp:{} {}", fmtp.format, fmtp.parameters)
            }
            ParsedAttribute::Direction(dir) => {
                match dir {
                    MediaDirection::SendRecv => write!(f, "sendrecv"),
                    MediaDirection::SendOnly => write!(f, "sendonly"),
                    MediaDirection::RecvOnly => write!(f, "recvonly"),
                    MediaDirection::Inactive => write!(f, "inactive"),
                }
            }
            ParsedAttribute::Ptime(ptime) => write!(f, "a=ptime:{}", ptime),
            ParsedAttribute::MaxPtime(maxptime) => write!(f, "a=maxptime:{}", maxptime),
            ParsedAttribute::Candidate(candidate) => {
                write!(
                    f,
                    "a=candidate:{} {} {} {} {} {} typ {}",
                    candidate.foundation,
                    candidate.component_id,
                    candidate.transport,
                    candidate.priority,
                    candidate.connection_address,
                    candidate.port,
                    candidate.candidate_type
                )?;
                if let Some(rel_addr) = &candidate.related_address {
                    write!(f, " raddr {}", rel_addr)?;
                }
                if let Some(rel_port) = candidate.related_port {
                    write!(f, " rport {}", rel_port)?;
                }
                // Handle extensions as separate key-value pairs
                for (key, value) in &candidate.extensions {
                    if let Some(v) = value {
                        write!(f, " {} {}", key, v)?;
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
            ParsedAttribute::Fingerprint(hash, fingerprint) => {
                write!(f, "a=fingerprint:{} {}", hash, fingerprint)
            }
            ParsedAttribute::Setup(role) => write!(f, "a=setup:{}", role),
            ParsedAttribute::Mid(id) => write!(f, "a=mid:{}", id),
            ParsedAttribute::Group(semantics, ids) => {
                write!(f, "a=group:{}", semantics)?;
                for id in ids {
                    write!(f, " {}", id)?;
                }
                Ok(())
            }
            ParsedAttribute::RtcpMux => write!(f, "a=rtcp-mux"),
            ParsedAttribute::RtcpFb(pt, feedback_type, param) => {
                write!(f, "a=rtcp-fb:{} {}", pt, feedback_type)?;
                if let Some(p) = param {
                    write!(f, " {}", p)?;
                }
                Ok(())
            }
            ParsedAttribute::ExtMap(id, direction, uri, params) => {
                write!(f, "a=extmap:{}", id)?;
                if let Some(dir_str) = direction {
                    write!(f, "/{}", dir_str)?;
                }
                write!(f, " {}", uri)?;
                if let Some(p) = params {
                    write!(f, " {}", p)?;
                }
                Ok(())
            }
            ParsedAttribute::Msid(stream_id, track_id) => {
                write!(f, "a=msid:{}", stream_id)?;
                if let Some(id) = track_id {
                    write!(f, " {}", id)?;
                }
                Ok(())
            }
            ParsedAttribute::Rid(rid) => {
                write!(f, "a=rid:{} {}", rid.id, match rid.direction {
                    RidDirection::Send => "send",
                    RidDirection::Recv => "recv",
                })?;
                
                // Format payload types
                if !rid.formats.is_empty() {
                    write!(f, " pt={}", rid.formats.join(","))?;
                }
                
                // Format restrictions
                if !rid.restrictions.is_empty() {
                    for (key, value) in &rid.restrictions {
                        write!(f, ";{}={}", key, value)?;
                    }
                }
                
                Ok(())
            }
            ParsedAttribute::Simulcast(send, recv) => {
                write!(f, "a=simulcast:")?;
                let mut first = true;

                if !send.is_empty() {
                    write!(f, "send {}", send.join(";"))?;
                    first = false;
                }

                if !recv.is_empty() {
                    if !first {
                        write!(f, " ")?;
                    }
                    write!(f, "recv {}", recv.join(";"))?;
                }

                Ok(())
            }
            ParsedAttribute::IceOptions(options) => {
                write!(f, "a=ice-options:{}", options.join(" "))
            }
            ParsedAttribute::EndOfCandidates => write!(f, "a=end-of-candidates"),
            ParsedAttribute::SctpPort(port) => write!(f, "a=sctp-port:{}", port),
            ParsedAttribute::MaxMessageSize(size) => write!(f, "a=max-message-size:{}", size),
            ParsedAttribute::SctpMap(number, app, streams) => {
                write!(f, "a=sctpmap:{} {} {}", number, app, streams)
            }
            ParsedAttribute::Bandwidth(bwtype, bandwidth) => {
                write!(f, "b={}:{}", bwtype, bandwidth)
            }
            ParsedAttribute::Flag(name) => write!(f, "a={}", name),
            ParsedAttribute::Value(name, value) => write!(f, "a={}:{}", name, value),
            ParsedAttribute::Other(name, value) => {
                if let Some(val) = value {
                    write!(f, "a={}:{}", name, val)
                } else {
                    write!(f, "a={}", name)
                }
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
        let mut address = self.connection_address.clone();
        
        // Add TTL if present
        if let Some(ttl) = self.ttl {
            address.push_str(&format!("/{}", ttl));
            
            // Add multicast count if present
            if let Some(count) = self.multicast_count {
                address.push_str(&format!("/{}", count));
            }
        }
        
        write!(f, "{} {} {}", self.net_type, self.addr_type, address)
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
         if let Some(ref direction) = self.direction {
            match direction {
                MediaDirection::SendRecv => writeln!(f, "a=sendrecv")?,
                MediaDirection::SendOnly => writeln!(f, "a=sendonly")?,
                MediaDirection::RecvOnly => writeln!(f, "a=recvonly")?,
                MediaDirection::Inactive => writeln!(f, "a=inactive")?,
            }
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
         if let Some(ref direction) = self.direction {
             match direction {
                MediaDirection::SendRecv => writeln!(f, "a=sendrecv")?,
                MediaDirection::SendOnly => writeln!(f, "a=sendonly")?,
                MediaDirection::RecvOnly => writeln!(f, "a=recvonly")?,
                MediaDirection::Inactive => writeln!(f, "a=inactive")?,
            }
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

// Add Display implementations for enums when the sdp feature is not enabled
#[cfg(not(feature = "sdp"))]
impl std::fmt::Display for MediaDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaDirection::SendRecv => write!(f, "sendrecv"),
            MediaDirection::SendOnly => write!(f, "sendonly"),
            MediaDirection::RecvOnly => write!(f, "recvonly"),
            MediaDirection::Inactive => write!(f, "inactive"),
        }
    }
}

#[cfg(not(feature = "sdp"))]
impl std::fmt::Display for RidDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RidDirection::Send => write!(f, "send"),
            RidDirection::Recv => write!(f, "recv"),
        }
    }
} 