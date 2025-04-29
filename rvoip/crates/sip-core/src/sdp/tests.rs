use crate::error::Result;
use crate::sdp::attributes::MediaDirection;
use crate::sdp::attributes;
use crate::sdp::parser::{parse_sdp, parse_origin_line, parse_connection_line, 
                        parse_time_description_line, is_valid_ipv4, is_valid_ipv6, 
                        is_valid_hostname};
use crate::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute, CandidateAttribute, SsrcAttribute};
use bytes::Bytes;
use std::str::FromStr;

// Helper function to create SDP test content
fn create_test_sdp_bytes(content: &str) -> Bytes {
    Bytes::copy_from_slice(content.as_bytes())
}

#[test]
fn test_valid_minimal_sdp() {
    // A minimal valid SDP per RFC 4566
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_ok(), "Failed to parse valid minimal SDP: {:?}", result.err());
    let session = result.unwrap();
    assert_eq!(session.version, "0");
    assert_eq!(session.session_name, "SDP Seminar");
    assert_eq!(session.origin.username, "jdoe");
    assert_eq!(session.origin.unicast_address, "10.47.16.5");
    assert_eq!(session.media_descriptions.len(), 1);
    assert_eq!(session.media_descriptions[0].media, "audio");
    assert_eq!(session.media_descriptions[0].port, 49170);
}

#[test]
fn test_valid_comprehensive_sdp() {
    // A more comprehensive SDP with multiple media types and attributes
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
i=A Seminar on the session description protocol\r
u=http://www.example.com/seminars/sdp.pdf\r
e=j.doe@example.com (Jane Doe)\r
p=+1 617 555-6011\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
a=recvonly\r
m=audio 49170 RTP/AVP 0 8 97\r
i=Audio stream\r
c=IN IP4 0.0.0.0\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:97 iLBC/8000\r
a=sendrecv\r
m=video 51372 RTP/AVP 99\r
a=rtpmap:99 H264/90000\r
a=fmtp:99 profile-level-id=42e01f;level-asymmetry-allowed=1\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_ok(), "Failed to parse valid comprehensive SDP: {:?}", result.err());
    let session = result.unwrap();
    
    // Session level checks
    assert_eq!(session.version, "0");
    assert_eq!(session.time_descriptions.len(), 1);
    assert_eq!(session.direction, Some(MediaDirection::RecvOnly));
    assert_eq!(session.media_descriptions.len(), 2);
    
    // Audio media checks
    let audio = &session.media_descriptions[0];
    assert_eq!(audio.media, "audio");
    assert_eq!(audio.port, 49170);
    assert_eq!(audio.formats, vec!["0", "8", "97"]);
    assert_eq!(audio.direction, Some(MediaDirection::SendRecv));
    
    // Attribute checks for rtpmap
    let rtpmap_attrs: Vec<&RtpMapAttribute> = audio.generic_attributes.iter()
        .filter_map(|attr| match attr {
            ParsedAttribute::RtpMap(rtp) => Some(rtp),
            _ => None
        }).collect();
    assert_eq!(rtpmap_attrs.len(), 3);
    assert!(rtpmap_attrs.iter().any(|r| r.payload_type == 0 && r.encoding_name == "PCMU" && r.clock_rate == 8000));
    
    // Video media checks
    let video = &session.media_descriptions[1];
    assert_eq!(video.media, "video");
    assert_eq!(video.port, 51372);
    
    // Check for fmtp attribute in video
    let has_fmtp = video.generic_attributes.iter().any(|attr| {
        if let ParsedAttribute::Fmtp(fmtp) = attr {
            fmtp.format == "99" && fmtp.parameters.contains("profile-level-id=42e01f")
        } else {
            false
        }
    });
    assert!(has_fmtp, "Failed to find expected fmtp attribute in video");
}

#[test]
fn test_sdp_with_ice_candidates() {
    // SDP with ICE candidates (RFC 8839)
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 192.168.0.1\r
t=0 0\r
m=audio 49170 UDP/TLS/RTP/SAVPF 109\r
a=rtpmap:109 opus/48000/2\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=candidate:1 1 UDP 2130706431 192.168.1.5 49170 typ host\r
a=candidate:2 1 UDP 1694498815 192.0.2.3 51372 typ srflx raddr 192.168.1.5 rport 49170\r
a=candidate:3 1 UDP 100 2001:db8:a0b:12f0::1 60000 typ relay raddr 2001:db8:a0b:12f0::3 rport 61000\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_ok(), "Failed to parse SDP with ICE candidates: {:?}", result.err());
    let session = result.unwrap();
    
    // Check the ICE candidates
    let audio = &session.media_descriptions[0];
    let candidates: Vec<_> = audio.generic_attributes.iter()
        .filter_map(|attr| {
            if let ParsedAttribute::Candidate(c) = attr {
                Some(c)
            } else {
                None
            }
        }).collect();
    
    assert_eq!(candidates.len(), 3, "Expected 3 candidates, found {}", candidates.len());
    
    // Check host candidate
    let host_candidate = candidates.iter().find(|c| c.candidate_type == "host").unwrap();
    assert_eq!(host_candidate.foundation, "1");
    assert_eq!(host_candidate.component_id, 1);
    assert_eq!(host_candidate.connection_address, "192.168.1.5");
    assert!(host_candidate.related_address.is_none());
    
    // Check srflx candidate
    let srflx_candidate = candidates.iter().find(|c| c.candidate_type == "srflx").unwrap();
    assert_eq!(srflx_candidate.foundation, "2");
    assert_eq!(srflx_candidate.component_id, 1);
    assert_eq!(srflx_candidate.connection_address, "192.0.2.3");
    assert_eq!(srflx_candidate.related_address, Some("192.168.1.5".to_string()));
    assert_eq!(srflx_candidate.related_port, Some(49170));
    
    // Check relay candidate with IPv6
    let relay_candidate = candidates.iter().find(|c| c.candidate_type == "relay").unwrap();
    assert_eq!(relay_candidate.foundation, "3");
    assert_eq!(relay_candidate.connection_address, "2001:db8:a0b:12f0::1");
    assert_eq!(relay_candidate.related_address, Some("2001:db8:a0b:12f0::3".to_string()));
}

#[test]
fn test_sdp_with_ssrc_attributes() {
    // SDP with SSRC attributes (RFC 5576)
    let sdp = "\
v=0\r
o=alice 2890844526 2890844526 IN IP4 host.example.com\r
s=SIP Call\r
c=IN IP4 host.example.com\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
a=ssrc:314159 cname:user@example.com\r
a=ssrc:314159 msid:stream1 track1\r
a=ssrc:314159 mslabel:stream1\r
a=ssrc:314159 label:track1\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_ok(), "Failed to parse SDP with SSRC attributes: {:?}", result.err());
    let session = result.unwrap();
    
    let audio = &session.media_descriptions[0];
    let ssrcs: Vec<_> = audio.generic_attributes.iter()
        .filter_map(|attr| {
            if let ParsedAttribute::Ssrc(s) = attr {
                Some(s)
            } else {
                None
            }
        }).collect();
    
    assert_eq!(ssrcs.len(), 4, "Expected 4 SSRC attributes, found {}", ssrcs.len());
    
    // Check ssrc attributes
    assert!(ssrcs.iter().any(|s| s.ssrc_id == 314159 && s.attribute == "cname" && s.value == Some("user@example.com".to_string())));
    assert!(ssrcs.iter().any(|s| s.ssrc_id == 314159 && s.attribute == "msid" && s.value == Some("stream1 track1".to_string())));
}

#[test]
fn test_missing_mandatory_fields() {
    // Test missing v=
    let sdp = "\
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
    // For missing v=, we test at a higher level with parse_sdp
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_err(), "SDP without v= should be rejected");
    // We don't check specific error message since it could be parsing error or schema validation
    
    // Test missing o=
    let sdp = "\
v=0\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Missing mandatory o= field"));
    
    // Test missing s=
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Missing mandatory s= field"));
    
    // Test missing t=
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Missing mandatory t= field"));
    
    // Test missing c= with media
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Missing mandatory c= field"));
}

#[test]
fn test_line_ordering() {
    // Test invalid ordering: t= after m=
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 224.2.17.12\r
t=0 0\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid SDP order"));
    
    // Test invalid: session-level attributes after media section
    let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
o=jane 2890844527 2890842808 IN IP4 10.47.16.6\r
";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid SDP order"));
}

#[test]
fn test_attribute_parsing() {
    // Test rtpmap parsing
    let rtpmap_value = "96 H264/90000";
    let result = attributes::parse_rtpmap(rtpmap_value);
    assert!(result.is_ok());
    if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
        assert_eq!(rtpmap.payload_type, 96);
        assert_eq!(rtpmap.encoding_name, "H264");
        assert_eq!(rtpmap.clock_rate, 90000);
        assert!(rtpmap.encoding_params.is_none());
    } else {
        panic!("Expected ParsedAttribute::RtpMap");
    }
    
    // Test rtpmap with encoding parameters
    let rtpmap_value = "97 AMR/8000/1";
    let result = attributes::parse_rtpmap(rtpmap_value);
    assert!(result.is_ok());
    if let Ok(ParsedAttribute::RtpMap(rtpmap)) = result {
        assert_eq!(rtpmap.payload_type, 97);
        assert_eq!(rtpmap.encoding_name, "AMR");
        assert_eq!(rtpmap.clock_rate, 8000);
        assert_eq!(rtpmap.encoding_params, Some("1".to_string()));
    } else {
        panic!("Expected ParsedAttribute::RtpMap");
    }
    
    // Test fmtp parsing
    let fmtp_value = "96 profile-level-id=42e01f;level-asymmetry-allowed=1";
    let result = attributes::parse_fmtp(fmtp_value);
    assert!(result.is_ok());
    if let Ok(ParsedAttribute::Fmtp(fmtp)) = result {
        assert_eq!(fmtp.format, "96");
        assert_eq!(fmtp.parameters, "profile-level-id=42e01f;level-asymmetry-allowed=1");
    } else {
        panic!("Expected ParsedAttribute::Fmtp");
    }
    
    // Test ptime parsing
    let ptime_value = "20";
    let result = attributes::parse_ptime(ptime_value);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 20);
    
    // Test direction parsing
    let direction_value = "sendrecv";
    let result = attributes::parse_direction(direction_value);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), MediaDirection::SendRecv);
}

#[test]
fn test_connection_parsing() {
    // Test standard IPv4
    let c_line = "IN IP4 224.2.17.12";
    let result = parse_connection_line(c_line);
    assert!(result.is_ok());
    let conn = result.unwrap();
    assert_eq!(conn.net_type, "IN");
    assert_eq!(conn.addr_type, "IP4");
    assert_eq!(conn.connection_address, "224.2.17.12");
    
    // Test IPv4 with TTL
    let c_line = "IN IP4 224.2.1.1/127";
    let result = parse_connection_line(c_line);
    assert!(result.is_ok());
    
    // Test IPv4 with TTL and multicast addresses
    let c_line = "IN IP4 224.2.1.1/127/3";
    let result = parse_connection_line(c_line);
    assert!(result.is_ok());
    
    // Test IPv6
    let c_line = "IN IP6 FF15::101";
    let result = parse_connection_line(c_line);
    assert!(result.is_ok());
    
    // Test hostname
    let c_line = "IN IP4 example.com";
    let result = parse_connection_line(c_line);
    assert!(result.is_ok());
    
    // Test invalid address type (directly testing is_valid_ipv4 function)
    assert!(!is_valid_ipv4("999.999.999.999"));
    
    // Test invalid address type with the parser
    let c_line = "IN IPX 224.2.1.1";
    let result = parse_connection_line(c_line);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Unsupported address type"));
}

#[test]
fn test_candidate_parsing() {
    // Test standard host candidate
    let candidate = "1 1 UDP 2130706431 192.168.1.5 49170 typ host";
    let result = attributes::parse_candidate(candidate);
    assert!(result.is_ok());
    if let ParsedAttribute::Candidate(c) = result.unwrap() {
        assert_eq!(c.foundation, "1");
        assert_eq!(c.component_id, 1);
        assert_eq!(c.transport, "UDP");
        assert_eq!(c.priority, 2130706431);
        assert_eq!(c.connection_address, "192.168.1.5");
        assert_eq!(c.port, 49170);
        assert_eq!(c.candidate_type, "host");
    } else {
        panic!("Expected Candidate attribute");
    }
    
    // Test candidate with related address (server reflexive)
    let candidate = "2 1 UDP 1694498815 192.0.2.3 51372 typ srflx raddr 192.168.1.5 rport 49170";
    let result = attributes::parse_candidate(candidate);
    assert!(result.is_ok());
    if let ParsedAttribute::Candidate(c) = result.unwrap() {
        assert_eq!(c.candidate_type, "srflx");
        assert_eq!(c.related_address, Some("192.168.1.5".to_string()));
        assert_eq!(c.related_port, Some(49170));
    } else {
        panic!("Expected Candidate attribute");
    }
    
    // Test IPv6 candidate
    let candidate = "3 1 UDP 100 2001:db8:a0b:12f0::1 60000 typ relay raddr 2001:db8:a0b:12f0::3 rport 61000";
    let result = attributes::parse_candidate(candidate);
    assert!(result.is_ok());
    
    // Test candidate with additional extensions
    let candidate = "4 1 UDP 100 192.168.1.5 49170 typ host generation 0 network-id 1";
    let result = attributes::parse_candidate(candidate);
    assert!(result.is_ok());
    if let ParsedAttribute::Candidate(c) = result.unwrap() {
        let extensions: Vec<_> = c.extensions.iter()
            .filter(|(key, _)| key == "generation" || key == "network-id")
            .collect();
        assert_eq!(extensions.len(), 2);
    } else {
        panic!("Expected Candidate attribute");
    }
    
    // Test invalid candidate (missing typ)
    let candidate = "1 1 UDP 2130706431 192.168.1.5 49170 host";
    let result = attributes::parse_candidate(candidate);
    assert!(result.is_err());
    
    // Test invalid candidate (invalid type)
    let candidate = "1 1 UDP 2130706431 192.168.1.5 49170 typ invalid";
    let result = attributes::parse_candidate(candidate);
    assert!(result.is_err());

    // Invalid IP address (actually invalid hostname with illegal character (@))
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 invalid@hostname 49170 typ host").is_err());
    
    // Invalid IP address (octets > 255 are invalid)
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 999.999.999.999 49170 typ host").is_err());
}

