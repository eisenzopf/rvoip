//! Truly Simple API - Minimal wrapper over state machine
//!
//! This is the simplest possible API - just thin wrappers over the helpers.

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::unified::{UnifiedCoordinator, Config};
use crate::state_table::types::SessionId;
use crate::types::IncomingCallInfo;
use crate::errors::Result;
use rvoip_media_core::types::AudioFrame;

/// A simple SIP peer that can make and receive calls
pub struct SimplePeer {
    /// The coordinator that does all the work
    coordinator: Arc<UnifiedCoordinator>,
    
    /// Incoming call receiver
    incoming_rx: mpsc::Receiver<IncomingCallInfo>,
}

impl SimplePeer {
    /// Create a new peer with default configuration
    pub async fn new(name: &str) -> Result<Self> {
        Self::with_config(name, Config {
            sip_port: 5060,
            media_port_start: 6000,
            media_port_end: 7000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: "127.0.0.1:5060".parse().unwrap(),
            state_table_path: None,
        }).await
    }
    
    /// Create a new peer with custom configuration
    pub async fn with_config(name: &str, config: Config) -> Result<Self> {
        let coordinator = UnifiedCoordinator::new(config).await?;
        let (_tx, incoming_rx) = mpsc::channel(100);
        
        Ok(Self {
            coordinator,
            incoming_rx,
        })
    }
    
    // ===== Core Operations =====
    
    /// Make an outgoing call
    pub async fn call(&self, to: &str) -> Result<CallId> {
        let from = "sip:user@localhost"; // Simple default
        self.coordinator.make_call(from, to).await
    }
    
    /// Accept an incoming call
    pub async fn accept(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.accept_call(call_id).await
    }
    
    /// Reject an incoming call
    pub async fn reject(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.reject_call(call_id, "Busy").await
    }
    
    /// Hangup a call
    pub async fn hangup(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hangup(call_id).await
    }
    
    /// Put call on hold
    pub async fn hold(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.hold(call_id).await
    }
    
    /// Resume from hold
    pub async fn resume(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.resume(call_id).await
    }
    
    // ===== Incoming Calls =====
    
    /// Check for incoming call (non-blocking)
    pub async fn incoming_call(&mut self) -> Option<IncomingCall> {
        match self.incoming_rx.try_recv() {
            Ok(info) => Some(IncomingCall {
                id: info.session_id,
                from: info.from,
                to: info.to,
            }),
            Err(_) => None,
        }
    }
    
    /// Wait for incoming call (blocking)
    pub async fn wait_for_call(&mut self) -> Result<IncomingCall> {
        match self.incoming_rx.recv().await {
            Some(info) => Ok(IncomingCall {
                id: info.session_id,
                from: info.from,
                to: info.to,
            }),
            None => Err(crate::errors::SessionError::Other("Channel closed".to_string())),
        }
    }
    
    // ===== Audio =====
    
    /// Send audio to a call
    pub async fn send_audio(&self, call_id: &CallId, frame: AudioFrame) -> Result<()> {
        self.coordinator.send_audio(call_id, frame).await
    }
    
    /// Subscribe to receive audio from a call
    pub async fn subscribe_audio(
        &self,
        call_id: &CallId,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        self.coordinator.subscribe_to_audio(call_id).await
    }
    
    // ===== Advanced Features =====
    
    /// Send DTMF digit
    pub async fn send_dtmf(&self, call_id: &CallId, digit: char) -> Result<()> {
        self.coordinator.send_dtmf(call_id, digit).await
    }
    
    /// Blind transfer
    pub async fn transfer(&self, call_id: &CallId, target: &str) -> Result<()> {
        self.coordinator.blind_transfer(call_id, target).await
    }
    
    /// Start recording
    pub async fn start_recording(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.start_recording(call_id).await
    }
    
    /// Stop recording
    pub async fn stop_recording(&self, call_id: &CallId) -> Result<()> {
        self.coordinator.stop_recording(call_id).await
    }
    
    // ===== Conference =====
    
    /// Create conference from existing call
    pub async fn create_conference(&self, call_id: &CallId, name: &str) -> Result<()> {
        self.coordinator.create_conference(call_id, name).await
    }
    
    /// Add participant to conference
    pub async fn add_to_conference(&self, host_id: &CallId, participant_id: &CallId) -> Result<()> {
        self.coordinator.add_to_conference(host_id, participant_id).await
    }
}

/// Represents a call ID (just a SessionId)
pub type CallId = SessionId;

/// Represents an incoming call
#[derive(Debug, Clone)]
pub struct IncomingCall {
    /// The call ID to use for operations
    pub id: CallId,
    /// Who is calling
    pub from: String,
    /// Who they're calling
    pub to: String,
}

impl IncomingCall {
    /// Accept this call
    pub async fn accept(&self, peer: &SimplePeer) -> Result<()> {
        peer.accept(&self.id).await
    }
    
    /// Reject this call
    pub async fn reject(&self, peer: &SimplePeer) -> Result<()> {
        peer.reject(&self.id).await
    }
}
