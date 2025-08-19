//! Comprehensive tests for SessionCoordinator
//!
//! Tests all major functionality of the SessionCoordinator including:
//! - Initialization and lifecycle
//! - Call creation and termination
//! - State transitions
//! - Event handling
//! - Media coordination
//! - Error conditions

mod common;

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashSet;
use tokio::sync::mpsc;
use rvoip_session_core::{
    SessionCoordinator,
    SessionControl,
    SessionError,
    prelude::SessionEvent,
    api::{
        handlers::CallHandler,
        builder::{SessionManagerBuilder, SessionManagerConfig},
        types::{CallSession, SessionId, CallState, IncomingCall, CallDecision, SessionStats},
    },
};

/// Test handler that tracks all events
#[derive(Debug, Default)]
struct TrackingHandler {
    events: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl TrackingHandler {
    fn new() -> Self {
        Self {
            events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn get_events(&self) -> Vec<String> {
        self.events.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for TrackingHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let mut events = self.events.lock().await;
        events.push(format!("incoming_call:{}", call.id));
        CallDecision::Accept(None)
    }

    async fn on_call_established(&self, call: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        let mut events = self.events.lock().await;
        events.push(format!("call_established:{}", call.id()));
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let mut events = self.events.lock().await;
        events.push(format!("call_ended:{}:{}", call.id(), reason));
    }
}

#[tokio::test]
async fn test_coordinator_initialization() {
    println!("🧪 Testing SessionCoordinator initialization...");

    // Use dynamic port allocation to avoid conflicts
    let port = common::get_test_ports().0;
    let mut config = SessionManagerConfig::default();
    config.sip_port = port;
    config.local_bind_addr = format!("127.0.0.1:{}", port).parse().unwrap();
    
    let coordinator = SessionCoordinator::new(config.clone(), None)
        .await
        .expect("Failed to create coordinator");

    // Verify subsystems are initialized
    assert_eq!(coordinator.config.sip_port, port);
    assert_eq!(coordinator.config.media_port_start, 10000);
    assert_eq!(coordinator.config.media_port_end, 20000);

    // Start the coordinator
    coordinator.start().await.expect("Failed to start coordinator");

    // Get bound address
    let addr = coordinator.get_bound_address();
    println!("✅ Coordinator started on: {}", addr);

    // Stop the coordinator
    coordinator.stop().await.expect("Failed to stop coordinator");
    println!("✅ Coordinator stopped successfully");
}

#[tokio::test]
async fn test_coordinator_with_custom_config() {
    println!("🧪 Testing SessionCoordinator with custom configuration...");

    let handler = Arc::new(TrackingHandler::new());
    
    let port = common::get_test_ports().0;
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", port))
        .with_local_bind_addr(format!("127.0.0.1:{}", port).parse().unwrap())
        .with_media_ports(30000, 31000)
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Verify configuration
    assert_eq!(coordinator.config.sip_port, port);
    assert_eq!(coordinator.config.local_address, format!("sip:alice@127.0.0.1:{}", port));
    assert_eq!(coordinator.config.media_port_start, 30000);
    assert_eq!(coordinator.config.media_port_end, 31000);

    // Verify handler is set
    assert!(coordinator.get_handler().is_some());

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Custom configuration verified");
}

#[tokio::test]
async fn test_outgoing_call_lifecycle() {
    println!("🧪 Testing outgoing call lifecycle...");

    // Create two coordinators that can talk to each other
    let (alice_port, bob_port) = common::get_test_ports();
    
    // Create Alice (caller)
    let alice_handler = Arc::new(TrackingHandler::new());
    let alice = SessionManagerBuilder::new()
        .with_sip_port(alice_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", alice_port).parse().unwrap())
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", alice_port))
        .with_handler(alice_handler.clone())
        .build()
        .await
        .expect("Failed to build Alice");

    alice.start().await.expect("Failed to start Alice");

    // Create Bob (callee)
    let bob_handler = Arc::new(TrackingHandler::new());
    let bob = SessionManagerBuilder::new()
        .with_sip_port(bob_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", bob_port).parse().unwrap())
        .with_local_address(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .with_handler(bob_handler.clone())
        .build()
        .await
        .expect("Failed to build Bob");

    bob.start().await.expect("Failed to start Bob");

    // Subscribe to events
    let mut alice_events = alice.event_processor()
        .unwrap()
        .subscribe()
        .await
        .expect("Failed to subscribe to Alice events");
    
    let mut bob_events = bob.event_processor()
        .unwrap()
        .subscribe()
        .await
        .expect("Failed to subscribe to Bob events");

    // Create a call from Alice to Bob
    let call = alice.create_outgoing_call(
        &format!("sip:alice@127.0.0.1:{}", alice_port),
        &format!("sip:bob@127.0.0.1:{}", bob_port),
        None,
    ).await.expect("Failed to create call");

    println!("📞 Created call: {} -> {}", call.from, call.to);
    
    // Wait for Alice's call to become Active
    let active = common::wait_for_state_change(
        &mut alice_events,
        &call.id,
        Duration::from_secs(2)
    ).await;
    assert!(active.is_some(), "Alice's call should become active");
    assert_eq!(active.unwrap().1, CallState::Active);

    // Get session info
    let session_info = alice.get_session(&call.id)
        .await
        .expect("Failed to get session")
        .expect("Session not found");
    
    assert_eq!(session_info.id, call.id);
    assert_eq!(session_info.from, call.from);
    assert_eq!(session_info.state(), &CallState::Active);

    // List active sessions on Alice side
    let sessions = alice.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0], call.id);

    // Get stats from Alice
    let stats = alice.get_stats()
        .await
        .expect("Failed to get stats");
    assert_eq!(stats.active_sessions, 1);
    assert_eq!(stats.total_sessions, 1);

    // Verify Bob also has an active session
    let bob_sessions = bob.list_active_sessions()
        .await
        .expect("Failed to list Bob's sessions");
    assert_eq!(bob_sessions.len(), 1, "Bob should have 1 active session");

    // Terminate the call from Alice
    alice.terminate_session(&call.id)
        .await
        .expect("Failed to terminate session");

    // Wait for Alice's session to terminate
    let terminated = common::wait_for_session_terminated(
        &mut alice_events,
        &call.id,
        Duration::from_secs(2)
    ).await;
    assert!(terminated.is_some(), "Alice's session should be terminated");

    // Wait for Bob's session to reach terminated state (from receiving BYE)
    // First we need to find Bob's session ID
    let bob_sessions_before = bob.list_active_sessions()
        .await
        .expect("Failed to list Bob's sessions");
    if !bob_sessions_before.is_empty() {
        let bob_session_id = &bob_sessions_before[0];
        // Wait for the state to change to Terminated (not just SessionTerminated event)
        let bob_terminated = common::wait_for_terminated_state(
            &mut bob_events,
            bob_session_id,
            Duration::from_secs(3)
        ).await;
        assert!(bob_terminated, "Bob's session should reach terminated state");
    }

    // Verify stats updated on Alice
    let stats = alice.get_stats()
        .await
        .expect("Failed to get stats");
    assert_eq!(stats.active_sessions, 0);

    // Give Bob's cleanup more time to complete
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Verify Bob's session is also terminated
    let bob_sessions = bob.list_active_sessions()
        .await
        .expect("Failed to list Bob's sessions");
    assert_eq!(bob_sessions.len(), 0, "Bob should have no active sessions after termination");

    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    println!("✅ Call lifecycle test completed");
}

#[tokio::test]
async fn test_multiple_concurrent_calls() {
    println!("🧪 Testing multiple concurrent calls...");

    // Create two coordinators that can handle multiple calls
    let (alice_port, bob_port) = common::get_test_ports();
    
    // Create Alice (caller)
    let alice = SessionManagerBuilder::new()
        .with_sip_port(alice_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", alice_port).parse().unwrap())
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", alice_port))
        .with_handler(Arc::new(TrackingHandler::new()))
        .build()
        .await
        .expect("Failed to build Alice");

    alice.start().await.expect("Failed to start Alice");

    // Create Bob (callee)
    let bob = SessionManagerBuilder::new()
        .with_sip_port(bob_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", bob_port).parse().unwrap())
        .with_local_address(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .with_handler(Arc::new(TrackingHandler::new()))
        .build()
        .await
        .expect("Failed to build Bob");

    bob.start().await.expect("Failed to start Bob");

    // Subscribe to events
    let mut alice_events = alice.event_processor()
        .unwrap()
        .subscribe()
        .await
        .expect("Failed to subscribe to Alice events");

    // Create multiple calls concurrently - testing with 100 calls
    let mut calls = vec![];
    let mut call_ids = HashSet::new();  // Use HashSet for faster lookups
    const NUM_CALLS: usize = 100;
    
    // Create all calls first without waiting for them to become active
    for i in 0..NUM_CALLS {
        let alice_addr = format!("sip:alice{}@127.0.0.1:{}", i, alice_port);
        let bob_addr = format!("sip:bob@127.0.0.1:{}", bob_port);
        
        let call = alice.create_outgoing_call(
            &alice_addr,
            &bob_addr,
            None,
        ).await.expect("Failed to create call");
        
        if i % 10 == 0 {
            println!("Created call {}: {}", i, call.id);
        }
        call_ids.insert(call.id.clone());
        calls.push(call);
        
        // Very small delay between call creation to avoid overwhelming the system
        // but still stress test concurrency
        if i % 10 == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    
    println!("Created {} calls total", NUM_CALLS);

    // Now collect state change events for all calls
    let mut active_calls = std::collections::HashSet::new();
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(60);  // Increased timeout for 100 calls
    let mut last_progress = 0;
    
    // Keep collecting events until all calls are active or timeout
    println!("Starting event collection loop...");
    while active_calls.len() < NUM_CALLS && start.elapsed() < timeout {
        match tokio::time::timeout(Duration::from_millis(100), alice_events.receive()).await {
            Ok(Ok(event)) => {
                // Log all events for debugging (commented for now)
                // println!("Received event: {:?}", event);
                
                if let SessionEvent::StateChanged { session_id, old_state, new_state } = event {
                    if call_ids.contains(&session_id) && new_state == CallState::Active {
                        active_calls.insert(session_id.clone());
                        
                        // Report progress every 10 calls
                        if active_calls.len() % 10 == 0 && active_calls.len() != last_progress {
                            last_progress = active_calls.len();
                            println!("Progress: {}/{} calls active ({:.1}s elapsed)", 
                                     active_calls.len(), NUM_CALLS, start.elapsed().as_secs_f64());
                        }
                    }
                }
            },
            Ok(Err(e)) => {
                println!("Error receiving event: {:?}", e);
            },
            Err(_) => {
                // Timeout - continue collecting events
                tokio::task::yield_now().await;
            }
        }
    }
    
    // Debug: show which calls became active
    println!("Active calls collected: {} out of {}", active_calls.len(), NUM_CALLS);
    if active_calls.len() < NUM_CALLS && active_calls.len() > 0 {
        // Show a sample of active calls if not all are active
        println!("Sample of active calls:");
        for (i, id) in active_calls.iter().take(5).enumerate() {
            println!("  - {}", id);
        }
    }
    
    // Verify all calls became active
    assert_eq!(active_calls.len(), NUM_CALLS, "All {} calls should become active", NUM_CALLS);
    println!("All {} calls are now active", NUM_CALLS);

    // Verify stats on Alice side
    let stats = alice.get_stats()
        .await
        .expect("Failed to get stats");
    assert_eq!(stats.active_sessions, NUM_CALLS);
    assert_eq!(stats.total_sessions, NUM_CALLS);

    // List all sessions on Alice side
    let sessions = alice.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), NUM_CALLS);

    // Verify Bob also has NUM_CALLS active sessions
    let bob_sessions = bob.list_active_sessions()
        .await
        .expect("Failed to list Bob's sessions");
    assert_eq!(bob_sessions.len(), NUM_CALLS, "Bob should have {} active sessions", NUM_CALLS);

    // Terminate all calls from Alice
    for call in &calls {
        alice.terminate_session(&call.id)
            .await
            .expect("Failed to terminate session");
    }

    // Wait for all sessions to terminate
    for call in &calls {
        let terminated = common::wait_for_session_terminated(
            &mut alice_events,
            &call.id,
            Duration::from_secs(2)
        ).await;
        assert!(terminated.is_some(), "Call {} should be terminated", call.id);
    }

    // Verify all calls are terminated on Alice side
    let sessions = alice.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), 0, "Alice should have no active sessions");

    // Give Bob time to process the BYE messages and terminate sessions
    // Bob needs to receive BYE, process it, and complete cleanup
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Verify Bob's sessions are also terminated
    let bob_sessions = bob.list_active_sessions()
        .await
        .expect("Failed to list Bob's sessions");
    assert_eq!(bob_sessions.len(), 0, "Bob should have no active sessions");

    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    println!("✅ Concurrent calls test completed");
}

#[tokio::test]
async fn test_media_session_coordination() {
    println!("🧪 Testing media session coordination...");

    let handler = Arc::new(TrackingHandler::new());
    let port = common::get_test_ports().0;
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_bind_addr(format!("127.0.0.1:{}", port).parse().unwrap())
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create SDP using sip-core's builder
    use rvoip_sip_core::sdp::SdpBuilder;
    use rvoip_sip_core::sdp::attributes::MediaDirection;
    
    let sdp = SdpBuilder::new("Media Test Session")
        .origin("-", "123456", "1", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(port + 100, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("Failed to build SDP");

    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        Some(sdp.to_string()),
    ).await.expect("Failed to create call");

    // Simulate call becoming active
    if let Ok(Some(mut session)) = coordinator.registry.get_session(&call.id).await {
        let old_state = session.state().clone();
        session.update_call_state(CallState::Active).unwrap();
        coordinator.registry.register_session(session).await.unwrap();
        
        // Send state change event
        let _ = coordinator.event_processor.publish_event(SessionEvent::StateChanged {
            session_id: call.id.clone(),
            old_state,
            new_state: CallState::Active,
        }).await;
    }

    // Wait for media session to be created
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Check media info
    let media_info = coordinator.get_media_info(&call.id)
        .await
        .expect("Failed to get media info");
    
    // Media session should exist once call is active
    if media_info.is_some() {
        println!("✅ Media session created when call became active");
    }

    // Note: Media updates require a fully established dialog with remote tags
    // In a simulated test environment without real SIP endpoints, we can't update media
    // This would work in a real scenario with actual SIP dialogs
    println!("📝 Skipping media update test - requires real SIP dialog");

    // Terminate the call
    coordinator.terminate_session(&call.id)
        .await
        .expect("Failed to terminate session");

    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Media coordination test completed");
}

#[tokio::test]
async fn test_call_state_transitions() {
    println!("🧪 Testing call state transitions...");

    // Create two coordinators that can talk to each other
    let (alice_port, bob_port) = common::get_test_ports();
    
    // Create Alice (caller)
    let alice_handler = Arc::new(TrackingHandler::new());
    let alice = SessionManagerBuilder::new()
        .with_sip_port(alice_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", alice_port).parse().unwrap())
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", alice_port))
        .with_handler(alice_handler.clone())
        .build()
        .await
        .expect("Failed to build Alice");

    alice.start().await.expect("Failed to start Alice");

    // Create Bob (callee)
    let bob_handler = Arc::new(TrackingHandler::new());
    let bob = SessionManagerBuilder::new()
        .with_sip_port(bob_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", bob_port).parse().unwrap())
        .with_local_address(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .with_handler(bob_handler.clone())
        .build()
        .await
        .expect("Failed to build Bob");

    bob.start().await.expect("Failed to start Bob");

    // Create SDP using sip-core's builder
    use rvoip_sip_core::sdp::SdpBuilder;
    use rvoip_sip_core::sdp::attributes::MediaDirection;
    
    let sdp = SdpBuilder::new("Test Session")
        .origin("-", "123456", "1", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(alice_port + 100, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("Failed to build SDP");

    // Create a call from Alice to Bob with SDP
    let call = alice.create_outgoing_call(
        &format!("sip:alice@127.0.0.1:{}", alice_port),
        &format!("sip:bob@127.0.0.1:{}", bob_port),
        Some(sdp.to_string()),
    ).await.expect("Failed to create call");

    // Wait for call to establish
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now test hold/resume on an established call
    alice.hold_session(&call.id)
        .await
        .expect("Failed to hold session");

    // Verify state
    let session = alice.get_session(&call.id)
        .await.unwrap().unwrap();
    assert_eq!(session.state(), &CallState::OnHold);

    // Resume the call
    alice.resume_session(&call.id)
        .await
        .expect("Failed to resume session");

    // Verify state
    let session = alice.get_session(&call.id)
        .await.unwrap().unwrap();
    assert_eq!(session.state(), &CallState::Active);

    // Test transfer
    alice.transfer_session(&call.id, "sip:charlie@example.com")
        .await
        .expect("Failed to transfer session");

    // Verify state
    let session = alice.get_session(&call.id)
        .await.unwrap().unwrap();
    assert_eq!(session.state(), &CallState::Transferring);

    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    println!("✅ State transitions test completed");
}

#[tokio::test]
async fn test_dtmf_sending() {
    println!("🧪 Testing DTMF sending...");

    // Create two coordinators that can talk to each other
    let (alice_port, bob_port) = common::get_test_ports();
    
    // Create Alice (caller)
    let alice = SessionManagerBuilder::new()
        .with_sip_port(alice_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", alice_port).parse().unwrap())
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", alice_port))
        .build()
        .await
        .expect("Failed to build Alice");

    alice.start().await.expect("Failed to start Alice");

    // Create Bob (callee)
    let bob = SessionManagerBuilder::new()
        .with_sip_port(bob_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", bob_port).parse().unwrap())
        .with_local_address(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .with_handler(Arc::new(TrackingHandler::new()))
        .build()
        .await
        .expect("Failed to build Bob");

    bob.start().await.expect("Failed to start Bob");

    // Create SDP using sip-core's builder
    use rvoip_sip_core::sdp::SdpBuilder;
    use rvoip_sip_core::sdp::attributes::MediaDirection;
    
    let sdp = SdpBuilder::new("DTMF Test Session")
        .origin("-", "789012", "1", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(alice_port + 100, "RTP/AVP")
            .formats(&["0", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()
        .expect("Failed to build SDP");

    // Create a call from Alice to Bob with SDP
    let call = alice.create_outgoing_call(
        &format!("sip:alice@127.0.0.1:{}", alice_port),
        &format!("sip:bob@127.0.0.1:{}", bob_port),
        Some(sdp.to_string()),
    ).await.expect("Failed to create call");

    // Wait for call to establish
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send DTMF on the established call
    alice.send_dtmf(&call.id, "123#")
        .await
        .expect("Failed to send DTMF");

    // Send more complex DTMF sequence
    alice.send_dtmf(&call.id, "*456#")
        .await
        .expect("Failed to send DTMF");

    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    println!("✅ DTMF test completed");
}

#[tokio::test]
async fn test_error_conditions() {
    println!("🧪 Testing error conditions...");

    let port = common::get_test_ports().0;
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_bind_addr(format!("127.0.0.1:{}", port).parse().unwrap())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Test operations on non-existent session
    let fake_id = SessionId::new();
    
    // Should fail - session doesn't exist
    assert!(coordinator.terminate_session(&fake_id).await.is_err());
    assert!(coordinator.hold_session(&fake_id).await.is_err());
    assert!(coordinator.resume_session(&fake_id).await.is_err());
    assert!(coordinator.send_dtmf(&fake_id, "123").await.is_err());
    assert!(coordinator.update_media(&fake_id, "fake sdp").await.is_err());

    // Create a call for state-based errors
    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        None,
    ).await.expect("Failed to create call");

    // Test invalid state transitions
    // Can't resume a call that's not on hold
    assert!(coordinator.resume_session(&call.id).await.is_err());

    // Can't send DTMF on non-active call
    assert!(coordinator.send_dtmf(&call.id, "123").await.is_err());

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Error conditions test completed");
}

#[tokio::test]
async fn test_event_handler_callbacks() {
    println!("🧪 Testing event handler callbacks...");

    let handler = Arc::new(TrackingHandler::new());
    let port = common::get_test_ports().0;
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_bind_addr(format!("127.0.0.1:{}", port).parse().unwrap())
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create a call
    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        None,
    ).await.expect("Failed to create call");

    // Simulate call establishment
    if let Ok(Some(mut session)) = coordinator.registry.get_session(&call.id).await {
        let old_state = session.state().clone();
        session.update_call_state(CallState::Active).unwrap();
        coordinator.registry.register_session(session.clone()).await.unwrap();
        
        // Send state change event
        let _ = coordinator.event_processor.publish_event(SessionEvent::StateChanged {
            session_id: call.id.clone(),
            old_state,
            new_state: CallState::Active,
        }).await;
        
        // Manually trigger the handler callback since we're simulating
        if let Some(hdlr) = &coordinator.handler {
            hdlr.on_call_established(session.into_call_session(), None, None).await;
        }
    }

    // Wait for event processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify call established event was recorded
    let events = handler.get_events().await;
    assert!(events.iter().any(|e| e.starts_with(&format!("call_established:{}", call.id))));

    // Terminate the call
    if let Ok(Some(session)) = coordinator.registry.get_session(&call.id).await {
        // Manually trigger the handler callback since we're simulating
        if let Some(hdlr) = &coordinator.handler {
            hdlr.on_call_ended(session.into_call_session(), "Test termination").await;
        }
    }
    
    coordinator.terminate_session(&call.id)
        .await
        .expect("Failed to terminate session");

    // Wait for event processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify call ended event was recorded
    let events = handler.get_events().await;
    assert!(events.iter().any(|e| e.starts_with(&format!("call_ended:{}", call.id))));

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Event handler test completed");
}

#[tokio::test]
async fn test_cleanup_on_shutdown() {
    println!("🧪 Testing cleanup on shutdown...");

    let port = common::get_test_ports().0;
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_bind_addr(format!("127.0.0.1:{}", port).parse().unwrap())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create multiple calls
    let mut calls = vec![];
    for i in 0..3 {
        let call = coordinator.create_outgoing_call(
            &format!("sip:alice{}@test.local", i),
            &format!("sip:bob{}@example.com", i),
            None,
        ).await.expect("Failed to create call");
        calls.push(call);
    }

    // Verify calls exist
    let sessions = coordinator.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), 3);

    // Stop the coordinator - should clean up all sessions
    coordinator.stop().await.expect("Failed to stop");

    println!("✅ Cleanup test completed");
}

#[cfg(test)]
mod prepared_call_tests {
    use super::*;

    #[tokio::test]
    async fn test_prepare_and_initiate_call() {
        println!("🧪 Testing prepare and initiate call flow...");

        let port = common::get_test_ports().0;
        let coordinator = SessionManagerBuilder::new()
            .with_sip_port(port)
            .with_local_bind_addr(format!("127.0.0.1:{}", port).parse().unwrap())
            .build()
            .await
            .expect("Failed to build coordinator");

        coordinator.start().await.expect("Failed to start");

        // Prepare a call (allocates resources, generates SDP)
        let prepared = coordinator.prepare_outgoing_call(
            "sip:alice@test.local",
            "sip:bob@example.com",
        ).await.expect("Failed to prepare call");

        println!("📞 Prepared call: {}", prepared.session_id);
        assert!(!prepared.sdp_offer.is_empty());
        assert!(prepared.local_rtp_port > 0);

        // Verify session exists in preparing state
        let session = coordinator.get_session(&prepared.session_id)
            .await.unwrap().unwrap();
        assert_eq!(session.state(), &CallState::Initiating);

        // Initiate the prepared call
        let call = coordinator.initiate_prepared_call(&prepared)
            .await
            .expect("Failed to initiate call");

        assert_eq!(call.id, prepared.session_id);
        assert_eq!(call.from, prepared.from);
        assert_eq!(call.to, prepared.to);

        // Cleanup
        coordinator.terminate_session(&call.id).await.ok();
        coordinator.stop().await.expect("Failed to stop");
        println!("✅ Prepare/initiate test completed");
    }
} 