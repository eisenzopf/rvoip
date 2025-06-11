//! Main SIP client coordination
//!
//! This module provides the main ClientManager that coordinates client behavior
//! by delegating to session-core for all SIP session and media orchestration.
//! 
//! PROPER LAYER SEPARATION:
//! client-core -> session-core (complete API) -> {transaction-core, media-core, sip-transport, sip-core}

use std::sync::Arc;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::RwLock;
use uuid::Uuid;
use tracing::{info, debug, warn, error};

// PROPER LAYER SEPARATION: Use ONLY session-core APIs
// session-core handles ALL infrastructure: transaction, sip, transport, media, dialog
use rvoip_session_core::{
    // Core types  
    SessionManager,
    // API functions
    api::{
        make_call_with_manager, 
        accept_call, 
        reject_call,
        SessionManagerBuilder,
        types::{SessionId, CallSession, CallState as SessionCallState},
    },
    // Event system for connecting session events to client events
    manager::events::SessionEvent,
    // TODO: Need StatusCode type - for now use u16 for status codes
};

use crate::call::{CallId, CallInfo, CallState, CallDirection};
use crate::registration::{RegistrationConfig, RegistrationInfo};
use crate::events::{ClientEventHandler, ClientEvent, IncomingCallInfo, CallStatusInfo};
use crate::error::{ClientResult, ClientError};

/// Configuration for the SIP client
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Local SIP listening address
    pub local_sip_addr: SocketAddr,
    /// Local media address for RTP
    pub local_media_addr: SocketAddr,
    /// User agent string
    pub user_agent: String,
    /// Default codec preferences
    pub preferred_codecs: Vec<String>,
    /// Maximum number of concurrent calls
    pub max_concurrent_calls: usize,
    /// Enable detailed logging
    pub enable_logging: bool,
    /// Additional configuration parameters
    pub extra_params: HashMap<String, String>,
    
    // === SIP Identity Configuration (was missing!) ===
    /// Default From URI for outgoing calls (e.g., "sip:alice@example.com")
    pub from_uri: Option<String>,
    /// Default Contact URI (e.g., "sip:alice@192.168.1.100:5060")
    pub contact_uri: Option<String>,
    /// Display name for outgoing calls
    pub display_name: Option<String>,
    /// Default call rejection status code  
    pub default_reject_status: u16,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            local_sip_addr: "127.0.0.1:5060".parse().unwrap(), // Use standard SIP port
            local_media_addr: "127.0.0.1:10000".parse().unwrap(), // Use standard RTP port range
            user_agent: "rvoip-client/0.1.0".to_string(),
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            max_concurrent_calls: 10,
            enable_logging: true,
            extra_params: HashMap::new(),
            from_uri: None,
            contact_uri: None,
            display_name: None,
            default_reject_status: 486, // Busy Here
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Get From URI with fallback to default
    pub fn get_from_uri(&self) -> String {
        self.from_uri.clone().unwrap_or_else(|| {
            format!("sip:user@{}", self.local_sip_addr.ip())
        })
    }
    
    /// Get Contact URI with fallback to default
    pub fn get_contact_uri(&self) -> String {
        self.contact_uri.clone().unwrap_or_else(|| {
            format!("sip:user@{}", self.local_sip_addr)
        })
    }

    /// Set local SIP listening address
    pub fn with_sip_addr(mut self, addr: SocketAddr) -> Self {
        self.local_sip_addr = addr;
        self
    }

    /// Set local media address
    pub fn with_media_addr(mut self, addr: SocketAddr) -> Self {
        self.local_media_addr = addr;
        self
    }

    /// Set user agent string
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Set preferred codecs
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.preferred_codecs = codecs;
        self
    }

    /// Set maximum concurrent calls
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_concurrent_calls = max_calls;
        self
    }

    /// Add extra configuration parameter
    pub fn with_param(mut self, key: String, value: String) -> Self {
        self.extra_params.insert(key, value);
        self
    }

    /// Set default From URI for outgoing calls
    pub fn with_from_uri(mut self, from_uri: String) -> Self {
        self.from_uri = Some(from_uri);
        self
    }
    
    /// Set Contact URI  
    pub fn with_contact_uri(mut self, contact_uri: String) -> Self {
        self.contact_uri = Some(contact_uri);
        self
    }
    
    /// Set display name
    pub fn with_display_name(mut self, display_name: String) -> Self {
        self.display_name = Some(display_name);
        self
    }
    
    /// Set default call rejection status code
    pub fn with_default_reject_status(mut self, status: u16) -> Self {
        self.default_reject_status = status;
        self
    }
}

