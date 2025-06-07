//! Session Test Utilities
//!
//! Common functions and helpers for session library testing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

use rvoip_session_core::{
    api::types::{SessionId, CallState, MediaInfo},
    session::{SessionImpl, StateManager, lifecycle::LifecycleManager, media::MediaCoordinator},
    SessionError,
};

// Define Result type for tests
type Result<T> = std::result::Result<T, SessionError>;

/// Configuration for session tests
#[derive(Debug, Clone)]
pub struct SessionTestConfig {
    pub default_timeout: Duration,
    pub state_transition_delay: Duration,
    pub media_setup_timeout: Duration,
    pub max_concurrent_sessions: usize,
    pub enable_lifecycle_tracking: bool,
}

impl Default for SessionTestConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(10),
            state_transition_delay: Duration::from_millis(100),
            media_setup_timeout: Duration::from_secs(5),
            max_concurrent_sessions: 100,
            enable_lifecycle_tracking: true,
        }
    }
}

impl SessionTestConfig {
    pub fn fast() -> Self {
        Self {
            default_timeout: Duration::from_secs(2),
            state_transition_delay: Duration::from_millis(10),
            media_setup_timeout: Duration::from_secs(1),
            max_concurrent_sessions: 50,
            enable_lifecycle_tracking: false,
        }
    }

    pub fn stress() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
            state_transition_delay: Duration::from_millis(1),
            media_setup_timeout: Duration::from_secs(10),
            max_concurrent_sessions: 1000,
            enable_lifecycle_tracking: true,
        }
    }
}

/// Helper for testing SessionImpl
pub struct SessionImplTestHelper {
    sessions: Arc<RwLock<HashMap<SessionId, SessionImpl>>>,
    pub config: SessionTestConfig,
}

impl SessionImplTestHelper {
    pub fn new() -> Self {
        Self::new_with_config(SessionTestConfig::default())
    }

    pub fn new_with_config(config: SessionTestConfig) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub async fn create_test_session(&self) -> SessionId {
        let session_id = SessionId::new();
        let session = SessionImpl::new(session_id.clone());
        
        self.sessions.write().await.insert(session_id.clone(), session);
        session_id
    }

    pub async fn create_test_session_with_state(&self, state: CallState) -> SessionId {
        let session_id = self.create_test_session().await;
        self.update_session_state(&session_id, state).await.unwrap();
        session_id
    }

    pub async fn get_session(&self, session_id: &SessionId) -> Option<SessionImpl> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub async fn update_session_state(&self, session_id: &SessionId, new_state: CallState) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.update_state(new_state)?;
            Ok(())
        } else {
            Err(SessionError::session_not_found(&session_id.to_string()))
        }
    }

    pub async fn verify_session_state(&self, session_id: &SessionId, expected_state: CallState) {
        let session = self.get_session(session_id).await
            .expect(&format!("Session {} should exist", session_id));
        assert_eq!(session.state, expected_state, 
                  "Session {} state mismatch", session_id);
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    pub async fn clear_sessions(&self) {
        self.sessions.write().await.clear();
    }
}

/// Helper for testing StateManager
pub struct StateManagerTestHelper {
    state_manager: StateManager,
    transition_history: Arc<Mutex<Vec<(CallState, CallState, bool)>>>,
}

