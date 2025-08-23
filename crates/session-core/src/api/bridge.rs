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

// ============================================================================
// N-Party Call Bridge Implementation
// ============================================================================

use crate::api::call::SimpleCall;
use crate::errors::{Result, SessionError};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Bridge for connecting multiple calls
/// 
/// This provides a high-level interface for bridging multiple calls together,
/// supporting various topologies (full mesh, linear chain, custom).
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::bridge::{CallBridge, BridgeType};
/// 
/// async fn create_conference(calls: Vec<SimpleCall>) -> Result<()> {
///     let bridge = CallBridge::new();
///     
///     // Add all calls to the bridge
///     for call in calls {
///         bridge.add(call).await;
///     }
///     
///     // Connect everyone to everyone
///     bridge.set_type(BridgeType::Full).await;
///     bridge.connect().await?;
///     
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct CallBridge {
    inner: Arc<RwLock<CallBridgeInner>>,
}

struct CallBridgeInner {
    calls: Vec<SimpleCall>,
    bridge_type: BridgeType,
    active_bridges: Vec<BridgeId>,
}

/// Defines how calls are connected within a bridge
#[derive(Clone, Debug)]
pub enum BridgeType {
    /// Everyone connected to everyone (conference)
    Full,
    /// Linear chain: 0 <-> 1 <-> 2 <-> 3
    Linear,
    /// Custom connections between specific call indices
    Selective(Vec<(usize, usize)>),
}

impl CallBridge {
    /// Create a new empty bridge
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(CallBridgeInner {
                calls: Vec::new(),
                bridge_type: BridgeType::Full,
                active_bridges: Vec::new(),
            }))
        }
    }
    
    /// Add a call to the bridge
    /// 
    /// Returns the index of the added call for future reference.
    pub async fn add(&self, call: SimpleCall) -> usize {
        let mut inner = self.inner.write().await;
        inner.calls.push(call);
        inner.calls.len() - 1
    }
    
    /// Remove a call from the bridge by index
    /// 
    /// Returns the removed call if the index was valid.
    pub async fn remove(&self, index: usize) -> Option<SimpleCall> {
        let mut inner = self.inner.write().await;
        if index < inner.calls.len() {
            // Also need to clean up any bridges involving this call
            // TODO: Implement bridge cleanup
            Some(inner.calls.remove(index))
        } else {
            None
        }
    }
    
    /// Set the bridge type
    pub async fn set_type(&self, bridge_type: BridgeType) {
        self.inner.write().await.bridge_type = bridge_type;
    }
    
    /// Get the current bridge type
    pub async fn get_type(&self) -> BridgeType {
        self.inner.read().await.bridge_type.clone()
    }
    
    /// Connect all calls according to bridge type
    pub async fn connect(&self) -> Result<()> {
        let mut inner = self.inner.write().await;
        
        // Clear existing bridges
        for bridge_id in inner.active_bridges.drain(..) {
            // TODO: Call coordinator.destroy_bridge(&bridge_id)
            tracing::debug!("Would destroy bridge: {}", bridge_id);
        }
        
        // Determine which connections to make
        let connections = match &inner.bridge_type {
            BridgeType::Full => {
                // Connect everyone to everyone
                let mut conns = Vec::new();
                for i in 0..inner.calls.len() {
                    for j in i+1..inner.calls.len() {
                        conns.push((i, j));
                    }
                }
                conns
            }
            BridgeType::Linear => {
                // Chain connections: 0-1, 1-2, 2-3, etc.
                let mut conns = Vec::new();
                for i in 0..inner.calls.len().saturating_sub(1) {
                    conns.push((i, i + 1));
                }
                conns
            }
            BridgeType::Selective(pairs) => pairs.clone(),
        };
        
        // Create the actual bridges
        for (i, j) in connections {
            if i < inner.calls.len() && j < inner.calls.len() {
                let call_a = &inner.calls[i];
                let call_b = &inner.calls[j];
                
                // Use the coordinator to bridge the sessions
                let bridge_id = call_a.coordinator()
                    .bridge_sessions(call_a.id(), call_b.id())
                    .await?;
                
                inner.active_bridges.push(bridge_id);
            }
        }
        
        Ok(())
    }
    
    /// Disconnect all bridges
    pub async fn disconnect(&self) -> Result<()> {
        let mut inner = self.inner.write().await;
        
        // Collect bridge IDs to destroy
        let bridge_ids: Vec<_> = inner.active_bridges.drain(..).collect();
        
        // Get coordinator from first call (they should all have the same one)
        if let Some(call) = inner.calls.first() {
            for bridge_id in bridge_ids {
                call.coordinator().destroy_bridge(&bridge_id).await?;
            }
        }
        
        Ok(())
    }
    
    /// Get the number of calls in the bridge
    pub async fn call_count(&self) -> usize {
        self.inner.read().await.calls.len()
    }
    
    /// Hold a specific call by index
    pub async fn hold(&self, index: usize) -> Result<()> {
        let inner = self.inner.read().await;
        inner.calls.get(index)
            .ok_or(SessionError::Other("Invalid call index".to_string()))?
            .hold()
            .await
    }
    
    /// Resume a specific call by index
    pub async fn resume(&self, index: usize) -> Result<()> {
        let inner = self.inner.read().await;
        inner.calls.get(index)
            .ok_or(SessionError::Other("Invalid call index".to_string()))?
            .resume()
            .await
    }
    
    /// Mute a specific call by index
    pub async fn mute(&self, index: usize) -> Result<()> {
        let inner = self.inner.read().await;
        inner.calls.get(index)
            .ok_or(SessionError::Other("Invalid call index".to_string()))?
            .mute()
            .await
    }
    
    /// Unmute a specific call by index
    pub async fn unmute(&self, index: usize) -> Result<()> {
        let inner = self.inner.read().await;
        inner.calls.get(index)
            .ok_or(SessionError::Other("Invalid call index".to_string()))?
            .unmute()
            .await
    }
    
    /// Get call info by index
    pub async fn get_call_info(&self, index: usize) -> Option<String> {
        let inner = self.inner.read().await;
        inner.calls.get(index).map(|call| call.remote_uri().to_string())
    }
    
    /// List all call URIs in the bridge
    pub async fn list_calls(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner.calls.iter().map(|call| call.remote_uri().to_string()).collect()
    }
}

