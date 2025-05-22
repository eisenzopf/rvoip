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
            debug!("Server processing DTLS packet of {} bytes from client {}", data.len(), self.address);
            
            // Process the packet with the DTLS library
            match conn.process_packet(data).await {
                Ok(_) => {
                    // Take action based on handshake step
                    if let Some(step) = conn.handshake_step() {
                        debug!("Current handshake step: {:?}", step);
                        
                        match step {
                            crate::dtls::handshake::HandshakeStep::SentHelloVerifyRequest => {
                                debug!("Server sent HelloVerifyRequest, waiting for ClientHello with cookie");
                                // No action needed, wait for client to respond with cookie
                            },
                            crate::dtls::handshake::HandshakeStep::ReceivedClientHello => {
                                debug!("Server received ClientHello, sending ServerHello");
                                
                                // Continue the handshake to send ServerHello and ServerKeyExchange
                                if let Err(e) = conn.continue_handshake().await {
                                    warn!("Failed to continue handshake after ClientHello: {}", e);
                                } else {
                                    debug!("Successfully sent ServerHello after ClientHello");
                                }
                            },
                            crate::dtls::handshake::HandshakeStep::ReceivedClientKeyExchange => {
                                debug!("Server received ClientKeyExchange, sending ChangeCipherSpec and Finished");
                                
                                // Continue the handshake to send final messages
                                if let Err(e) = conn.continue_handshake().await {
                                    warn!("Failed to continue handshake after ClientKeyExchange: {}", e);
                                } else {
                                    debug!("Successfully completed handshake from server side");
                                }
                            },
                            crate::dtls::handshake::HandshakeStep::Complete => {
                                debug!("Server handshake complete with client {}", self.address);
                                
                                // Set handshake completed flag
                                let mut completed = self.handshake_completed.lock().await;
                                if !*completed {
                                    *completed = true;
                                    
                                    // Extract SRTP keys
                                    match conn.extract_srtp_keys() {
                                        Ok(srtp_ctx) => {
                                            // Get server key (false = server)
                                            let server_key = srtp_ctx.get_key_for_role(false).clone();
                                            
                                            // Create SRTP context for server role
                                            match SrtpContext::new(srtp_ctx.profile, server_key) {
                                                Ok(ctx) => {
                                                    // Store SRTP context
                                                    let mut srtp_guard = self.srtp_context.lock().await;
                                                    *srtp_guard = Some(ctx);
                                                    info!("Server successfully extracted SRTP keys for client {}", self.address);
                                                },
                                                Err(e) => warn!("Failed to create server SRTP context: {}", e)
                                            }
                                        },
                                        Err(e) => warn!("Failed to extract SRTP keys: {}", e)
                                    }
                                }
                            },
                            _ => {} // Ignore other steps
                        }
                    }
                    
                    Ok(())
                },
                Err(e) => {
                    debug!("Error processing DTLS packet: {}", e);
                    
                    // If this was a cookie validation error, we might need to restart
                    if e.to_string().contains("Invalid cookie") {
                        debug!("Cookie validation failed, restarting handshake");
                        
                        // Start a new handshake
                        if let Err(restart_err) = conn.start_handshake(self.address).await {
                            warn!("Failed to restart handshake: {}", restart_err);
                        }
                    }
                    
                    // Return success to allow handshake to continue
                    Ok(())
                }
            }
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
            debug!("Waiting for DTLS handshake completion for client {}", address);
            
            let conn_result = {
                let mut conn_guard = connection.lock().await;
                match conn_guard.as_mut() {
                    Some(conn) => conn.wait_handshake().await,
                    None => {
                        error!("No DTLS connection for client {}", address);
                        return;
                    }
                }
            };
            
            match conn_result {
                Ok(_) => {
                    debug!("DTLS handshake completed for client {}", address);
                    
                    // Extract SRTP keys
                    let conn_guard = connection.lock().await;
                    if let Some(conn) = conn_guard.as_ref() {
                        match conn.extract_srtp_keys() {
                            Ok(srtp_ctx) => {
                                // Get the key for server role
                                let server_key = srtp_ctx.get_key_for_role(false).clone();
                                debug!("Extracted SRTP keys for client {}", address);
                                
                                // Convert to SRTP profile
                                let profile = match srtp_ctx.profile {
                                    crate::srtp::SrtpCryptoSuite { authentication: crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80, .. } => {
                                        crate::srtp::SRTP_AES128_CM_SHA1_80
                                    },
                                    _ => {
                                        error!("Unsupported SRTP profile for client {}", address);
                                        return;
                                    }
                                };
                                
                                // Create SRTP context
                                match SrtpContext::new(profile, server_key) {
                                    Ok(srtp_ctx) => {
                                        debug!("Created SRTP context for client {}", address);
                                        
                                        // Store SRTP context
                                        let mut srtp_guard = srtp_context.lock().await;
                                        *srtp_guard = Some(srtp_ctx);
                                        
                                        // Set handshake completed flag
                                        let mut completed = handshake_completed.lock().await;
                                        *completed = true;
                                        
                                        debug!("DTLS handshake fully completed for client {}", address);
                                    },
                                    Err(e) => {
                                        error!("Failed to create SRTP context for client {}: {}", address, e);
                                    }
                                }
                            },
                            Err(e) => {
                                error!("Failed to extract SRTP keys for client {}: {}", address, e);
                            }
                        }
                    }
                },
                Err(e) => {
                    error!("DTLS handshake failed for client {}: {}", address, e);
                }
            }
        });
        
        Ok(())
    }

    /// Start a handshake with the remote
    pub async fn start_handshake_with_remote(&self, remote_addr: SocketAddr) -> Result<(), SecurityError> {
        // Access the DTLS connection
        let mut conn_guard = self.connection.lock().await;
        
        if let Some(conn) = conn_guard.as_mut() {
            // Start the DTLS handshake - matches dtls_test.rs sequence
            debug!("Starting DTLS handshake with client {}", remote_addr);
            
            match conn.start_handshake(remote_addr).await {
                Ok(_) => Ok(()),
                Err(e) => Err(SecurityError::Handshake(format!("Failed to start DTLS handshake: {}", e)))
            }
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
                .map(|p| match p {
                    SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
                    SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
                    SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM",
                    SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM",
                    _ => "UNKNOWN",
                })
                .map(|s| s.to_string())
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