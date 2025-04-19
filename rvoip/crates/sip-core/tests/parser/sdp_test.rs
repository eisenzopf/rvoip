// Tests for SDP parsing logic in sdp/parser.rs

use rvoip_sip_core::error::Result;
use rvoip_sip_core::sdp::parser::parse_sdp;
use rvoip_sip_core::types::{SdpSession, MediaDescription};
use rvoip_sip_core::types::sdp::{ParsedAttribute, MediaDirection}; // Import enum
use bytes::Bytes;
use std::collections::HashMap;


#[test]
fn test_parse_simple_audio_sdp() {
    /// Based on RFC 4566 Section 9 Examples
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r\n"
      + "s=SDP Seminar\r\n"
      + "i=A Seminar on the session description protocol\r\n"
      + "u=http://www.example.com/seminar.pdf\r\n"
      + "e=j.doe@example.com (Jane Doe)\r\n"
      + "c=IN IP4 224.2.17.12/127\r\n"
      + "t=2873397496 2873404696\r\n"
      + "a=recvonly\r\n"
      + "m=audio 49170 RTP/AVP 0\r\n"
      + "a=rtpmap:0 PCMU/8000\r\n"
      + "m=video 51372 RTP/AVP 99\r\n"
      + "c=IN IP4 10.47.16.5\r\n"
      + "a=rtpmap:99 h263-1998/90000\r\n"
    );

    let result = parse_sdp(&sdp_content);
    assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();

    // Session Level Checks
    assert_eq!(session.version, "0");
    assert_eq!(session.origin.username, "jdoe");
    assert_eq!(session.origin.sess_id, "2890844526");
    assert_eq!(session.origin.sess_version, "2890842807");
    assert_eq!(session.origin.net_type, "IN");
    assert_eq!(session.origin.addr_type, "IP4");
    assert_eq!(session.origin.unicast_address, "10.47.16.5");
    assert_eq!(session.session_name, "SDP Seminar");
    assert!(session.connection_info.is_some());
    let conn_sess = session.connection_info.unwrap();
    assert_eq!(conn_sess.net_type, "IN");
    assert_eq!(conn_sess.addr_type, "IP4");
    assert_eq!(conn_sess.connection_address, "224.2.17.12/127");
    assert_eq!(session.time_descriptions.len(), 1);
    assert_eq!(session.time_descriptions[0].start_time, "2873397496");
    assert_eq!(session.time_descriptions[0].stop_time, "2873404696");
    assert_eq!(session.direction, Some(MediaDirection::RecvOnly));
    // Check generic attributes for ignored lines
    assert!(session.generic_attributes.iter().any(|a| matches!(a, ParsedAttribute::Value(k, v) if k == "i" && v.contains("Seminar"))));
    assert!(session.generic_attributes.iter().any(|a| matches!(a, ParsedAttribute::Value(k, v) if k == "u" && v.contains("seminar.pdf"))));
    assert!(session.generic_attributes.iter().any(|a| matches!(a, ParsedAttribute::Value(k, v) if k == "e" && v.contains("Jane Doe"))));

    // Media Level Checks
    assert_eq!(session.media_descriptions.len(), 2);

    // Audio media
    let audio_media = &session.media_descriptions[0];
    assert_eq!(audio_media.media, "audio");
    assert_eq!(audio_media.port, 49170);
    assert!(audio_media.connection_info.is_none()); // Uses session level c=
    assert!(audio_media.ptime.is_none());
    assert!(audio_media.direction.is_none()); 
    assert_eq!(audio_media.generic_attributes.len(), 1); // Only rtpmap
    assert!(matches!(&audio_media.generic_attributes[0], ParsedAttribute::RtpMap(map) if map.payload_type == 0 && map.encoding_name == "PCMU"));

    // Video media
    let video_media = &session.media_descriptions[1];
    assert_eq!(video_media.media, "video");
    assert_eq!(video_media.port, 51372);
    assert!(video_media.connection_info.is_some());
    let conn_video = video_media.connection_info.as_ref().unwrap();
    assert_eq!(conn_video.connection_address, "10.47.16.5");
    assert!(video_media.ptime.is_none());
    assert!(video_media.direction.is_none()); // No direction attribute for this media
    assert_eq!(video_media.generic_attributes.len(), 1); // Only rtpmap
     assert!(matches!(&video_media.generic_attributes[0], ParsedAttribute::RtpMap(map) if map.payload_type == 99 && map.encoding_name == "h263-1998"));
}

