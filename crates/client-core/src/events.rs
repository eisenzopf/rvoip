//! Event handling for client-core operations
//!
//! This module provides a comprehensive event system for VoIP client operations,
//! including call events, media events, registration events, and error handling.
//! The event system supports filtering, prioritization, and async handling.
//!
//! # Event Types
//!
//! - **Call Events** - Incoming calls, state changes, completion
//! - **Media Events** - Audio start/stop, quality changes, DTMF
//! - **Registration Events** - SIP registration status changes
//! - **Network Events** - Connectivity changes
//! - **Error Events** - Client errors and failures
//!
//! # Usage Examples
//!
//! ## Basic Event Handler
//!
//! ```rust
//! use rvoip_client_core::events::{ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
//! use async_trait::async_trait;
//!
//! struct MyEventHandler;
//!
//! #[async_trait]
//! impl ClientEventHandler for MyEventHandler {
//!     async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
//!         println!("Incoming call from: {}", call_info.caller_uri);
//!         CallAction::Accept
//!     }
//!
//!     async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
//!         println!("Call {:?} state changed to {:?}", status_info.call_id, status_info.new_state);
//!     }
//!
//!     async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo) {
//!         println!("Registration status: {:?}", status_info.status);
//!     }
//! }
//! ```
//!
//! ## Event Filtering
//!
//! ```rust
//! use rvoip_client_core::events::{EventFilter, EventPriority, MediaEventType};
//! use std::collections::HashSet;
//!
//! // Create a filter for high-priority events only
//! let filter = EventFilter {
//!     min_priority: Some(EventPriority::High),
//!     media_event_types: None,
//!     call_ids: None,
//!     call_states: None,
//!     registration_ids: None,
//! };
//!
//! // Create a filter for specific media events
//! let mut media_types = HashSet::new();
//! media_types.insert(MediaEventType::AudioStarted);
//! media_types.insert(MediaEventType::AudioStopped);
//!
//! let media_filter = EventFilter {
//!     media_event_types: Some(media_types),
//!     min_priority: None,
//!     call_ids: None,
//!     call_states: None,
//!     registration_ids: None,
//! };
//! ```
//!
//! ## Event Subscription
//!
//! ```rust
//! use rvoip_client_core::events::{EventSubscription, EventFilter, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
//! use async_trait::async_trait;
//! use std::sync::Arc;
//!
//! struct TestHandler;
//!
//! #[async_trait]
//! impl ClientEventHandler for TestHandler {
//!     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
//!         CallAction::Accept
//!     }
//!     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
//!     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
//! }
//!
//! let handler = Arc::new(TestHandler);
//! let subscription = EventSubscription::all_events(handler);
//! println!("Subscription ID: {}", subscription.id());
//! ```

use std::sync::Arc;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use async_trait::async_trait;
use std::collections::HashSet;

use crate::call::{CallId, CallState};

/// Action to take for an incoming call
/// 
/// Determines how the client should respond to an incoming call invitation.
/// This is returned by event handlers to control call behavior.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::CallAction;
/// 
/// let action = CallAction::Accept;
/// assert_eq!(action, CallAction::Accept);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallAction {
    /// Accept the incoming call and establish the connection
    Accept,
    /// Reject the incoming call (sends busy or decline response)
    Reject,
    /// Ignore the incoming call (let it ring without responding)
    Ignore,
}

/// Information about an incoming call
/// 
/// Contains all available details about a call invitation, including
/// caller information, call metadata, and timing information.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::IncomingCallInfo;
/// use rvoip_client_core::call::CallId;
/// use chrono::Utc;
/// 
/// let call_info = IncomingCallInfo {
///     call_id: uuid::Uuid::new_v4(),
///     caller_uri: "sip:alice@example.com".to_string(),
///     callee_uri: "sip:bob@example.com".to_string(),
///     caller_display_name: Some("Alice Smith".to_string()),
///     subject: Some("Business call".to_string()),
///     created_at: Utc::now(),
/// };
/// 
/// assert_eq!(call_info.caller_uri, "sip:alice@example.com");
/// ```
#[derive(Debug, Clone)]
pub struct IncomingCallInfo {
    /// Unique call identifier assigned by the client
    pub call_id: CallId,
    /// SIP URI of the caller (e.g., "sip:alice@example.com")
    pub caller_uri: String,
    /// SIP URI of the callee/local user (e.g., "sip:bob@example.com")
    pub callee_uri: String,
    /// Display name of the caller, if provided in the SIP headers
    pub caller_display_name: Option<String>,
    /// Call subject or reason, if provided in the SIP headers
    pub subject: Option<String>,
    /// Timestamp when the call invitation was received
    pub created_at: DateTime<Utc>,
}

