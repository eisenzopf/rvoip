use rvoip_session_core::api::control::SessionControl;
// Bridge Test Utilities
//
// Common helper functions and test utilities for testing bridge functionality
// across different bridge scenarios and configurations.

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashSet;
use tokio::sync::Mutex;
use rvoip_session_core::{
    SessionCoordinator,
    api::{
        types::{SessionId, CallSession},
        handlers::CallHandler,
    },
    bridge::{SessionBridge, BridgeId, BridgeConfig},
};
use crate::common::*;

/// Test bridge configuration for consistent testing
#[derive(Debug, Clone)]
pub struct BridgeTestConfig {
    pub max_sessions: usize,
    pub auto_start: bool,
    pub auto_stop_on_empty: bool,
    pub bridge_timeout: Duration,
}

impl Default for BridgeTestConfig {
    fn default() -> Self {
        Self {
            max_sessions: 5,
            auto_start: true,
            auto_stop_on_empty: true,
            bridge_timeout: Duration::from_secs(2),
        }
    }
}

impl BridgeTestConfig {
    pub fn small_bridge() -> Self {
        Self {
            max_sessions: 2,
            auto_start: false,
            auto_stop_on_empty: false,
            bridge_timeout: Duration::from_secs(1),
        }
    }

    pub fn large_bridge() -> Self {
        Self {
            max_sessions: 20,
            auto_start: true,
            auto_stop_on_empty: true,
            bridge_timeout: Duration::from_secs(5),
        }
    }
}

/// Create a test bridge with default configuration
pub fn create_test_bridge(bridge_id: &str) -> SessionBridge {
    SessionBridge::new(bridge_id.to_string())
}

/// Create a test bridge with custom configuration
pub fn create_test_bridge_with_config(bridge_id: &str, config: BridgeTestConfig) -> SessionBridge {
    let bridge_config = BridgeConfig {
        max_sessions: config.max_sessions,
        auto_start: config.auto_start,
        auto_stop_on_empty: config.auto_stop_on_empty,
    };
    
    // Note: SessionBridge constructor doesn't take BridgeConfig in current implementation
    // This would need to be updated when BridgeConfig integration is added
    SessionBridge::new(bridge_id.to_string())
}

/// Create a set of test session IDs
pub fn create_test_session_ids(count: usize) -> Vec<SessionId> {
    (0..count).map(|i| SessionId(format!("test-session-{}", i))).collect()
}

/// Create a bridge with pre-populated sessions
pub fn create_bridge_with_sessions(bridge_id: &str, session_count: usize) -> (SessionBridge, Vec<SessionId>) {
    let mut bridge = create_test_bridge(bridge_id);
    let session_ids = create_test_session_ids(session_count);
    
    for session_id in &session_ids {
        bridge.add_session(session_id.clone()).expect("Failed to add session to bridge");
    }
    
    (bridge, session_ids)
}

/// Verify bridge state matches expected values
pub fn verify_bridge_state(
    bridge: &SessionBridge,
    expected_active: bool,
    expected_session_count: usize,
) {
    assert_eq!(bridge.is_active(), expected_active, "Bridge active state mismatch");
    assert_eq!(bridge.session_count(), expected_session_count, "Bridge session count mismatch");
}

/// Test bridge session management operations
pub struct BridgeSessionCoordinator {
    bridge: SessionBridge,
    sessions: HashSet<SessionId>,
}

impl BridgeSessionCoordinator {
    pub fn new(bridge_id: &str) -> Self {
        Self {
            bridge: create_test_bridge(bridge_id),
            sessions: HashSet::new(),
        }
    }

    pub fn add_session(&mut self, session_id: SessionId) -> Result<(), rvoip_session_core::SessionError> {
        self.bridge.add_session(session_id.clone())?;
        self.sessions.insert(session_id);
        Ok(())
    }

    pub fn remove_session(&mut self, session_id: &SessionId) -> Result<(), rvoip_session_core::SessionError> {
        self.bridge.remove_session(session_id)?;
        self.sessions.remove(session_id);
        Ok(())
    }

