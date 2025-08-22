//! UAC Call handle and operations

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use crate::api::types::{SessionId, CallState, AudioFrame, AudioFrameSubscriber};
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Handle to an active UAC call
#[derive(Clone)]
pub struct UacCall {
    /// Session ID
    pub(crate) session_id: SessionId,
    /// Reference to the coordinator
    pub(crate) coordinator: Arc<SessionCoordinator>,
    /// Remote URI
    pub(crate) remote_uri: String,
}

impl UacCall {
    /// Create a new UAC call handle
    pub(crate) fn new(
        session_id: SessionId,
        coordinator: Arc<SessionCoordinator>,
        remote_uri: String,
    ) -> Self {
        Self {
            session_id,
            coordinator,
            remote_uri,
        }
    }
    
    /// Get the session ID
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    
    /// Get the remote URI
    pub fn remote_uri(&self) -> &str {
        &self.remote_uri
    }
    
    /// Get current call state
    pub async fn state(&self) -> Result<CallState> {
        let session = SessionControl::get_session(&self.coordinator, &self.session_id)
            .await?
            .ok_or_else(|| crate::errors::SessionError::SessionNotFound(self.session_id.to_string()))?;
        Ok(session.state)
    }
    
    /// Wait for the call to be answered
    pub async fn wait_for_answer(&self, timeout: Duration) -> Result<()> {
        SessionControl::wait_for_answer(&self.coordinator, &self.session_id, timeout).await
    }
    
    /// Hangup the call
    pub async fn hangup(&self) -> Result<()> {
        SessionControl::terminate_session(&self.coordinator, &self.session_id).await
    }
    
    /// Send DTMF digit
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        SessionControl::send_dtmf(&self.coordinator, &self.session_id, &digit.to_string()).await
    }
    
    /// Transfer the call
    pub async fn transfer(&self, target: &str) -> Result<()> {
        SessionControl::transfer_session(&self.coordinator, &self.session_id, target).await
    }
    
    /// Put call on hold
    pub async fn hold(&self) -> Result<()> {
        SessionControl::hold_session(&self.coordinator, &self.session_id).await
    }
    
    /// Resume call from hold
    pub async fn unhold(&self) -> Result<()> {
        SessionControl::resume_session(&self.coordinator, &self.session_id).await
    }
    
    /// Start audio transmission (unmute microphone)
    pub async fn unmute(&self) -> Result<()> {
        MediaControl::start_audio_transmission(&self.coordinator, &self.session_id).await
    }
    
    /// Stop audio transmission (mute microphone)
    pub async fn mute(&self) -> Result<()> {
        MediaControl::stop_audio_transmission(&self.coordinator, &self.session_id).await
    }
    
    /// Check if audio is muted
    pub async fn is_muted(&self) -> Result<bool> {
        let active = MediaControl::is_audio_transmission_active(&self.coordinator, &self.session_id).await?;
        Ok(!active)
    }
    
    /// Subscribe to audio frames for this call
    pub async fn subscribe_to_audio_frames(&self) -> Result<AudioFrameSubscriber> {
        MediaControl::subscribe_to_audio_frames(&self.coordinator, &self.session_id).await
    }
    
    /// Send an audio frame to this call
    pub async fn send_audio_frame(&self, frame: AudioFrame) -> Result<()> {
        MediaControl::send_audio_frame(&self.coordinator, &self.session_id, frame).await
    }
    
    /// Receive an audio frame from this call
    pub async fn receive_audio_frame(&self) -> Result<Option<AudioFrame>> {
        MediaControl::receive_audio_frame(&self.coordinator, &self.session_id).await
    }
    
    /// Get call quality score (MOS)
    pub async fn get_quality_score(&self) -> Result<Option<f32>> {
        MediaControl::get_call_quality_score(&self.coordinator, &self.session_id).await
    }
    
    /// Get packet loss rate
    pub async fn get_packet_loss(&self) -> Result<Option<f32>> {
        MediaControl::get_packet_loss_rate(&self.coordinator, &self.session_id).await
    }
    
    /// Get current bitrate in kbps
    pub async fn get_bitrate(&self) -> Result<Option<u32>> {
        MediaControl::get_current_bitrate(&self.coordinator, &self.session_id).await
    }
}

/// Lightweight handle for tracking calls
#[derive(Clone)]
pub struct UacCallHandle {
    session_id: SessionId,
    remote_uri: String,
}

impl UacCallHandle {
    pub(crate) fn new(session_id: SessionId, remote_uri: String) -> Self {
        Self { session_id, remote_uri }
    }
    
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    
    pub fn remote_uri(&self) -> &str {
        &self.remote_uri
    }
}