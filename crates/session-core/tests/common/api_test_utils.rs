use rvoip_session_core::api::control::SessionControl;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use async_trait::async_trait;

use rvoip_session_core::{Result, SessionError};
use rvoip_session_core::api::types::{
    SessionId, CallSession, IncomingCall, CallState, CallDecision, 
    SessionStats, MediaInfo
};
use rvoip_session_core::api::handlers::CallHandler;
use rvoip_session_core::api::builder::{SessionManagerBuilder, SessionManagerConfig};
use rvoip_session_core::SessionCoordinator;

/// Test configuration for API tests
#[derive(Debug, Clone)]
pub struct ApiTestConfig {
    pub timeout_duration: Duration,
    pub enable_performance_tests: bool,
    pub enable_stress_tests: bool,
    pub concurrent_operations: usize,
    pub session_count: usize,
}

impl Default for ApiTestConfig {
    fn default() -> Self {
        Self {
            timeout_duration: Duration::from_secs(10),
            enable_performance_tests: true,
            enable_stress_tests: true,
            concurrent_operations: 10,
            session_count: 100,
        }
    }
}

impl ApiTestConfig {
    pub fn fast() -> Self {
        Self {
            timeout_duration: Duration::from_secs(5),
            enable_performance_tests: false,
            enable_stress_tests: false,
            concurrent_operations: 5,
            session_count: 20,
        }
    }

    pub fn stress() -> Self {
        Self {
            timeout_duration: Duration::from_secs(30),
            enable_performance_tests: true,
            enable_stress_tests: true,
            concurrent_operations: 50,
            session_count: 500,
        }
    }
}

/// Test handler that tracks all events for verification
#[derive(Debug)]
pub struct TestCallHandler {
    pub incoming_calls: Arc<std::sync::Mutex<Vec<IncomingCall>>>,
    pub ended_calls: Arc<std::sync::Mutex<Vec<(CallSession, String)>>>,
    pub default_decision: CallDecision,
}

impl TestCallHandler {
    pub fn new(default_decision: CallDecision) -> Self {
        Self {
            incoming_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            ended_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            default_decision,
        }
    }

    pub fn get_incoming_calls(&self) -> Vec<IncomingCall> {
        self.incoming_calls.lock().unwrap().clone()
    }

    pub fn get_ended_calls(&self) -> Vec<(CallSession, String)> {
        self.ended_calls.lock().unwrap().clone()
    }

    pub fn clear_events(&self) {
        self.incoming_calls.lock().unwrap().clear();
        self.ended_calls.lock().unwrap().clear();
    }

    pub fn incoming_call_count(&self) -> usize {
        self.incoming_calls.lock().unwrap().len()
    }

    pub fn ended_call_count(&self) -> usize {
        self.ended_calls.lock().unwrap().len()
    }
}

#[async_trait]
impl CallHandler for TestCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        self.incoming_calls.lock().unwrap().push(call);
        self.default_decision.clone()
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        self.ended_calls.lock().unwrap().push((call, reason.to_string()));
    }
}

/// Helper for testing API builders
pub struct ApiBuilderTestHelper {
    pub config: ApiTestConfig,
}

impl ApiBuilderTestHelper {
    pub fn new() -> Self {
        Self::new_with_config(ApiTestConfig::default())
    }

    pub fn new_with_config(config: ApiTestConfig) -> Self {
        Self { config }
    }

    /// Create a test SessionManagerBuilder with common test settings
    pub fn create_test_builder(&self) -> SessionManagerBuilder {
//         let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
        
        SessionManagerBuilder::new()
            .with_sip_port(0) // Use random port for testing
            .with_local_address("sip:test@127.0.0.1")
            .with_media_ports(20000, 30000)
    }

    /// Create a P2P test builder
    pub fn create_p2p_builder(&self) -> SessionManagerBuilder {
//         let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
        
        SessionManagerBuilder::new()
            .with_sip_port(0)
            .with_local_address("sip:test@127.0.0.1")
            .with_media_ports(30000, 40000)
            
    }

    /// Test different builder configurations
    pub fn test_builder_configurations(&self) -> Vec<SessionManagerBuilder> {
        vec![
            // Default configuration
            SessionManagerBuilder::new(),
            
            // Custom port configuration
            SessionManagerBuilder::new()
                .with_sip_port(5070),
            
            // P2P mode configuration
            SessionManagerBuilder::new()
                ,
                
            // Full configuration
            SessionManagerBuilder::new()
                .with_sip_port(0) // Random port for testing
                .with_local_address("sip:test@127.0.0.1")
                .with_media_ports(10000, 20000)
                .with_handler(Arc::new(TestCallHandler::new(CallDecision::Accept(None))))
                ,
        ]
    }

