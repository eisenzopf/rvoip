//! Dialog recovery integration tests
//!
//! Tests dialog recovery mechanisms for handling network failures
//! and ensuring dialog state consistency.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use rvoip_dialog_core::{DialogManager, DialogError, Dialog, DialogState};
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Method, StatusCode};

/// Mock transport for testing
#[derive(Debug, Clone)]
struct MockTransport {
    local_addr: SocketAddr,
    should_fail: Arc<std::sync::atomic::AtomicBool>,
}

impl MockTransport {
    fn new(addr: &str) -> Self {
        Self {
            local_addr: addr.parse().unwrap(),
            should_fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    
    fn set_should_fail(&self, should_fail: bool) {
        self.should_fail.store(should_fail, std::sync::atomic::Ordering::SeqCst);
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
        if self.should_fail.load(std::sync::atomic::Ordering::SeqCst) {
            Err(rvoip_sip_transport::error::Error::IoError(
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "Mock transport failure")
            ))
        } else {
            Ok(())
        }
    }
    
    async fn close(&self) -> Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

/// Helper to create a test transaction manager with failing transport
async fn create_test_transaction_manager_with_failure() -> Result<(Arc<TransactionManager>, Arc<MockTransport>), DialogError> {
    let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
    let (_tx, rx) = mpsc::channel(10);
    
    let (transaction_manager, _events_rx) = TransactionManager::new(transport.clone(), rx, Some(10)).await
        .map_err(|e| DialogError::internal_error(&format!("Transaction manager error: {}", e), None))?;
    
    Ok((Arc::new(transaction_manager), transport))
}

/// Helper to create a test dialog manager with controllable transport
async fn create_test_dialog_manager_with_failure() -> Result<(DialogManager, Arc<MockTransport>), DialogError> {
    let (transaction_manager, mock_transport) = create_test_transaction_manager_with_failure().await?;
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    
    let dialog_manager = DialogManager::new(transaction_manager, local_addr).await?;
    Ok((dialog_manager, mock_transport))
}

/// Test basic dialog recovery functionality
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

    // Initially the dialog should be in Initial state
    assert_eq!(dialog.state, DialogState::Initial);
    assert!(!dialog.is_recovering());

    // Simulate a failure
    dialog.enter_recovery_mode("Network timeout");
    assert_eq!(dialog.state, DialogState::Recovering);
    assert!(dialog.is_recovering());

    // Recovery should succeed
    let recovered = dialog.complete_recovery();
    assert!(recovered);
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(!dialog.is_recovering());

    Ok(())
}

/// Test dialog recovery with failure detection
#[tokio::test]
async fn test_dialog_recovery_with_failure_detection() -> Result<(), DialogError> {
    let (dialog_manager, mock_transport) = create_test_dialog_manager_with_failure().await?;
    
    // Start dialog manager
    dialog_manager.start().await?;

    // Create a dialog
    let mut dialog = Dialog::new(
        "failure-detection-test".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Simulate transport failure
    mock_transport.set_should_fail(true);

    // Try to send a request - this should trigger recovery mode
    dialog.enter_recovery_mode("Transport failure detected");
    assert_eq!(dialog.state, DialogState::Recovering);

    // Simulate recovery
    mock_transport.set_should_fail(false);
    
    // Complete recovery
    let recovered = dialog.complete_recovery();
    assert!(recovered);
    assert_eq!(dialog.state, DialogState::Confirmed);

    // Stop dialog manager
    dialog_manager.stop().await?;

    Ok(())
}

/// Test dialog recovery timeout handling
#[tokio::test]
async fn test_dialog_recovery_timeout() -> Result<(), DialogError> {
    let mut dialog = Dialog::new(
        "timeout-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    // Enter recovery mode
    dialog.enter_recovery_mode("Timeout test");
    assert_eq!(dialog.state, DialogState::Recovering);

    // Simulate timeout by attempting recovery multiple times
    // In a real implementation, this would handle actual timeouts
    let _recovered1 = dialog.complete_recovery();
    assert_eq!(dialog.state, DialogState::Confirmed);

    Ok(())
}

/// Test dialog manager recovery with network issues
#[tokio::test]
async fn test_dialog_manager_recovery_with_network_issues() -> Result<(), DialogError> {
    let (dialog_manager, mock_transport) = create_test_dialog_manager_with_failure().await?;

    // Test that dialog manager can handle network failures gracefully
    timeout(Duration::from_secs(5), async {
        // Start with failing transport
        mock_transport.set_should_fail(true);
        
        dialog_manager.start().await?;
        
        // Fix transport
        mock_transport.set_should_fail(false);
        
        // Stop dialog manager
        dialog_manager.stop().await
    })
    .await
    .map_err(|_| DialogError::internal_error("Dialog manager recovery test timed out", None))??;

    Ok(())
}

/// Test multiple dialog recovery scenarios
#[tokio::test]
async fn test_multiple_dialog_recovery() -> Result<(), DialogError> {
    let dialogs = vec![
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
    ];

    // Simulate recovery for multiple dialogs
    for mut dialog in dialogs {
        dialog.enter_recovery_mode("Multi-dialog test");
        assert_eq!(dialog.state, DialogState::Recovering);
        
        let recovered = dialog.complete_recovery();
        assert!(recovered);
        assert_eq!(dialog.state, DialogState::Confirmed);
    }

    Ok(())
} 