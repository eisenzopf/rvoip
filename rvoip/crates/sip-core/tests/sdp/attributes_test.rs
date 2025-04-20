// Tests for SDP attribute parsing logic in sdp/attributes.rs

// use rvoip_sip_core::error::SipError; // Commented out - likely not public
use rvoip_sip_core::sdp::attributes::{parse_rtpmap, parse_fmtp, parse_ptime, parse_direction, MediaDirection, parse_candidate, parse_ssrc};
use rvoip_sip_core::types::sdp::{RtpMapAttribute, FmtpAttribute, CandidateAttribute, SsrcAttribute, ParsedAttribute};
use std::str::FromStr;

#[test]
fn test_parse_rtpmap_attribute() {
    /// Test parsing a=rtpmap lines (RFC 4566 Section 6)
    let value1 = "0 PCMU/8000";
    let expected1 = RtpMapAttribute {
        payload_type: 0, encoding_name: "PCMU".to_string(), clock_rate: 8000, encoding_params: None 
    };
    assert_eq!(parse_rtpmap(value1).unwrap(), expected1);
    
    let value2 = "8 PCMA/8000/1";
     let expected2 = RtpMapAttribute {
        payload_type: 8, encoding_name: "PCMA".to_string(), clock_rate: 8000, encoding_params: Some("1".to_string()) 
    };
    assert_eq!(parse_rtpmap(value2).unwrap(), expected2);

    let value3 = "96 H264/90000";
    let result3 = parse_rtpmap(value3);
    assert!(result3.is_ok());
    let attr3 = result3.unwrap();
    assert_eq!(attr3.payload_type, 96);
    assert_eq!(attr3.encoding_name, "H264");
    assert_eq!(attr3.clock_rate, 90000);
    assert!(attr3.encoding_params.is_none());

    // Failure cases
    assert!(parse_rtpmap("PCMU/8000").is_err()); // Missing payload type
    assert!(parse_rtpmap("0 PCMU").is_err()); // Missing clock rate
    assert!(parse_rtpmap("0 PCMU/badrate").is_err()); // Invalid clock rate
    assert!(parse_rtpmap("badtype PCMU/8000").is_err()); // Invalid payload type
}

#[test]
fn test_parse_fmtp_attribute() {
    /// Test parsing a=fmtp lines (RFC 4566 Section 6)
    let value1 = "97 profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1";
    let expected1 = FmtpAttribute {
        format: "97".to_string(),
        parameters: "profile-level-id=42e01f;level-asymmetry-allowed=1;packetization-mode=1".to_string()
    };
    assert_eq!(parse_fmtp(value1).unwrap(), expected1);

    let value2 = "101 0-15"; // Example for telephone-event
    let result2 = parse_fmtp(value2);
    assert!(result2.is_ok());
    let attr2 = result2.unwrap();
    assert_eq!(attr2.format, "101");
    assert_eq!(attr2.parameters, "0-15");

    // Failure cases
    assert!(parse_fmtp("97").is_err()); // Missing parameters part
}

#[test]
fn test_parse_ptime_attribute() {
    /// Test parsing a=ptime lines (RFC 4566 Section 6)
    // Note: parser returns u32 directly, not ParsedAttribute
    assert_eq!(parse_ptime("20").unwrap(), 20);
    assert_eq!(parse_ptime(" 10 ").unwrap(), 10);
    assert!(parse_ptime("abc").is_err());
    assert!(parse_ptime("-5").is_err());
    assert!(parse_ptime("").is_err());
}

#[test]
fn test_parse_direction_attribute() {
    /// Test parsing direction attributes (RFC 4566 Section 6)
    // Note: parser returns MediaDirection directly, not ParsedAttribute
    assert_eq!(parse_direction("sendrecv").unwrap(), MediaDirection::SendRecv);
    assert_eq!(parse_direction(" recvonly\t").unwrap(), MediaDirection::RecvOnly);
    assert_eq!(parse_direction("sendonly").unwrap(), MediaDirection::SendOnly);
    assert_eq!(parse_direction("inactive").unwrap(), MediaDirection::Inactive);
    assert!(parse_direction("send receive").is_err());
    assert!(parse_direction("").is_err());
}

