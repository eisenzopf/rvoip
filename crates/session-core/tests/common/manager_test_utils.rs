use rvoip_session_core::api::control::SessionControl;
// Manager Test Utilities
//
// Common helper functions and test utilities for testing manager functionality
// across different scenarios including core operations, registry, events, and cleanup.

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use tokio::sync::{Mutex, RwLock};
use rvoip_session_core::{
    SessionCoordinator,
    api::{
        types::{SessionId, CallSession, CallState, SessionStats},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
    coordinator::registry::InternalSessionRegistry,
    manager::{
        events::{SessionEvent, SessionEventProcessor, SessionEventSubscriber},
        cleanup::CleanupManager,
    },
    SessionError,
};
use crate::common::*;

/// Test configuration for manager testing
#[derive(Debug, Clone)]
pub struct ManagerTestConfig {
    pub session_timeout: Duration,
    pub cleanup_interval: Duration,
    pub event_timeout: Duration,
    pub max_sessions: usize,
    pub bind_address: String,
    pub from_uri_base: String,
}

impl Default for ManagerTestConfig {
    fn default() -> Self {
        Self {
            session_timeout: Duration::from_secs(30),
            cleanup_interval: Duration::from_secs(5),
            event_timeout: Duration::from_secs(2),
            max_sessions: 100,
            bind_address: "127.0.0.1".to_string(),
            from_uri_base: "test@localhost".to_string(),
        }
    }
}

impl ManagerTestConfig {
    pub fn fast() -> Self {
        Self {
            session_timeout: Duration::from_secs(5),
            cleanup_interval: Duration::from_secs(1),
            event_timeout: Duration::from_millis(500),
            max_sessions: 10,
            bind_address: "127.0.0.1".to_string(),
            from_uri_base: "test@localhost".to_string(),
        }
    }

    pub fn stress() -> Self {
        Self {
            session_timeout: Duration::from_secs(60),
            cleanup_interval: Duration::from_secs(10),
            event_timeout: Duration::from_secs(5),
            max_sessions: 1000,
            bind_address: "127.0.0.1".to_string(),
            from_uri_base: "test@localhost".to_string(),
        }
    }
}

/// Create a test session manager with default configuration
pub async fn create_test_session_manager() -> Result<Arc<SessionCoordinator>, SessionError> {
    let handler = TestCallHandler::new(true);
    create_session_manager(Arc::new(handler), None, None).await
}

/// Create a test session manager with custom handler
pub async fn create_test_session_manager_with_handler(
    handler: Arc<dyn CallHandler>
) -> Result<Arc<SessionCoordinator>, SessionError> {
    create_session_manager(handler, None, None).await
}

/// Create a test session manager with specific configuration
pub async fn create_test_session_manager_with_config(
    config: ManagerTestConfig,
    handler: Arc<dyn CallHandler>,
) -> Result<Arc<SessionCoordinator>, SessionError> {
    let port = get_test_ports().0;
    // Don't add sip: prefix if it's already there
    let from_uri = if config.from_uri_base.starts_with("sip:") {
        config.from_uri_base.clone()
    } else {
        format!("sip:{}", config.from_uri_base)
    };
    
    let manager = SessionManagerBuilder::new()
        .with_local_address(&from_uri)
        .with_sip_port(port)
        .with_handler(handler)
        .build()
        .await?;
    
    manager.start().await?;
    Ok(manager)
}

/// Create test session IDs for manager tests
pub fn create_manager_test_session_ids(count: usize) -> Vec<SessionId> {
    (0..count).map(|i| SessionId(format!("manager-test-session-{}", i))).collect()
}

/// Create a test call session
pub fn create_test_call_session(session_id: SessionId, from: &str, to: &str, state: CallState) -> CallSession {
    CallSession {
        id: session_id,
        from: from.to_string(),
        to: to.to_string(),
        state,
        started_at: Some(std::time::Instant::now()),
        sip_call_id: None,
    }
}

/// Registry test helper for direct registry testing
pub struct RegistryTestHelper {
    registry: Arc<InternalSessionRegistry>,
    test_sessions: Vec<CallSession>,
}

impl RegistryTestHelper {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(InternalSessionRegistry::new()),
            test_sessions: Vec::new(),
        }
    }

    pub async fn add_test_session(&mut self, from: &str, to: &str, state: CallState) -> SessionId {
        let session_id = SessionId::new();
        let call_session = create_test_call_session(session_id.clone(), from, to, state);
        let session = rvoip_session_core::session::Session::from_call_session(call_session.clone());
        
        self.registry.register_session(session).await
            .expect("Failed to register test session");
        self.test_sessions.push(call_session);
        
        session_id
    }

    pub async fn verify_session_count(&self, expected: usize) {
        let sessions = self.registry.list_active_sessions().await
            .expect("Failed to list active sessions");
        let count = sessions.len();
        assert_eq!(count, expected, "Registry session count mismatch");
    }

    pub async fn verify_session_exists(&self, session_id: &SessionId) -> CallSession {
        self.registry.get_public_session(session_id).await
            .expect("Registry operation failed")
            .expect(&format!("Session {} should exist", session_id))
    }

    pub async fn verify_session_not_exists(&self, session_id: &SessionId) {
        let session = self.registry.get_public_session(session_id).await
            .expect("Registry operation failed");
        assert!(session.is_none(), "Session {} should not exist", session_id);
    }

    pub async fn get_stats(&self) -> SessionStats {
        self.registry.get_stats().await.expect("Failed to get registry stats")
    }

    pub fn registry(&self) -> &Arc<InternalSessionRegistry> {
        &self.registry
    }

    pub fn test_sessions(&self) -> &[CallSession] {
        &self.test_sessions
    }
}

