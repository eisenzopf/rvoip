//! SIP REGISTER Without Auto-Response Demo
//!
//! This example demonstrates that dialog-core no longer auto-responds to REGISTER
//! requests. Instead, the CallCenterEngine processes them and sends proper responses.

use anyhow::Result;
use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_dialog_core=debug")
        .init();

    println!("🚀 SIP REGISTER Without Auto-Response Demo\n");

    // Create configuration
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "127.0.0.1:5060".parse()?;
    
    // Create CallCenterEngine with in-memory database
    println!("🔧 Creating CallCenterEngine...");
    let engine = CallCenterEngine::new(config, Some(":memory:".to_string())).await?;
    
    // Start event monitoring to handle REGISTER requests
    println!("📡 Starting event monitoring for REGISTER requests...");
    engine.clone().start_event_monitoring().await?;
    
    println!("\n✅ System ready!");
    println!("\n📋 Current Behavior:");
    println!("  - dialog-core is configured WITHOUT auto_register_response");
    println!("  - REGISTER requests are forwarded to CallCenterEngine");
    println!("  - CallCenterEngine processes with SipRegistrar");
    println!("  - Proper SIP responses are sent back\n");
    
    println!("📝 When a REGISTER arrives:");
    println!("  1. dialog-core receives it but doesn't auto-respond");
    println!("  2. Event flows: dialog-core → session-core → CallCenterEngine");
    println!("  3. SipRegistrar processes the registration");
    println!("  4. CallCenterEngine sends proper 200 OK with Expires header");
    println!("  5. Agent's endpoint receives the correct response\n");
    
    println!("🔍 Benefits:");
    println!("  - Can validate agent credentials before accepting");
    println!("  - Can send proper Contact headers in response");
    println!("  - Can reject invalid registrations (404, 401, etc.)");
    println!("  - Registration state is properly tracked\n");
    
    println!("⏳ System running. Send REGISTER to port 5060...");
    println!("   Press Ctrl+C to stop\n");
    
    // Keep the system running
    loop {
        sleep(Duration::from_secs(60)).await;
        println!("⏰ Still listening for REGISTER requests...");
    }
} 