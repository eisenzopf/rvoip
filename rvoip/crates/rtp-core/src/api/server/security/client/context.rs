//! Client security context implementation
//!
//! This module handles client security contexts managed by the server.

use std::net::SocketAddr;
use std::sync::Arc;
use std::any::Any;
use tokio::sync::Mutex;
use async_trait::async_trait;
use tracing::{debug, error, info, warn};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::server::security::{ClientSecurityContext, ServerSecurityConfig, SocketHandle};
use crate::dtls::{DtlsConnection};
use crate::srtp::{SrtpContext};
use crate::api::server::security::dtls::{handshake, transport};
use crate::api::server::security::srtp::keys;

/// Client security context managed by the server
pub struct DefaultClientSecurityContext {
    /// Client address
    pub address: SocketAddr,
    /// DTLS connection for this client
    pub connection: Arc<Mutex<Option<DtlsConnection>>>,
    /// SRTP context for secure media with this client
    pub srtp_context: Arc<Mutex<Option<SrtpContext>>>,
    /// Handshake completed flag
    pub handshake_completed: Arc<Mutex<bool>>,
    /// Socket for DTLS
    pub socket: Arc<Mutex<Option<SocketHandle>>>,
    /// Server config (shared)
    pub config: ServerSecurityConfig,
    /// Transport used for DTLS
    pub transport: Arc<Mutex<Option<Arc<Mutex<crate::dtls::transport::udp::UdpTransport>>>>>,
    /// Flag indicating that handshake is waiting for first packet
    pub waiting_for_first_packet: Arc<Mutex<bool>>,
    /// Initial packet from client (if received)
    pub initial_packet: Arc<Mutex<Option<Vec<u8>>>>,
}

impl DefaultClientSecurityContext {
    /// Create a new DefaultClientSecurityContext
    pub fn new(
        address: SocketAddr,
        connection: Option<DtlsConnection>,
        socket: Option<SocketHandle>,
        config: ServerSecurityConfig,
        transport: Option<Arc<Mutex<crate::dtls::transport::udp::UdpTransport>>>,
    ) -> Self {
        Self {
            address,
            connection: Arc::new(Mutex::new(connection)),
            srtp_context: Arc::new(Mutex::new(None)),
            handshake_completed: Arc::new(Mutex::new(false)),
            socket: Arc::new(Mutex::new(socket)),
            config,
            transport: Arc::new(Mutex::new(transport)),
            initial_packet: Arc::new(Mutex::new(None)),
            waiting_for_first_packet: Arc::new(Mutex::new(false)),
        }
    }

    /// Process a DTLS packet received from the client
    pub async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        let mut conn_guard = self.connection.lock().await;
        
        if let Some(conn) = conn_guard.as_mut() {
            // Delegate to the handshake module to process the packet
            handshake::process_dtls_packet(
                conn, 
                data, 
                self.address, 
                &self.handshake_completed, 
                &self.srtp_context
            ).await
        } else {
            Err(SecurityError::NotInitialized("DTLS connection not initialized for client".to_string()))
        }
    }
    
    /// Spawn a task to wait for handshake completion
    pub async fn spawn_handshake_task(&self) -> Result<(), SecurityError> {
        // Clone values needed for the task
        let address = self.address;
        let connection = self.connection.clone();
        let srtp_context = self.srtp_context.clone();
        let handshake_completed = self.handshake_completed.clone();
        
        // Spawn the task
        tokio::spawn(async move {
            // Delegate to the handshake module
            let result = handshake::wait_for_handshake(
                &connection,
                address,
                &handshake_completed,
                &srtp_context
            ).await;
            
            if let Err(e) = result {
                error!("Handshake task failed for client {}: {}", address, e);
            }
        });
        
        Ok(())
    }

    /// Start a handshake with the remote
    pub async fn start_handshake_with_remote(&self, remote_addr: SocketAddr) -> Result<(), SecurityError> {
        // Access the DTLS connection
        let mut conn_guard = self.connection.lock().await;
        
        if let Some(conn) = conn_guard.as_mut() {
            // Delegate to the handshake module
            handshake::start_handshake(conn, remote_addr).await
        } else {
            Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
        }
    }
}

