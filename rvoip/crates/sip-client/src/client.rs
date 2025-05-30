//! SIP Client - Simple wrapper around client-core::ClientManager
//!
//! This module provides a clean, easy-to-use SIP client API that leverages
//! the robust client-core infrastructure.

use std::time::Duration;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, debug, warn};
use async_trait::async_trait;

use rvoip_client_core::{
    ClientManager, ClientConfig as CoreConfig, CallId, RegistrationConfig,
    events::{ClientEventHandler, IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventType, CallAction, Credentials}
};

use crate::{Config, Call, IncomingCall, SipEvent, Error, Result};

/// Internal event handler that bridges client-core events to sip-client
struct SipClientEventHandler {
    /// Channel to send events to the main client
    event_tx: mpsc::UnboundedSender<InternalEvent>,
    /// Current registration status by server URI
    registration_status: Arc<RwLock<HashMap<String, bool>>>,
    /// Pending incoming calls waiting for user decision
    incoming_calls: Arc<RwLock<HashMap<CallId, IncomingCallInfo>>>,
    /// Call state tracking for wait_for_answer functionality
    call_states: Arc<RwLock<HashMap<CallId, rvoip_client_core::call::CallState>>>,
}

/// Internal events between the event handler and main client
#[derive(Debug, Clone)]
enum InternalEvent {
    IncomingCall(IncomingCallInfo),
    CallStateChanged(CallStatusInfo),
    RegistrationChanged(RegistrationStatusInfo),
    MediaEvent { call_id: Option<CallId>, event_type: MediaEventType, description: String },
    Error { message: String, recoverable: bool },
}

#[async_trait]
impl ClientEventHandler for SipClientEventHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 Incoming call from {} to {}", call_info.caller_uri, call_info.callee_uri);
        
        // Store the incoming call for later retrieval by next_incoming_call()
        {
            let mut incoming = self.incoming_calls.write().await;
            incoming.insert(call_info.call_id, call_info.clone());
        }
        
        // Send event to main client
        if let Err(e) = self.event_tx.send(InternalEvent::IncomingCall(call_info)) {
            warn!("Failed to send incoming call event: {}", e);
        }
        
        // Return "defer" - we'll handle the decision through next_incoming_call()
        // This means we don't auto-accept or auto-reject here
        CallAction::Reject // We'll implement proper handling in next_incoming_call()
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        debug!("📞 Call {} state changed to {:?}", status_info.call_id, status_info.new_state);
        
        // Update call state tracking
        {
            let mut states = self.call_states.write().await;
            states.insert(status_info.call_id, status_info.new_state.clone());
        }
        
        // Send event to main client
        if let Err(e) = self.event_tx.send(InternalEvent::CallStateChanged(status_info)) {
            warn!("Failed to send call state change event: {}", e);
        }
    }
    
    async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo) {
        let is_registered = matches!(status_info.status, rvoip_client_core::registration::RegistrationStatus::Registered);
        info!("📝 Registration status for {}: {}", status_info.server_uri, if is_registered { "registered" } else { "not registered" });
        
        // Update registration status tracking
        {
            let mut status = self.registration_status.write().await;
            status.insert(status_info.server_uri.clone(), is_registered);
        }
        
        // Send event to main client
        if let Err(e) = self.event_tx.send(InternalEvent::RegistrationChanged(status_info)) {
            warn!("Failed to send registration status change event: {}", e);
        }
    }
    
    async fn on_network_status_changed(&self, connected: bool, server: String, message: Option<String>) {
        debug!("🌐 Network status changed: {} for {}", if connected { "connected" } else { "disconnected" }, server);
        
        if let Some(msg) = message {
            debug!("   Message: {}", msg);
        }
    }
    
    async fn on_media_event(&self, call_id: Option<CallId>, event_type: MediaEventType, description: String) {
        debug!("🎵 Media event: {:?} - {}", event_type, description);
        
        // Send event to main client
        if let Err(e) = self.event_tx.send(InternalEvent::MediaEvent { call_id, event_type, description }) {
            warn!("Failed to send media event: {}", e);
        }
    }
    
    async fn on_error(&self, error: String, recoverable: bool, context: Option<String>) {
        if recoverable {
            warn!("⚠️ Recoverable error: {} (context: {:?})", error, context);
        } else {
            warn!("❌ Non-recoverable error: {} (context: {:?})", error, context);
        }
        
        // Send event to main client
        if let Err(e) = self.event_tx.send(InternalEvent::Error { message: error, recoverable }) {
            warn!("Failed to send error event: {}", e);
        }
    }
    
    async fn get_credentials(&self, realm: String, server: String) -> Option<Credentials> {
        info!("🔐 Credentials requested for {} @ {}", realm, server);
        // TODO: Could potentially integrate with config or prompt user
        // For now, return None (no additional credentials)
        None
    }
}

