//! Cross-crate integration tests for SIP call flow.
//!
//! Tests the full path: sip-core message building -> serialization -> parsing -> verification.
//! Validates that SIP messages survive a serialize/parse round-trip and that
//! INVITE requests with SDP bodies are correctly constructed.

use bytes::Bytes;

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::Method;
use rvoip_sip_core::{parse_message, Message, TypedHeader, ContentLength};

// =============================================================================
// Test 1: SIP INVITE request round-trip (serialize -> parse -> verify)
// =============================================================================

#[test]
fn test_sip_invite_round_trip_serialization() {
    let from_tag = "ftag-roundtrip-1";
    let call_id = "roundtrip-test-001@example.com";
    let branch = "z9hG4bK-roundtrip-branch";

    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@192.168.1.200:5060")
        .expect("valid URI")
        .from("Alice", "sip:alice@192.168.1.100:5060", Some(from_tag))
        .to("Bob", "sip:bob@192.168.1.200:5060", None)
        .call_id(call_id)
        .cseq(1)
        .via("192.168.1.100:5060", "UDP", Some(branch))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    // Serialize to bytes
    let message: Message = request.into();
    let serialized = message.to_bytes();

    // Parse back from bytes
    let parsed = parse_message(&serialized).expect("should parse serialized SIP message");

    // Verify it's a request
    let parsed_request = match parsed {
        Message::Request(r) => r,
        Message::Response(_) => panic!("Expected a Request after round-trip, got Response"),
    };

    // Verify method
    assert_eq!(parsed_request.method(), Method::Invite, "Method should survive round-trip");

    // Verify Call-ID
    let parsed_call_id = parsed_request.call_id().expect("should have Call-ID");
    assert_eq!(parsed_call_id.to_string(), call_id, "Call-ID should survive round-trip");

    // Verify From header contains the tag
    let from = parsed_request.from().expect("should have From header");
    let from_str = from.to_string();
    assert!(from_str.contains(from_tag), "From tag should survive round-trip");

    // Verify Via header contains the branch
    let vias = parsed_request.via_headers();
    assert!(!vias.is_empty(), "Via headers should survive round-trip");
    let via_str = format!("{}", vias[0]);
    assert!(via_str.contains(branch), "Via branch should survive round-trip");
}

// =============================================================================
// Test 2: SIP REGISTER request round-trip
// =============================================================================

#[test]
fn test_sip_register_round_trip() {
    let from_tag = "reg-tag-42";
    let call_id = "register-round-trip@example.com";
    let branch = "z9hG4bK-register-branch";

    let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some(from_tag))
        .to("Alice", "sip:alice@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .via("192.168.1.100:5060", "UDP", Some(branch))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    let message: Message = request.into();
    let serialized = message.to_bytes();
    let parsed = parse_message(&serialized).expect("should parse REGISTER");

    let parsed_request = match parsed {
        Message::Request(r) => r,
        Message::Response(_) => panic!("Expected Request, got Response"),
    };

    assert_eq!(parsed_request.method(), Method::Register);

    // Verify CSeq
    let cseq = parsed_request.cseq().expect("should have CSeq");
    assert_eq!(cseq.sequence(), 1, "CSeq sequence number should survive round-trip");
}

// =============================================================================
// Test 3: INVITE request with SDP body
// =============================================================================

