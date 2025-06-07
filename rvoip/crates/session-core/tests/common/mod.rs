//! Common Test Helpers for Session-Core Dialog Testing
//!
//! This module provides shared test utilities for testing session-core functionality
//! across different SIP dialog types (INVITE, BYE, INFO, etc.).
//! 
//! The helpers ensure consistent test setup and provide real event-driven testing
//! using the infra-common zero-copy event system.

// Re-export media-core integration test utilities
pub mod media_test_utils;
pub use media_test_utils::*;

// Re-export bridge test utilities
pub mod bridge_test_utils;
pub use bridge_test_utils::*;

// Re-export manager test utilities
pub mod manager_test_utils;
pub use manager_test_utils::*;

// Re-export session test utilities
pub mod session_test_utils;
pub use session_test_utils::*;

// Re-export API test utilities  
pub mod api_test_utils;
pub use api_test_utils::*;

// Re-export coordination test utilities
pub mod coordination_test_utils;
pub use coordination_test_utils::*;

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
pub fn get_test_ports() -> (u16, u16) {
    let base = NEXT_PORT_BASE.fetch_add(10, Ordering::SeqCst);
    (base, base + 1)
}

/// Enhanced test handler that tracks call events and state changes
#[derive(Debug)]
pub struct EventTrackingHandler {
    incoming_calls: Arc<Mutex<Vec<IncomingCall>>>,
    call_events: Arc<Mutex<mpsc::UnboundedSender<CallEvent>>>,
    ended_calls: Arc<Mutex<Vec<(SessionId, String)>>>,
}

#[derive(Debug, Clone)]
pub enum CallEvent {
    IncomingCall(SessionId),
    CallEnded(SessionId, String),
}

impl EventTrackingHandler {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<CallEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let handler = Self {
            incoming_calls: Arc::new(Mutex::new(Vec::new())),
            call_events: Arc::new(Mutex::new(tx)),
            ended_calls: Arc::new(Mutex::new(Vec::new())),
        };
        (handler, rx)
    }

    pub async fn get_incoming_calls(&self) -> Vec<IncomingCall> {
        self.incoming_calls.lock().await.clone()
    }

    pub async fn get_ended_calls(&self) -> Vec<(SessionId, String)> {
        self.ended_calls.lock().await.clone()
    }

    pub async fn clear_events(&self) {
        self.incoming_calls.lock().await.clear();
        self.ended_calls.lock().await.clear();
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
        
        // Track the ended call
        self.ended_calls.lock().await.push((call.id().clone(), reason.to_string()));
        
        // Send event notification
        if let Ok(tx) = self.call_events.try_lock() {
            let _ = tx.send(CallEvent::CallEnded(call.id().clone(), reason.to_string()));
        }
    }
}

/// Test handler that rejects all calls
#[derive(Debug)]
pub struct RejectHandler;

#[async_trait::async_trait]
impl CallHandler for RejectHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Reject("Test rejection".to_string())
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Rejected call {} ended: {}", call.id(), reason);
    }
}

/// Test handler that defers all calls (for queue testing)
#[derive(Debug)]
pub struct DeferHandler;

#[async_trait::async_trait]
impl CallHandler for DeferHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Defer
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Deferred call {} ended: {}", call.id(), reason);
    }
}

/// Wait for a specific session event
pub async fn wait_for_session_event(
    subscriber: &mut Box<dyn EventSubscriber<SessionEvent> + Send>,
    timeout: Duration,
) -> Option<SessionEvent> {
    match tokio::time::timeout(timeout, subscriber.receive()).await {
        Ok(Ok(event)) => Some((*event).clone()),
        _ => None,
    }
}

