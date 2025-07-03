//! Call management for SIP client
//!
//! This module provides call information structures and lightweight call tracking.
//! All actual SIP/media operations are delegated to session-core.
//!
//! # Architecture
//!
//! PROPER LAYER SEPARATION:
//! client-core -> session-core -> {transaction-core, media-core, sip-transport, sip-core}
//!
//! # Key Components
//!
//! - **CallId** - Unique identifier for each call
//! - **CallState** - Current state in the call lifecycle  
//! - **CallDirection** - Whether call is incoming or outgoing
//! - **CallInfo** - Comprehensive call metadata and status
//! - **CallStats** - Aggregate statistics about active calls
//!
//! # Usage Examples
//!
//! ## Creating Call Information
//!
//! ```rust
//! use rvoip_client_core::call::{CallInfo, CallState, CallDirection, CallId};
//! use chrono::Utc;
//! use std::collections::HashMap;
//!
//! let call_info = CallInfo {
//!     call_id: CallId::new_v4(),
//!     state: CallState::Initiating,
//!     direction: CallDirection::Outgoing,
//!     local_uri: "sip:user@example.com".to_string(),
//!     remote_uri: "sip:target@example.com".to_string(),
//!     remote_display_name: Some("Alice Smith".to_string()),
//!     subject: None,
//!     created_at: Utc::now(),
//!     connected_at: None,
//!     ended_at: None,
//!     remote_addr: None,
//!     media_session_id: None,
//!     sip_call_id: "abc123@example.com".to_string(),
//!     metadata: HashMap::new(),
//! };
//!
//! assert_eq!(call_info.state, CallState::Initiating);
//! assert_eq!(call_info.direction, CallDirection::Outgoing);
//! ```
//!
//! ## Working with Call States
//!
//! ```rust
//! use rvoip_client_core::call::CallState;
//!
//! let state = CallState::Connected;
//! assert!(state.is_active());
//! assert!(state.is_in_progress());
//! assert!(!state.is_terminated());
//!
//! let terminated_state = CallState::Terminated;
//! assert!(!terminated_state.is_active());
//! assert!(terminated_state.is_terminated());
//! assert!(!terminated_state.is_in_progress());
//! ```
//!
//! ## Checking Call Statistics
//!
//! ```rust
//! use rvoip_client_core::call::CallStats;
//!
//! let stats = CallStats {
//!     total_active_calls: 5,
//!     connected_calls: 3,
//!     incoming_pending_calls: 2,
//! };
//!
//! assert_eq!(stats.total_active_calls, 5);
//! assert_eq!(stats.connected_calls, 3);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Unique identifier for a call
/// 
/// Each call in the system is assigned a unique UUID that remains constant
/// throughout the call lifecycle. This ID is used to correlate events,
/// state changes, and operations across all system components.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::call::CallId;
/// use uuid::Uuid;
/// 
/// let call_id: CallId = Uuid::new_v4();
/// println!("Call ID: {}", call_id);
/// ```
pub type CallId = Uuid;

/// Current state of a call in its lifecycle
/// 
/// Represents the various states a call can be in, from initiation through
/// termination. These states correspond to SIP call flow stages and help
/// determine what operations are valid at any given time.
/// 
/// # State Transitions
/// 
/// Typical outgoing call flow:
/// `Initiating` → `Proceeding` → `Ringing` → `Connected` → `Terminating` → `Terminated`
/// 
/// Typical incoming call flow:
/// `IncomingPending` → `Connected` → `Terminating` → `Terminated`
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::call::CallState;
/// 
/// let state = CallState::Connected;
/// assert!(state.is_active());
/// assert!(state.is_in_progress());
/// assert!(!state.is_terminated());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallState {
    /// Call is being initiated (sending INVITE)
    /// 
    /// Initial state for outgoing calls when the INVITE request is being sent
    /// but no response has been received yet.
    Initiating,
    /// Received 100 Trying or similar provisional response
    /// 
    /// The remote party has acknowledged receipt of the INVITE and is
    /// processing the call request.
    Proceeding,
    /// Received 180 Ringing
    /// 
    /// The remote party's phone is ringing. This indicates the call setup
    /// is progressing normally.
    Ringing,
    /// Call is connected and media is flowing
    /// 
    /// The call has been answered and media (audio/video) can be exchanged.
    /// This is the primary active state for established calls.
    Connected,
    /// Call is being terminated (sending/received BYE)
    /// 
    /// Either party has initiated call termination but the termination
    /// process is not yet complete.
    Terminating,
    /// Call has ended normally
    /// 
    /// The call was successfully terminated by either party. This is a
    /// final state.
    Terminated,
    /// Call failed to establish
    /// 
    /// The call setup failed due to network issues, busy signal, rejection,
    /// or other errors. This is a final state.
    Failed,
    /// Call was cancelled before connection
    /// 
    /// The call was cancelled by the caller before the remote party answered.
    /// This is a final state.
    Cancelled,
    /// Incoming call waiting for user decision
    /// 
    /// A call invitation has been received and is waiting for the user to
    /// accept, reject, or ignore it.
    IncomingPending,
}

impl CallState {
    /// Check if the call is in an active state (can send/receive media)
    /// 
    /// Returns `true` only for the `Connected` state where media can be
    /// actively exchanged between parties.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::call::CallState;
    /// 
    /// assert!(CallState::Connected.is_active());
    /// assert!(!CallState::Ringing.is_active());
    /// assert!(!CallState::Terminated.is_active());
    /// ```
    pub fn is_active(&self) -> bool {
        matches!(self, CallState::Connected)
    }

