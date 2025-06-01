//! Integration tests for dialog lifecycle management
//!
//! Tests the complete lifecycle of SIP dialogs from creation to termination.

use std::sync::Arc;
use tokio::time::{timeout, Duration};

use rvoip_dialog_core::{DialogManager, DialogError, Dialog, DialogState};
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Method, StatusCode};

/// Test basic dialog creation and termination
#[tokio::test]
async fn test_dialog_creation_and_termination() -> Result<(), DialogError> {
    // Create transaction manager
    let transaction_manager = Arc::new(
        TransactionManager::new().await
            .map_err(|e| DialogError::internal_error(&format!("Transaction manager error: {}", e), None))?
    );

    // Create dialog manager
    let dialog_manager = DialogManager::new(transaction_manager).await?;

    // Start dialog manager
    dialog_manager.start().await?;

    // Create a test dialog
    let dialog = Dialog::new(
        "test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Verify initial state
    assert_eq!(dialog.state, DialogState::Initial);
    assert!(dialog.is_initiator);
    assert!(!dialog.is_terminated());

    // Stop dialog manager
    dialog_manager.stop().await?;

    Ok(())
}

/// Test dialog state transitions
#[tokio::test]
async fn test_dialog_state_transitions() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "state-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        None, // No remote tag initially
        true,
    );

    // Initial state
    assert_eq!(dialog.state, DialogState::Initial);

    // Simulate entering recovery mode
    dialog.enter_recovery_mode("Test failure");
    assert_eq!(dialog.state, DialogState::Recovering);
    assert!(dialog.is_recovering());

    // Simulate recovery completion
    assert!(dialog.complete_recovery());
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(!dialog.is_recovering());

    // Terminate dialog
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

    let tuple = dialog.dialog_id_tuple().unwrap();
    assert_eq!(tuple.0, "tuple-test-call-id");
    assert_eq!(tuple.1, "alice-tag");
    assert_eq!(tuple.2, "bob-tag");
}

/// Test dialog request creation
#[tokio::test]
async fn test_dialog_request_creation() {
    let mut dialog = Dialog::new(
        "request-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Create a BYE request
    let request = dialog.create_request(Method::Bye);
    
    // Verify the request has the correct method
    assert_eq!(request.method, Method::Bye);
    
    // Verify sequence number was incremented
    assert_eq!(dialog.local_seq, 1);
    
    // Create another request
    let request2 = dialog.create_request(Method::Info);
    assert_eq!(request2.method, Method::Info);
    assert_eq!(dialog.local_seq, 2);
}

/// Test dialog manager lifecycle with timeout
#[tokio::test]
async fn test_dialog_manager_lifecycle_with_timeout() -> Result<(), DialogError> {
    let transaction_manager = Arc::new(
        TransactionManager::new().await
            .map_err(|e| DialogError::internal_error(&format!("Transaction manager error: {}", e), None))?
    );

    let dialog_manager = DialogManager::new(transaction_manager).await?;

    // Test start/stop with timeout to ensure it doesn't hang
    timeout(Duration::from_secs(5), async {
        dialog_manager.start().await?;
        dialog_manager.stop().await
    })
    .await
    .map_err(|_| DialogError::internal_error("Dialog manager lifecycle timed out", None))??;

    Ok(())
} 