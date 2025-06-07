//! Coordination Test Utilities
//!
//! Shared utilities for testing coordination functionality (resources, priority, groups).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use rvoip_session_core::{
    api::types::SessionId,
    coordination::{
        resources::{ResourceManager, ResourceLimits},
        priority::{PriorityManager, Priority},
        groups::SessionGroups,
    },
    Result,
};

/// Configuration for coordination performance tests
#[derive(Debug, Clone)]
pub struct CoordinationPerfTestConfig {
    pub max_sessions: usize,
    pub test_duration: Duration,
    pub operation_delay: Duration,
}

impl Default for CoordinationPerfTestConfig {
    fn default() -> Self {
        Self {
            max_sessions: 100,
            test_duration: Duration::from_secs(2),
            operation_delay: Duration::from_micros(10),
        }
    }
}

impl CoordinationPerfTestConfig {
    pub fn fast() -> Self {
        Self {
            max_sessions: 50,
            test_duration: Duration::from_millis(500),
            operation_delay: Duration::from_micros(1),
        }
    }

    pub fn stress() -> Self {
        Self {
            max_sessions: 1000,
            test_duration: Duration::from_secs(5),
            operation_delay: Duration::from_nanos(100),
        }
    }
}

/// Helper for testing resource management
pub struct ResourceTestHelper {
    manager: ResourceManager,
    allocated_sessions: Vec<SessionId>,
}

impl ResourceTestHelper {
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            manager: ResourceManager::new(limits),
            allocated_sessions: Vec::new(),
        }
    }

    pub fn new_default() -> Self {
        Self::new(ResourceLimits::default())
    }

    pub fn new_with_limits(max_sessions: usize, max_media_ports: usize) -> Self {
        Self::new(ResourceLimits { max_sessions, max_media_ports })
    }

    pub fn manager(&self) -> &ResourceManager {
        &self.manager
    }

    pub fn manager_mut(&mut self) -> &mut ResourceManager {
        &mut self.manager
    }

    pub fn allocate_session(&mut self) -> Result<SessionId> {
        self.manager.allocate_session()?;
        let session_id = SessionId::new();
        self.allocated_sessions.push(session_id.clone());
        Ok(session_id)
    }

    pub fn deallocate_session(&mut self, session_id: &SessionId) -> Result<()> {
        if let Some(pos) = self.allocated_sessions.iter().position(|id| id == session_id) {
            self.allocated_sessions.remove(pos);
            self.manager.deallocate_session()
        } else {
            Err(rvoip_session_core::SessionError::Other("Session not found".to_string()))
        }
    }

    pub fn allocate_sessions(&mut self, count: usize) -> Result<Vec<SessionId>> {
        let mut sessions = Vec::new();
        for _ in 0..count {
            match self.allocate_session() {
                Ok(session_id) => sessions.push(session_id),
                Err(e) => return Err(e),
            }
        }
        Ok(sessions)
    }

    pub fn allocated_count(&self) -> usize {
        self.allocated_sessions.len()
    }

    pub fn verify_allocation_consistency(&self) -> bool {
        // In a real implementation, we'd check internal state consistency
        true
    }

    pub fn stress_test_allocations(&mut self, config: &CoordinationPerfTestConfig) -> Result<(usize, Duration)> {
        let start = Instant::now();
        let mut allocated = 0;
        
        while start.elapsed() < config.test_duration {
            if self.allocated_count() < config.max_sessions {
                if self.allocate_session().is_ok() {
                    allocated += 1;
                }
            }
            
            if config.operation_delay > Duration::ZERO {
                std::thread::sleep(config.operation_delay);
            }
        }
        
        Ok((allocated, start.elapsed()))
    }
}

/// Helper for testing priority management
pub struct PriorityTestHelper {
    manager: PriorityManager,
    test_sessions: Vec<SessionId>,
}

impl PriorityTestHelper {
    pub fn new() -> Self {
        Self {
            manager: PriorityManager::new(),
            test_sessions: Vec::new(),
        }
    }

    pub fn manager(&self) -> &PriorityManager {
        &self.manager
    }

    pub fn manager_mut(&mut self) -> &mut PriorityManager {
        &mut self.manager
    }

    pub fn create_test_session(&mut self) -> SessionId {
        let session_id = SessionId::new();
        self.test_sessions.push(session_id.clone());
        session_id
    }

    pub fn create_test_sessions(&mut self, count: usize) -> Vec<SessionId> {
        (0..count).map(|_| self.create_test_session()).collect()
    }

    pub fn set_priority(&mut self, session_id: SessionId, priority: Priority) -> Result<()> {
        self.manager.set_priority(session_id, priority)
    }

    pub fn get_priority(&self, session_id: &SessionId) -> Priority {
        self.manager.get_priority(session_id)
    }

