//! Alice - Makes a call and sends audio

use rvoip_session_core_v2::api::unified::{UnifiedSession, UnifiedCoordinator, Config};
use rvoip_session_core_v2::state_table::types::Role;
use rvoip_media_core::types::AudioFrame;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Simple logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    
    println!("[ALICE] Starting...");
    
    // Configure Alice
    let config = Config {
        sip_port: 5060,
        media_port_start: 10000,
        media_port_end: 10100,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        state_table_path: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/api_peer_audio/peer_audio_states.yaml")
            .to_string_lossy()
            .to_string()),
    };
    
    let coordinator = UnifiedCoordinator::new(config).await?;
    let session = UnifiedSession::new(coordinator, Role::UAC).await?;
    
    // Give Bob time to start
    sleep(Duration::from_secs(1)).await;
    
    // Make the call
    println!("[ALICE] Calling Bob...");
    session.make_call("sip:bob@127.0.0.1:5061").await?;
    
    // Wait for call to establish (with timeout)
    sleep(Duration::from_secs(3)).await;
    
    // Subscribe to receive audio
    let mut audio_rx = session.subscribe_to_audio_frames().await?;
    
    // Send audio - 5 seconds of 440Hz tone
    println!("[ALICE] Sending audio...");
    let sample_rate = 8000u32;
    let duration_ms = 20u32;
    let samples_per_frame = (sample_rate * duration_ms / 1000) as usize;
    
    for i in 0u32..250 {  // 250 frames = 5 seconds
        let mut samples = Vec::with_capacity(samples_per_frame);
        for j in 0..samples_per_frame {
            let t = ((i as usize * samples_per_frame + j) as f32) / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            samples.push((sample * 16384.0) as i16);
        }
        
        let frame = AudioFrame {
            samples,
            sample_rate,
            channels: 1,
            duration: Duration::from_millis(duration_ms as u64),
            timestamp: i * duration_ms,
        };
        
        session.send_audio_frame(frame).await?;
        sleep(Duration::from_millis(duration_ms as u64)).await;
    }
    
    // Receive some audio
    let mut received = 0;
    while let Ok(Some(frame)) = tokio::time::timeout(
        Duration::from_millis(100),
        audio_rx.recv()
    ).await {
        received += 1;
        if received >= 50 {  // Receive at least 50 frames
            break;
        }
    }
    
    println!("[ALICE] Sent 250 frames, received {} frames", received);
    println!("[ALICE] Done!");
    Ok(())
}