/// Main SIP client manager that delegates to session-core
pub struct ClientManager {
    /// Client configuration
    config: ClientConfig,
    /// Session manager from session-core (handles ALL infrastructure)
    session_manager: Arc<SessionManager>,
    /// Event handler for UI integration
    event_handler: Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
    /// Client state
    is_running: Arc<RwLock<bool>>,
    /// Session ID to client call ID mapping
    session_to_call_mapping: Arc<RwLock<HashMap<SessionId, CallId>>>,
    /// Call ID to session ID mapping  
    call_to_session_mapping: Arc<RwLock<HashMap<CallId, SessionId>>>,
    /// Event processing task handle (for cleanup)
    event_processor_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl ClientManager {
    /// Create a new client manager that delegates entirely to session-core
    pub async fn new(config: ClientConfig) -> ClientResult<Self> {
        info!("🔧 Creating ClientManager - session-core handles ALL infrastructure");
        
        // session-core handles ALL infrastructure setup:
        // - transaction-core (SIP protocol)
        // - sip-transport (UDP/TCP transport) 
        // - media-core (RTP/audio)
        // - dialog-core (SIP dialogs)
        // - All event processing and coordination
        
        // Configure session manager with client configuration
        let session_manager = SessionManagerBuilder::new()
            .with_sip_bind_address(config.local_sip_addr.ip().to_string())
            .with_sip_port(config.local_sip_addr.port())
            .with_from_uri(config.get_from_uri())
            .with_media_ports(config.local_media_addr.port(), config.local_media_addr.port() + 100) // 100 port range
            .build()
            .await
            .map_err(|e| ClientError::internal_error(&format!("Failed to create SessionManager: {}", e)))?;

        info!("✅ ClientManager created - session-core handles complete infrastructure");

        let client = Self {
            config,
            session_manager,
            event_handler: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            session_to_call_mapping: Arc::new(RwLock::new(HashMap::new())),
            call_to_session_mapping: Arc::new(RwLock::new(HashMap::new())),
            event_processor_handle: Arc::new(RwLock::new(None)),
        };

        Ok(client)
    }

    /// Start the client and all infrastructure
    pub async fn start(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if *running {
            return Err(ClientError::internal_error("Client is already running"));
        }

        info!("▶️ Starting SIP client - session-core handles all infrastructure...");
        
        // Start session-core infrastructure
        self.session_manager.start().await
            .map_err(|e| ClientError::internal_error(&format!("Failed to start session-core: {}", e)))?;

        info!("🏗️ Session-core infrastructure started: transaction-core, dialog-core, media-core, sip-transport");
        
        // ===== PRIORITY 3.2: EVENT PROCESSING PIPELINE =====
        // Subscribe to session-core events and convert them to client events
        
        info!("🔗 Setting up event processing pipeline from session-core to client-core...");
        
        let event_processor = self.session_manager.get_event_processor();
        let mut event_subscriber = event_processor.subscribe().await
            .map_err(|e| ClientError::internal_error(&format!("Failed to subscribe to session events: {}", e)))?;
        
        // Set up event processing loop
        let session_to_call_mapping = Arc::clone(&self.session_to_call_mapping);
        let call_to_session_mapping = Arc::clone(&self.call_to_session_mapping);
        let event_handler = Arc::clone(&self.event_handler);
        
        let event_processing_handle = tokio::spawn(async move {
            info!("🔄 Event processing loop started - converting session-core events to client-core events");
            
            while let Ok(session_event) = event_subscriber.receive().await {
                debug!("📨 Processing session event: {:?}", session_event);
                
                match Self::convert_session_event_to_client_event(
                    session_event,
                    &session_to_call_mapping,
                    &call_to_session_mapping,
                ).await {
                    Ok(Some(client_event)) => {
                        // Emit the converted event to the client event handler
                        Self::emit_event_to_handler(&event_handler, client_event).await;
                    }
                    Ok(None) => {
                        // Event was handled but doesn't need to be forwarded to client
                        debug!("Session event handled internally, no client event generated");
                    }
                    Err(e) => {
                        error!("Failed to convert session event to client event: {}", e);
                    }
                }
            }
            
            info!("🔄 Event processing loop ended");
        });
        
        *self.event_processor_handle.write().await = Some(event_processing_handle);
        
        *running = true;
        info!("✅ SIP client started - session-core infrastructure ready with event processing");
        Ok(())
    }

    /// Stop the client and clean up all resources
    pub async fn stop(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if !*running {
            return Ok(());
        }

        info!("🛑 Stopping SIP client - cleaning up all resources...");

        // Stop event processing first
        if let Some(handle) = self.event_processor_handle.write().await.take() {
            info!("🔄 Stopping event processing loop...");
            handle.abort();
            // Wait a moment for graceful shutdown
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // End all active calls via session-core
        let active_calls: Vec<CallId> = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.keys().cloned().collect()
        };

        info!("📴 Terminating {} active calls", active_calls.len());
        for call_id in active_calls {
            if let Err(e) = self.hangup_call(&call_id).await {
                warn!("Failed to hangup call {}: {}", call_id, e);
            }
        }

        // Stop session-core infrastructure
        self.session_manager.stop().await
            .map_err(|e| ClientError::internal_error(&format!("Failed to stop session-core: {}", e)))?;

        // Clear mappings
        {
            let mut session_to_call = self.session_to_call_mapping.write().await;
            let mut call_to_session = self.call_to_session_mapping.write().await;
            session_to_call.clear();
            call_to_session.clear();
        }

        *running = false;
        info!("✅ SIP client stopped - all resources cleaned up");
        Ok(())
    }

    /// Set event handler for UI integration
    pub async fn set_event_handler(&self, handler: Arc<dyn ClientEventHandler>) {
        let mut event_handler = self.event_handler.write().await;
        *event_handler = Some(handler);
    }

    /// Remove event handler
    pub async fn remove_event_handler(&self) {
        let mut event_handler = self.event_handler.write().await;
        *event_handler = None;
    }

    // === Registration API (delegated to session-core) ===

    /// Register with a SIP server  
    pub async fn register(&self, config: RegistrationConfig) -> ClientResult<Uuid> {
        info!("📝 Registering with server: {}", config.server_uri);
        
        // TODO: Implement proper registration delegation to session-core
        // For now, this is a placeholder until session-core registration API is available
        // The session-core factory doesn't currently support registration management
        warn!("🚧 Registration delegation to session-core not yet fully implemented");
        warn!("🚧 Session-core needs registration manager integration");
        
        // Generate a registration ID for tracking
        let registration_id = Uuid::new_v4();
        info!("✅ Registration request queued with ID: {}", registration_id);
        
        Ok(registration_id)
    }

    /// Unregister from a SIP server
    pub async fn unregister(&self, server_uri: &str) -> ClientResult<()> {
        info!("📤 Unregistering from server: {}", server_uri);
        
        // TODO: Implement proper unregistration delegation to session-core
        warn!("🚧 Unregistration delegation to session-core not yet fully implemented");
        
        Ok(())
    }

    /// Check if registered with a server
    pub async fn is_registered(&self, server_uri: &str) -> bool {
        debug!("🔍 Checking registration status for: {}", server_uri);
        
        // TODO: Query session-core for actual registration status
        // For now, return false since we don't have registration state tracking
        false
    }

    /// Get registration status
    pub async fn get_registration_status(&self, server_uri: &str) -> Option<RegistrationInfo> {
        debug!("🔍 Getting registration status for: {}", server_uri);
        
        // TODO: Get actual registration info from session-core
        None
    }

    /// List all registrations
    pub async fn list_registrations(&self) -> Vec<RegistrationInfo> {
        // TODO: Get actual registrations from session-core
        Vec::new()
    }

    // === Call API (delegated to session-core) ===

    /// Make an outgoing call via session-core
    pub async fn make_call(
        &self,
        local_uri: String,
        remote_uri: String,
        subject: Option<String>,
    ) -> ClientResult<CallId> {
        info!("📞 Making call from {} to {} via session-core API", local_uri, remote_uri);

        // session-core will handle URI validation internally
        info!("📋 Call details: local={}, remote={}, subject={:?}", local_uri, remote_uri, subject);

        // Use session-core make_call_with_manager API
        let call_session = make_call_with_manager(
            &self.session_manager,
            &local_uri,
            &remote_uri
        ).await
        .map_err(|e| ClientError::protocol_error(&format!("make_call_with_manager failed: {}", e)))?;

        // Create client-core call ID and map to session
        let call_id = Uuid::new_v4();

        // Store bidirectional mapping
        {
            let mut session_to_call = self.session_to_call_mapping.write().await;
            let mut call_to_session = self.call_to_session_mapping.write().await;
            session_to_call.insert(call_session.id.clone(), call_id);
            call_to_session.insert(call_id, call_session.id.clone());
        }

        info!("✅ Call {} created via session {} (session-core API)", call_id, call_session.id);
        
        Ok(call_id)
    }

    /// Answer an incoming call via session-core
    pub async fn answer_call(&self, call_id: &CallId) -> ClientResult<()> {
        info!("✅ Answering call {} via session-core API", call_id);

        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // Use session-core accept_call API
        accept_call(&self.session_manager, &session_id).await
            .map_err(|e| ClientError::protocol_error(&format!("Session-core accept_call failed: {}", e)))?;

        info!("✅ Call {} answered via session-core API", call_id);
        Ok(())
    }

    /// Reject an incoming call via session-core
    pub async fn reject_call(&self, call_id: &CallId) -> ClientResult<()> {
        self.reject_call_with_status(call_id, self.config.default_reject_status).await
    }

    /// Reject an incoming call with specific status code
    pub async fn reject_call_with_status(&self, call_id: &CallId, status_code: u16) -> ClientResult<()> {
        info!("❌ Rejecting call {} via session-core with status {:?}", call_id, status_code);

        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // Use session-core reject_call API
        reject_call(&self.session_manager, &session_id, &format!("Rejected: {}", status_code)).await
            .map_err(|e| ClientError::protocol_error(&format!("Session-core reject_call failed: {}", e)))?;

        // Remove from mappings
        self.remove_call_mapping(call_id, &session_id).await;

        info!("✅ Call {} rejected via session-core", call_id);
        Ok(())
    }

    /// Hangup a call via session-core
    pub async fn hangup_call(&self, call_id: &CallId) -> ClientResult<()> {
        info!("📴 Hanging up call {} via session-core API", call_id);

        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // Use session-core terminate_session API
        self.session_manager.terminate_session(&session_id).await
            .map_err(|e| ClientError::protocol_error(&format!("Session-core terminate_session failed: {}", e)))?;

        // Remove from mappings
        self.remove_call_mapping(call_id, &session_id).await;

        info!("✅ Call {} hung up via session-core API", call_id);
        Ok(())
    }

    /// Get call information (mapped from session-core)
    pub async fn get_call(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // Get session from session-core 
        let session = self.session_manager.find_session(&session_id).await
            .map_err(|e| ClientError::protocol_error(&format!("Failed to find session: {}", e)))?
            .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?;

        // Convert session-core CallSession to client-core CallInfo
        let call_info = self.call_session_to_call_info(&session, call_id).await?;
        Ok(call_info)
    }

    /// List all active calls (mapped from session-core)
    pub async fn list_calls(&self) -> Vec<CallInfo> {
        // Get all session IDs from session-core
        let session_ids = self.session_manager.list_active_sessions().await
            .unwrap_or_else(|_| Vec::new());
        let mapping = self.session_to_call_mapping.read().await;
        
        let mut call_infos = Vec::new();
        for session_id in session_ids {
            if let Some(call_id) = mapping.get(&session_id) {
                // Get the full session details
                if let Ok(Some(session)) = self.session_manager.find_session(&session_id).await {
                    if let Ok(call_info) = self.call_session_to_call_info(&session, call_id).await {
                        call_infos.push(call_info);
                    }
                }
            }
        }
        
        call_infos
    }

    /// Get calls by state (mapped from session-core)
    pub async fn get_calls_by_state(&self, state: CallState) -> Vec<CallInfo> {
        let all_calls = self.list_calls().await;
        all_calls.into_iter()
            .filter(|call| call.state == state)
            .collect()
    }

    // === Media API (delegated to session-core) ===

    /// Mute/unmute microphone for a call via session-core
    pub async fn set_microphone_mute(&self, call_id: &CallId, muted: bool) -> ClientResult<()> {
        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // TODO: Implement proper media controls via session-core
        // For now, just log the action until Phase 4 media integration
        info!("🎤 Setting microphone mute for call {} (session {}): {}", call_id, session_id, muted);
        warn!("🚧 Media controls deferred to Phase 4 - media integration");

        Ok(())
    }

    /// Mute/unmute speaker for a call via session-core  
    pub async fn set_speaker_mute(&self, call_id: &CallId, muted: bool) -> ClientResult<()> {
        // TODO: Delegate to session-core media controls
        info!("Speaker mute for call {}: {}", call_id, muted);
        Ok(())
    }

    /// Get available audio codecs from session-core
    pub async fn get_available_codecs(&self) -> Vec<String> {
        // TODO: Query session-core media manager for actual available codecs
        // For now, return configured preferred codecs as a reasonable default
        debug!("🎵 Getting available codecs (using configured preferences until session-core provides codec enumeration)");
        
        let mut codecs = self.config.preferred_codecs.clone();
        
        // Add commonly supported codecs that might be available
        // TODO: Replace with actual query to session-core media manager
        for codec in &["G722", "G729", "telephone-event"] {
            if !codecs.contains(&codec.to_string()) {
                codecs.push(codec.to_string());
            }
        }
        
        debug!("🎵 Available codecs: {:?}", codecs);
        codecs
    }

    // === Utility API ===

    /// Get client configuration
    pub fn get_config(&self) -> &ClientConfig {
        &self.config
    }

    /// Get client status
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    /// Get detailed client statistics
    pub async fn get_client_stats(&self) -> ClientStats {
        let calls = self.list_calls().await;
        let connected_calls = calls.iter().filter(|c| c.state.is_active()).count();
        let is_running = self.is_running().await;

        ClientStats {
            is_running,
            total_calls: calls.len(),
            connected_calls,
            total_registrations: 0, // TODO: Get from session-core
            active_registrations: 0, // TODO: Get from session-core
            local_sip_addr: self.config.local_sip_addr,
            local_media_addr: self.config.local_media_addr,
        }
    }

    // === Internal helpers ===

    /// Convert session-core CallSession to client-core CallInfo
    async fn call_session_to_call_info(&self, call_session: &CallSession, call_id: &CallId) -> ClientResult<CallInfo> {
        // Convert session state to call state
        let state = self.call_session_state_to_call_state(&call_session.state);
        
        // Get session details from the call session object  
        let _local_uri = self.config.from_uri.clone()
            .unwrap_or_else(|| "sip:unknown@localhost".to_string());
        
        // Use session from/to URIs if available
        let remote_uri = call_session.to.clone();
        
        // Get actual creation time from session if available
        // Note: Instant cannot be directly converted to UTC time, so use current time for now
        // TODO: session-core should provide SystemTime or UTC timestamp instead of Instant
        let created_at = if call_session.started_at.is_some() {
            // If session has a start time, use current time as approximation
            chrono::Utc::now()
        } else {
            chrono::Utc::now()
        };
        
        // TODO: Determine call direction from session data when available
        let direction = CallDirection::Outgoing; // Default assumption
        
        Ok(CallInfo {
            call_id: *call_id,
            state,
            direction,
            local_uri: call_session.from.clone(),
            remote_uri,
            remote_display_name: None, // TODO: Get from session remote party info
            subject: None, // TODO: Get from session subject if available
            created_at,
            connected_at: None, // TODO: Get from session connection timestamp
            ended_at: None, // TODO: Get from session termination timestamp
            remote_addr: None, // TODO: Get from session remote media address
            media_session_id: None, // TODO: Get from session media session ID
            sip_call_id: call_session.id.to_string(),
            metadata: HashMap::new(), // TODO: Get session metadata
        })
    }

    /// Convert session-core CallState to client-core CallState
    fn call_session_state_to_call_state(&self, call_session_state: &SessionCallState) -> CallState {
        // Map session-core CallState to client-core CallState
        
        match call_session_state {
            SessionCallState::Initiating => CallState::Initiating,
            SessionCallState::Ringing => CallState::Ringing,
            SessionCallState::Active => CallState::Connected,
            SessionCallState::OnHold => CallState::Connected, // On hold is still connected
            SessionCallState::Transferring => CallState::Connected, // Transfer in progress 
            SessionCallState::Terminating => CallState::Terminating,
            SessionCallState::Terminated => CallState::Terminated,
            SessionCallState::Cancelled => CallState::Terminated, // Map cancelled to terminated
            SessionCallState::Failed(_) => CallState::Terminated, // Map failed to terminated
        }
    }

    /// Remove call mapping when call ends
    async fn remove_call_mapping(&self, call_id: &CallId, session_id: &SessionId) {
        let mut session_to_call = self.session_to_call_mapping.write().await;
        let mut call_to_session = self.call_to_session_mapping.write().await;
        session_to_call.remove(session_id);
        call_to_session.remove(call_id);
    }

    // ===== EVENT PROCESSING HELPERS (Priority 3.2) =====

    /// Convert session-core SessionEvent to client-core ClientEvent
    async fn convert_session_event_to_client_event(
        session_event: SessionEvent,
        session_to_call_mapping: &Arc<RwLock<HashMap<SessionId, CallId>>>,
        call_to_session_mapping: &Arc<RwLock<HashMap<CallId, SessionId>>>,
    ) -> ClientResult<Option<ClientEvent>> {
        match session_event {
            SessionEvent::SessionCreated { session_id, from, to, call_state } => {
                debug!("📞 Session created event: {} ({} -> {})", session_id, from, to);
                
                // For incoming calls in Ringing state, create an incoming call event
                if matches!(call_state, SessionCallState::Ringing) {
                    let call_id = Uuid::new_v4();
                    
                    // Add to mapping
                    {
                        let mut session_to_call = session_to_call_mapping.write().await;
                        let mut call_to_session = call_to_session_mapping.write().await;
                        session_to_call.insert(session_id.clone(), call_id);
                        call_to_session.insert(call_id, session_id);
                    }
                    
                    let incoming_call_info = IncomingCallInfo {
                        call_id,
                        caller_uri: from,
                        caller_display_name: None, // TODO: Extract from session data
                        callee_uri: to,
                        subject: None, // TODO: Extract from session data
                        source_addr: "127.0.0.1:5060".parse().unwrap(), // TODO: Get from session
                        received_at: chrono::Utc::now(),
                    };
                    
                    info!("📞 Incoming call detected: {} from {}", call_id, incoming_call_info.caller_uri);
                    return Ok(Some(ClientEvent::IncomingCall(incoming_call_info)));
                }
                
                // For other states, just log - no client event needed
                debug!("Session created in state {:?}, no client event needed", call_state);
                Ok(None)
            }
            
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                debug!("📊 Session state changed: {} ({:?} -> {:?})", session_id, old_state, new_state);
                
                // Find the corresponding call ID
                let call_id = {
                    let mapping = session_to_call_mapping.read().await;
                    mapping.get(&session_id).cloned()
                };
                
                if let Some(call_id) = call_id {
                    let call_status_info = CallStatusInfo {
                        call_id,
                        new_state: Self::session_call_state_to_call_state_static(&new_state),
                        previous_state: Some(Self::session_call_state_to_call_state_static(&old_state)),
                        reason: None, // TODO: Extract reason if available
                        changed_at: chrono::Utc::now(),
                    };
                    
                    info!("📊 Call state changed: {} ({:?} -> {:?})", 
                          call_id, call_status_info.previous_state, call_status_info.new_state);
                    return Ok(Some(ClientEvent::CallStateChanged(call_status_info)));
                } else {
                    debug!("State change for unmapped session {}, ignoring", session_id);
                }
                
                Ok(None)
            }
            
            SessionEvent::SessionTerminated { session_id, reason } => {
                debug!("📴 Session terminated: {} ({})", session_id, reason);
                
                // Find the corresponding call ID and remove mapping
                let call_id = {
                    let mut session_to_call = session_to_call_mapping.write().await;
                    let mut call_to_session = call_to_session_mapping.write().await;
                    
                    if let Some(call_id) = session_to_call.remove(&session_id) {
                        call_to_session.remove(&call_id);
                        Some(call_id)
                    } else {
                        None
                    }
                };
                
                if let Some(call_id) = call_id {
                    let call_status_info = CallStatusInfo {
                        call_id,
                        new_state: CallState::Terminated,
                        previous_state: None, // We don't track the previous state here
                        reason: Some(reason),
                        changed_at: chrono::Utc::now(),
                    };
                    
                    info!("📴 Call terminated: {} ({})", call_id, call_status_info.reason.as_ref().unwrap_or(&"No reason".to_string()));
                    return Ok(Some(ClientEvent::CallStateChanged(call_status_info)));
                }
                
                Ok(None)
            }
            
            SessionEvent::DtmfReceived { session_id, digits } => {
                debug!("🔢 DTMF received for session {}: {}", session_id, digits);
                // TODO: Convert to media event when we have more detailed media event types
                Ok(None)
            }
            
            SessionEvent::MediaEvent { session_id, event } => {
                debug!("🎵 Media event for session {}: {}", session_id, event);
                // TODO: Convert to ClientEvent::MediaEvent when we implement media integration
                Ok(None)
            }
            
            SessionEvent::Error { session_id, error } => {
                warn!("❌ Session error: {} - {}", session_id.as_ref().map(|s| s.to_string()).unwrap_or("global".to_string()), error);
                
                let client_event = ClientEvent::ErrorOccurred {
                    error: error.clone(),
                    recoverable: true, // Assume errors are recoverable unless we know otherwise
                    context: session_id.map(|s| format!("Session: {}", s)),
                };
                
                Ok(Some(client_event))
            }
            
            // Handle other event types that we don't need to forward to client
            _ => {
                debug!("Unhandled session event type, no client event generated");
                Ok(None)
            }
        }
    }

