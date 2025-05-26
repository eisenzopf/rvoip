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
use rvoip_sip_core::Request;

use crate::dialog::{Dialog, DialogId, DialogManager};
use crate::dialog::DialogState;
use crate::events::{EventBus, SessionEvent};
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::media::{MediaManager, MediaSessionId, MediaConfig, MediaStatus};
use crate::sdp::SessionDescription;
use super::SessionConfig;
use super::session::Session;
use super::SessionId;
use super::SessionState;
use super::SessionDirection;

// Constants for configuration
const DEFAULT_EVENT_CHANNEL_SIZE: usize = 100;
const CLEANUP_INTERVAL_MS: u64 = 30000; // 30 seconds

/// Manager for SIP sessions with integrated media coordination
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
    
    /// Media manager for RTP stream coordination
    media_manager: Arc<MediaManager>,
    
    /// Event bus for session events
    event_bus: EventBus,
    
    /// Running flag
    running: Arc<std::sync::atomic::AtomicBool>,
    
    /// Event channel for session-specific events
    event_sender: mpsc::Sender<SessionEvent>,
}

impl SessionManager {
    /// Create a new session manager with integrated media coordination
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: SessionConfig,
        event_bus: EventBus
    ) -> Result<Self, Error> {
        // Create a dialog manager
        let dialog_manager = DialogManager::new(transaction_manager.clone(), event_bus.clone());
        
        // Create media manager with zero-copy event system
        let media_manager = MediaManager::new().await
            .map_err(|e| Error::InternalError(
                format!("Failed to create media manager: {}", e),
                ErrorContext {
                    category: ErrorCategory::Internal,
                    severity: ErrorSeverity::Critical,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    timestamp: SystemTime::now(),
                    details: Some("Media manager initialization failed".to_string()),
                    ..Default::default()
                }
            ))?;
        
        // Create the session event channel
        let (event_sender, event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        let session_manager = Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager: Arc::new(dialog_manager),
            media_manager: Arc::new(media_manager),
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
        
        Ok(session_manager)
    }
    
    /// Create a new session manager (legacy method for backward compatibility)
    pub fn new_sync(
        transaction_manager: Arc<TransactionManager>,
        config: SessionConfig,
        event_bus: EventBus
    ) -> Self {
        // Create a dialog manager
        let dialog_manager = DialogManager::new(transaction_manager.clone(), event_bus.clone());
        
        // Create the session event channel
        let (event_sender, event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        // Create a runtime for media manager initialization
        let rt = tokio::runtime::Handle::current();
        let media_manager = rt.block_on(async {
            MediaManager::new().await.unwrap_or_else(|e| {
                error!("Failed to create media manager: {}", e);
                panic!("Media manager initialization failed");
            })
        });
        
        let session_manager = Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager: Arc::new(dialog_manager),
            media_manager: Arc::new(media_manager),
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

    /// Terminate a session and clean up resources
    pub async fn terminate_session(&self, session_id: &SessionId, reason: Option<String>) -> Result<(), Error> {
        // Implementation details...
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
                    match method {
                        rvoip_sip_core::Method::Bye => {
                            // BYE request indicates session termination
                            let _ = session.set_state(SessionState::Terminating).await;
                        },
                        _ => {}
                    }
                    
                    return true;
                }
            }
        }
        
        false
    }
    
    // ==== Media Coordination Methods (Task 2 from Integration Plan) ====
    
    /// Get reference to the media manager
    pub fn media_manager(&self) -> &Arc<MediaManager> {
        &self.media_manager
    }
    
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
    
    // ==== Enhanced Session Creation Methods (Priority A from Integration Plan) ====
    
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
    
    // ==== Call Transfer Methods (REFER Support) ====
    
    /// Initiate a call transfer for a session
    pub async fn initiate_transfer(
        &self, 
        session_id: &SessionId, 
        target_uri: String, 
        transfer_type: crate::session::session_types::TransferType,
        referred_by: Option<String>
    ) -> Result<crate::session::session_types::TransferId, Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Delegate to the session
        session.initiate_transfer(target_uri, transfer_type, referred_by).await
    }
    
    /// Handle an incoming REFER request
    pub async fn handle_refer_request(
        &self,
        refer_request: &rvoip_sip_core::Request,
        dialog_id: &crate::dialog::DialogId
    ) -> Result<crate::session::session_types::TransferId, Error> {
        // Find the session for this dialog
        let session = self.find_session_by_dialog(dialog_id)?;
        
        // Extract transfer information from REFER request
        // TODO: Replace with proper header parsing once SIP header access is available
        let refer_to = "sip:placeholder@example.com"; // placeholder for refer_request.header("Refer-To")
        let referred_by: Option<String> = None; // placeholder for refer_request.header("Referred-By")
        
        // Extract transfer type from Refer-To header
        let transfer_type = if refer_to.contains("Replaces=") {
            crate::session::session_types::TransferType::Attended
        } else {
            crate::session::session_types::TransferType::Blind
        };
        
        // Initiate the transfer
        let transfer_id = session.initiate_transfer(
            refer_to.to_string(),
            transfer_type,
            referred_by
        ).await?;
        
        // Accept the transfer immediately (this sends 202 Accepted)
        session.accept_transfer(&transfer_id).await?;
        
        debug!("Handled REFER request for session {}, transfer ID: {}", session.id, transfer_id);
        
        Ok(transfer_id)
    }
    
    /// Create a consultation call for attended transfer
    pub async fn create_consultation_call(
        &self,
        original_session_id: &SessionId,
        target_uri: String
    ) -> Result<Arc<Session>, Error> {
        // Get the original session
        let original_session = self.get_session(original_session_id)?;
        
        // Create a new outgoing session for consultation
        let consultation_session = self.create_outgoing_session().await?;
        
        // Link the consultation session to the original session
        original_session.set_consultation_session(Some(consultation_session.id.clone())).await;
        
        // Publish consultation call created event
        self.event_bus.publish(SessionEvent::ConsultationCallCreated {
            original_session_id: original_session_id.clone(),
            consultation_session_id: consultation_session.id.clone(),
            transfer_id: "consultation".to_string(), // Would be a real transfer ID in full implementation
        });
        
        debug!("Created consultation call {} for original session {}", consultation_session.id, original_session_id);
        
        Ok(consultation_session)
    }
    
    /// Complete an attended transfer by connecting two sessions
    pub async fn complete_attended_transfer(
        &self,
        transfer_id: &crate::session::session_types::TransferId,
        transferor_session_id: &SessionId,
        transferee_session_id: &SessionId
    ) -> Result<(), Error> {
        // Get both sessions
        let transferor_session = self.get_session(transferor_session_id)?;
        let transferee_session = self.get_session(transferee_session_id)?;
        
        // Setup RTP relay between the sessions
        let relay_id = self.setup_rtp_relay(transferor_session_id, transferee_session_id).await?;
        
        // Complete the transfer on the transferor session
        transferor_session.complete_transfer(transfer_id, "200 OK".to_string()).await?;
        
        // Publish completion event
        self.event_bus.publish(SessionEvent::ConsultationCallCompleted {
            original_session_id: transferor_session_id.clone(),
            consultation_session_id: transferee_session_id.clone(),
            transfer_id: transfer_id.to_string(),
            success: true,
        });
        
        debug!("Completed attended transfer {}, relay ID: {:?}", transfer_id, relay_id);
        
        Ok(())
    }
    
    /// Handle transfer progress notifications (NOTIFY)
    pub async fn handle_transfer_notify(
        &self,
        notify_request: &rvoip_sip_core::Request,
        dialog_id: &crate::dialog::DialogId
    ) -> Result<(), Error> {
        // Find the session for this dialog
        let session = self.find_session_by_dialog(dialog_id)?;
        
        // Extract transfer status from NOTIFY body
        let status = if notify_request.body().len() > 0 {
            // Parse the subscription state - simplified for this implementation
            let body_str = String::from_utf8_lossy(notify_request.body());
            if body_str.contains("200") {
                "200 OK".to_string()
            } else if body_str.contains("100") {
                "100 Trying".to_string()
            } else {
                body_str.to_string()
            }
        } else {
            "Unknown status".to_string()
        };
        
        // Find the current transfer for this session
        if let Some(transfer_context) = session.current_transfer().await {
            session.update_transfer_progress(&transfer_context.id, status.clone()).await?;
            
            // If this is a final success response, complete the transfer
            if status.contains("200") {
                session.complete_transfer(&transfer_context.id, status.clone()).await?;
            } else if status.contains("4") || status.contains("5") || status.contains("6") {
                // Error response, fail the transfer
                session.fail_transfer(&transfer_context.id, status.clone()).await?;
            }
        }
        
        debug!("Handled transfer NOTIFY for dialog {}: {}", dialog_id, status);
        
        Ok(())
    }
    
    /// Get all sessions with active transfers
    pub async fn get_sessions_with_transfers(&self) -> Vec<Arc<Session>> {
        let mut sessions_with_transfers = Vec::new();
        
        for entry in self.sessions.iter() {
            let session = entry.value().clone();
            if session.has_transfer_in_progress().await {
                sessions_with_transfers.push(session);
            }
        }
        
        sessions_with_transfers
    }
    
    /// Cancel an ongoing transfer
    pub async fn cancel_transfer(
        &self,
        session_id: &SessionId,
        transfer_id: &crate::session::session_types::TransferId,
        reason: String
    ) -> Result<(), Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Fail the transfer
        session.fail_transfer(transfer_id, reason).await?;
        
        debug!("Cancelled transfer {} for session {}", transfer_id, session_id);
        
        Ok(())
    }
    
    /// Handle blind transfer completion
    pub async fn handle_blind_transfer_completion(
        &self,
        session_id: &SessionId,
        transfer_id: &crate::session::session_types::TransferId
    ) -> Result<(), Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Complete the transfer and terminate the session
        session.complete_transfer(transfer_id, "200 OK".to_string()).await?;
        
        debug!("Completed blind transfer {} for session {}", transfer_id, session_id);
        
        Ok(())
    }
} 