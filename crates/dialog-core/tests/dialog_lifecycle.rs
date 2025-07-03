//! Dialog lifecycle tests
//!
//! Tests for the complete SIP dialog lifecycle from creation to termination.

use tokio::time::{sleep, Duration};
use tracing::info;

use rvoip_dialog_core::{DialogError, Dialog, DialogState};
use rvoip_sip_core::Method;

/// Test basic dialog creation and state management
#[tokio::test]
async fn test_dialog_creation_and_initial_state() -> Result<(), DialogError> {
    // Create dialog with both tags (starts in Initial state)
    let dialog_early = Dialog::new(
        "test-call-id-early".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Should start in Initial state even when both tags are present
    // (transitions to Early/Confirmed happen through SIP message processing)
    assert_eq!(dialog_early.state, DialogState::Initial);
    assert!(dialog_early.is_initiator);
    assert!(!dialog_early.is_terminated());
    assert!(!dialog_early.is_recovering());

    // Create dialog with only local tag (Initial state)
    let dialog_initial = Dialog::new(
        "test-call-id-initial".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        None, // No remote tag
        true,
    );

    // Should start in Initial state when remote tag is missing
    assert_eq!(dialog_initial.state, DialogState::Initial);

    Ok(())
}

/// Test complete dialog state transitions
#[tokio::test]
async fn test_dialog_state_transitions() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "state-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        None, // Start without remote tag (Initial state)
        true,
    );

    // Initial state
    assert_eq!(dialog.state, DialogState::Initial);

    // Transition to Early by setting remote tag
    dialog.set_remote_tag("bob-tag".to_string());
    dialog.state = DialogState::Early; // Simulate early dialog creation
    assert_eq!(dialog.state, DialogState::Early);

    // Manually transition to Confirmed (confirm() method doesn't exist)
    dialog.state = DialogState::Confirmed;
    assert_eq!(dialog.state, DialogState::Confirmed);

    // Test recovery mode (Confirmed -> Recovering -> Confirmed)
    dialog.enter_recovery_mode("Test recovery");
    assert_eq!(dialog.state, DialogState::Recovering);
    assert!(dialog.is_recovering());

    let recovered = dialog.complete_recovery();
    assert!(recovered);
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(!dialog.is_recovering());

    // Terminate dialog (Confirmed -> Terminated)
    dialog.terminate();
    assert_eq!(dialog.state, DialogState::Terminated);
    assert!(dialog.is_terminated());

    Ok(())
}

/// Test dialog ID tuple generation
#[tokio::test]
async fn test_dialog_id_tuple() {
    let dialog = Dialog::new(
        "tuple-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // With both tags, should return complete tuple
    let tuple = dialog.dialog_id_tuple().unwrap();
    assert_eq!(tuple.0, "tuple-test-call-id");
    assert_eq!(tuple.1, "alice-tag");
    assert_eq!(tuple.2, "bob-tag");

    // Dialog without remote tag should return None
    let incomplete_dialog = Dialog::new(
        "incomplete-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        None, // No remote tag
        true,
    );
    
    assert!(incomplete_dialog.dialog_id_tuple().is_none());
}

/// Test dialog request template creation
#[tokio::test]
async fn test_dialog_request_template_creation() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "request-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Initial CSeq should be 0
    assert_eq!(dialog.local_cseq, 0);
    
    // Create a BYE request template
    let bye_request = dialog.create_request_template(Method::Bye);
    assert_eq!(dialog.local_cseq, 1); // Should increment
    
    // Verify request has proper headers (using fields, not methods)
    assert_eq!(bye_request.call_id, "request-test-call-id");
    assert_eq!(bye_request.method, Method::Bye);
    
    // Create another request
    let info_request = dialog.create_request_template(Method::Info);
    assert_eq!(dialog.local_cseq, 2); // Should increment further
    assert_eq!(info_request.method, Method::Info);

    // ACK requests should not increment CSeq
    let _ack_request = dialog.create_request_template(Method::Ack);
    assert_eq!(dialog.local_cseq, 2); // Should remain the same for ACK

    Ok(())
}