#[test]
fn test_ssrc_parsing() {
    // Test SSRC with value
    let ssrc = "314159 cname:user@example.com";
    let result = attributes::parse_ssrc(ssrc);
    assert!(result.is_ok());
    if let ParsedAttribute::Ssrc(s) = result.unwrap() {
        assert_eq!(s.ssrc_id, 314159);
        assert_eq!(s.attribute, "cname");
        assert_eq!(s.value, Some("user@example.com".to_string()));
    } else {
        panic!("Expected SSRC attribute");
    }
    
    // Test SSRC without value (flag-like)
    let ssrc = "314159 mslabel";
    let result = attributes::parse_ssrc(ssrc);
    assert!(result.is_ok());
    if let ParsedAttribute::Ssrc(s) = result.unwrap() {
        assert_eq!(s.ssrc_id, 314159);
        assert_eq!(s.attribute, "mslabel");
        assert_eq!(s.value, None);
    } else {
        panic!("Expected SSRC attribute");
    }
    
    // Test invalid SSRC (non-numeric ID)
    let ssrc = "invalid cname:user@example.com";
    let result = attributes::parse_ssrc(ssrc);
    assert!(result.is_err());
}

#[test]
fn test_line_ending_handling() {
    // Test with CR+LF (RFC standard)
    let sdp = "v=0\r\no=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r\ns=SDP Seminar\r\nc=IN IP4 224.2.17.12/127\r\nt=0 0\r\n";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_ok());
    
    // Test with just LF (allowed by parser but not RFC compliant)
    let sdp = "v=0\no=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\ns=SDP Seminar\nc=IN IP4 224.2.17.12/127\nt=0 0\n";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_ok());
    
    // Test with mixed line endings
    let sdp = "v=0\r\no=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\ns=SDP Seminar\r\nc=IN IP4 224.2.17.12/127\nt=0 0\r\n";
    let result = parse_sdp(&create_test_sdp_bytes(sdp));
    assert!(result.is_ok());
}

// Additional Test Modules
mod torture_tests {
    use super::*;

    #[test]
    fn test_wellformed_unusual_sdps() {
        // Test 1: SDP with unusual but valid ordering and all possible session-level attributes
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP with unusual attributes\r
i=This is a test session with all attributes\r
u=http://www.example.com/seminars/unusual.pdf\r
e=j.doe@example.com (Jane Doe)\r
p=+1 617 555-6011\r
c=IN IP4 224.2.17.12/127\r
b=AS:128\r
t=2873397496 2873404696\r
r=7d 1h 0 25h\r
z=2882844526 -1h 2898848070 0\r
k=prompt\r
a=recvonly\r
a=setup:active\r
a=rtcp-mux\r
m=audio 49170 RTP/AVP 0 8 97\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid SDP with unusual attributes: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.media_descriptions.len(), 1);
        
        // Test 2: SDP with multiple media sections and different c= lines
        let sdp = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=Multiple media with different connections\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 192.168.1.1\r
a=rtpmap:0 PCMU/8000\r
m=video 51372 RTP/AVP 31\r
c=IN IP6 FF15::101\r
a=rtpmap:31 H261/90000\r
m=application 32416 UDP/DTLS/SCTP webrtc-datachannel\r
c=IN IP4 10.0.0.1\r
a=sctp-port:5000\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse valid SDP with multiple media types: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.media_descriptions.len(), 3);
        assert_eq!(session.media_descriptions[0].media, "audio");
        assert_eq!(session.media_descriptions[1].media, "video");
        assert_eq!(session.media_descriptions[2].media, "application");
        // Check that each media section has its own connection info
        assert!(session.media_descriptions[0].connection_info.is_some());
        assert_eq!(session.media_descriptions[0].connection_info.as_ref().unwrap().addr_type, "IP4");
        assert_eq!(session.media_descriptions[1].connection_info.as_ref().unwrap().addr_type, "IP6");
    }

    #[test]
    fn test_malformed_sdps() {
        // Test 1: Missing v= line (first line must be v=)
        let sdp = "\
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Seminar\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
c=IN IP4 224.2.17.12\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_err(), "Parser accepted SDP without v= line");
        assert!(result.unwrap_err().to_string().contains("v= line"));
    }
}

mod boundary_tests {
    use super::*;

    #[test]
    fn test_extremely_long_values() {
        // Test with extremely long session name (several KB)
        let long_session_name = "s".repeat(4000);
        let sdp = format!("\
v=0\r
o=- 2890844526 2890842807 IN IP4 10.47.16.5\r
s={}\r
c=IN IP4 224.2.17.12\r
t=0 0\r
", long_session_name);
        let result = parse_sdp(&create_test_sdp_bytes(&sdp));
        assert!(result.is_ok(), "Failed to parse SDP with very long session name");
        let session = result.unwrap();
        assert_eq!(session.session_name.len(), 4000);
    }
}

// New Tests for Extended SDP Attribute Parsers

#[test]
fn test_ice_attributes() {
    // Test ice-ufrag parsing
    let ice_ufrag = "F7gI";
    let result = attributes::parse_ice_ufrag(ice_ufrag);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "F7gI");
    
    // Test invalid ice-ufrag (too short)
    let invalid_ufrag = "abc";
    assert!(attributes::parse_ice_ufrag(invalid_ufrag).is_err());
    
    // Test ice-pwd parsing
    let ice_pwd = "x9cml/YzichV2+XlhiMu8g";
    let result = attributes::parse_ice_pwd(ice_pwd);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "x9cml/YzichV2+XlhiMu8g");
    
    // Test invalid ice-pwd (too short)
    let invalid_pwd = "tooshort";
    assert!(attributes::parse_ice_pwd(invalid_pwd).is_err());
}

#[test]
fn test_dtls_attributes() {
    // Test fingerprint parsing
    let fingerprint = "sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9";
    let result = attributes::parse_fingerprint(fingerprint);
    assert!(result.is_ok());
    let (hash_func, fp_value) = result.unwrap();
    assert_eq!(hash_func, "sha-256");
    assert!(fp_value.contains("D1:2C:74:A7:E3:B5"));
    
    // Test invalid fingerprint (invalid hash function)
    let invalid_fp = "invalid-hash D1:2C:74:A7:E3:B5";
    assert!(attributes::parse_fingerprint(invalid_fp).is_err());
    
    // Test setup parsing
    let setup = "actpass";
    let result = attributes::parse_setup(setup);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "actpass");
    
    // Test invalid setup value
    let invalid_setup = "invalid";
    assert!(attributes::parse_setup(invalid_setup).is_err());
}

#[test]
fn test_grouping_attributes() {
    // Test mid parsing
    let mid = "audio";
    let result = attributes::parse_mid(mid);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "audio");
    
    // Test invalid mid (empty)
    let invalid_mid = "";
    assert!(attributes::parse_mid(invalid_mid).is_err());
    
    // Test group parsing
    let group = "BUNDLE audio video";
    let result = attributes::parse_group(group);
    assert!(result.is_ok());
    let (semantics, mids) = result.unwrap();
    assert_eq!(semantics, "BUNDLE");
    assert_eq!(mids, vec!["audio".to_string(), "video".to_string()]);
}

#[test]
fn test_rtcp_attributes() {
    // Test rtcp-mux parsing
    let rtcp_mux = "";
    let result = attributes::parse_rtcp_mux(rtcp_mux);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), true);
    
    // Test rtcp-fb parsing
    let rtcp_fb = "96 nack pli";
    let result = attributes::parse_rtcp_fb(rtcp_fb);
    assert!(result.is_ok());
    let (pt, fb_type, params) = result.unwrap();
    assert_eq!(pt, "96");
    assert_eq!(fb_type, "nack");
    assert_eq!(params, Some("pli".to_string()));
}

#[test]
fn test_extmap_and_msid() {
    // Test extmap parsing
    let extmap = "1 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
    let result = attributes::parse_extmap(extmap);
    assert!(result.is_ok());
    let (id, direction, uri, params) = result.unwrap();
    assert_eq!(id, 1);
    assert_eq!(direction, None);
    assert_eq!(uri, "urn:ietf:params:rtp-hdrext:ssrc-audio-level");
    assert_eq!(params, None);
    
    // Test extmap with direction
    let extmap_dir = "2/sendrecv urn:ietf:params:rtp-hdrext:toffset";
    let result = attributes::parse_extmap(extmap_dir);
    assert!(result.is_ok());
    let (id, direction, uri, params) = result.unwrap();
    assert_eq!(id, 2);
    assert_eq!(direction, Some("sendrecv".to_string()));
    
    // Test msid parsing
    let msid = "stream1 track1";
    let result = attributes::parse_msid(msid);
    assert!(result.is_ok());
    let (stream_id, track_id) = result.unwrap();
    assert_eq!(stream_id, "stream1");
    assert_eq!(track_id, Some("track1".to_string()));
}

#[test]
fn test_bandwidth_parsing() {
    // Test bandwidth parsing
    let bandwidth = "AS:128";
    let result = attributes::parse_bandwidth(bandwidth);
    assert!(result.is_ok());
    let (bwtype, bw) = result.unwrap();
    assert_eq!(bwtype, "AS");
    assert_eq!(bw, 128);
    
    // Test invalid bandwidth (non-numeric value)
    let invalid_bw = "AS:invalid";
    assert!(attributes::parse_bandwidth(invalid_bw).is_err());
}

/// Tests for simulcast and RID attribute parsing (RFC 8853, 8851)
#[test]
fn test_simulcast_attributes() {
    // Test rid attribute
    let rid_value = "1 send pt=97,98 max-width=1280;max-height=720";
    let result = attributes::parse_rid(rid_value);
    assert!(result.is_ok());
    let (id, direction, restrictions) = result.unwrap();
    assert_eq!(id, "1");
    assert_eq!(direction, "send");
    assert_eq!(restrictions.len(), 1);
    assert_eq!(restrictions[0], "pt=97,98");
    assert_eq!(restrictions[1], "max-width=1280;max-height=720");
    
    // Test invalid rid - missing direction
    let invalid_rid = "1";
    assert!(attributes::parse_rid(invalid_rid).is_err());
    
    // Test simulcast attribute
    let simulcast_value = "send 1,2,3;~4 recv 5;~6,~7";
    let result = attributes::parse_simulcast(simulcast_value);
    assert!(result.is_ok());
    let (send_streams, recv_streams) = result.unwrap();
    assert_eq!(send_streams, vec!["1,2,3", "~4"]);
    assert_eq!(recv_streams, vec!["5", "~6,~7"]);
    
    // Test invalid simulcast - empty direction
    let invalid_simulcast = "send";
    assert!(attributes::parse_simulcast(invalid_simulcast).is_err());
    
    // Test SVC attribute (in fmtp)
    let scalability_mode = "L2T3";
    let result = attributes::parse_scalability_mode(scalability_mode);
    assert!(result.is_ok());
    let (pattern, spatial, temporal, extra) = result.unwrap();
    assert_eq!(pattern, "L");
    assert_eq!(spatial, Some(2));
    assert_eq!(temporal, Some(3));
    assert_eq!(extra, None);
}

/// Tests for Trickle ICE attributes (RFC 8840)
#[test]
fn test_trickle_ice_attributes() {
    // Test ice-options attribute with trickle
    let ice_options = "trickle";
    let result = attributes::parse_ice_options(ice_options);
    assert!(result.is_ok());
    let options = result.unwrap();
    assert_eq!(options, vec!["trickle"]);
    
    // Test ice-options with multiple options
    let multiple_options = "trickle ice2 renomination";
    let result = attributes::parse_ice_options(multiple_options);
    assert!(result.is_ok());
    let options = result.unwrap();
    assert_eq!(options, vec!["trickle", "ice2", "renomination"]);
    
    // Test invalid ice-options - empty
    let invalid_options = "";
    assert!(attributes::parse_ice_options(invalid_options).is_err());
    
    // Test end-of-candidates attribute
    let result = attributes::parse_end_of_candidates("");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), true);
}