/// Main SIP client for making and receiving calls
pub struct SipClient {
    /// Core client manager (does the heavy lifting)
    core: Arc<ClientManager>,
    /// Configuration
    config: Config,
    /// Event receiver for incoming calls and events
    event_rx: Option<mpsc::UnboundedReceiver<InternalEvent>>,
    /// Event handler for interfacing with client-core
    event_handler: Arc<SipClientEventHandler>,
    /// Running state
    is_running: bool,
}

impl SipClient {
    /// Create a new SIP client
    pub async fn new(config: Config) -> Result<Self> {
        info!("🚀 Creating SIP client with config: {:?}", config.username().unwrap_or("anonymous"));
        
        // Convert our config to client-core config
        let core_config = CoreConfig::new()
            .with_sip_addr(config.local_sip_addr())
            .with_media_addr(config.local_media_addr())
            .with_user_agent(config.user_agent.clone())
            .with_codecs(config.preferred_codecs().to_vec())
            .with_max_calls(config.max_concurrent_calls);

        // Create the core client manager
        let core = ClientManager::new(core_config).await
            .map_err(|e| Error::Core(e.to_string()))?;

        // Create event handling infrastructure
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        let event_handler = Arc::new(SipClientEventHandler {
            event_tx,
            registration_status: Arc::new(RwLock::new(HashMap::new())),
            incoming_calls: Arc::new(RwLock::new(HashMap::new())),
            call_states: Arc::new(RwLock::new(HashMap::new())),
        });

        // Set the event handler in client-core
        core.set_event_handler(Arc::clone(&event_handler) as Arc<dyn ClientEventHandler>).await;

        // Start the core client
        core.start().await
            .map_err(|e| Error::Core(e.to_string()))?;

        Ok(Self {
            core: Arc::new(core),
            config,
            event_rx: Some(event_rx),
            event_handler,
            is_running: true,
        })
    }

    /// Register with SIP server using credentials from config
    pub async fn register(&self) -> Result<()> {
        if let Some(ref creds) = self.config.credentials {
            self.register_with(&creds.username, &creds.password, &creds.domain).await
        } else {
            Err(Error::Configuration("No credentials configured".to_string()))
        }
    }

    /// Register with specific credentials
    pub async fn register_with(&self, username: &str, password: &str, domain: &str) -> Result<()> {
        info!("📝 Registering {} with {}", username, domain);
        
        let server_uri = format!("sip:{}", domain);
        let user_uri = format!("sip:{}@{}", username, domain);
        
        let reg_config = RegistrationConfig::new(
            server_uri,
            user_uri,
            username.to_string(),
            password.to_string(),
        );

        self.core.register(reg_config).await
            .map_err(|e| Error::Core(e.to_string()))?;

        info!("✅ Registration initiated for {}", username);
        Ok(())
    }

