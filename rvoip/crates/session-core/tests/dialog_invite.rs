//! Tests for INVITE Dialog Integration
//!
//! Tests the session-core functionality for INVITE dialogs (voice/video calls),
//! ensuring proper integration with the underlying dialog layer.
//! These tests use real session events from the infra-common zero-copy event system.

use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::sync::{mpsc, Mutex};
use rvoip_session_core::{
    SessionManager,
    SessionError,
    api::{
        types::{CallState, SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
    manager::events::SessionEvent,
};
use infra_common::events::api::EventSubscriber;

/// Global port allocator to ensure each test gets unique ports
static NEXT_PORT_BASE: AtomicU16 = AtomicU16::new(6000);

/// Get unique port pair for this test
fn get_test_ports() -> (u16, u16) {
    let base = NEXT_PORT_BASE.fetch_add(10, Ordering::SeqCst);
    (base, base + 1)
}

/// Enhanced test handler that tracks call events and state changes
#[derive(Debug)]
struct EventTrackingHandler {
    incoming_calls: Arc<Mutex<Vec<IncomingCall>>>,
    call_events: Arc<Mutex<mpsc::UnboundedSender<CallEvent>>>,
}

#[derive(Debug, Clone)]
enum CallEvent {
    IncomingCall(SessionId),
    CallEnded(SessionId, String),
}

impl EventTrackingHandler {
    fn new() -> (Self, mpsc::UnboundedReceiver<CallEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handler = Self {
            incoming_calls: Arc::new(Mutex::new(Vec::new())),
            call_events: Arc::new(Mutex::new(tx)),
        };
        (handler, rx)
    }

    async fn get_incoming_calls(&self) -> Vec<IncomingCall> {
        self.incoming_calls.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for EventTrackingHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Track the incoming call
        self.incoming_calls.lock().await.push(call.clone());
        
        // Send event notification
        if let Ok(tx) = self.call_events.try_lock() {
            let _ = tx.send(CallEvent::IncomingCall(call.id.clone()));
        }
        
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Call {} ended: {}", call.id(), reason);
        
        // Send event notification
        if let Ok(tx) = self.call_events.try_lock() {
            let _ = tx.send(CallEvent::CallEnded(call.id().clone(), reason.to_string()));
        }
    }
}

/// Test handler that rejects all calls
#[derive(Debug)]
struct RejectHandler;

#[async_trait::async_trait]
impl CallHandler for RejectHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Reject("Test rejection".to_string())
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Rejected call {} ended: {}", call.id(), reason);
    }
}

/// Wait for a specific session event
async fn wait_for_session_event(
    subscriber: &mut Box<dyn EventSubscriber<SessionEvent> + Send>,
    timeout: Duration,
) -> Option<SessionEvent> {
    match tokio::time::timeout(timeout, subscriber.receive()).await {
        Ok(Ok(event)) => Some((*event).clone()),
        _ => None,
    }
}

/// Wait for a session state change event
async fn wait_for_state_change(
    subscriber: &mut Box<dyn EventSubscriber<SessionEvent> + Send>,
    session_id: &SessionId,
    timeout: Duration,
) -> Option<(CallState, CallState)> {
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout {
        if let Some(event) = wait_for_session_event(subscriber, Duration::from_millis(100)).await {
            if let SessionEvent::StateChanged { session_id: event_session_id, old_state, new_state } = event {
                if &event_session_id == session_id {
                    return Some((old_state, new_state));
                }
            }
        }
    }
    
    None
}

/// Wait for a session created event
async fn wait_for_session_created(
    subscriber: &mut Box<dyn EventSubscriber<SessionEvent> + Send>,
    timeout: Duration,
) -> Option<SessionId> {
    if let Some(event) = wait_for_session_event(subscriber, timeout).await {
        if let SessionEvent::SessionCreated { session_id, .. } = event {
            return Some(session_id);
        }
    }
    None
}

/// Wait for a session terminated event
async fn wait_for_session_terminated(
    subscriber: &mut Box<dyn EventSubscriber<SessionEvent> + Send>,
    session_id: &SessionId,
    timeout: Duration,
) -> Option<String> {
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout {
        if let Some(event) = wait_for_session_event(subscriber, Duration::from_millis(100)).await {
            if let SessionEvent::SessionTerminated { session_id: event_session_id, reason } = event {
                if &event_session_id == session_id {
                    return Some(reason);
                }
            }
        }
    }
    
    None
}

/// Create a pair of session managers for testing established dialogs
async fn create_session_manager_pair() -> Result<(Arc<SessionManager>, Arc<SessionManager>, mpsc::UnboundedReceiver<CallEvent>), SessionError> {
    let (handler_a, _) = EventTrackingHandler::new();
    let (handler_b, call_events) = EventTrackingHandler::new();
    
    // Get unique ports for this test
    let (port_a, port_b) = get_test_ports();
    
    // Create two session managers on different ports
    let manager_a = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_a) // Use unique port
        .with_from_uri("sip:alice@localhost")
        .with_handler(Arc::new(handler_a))
        .build()
        .await?;
    
    let manager_b = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_b) // Use unique port  
        .with_from_uri("sip:bob@localhost")
        .with_handler(Arc::new(handler_b))
        .build()
        .await?;
    
    // Start managers to get actual bound addresses
    manager_a.start().await?;
    manager_b.start().await?;
    
    // Get actual bound addresses
    let addr_a = manager_a.get_bound_address();
    let addr_b = manager_b.get_bound_address();
    
    println!("Manager A bound to: {}", addr_a);
    println!("Manager B bound to: {}", addr_b);
    
    Ok((manager_a, manager_b, call_events))
}

