//! Alice - Simple peer that makes a call and exchanges audio
//! 
//! This shows how simple it is to use the SimplePeer API as a caller (UAC)

use rvoip_session_core::api::{SimplePeer, Result};

mod audio_utils;

#[tokio::main]
async fn main() -> Result<()> {
    println!("📞 Alice starting on port 5060...");
    
    // Create peer
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5060)
        .await?;
    
    // Give Bob time to start
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    println!("📞 Alice calling Bob...");
    
    // Make call
    let mut call = alice.call("bob@127.0.0.1")
        .port(5061)
        .await?;
    
    // Wait for call to be active
    while !call.is_active().await {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    
    println!("✅ Call connected!");
    
    // Get audio channels (now async - waits for media session readiness)
    let (tx, rx) = call.audio_channels().await?;
    
    // Exchange audio (440Hz tone for Alice)
    audio_utils::exchange_audio(tx, rx, 440.0, "alice").await?;
    
    // Hang up
    call.hangup().await?;
    alice.shutdown().await?;
    
    println!("👋 Alice done");
    Ok(())
}