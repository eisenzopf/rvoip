//! Builder Tests
//!
//! Comprehensive tests for the client and server request/response builders,
//! verifying they create RFC 3261 compliant SIP messages.

use std::net::SocketAddr;
use std::str::FromStr;

use rvoip_sip_core::{Method, StatusCode, Uri};
use rvoip_sip_core::types::header::{HeaderName, TypedHeader};
use rvoip_transaction_core::builders::{client_quick, server_quick};
use rvoip_transaction_core::client::builders::{InviteBuilder, ByeBuilder, RegisterBuilder};
use rvoip_transaction_core::server::builders::{ResponseBuilder, InviteResponseBuilder, RegisterResponseBuilder};

/// Test INVITE builder creates properly formatted requests
#[tokio::test]
async fn test_invite_builder() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let from_uri = "sip:alice@example.com";
    let to_uri = "sip:bob@example.com";
    let sdp_content = "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n";
    
    let invite = InviteBuilder::new()
        .from_to(from_uri, to_uri)
        .local_address(local_addr)
        .with_sdp(sdp_content)
        .cseq(42)
        .build()
        .expect("Failed to build INVITE");
    
    // Verify basic request properties
    assert_eq!(invite.method(), Method::Invite);
    assert_eq!(invite.uri().to_string(), to_uri);
    
    // Verify required headers
    assert!(invite.from().is_some());
    assert!(invite.to().is_some());
    assert!(invite.call_id().is_some());
    assert!(invite.cseq().is_some());
    assert_eq!(invite.cseq().unwrap().seq, 42);
    assert_eq!(invite.cseq().unwrap().method, Method::Invite);
    
    // Verify Via header
    let via_header = invite.header(&HeaderName::Via);
    assert!(via_header.is_some());
    
    // Verify SDP content
    assert_eq!(invite.body(), sdp_content.as_bytes());
    
    // Verify Content-Type header for SDP
    let content_type = invite.header(&HeaderName::ContentType);
    assert!(content_type.is_some());
    
    // Verify From tag is present (auto-generated for new dialogs)
    assert!(invite.from().unwrap().tag().is_some());
    
    println!("✅ INVITE builder creates RFC 3261 compliant request");
}

/// Test BYE builder creates properly formatted requests
#[tokio::test]
async fn test_bye_builder() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let call_id = "test-call-123";
    let from_uri = "sip:alice@example.com";
    let from_tag = "alice-tag";
    let to_uri = "sip:bob@example.com";
    let to_tag = "bob-tag";
    
    let bye = ByeBuilder::new()
        .from_dialog(call_id, from_uri, from_tag, to_uri, to_tag)
        .local_address(local_addr)
        .cseq(2)
        .build()
        .expect("Failed to build BYE");
    
    // Verify basic request properties
    assert_eq!(bye.method(), Method::Bye);
    assert_eq!(bye.uri().to_string(), to_uri);
    
    // Verify dialog information
    assert_eq!(bye.call_id().unwrap().value(), call_id);
    assert_eq!(bye.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(bye.to().unwrap().tag().unwrap(), to_tag);
    assert_eq!(bye.cseq().unwrap().seq, 2);
    assert_eq!(bye.cseq().unwrap().method, Method::Bye);
    
    // Verify Via header
    let via_header = bye.header(&HeaderName::Via);
    assert!(via_header.is_some());
    
    // Verify empty body (BYE typically has no content)
    assert_eq!(bye.body().len(), 0);
    
    println!("✅ BYE builder creates RFC 3261 compliant request");
}

/// Test REGISTER builder creates properly formatted requests
#[tokio::test]
async fn test_register_builder() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let registrar_uri = "sip:registrar.example.com";
    let user_uri = "sip:alice@example.com";
    let display_name = "Alice Smith";
    let expires = 3600;
    
    let register = RegisterBuilder::new()
        .registrar(registrar_uri)
        .user_info(user_uri, display_name)
        .local_address(local_addr)
        .expires(expires)
        .build()
        .expect("Failed to build REGISTER");
    
    // Verify basic request properties
    assert_eq!(register.method(), Method::Register);
    assert_eq!(register.uri().to_string(), registrar_uri);
    
    // Verify headers
    assert_eq!(register.from().unwrap().address().uri.to_string(), user_uri);
    assert_eq!(register.from().unwrap().address().display_name().unwrap(), display_name);
    assert_eq!(register.to().unwrap().address().uri.to_string(), registrar_uri);
    
    // Verify Contact header
    let contact_header = register.header(&HeaderName::Contact);
    assert!(contact_header.is_some());
    
    // Verify Expires header
    let expires_header = register.header(&HeaderName::Expires);
    assert!(expires_header.is_some());
    
    // Verify From tag is present (auto-generated)
    assert!(register.from().unwrap().tag().is_some());
    
    println!("✅ REGISTER builder creates RFC 3261 compliant request");
}