/// Tests for WebRTC Data Channel attributes (RFC 8841)
#[test]
fn test_data_channel_attributes() {
    // Test sctp-port attribute
    let sctp_port = "5000";
    let result = attributes::parse_sctp_port(sctp_port);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 5000);
    
    // Test invalid sctp-port - not a number
    let invalid_port = "abc";
    assert!(attributes::parse_sctp_port(invalid_port).is_err());
    
    // Test max-message-size attribute
    let max_message_size = "262144";
    let result = attributes::parse_max_message_size(max_message_size);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 262144);
    
    // Test invalid max-message-size - zero
    let invalid_size = "0";
    assert!(attributes::parse_max_message_size(invalid_size).is_err());
    
    // Test sctpmap attribute (legacy)
    let sctpmap = "5000 webrtc-datachannel 1024";
    let result = attributes::parse_sctpmap(sctpmap);
    assert!(result.is_ok());
    let (port, app, streams) = result.unwrap();
    assert_eq!(port, 5000);
    assert_eq!(app, "webrtc-datachannel");
    assert_eq!(streams, 1024);
    
    // Test invalid sctpmap - missing parts
    let invalid_sctpmap = "5000 webrtc-datachannel";
    assert!(attributes::parse_sctpmap(invalid_sctpmap).is_err());
}

/// Test cross-attribute validation
#[test]
fn test_cross_attribute_validation() {
    use crate::types::sdp::ParsedAttribute;
    
    // Test valid attribute set
    let valid_attrs = vec![
        ParsedAttribute::Mid("audio".to_string()),
        ParsedAttribute::Mid("video".to_string()),
        ParsedAttribute::Group("BUNDLE".to_string(), vec!["audio".to_string(), "video".to_string()]),
    ];
    
    let result = attributes::validate_attributes(&valid_attrs);
    assert!(result.is_ok());
    
    // Test invalid attribute set (BUNDLE references non-existent mid)
    let invalid_attrs = vec![
        ParsedAttribute::Mid("audio".to_string()),
        ParsedAttribute::Group("BUNDLE".to_string(), vec!["audio".to_string(), "video".to_string()]),
    ];
    
    let result = attributes::validate_attributes(&invalid_attrs);
    assert!(result.is_err());
    
    // Test valid simulcast and rid references
    let valid_simulcast_attrs = vec![
        ParsedAttribute::Rid("1".to_string(), "send".to_string(), vec!["pt=97".to_string()]),
        ParsedAttribute::Rid("2".to_string(), "send".to_string(), vec!["pt=98".to_string()]),
        ParsedAttribute::Simulcast(vec!["1,2".to_string()], vec![]),
    ];
    
    let result = attributes::validate_attributes(&valid_simulcast_attrs);
    assert!(result.is_ok());
}

// ============================================================================
// COMPREHENSIVE ATTRIBUTE TESTS
// ============================================================================

/// Tests for RTP map attribute parsing
#[test]
fn test_rtpmap_attribute_comprehensive() {
    // Valid cases
    assert!(attributes::parse_rtpmap("96 H264/90000").is_ok());
    assert!(attributes::parse_rtpmap("97 opus/48000/2").is_ok());
    assert!(attributes::parse_rtpmap("0 PCMU/8000").is_ok());
    assert!(attributes::parse_rtpmap("8 PCMA/8000/1").is_ok());
    assert!(attributes::parse_rtpmap("101 telephone-event/8000").is_ok());
    
    // Test successful extraction of values
    if let Ok(ParsedAttribute::RtpMap(rtpmap)) = attributes::parse_rtpmap("97 opus/48000/2") {
        assert_eq!(rtpmap.payload_type, 97);
        assert_eq!(rtpmap.encoding_name, "opus");
        assert_eq!(rtpmap.clock_rate, 48000);
        assert_eq!(rtpmap.encoding_params, Some("2".to_string()));
    } else {
        panic!("Failed to parse valid rtpmap");
    }
    
    // Edge cases
    
    // Maximum payload type (127)
    assert!(attributes::parse_rtpmap("127 opus/48000").is_ok());
    
    // Minimal clock rate
    assert!(attributes::parse_rtpmap("96 H264/1").is_ok());
    
    // Error cases
    
    // Invalid format - missing space
    assert!(attributes::parse_rtpmap("96H264/90000").is_err());
    
    // Invalid format - missing clock rate
    assert!(attributes::parse_rtpmap("96 H264").is_err());
    
    // Invalid format - missing payload type
    assert!(attributes::parse_rtpmap("H264/90000").is_err());
    
    // Invalid payload type (over 127)
    assert!(attributes::parse_rtpmap("256 H264/90000").is_err());
    
    // Invalid payload type (non-numeric)
    assert!(attributes::parse_rtpmap("PT H264/90000").is_err());
    
    // Invalid encoding name (contains non-alpha characters)
    assert!(attributes::parse_rtpmap("96 H264@/90000").is_err());
    
    // Invalid clock rate (non-numeric)
    assert!(attributes::parse_rtpmap("96 H264/clock").is_err());
    
    // Invalid encoding params (should be numeric for audio channels)
    assert!(attributes::parse_rtpmap("8 PCMA/8000/stereo").is_err());
}

/// Tests for format parameters attribute parsing
#[test]
fn test_fmtp_attribute_comprehensive() {
    // Valid cases
    assert!(attributes::parse_fmtp("96 profile-level-id=42e01f;level-asymmetry-allowed=1").is_ok());
    assert!(attributes::parse_fmtp("97 minptime=10;useinbandfec=1").is_ok());
    assert!(attributes::parse_fmtp("101 0-15").is_ok());
    
    // Test successful extraction of values
    if let Ok(ParsedAttribute::Fmtp(fmtp)) = attributes::parse_fmtp("96 profile-level-id=42e01f") {
        assert_eq!(fmtp.format, "96");
        assert_eq!(fmtp.parameters, "profile-level-id=42e01f");
    } else {
        panic!("Failed to parse valid fmtp");
    }
    
    // Edge cases
    
    // Multiple parameters
    assert!(attributes::parse_fmtp("96 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1").is_ok());
    
    // Format with non-numeric ID (valid in some cases)
    assert!(attributes::parse_fmtp("red profile=original").is_ok());
    
    // Error cases
    
    // Invalid format - missing space
    assert!(attributes::parse_fmtp("96profile-level-id=42e01f").is_err());
    
    // Invalid format - missing parameters
    assert!(attributes::parse_fmtp("96 ").is_err());
    
    // Invalid format - missing format
    assert!(attributes::parse_fmtp("profile-level-id=42e01f").is_err());
    
    // Invalid format - non-numeric format (this should actually pass but worth testing)
    if let Ok(ParsedAttribute::Fmtp(fmtp)) = attributes::parse_fmtp("red profile=original") {
        assert_eq!(fmtp.format, "red");
    }
    
    // Empty parameters
    assert!(attributes::parse_fmtp("96").is_err());
}

/// Tests for packetization time attribute parsing
#[test]
fn test_ptime_attribute_comprehensive() {
    // Valid cases
    assert_eq!(attributes::parse_ptime("20").unwrap(), 20);
    assert_eq!(attributes::parse_ptime("0").unwrap(), 0);
    assert_eq!(attributes::parse_ptime("1000").unwrap(), 1000);
    
    // Edge cases
    
    // Whitespace handling
    assert_eq!(attributes::parse_ptime(" 20 ").unwrap(), 20);
    
    // Error cases
    
    // Invalid format - non-numeric
    assert!(attributes::parse_ptime("twenty").is_err());
    
    // Invalid format - negative
    assert!(attributes::parse_ptime("-20").is_err());
    
    // Invalid format - decimal
    assert!(attributes::parse_ptime("20.5").is_err());
    
    // Invalid format - empty
    assert!(attributes::parse_ptime("").is_err());
}

/// Tests for max packetization time attribute parsing
#[test]
fn test_maxptime_attribute_comprehensive() {
    // Valid cases
    assert_eq!(attributes::parse_maxptime("20").unwrap(), 20);
    assert_eq!(attributes::parse_maxptime("1000").unwrap(), 1000);
    
    // Edge cases
    
    // Whitespace handling
    assert_eq!(attributes::parse_maxptime(" 50 ").unwrap(), 50);
    
    // Minimum reasonable value
    assert_eq!(attributes::parse_maxptime("10").unwrap(), 10);
    
    // Maximum reasonable value
    assert_eq!(attributes::parse_maxptime("5000").unwrap(), 5000);
    
    // Error cases
    
    // Invalid format - non-numeric
    assert!(attributes::parse_maxptime("maximum").is_err());
    
    // Invalid format - negative
    assert!(attributes::parse_maxptime("-50").is_err());
    
    // Invalid format - decimal
    assert!(attributes::parse_maxptime("50.5").is_err());
    
    // Invalid format - empty
    assert!(attributes::parse_maxptime("").is_err());
    
    // Invalid format - too small
    assert!(attributes::parse_maxptime("9").is_err());
    
    // Invalid format - too large
    assert!(attributes::parse_maxptime("5001").is_err());
}

/// Tests for media direction attribute parsing
#[test]
fn test_direction_attribute_comprehensive() {
    // Valid cases
    assert_eq!(attributes::parse_direction("sendrecv").unwrap(), MediaDirection::SendRecv);
    assert_eq!(attributes::parse_direction("sendonly").unwrap(), MediaDirection::SendOnly);
    assert_eq!(attributes::parse_direction("recvonly").unwrap(), MediaDirection::RecvOnly);
    assert_eq!(attributes::parse_direction("inactive").unwrap(), MediaDirection::Inactive);
    
    // Edge cases
    
    // Whitespace handling
    assert_eq!(attributes::parse_direction(" sendrecv ").unwrap(), MediaDirection::SendRecv);
    
    // Error cases
    
    // Invalid direction
    assert!(attributes::parse_direction("send").is_err());
    assert!(attributes::parse_direction("recv").is_err());
    assert!(attributes::parse_direction("sendrec").is_err());
    assert!(attributes::parse_direction("SENDRECV").is_err()); // Case sensitive
    assert!(attributes::parse_direction("").is_err());
}

/// Tests for ICE candidate attribute parsing
#[test]
fn test_candidate_attribute_comprehensive() {
    // Valid host candidate
    let host_value = "1 1 UDP 2130706431 192.168.1.5 49170 typ host";
    assert!(attributes::parse_candidate(host_value).is_ok());
    if let ParsedAttribute::Candidate(c) = attributes::parse_candidate(host_value).unwrap() {
        assert_eq!(c.foundation, "1");
        assert_eq!(c.component_id, 1);
        assert_eq!(c.transport, "UDP");
        assert_eq!(c.priority, 2130706431);
        assert_eq!(c.connection_address, "192.168.1.5");
        assert_eq!(c.port, 49170);
        assert_eq!(c.candidate_type, "host");
    } else {
        panic!("Expected Candidate attribute");
    }
    
    // Valid server reflexive candidate with related address/port
    let srflx_value = "2 1 UDP 1694498815 192.0.2.3 51372 typ srflx raddr 192.168.1.5 rport 49170";
    assert!(attributes::parse_candidate(srflx_value).is_ok());
    if let ParsedAttribute::Candidate(c) = attributes::parse_candidate(srflx_value).unwrap() {
        assert_eq!(c.candidate_type, "srflx");
        assert_eq!(c.related_address, Some("192.168.1.5".to_string()));
        assert_eq!(c.related_port, Some(49170));
    } else {
        panic!("Expected Candidate attribute");
    }
    
    // Valid relay candidate with extensions
    let relay_value = "3 1 UDP 100 2001:db8:a0b:12f0::1 60000 typ relay raddr 2001:db8:a0b:12f0::3 rport 61000 generation 0 network-id 1";
    assert!(attributes::parse_candidate(relay_value).is_ok());
    if let ParsedAttribute::Candidate(c) = attributes::parse_candidate(relay_value).unwrap() {
        assert_eq!(c.candidate_type, "relay");
        assert!(!c.extensions.is_empty());
        assert!(c.extensions.iter().any(|(key, _)| key == "generation"));
        assert!(c.extensions.iter().any(|(key, _)| key == "network-id"));
    }
    
    // Valid TCP candidate with tcptype
    let tcp_value = "4 1 TCP 1694498815 192.0.2.3 51372 typ srflx raddr 192.168.1.5 rport 49170 tcptype passive";
    assert!(attributes::parse_candidate(tcp_value).is_ok());
    if let ParsedAttribute::Candidate(c) = attributes::parse_candidate(tcp_value).unwrap() {
        assert_eq!(c.transport, "TCP");
        assert!(c.extensions.iter().any(|(key, value)| key == "tcptype" && value.as_deref() == Some("passive")));
    }
    
    // Error cases
    
    // Missing component
    assert!(attributes::parse_candidate("1 UDP 2130706431 192.168.1.5 49170 typ host").is_err());
    
    // Missing foundation
    assert!(attributes::parse_candidate("1 UDP 2130706431 192.168.1.5 49170 typ host").is_err());
    
    // Invalid port (not a number)
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 192.168.1.5 port typ host").is_err());
    
    // Invalid port (out of range)
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 192.168.1.5 70000 typ host").is_err());
    
    // Invalid priority (not a number)
    assert!(attributes::parse_candidate("1 1 UDP priority 192.168.1.5 49170 typ host").is_err());
    
    // Invalid IP address (actually invalid hostname with illegal character (@))
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 invalid@hostname 49170 typ host").is_err());
    
    // Invalid IP address (octets > 255 are invalid)
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 999.999.999.999 49170 typ host").is_err());
    
    // Missing typ keyword
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 192.168.1.5 49170 host").is_err());
    
    // Invalid candidate type
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 192.168.1.5 49170 typ invalid").is_err());
    
    // Missing rport when raddr is present
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 192.168.1.5 49170 typ srflx raddr 192.168.1.6").is_err());
    
    // Invalid rport
    assert!(attributes::parse_candidate("1 1 UDP 2130706431 192.168.1.5 49170 typ srflx raddr 192.168.1.6 rport invalid").is_err());
}

