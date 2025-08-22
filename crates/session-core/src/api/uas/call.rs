//! UAS Call Handle - Provides operations on established incoming calls

use std::sync::Arc;
use crate::api::control::SessionControl as SessionControlTrait;
use crate::api::media::MediaControl;
use crate::api::types::{SessionId, CallState, AudioFrame, AudioFrameSubscriber};
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Handle to an active UAS call session with operations
#[derive(Clone)]
pub struct UasCallHandle {
    session_id: SessionId,
    coordinator: Arc<SessionCoordinator>,
    remote_uri: String,
    local_uri: String,
}

impl UasCallHandle {
    /// Create a new UAS call handle
    pub fn new(
        session_id: SessionId,
        coordinator: Arc<SessionCoordinator>,
        remote_uri: String,
        local_uri: String,
    ) -> Self {
        Self {
            session_id,
            coordinator,
            remote_uri,
            local_uri,
        }
    }
    
    /// Get the session ID
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    
    /// Get the remote URI (caller)
    pub fn remote_uri(&self) -> &str {
        &self.remote_uri
    }
    
    /// Get the local URI (callee)
    pub fn local_uri(&self) -> &str {
        &self.local_uri
    }
    
    /// Get current call state
    pub async fn state(&self) -> Result<CallState> {
        let session = <Arc<SessionCoordinator> as SessionControlTrait>::get_session(&self.coordinator, &self.session_id)
            .await?
            .ok_or_else(|| crate::errors::SessionError::SessionNotFound(self.session_id.to_string()))?;
        Ok(session.state)
    }
    
    /// Answer the call if it's still ringing
    pub async fn answer(&self) -> Result<()> {
        // For UAS, the call is already answered when accepted
        // This is a no-op but kept for API consistency
        Ok(())
    }
    
    /// Reject/terminate the call
    pub async fn reject(&self, _reason: &str) -> Result<()> {
        // For an established call, rejection means termination
        <Arc<SessionCoordinator> as SessionControlTrait>::terminate_session(&self.coordinator, &self.session_id).await
    }
    
    /// Terminate the call
    pub async fn hangup(&self) -> Result<()> {
        <Arc<SessionCoordinator> as SessionControlTrait>::terminate_session(&self.coordinator, &self.session_id).await
    }
    
    /// Put call on hold
    pub async fn hold(&self) -> Result<()> {
        <Arc<SessionCoordinator> as SessionControlTrait>::hold_session(&self.coordinator, &self.session_id).await
    }
    
    /// Resume call from hold
    pub async fn unhold(&self) -> Result<()> {
        <Arc<SessionCoordinator> as SessionControlTrait>::resume_session(&self.coordinator, &self.session_id).await
    }
    
    /// Send DTMF digit
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        <Arc<SessionCoordinator> as SessionControlTrait>::send_dtmf(&self.coordinator, &self.session_id, &digit.to_string()).await
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
    
    /// Get call quality score (MOS)
    pub async fn get_quality_score(&self) -> Result<Option<f32>> {
        MediaControl::get_call_quality_score(&self.coordinator, &self.session_id).await
    }
    
    /// Get packet loss rate
    pub async fn get_packet_loss(&self) -> Result<Option<f32>> {
        MediaControl::get_packet_loss_rate(&self.coordinator, &self.session_id).await
    }
}