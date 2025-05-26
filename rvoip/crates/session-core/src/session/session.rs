use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::time::SystemTime;
use tracing::{debug, info, error, warn};

use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent, 
    TransactionState, 
    TransactionKey,
    TransactionKind
};

use crate::dialog::{Dialog, DialogId};
use crate::dialog::DialogState;
use crate::events::{EventBus, SessionEvent};
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::media::{MediaSessionId, MediaStatus, QualityMetrics, RtpStreamInfo};
use super::session_id::SessionId;
use super::session_types::{SessionState, SessionDirection, SessionTransactionType};
use super::session_config::SessionConfig;

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
#[derive(Clone)]
pub struct Session {
    /// Unique session identifier
    pub id: SessionId,
    
    /// Current session state
    state: Arc<Mutex<SessionState>>,
    
    /// Current media state
    media_state: Arc<Mutex<SessionMediaState>>,
    
    /// Direction of the session (incoming or outgoing)
    direction: SessionDirection,
    
    /// Session configuration
    config: SessionConfig,
    
    /// Transaction manager reference
    transaction_manager: Arc<TransactionManager>,
    
    /// Active dialog (if any)
    dialog: Arc<Mutex<Option<Dialog>>>,
    
    /// Active transactions for this session
    transactions: Arc<Mutex<HashMap<TransactionKey, SessionTransactionType>>>,
    
    /// Media session ID (if media is active)
    media_session_id: Arc<Mutex<Option<MediaSessionId>>>,
    
    /// Latest media quality metrics
    media_metrics: Arc<Mutex<Option<QualityMetrics>>>,
    
    /// RTP stream information
    rtp_stream_info: Arc<Mutex<Option<RtpStreamInfo>>>,
    
    /// Event bus for publishing session events
    event_bus: EventBus,
}

impl Session {
    /// Create a new session with media support
    pub fn new(
        direction: SessionDirection,
        config: SessionConfig,
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus
    ) -> Self {
        let id = SessionId::new();
        let session = Self {
            id: id.clone(),
            state: Arc::new(Mutex::new(SessionState::Initializing)),
            media_state: Arc::new(Mutex::new(SessionMediaState::None)),
            direction,
            config,
            transaction_manager,
            dialog: Arc::new(Mutex::new(None)),
            transactions: Arc::new(Mutex::new(HashMap::new())),
            media_session_id: Arc::new(Mutex::new(None)),
            media_metrics: Arc::new(Mutex::new(None)),
            rtp_stream_info: Arc::new(Mutex::new(None)),
            event_bus: event_bus.clone(),
        };
        
        // Publish session creation event
        event_bus.publish(SessionEvent::Created { session_id: id });
        
        session
    }
    
    /// Get the current session state
    pub async fn state(&self) -> SessionState {
        *self.state.lock().await
    }
    
    /// Set a new session state
    pub async fn set_state(&self, new_state: SessionState) -> Result<(), Error> {
        let mut state_guard = self.state.lock().await;
        let old_state = state_guard.clone();
        
        // Validate state transition
        if !Self::is_valid_transition(&old_state, &new_state) {
            return Err(Error::InvalidSessionStateTransition {
                from: old_state.to_string(),
                to: new_state.to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Invalid state transition attempted from {} to {}", old_state, new_state)),
                    ..Default::default()
                }
            });
        }
        
        // Update state and emit event
        *state_guard = new_state.clone();
        
        // Drop lock before emitting event
        drop(state_guard);
        
        // Emit state changed event
        self.event_bus.publish(SessionEvent::StateChanged { 
            session_id: self.id.clone(),
            old_state,
            new_state,
        });
        
        Ok(())
    }
    
    /// Check if a state transition is valid
    fn is_valid_transition(from: &SessionState, to: &SessionState) -> bool {
        use SessionState::*;
        
        match (from, to) {
            // Valid transitions from Initializing
            (Initializing, Dialing) => true,
            (Initializing, Ringing) => true,
            (Initializing, Terminating) => true,
            (Initializing, Terminated) => true,
            
            // Valid transitions from Dialing
            (Dialing, Ringing) => true,
            (Dialing, Connected) => true,
            (Dialing, Terminating) => true,
            (Dialing, Terminated) => true,
            
            // Valid transitions from Ringing
            (Ringing, Connected) => true,
            (Ringing, Terminating) => true,
            (Ringing, Terminated) => true,
            
            // Valid transitions from Connected
            (Connected, OnHold) => true,
            (Connected, Transferring) => true,
            (Connected, Terminating) => true,
            (Connected, Terminated) => true,
            
            // Valid transitions from OnHold
            (OnHold, Connected) => true,
            (OnHold, Transferring) => true,
            (OnHold, Terminating) => true,
            (OnHold, Terminated) => true,
            
            // Valid transitions from Transferring
            (Transferring, Connected) => true,
            (Transferring, OnHold) => true,
            (Transferring, Terminating) => true,
            (Transferring, Terminated) => true,
            
            // Valid transitions from Terminating
            (Terminating, Terminated) => true,
            
            // No transitions from Terminated
            (Terminated, _) => false,
            
            // Any other transition is invalid
            _ => false,
        }
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
    
    /// Get the active dialog for this session (if any)
    pub async fn dialog(&self) -> Option<Dialog> {
        self.dialog.lock().await.clone()
    }
    
    /// Set the active dialog for this session
    pub async fn set_dialog(&self, dialog: Option<Dialog>) {
        let mut dialog_guard = self.dialog.lock().await;
        *dialog_guard = dialog;
    }
    
    /// Track a transaction associated with this session
    pub async fn track_transaction(&self, transaction_id: TransactionKey, tx_type: SessionTransactionType) {
        let mut txs = self.transactions.lock().await;
        txs.insert(transaction_id, tx_type);
    }
    
    /// Get the type of a tracked transaction
    pub async fn get_transaction_type(&self, transaction_id: &TransactionKey) -> Option<SessionTransactionType> {
        let txs = self.transactions.lock().await;
        txs.get(transaction_id).cloned()
    }
    
    /// Remove a transaction from tracking
    pub async fn remove_transaction(&self, transaction_id: &TransactionKey) -> Option<SessionTransactionType> {
        let mut txs = self.transactions.lock().await;
        txs.remove(transaction_id)
    }
    
    // ==== Enhanced Media Management Methods ====
    
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
            SessionMediaState::Active => {
                *media_state = SessionMediaState::Paused;
                debug!("Paused media for session {}", self.id);
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
                        details: Some("Media not active, cannot pause".to_string()),
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
                *media_state = SessionMediaState::Active;
                debug!("Resumed media for session {}", self.id);
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
                        details: Some("Media not paused, cannot resume".to_string()),
                        ..Default::default()
                    }
                })
            }
        }
    }
    
    /// Get the current media state
    pub async fn media_state(&self) -> SessionMediaState {
        self.media_state.lock().await.clone()
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