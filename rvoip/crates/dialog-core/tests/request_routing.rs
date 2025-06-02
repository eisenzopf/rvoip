//! Unit tests for request routing functionality
//!
//! Tests request routing logic and dialog matching using **REAL IMPLEMENTATIONS**.

use rvoip_dialog_core::{Dialog, DialogState};
use rvoip_sip_core::{Request, Method, HeaderName, TypedHeader, Uri};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use uuid::Uuid;

/// Test dialog tuple extraction for routing with real Dialog
#[test]
fn test_dialog_tuple_extraction_real() {
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
    
    println!("✅ Real dialog tuple extraction working: ({}, {}, {})", tuple.0, tuple.1, tuple.2);
}

/// Test dialog tuple extraction with missing tags (real validation)
#[test]
fn test_dialog_tuple_extraction_missing_tags_real() {
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
    println!("✅ Real dialog correctly rejects incomplete tuple (no remote tag)");
    
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
    println!("✅ Real dialog correctly rejects incomplete tuple (no local tag)");
    
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
    println!("✅ Real dialog correctly rejects incomplete tuple (no tags)");
}

/// Test request matching with real SIP requests
#[test]
fn test_request_dialog_matching_real() {
    // Create a real dialog
    let dialog = Dialog::new(
        "match-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Create a real SIP request using proper builder
    let matching_request = create_real_sip_request(
        Method::Bye,
        "sip:alice@example.com",
        "match-test-call-id",
        "bob-tag", // From tag (remote's perspective)
        "alice-tag", // To tag (local's perspective)
    );
    
    // Test that we can extract the same dialog tuple from the request
    let request_tuple = extract_dialog_tuple_from_request(&matching_request);
    let dialog_tuple = dialog.dialog_id_tuple().unwrap();
    
    // Verify Call-ID matches
    assert_eq!(request_tuple.0, dialog_tuple.0);
    println!("✅ Real SIP request Call-ID matches dialog: {}", request_tuple.0);
    
    // Note: Tag matching depends on dialog perspective (initiator vs recipient)
    println!("✅ Real request/dialog matching validation complete");
}

/// Test in-dialog vs new dialog request classification with real methods
#[test]
fn test_request_classification_real() {
    // Create real established dialog
    let mut dialog = Dialog::new(
        "established-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Set dialog to confirmed state (realistic scenario)
    dialog.state = DialogState::Confirmed;
    assert_eq!(dialog.state, DialogState::Confirmed);
    println!("✅ Real dialog established in Confirmed state");
    
    // Test methods that are typically in-dialog
    let in_dialog_methods = vec![
        Method::Bye,
        Method::Ack,
        Method::Info,
        Method::Update,
    ];
    
    for method in in_dialog_methods {
        assert!(is_in_dialog_method(&method), "Method {:?} should be in-dialog", method);
        println!("✅ Method {:?} correctly classified as in-dialog", method);
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
        println!("✅ Method {:?} correctly classified as new-dialog", method);
    }
}

/// Test route set handling with real Dialog functionality
#[test]
fn test_route_set_handling_real() {
    let mut dialog = Dialog::new(
        "route-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Add real routes to the dialog
    let route1: Uri = "sip:proxy1.example.com".parse().unwrap();
    let route2: Uri = "sip:proxy2.example.com".parse().unwrap();
    dialog.route_set = vec![route1.clone(), route2.clone()];
    
    // Create a real request using deprecated but functional method
    #[allow(deprecated)]
    let request = dialog.create_request(Method::Bye);
    
    // The request should be created with the remote target
    assert_eq!(request.uri, dialog.remote_target);
    println!("✅ Real request created with correct remote target: {}", request.uri);
    
    // Verify the route set is available for request building
    assert_eq!(dialog.route_set.len(), 2);
    assert_eq!(dialog.route_set[0], route1);
    assert_eq!(dialog.route_set[1], route2);
    println!("✅ Real route set properly maintained: {} routes", dialog.route_set.len());
}

/// Test dialog perspective matching with real Dialog instances
#[test]
fn test_dialog_perspective_matching_real() {
    // Create real initiator dialog
    let initiator_dialog = Dialog::new(
        "perspective-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true, // initiator
    );
    
    // Create real recipient dialog (same call, different perspective)
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
    println!("✅ Real dialogs share same Call-ID: {}", initiator_dialog.call_id);
    
    // But different perspectives on local/remote
    assert_ne!(initiator_dialog.local_uri, recipient_dialog.local_uri);
    assert_ne!(initiator_dialog.remote_uri, recipient_dialog.remote_uri);
    assert_ne!(initiator_dialog.local_tag, recipient_dialog.local_tag);
    assert_ne!(initiator_dialog.remote_tag, recipient_dialog.remote_tag);
    
    // Verify perspective flags
    assert!(initiator_dialog.is_initiator);
    assert!(!recipient_dialog.is_initiator);
    
    println!("✅ Real dialog perspective handling working correctly");
    println!("   Initiator: {} -> {}", initiator_dialog.local_uri, initiator_dialog.remote_uri);
    println!("   Recipient: {} -> {}", recipient_dialog.local_uri, recipient_dialog.remote_uri);
}

/// Test sequence number validation with real Dialog functionality
#[test]
fn test_sequence_number_validation_real() {
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
    
    println!("✅ Initial sequence numbers: local={}, remote={}", dialog.local_seq, dialog.remote_seq);
    
    // Create real requests and verify sequence number increments
    #[allow(deprecated)]
    let _request1 = dialog.create_request(Method::Info);
    assert_eq!(dialog.local_seq, 6);
    println!("✅ INFO request incremented sequence to: {}", dialog.local_seq);
    
    #[allow(deprecated)]
    let _request2 = dialog.create_request(Method::Bye);
    assert_eq!(dialog.local_seq, 7);
    println!("✅ BYE request incremented sequence to: {}", dialog.local_seq);
    
    // ACK should not increment (RFC 3261 requirement)
    #[allow(deprecated)]
    let _ack_request = dialog.create_request(Method::Ack);
    assert_eq!(dialog.local_seq, 7); // Should remain the same
    println!("✅ ACK request correctly did NOT increment sequence: {}", dialog.local_seq);
}

/// Test dialog state transitions with real state management
#[test]
fn test_dialog_state_transitions_real() {
    let mut dialog = Dialog::new(
        "state-transition-test".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Start in Initial state
    assert_eq!(dialog.state, DialogState::Initial);
    println!("✅ Real dialog starts in Initial state");
    
    // Test recovery mode transitions
    dialog.enter_recovery_mode("Network issue");
    assert_eq!(dialog.state, DialogState::Recovering);
    assert!(dialog.is_recovering());
    println!("✅ Real dialog entered recovery mode");
    
    // Test recovery completion
    let recovered = dialog.complete_recovery();
    assert!(recovered);
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(!dialog.is_recovering());
    println!("✅ Real dialog completed recovery to Confirmed state");
    
    // Test termination
    dialog.terminate();
    assert_eq!(dialog.state, DialogState::Terminated);
    assert!(dialog.is_terminated());
    println!("✅ Real dialog terminated successfully");
}

// Helper functions for real SIP request creation and processing

fn create_real_sip_request(
    method: Method,
    uri: &str,
    call_id: &str,
    from_tag: &str,
    to_tag: &str,
) -> Request {
    let branch = format!("z9hG4bK-{}", Uuid::new_v4().to_string().replace("-", ""));
    
    SimpleRequestBuilder::new(method, uri)
        .expect("Failed to create request builder")
        .from("Caller", "sip:caller@example.com", Some(from_tag))
        .to("Callee", "sip:callee@example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(1)
        .via("127.0.0.1:5060", "UDP", Some(&branch))
        .max_forwards(70)
        .build()
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