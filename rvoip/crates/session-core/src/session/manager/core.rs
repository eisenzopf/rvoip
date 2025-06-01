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

// ARCHITECTURE: session-core ONLY delegates to dialog-core API, NO direct manager access
use rvoip_dialog_core::api::{DialogServer, DialogClient, DialogConfig, CallHandle, DialogHandle};
use rvoip_dialog_core::{DialogId, SessionCoordinationEvent};
use rvoip_sip_core::{Request, Response, StatusCode, Method};
use rvoip_sip_core::json::ext::SipMessageJson;
use rvoip_sip_core::RequestBuilder;

use crate::dialog::{Dialog, DialogState}; // Keep Dialog and DialogState from local for now
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

// **NEW**: Import bridge types for multi-session bridging
use super::super::bridge::{
    SessionBridge, BridgeId, BridgeState, BridgeInfo, BridgeConfig,
    BridgeEvent, BridgeEventType, BridgeStats, BridgeError
};

// Constants for configuration
const DEFAULT_EVENT_CHANNEL_SIZE: usize = 100;

// Helper trait for Request extensions
trait RequestExt {
    fn body_string(&self) -> Option<String>;
}

impl RequestExt for Request {
    fn body_string(&self) -> Option<String> {
        if self.body().is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(self.body()).to_string())
        }
    }
}

/// Manager for SIP sessions with integrated media coordination
/// 
/// **ARCHITECTURE**: SessionManager coordinates sessions and delegates 
/// ALL SIP protocol work to DialogManager (dialog-core)
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
    
    /// Dialog manager reference (from dialog-core) - ONLY SIP interface
    pub(crate) dialog_manager: Arc<DialogServer>,
    
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
    
    /// **NEW**: Pending incoming calls (Call-ID -> (SessionId, Request))
    pending_calls: Arc<RwLock<HashMap<String, (SessionId, Request)>>>,
    
    /// **NEW**: Pending outgoing calls (SessionId -> for ACK handling)
    pending_outgoing_calls: Arc<RwLock<HashMap<SessionId, String>>>,
    
    /// **FIXED**: Incoming call notification callback with interior mutability
    incoming_call_notifier: Arc<RwLock<Option<Arc<dyn IncomingCallNotification>>>>,
    
    /// **NEW**: Active session bridges for multi-session coordination
    pub(crate) session_bridges: Arc<DashMap<BridgeId, Arc<SessionBridge>>>,
    
    /// **NEW**: Session to bridge mapping for quick lookup
    pub(crate) session_to_bridge: Arc<DashMap<SessionId, BridgeId>>,
    
    /// **NEW**: Bridge event sender for call-engine notifications
    pub(crate) bridge_event_sender: Arc<RwLock<Option<mpsc::UnboundedSender<BridgeEvent>>>>,
}

impl SessionManager {
    /// Create a new session manager with integrated media coordination
    /// 
    /// **ARCHITECTURE**: Session-core receives DialogManager via dependency injection.
    /// Application level creates: TransactionManager -> DialogManager -> SessionManager
    pub async fn new(
        dialog_manager: Arc<DialogServer>,
        config: SessionConfig,
        event_bus: EventBus
    ) -> Result<Self, Error> {
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
            dialog_manager,
            media_manager,
            call_lifecycle_coordinator: Arc::new(call_lifecycle_coordinator),
            event_bus: event_bus.clone(),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            default_dialogs: DashMap::new(),
            dialog_to_session: DashMap::new(),
            event_sender,
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            pending_outgoing_calls: Arc::new(RwLock::new(HashMap::new())),
            incoming_call_notifier: Arc::new(RwLock::new(None)),
            session_bridges: Arc::new(DashMap::new()),
            session_to_bridge: Arc::new(DashMap::new()),
            bridge_event_sender: Arc::new(RwLock::new(None)),
        };
        
        // Set up session coordination channel with dialog-core
        let (coord_tx, coord_rx) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        session_manager.dialog_manager.set_session_coordinator(coord_tx).await;
        
        // Start session coordination event processing
        let manager_clone = session_manager.clone();
        tokio::spawn(async move {
            manager_clone.process_session_coordination_events(coord_rx).await;
        });
        
        // Start the session event processing
        let manager_clone = session_manager.clone();
        tokio::spawn(async move {
            manager_clone.process_session_events(event_receiver).await;
        });
        
        // Start the dialog manager
        session_manager.dialog_manager.start().await
            .map_err(|e| Error::InternalError(
                format!("Failed to start dialog manager: {}", e),
                ErrorContext::default().with_message("Dialog manager startup failed")
            ))?;
        
