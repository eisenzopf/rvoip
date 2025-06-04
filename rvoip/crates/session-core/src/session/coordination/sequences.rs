//! Session Sequence Coordination for A-leg/B-leg relationships

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use dashmap::DashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorContext};

/// Types of session sequences
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SequenceType {
    /// A-leg/B-leg call sequence
    ABLeg,
    /// Call forwarding sequence  
    Forwarding,
    /// Hunt group sequence
    Hunt,
    /// Custom sequence
    Custom,
}

/// State of a session sequence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequenceState {
    /// Initializing
    Initializing,
    /// Active
    Active,
    /// Completed
    Completed,
    /// Failed
    Failed,
}

/// Session sequence coordinator
pub struct SessionSequenceCoordinator {
    sequences: Arc<DashMap<String, SessionSequence>>,
}

/// Session sequence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSequence {
    pub id: String,
    pub sequence_type: SequenceType,
    pub state: SequenceState,
    pub sessions: Vec<SessionId>,
    pub config: SequenceConfig,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub completed_at: Option<SystemTime>,
    pub metadata: HashMap<String, String>,
    pub statistics: SequenceStatistics,
}

impl SessionSequenceCoordinator {
    pub fn new() -> Self {
        Self {
            sequences: Arc::new(DashMap::new()),
        }
    }
    
    pub async fn create_sequence(&self, sequence_type: SequenceType) -> Result<String, Error> {
        let sequence = SessionSequence {
            id: Uuid::new_v4().to_string(),
            sequence_type,
            state: SequenceState::Initializing,
            sessions: Vec::new(),
            config: SequenceConfig::default(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            started_at: None,
            completed_at: None,
            metadata: HashMap::new(),
            statistics: SequenceStatistics::default(),
        };
        
        let sequence_id = sequence.id.clone();
        self.sequences.insert(sequence_id.clone(), sequence);
        
        Ok(sequence_id)
    }
}

impl SessionSequence {
    /// Create a new session sequence
    pub fn new(sequence_type: SequenceType, config: SequenceConfig) -> Self {
        let now = SystemTime::now();
        Self {
            id: Uuid::new_v4().to_string(),
            sequence_type,
            state: SequenceState::Initializing,
            sessions: Vec::new(),
            config,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            metadata: HashMap::new(),
            statistics: SequenceStatistics::default(),
        }
    }

    /// Add missing fields to complete the struct
    pub fn add_missing_fields(&mut self) {
        // This method ensures all fields are properly initialized
        if self.sessions.is_empty() {
            self.sessions = Vec::new();
        }
    }
}

/// Session sequence configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceConfig {
    /// Maximum number of sessions in sequence
    pub max_sessions: usize,
}

impl Default for SequenceConfig {
    fn default() -> Self {
        Self {
            max_sessions: 10,
        }
    }
}

/// Sequence statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SequenceStatistics {
    /// Total sessions processed
    pub total_sessions: usize,
    /// Success rate
    pub success_rate: f64,
}

/// Coordinator configuration  
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// Maximum concurrent sequences
    pub max_concurrent_sequences: usize,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_sequences: 100,
        }
    }
}

/// Sequence metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SequenceMetrics {
    /// Total sequences created
    pub total_sequences: u64,
}

/// Sequence step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceStep {
    /// Step index
    pub index: usize,
    /// Session ID
    pub session_id: SessionId,
} 