impl StateManagerTestHelper {
    pub fn new() -> Self {
        Self {
            state_manager: StateManager,
            transition_history: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn test_transition(&self, from: CallState, to: CallState) -> bool {
        let result = StateManager::can_transition(&from, &to);
        
        // Record transition attempt
        self.transition_history.lock().await.push((from, to, result));
        
        result
    }

    pub async fn validate_transition(&self, from: CallState, to: CallState) -> Result<()> {
        if StateManager::can_transition(&from, &to) {
            Ok(())
        } else {
            Err(SessionError::invalid_state(&format!("Cannot transition from {:?} to {:?}", from, to)))
        }
    }

    pub async fn get_transition_history(&self) -> Vec<(CallState, CallState, bool)> {
        self.transition_history.lock().await.clone()
    }

    pub async fn clear_history(&self) {
        self.transition_history.lock().await.clear();
    }

    pub fn get_all_states() -> Vec<CallState> {
        vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
            CallState::Terminating,
            CallState::Terminated,
            CallState::Failed("test failure".to_string()),
        ]
    }

    pub fn get_valid_transitions() -> Vec<(CallState, CallState)> {
        vec![
            (CallState::Initiating, CallState::Ringing),
            (CallState::Ringing, CallState::Active),
            (CallState::Active, CallState::OnHold),
            (CallState::OnHold, CallState::Active),
            (CallState::Initiating, CallState::Terminated),
            (CallState::Ringing, CallState::Terminated),
            (CallState::Active, CallState::Terminated),
            (CallState::OnHold, CallState::Terminated),
            (CallState::Initiating, CallState::Failed("test".to_string())),
            (CallState::Ringing, CallState::Failed("test".to_string())),
            (CallState::Active, CallState::Failed("test".to_string())),
        ]
    }
}

/// Helper for testing LifecycleManager
pub struct LifecycleManagerTestHelper {
    lifecycle_manager: LifecycleManager,
    events: Arc<Mutex<Vec<LifecycleEvent>>>,
    config: SessionTestConfig,
}

#[derive(Debug, Clone)]
pub struct LifecycleEvent {
    pub event_type: String,
    pub session_id: String,
    pub timestamp: Instant,
    pub metadata: Option<String>,
}

impl LifecycleEvent {
    pub fn new(event_type: &str, session_id: String) -> Self {
        Self {
            event_type: event_type.to_string(),
            session_id,
            timestamp: Instant::now(),
            metadata: None,
        }
    }

    pub fn with_metadata(event_type: &str, session_id: String, metadata: String) -> Self {
        Self {
            event_type: event_type.to_string(),
            session_id,
            timestamp: Instant::now(),
            metadata: Some(metadata),
        }
    }
}

impl LifecycleManagerTestHelper {
    pub fn new() -> Self {
        Self::new_with_config(SessionTestConfig::default())
    }

    pub fn new_with_config(config: SessionTestConfig) -> Self {
        Self {
            lifecycle_manager: LifecycleManager::new(),
            events: Arc::new(Mutex::new(Vec::new())),
            config,
        }
    }

    pub async fn trigger_session_created(&self, session_id: &str) -> Result<()> {
        let event = LifecycleEvent::new("created", session_id.to_string());
        self.events.lock().await.push(event);
        Ok(())
    }

    pub async fn trigger_session_terminated(&self, session_id: &str) -> Result<()> {
        let event = LifecycleEvent::new("terminated", session_id.to_string());
        self.events.lock().await.push(event);
        Ok(())
    }

    pub async fn trigger_session_state_change(&self, session_id: &str, from_state: CallState, to_state: CallState) -> Result<()> {
        let metadata = format!("from={:?},to={:?}", from_state, to_state);
        let event = LifecycleEvent::with_metadata("state_change", session_id.to_string(), metadata);
        self.events.lock().await.push(event);
        Ok(())
    }

    pub async fn get_events(&self) -> Vec<LifecycleEvent> {
        self.events.lock().await.clone()
    }

    pub async fn get_event_count(&self) -> usize {
        self.events.lock().await.len()
    }

    pub async fn clear_events(&self) {
        self.events.lock().await.clear();
    }

    pub async fn get_events_by_type(&self, event_type: &str) -> Vec<LifecycleEvent> {
        self.events.lock().await.iter()
            .filter(|event| event.event_type == event_type)
            .cloned()
            .collect()
    }

    pub async fn get_events_by_session(&self, session_id: &str) -> Vec<LifecycleEvent> {
        self.events.lock().await.iter()
            .filter(|event| event.session_id == session_id)
            .cloned()
            .collect()
    }

