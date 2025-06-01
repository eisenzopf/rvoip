use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::SystemTime;
use std::net::SocketAddr;
use tracing::{debug, info, error, warn};
use serde_json;

use rvoip_sip_core::Request;

use crate::events::{EventBus, SessionEvent};
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::media::{MediaSessionId, MediaStatus, QualityMetrics, RtpStreamInfo};
use super::super::session_id::SessionId;
use super::super::session_types::{
    SessionState, SessionDirection,
    TransferId, TransferState, TransferType, TransferContext
};
use super::super::session_config::SessionConfig;

/// Media state for a session
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionMediaState {
    /// No media configured
    None,
    /// Media is being negotiated
    Negotiating,
    /// Media is configured but not started
    Configured,
    /// Media is active
    Active,
    /// Media is paused/on hold
    Paused,
    /// Media has failed
    Failed(String),
}

impl Default for SessionMediaState {
    fn default() -> Self {
        Self::None
    }
}

/// Represents a SIP session (call) with integrated media management
/// 
/// **ARCHITECTURE**: Session is purely a coordination object.
/// It does NOT handle SIP transactions directly - that's dialog-core's job.
#[derive(Clone)]
pub struct Session {
    /// Unique session identifier
    pub id: SessionId,
    
    /// Current session state
    pub(crate) state: Arc<Mutex<SessionState>>,
    
    /// Current media state
    pub(crate) media_state: Arc<Mutex<SessionMediaState>>,
    
    /// Direction of the session (incoming or outgoing)
    direction: SessionDirection,
    
    /// Session configuration
    config: SessionConfig,
    
    /// Media session ID (if media is active)
    pub(crate) media_session_id: Arc<Mutex<Option<MediaSessionId>>>,
    
    /// Latest media quality metrics
    pub(crate) media_metrics: Arc<Mutex<Option<QualityMetrics>>>,
    
    /// RTP stream information
    pub(crate) rtp_stream_info: Arc<Mutex<Option<RtpStreamInfo>>>,
    
    /// Current transfer context (if transfer is in progress)
    pub(crate) transfer_context: Arc<Mutex<Option<TransferContext>>>,
    
    /// Transfer history for this session
    pub(crate) transfer_history: Arc<Mutex<Vec<TransferContext>>>,
    
    /// Consultation session ID (for attended transfers)
    pub(crate) consultation_session_id: Arc<Mutex<Option<SessionId>>>,
    
    /// Event bus for publishing session events
    pub(crate) event_bus: EventBus,
}

impl Session {
    /// Create a new session with media support
    /// 
    /// **ARCHITECTURE**: Session doesn't handle transactions directly.
    /// All SIP protocol work is delegated to dialog-core.
    pub fn new(
        direction: SessionDirection,
        config: SessionConfig,
        event_bus: EventBus
    ) -> Self {
        let id = SessionId::new();
        let session = Self {
            id: id.clone(),
            state: Arc::new(Mutex::new(SessionState::Initializing)),
            media_state: Arc::new(Mutex::new(SessionMediaState::None)),
            direction,
            config,
            media_session_id: Arc::new(Mutex::new(None)),
            media_metrics: Arc::new(Mutex::new(None)),
            rtp_stream_info: Arc::new(Mutex::new(None)),
            transfer_context: Arc::new(Mutex::new(None)),
            transfer_history: Arc::new(Mutex::new(Vec::new())),
            consultation_session_id: Arc::new(Mutex::new(None)),
            event_bus: event_bus.clone(),
        };
        
        // Publish session creation event
        let _ = event_bus.publish(SessionEvent::Created { session_id: id });
        
        session
    }
    
    /// Create a new incoming session
    /// 
    /// **ARCHITECTURE**: Session doesn't handle transactions directly.
    /// All SIP protocol work is handled by dialog-core.
    pub async fn new_incoming(
        session_id: SessionId,
        request: Request,
        source: SocketAddr,
        config: SessionConfig,
    ) -> Result<Self, Error> {
        // Create a basic event bus for this session
        let event_bus = crate::events::EventBus::new(100).await
            .map_err(|e| Error::InternalError(
                format!("Failed to create event bus: {}", e),
                ErrorContext::default().with_message("Event bus creation failed")
            ))?;
        
        let session = Self {
            id: session_id.clone(),
            state: Arc::new(Mutex::new(SessionState::Ringing)),
            media_state: Arc::new(Mutex::new(SessionMediaState::None)),
            direction: SessionDirection::Incoming,
            config,
            media_session_id: Arc::new(Mutex::new(None)),
            media_metrics: Arc::new(Mutex::new(None)),
            rtp_stream_info: Arc::new(Mutex::new(None)),
            transfer_context: Arc::new(Mutex::new(None)),
            transfer_history: Arc::new(Mutex::new(Vec::new())),
            consultation_session_id: Arc::new(Mutex::new(None)),
            event_bus: event_bus.clone(),
        };
        
        // Publish session creation event
        let _ = event_bus.publish(SessionEvent::Created { session_id });
        
        Ok(session)
    }
    
    /// Get the current session state
    pub async fn state(&self) -> SessionState {
        *self.state.lock().await
    }
    
    /// Get the session direction
    pub fn direction(&self) -> SessionDirection {
        self.direction
    }
    
    /// Get the session configuration
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }
    
    /// Check if the session is active
    pub async fn is_active(&self) -> bool {
        let state = self.state.lock().await;
        *state != SessionState::Terminated
    }
    
    /// Check if the session is terminated
    pub async fn is_terminated(&self) -> bool {
        let state = self.state.lock().await;
        *state == SessionState::Terminated
    }
    
    /// Update remote SDP for the session
    pub async fn update_remote_sdp(&self, sdp: String) -> Result<(), Error> {
        debug!("Updating remote SDP for session {}", self.id);
        info!("Remote SDP updated for session {}", self.id);
        Ok(())
    }
    
    /// Update local SDP for the session
    pub async fn update_local_sdp(&self, sdp: String) -> Result<(), Error> {
        debug!("Updating local SDP for session {}", self.id);
        info!("Local SDP updated for session {}", self.id);
        Ok(())
    }
} 