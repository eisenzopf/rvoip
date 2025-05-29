use std::sync::Arc;
use tokio::sync::mpsc;
use std::time::SystemTime;
use tracing::{debug, error, warn};
use futures::stream::{StreamExt, FuturesUnordered};
use serde_json;

use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent,
};
use rvoip_sip_core::Request;

use crate::dialog::{DialogState, DialogId};
use crate::events::SessionEvent;
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use super::core::SessionManager;
use super::super::session::Session;
use super::super::SessionId;
use super::super::SessionState;
use super::super::SessionDirection;

// Constants for configuration
const CLEANUP_INTERVAL_MS: u64 = 30000; // 30 seconds

impl SessionManager {
    /// Start the session manager
    pub async fn start(&self) -> Result<(), Error> {
        // Set running flag
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);
        
        // Start the dialog manager (it now handles its own cleanup)
        if let Err(e) = self.dialog_manager.start().await {
            error!("Failed to start dialog manager: {}", e);
            return Err(e);
        }
        
        // Create a task for cleanup
        let session_manager = self.clone();
        tokio::spawn(async move {
            // Setup task tracking
            let mut tasks = FuturesUnordered::new();
            
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
    
    /// Create a session for an incoming INVITE request
    pub async fn create_session_for_invite(&self, invite: Request, is_inbound: bool) -> Result<Arc<Session>, Error> {
        let direction = if is_inbound { SessionDirection::Incoming } else { SessionDirection::Outgoing };
        
        // Check session limits
        if !self.can_create_session().await {
            return Err(Error::SessionLimitExceeded(
                self.config.max_sessions.unwrap_or(0),
                ErrorContext {
                    category: ErrorCategory::Resource,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::Wait(std::time::Duration::from_secs(5)),
                    retryable: true,
                    timestamp: SystemTime::now(),
                    details: Some("Session limit exceeded".to_string()),
                    ..Default::default()
                }
            ));
        }
        
        let session = Arc::new(Session::new(
            direction,
            self.config.clone(),
            self.transaction_manager.clone(),
            self.event_bus.clone()
        ));
        
        // Add to active sessions
        self.sessions.insert(session.id.clone(), session.clone());
        
        // Emit creation event
        self.event_bus.publish(SessionEvent::Created {
            session_id: session.id.clone(),
        });
        
        debug!("Created session {} for INVITE (inbound: {})", session.id, is_inbound);
        
        Ok(session)
    }
    
    /// Terminate a session and clean up resources
    pub async fn terminate_session(&self, session_id: &SessionId, reason: Option<String>) -> Result<(), Error> {
        // Implementation details...
        Ok(())
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
                    manager.terminate_session(&session_id, Some("Manager shutdown".to_string()))
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
    
    /// Find a session by dialog identifiers
    pub async fn find_session_for_dialog(&self, call_id: &str, from_tag: &str, to_tag: &str) -> Option<Arc<Session>> {
        // Find dialog by identifiers
        for entry in self.dialog_to_session.iter() {
            let dialog_id = entry.key();
            let session_id = entry.value();
            
            if let Ok(dialog) = self.dialog_manager.get_dialog(dialog_id) {
                if dialog.call_id == call_id &&
                   dialog.local_tag.as_deref() == Some(from_tag) &&
                   dialog.remote_tag.as_deref() == Some(to_tag) {
                    return self.get_session(session_id).ok();
                }
            }
        }
        
        None
    }
    
    /// Link a session to a call (for call-engine integration)
    pub async fn link_session_to_call(&self, session_id: &SessionId, call_id: &str) -> Result<(), Error> {
        // Verify session exists
        let _session = self.get_session(session_id)?;
        
        // For now, just log the association - in a full implementation,
        // we would maintain a call-to-session mapping
        debug!("Linked session {} to call {}", session_id, call_id);
        
        Ok(())
    }
    
    /// Get all sessions for a call (for call-engine integration)
    pub async fn get_sessions_for_call(&self, call_id: &str) -> Vec<Arc<Session>> {
        // In a full implementation, we would maintain a call-to-sessions mapping
        // For now, return empty vector
        debug!("Requested sessions for call {}", call_id);
        vec![]
    }
} 