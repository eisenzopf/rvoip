//! Event handling for client-core operations

use std::sync::Arc;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use async_trait::async_trait;
use std::collections::HashSet;

use crate::call::{CallId, CallState};

/// Action to take for an incoming call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallAction {
    /// Accept the incoming call
    Accept,
    /// Reject the incoming call
    Reject,
    /// Ignore the incoming call (let it ring)
    Ignore,
}

/// Information about an incoming call
#[derive(Debug, Clone)]
pub struct IncomingCallInfo {
    /// Unique call identifier
    pub call_id: CallId,
    /// URI of the caller
    pub caller_uri: String,
    /// URI of the callee (local user)
    pub callee_uri: String,
    /// Display name of the caller (if available)
    pub caller_display_name: Option<String>,
    /// Call subject/reason (if provided)
    pub subject: Option<String>,
    /// When the call was received
    pub created_at: DateTime<Utc>,
}

/// Information about a call state change
#[derive(Debug, Clone)]
pub struct CallStatusInfo {
    /// Call that changed state
    pub call_id: CallId,
    /// New call state
    pub new_state: CallState,
    /// Previous call state (if known)
    pub previous_state: Option<CallState>,
    /// Reason for the state change (if available)
    pub reason: Option<String>,
    /// When the state change occurred
    pub timestamp: DateTime<Utc>,
}

/// Information about registration status changes
#[derive(Debug, Clone)]
pub struct RegistrationStatusInfo {
    /// Registration identifier
    pub registration_id: uuid::Uuid,
    /// Registration server URI
    pub server_uri: String,
    /// User URI being registered
    pub user_uri: String,
    /// New registration status
    pub status: crate::registration::RegistrationStatus,
    /// Status change reason
    pub reason: Option<String>,
    /// When the status changed
    pub timestamp: DateTime<Utc>,
}

/// Types of media events that can occur
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaEventType {
    // Phase 4.1: Enhanced Media Integration
    MicrophoneStateChanged { muted: bool },
    SpeakerStateChanged { muted: bool },
    AudioStarted,
    AudioStopped,
    HoldStateChanged { on_hold: bool },
    DtmfSent { digits: String },
    TransferInitiated { target: String, transfer_type: String },
    
    // Phase 4.2: Media Session Coordination
    SdpOfferGenerated { sdp_size: usize },
    SdpAnswerProcessed { sdp_size: usize },
    MediaSessionStarted { media_session_id: String },
    MediaSessionStopped,
    MediaSessionUpdated { sdp_size: usize },
    
    // Quality and monitoring events (using integers for Hash/Eq compatibility)
    QualityChanged { mos_score_x100: u32 }, // MOS score * 100 (e.g., 425 = 4.25)
    PacketLoss { percentage_x100: u32 }, // Percentage * 100 (e.g., 150 = 1.5%)
    JitterChanged { jitter_ms: u32 },
}

/// Media event information
#[derive(Debug, Clone)]
pub struct MediaEventInfo {
    /// Call the media event relates to
    pub call_id: CallId,
    /// Type of media event
    pub event_type: MediaEventType,
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Additional event metadata
    pub metadata: std::collections::HashMap<String, String>,
}

/// Event filtering options for selective subscription
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Only receive events for specific calls
    pub call_ids: Option<HashSet<CallId>>,
    /// Only receive specific types of call state changes
    pub call_states: Option<HashSet<CallState>>,
    /// Only receive specific types of media events
    pub media_event_types: Option<HashSet<MediaEventType>>,
    /// Only receive events for specific registration IDs
    pub registration_ids: Option<HashSet<uuid::Uuid>>,
    /// Minimum event priority level
    pub min_priority: Option<EventPriority>,
}

/// Event priority levels for filtering
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventPriority {
    /// Low priority events (quality updates, routine status)
    Low,
    /// Normal priority events (state changes, DTMF)
    Normal,
    /// High priority events (incoming calls, errors)
    High,
    /// Critical priority events (failures, security issues)
    Critical,
}

