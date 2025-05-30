//! SIP Client - Simple wrapper around client-core::ClientManager
//!
//! This module provides a clean, easy-to-use SIP client API that leverages
//! the robust client-core infrastructure.

use std::time::Duration;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, debug, warn};

use rvoip_client_core::{ClientManager, ClientConfig as CoreConfig, CallId, RegistrationConfig};

use crate::{Config, Call, IncomingCall, SipEvent, Error, Result};

/// Main SIP client for making and receiving calls
pub struct SipClient {
    /// Core client manager (does the heavy lifting)
    core: Arc<ClientManager>,
    /// Configuration
    config: Config,
    /// Event receiver for incoming calls and events
    event_rx: Option<mpsc::Receiver<SipEvent>>,
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

        // Start the core client
        core.start().await
            .map_err(|e| Error::Core(e.to_string()))?;

        let (event_tx, event_rx) = mpsc::channel(32);

        Ok(Self {
            core: Arc::new(core),
            config,
            event_rx: Some(event_rx),
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
        // TODO: Implement by listening to core client events
        // For now, return None (no incoming calls)
        None
    }

    /// Get client status and statistics
    pub async fn status(&self) -> Result<ClientStatus> {
        let stats = self.core.get_client_stats().await;
        
        Ok(ClientStatus {
            is_running: stats.is_running,
            is_registered: false, // TODO: Check registration status
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
        // TODO: Check with core client
        false
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