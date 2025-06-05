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
use async_trait::async_trait;

// ARCHITECTURE: session-core ONLY delegates to dialog-core API, NO direct manager access
use rvoip_dialog_core::{UnifiedDialogApi, config::DialogManagerConfig};
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
use crate::session::CallLifecycleCoordinator;

// **NEW**: Import bridge types for multi-session bridging
use super::super::bridge::{
    SessionBridge, BridgeId, BridgeState, BridgeInfo, BridgeConfig,
    BridgeEvent, BridgeEventType, BridgeStats, BridgeError
};

// **NEW**: Import resource management types
use super::super::resource::{
    SessionResourceManager, SessionResourceConfig, SessionResourceMetrics, UserSessionLimits
};

// **NEW**: Import debug and tracing types
use super::super::debug::{
    SessionTracer, SessionCorrelationId, SessionLifecycleEventType, SessionDebugInfo
};

// Constants for configuration
const DEFAULT_EVENT_CHANNEL_SIZE: usize = 100;

// ✅ **INFRASTRUCTURE TYPES**: These are part of SessionManager infrastructure

/// Incoming call notification event
#[derive(Debug, Clone)]
pub struct IncomingCallEvent {
    /// The session ID created for this call
    pub session_id: SessionId,
    
    /// The original INVITE request
    pub request: Request,
    
    /// Source address of the INVITE
    pub source: SocketAddr,
    
    /// Caller information extracted from the request
    pub caller_info: CallerInfo,
    
    /// SDP offer (if present in the INVITE)
    pub sdp_offer: Option<String>,
}

/// Caller information extracted from SIP headers
#[derive(Debug, Clone)]
pub struct CallerInfo {
    /// From header (caller identity)
    pub from: String,
    
    /// To header (called party)
    pub to: String,
    
    /// Call-ID header
    pub call_id: String,
    
    /// Contact header (if present)
    pub contact: Option<String>,
    
    /// User-Agent header (if present)  
    pub user_agent: Option<String>,
}

impl CallerInfo {
    /// Extract caller information from a SIP request
    pub fn from_request(request: &Request, source: SocketAddr) -> Self {
        let from = request.from()
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());
            
        let to = request.to()
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());
            
        let call_id = request.call_id()
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());
            
        let contact = Some(format!("sip:user@{}", source.ip()));
        
        let user_agent = request.header(&rvoip_sip_core::HeaderName::UserAgent)
            .and_then(|h| match h {
                rvoip_sip_core::TypedHeader::UserAgent(ua) => {
                    if ua.is_empty() {
                        None
                    } else {
                        Some(ua.join(" "))
                    }
                },
                _ => None,
            });
        
        Self {
            from,
            to,
            call_id,
            contact,
            user_agent,
        }
    }
}

/// Call decision result from policy
#[derive(Debug, Clone)]
pub enum CallDecision {
    /// Accept the call
    Accept,
    
    /// Accept the call with custom SDP
    AcceptWithSdp(String),
    
    /// Reject the call with a specific status code and reason
    Reject { status_code: StatusCode, reason: Option<String> },
    
    /// Defer the decision (keep ringing, decide later)
    Defer,
}

/// Notification trait for receiving call events
#[async_trait]
pub trait IncomingCallNotification: Send + Sync {
    /// Called when a new incoming call is received
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision;
    
    /// Called when a call is terminated by the remote party (BYE received)
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String);
    
    /// Called when a call is ended by the server
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String);
}

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
/// ALL SIP protocol work to UnifiedDialogApi (dialog-core)
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
    
    /// Unified dialog API (from dialog-core) - handles all SIP modes
    pub(crate) dialog_api: Arc<UnifiedDialogApi>,
    
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
    
    /// **NEW**: Session resource manager for enhanced tracking and monitoring
    pub(crate) resource_manager: Arc<SessionResourceManager>,
    
    /// **NEW**: Session tracer for debugging and lifecycle tracking
    pub(crate) session_tracer: Arc<SessionTracer>,

    /// **NEW**: Simple call handler for developers (holds reference to current handler)
    simple_call_handler: Arc<RwLock<Option<Arc<dyn crate::api::simple::CallHandler>>>>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("config", &self.config)
            .field("active_sessions", &self.sessions.len())
            .field("running", &self.running.load(std::sync::atomic::Ordering::Relaxed))
            .finish()
    }
}

