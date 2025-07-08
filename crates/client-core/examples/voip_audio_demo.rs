//! VoIP Audio Demo
//!
//! This example demonstrates how to use the audio device system in a VoIP context
//! by simulating a simple VoIP call with real audio devices.
//!
//! This shows the integration between ClientManager and AudioDeviceManager for
//! managing audio sessions in a VoIP application.
//!
//! Run with: cargo run --example voip_audio_demo

use std::time::{Duration, Instant};
use tokio::time::{sleep, timeout};
use tracing::{info, warn, error};

use rvoip_client_core::{
    ClientManager, ClientConfig,
    client::config::MediaConfig,
    audio::{AudioDeviceManager, AudioDirection},
    call::CallId,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better output
    tracing_subscriber::fmt::init();
    
    println!("📞 VoIP Audio Demo");
    println!("==================");
    println!("🎯 This demo simulates a VoIP call using real audio devices\n");
    
    // Create client configuration
    let config = ClientConfig {
        local_sip_addr: "127.0.0.1:5060".parse()?,
        local_media_addr: "127.0.0.1:7060".parse()?,
        user_agent: "rvoip-demo/1.0".to_string(),
        media: MediaConfig::default(),
        max_concurrent_calls: 5,
        session_timeout_secs: 300,
        enable_audio: true,
        enable_video: false,
        domain: None,
    };
    
    // Create and start client
    println!("🚀 Starting VoIP client...");
    let client = ClientManager::new(config).await?;
    
    // Get available audio devices
    println!("🔍 Discovering audio devices...");
    let input_devices = client.list_audio_devices(AudioDirection::Input).await?;
    let output_devices = client.list_audio_devices(AudioDirection::Output).await?;
    
    println!("📱 Found {} input device(s) and {} output device(s)", 
             input_devices.len(), output_devices.len());
    
    if input_devices.is_empty() || output_devices.is_empty() {
        error!("❌ Need both input and output devices for VoIP demo");
        return Ok(());
    }
    
    // Display device information
    println!("\n🎤 Available input devices:");
    for (i, device) in input_devices.iter().enumerate() {
        println!("  {}. {} {}", i + 1, device.name, 
                if device.is_default { "(default)" } else { "" });
    }
    
    println!("\n🔊 Available output devices:");
    for (i, device) in output_devices.iter().enumerate() {
        println!("  {}. {} {}", i + 1, device.name, 
                if device.is_default { "(default)" } else { "" });
    }
    
    // Simulate a VoIP call
    println!("\n📞 Simulating VoIP call...");
    let call_id = CallId::new_v4();
    
    // Get default devices
    let default_input = client.get_default_audio_device(AudioDirection::Input).await?;
    let default_output = client.get_default_audio_device(AudioDirection::Output).await?;
    
    info!("Using input device: {} ({})", default_input.name, default_input.id);
    info!("Using output device: {} ({})", default_output.name, default_output.id);
    
    // Start audio capture (microphone)
    println!("🎙️  Starting audio capture for call {}...", call_id);
    client.start_audio_capture(&call_id, &default_input.id).await?;
    
    // Start audio playback (speakers)
    println!("🔊 Starting audio playback for call {}...", call_id);
    client.start_audio_playback(&call_id, &default_output.id).await?;
    
    // Verify audio sessions are active
    println!("✅ Audio sessions active:");
    println!("   Capture: {}", client.is_audio_capture_active(&call_id).await);
    println!("   Playback: {}", client.is_audio_playback_active(&call_id).await);
    
    // Simulate call duration with periodic status updates
    println!("\n📡 Simulating 30-second call...");
    let call_start = Instant::now();
    let call_duration = Duration::from_secs(30);
    
    while call_start.elapsed() < call_duration {
        sleep(Duration::from_secs(5)).await;
        
        let elapsed = call_start.elapsed();
        let remaining = call_duration.saturating_sub(elapsed);
        
        println!("⏱️  Call time: {:.0}s / {:.0}s ({:.0}s remaining)", 
                elapsed.as_secs_f64(), call_duration.as_secs_f64(), remaining.as_secs_f64());
        
        // Check session status
        let capture_active = client.is_audio_capture_active(&call_id).await;
        let playback_active = client.is_audio_playback_active(&call_id).await;
        
        if !capture_active || !playback_active {
            warn!("⚠️  Audio session status changed: capture={}, playback={}", 
                  capture_active, playback_active);
        }
        
        // Get active session counts
        let (active_playback_sessions, active_capture_sessions) = client.get_active_audio_sessions().await;
        
        println!("   Active sessions: {} capture, {} playback", 
                active_capture_sessions.len(), active_playback_sessions.len());
    }
    
    // End the call
    println!("\n📞 Ending call...");
    client.stop_audio_capture(&call_id).await?;
    client.stop_audio_playback(&call_id).await?;
    
    // Verify sessions are stopped
    sleep(Duration::from_millis(100)).await;
    
    println!("🛑 Audio sessions stopped:");
    println!("   Capture: {}", client.is_audio_capture_active(&call_id).await);
    println!("   Playback: {}", client.is_audio_playback_active(&call_id).await);
    
    // Demonstrate multiple concurrent calls
    println!("\n🔄 Demonstrating concurrent calls...");
    let call_id_1 = CallId::new_v4();
    let call_id_2 = CallId::new_v4();
    
    // Start first call
    client.start_audio_capture(&call_id_1, &default_input.id).await?;
    client.start_audio_playback(&call_id_1, &default_output.id).await?;
    
    // Start second call
    client.start_audio_capture(&call_id_2, &default_input.id).await?;
    client.start_audio_playback(&call_id_2, &default_output.id).await?;
    
    println!("📞 Two concurrent calls active");
    
    let (active_playback_sessions, active_capture_sessions) = client.get_active_audio_sessions().await;
    
    println!("   Active capture sessions: {:?}", active_capture_sessions);
    println!("   Active playback sessions: {:?}", active_playback_sessions);
    
    // Wait a moment
    sleep(Duration::from_secs(3)).await;
    
    // Stop all audio sessions
    println!("🛑 Stopping all audio sessions...");
    client.stop_all_audio_sessions().await?;
    
    // Verify all stopped
    sleep(Duration::from_millis(100)).await;
    let (final_playback_sessions, final_capture_sessions) = client.get_active_audio_sessions().await;
    
    println!("✅ All sessions stopped: {} capture, {} playback remaining", 
             final_capture_sessions.len(), final_playback_sessions.len());
    
    println!("\n✨ VoIP audio demo complete!");
    println!("🎉 Successfully demonstrated:");
    println!("   • Audio device discovery and selection");
    println!("   • VoIP call audio session management");  
    println!("   • Concurrent call handling");
    println!("   • Proper session cleanup");
    
    Ok(())
} 