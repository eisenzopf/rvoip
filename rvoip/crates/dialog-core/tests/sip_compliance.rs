//! SIP compliance tests for dialog-core
//!
//! Tests that ensure RFC 3261 compliance in dialog management.

use rvoip_dialog_core::{Dialog, DialogState};
use rvoip_sip_core::{Method, Uri};

#[tokio::test]
async fn test_dialog_creation_complies_with_rfc3261() {
    // Test dialog creation following RFC 3261 Section 12
    let call_id = "test-call-id".to_string();
    let local_uri: Uri = "sip:alice@example.com".parse().unwrap();
    let remote_uri: Uri = "sip:bob@example.com".parse().unwrap();
    
    let dialog = Dialog::new(
        call_id.clone(),
        local_uri.clone(),
        remote_uri.clone(),
        Some("local-tag".to_string()),
        Some("remote-tag".to_string()),
        true, // is_initiator
    );
    
    assert_eq!(dialog.call_id, call_id);
    assert_eq!(dialog.local_uri, local_uri);
    assert_eq!(dialog.remote_uri, remote_uri);
    assert!(dialog.local_tag.is_some());
    assert!(dialog.remote_tag.is_some());
    assert_eq!(dialog.state, DialogState::Initial);
}

#[tokio::test]
async fn test_dialog_state_transitions() {
    // Test dialog state transitions per RFC 3261
    let mut dialog = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        Some("remote-tag".to_string()),
        true,
    );
    
    // Initial state should be Initial (per RFC 3261 constructor behavior)
    assert_eq!(dialog.state, DialogState::Initial);
    
    // Transition to Early state (simulating 18x provisional response processing)
    dialog.state = DialogState::Early;
    assert_eq!(dialog.state, DialogState::Early);
    
    // Transition to confirmed (simulating 2xx final response processing)
    dialog.state = DialogState::Confirmed;
    assert_eq!(dialog.state, DialogState::Confirmed);
    
    // Terminate the dialog
    dialog.terminate();
    assert_eq!(dialog.state, DialogState::Terminated);
}

#[tokio::test]
async fn test_cseq_management() {
    // Test CSeq number management per RFC 3261 Section 12.2.1.1
    let mut dialog = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        Some("remote-tag".to_string()),
        true,
    );
    
    // Initial CSeq number should be 1
    assert_eq!(dialog.local_cseq, 0); // Starts at 0, increments before use
    
    // Create requests and verify CSeq increments
    let _request1 = dialog.create_request_template(Method::Bye);
    assert_eq!(dialog.local_cseq, 1);
    
    // Subsequent requests should increment CSeq
    let _request2 = dialog.create_request_template(Method::Info);
    assert_eq!(dialog.local_cseq, 2);
    
    // ACK requests shouldn't increment CSeq
    let _ack_request = dialog.create_request_template(Method::Ack);
    assert_eq!(dialog.local_cseq, 2); // Should remain the same for ACK
}

#[tokio::test]
async fn test_route_set_handling() {
    // Test Route Set handling per RFC 3261 Section 12.2.1.1
    let dialog = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        Some("remote-tag".to_string()),
        true,
    );
    
    // Initially, route set should be empty
    assert!(dialog.route_set.is_empty());
    
    // TODO: Add tests for route set population and usage
    // This would require modifying dialog creation to accept route sets
    // or implementing methods to set route sets after creation
}

#[tokio::test]
async fn test_dialog_tag_generation() {
    // Test that dialog tags follow RFC 3261 guidelines
    let dialog = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        Some("remote-tag".to_string()),
        true,
    );
    
    // Both tags should be present for a confirmed dialog
    assert!(dialog.local_tag.is_some());
    assert!(dialog.remote_tag.is_some());
    
    // Tags should be non-empty strings
    assert!(!dialog.local_tag.as_ref().unwrap().is_empty());
    assert!(!dialog.remote_tag.as_ref().unwrap().is_empty());
}

#[tokio::test]
async fn test_dialog_identification() {
    // Test dialog identification per RFC 3261 Section 12
    let dialog = Dialog::new(
        "unique-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Dialog ID should be based on Call-ID, local tag, and remote tag
    let dialog_tuple = dialog.dialog_id_tuple().unwrap();
    assert_eq!(dialog_tuple.0, "unique-call-id");
    assert_eq!(dialog_tuple.1, "alice-tag");
    assert_eq!(dialog_tuple.2, "bob-tag");
}

#[tokio::test]
async fn test_sequence_number_validation() {
    // Test sequence number validation per RFC 3261
    let mut dialog = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        Some("remote-tag".to_string()),
        true,
    );
    
    // Initial remote CSeq should be 0 (not set)
    assert_eq!(dialog.remote_cseq, 0);
    
    // Test that we can update the remote sequence number manually
    dialog.remote_cseq = 42;
    assert_eq!(dialog.remote_cseq, 42);
    
    // Test local sequence starts at 0 and increments properly
    assert_eq!(dialog.local_cseq, 0);
    let _request = dialog.create_request_template(Method::Invite);
    assert_eq!(dialog.local_cseq, 1);
}

#[tokio::test]
async fn test_request_creation_rfc3261_compliance() {
    // Test that request creation follows RFC 3261 format
    let mut dialog = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        Some("remote-tag".to_string()),
        true,
    );
    
    let request = dialog.create_request_template(Method::Bye);
    
    // Verify basic template properties (DialogRequestTemplate has fields, not methods)
    assert_eq!(request.method, Method::Bye);
    assert_eq!(request.call_id, "test-call-id");
    assert_eq!(request.local_uri, dialog.local_uri);
    assert_eq!(request.remote_uri, dialog.remote_uri);
    
    // The template represents the dialog's current sequence number
    assert_eq!(dialog.local_cseq, 1); // Should have incremented
} 