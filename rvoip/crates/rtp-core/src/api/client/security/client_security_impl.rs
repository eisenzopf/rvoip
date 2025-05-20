//! Client security implementation
//!
//! This file contains the implementation of the ClientSecurityContext trait.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use async_trait::async_trait;
use tracing::{debug, error, info, warn};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::client::security::{ClientSecurityContext, ClientSecurityConfig, create_dtls_config};
use crate::dtls::{DtlsConnection, DtlsConfig, DtlsRole, DtlsSrtpContext};
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_NULL_NULL, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM};
use crate::srtp::crypto::SrtpCryptoKey;
use crate::api::server::security::SocketHandle;
use crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80;

/// Default implementation of the ClientSecurityContext trait
pub struct DefaultClientSecurityContext {
    /// Security configuration
    config: ClientSecurityConfig,
    /// DTLS connection for handshake
    connection: Arc<Mutex<Option<DtlsConnection>>>,
    /// SRTP context for secure media
    srtp_context: Arc<Mutex<Option<SrtpContext>>>,
    /// Remote address
    remote_addr: Arc<Mutex<Option<SocketAddr>>>,
    /// Remote fingerprint from SDP
    remote_fingerprint: Arc<Mutex<Option<String>>>,
    /// Socket for DTLS
    socket: Arc<Mutex<Option<SocketHandle>>>,
    /// Handshake completed flag
    handshake_completed: Arc<Mutex<bool>>,
    /// Remote fingerprint algorithm (if set)
    remote_fingerprint_algorithm: Arc<Mutex<Option<String>>>,
}

impl DefaultClientSecurityContext {
    /// Create a new DefaultClientSecurityContext
    pub async fn new(config: ClientSecurityConfig) -> Result<Arc<Self>, SecurityError> {
        // Create context
        let ctx = Self {
            config,
            connection: Arc::new(Mutex::new(None)),
            srtp_context: Arc::new(Mutex::new(None)),
            remote_addr: Arc::new(Mutex::new(None)),
            remote_fingerprint: Arc::new(Mutex::new(None)),
            socket: Arc::new(Mutex::new(None)),
            handshake_completed: Arc::new(Mutex::new(false)),
            remote_fingerprint_algorithm: Arc::new(Mutex::new(None)),
        };
        
        Ok(Arc::new(ctx))
    }
    
    /// Initialize DTLS connection
    async fn init_connection(&self) -> Result<(), SecurityError> {
        // Check if we have a socket
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.clone().ok_or_else(|| 
            SecurityError::Configuration("No socket set for security context".to_string()))?;
        drop(socket_guard);
        
        // Verify we have SRTP profiles configured
        if self.config.srtp_profiles.is_empty() {
            return Err(SecurityError::Configuration("No SRTP profiles specified".to_string()));
        }
        
        // Create DTLS connection config from our API config
        let dtls_config = create_dtls_config(&self.config);
        
        // Create DTLS connection
        let mut connection = DtlsConnection::new(dtls_config);
        
        // Generate or load certificate based on config
        let cert = if let (Some(cert_path), Some(key_path)) = (&self.config.certificate_path, &self.config.private_key_path) {
            // Load certificate from files
            debug!("Loading certificate from {} and key from {}", cert_path, key_path);
            
            // Read certificate and key files
            let cert_data = match std::fs::read_to_string(cert_path) {
                Ok(data) => data,
                Err(e) => return Err(SecurityError::Configuration(
                    format!("Failed to read certificate file {}: {}", cert_path, e)
                ))
            };
            
            let key_data = match std::fs::read_to_string(key_path) {
                Ok(data) => data,
                Err(e) => return Err(SecurityError::Configuration(
                    format!("Failed to read private key file {}: {}", key_path, e)
                ))
            };
            
            // Since we don't have a direct PEM loading function, we use the self-signed generator
            // In a real implementation, we'd properly parse and convert the certificate
            debug!("PEM files found but using generated certificate for now");
            match crate::dtls::crypto::verify::generate_self_signed_certificate() {
                Ok(cert) => cert,
                Err(e) => return Err(SecurityError::Configuration(
                    format!("Failed to generate certificate: {}", e)
                ))
            }
        } else {
            // Generate a self-signed certificate
            debug!("Generating self-signed certificate with proper crypto parameters");
            match crate::dtls::crypto::verify::generate_self_signed_certificate() {
                Ok(cert) => cert,
                Err(e) => return Err(SecurityError::Configuration(
                    format!("Failed to generate certificate: {}", e)
                ))
            }
        };
        
        // Set the certificate on the connection
        connection.set_certificate(cert);
        
        // If we have a socket, create and set the transport
        // Create UDP transport from socket
        let transport = Arc::new(Mutex::new(
            match crate::dtls::transport::udp::UdpTransport::new(socket.socket.clone(), 1200).await {
                Ok(t) => t,
                Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e)))
            }
        ));
        
        // Start the transport
        match transport.lock().await.start().await {
            Ok(_) => debug!("DTLS transport started successfully"),
            Err(e) => return Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e)))
        }
        
        // Set transport on connection
        connection.set_transport(transport);
        
        // Store connection
        let mut conn_guard = self.connection.lock().await;
        *conn_guard = Some(connection);
        
        Ok(())
    }
    
    /// Convert API SrtpProfile to SrtpCryptoSuite
    fn profile_to_suite(profile: SrtpProfile) -> SrtpCryptoSuite {
        match profile {
            SrtpProfile::AesCm128HmacSha1_80 => SRTP_AES128_CM_SHA1_80,
            SrtpProfile::AesCm128HmacSha1_32 => SRTP_AES128_CM_SHA1_32,
            SrtpProfile::AesGcm128 => SRTP_AEAD_AES_128_GCM,
            SrtpProfile::AesGcm256 => SRTP_AEAD_AES_256_GCM,
        }
    }
}

