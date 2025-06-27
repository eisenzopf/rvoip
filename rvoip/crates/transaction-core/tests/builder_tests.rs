//! Builder Tests
//!
//! Comprehensive tests for the client and server request/response builders,
//! verifying they create RFC 3261 compliant SIP messages.

use std::net::SocketAddr;
use std::str::FromStr;

use rvoip_sip_core::{Method, StatusCode, Uri};
use rvoip_sip_core::types::header::{HeaderName, TypedHeader};
use rvoip_transaction_core::builders::{client_quick, server_quick};
use rvoip_transaction_core::client::builders::{InviteBuilder, ByeBuilder, RegisterBuilder, InDialogRequestBuilder};
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

// NEW TESTS FOR DIALOG-AWARE BUILDERS

/// Test InviteBuilder dialog-aware functionality
#[tokio::test]
async fn test_invite_builder_dialog_aware() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let call_id = "dialog-call-123";
    let from_uri = "sip:alice@example.com";
    let from_tag = "alice-tag-456";
    let to_uri = "sip:bob@example.com";
    let to_tag = "bob-tag-789";
    let cseq = 2;
    
    // Test basic dialog-aware INVITE (re-INVITE scenario)
    let reinvite = InviteBuilder::from_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        cseq,
        local_addr
    )
    .with_sdp("v=0\r\no=alice 789 012 IN IP4 127.0.0.1\r\n")
    .build()
    .expect("Failed to build dialog-aware INVITE");
    
    // Verify dialog context is properly applied
    assert_eq!(reinvite.call_id().unwrap().value(), call_id);
    assert_eq!(reinvite.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(reinvite.to().unwrap().tag().unwrap(), to_tag);
    assert_eq!(reinvite.cseq().unwrap().seq, cseq);
    assert_eq!(reinvite.uri().to_string(), to_uri);
    
    // Test enhanced dialog-aware INVITE with route set
    let route1: Uri = "sip:proxy1.example.com".parse().unwrap();
    let route2: Uri = "sip:proxy2.example.com".parse().unwrap();
    let route_set = vec![route1, route2];
    let request_uri = "sip:proxy.example.com";
    let contact = "sip:alice@192.168.1.100";
    
    let enhanced_invite = InviteBuilder::from_dialog_enhanced(
        call_id,
        from_uri,
        from_tag,
        Some("Alice".to_string()),
        to_uri,
        to_tag,
        Some("Bob".to_string()),
        request_uri,
        cseq + 1,
        local_addr,
        route_set.clone(),
        Some(contact.to_string())
    )
    .with_sdp("v=0\r\no=alice 890 123 IN IP4 127.0.0.1\r\n")
    .build()
    .expect("Failed to build enhanced dialog-aware INVITE");
    
    // Verify enhanced dialog context
    assert_eq!(enhanced_invite.uri().to_string(), request_uri);
    assert_eq!(enhanced_invite.cseq().unwrap().seq, cseq + 1);
    assert_eq!(enhanced_invite.from().unwrap().address().display_name().unwrap(), "Alice");
    assert_eq!(enhanced_invite.to().unwrap().address().display_name().unwrap(), "Bob");
    
    // Verify Route headers are present
    let route_headers: Vec<_> = enhanced_invite.headers.iter()
        .filter(|h| matches!(h, TypedHeader::Route(_)))
        .collect();
    assert_eq!(route_headers.len(), 2, "Should have two Route headers");
    
    println!("✅ InviteBuilder dialog-aware functionality works correctly");
}

/// Test ByeBuilder enhanced dialog functionality
#[tokio::test]
async fn test_bye_builder_enhanced_dialog() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let call_id = "dialog-call-456";
    let from_uri = "sip:alice@example.com";
    let from_tag = "alice-tag-789";
    let to_uri = "sip:bob@example.com";
    let to_tag = "bob-tag-012";
    let request_uri = "sip:proxy.example.com";
    let cseq = 3;
    
    // Test enhanced BYE with route set
    let route1: Uri = "sip:proxy1.example.com".parse().unwrap();
    let route2: Uri = "sip:proxy2.example.com".parse().unwrap();
    let route_set = vec![route1, route2];
    
    let enhanced_bye = ByeBuilder::from_dialog_enhanced(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        request_uri,
        cseq,
        local_addr,
        route_set.clone()
    )
    .build()
    .expect("Failed to build enhanced dialog-aware BYE");
    
    // Verify dialog context
    assert_eq!(enhanced_bye.call_id().unwrap().value(), call_id);
    assert_eq!(enhanced_bye.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(enhanced_bye.to().unwrap().tag().unwrap(), to_tag);
    assert_eq!(enhanced_bye.cseq().unwrap().seq, cseq);
    assert_eq!(enhanced_bye.uri().to_string(), request_uri);
    
    // Verify Route headers are present
    let route_headers: Vec<_> = enhanced_bye.headers.iter()
        .filter(|h| matches!(h, TypedHeader::Route(_)))
        .collect();
    assert_eq!(route_headers.len(), 2, "Should have two Route headers");
    
    println!("✅ ByeBuilder enhanced dialog functionality works correctly");
}