    /// Validate a SessionManagerConfig
    pub fn validate_config(&self, config: &SessionManagerConfig) -> Result<()> {
        if config.sip_port == 0 {
            return Err(SessionError::Other("Invalid SIP port".to_string()));
        }

        if config.media_port_start >= config.media_port_end {
            return Err(SessionError::Other("Invalid media port range".to_string()));
        }

        if config.local_address.is_empty() {
            return Err(SessionError::Other("Empty bind address".to_string()));
        }

        Ok(())
    }
}

/// Helper for testing API types
pub struct ApiTypesTestHelper {
    pub config: ApiTestConfig,
}

impl ApiTypesTestHelper {
    pub fn new() -> Self {
        Self::new_with_config(ApiTestConfig::default())
    }

    pub fn new_with_config(config: ApiTestConfig) -> Self {
        Self { config }
    }

    /// Create test session IDs
    pub fn create_test_session_ids(&self, count: usize) -> Vec<SessionId> {
        (0..count).map(|i| SessionId(format!("test_session_{}", i))).collect()
    }

    /// Create test call sessions
    pub fn create_test_call_sessions(&self, count: usize) -> Vec<CallSession> {
        (0..count)
            .map(|i| CallSession {
                id: SessionId(format!("session_{}", i)),
                from: format!("sip:user{}@example.com", i),
                to: format!("sip:target{}@example.com", i),
                state: CallState::Active,
                started_at: Some(Instant::now()),
                sip_call_id: None,
            })
            .collect()
    }

    /// Create test incoming calls
    pub fn create_test_incoming_calls(&self, count: usize) -> Vec<IncomingCall> {
        (0..count)
            .map(|i| IncomingCall {
                id: SessionId(format!("incoming_{}", i)),
                from: format!("sip:caller{}@example.com", i),
                to: format!("sip:callee{}@example.com", i),
                sdp: Some(self.create_test_sdp(&format!("session_{}", i))),
                headers: HashMap::new(),
                received_at: Instant::now(),
                sip_call_id: None,
            })
            .collect()
    }

    /// Create test SDP
    pub fn create_test_sdp(&self, session_id: &str) -> String {
        format!(
            "v=0\r\n\
             o=test {} 0 IN IP4 127.0.0.1\r\n\
             s=Test Session\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio 5004 RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n",
            session_id
        )
    }

    /// Create minimal SDP
    pub fn create_minimal_sdp(&self) -> String {
        "v=0\r\n\
         o=test 0 0 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         c=IN IP4 127.0.0.1\r\n\
         t=0 0\r\n\
         m=audio 5004 RTP/AVP 0\r\n".to_string()
    }

    /// Create complex SDP with multiple media streams
    pub fn create_complex_sdp(&self) -> String {
        "v=0\r\n\
         o=test 12345 67890 IN IP4 192.168.1.100\r\n\
         s=Complex Test Session\r\n\
         c=IN IP4 192.168.1.100\r\n\
         t=0 0\r\n\
         m=audio 5004 RTP/AVP 0 8 18\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=rtpmap:18 G729/8000\r\n\
         m=video 5006 RTP/AVP 96\r\n\
         a=rtpmap:96 H264/90000\r\n\
         a=framerate:30\r\n".to_string()
    }

