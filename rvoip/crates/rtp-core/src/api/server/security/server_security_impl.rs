//! DEPRECATED: Server security implementation
//!
//! WARNING: This file is being phased out as part of a refactoring effort.
//! The code is being moved to smaller, more maintainable modules.
//! This file will be removed in a future version. Please use the refactored modules instead.
//!
//! See the directory structure under src/api/server/security/ for the new organization.

// Original implementation follows:
//! Server security implementation
//!
//! This file contains the implementation of the ServerSecurityContext trait.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::any::Any;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, RwLock};
use async_trait::async_trait;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use std::time::Duration;

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
use crate::dtls::transport::udp::UdpTransport;

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
    /// Process a DTLS packet received from the client
    async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
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

    /// Start a handshake with the remote
    async fn start_handshake_with_remote(&self, remote_addr: SocketAddr) -> Result<(), SecurityError> {
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

/// Default server security context implementation
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
            mtu: 1500,
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

    /// Capture the first packet from a client for proper handshake sequence
    pub async fn capture_initial_packet(&self) -> Result<Option<(Vec<u8>, SocketAddr)>, SecurityError> {
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.as_ref().ok_or_else(||
            SecurityError::NotInitialized("No socket set for server security context".to_string()))?;
        
        // Create a buffer to receive packet
        let mut buffer = vec![0u8; 2048];
        
        // Set a short timeout to avoid blocking indefinitely
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            socket.socket.recv_from(&mut buffer)
        ).await {
            Ok(Ok((size, addr))) => {
                println!("Captured initial packet: {} bytes from {}", size, addr);
                Ok(Some((buffer[..size].to_vec(), addr)))
            },
            Ok(Err(e)) => {
                Err(SecurityError::HandshakeError(format!("Error receiving initial packet: {}", e)))
            },
            Err(_) => {
                // Timeout occurred
                Ok(None)
            }
        }
    }

    /// Reset and restart the DTLS handshake for a client
    async fn restart_client_handshake(&self, addr: SocketAddr) -> Result<(), SecurityError> {
        // Get the client context if it exists
        let client_ctx = {
            let clients = self.clients.read().await;
            clients.get(&addr).cloned()
        };
        
        if let Some(client_ctx) = client_ctx {
            debug!("Restarting handshake for client {}", addr);
            
            // Get the socket for the client
            let socket = match client_ctx.socket.lock().await.clone() {
                Some(s) => s,
                None => return Err(SecurityError::NotInitialized("No socket set for client context".to_string())),
            };
            
            // Create a completely new connection
            let config = self.config.clone();
            
            // Create DTLS connection with server role
            let dtls_config = DtlsConfig {
                role: DtlsRole::Server,
                version: crate::dtls::DtlsVersion::Dtls12,
                mtu: 1500,
                max_retransmissions: 5,
                srtp_profiles: Self::convert_profiles(&config.srtp_profiles),
            };
            let mut connection = DtlsConnection::new(dtls_config);
            
            // Set certificate
            let cert = match self.connection_template.lock().await.as_ref() {
                Some(conn) => match conn.local_certificate() {
                    Some(cert) => cert.clone(),
                    None => return Err(SecurityError::Configuration("No certificate in template".to_string())),
                },
                None => return Err(SecurityError::Configuration("No template connection".to_string())),
            };
            connection.set_certificate(cert);
            
            // Create a new transport
            let transport = match UdpTransport::new(socket.socket.clone(), 1500).await {
                Ok(mut t) => {
                    // Start the transport (CRUCIAL)
                    if let Err(e) = t.start().await {
                        return Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e)));
                    }
                    t
                },
                Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e))),
            };
            
            // Wrap transport in Arc<Mutex<>>
            let transport_arc = Arc::new(Mutex::new(transport));
            
            // Set transport on the connection
            connection.set_transport(transport_arc.clone());
            
            // Update the client's connection
            {
                let mut conn_guard = client_ctx.connection.lock().await;
                *conn_guard = Some(connection);
            }
            
            // Reset the handshake completed flag
            {
                let mut completed = client_ctx.handshake_completed.lock().await;
                *completed = false;
            }
            
            // Reset transport
            {
                let mut transport_guard = client_ctx.transport.lock().await;
                *transport_guard = Some(transport_arc);
            }
            
            // Start the handshake
            client_ctx.start_handshake_with_remote(addr).await?;
            
            debug!("Successfully restarted handshake for client {}", addr);
            Ok(())
        } else {
            Err(SecurityError::Configuration(format!("Client {} not found", addr)))
        }
    }
}