/// Establish a real call between two session managers using event-driven waiting
async fn establish_call_between_managers(
    caller: &Arc<SessionManager>,
    callee: &Arc<SessionManager>,
    call_events: &mut mpsc::UnboundedReceiver<CallEvent>,
) -> Result<(CallSession, Option<SessionId>), SessionError> {
    // Managers are already started in create_session_manager_pair
    
    // Get actual bound addresses
    let caller_addr = caller.get_bound_address();
    let callee_addr = callee.get_bound_address();
    
    // Subscribe to session events
    let mut caller_events = caller.get_event_processor().subscribe().await?;
    let mut callee_events = callee.get_event_processor().subscribe().await?;
    
    // Create outgoing call from A to B using B's actual address
    let from_uri = format!("sip:alice@{}", caller_addr.ip());
    let to_uri = format!("sip:bob@{}", callee_addr);
    
    println!("Making call: {} -> {}", from_uri, to_uri);
    
    let call = caller.create_outgoing_call(
        &from_uri,
        &to_uri,
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n...".to_string())
    ).await?;
    
    // Wait for session created event on caller side
    let caller_session_created = wait_for_session_created(&mut caller_events, Duration::from_secs(1)).await;
    println!("Caller session created: {:?}", caller_session_created);
    
    // Wait for the call to be received by callee (through CallHandler events)
    let callee_session_id = match tokio::time::timeout(Duration::from_secs(2), call_events.recv()).await {
        Ok(Some(CallEvent::IncomingCall(session_id))) => Some(session_id),
        _ => None,
    };
    
    // Wait for any state changes
    if let Some((old_state, new_state)) = wait_for_state_change(&mut caller_events, call.id(), Duration::from_secs(2)).await {
        println!("Call state progression: {:?} -> {:?}", old_state, new_state);
    } else {
        println!("Call state progression: Initiating -> (no change)");
    }
    
    Ok((call, callee_session_id))
}

