//! Session Resource Sharing Policies
//! 
//! This module provides sophisticated policy management for session coordination:
//! 
//! - Resource sharing policies between sessions
//! - Coordination policies for session interactions
//! - Policy enforcement and conflict resolution
//! - Dynamic policy adaptation
//! - Policy-based access control

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use dashmap::DashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorContext};

/// Types of resource sharing policies
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceSharingPolicy {
    /// Exclusive access - no sharing
    Exclusive,
    
    /// Shared access with coordination
    Shared,
    
    /// Round-robin sharing
    RoundRobin,
    
    /// Priority-based sharing
    PriorityBased,
    
    /// Time-sliced sharing
    TimeSliced,
    
    /// Load-balanced sharing
    LoadBalanced,
    
    /// Custom sharing policy
    Custom(String),
}

impl std::fmt::Display for ResourceSharingPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceSharingPolicy::Exclusive => write!(f, "Exclusive"),
            ResourceSharingPolicy::Shared => write!(f, "Shared"),
            ResourceSharingPolicy::RoundRobin => write!(f, "RoundRobin"),
            ResourceSharingPolicy::PriorityBased => write!(f, "PriorityBased"),
            ResourceSharingPolicy::TimeSliced => write!(f, "TimeSliced"),
            ResourceSharingPolicy::LoadBalanced => write!(f, "LoadBalanced"),
            ResourceSharingPolicy::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

/// Types of coordination policies
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CoordinationPolicy {
    /// No coordination required
    None,
    
    /// Loose coordination (advisory)
    Loose,
    
    /// Strict coordination (mandatory)
    Strict,
    
    /// Consensus-based coordination
    Consensus,
    
    /// Leader-follower coordination
    LeaderFollower,
    
    /// Peer-to-peer coordination
    PeerToPeer,
    
    /// Custom coordination policy
    Custom(String),
}

impl std::fmt::Display for CoordinationPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoordinationPolicy::None => write!(f, "None"),
            CoordinationPolicy::Loose => write!(f, "Loose"),
            CoordinationPolicy::Strict => write!(f, "Strict"),
            CoordinationPolicy::Consensus => write!(f, "Consensus"),
            CoordinationPolicy::LeaderFollower => write!(f, "LeaderFollower"),
            CoordinationPolicy::PeerToPeer => write!(f, "PeerToPeer"),
            CoordinationPolicy::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

/// Resource types that can be shared
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceType {
    /// Bandwidth resource
    Bandwidth,
    
    /// CPU resource
    CPU,
    
    /// Memory resource
    Memory,
    
    /// Media bridge resource
    MediaBridge,
    
    /// Network connection
    NetworkConnection,
    
    /// Storage resource
    Storage,
    
    /// Custom resource type
    Custom(String),
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::Bandwidth => write!(f, "Bandwidth"),
            ResourceType::CPU => write!(f, "CPU"),
            ResourceType::Memory => write!(f, "Memory"),
            ResourceType::MediaBridge => write!(f, "MediaBridge"),
            ResourceType::NetworkConnection => write!(f, "NetworkConnection"),
            ResourceType::Storage => write!(f, "Storage"),
            ResourceType::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

/// Policy enforcement level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnforcementLevel {
    /// Advisory only - violations are logged
    Advisory,
    
    /// Warning - violations trigger warnings
    Warning,
    
    /// Strict - violations block operations
    Strict,
    
    /// Automatic - violations trigger automatic remediation
    Automatic,
}

impl std::fmt::Display for EnforcementLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnforcementLevel::Advisory => write!(f, "Advisory"),
            EnforcementLevel::Warning => write!(f, "Warning"),
            EnforcementLevel::Strict => write!(f, "Strict"),
            EnforcementLevel::Automatic => write!(f, "Automatic"),
        }
    }
}