#[test]
fn test_parse_candidate_attribute() {
    /// Test parsing a=candidate lines (RFC 8839)
    let val1 = "foundation 1 udp 2122260223 192.168.1.100 8998 typ host generation 0 ufrag abcdefgh network-cost 10";
    let res1 = parse_candidate(val1);
    assert!(res1.is_ok());
    if let Ok(ParsedAttribute::Candidate(cand1)) = res1 {
        assert_eq!(cand1.foundation, "foundation");
        assert_eq!(cand1.component_id, 1);
        assert_eq!(cand1.transport, "udp");
        assert_eq!(cand1.priority, 2122260223);
        assert_eq!(cand1.connection_address, "192.168.1.100");
        assert_eq!(cand1.port, 8998);
        assert_eq!(cand1.candidate_type, "host");
        assert!(cand1.related_address.is_none());
        assert!(cand1.related_port.is_none());
        assert_eq!(cand1.extensions.len(), 3); 
        assert_eq!(cand1.extensions[0], ("generation".to_string(), Some("0".to_string())));
        assert_eq!(cand1.extensions[1], ("ufrag".to_string(), Some("abcdefgh".to_string())));
        assert_eq!(cand1.extensions[2], ("network-cost".to_string(), Some("10".to_string())));
    } else {
        panic!("Parsed as wrong variant or failed: {:?}", res1);
    }

    let val2 = "foundation 2 tcp 1845501695 10.0.1.5 9 typ srflx raddr 198.51.100.1 rport 8999 generation 0";
     let res2 = parse_candidate(val2);
    assert!(res2.is_ok());
     if let Ok(ParsedAttribute::Candidate(cand2)) = res2 {
        assert_eq!(cand2.candidate_type, "srflx");
        assert_eq!(cand2.related_address, Some("198.51.100.1".to_string()));
        assert_eq!(cand2.related_port, Some(8999));
        assert_eq!(cand2.extensions.len(), 1);
        assert_eq!(cand2.extensions[0], ("generation".to_string(), Some("0".to_string())));
     } else {
        panic!("Parsed as wrong variant or failed: {:?}", res2);
     }
     
     // Failure cases
     assert!(parse_candidate("foundation 1 udp 2122260223 192.168.1.100 8998 host").is_err()); // Missing 'typ'
     assert!(parse_candidate("foundation bad_id udp 2122260223 192.168.1.100 8998 typ host").is_err()); // Bad component id
     assert!(parse_candidate("foundation 1 udp bad_prio 192.168.1.100 8998 typ host").is_err()); // Bad priority
     assert!(parse_candidate("foundation 1 udp 2122260223 192.168.1.100 bad_port typ host").is_err()); // Bad port
     assert!(parse_candidate("foundation 1 udp 2122260223 192.168.1.100 8998 typ host raddr").is_err()); // raddr missing value
     assert!(parse_candidate("foundation 1 udp 2122260223 192.168.1.100 8998 typ host rport bad").is_err()); // rport bad value

}

#[test]
fn test_parse_ssrc_attribute() {
    /// Test parsing a=ssrc lines (RFC 5576)
    let val1 = "123456789 cname:user@example.com";
    let res1 = parse_ssrc(val1);
    assert!(res1.is_ok());
    if let Ok(ParsedAttribute::Ssrc(ssrc1)) = res1 {
        assert_eq!(ssrc1.ssrc_id, 123456789);
        assert_eq!(ssrc1.attribute, "cname");
        assert_eq!(ssrc1.value, Some("user@example.com".to_string()));
    } else {
        panic!("Parsed as wrong variant or failed: {:?}", res1);
    }

    let val2 = "987654321 msid:stream1 track1"; // Value contains space
    let res2 = parse_ssrc(val2);
     assert!(res2.is_ok());
    if let Ok(ParsedAttribute::Ssrc(ssrc2)) = res2 {
         assert_eq!(ssrc2.ssrc_id, 987654321);
         assert_eq!(ssrc2.attribute, "msid");
         assert_eq!(ssrc2.value, Some("stream1 track1".to_string())); 
    } else {
        panic!("Parsed as wrong variant or failed: {:?}", res2);
    }
    
    let val3 = "111 mslabel:label1"; // Attribute with value after space
    let res3 = parse_ssrc(val3);
     assert!(res3.is_ok());
    if let Ok(ParsedAttribute::Ssrc(ssrc3)) = res3 {
         assert_eq!(ssrc3.ssrc_id, 111);
         assert_eq!(ssrc3.attribute, "mslabel");
         assert_eq!(ssrc3.value, Some("label1".to_string()));
    } else {
        panic!("Parsed as wrong variant or failed: {:?}", res3);
    }
    
    let val4 = "222 label"; // Attribute without value part (RFC 5576 allows this for some attributes like cname, msid, etc)
    let res4 = parse_ssrc(val4);
    assert!(res4.is_ok());
    if let Ok(ParsedAttribute::Ssrc(ssrc4)) = res4 {
        assert_eq!(ssrc4.ssrc_id, 222);
        assert_eq!(ssrc4.attribute, "label");
        assert_eq!(ssrc4.value, None); // Now expects None if no :value
    } else {
        panic!("Parsed as wrong variant or failed: {:?}", res4);
    }

    // Failure cases
    assert!(parse_ssrc("badid cname:test").is_err());
    assert!(parse_ssrc("12345").is_err()); // Missing attribute part
    assert!(parse_ssrc("12345 cname:").is_ok()); // Empty value after colon is ok
    if let Ok(ParsedAttribute::Ssrc(ssrc5)) = parse_ssrc("12345 cname:") {
         assert_eq!(ssrc5.value, Some("".to_string()));
    } else {
         panic!("Parsing 'cname:' failed unexpectedly");
    }
     assert!(parse_ssrc("12345 ").is_err()); // Empty attribute name
}

// Add tests for other attribute parsers when implemented 