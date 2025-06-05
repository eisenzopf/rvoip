//! Basic Resource Tracking Primitives
//! 
//! This module provides low-level resource tracking data structures and basic operations
//! for session resource coordination. Complex business logic and policy enforcement 
//! is handled by higher layers (call-engine).
//! 
//! ## Scope
//! 
//! **✅ Included (Basic Primitives)**:
//! - Basic resource type definitions
//! - Simple resource usage tracking data structures
//! - Basic resource allocation tracking (without enforcement)
//! - Core resource limit configuration
//! 
//! **❌ Not Included (Business Logic - belongs in call-engine)**:
//! - Policy enforcement and violation detection
//! - Complex resource allocation algorithms
//! - Resource sharing strategies and business rules
//! - Advanced coordination and priority management

use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use serde::{Serialize, Deserialize};

use crate::session::SessionId;
use crate::errors::{Error, ErrorContext};

/// Resource types that can be tracked (basic classification)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BasicResourceType {
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

impl std::fmt::Display for BasicResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BasicResourceType::Bandwidth => write!(f, "Bandwidth"),
            BasicResourceType::CPU => write!(f, "CPU"),
            BasicResourceType::Memory => write!(f, "Memory"),
            BasicResourceType::MediaBridge => write!(f, "MediaBridge"),
            BasicResourceType::NetworkConnection => write!(f, "NetworkConnection"),
            BasicResourceType::Storage => write!(f, "Storage"),
            BasicResourceType::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

/// Basic resource allocation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicResourceAllocation {
    /// Allocation ID
    pub allocation_id: String,
    
    /// Session that received the allocation
    pub session_id: SessionId,
    
    /// Resource type allocated
    pub resource_type: BasicResourceType,
    
    /// Amount allocated
    pub amount: u64,
    
    /// When the allocation was made
    pub allocated_at: SystemTime,
    
    /// When the allocation expires (if any)
    pub expires_at: Option<SystemTime>,
    
    /// Allocation metadata
    pub metadata: HashMap<String, String>,
}

impl BasicResourceAllocation {
    /// Create a new basic resource allocation
    pub fn new(
        allocation_id: String,
        session_id: SessionId,
        resource_type: BasicResourceType,
        amount: u64,
    ) -> Self {
        Self {
            allocation_id,
            session_id,
            resource_type,
            amount,
            allocated_at: SystemTime::now(),
            expires_at: None,
            metadata: HashMap::new(),
        }
    }
    
    /// Check if allocation has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            SystemTime::now() >= expires_at
        } else {
            false
        }
    }
    
    /// Add metadata
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }
}

/// Basic resource usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicResourceUsage {
    /// Resource type
    pub resource_type: BasicResourceType,
    
    /// Total amount available
    pub total_available: u64,
    
    /// Amount currently used
    pub current_used: u64,
    
    /// Number of active allocations
    pub allocation_count: usize,
    
    /// When usage was last updated
    pub last_updated: SystemTime,
}

impl BasicResourceUsage {
    /// Create new resource usage tracking
    pub fn new(resource_type: BasicResourceType, total_available: u64) -> Self {
        Self {
            resource_type,
            total_available,
            current_used: 0,
            allocation_count: 0,
            last_updated: SystemTime::now(),
        }
    }
    
    /// Calculate usage percentage
    pub fn usage_percentage(&self) -> f64 {
        if self.total_available == 0 {
            0.0
        } else {
            (self.current_used as f64 / self.total_available as f64) * 100.0
        }
    }
    
    /// Check if resource is available
    pub fn is_available(&self, amount: u64) -> bool {
        self.current_used + amount <= self.total_available
    }
    
    /// Update usage (basic operation)
    pub fn update_usage(&mut self, used: u64, allocation_count: usize) {
        self.current_used = used;
        self.allocation_count = allocation_count;
        self.last_updated = SystemTime::now();
    }
}

/// Basic resource limits configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicResourceLimits {
    /// Per-session resource limits
    pub per_session_limits: HashMap<BasicResourceType, u64>,
    
    /// Global resource limits
    pub global_limits: HashMap<BasicResourceType, u64>,
    
    /// Maximum allocations per resource type
    pub max_allocations: HashMap<BasicResourceType, usize>,
    
    /// Default allocation timeout
    pub default_timeout: Duration,
    
    /// Configuration metadata
    pub metadata: HashMap<String, String>,
}

