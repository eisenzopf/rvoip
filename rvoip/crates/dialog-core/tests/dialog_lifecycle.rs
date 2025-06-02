//! Integration tests for dialog lifecycle management
//!
//! Tests the complete lifecycle of SIP dialogs from creation to termination.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::time::{timeout, Duration};
use tokio::sync::mpsc;

use rvoip_dialog_core::{DialogManager, DialogError, Dialog, DialogState};
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Method, StatusCode};

/// Mock transport for testing
#[derive(Debug, Clone)]
struct MockTransport {
    local_addr: SocketAddr,
}

impl MockTransport {
    fn new(addr: &str) -> Self {
        Self {
            local_addr: addr.parse().unwrap(),
        }
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    fn local_addr(&self) -> Result<SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn send_message(
        &self, 
        _message: rvoip_sip_core::Message, 
        _destination: SocketAddr
    ) -> Result<(), rvoip_sip_transport::error::Error> {
        // Mock implementation: just succeed
        Ok(())
    }
    
    async fn close(&self) -> Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

/// Helper to create a test transaction manager
async fn create_test_transaction_manager() -> Result<Arc<TransactionManager>, DialogError> {
    let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
    let (_tx, rx) = mpsc::channel(10);
    
    let (transaction_manager, _events_rx) = TransactionManager::new(transport, rx, Some(10)).await
        .map_err(|e| DialogError::internal_error(&format!("Transaction manager error: {}", e), None))?;
    
    Ok(Arc::new(transaction_manager))
}

/// Helper to create a test dialog manager
async fn create_test_dialog_manager() -> Result<DialogManager, DialogError> {
    let transaction_manager = create_test_transaction_manager().await?;
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    DialogManager::new(transaction_manager, local_addr).await
}

/// Test basic dialog creation and termination
#[tokio::test]
async fn test_dialog_creation_and_termination() -> Result<(), DialogError> {
    // Create dialog manager
    let dialog_manager = create_test_dialog_manager().await?;

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

/// Test dialog request creation (via dialog manager)
#[tokio::test]
async fn test_dialog_request_creation() -> Result<(), DialogError> {
    let dialog_manager = create_test_dialog_manager().await?;
    
    // Start dialog manager
    dialog_manager.start().await?;

    let mut dialog = Dialog::new(
        "request-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Note: In the new architecture, request creation should go through DialogManager
    // For now, we'll test the deprecation warning and move toward proper API usage
    
    // Create a BYE request (this will show deprecation warning)
    let _request = dialog.create_request(Method::Bye);
    
    // Verify sequence number was incremented
    assert_eq!(dialog.local_seq, 1);
    
    // Create another request
    let _request2 = dialog.create_request(Method::Info);
    assert_eq!(dialog.local_seq, 2);

    // Stop dialog manager
    dialog_manager.stop().await?;

    Ok(())
}

/// Test dialog manager lifecycle with timeout
#[tokio::test]
async fn test_dialog_manager_lifecycle_with_timeout() -> Result<(), DialogError> {
    let dialog_manager = create_test_dialog_manager().await?;

    // Test start/stop with timeout to ensure it doesn't hang
    timeout(Duration::from_secs(5), async {
        dialog_manager.start().await?;
        dialog_manager.stop().await
    })
    .await
    .map_err(|_| DialogError::internal_error("Dialog manager lifecycle timed out", None))??;

    Ok(())
} 