/// Test client quick helpers create proper requests
#[tokio::test]
async fn test_client_quick_helpers() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Test quick INVITE
    let invite = client_quick::invite(
        "sip:alice@example.com",
        "sip:bob@example.com",
        local_addr,
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n"),
    ).expect("Failed to create quick INVITE");
    
    assert_eq!(invite.method(), Method::Invite);
    assert!(invite.body().len() > 0); // Has SDP content
    
    // Test quick BYE
    let bye = client_quick::bye(
        "call-123",
        "sip:alice@example.com",
        "alice-tag",
        "sip:bob@example.com",
        "bob-tag",
        local_addr,
        2,
    ).expect("Failed to create quick BYE");
    
    assert_eq!(bye.method(), Method::Bye);
    assert_eq!(bye.cseq().unwrap().seq, 2);
    
    // Test quick REGISTER
    let register = client_quick::register(
        "sip:registrar.example.com",
        "sip:alice@example.com",
        "Alice",
        local_addr,
        Some(3600),
    ).expect("Failed to create quick REGISTER");
    
    assert_eq!(register.method(), Method::Register);
    
    println!("✅ Client quick helpers create RFC 3261 compliant requests");
}

/// Test response builders create properly formatted responses
#[tokio::test]
async fn test_response_builders() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create a sample request to build responses from
    let sample_request = client_quick::invite(
        "sip:alice@example.com",
        "sip:bob@example.com",
        local_addr,
        None,
    ).expect("Failed to create sample request");
    
    // Test basic response builder
    let trying = ResponseBuilder::new(StatusCode::Trying)
        .from_request(&sample_request)
        .build()
        .expect("Failed to build 100 Trying");
    
    assert_eq!(trying.status_code(), 100);
    assert_eq!(trying.reason_phrase(), "Trying");
    
    // Test INVITE response builder with SDP
    let ok_with_sdp = InviteResponseBuilder::new(StatusCode::Ok)
        .from_request(&sample_request)
        .with_sdp_answer("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n")
        .with_contact_address(local_addr, Some("server"))
        .build()
        .expect("Failed to build 200 OK with SDP");
    
    assert_eq!(ok_with_sdp.status_code(), 200);
    assert!(ok_with_sdp.body().len() > 0); // Has SDP content
    assert!(ok_with_sdp.to().unwrap().tag().is_some()); // Auto-generated To tag
    
    // Test REGISTER response builder
    let register_request = client_quick::register(
        "sip:registrar.example.com",
        "sip:alice@example.com",
        "Alice",
        local_addr,
        Some(3600),
    ).expect("Failed to create REGISTER request");
    
    let register_ok = RegisterResponseBuilder::new(StatusCode::Ok)
        .from_request(&register_request)
        .with_expires(3600)
        .with_registered_contacts(vec!["sip:alice@192.168.1.100".to_string()])
        .build()
        .expect("Failed to build REGISTER 200 OK");
    
    assert_eq!(register_ok.status_code(), 200);
    
    println!("✅ Response builders create RFC 3261 compliant responses");
}

/// Test server quick helpers create proper responses
#[tokio::test]
async fn test_server_quick_helpers() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create sample requests for testing responses
    let invite_request = client_quick::invite(
        "sip:alice@example.com",
        "sip:bob@example.com",
        local_addr,
        None,
    ).expect("Failed to create INVITE request");
    
    let bye_request = client_quick::bye(
        "call-123",
        "sip:alice@example.com",
        "alice-tag",
        "sip:bob@example.com",
        "bob-tag",
        local_addr,
        2,
    ).expect("Failed to create BYE request");
    
    let register_request = client_quick::register(
        "sip:registrar.example.com",
        "sip:alice@example.com",
        "Alice",
        local_addr,
        Some(3600),
    ).expect("Failed to create REGISTER request");
    
    // Test server quick helpers
    let trying = server_quick::trying(&invite_request)
        .expect("Failed to create 100 Trying");
    assert_eq!(trying.status_code(), 100);
    
    let ringing = server_quick::ringing(&invite_request, Some("sip:server@example.com".to_string()))
        .expect("Failed to create 180 Ringing");
    assert_eq!(ringing.status_code(), 180);
    
    let ok_invite = server_quick::ok_invite(&invite_request, None, "sip:server@example.com".to_string())
        .expect("Failed to create 200 OK for INVITE");
    assert_eq!(ok_invite.status_code(), 200);
    
    let ok_bye = server_quick::ok_bye(&bye_request)
        .expect("Failed to create 200 OK for BYE");
    assert_eq!(ok_bye.status_code(), 200);
    
    let ok_register = server_quick::ok_register(&register_request, 3600, vec!["sip:alice@192.168.1.100".to_string()])
        .expect("Failed to create 200 OK for REGISTER");
    assert_eq!(ok_register.status_code(), 200);
    
    let busy_here = server_quick::busy_here(&invite_request)
        .expect("Failed to create 486 Busy Here");
    assert_eq!(busy_here.status_code(), 486);
    
    let not_found = server_quick::not_found(&invite_request)
        .expect("Failed to create 404 Not Found");
    assert_eq!(not_found.status_code(), 404);
    
    println!("✅ Server quick helpers create RFC 3261 compliant responses");
}