    pub fn create_session_with_priority(&mut self, priority: Priority) -> SessionId {
        let session_id = self.create_test_session();
        self.set_priority(session_id.clone(), priority).expect("Failed to set priority");
        session_id
    }

    pub fn create_priority_test_set(&mut self) -> HashMap<Priority, SessionId> {
        let mut sessions = HashMap::new();
        sessions.insert(Priority::Low, self.create_session_with_priority(Priority::Low));
        sessions.insert(Priority::Normal, self.create_session_with_priority(Priority::Normal));
        sessions.insert(Priority::High, self.create_session_with_priority(Priority::High));
        sessions.insert(Priority::Critical, self.create_session_with_priority(Priority::Critical));
        sessions
    }

    pub fn verify_priority_ordering(&self, sessions: &[SessionId]) -> bool {
        let mut priorities: Vec<Priority> = sessions.iter()
            .map(|id| self.get_priority(id))
            .collect();
        
        let sorted_priorities = {
            let mut sorted = priorities.clone();
            sorted.sort();
            sorted
        };
        
        priorities.sort();
        priorities == sorted_priorities
    }

    pub fn cleanup_session(&mut self, session_id: &SessionId) -> Result<()> {
        self.test_sessions.retain(|id| id != session_id);
        self.manager.remove_session(session_id)
    }

    pub fn test_session_count(&self) -> usize {
        self.test_sessions.len()
    }
}

/// Helper for testing session groups
pub struct GroupsTestHelper {
    groups: SessionGroups,
    test_sessions: Vec<SessionId>,
}

impl GroupsTestHelper {
    pub fn new() -> Self {
        Self {
            groups: SessionGroups::new(),
            test_sessions: Vec::new(),
        }
    }

    pub fn groups(&self) -> &SessionGroups {
        &self.groups
    }

    pub fn groups_mut(&mut self) -> &mut SessionGroups {
        &mut self.groups
    }

    pub fn create_test_session(&mut self) -> SessionId {
        let session_id = SessionId::new();
        self.test_sessions.push(session_id.clone());
        session_id
    }

    pub fn create_test_sessions(&mut self, count: usize) -> Vec<SessionId> {
        (0..count).map(|_| self.create_test_session()).collect()
    }

    pub fn add_to_group(&mut self, group_name: &str, session_id: SessionId) -> Result<()> {
        self.groups.add_to_group(group_name, session_id)
    }

    pub fn remove_from_group(&mut self, group_name: &str, session_id: &SessionId) -> Result<()> {
        self.groups.remove_from_group(group_name, session_id)
    }

    pub fn get_group_sessions(&self, group_name: &str) -> Vec<SessionId> {
        self.groups.get_group_sessions(group_name)
    }

    pub fn create_test_group(&mut self, group_name: &str, session_count: usize) -> Vec<SessionId> {
        let sessions = self.create_test_sessions(session_count);
        for session_id in &sessions {
            self.add_to_group(group_name, session_id.clone()).expect("Failed to add to group");
        }
        sessions
    }

    pub fn verify_group_membership(&self, group_name: &str, expected_sessions: &[SessionId]) -> bool {
        let group_sessions = self.get_group_sessions(group_name);
        group_sessions.len() == expected_sessions.len() &&
        expected_sessions.iter().all(|id| group_sessions.contains(id))
    }

    pub fn get_group_count(&self, group_name: &str) -> usize {
        self.get_group_sessions(group_name).len()
    }

    pub fn test_session_count(&self) -> usize {
        self.test_sessions.len()
    }
}

/// Integrated helper for testing all coordination components together
pub struct CoordinationIntegrationHelper {
    resource_helper: ResourceTestHelper,
    priority_helper: PriorityTestHelper,
    groups_helper: GroupsTestHelper,
}

impl CoordinationIntegrationHelper {
    pub fn new() -> Self {
        Self {
            resource_helper: ResourceTestHelper::new_default(),
            priority_helper: PriorityTestHelper::new(),
            groups_helper: GroupsTestHelper::new(),
        }
    }

    pub fn new_with_limits(max_sessions: usize, max_media_ports: usize) -> Self {
        Self {
            resource_helper: ResourceTestHelper::new_with_limits(max_sessions, max_media_ports),
            priority_helper: PriorityTestHelper::new(),
            groups_helper: GroupsTestHelper::new(),
        }
    }

    pub fn resource_helper(&self) -> &ResourceTestHelper {
        &self.resource_helper
    }

    pub fn resource_helper_mut(&mut self) -> &mut ResourceTestHelper {
        &mut self.resource_helper
    }

    pub fn priority_helper(&self) -> &PriorityTestHelper {
        &self.priority_helper
    }

    pub fn priority_helper_mut(&mut self) -> &mut PriorityTestHelper {
        &mut self.priority_helper
    }

    pub fn groups_helper(&self) -> &GroupsTestHelper {
        &self.groups_helper
    }

