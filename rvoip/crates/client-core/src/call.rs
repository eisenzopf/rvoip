//! Call management for SIP client
//!
//! This module provides call information structures and lightweight call tracking.
//! All actual SIP/media operations are delegated to session-core.
//!
//! PROPER LAYER SEPARATION:
//! client-core -> session-core -> {transaction-core, media-core, sip-transport, sip-core}

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Unique identifier for a call
pub type CallId = Uuid;

/// Current state of a call
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallState {
    /// Call is being initiated (sending INVITE)
    Initiating,
    /// Received 100 Trying or similar provisional response
    Proceeding,
    /// Received 180 Ringing
    Ringing,
    /// Call is connected and media is flowing
    Connected,
    /// Call is being terminated (sending/received BYE)
    Terminating,
    /// Call has ended
    Terminated,
    /// Call failed to establish
    Failed,
    /// Call was cancelled before connection
    Cancelled,
    /// Incoming call waiting for user decision
    IncomingPending,
}

impl CallState {
    /// Check if the call is in an active state (can send/receive media)
    pub fn is_active(&self) -> bool {
        matches!(self, CallState::Connected)
    }

    /// Check if the call is in a terminated state
    pub fn is_terminated(&self) -> bool {
        matches!(
            self,
            CallState::Terminated | CallState::Failed | CallState::Cancelled
        )
    }

    /// Check if the call is still in progress
    pub fn is_in_progress(&self) -> bool {
        !self.is_terminated()
    }
}

/// Direction of a call (from client's perspective)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Outgoing call (client initiated)
    Outgoing,
    /// Incoming call (received from network)
    Incoming,
}

/// Information about a SIP call
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// Unique call identifier
    pub call_id: CallId,
    /// Current state of the call
    pub state: CallState,
    /// Direction of the call
    pub direction: CallDirection,
    /// Local party URI (our user)
    pub local_uri: String,
    /// Remote party URI (who we're calling/called by)
    pub remote_uri: String,
    /// Display name of remote party (if available)
    pub remote_display_name: Option<String>,
    /// Call subject/reason
    pub subject: Option<String>,
    /// When the call was created
    pub created_at: DateTime<Utc>,
    /// When the call was connected (if applicable)
    pub connected_at: Option<DateTime<Utc>>,
    /// When the call ended (if applicable)
    pub ended_at: Option<DateTime<Utc>>,
    /// Remote network address
    pub remote_addr: Option<SocketAddr>,
    /// Associated media session ID (if any)
    pub media_session_id: Option<String>,
    /// SIP Call-ID header value
    pub sip_call_id: String,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Statistics about current calls
#[derive(Debug, Clone)]
pub struct CallStats {
    pub total_active_calls: usize,
    pub connected_calls: usize,
    pub incoming_pending_calls: usize,
} 