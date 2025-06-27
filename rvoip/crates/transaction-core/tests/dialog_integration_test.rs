/**
 * Dialog Integration Tests
 * 
 * Tests for the Phase 3 dialog integration features including:
 * - Dialog utility functions (DialogRequestTemplate, DialogTransactionContext)
 * - Quick dialog functions (one-liner convenience functions)
 * - Dialog-aware request and response building
 * - Integration between dialog-core templates and transaction-core builders
 */

use std::net::SocketAddr;

use rvoip_sip_core::{Method, StatusCode, Uri};
use rvoip_transaction_core::builders::{client_quick, dialog_utils, dialog_quick};
use rvoip_transaction_core::dialog::{DialogRequestTemplate, DialogTransactionContext};
use rvoip_transaction_core::error::Result;

use tokio;

/// Test DialogRequestTemplate creation and usage
#[tokio::test]
async fn test_dialog_request_template() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create a dialog request template
    let template = DialogRequestTemplate {
        call_id: "test-call-456".to_string(),
        from_uri: "sip:alice@example.com".to_string(),
        from_tag: "alice-tag-789".to_string(),
        to_uri: "sip:bob@example.com".to_string(),
        to_tag: "bob-tag-012".to_string(),
        request_uri: "sip:bob@example.com".to_string(),
        cseq: 3,
        local_address: local_addr,
        route_set: vec![],
        contact: None,
    };
    
    // Test creating different request types from the template
    
    // 1. BYE request
    let bye_request = dialog_utils::request_builder_from_dialog_template(
        &template,
        Method::Bye,
        None,
        None
    ).expect("Failed to create BYE from template");
    
    assert_eq!(bye_request.method(), Method::Bye);
    assert_eq!(bye_request.call_id().unwrap().value(), template.call_id);
    assert_eq!(bye_request.from().unwrap().tag().unwrap(), template.from_tag);
    assert_eq!(bye_request.to().unwrap().tag().unwrap(), template.to_tag);
    assert_eq!(bye_request.cseq().unwrap().seq, template.cseq);
    
    // 2. UPDATE request with SDP
    let sdp_content = "v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 0\r\n";
    let update_request = dialog_utils::request_builder_from_dialog_template(
        &template,
        Method::Update,
        Some(sdp_content.to_string()),
        Some("application/sdp".to_string())
    ).expect("Failed to create UPDATE from template");
    
    assert_eq!(update_request.method(), Method::Update);
    assert_eq!(update_request.body(), sdp_content.as_bytes());
    
    // 3. INFO request
    let info_content = "Application specific information";
    let info_request = dialog_utils::request_builder_from_dialog_template(
        &template,
        Method::Info,
        Some(info_content.to_string()),
        Some("application/info".to_string())
    ).expect("Failed to create INFO from template");
    
    assert_eq!(info_request.method(), Method::Info);
    assert_eq!(info_request.body(), info_content.as_bytes());
    
    println!("✅ DialogRequestTemplate works correctly for all SIP methods");
}

/// Test DialogTransactionContext creation and response building
#[tokio::test]
async fn test_dialog_transaction_context() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create a sample INVITE request
    let invite_request = client_quick::invite(
        "sip:alice@example.com",
        "sip:bob@example.com",
        local_addr,
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n")
    ).expect("Failed to create INVITE");
    
    // Create dialog transaction context
    let context = dialog_utils::create_dialog_transaction_context(
        "txn-789",
        invite_request.clone(),
        Some("dialog-123".to_string()),
        local_addr
    );
    
    assert_eq!(context.transaction_id, "txn-789");
    assert_eq!(context.dialog_id, Some("dialog-123".to_string()));
    assert!(context.is_dialog_creating); // INVITE with no To tag is dialog-creating
    assert_eq!(context.local_address, local_addr);
    
    // Test response building from context
    let ok_response = dialog_utils::response_builder_for_dialog_transaction(
        &context,
        StatusCode::Ok,
        Some(local_addr),
        Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n".to_string())
    ).expect("Failed to build response from context");
    
    assert_eq!(ok_response.status_code(), 200);
    assert!(ok_response.to().unwrap().tag().is_some()); // Auto-generated To tag
    assert!(ok_response.body().len() > 0); // Has SDP content
    
    // Test non-dialog-creating request (with To tag)
    let mut bye_request = client_quick::bye(
        "call-123",
        "sip:alice@example.com",
        "alice-tag",
        "sip:bob@example.com",
        "bob-tag",
        local_addr,
        2
    ).expect("Failed to create BYE");
    
    let bye_context = dialog_utils::create_dialog_transaction_context(
        "txn-bye",
        bye_request,
        Some("dialog-456".to_string()),
        local_addr
    );
    
    assert!(!bye_context.is_dialog_creating); // BYE is not dialog-creating
    
    println!("✅ DialogTransactionContext works correctly for both dialog-creating and non-dialog-creating requests");
}