/// Information about a call state change
/// 
/// Provides details about call state transitions, including the previous
/// and new states, timing, and reasons for the change.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::CallStatusInfo;
/// use rvoip_client_core::call::{CallId, CallState};
/// use chrono::Utc;
/// 
/// let status_info = CallStatusInfo {
///     call_id: uuid::Uuid::new_v4(),
///     new_state: CallState::Connected,
///     previous_state: Some(CallState::Ringing),
///     reason: Some("Call answered".to_string()),
///     timestamp: Utc::now(),
/// };
/// 
/// assert_eq!(status_info.new_state, CallState::Connected);
/// ```
#[derive(Debug, Clone)]
pub struct CallStatusInfo {
    /// Call that changed state
    pub call_id: CallId,
    /// New call state after the transition
    pub new_state: CallState,
    /// Previous call state before the transition (if known)
    pub previous_state: Option<CallState>,
    /// Reason for the state change (e.g., "Call answered", "Network error")
    pub reason: Option<String>,
    /// When the state change occurred
    pub timestamp: DateTime<Utc>,
}

/// Information about registration status changes
/// 
/// Contains details about SIP registration state changes, including
/// server information, user details, and status transition data.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::RegistrationStatusInfo;
/// use rvoip_client_core::registration::RegistrationStatus;
/// use chrono::Utc;
/// use uuid::Uuid;
/// 
/// let reg_info = RegistrationStatusInfo {
///     registration_id: Uuid::new_v4(),
///     server_uri: "sip:registrar.example.com".to_string(),
///     user_uri: "sip:user@example.com".to_string(),
///     status: RegistrationStatus::Active,
///     reason: Some("Registration successful".to_string()),
///     timestamp: Utc::now(),
/// };
/// 
/// assert_eq!(reg_info.status, RegistrationStatus::Active);
/// ```
#[derive(Debug, Clone)]
pub struct RegistrationStatusInfo {
    /// Unique registration identifier
    pub registration_id: uuid::Uuid,
    /// SIP registrar server URI
    pub server_uri: String,
    /// User URI being registered with the server
    pub user_uri: String,
    /// Current registration status
    pub status: crate::registration::RegistrationStatus,
    /// Reason for the status change (e.g., "Registration expired")
    pub reason: Option<String>,
    /// When the status change occurred
    pub timestamp: DateTime<Utc>,
}

/// Types of media events that can occur during calls
/// 
/// Categorizes different media-related events including audio control,
/// session management, and quality monitoring events.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::MediaEventType;
/// 
/// let event = MediaEventType::AudioStarted;
/// let mute_event = MediaEventType::MicrophoneStateChanged { muted: true };
/// let quality_event = MediaEventType::QualityChanged { mos_score_x100: 425 }; // 4.25 MOS
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaEventType {
    // Phase 4.1: Enhanced Media Integration
    /// Microphone mute state changed
    MicrophoneStateChanged { 
        /// Whether the microphone is now muted
        muted: bool 
    },
    /// Speaker mute state changed
    SpeakerStateChanged { 
        /// Whether the speaker is now muted
        muted: bool 
    },
    /// Audio streaming started for a call
    AudioStarted,
    /// Audio streaming stopped for a call
    AudioStopped,
    /// Call hold state changed
    HoldStateChanged { 
        /// Whether the call is now on hold
        on_hold: bool 
    },
    /// DTMF digits were sent during the call
    DtmfSent { 
        /// The DTMF digits that were sent
        digits: String 
    },
    /// Call transfer was initiated
    TransferInitiated { 
        /// Target URI for the transfer
        target: String, 
        /// Type of transfer (e.g., "blind", "attended")
        transfer_type: String 
    },
    
    // Phase 4.2: Media Session Coordination
    /// SDP offer was generated for media negotiation
    SdpOfferGenerated { 
        /// Size of the SDP offer in bytes
        sdp_size: usize 
    },
    /// SDP answer was processed during media negotiation
    SdpAnswerProcessed { 
        /// Size of the SDP answer in bytes
        sdp_size: usize 
    },
    /// Media session started successfully
    MediaSessionStarted { 
        /// Unique identifier for the media session
        media_session_id: String 
    },
    /// Media session stopped
    MediaSessionStopped,
    /// Media session was updated/modified
    MediaSessionUpdated { 
        /// Size of the updated SDP in bytes
        sdp_size: usize 
    },
    
    // Quality and monitoring events (using integers for Hash/Eq compatibility)
    /// Call quality changed (MOS score)
    QualityChanged { 
        /// MOS score * 100 (e.g., 425 = 4.25 MOS score)
        mos_score_x100: u32 
    },
    /// Packet loss detected
    PacketLoss { 
        /// Packet loss percentage * 100 (e.g., 150 = 1.5%)
        percentage_x100: u32 
    },
    /// Jitter levels changed
    JitterChanged { 
        /// Current jitter in milliseconds
        jitter_ms: u32 
    },
}

