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
use super::session_id::SessionId;
use super::session_types::{SessionState, SessionDirection, SessionTransactionType};
use super::session_config::SessionConfig;

/// Represents a SIP session (call)
#[derive(Clone)]
pub struct Session {
    /// Unique session identifier
    pub id: SessionId,
    
    /// Current session state
    state: Arc<Mutex<SessionState>>,
    
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
    
    /// Event bus for publishing session events
    event_bus: EventBus,
}

impl Session {
    /// Create a new session
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
            direction,
            config,
            transaction_manager,
            dialog: Arc::new(Mutex::new(None)),
            transactions: Arc::new(Mutex::new(HashMap::new())),
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
    
    /// Start media for this session (basic implementation for tests)
    pub async fn start_media(&self) -> Result<(), Error> {
        // This is a mock implementation for tests
        debug!("Starting media for session {}", self.id);
        Ok(())
    }
    
    /// Stop media for this session (basic implementation for tests)
    pub async fn stop_media(&self) -> Result<(), Error> {
        // This is a mock implementation for tests
        debug!("Stopping media for session {}", self.id);
        Ok(())
    }
} 