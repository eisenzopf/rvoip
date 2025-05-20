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
use crate::srtp::{SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_NULL_NULL, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM};
use crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80;

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
    /// Transport used for DTLS
    transport: Arc<Mutex<Option<Arc<Mutex<crate::dtls::transport::udp::UdpTransport>>>>>,
    /// Flag indicating that handshake is waiting for first packet
    waiting_for_first_packet: Arc<Mutex<bool>>,
    /// Initial packet from client (if received)
    initial_packet: Arc<Mutex<Option<Vec<u8>>>>,
}

impl DefaultClientSecurityContext {
    /// Process a DTLS packet received from the client - implementation
    async fn process_dtls_packet_impl(&self, data: &[u8]) -> Result<(), SecurityError> {
        // Check if we're waiting for the first packet
        let is_waiting = {
            let waiting_guard = self.waiting_for_first_packet.lock().await;
            *waiting_guard
        };
        
        if is_waiting {
            debug!("Received first DTLS packet from client {}, initializing handshake", self.address);
            
            // Store the initial packet
            {
                let mut packet_guard = self.initial_packet.lock().await;
                *packet_guard = Some(data.to_vec());
            }
            
            // Create a fresh DTLS connection
            let mut conn_guard = self.connection.lock().await;
            
            // Create a new DTLS config with server role
            let dtls_config = DtlsConfig {
                role: DtlsRole::Server,
                version: crate::dtls::DtlsVersion::Dtls12,
                mtu: 1200,
                max_retransmissions: 5,
                srtp_profiles: Self::convert_profiles(&self.config.srtp_profiles),
            };
            
            // Create a new DtlsConnection
            let mut new_conn = DtlsConnection::new(dtls_config);
            
            // Create a certificate if needed
            let certificate = crate::dtls::crypto::verify::generate_self_signed_certificate()
                .map_err(|e| SecurityError::Configuration(format!("Failed to generate certificate: {}", e)))?;
            
            // Set the certificate
            new_conn.set_certificate(certificate);
            
            // Set the transport
            let transport = {
                let transport_guard = self.transport.lock().await;
                transport_guard.as_ref().ok_or_else(|| 
                    SecurityError::Configuration("No transport available".to_string())
                )?.clone()
            };
            new_conn.set_transport(transport);
            
            // Start the handshake
            if let Err(e) = new_conn.start_handshake(self.address).await {
                return Err(SecurityError::Handshake(
                    format!("Failed to start DTLS handshake: {}", e)
                ));
            }
            
            // Process the initial packet
            if let Err(e) = new_conn.process_packet(data).await {
                return Err(SecurityError::Handshake(
                    format!("Failed to process initial packet: {}", e)
                ));
            }
            
            // Store the connection
            *conn_guard = Some(new_conn);
            
            // Clear waiting flag
            {
                let mut waiting_guard = self.waiting_for_first_packet.lock().await;
                *waiting_guard = false;
            }
            
            // Spawn handshake completion task
            self.spawn_handshake_task_impl().await?;
            
            debug!("DTLS handshake initialized for client {}", self.address);
        } else {
            // Normal packet processing - pass to connection
            let mut conn_guard = self.connection.lock().await;
            if let Some(conn) = conn_guard.as_mut() {
                if let Err(e) = conn.process_packet(data).await {
                    return Err(SecurityError::Handshake(
                        format!("Failed to process DTLS packet: {}", e)
                    ));
                }
            } else {
                return Err(SecurityError::NotInitialized(
                    "DTLS connection not initialized".to_string()
                ));
            }
        }
        
        Ok(())
    }
    