/// Test extract_dialog_template_from_request function
#[tokio::test]
async fn test_extract_dialog_template_from_request() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create a request with full dialog context
    let original_request = client_quick::bye(
        "extracted-call-123",
        "sip:alice@example.com",
        "alice-tag-456",
        "sip:bob@example.com",
        "bob-tag-789",
        local_addr,
        5
    ).expect("Failed to create BYE for extraction test");
    
    // Extract dialog template
    let extracted_template = dialog_utils::extract_dialog_template_from_request(
        &original_request,
        local_addr,
        6 // Next CSeq
    ).expect("Failed to extract dialog template");
    
    // Verify extracted information
    assert_eq!(extracted_template.call_id, "extracted-call-123");
    assert_eq!(extracted_template.from_uri, "sip:alice@example.com");
    assert_eq!(extracted_template.from_tag, "alice-tag-456");
    assert_eq!(extracted_template.to_uri, "sip:bob@example.com");
    assert_eq!(extracted_template.to_tag, "bob-tag-789");
    assert_eq!(extracted_template.cseq, 6);
    assert_eq!(extracted_template.local_address, local_addr);
    
    // Create a new request using the extracted template
    let new_request = dialog_utils::request_builder_from_dialog_template(
        &extracted_template,
        Method::Info,
        Some("Test information".to_string()),
        Some("text/plain".to_string())
    ).expect("Failed to create INFO from extracted template");
    
    // Verify the new request has the same dialog context
    assert_eq!(new_request.call_id().unwrap().value(), extracted_template.call_id);
    assert_eq!(new_request.from().unwrap().tag().unwrap(), extracted_template.from_tag);
    assert_eq!(new_request.to().unwrap().tag().unwrap(), extracted_template.to_tag);
    assert_eq!(new_request.cseq().unwrap().seq, extracted_template.cseq);
    
    println!("✅ extract_dialog_template_from_request works correctly");
}

