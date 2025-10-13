//! Truly Simple API - Minimal wrapper over state machine
//!
//! This is the simplest possible API - just thin wrappers over the helpers.

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::unified::UnifiedCoordinator;
use crate::state_table::types::SessionId;
use crate::types::IncomingCallInfo;
use crate::errors::Result;

// Re-export types that users of SimplePeer will need
pub use rvoip_media_core::types::AudioFrame;
pub use crate::api::unified::Config;

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

        // Enable automatic blind transfer handling
        // This makes transfers "just work" for SimplePeer users
        coordinator.enable_auto_transfer();

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
    
    /// Initiate a blind transfer (transferor side - Bob)
    /// This sends REFER to the peer, asking them to call the target
    pub async fn transfer(&self, call_id: &CallId, target: &str) -> Result<()> {
        self.coordinator.blind_transfer(call_id, target).await
    }

    /// Complete a blind transfer (transferee side - Alice)
    /// Call this when you receive a transfer request to actually make the call
    /// Returns the new call ID to the transfer target
    pub async fn complete_transfer(&self, call_id: &CallId, target: &str) -> Result<CallId> {
        self.coordinator.complete_blind_transfer(call_id, target).await
    }

    /// Start attended transfer (returns consultation call ID)
    pub async fn start_attended_transfer(&self, call_id: &CallId, target: &str) -> Result<CallId> {
        let session_id = SessionId(call_id.to_string());
        let consultation_session_id = self.coordinator.start_attended_transfer(&session_id, target).await?;
        // CallId is SessionId in this module
        Ok(consultation_session_id)
    }

    /// Complete attended transfer
    pub async fn complete_attended_transfer(&self, call_id: &CallId) -> Result<()> {
        let session_id = SessionId(call_id.to_string());
        self.coordinator.complete_attended_transfer(&session_id).await
    }

    /// Cancel attended transfer
    pub async fn cancel_attended_transfer(&self, call_id: &CallId) -> Result<()> {
        let session_id = SessionId(call_id.to_string());
        self.coordinator.cancel_attended_transfer(&session_id).await
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

    // ===== Session Management =====

    /// Get the most recent active call ID
    /// This is useful after a transfer where a new session is created
    /// Returns None if no active calls exist
    pub async fn get_latest_call_id(&self) -> Option<CallId> {
        let sessions = self.coordinator.list_sessions().await;

        tracing::info!("[get_latest_call_id] Total sessions: {}", sessions.len());
        for s in &sessions {
            tracing::info!("[get_latest_call_id] Session {}: state={:?}, is_final={}, is_in_progress={}",
                s.session_id, s.state, s.state.is_final(), s.state.is_in_progress());
        }

        // Find the most recently created session that's NOT final
        // Don't require is_in_progress() - just exclude terminated sessions
        let result = sessions.into_iter()
            .filter(|s| !s.state.is_final())
            .max_by_key(|s| s.start_time)
            .map(|s| s.session_id);

        tracing::info!("[get_latest_call_id] Result: {:?}", result);
        result
    }

    /// Get active call ID for a specific remote party
    /// This is useful after a transfer to find the new session with the transfer target
    pub async fn get_call_id_for(&self, remote_uri: &str) -> Option<CallId> {
        let sessions = self.coordinator.list_sessions().await;

        // Normalize the remote URI for comparison (remove display name, params, etc)
        let normalized_target = Self::normalize_uri(remote_uri);

        tracing::info!("[get_call_id_for] Looking for remote_uri: {}", remote_uri);
        tracing::info!("[get_call_id_for] Normalized target: {}", normalized_target);
        tracing::info!("[get_call_id_for] Found {} total sessions", sessions.len());

        for s in &sessions {
            tracing::info!("[get_call_id_for] Session {}: state={:?}, to={}", s.session_id, s.state, s.to);
            let normalized_remote = Self::normalize_uri(&s.to);
            tracing::info!("[get_call_id_for]   Normalized remote: {}", normalized_remote);
        }

        // Find active session with matching remote URI
        // Note: Don't filter by state - include all sessions that match the URI
        // This allows finding sessions in Initiating, Ringing, or Active states
        let result = sessions.into_iter()
            .filter(|s| !s.state.is_final()) // Only exclude terminated/failed sessions
            .find(|s| {
                let normalized_remote = Self::normalize_uri(&s.to);
                normalized_remote == normalized_target
            })
            .map(|s| s.session_id);

        tracing::info!("[get_call_id_for] Result: {:?}", result);
        result
    }

    /// Helper to normalize SIP URI for comparison
    /// Extracts just the user@host:port part
    fn normalize_uri(uri: &str) -> String {
        // Simple normalization: extract URI from "Display Name <sip:user@host:port>;params"
        if let Some(start) = uri.find("sip:") {
            let uri_part = &uri[start..];
            if let Some(end) = uri_part.find('>') {
                uri_part[..end].to_string()
            } else if let Some(end) = uri_part.find(';') {
                uri_part[..end].to_string()
            } else {
                uri_part.to_string()
            }
        } else {
            uri.to_string()
        }
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
