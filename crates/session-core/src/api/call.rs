//! Unified call handle for both UAC and UAS sides
//! 
//! This module provides a symmetric SimpleCall type that works the same
//! whether the call was initiated (UAC) or received (UAS).

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use crate::api::types::{SessionId, CallState, AudioFrame, CallSession};
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::api::common::setup_audio_channels;
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};

/// A simple call handle with all operations
/// 
/// This type provides a unified interface for call control, regardless of
/// whether the call was initiated or received. Both UAC and UAS sides get
/// the same capabilities.
/// 
/// # Example
/// ```
/// use rvoip_session_core::api::call::SimpleCall;
/// 
/// async fn handle_call(mut call: SimpleCall) -> Result<(), Box<dyn std::error::Error>> {
///     // Get audio channels
///     let (tx, rx) = call.audio_channels()?;
///     
///     // Control the call
///     call.hold().await?;
///     call.resume().await?;
///     call.send_dtmf("123").await?;
///     
///     // End the call
///     call.hangup().await?;
///     
///     Ok(())
/// }
/// ```
pub struct SimpleCall {
    session_id: SessionId,
    coordinator: Arc<SessionCoordinator>,
    audio_tx: Option<mpsc::Sender<AudioFrame>>,
    audio_rx: Option<mpsc::Receiver<AudioFrame>>,
    remote_uri: String,
    start_time: Instant,
    state: Arc<RwLock<CallState>>,
}

/// Builder for call transfers with blind or attended options
pub struct TransferBuilder<'a> {
    call: &'a SimpleCall,
    target: String,
    consult_call: Option<SimpleCall>,
}

impl<'a> TransferBuilder<'a> {
    /// Create a new transfer builder
    fn new(call: &'a SimpleCall, target: &str) -> Self {
        Self {
            call,
            target: target.to_string(),
            consult_call: None,
        }
    }
    
    /// Make this an attended transfer with a consultation call
    pub fn attended(mut self, consult_call: SimpleCall) -> Self {
        self.consult_call = Some(consult_call);
        self
    }
}

// Implement IntoFuture for TransferBuilder to allow await directly
impl<'a> std::future::IntoFuture for TransferBuilder<'a> {
    type Output = Result<()>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            if let Some(consult_call) = self.consult_call {
                // Attended transfer - bridge the calls first, then transfer
                // TODO: Implement attended transfer logic
                tracing::info!("Attended transfer to {} with consultation call", self.target);
                use crate::api::common::call_ops;
                call_ops::transfer(&self.call.coordinator, &self.call.session_id, &self.target).await
            } else {
                // Blind transfer
                use crate::api::common::call_ops;
                call_ops::transfer(&self.call.coordinator, &self.call.session_id, &self.target).await
            }
        })
    }
}

impl SimpleCall {
    /// Create from a session (internal use)
    pub(crate) async fn from_session(
        session: CallSession,
        coordinator: Arc<SessionCoordinator>,
    ) -> Result<Self> {
        // Set up audio channels
        let (audio_tx, audio_rx) = setup_audio_channels(
            &coordinator,
            &session.id,
        ).await?;
        
        Ok(Self {
            session_id: session.id.clone(),
            coordinator,
            audio_tx: Some(audio_tx),
            audio_rx: Some(audio_rx),
            remote_uri: session.to.clone(),
            start_time: Instant::now(),
            state: Arc::new(RwLock::new(session.state)),
        })
    }
    
    /// Create from an accepted incoming call (internal use)
    pub(crate) async fn from_incoming(
        session_id: SessionId,
        coordinator: Arc<SessionCoordinator>,
        remote_uri: String,
    ) -> Result<Self> {
        // Set up audio channels
        let (audio_tx, audio_rx) = setup_audio_channels(
            &coordinator,
            &session_id,
        ).await?;
        
        Ok(Self {
            session_id,
            coordinator,
            audio_tx: Some(audio_tx),
            audio_rx: Some(audio_rx),
            remote_uri,
            start_time: Instant::now(),
            state: Arc::new(RwLock::new(CallState::Active)),
        })
    }
    
