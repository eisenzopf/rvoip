use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::mpsc;
use std::time::SystemTime;
use tracing::{debug, info, error, warn};
use uuid::Uuid;

use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent,
};

use crate::dialog::{Dialog, DialogId, DialogManager};
use crate::dialog::DialogState;
use crate::events::{EventBus, SessionEvent};
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use super::SessionConfig;
use super::session::Session;
use super::SessionId;
use super::SessionState;
use super::SessionDirection;

/// Manager for SIP sessions
#[derive(Clone)]
pub struct SessionManager {
    /// Session manager configuration
    config: SessionConfig,
    
    /// Active sessions by ID
    sessions: Arc<DashMap<SessionId, Arc<Session>>>,
    
    /// Default dialog for each session
    default_dialogs: DashMap<SessionId, DialogId>,
    
    /// Mapping between dialogs and sessions
    dialog_to_session: DashMap<DialogId, SessionId>,
    
    /// Transaction manager reference
    transaction_manager: Arc<TransactionManager>,
    
    /// Dialog manager reference
    dialog_manager: Arc<DialogManager>,
    
    /// Event bus for session events
    event_bus: EventBus,
    
    /// Running flag
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        config: SessionConfig,
        event_bus: EventBus
    ) -> Self {
        // Create a dialog manager
        let dialog_manager = DialogManager::new(transaction_manager.clone(), event_bus.clone());
        
        Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager: Arc::new(dialog_manager),
            event_bus,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            default_dialogs: DashMap::new(),
            dialog_to_session: DashMap::new(),
        }
    }
    
    /// Start the session manager
    pub async fn start(&self) -> Result<(), Error> {
        // Set running flag
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);
        
        // Start the dialog manager
        let events_rx = self.dialog_manager.start().await;
        
        // Create a task to process dialog manager events
        let session_manager = self.clone();
        tokio::spawn(async move {
            session_manager.process_dialog_events(events_rx).await;
        });
        
        Ok(())
    }
    
    /// Process events from the dialog manager
    async fn process_dialog_events(&self, mut events_rx: mpsc::Receiver<TransactionEvent>) {
        while let Some(event) = events_rx.recv().await {
            if !self.running.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            
            // Process transaction events that might affect sessions
            // In a real implementation, this would handle various transaction events
            // For the test, we just need a placeholder
            debug!("Session manager received transaction event: {:?}", event);
        }
    }
    
    /// Create a new outgoing session
    pub async fn create_outgoing_session(&self) -> Result<Arc<Session>, Error> {
        // Check if we've hit session limit
        let active_count = self.sessions.len();
        if let Some(max_sessions) = self.config.max_sessions {
            if active_count >= max_sessions {
                return Err(Error::SessionLimitExceeded(
                    max_sessions,
                    ErrorContext {
                        category: ErrorCategory::Resource,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Wait(std::time::Duration::from_secs(5)),
                        retryable: true,
                        timestamp: SystemTime::now(),
                        details: Some(format!("Can't create more than {} sessions", max_sessions)),
                        ..Default::default()
                    }
                ));
            }
        }
        
        let session = Arc::new(Session::new(
            SessionDirection::Outgoing,
            self.config.clone(),
            self.transaction_manager.clone(),
            self.event_bus.clone()
        ));
        
        // Add to active sessions
        self.sessions.insert(session.id.clone(), session.clone());
        
        Ok(session)
    }
    
    /// Create a new incoming session
    pub async fn create_incoming_session(&self) -> Result<Arc<Session>, Error> {
        // Check if we've hit session limit
        let active_count = self.sessions.len();
        if let Some(max_sessions) = self.config.max_sessions {
            if active_count >= max_sessions {
                return Err(Error::SessionLimitExceeded(
                    max_sessions,
                    ErrorContext {
                        category: ErrorCategory::Resource,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Wait(std::time::Duration::from_secs(5)),
                        retryable: true,
                        timestamp: SystemTime::now(),
                        details: Some(format!("Can't create more than {} sessions", max_sessions)),
                        ..Default::default()
                    }
                ));
            }
        }
        
        let session = Arc::new(Session::new(
            SessionDirection::Incoming,
            self.config.clone(),
            self.transaction_manager.clone(),
            self.event_bus.clone()
        ));
        
        // Add to active sessions
        self.sessions.insert(session.id.clone(), session.clone());
        
        Ok(session)
    }
    
    /// Get a session by ID
    pub fn get_session(&self, id: &SessionId) -> Result<Arc<Session>, Error> {
        match self.sessions.get(id) {
            Some(session) => Ok(session.value().clone()),
            None => Err(Error::SessionNotFoundWithId(
                id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(id.to_string()),
                    timestamp: std::time::SystemTime::now(),
                    details: Some(format!("Session {} not found", id)),
                    ..Default::default()
                }
            )),
        }
    }
    
    /// Get a session by ID with error handling
    pub fn get_session_or_error(&self, session_id: &SessionId) -> Result<Arc<Session>, Error> {
        match self.get_session(session_id) {
            Ok(session) => Ok(session),
            Err(_) => Err(Error::session_not_found(&session_id.to_string()))
        }
    }
    
    /// List all active sessions
    pub fn list_sessions(&self) -> Vec<Arc<Session>> {
        self.sessions
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Terminate all active sessions
    pub async fn terminate_all(&self) -> Result<(), Error> {
        let sessions = self.list_sessions();
        
        for session in sessions {
            let current_state = session.state().await;
            
            // Skip already terminated or terminating sessions
            if current_state == SessionState::Terminated || current_state == SessionState::Terminating {
                continue;
            }
            
            // Set to terminating first
            if current_state != SessionState::Terminating {
                let _ = session.set_state(SessionState::Terminating).await;
            }
            
            // Then to terminated
            let _ = session.set_state(SessionState::Terminated).await;
        }
        
        Ok(())
    }
    
    /// Clean up terminated sessions
    pub async fn cleanup_terminated(&self) -> usize {
        let mut count = 0;
        
        // First, collect all the session IDs
        let session_ids: Vec<_> = self.sessions.iter().map(|entry| entry.key().clone()).collect();
        
        // Now check each session and remove terminated ones
        for id in session_ids {
            if let Ok(session) = self.get_session(&id) {
                let state = session.state().await;
                if state == SessionState::Terminated {
                    if self.sessions.remove(&id).is_some() {
                        count += 1;
                    }
                }
            }
        }
        
        count
    }
    
    /// Get a reference to the dialog manager
    pub fn dialog_manager(&self) -> &Arc<DialogManager> {
        &self.dialog_manager
    }
    
    /// Get the current number of active sessions
    pub async fn session_count(&self) -> usize {
        self.sessions.len()
    }
    
    /// Check if we're below the max session limit
    async fn can_create_session(&self) -> bool {
        if let Some(max_sessions) = self.config.max_sessions {
            return self.sessions.len() < max_sessions;
        }
        true
    }
    
    /// Stop the session manager
    pub async fn stop(&self) {
        // Set running flag to false
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    
    /// Get session with dialog
    pub fn get_session_with_dialog(&self, session_id: &SessionId) -> Result<Arc<Session>, Error> {
        // Get the session
        match self.get_session(session_id) {
            Ok(session) => Ok(session),
            Err(e) => Err(e)
        }
    }

    /// Terminate session
    pub async fn terminate_session(&self, session_id: &SessionId, reason: &str) -> Result<(), Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Set the session state to terminating
        session.set_state(SessionState::Terminating).await?;
        
        // Publish event
        self.event_bus.publish(SessionEvent::Terminated {
            session_id: session_id.clone(),
            reason: reason.to_string(),
        });
        
        // Set the session state to terminated
        session.set_state(SessionState::Terminated).await?;
        
        // Remove the session from the repository
        self.sessions.remove(session_id);
        
        Ok(())
    }
    
    /// Find session by dialog
    pub fn find_session_by_dialog(&self, dialog_id: &DialogId) -> Result<Arc<Session>, Error> {
        for entry in self.dialog_to_session.iter() {
            if entry.key() == dialog_id {
                let id = entry.value().clone();
                return self.get_session(&id);
            }
        }
        
        Err(Error::session_not_found(&format!("No session found for dialog {}", dialog_id)))
    }

    /// Set default dialog for a session
    pub fn set_default_dialog(&self, session_id: &SessionId, dialog_id: &DialogId) -> Result<(), Error> {
        // Verify the session exists
        self.get_session(session_id)?;
        
        // Set the default dialog
        self.default_dialogs.insert(session_id.clone(), dialog_id.clone());
        Ok(())
    }

    /// Check if a session with the given ID exists
    pub fn has_session(&self, id: &SessionId) -> bool {
        match self.get_session(id) {
            Ok(_) => true,
            Err(_) => false
        }
    }

    // A helper to handle session-based transaction
    async fn handle_session_based_transaction(
        &self,
        transaction_id: &rvoip_transaction_core::TransactionKey,
        method: &rvoip_sip_core::Method,
    ) -> bool {
        // Get the dialog associated with this transaction if any
        if let Some(dialog_id) = self.dialog_manager.find_dialog_for_transaction(transaction_id) {
            // Find the session for this dialog
            if let Ok(session) = self.find_session_by_dialog(&dialog_id) {
                // Map the SIP method to SessionTransactionType
                let tx_type = match method {
                    rvoip_sip_core::Method::Invite => crate::session::SessionTransactionType::InitialInvite,
                    rvoip_sip_core::Method::Bye => crate::session::SessionTransactionType::Bye,
                    rvoip_sip_core::Method::Update => crate::session::SessionTransactionType::Update,
                    _ => crate::session::SessionTransactionType::Other(method.to_string()),
                };
                
                session.track_transaction(transaction_id.clone(), tx_type).await;
                
                // Process specific methods that might affect session state
                match *method {
                    rvoip_sip_core::Method::Bye => {
                        let _ = session.set_state(SessionState::Terminating).await;
                    },
                    _ => {}
                }
                
                // We handled this transaction
                return true;
            } else {
                // No session found for this dialog
                debug!("No session found for dialog {}", dialog_id);
            }
        }
        
        // We did not handle this transaction
        false
    }
} 