//! Dialog recovery tests
//!
//! Tests for dialog recovery mechanisms and state preservation
//! in the face of network failures and other issues.

use tokio::time::{sleep, Duration};
use tracing::info;

use rvoip_dialog_core::{DialogError, Dialog, DialogState};

/// Test basic dialog recovery functionality without transport concerns
#[tokio::test]
async fn test_dialog_recovery_basic() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "recovery-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Initially the dialog should be in Initial state (constructor always creates Initial state)
    assert_eq!(dialog.state, DialogState::Initial);
    assert!(!dialog.is_recovering());

    // Simulate a failure by entering recovery mode
    dialog.enter_recovery_mode("Network timeout");
    assert_eq!(dialog.state, DialogState::Recovering);
    assert!(dialog.is_recovering());

    // Recovery should succeed and go to Confirmed state
    let recovered = dialog.complete_recovery();
    assert!(recovered);
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(!dialog.is_recovering());

    Ok(())
}

/// Test dialog recovery state transitions
#[tokio::test]
async fn test_dialog_recovery_state_transitions() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "state-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Test multiple recovery cycles
    for i in 0..3 {
        let reason = format!("Recovery test cycle {}", i + 1);
        
        // Enter recovery mode
        dialog.enter_recovery_mode(&reason);
        assert_eq!(dialog.state, DialogState::Recovering);
        assert!(dialog.is_recovering());
        
        // Complete recovery
        let recovered = dialog.complete_recovery();
        assert!(recovered);
        assert_eq!(dialog.state, DialogState::Confirmed);
        assert!(!dialog.is_recovering());
    }

    Ok(())
}

/// Test dialog recovery failure scenarios
#[tokio::test]
async fn test_dialog_recovery_failure_scenarios() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "failure-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Confirm the dialog first
    dialog.state = DialogState::Confirmed;
    assert_eq!(dialog.state, DialogState::Confirmed);

    // Enter recovery mode
    dialog.enter_recovery_mode("Simulated failure");
    assert_eq!(dialog.state, DialogState::Recovering);

    // Simulate failed recovery by terminating dialog instead
    dialog.terminate();
    assert_eq!(dialog.state, DialogState::Terminated);
    assert!(dialog.is_terminated());

    // Recovery should not be possible after termination
    let recovered = dialog.complete_recovery();
    assert!(!recovered); // Should fail because dialog is terminated
    assert_eq!(dialog.state, DialogState::Terminated);

    Ok(())
}

/// Test multiple dialog recovery scenarios
#[tokio::test]
async fn test_multiple_dialog_recovery() -> Result<(), DialogError> {
    let mut dialogs = vec![
        Dialog::new(
            "multi-recovery-1".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("alice-tag-1".to_string()),
            Some("bob-tag-1".to_string()),
            true,
        ),
        Dialog::new(
            "multi-recovery-2".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:carol@example.com".parse().unwrap(),
            Some("alice-tag-2".to_string()),
            Some("carol-tag-2".to_string()),
            true,
        ),
        Dialog::new(
            "multi-recovery-3".to_string(),
            "sip:bob@example.com".parse().unwrap(),
            "sip:david@example.com".parse().unwrap(),
            Some("bob-tag-3".to_string()),
            Some("david-tag-3".to_string()),
            false, // Not initiator
        ),
    ];

    // Simulate recovery for multiple dialogs
    for dialog in &mut dialogs {
        let call_id = dialog.call_id.clone();
        
        // Enter recovery mode
        dialog.enter_recovery_mode("Multi-dialog test");
        assert_eq!(dialog.state, DialogState::Recovering);
        info!("Dialog {} entered recovery mode", call_id);
        
        // Complete recovery
        let recovered = dialog.complete_recovery();
        assert!(recovered);
        assert_eq!(dialog.state, DialogState::Confirmed);
        info!("Dialog {} recovered successfully", call_id);
    }

    Ok(())
}

/// Test dialog recovery metadata tracking
#[tokio::test]
async fn test_dialog_recovery_metadata() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "metadata-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Check initial recovery metadata
    assert_eq!(dialog.recovery_attempts, 0);
    assert!(dialog.recovery_reason.is_none());
    assert!(dialog.recovered_at.is_none());

    // Enter recovery mode with specific reason
    let failure_reason = "Network timeout detected";
    dialog.enter_recovery_mode(failure_reason);
    
    // Check recovery metadata is updated
    assert_eq!(dialog.recovery_attempts, 0); // Still 0, not auto-incremented
    assert_eq!(dialog.recovery_reason.as_ref().unwrap(), failure_reason);
    assert!(dialog.recovered_at.is_none()); // Not recovered yet

    // Complete recovery
    let recovered = dialog.complete_recovery();
    assert!(recovered);
    
    // Check that recovery completion time is recorded
    assert!(dialog.recovered_at.is_some());
    info!("Dialog recovered at: {:?}", dialog.recovered_at);

    Ok(())
}

/// Test recovery with async delay simulation
#[tokio::test]
async fn test_dialog_recovery_with_delay() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "delay-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Enter recovery mode
    dialog.enter_recovery_mode("Simulated network delay");
    assert_eq!(dialog.state, DialogState::Recovering);

    // Simulate time passing during recovery process
    sleep(Duration::from_millis(100)).await;

    // Complete recovery after delay
    let recovered = dialog.complete_recovery();
    assert!(recovered);
    assert_eq!(dialog.state, DialogState::Confirmed);

    Ok(())
}

/// Integration test note: For full recovery testing with network failures,
/// use transaction-core integration tests that can simulate transport failures
/// at the proper architectural layer.
#[tokio::test]
async fn test_recovery_architecture_note() {
    // This test documents the proper architectural approach
    info!("ARCHITECTURAL NOTE:");
    info!("  - dialog-core: Manages dialog state recovery logic");
    info!("  - transaction-core: Handles network failure detection and transport recovery");
    info!("  - Full integration tests should be at transaction-core level");
    info!("  - dialog-core tests focus on pure dialog state management");
} 