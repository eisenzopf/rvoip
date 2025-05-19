//! Server security implementation
//!
//! This file contains the implementation of the ServerSecurityContext trait.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, RwLock};
use async_trait::async_trait;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::server::security::{ServerSecurityContext, ClientSecurityContext, ServerSecurityConfig};
use crate::api::server::security::{SocketHandle, ConnectionConfig, ConnectionRole};

// Use SrtpProfile from srtp module
use crate::srtp::{SrtpContext, SrtpCryptoSuite};
use crate::srtp::crypto::SrtpCryptoKey;
use crate::dtls::DtlsConnection;
use crate::dtls::DtlsConfig;
use crate::dtls::DtlsRole;

/// Client security context managed by the server
pub struct DefaultClientSecurityContext {
    /// Client address
    address: SocketAddr,
    /// DTLS connection for this client
    connection: Arc<Mutex<Option<DtlsConnection>>>,
    /// SRTP context for secure media with this client
    srtp_context: Arc<Mutex<Option<SrtpContext>>>,
    /// Handshake completed flag
    handshake_completed: Arc<Mutex<bool>>,
    /// Socket for DTLS
    socket: Arc<Mutex<Option<SocketHandle>>>,
    /// Server config (shared)
    config: ServerSecurityConfig,
}

#[async_trait]
impl ClientSecurityContext for DefaultClientSecurityContext {
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError> {
        // Store socket
        let mut socket_lock = self.socket.lock().await;
        *socket_lock = Some(socket.clone());
        
        // Set socket on connection if initialized
        let mut conn = self.connection.lock().await;
        if let Some(conn) = conn.as_mut() {
            conn.set_socket(socket)
                .map_err(|e| SecurityError::Configuration(format!("Failed to set socket on client context: {}", e)))?;
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
    
    async fn wait_for_handshake(&self) -> Result<(), SecurityError> {
        let conn = self.connection.lock().await;
        if let Some(conn) = conn.as_ref() {
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
}

/// Default implementation of the ServerSecurityContext
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
    client_secure_callbacks: Arc<Mutex<Vec<Box<dyn Fn(Arc<dyn ClientSecurityContext>) + Send + Sync>>>>,
}

impl DefaultServerSecurityContext {
    /// Create a new DefaultServerSecurityContext
    pub async fn new(config: ServerSecurityConfig) -> Result<Arc<dyn ServerSecurityContext>, SecurityError> {
        // Create DTLS connection template for the certificate
        let mut conn_config = ConnectionConfig::default();
        conn_config.role = ConnectionRole::Server; // Server is passive
        
        // TODO: Set certificate from config if provided
        
        // Create DTLS connection template for the certificate
        let mut dtls_config = DtlsConfig {
            role: match conn_config.role {
                ConnectionRole::Client => DtlsRole::Client,
                ConnectionRole::Server => DtlsRole::Server,
            },
            version: crate::dtls::DtlsVersion::Dtls12,
            mtu: 1200,
            max_retransmissions: 5,
            srtp_profiles: Self::convert_profiles(&conn_config.srtp_profiles),
        };

        // Create DTLS connection template
        let mut connection = DtlsConnection::new(dtls_config);

        // Generate or load certificate (would be better to do this from config)
        let cert = crate::dtls::crypto::verify::generate_self_signed_certificate()
            .map_err(|e| SecurityError::Configuration(format!("Failed to generate certificate: {}", e)))?;

        // Set the certificate on the connection
        connection.set_certificate(cert);
            
        let ctx = Self {
            config,
            connection_template: Arc::new(Mutex::new(Some(connection))),
            clients: Arc::new(RwLock::new(HashMap::new())),
            socket: Arc::new(Mutex::new(None)),
            client_secure_callbacks: Arc::new(Mutex::new(Vec::new())),
        };
        
        Ok(Arc::new(ctx))
    }
    
    /// Convert external SRTP profile to internal format
    fn convert_profile(profile: SrtpProfile) -> crate::srtp::SrtpCryptoSuite {
        match profile {
            SrtpProfile::AesGcm128 => crate::srtp::SRTP_AEAD_AES_128_GCM,
            SrtpProfile::AesGcm256 => crate::srtp::SRTP_AEAD_AES_256_GCM,
            SrtpProfile::AesCm128HmacSha1_80 => crate::srtp::SRTP_AES128_CM_SHA1_80,
            SrtpProfile::AesCm128HmacSha1_32 => crate::srtp::SRTP_AES128_CM_SHA1_32,
        }
    }
    
    /// Convert u16 profile ID to SrtpCryptoSuite
    fn profile_to_suite(profile_id: u16) -> crate::srtp::SrtpCryptoSuite {
        match profile_id {
            0x0001 => crate::srtp::SRTP_AES128_CM_SHA1_80,
            0x0002 => crate::srtp::SRTP_AES128_CM_SHA1_32,
            0x0007 => crate::srtp::SRTP_AEAD_AES_128_GCM,
            0x0008 => crate::srtp::SRTP_AEAD_AES_256_GCM,
            _ => crate::srtp::SRTP_AES128_CM_SHA1_80,
        }
    }
    
    /// Get the connection template for cloning
    async fn get_connection_template(&self) -> Result<(), SecurityError> {
        let template = self.connection_template.lock().await;
        if template.is_none() {
            Err(SecurityError::Configuration("DTLS connection template not initialized".to_string()))
        } else {
            Ok(())
        }
    }

    /// Get the fingerprint from the template
    async fn get_fingerprint_from_template(&self) -> Result<String, SecurityError> {
        let template = self.connection_template.lock().await;
        if let Some(template) = template.as_ref() {
            template.get_local_fingerprint()
                .map_err(|e| SecurityError::Configuration(format!("Failed to get fingerprint: {}", e)))
        } else {
            Err(SecurityError::Configuration("DTLS connection template not initialized".to_string()))
        }
    }

    /// Get the fingerprint algorithm from the template
    async fn get_fingerprint_algorithm_from_template(&self) -> Result<String, SecurityError> {
        let template = self.connection_template.lock().await;
        if let Some(template) = template.as_ref() {
            template.get_local_fingerprint_algorithm()
                .map_err(|e| SecurityError::Configuration(format!("Failed to get fingerprint algorithm: {}", e)))
        } else {
            Err(SecurityError::Configuration("DTLS connection template not initialized".to_string()))
        }
    }

    /// Convert SrtpProfile vector to SrtpCryptoSuite vector
    fn convert_profiles(profiles: &[SrtpProfile]) -> Vec<crate::srtp::SrtpCryptoSuite> {
        profiles.iter().map(|p| Self::convert_profile(*p)).collect()
    }
}

#[async_trait]
impl ServerSecurityContext for DefaultServerSecurityContext {
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError> {
        let mut socket_lock = self.socket.lock().await;
        *socket_lock = Some(socket);
        Ok(())
    }
    
    async fn get_fingerprint(&self) -> Result<String, SecurityError> {
        self.get_fingerprint_from_template().await
    }
    
    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError> {
        self.get_fingerprint_algorithm_from_template().await
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
    
    async fn create_client_context(&self, addr: SocketAddr) -> Result<Arc<dyn ClientSecurityContext>, SecurityError> {
        // Check if we already have a context for this client
        let clients = self.clients.read().await;
        if let Some(client) = clients.get(&addr) {
            return Ok(client.clone() as Arc<dyn ClientSecurityContext>);
        }
        drop(clients);
        
        debug!("Creating new security context for client {}", addr);
        
        // Get socket
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.clone().ok_or_else(|| 
            SecurityError::Configuration("No socket set for server security context".to_string()))?;
        drop(socket_guard);
        
        // Create DTLS connection for this client
        let mut conn_config = ConnectionConfig::default();
        conn_config.role = ConnectionRole::Server; // Server is passive
        
        // TODO: Set certificate from config if provided
        
        // Create DTLS connection
        let mut dtls_config = DtlsConfig {
            role: DtlsRole::Server,
            version: crate::dtls::DtlsVersion::Dtls12,
            mtu: 1200,
            max_retransmissions: 5,
            srtp_profiles: Self::convert_profiles(&conn_config.srtp_profiles),
        };

        let connection = DtlsConnection::new(dtls_config);

        // Set remote address
        connection.set_remote_address(addr)
            .map_err(|e| SecurityError::Configuration(format!("Failed to set client address: {}", e)))?;
            
        // Create client context
        let client = DefaultClientSecurityContext {
            address: addr,
            connection: Arc::new(Mutex::new(Some(connection))),
            srtp_context: Arc::new(Mutex::new(None)),
            handshake_completed: Arc::new(Mutex::new(false)),
            socket: Arc::new(Mutex::new(Some(socket))),
            config: self.config.clone(),
        };
        
        // Wrap in Arc
        let client_arc = Arc::new(client);
        
        // Store in clients map
        let mut clients = self.clients.write().await;
        clients.insert(addr, client_arc.clone());
        
        // Start DTLS listener
        let conn = client_arc.connection.lock().await;
        if let Some(conn) = conn.as_ref() {
            conn.start_listening()
                .map_err(|e| SecurityError::HandshakeError(format!("Failed to start DTLS listener: {}", e)))?;
        }
        drop(conn);
        
        // Spawn task to handle handshake completion
        let client_clone = client_arc.clone() as Arc<dyn ClientSecurityContext>;
        let callbacks = self.client_secure_callbacks.clone();
        let connection = client_arc.connection.clone();
        let srtp_context = client_arc.srtp_context.clone();
        let handshake_completed = client_arc.handshake_completed.clone();
        let profiles = self.config.srtp_profiles.clone();
        
        tokio::spawn(async move {
            let conn_guard = connection.lock().await;
            if let Some(conn) = conn_guard.as_ref() {
                match conn.wait_handshake().await {
                    Ok(()) => {
                        debug!("DTLS handshake completed successfully for client {}", addr);
                        
                        // Create SRTP context from DTLS connection
                        match conn.extract_srtp_keys() {
                            Ok(srtp_context) => {
                                debug!("Successfully derived SRTP keys for client {}", addr);
                                
                                // Get the profile to use
                                let profile = if !profiles.is_empty() {
                                    match Self::convert_profile(profiles[0]) {
                                        // These numbers correspond to SrtpCryptoSuite values
                                        0x0001 => crate::srtp::SRTP_AES128_CM_SHA1_80,
                                        0x0002 => crate::srtp::SRTP_AES128_CM_SHA1_32,
                                        0x0007 => crate::srtp::SRTP_AEAD_AES_128_GCM,
                                        0x0008 => crate::srtp::SRTP_AEAD_AES_256_GCM,
                                        _ => crate::srtp::SRTP_AES128_CM_SHA1_80,
                                    }
                                } else {
                                    crate::srtp::SRTP_AES128_CM_SHA1_80
                                };
                                
                                // Get the server key from the SRTP context (false = server role)
                                let server_key = srtp_context.get_key_for_role(false);
                                
                                // Create SRTP context with the server key
                                let srtp_ctx = match SrtpContext::new(profile, server_key.clone()) {
                                    Ok(context) => context,
                                    Err(e) => {
                                        error!("Failed to create SRTP context for client {}: {}", addr, e);
                                        return Err(SecurityError::Configuration(format!("Failed to create SRTP context: {}", e)));
                                    }
                                };
                                
                                // Store SRTP context
                                let mut srtp_guard = srtp_context.lock().await;
                                *srtp_guard = Some(srtp_ctx);
                                
                                // Set handshake completed flag
                                let mut completed = handshake_completed.lock().await;
                                *completed = true;
                                
                                // Notify callbacks
                                let callback_guard = callbacks.lock().await;
                                for callback in callback_guard.iter() {
                                    callback(client_clone.clone());
                                }
                            },
                            Err(e) => {
                                error!("Failed to derive SRTP keys for client {}: {}", addr, e);
                            }
                        }
                    },
                    Err(e) => {
                        error!("DTLS handshake failed for client {}: {}", addr, e);
                    }
                }
            }
            Ok(())
        });
        
        Ok(client_arc as Arc<dyn ClientSecurityContext>)
    }
    
    async fn get_client_contexts(&self) -> Vec<Arc<dyn ClientSecurityContext>> {
        let clients = self.clients.read().await;
        clients.values()
            .map(|c| c.clone() as Arc<dyn ClientSecurityContext>)
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
    
    async fn on_client_secure(&self, callback: Box<dyn Fn(Arc<dyn ClientSecurityContext>) + Send + Sync>) -> Result<(), SecurityError> {
        let mut callbacks = self.client_secure_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
    }
    
    async fn get_supported_srtp_profiles(&self) -> Vec<SrtpProfile> {
        self.config.srtp_profiles.clone()
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
                })
                .map(|s| s.to_string())
                .collect(),
            key_params: None,
            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()),
        }
    }
} 