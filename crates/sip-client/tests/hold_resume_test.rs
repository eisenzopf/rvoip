//! Hold/Resume test with two SIP clients
//!
//! This test creates two SIP clients that:
//! 1. Establish a call between them
//! 2. One client puts the call on hold
//! 3. Verifies the hold state
//! 4. Resumes the call
//! 5. Verifies the call is active again

use rvoip_sip_client::{SipClientBuilder, StreamExt, CallState, SipClientEvent};
use std::sync::Arc;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, timeout};
use tracing::{info, debug, error};
use serial_test::serial;

/// Create a simple test WAV file for music-on-hold
fn create_test_wav(path: &PathBuf) -> std::io::Result<()> {
    use std::io::Write;
    
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

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn test_hold_resume_between_sip_clients() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_sip_client=debug,rvoip_client_core=debug,rvoip_session_core=debug")
        .with_test_writer()
        .try_init();

    info!("🚀 Starting hold/resume test between two SIP clients");

    // Create temp WAV file for music-on-hold
    let temp_dir = std::env::temp_dir();
    let moh_file = temp_dir.join("sip_client_test_moh.wav");
    create_test_wav(&moh_file).expect("Failed to create test WAV");
    
    // Create caller (Alice) with music-on-hold configuration
    info!("📞 Creating Alice (caller) with music-on-hold");
    let alice = Arc::new(
        SipClientBuilder::new()
            .sip_identity("sip:alice@localhost")
            .local_address("127.0.0.1:17100".parse().unwrap())
            .music_on_hold_file(&moh_file)
            .build()
            .await
            .expect("Failed to create Alice")
    );
    
    // Create callee (Bob)
    info!("📞 Creating Bob (callee)");
    let bob = Arc::new(
        SipClientBuilder::new()
            .sip_identity("sip:bob@localhost")
            .local_address("127.0.0.1:17101".parse().unwrap())
            .build()
            .await
            .expect("Failed to create Bob")
    );
    
    // Start both clients
    alice.start().await.expect("Failed to start Alice");
    bob.start().await.expect("Failed to start Bob");
    info!("✅ Started both SIP clients");
    
    // Set up Bob to answer incoming calls
    let bob_clone = bob.clone();
    let bob_call_id = Arc::new(Mutex::new(None::<String>));
    let bob_call_id_clone = bob_call_id.clone();
    let mut bob_events = bob.events();
    
    let bob_event_handler = tokio::spawn(async move {
        while let Some(event) = bob_events.next().await {
            match event {
                Ok(SipClientEvent::IncomingCall { call, from, .. }) => {
                    info!("Bob: Incoming call from {}", from);
                    *bob_call_id_clone.lock().await = Some(call.id.to_string());
                    
                    // Small delay to simulate user answering
                    sleep(Duration::from_millis(100)).await;
                    
                    match bob_clone.answer(&call.id).await {
                        Ok(_) => info!("Bob: Answered call"),
                        Err(e) => error!("Bob: Failed to answer call: {}", e),
                    }
                }
                Ok(SipClientEvent::CallConnected { call_id, .. }) => {
                    info!("Bob: Call {} connected", call_id);
                }
                Ok(SipClientEvent::CallStateChanged { call, previous_state, new_state, .. }) => {
                    debug!("Bob: Call {} state changed from {:?} to {:?}", call.id, previous_state, new_state);
                }
                _ => {}
            }
        }
    });
    
    // Track Alice's events
    let alice_events_log = Arc::new(Mutex::new(Vec::new()));
    let alice_events_log_clone = alice_events_log.clone();
    let mut alice_events = alice.events();
    
    let alice_event_handler = tokio::spawn(async move {
        while let Some(event) = alice_events.next().await {
            match event {
                Ok(SipClientEvent::CallConnected { call_id, .. }) => {
                    info!("Alice: Call {} connected", call_id);
                    alice_events_log_clone.lock().await.push("connected".to_string());
                }
                Ok(SipClientEvent::CallStateChanged { call, previous_state, new_state, .. }) => {
                    debug!("Alice: Call {} state changed from {:?} to {:?}", call.id, previous_state, new_state);
                    // Track connected state from state change as well
                    if matches!(new_state, CallState::Connected) && !matches!(previous_state, CallState::Connected) {
                        alice_events_log_clone.lock().await.push("connected".to_string());
                    }
                    if matches!(new_state, CallState::OnHold) {
                        alice_events_log_clone.lock().await.push("on_hold".to_string());
                    } else if matches!(new_state, CallState::Connected) && matches!(previous_state, CallState::OnHold) {
                        alice_events_log_clone.lock().await.push("resumed".to_string());
                    }
                }
                _ => {}
            }
        }
    });
    
    // Give clients time to initialize
    sleep(Duration::from_millis(200)).await;
    
    info!("\n=== Alice calling Bob ===");
    
    // Alice calls Bob
    let alice_call = alice.call("sip:bob@127.0.0.1:17101").await
        .expect("Failed to make call");
    
    info!("Alice: Created call {}", alice_call.id);
    
    // Wait for call to be answered and connected
    match timeout(Duration::from_secs(5), alice_call.wait_for_answer()).await {
        Ok(Ok(_)) => info!("✅ Call answered and connected"),
        Ok(Err(e)) => panic!("Call failed: {}", e),
        Err(_) => panic!("Call answer timeout"),
    }
    
    // Wait for media to be established
    sleep(Duration::from_millis(500)).await;
    
    // Verify both sides see the call as connected
    let alice_call_info = alice.get_call(&alice_call.id).expect("Alice should have the call");
    assert_eq!(*alice_call_info.state.read(), CallState::Connected, "Alice's call should be connected");
    
    let bob_call_id_str = bob_call_id.lock().await.clone()
        .expect("Bob didn't receive a call");
    // Bob's call ID is stored as string, need to parse it
    let bob_active = bob.active_call();
    assert!(bob_active.is_some(), "Bob should have an active call");
    assert_eq!(*bob_active.unwrap().state.read(), CallState::Connected, "Bob's call should be connected");
    
    info!("\n=== Testing hold operation ===");
    
    // Alice puts the call on hold
    alice.hold(&alice_call.id).await
        .expect("Alice failed to put call on hold");
    
    info!("Alice: Put call on hold");
    
    // Wait for hold to take effect
    sleep(Duration::from_millis(500)).await;
    
    // Verify Alice's call is on hold
    let alice_call_info = alice.get_call(&alice_call.id).expect("Alice should have the call");
    assert_eq!(*alice_call_info.state.read(), CallState::OnHold, "Alice's call should be on hold");
    
    info!("✅ Alice: Call confirmed on hold");
    
    // Let music-on-hold play for a bit
    info!("🎵 Playing music-on-hold for 2 seconds...");
    sleep(Duration::from_secs(2)).await;
    
    info!("\n=== Testing resume operation ===");
    
    // Alice resumes the call
    alice.resume(&alice_call.id).await
        .expect("Alice failed to resume call");
    
    info!("Alice: Resumed call");
    
    // Wait for resume to take effect
    sleep(Duration::from_millis(500)).await;
    
    // Verify Alice's call is active again
    let alice_call_info = alice.get_call(&alice_call.id).expect("Alice should have the call");
    assert_eq!(*alice_call_info.state.read(), CallState::Connected, "Alice's call should be connected after resume");
    
    info!("✅ Alice: Call confirmed active again");
    
    // Test multiple hold/resume cycles
    info!("\n=== Testing multiple hold/resume cycles ===");
    
    for i in 1..=3 {
        info!("\nCycle {}: Hold", i);
        alice.hold(&alice_call.id).await
            .expect("Failed to hold");
        sleep(Duration::from_millis(500)).await;
        
        let alice_call_info = alice.get_call(&alice_call.id).expect("Alice should have the call");
        assert_eq!(*alice_call_info.state.read(), CallState::OnHold, "Call should be on hold in cycle {}", i);
        
        info!("Cycle {}: Resume", i);
        alice.resume(&alice_call.id).await
            .expect("Failed to resume");
        sleep(Duration::from_millis(500)).await;
        
        let alice_call_info = alice.get_call(&alice_call.id).expect("Alice should have the call");
        assert_eq!(*alice_call_info.state.read(), CallState::Connected, "Call should be active in cycle {}", i);
    }
    
    info!("\n=== Hanging up calls ===");
    
    // Hang up the call
    alice.hangup(&alice_call.id).await
        .expect("Alice failed to hang up");
    
    // Wait for hangup to propagate
    sleep(Duration::from_millis(500)).await;
    
    // Stop both clients
    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    
    // Abort event handlers
    bob_event_handler.abort();
    alice_event_handler.abort();
    
    // Clean up temp file
    let _ = std::fs::remove_file(&moh_file);
    
    // Verify events
    let events = alice_events_log.lock().await;
    assert!(events.contains(&"connected".to_string()), "Should have connected event");
    assert!(events.contains(&"on_hold".to_string()), "Should have on_hold event");
    
    info!("\n✅ Hold/resume test completed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn test_hold_without_music_fallback() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_sip_client=debug")
        .with_test_writer()
        .try_init();

    info!("\n=== Testing hold without music-on-hold (fallback to mute) ===");
    
    // Create Alice WITHOUT music-on-hold
    let alice = Arc::new(
        SipClientBuilder::new()
            .sip_identity("sip:alice@localhost")
            .local_address("127.0.0.1:17102".parse().unwrap())
            // No music_on_hold_file!
            .build()
            .await
            .expect("Failed to create Alice")
    );
    
    // Create Bob
    let bob = Arc::new(
        SipClientBuilder::new()
            .sip_identity("sip:bob@localhost")
            .local_address("127.0.0.1:17103".parse().unwrap())
            .build()
            .await
            .expect("Failed to create Bob")
    );
    
    // Set up Bob to auto-answer
    let bob_clone = bob.clone();
    let mut bob_events = bob.events();
    let bob_handler = tokio::spawn(async move {
        while let Some(event) = bob_events.next().await {
            if let Ok(SipClientEvent::IncomingCall { call, .. }) = event {
                sleep(Duration::from_millis(100)).await;
                bob_clone.answer(&call.id).await.ok();
            }
        }
    });
    
    // Start both clients
    alice.start().await.expect("Failed to start Alice");
    bob.start().await.expect("Failed to start Bob");
    
    // Alice calls Bob
    let call = alice.call("sip:bob@127.0.0.1:17103").await
        .expect("Failed to make call");
    
    // Wait for answer
    timeout(Duration::from_secs(5), call.wait_for_answer()).await
        .expect("Timeout waiting for answer")
        .expect("Call failed");
    
    // Wait for establishment
    sleep(Duration::from_millis(500)).await;
    
    info!("Testing hold without MoH (should fallback to mute)...");
    
    // Hold the call (should fallback to mute)
    alice.hold(&call.id).await
        .expect("Failed to hold call");
    
    sleep(Duration::from_millis(500)).await;
    
    // Verify on hold
    let call_info = alice.get_call(&call.id).expect("Alice should have the call");
    assert_eq!(*call_info.state.read(), CallState::OnHold, "Call should be on hold");
    
    // Resume
    alice.resume(&call.id).await
        .expect("Failed to resume call");
    
    sleep(Duration::from_millis(500)).await;
    
    // Verify active
    let call_info = alice.get_call(&call.id).expect("Alice should have the call");
    assert_eq!(*call_info.state.read(), CallState::Connected, "Call should be active");
    
    // Clean up
    alice.hangup(&call.id).await.ok();
    sleep(Duration::from_millis(500)).await;
    
    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    
    bob_handler.abort();
    
    info!("\n✅ Hold without MoH test completed successfully!");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn test_invalid_hold_operations() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_sip_client=debug")
        .with_test_writer()
        .try_init();

    let client = SipClientBuilder::new()
        .sip_identity("sip:test@localhost")
        .local_address("127.0.0.1:17104".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client");
    
    client.start().await.expect("Failed to start client");
    
    // Create a call to ourselves (will fail but that's ok for this test)
    let call = client.call("sip:test@127.0.0.1:17104").await
        .expect("Failed to make call");
    
    // Try to hold a non-active call (should fail)
    let hold_result = client.hold(&call.id).await;
    assert!(hold_result.is_err(), "Should not be able to hold non-active call");
    
    // Clean up
    client.hangup(&call.id).await.ok();
    client.stop().await.expect("Failed to stop client");
    
    info!("\n✅ Invalid hold operations test completed!");
}