/// Policy scope definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyScope {
    /// Apply to specific sessions
    Sessions(Vec<SessionId>),
    
    /// Apply to session groups
    Groups(Vec<String>),
    
    /// Apply to sessions with specific roles
    Roles(Vec<String>),
    
    /// Apply to sessions in specific states
    States(Vec<SessionState>),
    
    /// Apply globally to all sessions
    Global,
    
    /// Custom scope definition
    Custom {
        criteria: HashMap<String, String>,
    },
}

/// Configuration for a resource sharing policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Policy identifier
    pub policy_id: String,
    
    /// Human-readable policy name
    pub name: String,
    
    /// Policy description
    pub description: String,
    
    /// Resource type this policy applies to
    pub resource_type: ResourceType,
    
    /// Sharing policy
    pub sharing_policy: ResourceSharingPolicy,
    
    /// Coordination policy
    pub coordination_policy: CoordinationPolicy,
    
    /// Enforcement level
    pub enforcement_level: EnforcementLevel,
    
    /// Policy scope
    pub scope: PolicyScope,
    
    /// Policy-specific parameters
    pub parameters: HashMap<String, String>,
    
    /// When the policy becomes active
    pub effective_from: SystemTime,
    
    /// When the policy expires (if any)
    pub expires_at: Option<SystemTime>,
    
    /// Policy priority (higher values take precedence)
    pub priority: u32,
    
    /// Whether the policy is enabled
    pub enabled: bool,
}

impl PolicyConfig {
    /// Create a new policy configuration
    pub fn new(
        policy_id: String,
        name: String,
        resource_type: ResourceType,
        sharing_policy: ResourceSharingPolicy,
    ) -> Self {
        Self {
            policy_id,
            name,
            description: String::new(),
            resource_type,
            sharing_policy,
            coordination_policy: CoordinationPolicy::Loose,
            enforcement_level: EnforcementLevel::Warning,
            scope: PolicyScope::Global,
            parameters: HashMap::new(),
            effective_from: SystemTime::now(),
            expires_at: None,
            priority: 50,
            enabled: true,
        }
    }
    
    /// Check if policy is currently active
    pub fn is_active(&self) -> bool {
        if !self.enabled {
            return false;
        }
        
        let now = SystemTime::now();
        
        if now < self.effective_from {
            return false;
        }
        
        if let Some(expires_at) = self.expires_at {
            if now >= expires_at {
                return false;
            }
        }
        
        true
    }
    
    /// Check if policy applies to a session
    pub fn applies_to_session(&self, session_id: SessionId, session_state: SessionState, role: Option<&str>) -> bool {
        if !self.is_active() {
            return false;
        }
        
        match &self.scope {
            PolicyScope::Sessions(sessions) => sessions.contains(&session_id),
            PolicyScope::Groups(_) => {
                // Would need group membership info to determine
                false
            },
            PolicyScope::Roles(roles) => {
                if let Some(session_role) = role {
                    roles.contains(&session_role.to_string())
                } else {
                    false
                }
            },
            PolicyScope::States(states) => states.contains(&session_state),
            PolicyScope::Global => true,
            PolicyScope::Custom { .. } => {
                // Would need custom evaluation logic
                false
            },
        }
    }
}

/// Resource allocation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequest {
    /// Requesting session
    pub session_id: SessionId,
    
    /// Resource type requested
    pub resource_type: ResourceType,
    
    /// Amount requested
    pub amount: u64,
    
    /// Priority of the request
    pub priority: u32,
    
    /// Maximum wait time
    pub max_wait_time: Option<Duration>,
    
    /// Request metadata
    pub metadata: HashMap<String, String>,
    
    /// When the request was made
    pub requested_at: SystemTime,
}

/// Resource allocation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAllocation {
    /// Allocation ID
    pub allocation_id: String,
    
    /// Session that received the allocation
    pub session_id: SessionId,
    
    /// Resource type allocated
    pub resource_type: ResourceType,
    
    /// Amount allocated
    pub amount: u64,
    
    /// Sharing policy applied
    pub sharing_policy: ResourceSharingPolicy,
    
    /// When the allocation was made
    pub allocated_at: SystemTime,
    
    /// When the allocation expires (if any)
    pub expires_at: Option<SystemTime>,
    
    /// Allocation metadata
    pub metadata: HashMap<String, String>,
}