    /// Get the audio channels (consumes them - can only be called once)
    /// 
    /// Returns (tx, rx) where:
    /// - `tx`: Send audio frames to the remote party
    /// - `rx`: Receive audio frames from the remote party
    /// 
    /// # Errors
    /// Returns an error if the channels have already been taken.
    pub fn audio_channels(&mut self) -> Result<(mpsc::Sender<AudioFrame>, mpsc::Receiver<AudioFrame>)> {
        let tx = self.audio_tx.take()
            .ok_or(SessionError::MediaError("Audio channels already taken".to_string()))?;
        let rx = self.audio_rx.take()
            .ok_or(SessionError::MediaError("Audio channels already taken".to_string()))?;
        Ok((tx, rx))
    }
    
    /// Check if audio channels are still available
    pub fn has_audio_channels(&self) -> bool {
        self.audio_tx.is_some() && self.audio_rx.is_some()
    }
    
    /// Put the call on hold
    /// 
    /// This stops media transmission in both directions.
    pub async fn hold(&self) -> Result<()> {
        use crate::api::common::call_ops;
        call_ops::hold(&self.coordinator, &self.session_id).await?;
        *self.state.write().await = CallState::OnHold;
        Ok(())
    }
    
    /// Resume from hold
    /// 
    /// This resumes media transmission in both directions.
    pub async fn resume(&self) -> Result<()> {
        use crate::api::common::call_ops;
        call_ops::unhold(&self.coordinator, &self.session_id).await?;
        *self.state.write().await = CallState::Active;
        Ok(())
    }
    
    /// Mute audio transmission
    /// 
    /// This stops sending audio to the remote party but continues receiving.
    pub async fn mute(&self) -> Result<()> {
        use crate::api::common::call_ops;
        call_ops::mute(&self.coordinator, &self.session_id).await
    }
    
    /// Unmute audio transmission
    /// 
    /// This resumes sending audio to the remote party.
    pub async fn unmute(&self) -> Result<()> {
        use crate::api::common::call_ops;
        call_ops::unmute(&self.coordinator, &self.session_id).await
    }
    
