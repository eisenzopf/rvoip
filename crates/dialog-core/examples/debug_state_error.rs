//! Debug State Error Example with Unified API
//!
//! This example demonstrates proper SIP dialog state validation and shows
//! the difference between unestablished and established dialogs using the
//! unified DialogManager architecture.

use rvoip_dialog_core::{config::DialogManagerConfig, DialogState, UnifiedDialogApi};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    info!("🔧 SIP Dialog State Validation Example - Unified API");
    info!("   Demonstrates proper dialog establishment vs. unestablished dialog errors");
    info!("   Using GLOBAL EVENTS pattern for reliable transaction handling");

    // Create unified configuration (client mode for outgoing operations)
    let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
        .with_from_uri("sip:debug@example.com")
        .build();

    let api = UnifiedDialogApi::create(config).await?;

    info!("✅ Created UnifiedDialogApi in client mode");

    // Start the API
    api.start().await?;
    info!("✅ Started dialog API for state validation testing");

    // Create dialogs for testing state validation
    let local_uri = "sip:debug@example.com";

    // Test 1: Create dialog (but don't establish it)
    info!("\n🧪 Test 1: Creating unestablished dialog");
    let dialog = api
        .create_dialog(local_uri, "sip:target@example.com")
        .await?;
    info!("✅ Created dialog: {} (state: unestablished)", dialog.id());

    // Test 2: Real SIP call establishment
    info!("\n🧪 Test 2: Making real SIP call for proper establishment");
    let call_result = api
        .make_call(local_uri, "sip:call-target@example.com", None)
        .await;

    match call_result {
        Ok(call) => {
            info!("✅ Real call initiated: {}", call.call_id());
            info!("📋 This creates a proper dialog that will be established via INVITE/200 OK");
        }
        Err(e) => {
            info!("⚠️  Call failed (expected in test environment): {}", e);
            info!("💡 In production, this would establish a real dialog");
        }
    }

    // Test 3: Demonstrate state checking
    info!("\n🧪 Test 3: Checking dialog states");
    match api.get_dialog_state(dialog.id()).await {
        Ok(state) => {
            info!("📋 Dialog {} state: {:?}", dialog.id(), state);
            match state {
                DialogState::Early => info!("   → Dialog is in early state (after INVITE sent)"),
                DialogState::Confirmed => info!("   → Dialog is confirmed (after 200 OK received)"),
                DialogState::Terminated => info!("   → Dialog is terminated"),
                _ => info!("   → Dialog is in other state"),
            }
        }
        Err(e) => {
            error!("❌ Failed to get dialog state: {}", e);
        }
    }

    // Show statistics
    let stats = api.get_stats().await;
    info!("\n📊 Final Statistics:");
    info!("   • Active dialogs: {}", stats.active_dialogs);
    info!("   • Total dialogs created: {}", stats.total_dialogs);

    // Brief pause
    sleep(Duration::from_millis(500)).await;

    // Clean up
    api.stop().await?;
    info!("✅ Stopped API");

    info!("\n🎯 === State Validation Lessons ===");
    info!("✅ Created dialogs start in unestablished state");
    info!("✅ Real calls via make_call() follow proper SIP establishment");
    info!("✅ Dialog state can be checked via get_dialog_state()");
    info!("💡 Always use proper SIP flows for dialog establishment");

    Ok(())
}
