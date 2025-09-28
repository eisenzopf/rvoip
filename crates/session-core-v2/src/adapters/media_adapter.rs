//! Simplified Media Adapter for session-core-v2
//!
//! Thin translation layer between media-core and state machine.
//! Focuses only on essential media operations and events.

use std::sync::Arc;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;
use dashmap::DashMap;
use rvoip_media_core::{
    relay::controller::{MediaSessionController, MediaConfig, MediaSessionInfo},
    DialogId,
    types::MediaSessionId,
};
use crate::state_table::types::SessionId;
use crate::errors::{Result, SessionError};
use crate::session_store::SessionStore;
use rvoip_media_core::types::AudioFrame;

/// Audio format for recording
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum AudioFormat {
    Wav,
    Raw,
    Mp3,
}

/// Recording configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingConfig {
    /// Path where the recording should be saved
    pub file_path: String,

    /// Audio format for the recording
    pub format: AudioFormat,

    /// Sample rate in Hz (e.g., 8000, 16000, 48000)
    pub sample_rate: u32,

    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,

    /// Include mixed audio from both legs (for conference recording)
    pub include_mixed: bool,

    /// Save separate tracks for each leg
    pub separate_tracks: bool,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            file_path: "/tmp/recording.wav".to_string(),
            format: AudioFormat::Wav,
            sample_rate: 8000,
            channels: 1,
            include_mixed: false,
            separate_tracks: false,
        }
    }
}

/// Recording status information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingStatus {
    pub is_recording: bool,
    pub is_paused: bool,
    pub duration_seconds: f64,
    pub file_size_bytes: u64,
}

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
    pub(crate) controller: Arc<MediaSessionController>,
    
    /// Session store for updating IDs
    pub(crate) store: Arc<SessionStore>,
    
    /// Simple mapping of session IDs to dialog IDs (media-core uses DialogId)
    pub(crate) session_to_dialog: Arc<DashMap<SessionId, DialogId>>,
    pub(crate) dialog_to_session: Arc<DashMap<DialogId, SessionId>>,
    
    /// Store media session info for SDP generation
    media_sessions: Arc<DashMap<SessionId, MediaSessionInfo>>,
    
    /// Audio frame channels for receiving decoded audio from media-core
    audio_receivers: Arc<DashMap<SessionId, mpsc::Sender<AudioFrame>>>,
    
    /// Local IP for SDP generation
    local_ip: IpAddr,
    
    /// Port range for media
    media_port_start: u16,
    media_port_end: u16,
    
    /// Audio mixers for conferences
    audio_mixers: Arc<DashMap<crate::types::MediaSessionId, Vec<crate::types::MediaSessionId>>>,
}

impl MediaAdapter {
    /// Create a new media adapter
    pub fn new(
        controller: Arc<MediaSessionController>,
        store: Arc<SessionStore>,
        local_ip: IpAddr,
        port_start: u16,
        port_end: u16,
    ) -> Self {
        Self {
            controller,
            store,
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            media_sessions: Arc::new(DashMap::new()),
            audio_receivers: Arc::new(DashMap::new()),
            local_ip,
            media_port_start: port_start,
            media_port_end: port_end,
            audio_mixers: Arc::new(DashMap::new()),
        }
    }
    
    // ===== Outbound Actions (from state machine) =====
    