impl SessionManager {
    /// Create a new session manager with integrated media coordination
    /// 
    /// **ARCHITECTURE**: Session-core receives UnifiedDialogApi via dependency injection.
    /// Application level creates: TransactionManager -> UnifiedDialogApi -> SessionManager
    pub async fn new(
        dialog_api: Arc<UnifiedDialogApi>,
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
            dialog_api,
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
            resource_manager: Arc::new(SessionResourceManager::new(SessionResourceConfig::default())),
            session_tracer: Arc::new(SessionTracer::new(10000)),
            simple_call_handler: Arc::new(RwLock::new(None)),
        };
        
        // Set up session coordination channel with dialog-core
        let (coord_tx, coord_rx) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        session_manager.dialog_api.set_session_coordinator(coord_tx).await;
        
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
        
        // **NEW**: Start resource management
        if let Err(e) = session_manager.resource_manager.start().await {
            error!("Failed to start session resource manager: {}", e);
        }
        
        // Start the dialog manager
        session_manager.dialog_api.start().await
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
                
                // **NEW**: Start session tracing with correlation ID
                let correlation_id = self.session_tracer.start_session_trace(
                    session_id,
                    SessionState::Ringing,
                    None,
                ).await;
                
                // **NEW**: Add initial context to trace
                self.session_tracer.add_context(session_id, "direction", "incoming").await;
                self.session_tracer.add_context(session_id, "source", &source.to_string()).await;
                self.session_tracer.add_context(session_id, "dialog_id", &dialog_id.to_string()).await;
                
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
                
                // **NEW**: Record dialog association
                self.session_tracer.record_event(
                    session_id,
                    SessionLifecycleEventType::DialogAssociated {
                        dialog_id: dialog_id.to_string(),
                    },
                    None,
                    HashMap::new(),
                ).await;
                
                // **NEW**: Register session with resource manager
                if let Err(e) = self.resource_manager.register_session(
                    session_id.clone(),
                    None, // TODO: Extract user ID from request
                    Some(source),
                    SessionState::Ringing,
                ).await {
                    warn!("Failed to register session {} with resource manager: {}", session_id, e);
                    
                    // **NEW**: Record error in trace
                    self.session_tracer.record_error(session_id, &e).await;
                }
                
