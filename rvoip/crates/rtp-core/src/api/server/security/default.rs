//! Default implementation for the server security context
//!
//! This file contains the implementation of the ServerSecurityContext trait
//! through the DefaultServerSecurityContext struct.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::any::Any;
use tokio::sync::{Mutex, RwLock};
use async_trait::async_trait;
use tracing::{debug, error, info, warn};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::server::security::{ServerSecurityContext, ClientSecurityContext, ServerSecurityConfig};
use crate::api::server::security::{SocketHandle, ConnectionConfig, ConnectionRole};
use crate::dtls::DtlsConnection;

// Import our core modules
use crate::api::server::security::core::connection;
use crate::api::server::security::core::context;

/// Default implementation of the ServerSecurityContext
#[derive(Clone)]
pub struct DefaultServerSecurityContext {
    /// Configuration
    config: ServerSecurityConfig,
    /// Main DTLS connection template (for certificate/settings)
    connection_template: Arc<Mutex<Option<DtlsConnection>>>,
    /// Client security contexts
    clients: Arc<RwLock<HashMap<SocketAddr, Arc<dyn ClientSecurityContext + Send + Sync>>>>,
    /// Main socket
    socket: Arc<Mutex<Option<SocketHandle>>>,
    /// Client security callbacks
    client_secure_callbacks: Arc<Mutex<Vec<Box<dyn Fn(Arc<dyn ClientSecurityContext + Send + Sync>) + Send + Sync>>>>,
}

impl DefaultServerSecurityContext {
    /// Create a new DefaultServerSecurityContext
    pub async fn new(config: ServerSecurityConfig) -> Result<Arc<dyn ServerSecurityContext + Send + Sync>, SecurityError> {
        // Verify we have SRTP profiles configured
        if config.srtp_profiles.is_empty() {
            return Err(SecurityError::Configuration("No SRTP profiles specified in server config".to_string()));
        }

        // Create the server context
        let ctx = Self {
            config: config.clone(),
            connection_template: Arc::new(Mutex::new(None)),
            clients: Arc::new(RwLock::new(HashMap::new())),
            socket: Arc::new(Mutex::new(None)),
            client_secure_callbacks: Arc::new(Mutex::new(Vec::new())),
        };
        
        // Initialize the connection template
        connection::initialize_connection_template(&config, &ctx.connection_template).await?;
        
        Ok(Arc::new(ctx))
    }
}

#[async_trait]
impl ServerSecurityContext for DefaultServerSecurityContext {
    async fn initialize(&self) -> Result<(), SecurityError> {
        // Delegate to the core context module
        context::initialize_security_context(&self.config, self.socket.lock().await.clone(), &self.connection_template).await
    }
    
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError> {
        let mut socket_lock = self.socket.lock().await;
        *socket_lock = Some(socket);
        Ok(())
    }
    
    async fn get_fingerprint(&self) -> Result<String, SecurityError> {
        // Delegate to the core context module
        context::get_fingerprint_from_template(&self.connection_template).await
    }
    
    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError> {
        // Delegate to the core context module
        context::get_fingerprint_algorithm_from_template(&self.connection_template).await
    }
    
    async fn start_listening(&self) -> Result<(), SecurityError> {
        // Nothing to do here - each client connection will be set up individually
        Ok(())
    }
    
    async fn stop_listening(&self) -> Result<(), SecurityError> {
        // Close all client connections
        let mut clients = self.clients.write().await;
        for (addr, client) in clients.iter() {
            if let Err(e) = client.close().await {
                warn!("Failed to close client security context for {}: {}", addr, e);
            }
        }
        
        // Clear clients
        clients.clear();
        
        Ok(())
    }
    
    async fn create_client_context(&self, addr: SocketAddr) -> Result<Arc<dyn ClientSecurityContext + Send + Sync>, SecurityError> {
        // This method will be fully implemented in Phase 3
        todo!("Implement create_client_context in Phase 3")
    }
    
    async fn get_client_contexts(&self) -> Vec<Arc<dyn ClientSecurityContext + Send + Sync>> {
        // This method will be fully implemented in Phase 3
        todo!("Implement get_client_contexts in Phase 3")
    }
    
    async fn remove_client(&self, addr: SocketAddr) -> Result<(), SecurityError> {
        // This method will be fully implemented in Phase 3
        todo!("Implement remove_client in Phase 3")
    }
    
    async fn on_client_secure(&self, callback: Box<dyn Fn(Arc<dyn ClientSecurityContext + Send + Sync>) + Send + Sync>) -> Result<(), SecurityError> {
        // This method will be fully implemented in Phase 3
        todo!("Implement on_client_secure in Phase 3")
    }
    
    async fn get_supported_srtp_profiles(&self) -> Vec<SrtpProfile> {
        // Return the configured profiles
        self.config.srtp_profiles.clone()
    }
    
    fn is_secure(&self) -> bool {
        // Basic implementation
        self.config.security_mode.is_enabled()
    }
    
    fn get_security_info(&self) -> SecurityInfo {
        // Create a basic security info with what we know synchronously
        SecurityInfo {
            mode: self.config.security_mode,
            fingerprint: None, // Will be filled by async get_fingerprint method
            fingerprint_algorithm: Some(self.config.fingerprint_algorithm.clone()),
            crypto_suites: self.config.srtp_profiles.iter()
                .map(|p| match p {
                    SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
                    SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
                    SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM",
                    SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM",
                })
                .map(|s| s.to_string())
                .collect(),
            key_params: None,
            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()),
        }
    }

    async fn process_client_packet(&self, addr: SocketAddr, data: &[u8]) -> Result<(), SecurityError> {
        // This method will be fully implemented in Phase 4
        todo!("Implement process_client_packet in Phase 4")
    }

    async fn start_packet_handler(&self) -> Result<(), SecurityError> {
        // This method will be fully implemented in Phase 4
        todo!("Implement start_packet_handler in Phase 4")
    }

    async fn capture_initial_packet(&self) -> Result<Option<(Vec<u8>, SocketAddr)>, SecurityError> {
        // This method will be fully implemented in Phase 4
        todo!("Implement capture_initial_packet in Phase 4")
    }

    async fn is_ready(&self) -> Result<bool, SecurityError> {
        // Delegate to the core context module
        context::is_security_context_ready(&self.socket, &self.connection_template).await
    }
} 