/// Media event information
/// 
/// Contains detailed information about a media event, including timing,
/// metadata, and the associated call.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::{MediaEventInfo, MediaEventType};
/// use rvoip_client_core::call::CallId;
/// use chrono::Utc;
/// use std::collections::HashMap;
/// 
/// let mut metadata = HashMap::new();
/// metadata.insert("codec".to_string(), "PCMU".to_string());
/// 
/// let media_event = MediaEventInfo {
///     call_id: uuid::Uuid::new_v4(),
///     event_type: MediaEventType::AudioStarted,
///     timestamp: Utc::now(),
///     metadata,
/// };
/// 
/// assert_eq!(media_event.event_type, MediaEventType::AudioStarted);
/// ```
#[derive(Debug, Clone)]
pub struct MediaEventInfo {
    /// Call the media event relates to
    pub call_id: CallId,
    /// Type of media event that occurred
    pub event_type: MediaEventType,
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Additional event metadata (codec info, quality details, etc.)
    pub metadata: std::collections::HashMap<String, String>,
}

/// Event filtering options for selective subscription
/// 
/// Allows clients to subscribe only to specific types of events,
/// reducing noise and improving performance for targeted use cases.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::{EventFilter, EventPriority};
/// use rvoip_client_core::call::CallId;
/// use std::collections::HashSet;
/// 
/// // Filter for high-priority events only
/// let priority_filter = EventFilter {
///     min_priority: Some(EventPriority::High),
///     call_ids: None,
///     call_states: None,
///     media_event_types: None,
///     registration_ids: None,
/// };
/// 
/// // Filter for specific call
/// let mut call_ids = HashSet::new();
/// call_ids.insert(uuid::Uuid::new_v4());
/// let call_filter = EventFilter {
///     call_ids: Some(call_ids),
///     min_priority: None,
///     call_states: None,
///     media_event_types: None,
///     registration_ids: None,
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Only receive events for specific calls (None = all calls)
    pub call_ids: Option<HashSet<CallId>>,
    /// Only receive specific types of call state changes (None = all states)
    pub call_states: Option<HashSet<CallState>>,
    /// Only receive specific types of media events (None = all media events)
    pub media_event_types: Option<HashSet<MediaEventType>>,
    /// Only receive events for specific registration IDs (None = all registrations)
    pub registration_ids: Option<HashSet<uuid::Uuid>>,
    /// Minimum event priority level (None = all priorities)
    pub min_priority: Option<EventPriority>,
}

/// Event priority levels for filtering and handling
/// 
/// Allows classification of events by importance, enabling priority-based
/// filtering and handling strategies.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::EventPriority;
/// 
/// assert!(EventPriority::Critical > EventPriority::High);
/// assert!(EventPriority::High > EventPriority::Normal);
/// assert!(EventPriority::Normal > EventPriority::Low);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventPriority {
    /// Low priority events (quality updates, routine status changes)
    Low,
    /// Normal priority events (state changes, DTMF, media events)
    Normal,
    /// High priority events (incoming calls, registration changes)
    High,
    /// Critical priority events (failures, security issues, network problems)
    Critical,
}

/// Transfer status for tracking transfer progress
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    /// Transfer accepted, attempting to call target
    Accepted,
    /// Target is ringing
    Ringing,
    /// Transfer completed successfully
    Completed,
    /// Transfer failed
    Failed(String),
}

