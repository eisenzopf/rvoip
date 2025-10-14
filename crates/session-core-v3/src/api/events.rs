//! Event types for SimplePeer API
//!
//! This module defines the event-driven architecture that replaces callbacks
//! for a truly simple API experience.

use crate::state_table::types::SessionId;
use tokio::sync::mpsc;
use crate::errors::Result;

/// Type alias for call ID (same as SessionId)
pub type CallId = SessionId;

/// Handle for managing a specific call
/// 
/// Provides audio channels and call identification for a specific call session.
/// Each call gets its own handle with dedicated audio send/receive channels.
#[derive(Debug)]
pub struct CallHandle {
    /// The call ID for this handle
    call_id: CallId,
    /// Channel for sending audio to this call
    audio_tx: mpsc::Sender<Vec<i16>>,
    /// Channel for receiving audio from this call
    audio_rx: mpsc::Receiver<Vec<i16>>,
}

impl CallHandle {
    /// Create a new call handle
    pub fn new(call_id: CallId) -> (Self, mpsc::Receiver<Vec<i16>>, mpsc::Sender<Vec<i16>>) {
        let (audio_tx, audio_rx_for_handle) = mpsc::channel(100);
        let (audio_tx_for_coordinator, audio_rx) = mpsc::channel(100);
        
        let handle = Self {
            call_id,
            audio_tx,
            audio_rx,
        };
        
        (handle, audio_rx_for_handle, audio_tx_for_coordinator)
    }
    
    /// Get the call ID for this handle
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }
    
    /// Send audio samples to this call
    /// 
    /// # Arguments
    /// * `samples` - PCM audio samples (16-bit, mono, 8kHz)
    /// 
    /// # Example
    /// ```rust,no_run
    /// let samples = vec![100, 200, 300]; // Simple audio data
    /// call_handle.send_audio(samples).await?;
    /// ```
    pub async fn send_audio(&mut self, samples: Vec<i16>) -> Result<()> {
        self.audio_tx.send(samples).await
            .map_err(|_| crate::errors::SessionError::Other("Audio channel closed".to_string()))?;
        Ok(())
    }
    
    /// Receive audio samples from this call (non-blocking)
    /// 
    /// # Returns
    /// * `Some(samples)` - Audio data received from remote party
    /// * `None` - No audio available right now
    /// 
    /// # Example
    /// ```rust,no_run
    /// if let Some(samples) = call_handle.recv_audio().await {
    ///     // Play or process the received audio
    ///     println!("Received {} audio samples", samples.len());
    /// }
    /// ```
    pub async fn recv_audio(&mut self) -> Option<Vec<i16>> {
        self.audio_rx.recv().await
    }
    
    /// Try to receive audio samples (non-blocking)
    /// 
    /// # Returns
    /// * `Ok(samples)` - Audio data received
    /// * `Err(TryRecvError::Empty)` - No audio available
    /// * `Err(TryRecvError::Disconnected)` - Call ended
    pub fn try_recv_audio(&mut self) -> std::result::Result<Vec<i16>, mpsc::error::TryRecvError> {
        self.audio_rx.try_recv()
    }
    
    /// Check if the call handle is still connected
    pub fn is_connected(&self) -> bool {
        !self.audio_tx.is_closed() && !self.audio_rx.is_closed()
    }
}

/// Events that SimplePeer can receive
/// 
/// These events are published by the state machine when SIP protocol
/// events occur. Developers handle these events to implement business logic.
#[derive(Debug, Clone)]
pub enum Event {
    // ===== Call Lifecycle Events =====
    
    /// Incoming call received
    /// 
    /// The state machine has already sent 180 Ringing. Developer must
    /// call `accept()` or `reject()` to complete the call handling.
    IncomingCall {
        call_id: CallId,
        from: String,
        to: String,
        sdp: Option<String>,
    },
    
    /// Call was answered (200 OK received for outgoing call)
    CallAnswered {
        call_id: CallId,
        sdp: Option<String>,
    },
    