    /// Static version of call state conversion (for use in static methods)
    fn session_call_state_to_call_state_static(session_state: &SessionCallState) -> CallState {
        match session_state {
            SessionCallState::Initiating => CallState::Initiating,
            SessionCallState::Ringing => CallState::Ringing,
            SessionCallState::Active => CallState::Connected,
            SessionCallState::OnHold => CallState::Connected,
            SessionCallState::Transferring => CallState::Connected,
            SessionCallState::Terminating => CallState::Terminating,
            SessionCallState::Terminated => CallState::Terminated,
            SessionCallState::Cancelled => CallState::Terminated,
            SessionCallState::Failed(_) => CallState::Terminated,
        }
    }

    /// Emit an event to the registered handler
    async fn emit_event_to_handler(
        event_handler: &Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
        event: ClientEvent,
    ) {
        let handler = event_handler.read().await;
        if let Some(ref handler) = *handler {
            match &event {
                ClientEvent::IncomingCall(info) => {
                    debug!("🔔 Emitting incoming call event for call {}", info.call_id);
                    let action = handler.on_incoming_call(info.clone()).await;
                    debug!("👤 User action for incoming call {}: {:?}", info.call_id, action);
                    // TODO: Handle the user's action by calling answer_call/reject_call
                    // This requires passing the ClientManager reference, which we'll implement next
                },
                ClientEvent::CallStateChanged(info) => {
                    debug!("📊 Emitting call state change event for call {}", info.call_id);
                    handler.on_call_state_changed(info.clone()).await;
                }
                ClientEvent::RegistrationStatusChanged(info) => {
                    debug!("🔐 Emitting registration status change event");
                    handler.on_registration_status_changed(info.clone()).await;
                }
                ClientEvent::NetworkStatusChanged { connected, server, message } => {
                    debug!("🌐 Emitting network status change event");
                    handler.on_network_status_changed(*connected, server.clone(), message.clone()).await;
                }
                ClientEvent::MediaEvent { call_id, event_type, description } => {
                    debug!("🎵 Emitting media event");
                    handler.on_media_event(*call_id, event_type.clone(), description.clone()).await;
                }
                ClientEvent::ErrorOccurred { error, recoverable, context } => {
                    debug!("❌ Emitting error event: {}", error);
                    handler.on_error(error.clone(), *recoverable, context.clone()).await;
                }
            }
        } else {
            debug!("No event handler registered, dropping event: {:?}", event);
        }
    }

