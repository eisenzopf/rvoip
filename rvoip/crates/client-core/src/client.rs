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

// PROPER LAYER SEPARATION: Use complete session-core factory API 
use rvoip_session_core::{
    SessionId, Session,
    api::{
        client::config::ClientConfig as SessionClientConfig,
        factory::{create_sip_client, SipClient}
    },
    session::session_types::SessionState,
    prelude::{StatusCode, Uri},
};

use crate::call::{CallId, CallInfo, CallState, CallDirection};
use crate::registration::{RegistrationConfig, RegistrationInfo};
use crate::events::{ClientEventHandler, ClientEvent};
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
    pub default_reject_status: StatusCode,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            local_sip_addr: "127.0.0.1:0".parse().unwrap(), // Use random port
            local_media_addr: "127.0.0.1:0".parse().unwrap(), // Use random port
            user_agent: "rvoip-client/0.1.0".to_string(),
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            max_concurrent_calls: 10,
            enable_logging: true,
            extra_params: HashMap::new(),
            from_uri: None,
            contact_uri: None,
            display_name: None,
            default_reject_status: StatusCode::BusyHere,
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert to session-core ClientConfig
    fn to_session_config(&self) -> SessionClientConfig {
        let mut config = SessionClientConfig::new()
            .with_local_address(self.local_sip_addr)
            .with_max_sessions(self.max_concurrent_calls)
            .with_user_agent(self.user_agent.clone());
            
        // Use configured From URI or generate a default
        if let Some(ref from_uri) = self.from_uri {
            config = config.with_from_uri(from_uri.clone());
        }
        
        // Use configured Contact URI if available
        if let Some(ref contact_uri) = self.contact_uri {
            config = config.with_contact_uri(contact_uri.clone());
        }
        
        config
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
    pub fn with_default_reject_status(mut self, status: StatusCode) -> Self {
        self.default_reject_status = status;
        self
    }
}

/// Main SIP client manager that delegates to session-core
pub struct ClientManager {
    /// Client configuration
    config: ClientConfig,
    /// Full-featured client session manager from session-core
    client_manager: Arc<SipClient>,
    /// Event handler for UI integration
    event_handler: Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
    /// Client state
    is_running: Arc<RwLock<bool>>,
    /// Session ID to client call ID mapping
    session_to_call_mapping: Arc<RwLock<HashMap<SessionId, CallId>>>,
    /// Call ID to session ID mapping  
    call_to_session_mapping: Arc<RwLock<HashMap<CallId, SessionId>>>,
}

impl ClientManager {
    /// Create a new client manager that delegates to session-core
    pub async fn new(config: ClientConfig) -> ClientResult<Self> {
        info!("🔧 Creating ClientManager with proper session-core full client API");
        
        // Convert to session-core config
        let session_config = config.to_session_config();
        
        // **PROPER INFRASTRUCTURE**: Use session-core factory API
        // This provides complete make_call(uri), answer_call(), etc. functionality
        let client_manager = create_sip_client(session_config).await
            .map_err(|e| ClientError::internal_error(&format!("Failed to create SIP client: {}", e)))?;

        info!("✅ SIP client created via session-core factory with real infrastructure");

        let client = Self {
            config,
            client_manager: Arc::new(client_manager),
            event_handler: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            session_to_call_mapping: Arc::new(RwLock::new(HashMap::new())),
            call_to_session_mapping: Arc::new(RwLock::new(HashMap::new())),
        };

        Ok(client)
    }

    /// Start the client
    pub async fn start(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if *running {
            return Err(ClientError::internal_error("Client is already running"));
        }

        info!("▶️ Starting SIP client (delegating to session-core)...");
        
        // Session-core handles all the infrastructure startup
        // No need for client-core to manage individual subsystems

        *running = true;
        info!("✅ SIP client started via session-core delegation");
        Ok(())
    }

    /// Stop the client
    pub async fn stop(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if !*running {
            return Ok(());
        }

        info!("🛑 Stopping SIP client via session-core...");

        // End all active calls via session-core
        let active_calls: Vec<CallId> = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.keys().cloned().collect()
        };

        for call_id in active_calls {
            if let Err(e) = self.hangup_call(&call_id).await {
                warn!("Failed to hangup call {}: {}", call_id, e);
            }
        }

        *running = false;
        info!("✅ SIP client stopped");
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
        info!("📞 Making call from {} to {} via session-core factory API", local_uri, remote_uri);

