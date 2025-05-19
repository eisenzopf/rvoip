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
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_NULL_NULL};
use crate::srtp::crypto::SrtpCryptoKey;
use crate::api::server::security::SocketHandle;

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
        
        // Create DTLS connection config from our API config
        let dtls_config = create_dtls_config(&self.config);
        
        // Create DTLS connection
        let mut connection = DtlsConnection::new(dtls_config);
        
        // Generate or load certificate (would be better to do this from config)
        let cert = crate::dtls::crypto::verify::generate_self_signed_certificate()
            .map_err(|e| SecurityError::Configuration(format!("Failed to generate certificate: {}", e)))?;
        
        // Set the certificate on the connection
        connection.set_certificate(cert);
        
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
            // For AES GCM, we'd need equivalent constants which aren't shown in the snippet
            // For now, fall back to AES CM
            _ => SRTP_AES128_CM_SHA1_80,
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
        let remote_addr = self.remote_addr.lock().await;
        if remote_addr.is_none() {
            return Err(SecurityError::Handshake("Remote address not set".to_string()));
        }
        let remote_addr = remote_addr.unwrap();
        drop(remote_addr);
        
        let socket = self.socket.lock().await;
        if socket.is_none() {
            return Err(SecurityError::Handshake("Socket not set".to_string()));
        }
        drop(socket);
        
        // Start DTLS handshake
        let mut conn = self.connection.lock().await;
        if let Some(conn) = conn.as_mut() {
            // Start the handshake
            match conn.start_handshake(remote_addr).await {
                Ok(_) => {},
                Err(e) => return Err(SecurityError::Handshake(format!("Failed to start DTLS handshake: {}", e)))
            }
                
            // Set up task to wait for handshake completion
            let connection_clone = self.connection.clone();
            let srtp_context_clone = self.srtp_context.clone();
            let handshake_completed_clone = self.handshake_completed.clone();
            let profiles = self.config.srtp_profiles.clone();
            
            tokio::spawn(async move {
                let mut conn_guard = connection_clone.lock().await;
                if let Some(conn) = conn_guard.as_mut() {
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
                                    match SrtpContext::new(profile, client_key) {
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
            // This would typically involve creating a DTLS transport
            // For now, we'll just set the remote address
            if let Some(remote_addr) = socket.remote_addr {
                let mut remote_addr_guard = self.remote_addr.lock().await;
                *remote_addr_guard = Some(remote_addr);
                
                // Set remote address on connection if supported
                if let Err(e) = self.set_remote_address(remote_addr).await {
                    warn!("Failed to set remote address on DTLS connection: {}", e);
                }
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
} 