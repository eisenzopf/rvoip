//! Multi-Session Bridge Infrastructure
//!
//! This module provides the core data structures and types for bridging multiple
//! SIP sessions together for audio routing and conferencing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use uuid::Uuid;
use serde::{Serialize, Deserialize};

use crate::session::SessionId;

/// Unique identifier for a bridge
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BridgeId(pub Uuid);

impl BridgeId {
    /// Create a new bridge ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for BridgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bridge-{}", self.0)
    }
}

impl From<Uuid> for BridgeId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

/// State of a bridge
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeState {
    /// Bridge is being created
    Creating,
    /// Bridge is active and routing audio
    Active,
    /// Bridge is paused (no audio routing)
    Paused,
    /// Bridge is being destroyed
    Destroying,
    /// Bridge has been destroyed
    Destroyed,
}

impl std::fmt::Display for BridgeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Creating => write!(f, "creating"),
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Destroying => write!(f, "destroying"),
            Self::Destroyed => write!(f, "destroyed"),
        }
    }
}

/// Configuration for creating a bridge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    /// Maximum number of sessions allowed in the bridge
    pub max_sessions: usize,
    /// Optional name for the bridge
    pub name: Option<String>,
    /// Bridge timeout (auto-destroy after this duration if empty)
    pub timeout_secs: Option<u64>,
    /// Enable audio mixing (vs point-to-point)
    pub enable_mixing: bool,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            max_sessions: 10,
            name: None,
            timeout_secs: Some(300), // 5 minutes
            enable_mixing: true,
        }
    }
}

/// Bridge statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeStats {
    /// Number of sessions currently in the bridge
    pub session_count: usize,
    /// Total bytes of audio processed
    pub total_bytes: u64,
    /// Number of packets processed
    pub packet_count: u64,
    /// Bridge uptime
    pub uptime_secs: u64,
    /// Last activity timestamp
    pub last_activity: Option<SystemTime>,
}

impl Default for BridgeStats {
    fn default() -> Self {
        Self {
            session_count: 0,
            total_bytes: 0,
            packet_count: 0,
            uptime_secs: 0,
            last_activity: None,
        }
    }
}

/// Information about a bridge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeInfo {
    /// Bridge ID
    pub id: BridgeId,
    /// Bridge name (if any)
    pub name: Option<String>,
    /// Current state
    pub state: BridgeState,
    /// Configuration
    pub config: BridgeConfig,
    /// List of session IDs in the bridge
    pub sessions: Vec<SessionId>,
    /// Creation timestamp
    pub created_at: SystemTime,
    /// Statistics
    pub stats: BridgeStats,
}

/// Bridge event types for notifications
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BridgeEventType {
    /// Bridge was created
    BridgeCreated,
    /// Bridge was destroyed
    BridgeDestroyed,
    /// Session was added to bridge
    SessionAdded,
    /// Session was removed from bridge
    SessionRemoved,
    /// Bridge state changed
    StateChanged,
    /// Bridge configuration changed
    ConfigChanged,
}

/// Bridge event for notifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeEvent {
    /// Type of event
    pub event_type: BridgeEventType,
    /// Bridge ID
    pub bridge_id: BridgeId,
    /// Session ID (if applicable)
    pub session_id: Option<SessionId>,
    /// Event timestamp
    pub timestamp: SystemTime,
    /// Additional event data
    pub data: HashMap<String, String>,
}

