use rvoip_session_core::api::control::SessionControl;
// Common Test Helpers for Session-Core Dialog Testing
//
// This module provides shared test utilities for testing session-core functionality
// across different SIP dialog types (INVITE, BYE, INFO, etc.).
// 
// The helpers ensure consistent test setup and provide real event-driven testing
// using the infra-common zero-copy event system.

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
    SessionCoordinator,
    SessionError,
    api::{
        types::{CallState, SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,

    },
    manager::events::{SessionEvent, SessionEventSubscriber},
};

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
        
        CallDecision::Accept(None)
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

/// Wait for a session state change event
pub async fn wait_for_state_change(
    subscriber: &mut SessionEventSubscriber,
    session_id: &SessionId,
    timeout: Duration,
) -> Option<(CallState, CallState)> {
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout {
        match tokio::time::timeout(Duration::from_millis(100), subscriber.receive()).await {
            Ok(Ok(event)) => {
                if let SessionEvent::StateChanged { session_id: event_session_id, old_state, new_state } = event {
                    if &event_session_id == session_id {
                        return Some((old_state, new_state));
                    }
                }
            },
            _ => {}
        }
    }
    
    None
}

/// Wait for a session created event
pub async fn wait_for_session_created(
    subscriber: &mut SessionEventSubscriber,
    timeout: Duration,
) -> Option<SessionId> {
    match tokio::time::timeout(timeout, subscriber.receive()).await {
        Ok(Ok(event)) => {
            if let SessionEvent::SessionCreated { session_id, .. } = event {
                return Some(session_id);
            }
        },
        _ => {}
    }
    None
}

/// Wait for a session terminated event
pub async fn wait_for_session_terminated(
    subscriber: &mut SessionEventSubscriber,
    session_id: &SessionId,  
    timeout: Duration,
) -> Option<String> {
    let start = std::time::Instant::now();
    
    // First yield to allow any pending events to propagate
    tokio::task::yield_now().await;
    
    while start.elapsed() < timeout {
        match tokio::time::timeout(Duration::from_millis(100), subscriber.receive()).await {
            Ok(Ok(event)) => {
                println!("Received event while waiting for termination: {:?}", event);
                if let SessionEvent::SessionTerminated { session_id: event_session_id, reason } = event {
                    if &event_session_id == session_id {
                        return Some(reason);
                    }
                }
            },
            Ok(Err(e)) => {
                println!("Error receiving event: {:?}", e);
            },
            Err(_) => {
                // Timeout, continue
                tokio::task::yield_now().await;
            }
        }
    }
    
    println!("Timeout waiting for SessionTerminated event for session {}", session_id);
    None
}

/// Wait for a session to transition to Terminated state
pub async fn wait_for_terminated_state(
    subscriber: &mut SessionEventSubscriber,
    session_id: &SessionId,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout {
        match tokio::time::timeout(Duration::from_millis(100), subscriber.receive()).await {
            Ok(Ok(event)) => {
                if let SessionEvent::StateChanged { session_id: event_session_id, new_state, .. } = event {
                    if &event_session_id == session_id && matches!(new_state, CallState::Terminated) {
                        return true;
                    }
                }
            },
            _ => {
                tokio::task::yield_now().await;
            }
        }
    }
    
    false
}

/// Create a single session manager for testing
pub async fn create_session_manager(
    handler: Arc<dyn CallHandler>,
    port: Option<u16>,
    from_uri: Option<&str>,
) -> Result<Arc<SessionCoordinator>, SessionError> {
    let port = port.unwrap_or_else(|| get_test_ports().0);
    let from_uri = from_uri.unwrap_or("sip:test@localhost");
    
    let manager = SessionManagerBuilder::new()
        .with_local_address(from_uri)
        .with_sip_port(port)
        .with_handler(handler)
        .build()
        .await?;
    
    manager.start().await?;
    Ok(manager)
}

/// Create a pair of session managers for testing established dialogs
pub async fn create_session_manager_pair() -> Result<(Arc<SessionCoordinator>, Arc<SessionCoordinator>, mpsc::UnboundedReceiver<CallEvent>), SessionError> {
//     let (handler_a, _) = EventTrackingHandler::new();
//     let (handler_b, call_events) = EventTrackingHandler::new();
    
    // Get unique ports for this test
    let (port_a, port_b) = get_test_ports();
    
    // Create two session managers on different SIP and media ports
    let manager_a = SessionManagerBuilder::new()
        .with_local_address("sip:alice@127.0.0.1")
        .with_sip_port(port_a)
        .with_media_ports(20000, 21000)  // Manager A uses RTP ports 20000-21000
        .with_handler(Arc::new(media_test_utils::TestCallHandler::new(true)))
        .build()
        .await?;
    
    let manager_b = SessionManagerBuilder::new()
        .with_local_address("sip:bob@127.0.0.1")
        .with_sip_port(port_b)
        .with_media_ports(22000, 23000)  // Manager B uses RTP ports 22000-23000
        .with_handler(Arc::new(media_test_utils::TestCallHandler::new(true)))
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
    
    // Create dummy call events receiver for now
    let (_tx, call_events) = mpsc::unbounded_channel();
    
    Ok((manager_a, manager_b, call_events))
}