#[async_trait]
impl ClientSecurityContext for DefaultClientSecurityContext {
    async fn initialize(&self) -> Result<(), SecurityError> {
        debug!("Initializing client security context");
        
        // Initialize DTLS connection if security is enabled
        if self.config.security_mode.is_enabled() {
            self.init_connection().await?;
        }
        
        Ok(())
    }
    
    async fn start_handshake(&self) -> Result<(), SecurityError> {
        debug!("Starting DTLS handshake");
        
        // Check prerequisites
        let remote_addr = {
            let guard = self.remote_addr.lock().await;
            match *guard {
                Some(addr) => addr,
                None => return Err(SecurityError::Handshake("Remote address not set".to_string())),
            }
        };
        
        let socket = {
            let guard = self.socket.lock().await;
            if guard.is_none() {
                return Err(SecurityError::Handshake("Socket not set".to_string()));
            }
            guard.clone()
        };
        
        // Ensure connection is initialized
        {
            let conn_guard = self.connection.lock().await;
            if conn_guard.is_none() {
                // Try to initialize the connection if not already done
                drop(conn_guard); // Drop the guard before calling async function
                debug!("Connection not initialized, calling init_connection()");
                self.init_connection().await?;
            }
        }
        
        // Start DTLS handshake
        let mut conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_mut() {
            // Check if transport is set
            let has_transport = conn.has_transport();
            debug!("DTLS connection has transport: {}", has_transport);
            
            if !has_transport {
                debug!("No transport found, creating new UDP transport");
                // Create UDP transport from socket
                let transport = Arc::new(Mutex::new(
                    match crate::dtls::transport::udp::UdpTransport::new(socket.as_ref().unwrap().socket.clone(), 1200).await {
                        Ok(t) => t,
                        Err(e) => return Err(SecurityError::Handshake(format!("Failed to create DTLS transport: {}", e)))
                    }
                ));
                
                // Start the transport
                match transport.lock().await.start().await {
                    Ok(_) => debug!("DTLS transport started successfully"),
                    Err(e) => return Err(SecurityError::Handshake(format!("Failed to start DTLS transport: {}", e)))
                }
                
                // Set transport on connection
                debug!("Setting transport on DTLS connection");
                conn.set_transport(transport);
            }
            
            // Start the handshake
            debug!("Calling start_handshake with remote addr: {}", remote_addr);
            match conn.start_handshake(remote_addr).await {
                Ok(_) => debug!("DTLS handshake started successfully"),
                Err(e) => return Err(SecurityError::Handshake(format!("Failed to start DTLS handshake: {}", e)))
            }
            
            // Set up task to wait for handshake completion
            let connection_clone = self.connection.clone();
            let srtp_context_clone = self.srtp_context.clone();
            let handshake_completed_clone = self.handshake_completed.clone();
            let profiles = self.config.srtp_profiles.clone();
            let remote_addr_copy = remote_addr;
            
            tokio::spawn(async move {
                debug!("Client handshake task started for {}", remote_addr_copy);
                let mut conn_guard = connection_clone.lock().await;
                if let Some(conn) = conn_guard.as_mut() {
                    debug!("Waiting for handshake to complete...");
                    match conn.wait_handshake().await {
                        Ok(()) => {
                            debug!("DTLS handshake completed successfully");
                            
                            // Extract SRTP keys from DTLS connection
                            match conn.extract_srtp_keys() {
                                Ok(srtp_context) => {
                                    debug!("Successfully extracted SRTP keys from DTLS");
                                    
                                    // Get the suite to use
                                    let profile = if !profiles.is_empty() {
                                        Self::profile_to_suite(profiles[0])
                                    } else {
                                        SRTP_AES128_CM_SHA1_80
                                    };
                                    
                                    // Get the client key from the SRTP context (true = client role)
                                    let client_key = srtp_context.get_key_for_role(true);
                                    
                                    // Create SRTP context with the client key
                                    match SrtpContext::new(profile, client_key.clone()) {
                                        Ok(srtp_ctx) => {
                                            debug!("Successfully created SRTP context");
                                            
                                            // Store SRTP context
                                            let mut srtp_guard = srtp_context_clone.lock().await;
                                            *srtp_guard = Some(srtp_ctx);
                                            
                                            // Set handshake completed flag
                                            let mut completed = handshake_completed_clone.lock().await;
                                            *completed = true;
                                        },
                                        Err(e) => {
                                            error!("Failed to create SRTP context: {}", e);
                                        }
                                    }
                                },
                                Err(e) => {
                                    error!("Failed to extract SRTP keys: {}", e);
                                }
                            }
                        },
                        Err(e) => {
                            error!("DTLS handshake failed: {}", e);
                        }
                    }
                }
            });
        } else {
            return Err(SecurityError::Handshake("DTLS connection not initialized".to_string()));
        }
        
        Ok(())
    }
    