    pub async fn verify_session_created_event(&self, session_id: &str) {
        let events = self.get_events().await;
        let found = events.iter().any(|event| {
            event.event_type == "created" && event.session_id == session_id
        });
        assert!(found, "Session created event not found for {}", session_id);
    }

    pub async fn verify_session_terminated_event(&self, session_id: &str) {
        let events = self.get_events().await;
        let found = events.iter().any(|event| {
            event.event_type == "terminated" && event.session_id == session_id
        });
        assert!(found, "Session terminated event not found for {}", session_id);
    }
}

/// Helper for testing MediaCoordinator
pub struct MediaCoordinatorTestHelper {
    media_coordinator: MediaCoordinator,
    media_sessions: Arc<RwLock<HashMap<SessionId, MediaInfo>>>,
    media_operations: Arc<Mutex<Vec<MediaOperation>>>,
    config: SessionTestConfig,
}

#[derive(Debug, Clone)]
pub struct MediaOperation {
    pub operation_type: String,
    pub session_id: String,
    pub timestamp: Instant,
    pub success: bool,
    pub metadata: Option<String>,
}

impl MediaOperation {
    pub fn new(operation_type: &str, session_id: &str, success: bool) -> Self {
        Self {
            operation_type: operation_type.to_string(),
            session_id: session_id.to_string(),
            timestamp: Instant::now(),
            success,
            metadata: None,
        }
    }
}

impl MediaCoordinatorTestHelper {
    pub fn new() -> Self {
        Self::new_with_config(SessionTestConfig::default())
    }

    pub fn new_with_config(config: SessionTestConfig) -> Self {
        Self {
            media_coordinator: MediaCoordinator::new(),
            media_sessions: Arc::new(RwLock::new(HashMap::new())),
            media_operations: Arc::new(Mutex::new(Vec::new())),
            config,
        }
    }

    pub async fn setup_media(&self, session_id: &str, sdp: &str) -> Result<()> {
        let operation = MediaOperation::new("setup", session_id, true);
        self.media_operations.lock().await.push(operation);
        Ok(())
    }

    pub async fn update_media(&self, session_id: &str, sdp: &str) -> Result<()> {
        let operation = MediaOperation::new("update", session_id, true);
        self.media_operations.lock().await.push(operation);
        Ok(())
    }

    pub async fn cleanup_media(&self, session_id: &str) -> Result<()> {
        let operation = MediaOperation::new("cleanup", session_id, true);
        self.media_operations.lock().await.push(operation);
        Ok(())
    }

    pub async fn get_operations(&self) -> Vec<MediaOperation> {
        self.media_operations.lock().await.clone()
    }

    pub async fn get_operations_by_type(&self, operation_type: &str) -> Vec<MediaOperation> {
        self.media_operations.lock().await.iter()
            .filter(|op| op.operation_type == operation_type)
            .cloned()
            .collect()
    }

    pub async fn get_operations_by_session(&self, session_id: &str) -> Vec<MediaOperation> {
        self.media_operations.lock().await.iter()
            .filter(|op| op.session_id == session_id)
            .cloned()
            .collect()
    }

    pub async fn clear_operations(&self) {
        self.media_operations.lock().await.clear();
    }

    pub async fn setup_test_media(&self, session_id: &SessionId, sdp: &str) -> Result<MediaInfo> {
        // Simplified for testing - create a basic MediaInfo
        let media_info = MediaInfo {
            local_sdp: Some(sdp.to_string()),
            remote_sdp: None,
            codec: Some("PCMU".to_string()),
            local_rtp_port: Some(49170),
            remote_rtp_port: None,
        };
        
        self.media_sessions.write().await.insert(session_id.clone(), media_info.clone());
        Ok(media_info)
    }

    pub async fn update_test_media(&self, session_id: &SessionId, new_sdp: &str) -> Result<()> {
        // Simplified for testing
        if let Some(media_info) = self.media_sessions.write().await.get_mut(session_id) {
            media_info.local_sdp = Some(new_sdp.to_string());
        }
        Ok(())
    }

