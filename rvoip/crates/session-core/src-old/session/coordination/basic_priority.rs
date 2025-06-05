//! Basic Priority Primitives
//! 
//! This module provides low-level priority classification data structures and basic operations
//! for session priority coordination. Complex business logic and sophisticated scheduling 
//! is handled by higher layers (call-engine).
//! 
//! ## Scope
//! 
//! **✅ Included (Basic Primitives)**:
//! - Basic session priority levels
//! - Simple priority class definitions
//! - Basic QoS level classification
//! - Core priority information structures
//! 
//! **❌ Not Included (Business Logic - belongs in call-engine)**:
//! - Complex scheduling policies and algorithms
//! - Resource allocation and enforcement
//! - Priority-based task management and queuing
//! - Advanced metrics and usage tracking

use std::collections::HashMap;
use std::time::SystemTime;
use serde::{Serialize, Deserialize};

use crate::session::SessionId;

/// Session priority levels (basic classification)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BasicSessionPriority {
    /// Emergency calls (911, etc.)
    Emergency = 100,
    
    /// Critical business calls
    Critical = 90,
    
    /// High priority calls
    High = 70,
    
    /// Normal priority calls
    Normal = 50,
    
    /// Low priority calls
    Low = 30,
    
    /// Background/maintenance calls
    Background = 10,
}

impl std::fmt::Display for BasicSessionPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BasicSessionPriority::Emergency => write!(f, "Emergency"),
            BasicSessionPriority::Critical => write!(f, "Critical"),
            BasicSessionPriority::High => write!(f, "High"),
            BasicSessionPriority::Normal => write!(f, "Normal"),
            BasicSessionPriority::Low => write!(f, "Low"),
            BasicSessionPriority::Background => write!(f, "Background"),
        }
    }
}

impl Default for BasicSessionPriority {
    fn default() -> Self {
        BasicSessionPriority::Normal
    }
}

impl BasicSessionPriority {
    /// Get priority as numeric value
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }
    
    /// Create priority from numeric value
    pub fn from_u8(value: u8) -> Self {
        match value {
            100 => BasicSessionPriority::Emergency,
            90 => BasicSessionPriority::Critical,
            70 => BasicSessionPriority::High,
            50 => BasicSessionPriority::Normal,
            30 => BasicSessionPriority::Low,
            10 => BasicSessionPriority::Background,
            _ => BasicSessionPriority::Normal, // Default for unknown values
        }
    }
    
    /// Check if this priority is higher than another
    pub fn is_higher_than(&self, other: &Self) -> bool {
        self > other
    }
    
    /// Check if this is an emergency priority
    pub fn is_emergency(&self) -> bool {
        matches!(self, BasicSessionPriority::Emergency)
    }
    
    /// Check if this is a critical or emergency priority
    pub fn is_critical_or_above(&self) -> bool {
        matches!(self, BasicSessionPriority::Emergency | BasicSessionPriority::Critical)
    }
}

/// Priority class for grouping sessions (basic classification)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BasicPriorityClass {
    /// Real-time communications
    RealTime,
    
    /// Interactive communications
    Interactive,
    
    /// Bulk data transfer
    Bulk,
    
    /// Best effort
    BestEffort,
    
    /// Custom priority class
    Custom(String),
}

impl std::fmt::Display for BasicPriorityClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BasicPriorityClass::RealTime => write!(f, "RealTime"),
            BasicPriorityClass::Interactive => write!(f, "Interactive"),
            BasicPriorityClass::Bulk => write!(f, "Bulk"),
            BasicPriorityClass::BestEffort => write!(f, "BestEffort"),
            BasicPriorityClass::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

impl Default for BasicPriorityClass {
    fn default() -> Self {
        BasicPriorityClass::Interactive
    }
}

impl BasicPriorityClass {
    /// Check if this is a real-time class
    pub fn is_realtime(&self) -> bool {
        matches!(self, BasicPriorityClass::RealTime)
    }
    
    /// Check if this is an interactive class
    pub fn is_interactive(&self) -> bool {
        matches!(self, BasicPriorityClass::Interactive | BasicPriorityClass::RealTime)
    }
    
    /// Get expected latency requirements (basic classification)
    pub fn expected_latency_ms(&self) -> u32 {
        match self {
            BasicPriorityClass::RealTime => 10,      // Very low latency
            BasicPriorityClass::Interactive => 100,  // Low latency
            BasicPriorityClass::Bulk => 1000,        // Moderate latency
            BasicPriorityClass::BestEffort => 5000,  // High latency acceptable
            BasicPriorityClass::Custom(_) => 500,    // Default for custom
        }
    }
}

/// Quality of Service levels (basic classification)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BasicQoSLevel {
    /// Best effort
    BestEffort,
    
    /// Assured forwarding
    AssuredForwarding,
    
    /// Expedited forwarding
    ExpeditedForwarding,
    
    /// Voice service
    Voice,
    
    /// Video service
    Video,
}