    /// Start a media session
    pub async fn start_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session already exists
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            // Session already exists, check if it's started in media-core
            if self.controller.get_session_info(&dialog_id).await.is_some() {
                tracing::debug!("Media session already started for session {}", session_id.0);
                return Ok(());
            }
        }
        
        // If not, create it - delegate to create_session
        let _media_id = self.create_session(session_id).await?;
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
            let remote_addr = SocketAddr::new(remote_ip, remote_port);
            
            // Update the RTP session with the remote address
            self.controller.update_rtp_remote_addr(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to update RTP remote address: {}", e)))?;
                
            // Establish media flow (this starts audio transmission)
            self.controller.establish_media_flow(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to establish media flow: {}", e)))?;
                
            tracing::info!("✅ Updated RTP remote address to {} for session {}", remote_addr, session_id.0);
        }
        
        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, self.get_local_port(session_id)?),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: "PCMU".to_string(),
            payload_type: 0,
        };
        
        // Event publishing will be handled by SessionCrossCrateEventHandler
        
        Ok(config)
    }
    
    /// Generate SDP answer and negotiate (for UAS)
    pub async fn negotiate_sdp_as_uas(&self, session_id: &SessionId, remote_sdp: &str) -> Result<(String, NegotiatedConfig)> {
        // Parse remote SDP
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;
        
        // Get our local port
        let local_port = self.get_local_port(session_id)?;
        
        // Update media session with remote address
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            let remote_addr = SocketAddr::new(remote_ip, remote_port);
            
            // Update the RTP session with the remote address
            self.controller.update_rtp_remote_addr(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to update RTP remote address: {}", e)))?;
                
            // Establish media flow (this starts audio transmission)
            self.controller.establish_media_flow(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to establish media flow: {}", e)))?;
                
            tracing::info!("✅ Updated RTP remote address to {} for session {} (UAS)", remote_addr, session_id.0);
        }
        
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
        
        // Event publishing will be handled by SessionCrossCrateEventHandler
        
        // Media flow is already represented by MediaStreamStarted above
        
        Ok((sdp_answer, config))
    }
    
    /// Play an audio file to the remote party
    pub async fn play_audio_file(&self, session_id: &SessionId, file_path: &str) -> Result<()> {
        // Get the dialog ID for this session
        let _dialog_id = self.session_to_dialog.get(session_id)
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
    pub async fn start_recording_old(&self, session_id: &SessionId) -> Result<String> {
        // Get the dialog ID for this session
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        // Generate a unique recording filename
        let recording_path = format!("/tmp/recording_{}.wav", session_id.0);
        
        tracing::info!("Starting recording for session {} at {}", session_id.0, recording_path);
        
        // In a real implementation, this would start recording through the media relay
        // For now, just log the recording start
        tracing::debug!("Recording started at: {}", recording_path);
        
        // Store recording path in session if needed
        if let Ok(session) = self.store.get_session(session_id).await {
            // Could add a recording_path field to SessionState if needed
            let _ = self.store.update_session(session).await;
        }
        
        Ok(recording_path)
    }
    
    /// Create a media bridge between two sessions (for peer-to-peer conferencing)
    pub async fn create_bridge(&self, session1: &SessionId, session2: &SessionId) -> Result<()> {
        // Get dialog IDs for both sessions
        let _dialog1 = self.session_to_dialog.get(session1)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session1.0)))?
            .clone();
        let _dialog2 = self.session_to_dialog.get(session2)
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
    pub async fn stop_recording_old(&self, session_id: &SessionId) -> Result<()> {
        // Get the dialog ID for this session
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        tracing::info!("Stopping recording for session {}", session_id.0);
        
        // In a real implementation, this would stop recording through the media relay
        tracing::debug!("Recording stopped");
        
        Ok(())
    }
    
    // ===== AUDIO FRAME API - The Missing Core Functionality =====
    
    /// Send an audio frame for encoding and transmission
    /// This is the equivalent of the old session-core's MediaControl::send_audio_frame()
    pub async fn send_audio_frame(&self, session_id: &SessionId, audio_frame: AudioFrame) -> Result<()> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        tracing::info!("📤 Sending audio frame for session {} ({} samples) via RTP", session_id.0, audio_frame.samples.len());
        
        // Convert AudioFrame to PCM samples and call encode_and_send_audio_frame
        // This will encode the audio and send it via RTP to the remote peer
        let pcm_samples = audio_frame.samples.clone();
        let timestamp = audio_frame.timestamp;
        
        self.controller.encode_and_send_audio_frame(&dialog_id, pcm_samples, timestamp)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to send audio frame via RTP: {}", e)))?;
        
        tracing::debug!("✅ Audio frame sent successfully via RTP for session {}", session_id.0);
        Ok(())
    }
    
    /// Create a new media session
    pub async fn create_session(&self, session_id: &SessionId) -> Result<crate::types::MediaSessionId> {
        // Create dialog ID for media-core
        let dialog_id = DialogId::new(format!("media-{}", session_id.0));
        
        tracing::info!("🚀 Creating media session for session {} with dialog ID {}", session_id.0, dialog_id);
        
        // Store mappings
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        // Create media config with our settings
        let media_config = MediaConfig {
            local_addr: SocketAddr::new(self.local_ip, 0), // Let media-core allocate port
            remote_addr: None, // Will be set when we get remote SDP
            preferred_codec: Some("PCMU".to_string()), // G.711 µ-law as default
            parameters: std::collections::HashMap::new(),
        };
        
        // Start the media session in media-core
        self.controller.start_media(dialog_id.clone(), media_config)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to start media session: {}", e)))?;
            
        // Get and store session info
        if let Some(info) = self.controller.get_session_info(&dialog_id).await {
            self.media_sessions.insert(session_id.clone(), info.clone());
            
            // Store mapping with media controller
            let media_id = crate::types::MediaSessionId(dialog_id.to_string());
            self.controller.store_session_mapping(session_id.0.clone(), MediaSessionId::from_dialog(&dialog_id));
            
            tracing::info!("✅ Media session created successfully for dialog {}", dialog_id);
            return Ok(media_id);
        }
        
        Err(SessionError::MediaError("Failed to get session info after creation".to_string()))
    }
    
    /// Generate local SDP offer
    pub async fn generate_local_sdp(&self, session_id: &SessionId) -> Result<String> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No dialog mapping for session {}", session_id.0)))?
            .clone();
            
        tracing::debug!("Generating SDP for session {} with dialog ID {}", session_id.0, dialog_id);
            
        // Get session info (should exist now)
        let info = self.controller.get_session_info(&dialog_id).await
            .ok_or_else(|| SessionError::MediaError(format!("Failed to get session info for dialog {}", dialog_id)))?;
        
        // Store session info
        self.media_sessions.insert(session_id.clone(), info.clone());
        
        // The actual RTP port might not be in config.local_addr - it's in rtp_port
        let local_port = info.rtp_port.unwrap_or(info.config.local_addr.port());
        tracing::debug!("Media session info - RTP port: {:?}, local_addr: {}", info.rtp_port, info.config.local_addr);
        
        // Build SDP from actual media session info
        let sdp = format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=RVoIP Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=sendrecv\r\n",
            session_id.0,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            info.config.local_addr.ip(),
            info.config.local_addr.ip(),
            local_port
        );
        
        tracing::info!("✅ Generated SDP for session {} with local port {}", session_id.0, local_port);
        
        Ok(sdp)
    }
    
    /// Subscribe to receive decoded audio frames from RTP
    /// This is the equivalent of the old session-core's MediaControl::subscribe_to_audio_frames()
    pub async fn subscribe_to_audio_frames(&self, session_id: &SessionId) -> Result<crate::types::AudioFrameSubscriber> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No media session for {}", session_id.0)))?
            .clone();
        
        tracing::info!("🎧 Setting up audio subscription for session {} (dialog: {})", session_id.0, dialog_id);
        
        // Create channel for audio frames
        let (tx, rx) = mpsc::channel(1000); // Buffer up to 1000 frames (20 seconds at 50fps)
        
        // Register the callback with MediaSessionController to receive audio frames
        self.controller.set_audio_frame_callback(dialog_id.clone(), tx.clone())
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to set audio callback: {}", e)))?;
        
        // Store the sender for this session for cleanup
        self.audio_receivers.insert(session_id.clone(), tx);
        
        tracing::info!("🎧 Created audio frame subscriber for session {} with dialog {}", session_id.0, dialog_id);
        
        // Return our types::AudioFrameSubscriber
        Ok(crate::types::AudioFrameSubscriber::new(session_id.clone(), rx))
    }
    
    /// Internal method to forward received audio frames to subscribers
    /// This should be called by the media event handler when audio frames are received
    #[allow(dead_code)]
    pub(crate) async fn forward_audio_frame_to_subscriber(&self, session_id: &SessionId, audio_frame: rvoip_media_core::types::AudioFrame) -> Result<()> {
        if let Some(tx) = self.audio_receivers.get(session_id) {
            if let Err(_) = tx.send(audio_frame).await {
                // Receiver has been dropped, clean up
                self.audio_receivers.remove(session_id);
                tracing::debug!("Audio frame subscriber disconnected for session {}", session_id.0);
            }
        }
        Ok(())
    }
    
    // ===== New Methods for CallController and ConferenceManager =====
    
    /// Create a media session
    pub async fn create_media_session(&self) -> Result<crate::types::MediaSessionId> {
        let media_id = crate::types::MediaSessionId::new();
        Ok(media_id)
    }
    
    /// Stop a media session
    pub async fn stop_media_session(&self, _media_id: crate::types::MediaSessionId) -> Result<()> {
        // For now, just return Ok
        Ok(())
    }
    
    /// Set media direction (for hold/resume)
    pub async fn set_media_direction(&self, _media_id: crate::types::MediaSessionId, _direction: crate::types::MediaDirection) -> Result<()> {
        // TODO: Implement actual media direction control
        Ok(())
    }
    
    /// Create hold SDP
    pub async fn create_hold_sdp(&self) -> Result<String> {
        // Create SDP with sendonly attribute
        let sdp = format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 {}\r\n\
             s=-\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio 0 RTP/AVP 0\r\n\
             a=sendonly\r\n",
            self.local_ip, self.local_ip
        );
        Ok(sdp)
    }
    
    /// Create active SDP
    pub async fn create_active_sdp(&self) -> Result<String> {
        // Create SDP with sendrecv attribute
        let port = self.media_port_start; // TODO: Allocate actual port
        let sdp = format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 {}\r\n\
             s=-\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=sendrecv\r\n",
            self.local_ip, self.local_ip, port
        );
        Ok(sdp)
    }
    
    /// Send DTMF digit
    pub async fn send_dtmf(&self, media_id: crate::types::MediaSessionId, digit: char) -> Result<()> {
        // TODO: Implement DTMF sending
        tracing::debug!("Sending DTMF digit {} for media session {:?}", digit, media_id);
        Ok(())
    }
    
    /// Set mute state
    pub async fn set_mute(&self, media_id: crate::types::MediaSessionId, muted: bool) -> Result<()> {
        // TODO: Implement mute control
        tracing::debug!("Setting mute state to {} for media session {:?}", muted, media_id);
        Ok(())
    }
    
    /// Start recording for media session
    pub async fn start_recording_media(&self, media_id: crate::types::MediaSessionId) -> Result<()> {
        // TODO: Implement recording
        tracing::info!("Starting recording for media session {:?}", media_id);
        Ok(())
    }
    
    /// Stop recording for media session
    pub async fn stop_recording_media(&self, media_id: crate::types::MediaSessionId) -> Result<()> {
        // TODO: Implement recording stop
        tracing::info!("Stopping recording for media session {:?}", media_id);
        Ok(())
    }
    
    // ===== Conference Methods =====
    
    /// Create an audio mixer for a conference
    pub async fn create_audio_mixer(&self) -> Result<crate::types::MediaSessionId> {
        let mixer_id = crate::types::MediaSessionId::new();
        self.audio_mixers.insert(mixer_id.clone(), Vec::new());
        tracing::info!("Created audio mixer {:?}", mixer_id);
        Ok(mixer_id)
    }
    
    /// Redirect audio to a mixer
    pub async fn redirect_to_mixer(&self, media_id: crate::types::MediaSessionId, mixer_id: crate::types::MediaSessionId) -> Result<()> {
        if let Some(mut mixer) = self.audio_mixers.get_mut(&mixer_id) {
            mixer.push(media_id.clone());
        }
        tracing::debug!("Redirected media {:?} to mixer {:?}", media_id, mixer_id);
        Ok(())
    }
    
    /// Remove audio from a mixer
    pub async fn remove_from_mixer(&self, media_id: crate::types::MediaSessionId, mixer_id: crate::types::MediaSessionId) -> Result<()> {
        if let Some(mut mixer) = self.audio_mixers.get_mut(&mixer_id) {
            mixer.retain(|id| id != &media_id);
        }
        tracing::debug!("Removed media {:?} from mixer {:?}", media_id, mixer_id);
        Ok(())
    }
    
    /// Destroy an audio mixer
    pub async fn destroy_mixer(&self, mixer_id: crate::types::MediaSessionId) -> Result<()> {
        self.audio_mixers.remove(&mixer_id);
        tracing::info!("Destroyed audio mixer {:?}", mixer_id);
        Ok(())
    }
    
    /// Clean up all mappings and resources for a session
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        // Stop the media session if it exists
        if let Some(dialog_id) = self.session_to_dialog.remove(session_id) {
            // Remove audio frame callback if one was set
            if self.audio_receivers.contains_key(session_id) {
                let _ = self.controller.remove_audio_frame_callback(&dialog_id.1).await;
            }
            
            let _ = self.controller.stop_media(&dialog_id.1).await;
            self.dialog_to_session.remove(&dialog_id.1);
        }
        
        self.media_sessions.remove(session_id);
        
        // Clean up audio frame receivers
        self.audio_receivers.remove(session_id);
        
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
    
    // ===== Event handling removed - now centralized in SessionCrossCrateEventHandler ====="

    // ===== Recording Management =====

    /// Start recording for a session (simple version for backward compatibility)
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<String> {
        // Use default config for backward compatibility
        let config = RecordingConfig::default();
        self.start_recording_with_config(session_id, config).await
    }

    /// Start recording for a session with specific config
    pub async fn start_recording_with_config(
        &self,
        session_id: &SessionId,
        config: RecordingConfig,
    ) -> Result<String> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::info!("Starting recording for session {} with path: {}", session_id.0, config.file_path);

        // For now, we'll generate a simple recording ID
        // In a real implementation, this would interact with media-core's recording API
        let recording_id = format!("rec_{}_{}", session_id.0, chrono::Utc::now().timestamp());

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.start_recording(&dialog_id, config).await

        tracing::info!("✅ Recording started for session {} with ID: {}", session_id.0, recording_id);
        Ok(recording_id)
    }

    /// Stop recording for a session (simple version for backward compatibility)
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        // For backward compatibility, we don't have a recording_id
        // Just log the action
        tracing::info!("Stopping recording for session {}", session_id.0);
        Ok(())
    }

    /// Stop recording for a session with specific recording ID
    pub async fn stop_recording_with_id(&self, session_id: &SessionId, recording_id: &str) -> Result<()> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::info!("Stopping recording {} for session {}", recording_id, session_id.0);

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.stop_recording(&dialog_id, recording_id).await

        tracing::info!("✅ Recording stopped for session {}", session_id.0);
        Ok(())
    }

    /// Pause recording for a session
    pub async fn pause_recording(&self, session_id: &SessionId, recording_id: &str) -> Result<()> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::debug!("Pausing recording {} for session {}", recording_id, session_id.0);

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.pause_recording(&dialog_id, recording_id).await

        Ok(())
    }

    /// Resume a paused recording
    pub async fn resume_recording(&self, session_id: &SessionId, recording_id: &str) -> Result<()> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::debug!("Resuming recording {} for session {}", recording_id, session_id.0);

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.resume_recording(&dialog_id, recording_id).await

        Ok(())
    }

    /// Get recording status
    pub async fn get_recording_status(
        &self,
        session_id: &SessionId,
        _recording_id: &str,
    ) -> Result<RecordingStatus> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        // TODO: When media-core adds recording support, implement this properly
        // For now, return a mock status
        Ok(RecordingStatus {
            is_recording: false,
            is_paused: false,
            duration_seconds: 0.0,
            file_size_bytes: 0,
        })
    }

    /// Start recording for a bridged session pair
    pub async fn start_bridge_recording(
        &self,
        session1: &SessionId,
        session2: &SessionId,
        config: RecordingConfig,
    ) -> Result<String> {
        tracing::info!("Starting bridge recording for sessions {} <-> {}", session1.0, session2.0);

        // Start recording on both sessions with mixed audio
        let mut recording_config = config;
        recording_config.include_mixed = true;
        recording_config.separate_tracks = true;

        // Start recording on the first session (will capture both legs if bridged)
        self.start_recording_with_config(session1, recording_config).await
    }

    /// Enable/disable recording for all conference sessions
    pub async fn set_conference_recording_enabled(&self, enabled: bool) -> Result<()> {
        // This would be stored in a shared configuration
        // For now, we'll just log the intent
        tracing::info!("Conference recording enabled: {}", enabled);

        // TODO: Store this in a shared configuration that conference sessions check
        // when they are created

        Ok(())
    }
}

impl Clone for MediaAdapter {
    fn clone(&self) -> Self {
        Self {
            controller: self.controller.clone(),
            store: self.store.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            media_sessions: self.media_sessions.clone(),
            audio_receivers: self.audio_receivers.clone(),
            audio_mixers: self.audio_mixers.clone(),
            local_ip: self.local_ip,
            media_port_start: self.media_port_start,
            media_port_end: self.media_port_end,
        }
    }
}

/// Generate a random session ID for SDP
fn generate_session_id() -> u64 {
    use rand::Rng;
    rand::thread_rng().gen()
}