    pub async fn cleanup_test_media(&self, session_id: &SessionId) -> Result<()> {
        self.media_sessions.write().await.remove(session_id);
        Ok(())
    }

    pub async fn get_media_info(&self, session_id: &SessionId) -> Option<MediaInfo> {
        self.media_sessions.read().await.get(session_id).cloned()
    }

    pub async fn verify_media_setup(&self, session_id: &SessionId, expected_codec: Option<&str>) {
        let media_info = self.get_media_info(session_id).await
            .expect(&format!("Media info should exist for session {}", session_id));
        
        assert!(media_info.local_sdp.is_some(), "Local SDP should be set");
        
        if let Some(expected_codec) = expected_codec {
            assert_eq!(media_info.codec.as_deref(), Some(expected_codec), 
                      "Codec mismatch for session {}", session_id);
        }
    }

    pub async fn active_media_count(&self) -> usize {
        self.media_sessions.read().await.len()
    }

    pub async fn clear_media_sessions(&self) {
        self.media_sessions.write().await.clear();
    }
}

/// Integration helper that combines all session components
pub struct SessionIntegrationHelper {
    pub session_helper: SessionImplTestHelper,
    pub state_helper: StateManagerTestHelper,
    pub lifecycle_helper: LifecycleManagerTestHelper,
    pub media_helper: MediaCoordinatorTestHelper,
    pub config: SessionTestConfig,
    session_states: Arc<Mutex<HashMap<String, CallState>>>,
}

impl SessionIntegrationHelper {
    pub fn new() -> Self {
        Self::new_with_config(SessionTestConfig::default())
    }