/// Test InDialogRequestBuilder for various SIP methods
#[tokio::test]
async fn test_in_dialog_request_builder() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let call_id = "dialog-call-789";
    let from_uri = "sip:alice@example.com";
    let from_tag = "alice-tag-123";
    let to_uri = "sip:bob@example.com";
    let to_tag = "bob-tag-456";
    let cseq = 4;
    
    // Test REFER request
    let refer_target = "sip:charlie@example.com";
    let refer = InDialogRequestBuilder::for_refer(refer_target)
        .from_dialog(call_id, from_uri, from_tag, to_uri, to_tag, cseq, local_addr)
        .build()
        .expect("Failed to build REFER request");
    
    assert_eq!(refer.method(), Method::Refer);
    assert_eq!(refer.call_id().unwrap().value(), call_id);
    assert!(refer.body().len() > 0); // Should have Refer-To body
    let body_str = String::from_utf8_lossy(refer.body());
    assert!(body_str.contains("Refer-To"));
    assert!(body_str.contains(refer_target));
    
    // Test UPDATE request with SDP
    let sdp_content = "v=0\r\no=alice 345 678 IN IP4 127.0.0.1\r\n";
    let update = InDialogRequestBuilder::for_update(Some(sdp_content.to_string()))
        .from_dialog(call_id, from_uri, from_tag, to_uri, to_tag, cseq + 1, local_addr)
        .build()
        .expect("Failed to build UPDATE request");
    
    assert_eq!(update.method(), Method::Update);
    assert_eq!(update.body(), sdp_content.as_bytes());
    
    // Verify Content-Type header for SDP
    let content_type = update.header(&HeaderName::ContentType);
    assert!(content_type.is_some());
    
    // Test INFO request
    let info_content = "Application specific information";
    let info = InDialogRequestBuilder::for_info(info_content, Some("application/info".to_string()))
        .from_dialog(call_id, from_uri, from_tag, to_uri, to_tag, cseq + 2, local_addr)
        .build()
        .expect("Failed to build INFO request");
    
    assert_eq!(info.method(), Method::Info);
    assert_eq!(info.body(), info_content.as_bytes());
    
    // Test NOTIFY request
    let event_type = "dialog";
    let notify_body = "Dialog state information";
    let notify = InDialogRequestBuilder::for_notify(event_type, Some(notify_body.to_string()))
        .from_dialog(call_id, from_uri, from_tag, to_uri, to_tag, cseq + 3, local_addr)
        .build()
        .expect("Failed to build NOTIFY request");
    
    assert_eq!(notify.method(), Method::Notify);
    assert_eq!(notify.body(), notify_body.as_bytes());
    
    // Verify Event header is present
    let event_header = notify.header(&HeaderName::Event);
    assert!(event_header.is_some());
    
    // Test MESSAGE request
    let message_content = "Hello from Alice!";
    let message = InDialogRequestBuilder::for_message(message_content, Some("text/plain".to_string()))
        .from_dialog(call_id, from_uri, from_tag, to_uri, to_tag, cseq + 4, local_addr)
        .build()
        .expect("Failed to build MESSAGE request");
    
    assert_eq!(message.method(), Method::Message);
    assert_eq!(message.body(), message_content.as_bytes());
    
    println!("✅ InDialogRequestBuilder works correctly for all SIP methods");
}

