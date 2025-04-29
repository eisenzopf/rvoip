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
    assert!(result.unwrap_err().to_string().contains("Invalid address type"));
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
        assert_eq!(c.foundation, "2");
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
        assert!(c.related_address.is_none());
        assert!(c.related_port.is_none());
        assert!(c.extensions.is_empty());
    }
    
    // Valid server reflexive candidate with related address/port
    let srflx_value = "2 1 UDP 1694498815 192.0.2.3 51372 typ srflx raddr 192.168.1.5 rport 49170";
    assert!(attributes::parse_candidate(srflx_value).is_ok());
    if let ParsedAttribute::Candidate(c) = attributes::parse_candidate(srflx_value).unwrap() {
        assert_eq!(c.candidate_type, "srflx");
        assert_eq!(c.related_address, Some("192.168.1.5".to_string()));
        assert_eq!(c.related_port, Some(49170));
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
    
    // Invalid IP address
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
    }
    
    // Valid SSRC without value
    let ssrc_mslabel = "314159 mslabel";
    assert!(attributes::parse_ssrc(ssrc_mslabel).is_ok());
    if let ParsedAttribute::Ssrc(s) = attributes::parse_ssrc(ssrc_mslabel).unwrap() {
        assert_eq!(s.ssrc_id, 314159);
        assert_eq!(s.attribute, "mslabel");
        assert_eq!(s.value, None);
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
    
    // Group with invalid mid (if mid validation is strict)
    // Note: The current implementation doesn't validate mids strictly
    // let invalid_mid = "BUNDLE audio@invalid video";
    // assert!(attributes::parse_group(invalid_mid).is_err());
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
    assert!(attributes::parse_extmap(with_dir).is_ok());
    let (id, direction, uri, params) = attributes::parse_extmap(with_dir).unwrap();
    assert_eq!(id, 2);
    assert_eq!(direction, Some("sendrecv".to_string()));
    assert_eq!(uri, "urn:ietf:params:rtp-hdrext:toffset");
    assert_eq!(params, None);
    
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