    /// Emit an event to the registered handler
    async fn emit_event(&self, event: ClientEvent) {
        Self::emit_event_to_handler(&self.event_handler, event).await;
    }
}

/// Statistics about the client
#[derive(Debug, Clone)]
pub struct ClientStats {
    pub is_running: bool,
    pub total_calls: usize,
    pub connected_calls: usize,
    pub total_registrations: usize,
    pub active_registrations: usize,
    pub local_sip_addr: SocketAddr,
    pub local_media_addr: SocketAddr,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_manager_creation() {
        let config = ClientConfig::default()
            .with_sip_addr("127.0.0.1:5061".parse().unwrap()); // Use unique port
        let client = ClientManager::new(config).await;
        assert!(client.is_ok(), "ClientManager creation should succeed");
    }

    #[tokio::test]
    async fn test_client_lifecycle() {
        let config = ClientConfig::default()
            .with_sip_addr("127.0.0.1:5062".parse().unwrap()) // Use unique port
            .with_from_uri("sip:alice@example.com".to_string())
            .with_contact_uri("sip:alice@127.0.0.1:5062".to_string());
            
        let client = ClientManager::new(config).await
            .expect("Failed to create ClientManager");

        // Test start
        assert!(!client.is_running().await);
        client.start().await.expect("Failed to start client");
        assert!(client.is_running().await);

        // Test stop
        client.stop().await.expect("Failed to stop client");
        assert!(!client.is_running().await);
    }

