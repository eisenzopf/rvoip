//! Integration test for hold/resume functionality between two clients
//! 
//! This test verifies that the hold/resume implementation in session-core
//! works correctly when used through the client-core API.

use rvoip_client_core::{
    ClientBuilder, ClientManager, ClientEvent, CallState, CallId,
    ClientEventHandler, CallAction, IncomingCallInfo, CallStatusInfo,
    MediaEventInfo, RegistrationStatusInfo, ClientError,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use serial_test::serial;
use std::path::PathBuf;
use std::io::Write;
use async_trait::async_trait;

/// Create a simple test WAV file for music-on-hold
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

#[tokio::test]
#[serial]
async fn test_hold_resume_between_clients() {
    // Set up logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_client_core=debug,rvoip_session_core=debug")
        .with_test_writer()
        .try_init();

    // Create temp WAV file for music-on-hold
    let temp_dir = std::env::temp_dir();
    let moh_file = temp_dir.join("client_test_moh.wav");
    create_test_wav(&moh_file).expect("Failed to create test WAV");
    
    println!("\n=== Creating Alice (caller) with music-on-hold ===");
    
    // Create Alice's client with music-on-hold
    let alice = Arc::new(
        ClientBuilder::new()
            .user_agent("Alice/1.0")
            .local_address("127.0.0.1:16100".parse().unwrap())
            .with_music_on_hold_file(&moh_file)
            .build()
            .await
            .expect("Failed to build Alice's client")
    );
    
    // Set up event handler for Alice
    struct TestEventHandler {
        events: Arc<tokio::sync::Mutex<Vec<ClientEvent>>>,
    }
    
    #[async_trait]
    impl ClientEventHandler for TestEventHandler {
        async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
            CallAction::Accept
        }
        
        async fn on_call_state_changed(&self, info: CallStatusInfo) {
            self.events.lock().await.push(ClientEvent::CallStateChanged {
                info,
                priority: rvoip_client_core::EventPriority::Normal,
            });
        }
        
        async fn on_media_event(&self, _info: MediaEventInfo) {}
        async fn on_registration_status_changed(&self, _info: RegistrationStatusInfo) {}
        async fn on_client_error(&self, _error: ClientError, _call_id: Option<CallId>) {}
        async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {}
    }
    
    let alice_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let alice_handler = Arc::new(TestEventHandler {
        events: alice_events.clone(),
    });
    alice.set_event_handler(alice_handler).await;
    
    println!("\n=== Creating Bob (callee) ===");
    
    // Create Bob's client
    let bob = Arc::new(
        ClientBuilder::new()
            .user_agent("Bob/1.0")
            .local_address("127.0.0.1:16101".parse().unwrap())
            .build()
            .await
            .expect("Failed to build Bob's client")
    );
    
    // Set up event handler for Bob
    struct BobEventHandler {
        events: Arc<tokio::sync::Mutex<Vec<ClientEvent>>>,
        incoming_call_id: Arc<tokio::sync::Mutex<Option<CallId>>>,
    }
    
    #[async_trait]
    impl ClientEventHandler for BobEventHandler {
        async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
            println!("Bob: Incoming call from {}", call_info.caller_uri);
            *self.incoming_call_id.lock().await = Some(call_info.call_id);
            self.events.lock().await.push(ClientEvent::IncomingCall {
                info: call_info,
                priority: rvoip_client_core::EventPriority::Normal,
            });
            CallAction::Ignore // Let the test explicitly answer
        }
        
        async fn on_call_state_changed(&self, info: CallStatusInfo) {
            self.events.lock().await.push(ClientEvent::CallStateChanged {
                info,
                priority: rvoip_client_core::EventPriority::Normal,
            });
        }
        
        async fn on_media_event(&self, _info: MediaEventInfo) {}
        async fn on_registration_status_changed(&self, _info: RegistrationStatusInfo) {}
        async fn on_client_error(&self, _error: ClientError, _call_id: Option<CallId>) {}
        async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {}
    }
    
    let bob_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let bob_call_id = Arc::new(tokio::sync::Mutex::new(None::<CallId>));
    let bob_handler = Arc::new(BobEventHandler {
        events: bob_events.clone(),
        incoming_call_id: bob_call_id.clone(),
    });
    bob.set_event_handler(bob_handler).await;
    
    // Start both clients
    alice.start().await.expect("Failed to start Alice");
    bob.start().await.expect("Failed to start Bob");
    
    println!("\n=== Alice calling Bob ===");
    
    // Alice calls Bob
    let alice_call_id = alice.make_call(
        "sip:alice@127.0.0.1:16100".to_string(),
        "sip:bob@127.0.0.1:16101".to_string(),
        Some("Test call with hold/resume".to_string()),
    ).await.expect("Failed to make call");
    
    println!("Alice: Created call {}", alice_call_id);
    
    // Wait for Bob to receive the call
    sleep(Duration::from_millis(500)).await;
    
    // Bob answers the call
    let bob_call_id = bob_call_id.lock().await.expect("Bob didn't receive call");
    
    // Check if the call is already in a state where it can't be answered
    if let Ok(bob_call_info) = bob.get_call(&bob_call_id).await {
        println!("Bob: Call {} is in state {:?}", bob_call_id, bob_call_info.state);
        if matches!(bob_call_info.state, CallState::IncomingPending) {
            bob.answer_call(&bob_call_id).await.expect("Bob failed to answer");
            println!("Bob: Answered call {}", bob_call_id);
        } else {
            println!("Bob: Call not in answerable state (IncomingPending), current state: {:?}", bob_call_info.state);
        }
    } else {
        bob.answer_call(&bob_call_id).await.expect("Bob failed to answer");
        println!("Bob: Answered call {}", bob_call_id);
    }
    
    // Wait for call to be established (with retry loop)
    let mut alice_connected = false;
    let mut bob_connected = false;
    
    for i in 0..10 {
        sleep(Duration::from_millis(500)).await;
        
        // Check Alice's call state
        if let Ok(alice_call) = alice.get_call(&alice_call_id).await {
            println!("Alice's call state (attempt {}): {:?}", i + 1, alice_call.state);
            if alice_call.state == CallState::Connected {
                alice_connected = true;
            }
        }
        
        // Check Bob's call state
        if let Ok(bob_call) = bob.get_call(&bob_call_id).await {
            println!("Bob's call state (attempt {}): {:?}", i + 1, bob_call.state);
            if bob_call.state == CallState::Connected {
                bob_connected = true;
            }
        }
        
        if alice_connected && bob_connected {
            println!("Both calls are connected!");
            break;
        }
    }
    
    // Final verification
    let alice_call = alice.get_call(&alice_call_id).await
        .expect("Failed to get Alice's call");
    assert_eq!(alice_call.state, CallState::Connected, "Alice's call should be connected");
    
    let bob_call = bob.get_call(&bob_call_id).await
        .expect("Failed to get Bob's call");
    assert_eq!(bob_call.state, CallState::Connected, "Bob's call should be connected");
    
    println!("\n=== Testing hold operation ===");
    
    // Alice puts the call on hold
    alice.hold_call(&alice_call_id).await
        .expect("Alice failed to put call on hold");
    
    println!("Alice: Put call on hold");
    
    // Wait for hold to take effect
    sleep(Duration::from_millis(500)).await;
    
    // Verify Alice's call is on hold
    let is_on_hold = alice.is_call_on_hold(&alice_call_id).await
        .expect("Failed to check Alice's hold status");
    assert!(is_on_hold, "Alice's call should be on hold");
    
    println!("Alice: Call confirmed on hold");
    
    // Let music-on-hold play for a bit
    println!("Playing music-on-hold for 2 seconds...");
    sleep(Duration::from_secs(2)).await;
    
    println!("\n=== Testing resume operation ===");
    
    // Alice resumes the call
    alice.resume_call(&alice_call_id).await
        .expect("Alice failed to resume call");
    
    println!("Alice: Resumed call");
    
    // Wait for resume to take effect
    sleep(Duration::from_millis(500)).await;
    
    // Verify Alice's call is no longer on hold
    let is_on_hold = alice.is_call_on_hold(&alice_call_id).await
        .expect("Failed to check Alice's hold status");
    assert!(!is_on_hold, "Alice's call should not be on hold");
    
    println!("Alice: Call confirmed active again");
    
    // Test multiple hold/resume cycles
    println!("\n=== Testing multiple hold/resume cycles ===");
    
    for i in 1..=3 {
        println!("\nCycle {}: Hold", i);
        alice.hold_call(&alice_call_id).await
            .expect("Failed to hold");
        sleep(Duration::from_millis(500)).await;
        
        let on_hold = alice.is_call_on_hold(&alice_call_id).await.unwrap();
        assert!(on_hold, "Call should be on hold in cycle {}", i);
        
        println!("Cycle {}: Resume", i);
        alice.resume_call(&alice_call_id).await
            .expect("Failed to resume");
        sleep(Duration::from_millis(500)).await;
        
        let on_hold = alice.is_call_on_hold(&alice_call_id).await.unwrap();
        assert!(!on_hold, "Call should be active in cycle {}", i);
    }
    
    println!("\n=== Hanging up calls ===");
    
    // Hang up the calls
    alice.hangup_call(&alice_call_id).await
        .expect("Alice failed to hang up");
    
    // Wait for hangup to propagate
    sleep(Duration::from_millis(500)).await;
    
    // Stop both clients
    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    
    // Clean up temp file
    let _ = std::fs::remove_file(&moh_file);
    
    // Check events
    let alice_events = alice_events.lock().await;
    let bob_events = bob_events.lock().await;
    
    println!("\n=== Event Summary ===");
    println!("Alice received {} events", alice_events.len());
    println!("Bob received {} events", bob_events.len());
    
    // Verify key events occurred
    assert!(alice_events.iter().any(|e| matches!(e, ClientEvent::CallStateChanged { .. })));
    assert!(bob_events.iter().any(|e| matches!(e, ClientEvent::IncomingCall { .. })));
    
    println!("\n✅ Hold/resume test completed successfully!");
}