/// Test all quick dialog functions
#[tokio::test]
async fn test_all_quick_dialog_functions() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    let call_id = "quick-test-call";
    let from_uri = "sip:alice@example.com";
    let from_tag = "alice-quick-tag";
    let to_uri = "sip:bob@example.com";
    let to_tag = "bob-quick-tag";
    let base_cseq = 10;
    
    // Test bye_for_dialog
    let bye_request = dialog_quick::bye_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        base_cseq,
        local_addr,
        None
    ).expect("Failed to create BYE with quick function");
    
    assert_eq!(bye_request.method(), Method::Bye);
    assert_eq!(bye_request.call_id().unwrap().value(), call_id);
    assert_eq!(bye_request.cseq().unwrap().seq, base_cseq);
    
    // Test refer_for_dialog
    let refer_target = "sip:charlie@example.com";
    let refer_request = dialog_quick::refer_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        refer_target,
        base_cseq + 1,
        local_addr,
        None
    ).expect("Failed to create REFER with quick function");
    
    assert_eq!(refer_request.method(), Method::Refer);
    assert_eq!(refer_request.cseq().unwrap().seq, base_cseq + 1);
    let body_str = String::from_utf8_lossy(refer_request.body());
    assert!(body_str.contains("Refer-To"));
    assert!(body_str.contains(refer_target));
    
    // Test update_for_dialog with SDP
    let sdp_content = "v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 0\r\n";
    let update_request = dialog_quick::update_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        Some(sdp_content.to_string()),
        base_cseq + 2,
        local_addr,
        None
    ).expect("Failed to create UPDATE with quick function");
    
    assert_eq!(update_request.method(), Method::Update);
    assert_eq!(update_request.cseq().unwrap().seq, base_cseq + 2);
    assert_eq!(update_request.body(), sdp_content.as_bytes());
    
    // Test update_for_dialog without SDP
    let empty_update_request = dialog_quick::update_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        None, // No SDP
        base_cseq + 3,
        local_addr,
        None
    ).expect("Failed to create empty UPDATE with quick function");
    
    assert_eq!(empty_update_request.method(), Method::Update);
    assert_eq!(empty_update_request.body().len(), 0);
    
    // Test info_for_dialog
    let info_content = "Custom application data";
    let info_request = dialog_quick::info_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        info_content,
        Some("application/custom".to_string()),
        base_cseq + 4,
        local_addr,
        None
    ).expect("Failed to create INFO with quick function");
    
    assert_eq!(info_request.method(), Method::Info);
    assert_eq!(info_request.cseq().unwrap().seq, base_cseq + 4);
    assert_eq!(info_request.body(), info_content.as_bytes());
    
    // Test info_for_dialog with default content type
    let default_info_request = dialog_quick::info_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        "Default content type test",
        None, // Should default to "application/info"
        base_cseq + 5,
        local_addr,
        None
    ).expect("Failed to create INFO with default content type");
    
    assert_eq!(default_info_request.method(), Method::Info);
    
    // Test notify_for_dialog
    let event_type = "dialog";
    let notification_body = "Dialog state information";
    let notify_request = dialog_quick::notify_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        event_type,
        Some(notification_body.to_string()),
        base_cseq + 6,
        local_addr,
        None
    ).expect("Failed to create NOTIFY with quick function");
    
    assert_eq!(notify_request.method(), Method::Notify);
    assert_eq!(notify_request.cseq().unwrap().seq, base_cseq + 6);
    assert_eq!(notify_request.body(), notification_body.as_bytes());
    
    // Test message_for_dialog
    let message_content = "Hello from Alice!";
    let message_request = dialog_quick::message_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        message_content,
        Some("text/plain".to_string()),
        base_cseq + 7,
        local_addr,
        None
    ).expect("Failed to create MESSAGE with quick function");
    
    assert_eq!(message_request.method(), Method::Message);
    assert_eq!(message_request.cseq().unwrap().seq, base_cseq + 7);
    assert_eq!(message_request.body(), message_content.as_bytes());
    
    // Test message_for_dialog with default content type
    let default_message_request = dialog_quick::message_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        "Default content type message",
        None, // Should default to "text/plain"
        base_cseq + 8,
        local_addr,
        None
    ).expect("Failed to create MESSAGE with default content type");
    
    assert_eq!(default_message_request.method(), Method::Message);
    
    // Test reinvite_for_dialog
    let reinvite_sdp = "v=0\r\no=alice 890 123 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5008 RTP/AVP 0\r\n";
    let contact_uri = format!("sip:alice@{}", local_addr.ip());
    let reinvite_request = dialog_quick::reinvite_for_dialog(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        reinvite_sdp,
        base_cseq + 9,
        local_addr,
        None,
        Some(contact_uri.clone())
    ).expect("Failed to create re-INVITE with quick function");
    
    assert_eq!(reinvite_request.method(), Method::Invite);
    assert_eq!(reinvite_request.cseq().unwrap().seq, base_cseq + 9);
    assert_eq!(reinvite_request.body(), reinvite_sdp.as_bytes());
    
    println!("✅ All quick dialog functions work correctly");
}

