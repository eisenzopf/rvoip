//! Bridge Management API
//!
//! This module provides types and functionality for managing bridges between sessions.
//! A bridge connects two SIP sessions, allowing audio to flow between them.
//! 
//! # Overview
//! 
//! Bridges are implemented as 2-party conferences, providing a clean abstraction
//! for connecting calls. This is commonly used in call center scenarios to connect
//! customers with agents, or in PBX systems for call transfers.
//! 
//! # Example Usage
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::sync::Arc;
//! 
//! async fn bridge_calls(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
//!     // Assume we have two active sessions
//!     let customer_session_id = SessionId::from_string("sess_customer_123".to_string());
//!     let agent_session_id = SessionId::from_string("sess_agent_456".to_string());
//!     
//!     // Create a bridge between them
//!     let bridge_id = coordinator.bridge_sessions(
//!         &customer_session_id,
//!         &agent_session_id
//!     ).await?;
//!     
//!     println!("Created bridge: {}", bridge_id);
//!     
//!     // Monitor bridge events
//!     let mut events = coordinator.subscribe_to_bridge_events().await;
//!     
//!     while let Some(event) = events.recv().await {
//!         match event {
//!             BridgeEvent::ParticipantAdded { bridge_id, session_id } => {
//!                 println!("Session {} joined bridge {}", session_id, bridge_id);
//!             }
//!             BridgeEvent::ParticipantRemoved { bridge_id, session_id, reason } => {
//!                 println!("Session {} left bridge {}: {}", session_id, bridge_id, reason);
//!             }
//!             BridgeEvent::BridgeDestroyed { bridge_id } => {
//!                 println!("Bridge {} destroyed", bridge_id);
//!                 break;
//!             }
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Bridge Lifecycle
//! 
//! 1. **Creation**: Bridge is created via `bridge_sessions()` or `create_bridge()`
//! 2. **Active**: Participants can be added/removed, audio flows between them
//! 3. **Destruction**: Bridge is destroyed when empty or explicitly via `destroy_bridge()`
//! 
//! # Use Cases
//! 
//! - **Call Center**: Connect customer and agent calls
//! - **Call Transfer**: Bridge original call with transfer target
//! - **Consultation**: Agent consults with supervisor while customer is on hold
//! - **Conference**: Simple 2-party conference calls

use std::time::Instant;
use serde::{Serialize, Deserialize};

/// Unique identifier for a bridge
/// 
/// Bridges are identified by a unique string ID that is automatically
/// generated when the bridge is created.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BridgeId(pub String);

impl BridgeId {
    /// Create a new unique bridge ID
    pub fn new() -> Self {
        Self(format!("bridge_{}", uuid::Uuid::new_v4()))
    }
    
    /// Get the ID as a string reference
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BridgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for BridgeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about an active bridge
/// 
/// This struct contains metadata about a bridge including its participants
/// and creation time.
#[derive(Debug, Clone)]
pub struct BridgeInfo {
    /// Unique identifier for the bridge
    pub id: BridgeId,
    /// List of session IDs currently in the bridge
    pub sessions: Vec<crate::api::types::SessionId>,
    /// When the bridge was created
    pub created_at: Instant,
    /// Number of participants (convenience field, same as sessions.len())
    pub participant_count: usize,
}

/// Events that can occur on a bridge
/// 
/// These events are sent via the bridge event subscription channel
/// to allow monitoring of bridge state changes.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// A participant was added to the bridge
    ParticipantAdded {
        /// The bridge that was modified
        bridge_id: BridgeId,
        /// The session that was added
        session_id: crate::api::types::SessionId,
    },
    /// A participant was removed from the bridge
    ParticipantRemoved {
        /// The bridge that was modified
        bridge_id: BridgeId,
        /// The session that was removed
        session_id: crate::api::types::SessionId,
        /// Reason for removal (e.g., "Call ended", "Removed by admin")
        reason: String,
    },
    /// The bridge was destroyed
    BridgeDestroyed {
        /// The bridge that was destroyed
        bridge_id: BridgeId,
    },
}

/// Bridge event types (deprecated - use BridgeEvent enum directly)
#[deprecated(since = "0.2.0", note = "Use BridgeEvent enum pattern matching instead")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeEventType {
    Created,
    SessionAdded,
    SessionRemoved,
    Destroyed,
    MediaEstablished,
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