/// Comprehensive client event types
/// 
/// Unified event type that encompasses all possible events in the VoIP client,
/// with associated priority levels for filtering and handling.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::{ClientEvent, IncomingCallInfo, EventPriority};
/// use rvoip_client_core::call::CallId;
/// use chrono::Utc;
/// 
/// let call_info = IncomingCallInfo {
///     call_id: uuid::Uuid::new_v4(),
///     caller_uri: "sip:caller@example.com".to_string(),
///     callee_uri: "sip:callee@example.com".to_string(),
///     caller_display_name: None,
///     subject: None,
///     created_at: Utc::now(),
/// };
/// 
/// let event = ClientEvent::IncomingCall {
///     info: call_info,
///     priority: EventPriority::High,
/// };
/// 
/// assert_eq!(event.priority(), EventPriority::High);
/// ```
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// Incoming call received from a remote party
    IncomingCall {
        /// Information about the incoming call
        info: IncomingCallInfo,
        /// Priority level of this event
        priority: EventPriority,
    },
    /// Call state changed (connecting, connected, disconnected, etc.)
    CallStateChanged {
        /// Information about the state change
        info: CallStatusInfo,
        /// Priority of this event  
        priority: EventPriority,
    },
    /// Media event occurred (audio start/stop, quality change, etc.)
    MediaEvent {
        /// Information about the media event
        info: MediaEventInfo,
        /// Priority of this event
        priority: EventPriority,
    },
    /// Registration status changed with SIP server
    RegistrationStatusChanged {
        /// Information about the registration change
        info: RegistrationStatusInfo,
        /// Priority of this event
        priority: EventPriority,
    },
    /// Client error occurred
    ClientError {
        /// The error that occurred
        error: crate::ClientError,
        /// Call ID associated with the error (if any)
        call_id: Option<CallId>,
        /// Priority of this event
        priority: EventPriority,
    },
    /// Network connectivity changed
    NetworkEvent {
        /// Whether the network is now connected
        connected: bool,
        /// Reason for the connectivity change (if known)
        reason: Option<String>,
        /// Priority of this event
        priority: EventPriority,
    },
    /// Incoming transfer request received
    IncomingTransferRequest {
        /// Call ID being transferred
        call_id: CallId,
        /// Target URI to transfer to
        target_uri: String,
        /// Who initiated the transfer (optional)
        referred_by: Option<String>,
        /// Whether this is attended transfer (has Replaces)
        is_attended: bool,
        /// Priority of this event
        priority: EventPriority,
    },
    /// Transfer progress update
    TransferProgress {
        /// Call ID of the original call
        call_id: CallId,
        /// Transfer status
        status: TransferStatus,
        /// Priority of this event
        priority: EventPriority,
    },
}