        Ok(session_manager)
    }
    
    /// Process session coordination events from dialog-core
    async fn process_session_coordination_events(&self, mut rx: mpsc::Receiver<SessionCoordinationEvent>) {
        while let Some(event) = rx.recv().await {
            if let Err(e) = self.handle_session_coordination_event(event).await {
                error!("Failed to handle session coordination event: {}", e);
            }
        }
    }
    
    /// Handle session coordination events from dialog-core
    async fn handle_session_coordination_event(&self, event: SessionCoordinationEvent) -> Result<(), Error> {
        match event {
            SessionCoordinationEvent::IncomingCall { dialog_id, transaction_id, request, source } => {
                info!("📞 Incoming call from {} via dialog {}", source, dialog_id);
                
                // Create new session for incoming call
                let session_id = SessionId::new();
                let session = Arc::new(Session::new_incoming(
                    session_id.clone(),
                    request.clone(),
                    source,
                    self.config.clone(),
                ).await?);
                
                // Store session
                self.sessions.insert(session_id.clone(), session.clone());
                
                // Associate dialog with session
                self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
                self.default_dialogs.insert(session_id.clone(), dialog_id.clone());
                
                // Handle incoming call notification
                if let Some(notifier) = self.incoming_call_notifier.read().await.as_ref() {
                    let caller_info = CallerInfo::from_request(&request, source);
                    let incoming_call = IncomingCallEvent {
                        session_id: session_id.clone(),
                        request: request.clone(),
                        source,
                        caller_info,
                        sdp_offer: request.body_string(),
                    };
                    
                    // Notify about incoming call
                    notifier.on_incoming_call(incoming_call).await;
                } else {
                    warn!("No incoming call notifier configured - auto-rejecting call");
                    // Auto-reject if no handler
                    if let Err(e) = self.reject_call(&session_id, StatusCode::TemporarilyUnavailable).await {
                        error!("Failed to auto-reject call: {}", e);
                    }
                }
                
                info!("✅ Incoming call session {} created for dialog {}", session_id, dialog_id);
            },
            
            SessionCoordinationEvent::ReInvite { dialog_id, transaction_id, request } => {
                info!("🔄 Re-INVITE received for dialog {}", dialog_id);
                
                // Find associated session
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id.clone();
                    
                    // Handle re-INVITE in session context
                    if let Ok(session) = self.get_session(&session_id) {
                        // Update session with new SDP if present
                        if let Some(sdp) = request.body_string() {
                            if !sdp.is_empty() {
                                session.update_remote_sdp(sdp).await?;
                            }
                        }
                        
                        info!("✅ Re-INVITE processed for session {}", session_id);
                    }
                } else {
                    warn!("Re-INVITE received for unknown dialog {}", dialog_id);
                }
            },
            
            SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
                info!("📞 Call terminated for dialog {}: {}", dialog_id, reason);
                
                // Find and terminate associated session
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id.clone();
                    
                    if let Ok(session) = self.get_session(&session_id) {
                        session.set_state(SessionState::Terminated).await?;
                    }
                    
                    // Clean up mappings
                    self.sessions.remove(&session_id);
                    self.dialog_to_session.remove(&dialog_id);
                    self.default_dialogs.remove(&session_id);
                    
                    // Publish session event
                    let session_event = SessionEvent::Terminated {
                        session_id: session_id.clone(),
                        reason: reason.clone(),
                    };
                    
                    if let Err(e) = self.event_sender.send(session_event).await {
                        warn!("Failed to send session terminated event: {}", e);
                    }
                    
                    info!("✅ Session {} terminated due to: {}", session_id, reason);
                } else {
                    warn!("Call termination received for unknown dialog {}", dialog_id);
                }
            },
            
            SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
                info!("✅ Call answered for dialog {}", dialog_id);
                
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id.clone();
                    
                    if let Ok(session) = self.get_session(&session_id) {
                        session.set_state(SessionState::Connected).await?;
                        session.update_local_sdp(session_answer).await?;
                    }
                    
                    info!("✅ Session {} connected", session_id);
                }
            },
            
            SessionCoordinationEvent::RegistrationRequest { transaction_id, from_uri, contact_uri, expires } => {
                info!("📝 Registration request: {} expires in {}s", from_uri, expires);
                // Handle registration - usually goes to registrar service
                // For now, just log it
            },
            
            _ => {
                debug!("Unhandled session coordination event: {:?}", event);
            }
        }
        
        Ok(())
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
    pub fn dialog_manager(&self) -> &Arc<DialogServer> {
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

    /// **NEW**: Set the incoming call notification callback
    pub async fn set_incoming_call_notifier(&self, notifier: Arc<dyn IncomingCallNotification>) {
        let mut lock = self.incoming_call_notifier.write().await;
        *lock = Some(notifier);
    }
    
    /// **NEW**: Generate SDP answer using CallLifecycleCoordinator (moved from direct implementation)
    async fn build_sdp_answer(&self, session_id: &SessionId, offer_sdp: &str) -> Result<String, Error> {
        info!("🎵 Generating SDP answer using session-level CallLifecycleCoordinator for session {}...", session_id);
        
        // **FIXED**: Use the actual session_id instead of creating a temporary one
        // This ensures media sessions are properly mapped for cleanup during BYE
        let sdp_answer = self.call_lifecycle_coordinator
            .coordinate_session_establishment(session_id, offer_sdp)
            .await
            .map_err(|e| Error::InternalError(
                format!("Failed to coordinate session establishment: {}", e),
                ErrorContext::default().with_message("CallLifecycleCoordinator failed")
            ))?;
        
        info!("✅ Generated SDP answer using session-level coordination for session {}", session_id);
        
        Ok(sdp_answer)
    }
    
    /// **NEW**: Public API for call-engine to accept calls
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<(), Error> {
        info!("Accepting call for session {}", session_id);
        
        // Get the dialog ID for this session
        let dialog_id = self.default_dialogs.get(session_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?;
        
        // Get session to generate SDP answer
        let session = self.get_session(session_id)?;
        
        // Generate SDP answer (this should have SDP offer from the incoming call)
        let sdp_answer = self.build_sdp_answer(session_id, "").await?;
        
        // Use dialog-core to send 200 OK response
        // This will be handled through the session coordination events
        info!("✅ Call accepted for session {} - 200 OK will be sent by dialog-core", session_id);
        
        Ok(())
    }
    
    /// **NEW**: Public API for call-engine to reject calls
    pub async fn reject_call(&self, session_id: &SessionId, status_code: StatusCode) -> Result<(), Error> {
        info!("Rejecting call for session {} with status {}", session_id, status_code);
        
        // Get the dialog ID for this session
        let dialog_id = self.default_dialogs.get(session_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?;
        
        // Update session state and cleanup
        let session = self.get_session(session_id)?;
        session.set_state(SessionState::Terminated).await
            .map_err(|e| Error::InternalError(
                format!("Failed to set session state to terminated: {}", e),
                ErrorContext::default().with_message("State transition failed")
            ))?;
        
        // Remove from sessions
        self.sessions.remove(session_id);
        self.default_dialogs.remove(session_id);
        self.dialog_to_session.remove(&dialog_id);
        
        info!("✅ Call rejected for session {} - response will be sent by dialog-core", session_id);
        Ok(())
    }
    
    /// **NEW**: Public API for call-engine to terminate calls
    pub async fn terminate_call(&self, session_id: &SessionId) -> Result<(), Error> {
        info!("Terminating call for session {}", session_id);
        
        // Session coordination - media cleanup and state management
        let session = self.get_session(session_id)?;
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
        session.set_state(SessionState::Terminated).await
            .map_err(|e| Error::InternalError(
                format!("Failed to set session state to terminated: {}", e),
                ErrorContext::default().with_message("State transition failed")
            ))?;
        
        // Get dialog ID and send BYE through dialog-core
        if let Some(dialog_id) = self.default_dialogs.get(session_id) {
            let dialog_id = dialog_id.clone();
            if let Err(e) = self.dialog_manager.send_request(&dialog_id, rvoip_sip_core::Method::Bye, None).await {
                warn!("Failed to send BYE for session {}: {}", session_id, e);
            } else {
                info!("✅ BYE sent for session {} via dialog-core", session_id);
            }
        }
        
        // Clean up mappings
        self.sessions.remove(session_id);
        if let Some(dialog_id) = self.default_dialogs.remove(session_id) {
            self.dialog_to_session.remove(&dialog_id.1);
        }
        
        info!("✅ Call termination coordinated for session {} (state: Terminated, coordinated cleanup)", session_id);
        Ok(())
    }

    /// **NEW**: Initiate outgoing call by sending INVITE
    pub async fn initiate_outgoing_call(
        &self,
        session_id: &SessionId,
        target_uri: &str,
        from_uri: &str,
        sdp_offer: Option<String>,
    ) -> Result<(), Error> {
        info!("📞 Initiating outgoing call for session {} to {}", session_id, target_uri);
        
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Verify session is in correct state for outgoing call
        let current_state = session.state().await;
        if current_state != SessionState::Initializing {
            return Err(Error::InternalError(
                format!("Session {} not in Initializing state for outgoing call (current: {})", session_id, current_state),
                ErrorContext::default().with_message("Invalid session state for outgoing call")
            ));
        }
        
        // Parse target URI for validation
        let _target_uri_parsed: rvoip_sip_core::Uri = target_uri.parse()
            .map_err(|e| Error::InternalError(
                format!("Invalid target URI '{}': {}", target_uri, e),
                ErrorContext::default().with_message("URI parsing failed")
            ))?;
        
        // Generate SDP offer if not provided using media coordination
        let sdp_offer_body = if let Some(offer) = sdp_offer {
            offer
        } else {
            info!("🎵 Generating SDP offer for outgoing call...");
            self.call_lifecycle_coordinator
                .coordinate_session_establishment(session_id, "")
                .await
                .map_err(|e| Error::InternalError(
                    format!("Failed to generate SDP offer: {}", e),
                    ErrorContext::default().with_message("SDP generation failed")
                ))?
        };
        
        // **ARCHITECTURE FIX**: Use dialog-core to create outgoing dialog and send INVITE
        // This replaces direct SIP message building with proper delegation
        let local_uri: rvoip_sip_core::Uri = from_uri.parse()
            .map_err(|e| Error::InternalError(
                format!("Invalid from URI '{}': {}", from_uri, e),
                ErrorContext::default().with_message("From URI parsing failed")
            ))?;
        
        let remote_uri: rvoip_sip_core::Uri = target_uri.parse()
            .map_err(|e| Error::InternalError(
                format!("Invalid target URI '{}': {}", target_uri, e),
                ErrorContext::default().with_message("Target URI parsing failed")
            ))?;
        
        // Create outgoing dialog using dialog-core
        let dialog_id = self.dialog_manager.create_outgoing_dialog(
            local_uri,
            remote_uri,
            None // Let dialog-core generate call-id
        ).await.map_err(|e| Error::InternalError(
            format!("Failed to create outgoing dialog: {}", e),
            ErrorContext::default().with_message("Dialog creation failed")
        ))?;
        
        // Associate dialog with session
        self.set_default_dialog(session_id, &dialog_id)?;
        
        // **ARCHITECTURE COMPLIANCE**: Use dialog-core to send INVITE with SDP body
        // This replaces manual RequestBuilder usage with proper delegation
        let sdp_body = bytes::Bytes::from(sdp_offer_body);
        let _transaction_id = self.dialog_manager.send_request(
            &dialog_id, 
            Method::Invite, 
            Some(sdp_body)
        ).await.map_err(|e| Error::InternalError(
            format!("Failed to send INVITE via dialog-core: {}", e),
            ErrorContext::default().with_message("INVITE transmission failed")
        ))?;
        
        // Update session state to Dialing
        session.set_state(SessionState::Dialing).await
            .map_err(|e| Error::InternalError(
                format!("Failed to update session state to Dialing: {}", e),
                ErrorContext::default().with_message("State transition failed")
            ))?;
        
        info!("✅ INVITE sent successfully via dialog-core for session {}", session_id);
        info!("📞 Call state: Dialing → waiting for response from {}", target_uri);
        
        Ok(())
    }

    /// Create a new session manager with default event bus
    /// 
    /// **CONVENIENCE METHOD**: Creates the full dependency chain internally.
    /// For production, prefer creating dependencies explicitly at application level.
    pub async fn new_with_default_events(
        dialog_manager: Arc<DialogServer>,
        config: SessionConfig,
    ) -> Result<Self, Error> {
        // Create default zero-copy event bus
        let event_bus = EventBus::new(1000).await
            .map_err(|e| Error::InternalError(
                format!("Failed to create event bus: {}", e),
                ErrorContext::default().with_message("Event bus initialization failed")
            ))?;
        
        Self::new(dialog_manager, config, event_bus).await
    }
    
    /// Create a new session manager (legacy method for backward compatibility)
    /// 
    /// **CONVENIENCE METHOD**: Uses provided dialog manager.
    /// For production, prefer creating dependencies explicitly at application level.
    pub fn new_sync(
        dialog_manager: Arc<DialogServer>,
        config: SessionConfig,
        event_bus: EventBus
    ) -> Self {
        // Use the current runtime or create a temporary one
        let rt = tokio::runtime::Handle::try_current()
            .or_else(|_| {
                tokio::runtime::Runtime::new().map(|rt| rt.handle().clone())
            })
            .expect("No tokio runtime available");
        
        rt.block_on(async {
            Self::new(dialog_manager, config, event_bus).await
                .expect("Failed to create session manager")
        })
    }
} 