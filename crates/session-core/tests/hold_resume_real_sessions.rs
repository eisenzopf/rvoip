//! Real session hold/resume tests with proper dialog establishment
//!
//! Creates two session managers that establish calls between each other,
//! then tests hold/resume functionality with music-on-hold.

use rvoip_session_core::{
    api::*,
    media::sdp_utils::*,
};
use std::sync::Arc;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::Mutex;
use std::io::Write;

mod common;
use common::*;

/// Create a simple test WAV file
fn create_test_wav(path: &PathBuf) -> std::io::Result<()> {
    // WAV header for 8kHz mono 16-bit PCM
    let wav_header = vec![
        0x52, 0x49, 0x46, 0x46, // "RIFF"
        0x24, 0x08, 0x00, 0x00, // File size - 8
        0x57, 0x41, 0x56, 0x45, // "WAVE"
        0x66, 0x6D, 0x74, 0x20, // "fmt "
        0x10, 0x00, 0x00, 0x00, // Subchunk size
        0x01, 0x00,             // Audio format (PCM)
        0x01, 0x00,             // Number of channels (mono)
        0x40, 0x1F, 0x00, 0x00, // Sample rate (8000)
        0x80, 0x3E, 0x00, 0x00, // Byte rate
        0x02, 0x00,             // Block align
        0x10, 0x00,             // Bits per sample
        0x64, 0x61, 0x74, 0x61, // "data"
        0x00, 0x08, 0x00, 0x00, // Data size
    ];
    
    let mut file = std::fs::File::create(path)?;
    file.write_all(&wav_header)?;
    
    // Write 1 second of silence (8000 samples * 2 bytes)
    let silence = vec![0u8; 16000];
    file.write_all(&silence)?;
    
    Ok(())
}

/// Test handler that auto-accepts calls
#[derive(Debug, Clone)]
struct AutoAcceptHandler {
    events: Arc<Mutex<Vec<String>>>,
}

impl AutoAcceptHandler {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    async fn get_events(&self) -> Vec<String> {
        self.events.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for AutoAcceptHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let mut events = self.events.lock().await;
        events.push(format!("incoming_call: {} -> {}", call.from, call.to));
        
        // Auto-accept and let the library handle SDP negotiation
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, call: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        let mut events = self.events.lock().await;
        events.push(format!("call_established: {}", call.id()));
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let mut events = self.events.lock().await;
        events.push(format!("call_ended: {} ({})", call.id(), reason));
    }
}