                // Handle incoming call notification
                if let Some(notifier) = self.incoming_call_notifier.read().await.as_ref() {
                    let caller_info = CallerInfo::from_request(&request, source);
                    let incoming_call = IncomingCallEvent {
                        session_id: session_id.clone(),
                        request: request.clone(),
                        source,
                        caller_info,
                        sdp_offer: if let Some(ref offer) = request.body_string() {
                            if !offer.is_empty() {
                                Some(offer.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        },
                    };
                    
                    // Notify about incoming call
                    notifier.on_incoming_call(incoming_call).await;
                } else {
                    warn!("No incoming call notifier configured - auto-rejecting call");
                    // Auto-reject if no handler
                    if let Err(e) = self.reject_call(&session_id, StatusCode::TemporarilyUnavailable).await {
                        error!("Failed to auto-reject call: {}", e);
                        
                        // **NEW**: Record error in trace
                        self.session_tracer.record_error(session_id, &e).await;
                    }
                }
                
                info!("✅ Incoming call session {} created for dialog {} with correlation {}", 
                    session_id, dialog_id, correlation_id);
            },
            
            SessionCoordinationEvent::ReInvite { dialog_id, transaction_id, request } => {
                info!("🔄 Re-INVITE received for dialog {}", dialog_id);
                
                // Find associated session
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id.clone();
                    
                    // **NEW**: Start operation tracking
                    self.session_tracer.start_operation(session_id, "re_invite").await;
                    
                    // Handle re-INVITE in session context
                    if let Ok(session) = self.get_session(&session_id) {
                        // Update session with new SDP if present
                        if let Some(sdp) = request.body_string() {
                            if !sdp.is_empty() {
                                if let Err(e) = session.update_remote_sdp(sdp).await {
                                    self.session_tracer.record_error(session_id, &e).await;
                                    self.session_tracer.complete_operation(session_id, "re_invite", false).await;
                                    return Err(e);
                                }
                            }
                        }
                        
                        self.session_tracer.complete_operation(session_id, "re_invite", true).await;
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
                    
                    // **NEW**: Start termination tracking
                    self.session_tracer.start_operation(session_id, "call_termination").await;
                    
                    if let Ok(session) = self.get_session(&session_id) {
                        if let Err(e) = session.set_state(SessionState::Terminated).await {
                            self.session_tracer.record_error(session_id, &e).await;
                        } else {
                            // **NEW**: Record state change
                            self.session_tracer.record_state_change(
                                session_id,
                                SessionState::Connected, // Assume was connected
                                SessionState::Terminated,
                            ).await;
                        }
                    }
                    
                    // **NEW**: Update resource manager state
                    if let Err(e) = self.resource_manager.update_session_state(&session_id, SessionState::Terminated).await {
                        warn!("Failed to update resource manager state for session {}: {}", session_id, e);
                    }
                    
                    // Clean up mappings
                    self.sessions.remove(&session_id);
                    self.dialog_to_session.remove(&dialog_id);
                    self.default_dialogs.remove(&session_id);
                    
                    // **NEW**: Terminate session tracing
                    self.session_tracer.terminate_session_trace(session_id, &reason).await;
                    self.session_tracer.complete_operation(session_id, "call_termination", true).await;
                    
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
                    
                    // **NEW**: Start operation tracking
                    self.session_tracer.start_operation(session_id, "call_answer").await;
                    
                    if let Ok(session) = self.get_session(&session_id) {
                        // **NEW**: Record state change
                        let old_state = session.state().await;
                        
                        if let Err(e) = session.set_state(SessionState::Connected).await {
                            self.session_tracer.record_error(session_id, &e).await;
                            self.session_tracer.complete_operation(session_id, "call_answer", false).await;
                        } else {
                            self.session_tracer.record_state_change(session_id, old_state, SessionState::Connected).await;
                            
                            if let Err(e) = session.update_local_sdp(session_answer).await {
                                self.session_tracer.record_error(session_id, &e).await;
                            }
                            
                            self.session_tracer.complete_operation(session_id, "call_answer", true).await;
                        }
                    }
                    
                    // **NEW**: Update resource manager state
                    if let Err(e) = self.resource_manager.update_session_state(&session_id, SessionState::Connected).await {
                        warn!("Failed to update resource manager state for session {}: {}", session_id, e);
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
    
    /// Get a reference to the dialog API
    pub fn dialog_api(&self) -> &Arc<UnifiedDialogApi> {
        &self.dialog_api
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
    
    /// **NEW**: Get session resource metrics
    pub async fn get_session_metrics(&self, session_id: &SessionId) -> Option<SessionResourceMetrics> {
        self.resource_manager.get_session_metrics(session_id).await
    }
    
    /// **NEW**: Get metrics for all active sessions
    pub async fn get_all_session_metrics(&self) -> Vec<SessionResourceMetrics> {
        self.resource_manager.get_all_session_metrics().await
    }
    
    /// **NEW**: Get global resource metrics
    pub async fn get_global_metrics(&self) -> super::super::resource::GlobalResourceMetrics {
        self.resource_manager.get_global_metrics().await
    }
    
    /// **NEW**: Get user session limits and current usage
    pub async fn get_user_limits(&self, user_id: &str) -> Option<UserSessionLimits> {
        self.resource_manager.get_user_limits(user_id).await
    }
    
    /// **NEW**: Manually trigger cleanup of terminated sessions
    pub async fn cleanup_terminated_sessions(&self) -> Result<usize, Error> {
        self.resource_manager.cleanup_terminated_sessions().await
    }
    
    /// **NEW**: Perform health checks on all sessions
    pub async fn perform_health_checks(&self) -> Result<usize, Error> {
        self.resource_manager.perform_health_checks().await
    }
    
    /// **NEW**: Update session resource usage (for integration with media-core, etc.)
    pub async fn update_session_resources(
        &self,
        session_id: &SessionId,
        dialog_count: Option<usize>,
        media_session_count: Option<usize>,
        memory_usage: Option<usize>,
    ) -> Result<(), Error> {
        self.resource_manager.update_session_resources(
            session_id,
            dialog_count,
            media_session_count,
            memory_usage,
        ).await
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
    pub async fn terminate_call(&self, session_id: &SessionId, reason: &str) -> Result<(), Error> {
        info!("Terminating call for session {} with reason: {}", session_id, reason);
        
        // **NEW**: Start operation tracking
        self.session_tracer.start_operation(session_id.clone(), "terminate_call").await;
        
        let session = self.get_session(session_id)?;
        
        // **NEW**: Use CallLifecycleCoordinator for session-level cleanup  
        if let Err(e) = self.call_lifecycle_coordinator.cleanup_session_media(session_id).await {
            warn!("Failed to cleanup session media for {}: {}", session_id, e);
            let media_error = Error::media_coordination_error(
                &session_id.to_string(),
                &format!("Failed to cleanup session media: {}", e)
            );
            self.session_tracer.record_error(session_id.clone(), &media_error).await;
        } else {
            info!("✅ Session termination coordinated via CallLifecycleCoordinator");
        }
        
        // Set session to terminated state
        let old_state = session.state().await;
        session.set_state(SessionState::Terminated).await
            .map_err(|e| {
                // **NEW**: Enhanced error context for state transition failures
                let context_error = Error::session_state_error(
                    &session_id.to_string(),
                    &old_state.to_string(),
                    "Terminated",
                    &format!("Failed to set session state during termination: {}", e)
                );
                let _ = self.session_tracer.record_error(session_id.clone(), &context_error);
                context_error
            })?;
        
        // **NEW**: Record state change
        self.session_tracer.record_state_change(session_id.clone(), old_state, SessionState::Terminated).await;
        
        // Clean up dialog in dialog-core
        if let Some(dialog_id) = self.default_dialogs.get(session_id) {
            let dialog_id = dialog_id.clone();
            if let Err(e) = self.dialog_api.send_bye(&dialog_id).await {
                warn!("Failed to send BYE for session {}: {}", session_id, e);
                let bye_error = Error::dialog_error(
                    &dialog_id.to_string(),
                    Some(&session_id.to_string()),
                    &format!("Failed to send BYE: {}", e)
                );
                self.session_tracer.record_error(session_id.clone(), &bye_error).await;
            } else {
                info!("✅ BYE sent for session {} via dialog {}", session_id, dialog_id);
            }
        }
        
        // Clean up mappings
        self.sessions.remove(session_id);
        if let Some(dialog_id) = self.default_dialogs.remove(session_id) {
            self.dialog_to_session.remove(&dialog_id.1);
        }
        
        // **NEW**: Complete operation tracking and terminate trace
        self.session_tracer.complete_operation(session_id.clone(), "terminate_call", true).await;
        self.session_tracer.terminate_session_trace(session_id.clone(), "call_terminated").await;
        
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
        let sdp_offer_body = if let Some(ref offer) = sdp_offer {
            offer.clone()
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
        
        // Create outgoing dialog using unified dialog API
        let dialog = self.dialog_api.create_dialog(
            &local_uri.to_string(),
            &remote_uri.to_string()
        ).await.map_err(|e| Error::InternalError(
            format!("Failed to create outgoing dialog: {}", e),
            ErrorContext::default().with_message("Dialog creation failed")
        ))?;
        
        let dialog_id = dialog.id().clone();
        
        // Associate dialog with session
        self.set_default_dialog(session_id, &dialog_id)?;
        
        info!("✅ Created outgoing dialog {} for session {}", dialog_id, session_id);
        
        // Instead of manually building INVITE, use make_call for real SIP establishment
        let call_result = self.dialog_api.make_call(&local_uri.to_string(), &remote_uri.to_string(), sdp_offer.map(|s| s.clone())).await;
        match call_result {
            Ok(call) => {
                info!("✅ Outgoing call initiated successfully: {}", call.call_id());
                Ok(())
            },
            Err(e) => {
                warn!("Failed to initiate outgoing call: {}", e);
                Err(Error::InternalError(
                    format!("Failed to initiate outgoing call: {}", e),
                    ErrorContext::default().with_message("Call initiation failed")
                ))
            }
        }
    }

    /// Create a new session manager with default event bus
    /// 
    /// **CONVENIENCE METHOD**: Creates the full dependency chain internally.
    /// For production, prefer creating dependencies explicitly at application level.
    pub async fn new_with_default_events(
        dialog_api: Arc<UnifiedDialogApi>,
        config: SessionConfig,
    ) -> Result<Self, Error> {
        // Create default zero-copy event bus
        let event_bus = EventBus::new(1000).await
            .map_err(|e| Error::InternalError(
                format!("Failed to create event bus: {}", e),
                ErrorContext::default().with_message("Event bus initialization failed")
            ))?;
        
        Self::new(dialog_api, config, event_bus).await
    }
    
    /// Create a new session manager (legacy method for backward compatibility)
    /// 
    /// **CONVENIENCE METHOD**: Uses provided dialog API.
    /// For production, prefer creating dependencies explicitly at application level.
    pub fn new_sync(
        dialog_api: Arc<UnifiedDialogApi>,
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
            Self::new(dialog_api, config, event_bus).await
                .expect("Failed to create session manager")
        })
    }

    /// **NEW**: Get session debug information for troubleshooting
    pub async fn get_session_debug_info(&self, session_id: &SessionId) -> Option<SessionDebugInfo> {
        self.session_tracer.get_session_debug_info(session_id.clone()).await
    }
    
    /// **NEW**: Get session by correlation ID for distributed tracing
    pub async fn get_session_by_correlation(&self, correlation_id: &SessionCorrelationId) -> Option<SessionId> {
        self.session_tracer.get_session_by_correlation(correlation_id).await
    }
    
    /// **NEW**: Get session tracing metrics
    pub async fn get_tracing_metrics(&self) -> super::super::debug::TracingMetrics {
        self.session_tracer.get_metrics().await
    }
    
    /// **NEW**: Add context to session trace for debugging
    pub async fn add_session_trace_context(&self, session_id: &SessionId, key: &str, value: &str) {
        self.session_tracer.add_context(session_id.clone(), key, value).await;
    }
    
    /// **NEW**: Start tracking a session operation for performance monitoring
    pub async fn start_session_operation(&self, session_id: &SessionId, operation: &str) {
        self.session_tracer.start_operation(session_id.clone(), operation).await;
    }
    
    /// **NEW**: Complete tracking a session operation
    pub async fn complete_session_operation(&self, session_id: &SessionId, operation: &str, success: bool) {
        self.session_tracer.complete_operation(session_id.clone(), operation, success).await;
    }
    
    /// **NEW**: Record a session error for debugging
    pub async fn record_session_error(&self, session_id: &SessionId, error: &Error) {
        self.session_tracer.record_error(session_id.clone(), error).await;
    }
    
    /// **NEW**: Generate human-readable session timeline for debugging
    pub async fn generate_session_timeline(&self, session_id: &SessionId) -> Option<String> {
        if let Some(debug_info) = self.get_session_debug_info(session_id).await {
            Some(super::super::debug::SessionDebugger::generate_session_timeline(&debug_info))
        } else {
            None
        }
    }
    
    /// **NEW**: Analyze session health and get diagnostic issues
    pub async fn analyze_session_health(&self, session_id: &SessionId) -> Option<Vec<String>> {
        if let Some(debug_info) = self.get_session_debug_info(session_id).await {
            Some(super::super::debug::SessionDebugger::analyze_session_health(&debug_info))
        } else {
            None
        }
    }

    /// Create an outgoing session (placeholder implementation)
    pub async fn create_outgoing_session(&self) -> Result<Arc<Session>, Error> {
        let session_id = SessionId::new();
        let session = Arc::new(Session::new_outgoing(
            session_id.clone(),
            "sip:remote@example.com".to_string(),
            self.config.clone(),
        ).await?);
        
        self.sessions.insert(session_id, session.clone());
        Ok(session)
    }
    
    /// Start the session manager (placeholder implementation)
    pub async fn start(&self) -> Result<(), Error> {
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);
        info!("SessionManager started");
        Ok(())
    }

    // ========================================
    // SIMPLE DEVELOPER INTERFACE METHODS
    // ========================================
    
    /// Set a simple call handler for incoming calls
    /// 
    /// This is the primary method developers use to handle incoming calls.
    /// The handler will be called for all incoming calls and can make simple decisions.
    /// 
    /// # Arguments
    /// * `handler` - Implementation of `CallHandler` trait
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # use std::sync::Arc;
    /// struct MyHandler;
    /// impl CallHandler for MyHandler {
    ///     async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
    ///         CallAction::Answer
    ///     }
    /// }
    /// 
    /// // session_manager.set_call_handler(Arc::new(MyHandler)).await?;
    /// ```
    pub async fn set_call_handler(&self, handler: Arc<dyn crate::api::simple::CallHandler>) -> Result<(), Error> {
        info!("Setting simple call handler");
        
        // Store the handler
        *self.simple_call_handler.write().await = Some(handler);
        
        // Set up internal incoming call notification that translates to simple handler
        let simple_handler_bridge = Arc::new(SimpleHandlerBridge {
            session_manager: Arc::new(self.clone()),
        });
        
        self.set_incoming_call_notifier(simple_handler_bridge).await?;
        
        info!("✅ Simple call handler configured");
        Ok(())
    }
    
    /// Make an outgoing call (simple interface)
    /// 
    /// Creates and manages all the complexity of SIP call setup, returning a simple
    /// `CallSession` for call control.
    /// 
    /// # Arguments
    /// * `from` - Local URI (e.g., "sip:alice@example.com")
    /// * `to` - Remote URI to call (e.g., "sip:bob@example.com") 
    /// * `sdp` - Optional SDP offer (if None, auto-generated)
    /// 
    /// # Returns
    /// `CallSession` for high-level call control
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let call = session_manager.make_call(
    ///     "sip:alice@example.com",
    ///     "sip:bob@example.com",
    ///     None // Auto-generate SDP
    /// ).await?;
    /// 
    /// // Wait for call to connect, then terminate
    /// while call.is_connecting().await {
    ///     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    /// }
    /// 
    /// if call.is_active().await {
    ///     call.terminate().await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn make_call(
        &self, 
        from: &str, 
        to: &str, 
        sdp: Option<String>
    ) -> Result<crate::api::simple::CallSession, Error> {
        info!("Making outgoing call from {} to {}", from, to);
        
        // Create outgoing session
        let session_id = SessionId::new();
        let session = self.create_outgoing_session(session_id.clone(), to.to_string()).await?;
        
        // Create call session wrapper
        let call_session = crate::api::simple::CallSession::new(session_id.clone(), Arc::new(self.clone()));
        
        // Initiate the call via internal API
        let sdp_offer = if let Some(sdp) = sdp {
            sdp
        } else {
            // Auto-generate SDP offer via media coordination
            self.media_manager.generate_default_sdp_offer().await
                .map_err(|e| Error::media_coordination_error(
                    &session_id.to_string(),
                    &format!("Failed to generate SDP offer: {}", e)
                ))?
        };
        
        self.initiate_outgoing_call(&session_id, to, from, Some(sdp_offer)).await?;
        
        info!("✅ Outgoing call initiated for session {}", session_id);
        Ok(call_session)
    }
    
    /// Start SIP server (simple interface)
    /// 
    /// Starts accepting incoming calls on the specified address.
    /// All SIP infrastructure setup is handled internally.
    /// 
    /// # Arguments
    /// * `listen_addr` - Address to listen on (e.g., "127.0.0.1:5060")
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// session_manager.start_server("127.0.0.1:5060".parse()?).await?;
    /// println!("🚀 SIP server running on port 5060");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_server(&self, listen_addr: std::net::SocketAddr) -> Result<(), Error> {
        info!("Starting SIP server on {}", listen_addr);
        
        // Start the session manager infrastructure
        self.start().await?;
        
        // Configure dialog manager to listen on specified address
        self.dialog_api.configure_server_listen_address(listen_addr).await
            .map_err(|e| Error::InternalError(
                format!("Failed to configure server listen address: {}", e),
                ErrorContext::default().with_message("Server startup failed")
            ))?;
        
        info!("✅ SIP server started and listening on {}", listen_addr);
        Ok(())
    }
    
    /// Get list of active calls (simple interface)
    /// 
    /// Returns all currently active call sessions for monitoring or management.
    /// 
    /// # Returns
    /// Vector of active `CallSession` instances
    pub async fn active_calls(&self) -> Vec<crate::api::simple::CallSession> {
        self.sessions
            .iter()
            .filter_map(|entry| {
                let session_id = entry.key().clone();
                let session = entry.value();
                
                // Only include active sessions
                if matches!(
                    futures::executor::block_on(session.state()),
                    SessionState::Dialing | SessionState::Ringing | 
                    SessionState::Connected | SessionState::OnHold | 
                    SessionState::Transferring
                ) {
                    Some(crate::api::simple::CallSession::new(session_id, Arc::new(self.clone())))
                } else {
                    None
                }
            })
            .collect()
    }

    /// **NEW**: Resource management integration
    pub fn get_resource_manager(&self) -> &Arc<SessionResourceManager> {
        &self.resource_manager
    }
    
    /// **NEW**: Debug tracing integration
    pub fn get_session_tracer(&self) -> &Arc<SessionTracer> {
        &self.session_tracer
    }
    
    // ========================================
    // ADDITIONAL SIMPLE INTERFACE METHODS
    // ========================================
    
    /// Answer an incoming call with auto-generated SDP
    /// 
    /// Used by the simple interface to answer calls with automatically generated SDP.
    /// Coordinates with media-core for SDP generation and RTP session setup.
    pub async fn answer_call(&self, session_id: &SessionId) -> Result<(), Error> {
        info!("Answering call for session {}", session_id);
        
        let session = self.get_session(session_id)?;
        
        // Check if session is in ringing state
        let current_state = session.state().await;
        if current_state != SessionState::Ringing {
            return Err(Error::InternalError(
                format!("Cannot answer call in state: {}", current_state),
                ErrorContext::default().with_message("Invalid session state for answer")
            ));
        }
        
        // Generate SDP answer via media coordination
        let sdp_answer = self.media_manager.generate_default_sdp_answer().await
            .map_err(|e| Error::media_coordination_error(
                &session_id.to_string(),
                &format!("Failed to generate SDP answer: {}", e)
            ))?;
        
        // Get dialog ID for this session
        let dialog_id = self.default_dialogs.get(session_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?;
        
        // Send 200 OK with SDP via dialog manager
        self.dialog_api.send_response_with_sdp(&dialog_id, 200, Some("OK".to_string()), sdp_answer).await
            .map_err(|e| Error::dialog_error(
                &dialog_id.to_string(),
                Some(&session_id.to_string()),
                &format!("Failed to send 200 OK: {}", e)
            ))?;
        
        // Update session state
        session.set_state(SessionState::Connected).await?;
        
        info!("✅ Call answered for session {}", session_id);
        Ok(())
    }
    
    /// Answer an incoming call with custom SDP
    /// 
    /// Used by the simple interface when developers provide custom SDP.
    pub async fn answer_call_with_sdp(&self, session_id: &SessionId, sdp: String) -> Result<(), Error> {
        info!("Answering call for session {} with custom SDP", session_id);
        
        let session = self.get_session(session_id)?;
        
        // Check if session is in ringing state
        let current_state = session.state().await;
        if current_state != SessionState::Ringing {
            return Err(Error::InternalError(
                format!("Cannot answer call in state: {}", current_state),
                ErrorContext::default().with_message("Invalid session state for answer")
            ));
        }
        
        // Get dialog ID for this session
        let dialog_id = self.default_dialogs.get(session_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?;
        
        // Send 200 OK with custom SDP via dialog manager
        self.dialog_api.send_response_with_sdp(&dialog_id, 200, Some("OK".to_string()), sdp).await
            .map_err(|e| Error::dialog_error(
                &dialog_id.to_string(),
                Some(&session_id.to_string()),
                &format!("Failed to send 200 OK with custom SDP: {}", e)
            ))?;
        
        // Update session state
        session.set_state(SessionState::Connected).await?;
        
        info!("✅ Call answered for session {} with custom SDP", session_id);
        Ok(())
    }
    
    /// Put a call on hold
    /// 
    /// Sends a re-INVITE with hold SDP (sendonly or inactive).
    pub async fn hold_call(&self, session_id: &SessionId) -> Result<(), Error> {
        info!("Putting call {} on hold", session_id);
        
        let session = self.get_session(session_id)?;
        
        // Check if session is connected
        let current_state = session.state().await;
        if current_state != SessionState::Connected {
            return Err(Error::InternalError(
                format!("Cannot hold call in state: {}", current_state),
                ErrorContext::default().with_message("Invalid session state for hold")
            ));
        }
        
        // Get dialog ID for this session
        let dialog_id = self.default_dialogs.get(session_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?;
        
        // Generate hold SDP via media coordination
        let hold_sdp = self.media_manager.generate_hold_sdp().await
            .map_err(|e| Error::media_coordination_error(
                &session_id.to_string(),
                &format!("Failed to generate hold SDP: {}", e)
            ))?;
        
        // Send re-INVITE with hold SDP
        self.dialog_api.send_request_with_sdp(&dialog_id, "INVITE", hold_sdp).await
            .map_err(|e| Error::dialog_error(
                &dialog_id.to_string(),
                Some(&session_id.to_string()),
                &format!("Failed to send hold re-INVITE: {}", e)
            ))?;
        
        // Update session state
        session.set_state(SessionState::OnHold).await?;
        
        info!("✅ Call {} put on hold", session_id);
        Ok(())
    }
    
    /// Resume a held call
    /// 
    /// Sends a re-INVITE with active SDP (sendrecv).
    pub async fn resume_call(&self, session_id: &SessionId) -> Result<(), Error> {
        info!("Resuming call {}", session_id);
        
        let session = self.get_session(session_id)?;
        
        // Check if session is on hold
        let current_state = session.state().await;
        if current_state != SessionState::OnHold {
            return Err(Error::InternalError(
                format!("Cannot resume call in state: {}", current_state),
                ErrorContext::default().with_message("Invalid session state for resume")
            ));
        }
        
        // Get dialog ID for this session
        let dialog_id = self.default_dialogs.get(session_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::session_not_found(&session_id.to_string()))?;
        
        // Generate active SDP via media coordination
        let active_sdp = self.media_manager.generate_active_sdp().await
            .map_err(|e| Error::media_coordination_error(
                &session_id.to_string(),
                &format!("Failed to generate active SDP: {}", e)
            ))?;
        
        // Send re-INVITE with active SDP
        self.dialog_api.send_request_with_sdp(&dialog_id, "INVITE", active_sdp).await
            .map_err(|e| Error::dialog_error(
                &dialog_id.to_string(),
                Some(&session_id.to_string()),
                &format!("Failed to send resume re-INVITE: {}", e)
            ))?;
        
        // Update session state
        session.set_state(SessionState::Connected).await?;
        
        info!("✅ Call {} resumed", session_id);
        Ok(())
    }
}

/// Bridge that translates simple CallHandler to IncomingCallNotification
/// 
/// This internal bridge allows developers to use the simple CallHandler interface
/// while maintaining compatibility with the existing session coordination system.
struct SimpleHandlerBridge {
    session_manager: Arc<SessionManager>,
}

#[async_trait]
impl IncomingCallNotification for SimpleHandlerBridge {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        // Check if we have a simple call handler
        let handler = {
            let handler_guard = self.session_manager.simple_call_handler.read().await;
            handler_guard.clone()
        };
        
        if let Some(handler) = handler {
            // Extract information for simple interface
            let from_uri = event.caller_info.from.clone();
            let to_uri = event.caller_info.to.clone();
            let sdp_offer = if let Some(ref offer) = event.sdp_offer {
                if !offer.is_empty() {
                    Some(offer.clone())
                } else {
                    None
                }
            } else {
                None
            };
            
            // Extract display name from From header if present
            let display_name = None; // TODO: Parse From header for display name
            let user_agent = None; // TODO: Extract User-Agent header
            
            // Create CallSession for this incoming call
            let call_session = crate::api::simple::CallSession::new(
                event.session_id.clone(), 
                self.session_manager.clone()
            );
            
            // Create IncomingCall struct
            let incoming_call = crate::api::simple::IncomingCall::new(
                call_session.clone(),
                from_uri,
                to_uri,
                sdp_offer,
                display_name,
                user_agent,
            );
            
            // Call the simple handler
            let action = handler.on_incoming_call(&incoming_call).await;
            
            // Translate CallAction to CallDecision
            match action {
                crate::api::simple::CallAction::Answer => {
                    info!("📞 Simple handler decided to answer call {}", event.session_id);
                    CallDecision::Accept
                },
                crate::api::simple::CallAction::AnswerWithSdp(sdp) => {
                    info!("📞 Simple handler decided to answer call {} with custom SDP", event.session_id);
                    CallDecision::AcceptWithSdp(sdp)
                },
                crate::api::simple::CallAction::Reject => {
                    info!("📞 Simple handler decided to reject call {}", event.session_id);
                    CallDecision::Reject {
                        status_code: rvoip_sip_core::StatusCode::BusyHere,
                        reason: Some("Busy Here".to_string()),
                    }
                },
                crate::api::simple::CallAction::RejectWith { status, reason } => {
                    info!("📞 Simple handler decided to reject call {} with {}: {}", 
                          event.session_id, status, reason);
                    CallDecision::Reject {
                        status_code: status,
                        reason: Some(reason),
                    }
                },
                crate::api::simple::CallAction::Defer => {
                    info!("📞 Simple handler deferred decision for call {}", event.session_id);
                    CallDecision::Defer
                },
            }
        } else {
            // No simple handler set - use default behavior (accept)
            debug!("📞 No simple call handler set - accepting call {} by default", event.session_id);
            CallDecision::Accept
        }
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, _call_id: String) {
        // Notify simple handler if present
        if let Some(handler) = &*self.session_manager.simple_call_handler.read().await {
            let call_session = crate::api::simple::CallSession::new(
                session_id.clone(), 
                self.session_manager.clone()
            );
            handler.on_call_ended(&call_session, "Remote party hung up").await;
        }
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, _call_id: String) {
        // Notify simple handler if present
        if let Some(handler) = &*self.session_manager.simple_call_handler.read().await {
            let call_session = crate::api::simple::CallSession::new(
                session_id.clone(), 
                self.session_manager.clone()
            );
            handler.on_call_ended(&call_session, "Server ended call").await;
        }
    }
} 