impl ClientEvent {
    /// Get the priority of this event
    /// 
    /// Returns the priority level assigned to this event, which can be used
    /// for filtering and prioritization in event handling systems.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{ClientEvent, EventPriority};
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error_event = ClientEvent::ClientError {
    ///     error: ClientError::internal_error("Test error"),
    ///     call_id: None,
    ///     priority: EventPriority::High,
    /// };
    /// 
    /// assert_eq!(error_event.priority(), EventPriority::High);
    /// ```
    pub fn priority(&self) -> EventPriority {
        match self {
            ClientEvent::IncomingCall { priority, .. } => priority.clone(),
            ClientEvent::CallStateChanged { priority, .. } => priority.clone(),
            ClientEvent::MediaEvent { priority, .. } => priority.clone(),
            ClientEvent::RegistrationStatusChanged { priority, .. } => priority.clone(),
            ClientEvent::ClientError { priority, .. } => priority.clone(),
            ClientEvent::NetworkEvent { priority, .. } => priority.clone(),
            ClientEvent::IncomingTransferRequest { priority, .. } => priority.clone(),
            ClientEvent::TransferProgress { priority, .. } => priority.clone(),
        }
    }
    
    /// Get the call ID associated with this event (if any)
    /// 
    /// Returns the call ID for events that are related to a specific call.
    /// Not all events have an associated call ID (e.g., network events).
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{ClientEvent, EventPriority};
    /// 
    /// let network_event = ClientEvent::NetworkEvent {
    ///     connected: true,
    ///     reason: Some("Connection restored".to_string()),
    ///     priority: EventPriority::Normal,
    /// };
    /// 
    /// assert_eq!(network_event.call_id(), None);
    /// ```
    pub fn call_id(&self) -> Option<CallId> {
        match self {
            ClientEvent::IncomingCall { info, .. } => Some(info.call_id),
            ClientEvent::CallStateChanged { info, .. } => Some(info.call_id),
            ClientEvent::MediaEvent { info, .. } => Some(info.call_id),
            ClientEvent::ClientError { call_id, .. } => *call_id,
            _ => None,
        }
    }
    
    /// Check if this event passes the given filter
    /// 
    /// Tests whether this event matches the criteria specified in the filter.
    /// This is used by the event system to determine which subscriptions
    /// should receive this event.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{ClientEvent, EventFilter, EventPriority};
    /// 
    /// let event = ClientEvent::NetworkEvent {
    ///     connected: true,
    ///     reason: None,
    ///     priority: EventPriority::Normal,
    /// };
    /// 
    /// let filter = EventFilter {
    ///     min_priority: Some(EventPriority::High),
    ///     call_ids: None,
    ///     call_states: None,
    ///     media_event_types: None,
    ///     registration_ids: None,
    /// };
    /// 
    /// // This normal priority event should not pass a high-priority filter
    /// assert!(!event.passes_filter(&filter));
    /// ```
    pub fn passes_filter(&self, filter: &EventFilter) -> bool {
        // Check priority filter
        if let Some(min_priority) = &filter.min_priority {
            if self.priority() < *min_priority {
                return false;
            }
        }
        
        // Check call ID filter
        if let Some(call_ids) = &filter.call_ids {
            if let Some(call_id) = self.call_id() {
                if !call_ids.contains(&call_id) {
                    return false;
                }
            } else {
                // Event has no call ID but filter requires specific call IDs
                return false;
            }
        }
        
        // Check call state filter
        if let Some(call_states) = &filter.call_states {
            if let ClientEvent::CallStateChanged { info, .. } = self {
                if !call_states.contains(&info.new_state) {
                    return false;
                }
            }
        }
        
        // Check media event type filter
        if let Some(media_types) = &filter.media_event_types {
            if let ClientEvent::MediaEvent { info, .. } = self {
                if !media_types.contains(&info.event_type) {
                    return false;
                }
            }
        }
        
        // Check registration ID filter
        if let Some(registration_ids) = &filter.registration_ids {
            if let ClientEvent::RegistrationStatusChanged { info, .. } = self {
                if !registration_ids.contains(&info.registration_id) {
                    return false;
                }
            }
        }
        
        true
    }
}

/// Enhanced event handler with filtering capabilities
/// 
/// Trait for handling VoIP client events. Implement this trait to receive
/// and respond to various events that occur during client operation.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::{ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
/// use async_trait::async_trait;
/// 
/// struct LoggingEventHandler;
/// 
/// #[async_trait]
/// impl ClientEventHandler for LoggingEventHandler {
///     async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
///         println!("Incoming call from: {}", call_info.caller_uri);
///         CallAction::Accept
///     }
/// 
///     async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
///         println!("Call state changed: {:?}", status_info.new_state);
///     }
/// 
///     async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo) {
///         println!("Registration status: {:?}", status_info.status);
///     }
/// }
/// ```
#[async_trait]
pub trait ClientEventHandler: Send + Sync {
    /// Handle an incoming call with action decision
    /// 
    /// Called when a new call invitation is received. The implementation
    /// should return the desired action (Accept, Reject, or Ignore).
    /// 
    /// # Arguments
    /// 
    /// * `call_info` - Information about the incoming call
    /// 
    /// # Returns
    /// 
    /// The action to take for this incoming call
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction;
    
    /// Handle call state changes
    /// 
    /// Called when a call's state changes (e.g., from ringing to connected).
    /// This is useful for updating UI, logging, or triggering other actions.
    /// 
    /// # Arguments
    /// 
    /// * `status_info` - Information about the call state change
    async fn on_call_state_changed(&self, status_info: CallStatusInfo);
    
    /// Handle registration status changes
    /// 
    /// Called when the SIP registration status changes with a server.
    /// This includes successful registrations, failures, and expirations.
    /// 
    /// # Arguments
    /// 
    /// * `status_info` - Information about the registration status change
    async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo);
    
    /// Handle media events (optional - default implementation does nothing)
    /// 
    /// Called when media-related events occur during calls. Override this
    /// method to handle audio start/stop, quality changes, DTMF, etc.
    /// 
    /// # Arguments
    /// 
    /// * `media_info` - Information about the media event
    async fn on_media_event(&self, _media_info: MediaEventInfo) {
        // Default implementation - can be overridden for media event handling
    }
    