    pub fn start_bridge(&mut self) -> Result<(), rvoip_session_core::SessionError> {
        self.bridge.start()
    }

    pub fn stop_bridge(&mut self) -> Result<(), rvoip_session_core::SessionError> {
        self.bridge.stop()
    }

    pub fn bridge(&self) -> &SessionBridge {
        &self.bridge
    }

    pub fn sessions(&self) -> &HashSet<SessionId> {
        &self.sessions
    }

    pub fn verify_consistency(&self) {
        assert_eq!(
            self.bridge.session_count(),
            self.sessions.len(),
            "Bridge and manager session counts don't match"
        );
    }
}

/// Bridge integration test helper
pub struct BridgeIntegrationHelper {
    pub managers: Vec<Arc<SessionCoordinator>>,
    pub bridges: Vec<Arc<Mutex<SessionBridge>>>,
}

impl BridgeIntegrationHelper {
    pub async fn new(manager_count: usize, bridge_count: usize) -> Result<Self, rvoip_session_core::SessionError> {
        let mut managers = Vec::new();
        
        // Create session managers
        for i in 0..manager_count {
            let handler = TestCallHandler::new(true);
            let manager = create_session_manager(
                Arc::new(handler),
                None,
                Some(&format!("sip:user{}@localhost", i))
            ).await?;
            managers.push(manager);
        }
        
        // Create bridges
        let mut bridges = Vec::new();
        for i in 0..bridge_count {
            let bridge = SessionBridge::new(format!("test-bridge-{}", i));
            bridges.push(Arc::new(Mutex::new(bridge)));
        }
        
        Ok(Self { managers, bridges })
    }

    pub async fn create_call_between_managers(&self, caller_idx: usize, callee_idx: usize) -> Result<CallSession, rvoip_session_core::SessionError> {
        if caller_idx >= self.managers.len() || callee_idx >= self.managers.len() {
            return Err(rvoip_session_core::SessionError::Other("Manager index out of bounds".to_string()));
        }
        
        let caller = &self.managers[caller_idx];
        let callee = &self.managers[callee_idx];
        
        let caller_addr = caller.get_bound_address();
        let callee_addr = callee.get_bound_address();
        
        let from_uri = format!("sip:user{}@{}", caller_idx, caller_addr.ip());
        let to_uri = format!("sip:user{}@{}", callee_idx, callee_addr);
        
        caller.create_outgoing_call(&from_uri, &to_uri, Some("test SDP".to_string())).await
    }

    pub async fn add_session_to_bridge(&self, bridge_idx: usize, session_id: SessionId) -> Result<(), rvoip_session_core::SessionError> {
        if bridge_idx >= self.bridges.len() {
            return Err(rvoip_session_core::SessionError::Other("Bridge index out of bounds".to_string()));
        }
        
        let bridge = self.bridges[bridge_idx].clone();
        let mut bridge_guard = bridge.lock().await;
        bridge_guard.add_session(session_id)
    }

    pub async fn start_bridge(&self, bridge_idx: usize) -> Result<(), rvoip_session_core::SessionError> {
        if bridge_idx >= self.bridges.len() {
            return Err(rvoip_session_core::SessionError::Other("Bridge index out of bounds".to_string()));
        }
        
        let bridge = self.bridges[bridge_idx].clone();
        let mut bridge_guard = bridge.lock().await;
        bridge_guard.start()
    }

    pub async fn get_bridge_state(&self, bridge_idx: usize) -> Option<(bool, usize)> {
        if bridge_idx >= self.bridges.len() {
            return None;
        }
        
        let bridge = self.bridges[bridge_idx].clone();
        let bridge_guard = bridge.lock().await;
        Some((bridge_guard.is_active(), bridge_guard.session_count()))
    }

    pub async fn cleanup(&self) -> Result<(), rvoip_session_core::SessionError> {
        // Stop all bridges
        for bridge in &self.bridges {
            let mut bridge_guard = bridge.lock().await;
            let _ = bridge_guard.stop();
        }
        
        // Cleanup managers
        cleanup_managers(self.managers.clone()).await
    }
}

