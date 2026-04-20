//! Standard UAC Client - Balance of simplicity and control

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use tokio::sync::RwLock;
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::api::types::{SessionId, CallState};
use crate::api::builder::SessionManagerConfig;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;
use super::{UacCall, UacConfig, CallOptions, UacEventHandler, NoOpEventHandler};

/// Standard UAC client with event handling and more control
pub struct UacClient {
    coordinator: Arc<SessionCoordinator>,
    config: UacConfig,
    event_handler: Arc<dyn UacEventHandler>,
    active_calls: Arc<RwLock<HashMap<SessionId, UacCall>>>,
}

impl UacClient {
    /// Create a new UAC client with configuration
    pub async fn new(config: UacConfig) -> Result<Self> {
        Self::new_with_handler(config, Arc::new(NoOpEventHandler)).await
    }
    
    /// Create a new UAC client with custom event handler
    pub async fn new_with_handler(
        config: UacConfig,
        event_handler: Arc<dyn UacEventHandler>,
    ) -> Result<Self> {
        // Parse local address to get bind address
        let local_bind_addr: std::net::SocketAddr = config.local_addr.parse()
            .unwrap_or_else(|_| "0.0.0.0:5061".parse().unwrap());
        
        // Create SessionManagerConfig
        let manager_config = SessionManagerConfig {
            sip_port: if local_bind_addr.port() == 0 { 5061 } else { local_bind_addr.port() },
            local_address: config.identity.clone(),
            local_bind_addr,
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
            enable_sip_client: true,
            media_config: Default::default(),
        };
        
        // Create coordinator
        let coordinator = SessionCoordinator::new(manager_config, None).await?;
        
        // Start the coordinator to enable SIP transport
        coordinator.start().await?;
        
        let client = Self {
            coordinator,
            config,
            event_handler,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
        };
        
        // Event monitoring will be implemented when we have proper event subscription
        
        Ok(client)
    }
    
    /// Make an outgoing call with options
    pub async fn call(&self, remote_uri: &str, options: CallOptions) -> Result<UacCall> {
        // Build the actual target URI with the server address
        // Parse the URI to see if it needs the server address
        let target_uri = if remote_uri.starts_with("sip:") {
            let after_sip = &remote_uri[4..];
            if after_sip.contains('@') {
                let parts: Vec<&str> = after_sip.split('@').collect();
                if parts.len() == 2 && !parts[1].contains(':') {
                    // Has user@host but no port, add server's address
                    format!("sip:{}@{}", parts[0], self.config.server_addr)
                } else {
                    // Already complete
                    remote_uri.to_string()
                }
            } else {
                // No @, assume it's just a username
                format!("sip:{}@{}", after_sip, self.config.server_addr)
            }
        } else {
            // Not a SIP URI, build one
            format!("sip:{}@{}", remote_uri, self.config.server_addr)
        };
        
        // Prepare the call
        let mut prepared = SessionControl::prepare_outgoing_call(
            &self.coordinator,
            &self.config.identity,
            &target_uri,
        ).await?;
        
        // Apply custom SDP if provided
        if let Some(custom_sdp) = options.custom_sdp {
            prepared.sdp_offer = custom_sdp;
        }
        
        // TODO: Apply custom headers when dialog-core supports them
        
        // Initiate the call
        let session = SessionControl::initiate_prepared_call(
            &self.coordinator,
            &prepared,
        ).await?;
        
        let call = UacCall::new(
            session.id.clone(),
            self.coordinator.clone(),
            remote_uri.to_string(),
        );
        
        // Track the call
        {
            let mut calls = self.active_calls.write().await;
            calls.insert(session.id.clone(), call.clone());
        }
        
        // Notify handler
        self.event_handler.on_call_state_changed(
            session.id,
            CallState::Initiating,
            CallState::Initiating,
        ).await;
        
        Ok(call)
    }
    
    /// Make a simple call without options
    pub async fn call_simple(&self, remote_uri: &str) -> Result<UacCall> {
        self.call(remote_uri, CallOptions::default()).await
    }
    
    /// Get an active call by session ID
    pub async fn get_call(&self, session_id: &SessionId) -> Option<UacCall> {
        let calls = self.active_calls.read().await;
        calls.get(session_id).cloned()
    }
    
    /// List all active calls
    pub async fn list_calls(&self) -> Vec<UacCall> {
        let calls = self.active_calls.read().await;
        calls.values().cloned().collect()
    }
    
    /// Register with the SIP server
    pub async fn register(&self) -> Result<()> {
        // Will be implemented in Phase 3
        self.event_handler.on_registration_state_changed(true, None).await;
        Ok(())
    }
    
    /// Unregister from the SIP server
    pub async fn unregister(&self) -> Result<()> {
        // Will be implemented in Phase 3
        self.event_handler.on_registration_state_changed(false, Some("User requested".to_string())).await;
        Ok(())
    }
    
    
    /// Get the coordinator for advanced operations
    pub fn coordinator(&self) -> &Arc<SessionCoordinator> {
        &self.coordinator
    }
    
    /// Get the configuration
    pub fn config(&self) -> &UacConfig {
        &self.config
    }
    
    /// Shutdown the client
    pub async fn shutdown(&self) -> Result<()> {
        // Terminate all active calls
        let calls = self.active_calls.read().await;
        for call in calls.values() {
            let _ = call.hangup().await;
        }
        drop(calls);
        
        // Clear active calls
        self.active_calls.write().await.clear();
        
        Ok(())
    }
}