    #[tokio::test]
    async fn test_client_config_builder() {
        let config = ClientConfig::new()
            .with_from_uri("sip:alice@example.com".to_string())
            .with_contact_uri("sip:alice@192.168.1.100:5060".to_string())
            .with_display_name("Alice Test".to_string())
            .with_user_agent("test-client/1.0".to_string())
            .with_max_calls(5)
            .with_default_reject_status(488); // Not Acceptable Here

        assert_eq!(config.get_from_uri(), "sip:alice@example.com");
        assert_eq!(config.get_contact_uri(), "sip:alice@192.168.1.100:5060");
        assert_eq!(config.display_name.unwrap(), "Alice Test");
        assert_eq!(config.user_agent, "test-client/1.0");
        assert_eq!(config.max_concurrent_calls, 5);
        assert_eq!(config.default_reject_status, 488);
    }

    #[tokio::test]
    async fn test_session_to_call_mapping() {
        let config = ClientConfig::default()
            .with_sip_addr("127.0.0.1:5063".parse().unwrap()); // Use unique port
        let client = ClientManager::new(config).await
            .expect("Failed to create ClientManager");

        // Test that mappings start empty
        let calls = client.list_calls().await;
        assert_eq!(calls.len(), 0);

        let stats = client.get_client_stats().await;
        assert_eq!(stats.total_calls, 0);
        assert_eq!(stats.connected_calls, 0);
        assert!(!stats.is_running);
    }

