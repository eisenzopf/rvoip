use std::time::SystemTime;
use tracing::debug;

use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::media::{MediaSessionId, MediaConfig};
use crate::sdp::SessionDescription;
use crate::dialog::DialogId;
use super::core::SessionManager;
use super::super::SessionId;

impl SessionManager {
    /// Start media for a session based on SDP negotiation
    pub async fn start_session_media(&self, session_id: &SessionId) -> Result<(), Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // For now, just start media on the session directly
        // TODO: Re-enable full SDP-based media coordination when compilation issues are resolved
        session.start_media().await?;
        
        Ok(())
    }
    
    /// Stop media for a session
    pub async fn stop_session_media(&self, session_id: &SessionId) -> Result<(), Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Get the media session ID
        if let Some(media_session_id) = self.media_manager.get_media_session(session_id).await {
            // Stop the media session
            self.media_manager.stop_media(&media_session_id, "Session terminated".to_string()).await
                .map_err(|e| Error::MediaResourceError(
                    format!("Failed to stop media: {}", e),
                    ErrorContext {
                        category: ErrorCategory::Media,
                        severity: ErrorSeverity::Warning,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(session_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some(format!("Media stop failed: {}", e)),
                        ..Default::default()
                    }
                ))?;
        }
        
        // Stop media on the session
        session.stop_media().await?;
        
        Ok(())
    }
    
    /// Update session media based on new SDP
    pub async fn update_session_media(&self, session_id: &SessionId, sdp: &SessionDescription) -> Result<(), Error> {
        // Get the session
        let _session = self.get_session(session_id)?;
        
        // For now, this is a placeholder - in a full implementation,
        // we would update the media configuration based on the new SDP
        // This might involve creating a new media session or updating the existing one
        
        debug!("Media update requested for session {}", session_id);
        Ok(())
    }
    
    /// Setup media for a dialog using negotiated SDP
    pub async fn setup_media_for_dialog(&self, dialog_id: &DialogId, local_sdp: &SessionDescription, remote_sdp: &SessionDescription) -> Result<MediaSessionId, Error> {
        // Extract media configuration
        let media_config = crate::sdp::extract_media_config(local_sdp, remote_sdp)
            .map_err(|e| Error::MediaNegotiationError(
                format!("Failed to extract media config: {}", e),
                ErrorContext {
                    category: ErrorCategory::Media,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Media config extraction failed: {}", e)),
                    ..Default::default()
                }
            ))?;
        
        // Create and return media session
        self.media_manager.create_media_session(media_config).await
            .map_err(|e| Error::MediaResourceError(
                format!("Failed to create media session: {}", e),
                ErrorContext {
                    category: ErrorCategory::Media,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::Retry,
                    retryable: true,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Media session creation failed: {}", e)),
                    ..Default::default()
                }
            ))
    }
    
    /// Teardown media for a session
    pub async fn teardown_media_for_session(&self, session_id: &SessionId) -> Result<(), Error> {
        self.stop_session_media(session_id).await
    }
    
    /// Setup RTP relay between two sessions
    pub async fn setup_rtp_relay(&self, session_a_id: &SessionId, session_b_id: &SessionId) -> Result<crate::media::RelayId, Error> {
        // Verify both sessions exist
        let _session_a = self.get_session(session_a_id)?;
        let _session_b = self.get_session(session_b_id)?;
        
        // Setup relay in media manager
        self.media_manager.setup_rtp_relay(session_a_id, session_b_id).await
            .map_err(|e| Error::MediaResourceError(
                format!("Failed to setup RTP relay: {}", e),
                ErrorContext {
                    category: ErrorCategory::Media,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::Retry,
                    retryable: true,
                    timestamp: SystemTime::now(),
                    details: Some(format!("RTP relay setup failed: {}", e)),
                    ..Default::default()
                }
            ))
    }
    
    /// Teardown RTP relay
    pub async fn teardown_rtp_relay(&self, relay_id: &crate::media::RelayId) -> Result<(), Error> {
        self.media_manager.teardown_rtp_relay(relay_id).await
            .map_err(|e| Error::MediaResourceError(
                format!("Failed to teardown RTP relay: {}", e),
                ErrorContext {
                    category: ErrorCategory::Media,
                    severity: ErrorSeverity::Warning,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    timestamp: SystemTime::now(),
                    details: Some(format!("RTP relay teardown failed: {}", e)),
                    ..Default::default()
                }
            ))
    }
} 