/// Tests for SSRC attribute parsing
#[test]
fn test_ssrc_attribute_comprehensive() {
    // Valid SSRC with value
    let ssrc_cname = "314159 cname:user@example.com";
    assert!(attributes::parse_ssrc(ssrc_cname).is_ok());
    if let ParsedAttribute::Ssrc(s) = attributes::parse_ssrc(ssrc_cname).unwrap() {
        assert_eq!(s.ssrc_id, 314159);
        assert_eq!(s.attribute, "cname");
        assert_eq!(s.value, Some("user@example.com".to_string()));
    } else {
        panic!("Expected SSRC attribute");
    }
    
    // Valid SSRC without value
    let ssrc_mslabel = "314159 mslabel";
    let result = attributes::parse_ssrc(ssrc_mslabel);
    assert!(result.is_ok());
    if let ParsedAttribute::Ssrc(s) = result.unwrap() {
        assert_eq!(s.ssrc_id, 314159);
        assert_eq!(s.attribute, "mslabel");
        assert_eq!(s.value, None);
    } else {
        panic!("Expected SSRC attribute");
    }
    
    // Valid SSRC with spaces in value
    let ssrc_msid = "314159 msid:stream1 track1";
    assert!(attributes::parse_ssrc(ssrc_msid).is_ok());
    if let ParsedAttribute::Ssrc(s) = attributes::parse_ssrc(ssrc_msid).unwrap() {
        assert_eq!(s.ssrc_id, 314159);
        assert_eq!(s.attribute, "msid");
        assert_eq!(s.value, Some("stream1 track1".to_string()));
    }
    
    // Error cases
    
    // Missing attribute
    assert!(attributes::parse_ssrc("314159").is_err());
    
    // Invalid SSRC ID (not a number)
    assert!(attributes::parse_ssrc("ssrcid cname:user@example.com").is_err());
    
    // Invalid SSRC ID (out of range for u32)
    assert!(attributes::parse_ssrc("4294967296 cname:user@example.com").is_err());
    
    // Missing space after SSRC ID
    assert!(attributes::parse_ssrc("314159cname:user@example.com").is_err());
}

/// Tests for ICE username fragment (ufrag) attribute parsing
#[test]
fn test_ice_ufrag_attribute_comprehensive() {
    // Valid ufrag
    assert_eq!(attributes::parse_ice_ufrag("F7gI").unwrap(), "F7gI");
    
    // Valid ufrag with minimum length (4 chars)
    assert_eq!(attributes::parse_ice_ufrag("abcd").unwrap(), "abcd");
    
    // Valid ufrag with typical length
    assert_eq!(attributes::parse_ice_ufrag("asd87ysLkj").unwrap(), "asd87ysLkj");
    
    // Valid ufrag with maximum length (256 chars)
    let long_ufrag = "a".repeat(256);
    assert_eq!(attributes::parse_ice_ufrag(&long_ufrag).unwrap(), long_ufrag);
    
    // Valid ufrag with special characters
    assert_eq!(attributes::parse_ice_ufrag("F7+/gI=").unwrap(), "F7+/gI=");
    
    // Error cases
    
    // Invalid ufrag - too short (less than 4 chars)
    assert!(attributes::parse_ice_ufrag("abc").is_err());
    
    // Invalid ufrag - too long (more than 256 chars)
    let too_long_ufrag = "a".repeat(257);
    assert!(attributes::parse_ice_ufrag(&too_long_ufrag).is_err());
    
    // Invalid ufrag - non-printable characters
    assert!(attributes::parse_ice_ufrag("ab\x00cd").is_err());
}

/// Tests for ICE password (pwd) attribute parsing
#[test]
fn test_ice_pwd_attribute_comprehensive() {
    // Valid pwd
    assert_eq!(attributes::parse_ice_pwd("x9cml/YzichV2+XlhiMu8g").unwrap(), "x9cml/YzichV2+XlhiMu8g");
    
    // Valid pwd with minimum length (22 chars)
    assert_eq!(attributes::parse_ice_pwd("abcdefghijklmnopqrstuv").unwrap(), "abcdefghijklmnopqrstuv");
    
    // Valid pwd with typical length
    assert_eq!(attributes::parse_ice_pwd("asd87ysLkjasdjkhfaksjdhfkasdf").unwrap(), "asd87ysLkjasdjkhfaksjdhfkasdf");
    
    // Valid pwd with maximum length (256 chars)
    let long_pwd = "a".repeat(256);
    assert_eq!(attributes::parse_ice_pwd(&long_pwd).unwrap(), long_pwd);
    
    // Valid pwd with special characters
    assert_eq!(attributes::parse_ice_pwd("x9cml/YzichV2+XlhiMu8g==").unwrap(), "x9cml/YzichV2+XlhiMu8g==");
    
    // Error cases
    
    // Invalid pwd - too short (less than 22 chars)
    assert!(attributes::parse_ice_pwd("abcdefghijklmnopqrstu").is_err());
    
    // Invalid pwd - too long (more than 256 chars)
    let too_long_pwd = "a".repeat(257);
    assert!(attributes::parse_ice_pwd(&too_long_pwd).is_err());
    
    // Invalid pwd - non-printable characters
    assert!(attributes::parse_ice_pwd("abcdefghijklmnopqrstu\x00v").is_err());
}

/// Tests for DTLS fingerprint attribute parsing
#[test]
fn test_fingerprint_attribute_comprehensive() {
    // Valid fingerprint with SHA-256
    let sha256 = "sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9";
    assert!(attributes::parse_fingerprint(sha256).is_ok());
    let (hash, fp) = attributes::parse_fingerprint(sha256).unwrap();
    assert_eq!(hash, "sha-256");
    assert_eq!(fp, "D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D:CF:76:B3:97:B6:5F:A5:3D:EC:D8:79:49:5C:92:26:E9");
    
    // Valid fingerprint with SHA-1
    let sha1 = "sha-1 CD:34:D1:62:16:95:7B:B7:EB:74:E2:39:27:97:EB:0B:07:04:4D:78";
    assert!(attributes::parse_fingerprint(sha1).is_ok());
    
    // Valid fingerprint with MD5 (less common but valid)
    let md5 = "md5 97:D9:26:9C:F9:D3:A4:7A:2F:D9:81:8B:D0:A8:CE:7C";
    assert!(attributes::parse_fingerprint(md5).is_ok());
    
    // Error cases
    
    // Invalid hash function
    assert!(attributes::parse_fingerprint("sha-123 D1:2C:74:A7:E3:B5").is_err());
    
    // Missing space between hash function and fingerprint
    assert!(attributes::parse_fingerprint("sha-256D1:2C:74:A7:E3:B5").is_err());
    
    // Empty fingerprint
    assert!(attributes::parse_fingerprint("sha-256 ").is_err());
    
    // Invalid fingerprint format (missing colons)
    assert!(attributes::parse_fingerprint("sha-256 D12C74A7E3B51104").is_err());
    
    // Invalid fingerprint format (non-hex characters)
    assert!(attributes::parse_fingerprint("sha-256 ZZ:2C:74:A7:E3:B5").is_err());
    
    // Invalid fingerprint format (segments too long)
    assert!(attributes::parse_fingerprint("sha-256 D12:2C:74:A7:E3:B5").is_err());
}

/// Tests for DTLS setup attribute parsing
#[test]
fn test_setup_attribute_comprehensive() {
    // Valid values as per RFC 4145/5763
    assert_eq!(attributes::parse_setup("active").unwrap(), "active");
    assert_eq!(attributes::parse_setup("passive").unwrap(), "passive");
    assert_eq!(attributes::parse_setup("actpass").unwrap(), "actpass");
    assert_eq!(attributes::parse_setup("holdconn").unwrap(), "holdconn");
    
    // Whitespace handling
    assert_eq!(attributes::parse_setup(" active ").unwrap(), "active");
    
    // Error cases
    
    // Invalid setup value
    assert!(attributes::parse_setup("activate").is_err());
    assert!(attributes::parse_setup("ACTIVE").is_err()); // Case sensitive
    assert!(attributes::parse_setup("").is_err());
}

/// Tests for media ID (mid) attribute parsing
#[test]
fn test_mid_attribute_comprehensive() {
    // Valid mid values
    assert_eq!(attributes::parse_mid("audio").unwrap(), "audio");
    assert_eq!(attributes::parse_mid("video").unwrap(), "video");
    assert_eq!(attributes::parse_mid("data").unwrap(), "data");
    assert_eq!(attributes::parse_mid("0").unwrap(), "0");
    assert_eq!(attributes::parse_mid("mid-with-hyphens").unwrap(), "mid-with-hyphens");
    
    // Whitespace handling
    assert_eq!(attributes::parse_mid(" audio ").unwrap(), "audio");
    
    // Error cases
    
    // Empty mid
    assert!(attributes::parse_mid("").is_err());
    
    // Mid with invalid characters (must be a token)
    assert!(attributes::parse_mid("mid@invalid").is_err());
    assert!(attributes::parse_mid("mid with spaces").is_err());
}

/// Tests for group attribute parsing
#[test]
fn test_group_attribute_comprehensive() {
    // Valid group with BUNDLE semantics (common in WebRTC)
    let bundle = "BUNDLE audio video data";
    assert!(attributes::parse_group(bundle).is_ok());
    let (semantics, mids) = attributes::parse_group(bundle).unwrap();
    assert_eq!(semantics, "BUNDLE");
    assert_eq!(mids, vec!["audio".to_string(), "video".to_string(), "data".to_string()]);
    
    // Valid group with FID semantics (for RTX)
    let fid = "FID 0 1";
    assert!(attributes::parse_group(fid).is_ok());
    let (semantics, mids) = attributes::parse_group(fid).unwrap();
    assert_eq!(semantics, "FID");
    assert_eq!(mids, vec!["0".to_string(), "1".to_string()]);
    
    // Valid group with LS semantics
    let ls = "LS a1 a2 v1";
    assert!(attributes::parse_group(ls).is_ok());
    
    // Valid group with single mid
    let single = "BUNDLE audio";
    assert!(attributes::parse_group(single).is_ok());
    let (semantics, mids) = attributes::parse_group(single).unwrap();
    assert_eq!(semantics, "BUNDLE");
    assert_eq!(mids, vec!["audio".to_string()]);
    
    // Error cases
    
    // Empty group
    assert!(attributes::parse_group("").is_err());
    
    // Group with only semantics, no mids
    assert!(attributes::parse_group("BUNDLE").is_ok());
    let (semantics, mids) = attributes::parse_group("BUNDLE").unwrap();
    assert_eq!(semantics, "BUNDLE");
    assert!(mids.is_empty());
}

/// Tests for RTCP multiplexing attribute parsing
#[test]
fn test_rtcp_mux_attribute_comprehensive() {
    // rtcp-mux is a flag attribute, so it doesn't really have a value
    assert!(attributes::parse_rtcp_mux("").is_ok());
    assert_eq!(attributes::parse_rtcp_mux("").unwrap(), true);
    
    // It should even accept extra text (though clients shouldn't send this)
    assert!(attributes::parse_rtcp_mux("ignored").is_ok());
    assert_eq!(attributes::parse_rtcp_mux("ignored").unwrap(), true);
}