/// Policy violation information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    /// Violation ID
    pub violation_id: String,
    
    /// Policy that was violated
    pub policy_id: String,
    
    /// Session that caused the violation
    pub session_id: SessionId,
    
    /// Violation description
    pub description: String,
    
    /// Severity of the violation
    pub severity: ViolationSeverity,
    
    /// When the violation occurred
    pub occurred_at: SystemTime,
    
    /// Action taken (if any)
    pub action_taken: Option<String>,
    
    /// Violation metadata
    pub metadata: HashMap<String, String>,
}

/// Severity levels for policy violations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViolationSeverity {
    /// Informational - minor deviation
    Info,
    
    /// Warning - notable deviation
    Warning,
    
    /// Error - significant violation
    Error,
    
    /// Critical - severe violation requiring immediate action
    Critical,
}

impl std::fmt::Display for ViolationSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViolationSeverity::Info => write!(f, "Info"),
            ViolationSeverity::Warning => write!(f, "Warning"),
            ViolationSeverity::Error => write!(f, "Error"),
            ViolationSeverity::Critical => write!(f, "Critical"),
        }
    }
}

/// Policy management metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyMetrics {
    /// Total policies created
    pub total_policies_created: u64,
    
    /// Active policies count
    pub active_policies: u64,
    
    /// Policies by type
    pub policies_by_type: HashMap<String, u64>,
    
    /// Resource requests handled
    pub total_resource_requests: u64,
    
    /// Resource allocations made
    pub total_resource_allocations: u64,
    
    /// Policy violations detected
    pub total_violations: u64,
    
    /// Violations by severity
    pub violations_by_severity: HashMap<String, u64>,
    
    /// Average resource utilization
    pub average_resource_utilization: HashMap<String, f64>,
    
    /// Policy enforcement actions taken
    pub enforcement_actions: u64,
}

/// Session policy manager
pub struct SessionPolicyManager {
    /// Active policies
    policies: Arc<DashMap<String, PolicyConfig>>,
    
    /// Resource allocations
    allocations: Arc<DashMap<String, ResourceAllocation>>,
    
    /// Policy violations
    violations: Arc<DashMap<String, PolicyViolation>>,
    
    /// Session to allocations mapping
    session_allocations: Arc<DashMap<SessionId, Vec<String>>>,
    
    /// Resource usage tracking
    resource_usage: Arc<DashMap<ResourceType, u64>>,
    
    /// Policy metrics
    metrics: Arc<RwLock<PolicyMetrics>>,
    
    /// Configuration
    config: PolicyManagerConfig,
}

/// Configuration for policy manager
#[derive(Debug, Clone)]
pub struct PolicyManagerConfig {
    /// Maximum number of policies
    pub max_policies: Option<usize>,
    
    /// Default enforcement level
    pub default_enforcement_level: EnforcementLevel,
    
    /// Whether to track detailed metrics
    pub track_metrics: bool,
    
    /// Policy evaluation interval
    pub evaluation_interval: Duration,
    
    /// Maximum violations to retain
    pub max_violations: usize,
    
    /// Enable automatic policy adaptation
    pub enable_adaptation: bool,
}

impl Default for PolicyManagerConfig {
    fn default() -> Self {
        Self {
            max_policies: Some(1000),
            default_enforcement_level: EnforcementLevel::Warning,
            track_metrics: true,
            evaluation_interval: Duration::from_secs(30),
            max_violations: 10000,
            enable_adaptation: false,
        }
    }
}

impl SessionPolicyManager {
    /// Create a new policy manager
    pub fn new(config: PolicyManagerConfig) -> Self {
        Self {
            policies: Arc::new(DashMap::new()),
            allocations: Arc::new(DashMap::new()),
            violations: Arc::new(DashMap::new()),
            session_allocations: Arc::new(DashMap::new()),
            resource_usage: Arc::new(DashMap::new()),
            metrics: Arc::new(RwLock::new(PolicyMetrics::default())),
            config,
        }
    }
    
