//! Event system for client-core operations
//!
//! This module defines the event-driven architecture that allows UI layers
//! to integrate with the SIP client functionality.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::call::{CallId, CallState};
use crate::registration::RegistrationStatus;

/// Information about an incoming call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingCallInfo {
    /// Unique identifier for this call
    pub call_id: CallId,
    /// Who is calling (From header)
    pub caller_uri: String,
    /// Display name of caller (if available)
    pub caller_display_name: Option<String>,
    /// Who they're calling (To header)
    pub callee_uri: String,
    /// Subject/reason for the call
    pub subject: Option<String>,
    /// Source network address of the call
    pub source_addr: SocketAddr,
    /// When the call was received
    pub received_at: chrono::DateTime<chrono::Utc>,
}

/// Information about call state changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallStatusInfo {
    /// Call identifier
    pub call_id: CallId,
    /// New call state
    pub new_state: CallState,
    /// Previous call state (if available)
    pub previous_state: Option<CallState>,
    /// Additional information about the state change
    pub reason: Option<String>,
    /// When the state change occurred
    pub changed_at: chrono::DateTime<chrono::Utc>,
}

/// Information about registration status changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationStatusInfo {
    /// SIP server URI
    pub server_uri: String,
    /// User URI being registered
    pub user_uri: String,
    /// New registration status
    pub status: RegistrationStatus,
    /// Additional status information
    pub message: Option<String>,
    /// When the status changed
    pub changed_at: chrono::DateTime<chrono::Utc>,
}

/// Comprehensive client events that can be emitted
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// An incoming call was received
    IncomingCall(IncomingCallInfo),
    
    /// Call state changed
    CallStateChanged(CallStatusInfo),
    
    /// Registration status changed
    RegistrationStatusChanged(RegistrationStatusInfo),
    
    /// Network connectivity changed
    NetworkStatusChanged {
        connected: bool,
        server: String,
        message: Option<String>,
    },
    
    /// Audio/media event
    MediaEvent {
        call_id: Option<CallId>,
        event_type: MediaEventType,
        description: String,
    },
    
    /// Error occurred
    ErrorOccurred {
        error: String,
        recoverable: bool,
        context: Option<String>,
    },
}

/// Types of media events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaEventType {
    /// Audio started flowing
    AudioStarted,
    /// Audio stopped
    AudioStopped,
    /// Audio quality changed
    AudioQualityChanged,
    /// Microphone muted/unmuted
    MicrophoneStateChanged { muted: bool },
    /// Speaker muted/unmuted
    SpeakerStateChanged { muted: bool },
    /// Codec changed
    CodecChanged { codec: String },
}

/// Trait for handling client events
/// 
/// UI applications implement this trait to receive notifications about
/// SIP client events (incoming calls, registration status, etc.)
#[async_trait]
pub trait ClientEventHandler: Send + Sync {
    /// Called when an incoming call is received
    /// 
    /// The UI should prompt the user to accept or reject the call.
    /// Return value indicates the user's decision.
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction;
    
    /// Called when a call's state changes
    async fn on_call_state_changed(&self, status_info: CallStatusInfo);
    
    /// Called when registration status changes
    async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo);
    
    /// Called when network connectivity changes
    async fn on_network_status_changed(&self, connected: bool, server: String, message: Option<String>);
    
    /// Called for media-related events
    async fn on_media_event(&self, call_id: Option<CallId>, event_type: MediaEventType, description: String);
    
    /// Called when an error occurs
    async fn on_error(&self, error: String, recoverable: bool, context: Option<String>);
    
    /// Called to get user credentials for authentication
    /// 
    /// The UI should prompt the user for username/password and return them.
    /// Return None to cancel authentication.
    async fn get_credentials(&self, realm: String, server: String) -> Option<Credentials>;
}

/// User action in response to an incoming call
#[derive(Debug, Clone)]
pub enum CallAction {
    /// Accept the call
    Accept,
    /// Reject the call
    Reject,
    /// Forward the call to another number
    Forward { target: String },
    /// Send to voicemail (if available)
    Voicemail,
}

/// User credentials for authentication
#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// A no-op event handler for testing or minimal implementations
pub struct NoOpEventHandler;

#[async_trait]
impl ClientEventHandler for NoOpEventHandler {
    async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
        // Default: reject all calls
        CallAction::Reject
    }
    
    async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {
        // No-op
    }
    
    async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {
        // No-op
    }
    
    async fn on_network_status_changed(&self, _connected: bool, _server: String, _message: Option<String>) {
        // No-op
    }
    
    async fn on_media_event(&self, _call_id: Option<CallId>, _event_type: MediaEventType, _description: String) {
        // No-op
    }
    
    async fn on_error(&self, _error: String, _recoverable: bool, _context: Option<String>) {
        // No-op
    }
    
    async fn get_credentials(&self, _realm: String, _server: String) -> Option<Credentials> {
        // No credentials available
        None
    }
} 