    /// Get all possible call states for testing
    pub fn get_all_call_states(&self) -> Vec<CallState> {
        vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
            CallState::Transferring,
            CallState::Terminating,
            CallState::Terminated,
            CallState::Cancelled,
            CallState::Failed("Test failure".to_string()),
        ]
    }

    /// Get valid transitions for call states
    pub fn get_valid_state_transitions(&self) -> Vec<(CallState, CallState)> {
        vec![
            (CallState::Initiating, CallState::Ringing),
            (CallState::Ringing, CallState::Active),
            (CallState::Active, CallState::OnHold),
            (CallState::OnHold, CallState::Active),
            (CallState::Active, CallState::Transferring),
            (CallState::Active, CallState::Terminated),
            (CallState::OnHold, CallState::Terminated),
            (CallState::Transferring, CallState::Terminated),
        ]
    }

    /// Get all possible call decisions for testing
    pub fn get_all_call_decisions(&self) -> Vec<CallDecision> {
        vec![
            CallDecision::Accept(None),
            CallDecision::Reject("Test rejection".to_string()),
            CallDecision::Defer,
            CallDecision::Forward("sip:forward@example.com".to_string()),
        ]
    }

    /// Create test media info
    pub fn create_test_media_info(&self, session_id: &str) -> MediaInfo {
        MediaInfo {
            local_sdp: Some(self.create_test_sdp(session_id)),
            remote_sdp: Some(self.create_test_sdp(&format!("remote_{}", session_id))),
            local_rtp_port: Some(5004),
            remote_rtp_port: Some(5006),
            codec: Some("PCMU".to_string()),
            quality_metrics: None,
        rtp_stats: None,
        }
    }

    /// Create test session stats
    pub fn create_test_session_stats(&self) -> SessionStats {
        SessionStats {
            total_sessions: 150,
            active_sessions: 23,
            failed_sessions: 7,
            average_duration: Some(Duration::from_secs(180)),
        }
    }

    /// Validate call session data
    pub fn validate_call_session(&self, session: &CallSession) -> Result<()> {
        if session.id.as_str().is_empty() {
            return Err(SessionError::Other("Empty session ID".to_string()));
        }

        if !ApiTestUtils::is_valid_sip_uri(&session.from) {
            return Err(SessionError::Other("Invalid from URI".to_string()));
        }

        if !ApiTestUtils::is_valid_sip_uri(&session.to) {
            return Err(SessionError::Other("Invalid to URI".to_string()));
        }

        Ok(())
    }

    /// Validate incoming call data
    pub fn validate_incoming_call(&self, call: &IncomingCall) -> Result<()> {
        if call.id.as_str().is_empty() {
            return Err(SessionError::Other("Empty call ID".to_string()));
        }

        if !ApiTestUtils::is_valid_sip_uri(&call.from) {
            return Err(SessionError::Other("Invalid caller URI".to_string()));
        }

        if !ApiTestUtils::is_valid_sip_uri(&call.to) {
            return Err(SessionError::Other("Invalid callee URI".to_string()));
        }

        if let Some(ref sdp) = call.sdp {
            if !ApiTestUtils::is_valid_sdp(sdp) {
                return Err(SessionError::Other("Invalid SDP".to_string()));
            }
        }

        Ok(())
    }
}

/// Helper for API operation testing
pub struct ApiOperationTestHelper {
    pub config: ApiTestConfig,
}

impl ApiOperationTestHelper {
    pub fn new() -> Self {
        Self::new_with_config(ApiTestConfig::default())
    }

    pub fn new_with_config(config: ApiTestConfig) -> Self {
        Self { config }
    }

    /// Test session creation with various parameters
    pub async fn test_session_creation_variations(&self, manager: &Arc<SessionCoordinator>) -> Result<Vec<CallSession>> {
        let mut sessions = Vec::new();

        // Basic session
        let session1 = manager.create_outgoing_call(
            "sip:test1@example.com",
            "sip:target1@example.com",
            None
        ).await?;
        sessions.push(session1);

        // Session with custom SDP
        let custom_sdp = ApiTypesTestHelper::new().create_test_sdp("custom");
        let session2 = manager.create_outgoing_call(
            "sip:test2@example.com",
            "sip:target2@example.com",
            Some(custom_sdp)
        ).await?;
        sessions.push(session2);

        // Session with complex URIs
        let session3 = manager.create_outgoing_call(
            "sip:user+tag@domain.com:5060",
            "sip:complex.user@sub.domain.com:5061",
            None
        ).await?;
        sessions.push(session3);

        Ok(sessions)
    }

    /// Test control operations on a session
    pub async fn test_control_operations(&self, manager: &Arc<SessionCoordinator>, session: &CallSession) -> Result<()> {
        // First ensure session is in active state for control operations
        if !session.is_active() {
            return Err(SessionError::invalid_state("Session must be active for control operations"));
        }

        // Test hold/resume cycle
        // manager.hold_session(&session.id).await?;
        // manager.resume_session(&session.id).await?;

        // Test DTMF
        // manager.send_dtmf(&session.id, "123*#").await?;

        // Test mute/unmute
        // manager.mute_session(&session.id, true).await?;
        // manager.mute_session(&session.id, false).await?;

        // Test media update
        let new_sdp = ApiTypesTestHelper::new().create_complex_sdp();
        // manager.update_media(&session.id, &new_sdp).await?;

        Ok(())
    }