    /// Call ended (BYE sent/received)
    CallEnded {
        call_id: CallId,
        reason: String,
    },
    
    /// Call failed (4xx/5xx response or timeout)
    CallFailed {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    
    // ===== Transfer Events =====
    
    /// REFER request received
    /// 
    /// The state machine has already sent 202 Accepted. Developer can
    /// create a new session to the transfer target or ignore the transfer.
    ReferReceived {
        call_id: CallId,
        refer_to: String,
        referred_by: Option<String>,
        replaces: Option<String>,
        transaction_id: String,    // For NOTIFY correlation
        transfer_type: String,      // "blind" or "attended"
    },
    
    /// Transfer accepted by recipient
    TransferAccepted {
        call_id: CallId,
        refer_to: String,
    },
    
    /// Transfer completed successfully
    TransferCompleted {
        old_call_id: CallId,
        new_call_id: CallId,
        target: String,
    },
    
    /// Transfer failed
    TransferFailed {
        call_id: CallId,
        reason: String,
        status_code: u16,
    },
    
    /// Transfer progress update (for transferor monitoring)
    TransferProgress {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    
    // ===== Call State Events =====
    
    /// Call was put on hold (re-INVITE with inactive SDP received)
    CallOnHold {
        call_id: CallId,
    },
    
    /// Call was resumed from hold
    CallResumed {
        call_id: CallId,
    },
    
    /// Call was muted locally
    CallMuted {
        call_id: CallId,
    },
    
    /// Call was unmuted locally
    CallUnmuted {
        call_id: CallId,
    },
    
    // ===== Media Events =====
    
    /// DTMF digit received
    DtmfReceived {
        call_id: CallId,
        digit: char,
    },
    
    /// Media quality changed
    MediaQualityChanged {
        call_id: CallId,
        packet_loss_percent: u32,
        jitter_ms: u32,
    },
    
    // ===== Error Events =====
    
    /// Network error occurred
    NetworkError {
        call_id: Option<CallId>,
        error: String,
    },
    
    /// Authentication required (401/407 response)
    AuthenticationRequired {
        call_id: CallId,
        realm: String,
    },
}

impl Event {
    /// Get the call ID associated with this event (if any)
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            Event::IncomingCall { call_id, .. } |
            Event::CallAnswered { call_id, .. } |
            Event::CallEnded { call_id, .. } |
            Event::CallFailed { call_id, .. } |
            Event::ReferReceived { call_id, .. } |
            Event::TransferAccepted { call_id, .. } |
            Event::TransferFailed { call_id, .. } |
            Event::TransferProgress { call_id, .. } |
            Event::CallOnHold { call_id, .. } |
            Event::CallResumed { call_id, .. } |
            Event::CallMuted { call_id, .. } |
            Event::CallUnmuted { call_id, .. } |
            Event::DtmfReceived { call_id, .. } |
            Event::MediaQualityChanged { call_id, .. } |
            Event::AuthenticationRequired { call_id, .. } => Some(call_id),
            Event::TransferCompleted { old_call_id, .. } => Some(old_call_id),
            Event::NetworkError { call_id, .. } => call_id.as_ref(),
        }
    }
    
    /// Check if this is a call-related event
    pub fn is_call_event(&self) -> bool {
        matches!(self, 
            Event::IncomingCall { .. } |
            Event::CallAnswered { .. } |
            Event::CallEnded { .. } |
            Event::CallFailed { .. }
        )
    }
    
    /// Check if this is a transfer-related event
    pub fn is_transfer_event(&self) -> bool {
        matches!(self, 
            Event::ReferReceived { .. } |
            Event::TransferAccepted { .. } |
            Event::TransferCompleted { .. } |
            Event::TransferFailed { .. } |
            Event::TransferProgress { .. }
        )
    }
    
    /// Check if this is a media-related event
    pub fn is_media_event(&self) -> bool {
        matches!(self, 
            Event::DtmfReceived { .. } |
            Event::MediaQualityChanged { .. }
        )
    }
}