#[async_trait]
impl ServerSecurityContext for DefaultServerSecurityContext {
    async fn initialize(&self) -> Result<(), SecurityError> {
        debug!("Initializing server security context");
        
        // Verify that the connection template is initialized
        self.get_connection_template().await?;
        
        // Nothing more to do for initialization - each client connection
        // will be initialized individually on first contact
        Ok(())
    }
    
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
        let dtls_config = DtlsConfig {
            role: DtlsRole::Server,
            version: crate::dtls::DtlsVersion::Dtls12,
            mtu: 1500,
            max_retransmissions: 5,
            srtp_profiles: Self::convert_profiles(&self.config.srtp_profiles),
        };
        let mut connection = DtlsConnection::new(dtls_config);
        
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
        let transport = match UdpTransport::new(socket.socket.clone(), 1500).await {
            Ok(mut t) => {
                // Start the transport (CRUCIAL)
                if let Err(e) = t.start().await {
                    return Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e)));
                }
                t
            },
            Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e))),
        };
        
        // Wrap transport in Arc<Mutex<>>
        let transport_arc = Arc::new(Mutex::new(transport));
        debug!("DTLS transport started for client {}", addr);
        
        // Set transport on the connection (clone the Arc)
        connection.set_transport(transport_arc.clone());
        
        // Create client context
        let client_ctx = Arc::new(DefaultClientSecurityContext {
            address: addr,
            connection: Arc::new(Mutex::new(Some(connection))),
            srtp_context: Arc::new(Mutex::new(None)),
            handshake_completed: Arc::new(Mutex::new(false)),
            socket: Arc::new(Mutex::new(Some(socket.clone()))),
            config: self.config.clone(),
            transport: Arc::new(Mutex::new(Some(transport_arc))),
            initial_packet: Arc::new(Mutex::new(None)),
            waiting_for_first_packet: Arc::new(Mutex::new(false)),
        });
        
        // Start a task to monitor the handshake
        let client_ctx_clone = client_ctx.clone();
        tokio::spawn(async move {
            debug!("Starting handshake monitor task for client {}", addr);
            
            // Wait for handshake to complete (with timeout)
            match tokio::time::timeout(
                Duration::from_secs(10),
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
        
        // Clone what we need for the task
        let server_ctx = Arc::new(self.clone());
        let socket_clone = socket.socket.clone();
        
        debug!("Starting automatic DTLS packet handler task for server");
        
        // Spawn the packet handler task
        tokio::spawn(async move {
            // Create a transport for packet reception - this is critical for proper DTLS handling
            let transport = match UdpTransport::new(socket_clone.clone(), 1500).await {
                Ok(mut t) => {
                    // Start the transport - this is essential
                    if let Err(e) = t.start().await {
                        error!("Failed to start server transport for packet handler: {}", e);
                        return;
                    }
                    debug!("Server packet handler transport started successfully");
                    Arc::new(Mutex::new(t))
                },
                Err(e) => {
                    error!("Failed to create server transport for packet handler: {}", e);
                    return;
                }
            };
            
            // Main packet handling loop
            loop {
                // Use the transport to receive packets
                let receive_result = transport.lock().await.recv().await;
                
                match receive_result {
                    Some((data, addr)) => {
                        debug!("Server received {} bytes from {}", data.len(), addr);
                        
                        // Process the packet through the server context
                        match server_ctx.process_client_packet(addr, &data).await {
                            Ok(_) => debug!("Server successfully processed client DTLS packet"),
                            Err(e) => debug!("Error processing client DTLS packet: {:?}", e),
                        }
                    },
                    None => {
                        // Transport returned None - likely an error or shutdown
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });
        
        Ok(())
    }

    async fn capture_initial_packet(&self) -> Result<Option<(Vec<u8>, SocketAddr)>, SecurityError> {
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.as_ref().ok_or_else(||
            SecurityError::NotInitialized("No socket set for server security context".to_string()))?;
        
        // Create a buffer to receive packet
        let mut buffer = vec![0u8; 2048];
        
        // Set a short timeout to avoid blocking indefinitely
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            socket.socket.recv_from(&mut buffer)
        ).await {
            Ok(Ok((size, addr))) => {
                println!("Captured initial packet: {} bytes from {}", size, addr);
                Ok(Some((buffer[..size].to_vec(), addr)))
            },
            Ok(Err(e)) => {
                Err(SecurityError::HandshakeError(format!("Error receiving initial packet: {}", e)))
            },
            Err(_) => {
                // Timeout occurred
                Ok(None)
            }
        }
    }

    async fn is_ready(&self) -> Result<bool, SecurityError> {
        // Check if socket is set
        let socket_set = self.socket.lock().await.is_some();
        
        // Check if template connection is initialized (needed for certificate)
        let connection_initialized = self.connection_template.lock().await.is_some();
        
        // Check if template certificate is initialized
        let certificate_initialized = self.get_fingerprint_from_template().await.is_ok();
        
        // All prerequisites must be met for the context to be ready
        let is_ready = socket_set && connection_initialized && certificate_initialized;
        
        debug!("Server security context ready: {}", is_ready);
        debug!("  - Socket set: {}", socket_set);
        debug!("  - Connection initialized: {}", connection_initialized);
        debug!("  - Certificate initialized: {}", certificate_initialized);
        
        Ok(is_ready)
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

    /// Start a handshake with the remote
    async fn start_handshake_with_remote(&self, remote_addr: SocketAddr) -> Result<(), SecurityError> {
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

    /// Allow downcasting for internal implementation details
    fn as_any(&self) -> &dyn Any {
        self
    }
} 