/// Comprehensive client event types
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// Incoming call received
    IncomingCall {
        info: IncomingCallInfo,
        priority: EventPriority,
    },
    /// Call state changed
    CallStateChanged {
        /// Information about the state change
        info: CallStatusInfo,
        /// Priority of this event  
        priority: EventPriority,
    },
    /// Media event occurred
    MediaEvent {
        info: MediaEventInfo,
        priority: EventPriority,
    },
    /// Registration status changed
    RegistrationStatusChanged {
        info: RegistrationStatusInfo,
        priority: EventPriority,
    },
    /// Client error occurred
    ClientError {
        error: crate::ClientError,
        call_id: Option<CallId>,
        priority: EventPriority,
    },
    /// Network connectivity changed
    NetworkEvent {
        connected: bool,
        reason: Option<String>,
        priority: EventPriority,
    },
}

impl ClientEvent {
    /// Get the priority of this event
    pub fn priority(&self) -> EventPriority {
        match self {
            ClientEvent::IncomingCall { priority, .. } => priority.clone(),
            ClientEvent::CallStateChanged { priority, .. } => priority.clone(),
            ClientEvent::MediaEvent { priority, .. } => priority.clone(),
            ClientEvent::RegistrationStatusChanged { priority, .. } => priority.clone(),
            ClientEvent::ClientError { priority, .. } => priority.clone(),
            ClientEvent::NetworkEvent { priority, .. } => priority.clone(),
        }
    }
    
    /// Get the call ID associated with this event (if any)
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
#[async_trait]
pub trait ClientEventHandler: Send + Sync {
    /// Handle an incoming call with action decision
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction;
    
    /// Handle call state changes
    async fn on_call_state_changed(&self, status_info: CallStatusInfo);
    
    /// Handle registration status changes
    async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo);
    
    /// Handle media events (optional - default implementation does nothing)
    async fn on_media_event(&self, _media_info: MediaEventInfo) {
        // Default implementation - can be overridden for media event handling
    }
    
    /// Handle client errors (optional - default implementation logs)
    async fn on_client_error(&self, _error: crate::ClientError, _call_id: Option<CallId>) {
        // Default implementation - can be overridden for error handling
    }
    
    /// Handle network events (optional - default implementation does nothing)
    async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {
        // Default implementation - can be overridden for network event handling
    }
    
    /// Handle comprehensive client events with filtering
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
        }
    }
}

/// Enhanced event subscription with filtering capabilities
pub struct EventSubscription {
    handler: Arc<dyn ClientEventHandler>,
    filter: EventFilter,
    id: uuid::Uuid,
}

impl EventSubscription {
    /// Create a new event subscription with filtering
    pub fn new(handler: Arc<dyn ClientEventHandler>, filter: EventFilter) -> Self {
        Self {
            handler,
            filter,
            id: uuid::Uuid::new_v4(),
        }
    }
    
    /// Create a subscription that receives all events
    pub fn all_events(handler: Arc<dyn ClientEventHandler>) -> Self {
        Self::new(handler, EventFilter::default())
    }
    
    /// Create a subscription for specific call events only
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
    pub fn high_priority_events(handler: Arc<dyn ClientEventHandler>) -> Self {
        let filter = EventFilter {
            min_priority: Some(EventPriority::High),
            ..Default::default()
        };
        Self::new(handler, filter)
    }
    
    /// Get the subscription ID
    pub fn id(&self) -> uuid::Uuid {
        self.id
    }
    
    /// Check if this subscription should receive the given event
    pub fn should_receive(&self, event: &ClientEvent) -> bool {
        event.passes_filter(&self.filter)
    }
    
    /// Deliver an event to this subscription's handler
    pub async fn deliver_event(&self, event: ClientEvent) {
        if self.should_receive(&event) {
            self.handler.on_client_event(event).await;
        }
    }
}

/// Event emission utilities for the ClientManager
pub struct EventEmitter {
    subscriptions: std::sync::RwLock<Vec<EventSubscription>>,
}

impl EventEmitter {
    /// Create a new event emitter
    pub fn new() -> Self {
        Self {
            subscriptions: std::sync::RwLock::new(Vec::new()),
        }
    }
    
    /// Add an event subscription
    pub fn subscribe(&self, subscription: EventSubscription) -> uuid::Uuid {
        let id = subscription.id();
        self.subscriptions.write().unwrap().push(subscription);
        id
    }
    
    /// Remove an event subscription
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