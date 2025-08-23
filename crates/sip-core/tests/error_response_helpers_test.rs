//! Tests for error response helper methods

use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::types::Method;

#[test]
fn test_unauthorized_helper() {
    let response = SimpleResponseBuilder::unauthorized()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Register)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .www_authenticate_digest("example.com", "abc123xyz")
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("401 Unauthorized"));
    assert!(response_str.contains("WWW-Authenticate: Digest"));
}

#[test]
fn test_forbidden_helper() {
    let response = SimpleResponseBuilder::forbidden()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@blocked.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Subscribe)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("403 Forbidden"));
}

#[test]
fn test_interval_too_brief_helper() {
    let response = SimpleResponseBuilder::interval_too_brief()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Subscribe)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .min_expires(3600)
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("423 Interval Too Brief"));
    assert!(response_str.contains("Min-Expires: 3600"));
}

#[test]
fn test_bad_event_helper() {
    let response = SimpleResponseBuilder::bad_event()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Subscribe)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .allow_events(&["presence", "dialog"])
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("489 Bad Event"));
    assert!(response_str.contains("Allow-Events: presence, dialog"));
}

#[test]
fn test_bad_request_helper() {
    // Test the existing bad_request helper
    let response = SimpleResponseBuilder::bad_request()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Publish)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("400 Bad Request"));
}

#[test]
fn test_not_found_helper() {
    // Test the existing not_found helper
    let response = SimpleResponseBuilder::not_found()
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:unknown@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Subscribe)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("404 Not Found"));
}