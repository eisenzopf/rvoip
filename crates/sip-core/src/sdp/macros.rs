//! Declarative macros for creating SDP sessions
//!
//! This module provides a concise, declarative macro-based syntax for creating
//! Session Description Protocol (SDP) messages as defined in RFC 8866.
//! 
//! # Overview
//!
//! The `sdp!` macro offers a domain-specific language for defining SDP sessions
//! with minimal boilerplate. This approach complements the builder pattern
//! found in the `builder` module, offering different trade-offs:
//!
//! - **Declarative syntax**: Define SDP sessions in a nested, structured format 
//!   that closely resembles the logical structure of an SDP message
//! - **Compile-time checks**: Benefit from Rust's macro system to catch some
//!   structural errors at compile time
//! - **Concise format**: Express complex SDP messages with minimal code
//!
//! # Usage
//!
//! The `sdp!` macro takes a structured set of parameters that match SDP fields
//! and attributes. Required fields include `origin` and `session_name`, with
//! various optional fields and media sections.
//!
//! The macro returns a `Result<SdpSession>`, performing validation before
//! returning the session.
//!
//! # Examples
//!
//! ## Simple audio SDP
//!
//! ```
//! use rvoip_sip_core::sdp;
//! use rvoip_sip_core::types::sdp::SdpSession;
//! use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription, MediaDescription};
//! use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute};
//! use rvoip_sip_core::sdp::attributes::MediaDirection;
//!
//! let result = sdp! {
//!     origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
//!     session_name: "Audio Call",
//!     connection: ("IN", "IP4", "192.168.1.100"),
//!     time: ("0", "0"),
//!     media: {
//!         type: "audio",
//!         port: 49170,
//!         protocol: "RTP/AVP",
//!         formats: ["0", "8"],
//!         rtpmap: ("0", "PCMU/8000"),
//!         rtpmap: ("8", "PCMA/8000"),
//!         direction: "sendrecv"
//!     }
//! };
//!
//! match result {
//!     Ok(session) => println!("Valid SDP: {}", session),
//!     Err(e) => eprintln!("Invalid SDP: {}", e),
//! }
//! ```
//!
//! ## WebRTC SDP offer with audio and video
//!
//! ```
//! use rvoip_sip_core::sdp;
//! use rvoip_sip_core::types::sdp::SdpSession;
//! use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription, MediaDescription};
//! use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute};
//! use rvoip_sip_core::sdp::attributes::MediaDirection;
//!
//! let result = sdp! {
//!     origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
//!     session_name: "WebRTC Session",
//!     connection: ("IN", "IP4", "192.168.1.100"),
//!     time: ("0", "0"),
//!     media: {
//!         type: "audio",
//!         port: 9, 
//!         protocol: "UDP/TLS/RTP/SAVPF",
//!         formats: ["111", "103"],
//!         rtpmap: ("111", "opus/48000/2"),
//!         rtpmap: ("103", "ISAC/16000"),
//!         fmtp: ("111", "minptime=10;useinbandfec=1"),
//!         direction: "sendrecv"
//!     },
//!     media: {
//!         type: "video",
//!         port: 9,
//!         protocol: "UDP/TLS/RTP/SAVPF",
//!         formats: ["96", "97"],
//!         rtpmap: ("96", "VP8/90000"),
//!         rtpmap: ("97", "H264/90000"),
//!         fmtp: ("97", "profile-level-id=42e01f;level-asymmetry-allowed=1"),
//!         direction: "sendrecv"
//!     }
//! };
//! ```
//!
//! # RFC compliance
//!
//! The `sdp!` macro generates SDP sessions that are validated against RFC 8866 requirements
//! before being returned. This ensures that the generated SDP messages are compliant with
//! the standard.
//!
//! # When to use macros vs. builder
//!
//! The SDK offers two approaches for creating SDP messages:
//!
//! - **Macro approach** (`sdp!`): Best for static SDP configurations known at compile time
//!   where you want concise, declarative syntax.
//!
//! - **Builder approach** (`SdpBuilder`): Better for dynamic SDP generation where values
//!   are determined at runtime, or when you need more complex programmatic control over
//!   the SDP construction process.
//!
//! Both approaches validate the resulting SDP message against RFC 8866 requirements.

