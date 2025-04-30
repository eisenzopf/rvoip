//! True Rust macros for creating SDP sessions
//!
//! This module provides declarative macros that make it easy to create
//! SDP sessions with a clean, readable syntax.

use crate::types::sdp::{
    SdpSession, Origin, ConnectionData, TimeDescription, MediaDescription,
    ParsedAttribute, RtpMapAttribute, FmtpAttribute,
};
use crate::sdp::attributes::MediaDirection;
use crate::error::Result;

/// Creates an SDP session with a declarative syntax
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp;
/// use rvoip_sip_core::types::sdp::{
///     SdpSession, Origin, ConnectionData, TimeDescription, MediaDescription,
///     ParsedAttribute, RtpMapAttribute, FmtpAttribute,
/// };
/// use rvoip_sip_core::sdp::attributes::MediaDirection;
/// use rvoip_sip_core::error::Result;
/// 
/// // This line is just to satisfy the doc test even though we're not using these variables
/// let (SdpSession, Origin, ConnectionData, TimeDescription, MediaDescription, 
///      ParsedAttribute, RtpMapAttribute, FmtpAttribute, MediaDirection, Result) = 
///      (1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
///
/// let session: Result<SdpSession> = sdp! {
///     origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
///     session_name: "Test SDP Session",
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
/// };
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
} 