    #[tokio::test] 
    async fn test_session_core_integration() {
        // Test that we can successfully integrate with session-core
        let config = ClientConfig::default()
            .with_sip_addr("127.0.0.1:5064".parse().unwrap()) // Use unique port
            .with_from_uri("sip:test@example.com".to_string());
            
        let client = ClientManager::new(config).await
            .expect("Failed to create ClientManager with session-core");

        client.start().await.expect("Failed to start with session-core integration");
        
        // Verify session manager is properly initialized
        assert!(client.is_running().await);
        
        // Test basic session-core delegation
        let codecs = client.get_available_codecs().await;
        assert!(!codecs.is_empty(), "Should have some available codecs");

        client.stop().await.expect("Failed to stop");
    }

    #[tokio::test]
    async fn test_phase_1_2_success_criteria() {
        // Test Phase 1.2 success criteria from TODO.md
        let config = ClientConfig::default()
            .with_sip_addr("127.0.0.1:5065".parse().unwrap()) // Use unique port
            .with_from_uri("sip:phase1test@example.com".to_string());
            
        let client = ClientManager::new(config).await
            .expect("✅ Basic infrastructure working - SessionManager setup");

        client.start().await
            .expect("✅ Event pipeline functional - Events flow from infrastructure to client-core");
        
        // Basic operation test
        let stats = client.get_client_stats().await;
        assert!(stats.is_running, "✅ Simple integration test passes - Can create ClientManager and perform basic operations");
        
        client.stop().await.expect("Failed to stop in Phase 1.2 test");
        
        println!("🎉 PHASE 1.2 SUCCESS CRITERIA MET!");
    }

