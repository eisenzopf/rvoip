use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, Level};
use tracing_subscriber;

use rvoip_session_core::{
    SessionId, TransferId, TransferType,
    errors::Error,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("🚀 Starting REFER Method Call Transfer Demo");
    info!("📋 This demo showcases the REFER method implementation for SIP call transfers");

    // Demo 1: Transfer Types and IDs
    info!("\n🔄 Demo 1: Transfer Types and Identifiers");
    demo_transfer_types().await?;

    // Demo 2: Transfer State Management
    info!("\n🔄 Demo 2: Transfer State Management");
    demo_transfer_states().await?;

    // Demo 3: REFER Request Building
    info!("\n🔄 Demo 3: REFER Request Building");
    demo_refer_request_building().await?;

    // Demo 4: Transfer Event Types
    info!("\n🔄 Demo 4: Transfer Event Types");
    demo_transfer_events().await?;

    info!("\n🎉 REFER Method Demo completed successfully!");
    info!("📊 All transfer scenarios demonstrated");

    Ok(())
}

async fn demo_transfer_types() -> Result<(), Error> {
    info!("📞 Demonstrating transfer types...");
    
    // Create session and transfer IDs
    let session_id = SessionId::new();
    let transfer_id = TransferId::new();
    
    info!("   📋 Session ID: {}", session_id);
    info!("   🆔 Transfer ID: {}", transfer_id);
    
    // Demonstrate different transfer types
    let transfer_types = vec![
        TransferType::Blind,
        TransferType::Attended,
        TransferType::Consultative,
    ];
    
    for transfer_type in transfer_types {
        info!("   🔄 Transfer Type: {:?}", transfer_type);
        
        match transfer_type {
            TransferType::Blind => {
                info!("      📝 Blind transfer: Direct transfer without consultation");
                info!("      📨 Uses simple Refer-To header with target URI");
            },
            TransferType::Attended => {
                info!("      📝 Attended transfer: Transfer after consultation");
                info!("      📨 Uses Refer-To header with Replaces parameter");
            },
            TransferType::Consultative => {
                info!("      📝 Consultative transfer: Transfer with consultation session");
                info!("      📨 Uses consultation session for transfer coordination");
            }
        }
    }
    
    Ok(())
}

async fn demo_transfer_states() -> Result<(), Error> {
    info!("🔧 Demonstrating transfer state management...");
    
    // Simulate transfer state progression
    let states = vec![
        "Initiated - Transfer request created",
        "Accepted - 202 Accepted response received",
        "Progress - NOTIFY with progress updates",
        "Confirmed - Transfer completed successfully",
    ];
    
    for (i, state) in states.iter().enumerate() {
        sleep(Duration::from_millis(300)).await;
        info!("   📊 Step {}: {}", i + 1, state);
    }
    
    // Demonstrate error scenarios
    info!("   ❌ Error scenarios:");
    info!("      🚫 Failed - Transfer rejected or failed");
    info!("      ⏰ Timeout - Transfer timed out");
    info!("      🔄 Cancelled - Transfer cancelled by user");
    
    Ok(())
}

async fn demo_refer_request_building() -> Result<(), Error> {
    info!("🔨 Demonstrating REFER request building...");
    
    // Simulate building different types of REFER requests
    info!("   📨 Building REFER requests:");
    
    // Blind transfer REFER
    let blind_target = "sip:carol@example.com";
    info!("   🎯 Blind Transfer REFER:");
    info!("      📋 Method: REFER");
    info!("      📋 Request-URI: sip:bob@example.com");
    info!("      📋 Refer-To: {}", blind_target);
    info!("      📋 Referred-By: sip:alice@example.com");
    
    sleep(Duration::from_millis(200)).await;
    
    // Attended transfer REFER
    let attended_target = "sip:david@example.com?Replaces=call123%40example.com%3Bto-tag%3Dtag1%3Bfrom-tag%3Dtag2";
    info!("   🎯 Attended Transfer REFER:");
    info!("      📋 Method: REFER");
    info!("      📋 Request-URI: sip:bob@example.com");
    info!("      📋 Refer-To: {}", attended_target);
    info!("      📋 Referred-By: sip:alice@example.com");
    
    sleep(Duration::from_millis(200)).await;
    
    // NOTIFY for transfer progress
    info!("   📨 NOTIFY for transfer progress:");
    info!("      📋 Method: NOTIFY");
    info!("      📋 Content-Type: message/sipfrag");
    info!("      📋 Body: SIP/2.0 200 OK");
    
    Ok(())
}