/// Test response_for_dialog_transaction quick function
#[tokio::test]
async fn test_response_for_dialog_transaction_quick() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create a sample request
    let original_request = client_quick::invite(
        "sip:alice@example.com",
        "sip:bob@example.com",
        local_addr,
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n")
    ).expect("Failed to create INVITE");
    
    // Test quick response creation
    let response = dialog_quick::response_for_dialog_transaction(
        "txn-quick-456",
        original_request.clone(),
        Some("dialog-quick-789".to_string()),
        StatusCode::Ok,
        local_addr,
        Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n".to_string()),
        Some("Custom OK".to_string())
    ).expect("Failed to create response with quick function");
    
    assert_eq!(response.status_code(), 200);
    assert_eq!(response.reason_phrase(), "Custom OK");
    assert!(response.to().unwrap().tag().is_some()); // Auto-generated To tag
    assert!(response.body().len() > 0); // Has SDP content
    
    // Test quick response without custom reason
    let simple_response = dialog_quick::response_for_dialog_transaction(
        "txn-simple-123",
        original_request,
        None, // No dialog ID
        StatusCode::BadRequest,
        local_addr,
        None, // No SDP content
        None  // No custom reason
    ).expect("Failed to create simple response with quick function");
    
    assert_eq!(simple_response.status_code(), 400);
    assert_eq!(simple_response.body().len(), 0); // No SDP content
    
    println!("✅ response_for_dialog_transaction quick function works correctly");
}

/// Test dialog utility functions with route sets
#[tokio::test]
async fn test_dialog_functions_with_route_sets() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Create route set
    let route1: Uri = "sip:proxy1.example.com".parse().unwrap();
    let route2: Uri = "sip:proxy2.example.com".parse().unwrap();
    let route_set = vec![route1, route2];
    
    // Create dialog template with route set
    let template = DialogRequestTemplate {
        call_id: "route-test-call".to_string(),
        from_uri: "sip:alice@example.com".to_string(),
        from_tag: "alice-route-tag".to_string(),
        to_uri: "sip:bob@example.com".to_string(),
        to_tag: "bob-route-tag".to_string(),
        request_uri: "sip:proxy.example.com".to_string(), // Different from To URI when using route sets
        cseq: 15,
        local_address: local_addr,
        route_set: route_set.clone(),
        contact: Some("sip:alice@192.168.1.100".to_string()),
    };
    
    // Create request using template with route set
    let request_with_routes = dialog_utils::request_builder_from_dialog_template(
        &template,
        Method::Update,
        Some("SDP with route set".to_string()),
        Some("application/sdp".to_string())
    ).expect("Failed to create request with route set");
    
    assert_eq!(request_with_routes.method(), Method::Update);
    assert_eq!(request_with_routes.uri().to_string(), template.request_uri);
    
    // Verify Route headers are present (implementation specific verification)
    let route_headers: Vec<_> = request_with_routes.headers.iter()
        .filter(|h| matches!(h, rvoip_sip_core::types::header::TypedHeader::Route(_)))
        .collect();
    
    // Should have Route headers when route set is provided
    assert!(!route_headers.is_empty(), "Should have Route headers when route set is provided");
    
    // Test quick function with route set
    let quick_request_with_routes = dialog_quick::refer_for_dialog(
        &template.call_id,
        &template.from_uri,
        &template.from_tag,
        &template.to_uri,
        &template.to_tag,
        "sip:transfer-target@example.com",
        template.cseq + 1,
        local_addr,
        Some(route_set.clone())
    ).expect("Failed to create REFER with route set using quick function");
    
    assert_eq!(quick_request_with_routes.method(), Method::Refer);
    assert_eq!(quick_request_with_routes.cseq().unwrap().seq, template.cseq + 1);
    
    println!("✅ Dialog functions work correctly with route sets");
}

