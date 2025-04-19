// Tests for SDP types (SdpSession, MediaDescription, etc.)

use crate::common::{assert_display_parses_back}; // Use helper
use rvoip_sip_core::types::sdp::{SdpSession, MediaDescription, ParsedAttribute, RtpMapAttribute, MediaDirection, Origin, ConnectionData, TimeDescription};
use std::str::FromStr;
use std::collections::HashMap;

#[test]
fn test_sdp_session_display_parse_roundtrip() {
    // Construct a sample SdpSession using the new structs
    let origin = Origin {
        username: "user".to_string(), sess_id: "123".to_string(), sess_version: "456".to_string(),
        net_type: "IN".to_string(), addr_type: "IP4".to_string(), unicast_address: "1.1.1.1".to_string()
    };
    let conn_sess = ConnectionData {
        net_type: "IN".to_string(), addr_type: "IP4".to_string(), connection_address: "192.168.4.1".to_string()
    };
    let time1 = TimeDescription { start_time: "0".to_string(), stop_time: "0".to_string() };
    let conn_media = ConnectionData {
        net_type: "IN".to_string(), addr_type: "IP4".to_string(), connection_address: "192.168.4.2".to_string()
    };
    
    let session = SdpSession {
        version: "0".to_string(),
        origin: origin.clone(), 
        session_name: "Test Session".to_string(),
        connection_info: Some(conn_sess.clone()), 
        time_descriptions: vec![time1.clone()], 
        media_descriptions: vec![
             MediaDescription {
                media: "audio".to_string(), port: 5004, protocol: "RTP/AVP".to_string(), 
                formats: vec!["0".to_string(), "8".to_string()],
                connection_info: None, // Uses session level
                ptime: Some(20), 
                direction: None, 
                generic_attributes: vec![
                    ParsedAttribute::RtpMap(RtpMapAttribute {
                        payload_type: 0,
                        encoding_name: "PCMU".to_string(),
                        clock_rate: 8000,
                        encoding_params: None,
                    }),
                    ParsedAttribute::RtpMap(RtpMapAttribute {
                        payload_type: 8,
                        encoding_name: "PCMA".to_string(),
                        clock_rate: 8000,
                        encoding_params: Some("1".to_string()), // Add encoding param example
                    }),
                ],
            },
             MediaDescription {
                media: "video".to_string(), port: 5006, protocol: "RTP/AVP".to_string(), 
                formats: vec!["99".to_string()],
                connection_info: Some(conn_media.clone()), // Media specific c=
                ptime: None,
                direction: Some(MediaDirection::SendOnly),
                generic_attributes: vec![
                     ParsedAttribute::RtpMap(RtpMapAttribute {
                        payload_type: 99,
                        encoding_name: "H264".to_string(),
                        clock_rate: 90000,
                        encoding_params: None,
                    }),
                    ParsedAttribute::Flag("framerate:25".to_string()) // Example flag-like value attribute
                ],
            }
        ],
        direction: Some(MediaDirection::SendRecv),
        generic_attributes: vec![
            ParsedAttribute::Value("tool".to_string(), "TestTool 1.0".to_string()), 
            ParsedAttribute::Flag("orient:portrait".to_string()),
        ],
    };
    
    // Test Display and FromStr round trip using the helper
    assert_display_parses_back(&session);
    
    // Manually check string output for specific lines
    let sdp_string = session.to_string();
    assert!(sdp_string.contains(&format!("o={}", origin)));
    assert!(sdp_string.contains(&format!("c={}", conn_sess)));
    assert!(sdp_string.contains(&format!("t={}", time1)));
    assert!(sdp_string.contains(&format!("c={}", conn_media))); // Check media connection line
} 