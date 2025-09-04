//! Bob - Peer 2 that accepts a call and exchanges audio
//! 
//! This example demonstrates using the session-core-v2 API as a UAS (callee)
//! to accept an incoming call and exchange audio data.

use rvoip_session_core_v2::api::unified::{UnifiedSession, UnifiedCoordinator, Config, SessionEvent};
use rvoip_session_core_v2::state_table::types::{Role, CallState};
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
    
    println!("[BOB] 📞 Starting on port 5061...");
    
    // Create coordinator with configuration for Bob
    // Load the custom state table from this example's YAML file
    let state_table_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/api_peer_audio/peer_audio_states.yaml");
    
    let config = Config {
        sip_port: 5061,
        media_port_start: 10100,
        media_port_end: 10200,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5061".parse()?,
        state_table_path: Some(state_table_path.to_string_lossy().to_string()),
    };
    let coordinator = UnifiedCoordinator::new(config).await?;
    
    // Create Bob's session (UAS - callee)
    let session = UnifiedSession::new(coordinator.clone(), Role::UAS).await?;
    println!("[BOB] Created session: {}", session.id.0);
    
    // Subscribe to events
    let incoming_call = Arc::new(AtomicBool::new(false));
    let incoming_call_clone = incoming_call.clone();
    let call_established = Arc::new(AtomicBool::new(false));
    let call_established_clone = call_established.clone();
    let media_established = Arc::new(AtomicBool::new(false));
    let media_established_clone = media_established.clone();
    let call_terminated = Arc::new(AtomicBool::new(false));
    let call_terminated_clone = call_terminated.clone();
    let dtmf_received = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let dtmf_received_clone = dtmf_received.clone();
    
    session.on_event(move |event| {
        match event {
            SessionEvent::StateChanged { from, to } => {
                println!("[BOB] State changed: {:?} -> {:?}", from, to);
                if to == CallState::Ringing {
                    incoming_call_clone.store(true, Ordering::SeqCst);
                }
            }
            SessionEvent::CallEstablished => {
                println!("[BOB] ✅ Call established!");
                call_established_clone.store(true, Ordering::SeqCst);
            }
            SessionEvent::MediaFlowEstablished { local_addr, remote_addr } => {
                println!("[BOB] 🔊 Media flow established");
                println!("[BOB]    Local: {}, Remote: {}", local_addr, remote_addr);
                media_established_clone.store(true, Ordering::SeqCst);
            }
            SessionEvent::DtmfReceived { digit } => {
                println!("[BOB] 🎹 DTMF received: {}", digit);
                let dtmf = dtmf_received_clone.clone();
                tokio::spawn(async move {
                    dtmf.lock().await.push(digit);
                });
            }
            SessionEvent::HoldStarted => {
                println!("[BOB] ⏸️ Call put on hold");
            }
            SessionEvent::HoldReleased => {
                println!("[BOB] ▶️ Call resumed from hold");
            }
            SessionEvent::CallTerminated { reason } => {
                println!("[BOB] 📵 Call terminated: {}", reason);
                call_terminated_clone.store(true, Ordering::SeqCst);
            }
            _ => {}
        }
    }).await?;
    
    println!("[BOB] ⏳ Waiting for incoming call from Alice...");
    
    // Simulate receiving an incoming call
    // In a real implementation, this would come from the SIP stack
    // For this example, we simulate it after a short delay
    sleep(Duration::from_secs(2)).await;
    
    // Simulate incoming INVITE with SDP
    let alice_sdp = r#"v=0
o=alice 2890844526 2890844526 IN IP4 127.0.0.1
s=-
c=IN IP4 127.0.0.1
t=0 0
m=audio 10000 RTP/AVP 0 8 101
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000
a=rtpmap:101 telephone-event/8000
a=sendrecv"#;
    
    println!("[BOB] 📞 Incoming call from Alice!");
    session.on_incoming_call("sip:alice@127.0.0.1:5060", Some(alice_sdp.to_string())).await?;
    
    // Wait for the state to change to Ringing
    let mut retries = 0;
    while !incoming_call.load(Ordering::SeqCst) && retries < 20 {
        sleep(Duration::from_millis(100)).await;
        retries += 1;
    }
    
    if !incoming_call.load(Ordering::SeqCst) {
        println!("[BOB] ⚠️ No incoming call detected");
        return Ok(());
    }
    
    // Accept the call
    println!("[BOB] ✅ Accepting the call...");
    session.accept().await?;
    
    // Wait for call to be established
    retries = 0;
    while !call_established.load(Ordering::SeqCst) && retries < 30 {
        sleep(Duration::from_millis(100)).await;
        retries += 1;
    }
    
    if !call_established.load(Ordering::SeqCst) {
        println!("[BOB] ⚠️ Call was not established after 3 seconds");
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
    println!("[BOB] 🎵 Starting audio exchange simulation...");
    
    // Generate tone data (880 Hz for Bob, 8000 Hz sample rate)
    let sample_rate = 8000;
    let frequency = 880.0; // A5 note (octave higher than Alice)
    let duration_secs = 3;
    let num_samples = sample_rate * duration_secs;
    
    if std::env::var("RECORD_AUDIO").is_ok() {
        println!("[BOB] 📼 Recording enabled - generating audio tone");
        
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
        
        std::fs::write("output/bob_sent.raw", &samples)?;
        println!("[BOB] 💾 Saved {} bytes of 880Hz tone to output/bob_sent.raw", samples.len());
        
        // Simulate receiving audio from Alice
        // Alice sends 440 Hz tone
        let alice_frequency = 440.0; // A4 note
        samples.clear();
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * alice_frequency * t).sin();
            let pcm_sample = (sample * 32767.0) as i16;
            samples.extend_from_slice(&pcm_sample.to_le_bytes());
        }
        
        std::fs::write("output/bob_received.raw", &samples)?;
        println!("[BOB] 💾 Saved {} bytes of simulated received audio to output/bob_received.raw", samples.len());
    } else {
        println!("[BOB] 🎵 Would generate 880Hz tone for 3 seconds");
        println!("[BOB] 🎵 Would receive 440Hz tone from Alice");
    }
    
    // Simulate audio playback
    println!("[BOB] 🎵 Playing welcome message...");
    session.play_audio("welcome.wav").await.ok(); // This might not be implemented yet
    
    // Start recording (simulation)
    println!("[BOB] 🔴 Starting call recording...");
    session.start_recording().await.ok(); // This might not be implemented yet
    
    // Wait for Alice's actions (DTMF, hold, resume)
    sleep(Duration::from_secs(5)).await;
    
    // Check if we received DTMF
    let dtmf_digits = dtmf_received.lock().await;
    if !dtmf_digits.is_empty() {
        println!("[BOB] 🎹 Received DTMF sequence: {:?}", dtmf_digits);
    }
    
    // Stop recording
    println!("[BOB] ⏹️ Stopping recording...");
    session.stop_recording().await.ok(); // This might not be implemented yet
    
    // Wait for call termination
    retries = 0;
    while !call_terminated.load(Ordering::SeqCst) && retries < 50 {
        sleep(Duration::from_millis(100)).await;
        retries += 1;
    }
    
    if !call_terminated.load(Ordering::SeqCst) {
        println!("[BOB] 📵 Hanging up...");
        session.hangup().await?;
    }
    
    // Give time for cleanup
    sleep(Duration::from_millis(500)).await;
    
    println!("[BOB] ✅ Test completed successfully");
    Ok(())
}