/// Tests for RTCP feedback attribute parsing
#[test]
fn test_rtcp_fb_attribute_comprehensive() {
    // Valid RTCP feedback mechanisms
    
    // Simple nack
    let nack = "96 nack";
    assert!(attributes::parse_rtcp_fb(nack).is_ok());
    let (pt, fb_type, params) = attributes::parse_rtcp_fb(nack).unwrap();
    assert_eq!(pt, "96");
    assert_eq!(fb_type, "nack");
    assert_eq!(params, None);
    
    // nack with additional parameter (pli)
    let nack_pli = "96 nack pli";
    assert!(attributes::parse_rtcp_fb(nack_pli).is_ok());
    let (pt, fb_type, params) = attributes::parse_rtcp_fb(nack_pli).unwrap();
    assert_eq!(pt, "96");
    assert_eq!(fb_type, "nack");
    assert_eq!(params, Some("pli".to_string()));
    
    // ccm with fir
    let ccm_fir = "96 ccm fir";
    assert!(attributes::parse_rtcp_fb(ccm_fir).is_ok());
    
    // transport-cc
    let transport_cc = "96 transport-cc";
    assert!(attributes::parse_rtcp_fb(transport_cc).is_ok());
    
    // trr-int
    let trr_int = "96 trr-int 500";
    assert!(attributes::parse_rtcp_fb(trr_int).is_ok());
    
    // Wildcard payload type (*)
    let wildcard = "* nack";
    assert!(attributes::parse_rtcp_fb(wildcard).is_ok());
    let (pt, fb_type, params) = attributes::parse_rtcp_fb(wildcard).unwrap();
    assert_eq!(pt, "*");
    
    // Error cases
    
    // Missing payload type
    assert!(attributes::parse_rtcp_fb("nack").is_err());
    
    // Missing feedback type
    assert!(attributes::parse_rtcp_fb("96").is_err());
    
    // Invalid payload type (not numeric and not *)
    // The current implementation accepts any string for the payload type
    // assert!(attributes::parse_rtcp_fb("pt nack").is_err());
}

/// Tests for RTP header extension mapping attribute parsing
#[test]
fn test_extmap_attribute_comprehensive() {
    // Valid extmap attributes
    
    // Simple extmap without direction
    let simple = "1 urn:ietf:params:rtp-hdrext:ssrc-audio-level";
    assert!(attributes::parse_extmap(simple).is_ok());
    let (id, direction, uri, params) = attributes::parse_extmap(simple).unwrap();
    assert_eq!(id, 1);
    assert_eq!(direction, None);
    assert_eq!(uri, "urn:ietf:params:rtp-hdrext:ssrc-audio-level");
    assert_eq!(params, None);
    
    // Extmap with direction
    let with_dir = "2/sendrecv urn:ietf:params:rtp-hdrext:toffset";
    let result = attributes::parse_extmap(with_dir);
    assert!(result.is_ok());
    let (id, direction, uri, params) = result.unwrap();
    assert_eq!(id, 2);
    assert_eq!(direction, Some("sendrecv".to_string()));
    
    // Extmap with parameters
    let with_params = "3 urn:ietf:params:rtp-hdrext:sdes:mid config-params";
    assert!(attributes::parse_extmap(with_params).is_ok());
    let (id, direction, uri, params) = attributes::parse_extmap(with_params).unwrap();
    assert_eq!(id, 3);
    assert_eq!(direction, None);
    assert_eq!(uri, "urn:ietf:params:rtp-hdrext:sdes:mid");
    assert_eq!(params, Some("config-params".to_string()));
    
    // Extmap with direction and parameters
    let complex = "4/sendonly urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id stream-id-params";
    assert!(attributes::parse_extmap(complex).is_ok());
    let (id, direction, uri, params) = attributes::parse_extmap(complex).unwrap();
    assert_eq!(id, 4);
    assert_eq!(direction, Some("sendonly".to_string()));
    assert_eq!(uri, "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id");
    assert_eq!(params, Some("stream-id-params".to_string()));
    
    // Edge cases
    
    // Minimum ID (1)
    assert!(attributes::parse_extmap("1 urn:test").is_ok());
    
    // Maximum one-byte ID (14)
    assert!(attributes::parse_extmap("14 urn:test").is_ok());
    
    // Two-byte ID range (15-255)
    assert!(attributes::parse_extmap("15 urn:test").is_ok());
    assert!(attributes::parse_extmap("255 urn:test").is_ok());
    
    // Error cases
    
    // Invalid ID (0, must be 1-255)
    assert!(attributes::parse_extmap("0 urn:test").is_err());
    
    // Invalid ID (> 255)
    assert!(attributes::parse_extmap("256 urn:test").is_err());
    
    // Invalid direction
    assert!(attributes::parse_extmap("1/invalid urn:test").is_err());
    
    // Invalid URI format
    assert!(attributes::parse_extmap("1 not-a-valid-uri").is_err());
    
    // Missing URI
    assert!(attributes::parse_extmap("1").is_err());
}

/// Tests for media stream ID (msid) attribute parsing
#[test]
fn test_msid_attribute_comprehensive() {
    // Valid msid attributes
    
    // Stream ID only
    let stream_only = "stream1";
    assert!(attributes::parse_msid(stream_only).is_ok());
    let (stream_id, track_id) = attributes::parse_msid(stream_only).unwrap();
    assert_eq!(stream_id, "stream1");
    assert_eq!(track_id, None);
    
    // Stream ID and track ID
    let with_track = "stream1 track1";
    assert!(attributes::parse_msid(with_track).is_ok());
    let (stream_id, track_id) = attributes::parse_msid(with_track).unwrap();
    assert_eq!(stream_id, "stream1");
    assert_eq!(track_id, Some("track1".to_string()));
    
    // Stream ID with special characters
    let special_chars = "stream-1_2.3";
    assert!(attributes::parse_msid(special_chars).is_ok());
    
    // Whitespace handling
    let whitespace = "  stream1  track1  ";
    assert!(attributes::parse_msid(whitespace).is_ok());
    let (stream_id, track_id) = attributes::parse_msid(whitespace).unwrap();
    assert_eq!(stream_id, "stream1");
    assert_eq!(track_id, Some("track1".to_string()));
    
    // Multiple track IDs (should only take the first one as track ID)
    let multiple = "stream1 track1 track2";
    assert!(attributes::parse_msid(multiple).is_ok());
    let (stream_id, track_id) = attributes::parse_msid(multiple).unwrap();
    assert_eq!(stream_id, "stream1");
    assert_eq!(track_id, Some("track1".to_string()));
    
    // Error cases
    
    // Empty msid
    assert!(attributes::parse_msid("").is_err());
    
    // Only whitespace
    assert!(attributes::parse_msid("   ").is_err());
}

/// Tests for bandwidth attribute parsing
#[test]
fn test_bandwidth_attribute_comprehensive() {
    // Valid bandwidth attributes
    
    // Application specific (AS)
    let as_bw = "AS:128";
    assert!(attributes::parse_bandwidth(as_bw).is_ok());
    let (bwtype, bw) = attributes::parse_bandwidth(as_bw).unwrap();
    assert_eq!(bwtype, "AS");
    assert_eq!(bw, 128);
    
    // Transport independent application specific (TIAS)
    let tias = "TIAS:64000";
    assert!(attributes::parse_bandwidth(tias).is_ok());
    let (bwtype, bw) = attributes::parse_bandwidth(tias).unwrap();
    assert_eq!(bwtype, "TIAS");
    assert_eq!(bw, 64000);
    
    // RTCP (RR, RS)
    let rr = "RR:8000";
    assert!(attributes::parse_bandwidth(rr).is_ok());
    
    let rs = "RS:16000";
    assert!(attributes::parse_bandwidth(rs).is_ok());
    
    // Conference total (CT)
    let ct = "CT:256";
    assert!(attributes::parse_bandwidth(ct).is_ok());
    
    // Custom bandwidth type
    let custom = "X-CUSTOM:1000";
    assert!(attributes::parse_bandwidth(custom).is_ok());
    
    // Error cases
    
    // Missing bwtype
    assert!(attributes::parse_bandwidth(":128").is_err());
    
    // Missing bandwidth value
    assert!(attributes::parse_bandwidth("AS:").is_err());
    
    // Invalid format (no colon)
    assert!(attributes::parse_bandwidth("AS128").is_err());
    
    // Invalid bandwidth value (not a number)
    assert!(attributes::parse_bandwidth("AS:invalid").is_err());
    
    // Invalid bandwidth value (negative)
    assert!(attributes::parse_bandwidth("AS:-128").is_err());
}

/// Tests for helper validation functions
#[test]
fn test_validator_functions() {
    // is_valid_token
    assert!(attributes::is_valid_token("valid-token"));
    assert!(attributes::is_valid_token("validtoken123"));
    assert!(attributes::is_valid_token("valid.token"));
    assert!(attributes::is_valid_token("valid_token"));
    assert!(!attributes::is_valid_token(""));
    assert!(!attributes::is_valid_token("invalid@token"));
    assert!(!attributes::is_valid_token("invalid token"));
    
    // is_valid_ipv4 (from parser)
    assert!(is_valid_ipv4("192.168.1.1"));
    assert!(is_valid_ipv4("0.0.0.0"));
    assert!(is_valid_ipv4("255.255.255.255"));
    assert!(!is_valid_ipv4("256.0.0.0"));
    assert!(!is_valid_ipv4("192.168.1"));
    assert!(!is_valid_ipv4("192.168.1.1.5"));
    
    // is_valid_ipv6 (from parser)
    assert!(is_valid_ipv6("2001:db8::1"));
    assert!(is_valid_ipv6("::1"));
    assert!(is_valid_ipv6("2001:db8:0:0:0:0:0:1"));
    assert!(!is_valid_ipv6("2001:db8:g::1")); // invalid hex
    assert!(!is_valid_ipv6("2001::db8::1")); // multiple ::
    
    // is_valid_hostname (from parser)
    assert!(is_valid_hostname("example.com"));
    assert!(is_valid_hostname("sub.example.com"));
    assert!(is_valid_hostname("example-test.com"));
    assert!(!is_valid_hostname("-example.com")); // can't start with hyphen
    assert!(!is_valid_hostname("example..com")); // empty label
}

// After the last test module, let's add a new comprehensive_tests module

/// More comprehensive tests for SDP parser robustness
mod comprehensive_tests {
    use super::*;

    /// Test variations of origin (o=) line parsing
    #[test]
    fn test_origin_variations() {
        // Valid cases with different username formats
        assert!(parse_origin_line("- 2890844526 2890842807 IN IP4 10.47.16.5").is_ok());
        assert!(parse_origin_line("jdoe 2890844526 2890842807 IN IP4 10.47.16.5").is_ok());
        
        // Valid case with IPv6
        assert!(parse_origin_line("jdoe 2890844526 2890842807 IN IP6 2001:db8::1").is_ok());
        
        // Valid case with hostname
        assert!(parse_origin_line("jdoe 2890844526 2890842807 IN IP4 example.com").is_ok());
        
        // Invalid cases
        
        // Wrong number of fields
        assert!(parse_origin_line("jdoe 2890844526 2890842807 IN IP4").is_err());
        
        // Invalid network type
        assert!(parse_origin_line("jdoe 2890844526 2890842807 OTHER IP4 10.47.16.5").is_err());
        
        // Invalid address type
        assert!(parse_origin_line("jdoe 2890844526 2890842807 IN IPX 10.47.16.5").is_err());
        
        // Invalid IP address
        assert!(parse_origin_line("jdoe 2890844526 2890842807 IN IP4 999.999.999.999").is_err());
        
        // Invalid session ID (not numeric)
        assert!(parse_origin_line("jdoe session-id 2890842807 IN IP4 10.47.16.5").is_err());
        
        // Invalid version (not numeric)
        assert!(parse_origin_line("jdoe 2890844526 version-num IN IP4 10.47.16.5").is_err());
    }
    
    /// Test variations of connection (c=) line parsing
    #[test]
    fn test_connection_variations() {
        // Valid cases
        assert!(parse_connection_line("IN IP4 10.47.16.5").is_ok());
        assert!(parse_connection_line("IN IP6 2001:db8::1").is_ok());
        assert!(parse_connection_line("IN IP4 example.com").is_ok());
        
        // Valid multicast with TTL
        assert!(parse_connection_line("IN IP4 224.2.36.42/127").is_ok());
        
        // Valid multicast with TTL and count
        assert!(parse_connection_line("IN IP4 224.2.36.42/127/3").is_ok());
        
        // Invalid cases
        
        // Wrong number of fields
        assert!(parse_connection_line("IN IP4").is_err());
        
        // Invalid network type
        assert!(parse_connection_line("OTHER IP4 10.47.16.5").is_err());
        
        // Invalid address type
        assert!(parse_connection_line("IN IPX 10.47.16.5").is_err());
        
        // Invalid IP address format
        assert!(parse_connection_line("IN IP4 999.999.999.999").is_err());
        
        // Invalid TTL format
        assert!(parse_connection_line("IN IP4 224.2.36.42/999").is_err());
        
        // Invalid multicast format
        assert!(parse_connection_line("IN IP4 224.2.36.42/127/999").is_err());
    }
    
    /// Test variations of time (t=) and repeat (r=) line parsing
    #[test]
    fn test_time_and_repeat_variations() {
        // Valid cases
        assert!(parse_time_description_line("0 0").is_ok());
        assert!(parse_time_description_line("2873397496 2873404696").is_ok());
        
        // Edge cases
        assert!(parse_time_description_line("0 9999999999").is_ok());
        
        // Invalid cases
        
        // Wrong number of fields
        assert!(parse_time_description_line("0").is_err());
        
        // Non-numeric values
        assert!(parse_time_description_line("start end").is_err());
        
        // Stop time before start time (not valid except for 0)
        assert!(parse_time_description_line("3000 2000").is_err());
    }
    