async fn demo_transfer_events() -> Result<(), Error> {
    info!("📡 Demonstrating transfer event types...");
    
    let session_id = SessionId::new();
    let transfer_id = TransferId::new();
    
    // Simulate transfer event sequence
    let events = vec![
        ("TransferInitiated", format!("Transfer {} initiated for session {}", transfer_id, session_id)),
        ("TransferAccepted", format!("Transfer {} accepted", transfer_id)),
        ("TransferProgress", format!("Transfer {} progress: 180 Ringing", transfer_id)),
        ("TransferProgress", format!("Transfer {} progress: 183 Session Progress", transfer_id)),
        ("TransferCompleted", format!("Transfer {} completed successfully", transfer_id)),
    ];
    
    for (event_type, description) in events {
        sleep(Duration::from_millis(400)).await;
        info!("   📡 Event: {} - {}", event_type, description);
    }
    
    // Demonstrate consultation events
    info!("   📞 Consultation Events:");
    let consultation_id = SessionId::new();
    
    sleep(Duration::from_millis(200)).await;
    info!("   📡 Event: ConsultationCallCreated - Consultation {} created", consultation_id);
    
    sleep(Duration::from_millis(200)).await;
    info!("   📡 Event: ConsultationCallCompleted - Consultation {} completed", consultation_id);
    
    // Demonstrate error events
    info!("   ❌ Error Events:");
    sleep(Duration::from_millis(200)).await;
    info!("   📡 Event: TransferFailed - Transfer failed: 486 Busy Here");
    
    Ok(())
}

// Additional helper functions to demonstrate the REFER method concepts

fn demonstrate_refer_to_header_formats() {
    info!("📋 REFER-TO Header Formats:");
    
    // Simple URI
    info!("   🔹 Simple: <sip:target@example.com>");
    
    // With display name
    info!("   🔹 With display name: \"Target User\" <sip:target@example.com>");
    
    // With method parameter
    info!("   🔹 With method: <sip:target@example.com;method=INVITE>");
    
    // With Replaces for attended transfer
    info!("   🔹 With Replaces: <sip:target@example.com?Replaces=call123%40example.com%3Bto-tag%3Dtag1%3Bfrom-tag%3Dtag2>");
}

fn demonstrate_transfer_scenarios() {
    info!("🎭 Transfer Scenarios:");
    
    info!("   📞 Scenario 1: Basic Blind Transfer");
    info!("      1. Alice calls Bob");
    info!("      2. Bob wants to transfer to Carol");
    info!("      3. Bob sends REFER to Alice with Refer-To: Carol");
    info!("      4. Alice calls Carol and hangs up with Bob");
    
    info!("   📞 Scenario 2: Attended Transfer");
    info!("      1. Alice calls Bob");
    info!("      2. Bob calls Carol (consultation)");
    info!("      3. Bob sends REFER to Alice with Replaces header");
    info!("      4. Alice replaces Bob's call with Carol");
    
    info!("   📞 Scenario 3: Transfer with Progress");
    info!("      1. Transfer initiated");
    info!("      2. 202 Accepted response");
    info!("      3. NOTIFY with 100 Trying");
    info!("      4. NOTIFY with 180 Ringing");
    info!("      5. NOTIFY with 200 OK (success)");
} 