use std::time::SystemTime;
use tracing::{debug, error};

use crate::events::SessionEvent;
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::media::{MediaSessionId, QualityMetrics, RtpStreamInfo};
use super::core::{Session, SessionMediaState};

impl Session {
    /// Start media for this session
    pub async fn start_media(&self) -> Result<(), Error> {
        let mut media_state = self.media_state.lock().await;
        
        match *media_state {
            SessionMediaState::None => {
                return Err(Error::InvalidMediaState {
                    context: ErrorContext {
                        category: ErrorCategory::Media,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("No media configured for session".to_string()),
                        ..Default::default()
                    }
                });
            },
            SessionMediaState::Active => {
                debug!("Media already active for session {}", self.id);
                return Ok(());
            },
            SessionMediaState::Failed(ref reason) => {
                return Err(Error::MediaResourceError(
                    format!("Media previously failed: {}", reason),
                    ErrorContext {
                        category: ErrorCategory::Media,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Retry,
                        retryable: true,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some(format!("Previous media failure: {}", reason)),
                        ..Default::default()
                    }
                ));
            },
            _ => {}
        }
        
        // Update media state to active
        *media_state = SessionMediaState::Active;
        drop(media_state);
        
        debug!("Started media for session {}", self.id);
        
        // Publish media started event
        self.event_bus.publish(SessionEvent::Custom {
            session_id: self.id.clone(),
            event_type: "media_started".to_string(),
            data: serde_json::json!({
                "session_id": self.id.to_string(),
                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
            }),
        });
        
        Ok(())
    }
    
    /// Stop media for this session
    pub async fn stop_media(&self) -> Result<(), Error> {
        let mut media_state = self.media_state.lock().await;
        
        if *media_state == SessionMediaState::None {
            debug!("No media to stop for session {}", self.id);
            return Ok(());
        }
        
        // Update media state
        *media_state = SessionMediaState::None;
        drop(media_state);
        
        // Clear media session references
        {
            let mut media_session_id = self.media_session_id.lock().await;
            *media_session_id = None;
        }
        {
            let mut metrics = self.media_metrics.lock().await;
            *metrics = None;
        }
        {
            let mut stream_info = self.rtp_stream_info.lock().await;
            *stream_info = None;
        }
        
        debug!("Stopped media for session {}", self.id);
        
        // Publish media stopped event
        self.event_bus.publish(SessionEvent::Custom {
            session_id: self.id.clone(),
            event_type: "media_stopped".to_string(),
            data: serde_json::json!({
                "session_id": self.id.to_string(),
                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
            }),
        });
        
        Ok(())
    }
    
    /// Pause/hold media for this session
    pub async fn pause_media(&self) -> Result<(), Error> {
        let mut media_state = self.media_state.lock().await;
        
        match *media_state {
            SessionMediaState::Active | SessionMediaState::Configured | SessionMediaState::Negotiating => {
                *media_state = SessionMediaState::Paused;
                debug!("Paused media for session {}", self.id);
                Ok(())
            },
            SessionMediaState::Paused => {
                debug!("Media already paused for session {}", self.id);
                Ok(())
            },
            _ => {
                Err(Error::InvalidMediaState {
                    context: ErrorContext {
                        category: ErrorCategory::Media,
                        severity: ErrorSeverity::Warning,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some(format!("Media in state {:?}, cannot pause", *media_state)),
                        ..Default::default()
                    }
                })
            }
        }
    }
    
    /// Resume media for this session
    pub async fn resume_media(&self) -> Result<(), Error> {
        let mut media_state = self.media_state.lock().await;
        
        match *media_state {
            SessionMediaState::Paused => {
                // Resume to Active state (assuming media was previously active)
                *media_state = SessionMediaState::Active;
                debug!("Resumed media for session {}", self.id);
                Ok(())
            },
            SessionMediaState::Active => {
                debug!("Media already active for session {}", self.id);
                Ok(())
            },
            _ => {
                Err(Error::InvalidMediaState {
                    context: ErrorContext {
                        category: ErrorCategory::Media,
                        severity: ErrorSeverity::Warning,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some(format!("Media in state {:?}, cannot resume", *media_state)),
                        ..Default::default()
                    }
                })
            }
        }
    }
    
