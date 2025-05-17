use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::mpsc;
use std::time::SystemTime;
use tracing::{debug, info, error, warn};
use uuid::Uuid;
use futures::stream::{StreamExt, FuturesUnordered};
use serde_json;

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

// Constants for configuration
const DEFAULT_EVENT_CHANNEL_SIZE: usize = 100;
const CLEANUP_INTERVAL_MS: u64 = 30000; // 30 seconds

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
    
    /// Event channel for session-specific events
    event_sender: mpsc::Sender<SessionEvent>,
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
        
        // Create the session event channel
        let (event_sender, event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        let session_manager = Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager: Arc::new(dialog_manager),
            event_bus: event_bus.clone(),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            default_dialogs: DashMap::new(),
            dialog_to_session: DashMap::new(),
            event_sender,
        };
        
        // Start the session event processing
        let manager_clone = session_manager.clone();
        tokio::spawn(async move {
            manager_clone.process_session_events(event_receiver).await;
        });
        
        session_manager
    }
    
    /// Start the session manager
    pub async fn start(&self) -> Result<(), Error> {
        // Set running flag
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);
        
        // Start the dialog manager
        let events_rx = self.dialog_manager.start().await;
        
        // Create a task to process dialog manager events and cleanup
        let session_manager = self.clone();
        tokio::spawn(async move {
            // Setup task tracking
            let mut tasks = FuturesUnordered::new();
            
            // Create a task for processing dialog events
            let manager_clone = session_manager.clone();
            let dialog_task = tokio::spawn(async move {
                manager_clone.process_dialog_events(events_rx).await;
            });
            tasks.push(dialog_task);
            
            // Setup cleanup interval
            let mut cleanup_interval = tokio::time::interval(
                std::time::Duration::from_millis(CLEANUP_INTERVAL_MS)
            );
            
            // Main event loop
            loop {
                tokio::select! {
                    // Check if it's time to cleanup terminated sessions
                    _ = cleanup_interval.tick() => {
                        if session_manager.running.load(std::sync::atomic::Ordering::SeqCst) {
                            let manager_clone = session_manager.clone();
                            let cleanup_task = tokio::spawn(async move {
                                let count = manager_clone.cleanup_terminated().await;
                                if count > 0 {
                                    debug!("Cleaned up {} terminated sessions", count);
                                }
                            });
                            tasks.push(cleanup_task);
                        }
                    },
                    
                    // Process any completed tasks
                    Some(result) = tasks.next() => {
                        match result {
                            Ok(_) => {
                                // Task completed successfully
                            },
                            Err(e) => {
                                error!("Task error in session manager: {}", e);
                            }
                        }
                    },
                    
                    // Exit when all tasks are done
                    else => break,
                }
                
                // Check if we're still running
                if !session_manager.running.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
            }
            
            debug!("Session manager event processing stopped");
        });
        
        Ok(())
    }
    
    /// Process session-specific events
    async fn process_session_events(&self, mut rx: mpsc::Receiver<SessionEvent>) {
        while let Some(event) = rx.recv().await {
            match &event {
                SessionEvent::Terminated { session_id, reason } => {
                    // Handle session termination
                    debug!("Session {} terminated: {}", session_id, reason);
                    
                    // Try to update session state if it still exists
                    if let Ok(session) = self.get_session(session_id) {
                        let _ = session.set_state(SessionState::Terminated).await;
                    }
                    
                    // Remove from active sessions
                    self.sessions.remove(session_id);
                },
                _ => {
                    // Forward the event to the event bus
                    self.event_bus.publish(event);
                }
            }
        }
    }
    
    /// Process events from the dialog manager
    async fn process_dialog_events(&self, mut events_rx: mpsc::Receiver<TransactionEvent>) {
        while let Some(event) = events_rx.recv().await {
            if !self.running.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            
            // Handle events that might affect sessions
            match &event {
                TransactionEvent::Response { transaction_id, response, .. } => {
                    // Forward to any session associated with this transaction
                    if let Some(dialog_id) = self.dialog_manager.find_dialog_for_transaction(transaction_id) {
                        if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                            let session_id_clone = session_id.clone();
                            if let Ok(session) = self.get_session(&session_id_clone) {
                                // Process response based on type
                                if response.status().is_success() {
                                    debug!("Session {} received successful response for transaction {}", 
                                          session_id_clone, transaction_id);
                                } else if response.status().as_u16() >= 400 {
                                    debug!("Session {} received failure response for transaction {}: {}", 
                                          session_id_clone, transaction_id, response.status());
                                }
                            }
                        }
                    }
                },
                TransactionEvent::Error { transaction_id, error } => {
                    // Handle transaction errors that affect sessions
                    if let Some(tx_id) = transaction_id {
                        if let Some(dialog_id) = self.dialog_manager.find_dialog_for_transaction(tx_id) {
                            if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                                let session_id_clone = session_id.clone();
                                error!("Session {} transaction error: {}", session_id_clone, error);
                                
                                // Publish an error event
                                self.event_bus.publish(SessionEvent::Custom {
                                    session_id: session_id_clone.clone(),
                                    event_type: "transaction_error".to_string(),
                                    data: serde_json::json!({
                                        "transaction_id": tx_id.to_string(),
                                        "error": error.to_string()
                                    }),
                                });
                            }
                        }
                    }
                },
                _ => {
                    // Process other transaction events
                    debug!("Session manager received transaction event: {:?}", event);
                }
            }
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
        
        // Emit a creation event
        self.event_bus.publish(SessionEvent::Created {
            session_id: session.id.clone(),
        });
        
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
        
        // Emit a creation event
        self.event_bus.publish(SessionEvent::Created {
            session_id: session.id.clone(),
        });
        
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
    
    /// Terminate all active sessions with improved error handling
    pub async fn terminate_all(&self) -> Result<(), Error> {
        println!("Starting terminate_all process");
        let all_sessions = self.list_sessions();
        println!("Found {} total sessions to check", all_sessions.len());
        
        if all_sessions.is_empty() {
            println!("No sessions to terminate");
            return Ok(());
        }
        
        // Filter to only include non-terminated sessions
        let mut sessions = Vec::new();
        for session in all_sessions {
            let state = session.state().await;
            if state != SessionState::Terminated {
                sessions.push(session);
            }
        }
        
        println!("Found {} active non-terminated sessions to terminate", sessions.len());
        
        if sessions.is_empty() {
            println!("No non-terminated sessions to terminate");
            return Ok(());
        }
        
        // For each session, set to terminating without awaiting the results
        for session in &sessions {
            let _ = session.set_state(SessionState::Terminating).await;
        }
        
        // Set a reasonable total timeout based on session count
        let timeout_duration = std::time::Duration::from_secs(
            std::cmp::min(30, 5 + (sessions.len() as u64) / 100)
        );
        
        // Create futures for each termination operation
        let mut termination_futures = Vec::new();
        
        for session in sessions {
            let session_id = session.id.clone();
            let manager = self.clone();
            
            let future = async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    manager.terminate_session(&session_id, "Manager shutdown")
                ).await;
                
                match result {
                    Ok(Ok(_)) => {
                        println!("Session {} terminated successfully", session_id);
                        true
                    },
                    Ok(Err(e)) => {
                        error!("Error terminating session {}: {}", session_id, e);
                        // Remove from sessions map directly as a fallback
                        manager.sessions.remove(&session_id);
                        false
                    },
                    Err(_) => {
                        error!("Timeout terminating session {}", session_id);
                        // Remove from sessions map directly as a fallback
                        manager.sessions.remove(&session_id);
                        false
                    }
                }
            };
            
            termination_futures.push(future);
        }
        
        // Run all terminations with an overall timeout
        let results = match tokio::time::timeout(
            timeout_duration, 
            futures::future::join_all(termination_futures)
        ).await {
            Ok(results) => results,
            Err(_) => {
                error!("Global timeout waiting for all sessions to terminate");
                // Force cleanup of any remaining sessions
                self.force_cleanup_all_sessions();
                return Ok(());
            }
        };
        
        let success_count = results.iter().filter(|&success| *success).count();
        let failure_count = results.len() - success_count;
        
        println!("Terminated {} sessions successfully, {} failed", success_count, failure_count);
        
        // Cleanup any remaining terminated sessions
        let cleaned_up = self.cleanup_terminated().await;
        if cleaned_up > 0 {
            println!("Cleaned up {} terminated sessions", cleaned_up);
        }
        
        Ok(())
    }
    
    /// Force cleanup all sessions without waiting for termination
    fn force_cleanup_all_sessions(&self) {
        println!("Forcing cleanup of all sessions");
        
        // Get all session IDs
        let session_ids: Vec<_> = self.sessions.iter()
            .map(|entry| entry.key().clone())
            .collect();
            
        println!("Forcing removal of {} sessions", session_ids.len());
        
        // Remove all sessions and related mappings
        for id in session_ids {
            // Remove from sessions map
            self.sessions.remove(&id);
            
            // Remove from dialog mappings
            if let Some(dialog_id) = self.default_dialogs.get(&id) {
                let dialog_id = dialog_id.clone();
                self.dialog_to_session.remove(&dialog_id);
                self.default_dialogs.remove(&id);
            }
        }
        
        println!("Forced cleanup complete");
    }
    
    /// Clean up terminated sessions
    pub async fn cleanup_terminated(&self) -> usize {
        let mut count = 0;
        
        // First, collect all the session IDs for terminated sessions
        let session_ids: Vec<_> = {
            let mut terminated_ids = Vec::new();
            
            for entry in self.sessions.iter() {
                let id = entry.key().clone();
                let session = entry.value().clone();
                
                // Get the state without awaiting in the loop to avoid blocking
                // we'll check the actual state outside this loop
                drop(entry); // Release the read lock before async operations
                
                terminated_ids.push(id);
            }
            
            terminated_ids
        };
        
        // Now check each session state and remove terminated ones
        for id in session_ids {
            if let Ok(session) = self.get_session(&id) {
                let state = session.state().await;
                if state == SessionState::Terminated {
                    // Remove references in mappings
                    if let Some(dialog_id) = self.default_dialogs.get(&id) {
                        let dialog_id = dialog_id.clone();
                        self.dialog_to_session.remove(&dialog_id);
                    }
                    
                    // Remove from default dialogs
                    self.default_dialogs.remove(&id);
                    
                    // Remove the session
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
        println!("Stopping session manager");
        
        // Set running flag to false
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
        
        // Terminate all active sessions
        let terminate_result = match tokio::time::timeout(
            std::time::Duration::from_secs(10), 
            self.terminate_all()
        ).await {
            Ok(result) => result,
            Err(_) => {
                error!("Timeout terminating all sessions, forcing cleanup");
                self.force_cleanup_all_sessions();
                Ok(())
            }
        };
        
        if let Err(e) = terminate_result {
            error!("Error terminating sessions: {}", e);
        }
        
        // Stop the dialog manager
        if let Err(e) = self.dialog_manager.stop().await {
            error!("Error stopping dialog manager: {}", e);
        }
        
        println!("Session manager stopped");
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
        
        // Find any dialogs associated with this session and terminate them
        let dialog_ids: Vec<DialogId> = self.dialog_to_session.iter()
            .filter(|entry| entry.value() == session_id)
            .map(|entry| entry.key().clone())
            .collect();
        
        // Terminate each dialog
        for dialog_id in dialog_ids {
            if let Err(e) = self.dialog_manager.terminate_dialog(&dialog_id).await {
                warn!("Error terminating dialog {} for session {}: {}", dialog_id, session_id, e);
            }
            
            // Remove from mappings
            self.dialog_to_session.remove(&dialog_id);
        }
        
        // Publish event to our internal channel to handle cleanup asynchronously
        if let Err(e) = self.event_sender.send(SessionEvent::Terminated {
            session_id: session_id.clone(),
            reason: reason.to_string(),
        }).await {
            error!("Failed to send session termination event: {}", e);
            
            // As a fallback, set the state to terminated directly
            session.set_state(SessionState::Terminated).await?;
            self.sessions.remove(session_id);
        }
        
        // Also publish to the main event bus for external observers
        self.event_bus.publish(SessionEvent::Terminated {
            session_id: session_id.clone(),
            reason: reason.to_string(),
        });
        
        Ok(())
    }
    
    /// Find session by dialog
    pub fn find_session_by_dialog(&self, dialog_id: &DialogId) -> Result<Arc<Session>, Error> {
        if let Some(session_id) = self.dialog_to_session.get(dialog_id) {
            let session_id = session_id.clone();
            return self.get_session(&session_id);
        }
        
        Err(Error::session_not_found(&format!("No session found for dialog {}", dialog_id)))
    }

    /// Set default dialog for a session
    pub fn set_default_dialog(&self, session_id: &SessionId, dialog_id: &DialogId) -> Result<(), Error> {
        // Verify the session exists
        self.get_session(session_id)?;
        
        // Update the mappings
        self.default_dialogs.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        Ok(())
    }

    /// Check if a session with the given ID exists
    pub fn has_session(&self, id: &SessionId) -> bool {
        self.sessions.contains_key(id)
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
            if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                let session_id = session_id.clone();
                
                // Get the session
                if let Ok(session) = self.get_session(&session_id) {
                    // Map the SIP method to SessionTransactionType
                    let tx_type = match method {
                        rvoip_sip_core::Method::Invite => crate::session::SessionTransactionType::InitialInvite,
                        rvoip_sip_core::Method::Bye => crate::session::SessionTransactionType::Bye,
                        rvoip_sip_core::Method::Update => crate::session::SessionTransactionType::Update,
                        _ => crate::session::SessionTransactionType::Other(method.to_string()),
                    };
                    
                    // Track this transaction
                    session.track_transaction(transaction_id.clone(), tx_type).await;
                    
                    // Process specific methods that might affect session state
                    match *method {
                        rvoip_sip_core::Method::Bye => {
                            let _ = session.set_state(SessionState::Terminating).await;
                            
                            // Terminate the session asynchronously
                            let manager = self.clone();
                            let session_id = session_id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = manager.terminate_session(&session_id, "BYE received").await {
                                    error!("Failed to terminate session after BYE: {}", e);
                                }
                            });
                        },
                        _ => {}
                    }
                    
                    // We handled this transaction
                    return true;
                }
            }
            
            // No session found for this dialog
            debug!("No session found for dialog {}", dialog_id);
        }
        
        // We did not handle this transaction
        false
    }
} 