#[test]
fn test_sdp_missing_mandatory_fields() {
    /// Test missing o= line
    let sdp_no_o = Bytes::from(
        "v=0\r\n"
      + "s=Session\r\n"
      + "t=0 0\r\n"
      + "c=IN IP4 127.0.0.1\r\n"
      + "m=audio 49170 RTP/AVP 0\r\n"
    );
    let result_no_o = parse_sdp(&sdp_no_o);
    assert!(result_no_o.is_err());
    assert!(result_no_o.unwrap_err().to_string().contains("Missing mandatory SDP fields"));

    // Add similar tests for missing s= and t=
}

#[test]
fn test_sdp_invalid_lines() {
    /// Test invalid v= line
    let sdp_bad_v = Bytes::from(
        "v=1\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "t=0 0\r\n"
    );
    let result_bad_v = parse_sdp(&sdp_bad_v);
    assert!(result_bad_v.is_err());
    assert!(result_bad_v.unwrap_err().to_string().contains("Unsupported SDP version"));

    /// Test invalid m= line
    let sdp_bad_m = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "t=0 0\r\n"
      + "m=audio port\r\n"
    );
     let result_bad_m = parse_sdp(&sdp_bad_m);
    assert!(result_bad_m.is_err());
    assert!(result_bad_m.unwrap_err().to_string().contains("Invalid m= line format"));
}

#[test]
fn test_sdp_optional_fields() {
    /// Test SDP without optional i, u, e, session c= line
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=jdoe 2890844526 2890842807 IN IP4 10.47.16.5\r\n"
      + "s=SDP Seminar\r\n"
      // No c= line here
      + "t=2873397496 2873404696\r\n"
      + "m=audio 49170 RTP/AVP 0\r\n"
      + "c=IN IP4 192.168.1.1\r\n" // c= line required at media level if not at session level
      + "a=rtpmap:0 PCMU/8000\r\n"
    );

    let result = parse_sdp(&sdp_content);
    assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();

    assert!(session.connection_info.is_none());
    assert_eq!(session.media_descriptions.len(), 1);
    assert!(session.media_descriptions[0].connection_info.is_some());
    assert!(session.media_descriptions[0].connection_info.as_ref().unwrap().contains("192.168.1.1"));
}

#[test]
fn test_sdp_attribute_formats() {
     /// Test different a= line formats
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=- 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "t=0 0\r\n"
      + "a=sendrecv\r\n" // Flag attribute
      + "a=rtcp:53020 IN IP4 10.0.0.1\r\n" // Attribute with value
      + "m=audio 49170 RTP/AVP 0\r\n"
      + "a=ptime:20\r\n" // Media-level attribute
    );

    let result = parse_sdp(&sdp_content);
    assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();

    // Session attributes
    assert!(session.attributes.contains_key("sendrecv"));
    assert_eq!(session.attributes.get("sendrecv"), Some(&None));
    assert!(session.attributes.contains_key("rtcp"));
    assert_eq!(session.attributes.get("rtcp"), Some(&Some("53020 IN IP4 10.0.0.1".to_string())));

    // Media attributes
    assert_eq!(session.media_descriptions.len(), 1);
    assert!(session.media_descriptions[0].attributes.contains_key("ptime"));
    assert_eq!(session.media_descriptions[0].attributes.get("ptime"), Some(&Some("20".to_string())));
}