#[async_trait]
impl ClientSecurityContext for DefaultClientSecurityContext {
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError> {
        // Store socket
        let mut socket_lock = self.socket.lock().await;
        *socket_lock = Some(socket.clone());
        
        // Set up transport if not already done
        let mut transport_guard = self.transport.lock().await;
        if transport_guard.is_none() {
            debug!("Creating DTLS transport for client {}", self.address);
            
            // Create UDP transport
            let new_transport = match crate::dtls::transport::udp::UdpTransport::new(
                socket.socket.clone(), 1500
            ).await {
                Ok(t) => t,
                Err(e) => return Err(SecurityError::Configuration(
                    format!("Failed to create DTLS transport: {}", e)
                ))
            };
            
            // Start the transport
            let new_transport = Arc::new(Mutex::new(new_transport));
            if let Err(e) = new_transport.lock().await.start().await {
                return Err(SecurityError::Configuration(
                    format!("Failed to start DTLS transport: {}", e)
                ));
            }
            
            debug!("DTLS transport started for client {}", self.address);
            *transport_guard = Some(new_transport.clone());
            
            // Set transport on connection if it exists
            let mut conn_guard = self.connection.lock().await;
            if let Some(conn) = conn_guard.as_mut() {
                conn.set_transport(new_transport);
                debug!("Transport set on existing connection for client {}", self.address);
            }
        }
        
        Ok(())
    }
    
    async fn get_remote_fingerprint(&self) -> Result<Option<String>, SecurityError> {
        let conn = self.connection.lock().await;
        if let Some(conn) = conn.as_ref() {
            // Check if handshake is complete and remote certificate is available
            if let Some(remote_cert) = conn.remote_certificate() {
                // Create a mutable copy of the certificate to compute fingerprint
                let mut remote_cert_copy = remote_cert.clone();
                match remote_cert_copy.fingerprint("SHA-256") {
                    Ok(fingerprint) => Ok(Some(fingerprint)),
                    Err(e) => Err(SecurityError::Internal(format!("Failed to get remote fingerprint: {}", e)))
                }
            } else {
                // If no remote certificate yet, return None (not an error)
                Ok(None)
            }
        } else {
            Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
        }
    }
    
    /// Wait for the DTLS handshake to complete
    async fn wait_for_handshake(&self) -> Result<(), SecurityError> {
        let mut conn_guard = self.connection.lock().await;
        
        if let Some(conn) = conn_guard.as_mut() {
            conn.wait_handshake().await
                .map_err(|e| SecurityError::Handshake(format!("DTLS handshake failed: {}", e)))?;
                
            // Set handshake completed flag
            let mut completed = self.handshake_completed.lock().await;
            *completed = true;
            
            Ok(())
        } else {
            Err(SecurityError::HandshakeError("No DTLS connection available".to_string()))
        }
    }
    
    async fn is_handshake_complete(&self) -> Result<bool, SecurityError> {
        let completed = *self.handshake_completed.lock().await;
        Ok(completed)
    }
    
    async fn close(&self) -> Result<(), SecurityError> {
        // Close DTLS connection
        let mut conn = self.connection.lock().await;
        if let Some(conn) = conn.as_mut() {
            // Await the future first, then handle the Result
            match conn.close().await {
                Ok(_) => {},
                Err(e) => return Err(SecurityError::Internal(format!("Failed to close DTLS connection: {}", e)))
            }
        }
        *conn = None;
        
        // Reset handshake state
        let mut completed = self.handshake_completed.lock().await;
        *completed = false;
        
        // Clear SRTP context
        let mut srtp = self.srtp_context.lock().await;
        *srtp = None;
        
        Ok(())
    }
    
    fn is_secure(&self) -> bool {
        self.config.security_mode.is_enabled()
    }
    
    fn get_security_info(&self) -> SecurityInfo {
        SecurityInfo {
            mode: self.config.security_mode,
            fingerprint: None, // Will be filled by async get_fingerprint method
            fingerprint_algorithm: Some(self.config.fingerprint_algorithm.clone()),
            crypto_suites: self.config.srtp_profiles.iter()
                .map(|p| keys::profile_to_string(*p))
                .collect(),
            key_params: None,
            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()),
        }
    }

    async fn get_fingerprint(&self) -> Result<String, SecurityError> {
        let conn_guard = self.connection.lock().await;
        
        if let Some(conn) = conn_guard.as_ref() {
            // Get the certificate from the connection
            if let Some(cert) = conn.local_certificate() {
                // Create a mutable copy of the certificate to compute fingerprint
                let mut cert_copy = cert.clone();
                match cert_copy.fingerprint("SHA-256") {
                    Ok(fingerprint) => Ok(fingerprint),
                    Err(e) => Err(SecurityError::Internal(format!("Failed to get fingerprint: {}", e))),
                }
            } else {
                Err(SecurityError::Configuration("No certificate available".to_string()))
            }
        } else {
            Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
        }
    }

    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError> {
        // Return the default algorithm used
        Ok("sha-256".to_string())
    }

    /// Process a DTLS packet received from the client
    async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        self.process_dtls_packet(data).await
    }

    /// Start a handshake with the remote
    async fn start_handshake_with_remote(&self, remote_addr: SocketAddr) -> Result<(), SecurityError> {
        self.start_handshake_with_remote(remote_addr).await
    }

    /// Allow downcasting for internal implementation details
    fn as_any(&self) -> &dyn Any {
        self
    }
} 