use crate::types::sdp::{
    SdpSession, Origin, ConnectionData, TimeDescription, MediaDescription,
    ParsedAttribute, RtpMapAttribute, FmtpAttribute,
};
use crate::sdp::attributes::MediaDirection;
use crate::error::Result;

/// Creates a validated SDP session with a declarative syntax
///
/// The `sdp!` macro provides a structured, declarative syntax for creating SDP sessions
/// that closely mirrors the logical structure of SDP messages. It handles parsing and 
/// validation automatically, returning a `Result<SdpSession>`.
///
/// # Syntax
///
/// ```text
/// sdp! {
///     origin: (username, session_id, session_version, net_type, addr_type, unicast_address),
///     session_name: "name",
///     connection: (net_type, addr_type, connection_address),  // optional
///     time: (start_time, stop_time),  // required for RFC 8866 compliance
///     media: {  // optional, can have multiple
///         type: "media_type",
///         port: port_number,
///         protocol: "protocol",
///         formats: ["fmt1", "fmt2", ...],
///         rtpmap: ("payload_type", "encoding/clock_rate[/encoding_params]"),  // optional, can have multiple
///         fmtp: ("format", "parameters"),  // optional, can have multiple
///         direction: "direction"  // optional: "sendrecv", "sendonly", "recvonly", "inactive"
///     }
/// }
/// ```
///
/// # Parameters
///
/// ## Session-level fields (required)
///
/// - **origin**: The originator information, consisting of:
///   - **username**: Username of originator (can be "-" if not applicable)
///   - **session_id**: Unique session identifier
///   - **session_version**: Version of this session description
///   - **net_type**: Network type (typically "IN" for Internet)
///   - **addr_type**: Address type (typically "IP4" or "IP6")
///   - **unicast_address**: Unicast address of the originator
///
/// - **session_name**: Name of the session
///
/// ## Session-level fields (optional)
///
/// - **connection**: Connection data, consisting of:
///   - **net_type**: Network type (typically "IN" for Internet)
///   - **addr_type**: Address type (typically "IP4" or "IP6")
///   - **connection_address**: Connection address
///
/// - **time**: Time the session is active, consisting of:
///   - **start_time**: Start time (use "0" for sessions that are always active)
///   - **stop_time**: Stop time (use "0" for sessions that never end)
///
/// ## Media sections (optional, can have multiple)
///
/// - **type**: Media type (e.g., "audio", "video", "application")
/// - **port**: Port number for the media
/// - **protocol**: Transport protocol (e.g., "RTP/AVP", "UDP/TLS/RTP/SAVPF")
/// - **formats**: Array of format descriptions or payload types
/// - **rtpmap**: Maps payload types to codecs (can have multiple):
///   - **payload_type**: The payload type to map
///   - **encoding**: Encoding description in format "encoding/clock_rate[/encoding_params]"
/// - **fmtp**: Format parameters (can have multiple):
///   - **format**: The format to apply parameters to
///   - **parameters**: The parameters string
/// - **direction**: Media direction ("sendrecv", "sendonly", "recvonly", "inactive")
///
/// # Return Value
///
/// Returns a `Result<SdpSession>`:
/// - `Ok(SdpSession)` if the SDP is valid according to RFC 8866
/// - `Err(Error)` with a detailed error message if validation fails
///
/// # Examples
///
/// ## Basic SDP session with audio
///
/// ```
/// use rvoip_sip_core::sdp;
/// use rvoip_sip_core::types::sdp::SdpSession;
/// use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription, MediaDescription};
/// use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute};
/// use rvoip_sip_core::sdp::attributes::MediaDirection;
///
/// let session = sdp! {
///     origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
///     session_name: "Test Session",
///     connection: ("IN", "IP4", "192.168.1.100"),
///     time: ("0", "0"),
///     media: {
///         type: "audio",
///         port: 49170,
///         protocol: "RTP/AVP",
///         formats: ["0", "8"],
///         rtpmap: ("0", "PCMU/8000"),
///         rtpmap: ("8", "PCMA/8000"),
///         direction: "sendrecv"
///     }
/// }.expect("Valid SDP");
/// ```
///
/// ## SDP with multiple media sections
///
/// ```
/// use rvoip_sip_core::sdp;
/// use rvoip_sip_core::types::sdp::SdpSession;
/// use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription, MediaDescription};
/// use rvoip_sip_core::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute};
/// use rvoip_sip_core::sdp::attributes::MediaDirection;
///
/// let session = sdp! {
///     origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
///     session_name: "Audio/Video Session",
///     connection: ("IN", "IP4", "192.168.1.100"),
///     time: ("0", "0"),
///     media: {
///         type: "audio",
///         port: 49170,
///         protocol: "RTP/AVP",
///         formats: ["0"],
///         rtpmap: ("0", "PCMU/8000"),
///         direction: "sendrecv"
///     },
///     media: {
///         type: "video",
///         port: 51372,
///         protocol: "RTP/AVP",
///         formats: ["96"],
///         rtpmap: ("96", "H264/90000"),
///         fmtp: ("96", "profile-level-id=42e01f"),
///         direction: "sendrecv"
///     }
/// }.expect("Valid SDP");
/// ```
///
/// ## Minimal valid SDP
///
/// ```
/// use rvoip_sip_core::sdp;
/// use rvoip_sip_core::types::sdp::SdpSession;
/// use rvoip_sip_core::types::sdp::{Origin, ConnectionData, TimeDescription};
///
/// let session = sdp! {
///     origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
///     session_name: "Minimal Session",
///     connection: ("IN", "IP4", "192.168.1.100"),
///     time: ("0", "0")
/// }.expect("Valid minimal SDP");
/// ```
#[macro_export]
macro_rules! sdp {
    (
        origin: ($username:expr, $sess_id:expr, $sess_version:expr, $net_type:expr, $addr_type:expr, $unicast_address:expr),
        session_name: $session_name:expr
        $(, connection: ($conn_net_type:expr, $conn_addr_type:expr, $conn_address:expr))?
        $(, time: ($start_time:expr, $stop_time:expr))?
        $(, media: {
            type: $media_type:expr,
            port: $media_port:expr,
            protocol: $media_protocol:expr,
            formats: [$($format:expr),*]
            $(, rtpmap: ($rtpmap_pt:expr, $rtpmap_encoding:expr))*
            $(, fmtp: ($fmtp_pt:expr, $fmtp_params:expr))*
            $(, direction: $media_direction:expr)?
        })*
    ) => {{
        // Create the origin
        let origin = Origin {
            username: String::from($username),
            sess_id: String::from($sess_id),
            sess_version: String::from($sess_version),
            net_type: String::from($net_type),
            addr_type: String::from($addr_type),
            unicast_address: String::from($unicast_address),
        };
        
        // Create the session
        let mut session = SdpSession::new(origin, String::from($session_name));
        
        // Clear default time description (we'll add our own below)
        session.time_descriptions.clear();
        
        // Add connection info if provided
        $(
            let connection = ConnectionData {
                net_type: String::from($conn_net_type),
                addr_type: String::from($conn_addr_type),
                connection_address: String::from($conn_address),
                ttl: None,
                multicast_count: None,
            };
            session = session.with_connection_data(connection);
        )?
        
        // Add time description if provided
        $(
            let time = TimeDescription {
                start_time: String::from($start_time),
                stop_time: String::from($stop_time),
                repeat_times: vec![],
            };
            session.time_descriptions.push(time);
        )?
        
        // Add media descriptions if provided
        $(
            let mut formats_vec: Vec<String> = Vec::new();
            $(
                formats_vec.push(String::from($format));
            )*

            let mut media = MediaDescription::new(
                String::from($media_type),
                $media_port,
                String::from($media_protocol),
                formats_vec
            );
            
            // Add rtpmap attributes
            $(
                let rtpmap_parts: Vec<&str> = $rtpmap_encoding.split('/').collect();
                let encoding_name = rtpmap_parts[0].to_string();
                let clock_rate = rtpmap_parts[1].parse::<u32>().unwrap_or(8000);
                let encoding_params = if rtpmap_parts.len() > 2 {
                    Some(rtpmap_parts[2].to_string())
                } else {
                    None
                };
                
                let payload_type = $rtpmap_pt.parse::<u8>().unwrap_or(0);
                let rtpmap = ParsedAttribute::RtpMap(RtpMapAttribute {
                    payload_type,
                    encoding_name,
                    clock_rate,
                    encoding_params,
                });
                media.generic_attributes.push(rtpmap);
            )*
            
            // Add fmtp attributes
            $(
                let fmtp = ParsedAttribute::Fmtp(FmtpAttribute {
                    format: String::from($fmtp_pt),
                    parameters: String::from($fmtp_params),
                });
                media.generic_attributes.push(fmtp);
            )*
            
            // Add direction if provided
            $(
                let direction = match $media_direction {
                    "sendrecv" => MediaDirection::SendRecv,
                    "sendonly" => MediaDirection::SendOnly,
                    "recvonly" => MediaDirection::RecvOnly,
                    "inactive" => MediaDirection::Inactive,
                    _ => MediaDirection::SendRecv,
                };
                media.direction = Some(direction);
                media.generic_attributes.push(ParsedAttribute::Direction(direction));
            )?
            
            session.add_media(media);
        )*
        
        // Validate the SDP session
        $crate::sdp::parser::validate_sdp(&session).map(|_| session)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_sdp_macro() {
        // Create a minimal SDP session with one audio media section
        let session: Result<SdpSession> = sdp! {
            origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
            session_name: "Test SDP Session",
            connection: ("IN", "IP4", "192.168.1.100"),
            time: ("0", "0"),
            media: {
                type: "audio",
                port: 49170,
                protocol: "RTP/AVP",
                formats: ["0", "8"],
                rtpmap: ("0", "PCMU/8000"),
                rtpmap: ("8", "PCMA/8000"),
                direction: "sendrecv"
            }
        };
        
        // Verify the session is valid
        assert!(session.is_ok(), "SDP validation failed: {:?}", session.err());
        
        let session = session.unwrap();
        
        // Verify basic session properties
        assert_eq!(session.origin.username, "-");
        assert_eq!(session.origin.sess_id, "1234567890");
        assert_eq!(session.origin.sess_version, "2");
        assert_eq!(session.origin.unicast_address, "192.168.1.100");
        assert_eq!(session.session_name, "Test SDP Session");
        
        // Verify connection info
        assert!(session.connection_info.is_some());
        if let Some(conn) = &session.connection_info {
            assert_eq!(conn.connection_address, "192.168.1.100");
        }
        
        // Verify time description
        assert_eq!(session.time_descriptions.len(), 1);
        assert_eq!(session.time_descriptions[0].start_time, "0");
        assert_eq!(session.time_descriptions[0].stop_time, "0");
        
        // Verify media section
        assert_eq!(session.media_descriptions.len(), 1);
        let media = &session.media_descriptions[0];
        assert_eq!(media.media, "audio");
        assert_eq!(media.port, 49170);
        assert_eq!(media.protocol, "RTP/AVP");
        assert_eq!(media.formats, vec!["0", "8"]);
        assert_eq!(media.direction, Some(MediaDirection::SendRecv));
        
        // Verify rtpmap attributes
        let rtpmaps: Vec<_> = media.generic_attributes.iter()
            .filter_map(|attr| {
                if let ParsedAttribute::RtpMap(rtpmap) = attr {
                    Some(rtpmap)
                } else {
                    None
                }
            })
            .collect();
        
        assert_eq!(rtpmaps.len(), 2);
        assert_eq!(rtpmaps[0].payload_type, 0);
        assert_eq!(rtpmaps[0].encoding_name, "PCMU");
        assert_eq!(rtpmaps[0].clock_rate, 8000);
        assert_eq!(rtpmaps[1].payload_type, 8);
        assert_eq!(rtpmaps[1].encoding_name, "PCMA");
        assert_eq!(rtpmaps[1].clock_rate, 8000);
    }
    
    #[test]
    fn test_minimal_sdp_macro() {
        // Create an SDP with only the required fields
        let session: Result<SdpSession> = sdp! {
            origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
            session_name: "Minimal SDP Session"
        };
        
        // This should fail validation as it's missing required fields (time description)
        assert!(session.is_err(), "Minimal SDP without time should fail validation");
        
        // Create a minimal valid SDP
        let session = sdp! {
            origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
            session_name: "Minimal SDP Session",
            connection: ("IN", "IP4", "192.168.1.100"),
            time: ("0", "0")
        };
        
        // This should pass validation
        assert!(session.is_ok(), "Minimal valid SDP failed validation: {:?}", session.err());
        
        let session = session.unwrap();
        assert_eq!(session.origin.username, "-");
        assert_eq!(session.session_name, "Minimal SDP Session");
        assert!(session.connection_info.is_some());
        assert_eq!(session.time_descriptions.len(), 1);
        assert_eq!(session.media_descriptions.len(), 0);
    }
    
    #[test]
    fn test_multi_media_sdp_macro() {
        // Create an SDP with multiple media sections
        let session: Result<SdpSession> = sdp! {
            origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
            session_name: "Multi-Media SDP Session",
            connection: ("IN", "IP4", "192.168.1.100"),
            time: ("0", "0"),
            media: {
                type: "audio",
                port: 49170,
                protocol: "RTP/AVP",
                formats: ["0", "8"],
                rtpmap: ("0", "PCMU/8000"),
                rtpmap: ("8", "PCMA/8000"),
                direction: "sendrecv"
            },
            media: {
                type: "video",
                port: 51372,
                protocol: "RTP/AVP",
                formats: ["96"],
                rtpmap: ("96", "H264/90000"),
                fmtp: ("96", "profile-level-id=42e01f"),
                direction: "sendrecv"
            }
        };
        
        // Verify the session is valid
        assert!(session.is_ok(), "Multi-media SDP validation failed: {:?}", session.err());
        
        let session = session.unwrap();
        
        // Verify we have two media sections
        assert_eq!(session.media_descriptions.len(), 2);
        
        // Verify audio media
        let audio = &session.media_descriptions[0];
        assert_eq!(audio.media, "audio");
        assert_eq!(audio.port, 49170);
        assert_eq!(audio.formats, vec!["0", "8"]);
        
        // Verify video media
        let video = &session.media_descriptions[1];
        assert_eq!(video.media, "video");
        assert_eq!(video.port, 51372);
        assert_eq!(video.formats, vec!["96"]);
        
        // Verify fmtp in video
        let fmtp = video.generic_attributes.iter()
            .find_map(|attr| {
                if let ParsedAttribute::Fmtp(fmtp) = attr {
                    Some(fmtp)
                } else {
                    None
                }
            });
        
        assert!(fmtp.is_some());
        let fmtp = fmtp.unwrap();
        assert_eq!(fmtp.format, "96");
        assert_eq!(fmtp.parameters, "profile-level-id=42e01f");
    }
} 