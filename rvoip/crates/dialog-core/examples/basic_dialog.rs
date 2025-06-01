//! Basic dialog management example
//!
//! This example demonstrates how to create and manage a basic SIP dialog
//! using the dialog-core crate.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::time::{sleep, Duration};
use tracing::{info, Level};
use tracing_subscriber;

use rvoip_dialog_core::{DialogManager, DialogError, SessionCoordinationEvent};
use rvoip_transaction_core::TransactionManager;

#[tokio::main]
async fn main() -> Result<(), DialogError> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    info!("Starting basic dialog example");

    // Create a mock transport for the example
    // In a real application, you would use a proper UDP/TCP transport
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    info!("Using local address: {}", local_addr);

    // For this example, we'll create a minimal setup
    // Note: This is a simplified example that may not work without proper transport setup
    // In a real application, you'd need to properly initialize the transport layer
    
    info!("Dialog manager basic example - configuration only");
    info!("Note: This example demonstrates API usage but requires transport setup for full functionality");

    // Create dialog manager with local address
    // Note: TransactionManager construction would require proper transport in real usage
    info!("Would create DialogManager with local address: {}", local_addr);
    
    // Demonstrate the new API signature
    info!("DialogManager::new() now requires:");
    info!("  1. Arc<TransactionManager> - for transaction handling");
    info!("  2. SocketAddr - local address for Via/Contact headers");
    
    // Set up session coordination channel
    let (session_tx, mut session_rx) = tokio::sync::mpsc::channel::<SessionCoordinationEvent>(100);
    
    info!("Session coordination channel created");

    // Spawn a task to handle session coordination events
    let event_handler = tokio::spawn(async move {
        while let Some(event) = session_rx.recv().await {
            match event {
                SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
                    info!("Received incoming call for dialog: {}", dialog_id);
                },
                SessionCoordinationEvent::CallAnswered { dialog_id, .. } => {
                    info!("Call answered for dialog: {}", dialog_id);
                },
                SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
                    info!("Call terminated for dialog: {} - {}", dialog_id, reason);
                },
                _ => {
                    info!("Received other session coordination event: {:?}", event);
                }
            }
        }
    });

    // Let the example run for a bit
    info!("Example configuration completed...");
    sleep(Duration::from_secs(2)).await;
    
    // Cancel the event handler
    event_handler.abort();

    info!("Basic dialog example completed");
    info!("To fully run this example, implement proper TransactionManager with transport");
    Ok(())
} 