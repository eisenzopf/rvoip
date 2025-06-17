//! Bridge Management Types
//! 
//! Types for managing bridges between sessions.

use std::time::Instant;
use crate::api::types::SessionId;

/// Unique identifier for a bridge
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BridgeId(pub String);

impl BridgeId {
    pub fn new() -> Self {
        Self(format!("bridge_{}", uuid::Uuid::new_v4()))
    }
    
    pub fn from_string(id: String) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for BridgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Information about an active bridge
#[derive(Debug, Clone)]
pub struct BridgeInfo {
    pub id: BridgeId,
    pub sessions: Vec<SessionId>,
    pub created_at: Instant,
    pub participant_count: usize,
}

/// Bridge event notifications
#[derive(Debug, Clone)]
pub struct BridgeEvent {
    pub bridge_id: BridgeId,
    pub event_type: BridgeEventType,
    pub session_id: Option<SessionId>,
    pub timestamp: Instant,
}

/// Types of bridge events
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeEventType {
    /// Bridge was created
    Created,
    /// Session was added to bridge
    SessionAdded,
    /// Session was removed from bridge
    SessionRemoved,
    /// Bridge was destroyed
    Destroyed,
    /// Media started flowing
    MediaEstablished,
    /// Media stopped
    MediaStopped,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bridge_id_creation() {
        let id1 = BridgeId::new();
        let id2 = BridgeId::new();
        assert_ne!(id1, id2);
        assert!(id1.0.starts_with("bridge_"));
    }
    
    #[test]
    fn test_bridge_event_type() {
        let event_type = BridgeEventType::Created;
        assert_eq!(event_type, BridgeEventType::Created);
    }
}
