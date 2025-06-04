//! Dialog recovery example
//!
//! This example demonstrates dialog recovery mechanisms in the face of
//! network failures and other issues.

use std::net::SocketAddr;
use tokio::time::{sleep, Duration};
use tracing::{info, warn, Level};
use tracing_subscriber;

use rvoip_dialog_core::{DialogError, SessionCoordinationEvent, Dialog};

#[tokio::main]
async fn main() -> Result<(), DialogError> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    info!("Starting dialog recovery example");

    // Configure local address for this dialog manager
    let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    info!("Using local address: {}", local_addr);

    // Note: This example demonstrates dialog recovery concepts
    // but requires proper transport setup for full functionality
    info!("Dialog recovery example - demonstrating recovery state management");

    // Set up session coordination channel
    let (_session_tx, mut session_rx) = tokio::sync::mpsc::channel::<SessionCoordinationEvent>(100);

    info!("Session coordination channel created for recovery testing");

    // Create a test dialog to demonstrate recovery
    let mut test_dialog = Dialog::new(
        "recovery-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );

    info!("Created test dialog: {} (state: {:?})", test_dialog.id, test_dialog.state);

    // Demonstrate recovery state transitions
    info!("Dialog recovery capabilities:");
    info!("  - Initial state: {:?}", test_dialog.state);
    info!("  - Can enter recovery: {}", !test_dialog.is_recovering());
    info!("  - Can be terminated: {}", !test_dialog.is_terminated());

    // Simulate a network failure by entering recovery mode
    warn!("Simulating network failure...");
    test_dialog.enter_recovery_mode("Simulated network timeout");
    
    info!("Dialog state after failure: {:?}", test_dialog.state);
    info!("Dialog is in recovery mode: {}", test_dialog.is_recovering());

    // Spawn a task to handle session coordination events
    let event_handler = tokio::spawn(async move {
        let mut event_count = 0;
        while let Some(event) = session_rx.recv().await {
            event_count += 1;
            match event {
                SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
                    info!("[{}] Incoming call for dialog: {}", event_count, dialog_id);
                },
                SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
                    info!("[{}] Call terminated for dialog: {} - {}", event_count, dialog_id, reason);
                },
                SessionCoordinationEvent::DialogStateChanged { dialog_id, new_state, previous_state } => {
                    info!("[{}] Dialog {} state changed: {} -> {}", event_count, dialog_id, previous_state, new_state);
                },
                _ => {
                    info!("[{}] Other session coordination event: {:?}", event_count, event);
                }
            }
        }
    });

    // Simulate recovery process
    sleep(Duration::from_secs(2)).await;
    
    info!("Attempting dialog recovery...");
    info!("Recovery process would involve:");
    info!("  1. Detecting the failure condition");
    info!("  2. Entering recovery mode (already done)");
    info!("  3. Re-establishing network connectivity");
    info!("  4. Validating dialog state");
    info!("  5. Completing recovery");

    if test_dialog.complete_recovery() {
        info!("Dialog recovery successful!");
        info!("Dialog state after recovery: {:?}", test_dialog.state);
    } else {
        warn!("Dialog recovery failed - dialog not in recovery mode");
    }

    // Demonstrate recovery statistics
    info!("Recovery statistics:");
    info!("  - Recovery attempts: {}", test_dialog.recovery_attempts);
    info!("  - Recovery reason: {:?}", test_dialog.recovery_reason);
    info!("  - Recovered at: {:?}", test_dialog.recovered_at);

    // Let the example run for a bit more
    sleep(Duration::from_secs(2)).await;

    // Demonstrate final termination
    test_dialog.terminate();
    info!("Dialog terminated: {} (state: {:?})", test_dialog.is_terminated(), test_dialog.state);

    // Cancel the event handler
    event_handler.abort();

    info!("Dialog recovery example completed");
    info!("With DialogManager::new(transaction_manager, local_addr), recovery would integrate with:");
    info!("  - Transaction layer for network monitoring");
    info!("  - Session layer for state coordination");
    info!("  - Transport layer for connectivity detection");
    
    Ok(())
} 