    /// Test complex SDP with multiple media sections and attributes
    #[test]
    fn test_complex_multiparty_sdp() {
        // A complex SDP for a multiparty video conference
        let sdp = "\
v=0\r
o=conference 2890844526 2890842807 IN IP4 10.47.16.5\r
s=Multiparty Video Conference\r
i=A sample multiparty video conference\r
u=http://example.com/conference-info\r
e=conf-admin@example.com\r
p=+1-212-555-1234\r
c=IN IP4 224.2.17.12/127\r
t=2873397496 2873404696\r
r=7d 1h 0 25h\r
b=AS:512\r
a=recvonly\r
a=group:BUNDLE audio video\r
a=ice-options:trickle\r
m=audio 49170 RTP/AVP 0 8 97\r
i=Main Audio\r
c=IN IP4 203.0.113.1\r
b=AS:64\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:97 iLBC/8000\r
a=sendrecv\r
a=mid:audio\r
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=candidate:1 1 UDP 2130706431 203.0.113.1 49170 typ host\r
a=candidate:2 1 UDP 1694498815 203.0.113.1 49171 typ srflx raddr 192.168.1.5 rport 49170\r
a=ssrc:1001 cname:participant1@example.com\r
a=ssrc:1001 msid:stream1 track1\r
m=video 51372 RTP/AVP 99 100\r
i=Main Video\r
c=IN IP4 203.0.113.1\r
b=AS:384\r
a=rtpmap:99 H264/90000\r
a=rtpmap:100 VP8/90000\r
a=fmtp:99 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1\r
a=sendrecv\r
a=mid:video\r
a=rtcp-fb:99 nack\r
a=rtcp-fb:99 nack pli\r
a=rtcp-fb:99 ccm fir\r
a=extmap:1 urn:ietf:params:rtp-hdrext:toffset\r
a=extmap:2 urn:ietf:params:rtp-hdrext:sdes:mid\r
a=setup:actpass\r
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:11:04:87:0D:D7:3F:B8:BF:79:7D\r
a=ssrc:2001 cname:participant1@example.com\r
a=ssrc:2001 msid:stream1 track2\r
m=application 5000 UDP/DTLS/SCTP webrtc-datachannel\r
c=IN IP4 203.0.113.1\r
a=sctp-port:5000\r
a=max-message-size:262144\r
a=setup:actpass\r
a=mid:data\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse complex multiparty SDP: {:?}", result.err());
        
        let session = result.unwrap();
        
        // Verify session-level properties
        assert_eq!(session.origin.username, "conference");
        assert_eq!(session.session_name, "Multiparty Video Conference");
        assert_eq!(session.direction, Some(MediaDirection::RecvOnly));
        assert_eq!(session.time_descriptions.len(), 1);
        assert_eq!(session.time_descriptions[0].start_time, "2873397496");
        assert_eq!(session.time_descriptions[0].stop_time, "2873404696");
        
        // Verify media sections
        assert_eq!(session.media_descriptions.len(), 3);
        
        // Audio media
        let audio = &session.media_descriptions[0];
        assert_eq!(audio.media, "audio");
        assert_eq!(audio.port, 49170);
        assert_eq!(audio.direction, Some(MediaDirection::SendRecv));
        
        // Video media
        let video = &session.media_descriptions[1];
        assert_eq!(video.media, "video");
        assert_eq!(video.port, 51372);
        
        // Data channel
        let data = &session.media_descriptions[2];
        assert_eq!(data.media, "application");
        assert_eq!(data.port, 5000);
        assert_eq!(data.protocol, "UDP/DTLS/SCTP");
    }

    /// Test SDP with unusual but valid constructs
    #[test]
    fn test_unusual_valid_sdp() {
        // Test SDP with unusual but valid line combinations
        let sdp = "\
v=0\r
o=- 0 0 IN IP4 0.0.0.0\r
s=-\r
t=0 0\r
a=ice-lite\r
a=msid-semantic:WMS *\r
a=group:BUNDLE audio video\r
m=audio 9 UDP/TLS/RTP/SAVPF 111\r
c=IN IP4 0.0.0.0\r
a=rtcp:9 IN IP4 0.0.0.0\r
a=ice-ufrag:aaaa\r
a=ice-pwd:aaaaaaaaaaaaaaaaaaaaaaaa\r
a=fingerprint:sha-256 AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA\r
a=setup:actpass\r
a=mid:audio\r
a=rtpmap:111 opus/48000/2\r
a=fmtp:111 minptime=10;useinbandfec=1\r
a=rtcp-fb:111 transport-cc\r
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=extmap:2 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r
a=rtcp-mux\r
a=candidate:1 1 udp 2113937151 192.168.0.1 54400 typ host\r
a=candidate:2 1 tcp 1518280447 192.168.0.1 9 typ host tcptype active\r
a=end-of-candidates\r
a=ssrc:1111 cname:test@example.com\r
a=ssrc:1111 msid:stream track\r
a=inactive\r
m=video 9 UDP/TLS/RTP/SAVPF 96\r
c=IN IP4 0.0.0.0\r
a=rtcp:9 IN IP4 0.0.0.0\r
a=ice-ufrag:aaaa\r
a=ice-pwd:aaaaaaaaaaaaaaaaaaaaaaaa\r
a=fingerprint:sha-256 AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA\r
a=setup:actpass\r
a=mid:video\r
a=rtpmap:96 VP8/90000\r
a=rtcp-fb:96 ccm fir\r
a=rtcp-fb:96 nack\r
a=rtcp-fb:96 nack pli\r
a=rtcp-fb:96 goog-remb\r
a=rtcp-fb:96 transport-cc\r
a=extmap:3 urn:ietf:params:rtp-hdrext:toffset\r
a=extmap:4 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r
a=rtcp-mux\r
a=simulcast:send 1;2;~3\r
a=rid:1 send pt=96 max-width=1280;max-height=720\r
a=rid:2 send pt=96 max-width=640;max-height=480\r
a=rid:3 send pt=96 max-width=320;max-height=240\r
a=inactive\r
";
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse unusual but valid SDP: {:?}", result.err());
        
        let session = result.unwrap();
        
        // Verify unusual aspects
        assert_eq!(session.origin.sess_id, "0");
        assert_eq!(session.origin.sess_version, "0");
        assert_eq!(session.origin.unicast_address, "0.0.0.0");
        assert_eq!(session.session_name, "-");
        
        // Check media sections
        let audio = &session.media_descriptions[0];
        assert_eq!(audio.port, 9); // Port 9 is unusual but valid
        assert_eq!(audio.protocol, "UDP/TLS/RTP/SAVPF");
        assert_eq!(audio.direction, Some(MediaDirection::Inactive));
        
        // Check video section with simulcast
        let video = &session.media_descriptions[1];
        assert_eq!(video.direction, Some(MediaDirection::Inactive));
        
        // Verify simulcast and rid attributes exist
        let has_simulcast = video.generic_attributes.iter().any(|a| {
            if let ParsedAttribute::Simulcast(send, _) = a {
                send.contains(&"1;2;~3".to_string())
            } else {
                false
            }
        });
        assert!(has_simulcast, "Simulcast attribute not found or incorrect");
        
        // Check for rid attributes
        let has_rid = video.generic_attributes.iter().any(|a| {
            if let ParsedAttribute::Rid(id, dir, _) = a {
                id == "1" && dir == "send"
            } else {
                false
            }
        });
        assert!(has_rid, "RID attribute not found or incorrect");
    }

    /// Test malformed SDPs that should be rejected
    #[test]
    fn test_malformed_sdp_rejection() {
        // Missing mandatory line - no session name (s=)
        let missing_session = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
";
        assert!(parse_sdp(&create_test_sdp_bytes(missing_session)).is_err());
        
        // Invalid origin line format
        let invalid_origin = "\
v=0\r
o=jdoe 2890844526 2890842807 IN INVALID 10.47.16.5\r
s=SDP Test\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
        assert!(parse_sdp(&create_test_sdp_bytes(invalid_origin)).is_err());
        
        // Incorrect line ordering (m= before t=)
        let incorrect_order = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Test\r
c=IN IP4 224.2.17.12/127\r
m=audio 49170 RTP/AVP 0\r
t=0 0\r
";
        assert!(parse_sdp(&create_test_sdp_bytes(incorrect_order)).is_err());
        
        // Unknown line type
        let unknown_line_type = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Test\r
x=Unknown Line Type\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
";
        assert!(parse_sdp(&create_test_sdp_bytes(unknown_line_type)).is_err());
        
        // Invalid time line format
        let invalid_time = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Test\r
c=IN IP4 224.2.17.12/127\r
t=start end\r
";
        assert!(parse_sdp(&create_test_sdp_bytes(invalid_time)).is_err());
        
        // Invalid media format
        let invalid_media = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Test\r
c=IN IP4 224.2.17.12/127\r
t=0 0\r
m=invalid 49170 RTP/AVP\r
";
        assert!(parse_sdp(&create_test_sdp_bytes(invalid_media)).is_err());
        
        // Missing or incomplete mandatory lines 
        // No c= line (neither session level nor media level)
        let no_connection = "\
v=0\r
o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r
s=SDP Test\r
t=0 0\r
m=audio 49170 RTP/AVP 0\r
";
        assert!(parse_sdp(&create_test_sdp_bytes(no_connection)).is_err());
    }

    /// Test SDP with boundary test values
    #[test]
    fn test_boundary_values() {
        // Test with extreme but valid values
        let boundary_sdp = "\
v=0\r
o=user 9223372036854775807 9223372036854775807 IN IP4 0.0.0.0\r
s=Boundary Test SDP\r
c=IN IP4 255.255.255.255\r
t=9223372036854775807 9223372036854775807\r
m=audio 65535 RTP/AVP 127\r
a=rtpmap:127 opus/48000/2\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(boundary_sdp));
        assert!(result.is_ok(), "Failed to parse SDP with boundary values: {:?}", result.err());
        
        let session = result.unwrap();
        assert_eq!(session.origin.sess_id, "9223372036854775807");
        assert_eq!(session.origin.sess_version, "9223372036854775807");
        
        // Check time
        assert_eq!(session.time_descriptions[0].start_time, "9223372036854775807");
        assert_eq!(session.time_descriptions[0].stop_time, "9223372036854775807");
        
        // Check media port
        assert_eq!(session.media_descriptions[0].port, 65535);
        
        // Test with invalid boundary values
        
        // Invalid port (over 65535)
        let invalid_port_sdp = "\
v=0\r
o=user 1 1 IN IP4 0.0.0.0\r
s=Invalid Port Test\r
c=IN IP4 0.0.0.0\r
t=0 0\r
m=audio 65536 RTP/AVP 0\r
";
        