/// Create a test session manager pair with custom handlers
pub async fn create_session_manager_pair_with_handlers(
    handler_a: Arc<dyn CallHandler>,
    handler_b: Arc<dyn CallHandler>,
) -> Result<(Arc<SessionCoordinator>, Arc<SessionCoordinator>), SessionError> {
    // Get unique ports for this test
    let (port_a, port_b) = get_test_ports();
    
    // Create two session managers on different ports
    let manager_a = SessionManagerBuilder::new()
        .with_local_address("sip:alice@127.0.0.1")
        .with_sip_port(port_a)
        .with_handler(handler_a)
        .build()
        .await?;
    
    let manager_b = SessionManagerBuilder::new()
        .with_local_address("sip:bob@127.0.0.1")
        .with_sip_port(port_b)
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
    caller: &Arc<SessionCoordinator>,
    callee: &Arc<SessionCoordinator>,
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
    caller: &Arc<SessionCoordinator>,
    callee: &Arc<SessionCoordinator>,
    call_events: &mut mpsc::UnboundedReceiver<CallEvent>,
    sdp: Option<String>,
) -> Result<(CallSession, Option<SessionId>), SessionError> {
    // Get actual bound addresses
    let caller_addr = caller.get_bound_address();
    let callee_addr = callee.get_bound_address();
    
    // Subscribe to session events to wait for state changes
    let mut caller_events = caller.event_processor.subscribe().await
        .map_err(|_| SessionError::Other("Failed to subscribe to events".to_string()))?;
    let mut callee_events = callee.event_processor.subscribe().await
        .map_err(|_| SessionError::Other("Failed to subscribe to events".to_string()))?;
    
    // Create outgoing call from A to B using B's actual address
    let from_uri = format!("sip:alice@{}", caller_addr.ip());
    let to_uri = format!("sip:bob@{}", callee_addr);
    
    println!("Making call: {} -> {}", from_uri, to_uri);
    
    let call = caller.create_outgoing_call(&from_uri, &to_uri, sdp).await?;
    let caller_session_id = call.id().clone();
    
    // Wait for caller's session to reach Active state
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(3);
    let mut caller_active = false;
    
    while start.elapsed() < timeout && !caller_active {
        match tokio::time::timeout(Duration::from_millis(100), caller_events.receive()).await {
            Ok(Ok(event)) => {
                if let SessionEvent::StateChanged { session_id, new_state, .. } = event {
                    if session_id == caller_session_id && new_state == CallState::Active {
                        caller_active = true;
                        println!("Caller session {} reached Active state", session_id);
                    }
                }
            },
            _ => {}
        }
    }
    
    if !caller_active {
        return Err(SessionError::Other("Timeout waiting for caller session to become active".to_string()));
    }
    
    // Find the callee's session ID by waiting for SessionCreated event
    let mut callee_session_id = None;
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout && callee_session_id.is_none() {
        match tokio::time::timeout(Duration::from_millis(100), callee_events.receive()).await {
            Ok(Ok(event)) => {
                if let SessionEvent::SessionCreated { session_id, .. } = event {
                    callee_session_id = Some(session_id.clone());
                    println!("Callee session {} created", session_id);
                }
            },
            _ => {}
        }
    }
    
    // Wait for callee's session to reach Active state if we found it
    if let Some(ref callee_id) = callee_session_id {
        let start = std::time::Instant::now();
        let mut callee_active = false;
        
        while start.elapsed() < timeout && !callee_active {
            match tokio::time::timeout(Duration::from_millis(100), callee_events.receive()).await {
                Ok(Ok(event)) => {
                    if let SessionEvent::StateChanged { session_id, new_state, .. } = event {
                        if session_id == *callee_id && new_state == CallState::Active {
                            callee_active = true;
                            println!("Callee session {} reached Active state", session_id);
                        }
                    }
                },
                _ => {}
            }
        }
    }
    
    Ok((call, callee_session_id))
}

/// Verify that a session exists and has the expected properties
pub async fn verify_session_exists(
    manager: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    expected_state: Option<&CallState>,
) -> Result<CallSession, SessionError> {
    let session = manager.get_session(session_id).await?
        .ok_or_else(|| SessionError::session_not_found(&session_id.to_string()))?;
    
    if let Some(expected) = expected_state {
        assert_eq!(session.state(), expected, "Session state mismatch");
    }
    
    Ok(session)
}

/// Verify that a session no longer exists or is in terminated state
pub async fn verify_session_removed(
    manager: &Arc<SessionCoordinator>,
    session_id: &SessionId,
) -> Result<(), SessionError> {
    let session = manager.get_session(session_id).await?;
    
    if let Some(session) = session {
        // Session might still exist but should be in Terminated state
        assert_eq!(
            session.state(), 
            &CallState::Terminated, 
            "Session {} should be terminated, but is in state {:?}", 
            session_id,
            session.state()
        );
    }
    // If None, that's also fine - session has been completely removed
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
pub async fn cleanup_managers(managers: Vec<Arc<SessionCoordinator>>) -> Result<(), SessionError> {
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