    /// Handle client errors (optional - default implementation logs)
    /// 
    /// Called when client errors occur. Override this method to implement
    /// custom error handling, logging, or recovery strategies.
    /// 
    /// # Arguments
    /// 
    /// * `error` - The error that occurred
    /// * `call_id` - Call ID associated with the error (if any)
    async fn on_client_error(&self, _error: crate::ClientError, _call_id: Option<CallId>) {
        // Default implementation - can be overridden for error handling
    }
    
    /// Handle network events (optional - default implementation does nothing)
    /// 
    /// Called when network connectivity changes are detected. Override this
    /// method to handle connection state changes and implement reconnection logic.
    /// 
    /// # Arguments
    /// 
    /// * `connected` - Whether the network is now connected
    /// * `reason` - Reason for the connectivity change (if known)
    async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {
        // Default implementation - can be overridden for network event handling
    }
    
    /// Handle comprehensive client events with filtering
    /// 
    /// This is the unified event handler that dispatches to specific event
    /// handling methods. Generally, you don't need to override this method
    /// unless you want custom event routing logic.
    /// 
    /// # Arguments
    /// 
    /// * `event` - The client event to handle
    async fn on_client_event(&self, event: ClientEvent) {
        match event {
            ClientEvent::IncomingCall { info, .. } => {
                self.on_incoming_call(info).await;
            }
            ClientEvent::CallStateChanged { info, .. } => {
                self.on_call_state_changed(info).await;
            }
            ClientEvent::MediaEvent { info, .. } => {
                self.on_media_event(info).await;
            }
            ClientEvent::RegistrationStatusChanged { info, .. } => {
                self.on_registration_status_changed(info).await;
            }
            ClientEvent::ClientError { error, call_id, .. } => {
                self.on_client_error(error, call_id).await;
            }
            ClientEvent::NetworkEvent { connected, reason, .. } => {
                self.on_network_event(connected, reason).await;
            }
            ClientEvent::IncomingTransferRequest { .. } => {
                // Default implementation does nothing for transfer requests
                // Apps can override ClientEventHandler to handle these
            }
            ClientEvent::TransferProgress { .. } => {
                // Default implementation does nothing for transfer progress
                // Apps can override ClientEventHandler to handle these
            }
        }
    }
}

/// Enhanced event subscription with filtering capabilities
/// 
/// Represents a subscription to client events with optional filtering.
/// Subscriptions determine which events are delivered to which handlers.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::{EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
/// use async_trait::async_trait;
/// use std::sync::Arc;
/// 
/// struct TestHandler;
/// 
/// #[async_trait]
/// impl ClientEventHandler for TestHandler {
///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
///         CallAction::Accept
///     }
///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
/// }
/// 
/// let handler = Arc::new(TestHandler);
/// let subscription = EventSubscription::all_events(handler);
/// ```
pub struct EventSubscription {
    /// The event handler that will receive events
    handler: Arc<dyn ClientEventHandler>,
    /// Filter criteria for this subscription
    filter: EventFilter,
    /// Unique identifier for this subscription
    id: uuid::Uuid,
}

impl EventSubscription {
    /// Create a new event subscription with filtering
    /// 
    /// Creates a subscription that will deliver events matching the specified
    /// filter criteria to the provided handler.
    /// 
    /// # Arguments
    /// 
    /// * `handler` - The event handler that will receive matching events
    /// * `filter` - Filter criteria to determine which events to receive
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventSubscription, EventFilter, EventPriority, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct TestHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for TestHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let handler = Arc::new(TestHandler);
    /// let filter = EventFilter {
    ///     min_priority: Some(EventPriority::High),
    ///     call_ids: None,
    ///     call_states: None,
    ///     media_event_types: None,
    ///     registration_ids: None,
    /// };
    /// let subscription = EventSubscription::new(handler, filter);
    /// ```
    pub fn new(handler: Arc<dyn ClientEventHandler>, filter: EventFilter) -> Self {
        Self {
            handler,
            filter,
            id: uuid::Uuid::new_v4(),
        }
    }
    
    /// Create a subscription that receives all events
    /// 
    /// Creates a subscription with no filtering - all events will be
    /// delivered to the handler.
    /// 
    /// # Arguments
    /// 
    /// * `handler` - The event handler that will receive all events
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct AllEventsHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for AllEventsHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let handler = Arc::new(AllEventsHandler);
    /// let subscription = EventSubscription::all_events(handler);
    /// ```
    pub fn all_events(handler: Arc<dyn ClientEventHandler>) -> Self {
        Self::new(handler, EventFilter::default())
    }
    
