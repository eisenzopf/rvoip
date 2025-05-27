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
use rvoip_sip_core::Request;

use crate::dialog::{Dialog, DialogId, DialogManager};
use crate::events::{EventBus, SessionEvent};
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::media::MediaManager;
use super::super::SessionConfig;
use super::super::session::Session;
use super::super::SessionId;
use super::super::SessionState;
use super::super::SessionDirection;

// Constants for configuration
const DEFAULT_EVENT_CHANNEL_SIZE: usize = 100;

/// Manager for SIP sessions with integrated media coordination
#[derive(Clone)]
pub struct SessionManager {
    /// Session manager configuration
    pub(crate) config: SessionConfig,
    
    /// Active sessions by ID
    pub(crate) sessions: Arc<DashMap<SessionId, Arc<Session>>>,
    
    /// Default dialog for each session
    pub(crate) default_dialogs: DashMap<SessionId, DialogId>,
    
    /// Mapping between dialogs and sessions
    pub(crate) dialog_to_session: DashMap<DialogId, SessionId>,
    
    /// Transaction manager reference
    pub(crate) transaction_manager: Arc<TransactionManager>,
    
    /// Dialog manager reference
    pub(crate) dialog_manager: Arc<DialogManager>,
    
    /// Media manager for RTP stream coordination
    pub(crate) media_manager: Arc<MediaManager>,
    
    /// Event bus for session events
    pub(crate) event_bus: EventBus,
    
    /// Running flag
    pub(crate) running: Arc<std::sync::atomic::AtomicBool>,
    
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
    
    /// Create a new session manager with default event bus
    pub async fn new_with_default_events(
        transaction_manager: Arc<TransactionManager>,
        config: SessionConfig,
    ) -> Result<Self, Error> {
        // Create default zero-copy event bus
        let event_bus = EventBus::new(1000).await
            .map_err(|e| Error::InternalError(
                format!("Failed to create event bus: {}", e),
                ErrorContext {
                    category: ErrorCategory::Internal,
                    severity: ErrorSeverity::Critical,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    timestamp: SystemTime::now(),
                    details: Some("Event bus initialization failed".to_string()),
                    ..Default::default()
                }
            ))?;
        
        Self::new(transaction_manager, config, event_bus).await
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
    
    /// Get a reference to the dialog manager
    pub fn dialog_manager(&self) -> &Arc<DialogManager> {
        &self.dialog_manager
    }
    
    /// Get the current number of active sessions
    pub async fn session_count(&self) -> usize {
        self.sessions.len()
    }
    
    /// Check if we're below the max session limit
    pub(crate) async fn can_create_session(&self) -> bool {
        if let Some(max_sessions) = self.config.max_sessions {
            return self.sessions.len() < max_sessions;
        }
        true
    }
    
    /// Get session with dialog
    pub fn get_session_with_dialog(&self, session_id: &SessionId) -> Result<Arc<Session>, Error> {
        // Get the session
        match self.get_session(session_id) {
            Ok(session) => Ok(session),
            Err(e) => Err(e)
        }
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
    
    /// Get reference to the media manager
    pub fn media_manager(&self) -> &Arc<MediaManager> {
        &self.media_manager
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
                _ => {}
            }
            
            // Forward the event to the zero-copy event bus (async)
            if let Err(e) = self.event_bus.publish(event).await {
                error!("Failed to publish event to zero-copy event bus: {}", e);
            }
        }
    }

    /// Create a new session manager with call lifecycle coordinator for automatic call handling
    pub async fn new_with_call_coordinator(
        transaction_manager: Arc<TransactionManager>,
        config: SessionConfig,
        event_bus: EventBus,
        media_manager: Arc<MediaManager>
    ) -> Result<Self, Error> {
        // Create a dialog manager with call lifecycle coordinator
        let (dialog_manager, _call_lifecycle_coordinator) = DialogManager::new_with_call_coordinator(
            transaction_manager.clone(), 
            event_bus.clone(),
            media_manager.clone()
        );
        
        // Create the session event channel
        let (event_sender, event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        let session_manager = Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager,
            media_manager,
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
        
        // Start the dialog manager
        let _ = session_manager.dialog_manager.start().await;
        
        info!("✅ SessionManager created with automatic call lifecycle coordination");
        
        Ok(session_manager)
    }
} 