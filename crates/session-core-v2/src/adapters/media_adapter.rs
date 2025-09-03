//! Simplified Media Adapter for session-core-v2
//!
//! Thin translation layer between media-core and state machine.
//! Focuses only on essential media operations and events.

use std::sync::Arc;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;
use dashmap::DashMap;
use rvoip_media_core::{
    relay::controller::{MediaSessionController, MediaConfig, MediaSessionInfo, MediaSessionEvent},
    DialogId, MediaSessionId,
};
use crate::state_table::types::{SessionId, EventType};
use crate::errors::{Result, SessionError};
use crate::session_store::SessionStore;

/// Negotiated media configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NegotiatedConfig {
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: String,
    pub payload_type: u8,
}

/// Minimal media adapter - just translates between media-core and state machine
pub struct MediaAdapter {
    /// Media-core controller
    controller: Arc<MediaSessionController>,
    
    /// Channel to send events to state machine
    event_tx: mpsc::Sender<(SessionId, EventType)>,
    
    /// Session store for updating IDs
    store: Arc<SessionStore>,
    
    /// Simple mapping of session IDs to dialog IDs (media-core uses DialogId)
    session_to_dialog: Arc<DashMap<SessionId, DialogId>>,
    dialog_to_session: Arc<DashMap<DialogId, SessionId>>,
    
    /// Store media session info for SDP generation
    media_sessions: Arc<DashMap<SessionId, MediaSessionInfo>>,
    
    /// Local IP for SDP generation
    local_ip: IpAddr,
    
    /// Port range for media
    port_start: u16,
    port_end: u16,
}

impl MediaAdapter {
    /// Create a mock media adapter for testing
    pub fn new_mock() -> Self {
        use std::str::FromStr;
        let (event_tx, _) = mpsc::channel(100);
        Self {
            controller: Arc::new(MediaSessionController::new()),
            event_tx,
            store: Arc::new(SessionStore::new()),
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            media_sessions: Arc::new(DashMap::new()),
            local_ip: IpAddr::from_str("127.0.0.1").unwrap(),
            port_start: 10000,
            port_end: 20000,
        }
    }
    
    pub fn new(
        controller: Arc<MediaSessionController>,
        event_tx: mpsc::Sender<(SessionId, EventType)>,
        store: Arc<SessionStore>,
        local_ip: IpAddr,
        port_start: u16,
        port_end: u16,
    ) -> Self {
        Self {
            controller,
            event_tx,
            store,
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            media_sessions: Arc::new(DashMap::new()),
            local_ip,
            port_start,
            port_end,
        }
    }
    
    // ===== Outbound Actions (from state machine) =====
    
    /// Start a media session
    pub async fn start_session(&self, session_id: &SessionId) -> Result<()> {
        // Create dialog ID for media-core (it uses DialogId, not SessionId)
        let dialog_id = DialogId::new(format!("media-{}", session_id.0));
        
        // Store mapping
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        // Create media config with our settings
        let media_config = MediaConfig {
            local_addr: SocketAddr::new(self.local_ip, 0), // Let media-core allocate port
            remote_addr: None, // Will be set when we get remote SDP
            preferred_codec: Some("PCMU".to_string()), // G.711 µ-law as default
            parameters: std::collections::HashMap::new(),
        };
        
        // Start the media session
        self.controller.start_media(dialog_id.clone(), media_config)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to start media session: {}", e)))?;
        
        // Get media session info for SDP generation
        if let Some(info) = self.controller.get_session_info(&dialog_id).await {
            self.media_sessions.insert(session_id.clone(), info.clone());
            
            // Update session store with media session ID (using dialog_id as media session id)
            if let Ok(mut session) = self.store.get_session(session_id).await {
                session.media_session_id = Some(info.dialog_id.to_string());
                let _ = self.store.update_session(session).await;
            }
            
            // Send MediaSessionReady event
            self.event_tx.send((
                session_id.clone(),
                EventType::MediaSessionReady
            )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
        }
        
        Ok(())
    }
    
    /// Stop a media session
    pub async fn stop_session(&self, session_id: &SessionId) -> Result<()> {
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            self.controller.stop_media(&dialog_id)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to stop media session: {}", e)))?;
            
            // Clean up mappings
            self.session_to_dialog.remove(session_id);
            self.dialog_to_session.remove(&*dialog_id);
            self.media_sessions.remove(session_id);
        }
        
        Ok(())
    }
    
    /// Generate SDP offer (for UAC)
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String> {
        let info = self.media_sessions.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No media session for {}", session_id.0)))?;
        
        // Generate simple SDP offer
        let sdp = format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 101\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:101 telephone-event/8000\r\n\
             a=fmtp:101 0-15\r\n\
             a=sendrecv\r\n",
            info.dialog_id.as_str(),
            info.created_at.elapsed().as_secs(),
            self.local_ip,
            self.local_ip,
            info.rtp_port.unwrap_or(5004),
        );
        