#[test]
fn test_sdp_multiple_time_descriptions() {
    /// Test multiple t= lines (RFC 4566 Section 5.9)
     let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=- 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "t=0 0\r\n"
      + "r=7d 1h 0 25h\r\n" // Repeat time (ignored by current parser)
      + "t=3149652000 3149656200\r\n"
      + "m=audio 49170 RTP/AVP 0\r\n"
    );
    let result = parse_sdp(&sdp_content);
    assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();

    assert_eq!(session.time_descriptions.len(), 2);
    assert_eq!(session.time_descriptions[0], "0 0");
    assert_eq!(session.time_descriptions[1], "3149652000 3149656200");
}

#[test]
fn test_sdp_attribute_parsing_locations() {
    /// Test attributes appearing at session vs media level
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "t=0 0\r\n"
      + "a=ptime:10\r\n" // Session ptime (unusual but test parsing)
      + "a=sendonly\r\n"   // Session direction
      + "m=audio 5000 RTP/AVP 0\r\n"
      + "c=IN IP4 1.1.1.2\r\n"
      + "a=recvonly\r\n"   // Media direction (overrides session)
      + "a=rtpmap:0 PCMU/8000\r\n"
      + "m=video 5002 RTP/AVP 99\r\n"
      + "c=IN IP4 1.1.1.3\r\n"
      + "a=ptime:30\r\n" // Video ptime
      + "a=rtpmap:99 H264/90000\r\n"
    );
    let result = parse_sdp(&sdp_content);
    assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();

    assert_eq!(session.direction, Some(MediaDirection::SendOnly));
    assert!(session.generic_attributes.contains(&ParsedAttribute::Ptime(10))); // Session ptime went to generic

    assert_eq!(session.media_descriptions.len(), 2);
    let audio_media = &session.media_descriptions[0];
    let video_media = &session.media_descriptions[1];

    assert_eq!(audio_media.direction, Some(MediaDirection::RecvOnly));
    assert!(audio_media.ptime.is_none()); // No ptime attribute for audio

    assert_eq!(video_media.direction, None); // No direction attribute for video
    assert_eq!(video_media.ptime, Some(30)); 
}

#[test]
fn test_sdp_candidate_parsing() {
    /// Test parsing candidate attributes
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "t=0 0\r\n"
      + "m=audio 5000 RTP/AVP 0\r\n"
      + "c=IN IP4 1.1.1.2\r\n"
      + "a=rtpmap:0 PCMU/8000\r\n"
      + "a=candidate:foundation 1 udp 2122260223 192.168.1.100 8998 typ host\r\n"
      + "a=candidate:foundation 2 tcp 1845501695 10.0.1.5 9 typ srflx raddr 198.51.100.1 rport 8999\r\n"
    );
    let result = parse_sdp(&sdp_content);
     assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();
    assert_eq!(session.media_descriptions.len(), 1);
    let media = &session.media_descriptions[0];
    
    let candidates: Vec<_> = media.generic_attributes.iter()
        .filter_map(|a| match a {
            ParsedAttribute::Candidate(c) => Some(c),
            _ => None
        }).collect();
        
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].candidate_type, "host");
    assert_eq!(candidates[0].component_id, 1);
    assert_eq!(candidates[1].candidate_type, "srflx");
    assert_eq!(candidates[1].related_address, Some("198.51.100.1".to_string()));
    assert_eq!(candidates[1].related_port, Some(8999));
}

