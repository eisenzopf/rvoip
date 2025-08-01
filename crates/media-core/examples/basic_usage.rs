//! Basic usage example for media-core
//!
//! This example demonstrates how to use the MediaEngine for basic
//! media session management in a SIP server context.

use rvoip_media_core::prelude::*;
use rvoip_media_core::types::payload_types;
use rvoip_media_core::MediaSessionParams;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("🎵 Media Core Demo - Professional SIP Media Processing");
    
    // Create MediaEngine with default configuration
    let config = MediaEngineConfig::default();
    let engine = MediaEngine::new(config).await?;
    
    // Start the engine
    println!("📡 Starting MediaEngine...");
    engine.start().await?;
    println!("✅ MediaEngine started successfully!");
    
    // Get supported codec capabilities (for SDP negotiation)
    let capabilities = engine.get_supported_codecs();
    println!("🎵 Supported Codecs:");
    for codec in &capabilities {
        println!("  - {} (PT: {}, Rate: {} Hz)", 
                 codec.name, codec.payload_type, codec.clock_rate);
    }
    
    // Create media session for SIP dialog
    let dialog_id = DialogId::new("call-demo-123");
    let params = MediaSessionParams::audio_only()
        .with_preferred_codec(payload_types::PCMU)
        .with_processing_enabled(true);
    
    println!("📞 Creating media session for dialog: {}", dialog_id);
    let session = engine.create_media_session(dialog_id.clone(), params).await?;
    
    // Get session stats
    let stats = session.get_stats().await?;
    println!("📊 Session Stats: {}", stats);
    
    // Check engine status
    println!("🔧 Engine Status: {:?}", engine.state().await);
    println!("📈 Active Sessions: {}", engine.session_count().await);
    
    // Demonstrate session lifecycle
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    
    // Clean shutdown
    println!("🛑 Destroying media session...");
    engine.destroy_media_session(dialog_id).await?;
    
    println!("⏹️  Stopping MediaEngine...");
    engine.stop().await?;
    
    println!("✨ Media Core Demo completed successfully!");
    Ok(())
} 