    /// Check if the call is in a terminated state
    /// 
    /// Returns `true` for any final state where the call has ended and
    /// no further operations are possible.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::call::CallState;
    /// 
    /// assert!(CallState::Terminated.is_terminated());
    /// assert!(CallState::Failed.is_terminated());
    /// assert!(CallState::Cancelled.is_terminated());
    /// assert!(!CallState::Connected.is_terminated());
    /// assert!(!CallState::Ringing.is_terminated());
    /// ```
    pub fn is_terminated(&self) -> bool {
        matches!(
            self,
            CallState::Terminated | CallState::Failed | CallState::Cancelled
        )
    }

    /// Check if the call is still in progress
    /// 
    /// Returns `true` for any non-terminal state where the call is still
    /// active or progressing through setup/teardown.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::call::CallState;
    /// 
    /// assert!(CallState::Connected.is_in_progress());
    /// assert!(CallState::Ringing.is_in_progress());
    /// assert!(CallState::Initiating.is_in_progress());
    /// assert!(!CallState::Terminated.is_in_progress());
    /// assert!(!CallState::Failed.is_in_progress());
    /// ```
    pub fn is_in_progress(&self) -> bool {
        !self.is_terminated()
    }
}

/// Direction of a call (from client's perspective)
/// 
/// Indicates whether the call was initiated by this client (outgoing) or
/// by a remote party (incoming). This affects call handling and UI presentation.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::call::CallDirection;
/// 
/// let outgoing = CallDirection::Outgoing;
/// let incoming = CallDirection::Incoming;
/// 
/// assert_eq!(outgoing, CallDirection::Outgoing);
/// assert_ne!(outgoing, incoming);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Outgoing call (client initiated)
    /// 
    /// This client initiated the call by sending an INVITE to a remote party.
    Outgoing,
    /// Incoming call (received from network)
    /// 
    /// A remote party initiated this call by sending an INVITE to this client.
    Incoming,
}

/// Comprehensive information about a SIP call
/// 
/// Contains all metadata and status information for a call, including
/// parties involved, timing, state, and technical details.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::call::{CallInfo, CallState, CallDirection, CallId};
/// use chrono::Utc;
/// use std::collections::HashMap;
/// 
/// let call_info = CallInfo {
///     call_id: CallId::new_v4(),
///     state: CallState::Connected,
///     direction: CallDirection::Outgoing,
///     local_uri: "sip:alice@example.com".to_string(),
///     remote_uri: "sip:bob@example.com".to_string(),
///     remote_display_name: Some("Bob Smith".to_string()),
///     subject: Some("Business call".to_string()),
///     created_at: Utc::now(),
///     connected_at: Some(Utc::now()),
///     ended_at: None,
///     remote_addr: None,
///     media_session_id: Some("media123".to_string()),
///     sip_call_id: "abc123@example.com".to_string(),
///     metadata: HashMap::new(),
/// };
/// 
/// assert_eq!(call_info.state, CallState::Connected);
/// assert!(call_info.state.is_active());
/// ```
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// Unique call identifier assigned by the client
    pub call_id: CallId,
    /// Current state of the call in its lifecycle
    pub state: CallState,
    /// Direction of the call (incoming or outgoing)
    pub direction: CallDirection,
    /// Local party URI (our user's SIP address)
    pub local_uri: String,
    /// Remote party URI (who we're calling or who called us)
    pub remote_uri: String,
    /// Display name of remote party, if provided in SIP headers
    pub remote_display_name: Option<String>,
    /// Call subject or reason, if provided in SIP headers
    pub subject: Option<String>,
    /// When the call was created/initiated
    pub created_at: DateTime<Utc>,
    /// When the call was answered and connected (if applicable)
    pub connected_at: Option<DateTime<Utc>>,
    /// When the call ended (if applicable)
    pub ended_at: Option<DateTime<Utc>>,
    /// Remote network address (IP and port)
    pub remote_addr: Option<SocketAddr>,
    /// Associated media session ID for audio/video (if any)
    pub media_session_id: Option<String>,
    /// SIP Call-ID header value for protocol correlation
    pub sip_call_id: String,
    /// Additional call metadata and custom properties
    pub metadata: HashMap<String, String>,
}

/// Statistics about current calls in the system
/// 
/// Provides aggregate counts and metrics about active calls, useful for
/// monitoring, load management, and user interface displays.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::call::CallStats;
/// 
/// let stats = CallStats {
///     total_active_calls: 10,
///     connected_calls: 7,
///     incoming_pending_calls: 3,
/// };
/// 
/// assert_eq!(stats.total_active_calls, 10);
/// assert_eq!(stats.connected_calls, 7);
/// assert_eq!(stats.incoming_pending_calls, 3);
/// 
/// // Calculate derived metrics
/// let ringing_calls = stats.total_active_calls - stats.connected_calls - stats.incoming_pending_calls;
/// assert_eq!(ringing_calls, 0);
/// ```
#[derive(Debug, Clone)]
pub struct CallStats {
    /// Total number of active calls (not in terminal states)
    pub total_active_calls: usize,
    /// Number of calls in Connected state (media flowing)
    pub connected_calls: usize,
    /// Number of incoming calls waiting for user decision
    pub incoming_pending_calls: usize,
} 