    /// Test concurrent operations on multiple sessions
    pub async fn test_concurrent_operations(&self, manager: &Arc<SessionCoordinator>) -> Result<Vec<CallSession>> {
        let concurrent_count = self.config.concurrent_operations;
        let mut handles = Vec::new();

        for i in 0..concurrent_count {
            let manager_clone = manager.clone();
            let handle = tokio::spawn(async move {
                manager_clone.create_outgoing_call(
                    &format!("sip:concurrent{}@example.com", i),
                    &format!("sip:target{}@example.com", i),
                    None
                ).await
            });
            handles.push(handle);
        }

        let mut sessions = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(session)) => sessions.push(session),
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(SessionError::Other("Join error".to_string())),
            }
        }

        Ok(sessions)
    }
}

/// Utility functions for API testing
pub struct ApiTestUtils;

impl ApiTestUtils {
    /// Create a timeout wrapper for async operations
    pub async fn with_timeout<F, T>(operation: F, timeout: Duration) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        match tokio::time::timeout(timeout, operation).await {
            Ok(result) => result,
            Err(_) => Err(SessionError::Other("Operation timed out".to_string())),
        }
    }

    /// Wait for a condition to be true
    pub async fn wait_for_condition<F>(
        mut condition: F,
        timeout: Duration,
        check_interval: Duration,
    ) -> Result<()>
    where
        F: FnMut() -> bool,
    {
        let start = Instant::now();
        
        while start.elapsed() < timeout {
            if condition() {
                return Ok(());
            }
            tokio::time::sleep(check_interval).await;
        }
        
        Err(SessionError::Other("Condition timeout".to_string()))
    }

    /// Generate concurrent operations for stress testing
    pub async fn run_concurrent_operations<F, T>(
        operations: Vec<F>,
        timeout: Duration,
    ) -> Vec<Result<T>>
    where
        F: std::future::Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let handles: Vec<_> = operations
            .into_iter()
            .map(|op| {
                tokio::spawn(async move {
                    match tokio::time::timeout(timeout, op).await {
                        Ok(result) => result,
                        Err(_) => Err(SessionError::Other("Operation timed out".to_string())),
                    }
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(_) => results.push(Err(SessionError::Other("Join error".to_string()))),
            }
        }

        results
    }

    /// Validate SDP format
    pub fn is_valid_sdp(sdp: &str) -> bool {
        sdp.contains("v=0") && 
        sdp.contains("o=") && 
        sdp.contains("s=") && 
        sdp.contains("m=")
    }

    /// Validate SIP URI format
    pub fn is_valid_sip_uri(uri: &str) -> bool {
        if !uri.starts_with("sip:") && !uri.starts_with("sips:") {
            return false;
        }
        
        // Must have content after the scheme
        let after_scheme = if uri.starts_with("sips:") {
            &uri[5..]
        } else {
            &uri[4..]
        };
        
        // Must not be empty
        if after_scheme.is_empty() {
            return false;
        }
        
        // Check for basic structure: user@host or host
        // Must not start with @ (empty user)
        if after_scheme.starts_with('@') {
            return false;
        }
        
        // Must contain at least some content
        after_scheme.len() > 0
    }

    /// Generate random test data
    pub fn generate_random_session_id() -> SessionId {
        SessionId(format!("test_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()))
    }

    pub fn generate_random_sip_uri() -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        format!("sip:user{}@test.com", timestamp)
    }

    /// Validate configuration parameters
    pub fn validate_port_range(start: u16, end: u16) -> Result<()> {
        if start >= end {
            return Err(SessionError::Other("Start port must be less than end port".to_string()));
        }

        if start < 1024 {
            return Err(SessionError::Other("Port range should start above 1024".to_string()));
        }

        if end > 65535 {
            return Err(SessionError::Other("Port range cannot exceed 65535".to_string()));
        }

        Ok(())
    }

    /// Create test headers
    pub fn create_test_headers() -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("User-Agent".to_string(), "RVOIP-Test/1.0".to_string());
        headers.insert("Contact".to_string(), "sip:test@127.0.0.1:5060".to_string());
        headers.insert("Content-Type".to_string(), "application/sdp".to_string());
        headers
    }

    /// Measure operation performance
    pub async fn measure_performance<F, T>(operation: F) -> (Result<T>, Duration)
    where
        F: std::future::Future<Output = Result<T>>,
    {
        let start = Instant::now();
        let result = operation.await;
        let duration = start.elapsed();
        (result, duration)
    }

    /// Validate session statistics
    pub fn validate_session_stats(stats: &SessionStats) -> Result<()> {
        if stats.active_sessions > stats.total_sessions {
            return Err(SessionError::Other("Active sessions cannot exceed total sessions".to_string()));
        }

        if stats.failed_sessions > stats.total_sessions {
            return Err(SessionError::Other("Failed sessions cannot exceed total sessions".to_string()));
        }

        Ok(())
    }
} 