/// Event system test helper
pub struct EventTestHelper {
    processor: Arc<SessionEventProcessor>,
    subscriber: Option<SessionEventSubscriber>,
    received_events: Arc<Mutex<Vec<SessionEvent>>>,
}

impl EventTestHelper {
    pub async fn new() -> Result<Self, SessionError> {
        let processor = Arc::new(SessionEventProcessor::new());
        processor.start().await?;
        
        Ok(Self {
            processor,
            subscriber: None,
            received_events: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub async fn subscribe(&mut self) -> Result<(), SessionError> {
        let subscriber = self.processor.subscribe().await?;
        self.subscriber = Some(subscriber);
        Ok(())
    }

    pub async fn publish_event(&self, event: SessionEvent) -> Result<(), SessionError> {
        self.processor.publish_event(event).await
    }

    pub async fn wait_for_event(&mut self, timeout: Duration) -> Option<SessionEvent> {
        if let Some(ref mut subscriber) = self.subscriber {
            match tokio::time::timeout(timeout, subscriber.receive()).await {
                Ok(Ok(event)) => Some(event),
                _ => None,
            }
        } else {
            None
        }
    }

    pub async fn wait_for_specific_event<F>(&mut self, timeout: Duration, predicate: F) -> Option<SessionEvent>
    where
        F: Fn(&SessionEvent) -> bool,
    {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            if let Some(event) = self.wait_for_event(Duration::from_millis(100)).await {
                if predicate(&event) {
                    return Some(event);
                }
            }
        }
        
        None
    }

    pub async fn verify_event_count(&self, expected: usize) {
        let events = self.received_events.lock().await;
        assert_eq!(events.len(), expected, "Event count mismatch");
    }

    pub async fn clear_events(&self) {
        self.received_events.lock().await.clear();
    }

    pub fn processor(&self) -> &Arc<SessionEventProcessor> {
        &self.processor
    }

    pub async fn cleanup(&self) -> Result<(), SessionError> {
        self.processor.stop().await
    }
}

/// Cleanup manager test helper
pub struct CleanupTestHelper {
    cleanup_manager: Arc<CleanupManager>,
    test_resources: Arc<RwLock<HashMap<String, bool>>>, // resource_id -> is_active
}

impl CleanupTestHelper {
    pub fn new() -> Self {
        Self {
            cleanup_manager: Arc::new(CleanupManager::new()),
            test_resources: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn start(&self) -> Result<(), SessionError> {
        self.cleanup_manager.start().await
    }

    pub async fn stop(&self) -> Result<(), SessionError> {
        self.cleanup_manager.stop().await
    }

    pub async fn add_test_resource(&self, resource_id: &str) {
        self.test_resources.write().await.insert(resource_id.to_string(), true);
    }

    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<(), SessionError> {
        self.cleanup_manager.cleanup_session(session_id).await
    }

    pub async fn force_cleanup_all(&self) -> Result<(), SessionError> {
        self.cleanup_manager.force_cleanup_all().await
    }

    pub async fn verify_resource_cleaned(&self, resource_id: &str) {
        let resources = self.test_resources.read().await;
        assert!(!resources.get(resource_id).unwrap_or(&false), 
               "Resource {} should be cleaned up", resource_id);
    }

    pub fn cleanup_manager(&self) -> &Arc<CleanupManager> {
        &self.cleanup_manager
    }
}

/// Integration test helper that combines all manager components
pub struct ManagerIntegrationHelper {
    pub manager: Arc<SessionCoordinator>,
    pub manager_b: Option<Arc<SessionCoordinator>>, // Second manager for dialog establishment
    pub call_events: Option<tokio::sync::mpsc::UnboundedReceiver<CallEvent>>, // For dialog establishment
    pub registry_helper: RegistryTestHelper,
    pub event_helper: EventTestHelper,
    pub cleanup_helper: CleanupTestHelper,
    pub config: ManagerTestConfig,
}

impl ManagerIntegrationHelper {
    pub async fn new() -> Result<Self, SessionError> {
        Self::new_with_config(ManagerTestConfig::default()).await
    }

    pub async fn new_with_config(config: ManagerTestConfig) -> Result<Self, SessionError> {
        // Create a pair of managers for real dialog establishment
        let (manager_a, manager_b, call_events) = create_session_manager_pair().await?;
        
        let registry_helper = RegistryTestHelper::new();
        let event_helper = EventTestHelper::new().await?;
        let cleanup_helper = CleanupTestHelper::new();
        
        Ok(Self {
            manager: manager_a,
            manager_b: Some(manager_b),
            call_events: Some(call_events),
            registry_helper,
            event_helper,
            cleanup_helper,
            config,
        })
    }

    /// Create a test call with proper SIP dialog establishment
    pub async fn create_test_call(&mut self, from: &str, to: &str) -> Result<CallSession, SessionError> {
        if let (Some(ref manager_b), Some(ref mut call_events)) = (&self.manager_b, &mut self.call_events) {
            // Establish real SIP dialog between the two managers
            let (call, _callee_session_id) = establish_call_between_managers(&self.manager, manager_b, call_events).await?;
            Ok(call)
        } else {
            // Fallback to simple call creation (for tests that don't need established dialogs)
            self.manager.create_outgoing_call(from, to, Some("test SDP".to_string())).await
        }
    }

    /// Create a simple call without dialog establishment (for tests that just need session creation)
    pub async fn create_simple_call(&self, from: &str, to: &str) -> Result<CallSession, SessionError> {
        self.manager.create_outgoing_call(from, to, Some("test SDP".to_string())).await
    }

    pub async fn verify_manager_stats(&self, expected_active: usize) -> SessionStats {
        let stats = self.manager.get_stats().await.expect("Failed to get manager stats");
        assert_eq!(stats.active_sessions, expected_active, "Manager active sessions mismatch");
        stats
    }

    pub async fn verify_session_in_manager(&self, session_id: &SessionId) -> CallSession {
        self.manager.find_session(session_id).await
            .expect("Manager operation failed")
            .expect(&format!("Session {} should exist in manager", session_id))
    }

    pub async fn cleanup(&self) -> Result<(), SessionError> {
        self.manager.stop().await?;
        if let Some(ref manager_b) = self.manager_b {
            manager_b.stop().await?;
        }
        self.event_helper.cleanup().await?;
        self.cleanup_helper.stop().await?;
        Ok(())
    }
}

/// Performance test helper for manager operations
pub struct ManagerPerformanceHelper {
    managers: Vec<Arc<SessionCoordinator>>,
    sessions: Vec<SessionId>,
    metrics: Arc<Mutex<PerformanceMetrics>>,
}

#[derive(Debug, Default)]
pub struct PerformanceMetrics {
    pub session_creation_times: Vec<Duration>,
    pub session_lookup_times: Vec<Duration>,
    pub event_publish_times: Vec<Duration>,
    pub cleanup_times: Vec<Duration>,
}

impl ManagerPerformanceHelper {
    pub async fn new(manager_count: usize) -> Result<Self, SessionError> {
        let mut managers = Vec::new();
        
        for i in 0..manager_count {
            let handler = TestCallHandler::new(true);
            let config = ManagerTestConfig {
                from_uri_base: format!("sip:perf-test-{}@localhost", i),
                ..ManagerTestConfig::default()
            };
            let manager = create_test_session_manager_with_config(config, Arc::new(handler)).await?;
            managers.push(manager);
        }
        
        Ok(Self {
            managers,
            sessions: Vec::new(),
            metrics: Arc::new(Mutex::new(PerformanceMetrics::default())),
        })
    }

    pub async fn benchmark_session_creation(&mut self, session_count: usize) -> Duration {
        let start = std::time::Instant::now();
        
        for i in 0..session_count {
            let manager_idx = i % self.managers.len();
            let manager = &self.managers[manager_idx];
            
            let from = format!("sip:perf-caller-{}@localhost", i);
            let to = format!("sip:perf-callee-{}@localhost", i);
            
            let session_start = std::time::Instant::now();
            let call = manager.create_outgoing_call(&from, &to, Some("perf test SDP".to_string())).await
                .expect("Failed to create session");
            let session_time = session_start.elapsed();
            
            self.sessions.push(call.id().clone());
            self.metrics.lock().await.session_creation_times.push(session_time);
        }
        
        start.elapsed()
    }

    pub async fn benchmark_session_lookup(&self, lookup_count: usize) -> Duration {
        let start = std::time::Instant::now();
        
        for i in 0..lookup_count {
            let manager_idx = i % self.managers.len();
            let session_idx = i % self.sessions.len();
            
            if session_idx < self.sessions.len() {
                let manager = &self.managers[manager_idx];
                let session_id = &self.sessions[session_idx];
                
                let lookup_start = std::time::Instant::now();
                let _ = manager.find_session(session_id).await;
                let lookup_time = lookup_start.elapsed();
                
                self.metrics.lock().await.session_lookup_times.push(lookup_time);
            }
        }
        
        start.elapsed()
    }

    pub async fn benchmark_event_publishing(&self, event_count: usize) -> Duration {
        // let event_processor = &self.managers[0].get_event_processor();
        let start = std::time::Instant::now();
        
        // for i in 0..event_count {
        //     let session_id = SessionId(format!("perf-event-session-{}", i));
        //     let event = SessionEvent::SessionCreated {
        //         session_id,
        //         from: format!("from-{}", i),
        //         to: format!("to-{}", i),
        //         call_state: CallState::Initiating,
        //     };
        //     
        //     let event_start = std::time::Instant::now();
        //     event_processor.publish_event(event).await.expect("Failed to publish event");
        //     let event_time = event_start.elapsed();
        //     
        //     self.metrics.lock().await.event_publish_times.push(event_time);
        // }
        
        start.elapsed()
    }

    pub async fn get_metrics(&self) -> PerformanceMetrics {
        self.metrics.lock().await.clone()
    }

    pub async fn cleanup(&self) -> Result<(), SessionError> {
        for manager in &self.managers {
            manager.stop().await?;
        }
        Ok(())
    }
}

impl Clone for PerformanceMetrics {
    fn clone(&self) -> Self {
        Self {
            session_creation_times: self.session_creation_times.clone(),
            session_lookup_times: self.session_lookup_times.clone(),
            event_publish_times: self.event_publish_times.clone(),
            cleanup_times: self.cleanup_times.clone(),
        }
    }
}

/// Simple test helpers for manager testing
pub mod test_handlers {
    use super::*;
    
    /// Create an accepting handler for tests
    pub fn create_accepting_handler() -> Arc<dyn CallHandler> {
        Arc::new(TestCallHandler::new(true))
    }
    
    /// Create a rejecting handler for tests  
    pub fn create_rejecting_handler() -> Arc<dyn CallHandler> {
        Arc::new(TestCallHandler::new(false))
    }
    
    /// Create a deferring handler for tests
    pub fn create_deferring_handler() -> Arc<dyn CallHandler> {
        Arc::new(TestCallHandler::new(true))
    }
}

/// Utility functions for common test operations
pub mod utils {
    use super::*;

    /// Wait for a condition to be true with timeout
    pub async fn wait_for_condition<F, Fut>(condition: F, timeout: Duration) -> bool
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            if condition().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        
        false
    }

    /// Create multiple test sessions for a manager
    pub async fn create_multiple_sessions(
        manager: &Arc<SessionCoordinator>,
        count: usize,
        prefix: &str,
    ) -> Result<Vec<SessionId>, SessionError> {
        let mut session_ids = Vec::new();
        
        for i in 0..count {
            let from = format!("{}from-{}@localhost", prefix, i);
            let to = format!("{}to-{}@localhost", prefix, i);
            let call = manager.create_outgoing_call(&from, &to, Some("test SDP".to_string())).await?;
            session_ids.push(call.id().clone());
        }
        
        Ok(session_ids)
    }

    /// Verify all sessions exist in manager
    pub async fn verify_all_sessions_exist(
        manager: &Arc<SessionCoordinator>,
        session_ids: &[SessionId],
    ) -> Result<(), SessionError> {
        for session_id in session_ids {
            let session = manager.find_session(session_id).await?;
            assert!(session.is_some(), "Session {} should exist", session_id);
        }
        Ok(())
    }
} 