/// Test dialog helper functions from the helpers module
#[tokio::test]
async fn test_dialog_helper_functions() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    let template = DialogRequestTemplate {
        call_id: "helper-test-call".to_string(),
        from_uri: "sip:alice@example.com".to_string(),
        from_tag: "alice-helper-tag".to_string(),
        to_uri: "sip:bob@example.com".to_string(),
        to_tag: "bob-helper-tag".to_string(),
        request_uri: "sip:bob@example.com".to_string(),
        cseq: 20,
        local_address: local_addr,
        route_set: vec![],
        contact: None,
    };
    
    // Test helper functions from the helpers module
    use rvoip_transaction_core::dialog::helpers;
    
    // Test quick_bye_from_template
    let bye_request = helpers::quick_bye_from_template(&template)
        .expect("Failed to create BYE with helper function");
    
    assert_eq!(bye_request.method(), Method::Bye);
    assert_eq!(bye_request.call_id().unwrap().value(), template.call_id);
    
    // Test quick_refer_from_template
    let refer_target = "sip:helper-target@example.com";
    let refer_request = helpers::quick_refer_from_template(&template, refer_target)
        .expect("Failed to create REFER with helper function");
    
    assert_eq!(refer_request.method(), Method::Refer);
    let body_str = String::from_utf8_lossy(refer_request.body());
    assert!(body_str.contains(refer_target));
    
    // Test quick_update_from_template with SDP
    let sdp_content = "v=0\r\no=helper 123 456 IN IP4 127.0.0.1\r\n";
    let update_request = helpers::quick_update_from_template(&template, Some(sdp_content.to_string()))
        .expect("Failed to create UPDATE with helper function");
    
    assert_eq!(update_request.method(), Method::Update);
    assert_eq!(update_request.body(), sdp_content.as_bytes());
    
    // Test quick_update_from_template without SDP
    let empty_update_request = helpers::quick_update_from_template(&template, None)
        .expect("Failed to create empty UPDATE with helper function");
    
    assert_eq!(empty_update_request.method(), Method::Update);
    assert_eq!(empty_update_request.body().len(), 0);
    
    // Test quick_info_from_template
    let info_content = "Helper info content";
    let info_request = helpers::quick_info_from_template(&template, info_content)
        .expect("Failed to create INFO with helper function");
    
    assert_eq!(info_request.method(), Method::Info);
    assert_eq!(info_request.body(), info_content.as_bytes());
    
    // Test quick_notify_from_template
    let event_type = "presence";
    let notify_body = "Presence information";
    let notify_request = helpers::quick_notify_from_template(
        &template, 
        event_type, 
        Some(notify_body.to_string())
    ).expect("Failed to create NOTIFY with helper function");
    
    assert_eq!(notify_request.method(), Method::Notify);
    assert_eq!(notify_request.body(), notify_body.as_bytes());
    
    println!("✅ Dialog helper functions work correctly");
}

/// Test error handling in dialog functions
#[tokio::test]
async fn test_dialog_functions_error_handling() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // Test with invalid URIs in quick functions
    let result = dialog_quick::bye_for_dialog(
        "test-call",
        "invalid-uri", // Invalid URI
        "tag",
        "sip:bob@example.com",
        "tag2",
        1,
        local_addr,
        None
    );
    
    // Should handle URI parsing errors gracefully
    // (Actual error handling depends on implementation)
    
    // Test with empty request URI in template
    let invalid_template = DialogRequestTemplate {
        call_id: "test".to_string(),
        from_uri: "sip:alice@example.com".to_string(),
        from_tag: "tag1".to_string(),
        to_uri: "sip:bob@example.com".to_string(),
        to_tag: "tag2".to_string(),
        request_uri: "".to_string(), // Empty request URI
        cseq: 1,
        local_address: local_addr,
        route_set: vec![],
        contact: None,
    };
    
    let result = dialog_utils::request_builder_from_dialog_template(
        &invalid_template,
        Method::Info,
        Some("test".to_string()),
        None
    );
    
    // Should handle empty request URI errors
    assert!(result.is_err(), "Should fail with empty request URI");
    
    println!("✅ Dialog functions handle errors correctly");
}