    /// Make an outgoing call
    pub async fn call(&self, target_uri: &str) -> Result<Call> {
        info!("📞 Making call to {}", target_uri);
        
        let local_uri = self.config.local_uri();
        
        let call_id = self.core.make_call(
            local_uri,
            target_uri.to_string(),
            None, // No subject
        ).await.map_err(|e| Error::Core(e.to_string()))?;

        Ok(Call::new(call_id, target_uri.to_string(), Arc::clone(&self.core)))
    }

    /// Wait for the next incoming call
    pub async fn next_incoming_call(&mut self) -> Option<IncomingCall> {
        // First, check if we have any pending incoming calls
        {
            let mut incoming = self.event_handler.incoming_calls.write().await;
            if let Some((call_id, call_info)) = incoming.iter().next() {
                let call_id = *call_id;
                let call_info = call_info.clone();
                incoming.remove(&call_id);
                
                return Some(IncomingCall::new(
                    call_id,
                    call_info.caller_uri,
                    call_info.caller_display_name,
                    Arc::clone(&self.core),
                ));
            }
        }
        
        // If no pending calls, wait for events
        if let Some(ref mut event_rx) = self.event_rx {
            while let Some(event) = event_rx.recv().await {
                match event {
                    InternalEvent::IncomingCall(call_info) => {
                        return Some(IncomingCall::new(
                            call_info.call_id,
                            call_info.caller_uri,
                            call_info.caller_display_name,
                            Arc::clone(&self.core),
                        ));
                    }
                    _ => {
                        // Handle other events but continue waiting for incoming calls
                        debug!("Received event while waiting for incoming call: {:?}", event);
                    }
                }
            }
        }
        
        None
    }

    /// Get client status and statistics
    pub async fn status(&self) -> Result<ClientStatus> {
        let stats = self.core.get_client_stats().await;
        
        // Check registration status
        let is_registered = if let Some(ref creds) = self.config.credentials {
            let server_uri = format!("sip:{}", creds.domain);
            self.core.is_registered(&server_uri).await
        } else {
            false
        };
        
        Ok(ClientStatus {
            is_running: stats.is_running,
            is_registered,
            total_calls: stats.total_calls,
            active_calls: stats.connected_calls,
            local_address: stats.local_sip_addr,
        })
    }

    /// Get available codecs
    pub async fn available_codecs(&self) -> Vec<String> {
        self.core.get_available_codecs().await
    }

    /// Check if client is registered
    pub async fn is_registered(&self) -> bool {
        if let Some(ref creds) = self.config.credentials {
            let server_uri = format!("sip:{}", creds.domain);
            self.core.is_registered(&server_uri).await
        } else {
            false
        }
    }

    /// Shutdown the client
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("🛑 Shutting down SIP client");
        
        self.core.stop().await
            .map_err(|e| Error::Core(e.to_string()))?;
        
        self.is_running = false;
        info!("✅ SIP client shutdown complete");
        Ok(())
    }

    // === Call-Engine Integration ===

    /// Register as an agent with call-engine (for call center integration)
    pub async fn register_as_agent(&self, _queue_name: &str) -> Result<()> {
        // TODO: Implement call-engine agent registration
        warn!("🚧 Call-engine agent registration not yet implemented");
        Ok(())
    }

    /// Wait for assigned calls from call-engine
    pub async fn next_assigned_call(&mut self) -> Option<IncomingCall> {
        // TODO: Implement call-engine assigned call handling
        None
    }

    /// Get a handle to track call state changes (for wait_for_answer functionality)
    pub async fn get_call_state(&self, call_id: CallId) -> Option<rvoip_client_core::call::CallState> {
        let states = self.event_handler.call_states.read().await;
        states.get(&call_id).cloned()
    }
}

/// Client status information
#[derive(Debug, Clone)]
pub struct ClientStatus {
    pub is_running: bool,
    pub is_registered: bool,
    pub total_calls: usize,
    pub active_calls: usize,
    pub local_address: std::net::SocketAddr,
} 