#[tokio::test]
async fn test_hold_resume_between_real_sessions() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Set up logging
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("info,rvoip_session_core=debug,rvoip_dialog_core=debug")
        .try_init();
    
    // Create temp WAV file for music-on-hold
    let temp_dir = std::env::temp_dir();
    let moh_file = temp_dir.join("test_hold_music.wav");
    create_test_wav(&moh_file)?;
    
    // Create handlers
    let alice_handler = Arc::new(AutoAcceptHandler::new());
    let bob_handler = Arc::new(AutoAcceptHandler::new());
    
    // Create Alice's session manager (caller) with MoH
    let (alice_port, bob_port) = get_test_ports();
    let alice = SessionManagerBuilder::new()
        .with_sip_port(alice_port)
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", alice_port))
        .with_media_ports(40000, 41000)
        .with_music_on_hold_file(&moh_file)
        .with_handler(alice_handler.clone())
        .build()
        .await?;
    
    // Create Bob's session manager (callee)
    let bob = SessionManagerBuilder::new()
        .with_sip_port(bob_port)
        .with_local_address(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .with_media_ports(42000, 43000)
        .with_handler(bob_handler.clone())
        .build()
        .await?;
    
    // Start both coordinators
    SessionControl::start(&alice).await?;
    SessionControl::start(&bob).await?;
    
    println!("Alice listening on port {}", alice_port);
    println!("Bob listening on port {}", bob_port);
    
    // Alice calls Bob - first prepare the call to generate SDP
    println!("\n=== Alice calling Bob ===");
    let prepared = SessionControl::prepare_outgoing_call(
        &alice,
        &format!("sip:alice@127.0.0.1:{}", alice_port),
        &format!("sip:bob@127.0.0.1:{}", bob_port),
    ).await?;
    
    // Now create the call with the generated SDP
    let alice_session = SessionControl::initiate_prepared_call(
        &alice,
        &prepared
    ).await?;
    
    println!("Created outgoing call with session ID: {}", alice_session.id());
    
    // Wait for call to be established (give more time for both sides)
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Check Bob received the call
    let bob_events = bob_handler.get_events().await;
    println!("Bob's events: {:?}", bob_events);
    assert!(bob_events.iter().any(|e| e.contains("incoming_call")), "Bob didn't receive the call");
    assert!(bob_events.iter().any(|e| e.contains("call_established")), "Call wasn't established on Bob's side");
    
    // Wait for call to stabilize
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Get Alice's session and verify it's active
    let alice_session_state = SessionControl::get_session(&alice, &alice_session.id).await?
        .expect("Alice's session should exist");
    assert_eq!(alice_session_state.state(), &CallState::Active, "Alice's call should be active");
    
    // Test hold operation
    println!("\n=== Testing hold operation ===");
    SessionControl::hold_session(&alice, &alice_session.id).await?;
    
    // Verify session is on hold
    tokio::time::sleep(Duration::from_millis(200)).await;
    let held_session = SessionControl::get_session(&alice, &alice_session.id).await?
        .expect("Session should exist");
    assert_eq!(held_session.state(), &CallState::OnHold, "Session should be on hold");
    
    // Give some time for MoH to play
    println!("Playing music-on-hold...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Test resume operation
    println!("\n=== Testing resume operation ===");
    SessionControl::resume_session(&alice, &alice_session.id).await?;
    
    // Verify session is active again
    tokio::time::sleep(Duration::from_millis(200)).await;
    let resumed_session = SessionControl::get_session(&alice, &alice_session.id).await?
        .expect("Session should exist");
    assert_eq!(resumed_session.state(), &CallState::Active, "Session should be active again");
    
    // Test multiple hold/resume cycles
    println!("\n=== Testing multiple hold/resume cycles ===");
    for i in 1..=3 {
        println!("Cycle {}: Hold", i);
        SessionControl::hold_session(&alice, &alice_session.id).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        let state = SessionControl::get_session(&alice, &alice_session.id).await?
            .expect("Session should exist");
        assert_eq!(state.state(), &CallState::OnHold, "Session should be on hold in cycle {}", i);
        
        println!("Cycle {}: Resume", i);
        SessionControl::resume_session(&alice, &alice_session.id).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        let state = SessionControl::get_session(&alice, &alice_session.id).await?
            .expect("Session should exist");
        assert_eq!(state.state(), &CallState::Active, "Session should be active in cycle {}", i);
    }
    
    // Clean up - terminate the call
    println!("\n=== Terminating call ===");
    SessionControl::terminate_session(&alice, &alice_session.id).await?;
    
    // Wait for termination to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Verify handler received events
    let alice_events = alice_handler.get_events().await;
    assert!(alice_events.iter().any(|e| e.contains("call_established")), "Alice didn't get call established");
    assert!(alice_events.iter().any(|e| e.contains("call_ended")), "Alice didn't get call ended");
    
    // Stop both coordinators
    SessionControl::stop(&alice).await?;
    SessionControl::stop(&bob).await?;
    
    // Clean up temp file
    let _ = std::fs::remove_file(&moh_file);
    
    println!("\n=== Test completed successfully ===");
    Ok(())
}

#[tokio::test] 
async fn test_hold_without_moh_fallback() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Set up logging
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("info,rvoip_session_core=debug")
        .try_init();
    
    // Create handlers
    let alice_handler = Arc::new(AutoAcceptHandler::new());
    let bob_handler = Arc::new(AutoAcceptHandler::new());
    
    // Create Alice's session manager WITHOUT MoH file
    let (alice_port, bob_port) = get_test_ports();
    let alice = SessionManagerBuilder::new()
        .with_sip_port(alice_port)
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", alice_port))
        .with_media_ports(44000, 45000)
        .with_handler(alice_handler.clone())
        // No music_on_hold_file!
        .build()
        .await?;
    
    // Create Bob's session manager
    let bob = SessionManagerBuilder::new()
        .with_sip_port(bob_port)
        .with_local_address(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .with_media_ports(46000, 47000)
        .with_handler(bob_handler.clone())
        .build()
        .await?;
    
    // Start both
    SessionControl::start(&alice).await?;
    SessionControl::start(&bob).await?;
    
    // Alice calls Bob - prepare first
    let prepared = SessionControl::prepare_outgoing_call(
        &alice,
        &format!("sip:alice@127.0.0.1:{}", alice_port),
        &format!("sip:bob@127.0.0.1:{}", bob_port),
    ).await?;
    
    let alice_session = SessionControl::initiate_prepared_call(
        &alice,
        &prepared
    ).await?;
    
    // Wait for establishment - need to ensure call is Active before hold
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Verify Alice's call is active before attempting hold
    let alice_session_state = SessionControl::get_session(&alice, &alice_session.id).await?
        .expect("Alice's session should exist");
    assert_eq!(alice_session_state.state(), &CallState::Active, "Alice's call should be active before hold");
    
    // Test hold (should fallback to mute since no MoH file)
    println!("Testing hold without MoH (should fallback to mute)...");
    SessionControl::hold_session(&alice, &alice_session.id).await?;
    
    // Verify on hold
    let held = SessionControl::get_session(&alice, &alice_session.id).await?
        .expect("Session should exist");
    assert_eq!(held.state(), &CallState::OnHold);
    
    // Resume
    SessionControl::resume_session(&alice, &alice_session.id).await?;
    
    // Verify active
    let resumed = SessionControl::get_session(&alice, &alice_session.id).await?
        .expect("Session should exist");
    assert_eq!(resumed.state(), &CallState::Active);
    
    // Clean up
    SessionControl::terminate_session(&alice, &alice_session.id).await?;
    SessionControl::stop(&alice).await?;
    SessionControl::stop(&bob).await?;
    
    Ok(())
}