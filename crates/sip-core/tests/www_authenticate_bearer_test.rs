//! Tests for WWW-Authenticate Bearer challenge support

use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::types::{Method, StatusCode};

#[test]
fn test_www_authenticate_digest() {
    let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Invite)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .www_authenticate_digest("example.com", "abc123xyz")
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("401 Unauthorized"));
    assert!(response_str.contains("WWW-Authenticate: Digest"));
    assert!(response_str.contains("realm=\"example.com\""));
    assert!(response_str.contains("nonce=\"abc123xyz\""));
}

#[test]
fn test_www_authenticate_bearer() {
    let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Register)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .www_authenticate_bearer("example.com")
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("401 Unauthorized"));
    assert!(response_str.contains("WWW-Authenticate: Bearer realm=\"example.com\""));
}

#[test]
fn test_www_authenticate_bearer_with_error() {
    let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Register)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .www_authenticate_bearer_error("example.com", "invalid_token", Some("The access token has expired"))
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("401 Unauthorized"));
    assert!(response_str.contains("WWW-Authenticate: Bearer"));
    assert!(response_str.contains("realm=\"example.com\""));
    assert!(response_str.contains("error=\"invalid_token\""));
    assert!(response_str.contains("error_description=\"The access token has expired\""));
}

#[test]
fn test_unauthorized_response_helper() {
    // Test that we can create an unauthorized response with Bearer auth
    let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Publish)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .www_authenticate_bearer("presence.example.com")
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("401 Unauthorized"));
    assert!(response_str.contains("WWW-Authenticate: Bearer realm=\"presence.example.com\""));
}

#[test]
fn test_multiple_challenges() {
    // Test that we can add multiple challenges using the generic header method
    use rvoip_sip_core::types::TypedHeader;
    use rvoip_sip_core::types::auth::{WwwAuthenticate, Challenge, DigestParam};
    
    // Create a WWW-Authenticate with multiple challenges
    let mut www_auth = WwwAuthenticate::new_bearer("example.com");
    www_auth.add_challenge(Challenge::Digest { 
        params: vec![
            DigestParam::Realm("example.com".to_string()),
            DigestParam::Nonce("xyz987".to_string()),
        ]
    });
    
    let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
        .from("Alice", "sip:alice@example.com", Some("1928301774"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("test-call-id")
        .cseq(1, Method::Subscribe)
        .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
        .header(TypedHeader::WwwAuthenticate(www_auth))
        .build();
    
    let response_str = response.to_string();
    assert!(response_str.contains("401 Unauthorized"));
    // Should contain both Bearer and Digest challenges
    assert!(response_str.contains("Bearer realm=\"example.com\""));
    assert!(response_str.contains("Digest"));
}