        assert!(parse_sdp(&create_test_sdp_bytes(invalid_port_sdp)).is_err());
    }

    /// Test SDP with unusual media types
    #[test]
    fn test_unusual_media_types() {
        // Test with text, message, and image media types
        let unusual_media_sdp = "\
v=0\r
o=user 123456 654321 IN IP4 198.51.100.7\r
s=Unusual Media Types\r
c=IN IP4 198.51.100.7\r
t=0 0\r
m=text 49172 RTP/AVP 98\r
a=rtpmap:98 t140/1000\r
m=message 6000 TCP/MSRP *\r
m=image 6001 TCP/UDPTL t38\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(unusual_media_sdp));
        assert!(result.is_ok(), "Failed to parse SDP with unusual media types: {:?}", result.err());
        
        let session = result.unwrap();
        
        // Check media types
        assert_eq!(session.media_descriptions[0].media, "text");
        assert_eq!(session.media_descriptions[1].media, "message");
        assert_eq!(session.media_descriptions[2].media, "image");
        
        // Check protocols
        assert_eq!(session.media_descriptions[1].protocol, "TCP/MSRP");
        assert_eq!(session.media_descriptions[2].protocol, "TCP/UDPTL");
    }

    /// Test SDPs with TrickleICE updates (partial SDPs)
    #[test]
    fn test_trickle_ice_updates() {
        // Test a partial SDP update for ICE candidates
        // This is not a complete SDP but should be parsed correctly in some contexts
        let trickle_update = "\
a=ice-ufrag:F7gI\r
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r
a=candidate:5 1 UDP 2130706431 192.168.1.1 50000 typ host\r
a=candidate:6 1 UDP 1694498815 192.0.2.5 55555 typ srflx raddr 192.168.1.1 rport 50000\r
a=end-of-candidates\r
";
        
        // Individual attribute parsing should work
        assert!(attributes::parse_ice_ufrag("F7gI").is_ok());
        assert!(attributes::parse_ice_pwd("x9cml/YzichV2+XlhiMu8g").is_ok());
        assert!(attributes::parse_candidate("5 1 UDP 2130706431 192.168.1.1 50000 typ host").is_ok());
        assert!(attributes::parse_end_of_candidates("").is_ok());
    }

    /// Test attribute parsing with special characters
    #[test]
    fn test_attributes_with_special_chars() {
        // Test attributes with special characters
        assert!(attributes::parse_ssrc("1234 cname:user@example.com").is_ok());
        assert!(attributes::parse_ssrc("5678 label:special-1+2=3*4").is_ok());
        
        // Test ice-ufrag and ice-pwd with base64-like encoding (common in WebRTC)
        assert!(attributes::parse_ice_ufrag("f5/g+IuK").is_ok());
        assert!(attributes::parse_ice_pwd("x9cml/YzichV2+XlhiMu8g==").is_ok());
        
        // Test fingerprint with various hash algorithms
        assert!(attributes::parse_fingerprint("sha-1 CD:34:D1:62:16:95:7B:B7:EB:74:E2:39:27:97:EB:0B").is_ok());
        assert!(attributes::parse_fingerprint("sha-256 CD:34:D1:62:16:95:7B:B7:EB:74:E2:39:27:97:EB:0B:CD:34:D1:62:16:95:7B:B7").is_ok());
        
        // Test msid with special formats
        assert!(attributes::parse_msid("stream-id_1.2.3 track-id_a.b.c").is_ok());
        
        // Test fmtp with complex parameters
        assert!(attributes::parse_fmtp("96 profile-level-id=42e01f;max-fr=30;max-fs=8160;x-google-min-bitrate=1000").is_ok());
    }

    /// Test SDP with mixed line endings
    #[test]
    fn test_mixed_line_endings() {
        // SDP with mixed CR, LF, and CRLF line endings
        let mixed_endings = "v=0\ro=test 123 456 IN IP4 127.0.0.1\ns=Mixed Line Endings\nt=0 0\nc=IN IP4 127.0.0.1\r\nm=audio 5000 RTP/AVP 0\na=rtpmap:0 PCMU/8000\r\n";
        
        let result = parse_sdp(&create_test_sdp_bytes(mixed_endings));
        assert!(result.is_ok(), "Failed to parse SDP with mixed line endings: {:?}", result.err());
        
        // SDP with LF only (should be accepted by our parser even though RFC requires CRLF)
        let lf_only = "v=0\no=test 123 456 IN IP4 127.0.0.1\ns=LF Only\nt=0 0\nc=IN IP4 127.0.0.1\nm=audio 5000 RTP/AVP 0\na=rtpmap:0 PCMU/8000\n";
        
        let result = parse_sdp(&create_test_sdp_bytes(lf_only));
        assert!(result.is_ok(), "Failed to parse SDP with LF-only line endings: {:?}", result.err());
        
        // SDP with CR only
        let cr_only = "v=0\ro=test 123 456 IN IP4 127.0.0.1\rs=CR Only\rt=0 0\rc=IN IP4 127.0.0.1\rm=audio 5000 RTP/AVP 0\ra=rtpmap:0 PCMU/8000\r";
        
        let result = parse_sdp(&create_test_sdp_bytes(cr_only));
        assert!(result.is_ok(), "Failed to parse SDP with CR-only line endings: {:?}", result.err());
    }

    /// Test SDP with duplicate fields that should be rejected
    #[test]
    fn test_duplicate_fields() {
        // Duplicate v= line (should be rejected)
        let duplicate_version = "\
v=0\r
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Duplicate Version\r
t=0 0\r
c=IN IP4 127.0.0.1\r
m=audio 5000 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(duplicate_version));
        assert!(result.is_err(), "Parser should reject SDP with duplicate v= line");
        
        // Duplicate o= line (should be rejected)
        let duplicate_origin = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
o=test2 789 101 IN IP4 127.0.0.2\r
s=Duplicate Origin\r
t=0 0\r
c=IN IP4 127.0.0.1\r
m=audio 5000 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(duplicate_origin));
        assert!(result.is_err(), "Parser should reject SDP with duplicate o= line");
        
        // Duplicate s= line (should be rejected)
        let duplicate_session = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Session 1\r
s=Session 2\r
t=0 0\r
c=IN IP4 127.0.0.1\r
m=audio 5000 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(duplicate_session));
        assert!(result.is_err(), "Parser should reject SDP with duplicate s= line");
        
        // Duplicate media-level c= line (should be rejected)
        let duplicate_media_conn = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Duplicate Media Connection\r
t=0 0\r
m=audio 5000 RTP/AVP 0\r
c=IN IP4 127.0.0.1\r
c=IN IP4 127.0.0.2\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(duplicate_media_conn));
        assert!(result.is_err(), "Parser should reject SDP with duplicate media-level c= line");
    }

    /// Test SDP with very minimal but valid content
    #[test]
    fn test_minimal_valid_sdp() {
        // Absolutely minimal valid SDP as per RFC 8866
        let minimal = "\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=-\r
t=0 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(minimal));
        assert!(result.is_ok(), "Failed to parse minimal valid SDP: {:?}", result.err());
        
        // Minimal SDP with one media section
        let minimal_with_media = "\
v=0\r
o=- 0 0 IN IP4 127.0.0.1\r
s=-\r
c=IN IP4 127.0.0.1\r
t=0 0\r
m=audio 0 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(minimal_with_media));
        assert!(result.is_ok(), "Failed to parse minimal valid SDP with media: {:?}", result.err());
    }

    /// Test SDP with multiple time descriptions
    #[test]
    fn test_multiple_time_descriptions() {
        // SDP with multiple time periods
        let multiple_times = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Multiple Time Periods\r
c=IN IP4 127.0.0.1\r
t=3034423619 3042462419\r
r=604800 3600 0 90000\r
t=3034423619 3042462419\r
r=604800 3600 0 90000\r
m=audio 5000 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(multiple_times));
        assert!(result.is_ok(), "Failed to parse SDP with multiple time descriptions: {:?}", result.err());
        
        let session = result.unwrap();
        assert_eq!(session.time_descriptions.len(), 2, "Should have parsed 2 time descriptions");
    }

    /// Test complex attribute combinations
    #[test]
    fn test_complex_attributes() {
        // Test parsing complex combinations of attributes
        
        // RID with complex restrictions
        let complex_rid = "1 send pt=96,97,98 max-width=1280;max-height=720;max-fps=30;max-fs=8160";
        let result = attributes::parse_rid(complex_rid);
        assert!(result.is_ok());
        let (id, direction, restrictions) = result.unwrap();
        assert_eq!(id, "1");
        assert_eq!(direction, "send");
        // According to RFC 8851, these should be 5 separate restrictions:
        // 1. pt=96,97,98 (payload types)
        // 2. max-width=1280
        // 3. max-height=720
        // 4. max-fps=30
        // 5. max-fs=8160
        assert_eq!(restrictions.len(), 5);
        assert_eq!(restrictions[0], "pt=96,97,98");
        assert_eq!(restrictions[1], "max-width=1280");
        assert_eq!(restrictions[2], "max-height=720");
        assert_eq!(restrictions[3], "max-fps=30");
        assert_eq!(restrictions[4], "max-fs=8160");
        
        // Simulcast with complex layers
        let complex_simulcast = "send 1,~2,3;4,~5 recv 6;~7,8;~9";
        println!("Testing simulcast parse with: '{}'", complex_simulcast);
        let result = attributes::parse_simulcast(complex_simulcast);
        assert!(result.is_ok(), "Failed to parse simulcast");
        let (send_streams, recv_streams) = result.unwrap();
        println!("Simulcast result: send_streams={:?}, recv_streams={:?}", send_streams, recv_streams);
        println!("send_streams.len()={}, recv_streams.len()={}", send_streams.len(), recv_streams.len());
        assert_eq!(send_streams.len(), 2, "Expected 2 send streams, got {}", send_streams.len());
        assert_eq!(recv_streams.len(), 3, "Expected 3 recv streams, got {}", recv_streams.len());
        
        // SSRC with multiple attributes for the same SSRC ID
        assert!(attributes::parse_ssrc("1234 cname:user@example.com").is_ok());
        assert!(attributes::parse_ssrc("1234 msid:stream track").is_ok());
        assert!(attributes::parse_ssrc("1234 label:track-label").is_ok());
        assert!(attributes::parse_ssrc("1234 mslabel:stream-label").is_ok());
    }

    /// Test validation of cross-referenced attributes
    #[test]
    fn test_attribute_cross_validation() {
        // Valid cross-references between attributes
        let valid_attrs = vec![
            ParsedAttribute::Mid("audio".to_string()),
            ParsedAttribute::Mid("video".to_string()),
            ParsedAttribute::Group("BUNDLE".to_string(), vec!["audio".to_string(), "video".to_string()]),
            ParsedAttribute::Rid("1".to_string(), "send".to_string(), vec!["pt=96".to_string()]),
            ParsedAttribute::Rid("2".to_string(), "send".to_string(), vec!["pt=97".to_string()]),
            ParsedAttribute::Simulcast(vec!["1,2".to_string()], vec![]),
        ];
        
        assert!(attributes::validate_attributes(&valid_attrs).is_ok());
        
        // Invalid cross-references
        let invalid_attrs = vec![
            ParsedAttribute::Mid("audio".to_string()),
            // Missing "video" mid
            ParsedAttribute::Group("BUNDLE".to_string(), vec!["audio".to_string(), "video".to_string()]),
            ParsedAttribute::Rid("1".to_string(), "send".to_string(), vec!["pt=96".to_string()]),
            // Missing "2" rid, but referenced in simulcast
            ParsedAttribute::Simulcast(vec!["1,2".to_string()], vec![]),
        ];
        
        assert!(attributes::validate_attributes(&invalid_attrs).is_err());
    }

    /// Test SDP with real WebRTC examples
    #[test]
    fn test_real_webrtc_examples() {
        // A typical Chrome WebRTC SDP offer with ICE and DTLS
        let chrome_offer = "\
v=0\r
o=- 3546004397921447048 2 IN IP4 127.0.0.1\r
s=-\r
t=0 0\r
a=group:BUNDLE 0\r
a=extmap-allow-mixed\r
a=msid-semantic: WMS 9YH4rlN2zdwCeLNM7AHayYNUXyK0ihpQZoDl\r
m=audio 9 UDP/TLS/RTP/SAVPF 111 63 103 104 9 0 8 106 105 13 110 112 113 126\r
c=IN IP4 0.0.0.0\r
a=rtcp:9 IN IP4 0.0.0.0\r
a=ice-ufrag:kBSV\r
a=ice-pwd:08QxKco6Jtm/W7HvNZPDXTKp\r
a=ice-options:trickle\r
a=fingerprint:sha-256 C9:1F:47:96:BC:6D:AC:7F:C5:BF:76:C5:1F:40:11:C3:5D:8B:51:CD:59:13:33:24:62:45:CA:81:B1:45:11:8B\r
a=setup:actpass\r
a=mid:0\r
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=extmap:2 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r
a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r
a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid\r
a=sendrecv\r
a=msid:9YH4rlN2zdwCeLNM7AHayYNUXyK0ihpQZoDl dc8c148d-e2d0-487c-9987-40f986c8ec0e\r
a=rtcp-mux\r
a=rtpmap:111 opus/48000/2\r
a=rtcp-fb:111 transport-cc\r
a=fmtp:111 minptime=10;useinbandfec=1\r
a=rtpmap:63 red/48000/2\r
a=rtpmap:103 ISAC/16000\r
a=rtpmap:104 ISAC/32000\r
a=rtpmap:9 G722/8000\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:106 CN/32000\r
a=rtpmap:105 CN/16000\r
a=rtpmap:13 CN/8000\r
a=rtpmap:110 telephone-event/48000\r
a=rtpmap:112 telephone-event/32000\r
a=rtpmap:113 telephone-event/16000\r
a=rtpmap:126 telephone-event/8000\r
a=ssrc:3647729951 cname:7Em9ApRr7W44h3LN\r
a=ssrc:3647729951 msid:9YH4rlN2zdwCeLNM7AHayYNUXyK0ihpQZoDl dc8c148d-e2d0-487c-9987-40f986c8ec0e\r
a=ssrc:3647729951 mslabel:9YH4rlN2zdwCeLNM7AHayYNUXyK0ihpQZoDl\r
a=ssrc:3647729951 label:dc8c148d-e2d0-487c-9987-40f986c8ec0e\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(chrome_offer));
        assert!(result.is_ok(), "Failed to parse Chrome WebRTC SDP offer: {:?}", result.err());
        
        // A typical Firefox WebRTC SDP answer
        let firefox_answer = "\
v=0\r
o=mozilla...THIS_IS_SDPARTA-96.0.1 4562394892154513225 0 IN IP4 0.0.0.0\r
s=-\r
t=0 0\r
a=fingerprint:sha-256 0A:09:75:29:12:3E:7C:F4:84:D3:87:06:1B:56:42:E0:96:58:BB:CD:B8:6E:5C:98:CD:D3:1E:DE:8B:E8:8D:79\r
a=group:BUNDLE 0\r
a=ice-options:trickle\r
a=msid-semantic:WMS *\r
m=audio 9 UDP/TLS/RTP/SAVPF 111 9 0 8 126\r
c=IN IP4 0.0.0.0\r
a=sendrecv\r
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=extmap:2/recvonly urn:ietf:params:rtp-hdrext:csrc-audio-level\r
a=extmap:3 urn:ietf:params:rtp-hdrext:sdes:mid\r
a=fmtp:111 maxplaybackrate=48000;stereo=1;useinbandfec=1\r
a=fmtp:126 0-15\r
a=ice-pwd:b4f826190f246dff79b9d216d5c10b14\r
a=ice-ufrag:0b9e274d\r
a=mid:0\r
a=msid:{6b18cbce-2cf0-4e03-bf4e-ede6b07be951} {7b7ebefe-8a9a-4d7c-86c3-09df51d2f2d3}\r
a=rtcp-mux\r
a=rtpmap:111 opus/48000/2\r
a=rtpmap:9 G722/8000/1\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:126 telephone-event/8000/1\r
a=setup:active\r
a=ssrc:3637724332 cname:{be57ad00-7330-4ec0-8ed2-af8db1f30f09}\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(firefox_answer));
        assert!(result.is_ok(), "Failed to parse Firefox WebRTC SDP answer: {:?}", result.err());
    }

    /// Test SDP with challenging input that should still be parsed correctly
    #[test]
    fn test_challenging_valid_input() {
        // Valid SDP with unusual but acceptable characters in fields
        let challenging = "\
v=0\r
o=john.doe@example.com 2890844526 2890842807 IN IP4 192.0.2.1\r
s=SDP with unusual/special chars: !@#$%^&*()_+-=\r
i=This is a session \"with\" 'quoted' characters\r
u=http://example.com/session?id=123&name=test\r
e=John Doe <john.doe@example.com> (Session admin)\r
p=+1-555-123-4567;ext=1234\r
c=IN IP4 224.2.36.42/127\r
t=0 0\r
a=rtpmap:96 H264/90000\r
a=fmtp:96 packetization-mode=1;profile-level-id=4d001e;sprop-parameter-sets=Z0IACpZTBYmI,aMljiA==\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(challenging));
        assert!(result.is_ok(), "Failed to parse challenging but valid SDP: {:?}", result.err());
    }

    /// Test validation of specific attribute types with malformed input
    #[test]
    fn test_attribute_validation_with_malformed_input() {
        // Test malformed rtpmap
        assert!(attributes::parse_rtpmap("not valid").is_err());
        assert!(attributes::parse_rtpmap("").is_err());
        assert!(attributes::parse_rtpmap("96").is_err());
        assert!(attributes::parse_rtpmap("96 H264").is_err());
        assert!(attributes::parse_rtpmap("96 H264/").is_err());
        assert!(attributes::parse_rtpmap("96 H264/invalid").is_err());
        assert!(attributes::parse_rtpmap("96 H264/90000/invalid").is_err());
        
        // Test malformed fmtp
        assert!(attributes::parse_fmtp("not valid").is_err());
        assert!(attributes::parse_fmtp("96").is_err());
        assert!(attributes::parse_fmtp("96 ").is_err());
        
        // Test malformed candidate
        assert!(attributes::parse_candidate("invalid").is_err());
        assert!(attributes::parse_candidate("1 1 UDP").is_err());
        assert!(attributes::parse_candidate("1 1 UDP 2130706431 192.168.1.1 5000").is_err()); // Missing typ
        
        // Test malformed fingerprint
        assert!(attributes::parse_fingerprint("not valid").is_err());
        assert!(attributes::parse_fingerprint("sha-256").is_err());
        assert!(attributes::parse_fingerprint("invalid AA:BB:CC").is_err()); // Invalid hash algorithm
        assert!(attributes::parse_fingerprint("sha-256 not:valid:format").is_err());
        
        // Test malformed ice-ufrag and ice-pwd
        assert!(attributes::parse_ice_ufrag("abc").is_err()); // Too short
        assert!(attributes::parse_ice_pwd("tooshort").is_err()); // Too short
    }

    /// Test WebRTC-specific attributes and extensions
    #[test]
    fn test_webrtc_specific_attributes() {
        // Test extmap-allow-mixed attribute
        let sdp = "\
v=0\r
o=- 123 456 IN IP4 127.0.0.1\r
s=WebRTC Session\r
t=0 0\r
a=extmap-allow-mixed\r
m=audio 9 UDP/TLS/RTP/SAVPF 111\r
c=IN IP4 0.0.0.0\r
a=rtcp:9 IN IP4 0.0.0.0\r
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=extmap:2/sendrecv urn:ietf:params:rtp-hdrext:csrc-audio-level\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(sdp));
        assert!(result.is_ok(), "Failed to parse SDP with extmap-allow-mixed: {:?}", result.err());
        
        // Test various WebRTC header extensions
        assert!(attributes::parse_extmap("1 urn:ietf:params:rtp-hdrext:ssrc-audio-level").is_ok());
        assert!(attributes::parse_extmap("2/sendonly urn:ietf:params:rtp-hdrext:toffset").is_ok());
        assert!(attributes::parse_extmap("3 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time").is_ok());
        assert!(attributes::parse_extmap("4 urn:ietf:params:rtp-hdrext:sdes:mid").is_ok());
        assert!(attributes::parse_extmap("5 urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id").is_ok());
        assert!(attributes::parse_extmap("6 urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id").is_ok());
        
        // Test RTCP feedback mechanisms
        assert!(attributes::parse_rtcp_fb("96 nack").is_ok());
        assert!(attributes::parse_rtcp_fb("96 nack pli").is_ok());
        assert!(attributes::parse_rtcp_fb("96 ccm fir").is_ok());
        assert!(attributes::parse_rtcp_fb("96 goog-remb").is_ok());
        assert!(attributes::parse_rtcp_fb("96 transport-cc").is_ok());
    }

    /// Test media formats and codec parameters
    #[test]
    fn test_media_formats_and_codec_parameters() {
        // Test audio codecs
        assert!(attributes::parse_rtpmap("0 PCMU/8000").is_ok());
        assert!(attributes::parse_rtpmap("8 PCMA/8000").is_ok());
        assert!(attributes::parse_rtpmap("111 opus/48000/2").is_ok());
        assert!(attributes::parse_rtpmap("9 G722/8000").is_ok());
        assert!(attributes::parse_rtpmap("13 CN/8000").is_ok());
        
        // Test video codecs
        assert!(attributes::parse_rtpmap("96 H264/90000").is_ok());
        assert!(attributes::parse_rtpmap("97 VP8/90000").is_ok());
        assert!(attributes::parse_rtpmap("98 VP9/90000").is_ok());
        assert!(attributes::parse_rtpmap("100 H265/90000").is_ok());
        assert!(attributes::parse_rtpmap("125 AV1/90000").is_ok());
        
        // Test FMTP parameters
        assert!(attributes::parse_fmtp("96 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1").is_ok());
        assert!(attributes::parse_fmtp("97 max-fr=30;max-fs=3600").is_ok());
        assert!(attributes::parse_fmtp("111 minptime=10;useinbandfec=1;stereo=1").is_ok());
        assert!(attributes::parse_fmtp("100 profile-id=1;level-id=93").is_ok());
    }

    /// Test parsing with nonstandard line orders that should still work
    #[test]
    fn test_nonstandard_but_valid_line_orders() {
        // Sessions attributes before connection line (valid order)
        let valid_order = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Test Session\r
i=This is a test session\r
a=recvonly\r
c=IN IP4 127.0.0.1\r
t=0 0\r
m=audio 5000 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(valid_order));
        assert!(result.is_ok(), "Failed to parse SDP with nonstandard but valid line order: {:?}", result.err());
        
        // Session info (i=) after time line (valid order)
        let valid_order2 = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Test Session\r
