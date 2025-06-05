//! Basic Session Grouping Primitives
//! 
//! This module provides low-level session grouping data structures and basic operations
//! for session coordination. Business logic and advanced group management is handled
//! by higher layers (call-engine).
//! 
//! ## Scope
//! 
//! **✅ Included (Basic Primitives)**:
//! - Basic data structures for grouping sessions
//! - Simple group membership tracking
//! - Basic group state management
//! - Core group operations (add/remove sessions)
//! 
//! **❌ Not Included (Business Logic - belongs in call-engine)**:
//! - Group lifecycle management and orchestration
//! - Complex group policies and leader election
//! - Group metrics and advanced coordination
//! - Bridge integration and business rules

use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use crate::session::SessionId;
use crate::errors::{Error, ErrorContext};

/// Types of session groups (basic classification)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BasicGroupType {
    /// Conference group (multiple participants)
    Conference,
    
    /// Transfer group (source, target, consultation sessions)
    Transfer,
    
    /// Bridge group (sessions connected via media bridge)
    Bridge,
    
    /// Consultation group (main call + consultation call)
    Consultation,
    
    /// Custom group with user-defined behavior
    Custom,
}

impl std::fmt::Display for BasicGroupType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BasicGroupType::Conference => write!(f, "Conference"),
            BasicGroupType::Transfer => write!(f, "Transfer"),
            BasicGroupType::Bridge => write!(f, "Bridge"),
            BasicGroupType::Consultation => write!(f, "Consultation"),
            BasicGroupType::Custom => write!(f, "Custom"),
        }
    }
}

/// State of a session group (basic states)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BasicGroupState {
    /// Group is being initialized
    Initializing,
    
    /// Group is active and coordinating sessions
    Active,
    
    /// Group is being terminated
    Terminating,
    
    /// Group has been terminated
    Terminated,
}

impl std::fmt::Display for BasicGroupState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BasicGroupState::Initializing => write!(f, "Initializing"),
            BasicGroupState::Active => write!(f, "Active"),
            BasicGroupState::Terminating => write!(f, "Terminating"),
            BasicGroupState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Basic configuration for session groups
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicGroupConfig {
    /// Maximum number of sessions in the group
    pub max_sessions: Option<usize>,
    
    /// Group-specific metadata
    pub metadata: HashMap<String, String>,
}

impl Default for BasicGroupConfig {
    fn default() -> Self {
        Self {
            max_sessions: Some(100),
            metadata: HashMap::new(),
        }
    }
}

/// Basic session membership information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicSessionMembership {
    /// Session ID
    pub session_id: SessionId,
    
    /// Role of the session in the group
    pub role: String,
    
    /// When the session joined the group
    pub joined_at: SystemTime,
    
    /// Whether the session is active in the group
    pub active: bool,
    
    /// Session-specific metadata within the group
    pub metadata: HashMap<String, String>,
}

impl BasicSessionMembership {
    /// Create a new session membership
    pub fn new(session_id: SessionId, role: String) -> Self {
        Self {
            session_id,
            role,
            joined_at: SystemTime::now(),
            active: true,
            metadata: HashMap::new(),
        }
    }
    
    /// Set session as active/inactive
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }
    
    /// Add metadata
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }
}

/// Basic group event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BasicGroupEvent {
    /// Group was created
    GroupCreated {
        group_id: String,
        group_type: BasicGroupType,
    },
    
    /// Session joined the group
    SessionJoined {
        group_id: String,
        session_id: SessionId,
        role: String,
    },
    
    /// Session left the group
    SessionLeft {
        group_id: String,
        session_id: SessionId,
        reason: String,
    },
    
    /// Group state changed
    StateChanged {
        group_id: String,
        old_state: BasicGroupState,
        new_state: BasicGroupState,
    },
    
    /// Group was terminated
    GroupTerminated {
        group_id: String,
        reason: String,
    },
}

/// Basic session group (data structure only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicSessionGroup {
    /// Unique group identifier
    pub id: String,
    
    /// Type of group
    pub group_type: BasicGroupType,
    
    /// Current state of the group
    pub state: BasicGroupState,
    
    /// Group configuration
    pub config: BasicGroupConfig,
    
    /// Sessions in the group
    pub members: HashMap<SessionId, BasicSessionMembership>,
    
    /// When the group was created
    pub created_at: SystemTime,
    
    /// When the group was last updated
    pub updated_at: SystemTime,
    
    /// Group-wide metadata
    pub metadata: HashMap<String, String>,
}

impl BasicSessionGroup {
    /// Create a new basic session group
    pub fn new(group_type: BasicGroupType, config: BasicGroupConfig) -> Self {
        let now = SystemTime::now();
        Self {
            id: Uuid::new_v4().to_string(),
            group_type,
            state: BasicGroupState::Initializing,
            config,
            members: HashMap::new(),
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
        }
    }
    
    /// Add a session to the group (basic operation)
    pub fn add_session(&mut self, session_id: SessionId, role: String) -> Result<(), Error> {
        // Basic capacity check
        if let Some(max) = self.config.max_sessions {
            if self.members.len() >= max {
                return Err(Error::InternalError(
                    format!("Group {} has reached maximum capacity of {}", self.id, max),
                    ErrorContext::default().with_message("Group capacity exceeded")
                ));
            }
        }
        
        // Check if session is already in the group
        if self.members.contains_key(&session_id) {
            return Err(Error::InternalError(
                format!("Session {} is already in group {}", session_id, self.id),
                ErrorContext::default().with_message("Duplicate session membership")
            ));
        }
        
        // Add the session
        let membership = BasicSessionMembership::new(session_id, role);
        self.members.insert(session_id, membership);
        self.updated_at = SystemTime::now();
        
        Ok(())
    }
    
    /// Remove a session from the group (basic operation)
    pub fn remove_session(&mut self, session_id: SessionId) -> Result<(), Error> {
        if self.members.remove(&session_id).is_some() {
            self.updated_at = SystemTime::now();
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Session {} not found in group {}", session_id, self.id),
                ErrorContext::default().with_message("Session not in group")
            ))
        }
    }
    
    /// Update group state (basic operation)
    pub fn update_state(&mut self, new_state: BasicGroupState) {
        if self.state != new_state {
            self.state = new_state;
            self.updated_at = SystemTime::now();
        }
    }
    
    /// Get active session count
    pub fn get_active_session_count(&self) -> usize {
        self.members.values()
            .filter(|m| m.active)
            .count()
    }
    
    /// Get all session IDs in the group
    pub fn get_session_ids(&self) -> Vec<SessionId> {
        self.members.keys().copied().collect()
    }
    
    /// Get active session IDs in the group
    pub fn get_active_session_ids(&self) -> Vec<SessionId> {
        self.members.iter()
            .filter(|(_, m)| m.active)
            .map(|(id, _)| *id)
            .collect()
    }
    
    /// Check if the group contains a session
    pub fn contains_session(&self, session_id: SessionId) -> bool {
        self.members.contains_key(&session_id)
    }
    
    /// Get session role in the group
    pub fn get_session_role(&self, session_id: SessionId) -> Option<String> {
        self.members.get(&session_id).map(|m| m.role.clone())
    }
    
    /// Check if the group is active
    pub fn is_active(&self) -> bool {
        self.state == BasicGroupState::Active
    }
    
    /// Check if the group is terminal
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, BasicGroupState::Terminated)
    }
    
    /// Add metadata to the group
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
        self.updated_at = SystemTime::now();
    }
} 