#[tokio::test]
#[serial]
async fn test_hold_without_moh_fallback() {
    // Set up logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    println!("\n=== Testing hold without music-on-hold (fallback to mute) ===");
    
    // Create Alice's client WITHOUT music-on-hold
    let alice = Arc::new(
        ClientBuilder::new()
            .user_agent("Alice/1.0")
            .local_address("127.0.0.1:16102".parse().unwrap())
            // No music_on_hold_file!
            .build()
            .await
            .expect("Failed to build Alice's client")
    );
    
    // Create Bob's client
    let bob = Arc::new(
        ClientBuilder::new()
            .user_agent("Bob/1.0")
            .local_address("127.0.0.1:16103".parse().unwrap())
            .build()
            .await
            .expect("Failed to build Bob's client")
    );
    
    // Set up Bob to auto-answer
    let bob_call_id = Arc::new(tokio::sync::Mutex::new(None::<CallId>));
    let bob_call_id_clone = bob_call_id.clone();
    
    struct SimpleBobHandler {
        incoming_call_id: Arc<tokio::sync::Mutex<Option<CallId>>>,
    }
    
    #[async_trait]
    impl ClientEventHandler for SimpleBobHandler {
        async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
            *self.incoming_call_id.lock().await = Some(call_info.call_id);
            CallAction::Ignore // Let the test explicitly answer
        }
        
        async fn on_call_state_changed(&self, _info: CallStatusInfo) {}
        async fn on_media_event(&self, _info: MediaEventInfo) {}
        async fn on_registration_status_changed(&self, _info: RegistrationStatusInfo) {}
        async fn on_client_error(&self, _error: ClientError, _call_id: Option<CallId>) {}
        async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {}
    }
    
    let bob_handler = Arc::new(SimpleBobHandler {
        incoming_call_id: bob_call_id_clone.clone(),
    });
    bob.set_event_handler(bob_handler).await;
    
    // Start both clients
    alice.start().await.expect("Failed to start Alice");
    bob.start().await.expect("Failed to start Bob");
    
    // Alice calls Bob
    let alice_call_id = alice.make_call(
        "sip:alice@127.0.0.1:16102".to_string(),
        "sip:bob@127.0.0.1:16103".to_string(),
        None,
    ).await.expect("Failed to make call");
    
    // Wait and answer
    sleep(Duration::from_millis(500)).await;
    let bob_call_id = bob_call_id.lock().await.expect("Bob didn't receive call");
    
    // Check if the call is already in a state where it can't be answered
    if let Ok(bob_call_info) = bob.get_call(&bob_call_id).await {
        if matches!(bob_call_info.state, CallState::IncomingPending) {
            bob.answer_call(&bob_call_id).await.expect("Bob failed to answer");
        } else {
            println!("Bob: Call not in answerable state (IncomingPending), current state: {:?}", bob_call_info.state);
        }
    } else {
        bob.answer_call(&bob_call_id).await.expect("Bob failed to answer");
    }
    
    // Wait for establishment
    sleep(Duration::from_secs(2)).await;
    
    println!("Testing hold without MoH (should fallback to mute)...");
    
    // Hold the call (should fallback to mute)
    alice.hold_call(&alice_call_id).await
        .expect("Failed to hold call");
    
    // Verify on hold
    let on_hold = alice.is_call_on_hold(&alice_call_id).await.unwrap();
    assert!(on_hold, "Call should be on hold");
    
    // Resume
    alice.resume_call(&alice_call_id).await
        .expect("Failed to resume call");
    
    // Verify active
    let on_hold = alice.is_call_on_hold(&alice_call_id).await.unwrap();
    assert!(!on_hold, "Call should be active");
    
    // Clean up
    alice.hangup_call(&alice_call_id).await.ok();
    sleep(Duration::from_millis(500)).await;
    
    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    
    println!("\n✅ Hold without MoH test completed successfully!");
}

#[tokio::test]
#[serial]
async fn test_invalid_hold_states() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("Test/1.0")
        .local_address("127.0.0.1:16104".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client");
    
    client.start().await.expect("Failed to start client");
    
    // Create a call to ourselves (will fail but that's ok for this test)
    let call_id = client.make_call(
        "sip:test@127.0.0.1:16104".to_string(),
        "sip:test@127.0.0.1:16104".to_string(),
        None,
    ).await.expect("Failed to make call");
    
    // Try to hold a non-active call (should fail)
    let hold_result = client.hold_call(&call_id).await;
    assert!(hold_result.is_err(), "Should not be able to hold non-active call");
    
    // Clean up
    client.hangup_call(&call_id).await.ok();
    client.stop().await.expect("Failed to stop client");
    
    println!("\n✅ Invalid hold states test completed!");
}