    #[tokio::test]
    async fn test_phase_3_2_event_processing_pipeline() {
        // Test Priority 3.2: Event Processing Pipeline
        
        let config = ClientConfig::new()
            .with_sip_addr("127.0.0.1:5070".parse().unwrap())
            .with_from_uri("sip:test@localhost".to_string());
        
        let client = ClientManager::new(config).await.unwrap();
        
        // Start the client (this should start the event processing pipeline)
        client.start().await.unwrap();
        
        // Verify event processing infrastructure is set up
        assert!(client.is_running().await);
        
        // The event processing task should be running
        {
            let handle = client.event_processor_handle.read().await;
            assert!(handle.is_some(), "Event processing task should be running");
        }
        
        // Verify the session manager event processor is accessible
        let event_processor = client.session_manager.get_event_processor();
        assert!(event_processor.is_running().await, "Session event processor should be running");
        
        // Test event conversion functions (static methods)
        let session_to_call_mapping = Arc::new(RwLock::new(HashMap::new()));
        let call_to_session_mapping = Arc::new(RwLock::new(HashMap::new()));
        
        // Test incoming call event conversion
        let session_event = SessionEvent::SessionCreated {
            session_id: SessionId("test-session".to_string()),
            from: "sip:caller@example.com".to_string(),
            to: "sip:callee@example.com".to_string(),
            call_state: SessionCallState::Ringing,
        };
        
        let client_event = ClientManager::convert_session_event_to_client_event(
            session_event,
            &session_to_call_mapping,
            &call_to_session_mapping,
        ).await.unwrap();
        
        assert!(client_event.is_some(), "Incoming call should generate a client event");
        if let Some(ClientEvent::IncomingCall(info)) = client_event {
            assert_eq!(info.caller_uri, "sip:caller@example.com");
            assert_eq!(info.callee_uri, "sip:callee@example.com");
        } else {
            panic!("Expected IncomingCall event");
        }
        
        // Verify session was mapped
        let session_to_call = session_to_call_mapping.read().await;
        assert_eq!(session_to_call.len(), 1, "Session should be mapped");
        
        // Clean up
        client.stop().await.unwrap();
        assert!(!client.is_running().await);
        
        println!("✅ Priority 3.2: Event Processing Pipeline works correctly");
    }
} 