impl Default for CallBridge {
    fn default() -> Self {
        Self::new()
    }
}

// Helper functions for common bridge patterns
pub mod helpers {
    use super::*;
    
    /// Create a simple two-party bridge
    /// 
    /// This is a convenience function for the common case of bridging two calls.
    pub async fn connect_two(call_a: SimpleCall, call_b: SimpleCall) -> Result<CallBridge> {
        let bridge = CallBridge::new();
        bridge.add(call_a).await;
        bridge.add(call_b).await;
        bridge.connect().await?;
        Ok(bridge)
    }
    
    /// Create a conference bridge with multiple parties
    /// 
    /// All parties will be connected to each other (full mesh).
    pub async fn create_conference(calls: Vec<SimpleCall>) -> Result<CallBridge> {
        let bridge = CallBridge::new();
        for call in calls {
            bridge.add(call).await;
        }
        bridge.set_type(BridgeType::Full).await;
        bridge.connect().await?;
        Ok(bridge)
    }
    
    /// Create a linear chain of calls
    /// 
    /// Useful for scenarios like whisper/coach mode where calls are
    /// connected in sequence rather than all-to-all.
    pub async fn create_chain(calls: Vec<SimpleCall>) -> Result<CallBridge> {
        let bridge = CallBridge::new();
        for call in calls {
            bridge.add(call).await;
        }
        bridge.set_type(BridgeType::Linear).await;
        bridge.connect().await?;
        Ok(bridge)
    }
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
    
    #[tokio::test]
    async fn test_call_bridge_creation() {
        let bridge = CallBridge::new();
        assert_eq!(bridge.call_count().await, 0);
    }
    
    #[tokio::test]
    async fn test_bridge_type_setting() {
        let bridge = CallBridge::new();
        bridge.set_type(BridgeType::Linear).await;
        match bridge.get_type().await {
            BridgeType::Linear => (),
            _ => panic!("Bridge type should be Linear"),
        }
    }
}