    /// Set media session ID (called by SessionManager)
    pub async fn set_media_session_id(&self, media_session_id: Option<MediaSessionId>) {
        let mut guard = self.media_session_id.lock().await;
        *guard = media_session_id;
        
        // Update media state based on whether we have a media session
        let mut media_state = self.media_state.lock().await;
        match (&*guard, &*media_state) {
            (Some(_), SessionMediaState::None) => {
                *media_state = SessionMediaState::Configured;
            },
            (None, SessionMediaState::Configured | SessionMediaState::Active | SessionMediaState::Paused) => {
                *media_state = SessionMediaState::None;
            },
            _ => {}
        }
    }
    
    /// Get the media session ID
    pub async fn media_session_id(&self) -> Option<MediaSessionId> {
        self.media_session_id.lock().await.clone()
    }
    
    /// Update media quality metrics
    pub async fn update_media_metrics(&self, metrics: QualityMetrics) {
        let mut guard = self.media_metrics.lock().await;
        *guard = Some(metrics);
    }
    
    /// Get the latest media quality metrics
    pub async fn media_metrics(&self) -> Option<QualityMetrics> {
        self.media_metrics.lock().await.clone()
    }
    
    /// Set RTP stream information
    pub async fn set_rtp_stream_info(&self, stream_info: Option<RtpStreamInfo>) {
        let mut guard = self.rtp_stream_info.lock().await;
        *guard = stream_info;
    }
    
    /// Get RTP stream information
    pub async fn rtp_stream_info(&self) -> Option<RtpStreamInfo> {
        self.rtp_stream_info.lock().await.clone()
    }
    
    /// Check if media is active
    pub async fn has_active_media(&self) -> bool {
        let media_state = self.media_state.lock().await;
        *media_state == SessionMediaState::Active
    }
    
    /// Check if media is configured
    pub async fn has_media_configured(&self) -> bool {
        let media_state = self.media_state.lock().await;
        matches!(*media_state, SessionMediaState::Configured | SessionMediaState::Active | SessionMediaState::Paused)
    }
    
    /// Handle media failure
    pub async fn handle_media_failure(&self, reason: String) -> Result<(), Error> {
        let mut media_state = self.media_state.lock().await;
        *media_state = SessionMediaState::Failed(reason.clone());
        drop(media_state);
        
        error!("Media failed for session {}: {}", self.id, reason);
        
        // Publish media failure event
        self.event_bus.publish(SessionEvent::Custom {
            session_id: self.id.clone(),
            event_type: "media_failed".to_string(),
            data: serde_json::json!({
                "session_id": self.id.to_string(),
                "reason": reason,
                "timestamp": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
            }),
        });
        
        Ok(())
    }
    
    /// Set media negotiation state
    pub async fn set_media_negotiating(&self) -> Result<(), Error> {
        let mut media_state = self.media_state.lock().await;
        *media_state = SessionMediaState::Negotiating;
        debug!("Media negotiation started for session {}", self.id);
        Ok(())
    }
    
    /// Complete media negotiation and set configured state
    pub async fn complete_media_negotiation(&self) -> Result<(), Error> {
        let mut media_state = self.media_state.lock().await;
        
        match *media_state {
            SessionMediaState::Negotiating => {
                *media_state = SessionMediaState::Configured;
                debug!("Media negotiation completed for session {}", self.id);
                Ok(())
            },
            _ => {
                Err(Error::InvalidMediaState {
                    context: ErrorContext {
                        category: ErrorCategory::Media,
                        severity: ErrorSeverity::Warning,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Media not in negotiating state".to_string()),
                        ..Default::default()
                    }
                })
            }
        }
    }
} 

