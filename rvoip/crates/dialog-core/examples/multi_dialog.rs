//! Multiple dialog management example
//!
//! This example demonstrates managing multiple concurrent SIP dialogs
//! using the dialog-core crate.

use std::net::SocketAddr;
use tokio::time::{sleep, Duration};
use tracing::{info, Level};
use tracing_subscriber;

use rvoip_dialog_core::{DialogError, SessionCoordinationEvent, Dialog};

#[tokio::main]
async fn main() -> Result<(), DialogError> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    info!("Starting multi-dialog example");

    // Configure local address for this dialog manager
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    info!("Using local address: {}", local_addr);

    // Note: This example demonstrates API usage for multiple dialogs
    // but requires proper transport setup for full functionality
    info!("Multi-dialog example - configuration and dialog creation demo");

    // Set up session coordination channel
    let (_session_tx, mut session_rx) = tokio::sync::mpsc::channel::<SessionCoordinationEvent>(100);

    info!("Session coordination channel created for multi-dialog management");

    // Create multiple test dialogs to demonstrate structure
    let dialog1 = Dialog::new(
        "call-1".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag-1".to_string()),
        Some("bob-tag-1".to_string()),
        true,
    );

    let dialog2 = Dialog::new(
        "call-2".to_string(),
        "sip:charlie@example.com".parse().unwrap(),
        "sip:david@example.com".parse().unwrap(),
        Some("charlie-tag-1".to_string()),
        Some("david-tag-1".to_string()),
        false,
    );

    let dialog3 = Dialog::new(
        "call-3".to_string(),
        "sip:eve@example.com".parse().unwrap(),
        "sip:frank@example.com".parse().unwrap(),
        Some("eve-tag-1".to_string()),
        Some("frank-tag-1".to_string()),
        true,
    );

    info!("Created example dialogs:");
    info!("  Dialog 1: {} (initiator: {}, state: {:?})", dialog1.id, dialog1.is_initiator, dialog1.state);
    info!("  Dialog 2: {} (initiator: {}, state: {:?})", dialog2.id, dialog2.is_initiator, dialog2.state);
    info!("  Dialog 3: {} (initiator: {}, state: {:?})", dialog3.id, dialog3.is_initiator, dialog3.state);

    // Demonstrate dialog ID extraction
    if let Some(tuple1) = dialog1.dialog_id_tuple() {
        info!("Dialog 1 tuple: Call-ID={}, Local-tag={}, Remote-tag={}", 
              tuple1.0, tuple1.1, tuple1.2);
    }

    // Spawn a task to handle session coordination events
    let event_handler = tokio::spawn(async move {
        let mut event_count = 0;
        while let Some(event) = session_rx.recv().await {
            event_count += 1;
            match event {
                SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
                    info!("[{}] Incoming call for dialog: {}", event_count, dialog_id);
                },
                SessionCoordinationEvent::CallAnswered { dialog_id, .. } => {
                    info!("[{}] Call answered for dialog: {}", event_count, dialog_id);
                },
                SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
                    info!("[{}] Call terminated for dialog: {} - {}", event_count, dialog_id, reason);
                },
                _ => {
                    info!("[{}] Other session coordination event: {:?}", event_count, event);
                }
            }
        }
    });

    // Simulate dialog lifecycle for multiple dialogs
    info!("Demonstrating multi-dialog management concepts...");
    
    // Show that dialogs can run concurrently
    sleep(Duration::from_secs(1)).await;
    info!("All dialogs would be active concurrently");

    // Demonstrate dialog operations that would be available
    info!("Available operations with DialogManager::new(transaction_manager, local_addr):");
    info!("  - dialog_manager.create_dialog(&request)");
    info!("  - dialog_manager.store_dialog(dialog)");
    info!("  - dialog_manager.get_dialog(&dialog_id)");
    info!("  - dialog_manager.terminate_dialog(&dialog_id)");
    info!("  - dialog_manager.list_dialogs()");
    info!("  - dialog_manager.send_request(&dialog_id, method, body)");

    // Show state information
    info!("Dialog states in this example:");
    info!("  Dialog 1: {:?}", dialog1.state);
    info!("  Dialog 2: {:?}", dialog2.state);
    info!("  Dialog 3: {:?}", dialog3.state);

    // Demonstrate transaction-core integration
    info!("With proper setup, the DialogManager would:");
    info!("  - Use transaction-core helpers for SIP message creation");
    info!("  - Delegate all transport concerns to transaction-core");
    info!("  - Use configured local address: {}", local_addr);
    info!("  - Maintain RFC 3261 dialog state compliance");

    sleep(Duration::from_secs(2)).await;
    
    // Cancel the event handler
    event_handler.abort();

    info!("Multi-dialog example completed");
    info!("To fully run this example, implement proper TransactionManager with transport");
    Ok(())
} 