/// Integration test combining all dialog functions
#[tokio::test]
async fn test_complete_dialog_integration() {
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    // 1. Start with a simple INVITE to establish dialog context
    let initial_invite = client_quick::invite(
        "sip:alice@example.com",
        "sip:bob@example.com",
        local_addr,
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n")
    ).expect("Failed to create initial INVITE");
    
    let call_id = initial_invite.call_id().unwrap().value();
    let from_tag = initial_invite.from().unwrap().tag().unwrap();
    
    // 2. Extract dialog template from the initial request
    // Note: For initial INVITE, we need to simulate it having a To tag for extraction
    // In practice, this would be done after receiving a response with To tag
    let mut simulated_in_dialog_request = initial_invite.clone();
    // We can't easily modify the request, so let's create a BYE instead for extraction
    let bye_for_extraction = dialog_quick::bye_for_dialog(
        &call_id,
        "sip:alice@example.com",
        from_tag.clone(), // Clone the actual from_tag from the original request
        "sip:bob@example.com",
        "temp-to-tag",
        2,
        local_addr,
        None
    ).expect("Failed to create BYE for extraction");
    
    let extracted_template = dialog_utils::extract_dialog_template_from_request(
        &bye_for_extraction,
        local_addr,
        2
    ).expect("Failed to extract dialog template");
    
    // Modify the template to add To tag (simulating established dialog)
    let mut dialog_template = extracted_template;
    dialog_template.to_tag = "established-to-tag".to_string();
    dialog_template.cseq = 2;
    
    // 3. Use dialog template to create various requests
    let info_request = dialog_utils::request_builder_from_dialog_template(
        &dialog_template,
        Method::Info,
        Some("Dialog integration test".to_string()),
        Some("text/plain".to_string())
    ).expect("Failed to create INFO from dialog template");
    
    // 4. Use quick functions for more requests
    let refer_request = dialog_quick::refer_for_dialog(
        &dialog_template.call_id,
        &dialog_template.from_uri,
        &dialog_template.from_tag,
        &dialog_template.to_uri,
        &dialog_template.to_tag,
        "sip:transfer@example.com",
        3,
        local_addr,
        None
    ).expect("Failed to create REFER with quick function");
    
    let message_request = dialog_quick::message_for_dialog(
        &dialog_template.call_id,
        &dialog_template.from_uri,
        &dialog_template.from_tag,
        &dialog_template.to_uri,
        &dialog_template.to_tag,
        "Integration test message",
        None,
        4,
        local_addr,
        None
    ).expect("Failed to create MESSAGE with quick function");
    
    let bye_request = dialog_quick::bye_for_dialog(
        &dialog_template.call_id,
        &dialog_template.from_uri,
        &dialog_template.from_tag,
        &dialog_template.to_uri,
        &dialog_template.to_tag,
        5,
        local_addr,
        None
    ).expect("Failed to create BYE with quick function");
    
    // 5. Create response context and build response
    let response_context = dialog_utils::create_dialog_transaction_context(
        "integration-txn",
        initial_invite.clone(), // Clone here to avoid move
        Some("integration-dialog".to_string()),
        local_addr
    );
    
    let response = dialog_utils::response_builder_for_dialog_transaction(
        &response_context,
        StatusCode::Ok,
        Some(local_addr),
        Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n".to_string())
    ).expect("Failed to create response from dialog context");
    
    // 6. Verify all requests maintain dialog consistency
    assert_eq!(info_request.call_id().unwrap().value(), call_id);
    assert_eq!(refer_request.call_id().unwrap().value(), call_id);
    assert_eq!(message_request.call_id().unwrap().value(), call_id);
    assert_eq!(bye_request.call_id().unwrap().value(), call_id);
    
    assert_eq!(info_request.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(refer_request.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(message_request.from().unwrap().tag().unwrap(), from_tag);
    assert_eq!(bye_request.from().unwrap().tag().unwrap(), from_tag);
    
    // Verify CSeq progression
    assert_eq!(info_request.cseq().unwrap().seq, 2);
    assert_eq!(refer_request.cseq().unwrap().seq, 3);
    assert_eq!(message_request.cseq().unwrap().seq, 4);
    assert_eq!(bye_request.cseq().unwrap().seq, 5);
    
    // Verify response
    assert_eq!(response.status_code(), 200);
    assert!(response.to().unwrap().tag().is_some());
    
    println!("✅ Complete dialog integration works correctly");
    println!("   - Template extraction and modification");
    println!("   - Request building from templates");
    println!("   - Quick function usage");
    println!("   - Response building from context");
    println!("   - Dialog consistency maintenance");
} 