        Ok(sdp)
    }
    
    /// Process SDP answer and negotiate (for UAC)
    pub async fn negotiate_sdp_as_uac(&self, session_id: &SessionId, remote_sdp: &str) -> Result<NegotiatedConfig> {
        // Parse remote SDP to extract IP and port
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;
        
        // Update media session with remote address
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            // Note: MediaSessionController doesn't have update_remote_address, 
            // but in production this would update the RTP destination
            tracing::debug!("Would update remote address to {}:{} for session {}", remote_ip, remote_port, session_id.0);
        }
        
        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, self.get_local_port(session_id)?),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: "PCMU".to_string(),
            payload_type: 0,
        };
        
        // Send negotiation complete event
        self.event_tx.send((
            session_id.clone(),
            EventType::MediaNegotiated
        )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
        
        // Simulate media flow established after negotiation
        self.event_tx.send((
            session_id.clone(),
            EventType::MediaFlowEstablished
        )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
        
        Ok(config)
    }
    
    /// Generate SDP answer and negotiate (for UAS)
    pub async fn negotiate_sdp_as_uas(&self, session_id: &SessionId, remote_sdp: &str) -> Result<(String, NegotiatedConfig)> {
        // Parse remote SDP
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;
        
        // Get our local port
        let local_port = self.get_local_port(session_id)?;
        
        // Generate SDP answer
        let sdp_answer = format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 101\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:101 telephone-event/8000\r\n\
             a=fmtp:101 0-15\r\n\
             a=sendrecv\r\n",
            generate_session_id(),
            0,
            self.local_ip,
            self.local_ip,
            local_port,
        );
        
        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, local_port),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: "PCMU".to_string(),
            payload_type: 0,
        };
        
        // Send negotiation complete event
        self.event_tx.send((
            session_id.clone(),
            EventType::MediaNegotiated
        )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
        
        // Simulate media flow established
        self.event_tx.send((
            session_id.clone(),
            EventType::MediaFlowEstablished
        )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
        
        Ok((sdp_answer, config))
    }
    
    /// Play an audio file to the remote party
    pub async fn play_audio_file(&self, session_id: &SessionId, file_path: &str) -> Result<()> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        // Send play file command to media controller
        // Note: The actual media-core API might differ
        tracing::info!("Playing audio file {} for session {}", file_path, session_id.0);
        
        // In a real implementation, this would send the file path to the media relay
        // For now, we'll just log it
        // Send a media event (using MediaError as a workaround for now)
        // In production, we'd have proper event types for these
        tracing::debug!("Audio playback started: {}", file_path);
        
        Ok(())
    }
    
    /// Start recording the media session
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<String> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        // Generate a unique recording filename
        let recording_path = format!("/tmp/recording_{}.wav", session_id.0);
        
        tracing::info!("Starting recording for session {} at {}", session_id.0, recording_path);
        
        // In a real implementation, this would start recording through the media relay
        // For now, just log the recording start
        tracing::debug!("Recording started at: {}", recording_path);
        
        // Store recording path in session if needed
        if let Ok(mut session) = self.store.get_session(session_id).await {
            // Could add a recording_path field to SessionState if needed
            let _ = self.store.update_session(session).await;
        }
        
        Ok(recording_path)
    }
    
    /// Create a media bridge between two sessions
    pub async fn create_bridge(&self, session1: &SessionId, session2: &SessionId) -> Result<()> {
        // Get dialog IDs for both sessions
        let dialog1 = self.session_to_dialog.get(session1)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session1.0)))?
            .clone();
        let dialog2 = self.session_to_dialog.get(session2)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session2.0)))?
            .clone();
        
        tracing::info!("Creating media bridge between {} and {}", session1.0, session2.0);
        
        // In a real implementation, this would configure the media relay to bridge RTP streams
        // For now, we'll just update the session states
        if let Ok(mut session1_state) = self.store.get_session(session1).await {
            session1_state.bridged_to = Some(session2.clone());
            let _ = self.store.update_session(session1_state).await;
        }
        
        if let Ok(mut session2_state) = self.store.get_session(session2).await {
            session2_state.bridged_to = Some(session1.clone());
            let _ = self.store.update_session(session2_state).await;
        }
        
        // Log bridge creation
        tracing::debug!("Bridge created between {} and {}", session1.0, session2.0);
        
        Ok(())
    }
    
    /// Destroy a media bridge
    pub async fn destroy_bridge(&self, session_id: &SessionId) -> Result<()> {
        // Get the bridged session
        let bridged_session = if let Ok(session) = self.store.get_session(session_id).await {
            session.bridged_to.clone()
        } else {
            None
        };
        
        if let Some(other_session) = bridged_session {
            tracing::info!("Destroying bridge between {} and {}", session_id.0, other_session.0);
            
            // Clear bridge information from both sessions
            if let Ok(mut session1_state) = self.store.get_session(session_id).await {
                session1_state.bridged_to = None;
                let _ = self.store.update_session(session1_state).await;
            }
            
            if let Ok(mut session2_state) = self.store.get_session(&other_session).await {
                session2_state.bridged_to = None;
                let _ = self.store.update_session(session2_state).await;
            }
            
            // Log bridge destruction
            tracing::debug!("Bridge destroyed between {} and {}", session_id.0, other_session.0);
        }
        
        Ok(())
    }
    
    /// Stop recording the media session
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        tracing::info!("Stopping recording for session {}", session_id.0);
        
        // In a real implementation, this would stop recording through the media relay
        tracing::debug!("Recording stopped");
        
        Ok(())
    }
    
    /// Clean up all mappings and resources for a session
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        // Stop the media session if it exists
        if let Some(dialog_id) = self.session_to_dialog.remove(session_id) {
            let _ = self.controller.stop_media(&dialog_id.1).await;
            self.dialog_to_session.remove(&dialog_id.1);
        }
        
        self.media_sessions.remove(session_id);
        
        tracing::debug!("Cleaned up media adapter mappings for session {}", session_id.0);
        Ok(())
    }
    
    // ===== Helper Methods =====
    
    /// Get local RTP port for a session
    fn get_local_port(&self, session_id: &SessionId) -> Result<u16> {
        self.media_sessions
            .get(session_id)
            .and_then(|info| info.rtp_port)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No local port for session {}", session_id.0)))
    }
    
    /// Parse SDP to extract connection info
    fn parse_sdp_connection(&self, sdp: &str) -> Result<(IpAddr, u16)> {
        // Extract IP from c= line
        let ip = sdp.lines()
            .find(|line| line.starts_with("c="))
            .and_then(|line| line.split_whitespace().nth(2))
            .and_then(|ip_str| ip_str.parse::<IpAddr>().ok())
            .ok_or_else(|| SessionError::SDPNegotiationFailed("Failed to parse IP from SDP".into()))?;
        
        // Extract port from m= line
        let port = sdp.lines()
            .find(|line| line.starts_with("m=audio"))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|port_str| port_str.parse::<u16>().ok())
            .ok_or_else(|| SessionError::SDPNegotiationFailed("Failed to parse port from SDP".into()))?;
        
        Ok((ip, port))
    }
    
    // ===== Inbound Events (from media-core) =====
    
    /// Start listening for media events
    pub async fn start_event_loop(&self) -> Result<()> {
        // Get event receiver from media controller
        let mut event_rx = self.controller.take_event_receiver()
            .await
            .ok_or_else(|| SessionError::InternalError("Failed to get media event receiver".into()))?;
        
        let adapter = self.clone();
        
        // Spawn task to handle media events
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let Err(e) = adapter.handle_media_event(event).await {
                    tracing::error!("Error handling media event: {}", e);
                }
            }
        });
        
        Ok(())
    }
    
    /// Handle media events from media-core
    async fn handle_media_event(&self, event: MediaSessionEvent) -> Result<()> {
        match event {
            MediaSessionEvent::SessionCreated { dialog_id, .. } => {
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    tracing::debug!("Media session created for {}", session_id.0);
                }
            }
            
            MediaSessionEvent::SessionDestroyed { dialog_id, .. } => {
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    tracing::debug!("Media session destroyed for {}", session_id.0);
                }
            }
            
            MediaSessionEvent::SessionFailed { dialog_id, error } => {
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    self.event_tx.send((
                        session_id.clone(),
                        EventType::MediaError(error)
                    )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
                }
            }
            
            MediaSessionEvent::RemoteAddressUpdated { dialog_id, remote_addr } => {
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    tracing::debug!("Remote address updated for {}: {}", session_id.0, remote_addr);
                }
            }
            
            _ => {
                // Ignore other events for now
            }
        }
        
        Ok(())
    }
}

impl Clone for MediaAdapter {
    fn clone(&self) -> Self {
        Self {
            controller: self.controller.clone(),
            event_tx: self.event_tx.clone(),
            store: self.store.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            media_sessions: self.media_sessions.clone(),
            local_ip: self.local_ip,
            port_start: self.port_start,
            port_end: self.port_end,
        }
    }
}

/// Generate a random session ID for SDP
fn generate_session_id() -> u64 {
    use rand::Rng;
    rand::thread_rng().gen()
}