/// Wait for a session state change event
pub async fn wait_for_state_change(
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
pub async fn wait_for_session_created(
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
pub async fn wait_for_session_terminated(
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

/// Create a single session manager for testing
pub async fn create_session_manager(
    handler: Arc<dyn CallHandler>,
    port: Option<u16>,
    from_uri: Option<&str>,
) -> Result<Arc<SessionManager>, SessionError> {
    let port = port.unwrap_or_else(|| get_test_ports().0);
    let from_uri = from_uri.unwrap_or("sip:test@localhost");
    
    let manager = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port)
        .with_from_uri(from_uri)
        .with_handler(handler)
        .build()
        .await?;
    
    manager.start().await?;
    Ok(manager)
}

/// Create a pair of session managers for testing established dialogs
pub async fn create_session_manager_pair() -> Result<(Arc<SessionManager>, Arc<SessionManager>, mpsc::UnboundedReceiver<CallEvent>), SessionError> {
    let (handler_a, _) = EventTrackingHandler::new();
    let (handler_b, call_events) = EventTrackingHandler::new();
    
    // Get unique ports for this test
    let (port_a, port_b) = get_test_ports();
    
    // Create two session managers on different ports
    let manager_a = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_a)
        .with_from_uri("sip:alice@localhost")
        .with_handler(Arc::new(handler_a))
        .build()
        .await?;
    
    let manager_b = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_b)
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

/// Create a test session manager pair with custom handlers
pub async fn create_session_manager_pair_with_handlers(
    handler_a: Arc<dyn CallHandler>,
    handler_b: Arc<dyn CallHandler>,
) -> Result<(Arc<SessionManager>, Arc<SessionManager>), SessionError> {
    // Get unique ports for this test
    let (port_a, port_b) = get_test_ports();
    
    // Create two session managers on different ports
    let manager_a = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_a)
        .with_from_uri("sip:alice@localhost")
        .with_handler(handler_a)
        .build()
        .await?;
    
    let manager_b = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(port_b)
        .with_from_uri("sip:bob@localhost")
        .with_handler(handler_b)
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
    
    Ok((manager_a, manager_b))
}

/// Establish a real call between two session managers using event-driven waiting
pub async fn establish_call_between_managers(
    caller: &Arc<SessionManager>,
    callee: &Arc<SessionManager>,
    call_events: &mut mpsc::UnboundedReceiver<CallEvent>,
) -> Result<(CallSession, Option<SessionId>), SessionError> {
    establish_call_between_managers_with_sdp(
        caller,
        callee, 
        call_events,
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n...".to_string())
    ).await
}

/// Establish a call with custom SDP
pub async fn establish_call_between_managers_with_sdp(
    caller: &Arc<SessionManager>,
    callee: &Arc<SessionManager>,
    call_events: &mut mpsc::UnboundedReceiver<CallEvent>,
    sdp: Option<String>,
) -> Result<(CallSession, Option<SessionId>), SessionError> {
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
    
    let call = caller.create_outgoing_call(&from_uri, &to_uri, sdp).await?;
    
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

/// Verify that a session exists and has the expected properties
pub async fn verify_session_exists(
    manager: &Arc<SessionManager>,
    session_id: &SessionId,
    expected_state: Option<&CallState>,
) -> Result<CallSession, SessionError> {
    let session = manager.find_session(session_id).await?
        .ok_or_else(|| SessionError::session_not_found(&session_id.to_string()))?;
    
    if let Some(expected) = expected_state {
        assert_eq!(session.state(), expected, "Session state mismatch");
    }
    
    Ok(session)
}

/// Verify that a session no longer exists
pub async fn verify_session_removed(
    manager: &Arc<SessionManager>,
    session_id: &SessionId,
) -> Result<(), SessionError> {
    let session = manager.find_session(session_id).await?;
    assert!(session.is_none(), "Session {} should have been removed", session_id);
    Ok(())
}

/// Wait for call events with timeout
pub async fn wait_for_call_event(
    call_events: &mut mpsc::UnboundedReceiver<CallEvent>,
    timeout: Duration,
) -> Option<CallEvent> {
    match tokio::time::timeout(timeout, call_events.recv()).await {
        Ok(Some(event)) => Some(event),
        _ => None,
    }
}

/// Clean shutdown of session managers
pub async fn cleanup_managers(managers: Vec<Arc<SessionManager>>) -> Result<(), SessionError> {
    for manager in managers {
        manager.stop().await?;
    }
    Ok(())
}

/// Test configuration for consistent testing
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub default_timeout: Duration,
    pub call_establishment_timeout: Duration,
    pub state_change_timeout: Duration,
    pub cleanup_delay: Duration,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(1),
            call_establishment_timeout: Duration::from_secs(2),
            state_change_timeout: Duration::from_secs(1),
            cleanup_delay: Duration::from_millis(50),
        }
    }
}

impl TestConfig {
    pub fn fast() -> Self {
        Self {
            default_timeout: Duration::from_millis(500),
            call_establishment_timeout: Duration::from_secs(1),
            state_change_timeout: Duration::from_millis(500),
            cleanup_delay: Duration::from_millis(25),
        }
    }
    
    pub fn slow() -> Self {
        Self {
            default_timeout: Duration::from_secs(2),
            call_establishment_timeout: Duration::from_secs(5),
            state_change_timeout: Duration::from_secs(2),
            cleanup_delay: Duration::from_millis(100),
        }
    }
}

// Re-export commonly used items
pub use bridge_test_utils::*;
pub use media_test_utils::*;
pub use manager_test_utils::*;
pub use session_test_utils::*;
pub use coordination_test_utils::*; 