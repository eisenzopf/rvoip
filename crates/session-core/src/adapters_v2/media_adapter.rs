//! Simplified Media Adapter for v2 modules (merged into session-core v1)

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
use crate::errors_v2::{Result, SessionError};
use crate::session_store_v2::SessionStore;
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
    pub file_path: String,
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub channels: u16,
    pub include_mixed: bool,
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

/// Minimal media adapter
pub struct MediaAdapter {
    pub(crate) controller: Arc<MediaSessionController>,
    pub(crate) store: Arc<SessionStore>,
    pub(crate) session_to_dialog: Arc<DashMap<SessionId, DialogId>>,
    pub(crate) dialog_to_session: Arc<DashMap<DialogId, SessionId>>,
    media_sessions: Arc<DashMap<SessionId, MediaSessionInfo>>,
    audio_receivers: Arc<DashMap<SessionId, mpsc::Sender<AudioFrame>>>,
    local_ip: IpAddr,
    media_port_start: u16,
    media_port_end: u16,
    audio_mixers: Arc<DashMap<crate::state_table::types::MediaSessionId, Vec<crate::state_table::types::MediaSessionId>>>,
}

impl MediaAdapter {
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

    pub async fn start_session(&self, session_id: &SessionId) -> Result<()> {
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            if self.controller.get_session_info(&dialog_id).await.is_some() {
                return Ok(());
            }
        }
        let _media_id = self.create_session(session_id).await?;
        Ok(())
    }

    pub async fn stop_session(&self, session_id: &SessionId) -> Result<()> {
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            self.controller.stop_media(&dialog_id)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to stop media session: {}", e)))?;
            self.session_to_dialog.remove(session_id);
            self.dialog_to_session.remove(&*dialog_id);
            self.media_sessions.remove(session_id);
        }
        Ok(())
    }

    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String> {
        let info = self.media_sessions.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No media session for {}", session_id.0)))?;
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

    pub async fn negotiate_sdp_as_uac(&self, session_id: &SessionId, remote_sdp: &str) -> Result<NegotiatedConfig> {
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            let remote_addr = SocketAddr::new(remote_ip, remote_port);
            self.controller.update_rtp_remote_addr(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to update RTP remote address: {}", e)))?;
            self.controller.establish_media_flow(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to establish media flow: {}", e)))?;
        }
        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, self.get_local_port(session_id)?),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: "PCMU".to_string(),
            payload_type: 0,
        };
        Ok(config)
    }

    pub async fn negotiate_sdp_as_uas(&self, session_id: &SessionId, remote_sdp: &str) -> Result<(String, NegotiatedConfig)> {
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;
        let local_port = self.get_local_port(session_id)?;
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            let remote_addr = SocketAddr::new(remote_ip, remote_port);
            self.controller.update_rtp_remote_addr(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to update RTP remote address: {}", e)))?;
            self.controller.establish_media_flow(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to establish media flow: {}", e)))?;
        }
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
            generate_session_id(), 0, self.local_ip, self.local_ip, local_port,
        );
        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, local_port),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: "PCMU".to_string(),
            payload_type: 0,
        };
        Ok((sdp_answer, config))
    }

    pub async fn play_audio_file(&self, session_id: &SessionId, file_path: &str) -> Result<()> {
        tracing::info!("Playing audio file {} for session {}", file_path, session_id.0);
        Ok(())
    }

    pub async fn send_audio_frame(&self, session_id: &SessionId, audio_frame: AudioFrame) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        let pcm_samples = audio_frame.samples.clone();
        let timestamp = audio_frame.timestamp;
        self.controller.encode_and_send_audio_frame(&dialog_id, pcm_samples, timestamp)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to send audio frame via RTP: {}", e)))?;
        Ok(())
    }

    pub async fn create_session(&self, session_id: &SessionId) -> Result<crate::state_table::types::MediaSessionId> {
        let dialog_id = DialogId::new(format!("media-{}", session_id.0));
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        let media_config = MediaConfig {
            local_addr: SocketAddr::new(self.local_ip, 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: std::collections::HashMap::new(),
        };
        self.controller.start_media(dialog_id.clone(), media_config)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to start media session: {}", e)))?;
        if let Some(info) = self.controller.get_session_info(&dialog_id).await {
            self.media_sessions.insert(session_id.clone(), info.clone());
            let media_id = crate::state_table::types::MediaSessionId(dialog_id.to_string());
            self.controller.store_session_mapping(session_id.0.clone(), MediaSessionId::from_dialog(&dialog_id));
            return Ok(media_id);
        }
        Err(SessionError::MediaError("Failed to get session info after creation".to_string()))
    }

    pub async fn generate_local_sdp(&self, session_id: &SessionId) -> Result<String> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No dialog mapping for session {}", session_id.0)))?
            .clone();
        let info = self.controller.get_session_info(&dialog_id).await
            .ok_or_else(|| SessionError::MediaError(format!("Failed to get session info for dialog {}", dialog_id)))?;
        self.media_sessions.insert(session_id.clone(), info.clone());
        let local_port = info.rtp_port.unwrap_or(info.config.local_addr.port());
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
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
            info.config.local_addr.ip(),
            info.config.local_addr.ip(),
            local_port
        );
        Ok(sdp)
    }

    pub async fn subscribe_to_audio_frames(&self, session_id: &SessionId) -> Result<crate::types_v2::AudioFrameSubscriber> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No media session for {}", session_id.0)))?
            .clone();
        let (tx, rx) = mpsc::channel(1000);
        self.controller.set_audio_frame_callback(dialog_id.clone(), tx.clone())
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to set audio callback: {}", e)))?;
        self.audio_receivers.insert(session_id.clone(), tx);
        Ok(crate::types_v2::AudioFrameSubscriber::new(session_id.clone(), rx))
    }

    pub async fn create_bridge(&self, session1: &SessionId, session2: &SessionId) -> Result<()> {
        tracing::info!("Creating media bridge between {} and {}", session1.0, session2.0);
        Ok(())
    }

    pub async fn destroy_bridge(&self, session_id: &SessionId) -> Result<()> {
        tracing::info!("Destroying bridge for session {}", session_id.0);
        Ok(())
    }

    pub async fn create_media_session(&self) -> Result<crate::state_table::types::MediaSessionId> {
        Ok(crate::state_table::types::MediaSessionId::new())
    }

    pub async fn stop_media_session(&self, _media_id: crate::state_table::types::MediaSessionId) -> Result<()> {
        Ok(())
    }

    pub async fn set_media_direction(&self, _media_id: crate::state_table::types::MediaSessionId, _direction: crate::types_v2::MediaDirection) -> Result<()> {
        Ok(())
    }

    pub async fn create_hold_sdp(&self) -> Result<String> {
        let sdp = format!(
            "v=0\r\no=- 0 0 IN IP4 {}\r\ns=-\r\nc=IN IP4 {}\r\nt=0 0\r\nm=audio 0 RTP/AVP 0\r\na=sendonly\r\n",
            self.local_ip, self.local_ip
        );
        Ok(sdp)
    }

    pub async fn create_active_sdp(&self) -> Result<String> {
        let port = self.media_port_start;
        let sdp = format!(
            "v=0\r\no=- 0 0 IN IP4 {}\r\ns=-\r\nc=IN IP4 {}\r\nt=0 0\r\nm=audio {} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n",
            self.local_ip, self.local_ip, port
        );
        Ok(sdp)
    }

    pub async fn send_dtmf(&self, media_id: crate::state_table::types::MediaSessionId, digit: char) -> Result<()> {
        tracing::debug!("Sending DTMF digit {} for media session {:?}", digit, media_id);
        Ok(())
    }

    pub async fn set_mute(&self, media_id: crate::state_table::types::MediaSessionId, muted: bool) -> Result<()> {
        tracing::debug!("Setting mute state to {} for media session {:?}", muted, media_id);
        Ok(())
    }

    pub async fn start_recording(&self, session_id: &SessionId) -> Result<String> {
        let config = RecordingConfig::default();
        self.start_recording_with_config(session_id, config).await
    }

    pub async fn start_recording_with_config(&self, session_id: &SessionId, config: RecordingConfig) -> Result<String> {
        tracing::info!("Starting recording for session {} with path: {}", session_id.0, config.file_path);
        let recording_id = format!("rec_{}_{}", session_id.0, chrono::Utc::now().timestamp());
        Ok(recording_id)
    }

    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        tracing::info!("Stopping recording for session {}", session_id.0);
        Ok(())
    }

    pub async fn create_audio_mixer(&self) -> Result<crate::state_table::types::MediaSessionId> {
        let mixer_id = crate::state_table::types::MediaSessionId::new();
        self.audio_mixers.insert(mixer_id.clone(), Vec::new());
        Ok(mixer_id)
    }

    pub async fn redirect_to_mixer(&self, media_id: crate::state_table::types::MediaSessionId, mixer_id: crate::state_table::types::MediaSessionId) -> Result<()> {
        if let Some(mut mixer) = self.audio_mixers.get_mut(&mixer_id) {
            mixer.push(media_id);
        }
        Ok(())
    }

    pub async fn remove_from_mixer(&self, media_id: crate::state_table::types::MediaSessionId, mixer_id: crate::state_table::types::MediaSessionId) -> Result<()> {
        if let Some(mut mixer) = self.audio_mixers.get_mut(&mixer_id) {
            mixer.retain(|id| id != &media_id);
        }
        Ok(())
    }

    pub async fn destroy_mixer(&self, mixer_id: crate::state_table::types::MediaSessionId) -> Result<()> {
        self.audio_mixers.remove(&mixer_id);
        Ok(())
    }

    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        if let Some(dialog_id) = self.session_to_dialog.remove(session_id) {
            if self.audio_receivers.contains_key(session_id) {
                if let Err(e) = self.controller.remove_audio_frame_callback(&dialog_id.1).await {
                    tracing::warn!("Failed to remove audio frame callback for dialog {}: {e}", dialog_id.1);
                }
            }
            if let Err(e) = self.controller.stop_media(&dialog_id.1).await {
                tracing::warn!("Failed to stop media for dialog {}: {e}", dialog_id.1);
            }
            self.dialog_to_session.remove(&dialog_id.1);
        }
        self.media_sessions.remove(session_id);
        self.audio_receivers.remove(session_id);
        Ok(())
    }

    fn get_local_port(&self, session_id: &SessionId) -> Result<u16> {
        self.media_sessions
            .get(session_id)
            .and_then(|info| info.rtp_port)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No local port for session {}", session_id.0)))
    }

    fn parse_sdp_connection(&self, sdp: &str) -> Result<(IpAddr, u16)> {
        let ip = sdp.lines()
            .find(|line| line.starts_with("c="))
            .and_then(|line| line.split_whitespace().nth(2))
            .and_then(|ip_str| ip_str.parse::<IpAddr>().ok())
            .ok_or_else(|| SessionError::SDPNegotiationFailed("Failed to parse IP from SDP".into()))?;
        let port = sdp.lines()
            .find(|line| line.starts_with("m=audio"))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|port_str| port_str.parse::<u16>().ok())
            .ok_or_else(|| SessionError::SDPNegotiationFailed("Failed to parse port from SDP".into()))?;
        Ok((ip, port))
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

fn generate_session_id() -> u64 {
    use rand::Rng;
    rand::thread_rng().r#gen()
}