/// Test builders handle edge cases and validation
#[tokio::test]
async fn test_builder_validation() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Test INVITE builder requires mandatory fields
    let invite_result = InviteBuilder::new().build();
    assert!(invite_result.is_err(), "INVITE builder should fail without required fields");
    
    // Test BYE builder requires dialog information
    let bye_result = ByeBuilder::new().build();
    assert!(bye_result.is_err(), "BYE builder should fail without dialog information");
    
    // Test REGISTER builder requires mandatory fields
    let register_result = RegisterBuilder::new().build();
    assert!(register_result.is_err(), "REGISTER builder should fail without required fields");
    
    // Test valid builder with minimal required fields
    let minimal_invite = InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr)
        .build();
    assert!(minimal_invite.is_ok(), "INVITE builder should succeed with minimal required fields");
    
    println!("✅ Builders properly validate required fields");
}

/// Test builders support customization options
#[tokio::test]
async fn test_builder_customization() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let custom_call_id = "custom-call-id-123";
    let custom_cseq = 99;
    let custom_max_forwards = 50;
    
    // Test INVITE builder customization
    let invite = InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr)
        .call_id(custom_call_id)
        .cseq(custom_cseq)
        .max_forwards(custom_max_forwards)
        .contact("sip:custom-contact@example.com")
        .build()
        .expect("Failed to build customized INVITE");
    
    assert_eq!(invite.call_id().unwrap().value(), custom_call_id);
    assert_eq!(invite.cseq().unwrap().seq, custom_cseq);
    
    // Verify custom contact
    let contact_header = invite.header(&HeaderName::Contact);
    assert!(contact_header.is_some());
    
    // Test response builder customization
    let sample_request = client_quick::invite(
        "sip:alice@example.com",
        "sip:bob@example.com",
        local_addr,
        None,
    ).expect("Failed to create sample request");
    
    let custom_response = ResponseBuilder::new(StatusCode::Ok)
        .from_request(&sample_request)
        .reason_phrase("Custom OK")
        .with_to_tag("custom-to-tag")
        .with_contact("sip:custom-server@example.com")
        .build()
        .expect("Failed to build customized response");
    
    assert_eq!(custom_response.reason_phrase(), "Custom OK");
    assert_eq!(custom_response.to().unwrap().tag().unwrap(), "custom-to-tag");
    
    println!("✅ Builders support proper customization options");
}

/// Test builders generate unique identifiers
#[tokio::test]
async fn test_builder_unique_identifiers() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create multiple INVITE requests and verify they have unique Call-IDs and tags
    let invite1 = InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr)
        .build()
        .expect("Failed to build first INVITE");
    
    let invite2 = InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr)
        .build()
        .expect("Failed to build second INVITE");
    
    // Verify unique Call-IDs
    let call_id1 = invite1.call_id().unwrap().value();
    let call_id2 = invite2.call_id().unwrap().value();
    assert_ne!(call_id1, call_id2, "Call-IDs should be unique");
    
    // Verify unique From tags
    let from_tag1 = invite1.from().unwrap().tag().unwrap();
    let from_tag2 = invite2.from().unwrap().tag().unwrap();
    assert_ne!(from_tag1, from_tag2, "From tags should be unique");
    
    // Create multiple responses and verify they have unique To tags
    let response1 = InviteResponseBuilder::new(StatusCode::Ok)
        .from_request(&invite1)
        .build()
        .expect("Failed to build first response");
    
    let response2 = InviteResponseBuilder::new(StatusCode::Ok)
        .from_request(&invite2)
        .build()
        .expect("Failed to build second response");
    
    let to_tag1 = response1.to().unwrap().tag().unwrap();
    let to_tag2 = response2.to().unwrap().tag().unwrap();
    assert_ne!(to_tag1, to_tag2, "To tags should be unique");
    
    println!("✅ Builders generate unique identifiers correctly");
} 