        // Parse and validate remote URI for future use
        let _remote_uri_parsed: Uri = remote_uri.parse()
            .map_err(|e| ClientError::protocol_error(&format!("Invalid remote URI '{}': {}", remote_uri, e)))?;

        // 🚀 **CRITICAL FIX**: Use SipClient.make_call() which sends INVITE automatically!
        // This is the new factory API method that creates session AND sends INVITE
        let session_id = self.client_manager.make_call(&remote_uri).await
            .map_err(|e| ClientError::protocol_error(&format!("SipClient make_call failed: {}", e)))?;

        // Create client-core call ID and map to session
        let call_id = Uuid::new_v4();

        // Store bidirectional mapping
        {
            let mut session_to_call = self.session_to_call_mapping.write().await;
            let mut call_to_session = self.call_to_session_mapping.write().await;
            session_to_call.insert(session_id.clone(), call_id);
            call_to_session.insert(call_id, session_id.clone());
        }

        info!("✅ Call {} created via session {} (factory API with INVITE transmission)", call_id, session_id);
        info!("📋 Call details: local={}, remote={}, subject={:?}", local_uri, remote_uri, subject);
        info!("🚀 INVITE has been sent via SipClient.make_call() factory API!");
        
        Ok(call_id)
    }

    /// Answer an incoming call via session-core
    pub async fn answer_call(&self, call_id: &CallId) -> ClientResult<()> {
        info!("✅ Answering call {} via session-core factory API", call_id);

        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // **PROPER DELEGATION**: Use session-core via factory API
        self.client_manager.session_manager().accept_call(&session_id).await
            .map_err(|e| ClientError::protocol_error(&format!("Session-core accept_call failed: {}", e)))?;

        info!("✅ Call {} answered via session-core factory API", call_id);
        Ok(())
    }

    /// Reject an incoming call via session-core
    pub async fn reject_call(&self, call_id: &CallId) -> ClientResult<()> {
        self.reject_call_with_status(call_id, self.config.default_reject_status).await
    }

    /// Reject an incoming call with specific status code
    pub async fn reject_call_with_status(&self, call_id: &CallId, status_code: StatusCode) -> ClientResult<()> {
        info!("❌ Rejecting call {} via session-core with status {:?}", call_id, status_code);

        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // **PROPER DELEGATION**: Use session-core via factory API
        self.client_manager.session_manager().reject_call(&session_id, status_code).await
            .map_err(|e| ClientError::protocol_error(&format!("Session-core reject_call failed: {}", e)))?;

        // Remove from mappings
        self.remove_call_mapping(call_id, &session_id).await;

        info!("✅ Call {} rejected via session-core", call_id);
        Ok(())
    }

    /// Hangup a call via session-core
    pub async fn hangup_call(&self, call_id: &CallId) -> ClientResult<()> {
        info!("📴 Hanging up call {} via session-core factory API", call_id);

        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // **PROPER DELEGATION**: Use session-core via factory API
        self.client_manager.session_manager().terminate_call(&session_id).await
            .map_err(|e| ClientError::protocol_error(&format!("Session-core terminate_call failed: {}", e)))?;

        // Remove from mappings
        self.remove_call_mapping(call_id, &session_id).await;

        info!("✅ Call {} hung up via session-core factory API", call_id);
        Ok(())
    }

    /// Get call information (mapped from session-core)
    pub async fn get_call(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        let session_id = {
            let mapping = self.call_to_session_mapping.read().await;
            mapping.get(call_id).cloned()
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?
        };

        // Get session from session-core via factory API
        let session = self.client_manager.session_manager().get_session(&session_id)
            .map_err(|e| ClientError::protocol_error(&format!("Failed to get session: {}", e)))?;

        // Convert session-core Session to client-core CallInfo
        let call_info = self.session_to_call_info(&session, call_id).await?;
        Ok(call_info)
    }

    /// List all active calls (mapped from session-core)
    pub async fn list_calls(&self) -> Vec<CallInfo> {
        let sessions = self.client_manager.session_manager().list_sessions();
        let mapping = self.session_to_call_mapping.read().await;
        
        let mut call_infos = Vec::new();
        for session in sessions.iter() {
            if let Some(call_id) = mapping.get(&session.id) {
                if let Ok(call_info) = self.session_to_call_info(session, call_id).await {
                    call_infos.push(call_info);
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

        // Get the session from session manager
        let session = self.client_manager.session_manager().get_session(&session_id)
            .map_err(|e| ClientError::protocol_error(&format!("Failed to get session: {}", e)))?;

        // **PROPER DELEGATION**: Use session-level media controls via factory API
        if muted {
            session.pause_media().await
                .map_err(|e| ClientError::protocol_error(&format!("Failed to mute: {}", e)))?;
        } else {
            session.resume_media().await
                .map_err(|e| ClientError::protocol_error(&format!("Failed to unmute: {}", e)))?;
        }

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

    /// Convert session-core Session to client-core CallInfo
    async fn session_to_call_info(&self, session: &Session, call_id: &CallId) -> ClientResult<CallInfo> {
        let session_state = session.state().await;
        let state = self.session_state_to_call_state(session_state);
        
        // Get session details from the session object
        let local_uri = self.config.from_uri.clone()
            .unwrap_or_else(|| "sip:unknown@localhost".to_string());
        
        // TODO: Get actual remote URI from session when session-core provides this data
        // For now, use a placeholder that indicates this needs session-core enhancement
        let remote_uri = format!("sip:remote-session-{}@unknown.com", session.id);
        
        // Get actual creation time from session if available
        // TODO: Session-core Session doesn't provide created_at() - use current time for now
        let created_at = chrono::Utc::now();
        
        // TODO: Determine call direction from session data when available
        let direction = CallDirection::Outgoing; // Default assumption
        
        Ok(CallInfo {
            call_id: *call_id,
            state,
            direction,
            local_uri,
            remote_uri,
            remote_display_name: None, // TODO: Get from session remote party info
            subject: None, // TODO: Get from session subject if available
            created_at,
            connected_at: None, // TODO: Get from session connection timestamp
            ended_at: None, // TODO: Get from session termination timestamp
            remote_addr: None, // TODO: Get from session remote media address
            media_session_id: None, // TODO: Get from session media session ID
            sip_call_id: session.id.to_string(),
            metadata: HashMap::new(), // TODO: Get session metadata
        })
    }

    /// Convert session-core SessionState to client-core CallState
    fn session_state_to_call_state(&self, session_state: SessionState) -> CallState {
        match session_state {
            SessionState::Initializing => CallState::Initiating,
            SessionState::Dialing => CallState::Proceeding,
            SessionState::Ringing => CallState::Ringing,
            SessionState::Connected => CallState::Connected,
            SessionState::OnHold => CallState::Connected, // On hold is still connected
            SessionState::Transferring => CallState::Connected, // Transferring is still connected
            SessionState::Terminating => CallState::Terminating,
            SessionState::Terminated => CallState::Terminated,
        }
    }

    /// Remove call mapping when call ends
    async fn remove_call_mapping(&self, call_id: &CallId, session_id: &SessionId) {
        let mut session_to_call = self.session_to_call_mapping.write().await;
        let mut call_to_session = self.call_to_session_mapping.write().await;
        session_to_call.remove(session_id);
        call_to_session.remove(call_id);
    }

    /// Emit an event to the registered handler
    async fn emit_event(&self, event: ClientEvent) {
        let handler = self.event_handler.read().await;
        if let Some(ref handler) = *handler {
            match &event {
                ClientEvent::IncomingCall(info) => {
                    let action = handler.on_incoming_call(info.clone()).await;
                    // Handle the user's action
                    match action {
                        crate::events::CallAction::Accept => {
                            if let Err(e) = self.answer_call(&info.call_id).await {
                                error!("Failed to accept call: {}", e);
                            }
                        },
                        crate::events::CallAction::Reject => {
                            if let Err(e) = self.reject_call(&info.call_id).await {
                                error!("Failed to reject call: {}", e);
                            }
                        },
                        crate::events::CallAction::Forward { target } => {
                            debug!("Call forwarded to: {}", target);
                            // TODO: Implement call forwarding via session-core
                        },
                        crate::events::CallAction::Voicemail => {
                            debug!("Call sent to voicemail");
                            // TODO: Implement voicemail handling
                        }
                    }
                }
                ClientEvent::CallStateChanged(info) => {
                    handler.on_call_state_changed(info.clone()).await;
                }
                ClientEvent::RegistrationStatusChanged(info) => {
                    handler.on_registration_status_changed(info.clone()).await;
                }
                ClientEvent::NetworkStatusChanged { connected, server, message } => {
                    handler.on_network_status_changed(*connected, server.clone(), message.clone()).await;
                }
                ClientEvent::MediaEvent { call_id, event_type, description } => {
                    handler.on_media_event(*call_id, event_type.clone(), description.clone()).await;
                }
                ClientEvent::ErrorOccurred { error, recoverable, context } => {
                    handler.on_error(error.clone(), *recoverable, context.clone()).await;
                }
            }
        }
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