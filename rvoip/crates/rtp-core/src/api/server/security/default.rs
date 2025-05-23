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
use crate::api::server::security::client::context::DefaultClientSecurityContext;
use crate::api::server::security::dtls::{handshake, transport};
use crate::api::server::security::srtp::keys;
use crate::api::server::security::util::conversion;

/// Default implementation of the ServerSecurityContext
#[derive(Clone)]
pub struct DefaultServerSecurityContext {
    /// Configuration
    config: ServerSecurityConfig,
    /// Main DTLS connection template (for certificate/settings)
    connection_template: Arc<Mutex<Option<DtlsConnection>>>,
    /// Client security contexts
    clients: Arc<RwLock<HashMap<SocketAddr, Arc<DefaultClientSecurityContext>>>>,
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
        // First check if this client already exists, and if so, completely recreate it
        {
            let mut clients = self.clients.write().await;
            if clients.contains_key(&addr) {
                debug!("Client {} already exists, removing old connection for clean restart", addr);
                clients.remove(&addr);
            }
        }
        
        // Create a new client context
        debug!("Creating new security context for client {}", addr);
        
        // Create DTLS connection with server role
        let dtls_config = crate::dtls::DtlsConfig {
            role: crate::dtls::DtlsRole::Server,
            version: crate::dtls::DtlsVersion::Dtls12,
            mtu: 1500,
            max_retransmissions: 5,
            srtp_profiles: keys::convert_profiles(&self.config.srtp_profiles),
        };
        let mut connection = crate::dtls::DtlsConnection::new(dtls_config);
        
        // Set certificate
        let cert = match self.connection_template.lock().await.as_ref() {
            Some(conn) => match conn.local_certificate() {
                Some(cert) => cert.clone(),
                None => return Err(SecurityError::Configuration("No certificate in template".to_string())),
            },
            None => return Err(SecurityError::Configuration("No template connection".to_string())),
        };
        connection.set_certificate(cert);
        
        // Create socket for the client if needed
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.clone().ok_or_else(|| 
            SecurityError::NotInitialized("Server socket not initialized".to_string()))?;
        drop(socket_guard);
        
        // Create a transport specifically for this client
        let transport = match connection::create_dtls_transport(&socket).await {
            Ok(transport) => transport,
            Err(e) => return Err(e),
        };
        
        debug!("DTLS transport started for client {}", addr);
        
        // Set transport on the connection (clone the Arc)
        connection.set_transport(transport.clone());
        
        // Create client context
        let client_ctx = Arc::new(DefaultClientSecurityContext::new(
            addr,
            Some(connection),
            Some(socket.clone()),
            self.config.clone(),
            Some(transport.clone()),
        ));
        
        // Start a task to monitor the handshake
        let client_ctx_clone = client_ctx.clone();
        tokio::spawn(async move {
            debug!("Starting handshake monitor task for client {}", addr);
            
            // Wait for handshake to complete (with timeout)
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                client_ctx_clone.wait_for_handshake()
            ).await {
                Ok(Ok(_)) => {
                    debug!("Server-side handshake completed successfully for client {}", addr);
                },
                Ok(Err(e)) => {
                    warn!("Server-side handshake failed for client {}: {}", addr, e);
                },
                Err(_) => {
                    warn!("Server-side handshake timed out for client {}", addr);
                }
            }
        });
        
        // Store the client context
        {
            let mut clients = self.clients.write().await;
            clients.insert(addr, client_ctx.clone());
        }
        
        debug!("Created client security context for {} - ready for handshake", addr);
        
        Ok(client_ctx as Arc<dyn ClientSecurityContext + Send + Sync>)
    }
    
    async fn get_client_contexts(&self) -> Vec<Arc<dyn ClientSecurityContext + Send + Sync>> {
        let clients = self.clients.read().await;
        clients.values()
            .map(|c| c.clone() as Arc<dyn ClientSecurityContext + Send + Sync>)
            .collect()
    }
    
    async fn remove_client(&self, addr: SocketAddr) -> Result<(), SecurityError> {
        let mut clients = self.clients.write().await;
        
        if let Some(client) = clients.remove(&addr) {
            // Close the client security context
            client.close().await?;
            Ok(())
        } else {
            // Client not found, nothing to do
            Ok(())
        }
    }
    
    async fn on_client_secure(&self, callback: Box<dyn Fn(Arc<dyn ClientSecurityContext + Send + Sync>) + Send + Sync>) -> Result<(), SecurityError> {
        let mut callbacks = self.client_secure_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
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
        conversion::create_security_info(
            self.config.security_mode,
            None, // Will be filled by async get_fingerprint method
            &self.config.fingerprint_algorithm,
            &self.config.srtp_profiles
        )
    }

    async fn process_client_packet(&self, addr: SocketAddr, data: &[u8]) -> Result<(), SecurityError> {
        // Check if there's an existing client context for this address
        let client_ctx = {
            let clients = self.clients.read().await;
            clients.get(&addr).cloned()
        };
        
        // If client context exists, delegate to it
        if let Some(client_ctx) = client_ctx {
            debug!("Processing DTLS packet from existing client {}", addr);
            client_ctx.process_dtls_packet(data).await
        } else {
            // No client context exists yet - create one
            debug!("Creating new client context for {}", addr);
            let client_ctx = self.create_client_context(addr).await?;
            
            // Start the handshake first - this initializes the server state machine
            debug!("Starting server handshake with client {}", addr);
            if let Err(e) = client_ctx.start_handshake_with_remote(addr).await {
                warn!("Failed to start handshake with client {}: {}", addr, e);
                return Err(e);
            }
            
            // Now process the first packet - usually a ClientHello
            debug!("Processing initial packet from client {}", addr);
            client_ctx.process_dtls_packet(data).await
        }
    }

    async fn start_packet_handler(&self) -> Result<(), SecurityError> {
        // Get the server socket
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.clone().ok_or_else(|| 
            SecurityError::Configuration("No socket set for server security context".to_string()))?;
        drop(socket_guard);
        
        // Create a handler function that captures self
        let this = self.clone();
        let handler = move |data: Vec<u8>, addr: SocketAddr| {
            // This is a bit of a hack because we need to convert to an async call
            // inside a sync closure - but it works because we use a separate task
            let this_clone = this.clone();
            let data_clone = data.clone();
            
            // Spawn a new task to handle the packet
            tokio::spawn(async move {
                if let Err(e) = this_clone.process_client_packet(addr, &data_clone).await {
                    debug!("Error processing client packet: {:?}", e);
                }
            });
            
            // Return success immediately to the caller
            Ok(())
        };
        
        // Start the packet handler using the transport module
        transport::start_packet_handler(&socket, handler).await
    }

    async fn capture_initial_packet(&self) -> Result<Option<(Vec<u8>, SocketAddr)>, SecurityError> {
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.as_ref().ok_or_else(||
            SecurityError::NotInitialized("No socket set for server security context".to_string()))?;
        
        // Use the transport module to capture the initial packet
        transport::capture_initial_packet(socket, 2).await
    }

    async fn is_ready(&self) -> Result<bool, SecurityError> {
        // Delegate to the core context module
        context::is_security_context_ready(&self.socket, &self.connection_template).await
    }
} 