/// Utility to create multiple bridges for testing
pub fn create_multiple_bridges(count: usize, prefix: &str) -> Vec<SessionBridge> {
    (0..count)
        .map(|i| create_test_bridge(&format!("{}-{}", prefix, i)))
        .collect()
}

/// Utility to create bridge with specific session configuration
pub fn create_bridge_scenario(
    bridge_id: &str,
    session_count: usize,
    start_bridge: bool,
) -> (SessionBridge, Vec<SessionId>) {
    let (mut bridge, session_ids) = create_bridge_with_sessions(bridge_id, session_count);
    
    if start_bridge {
        bridge.start().expect("Failed to start bridge");
    }
    
    (bridge, session_ids)
}

/// Performance test helper for bridge operations
pub struct BridgePerformanceTest {
    bridge: SessionBridge,
    session_ids: Vec<SessionId>,
}

impl BridgePerformanceTest {
    pub fn bridge(&self) -> &SessionBridge {
        &self.bridge
    }
}

impl BridgePerformanceTest {
    pub fn new(bridge_id: &str, session_count: usize) -> Self {
        let bridge = create_test_bridge(bridge_id);
        let session_ids = create_test_session_ids(session_count);
        
        Self { bridge, session_ids }
    }
    
    pub async fn run_add_session_benchmark(&mut self) -> Duration {
        let start = std::time::Instant::now();
        
        for session_id in &self.session_ids {
            self.bridge.add_session(session_id.clone()).expect("Failed to add session");
        }
        
        start.elapsed()
    }
    
    pub async fn run_remove_session_benchmark(&mut self) -> Duration {
        let start = std::time::Instant::now();
        
        for session_id in &self.session_ids {
            self.bridge.remove_session(session_id).expect("Failed to remove session");
        }
        
        start.elapsed()
    }
    
    pub fn verify_final_state(&self) {
        assert_eq!(self.bridge.session_count(), 0, "Bridge should be empty after performance test");
    }
}

/// Concurrency test helper
pub struct BridgeConcurrencyTest {
    bridges: Vec<Arc<Mutex<SessionBridge>>>,
}

impl BridgeConcurrencyTest {
    pub fn new(bridge_count: usize) -> Self {
        let bridges = (0..bridge_count)
            .map(|i| Arc::new(Mutex::new(create_test_bridge(&format!("concurrent-bridge-{}", i)))))
            .collect();
            
        Self { bridges }
    }
    
    pub async fn run_concurrent_operations(&self, operations_per_bridge: usize) -> Vec<Result<(), String>> {
        let mut handles = Vec::new();
        
        for (i, bridge) in self.bridges.iter().enumerate() {
            let bridge_clone = bridge.clone();
            let handle = tokio::spawn(async move {
                let session_ids = create_test_session_ids(operations_per_bridge);
                
                // Start bridge
                {
                    let mut bridge_guard = bridge_clone.lock().await;
                    bridge_guard.start().map_err(|e| format!("Failed to start bridge {}: {:?}", i, e))?;
                }
                
                // Add sessions
                for session_id in &session_ids {
                    let mut bridge_guard = bridge_clone.lock().await;
                    bridge_guard.add_session(session_id.clone())
                        .map_err(|e| format!("Failed to add session to bridge {}: {:?}", i, e))?;
                }
                
                // Remove sessions
                for session_id in &session_ids {
                    let mut bridge_guard = bridge_clone.lock().await;
                    bridge_guard.remove_session(session_id)
                        .map_err(|e| format!("Failed to remove session from bridge {}: {:?}", i, e))?;
                }
                
                // Stop bridge
                {
                    let mut bridge_guard = bridge_clone.lock().await;
                    bridge_guard.stop().map_err(|e| format!("Failed to stop bridge {}: {:?}", i, e))?;
                }
                
                Ok::<(), String>(())
            });
            handles.push(handle);
        }
        
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => results.push(Err(format!("Task failed: {:?}", e))),
            }
        }
        
        results
    }
} 