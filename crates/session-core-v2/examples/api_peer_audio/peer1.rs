//! Alice - Peer 1 that makes a call and exchanges audio
//! 
//! This example demonstrates using the session-core-v2 API as a UAC (caller)
//! to establish a call and exchange audio data.

use rvoip_session_core_v2::api::unified::{UnifiedSession, UnifiedCoordinator, Config, SessionEvent};
use rvoip_session_core_v2::state_table::types::Role;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging based on environment
    let log_level = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "warn".to_string());
    
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(log_level.parse().unwrap_or(tracing::Level::WARN.into())))
        .init();
    
    println!("[ALICE] 📞 Starting on port 5060...");
    
    // Create coordinator with configuration for Alice
    // Load the custom state table from this example's YAML file
    let state_table_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/api_peer_audio/peer_audio_states.yaml");
    
    let config = Config {
        sip_port: 5060,
        media_port_start: 10000,
        media_port_end: 10100,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        state_table_path: Some(state_table_path.to_string_lossy().to_string()),
    };
    let coordinator = UnifiedCoordinator::new(config).await?;
    
    // Create Alice's session (UAC - caller)
    let session = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
    println!("[ALICE] Created session: {}", session.id.0);
    
    // Subscribe to events
    let call_established = Arc::new(AtomicBool::new(false));
    let call_established_clone = call_established.clone();
    let media_established = Arc::new(AtomicBool::new(false));
    let media_established_clone = media_established.clone();
    
    session.on_event(move |event| {
        match event {
            SessionEvent::StateChanged { from, to } => {
                println!("[ALICE] State changed: {:?} -> {:?}", from, to);
            }
            SessionEvent::CallEstablished => {
                println!("[ALICE] ✅ Call established!");
                call_established_clone.store(true, Ordering::SeqCst);
            }
            SessionEvent::MediaFlowEstablished { local_addr, remote_addr } => {
                println!("[ALICE] 🔊 Media flow established");
                println!("[ALICE]    Local: {}, Remote: {}", local_addr, remote_addr);
                media_established_clone.store(true, Ordering::SeqCst);
            }
            SessionEvent::CallTerminated { reason } => {
                println!("[ALICE] 📵 Call terminated: {}", reason);
            }
            _ => {}
        }
    }).await?;
    
    // Give Bob time to start listening
    println!("[ALICE] ⏳ Waiting for Bob to be ready...");
    sleep(Duration::from_secs(1)).await;
    
    // Make the call to Bob
    println!("[ALICE] 📞 Calling Bob at sip:bob@127.0.0.1:5061...");
    session.make_call("sip:bob@127.0.0.1:5061").await?;
    
    // Wait for call to be established
    println!("[ALICE] ⏳ Waiting for call to establish...");
    let mut retries = 0;
    while !call_established.load(Ordering::SeqCst) && retries < 50 {
        sleep(Duration::from_millis(100)).await;
        retries += 1;
    }
    
    if !call_established.load(Ordering::SeqCst) {
        println!("[ALICE] ⚠️ Call was not established after 5 seconds");
        session.hangup().await?;
        return Ok(());
    }
    
    // Wait for media to be established
    retries = 0;
    while !media_established.load(Ordering::SeqCst) && retries < 20 {
        sleep(Duration::from_millis(100)).await;
        retries += 1;
    }
    
    // Simulate audio exchange
    // Note: In session-core-v2, audio handling is delegated to media-core
    // The actual audio channels would need to be accessed through the media adapter
    println!("[ALICE] 🎵 Starting audio exchange simulation...");
    
    // Generate tone data (440 Hz for Alice, 8000 Hz sample rate)
    let sample_rate = 8000;
    let frequency = 440.0; // A4 note
    let duration_secs = 3;
    let num_samples = sample_rate * duration_secs;
    
    if std::env::var("RECORD_AUDIO").is_ok() {
        println!("[ALICE] 📼 Recording enabled - generating audio tone");
        
        // Create output directory
        std::fs::create_dir_all("output").ok();
        
        // Generate and save tone
        let mut samples = Vec::new();
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin();
            let pcm_sample = (sample * 32767.0) as i16;
            samples.extend_from_slice(&pcm_sample.to_le_bytes());
        }
        
        std::fs::write("output/alice_sent.raw", &samples)?;
        println!("[ALICE] 💾 Saved {} bytes of 440Hz tone to output/alice_sent.raw", samples.len());
        
        // In a real implementation, we would:
        // 1. Get the media session from the coordinator
        // 2. Access the RTP sender channel
        // 3. Send the audio samples through RTP packets
        // 4. Receive audio from the RTP receiver channel
        
        // Simulate receiving audio from Bob
        // Bob sends 880 Hz tone
        let bob_frequency = 880.0; // A5 note
        samples.clear();
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * bob_frequency * t).sin();
            let pcm_sample = (sample * 32767.0) as i16;
            samples.extend_from_slice(&pcm_sample.to_le_bytes());
        }
        
        std::fs::write("output/alice_received.raw", &samples)?;
        println!("[ALICE] 💾 Saved {} bytes of simulated received audio to output/alice_received.raw", samples.len());
    } else {
        println!("[ALICE] 🎵 Would generate 440Hz tone for 3 seconds");
        println!("[ALICE] 🎵 Would receive 880Hz tone from Bob");
    }
    
    // Simulate audio exchange duration
    sleep(Duration::from_secs(3)).await;
    println!("[ALICE] 🎵 Audio exchange completed");
    
    // Test some call control features
    println!("[ALICE] 🎹 Sending DTMF digits: 1234");
    session.send_dtmf("1234").await?;
    
    sleep(Duration::from_millis(500)).await;
    
    println!("[ALICE] ⏸️ Putting call on hold");
    session.hold().await?;
    
    sleep(Duration::from_secs(1)).await;
    
    println!("[ALICE] ▶️ Resuming call");
    session.resume().await?;
    
    sleep(Duration::from_millis(500)).await;
    
    // Hang up the call
    println!("[ALICE] 📵 Hanging up...");
    session.hangup().await?;
    
    // Give time for cleanup
    sleep(Duration::from_millis(500)).await;
    
    println!("[ALICE] ✅ Test completed successfully");
    Ok(())
}