/// Test dialog-aware response builders
#[tokio::test]
async fn test_dialog_aware_response_builders() {
    use rvoip_transaction_core::server::builders::{ResponseBuilder, InviteResponseBuilder};
    
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create a sample INVITE request for testing responses
    let invite_request = InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr)
        .with_sdp("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n")
        .build()
        .expect("Failed to create INVITE request");
    
    // Test dialog-aware response building
    let dialog_id = "dialog-123";
    let dialog_response = ResponseBuilder::from_dialog_transaction(
        StatusCode::Ok,
        &invite_request,
        Some(dialog_id)
    )
    .with_contact_address(local_addr, Some("server"))
    .build()
    .expect("Failed to build dialog-aware response");
    
    assert_eq!(dialog_response.status_code(), 200);
    assert!(dialog_response.to().unwrap().tag().is_some()); // Auto-generated To tag
    
    // Test automatic dialog context detection
    let auto_detect_response = ResponseBuilder::from_request_with_dialog_detection(
        StatusCode::Ok,
        &invite_request
    )
    .with_contact_address(local_addr, Some("server"))
    .build()
    .expect("Failed to build auto-detected dialog response");
    
    assert_eq!(auto_detect_response.status_code(), 200);
    assert!(auto_detect_response.to().unwrap().tag().is_some()); // Auto-generated To tag for dialog-creating response
    
    // Test INVITE-specific dialog-aware responses
    let trying_response = InviteResponseBuilder::trying_for_dialog(&invite_request)
        .build()
        .expect("Failed to build trying response");
    
    assert_eq!(trying_response.status_code(), 100);
    assert!(trying_response.to().unwrap().tag().is_none()); // No To tag for 100 responses
    
    let ringing_response = InviteResponseBuilder::ringing_for_dialog(
        &invite_request,
        Some(dialog_id),
        None
    )
    .build()
    .expect("Failed to build ringing response");
    
    assert_eq!(ringing_response.status_code(), 180);
    assert!(ringing_response.to().unwrap().tag().is_some()); // To tag for 18x responses
    
    let ok_response = InviteResponseBuilder::ok_for_dialog(
        &invite_request,
        Some(dialog_id),
        "v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n".to_string(),
        "sip:server@example.com".to_string()
    )
    .build()
    .expect("Failed to build OK response");
    
    assert_eq!(ok_response.status_code(), 200);
    assert!(ok_response.to().unwrap().tag().is_some()); // To tag for 2xx responses
    assert!(ok_response.body().len() > 0); // Has SDP content
    
    let error_response = InviteResponseBuilder::error_for_dialog(
        &invite_request,
        StatusCode::BusyHere,
        Some("Busy Here".to_string())
    )
    .build()
    .expect("Failed to build error response");
    
    assert_eq!(error_response.status_code(), 486);
    assert_eq!(error_response.reason_phrase(), "Busy Here");
    
    println!("✅ Dialog-aware response builders work correctly");
}

/// Test builders integration with real-world dialog scenarios
#[tokio::test]
async fn test_builders_dialog_integration() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Simulate a complete dialog flow using the enhanced builders
    
    // 1. Initial INVITE (dialog-creating)
    let initial_invite = InviteBuilder::new()
        .from_to("sip:alice@example.com", "sip:bob@example.com")
        .local_address(local_addr)
        .with_sdp("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n")
        .build()
        .expect("Failed to create initial INVITE");
    
    let call_id = initial_invite.call_id().unwrap().value();
    let from_tag = initial_invite.from().unwrap().tag().unwrap();
    
    // 2. Response creating dialog
    let ok_response = InviteResponseBuilder::ok_for_dialog(
        &initial_invite,
        Some("dialog-123"),
        "v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n".to_string(),
        "sip:server@example.com".to_string()
    )
    .build()
    .expect("Failed to create dialog-creating response");
    
    let to_tag = ok_response.to().unwrap().tag().unwrap();
    
    // 3. Re-INVITE using dialog context
    let reinvite = InviteBuilder::from_dialog(
        call_id.clone(),
        "sip:alice@example.com",
        from_tag.clone(),
        "sip:bob@example.com",
        to_tag.clone(),
        2,
        local_addr
    )
    .with_sdp("v=0\r\no=alice 789 012 IN IP4 127.0.0.1\r\n")
    .build()
    .expect("Failed to create re-INVITE");
    
    // 4. REFER within dialog
    let refer = InDialogRequestBuilder::for_refer("sip:charlie@example.com")
        .from_dialog(call_id.clone(), "sip:alice@example.com", from_tag.clone(), "sip:bob@example.com", to_tag.clone(), 3, local_addr)
        .build()
        .expect("Failed to create REFER");
    
    // 5. BYE to terminate dialog
    let bye = ByeBuilder::new()
        .from_dialog(call_id.clone(), "sip:alice@example.com", from_tag.clone(), "sip:bob@example.com", to_tag.clone())
        .local_address(local_addr)
        .cseq(4)
        .build()
        .expect("Failed to create BYE");
    
    // Verify the dialog flow maintains consistency
    assert_eq!(initial_invite.call_id().unwrap().value(), call_id);
    assert_eq!(reinvite.call_id().unwrap().value(), call_id);
    assert_eq!(refer.call_id().unwrap().value(), call_id);
    assert_eq!(bye.call_id().unwrap().value(), call_id);
    
    // Verify CSeq progression
    assert_eq!(initial_invite.cseq().unwrap().seq, 1);
    assert_eq!(reinvite.cseq().unwrap().seq, 2);
    assert_eq!(refer.cseq().unwrap().seq, 3);
    assert_eq!(bye.cseq().unwrap().seq, 4);
    
    // Verify tag consistency
    assert_eq!(reinvite.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(refer.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(bye.from().unwrap().tag().unwrap(), from_tag);
    
    assert_eq!(reinvite.to().unwrap().tag().unwrap(), to_tag);
    assert_eq!(refer.to().unwrap().tag().unwrap(), to_tag);
    assert_eq!(bye.to().unwrap().tag().unwrap(), to_tag);
    
    println!("✅ Builders integrate correctly with real-world dialog scenarios");
} 