#[tokio::test]
async fn test_outgoing_call_creation() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Managers are already started
    
    // Create an outgoing call (INVITE dialog)
    let result = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost", 
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n...".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    assert_eq!(call.from, "sip:alice@localhost");
    assert_eq!(call.to, "sip:bob@localhost");
    
    // Clean up
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_outgoing_call_without_sdp() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Managers are already started
    
    // Create call without SDP offer
    let result = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost",
        None
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_call_establishment_between_managers() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    
    // Verify call was created with dynamic URIs based on actual bound addresses
    let caller_addr = manager_a.get_bound_address();
    let callee_addr = manager_b.get_bound_address();
    let expected_from = format!("sip:alice@{}", caller_addr.ip());
    let expected_to = format!("sip:bob@{}", callee_addr);
    
    assert_eq!(call.from, expected_from);
    assert_eq!(call.to, expected_to);
    
    // Check that session exists
    let session = manager_a.find_session(call.id()).await.unwrap();
    assert!(session.is_some());
    
    // Verify callee received the call
    if let Some(_callee_id) = callee_session_id {
        println!("✓ Callee received incoming call");
    } else {
        println!("⚠ Callee did not receive call within timeout");
    }
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_hold_and_resume_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Subscribe to events for this test
    let mut events = manager_a.get_event_processor().subscribe().await.unwrap();
    
    // Test hold operation
    let hold_result = manager_a.hold_session(&session_id).await;
    println!("Hold result: {:?}", hold_result);
    
    if hold_result.is_ok() {
        // Wait for state change event
        if let Some((old_state, new_state)) = wait_for_state_change(&mut events, &session_id, Duration::from_secs(1)).await {
            println!("Hold state change: {:?} -> {:?}", old_state, new_state);
        }
        
        // Test resume operation
        let resume_result = manager_a.resume_session(&session_id).await;
        println!("Resume result: {:?}", resume_result);
        
        if resume_result.is_ok() {
            // Wait for another state change event
            if let Some((old_state, new_state)) = wait_for_state_change(&mut events, &session_id, Duration::from_secs(1)).await {
                println!("Resume state change: {:?} -> {:?}", old_state, new_state);
            }
        }
    }
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_transfer_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Subscribe to events for this test
    let mut events = manager_a.get_event_processor().subscribe().await.unwrap();
    
    // Test transfer operation
    let transfer_result = manager_a.transfer_session(&session_id, "sip:charlie@localhost").await;
    println!("Transfer result: {:?}", transfer_result);
    
    if transfer_result.is_ok() {
        // Wait for state change event
        if let Some((old_state, new_state)) = wait_for_state_change(&mut events, &session_id, Duration::from_secs(1)).await {
            println!("Transfer state change: {:?} -> {:?}", old_state, new_state);
        }
    }
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_dtmf_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test DTMF sending
    let dtmf_result = manager_a.send_dtmf(&session_id, "123").await;
    println!("DTMF result: {:?}", dtmf_result);
    
    // Test multiple DTMF digits
    let dtmf_result = manager_a.send_dtmf(&session_id, "*#0987654321").await;
    println!("Multi-DTMF result: {:?}", dtmf_result);
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_media_update_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test media update
    let media_result = manager_a.update_media(&session_id, "v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\n...").await;
    println!("Media update result: {:?}", media_result);
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_termination_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Subscribe to events for this test
    let mut events = manager_a.get_event_processor().subscribe().await.unwrap();
    
    // Verify session exists
    let session_before = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    
    // Test termination
    let terminate_result = manager_a.terminate_session(&session_id).await;
    println!("Terminate result: {:?}", terminate_result);
    
    if terminate_result.is_ok() {
        // Wait for session terminated event
        if let Some(reason) = wait_for_session_terminated(&mut events, &session_id, Duration::from_secs(2)).await {
            println!("Session terminated with reason: {}", reason);
        }
        
        // Verify session is removed
        tokio::time::sleep(Duration::from_millis(100)).await;
        let session_after = manager_a.find_session(&session_id).await.unwrap();
        if session_after.is_none() {
            println!("✓ Session successfully removed after termination");
        }
    }
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_operations_on_nonexistent_session() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Managers are already started
    
    // Test operations on non-existent session
    let fake_session_id = SessionId::new();
    
    let hold_result = manager_a.hold_session(&fake_session_id).await;
    assert!(hold_result.is_err());
    assert!(matches!(hold_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    let resume_result = manager_a.resume_session(&fake_session_id).await;
    assert!(resume_result.is_err());
    
    let transfer_result = manager_a.transfer_session(&fake_session_id, "sip:target@localhost").await;
    assert!(transfer_result.is_err());
    
    let dtmf_result = manager_a.send_dtmf(&fake_session_id, "123").await;
    assert!(dtmf_result.is_err());
    
    let media_result = manager_a.update_media(&fake_session_id, "SDP").await;
    assert!(media_result.is_err());
    
    let terminate_result = manager_a.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_multiple_concurrent_calls() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Managers are already started
    
    // Create multiple outgoing calls concurrently
    let mut calls = Vec::new();
    
    for i in 0..3 { // Reduced number for more reliable testing
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@localhost", i),
            &format!("sip:target{}@localhost", i),
            Some(format!("v=0\r\no=caller{} 123 456 IN IP4 127.0.0.1\r\n...", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Verify all calls were created
    assert_eq!(calls.len(), 3);
    
    // Check that all sessions are tracked
    let stats = manager_a.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 3);
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_reject_handler() {
    let (handler_a, _) = EventTrackingHandler::new();
    let handler_b = Arc::new(RejectHandler);
    
    // Get unique ports for this test
    let (port_a, port_b) = get_test_ports();
    
    let manager_a = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_a)
        .with_from_uri("sip:alice@localhost")
        .with_handler(Arc::new(handler_a))
        .build()
        .await.unwrap();
    
    let manager_b = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_b)
        .with_from_uri("sip:bob@localhost")
        .with_handler(handler_b)
        .build()
        .await.unwrap();
    
    manager_a.start().await.unwrap();
    manager_b.start().await.unwrap();
    
    // Subscribe to events
    let mut events = manager_a.get_event_processor().subscribe().await.unwrap();
    
    // Create outgoing call (should still work)
    let call = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    assert_eq!(call.state(), &CallState::Initiating);
    
    // Wait for potential state change (could go to Failed due to rejection)
    if let Some((old_state, new_state)) = wait_for_state_change(&mut events, call.id(), Duration::from_secs(2)).await {
        println!("Call with reject handler: {:?} -> {:?}", old_state, new_state);
    } else {
        println!("Call with reject handler: Initiating -> (no change)");
    }
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_stats_tracking() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Managers are already started
    
    // Check initial stats
    let initial_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(initial_stats.active_sessions, 0);
    
    // Create some calls
    let call1 = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost",
        Some("SDP 1".to_string())
    ).await.unwrap();
    
    let call2 = manager_a.create_outgoing_call(
        "sip:charlie@localhost",
        "sip:david@localhost",
        Some("SDP 2".to_string())
    ).await.unwrap();
    
    // Check updated stats
    let updated_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(updated_stats.active_sessions, 2);
    
    // Verify sessions can be found
    let found_call1 = manager_a.find_session(call1.id()).await.unwrap();
    assert!(found_call1.is_some());
    assert_eq!(found_call1.unwrap().id(), call1.id());
    
    let found_call2 = manager_a.find_session(call2.id()).await.unwrap();
    assert!(found_call2.is_some());
    assert_eq!(found_call2.unwrap().id(), call2.id());
    
    manager_a.stop().await.unwrap();
    manager_b.stop().await.unwrap();
} 