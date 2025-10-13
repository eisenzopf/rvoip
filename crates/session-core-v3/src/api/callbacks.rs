//! Callback types and infrastructure for session-core-v3
//!
//! This module defines the callback system that allows developers to handle
//! SIP events (like REFER) manually instead of relying on automatic processing.

use crate::state_table::types::SessionId;

// ===== Callback Function Types =====

/// Callback for handling REFER requests (call transfers)
pub type OnReferCallback = Box<dyn Fn(ReferEvent) -> CallbackResult + Send + Sync>;

/// Callback for handling incoming calls
pub type OnIncomingCallCallback = Box<dyn Fn(IncomingCallEvent) -> CallbackResult + Send + Sync>;

/// Callback for handling call termination events
pub type OnCallTerminatedCallback = Box<dyn Fn(CallTerminatedEvent) + Send + Sync>;

/// Callback for handling call answered events
pub type OnCallAnsweredCallback = Box<dyn Fn(CallAnsweredEvent) + Send + Sync>;

// ===== Event Structures =====

/// Event data for REFER requests
#[derive(Debug, Clone)]
pub struct ReferEvent {
    /// The URI to transfer to (from Refer-To header)
    pub refer_to: String,
    /// Optional Referred-By header (who initiated the transfer)
    pub referred_by: Option<String>,
    /// Optional Replaces header (for attended transfers)
    pub replaces: Option<String>,
}

/// Event data for incoming calls
#[derive(Debug, Clone)]
pub struct IncomingCallEvent {
    /// Who is calling (From header)
    pub from: String,
    /// Who they're calling (To header)
    pub to: String,
    /// Optional SDP offer
    pub sdp: Option<String>,
}

/// Event data for call termination
#[derive(Debug, Clone)]
pub struct CallTerminatedEvent {
    /// Session that was terminated
    pub session_id: SessionId,
    /// Reason for termination
    pub reason: String,
}

/// Event data for call answered
#[derive(Debug, Clone)]
pub struct CallAnsweredEvent {
    /// Session that was answered
    pub session_id: SessionId,
    /// Remote SDP answer
    pub sdp: Option<String>,
}

// ===== Callback Results =====

/// Result returned by callbacks to control state machine behavior
#[derive(Debug, Clone)]
pub enum CallbackResult {
    /// Accept the event and continue with default state machine behavior
    Accept,
    /// Reject the event with a specific SIP response code and reason
    Reject(u16, String),
    /// Callback has handled the event completely, skip default actions
    Handle,
}

// ===== Callback Registry =====

/// Registry that holds all registered callbacks
/// 
/// This uses Option<T> for each callback type, allowing developers to register
/// only the callbacks they need. Unregistered callbacks use default behavior.
#[derive(Default)]
pub struct CallbackRegistry {
    /// Callback for REFER requests
    pub on_refer: Option<OnReferCallback>,
    /// Callback for incoming calls
    pub on_incoming_call: Option<OnIncomingCallCallback>,
    /// Callback for call termination
    pub on_call_terminated: Option<OnCallTerminatedCallback>,
    /// Callback for call answered
    pub on_call_answered: Option<OnCallAnsweredCallback>,
}

impl CallbackRegistry {
    /// Create a new empty callback registry
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Check if any callbacks are registered
    pub fn has_callbacks(&self) -> bool {
        self.on_refer.is_some()
            || self.on_incoming_call.is_some()
            || self.on_call_terminated.is_some()
            || self.on_call_answered.is_some()
    }
    
    /// Clear all registered callbacks
    pub fn clear(&mut self) {
        self.on_refer = None;
        self.on_incoming_call = None;
        self.on_call_terminated = None;
        self.on_call_answered = None;
    }
}

// ===== Convenience Implementations =====

impl CallbackResult {
    /// Create an Accept result
    pub fn accept() -> Self {
        CallbackResult::Accept
    }
    
    /// Create a Reject result with common codes
    pub fn reject_busy() -> Self {
        CallbackResult::Reject(486, "Busy Here".to_string())
    }
    
    /// Create a Reject result for not implemented
    pub fn reject_not_implemented() -> Self {
        CallbackResult::Reject(501, "Not Implemented".to_string())
    }
    
    /// Create a Reject result for decline
    pub fn reject_decline() -> Self {
        CallbackResult::Reject(603, "Decline".to_string())
    }
    
    /// Create a Handle result
    pub fn handle() -> Self {
        CallbackResult::Handle
    }
}