    /// Create a subscription for specific call events only
    /// 
    /// Creates a subscription that only receives events related to a
    /// specific call ID.
    /// 
    /// # Arguments
    /// 
    /// * `handler` - The event handler that will receive call events
    /// * `call_id` - The specific call ID to monitor
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use rvoip_client_core::call::CallId;
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct CallSpecificHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for CallSpecificHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let handler = Arc::new(CallSpecificHandler);
    /// let call_id = uuid::Uuid::new_v4();
    /// let subscription = EventSubscription::call_events(handler, call_id);
    /// ```
    pub fn call_events(handler: Arc<dyn ClientEventHandler>, call_id: CallId) -> Self {
        let mut call_ids = HashSet::new();
        call_ids.insert(call_id);
        let filter = EventFilter {
            call_ids: Some(call_ids),
            ..Default::default()
        };
        Self::new(handler, filter)
    }
    
    /// Create a subscription for high priority events only
    /// 
    /// Creates a subscription that only receives high and critical priority events,
    /// filtering out normal and low priority events.
    /// 
    /// # Arguments
    /// 
    /// * `handler` - The event handler that will receive high priority events
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct HighPriorityHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for HighPriorityHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let handler = Arc::new(HighPriorityHandler);
    /// let subscription = EventSubscription::high_priority_events(handler);
    /// ```
    pub fn high_priority_events(handler: Arc<dyn ClientEventHandler>) -> Self {
        let filter = EventFilter {
            min_priority: Some(EventPriority::High),
            ..Default::default()
        };
        Self::new(handler, filter)
    }
    
    /// Get the subscription ID
    /// 
    /// Returns the unique identifier for this subscription, which can be
    /// used to unsubscribe later.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct TestHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for TestHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let handler = Arc::new(TestHandler);
    /// let subscription = EventSubscription::all_events(handler);
    /// let id = subscription.id();
    /// println!("Subscription ID: {}", id);
    /// ```
    pub fn id(&self) -> uuid::Uuid {
        self.id
    }
    
    /// Check if this subscription should receive the given event
    /// 
    /// Tests whether the event matches this subscription's filter criteria.
    /// 
    /// # Arguments
    /// 
    /// * `event` - The event to test against the filter
    /// 
    /// # Returns
    /// 
    /// `true` if the event should be delivered to this subscription's handler
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventSubscription, ClientEvent, EventPriority, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct TestHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for TestHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let handler = Arc::new(TestHandler);
    /// let subscription = EventSubscription::high_priority_events(handler);
    /// 
    /// let high_priority_event = ClientEvent::NetworkEvent {
    ///     connected: false,
    ///     reason: Some("Connection lost".to_string()),
    ///     priority: EventPriority::High,
    /// };
    /// 
    /// assert!(subscription.should_receive(&high_priority_event));
    /// ```
    pub fn should_receive(&self, event: &ClientEvent) -> bool {
        event.passes_filter(&self.filter)
    }
    
    /// Deliver an event to this subscription's handler
    /// 
    /// Delivers the event to the handler if it passes the subscription's filter.
    /// 
    /// # Arguments
    /// 
    /// * `event` - The event to potentially deliver
    pub async fn deliver_event(&self, event: ClientEvent) {
        if self.should_receive(&event) {
            self.handler.on_client_event(event).await;
        }
    }
}

/// Event emission utilities for the ClientManager
/// 
/// Manages event subscriptions and handles event delivery to all matching
/// subscribers. This is the central hub for the event system.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::events::{EventEmitter, EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
/// use async_trait::async_trait;
/// use std::sync::Arc;
/// 
/// struct TestHandler;
/// 
/// #[async_trait]
/// impl ClientEventHandler for TestHandler {
///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
///         CallAction::Accept
///     }
///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
/// }
/// 
/// let emitter = EventEmitter::new();
/// let handler = Arc::new(TestHandler);
/// let subscription = EventSubscription::all_events(handler);
/// let subscription_id = emitter.subscribe(subscription);
/// 
/// assert_eq!(emitter.subscription_count(), 1);
/// ```
pub struct EventEmitter {
    /// List of active event subscriptions
    subscriptions: std::sync::RwLock<Vec<EventSubscription>>,
}