impl Default for BasicResourceLimits {
    fn default() -> Self {
        let mut per_session_limits = HashMap::new();
        per_session_limits.insert(BasicResourceType::Bandwidth, 1000000); // 1MB
        per_session_limits.insert(BasicResourceType::CPU, 25); // 25%
        per_session_limits.insert(BasicResourceType::Memory, 100 * 1024 * 1024); // 100MB
        
        let mut global_limits = HashMap::new();
        global_limits.insert(BasicResourceType::Bandwidth, 100000000); // 100MB
        global_limits.insert(BasicResourceType::CPU, 80); // 80%
        global_limits.insert(BasicResourceType::Memory, 8 * 1024 * 1024 * 1024); // 8GB
        
        let mut max_allocations = HashMap::new();
        max_allocations.insert(BasicResourceType::Bandwidth, 1000);
        max_allocations.insert(BasicResourceType::CPU, 100);
        max_allocations.insert(BasicResourceType::Memory, 500);
        
        Self {
            per_session_limits,
            global_limits,
            max_allocations,
            default_timeout: Duration::from_secs(300), // 5 minutes
            metadata: HashMap::new(),
        }
    }
}

impl BasicResourceLimits {
    /// Get per-session limit for a resource type
    pub fn get_session_limit(&self, resource_type: &BasicResourceType) -> Option<u64> {
        self.per_session_limits.get(resource_type).copied()
    }
    
    /// Get global limit for a resource type
    pub fn get_global_limit(&self, resource_type: &BasicResourceType) -> Option<u64> {
        self.global_limits.get(resource_type).copied()
    }
    
    /// Get maximum allocations for a resource type
    pub fn get_max_allocations(&self, resource_type: &BasicResourceType) -> Option<usize> {
        self.max_allocations.get(resource_type).copied()
    }
    
    /// Check if allocation is within session limits
    pub fn is_within_session_limit(&self, resource_type: &BasicResourceType, amount: u64) -> bool {
        if let Some(limit) = self.get_session_limit(resource_type) {
            amount <= limit
        } else {
            true // No limit configured
        }
    }
    
    /// Update session limit
    pub fn set_session_limit(&mut self, resource_type: BasicResourceType, limit: u64) {
        self.per_session_limits.insert(resource_type, limit);
    }
    
    /// Update global limit
    pub fn set_global_limit(&mut self, resource_type: BasicResourceType, limit: u64) {
        self.global_limits.insert(resource_type, limit);
    }
}

/// Basic resource request (data structure only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicResourceRequest {
    /// Requesting session
    pub session_id: SessionId,
    
    /// Resource type requested
    pub resource_type: BasicResourceType,
    
    /// Amount requested
    pub amount: u64,
    
    /// Maximum wait time
    pub max_wait_time: Option<Duration>,
    
    /// Request metadata
    pub metadata: HashMap<String, String>,
    
    /// When the request was made
    pub requested_at: SystemTime,
}

impl BasicResourceRequest {
    /// Create a new basic resource request
    pub fn new(
        session_id: SessionId,
        resource_type: BasicResourceType,
        amount: u64,
    ) -> Self {
        Self {
            session_id,
            resource_type,
            amount,
            max_wait_time: None,
            metadata: HashMap::new(),
            requested_at: SystemTime::now(),
        }
    }
    
    /// Check if request has timed out
    pub fn is_timed_out(&self) -> bool {
        if let Some(max_wait) = self.max_wait_time {
            self.requested_at.elapsed().unwrap_or(Duration::ZERO) >= max_wait
        } else {
            false
        }
    }
    
    /// Add metadata
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }
}

/// Basic resource tracking statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicResourceStats {
    /// Total resource requests
    pub total_requests: u64,
    
    /// Total allocations made
    pub total_allocations: u64,
    
    /// Current active allocations
    pub active_allocations: u64,
    
    /// Resource usage by type
    pub usage_by_type: HashMap<BasicResourceType, BasicResourceUsage>,
    
    /// When stats were last updated
    pub last_updated: SystemTime,
}

impl Default for BasicResourceStats {
    fn default() -> Self {
        Self::new()
    }
}

impl BasicResourceStats {
    /// Create new resource statistics
    pub fn new() -> Self {
        Self {
            total_requests: 0,
            total_allocations: 0,
            active_allocations: 0,
            usage_by_type: HashMap::new(),
            last_updated: SystemTime::now(),
        }
    }
    
    /// Update request count
    pub fn increment_requests(&mut self) {
        self.total_requests += 1;
        self.last_updated = SystemTime::now();
    }
    
    /// Update allocation count
    pub fn increment_allocations(&mut self) {
        self.total_allocations += 1;
        self.active_allocations += 1;
        self.last_updated = SystemTime::now();
    }
    
    /// Update deallocation count
    pub fn decrement_allocations(&mut self) {
        if self.active_allocations > 0 {
            self.active_allocations -= 1;
        }
        self.last_updated = SystemTime::now();
    }
    
    /// Get usage for a resource type
    pub fn get_usage(&self, resource_type: &BasicResourceType) -> Option<&BasicResourceUsage> {
        self.usage_by_type.get(resource_type)
    }
    
    /// Update usage for a resource type
    pub fn update_usage(&mut self, resource_type: BasicResourceType, usage: BasicResourceUsage) {
        self.usage_by_type.insert(resource_type, usage);
        self.last_updated = SystemTime::now();
    }
} 