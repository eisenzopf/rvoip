//! Event handling - Simple event system for SIP client callbacks
//!
//! This module provides a clean event system for UI integration and
//! asynchronous event handling.

use rvoip_client_core::{CallId, CallState};

/// Events emitted by the SIP client
#[derive(Debug, Clone)]
pub enum SipEvent {
    /// An incoming call was received
    IncomingCall {
        call_id: CallId,
        caller_uri: String,
        caller_name: Option<String>,
    },

    /// A call's state changed
    CallStateChanged {
        call_id: CallId,
        old_state: CallState,
        new_state: CallState,
    },

    /// Registration status changed
    RegistrationChanged {
        domain: String,
        registered: bool,
        message: Option<String>,
    },

    /// Network connectivity changed
    NetworkStatusChanged {
        connected: bool,
        message: String,
    },

    /// Media event (audio started/stopped, etc.)
    MediaEvent {
        call_id: Option<CallId>,
        event_type: MediaEventType,
        description: String,
    },

    /// An error occurred
    Error {
        message: String,
        recoverable: bool,
    },
}

/// Types of media events
#[derive(Debug, Clone)]
pub enum MediaEventType {
    /// Audio stream started
    AudioStarted,
    /// Audio stream stopped
    AudioStopped,
    /// Microphone muted/unmuted
    MicrophoneToggled { muted: bool },
    /// Speaker muted/unmuted
    SpeakerToggled { muted: bool },
    /// DTMF tone detected or sent
    DtmfTone { digit: char },
}

/// Specific call-related events
#[derive(Debug, Clone)]
pub enum CallEvent {
    /// Call is ringing
    Ringing { call_id: CallId },
    /// Call was answered
    Answered { call_id: CallId },
    /// Call was hung up
    HungUp { call_id: CallId },
    /// Call failed
    Failed { call_id: CallId, reason: String },
} 