impl EventEmitter {
    /// Create a new event emitter
    /// 
    /// Creates a new event emitter with no active subscriptions.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::EventEmitter;
    /// 
    /// let emitter = EventEmitter::new();
    /// assert_eq!(emitter.subscription_count(), 0);
    /// ```
    pub fn new() -> Self {
        Self {
            subscriptions: std::sync::RwLock::new(Vec::new()),
        }
    }
    
    /// Add an event subscription
    /// 
    /// Registers a new event subscription with the emitter. The subscription
    /// will start receiving matching events immediately.
    /// 
    /// # Arguments
    /// 
    /// * `subscription` - The event subscription to add
    /// 
    /// # Returns
    /// 
    /// The unique ID of the subscription for later unsubscribing
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventEmitter, EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct TestHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for TestHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let emitter = EventEmitter::new();
    /// let handler = Arc::new(TestHandler);
    /// let subscription = EventSubscription::all_events(handler);
    /// let subscription_id = emitter.subscribe(subscription);
    /// ```
    pub fn subscribe(&self, subscription: EventSubscription) -> uuid::Uuid {
        let id = subscription.id();
        self.subscriptions.write().unwrap().push(subscription);
        id
    }
    
    /// Remove an event subscription
    /// 
    /// Unregisters an event subscription from the emitter. The subscription
    /// will stop receiving events.
    /// 
    /// # Arguments
    /// 
    /// * `subscription_id` - The unique ID of the subscription to remove
    /// 
    /// # Returns
    /// 
    /// `true` if the subscription was found and removed, `false` otherwise
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventEmitter, EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct TestHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for TestHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let emitter = EventEmitter::new();
    /// let handler = Arc::new(TestHandler);
    /// let subscription = EventSubscription::all_events(handler);
    /// let subscription_id = emitter.subscribe(subscription);
    /// 
    /// assert!(emitter.unsubscribe(subscription_id));
    /// assert_eq!(emitter.subscription_count(), 0);
    /// ```
    pub fn unsubscribe(&self, subscription_id: uuid::Uuid) -> bool {
        let mut subscriptions = self.subscriptions.write().unwrap();
        if let Some(pos) = subscriptions.iter().position(|s| s.id() == subscription_id) {
            subscriptions.remove(pos);
            true
        } else {
            false
        }
    }
    
    /// Emit an event to all matching subscriptions
    /// 
    /// Delivers the event to all subscriptions whose filters match the event.
    /// Event delivery happens asynchronously and in parallel.
    /// 
    /// # Arguments
    /// 
    /// * `event` - The event to emit to matching subscriptions
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventEmitter, ClientEvent, EventPriority};
    /// 
    /// # async fn example() {
    /// let emitter = EventEmitter::new();
    /// 
    /// let network_event = ClientEvent::NetworkEvent {
    ///     connected: true,
    ///     reason: Some("Connection restored".to_string()),
    ///     priority: EventPriority::Normal,
    /// };
    /// 
    /// emitter.emit(network_event).await;
    /// # }
    /// ```
    pub async fn emit(&self, event: ClientEvent) {
        let subscriptions = self.subscriptions.read().unwrap().clone();
        
        // Deliver event to all matching subscriptions in parallel
        let tasks: Vec<_> = subscriptions
            .into_iter()
            .map(|subscription| {
                let event_clone = event.clone();
                tokio::spawn(async move {
                    subscription.deliver_event(event_clone).await;
                })
            })
            .collect();
            
        // Wait for all deliveries to complete
        for task in tasks {
            if let Err(e) = task.await {
                tracing::error!("Error delivering event: {}", e);
            }
        }
    }
    
    /// Get the number of active subscriptions
    /// 
    /// Returns the current count of registered event subscriptions.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::events::{EventEmitter, EventSubscription, ClientEventHandler, IncomingCallInfo, CallAction, CallStatusInfo, RegistrationStatusInfo};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    /// 
    /// struct TestHandler;
    /// 
    /// #[async_trait]
    /// impl ClientEventHandler for TestHandler {
    ///     async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
    ///         CallAction::Accept
    ///     }
    ///     async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {}
    /// }
    /// 
    /// let emitter = EventEmitter::new();
    /// assert_eq!(emitter.subscription_count(), 0);
    /// 
    /// let handler = Arc::new(TestHandler);
    /// let subscription = EventSubscription::all_events(handler);
    /// emitter.subscribe(subscription);
    /// 
    /// assert_eq!(emitter.subscription_count(), 1);
    /// ```
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.read().unwrap().len()
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for EventSubscription {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            filter: self.filter.clone(),
            id: self.id,
        }
    }
} 