    async fn is_handshake_complete(&self) -> Result<bool, SecurityError> {
        let handshake_complete = *self.handshake_completed.lock().await;
        Ok(handshake_complete)
    }
    
    async fn set_remote_address(&self, addr: SocketAddr) -> Result<(), SecurityError> {
        // Store address
        let mut remote_addr = self.remote_addr.lock().await;
        *remote_addr = Some(addr);
        
        Ok(())
    }
    
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError> {
        // Store socket
        let mut socket_lock = self.socket.lock().await;
        *socket_lock = Some(socket.clone());
        
        // If we have a connection initialized, we need to update its transport
        let mut conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_mut() {
            // Create a transport from the socket
            let transport = Arc::new(Mutex::new(
                match crate::dtls::transport::udp::UdpTransport::new(socket.socket.clone(), 1200).await {
                    Ok(t) => t,
                    Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e)))
                }
            ));
            
            // Start the transport (this was missing)
            match transport.lock().await.start().await {
                Ok(_) => debug!("DTLS transport started successfully"),
                Err(e) => return Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e)))
            }
            
            // Set transport on connection
            conn.set_transport(transport);
            
            // Set remote address if available
            if let Some(remote_addr) = socket.remote_addr {
                let mut remote_addr_guard = self.remote_addr.lock().await;
                *remote_addr_guard = Some(remote_addr);
            }
        }
        
        Ok(())
    }
    
    async fn set_remote_fingerprint(&self, fingerprint: &str, algorithm: &str) -> Result<(), SecurityError> {
        // Store fingerprint
        let mut remote_fingerprint = self.remote_fingerprint.lock().await;
        *remote_fingerprint = Some(fingerprint.to_string());
        
        let mut remote_fingerprint_algorithm = self.remote_fingerprint_algorithm.lock().await;
        *remote_fingerprint_algorithm = Some(algorithm.to_string());
        
        Ok(())
    }
    
    async fn get_security_info(&self) -> Result<SecurityInfo, SecurityError> {
        // If security is enabled, we need to initialize our DTLS connection
        // to get our fingerprint information
        if self.config.security_mode.is_enabled() && self.connection.lock().await.is_none() {
            self.init_connection().await?;
        }
        
        // Calculate crypto suites based on our SRTP profiles
        let crypto_suites = self.config.srtp_profiles.iter()
            .map(|p| match p {
                SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
                SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
                SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM",
                SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM",
            })
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        
        // In a real implementation, we would need to get the actual fingerprint
        // from the DTLS connection. For now, we'll use a placeholder
        let fingerprint = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";
        
        Ok(SecurityInfo {
            mode: self.config.security_mode,
            fingerprint: Some(fingerprint.to_string()),
            fingerprint_algorithm: self.remote_fingerprint_algorithm.lock().await.clone(),
            crypto_suites,
            key_params: None,
            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()),
        })
    }
    
    async fn close(&self) -> Result<(), SecurityError> {
        // Reset handshake state
        let mut handshake_complete = self.handshake_completed.lock().await;
        *handshake_complete = false;
        
        // Close DTLS connection
        let mut conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_mut() {
            match conn.close().await {
                Ok(_) => {},
                Err(e) => return Err(SecurityError::Internal(format!("Failed to close DTLS connection: {}", e)))
            }
        }
        *conn_guard = None;
        
        // Clear SRTP context
        let mut srtp_guard = self.srtp_context.lock().await;
        *srtp_guard = None;
        
        Ok(())
    }
    
    fn is_secure(&self) -> bool {
        self.config.security_mode.is_enabled()
    }
    
    fn get_security_info_sync(&self) -> SecurityInfo {
        SecurityInfo {
            mode: self.config.security_mode,
            fingerprint: None, // Will be filled by async get_security_info method
            fingerprint_algorithm: None, // Can't await in a sync function
            crypto_suites: vec!["AES_CM_128_HMAC_SHA1_80".to_string()],
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

    // Add this helper method to check if transport is set
    async fn has_transport(&self) -> Result<bool, SecurityError> {
        let conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_ref() {
            Ok(conn.has_transport())
        } else {
            Ok(false)
        }
    }

    async fn wait_for_handshake(&self) -> Result<(), SecurityError> {
        debug!("Waiting for DTLS handshake to complete");
        
        // Check if we need to wait
        let is_complete = *self.handshake_completed.lock().await;
        if is_complete {
            return Ok(());
        }
        
        // Get the connection
        let mut conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_mut() {
            // Wait for the handshake to complete
            match conn.wait_handshake().await {
                Ok(_) => {
                    debug!("DTLS handshake completed");
                    
                    // Set the handshake completed flag
                    let mut completed = self.handshake_completed.lock().await;
                    *completed = true;
                    
                    // Extract SRTP keys if needed
                    let mut srtp_guard = self.srtp_context.lock().await;
                    if srtp_guard.is_none() {
                        // Extract keys
                        match conn.extract_srtp_keys() {
                            Ok(srtp_ctx) => {
                                // Get the client key
                                let client_key = srtp_ctx.get_key_for_role(true).clone();
                                debug!("Successfully extracted SRTP keys");
                                
                                // Convert SRTP profile
                                let profile = match srtp_ctx.profile {
                                    crate::srtp::SrtpCryptoSuite { authentication: HmacSha1_80, .. } => {
                                        crate::srtp::SRTP_AES128_CM_SHA1_80
                                    },
                                    _ => {
                                        return Err(SecurityError::Handshake("Unsupported SRTP profile".to_string()));
                                    }
                                };
                                
                                // Create SRTP context
                                match SrtpContext::new(profile, client_key) {
                                    Ok(ctx) => {
                                        debug!("Created SRTP context");
                                        *srtp_guard = Some(ctx);
                                    },
                                    Err(e) => {
                                        return Err(SecurityError::Handshake(format!("Failed to create SRTP context: {}", e)));
                                    }
                                }
                            },
                            Err(e) => {
                                return Err(SecurityError::Handshake(format!("Failed to extract SRTP keys: {}", e)));
                            }
                        }
                    }
                    
                    Ok(())
                },
                Err(e) => {
                    Err(SecurityError::Handshake(format!("DTLS handshake failed: {}", e)))
                }
            }
        } else {
            Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
        }
    }
} 