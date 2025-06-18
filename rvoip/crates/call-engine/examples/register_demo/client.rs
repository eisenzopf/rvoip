//! SIP REGISTER Client Demo Using Session-Core API
//!
//! This client demonstrates sending a REGISTER request using session-core's
//! SipClient trait, which provides a clean API for non-session SIP operations.

use anyhow::Result;
use rvoip_session_core::api::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    println!("🚀 SIP REGISTER Client Demo (Using Session-Core API)\n");

    // Configuration
    let client_port = 5061; // Use different port than server
    let server_addr = "127.0.0.1:5060";
    let from_uri = "sip:agent001@callcenter.example.com";
    let contact_uri = "sip:agent001@192.168.1.100:5062";
    
    // Create SessionCoordinator with SIP client enabled
    println!("📡 Creating SessionCoordinator with SIP client support...");
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(client_port)
        .with_local_address(&format!("sip:client@127.0.0.1:{}", client_port))
        .enable_sip_client() // Enable SIP client features
        .build()
        .await?;
    
    println!("✅ Client coordinator created on port {}\n", client_port);
    
    // Start the coordinator
    SessionControl::start(&coordinator).await?;
    
    // Perform registration
    println!("📝 Sending REGISTER request...");
    println!("  From: {}", from_uri);
    println!("  Contact: {}", contact_uri);
    println!("  Expires: 3600 seconds");
    println!("  Server: {}\n", server_addr);
    
    match coordinator.register(
        &format!("sip:{}", server_addr),
        from_uri,
        contact_uri,
        3600, // 1 hour registration
    ).await {
        Ok(registration) => {
            println!("✅ Registration request sent!");
            println!("   Transaction ID: {}", registration.transaction_id);
            println!("   Expires: {} seconds", registration.expires);
            println!("   Contact: {}", registration.contact_uri);
            println!("   Registrar: {}", registration.registrar_uri);
            
            println!("\n⚠️  Note: Full implementation requires dialog-core support");
            println!("   for non-dialog requests. Currently returns mock success.\n");
        }
        Err(e) => {
            println!("❌ Registration failed: {}", e);
            return Err(e.into());
        }
    }
    
    // Wait a bit
    sleep(Duration::from_secs(2)).await;
    
    // Demonstrate de-registration
    println!("📝 Sending de-registration (expires=0)...");
    
    match coordinator.register(
        &format!("sip:{}", server_addr),
        from_uri,
        contact_uri,
        0, // De-register
    ).await {
        Ok(registration) => {
            println!("✅ De-registration request sent!");
            println!("   Transaction ID: {}", registration.transaction_id);
            println!("   Expires: {} seconds (unregistered)", registration.expires);
        }
        Err(e) => {
            println!("❌ De-registration failed: {}", e);
        }
    }
    
    // Stop the coordinator
    println!("\n🧹 Stopping coordinator...");
    SessionControl::stop(&coordinator).await?;
    
    println!("\n✅ Demo completed!");
    println!("\n📋 Next Steps:");
    println!("   1. Implement send_non_dialog_request in dialog-core");
    println!("   2. Complete the register() implementation to send real requests");
    println!("   3. Add support for authentication challenges (401/407)");
    println!("   4. Implement other SipClient methods (OPTIONS, MESSAGE, SUBSCRIBE)");
    
    Ok(())
} 