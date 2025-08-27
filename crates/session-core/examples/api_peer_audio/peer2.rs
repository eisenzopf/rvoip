//! Bob - Simple peer that accepts a call and exchanges audio
//! 
//! This shows how simple it is to use the SimplePeer API as a receiver (UAS)

use rvoip_session_core::api::{SimplePeer, Result};

mod audio_utils;

#[tokio::main]
async fn main() -> Result<()> {
    println!("📞 Bob starting on port 5061...");
    
    // Create peer
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5061)
        .await?;
    
    println!("⏳ Bob waiting for incoming call...");
    
    // Wait for incoming call
    let incoming = bob.next_incoming().await
        .ok_or_else(|| rvoip_session_core::errors::SessionError::Other("No incoming call".into()))?;
    
    println!("📞 Bob received call from: {}", incoming.from);
    
    // Accept the call
    let mut call = incoming.accept().await?;
    
    println!("✅ Call accepted!");
    
    // Get audio channels (now async - waits for media session readiness)
    let (tx, rx) = call.audio_channels().await?;
    
    // Exchange audio (880Hz tone for Bob)
    audio_utils::exchange_audio(tx, rx, 880.0, "bob").await?;
    
    // Hang up
    call.hangup().await?;
    bob.shutdown().await?;
    
    println!("👋 Bob done");
    Ok(())
}