    /// Send DTMF digits
    /// 
    /// # Arguments
    /// * `digits` - A string of DTMF digits (0-9, *, #, A-D)
    pub async fn send_dtmf(&self, digits: &str) -> Result<()> {
        for digit in digits.chars() {
            if !digit.is_ascii_digit() && !"*#ABCD".contains(digit) {
                return Err(SessionError::Other(
                    format!("Invalid DTMF digit: {}", digit)
                ));
            }
            // Handled at the end of the loop
            // Small delay between digits
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // Send all digits at once
        use crate::api::common::call_ops;
        call_ops::send_dtmf(&self.coordinator, &self.session_id, digits).await?;
        Ok(())
    }
    
    /// Create a transfer builder for blind or attended transfers
    /// 
    /// # Example
    /// ```
    /// // Blind transfer
    /// call.transfer("charlie@example.com").await?;
    /// 
    /// // Attended transfer
    /// let consult_call = peer.call("charlie@example.com").await?;
    /// call.transfer("charlie@example.com")
    ///     .attended(consult_call)
    ///     .await?;
    /// ```
    pub fn transfer(&self, target: &str) -> TransferBuilder {
        TransferBuilder::new(self, target)
    }
    
    /// Bridge this call with another call (3-way conference)
    /// 
    /// Creates a local bridge between two calls for conferencing.
    /// 
    /// # Arguments
    /// * `other` - The other call to bridge with
    /// 
    /// # Example
    /// ```
    /// let call1 = peer.call("bob@example.com").await?;
    /// let call2 = peer.call("charlie@example.com").await?;
    /// call1.bridge(call2).await?;
    /// ```
    pub async fn bridge(&self, other: SimpleCall) -> Result<()> {
        use crate::api::common::call_ops;
        call_ops::bridge(&self.coordinator, &self.session_id, other.id()).await
    }
    
    /// Get the session ID
    pub fn id(&self) -> &SessionId {
        &self.session_id
    }
    
    /// Get the remote party URI
    pub fn remote_uri(&self) -> &str {
        &self.remote_uri
    }
    
    /// Get call duration
    pub fn duration(&self) -> Duration {
        self.start_time.elapsed()
    }
    
    /// Get current state
    pub async fn state(&self) -> CallState {
        self.state.read().await.clone()
    }
    
    /// Check if the call is active
    pub async fn is_active(&self) -> bool {
        matches!(self.state().await, CallState::Active)
    }
    
    /// Check if the call is on hold
    pub async fn is_on_hold(&self) -> bool {
        matches!(self.state().await, CallState::OnHold)
    }
    
    /// Get packet loss rate for call quality monitoring
    /// 
    /// Returns a value between 0.0 and 1.0 representing the packet loss rate.
    pub async fn packet_loss_rate(&self) -> Result<f32> {
        use crate::api::common::call_ops;
        call_ops::get_packet_loss_rate(&self.coordinator, &self.session_id).await
    }
    
    /// Get call quality score (MOS - Mean Opinion Score)
    /// 
    /// Returns a value between 1.0 and 5.0 representing call quality.
    pub async fn quality_score(&self) -> Result<f32> {
        use crate::api::common::call_ops;
        call_ops::get_quality_score(&self.coordinator, &self.session_id).await
    }
    
    /// Get call statistics
    /// 
    /// Returns session statistics
    pub async fn statistics(&self) -> Result<crate::api::types::SessionStats> {
        // For now, return basic stats
        // TODO: Implement proper media statistics
        Ok(crate::api::types::SessionStats {
            total_sessions: 1,
            active_sessions: if self.is_active().await { 1 } else { 0 },
            failed_sessions: 0,
            average_duration: Some(self.duration()),
        })
    }
    
    /// Play an audio file to the remote party
    /// 
    /// # Arguments
    /// * `file_path` - Path to the audio file to play
    pub async fn play_audio(&self, file_path: &str) -> Result<()> {
        // This would need implementation in MediaControl
        tracing::warn!("play_audio not yet implemented for: {}", file_path);
        Ok(())
    }
    
    /// Start recording the call
    /// 
    /// # Arguments
    /// * `file_path` - Path where the recording should be saved
    pub async fn start_recording(&self, file_path: &str) -> Result<()> {
        // This would need implementation in MediaControl
        tracing::warn!("start_recording not yet implemented for: {}", file_path);
        Ok(())
    }
    
    /// Stop recording the call
    pub async fn stop_recording(&self) -> Result<()> {
        // This would need implementation in MediaControl
        tracing::warn!("stop_recording not yet implemented");
        Ok(())
    }
    
    /// Hang up the call
    /// 
    /// This terminates the call and consumes the SimpleCall object
    /// to prevent further operations on a terminated call.
    pub async fn hangup(self) -> Result<()> {
        *self.state.write().await = CallState::Terminated;
        use crate::api::control::SessionControl;
        SessionControl::terminate_session(&self.coordinator, &self.session_id).await
    }
    
    /// Get reference to coordinator (for advanced operations)
    /// 
    /// This allows advanced users to access the underlying coordinator
    /// for operations not exposed by SimpleCall.
    pub fn coordinator(&self) -> &Arc<SessionCoordinator> {
        &self.coordinator
    }
}

impl std::fmt::Debug for SimpleCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleCall")
            .field("session_id", &self.session_id)
            .field("remote_uri", &self.remote_uri)
            .field("duration", &self.duration())
            .field("has_audio", &self.has_audio_channels())
            .finish()
    }
}