impl std::fmt::Display for BasicQoSLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BasicQoSLevel::BestEffort => write!(f, "BestEffort"),
            BasicQoSLevel::AssuredForwarding => write!(f, "AssuredForwarding"),
            BasicQoSLevel::ExpeditedForwarding => write!(f, "ExpeditedForwarding"),
            BasicQoSLevel::Voice => write!(f, "Voice"),
            BasicQoSLevel::Video => write!(f, "Video"),
        }
    }
}

impl Default for BasicQoSLevel {
    fn default() -> Self {
        BasicQoSLevel::BestEffort
    }
}

impl BasicQoSLevel {
    /// Check if this QoS level requires real-time handling
    pub fn is_realtime(&self) -> bool {
        matches!(self, BasicQoSLevel::Voice | BasicQoSLevel::Video | BasicQoSLevel::ExpeditedForwarding)
    }
    
    /// Get priority score for this QoS level
    pub fn priority_score(&self) -> u8 {
        match self {
            BasicQoSLevel::Voice => 90,
            BasicQoSLevel::Video => 80,
            BasicQoSLevel::ExpeditedForwarding => 70,
            BasicQoSLevel::AssuredForwarding => 50,
            BasicQoSLevel::BestEffort => 10,
        }
    }
}

/// Basic priority information for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicPriorityInfo {
    /// Session ID
    pub session_id: SessionId,
    
    /// Priority level
    pub priority: BasicSessionPriority,
    
    /// Priority class
    pub priority_class: BasicPriorityClass,
    
    /// QoS level
    pub qos_level: BasicQoSLevel,
    
    /// When priority was assigned
    pub assigned_at: SystemTime,
    
    /// Priority expiration (if any)
    pub expires_at: Option<SystemTime>,
    
    /// Custom priority metadata
    pub metadata: HashMap<String, String>,
}

impl BasicPriorityInfo {
    /// Create new basic priority info
    pub fn new(
        session_id: SessionId,
        priority: BasicSessionPriority,
        priority_class: BasicPriorityClass,
        qos_level: BasicQoSLevel,
    ) -> Self {
        Self {
            session_id,
            priority,
            priority_class,
            qos_level,
            assigned_at: SystemTime::now(),
            expires_at: None,
            metadata: HashMap::new(),
        }
    }
    
    /// Check if priority has expired
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
    
    /// Get overall priority score (combination of priority level and QoS)
    pub fn overall_score(&self) -> u8 {
        // Combine priority level and QoS level scores
        let priority_weight = 0.7;
        let qos_weight = 0.3;
        
        let priority_score = self.priority.as_u8() as f32;
        let qos_score = self.qos_level.priority_score() as f32;
        
        ((priority_score * priority_weight) + (qos_score * qos_weight)) as u8
    }
    
    /// Check if this session should have precedence over another
    pub fn has_precedence_over(&self, other: &BasicPriorityInfo) -> bool {
        // First compare priority levels
        if self.priority != other.priority {
            return self.priority > other.priority;
        }
        
        // If priorities are equal, compare QoS levels
        if self.qos_level != other.qos_level {
            return self.qos_level.priority_score() > other.qos_level.priority_score();
        }
        
        // If both are equal, check assignment time (earlier assignments have precedence)
        self.assigned_at < other.assigned_at
    }
    
    /// Check if this is a high-priority session
    pub fn is_high_priority(&self) -> bool {
        self.priority.is_critical_or_above() || self.qos_level.is_realtime()
    }
}

/// Basic priority configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicPriorityConfig {
    /// Default priority for new sessions
    pub default_priority: BasicSessionPriority,
    
    /// Default priority class for new sessions
    pub default_class: BasicPriorityClass,
    
    /// Default QoS level for new sessions
    pub default_qos: BasicQoSLevel,
    
    /// Whether to automatically expire priorities
    pub auto_expire: bool,
    
    /// Default expiration time for priorities
    pub default_expiration: Option<SystemTime>,
    
    /// Configuration metadata
    pub metadata: HashMap<String, String>,
}

impl Default for BasicPriorityConfig {
    fn default() -> Self {
        Self {
            default_priority: BasicSessionPriority::Normal,
            default_class: BasicPriorityClass::Interactive,
            default_qos: BasicQoSLevel::BestEffort,
            auto_expire: false,
            default_expiration: None,
            metadata: HashMap::new(),
        }
    }
}

impl BasicPriorityConfig {
    /// Create priority info with default settings
    pub fn create_default_priority(&self, session_id: SessionId) -> BasicPriorityInfo {
        BasicPriorityInfo::new(
            session_id,
            self.default_priority,
            self.default_class.clone(),
            self.default_qos,
        )
    }
    
    /// Check if a priority level is allowed
    pub fn is_priority_allowed(&self, _priority: BasicSessionPriority) -> bool {
        // Basic implementation - always allow (call-engine handles restrictions)
        true
    }
} 