/// Errors that can occur during bridge operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeError {
    /// Bridge not found
    BridgeNotFound { bridge_id: BridgeId },
    /// Bridge is full (max sessions reached)
    BridgeFull { bridge_id: BridgeId, max_sessions: usize },
    /// Invalid bridge state for operation
    InvalidState { bridge_id: BridgeId, state: BridgeState },
    /// Session not found in bridge
    SessionNotInBridge { bridge_id: BridgeId, session_id: SessionId },
    /// Internal error
    Internal { message: String },
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BridgeNotFound { bridge_id } => {
                write!(f, "Bridge {} not found", bridge_id)
            },
            Self::BridgeFull { bridge_id, max_sessions } => {
                write!(f, "Bridge {} is full (max {} sessions)", bridge_id, max_sessions)
            },
            Self::InvalidState { bridge_id, state } => {
                write!(f, "Bridge {} is in invalid state: {}", bridge_id, state)
            },
            Self::SessionNotInBridge { bridge_id, session_id } => {
                write!(f, "Session {} not found in bridge {}", session_id, bridge_id)
            },
            Self::Internal { message } => {
                write!(f, "Bridge internal error: {}", message)
            },
        }
    }
}

impl std::error::Error for BridgeError {}

/// A bridge that connects multiple sessions for audio routing
pub struct SessionBridge {
    /// Bridge ID
    pub id: BridgeId,
    /// Bridge configuration
    pub config: BridgeConfig,
    /// Current state
    state: RwLock<BridgeState>,
    /// Sessions in the bridge
    sessions: RwLock<Vec<SessionId>>,
    /// Creation timestamp
    pub created_at: SystemTime,
    /// Statistics
    stats: RwLock<BridgeStats>,
}

impl SessionBridge {
    /// Create a new session bridge
    pub fn new(config: BridgeConfig) -> Self {
        Self {
            id: BridgeId::new(),
            config,
            state: RwLock::new(BridgeState::Creating),
            sessions: RwLock::new(Vec::new()),
            created_at: SystemTime::now(),
            stats: RwLock::new(BridgeStats::default()),
        }
    }
    
    /// Get the current state of the bridge
    pub async fn get_state(&self) -> BridgeState {
        self.state.read().await.clone()
    }
    
    /// Set the bridge state
    pub async fn set_state(&self, new_state: BridgeState) {
        let mut state = self.state.write().await;
        *state = new_state;
    }
    
    /// Add a session to the bridge
    pub async fn add_session(&self, session_id: SessionId) -> Result<(), BridgeError> {
        let mut sessions = self.sessions.write().await;
        
        // Check if bridge is full
        if sessions.len() >= self.config.max_sessions {
            return Err(BridgeError::BridgeFull {
                bridge_id: self.id.clone(),
                max_sessions: self.config.max_sessions,
            });
        }
        
        // Check if session is already in bridge
        if sessions.contains(&session_id) {
            return Ok(()); // Already in bridge, no-op
        }
        
        // Add the session
        sessions.push(session_id);
        
        // Update statistics
        let mut stats = self.stats.write().await;
        stats.session_count = sessions.len();
        stats.last_activity = Some(SystemTime::now());
        
        Ok(())
    }
    
    /// Remove a session from the bridge
    pub async fn remove_session(&self, session_id: &SessionId) -> Result<(), BridgeError> {
        let mut sessions = self.sessions.write().await;
        
        // Find and remove the session
        if let Some(pos) = sessions.iter().position(|s| s == session_id) {
            sessions.remove(pos);
            
            // Update statistics
            let mut stats = self.stats.write().await;
            stats.session_count = sessions.len();
            stats.last_activity = Some(SystemTime::now());
            
            Ok(())
        } else {
            Err(BridgeError::SessionNotInBridge {
                bridge_id: self.id.clone(),
                session_id: session_id.clone(),
            })
        }
    }
    
    /// Get list of session IDs in the bridge
    pub async fn get_session_ids(&self) -> Vec<SessionId> {
        self.sessions.read().await.clone()
    }
    
    /// Get bridge information
    pub async fn get_info(&self) -> BridgeInfo {
        let state = self.state.read().await.clone();
        let sessions = self.sessions.read().await.clone();
        let stats = self.stats.read().await.clone();
        
        BridgeInfo {
            id: self.id.clone(),
            name: self.config.name.clone(),
            state,
            config: self.config.clone(),
            sessions,
            created_at: self.created_at,
            stats,
        }
    }
} 