#[test]
fn test_invite_with_sdp_body() {
    let sdp_body = "v=0\r\n\
                    o=alice 2890844526 2890844526 IN IP4 192.168.1.100\r\n\
                    s=Test Session\r\n\
                    c=IN IP4 192.168.1.100\r\n\
                    t=0 0\r\n\
                    m=audio 49170 RTP/AVP 0 8\r\n\
                    a=rtpmap:0 PCMU/8000\r\n\
                    a=rtpmap:8 PCMA/8000\r\n";

    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@192.168.1.200:5060")
        .expect("valid URI")
        .from("Alice", "sip:alice@192.168.1.100:5060", Some("sdp-tag-1"))
        .to("Bob", "sip:bob@192.168.1.200:5060", None)
        .call_id("sdp-test-001@example.com")
        .cseq(1)
        .via("192.168.1.100:5060", "UDP", Some("z9hG4bK-sdp-branch"))
        .max_forwards(70)
        .content_type("application/sdp")
        .body(Bytes::from(sdp_body))
        .build();

    // Verify Content-Type header is set
    let message: Message = request.into();
    let serialized = message.to_bytes();
    let serialized_str = String::from_utf8_lossy(&serialized);

    assert!(
        serialized_str.contains("application/sdp"),
        "Serialized message should contain Content-Type: application/sdp"
    );

    // Verify the SDP body is present
    assert!(
        serialized_str.contains("m=audio"),
        "Serialized message should contain SDP media line"
    );
    assert!(
        serialized_str.contains("PCMU/8000"),
        "Serialized message should contain PCMU rtpmap"
    );
    assert!(
        serialized_str.contains("PCMA/8000"),
        "Serialized message should contain PCMA rtpmap"
    );

    // Parse back and verify body survives
    let parsed = parse_message(&serialized).expect("should parse INVITE with SDP");
    let parsed_body = parsed.body();
    let body_str = String::from_utf8_lossy(parsed_body);
    assert!(
        body_str.contains("m=audio"),
        "Parsed body should contain SDP media line"
    );
}

// =============================================================================
// Test 4: Multiple SIP methods serialize consistently
// =============================================================================

#[test]
fn test_multiple_methods_round_trip() {
    let methods = vec![
        (Method::Invite, "sip:bob@example.com"),
        (Method::Bye, "sip:bob@example.com"),
        (Method::Options, "sip:server@example.com"),
        (Method::Register, "sip:registrar.example.com"),
    ];

    for (method, uri) in methods {
        let request = SimpleRequestBuilder::new(method.clone(), uri)
            .expect("valid URI")
            .from("Alice", "sip:alice@example.com", Some("tag-multi"))
            .to("Bob", uri, None)
            .call_id(&format!("multi-{:?}@example.com", method))
            .cseq(1)
            .via("192.168.1.100:5060", "UDP", Some("z9hG4bK-multi"))
            .max_forwards(70)
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();

        let message: Message = request.into();
        let serialized = message.to_bytes();
        let parsed = parse_message(&serialized)
            .unwrap_or_else(|e| panic!("Failed to parse {:?} message: {}", method, e));

        match parsed {
            Message::Request(r) => {
                assert_eq!(r.method(), method, "Method {:?} should survive round-trip", method);
            }
            Message::Response(_) => {
                panic!("Expected Request for {:?}, got Response", method);
            }
        }
    }
}

// =============================================================================
// Test 5: SIP message preserves Max-Forwards through round-trip
// =============================================================================

#[test]
fn test_max_forwards_preserved() {
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("mf-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("max-fwd-test@example.com")
        .cseq(1)
        .via("192.168.1.100:5060", "UDP", Some("z9hG4bK-mf"))
        .max_forwards(42)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    let message: Message = request.into();
    let serialized = message.to_bytes();
    let serialized_str = String::from_utf8_lossy(&serialized);

    // Max-Forwards should be present in serialized form
    assert!(
        serialized_str.contains("Max-Forwards: 42") || serialized_str.contains("Max-forwards: 42"),
        "Serialized message should contain Max-Forwards: 42, got:\n{}",
        serialized_str
    );

    let parsed = parse_message(&serialized).expect("should parse");
    match parsed {
        Message::Request(r) => {
            // Re-serialize the parsed message and verify Max-Forwards is still present
            let reserialized = Message::Request(r).to_bytes();
            let reser_str = String::from_utf8_lossy(&reserialized);
            assert!(
                reser_str.contains("Max-Forwards: 42") || reser_str.contains("Max-forwards: 42"),
                "Max-Forwards: 42 should survive full round-trip, got:\n{}",
                reser_str
            );
        }
        _ => panic!("Expected Request"),
    }
}
