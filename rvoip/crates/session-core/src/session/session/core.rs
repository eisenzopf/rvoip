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
use super::super::session_id::SessionId;
use super::super::session_types::{
    SessionState, SessionDirection, SessionTransactionType,
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
    
    /// Transaction manager reference
    transaction_manager: Arc<TransactionManager>,
    
    /// Active dialog (if any)
    dialog: Arc<Mutex<Option<Dialog>>>,
    
    /// Active transactions for this session
    transactions: Arc<Mutex<HashMap<TransactionKey, SessionTransactionType>>>,
    
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
} 