    /// Add a new policy
    pub async fn add_policy(&self, policy: PolicyConfig) -> Result<(), Error> {
        // Check policy limit
        if let Some(max_policies) = self.config.max_policies {
            if self.policies.len() >= max_policies {
                return Err(Error::InternalError(
                    format!("Maximum policy limit {} reached", max_policies),
                    ErrorContext::default().with_message("Policy limit exceeded")
                ));
            }
        }
        
        // Validate policy
        if policy.policy_id.is_empty() {
            return Err(Error::InternalError(
                "Policy ID cannot be empty".to_string(),
                ErrorContext::default().with_message("Invalid policy configuration")
            ));
        }
        
        // Check for duplicate policy ID
        if self.policies.contains_key(&policy.policy_id) {
            return Err(Error::InternalError(
                format!("Policy {} already exists", policy.policy_id),
                ErrorContext::default().with_message("Duplicate policy ID")
            ));
        }
        
        let policy_id = policy.policy_id.clone();
        self.policies.insert(policy_id.clone(), policy);
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.total_policies_created += 1;
            if self.policies.get(&policy_id).map_or(false, |p| p.is_active()) {
                metrics.active_policies += 1;
            }
        }
        
        info!("✅ Added policy {}", policy_id);
        Ok(())
    }
    
    /// Remove a policy
    pub async fn remove_policy(&self, policy_id: &str) -> Result<(), Error> {
        if let Some((_, policy)) = self.policies.remove(policy_id) {
            // Update metrics
            if self.config.track_metrics {
                let mut metrics = self.metrics.write().await;
                if policy.is_active() && metrics.active_policies > 0 {
                    metrics.active_policies -= 1;
                }
            }
            
            info!("🗑️ Removed policy {}", policy_id);
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Policy {} not found", policy_id),
                ErrorContext::default().with_message("Policy not found")
            ))
        }
    }
    
    /// Request resource allocation
    pub async fn request_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<Option<ResourceAllocation>, Error> {
        // Find applicable policies
        let applicable_policies = self.find_applicable_policies(
            request.session_id,
            &request.resource_type,
            SessionState::Connected, // Default state
            None, // No role specified
        ).await;
        
        if applicable_policies.is_empty() {
            // No policies apply, allow allocation with default sharing
            return self.allocate_resource_default(request).await;
        }
        
        // Get the highest priority policy
        let policy = applicable_policies.into_iter()
            .max_by_key(|p| p.priority)
            .unwrap();
        
        // Apply the policy
        match policy.sharing_policy {
            ResourceSharingPolicy::Exclusive => {
                // Check if any other session has this resource
                if self.is_resource_allocated(&request.resource_type) {
                    self.handle_policy_violation(
                        &policy,
                        request.session_id,
                        "Exclusive resource already allocated".to_string(),
                        ViolationSeverity::Error,
                    ).await?;
                    
                    match policy.enforcement_level {
                        EnforcementLevel::Strict => return Ok(None),
                        _ => {
                            // Allow with warning
                            warn!("Allowing shared access to exclusive resource for session {}", request.session_id);
                        }
                    }
                }
            },
            ResourceSharingPolicy::Shared => {
                // Allow shared access
            },
            ResourceSharingPolicy::PriorityBased => {
                // Check if higher priority session needs the resource
                if self.has_higher_priority_request(&request.resource_type, request.priority) {
                    return Ok(None); // Defer to higher priority
                }
            },
            _ => {
                // Other policies can be implemented as needed
            }
        }
        
        // Allocate the resource
        self.allocate_resource(request, &policy).await
    }
    
    /// Allocate resource with default policy
    async fn allocate_resource_default(
        &self,
        request: ResourceRequest,
    ) -> Result<Option<ResourceAllocation>, Error> {
        let allocation = ResourceAllocation {
            allocation_id: uuid::Uuid::new_v4().to_string(),
            session_id: request.session_id,
            resource_type: request.resource_type.clone(),
            amount: request.amount,
            sharing_policy: ResourceSharingPolicy::Shared,
            allocated_at: SystemTime::now(),
            expires_at: None,
            metadata: request.metadata,
        };
        
        let allocation_id = allocation.allocation_id.clone();
        self.allocations.insert(allocation_id.clone(), allocation.clone());
        
        // Update session allocations
        self.session_allocations
            .entry(request.session_id)
            .or_insert_with(Vec::new)
            .push(allocation_id);
        
        // Update resource usage
        *self.resource_usage.entry(request.resource_type.clone()).or_insert(0) += request.amount;
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.total_resource_requests += 1;
            metrics.total_resource_allocations += 1;
        }
        
        info!("✅ Allocated {} units of {} to session {} (default policy)", 
            request.amount, request.resource_type, request.session_id);
        
        Ok(Some(allocation))
    }
    
    /// Allocate resource with specific policy
    async fn allocate_resource(
        &self,
        request: ResourceRequest,
        policy: &PolicyConfig,
    ) -> Result<Option<ResourceAllocation>, Error> {
        let allocation = ResourceAllocation {
            allocation_id: uuid::Uuid::new_v4().to_string(),
            session_id: request.session_id,
            resource_type: request.resource_type.clone(),
            amount: request.amount,
            sharing_policy: policy.sharing_policy.clone(),
            allocated_at: SystemTime::now(),
            expires_at: None,
            metadata: request.metadata,
        };
        
        let allocation_id = allocation.allocation_id.clone();
        self.allocations.insert(allocation_id.clone(), allocation.clone());
        
        // Update session allocations
        self.session_allocations
            .entry(request.session_id)
            .or_insert_with(Vec::new)
            .push(allocation_id);
        
        // Update resource usage
        *self.resource_usage.entry(request.resource_type.clone()).or_insert(0) += request.amount;
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.total_resource_requests += 1;
            metrics.total_resource_allocations += 1;
        }
        
        info!("✅ Allocated {} units of {} to session {} (policy: {})", 
            request.amount, request.resource_type, request.session_id, policy.policy_id);
        
        Ok(Some(allocation))
    }
    
    /// Release resource allocation
    pub async fn release_resource(&self, allocation_id: &str) -> Result<(), Error> {
        if let Some((_, allocation)) = self.allocations.remove(allocation_id) {
            // Update session allocations
            if let Some(mut session_allocs) = self.session_allocations.get_mut(&allocation.session_id) {
                session_allocs.retain(|id| id != allocation_id);
                if session_allocs.is_empty() {
                    drop(session_allocs);
                    self.session_allocations.remove(&allocation.session_id);
                }
            }
            
            // Update resource usage
            if let Some(mut usage) = self.resource_usage.get_mut(&allocation.resource_type) {
                *usage = usage.saturating_sub(allocation.amount);
            }
            
            info!("🗑️ Released allocation {} for session {}", allocation_id, allocation.session_id);
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Allocation {} not found", allocation_id),
                ErrorContext::default().with_message("Allocation not found")
            ))
        }
    }
    
    /// Find applicable policies for a session and resource
    async fn find_applicable_policies(
        &self,
        session_id: SessionId,
        resource_type: &ResourceType,
        session_state: SessionState,
        role: Option<&str>,
    ) -> Vec<PolicyConfig> {
        let mut applicable_policies = Vec::new();
        
        for entry in self.policies.iter() {
            let policy = entry.value();
            
            if policy.resource_type == *resource_type &&
               policy.applies_to_session(session_id, session_state, role) {
                applicable_policies.push(policy.clone());
            }
        }
        
        // Sort by priority (highest first)
        applicable_policies.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        applicable_policies
    }
    
    /// Check if a resource is currently allocated
    fn is_resource_allocated(&self, resource_type: &ResourceType) -> bool {
        self.resource_usage.get(resource_type)
            .map_or(false, |usage| *usage > 0)
    }
    
    /// Check if there's a higher priority request for a resource
    fn has_higher_priority_request(&self, _resource_type: &ResourceType, _priority: u32) -> bool {
        // This would require a request queue implementation
        // For now, return false
        false
    }
    
    /// Handle policy violation
    async fn handle_policy_violation(
        &self,
        policy: &PolicyConfig,
        session_id: SessionId,
        description: String,
        severity: ViolationSeverity,
    ) -> Result<(), Error> {
        let violation = PolicyViolation {
            violation_id: uuid::Uuid::new_v4().to_string(),
            policy_id: policy.policy_id.clone(),
            session_id,
            description: description.clone(),
            severity,
            occurred_at: SystemTime::now(),
            action_taken: None,
            metadata: HashMap::new(),
        };
        
        let violation_id = violation.violation_id.clone();
        self.violations.insert(violation_id, violation);
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.total_violations += 1;
            *metrics.violations_by_severity.entry(severity.to_string()).or_insert(0) += 1;
        }
        
        // Log based on severity
        match severity {
            ViolationSeverity::Info => info!("Policy violation: {}", description),
            ViolationSeverity::Warning => warn!("Policy violation: {}", description),
            ViolationSeverity::Error => error!("Policy violation: {}", description),
            ViolationSeverity::Critical => error!("CRITICAL policy violation: {}", description),
        }
        
        Ok(())
    }
    
    /// Handle session termination
    pub async fn handle_session_termination(&self, session_id: SessionId) -> Result<(), Error> {
        // Release all allocations for the session
        if let Some((_, allocation_ids)) = self.session_allocations.remove(&session_id) {
            for allocation_id in allocation_ids {
                if let Err(e) = self.release_resource(&allocation_id).await {
                    warn!("Failed to release allocation {} for terminated session {}: {}", 
                        allocation_id, session_id, e);
                }
            }
        }
        
        info!("✅ Completed policy cleanup for session {}", session_id);
        Ok(())
    }
    
    /// Get policy metrics
    pub async fn get_metrics(&self) -> PolicyMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Get active policies count
    pub async fn get_active_policies_count(&self) -> usize {
        self.policies.iter()
            .filter(|entry| entry.value().is_active())
            .count()
    }
    
    /// Get resource usage
    pub async fn get_resource_usage(&self) -> HashMap<ResourceType, u64> {
        self.resource_usage.iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect()
    }
    
    /// Get allocations for a session
    pub async fn get_session_allocations(&self, session_id: SessionId) -> Vec<ResourceAllocation> {
        let mut allocations = Vec::new();
        
        if let Some(allocation_ids) = self.session_allocations.get(&session_id) {
            for allocation_id in allocation_ids.iter() {
                if let Some(allocation) = self.allocations.get(allocation_id) {
                    allocations.push(allocation.value().clone());
                }
            }
        }
        
        allocations
    }
    
    /// Cleanup expired policies and violations
    pub async fn cleanup_expired(&self) -> Result<usize, Error> {
        let now = SystemTime::now();
        let mut cleanup_count = 0;
        
        // Cleanup expired policies
        let mut expired_policies = Vec::new();
        for entry in self.policies.iter() {
            if let Some(expires_at) = entry.value().expires_at {
                if now >= expires_at {
                    expired_policies.push(entry.key().clone());
                }
            }
        }
        
        for policy_id in expired_policies {
            self.remove_policy(&policy_id).await?;
            cleanup_count += 1;
        }
        
        // Cleanup old violations if over limit
        if self.violations.len() > self.config.max_violations {
            let excess = self.violations.len() - self.config.max_violations;
            let mut violation_ids: Vec<_> = self.violations.iter()
                .map(|entry| (entry.key().clone(), entry.value().occurred_at))
                .collect();
            
            // Sort by oldest first
            violation_ids.sort_by(|a, b| a.1.cmp(&b.1));
            
            for (violation_id, _) in violation_ids.into_iter().take(excess) {
                self.violations.remove(&violation_id);
                cleanup_count += 1;
            }
        }
        
        if cleanup_count > 0 {
            info!("🧹 Cleaned up {} expired policies and violations", cleanup_count);
        }
        
        Ok(cleanup_count)
    }
} 