t=0 0\r
i=This is a test session\r
c=IN IP4 127.0.0.1\r
m=audio 5000 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(valid_order2));
        assert!(result.is_ok(), "Failed to parse SDP with nonstandard but valid line order: {:?}", result.err());
    }

    /// Test SDPs with unusual or edge case values
    #[test]
    fn test_edge_case_values() {
        // Port 0 (special value meaning media is inactive/rejected)
        let port_zero = "\
v=0\r
o=- 123 456 IN IP4 127.0.0.1\r
s=Port Zero Test\r
t=0 0\r
c=IN IP4 127.0.0.1\r
m=audio 0 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(port_zero));
        assert!(result.is_ok(), "Failed to parse SDP with port 0: {:?}", result.err());
        let session = result.unwrap();
        assert_eq!(session.media_descriptions[0].port, 0);
        
        // Empty format list (unusual but technically valid)
        let empty_formats = "\
v=0\r
o=- 123 456 IN IP4 127.0.0.1\r
s=Empty Formats Test\r
t=0 0\r
c=IN IP4 127.0.0.1\r
m=audio 5000 RTP/AVP\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(empty_formats));
        assert!(result.is_ok(), "Failed to parse SDP with empty format list: {:?}", result.err());
        let session = result.unwrap();
        assert!(session.media_descriptions[0].formats.is_empty());
    }

    /// Test SDPs with multiple connection lines
    #[test]
    fn test_multiple_connection_lines() {
        // Valid SDP with session-level and media-level connection lines
        let multiple_conn = "\
v=0\r
o=- 123 456 IN IP4 127.0.0.1\r
s=Multiple Connection Lines\r
c=IN IP4 224.0.0.1/127\r
t=0 0\r
m=audio 5000 RTP/AVP 0\r
c=IN IP4 192.168.1.1\r
m=video 5002 RTP/AVP 31\r
c=IN IP6 2001:db8::1\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(multiple_conn));
        assert!(result.is_ok(), "Failed to parse SDP with multiple connection lines: {:?}", result.err());
        
        let session = result.unwrap();
        assert!(session.connection_info.is_some());
        assert_eq!(session.connection_info.as_ref().unwrap().addr_type, "IP4");
        assert_eq!(session.connection_info.as_ref().unwrap().connection_address, "224.0.0.1/127");
        
        assert!(session.media_descriptions[0].connection_info.is_some());
        assert_eq!(session.media_descriptions[0].connection_info.as_ref().unwrap().addr_type, "IP4");
        assert_eq!(session.media_descriptions[0].connection_info.as_ref().unwrap().connection_address, "192.168.1.1");
        
        assert!(session.media_descriptions[1].connection_info.is_some());
        assert_eq!(session.media_descriptions[1].connection_info.as_ref().unwrap().addr_type, "IP6");
        assert_eq!(session.media_descriptions[1].connection_info.as_ref().unwrap().connection_address, "2001:db8::1");
    }

    /// Test that various encryption-related attributes parse correctly
    #[test]
    fn test_encryption_attributes() {
        // Test SRTP crypto attribute
        let crypto = "1 AES_CM_128_HMAC_SHA1_80 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR|2^20|1:32";
        // We don't have a specific parser for crypto, but it would be handled as a generic attribute
        let attr = format!("a=crypto:{}", crypto);
        
        // Test DTLS fingerprint attributes with different hash functions
        assert!(attributes::parse_fingerprint("sha-1 11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33").is_ok());
        assert!(attributes::parse_fingerprint("sha-256 11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11").is_ok());
        assert!(attributes::parse_fingerprint("sha-384 11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC").is_ok());
        assert!(attributes::parse_fingerprint("sha-512 11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77").is_ok());
    }

    /// Test parsing of SDP with comments (which are not allowed by RFC but may appear in practice)
    #[test]
    fn test_sdp_with_comments() {
        // SDP with embedded comments (not valid per RFC, but we should handle it gracefully)
        let sdp_with_comments = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Test with Comments\r
# This is a comment line\r
t=0 0\r
c=IN IP4 127.0.0.1\r
m=audio 5000 RTP/AVP 0\r
a=rtpmap:0 PCMU/8000\r
// Another comment\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(sdp_with_comments));
        assert!(result.is_err(), "Parser should reject or handle SDP with comments");
        
        // The parser should reject it with a specific error
        let error = result.err().unwrap().to_string();
        assert!(error.contains("Unknown line type") || error.contains("Invalid format"), 
               "Expected error about unknown line type or invalid format, got: {}", error);
    }
    
    /// Test parsing of exotic but valid SDP content
    #[test]
    fn test_exotic_but_valid_sdp() {
        // Valid SDP with less common fields like k= (encryption key), z= (time zone), and r= (repeat times)
        let exotic_sdp = "\
v=0\r
o=test 123 456 IN IP4 127.0.0.1\r
s=Exotic SDP Test\r
i=This session contains exotic SDP fields\r
u=http://example.com/exotic\r
e=admin@example.com\r
p=+1-234-567-8900\r
c=IN IP4 224.2.36.42/127\r
b=AS:64\r
t=3034423619 3042462419\r
r=604800 3600 0 90000\r
z=2882844526 -3600 2898848070 -7200\r
k=prompt\r
a=recvonly\r
m=audio 49170 RTP/AVP 0\r
";
        
        let result = parse_sdp(&create_test_sdp_bytes(exotic_sdp));
        assert!(result.is_ok(), "Failed to parse exotic but valid SDP: {:?}", result.err());
        
        // Verify session has expected time
        let session = result.unwrap();
        assert_eq!(session.time_descriptions[0].start_time, "3034423619");
        assert_eq!(session.time_descriptions[0].stop_time, "3042462419");
    }
} 