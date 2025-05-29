use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::mpsc;
use std::time::SystemTime;
use tracing::{debug, info, error, warn};
use uuid::Uuid;
use std::collections::HashMap;
use std::net::SocketAddr;
use anyhow::{Result, Context};
use tokio::sync::RwLock;

use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent,
    TransactionKey,
};
use rvoip_sip_core::{Request, Response, StatusCode, Method};
use rvoip_sip_core::json::ext::SipMessageJson;

use crate::dialog::{Dialog, DialogId, DialogManager};
use crate::events::{EventBus, SessionEvent};
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::media::MediaManager;
use super::super::SessionConfig;
use super::super::session::Session;
use super::super::SessionId;
use super::super::SessionState;
use super::super::SessionDirection;
use crate::api::server::{IncomingCallEvent, CallerInfo, CallDecision, IncomingCallNotification};
use crate::session::CallLifecycleCoordinator;

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
    
    /// **NEW**: Call lifecycle coordinator for session-level coordination
    pub(crate) call_lifecycle_coordinator: Arc<CallLifecycleCoordinator>,
    
    /// Event bus for session events
    pub(crate) event_bus: EventBus,
    
    /// Running flag
    pub(crate) running: Arc<std::sync::atomic::AtomicBool>,
    
    /// Event channel for session-specific events
    event_sender: mpsc::Sender<SessionEvent>,
    
    /// **NEW**: Pending incoming calls (Call-ID -> (SessionId, TransactionKey, Request))
    pending_calls: Arc<RwLock<HashMap<String, (SessionId, TransactionKey, Request)>>>,
    
    /// **FIXED**: Incoming call notification callback with interior mutability
    incoming_call_notifier: Arc<RwLock<Option<Arc<dyn IncomingCallNotification>>>>,
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
        let media_manager = Arc::new(MediaManager::new().await
            .map_err(|e| Error::InternalError(
                format!("Failed to create media manager: {}", e),
                ErrorContext::default().with_message("Media manager initialization failed")
            ))?);
        
        // Create call lifecycle coordinator with media manager
        let call_lifecycle_coordinator = CallLifecycleCoordinator::new(media_manager.clone());
        
        // Create the session event channel
        let (event_sender, event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        let session_manager = Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager: Arc::new(dialog_manager),
            media_manager,
            call_lifecycle_coordinator: Arc::new(call_lifecycle_coordinator),
            event_bus: event_bus.clone(),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            default_dialogs: DashMap::new(),
            dialog_to_session: DashMap::new(),
            event_sender,
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            incoming_call_notifier: Arc::new(RwLock::new(None)),
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
                ErrorContext::default().with_message("Event bus initialization failed")
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
        let media_manager = Arc::new(rt.block_on(async {
            MediaManager::new().await.unwrap_or_else(|e| {
                error!("Failed to create media manager: {}", e);
                panic!("Media manager initialization failed");
            })
        }));
        
        // Create call lifecycle coordinator with media manager
        let call_lifecycle_coordinator = CallLifecycleCoordinator::new(media_manager.clone());
        
        let session_manager = Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager: Arc::new(dialog_manager),
            media_manager,
            call_lifecycle_coordinator: Arc::new(call_lifecycle_coordinator),
            event_bus: event_bus.clone(),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            default_dialogs: DashMap::new(),
            dialog_to_session: DashMap::new(),
            event_sender,
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            incoming_call_notifier: Arc::new(RwLock::new(None)),
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
                ErrorContext::default().with_message(&format!("Session {} not found", id))
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
        // Create a dialog manager (without call lifecycle coordinator - that's now in session layer)
        let dialog_manager = DialogManager::new(transaction_manager.clone(), event_bus.clone());
        
        // Create session-level call lifecycle coordinator
        let call_lifecycle_coordinator = CallLifecycleCoordinator::new(media_manager.clone());
        
        // Create the session event channel
        let (event_sender, event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        let session_manager = Self {
            config,
            sessions: Arc::new(DashMap::new()),
            transaction_manager,
            dialog_manager: Arc::new(dialog_manager),
            media_manager,
            call_lifecycle_coordinator: Arc::new(call_lifecycle_coordinator),
            event_bus: event_bus.clone(),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            default_dialogs: DashMap::new(),
            dialog_to_session: DashMap::new(),
            event_sender,
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            incoming_call_notifier: Arc::new(RwLock::new(None)),
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
    
    /// Handle transaction events by delegating to the DialogManager
    /// 
    /// **ARCHITECTURAL PRINCIPLE**: SessionManager coordinates sessions and delegates 
    /// SIP protocol handling to DialogManager, which handles transaction events properly.
    pub async fn handle_transaction_event(&self, event: TransactionEvent) -> Result<(), Error> {
        debug!("SessionManager delegating transaction event to DialogManager: {:?}", event);
        
        match &event {
            // **NEW**: Handle INVITE requests with notification system
            TransactionEvent::InviteRequest { transaction_id, request, source } => {
                self.handle_invite_request(transaction_id.clone(), request.clone(), *source).await?;
            },
            
            // **NEW**: Handle BYE requests with auto-termination
            TransactionEvent::NonInviteRequest { transaction_id, request, source } if request.method() == Method::Bye => {
                self.handle_bye_request(transaction_id.clone(), request.clone()).await?;
            },
            
            _ => {
                // Delegate all other events to the DialogManager
                self.dialog_manager.process_transaction_event(event).await;
            }
        }
        
        Ok(())
    }
    
    /// **NEW**: Set the incoming call notification callback
    pub async fn set_incoming_call_notifier(&self, notifier: Arc<dyn IncomingCallNotification>) {
        let mut lock = self.incoming_call_notifier.write().await;
        *lock = Some(notifier);
    }
    
    /// **NEW**: Handle incoming INVITE requests with notification to ServerManager
    async fn handle_invite_request(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
    ) -> Result<(), Error> {
        info!("📞 SessionManager processing INVITE request");
        
        // Extract Call-ID for session tracking
        let call_id = request.call_id()
            .ok_or_else(|| Error::InternalError(
                "INVITE missing Call-ID header".to_string(), 
                ErrorContext::default().with_message("Missing required Call-ID header")
            ))?
            .value();
        
        // Create incoming session
        let session = self.create_incoming_session().await?;
        let session_id = session.id.clone();
        
        // Set session to Ringing state
        session.set_state(crate::session::session_types::SessionState::Ringing).await
            .map_err(|e| Error::InternalError(
                format!("Failed to set session to ringing state: {}", e),
                ErrorContext::default().with_message("State transition failed")
            ))?;
        
        // Store the session mapping
        {
            let mut pending = self.pending_calls.write().await;
            pending.insert(call_id.clone(), (session_id.clone(), transaction_id.clone(), request.clone()));
        }
        
        // Store session in active sessions
        self.sessions.insert(session_id.clone(), session);
        
        // Extract caller information
        let caller_info = self.extract_caller_info(&request);
        
        // Extract SDP offer
        let sdp_offer = if !request.body().is_empty() {
            Some(String::from_utf8_lossy(request.body()).to_string())
        } else {
            None
        };
        
        // Create incoming call event
        let event = IncomingCallEvent {
            session_id: session_id.clone(),
            transaction_id: transaction_id.clone(),
            request: request.clone(),
            source,
            caller_info,
            sdp_offer,
        };
        
        // Notify ServerManager for decision
        if let Some(notifier) = self.incoming_call_notifier.read().await.as_ref() {
            let decision = notifier.on_incoming_call(event).await;
            
            match decision {
                CallDecision::Accept => {
                    self.accept_call_impl(&session_id, &transaction_id, &request).await?;
                },
                CallDecision::Reject { status_code, reason } => {
                    self.reject_call_impl(&session_id, &transaction_id, &request, status_code, reason).await?;
                },
                CallDecision::Defer => {
                    // Send 180 Ringing and wait for explicit decision
                    self.send_ringing_response(&transaction_id, &request).await?;
                }
            }
        } else {
            // No notifier, auto-accept (fallback behavior)
            warn!("No incoming call notifier set, auto-accepting call");
            self.accept_call_impl(&session_id, &transaction_id, &request).await?;
        }
        
        Ok(())
    }
    
    /// **NEW**: Handle BYE requests with auto-termination and notification
    async fn handle_bye_request(
        &self,
        transaction_id: TransactionKey,
        request: Request,
    ) -> Result<(), Error> {
        info!("📞 SessionManager processing BYE request");
        
        let call_id = request.call_id()
            .ok_or_else(|| Error::InternalError(
                "BYE missing Call-ID header".to_string(),
                ErrorContext::default().with_message("Missing required Call-ID header")
            ))?
            .value();
        
        // Find session for this call
        let session_id = {
            let pending = self.pending_calls.read().await;
            pending.get(&call_id).map(|(sid, _, _)| sid.clone())
        };
        
        // Terminate the call immediately
        if let Some(session_id) = session_id.clone() {
            self.terminate_call_impl(&session_id).await?;
        }
        
        // Send 200 OK response using transaction-core helper
        let bye_response = rvoip_transaction_core::utils::create_ok_response_for_bye(&request);
        self.transaction_manager.send_response(&transaction_id, bye_response).await
            .map_err(|e| Error::InternalError(
                format!("Failed to send BYE response: {}", e),
                ErrorContext::default().with_message("Failed to send response")
            ))?;
        
        // Notify ServerManager that call was terminated by remote
        if let Some(notifier) = self.incoming_call_notifier.read().await.as_ref() {
            if let Some(session_id) = session_id {
                notifier.on_call_terminated_by_remote(session_id, call_id).await;
            }
        }
        
        // **REMOVED**: No longer delegate to DialogManager since SessionManager handles BYE completely
        // This prevents double handling that could cause protocol violations
        
        Ok(())
    }
    
    /// **NEW**: Send 180 Ringing response
    async fn send_ringing_response(&self, transaction_id: &TransactionKey, request: &Request) -> Result<(), Error> {
        let ringing_response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            request,
            StatusCode::Ringing,
            Some("Ringing")
        ).build();
        
        self.transaction_manager.send_response(transaction_id, ringing_response).await
            .map_err(|e| Error::InternalError(
                format!("Failed to send 180 Ringing: {}", e),
                ErrorContext::default().with_message("Failed to send response")
            ))?;
        
        Ok(())
    }
    
    /// **NEW**: Extract caller information from SIP request
    fn extract_caller_info(&self, request: &Request) -> CallerInfo {
        let from = request.from()
            .map(|h| h.address().uri().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        
        let to = request.to()
            .map(|h| h.address().uri().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        
        let call_id = request.call_id()
            .map(|h| h.value().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        
        let contact = request.contact_uri()
            .map(|uri| uri.to_string());
        
        let user_agent = request.headers.iter()
            .find_map(|h| {
                if let rvoip_sip_core::TypedHeader::UserAgent(ua) = h {
                    Some(ua.join(" "))  // Join the Vec<String> into a single string
                } else {
                    None
                }
            });
        
        CallerInfo {
            from,
            to,
            call_id,
            contact,
            user_agent,
        }
    }
    
    /// **NEW**: Accept call implementation (moved from ServerManager)
    pub async fn accept_call_impl(
        &self,
        session_id: &SessionId,
        transaction_id: &TransactionKey,
        request: &Request,
    ) -> Result<(), Error> {
        info!("🎵 SessionManager implementing call acceptance for session {}", session_id);
        
        let session = self.sessions.get(session_id)
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?
            .clone();
        
        let current_state = session.state().await;
        if current_state != crate::session::session_types::SessionState::Ringing {
            return Err(Error::InternalError(
                format!("Session {} is not in Ringing state (current: {})", session_id, current_state),
                ErrorContext::default().with_message("Invalid session state")
            ));
        }
        
        // Extract SDP offer from INVITE request
        if !request.body().is_empty() {
            let sdp_str = String::from_utf8_lossy(request.body());
            info!("📋 Processing SDP offer: {} bytes", request.body().len());
            
            // Generate SDP answer using media-core integration
            let sdp_answer = self.build_sdp_answer(&sdp_str).await?;
            
            info!("✅ Generated SDP answer: {} bytes", sdp_answer.len());
            
            // Create 200 OK response with SDP
            let mut ok_response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
                request,
                StatusCode::Ok,
                Some("OK")
            ).build();
            
            // Add SDP answer as body
            ok_response = ok_response.with_body(bytes::Bytes::from(sdp_answer));
            
            // Add Content-Type header
            let content_type = rvoip_sip_core::types::content_type::ContentType::from_type_subtype("application", "sdp");
            ok_response.headers.push(rvoip_sip_core::TypedHeader::ContentType(content_type));
            
            // Send 200 OK through transaction-core
            self.transaction_manager.send_response(transaction_id, ok_response).await
                .map_err(|e| Error::InternalError(
                    format!("Failed to send 200 OK response: {}", e),
                    ErrorContext::default().with_message("Failed to send response")
                ))?;
            
            info!("✅ Sent 200 OK with SDP answer for session {}", session_id);
        } else {
            return Err(Error::InternalError(
                "INVITE request missing SDP offer".to_string(),
                ErrorContext::default().with_message("Missing SDP offer")
            ));
        }
        
        // Update session state to Connected
        session.set_state(crate::session::session_types::SessionState::Connected).await
            .map_err(|e| Error::InternalError(
                format!("Failed to transition session to connected: {}", e),
                ErrorContext::default().with_message("State transition failed")
            ))?;
        
        // Remove from pending calls
        {
            let mut pending = self.pending_calls.write().await;
            pending.retain(|_, (sid, _, _)| sid != session_id);
        }
        
        info!("✅ Call acceptance implemented for session {}", session_id);
        Ok(())
    }
    
    /// **NEW**: Reject call implementation (moved from ServerManager)
    pub async fn reject_call_impl(
        &self,
        session_id: &SessionId,
        transaction_id: &TransactionKey,
        request: &Request,
        status_code: StatusCode,
        reason: Option<String>,
    ) -> Result<(), Error> {
        info!("📞 SessionManager implementing call rejection for session {} with status {}", session_id, status_code);
        
        let session = self.sessions.get(session_id)
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?
            .clone();
        
        // Set session to terminated state
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .map_err(|e| Error::InternalError(
                format!("Failed to set session state to terminated: {}", e),
                ErrorContext::default().with_message("State transition failed")
            ))?;
        
        // Create rejection response
        let rejection_response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            request,
            status_code,
            reason.as_deref()
        ).build();
        
        // Send rejection response through transaction-core
        self.transaction_manager.send_response(transaction_id, rejection_response).await
            .map_err(|e| Error::InternalError(
                format!("Failed to send rejection response: {}", e),
                ErrorContext::default().with_message("Failed to send response")
            ))?;
        
        // Remove from sessions and pending calls
        self.sessions.remove(session_id);
        {
            let mut pending = self.pending_calls.write().await;
            pending.retain(|_, (sid, _, _)| sid != session_id);
        }
        
        info!("✅ Call rejection implemented for session {}", session_id);
        Ok(())
    }
    
    /// **NEW**: Terminate call implementation (moved from ServerManager)
    pub async fn terminate_call_impl(&self, session_id: &SessionId) -> Result<(), Error> {
        info!("📞 SessionManager implementing call termination for session {}", session_id);
        
        let session = self.sessions.get(session_id)
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?
            .clone();
        
        let current_state = session.state().await;
        info!("Session {} current state before ending: {}", session_id, current_state);
        
        // Use CallLifecycleCoordinator for proper session termination coordination
        if let Err(e) = self.call_lifecycle_coordinator.coordinate_session_termination(session_id).await {
            warn!("Failed to coordinate session termination via CallLifecycleCoordinator: {}", e);
            // Fall back to direct media cleanup
            if let Err(e) = session.stop_media().await {
                warn!("Failed to stop media for session {}: {}", session_id, e);
            } else {
                info!("✅ Media automatically cleaned up for session {}", session_id);
            }
            session.set_media_session_id(None).await;
        } else {
            info!("✅ Session termination coordinated via CallLifecycleCoordinator");
        }
        
        // Set session to terminated state
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .map_err(|e| Error::InternalError(
                format!("Failed to set session state to terminated: {}", e),
                ErrorContext::default().with_message("State transition failed")
            ))?;
        
        // Remove from sessions and pending calls
        self.sessions.remove(session_id);
        {
            let mut pending = self.pending_calls.write().await;
            pending.retain(|_, (sid, _, _)| sid != session_id);
        }
        
        info!("✅ Call termination implemented for session {} (state: Terminated, coordinated cleanup)", session_id);
        Ok(())
    }
    
    /// **NEW**: Generate SDP answer using CallLifecycleCoordinator (moved from direct implementation)
    async fn build_sdp_answer(&self, offer_sdp: &str) -> Result<String, Error> {
        info!("🎵 Generating SDP answer using session-level CallLifecycleCoordinator...");
        
        // For now, we need a session ID - in a real implementation this would be passed in
        // TODO: Refactor to pass session_id as parameter
        let temp_session_id = SessionId::new();
        
        // Use CallLifecycleCoordinator to create SDP answer with proper media coordination
        let sdp_answer = self.call_lifecycle_coordinator
            .coordinate_session_establishment(&temp_session_id, offer_sdp)
            .await
            .map_err(|e| Error::InternalError(
                format!("Failed to coordinate session establishment: {}", e),
                ErrorContext::default().with_message("CallLifecycleCoordinator failed")
            ))?;
        
        info!("✅ Generated SDP answer using session-level coordination");
        
        Ok(sdp_answer)
    }
    
    /// **NEW**: Public API for ServerManager to accept calls (delegates to implementation)
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<(), Error> {
        // Find the transaction info for this session
        let (transaction_id, request) = {
            let pending = self.pending_calls.read().await;
            pending.iter()
                .find(|(_, (sid, _, _))| sid == session_id)
                .map(|(_, (_, tx_id, req))| (tx_id.clone(), req.clone()))
                .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?
        };
        
        self.accept_call_impl(session_id, &transaction_id, &request).await
    }
    
    /// **NEW**: Public API for ServerManager to reject calls (delegates to implementation)
    pub async fn reject_call(&self, session_id: &SessionId, status_code: StatusCode) -> Result<(), Error> {
        // Find the transaction info for this session
        let (transaction_id, request) = {
            let pending = self.pending_calls.read().await;
            pending.iter()
                .find(|(_, (sid, _, _))| sid == session_id)
                .map(|(_, (_, tx_id, req))| (tx_id.clone(), req.clone()))
                .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?
        };
        
        self.reject_call_impl(session_id, &transaction_id, &request, status_code, None).await
    }
    
    /// **NEW**: Public API for ServerManager to terminate calls (delegates to implementation)
    pub async fn terminate_call(&self, session_id: &SessionId) -> Result<(), Error> {
        self.terminate_call_impl(session_id).await?;
        
        // Notify ServerManager that call was ended by server
        if let Some(notifier) = self.incoming_call_notifier.read().await.as_ref() {
            let call_id = "unknown".to_string(); // TODO: extract from session
            notifier.on_call_ended_by_server(session_id.clone(), call_id).await;
        }
        
        Ok(())
    }
} 