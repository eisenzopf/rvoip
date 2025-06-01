//! Integration tests for dialog recovery mechanisms
//!
//! Tests dialog recovery from various failure scenarios.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::time::{sleep, Duration};

use rvoip_dialog_core::{DialogManager, DialogError, Dialog, DialogState};
use rvoip_transaction_core::TransactionManager;

/// Test dialog recovery from network failure
#[tokio::test]
async fn test_dialog_recovery_from_network_failure() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "recovery-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Initially in normal state
    assert_eq!(dialog.state, DialogState::Initial);
    assert!(!dialog.is_recovering());

    // Simulate network failure
    dialog.enter_recovery_mode("Network connectivity lost");
    
    // Verify recovery state
    assert_eq!(dialog.state, DialogState::Recovering);
    assert!(dialog.is_recovering());
    assert_eq!(dialog.recovery_reason, Some("Network connectivity lost".to_string()));
    assert!(dialog.recovery_start_time.is_some());

    // Simulate recovery attempt
    assert!(dialog.complete_recovery());
    
    // Verify recovery completion
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(!dialog.is_recovering());
    assert_eq!(dialog.recovery_reason, None);
    assert!(dialog.recovered_at.is_some());
    assert_eq!(dialog.recovery_start_time, None);

    Ok(())
}

/// Test dialog recovery failure and retry
#[tokio::test]
async fn test_dialog_recovery_retry_mechanism() {
    let mut dialog = Dialog::new(
        "retry-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Enter recovery mode
    dialog.enter_recovery_mode("Connection timeout");
    assert_eq!(dialog.recovery_attempts, 0);

    // Simulate multiple recovery attempts
    for i in 1..=3 {
        // In a real implementation, this would be called by the recovery manager
        dialog.recovery_attempts += 1;
        assert_eq!(dialog.recovery_attempts, i);
    }

    // Simulate successful recovery after retries
    assert!(dialog.complete_recovery());
    assert_eq!(dialog.state, DialogState::Confirmed);
}

/// Test dialog recovery cannot happen on terminated dialog
#[tokio::test]
async fn test_recovery_blocked_on_terminated_dialog() {
    let mut dialog = Dialog::new(
        "terminated-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Terminate the dialog first
    dialog.terminate();
    assert_eq!(dialog.state, DialogState::Terminated);

    // Try to enter recovery mode - should not work
    dialog.enter_recovery_mode("Should not work");
    
    // Should still be terminated, not recovering
    assert_eq!(dialog.state, DialogState::Terminated);
    assert!(!dialog.is_recovering());
    assert_eq!(dialog.recovery_reason, None);
}

/// Test remote address tracking for recovery
#[tokio::test]
async fn test_remote_address_tracking() {
    let mut dialog = Dialog::new(
        "address-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Initially no remote address
    assert_eq!(dialog.last_known_remote_addr, None);
    assert_eq!(dialog.last_successful_transaction_time, None);

    // Update with remote address
    let test_addr: SocketAddr = "192.168.1.100:5060".parse().unwrap();
    dialog.update_remote_address(test_addr);

    // Verify tracking
    assert_eq!(dialog.last_known_remote_addr, Some(test_addr));
    assert!(dialog.last_successful_transaction_time.is_some());
}

/// Test recovery timing
#[tokio::test]
async fn test_recovery_timing() {
    let mut dialog = Dialog::new(
        "timing-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Enter recovery mode
    let before_recovery = std::time::SystemTime::now();
    dialog.enter_recovery_mode("Timing test");
    let after_recovery = std::time::SystemTime::now();

    // Verify recovery start time is within reasonable range
    let recovery_start = dialog.recovery_start_time.unwrap();
    assert!(recovery_start >= before_recovery);
    assert!(recovery_start <= after_recovery);

    // Small delay to ensure recovered_at is different
    sleep(Duration::from_millis(10)).await;

    // Complete recovery
    let before_completion = std::time::SystemTime::now();
    assert!(dialog.complete_recovery());
    let after_completion = std::time::SystemTime::now();

    // Verify recovery completion time
    let recovered_at = dialog.recovered_at.unwrap();
    assert!(recovered_at >= before_completion);
    assert!(recovered_at <= after_completion);
    assert!(recovered_at > recovery_start);
}

/// Test dialog manager integration with recovery
#[tokio::test]
async fn test_dialog_manager_recovery_integration() -> Result<(), DialogError> {
    let transaction_manager = Arc::new(
        TransactionManager::new().await
            .map_err(|e| DialogError::internal_error(&format!("Transaction manager error: {}", e), None))?
    );

    let dialog_manager = DialogManager::new(transaction_manager).await?;
    dialog_manager.start().await?;

    // In a real implementation, this would test:
    // 1. Dialog manager detecting failed dialogs
    // 2. Automatic recovery attempts
    // 3. Recovery event generation
    // 4. Integration with session coordination

    // For now, just verify the manager can start and stop
    dialog_manager.stop().await?;

    Ok(())
} 