    pub fn new_with_config(config: SessionTestConfig) -> Self {
        Self {
            session_helper: SessionImplTestHelper::new_with_config(config.clone()),
            state_helper: StateManagerTestHelper::new(),
            lifecycle_helper: LifecycleManagerTestHelper::new_with_config(config.clone()),
            media_helper: MediaCoordinatorTestHelper::new_with_config(config.clone()),
            config,
            session_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a complete session with lifecycle tracking and media setup
    pub async fn create_complete_session(&self, initial_state: CallState, sdp: &str) -> Result<SessionId> {
        // Create session
        let session_id = self.session_helper.create_test_session().await;
        
        // Trigger lifecycle event
        self.lifecycle_helper.trigger_session_created(&session_id.to_string()).await?;
        
        // Setup media (simplified for testing)
        self.media_helper.setup_media(&session_id.to_string(), sdp).await?;
        
        // Set initial state
        if initial_state != CallState::Initiating {
            self.session_helper.update_session_state(&session_id, initial_state).await?;
        }
        
        Ok(session_id)
    }

    /// Perform a state transition with validation
    pub async fn transition_session_state(&self, session_id: &SessionId, new_state: CallState) -> Result<()> {
        let current_session = self.session_helper.get_session(session_id).await
            .ok_or_else(|| SessionError::session_not_found(&session_id.to_string()))?;
        
        // Validate transition
        self.state_helper.validate_transition(current_session.state.clone(), new_state.clone()).await?;
        
        // Perform transition
        self.session_helper.update_session_state(session_id, new_state).await?;
        
        Ok(())
    }

    /// Terminate a session with full cleanup
    pub async fn terminate_session(&self, session_id: &SessionId, reason: &str) -> Result<()> {
        // Transition to terminated state
        self.transition_session_state(session_id, CallState::Terminated).await?;
        
        // Trigger lifecycle event
        self.lifecycle_helper.trigger_session_terminated(&session_id.to_string()).await?;
        
        // Cleanup media
        self.media_helper.cleanup_media(&session_id.to_string()).await?;
        
        Ok(())
    }

    pub async fn verify_complete_session(&self, session_id: &SessionId, expected_state: CallState) {
        // Verify session exists and has correct state
        self.session_helper.verify_session_state(session_id, expected_state).await;
        
        // Verify lifecycle event was recorded
        self.lifecycle_helper.verify_session_created_event(&session_id.to_string()).await;
        
        // Verify media setup (simplified for testing)
        // Additional verification can be added as needed
    }

    pub async fn get_comprehensive_stats(&self) -> SessionTestStats {
        SessionTestStats {
            total_sessions: self.session_helper.session_count().await,
            active_media_sessions: self.media_helper.active_media_count().await,
            lifecycle_events: self.lifecycle_helper.get_event_count().await,
            state_transitions: self.state_helper.get_transition_history().await.len(),
        }
    }

    pub async fn create_session(&self, session_id: &str, initial_state: CallState) -> Result<()> {
        // Track the session state
        self.session_states.lock().await.insert(session_id.to_string(), initial_state.clone());
        
        self.lifecycle_helper.trigger_session_created(session_id).await?;
        Ok(())
    }

    pub async fn update_session_state(&self, session_id: &str, new_state: CallState) -> Result<()> {
        // Get current state for lifecycle event
        let current_state = self.session_states.lock().await.get(session_id)
            .cloned()
            .unwrap_or(CallState::Initiating);
        
        // Update the tracked state
        self.session_states.lock().await.insert(session_id.to_string(), new_state.clone());
        
        self.lifecycle_helper.trigger_session_state_change(session_id, current_state, new_state).await?;
        Ok(())
    }

    pub async fn setup_media(&self, session_id: &str, sdp: &str) -> Result<()> {
        self.media_helper.setup_media(session_id, sdp).await
    }

    pub async fn update_media(&self, session_id: &str, sdp: &str) -> Result<()> {
        self.media_helper.update_media(session_id, sdp).await
    }

    pub async fn cleanup_media(&self, session_id: &str) -> Result<()> {
        self.media_helper.cleanup_media(session_id).await
    }

    pub async fn get_session_info(&self, session_id: &str) -> Option<SessionInfo> {
        let current_state = self.session_states.lock().await.get(session_id).cloned()?;
        Some(SessionInfo {
            session_id: session_id.to_string(),
            current_state,
        })
    }

    pub async fn get_lifecycle_events(&self) -> Vec<LifecycleEvent> {
        self.lifecycle_helper.get_events().await
    }

    pub async fn get_media_operations(&self) -> Vec<MediaOperation> {
        self.media_helper.get_operations().await
    }

    pub async fn get_lifecycle_events_by_type(&self, event_type: &str) -> Vec<LifecycleEvent> {
        self.lifecycle_helper.get_events_by_type(event_type).await
    }

    pub async fn get_lifecycle_events_by_session(&self, session_id: &str) -> Vec<LifecycleEvent> {
        self.lifecycle_helper.get_events_by_session(session_id).await
    }

    pub async fn get_media_operations_by_type(&self, operation_type: &str) -> Vec<MediaOperation> {
        self.media_helper.get_operations_by_type(operation_type).await
    }

    pub async fn get_media_operations_by_session(&self, session_id: &str) -> Vec<MediaOperation> {
        self.media_helper.get_operations_by_session(session_id).await
    }

    pub async fn clear_lifecycle_events(&self) {
        self.lifecycle_helper.clear_events().await;
    }

    pub async fn clear_media_operations(&self) {
        self.media_helper.clear_operations().await;
    }

    pub async fn cleanup_all(&self) {
        self.session_helper.clear_sessions().await;
        self.lifecycle_helper.clear_events().await;
        self.state_helper.clear_history().await;
        self.media_helper.clear_media_sessions().await;
        self.media_helper.clear_operations().await;
        self.session_states.lock().await.clear();
    }
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub current_state: CallState,
}

#[derive(Debug)]
pub struct SessionTestStats {
    pub total_sessions: usize,
    pub active_media_sessions: usize,
    pub lifecycle_events: usize,
    pub state_transitions: usize,
}

/// Performance testing helper for sessions
pub struct SessionPerformanceHelper {
    integration_helper: SessionIntegrationHelper,
    metrics: Arc<Mutex<SessionPerformanceMetrics>>,
}

#[derive(Debug, Default, Clone)]
pub struct SessionPerformanceMetrics {
    pub session_creation_times: Vec<Duration>,
    pub state_transition_times: Vec<Duration>,
    pub media_setup_times: Vec<Duration>,
    pub total_operations: usize,
}

impl SessionPerformanceHelper {
    pub fn new() -> Self {
        Self {
            integration_helper: SessionIntegrationHelper::new_with_config(SessionTestConfig::stress()),
            metrics: Arc::new(Mutex::new(SessionPerformanceMetrics::default())),
        }
    }

    pub async fn benchmark_session_creation(&self, session_count: usize) -> Duration {
        let start = Instant::now();
        
        for i in 0..session_count {
            let creation_start = Instant::now();
            let sdp = format!("v=0\r\no=test {} 0 IN IP4 127.0.0.1\r\n", i);
            let _session_id = self.integration_helper.create_complete_session(CallState::Initiating, &sdp).await
                .expect("Failed to create session");
            let creation_time = creation_start.elapsed();
            
            self.metrics.lock().await.session_creation_times.push(creation_time);
        }
        
        let total_time = start.elapsed();
        self.metrics.lock().await.total_operations += session_count;
        total_time
    }

    pub async fn benchmark_state_transitions(&self, transition_count: usize) -> Duration {
        let session_id = self.integration_helper.create_complete_session(CallState::Initiating, "test SDP").await
            .expect("Failed to create test session");
        
        let start = Instant::now();
        
        for _ in 0..transition_count {
            let transition_start = Instant::now();
            
            // Cycle through states
            self.integration_helper.transition_session_state(&session_id, CallState::Ringing).await.unwrap();
            self.integration_helper.transition_session_state(&session_id, CallState::Active).await.unwrap();
            self.integration_helper.transition_session_state(&session_id, CallState::OnHold).await.unwrap();
            self.integration_helper.transition_session_state(&session_id, CallState::Active).await.unwrap();
            
            let transition_time = transition_start.elapsed();
            self.metrics.lock().await.state_transition_times.push(transition_time);
        }
        
        start.elapsed()
    }

    pub async fn get_metrics(&self) -> SessionPerformanceMetrics {
        self.metrics.lock().await.clone()
    }

    pub async fn cleanup(&self) {
        self.integration_helper.cleanup_all().await;
    }
}

/// Utility struct for session testing
pub struct SessionTestUtils;

impl SessionTestUtils {
    /// Create standard test SDP
    pub fn create_test_sdp() -> String {
        "v=0\r\no=test 123 456 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n".to_string()
    }

    /// Create test SDP with specific codec
    pub fn create_test_sdp_with_codec(codec: &str, port: u16) -> String {
        format!(
            "v=0\r\no=test 123 456 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio {} RTP/AVP 0\r\na=rtpmap:0 {}/8000\r\n",
            port, codec
        )
    }
}

/// Utility functions for session testing
pub mod session_test_utils {
    use super::*;

    /// Create test SDP with specific codec
    pub fn create_test_sdp(codec: &str, port: u16) -> String {
        SessionTestUtils::create_test_sdp_with_codec(codec, port)
    }

    /// Generate multiple test session IDs
    pub fn create_test_session_ids(count: usize) -> Vec<SessionId> {
        (0..count).map(|i| SessionId(format!("test-session-{}", i))).collect()
    }

    /// Wait for a condition with timeout
    pub async fn wait_for_condition<F, Fut>(condition: F, timeout: Duration) -> bool
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let start = Instant::now();
        
        while start.elapsed() < timeout {
            if condition().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        
        false
    }

    /// Verify state transition validity for all state combinations
    pub async fn verify_all_state_transitions() -> Vec<(CallState, CallState, bool)> {
        let states = StateManagerTestHelper::get_all_states();
        let mut results = Vec::new();
        
        for from_state in &states {
            for to_state in &states {
                let is_valid = StateManager::can_transition(from_state, to_state);
                results.push((from_state.clone(), to_state.clone(), is_valid));
            }
        }
        
        results
    }
} 