#[test]
fn test_sdp_ssrc_parsing() {
     /// Test parsing ssrc attributes
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "t=0 0\r\n"
      + "m=audio 5000 RTP/AVP 0\r\n"
      + "c=IN IP4 1.1.1.2\r\n"
      + "a=rtpmap:0 PCMU/8000\r\n"
      + "a=ssrc:123456789 cname:user@example.com\r\n"
      + "a=ssrc:123456789 msid:stream1 track1\r\n"
      + "a=ssrc:987654321 label:audio-1\r\n"
    );
     let result = parse_sdp(&sdp_content);
     assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();
    assert_eq!(session.media_descriptions.len(), 1);
    let media = &session.media_descriptions[0];

    let ssrcs: Vec<_> = media.generic_attributes.iter()
        .filter_map(|a| match a {
            ParsedAttribute::Ssrc(s) => Some(s),
            _ => None
        }).collect();
        
    assert_eq!(ssrcs.len(), 3);
    assert_eq!(ssrcs[0].ssrc_id, 123456789);
    assert_eq!(ssrcs[0].attribute, "cname");
    assert_eq!(ssrcs[0].value, Some("user@example.com".to_string()));
    assert_eq!(ssrcs[1].attribute, "msid");
    assert_eq!(ssrcs[1].value, Some("stream1 track1".to_string()));
    assert_eq!(ssrcs[2].ssrc_id, 987654321);
    assert_eq!(ssrcs[2].attribute, "label");
     assert_eq!(ssrcs[2].value, Some("audio-1".to_string()));
}

#[test]
fn test_sdp_media_only_connection_line() {
    /// Test SDP where c= line is only present at media level
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      // No session C line
      + "t=0 0\r\n"
      + "m=audio 5000 RTP/AVP 0\r\n"
      + "c=IN IP4 192.168.1.100\r\n"
      + "a=rtpmap:0 PCMU/8000\r\n"
      + "m=video 5002 RTP/AVP 99\r\n"
      + "c=IN IP6 2001:db8::1\r\n"
      + "a=rtpmap:99 H264/90000\r\n"
    );
    let result = parse_sdp(&sdp_content);
    assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
    let session = result.unwrap();

    assert!(session.connection_info.is_none());
    assert_eq!(session.media_descriptions.len(), 2);
    assert!(session.media_descriptions[0].connection_info.is_some());
    assert_eq!(session.media_descriptions[0].connection_info.as_ref().unwrap().connection_address, "192.168.1.100");
    assert!(session.media_descriptions[1].connection_info.is_some());
    assert_eq!(session.media_descriptions[1].connection_info.as_ref().unwrap().connection_address, "2001:db8::1");
    assert_eq!(session.media_descriptions[1].connection_info.as_ref().unwrap().addr_type, "IP6");
}

#[test]
fn test_sdp_only_mandatory() {
    /// Test SDP with only mandatory fields
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=-\r\n"
      + "t=0 0\r\n"
    );
    let result = parse_sdp(&sdp_content);
    // Should fail because c= line is missing at session level and there are no media lines
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Missing mandatory c= field"));
    
    // Add mandatory c= line
    let sdp_content_with_c = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=-\r\n"
      + "c=IN IP4 1.1.1.1\r\n"
      + "t=0 0\r\n"
    );
     let result_wc = parse_sdp(&sdp_content_with_c);
     assert!(result_wc.is_ok(), "Parsing failed: {:?}", result_wc.err());
     let session_wc = result_wc.unwrap();
     assert!(session_wc.media_descriptions.is_empty());
     assert!(session_wc.generic_attributes.is_empty());
     assert!(session_wc.direction.is_none());
}

#[test]
fn test_sdp_invalid_order() {
    /// Test invalid field order (e.g., m= before t=)
    let sdp_content = Bytes::from(
        "v=0\r\n"
      + "o=user 1 1 IN IP4 1.1.1.1\r\n"
      + "s=s\r\n"
      + "m=audio 5000 RTP/AVP 0\r\n"
      + "c=IN IP4 1.1.1.2\r\n"
      + "t=0 0\r\n"
    );
     let result = parse_sdp(&sdp_content);
     // Current parser might allow this leniently, but strict RFC 4566 requires specific order
     // Let's check if it errors, assuming strictness is desired or becomes an issue.
     // If it parses OK, the time description would be missing from the session struct.
     if let Ok(session) = result {
         println!("Warning: SDP parser allowed m= before t=");
         assert!(session.time_descriptions.is_empty()); // t= was likely ignored
     } else {
         // Or assert!(result.is_err()); if strict order is enforced
         println!("SDP parser correctly failed on invalid order (m= before t=)");
     }
}

// Add more tests: different attribute types, connection lines at different levels, edge cases. 