    /// Spawn a task to wait for handshake completion
    async fn spawn_handshake_task_impl(&self) -> Result<(), SecurityError> {
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
                                    crate::srtp::SrtpCryptoSuite { authentication: HmacSha1_80, .. } => {
                                        SRTP_AES128_CM_SHA1_80
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
    
    /// Convert API SrtpProfile to SrtpCryptoSuite
    fn convert_profiles(profiles: &[SrtpProfile]) -> Vec<SrtpCryptoSuite> {
        profiles.iter()
            .filter_map(|p| {
                match p {
                    SrtpProfile::AesCm128HmacSha1_80 => Some(SRTP_AES128_CM_SHA1_80),
                    SrtpProfile::AesCm128HmacSha1_32 => Some(SRTP_AES128_CM_SHA1_32),
                    SrtpProfile::AesGcm128 => Some(SRTP_AEAD_AES_128_GCM),
                    SrtpProfile::AesGcm256 => Some(SRTP_AEAD_AES_256_GCM),
                    _ => None,
                }
            })
            .collect()
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
        // Verify we have SRTP profiles configured
        if config.srtp_profiles.is_empty() {
            return Err(SecurityError::Configuration("No SRTP profiles specified in server config".to_string()));
        }

        // Create DTLS connection template for the certificate
        let mut conn_config = ConnectionConfig::default();
        conn_config.role = ConnectionRole::Server; // Server is passive
        
        // Create DTLS connection template for the certificate
        let mut dtls_config = DtlsConfig {
            role: match conn_config.role {
                ConnectionRole::Client => DtlsRole::Client,
                ConnectionRole::Server => DtlsRole::Server,
            },
            version: crate::dtls::DtlsVersion::Dtls12,
            mtu: 1200,
            max_retransmissions: 5,
            srtp_profiles: Self::convert_profiles(&config.srtp_profiles),
        };

        // Create DTLS connection template
        let mut connection = DtlsConnection::new(dtls_config);

        // Generate or load certificate based on config
        let cert = if let (Some(cert_path), Some(key_path)) = (&config.certificate_path, &config.private_key_path) {
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
            if let Some(cert) = template.local_certificate() {
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
            Err(SecurityError::Configuration("DTLS connection template not initialized".to_string()))
        }
    }

    /// Get the fingerprint algorithm from the template
    async fn get_fingerprint_algorithm_from_template(&self) -> Result<String, SecurityError> {
        // We hardcode this for now since the algorithm is set during certificate creation
        Ok("sha-256".to_string())
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

        // Verify we have SRTP profiles configured
        if self.config.srtp_profiles.is_empty() {
            return Err(SecurityError::Configuration("No SRTP profiles specified for client context".to_string()));
        }
        
        // Get socket
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.clone().ok_or_else(|| 
            SecurityError::Configuration("No socket set for server security context".to_string()))?;
        drop(socket_guard);
        
        // Create socket with remote address set
        let mut client_socket = socket.clone();
        client_socket.remote_addr = Some(addr);

        // Create UDP transport
        let transport = match crate::dtls::transport::udp::UdpTransport::new(
            client_socket.socket.clone(), 1200
        ).await {
            Ok(t) => t,
            Err(e) => return Err(SecurityError::Configuration(
                format!("Failed to create DTLS transport: {}", e)
            ))
        };
        
        // Wrap transport in Arc<Mutex>
        let transport_arc = Arc::new(Mutex::new(transport));
        
        // Start the transport
        if let Err(e) = transport_arc.lock().await.start().await {
            return Err(SecurityError::Configuration(
                format!("Failed to start DTLS transport: {}", e)
            ));
        }
        
        debug!("DTLS transport started for client {}", addr);

        // Create client context with initialized transport
        let client = DefaultClientSecurityContext {
            address: addr,
            connection: Arc::new(Mutex::new(None)),
            srtp_context: Arc::new(Mutex::new(None)),
            handshake_completed: Arc::new(Mutex::new(false)),
            socket: Arc::new(Mutex::new(Some(client_socket))),
            config: self.config.clone(),
            transport: Arc::new(Mutex::new(Some(transport_arc))),
            waiting_for_first_packet: Arc::new(Mutex::new(true)),
            initial_packet: Arc::new(Mutex::new(None)),
        };
        
        // Wrap in Arc
        let client_arc = Arc::new(client);
        
        // Store in clients map
        let mut clients = self.clients.write().await;
        clients.insert(addr, client_arc.clone());
        
        debug!("Created client security context for {} - waiting for first packet", addr);
        
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

    async fn process_client_packet(&self, addr: SocketAddr, data: &[u8]) -> Result<(), SecurityError> {
        // Find the client context
        let clients = self.clients.read().await;
        if let Some(client) = clients.get(&addr) {
            // Process the packet with the client context
            client.process_dtls_packet(data).await
        } else {
            // Create a new client context if we don't have one
            drop(clients);
            let client = self.create_client_context(addr).await?;
            
            // Process the packet with the new client context
            client.process_dtls_packet(data).await
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
                socket.socket.clone(), 1200
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
            
            // Set waiting for first packet flag
            let mut waiting_guard = self.waiting_for_first_packet.lock().await;
            *waiting_guard = true;
            debug!("Client {} is now waiting for first packet", self.address);
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
        let mut conn = self.connection.lock().await;
        if let Some(conn) = conn.as_mut() {
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
        // Call our implementation
        self.process_dtls_packet_impl(data).await
    }
} 