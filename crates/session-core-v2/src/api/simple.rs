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
    
    /// Local SIP URI
    local_uri: String,
}

impl SimplePeer {
    /// Create a new peer with default configuration
    pub async fn new(name: &str) -> Result<Self> {
        let mut config = Config::default();
        config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        Self::with_config(name, config).await
    }
    
    /// Create a new peer with custom configuration
    pub async fn with_config(name: &str, mut config: Config) -> Result<Self> {
        // Update local_uri if not explicitly set
        if config.local_uri.starts_with("sip:user@") {
            config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        }
        let local_uri = config.local_uri.clone();
        let coordinator = UnifiedCoordinator::new(config).await?;
        
        // Create a channel to bridge incoming calls from coordinator
        let (tx, incoming_rx) = mpsc::channel(100);
        
        // Spawn a task to forward incoming calls from coordinator to our channel
        let coord_clone = coordinator.clone();
        tokio::spawn(async move {
            loop {
                if let Some(call_info) = coord_clone.get_incoming_call().await {
                    let _ = tx.send(call_info).await;
                }
            }
        });
        
        Ok(Self {
            coordinator,
            incoming_rx,
            local_uri,
        })
    }
    
    // ===== Core Operations =====
    
    /// Make an outgoing call
    pub async fn call(&self, to: &str) -> Result<CallId> {
        self.coordinator.make_call(&self.local_uri, to).await
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
