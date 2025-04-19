// Tests for SDP types (SdpSession, MediaDescription, etc.)

use crate::common::{assert_display_parses_back}; // Use helper
use rvoip_sip_core::types::sdp::{SdpSession, MediaDescription, ParsedAttribute, RtpMapAttribute, MediaDirection, Origin, ConnectionData, TimeDescription};
use std::str::FromStr;
use std::collections::HashMap;

#[test]
fn test_sdp_session_display_parse_roundtrip() {
    // Construct a sample SdpSession using the new structs
    let session = SdpSession {
        version: "0".to_string(),
        origin: Origin {
            username: "user".to_string(), sess_id: "123".to_string(), sess_version: "456".to_string(),
            net_type: "IN".to_string(), addr_type: "IP4".to_string(), unicast_address: "1.1.1.1".to_string()
        },
        session_name: "Test Session".to_string(),
        connection_info: Some(ConnectionData {
            net_type: "IN".to_string(), addr_type: "IP4".to_string(), connection_address: "192.168.4.1".to_string()
        }), 
        time_descriptions: vec![TimeDescription { start_time: "0".to_string(), stop_time: "0".to_string() }], 
        media_descriptions: vec![
             MediaDescription {
                media: "audio".to_string(), port: 5004, protocol: "RTP/AVP".to_string(), 
                formats: vec!["0".to_string(), "8".to_string()],
                connection_info: None,
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
                        encoding_params: Some("1".to_string()),
                    }),
                ],
            },
             MediaDescription {
                media: "video".to_string(), port: 5006, protocol: "RTP/AVP".to_string(), 
                formats: vec!["99".to_string()],
                connection_info: Some(ConnectionData {
                    net_type: "IN".to_string(), addr_type: "IP4".to_string(), connection_address: "192.168.4.2".to_string()
                }), 
                ptime: None,
                direction: Some(MediaDirection::SendOnly),
                generic_attributes: vec![
                     ParsedAttribute::RtpMap(RtpMapAttribute {
                        payload_type: 99,
                        encoding_name: "H264".to_string(),
                        clock_rate: 90000,
                        encoding_params: None,
                    }),
                    ParsedAttribute::Flag("framerate:25".to_string())
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
    
    // Manually check the string output for expected format using a multi-line string
    let sdp_string = session.to_string();
    let expected_sdp = "v=0\r\n"
                     + "o=user 123 456 IN IP4 1.1.1.1\r\n"
                     + "s=Test Session\r\n"
                     + "c=IN IP4 192.168.4.1\r\n"
                     + "t=0 0\r\n"
                     + "a=sendrecv\r\n"
                     + "a=tool:TestTool 1.0\r\n"
                     + "a=orient:portrait\r\n"
                     + "m=audio 5004 RTP/AVP 0 8\r\n"
                     + "a=ptime:20\r\n"
                     + "a=rtpmap:0 PCMU/8000\r\n"
                     + "a=rtpmap:8 PCMA/8000/1\r\n"
                     + "m=video 5006 RTP/AVP 99\r\n"
                     + "c=IN IP4 192.168.4.2\r\n"
                     + "a=sendonly\r\n"
                     + "a=rtpmap:99 H264/90000\r\n"
                     + "a=framerate:25\r\n";
      
    assert_eq!(sdp_string, expected_sdp);
} 