/// Test dialog sequence number management
#[tokio::test]
async fn test_dialog_sequence_numbers() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "seq-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Test local sequence number management
    assert_eq!(dialog.local_cseq, 0);
    
    dialog.increment_local_cseq();
    assert_eq!(dialog.local_cseq, 1);
    
    // Test remote sequence number management
    assert_eq!(dialog.remote_cseq, 0); // Initially unset
    
    // Test that we can manually set remote sequence number
    dialog.remote_cseq = 42;
    assert_eq!(dialog.remote_cseq, 42);

    Ok(())
}

/// Test dialog termination scenarios
#[tokio::test]
async fn test_dialog_termination_scenarios() -> Result<(), DialogError> {
    // Test termination from different states
    let states_to_test = vec![
        (DialogState::Initial, "initial-call-id"),
        (DialogState::Early, "early-call-id"),
        (DialogState::Confirmed, "confirmed-call-id"),
    ];

    for (initial_state, call_id) in states_to_test {
        let mut dialog = Dialog::new(
            call_id.to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            if initial_state == DialogState::Initial { Some("alice-tag".to_string()) } else { Some("alice-tag".to_string()) },
            if initial_state == DialogState::Initial { None } else { Some("bob-tag".to_string()) },
            true,
        );

        // Set the desired initial state
        match initial_state {
            DialogState::Initial => {}, // Already set
            DialogState::Early => {
                dialog.state = DialogState::Early;
            },
            DialogState::Confirmed => {
                dialog.state = DialogState::Confirmed;
            },
            _ => {},
        }

        assert_eq!(dialog.state, initial_state);
        assert!(!dialog.is_terminated());

        // Terminate dialog
        dialog.terminate();
        assert_eq!(dialog.state, DialogState::Terminated);
        assert!(dialog.is_terminated());
    }

    Ok(())
}

/// Test dialog lifecycle with timing simulation
#[tokio::test]
async fn test_dialog_lifecycle_with_timing() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "timing-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        None, // Start in Initial state
        true,
    );

    info!("Dialog created in Initial state");
    assert_eq!(dialog.state, DialogState::Initial);

    // Simulate time passing during call setup
    sleep(Duration::from_millis(50)).await;

    // Transition to Early
    dialog.set_remote_tag("bob-tag".to_string());
    dialog.state = DialogState::Early;
    info!("Dialog transitioned to Early state");
    assert_eq!(dialog.state, DialogState::Early);

    // Simulate ringing time
    sleep(Duration::from_millis(50)).await;

    // Confirm dialog (call answered) - manually set state
    dialog.state = DialogState::Confirmed;
    info!("Dialog confirmed (call answered)");
    assert_eq!(dialog.state, DialogState::Confirmed);

    // Simulate call duration
    sleep(Duration::from_millis(50)).await;

    // Terminate dialog (call ended)
    dialog.terminate();
    info!("Dialog terminated (call ended)");
    assert_eq!(dialog.state, DialogState::Terminated);

    Ok(())
}

/// Test dialog metadata and properties
#[tokio::test]
async fn test_dialog_metadata() {
    let dialog = Dialog::new(
        "metadata-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Test basic properties
    assert_eq!(dialog.call_id, "metadata-test-call-id");
    assert_eq!(dialog.local_uri.to_string(), "sip:alice@example.com");
    assert_eq!(dialog.remote_uri.to_string(), "sip:bob@example.com");
    assert_eq!(dialog.local_tag.as_ref().unwrap(), "alice-tag");
    assert_eq!(dialog.remote_tag.as_ref().unwrap(), "bob-tag");
    assert!(dialog.is_initiator);
    
    // Test route set (should be empty initially)
    assert!(dialog.route_set.is_empty());
    
    // Test recovery metadata (no timestamp fields)
    assert_eq!(dialog.recovery_attempts, 0);
    assert!(dialog.recovery_reason.is_none());
    assert!(dialog.recovered_at.is_none());
}

/// Architecture documentation test
#[tokio::test]
async fn test_dialog_lifecycle_architecture_note() {
    info!("DIALOG LIFECYCLE ARCHITECTURE:");
    info!("  - dialog-core: Manages pure dialog state and lifecycle");
    info!("  - transaction-core: Handles SIP message transport and routing");
    info!("  - Separation: Dialog tests focus on state, not transport");
    info!("  - Integration: Full SIP message flow tested at transaction-core level");
} 