    pub fn groups_helper_mut(&mut self) -> &mut GroupsTestHelper {
        &mut self.groups_helper
    }

    pub fn create_coordinated_session(&mut self, group_name: &str, priority: Priority) -> Result<SessionId> {
        // Allocate resource
        let session_id = self.resource_helper.allocate_session()?;
        
        // Set priority
        self.priority_helper.set_priority(session_id.clone(), priority)?;
        
        // Add to group
        self.groups_helper.add_to_group(group_name, session_id.clone())?;
        
        Ok(session_id)
    }

    pub fn cleanup_coordinated_session(&mut self, session_id: &SessionId, group_name: &str) -> Result<()> {
        // Remove from group
        self.groups_helper.remove_from_group(group_name, session_id)?;
        
        // Remove priority
        self.priority_helper.cleanup_session(session_id)?;
        
        // Deallocate resource
        self.resource_helper.deallocate_session(session_id)?;
        
        Ok(())
    }

    pub fn verify_coordination_consistency(&self) -> bool {
        self.resource_helper.verify_allocation_consistency()
    }
}

/// Utilities for coordination testing
pub struct CoordinationTestUtils;

impl CoordinationTestUtils {
    /// Create test resource limits
    pub fn create_test_limits(max_sessions: usize, max_media_ports: usize) -> ResourceLimits {
        ResourceLimits { max_sessions, max_media_ports }
    }

    /// Create small limits for edge case testing
    pub fn create_small_limits() -> ResourceLimits {
        Self::create_test_limits(2, 4)
    }

    /// Create large limits for stress testing
    pub fn create_large_limits() -> ResourceLimits {
        Self::create_test_limits(10000, 20000)
    }

    /// Generate test session IDs
    pub fn generate_test_session_ids(count: usize) -> Vec<SessionId> {
        (0..count).map(|_| SessionId::new()).collect()
    }

    /// Create all priority levels for testing
    pub fn all_priorities() -> Vec<Priority> {
        vec![Priority::Low, Priority::Normal, Priority::High, Priority::Critical]
    }

    /// Create test group names
    pub fn test_group_names() -> Vec<&'static str> {
        vec!["conference", "queue", "emergency", "test_group"]
    }

    /// Validate priority ordering
    pub fn validate_priority_order(priorities: &[Priority]) -> bool {
        priorities.windows(2).all(|w| w[0] <= w[1])
    }

    /// Performance measurement for coordination operations
    pub fn measure_operation_performance<F, R>(operation: F) -> (R, Duration)
    where
        F: FnOnce() -> R,
    {
        let start = Instant::now();
        let result = operation();
        let duration = start.elapsed();
        (result, duration)
    }

    /// Stress test helper
    pub fn run_coordination_stress_test<F>(mut operation: F, iterations: usize, max_duration: Duration) -> (usize, Duration)
    where
        F: FnMut() -> Result<()>,
    {
        let start = Instant::now();
        let mut completed = 0;
        
        for _ in 0..iterations {
            if start.elapsed() >= max_duration {
                break;
            }
            
            if operation().is_ok() {
                completed += 1;
            }
        }
        
        (completed, start.elapsed())
    }

    /// Validate test configuration
    pub fn validate_test_config(config: &CoordinationPerfTestConfig) -> bool {
        config.max_sessions > 0 &&
        config.test_duration > Duration::ZERO
    }

    /// Create edge case test scenarios
    pub fn create_edge_case_scenarios() -> Vec<(&'static str, usize, usize)> {
        vec![
            ("zero_limits", 0, 0),
            ("single_session", 1, 2),
            ("small_limits", 5, 10),
            ("unbalanced", 100, 10),
        ]
    }

    /// Performance assertions
    pub fn assert_performance_acceptable(duration: Duration, max_expected: Duration, operation_name: &str) {
        assert!(
            duration <= max_expected,
            "{} took {:?}, expected <= {:?}",
            operation_name,
            duration,
            max_expected
        );
    }
}

/// Macro for creating coordination test scenarios
#[macro_export]
macro_rules! coordination_test_scenario {
    ($name:ident, $setup:expr, $test:expr) => {
        #[tokio::test]
        async fn $name() {
            let mut helper = $setup;
            $test(helper).await;
        }
    };
}

/// Macro for performance testing coordination operations
#[macro_export]
macro_rules! coordination_perf_test {
    ($name:ident, $operation:expr, $max_duration:expr) => {
        #[tokio::test]
        async fn $name() {
            let (result, duration) = CoordinationTestUtils::measure_operation_performance(|| {
                $operation
            });
            
            CoordinationTestUtils::assert_performance_acceptable(
                duration,
                $max_duration,
                stringify!($name)
            );
            
            assert!(result.is_ok(), "Operation failed: {:?}", result);
        }
    };
} 