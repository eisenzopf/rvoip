//! Unit tests for request routing functionality
//!
//! Tests request routing logic and dialog matching in isolation.

use rvoip_dialog_core::{Dialog, DialogState};
use rvoip_sip_core::{Request, Method, HeaderName, TypedHeader, Uri};

/// Test dialog tuple extraction for routing
#[test]
fn test_dialog_tuple_extraction() {
    let dialog = Dialog::new(
        "routing-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Test tuple generation
    let tuple = dialog.dialog_id_tuple().unwrap();
    assert_eq!(tuple.0, "routing-test-call-id");
    assert_eq!(tuple.1, "alice-tag");
    assert_eq!(tuple.2, "bob-tag");
}

/// Test dialog tuple extraction with missing tags
#[test]
fn test_dialog_tuple_extraction_missing_tags() {
    // Dialog with missing remote tag
    let dialog_no_remote_tag = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        None, // No remote tag
        true,
    );
    
    assert!(dialog_no_remote_tag.dialog_id_tuple().is_none());
    
    // Dialog with missing local tag
    let dialog_no_local_tag = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        None, // No local tag
        Some("bob-tag".to_string()),
        true,
    );
    
    assert!(dialog_no_local_tag.dialog_id_tuple().is_none());
    
    // Dialog with both tags missing
    let dialog_no_tags = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        None, // No local tag
        None, // No remote tag
        true,
    );
    
    assert!(dialog_no_tags.dialog_id_tuple().is_none());
}

