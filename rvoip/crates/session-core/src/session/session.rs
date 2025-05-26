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
use super::session_types::{
    SessionState, SessionDirection, SessionTransactionType,
    TransferId, TransferState, TransferType, TransferContext
};
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
    
    /// Current transfer context (if transfer is in progress)
    transfer_context: Arc<Mutex<Option<TransferContext>>>,
    
    /// Transfer history for this session
    transfer_history: Arc<Mutex<Vec<TransferContext>>>,
    
    /// Consultation session ID (for attended transfers)
    consultation_session_id: Arc<Mutex<Option<SessionId>>>,
    
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
            transfer_context: Arc::new(Mutex::new(None)),
            transfer_history: Arc::new(Mutex::new(Vec::new())),
            consultation_session_id: Arc::new(Mutex::new(None)),
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
    
    // ==== Call Transfer Methods (REFER Support) ====
    
    /// Initiate a call transfer (send REFER)
    pub async fn initiate_transfer(&self, target_uri: String, transfer_type: TransferType, referred_by: Option<String>) -> Result<TransferId, Error> {
        // Check if session is in a valid state for transfer
        let state = self.state().await;
        if !matches!(state, SessionState::Connected | SessionState::OnHold) {
            return Err(Error::InvalidSessionStateTransition {
                from: state.to_string(),
                to: "transferring".to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Session must be connected or on hold to initiate transfer".to_string()),
                    ..Default::default()
                }
            });
        }
        
        // Check if there's already a transfer in progress
        {
            let current_transfer = self.transfer_context.lock().await;
            if current_transfer.is_some() {
                return Err(Error::InvalidSessionStateTransition {
                    from: "transfer_in_progress".to_string(),
                    to: "new_transfer".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Transfer already in progress".to_string()),
                        ..Default::default()
                    }
                });
            }
        }
        
        // Create transfer context
        let transfer_context = TransferContext {
            id: TransferId::new(),
            transfer_type,
            state: TransferState::Initiated,
            target_uri: target_uri.clone(),
            transferor_session_id: Some(self.id.clone()),
            transferee_session_id: None,
            consultation_session_id: None,
            refer_to: target_uri.clone(),
            referred_by,
            reason: None,
            initiated_at: SystemTime::now(),
            completed_at: None,
        };
        
        let transfer_id = transfer_context.id.clone();
        
        // Store transfer context
        {
            let mut current_transfer = self.transfer_context.lock().await;
            *current_transfer = Some(transfer_context.clone());
        }
        
        // Update session state to transferring
        self.set_state(SessionState::Transferring).await?;
        
        // Publish transfer initiated event
        self.event_bus.publish(SessionEvent::TransferInitiated {
            session_id: self.id.clone(),
            transfer_id: transfer_id.to_string(),
            transfer_type: transfer_type.to_string(),
            target_uri: target_uri,
        });
        
        debug!("Initiated {} transfer for session {} to {}", transfer_type, self.id, transfer_context.refer_to);
        
        Ok(transfer_id)
    }
    
    /// Accept an incoming transfer request (respond to REFER)
    pub async fn accept_transfer(&self, transfer_id: &TransferId) -> Result<(), Error> {
        let mut transfer_guard = self.transfer_context.lock().await;
        
        if let Some(ref mut transfer_context) = transfer_guard.as_mut() {
            if transfer_context.id == *transfer_id {
                match transfer_context.state {
                    TransferState::Initiated => {
                        transfer_context.state = TransferState::Accepted;
                        
                        // Publish transfer accepted event
                        self.event_bus.publish(SessionEvent::TransferAccepted {
                            session_id: self.id.clone(),
                            transfer_id: transfer_id.to_string(),
                        });
                        
                        debug!("Accepted transfer {} for session {}", transfer_id, self.id);
                        Ok(())
                    },
                    _ => {
                        Err(Error::InvalidSessionStateTransition {
                            from: transfer_context.state.to_string(),
                            to: "accepted".to_string(),
                            context: ErrorContext {
                                category: ErrorCategory::Session,
                                severity: ErrorSeverity::Error,
                                recovery: RecoveryAction::None,
                                retryable: false,
                                session_id: Some(self.id.to_string()),
                                timestamp: SystemTime::now(),
                                details: Some("Transfer not in initiated state".to_string()),
                                ..Default::default()
                            }
                        })
                    }
                }
            } else {
                Err(Error::InvalidSessionStateTransition {
                    from: "unknown_transfer".to_string(),
                    to: "accepted".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Transfer ID mismatch".to_string()),
                        ..Default::default()
                    }
                })
            }
        } else {
            Err(Error::InvalidSessionStateTransition {
                from: "no_transfer".to_string(),
                to: "accepted".to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("No transfer in progress".to_string()),
                    ..Default::default()
                }
            })
        }
    }
    
    /// Get current transfer context
    pub async fn current_transfer(&self) -> Option<TransferContext> {
        self.transfer_context.lock().await.clone()
    }
    
    /// Get transfer history
    pub async fn transfer_history(&self) -> Vec<TransferContext> {
        self.transfer_history.lock().await.clone()
    }
    
    /// Check if transfer is in progress
    pub async fn has_transfer_in_progress(&self) -> bool {
        self.transfer_context.lock().await.is_some()
    }
    
    /// Set consultation session for attended transfer
    pub async fn set_consultation_session(&self, consultation_session_id: Option<SessionId>) {
        let mut guard = self.consultation_session_id.lock().await;
        *guard = consultation_session_id;
    }
    
    /// Get consultation session ID
    pub async fn consultation_session_id(&self) -> Option<SessionId> {
        self.consultation_session_id.lock().await.clone()
    }
    
    /// Update transfer progress (NOTIFY handling)
    pub async fn update_transfer_progress(&self, transfer_id: &TransferId, status: String) -> Result<(), Error> {
        let transfer_guard = self.transfer_context.lock().await;
        
        if let Some(ref transfer_context) = transfer_guard.as_ref() {
            if transfer_context.id == *transfer_id {
                // Publish transfer progress event
                self.event_bus.publish(SessionEvent::TransferProgress {
                    session_id: self.id.clone(),
                    transfer_id: transfer_id.to_string(),
                    status: status.clone(),
                });
                
                debug!("Transfer {} progress for session {}: {}", transfer_id, self.id, status);
                Ok(())
            } else {
                Err(Error::InvalidSessionStateTransition {
                    from: "unknown_transfer".to_string(),
                    to: "progress".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Transfer ID mismatch".to_string()),
                        ..Default::default()
                    }
                })
            }
        } else {
            Err(Error::InvalidSessionStateTransition {
                from: "no_transfer".to_string(),
                to: "progress".to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("No transfer in progress".to_string()),
                    ..Default::default()
                }
            })
        }
    }
    
    /// Complete a transfer successfully
    pub async fn complete_transfer(&self, transfer_id: &TransferId, final_status: String) -> Result<(), Error> {
        let mut transfer_context = {
            let mut transfer_guard = self.transfer_context.lock().await;
            
            if let Some(mut transfer_context) = transfer_guard.take() {
                if transfer_context.id == *transfer_id {
                    transfer_context.state = TransferState::Confirmed;
                    transfer_context.completed_at = Some(SystemTime::now());
                    transfer_context
                } else {
                    return Err(Error::InvalidSessionStateTransition {
                        from: "unknown_transfer".to_string(),
                        to: "completed".to_string(),
                        context: ErrorContext {
                            category: ErrorCategory::Session,
                            severity: ErrorSeverity::Error,
                            recovery: RecoveryAction::None,
                            retryable: false,
                            session_id: Some(self.id.to_string()),
                            timestamp: SystemTime::now(),
                            details: Some("Transfer ID mismatch".to_string()),
                            ..Default::default()
                        }
                    });
                }
            } else {
                return Err(Error::InvalidSessionStateTransition {
                    from: "no_transfer".to_string(),
                    to: "completed".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("No transfer in progress".to_string()),
                        ..Default::default()
                    }
                });
            }
        };
        
        // Add to transfer history
        {
            let mut history = self.transfer_history.lock().await;
            history.push(transfer_context);
        }
        
        // Update session state back to connected or terminate if this was the transferor
        self.set_state(SessionState::Terminated).await?;
        
        // Publish transfer completed event
        self.event_bus.publish(SessionEvent::TransferCompleted {
            session_id: self.id.clone(),
            transfer_id: transfer_id.to_string(),
            final_status: final_status.clone(),
        });
        
        debug!("Completed transfer {} for session {} with status: {}", transfer_id, self.id, final_status);
        
        Ok(())
    }
    
    /// Fail a transfer
    pub async fn fail_transfer(&self, transfer_id: &TransferId, reason: String) -> Result<(), Error> {
        let mut transfer_context = {
            let mut transfer_guard = self.transfer_context.lock().await;
            
            if let Some(mut transfer_context) = transfer_guard.take() {
                if transfer_context.id == *transfer_id {
                    transfer_context.state = TransferState::Failed(reason.clone());
                    transfer_context.completed_at = Some(SystemTime::now());
                    transfer_context
                } else {
                    return Err(Error::InvalidSessionStateTransition {
                        from: "unknown_transfer".to_string(),
                        to: "failed".to_string(),
                        context: ErrorContext {
                            category: ErrorCategory::Session,
                            severity: ErrorSeverity::Error,
                            recovery: RecoveryAction::None,
                            retryable: false,
                            session_id: Some(self.id.to_string()),
                            timestamp: SystemTime::now(),
                            details: Some("Transfer ID mismatch".to_string()),
                            ..Default::default()
                        }
                    });
                }
            } else {
                return Err(Error::InvalidSessionStateTransition {
                    from: "no_transfer".to_string(),
                    to: "failed".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("No transfer in progress".to_string()),
                        ..Default::default()
                    }
                });
            }
        };
        
        // Add to transfer history
        {
            let mut history = self.transfer_history.lock().await;
            history.push(transfer_context);
        }
        
        // Update session state back to connected
        self.set_state(SessionState::Connected).await?;
        
        // Publish transfer failed event
        self.event_bus.publish(SessionEvent::TransferFailed {
            session_id: self.id.clone(),
            transfer_id: transfer_id.to_string(),
            reason: reason.clone(),
        });
        
        error!("Transfer {} failed for session {}: {}", transfer_id, self.id, reason);
        
        Ok(())
    }
} 