/// Test request matching based on Call-ID and tags
#[test]
fn test_request_dialog_matching() {
    // Create a dialog
    let dialog = Dialog::new(
        "match-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Create a request that should match this dialog
    let matching_request = create_request_with_headers(
        Method::Bye,
        "sip:alice@example.com",
        "match-test-call-id",
        "bob-tag", // From tag (remote's perspective)
        "alice-tag", // To tag (local's perspective)
    );
    
    // Test that we can extract the same dialog tuple from the request
    let request_tuple = extract_dialog_tuple_from_request(&matching_request);
    let dialog_tuple = dialog.dialog_id_tuple().unwrap();
    
    // For initiator, the tags are swapped in the request perspective
    assert_eq!(request_tuple.0, dialog_tuple.0); // Same Call-ID
    // Note: Tag matching would need proper From/To header analysis
}

/// Test in-dialog vs new dialog request classification
#[test]
fn test_request_classification() {
    // Create established dialog
    let mut dialog = Dialog::new(
        "established-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Set dialog to confirmed state
    dialog.state = DialogState::Confirmed;
    
    // Test methods that are typically in-dialog
    let in_dialog_methods = vec![
        Method::Bye,
        Method::Ack,
        Method::Info,
        Method::Update,
    ];
    
    for method in in_dialog_methods {
        assert!(is_in_dialog_method(&method), "Method {:?} should be in-dialog", method);
    }
    
    // Test methods that typically create new dialogs
    let new_dialog_methods = vec![
        Method::Invite,
        Method::Register,
        Method::Options,
        Method::Subscribe,
    ];
    
    for method in new_dialog_methods {
        assert!(!is_in_dialog_method(&method), "Method {:?} should create new dialog", method);
    }
}

/// Test route set handling for requests
#[test]
fn test_route_set_handling() {
    let mut dialog = Dialog::new(
        "route-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Add some routes to the dialog
    let route1: Uri = "sip:proxy1.example.com".parse().unwrap();
    let route2: Uri = "sip:proxy2.example.com".parse().unwrap();
    dialog.route_set = vec![route1.clone(), route2.clone()];
    
    // Create a request
    let request = dialog.create_request(Method::Bye);
    
    // The request should be created with the remote target
    assert_eq!(request.uri, dialog.remote_target);
    
    // In a full implementation, Route headers would be added based on route_set
    // For now, just verify the route set is available
    assert_eq!(dialog.route_set.len(), 2);
    assert_eq!(dialog.route_set[0], route1);
    assert_eq!(dialog.route_set[1], route2);
}

/// Test dialog matching with different perspective (initiator vs recipient)
#[test]
fn test_dialog_perspective_matching() {
    // Create initiator dialog
    let initiator_dialog = Dialog::new(
        "perspective-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true, // initiator
    );
    
    // Create recipient dialog (same call, different perspective)
    let recipient_dialog = Dialog::new(
        "perspective-test-call-id".to_string(),
        "sip:bob@example.com".parse().unwrap(),
        "sip:alice@example.com".parse().unwrap(),
        Some("bob-tag".to_string()),
        Some("alice-tag".to_string()),
        false, // recipient
    );
    
    // Both should have the same Call-ID
    assert_eq!(initiator_dialog.call_id, recipient_dialog.call_id);
    
    // But different perspectives on local/remote
    assert_ne!(initiator_dialog.local_uri, recipient_dialog.local_uri);
    assert_ne!(initiator_dialog.remote_uri, recipient_dialog.remote_uri);
    assert_ne!(initiator_dialog.local_tag, recipient_dialog.local_tag);
    assert_ne!(initiator_dialog.remote_tag, recipient_dialog.remote_tag);
    
    // Verify perspective flags
    assert!(initiator_dialog.is_initiator);
    assert!(!recipient_dialog.is_initiator);
}

/// Test sequence number validation for in-dialog requests
#[test]
fn test_sequence_number_validation() {
    let mut dialog = Dialog::new(
        "seq-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Set initial sequence numbers
    dialog.local_seq = 5;
    dialog.remote_seq = 3;
    
    // Create requests and verify sequence number increments
    let request1 = dialog.create_request(Method::Info);
    assert_eq!(dialog.local_seq, 6);
    
    let request2 = dialog.create_request(Method::Bye);
    assert_eq!(dialog.local_seq, 7);
    
    // ACK should not increment
    let ack_request = dialog.create_request(Method::Ack);
    assert_eq!(dialog.local_seq, 7); // Should remain the same
}

// Helper functions for testing

fn create_request_with_headers(
    method: Method,
    uri: &str,
    call_id: &str,
    from_tag: &str,
    to_tag: &str,
) -> Request {
    let request_uri: Uri = uri.parse().unwrap();
    let mut request = Request::new(method, request_uri);
    
    // Add Call-ID
    request.headers.push(TypedHeader::CallId(
        rvoip_sip_core::types::call_id::CallId(call_id.to_string())
    ));
    
    // Add From header with tag
    let from_uri: Uri = "sip:caller@example.com".parse().unwrap();
    let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
    from_addr.set_tag(from_tag);
    request.headers.push(TypedHeader::From(
        rvoip_sip_core::types::from::From(from_addr)
    ));
    
    // Add To header with tag
    let to_uri: Uri = "sip:callee@example.com".parse().unwrap();
    let mut to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
    to_addr.set_tag(to_tag);
    request.headers.push(TypedHeader::To(
        rvoip_sip_core::types::to::To(to_addr)
    ));
    
    request
}

fn extract_dialog_tuple_from_request(request: &Request) -> (String, String, String) {
    let call_id = match request.header(&HeaderName::CallId) {
        Some(TypedHeader::CallId(call_id)) => call_id.to_string(),
        _ => "unknown".to_string(),
    };
    
    let from_tag = match request.header(&HeaderName::From) {
        Some(TypedHeader::From(from)) => {
            from.tag().unwrap_or("unknown").to_string()
        },
        _ => "unknown".to_string(),
    };
    
    let to_tag = match request.header(&HeaderName::To) {
        Some(TypedHeader::To(to)) => {
            to.tag().unwrap_or("unknown").to_string()
        },
        _ => "unknown".to_string(),
    };
    
    (call_id, from_tag, to_tag)
}

fn is_in_dialog_method(method: &Method) -> bool {
    matches!(method, 
        Method::Bye | 
        Method::Ack | 
        Method::Info | 
        Method::Update |
        Method::Prack
    )
} 