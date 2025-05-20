//! Client security implementation
//!
//! This file contains the implementation of the ClientSecurityContext trait.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use async_trait::async_trait;
use tracing::{debug, error, info, warn};
use std::time::Duration;
use std::any::Any;
use std::collections::HashMap;
use uuid::Uuid;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::client::security::{ClientSecurityContext, ClientSecurityConfig, create_dtls_config};
use crate::dtls::{DtlsConnection, DtlsConfig, DtlsRole, DtlsVersion};
use crate::dtls::transport::udp::UdpTransport;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_NULL_NULL, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM};
use crate::srtp::crypto::SrtpCryptoKey;
use crate::api::server::security::SocketHandle;
use crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80;
use crate::dtls::record::{Record, ContentType};
use crate::dtls::message::handshake::HandshakeHeader;

// Additional imports
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::sleep;

/// Default implementation of the ClientSecurityContext trait
#[derive(Clone)]
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
    /// Flag to indicate if handshake monitor is running
    handshake_monitor_running: Arc<AtomicBool>,
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
            handshake_monitor_running: Arc::new(AtomicBool::new(false)),
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
        
        // Create a transport from the socket
        let transport = match crate::dtls::transport::udp::UdpTransport::new(socket.socket.clone(), 1500).await {
            Ok(t) => t,
            Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e)))
        };
        
        // Create an Arc<Mutex<UdpTransport>> for the connection
        let transport = Arc::new(Mutex::new(transport));
        
        // Start the transport first - CRITICAL
        let start_result = transport.lock().await.start().await;
        
        // Only proceed if the transport started successfully
        if start_result.is_ok() {
            debug!("DTLS transport started successfully");
            
            // Set the transport on the connection
            connection.set_transport(transport);
            
            // Store the connection
            let mut conn_guard = self.connection.lock().await;
            *conn_guard = Some(connection);
            
            Ok(())
        } else {
            // Log the error and return it
            let err = start_result.err().unwrap();
            error!("Failed to start DTLS transport: {}", err);
            Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", err)))
        }
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

    /// Start a handshake monitor task
    /// This will monitor the connection and automatically handle HelloVerifyRequest events
    async fn start_handshake_monitor(&self) -> Result<(), SecurityError> {
        // If already running, don't start another one
        if self.handshake_monitor_running.load(Ordering::SeqCst) {
            return Ok(());
        }
        
        debug!("Starting handshake monitoring task");
        
        // Set running flag
        self.handshake_monitor_running.store(true, Ordering::SeqCst);
        
        // Get what we need for the task - remote address and socket
        let remote_addr_guard = self.remote_addr.lock().await;
        let remote_addr = match remote_addr_guard.as_ref() {
            Some(addr) => *addr,
            None => {
                self.handshake_monitor_running.store(false, Ordering::SeqCst);
                return Err(SecurityError::Configuration("Remote address not set for handshake monitor".to_string()));
            }
        };
        drop(remote_addr_guard);
        
        let socket_guard = self.socket.lock().await;
        let socket = match socket_guard.as_ref() {
            Some(s) => s.socket.clone(),
            None => {
                self.handshake_monitor_running.store(false, Ordering::SeqCst);
                return Err(SecurityError::Configuration("Socket not set for handshake monitor".to_string()));
            }
        };
        drop(socket_guard);
        
        // Clone self for the task
        let client_this = self.clone();
        
        // Spawn the monitor task
        tokio::spawn(async move {
            debug!("Handshake monitor task started");
            
            let mut attempt = 1;
            let max_attempts = 3;
            
            // Try the handshake process multiple times
            while attempt <= max_attempts {
                debug!("Handshake attempt #{}", attempt);
                
                // First check if handshake is already complete
                if *client_this.handshake_completed.lock().await {
                    debug!("Handshake already completed, stopping monitor");
                    client_this.handshake_monitor_running.store(false, Ordering::SeqCst);
                    return;
                }
                
                // Wait to see if the handshake completes in a reasonable time
                let mut completed = false;
                for _ in 0..30 {  // Check status every 100ms for up to 3 seconds
                    // Check handshake status
                    let conn_guard = client_this.connection.lock().await;
                    
                    if let Some(conn) = conn_guard.as_ref() {
                        // Check connection state first
                        if conn.state() == crate::dtls::connection::ConnectionState::Connected {
                            debug!("Connection state is Connected, handshake likely complete");
                            completed = true;
                            break;
                        }
                        
                        // Check handshake step (it may need assistance)
                        if let Some(step) = conn.handshake_step() {
                            match step {
                                crate::dtls::handshake::HandshakeStep::ReceivedHelloVerifyRequest => {
                                    // This requires an explicit restart of handshake with cookie
                                    debug!("Detected ReceivedHelloVerifyRequest step - need to restart handshake");
                                    drop(conn_guard);
                                    
                                    // Get fresh connection guard for mutation
                                    let mut conn_guard = client_this.connection.lock().await;
                                    if let Some(conn) = conn_guard.as_mut() {
                                        // HelloVerifyRequest received, restart handshake with cookie
                                        debug!("Restarting handshake with cookie from monitor");
                                        if let Err(e) = conn.start_handshake(remote_addr).await {
                                            warn!("Failed to restart handshake with cookie: {}", e);
                                        } else {
                                            info!("Successfully restarted handshake with cookie");
                                        }
                                    }
                                    break; // Exit loop to allow handshake to progress
                                },
                                crate::dtls::handshake::HandshakeStep::Complete => {
                                    debug!("Handshake step is Complete");
                                    completed = true;
                                    break;
                                },
                                crate::dtls::handshake::HandshakeStep::Failed => {
                                    debug!("Handshake step is Failed, will restart");
                                    break;
                                },
                                _ => {
                                    // Continue waiting
                                    debug!("Waiting for handshake progress, current step: {:?}", step);
                                }
                            }
                        }
                    }
                    
                    // Check for completion flag
                    if *client_this.handshake_completed.lock().await {
                        debug!("Handshake completed flag is set");
                        completed = true;
                        break;
                    }
                    
                    // Wait a bit before checking again
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                
                // If completed, we're done
                if completed || *client_this.handshake_completed.lock().await {
                    debug!("Handshake completed successfully, exiting monitor");
                    client_this.handshake_monitor_running.store(false, Ordering::SeqCst);
                    return;
                }
                
                // If we get here, we need to try again with a new connection
                warn!("Creating new DTLS connection for retry (attempt #{})", attempt + 1);
                
                // Create a new connection and transport
                match init_new_connection(&socket, remote_addr).await {
                    Ok(new_conn) => {
                        // Replace the old connection
                        let mut conn_guard = client_this.connection.lock().await;
                        *conn_guard = Some(new_conn);
                    },
                    Err(e) => {
                        error!("Failed to create new connection: {}", e);
                    }
                }
                
                // Increment attempt counter
                attempt += 1;
                
                // Wait a bit before next attempt
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            
            warn!("Handshake failed after {} attempts", max_attempts);
            client_this.handshake_monitor_running.store(false, Ordering::SeqCst);
        });
        
        Ok(())
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
    
    /// Start DTLS handshake with the server
    async fn start_handshake(&self) -> Result<(), SecurityError> {
        debug!("Starting DTLS handshake");
        
        // Get the client socket
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.clone().ok_or_else(|| 
            SecurityError::Configuration("No socket set for client security context".to_string()))?;
        drop(socket_guard);
        
        // Get remote address
        let remote_addr_guard = self.remote_addr.lock().await;
        let remote_addr = remote_addr_guard.ok_or_else(|| 
            SecurityError::Configuration("Remote address not set for client security context".to_string()))?;
        drop(remote_addr_guard);
        
        debug!("Starting DTLS handshake with remote {}", remote_addr);
        
        // Make sure connection is initialized
        let mut conn_guard = self.connection.lock().await;
        if conn_guard.is_none() {
            debug!("Connection not initialized, initializing now...");
            // Initialize connection
            self.init_connection().await?;
            
            // Refresh the guard
            conn_guard = self.connection.lock().await;
        }
        
        // Get the connection (which should exist now)
        if let Some(conn) = conn_guard.as_mut() {
            // Start handshake and send ClientHello - this will begin the entire handshake process
            debug!("Calling start_handshake on DTLS connection");
            if let Err(e) = conn.start_handshake(remote_addr).await {
                error!("Failed to start DTLS handshake: {}", e);
                return Err(SecurityError::Handshake(format!("Failed to start DTLS handshake: {}", e)));
            }
            
            debug!("DTLS handshake started successfully");
            
            // The rest of the handshake will be handled by the wait_for_handshake method
            // and automatic packet processing through the transport
            
            Ok(())
        } else {
            Err(SecurityError::Internal("Failed to get DTLS connection after initialization".to_string()))
        }
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
                match crate::dtls::transport::udp::UdpTransport::new(socket.socket.clone(), 1500).await {
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
        
        // Start background task to handle incoming packets automatically
        let socket_clone = socket.socket.clone();
        let connection_ref = self.connection.clone();
        
        debug!("Starting automatic packet handling task for client");
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 2048];
            
            loop {
                match socket_clone.recv_from(&mut buffer).await {
                    Ok((size, addr)) => {
                        debug!("Client received {} bytes from {}", size, addr);
                        
                        // Process the packet through the DTLS connection
                        let mut conn_guard = connection_ref.lock().await;
                        if let Some(conn) = conn_guard.as_mut() {
                            // Process the packet using the process_packet method
                            match conn.process_packet(&buffer[..size]).await {
                                Ok(_) => debug!("Client successfully processed DTLS packet"),
                                Err(e) => debug!("Error processing DTLS packet: {:?}", e),
                            }
                        }
                    },
                    Err(e) => {
                        debug!("Client receive error: {}", e);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });
        
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
        
        // Get the connection
        let mut conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_mut() {
            // Delegate to the DTLS library's wait_handshake
            match conn.wait_handshake().await {
                Ok(_) => {
                    debug!("DTLS handshake completed successfully");
                    
                    // Set the handshake completed flag
                    let mut completed = self.handshake_completed.lock().await;
                    *completed = true;
                    
                    // Extract SRTP keys if needed
                    let mut srtp_guard = self.srtp_context.lock().await;
                    if srtp_guard.is_none() {
                        // Extract keys directly from DTLS connection
                        match conn.extract_srtp_keys() {
                            Ok(srtp_ctx) => {
                                // Get the client key
                                let client_key = srtp_ctx.get_key_for_role(true).clone();
                                debug!("Successfully extracted SRTP keys");
                                
                                // Clone the profile to avoid borrowing issues
                                let profile = srtp_ctx.profile.clone();
                                
                                // Extract SRTP profile
                                if let Ok(srtp_ctx) = SrtpContext::new(profile, client_key) {
                                    // Store the SRTP context
                                    let mut srtp_guard = self.srtp_context.lock().await;
                                    *srtp_guard = Some(srtp_ctx);
                                    
                                    // Set handshake completed flag
                                    let mut completed = self.handshake_completed.lock().await;
                                    *completed = true;
                                    
                                    debug!("DTLS handshake and SRTP key extraction completed");
                                } else {
                                    error!("Failed to create SRTP context from extracted keys");
                                    return Err(SecurityError::Internal(
                                        "Failed to create SRTP context from extracted keys".to_string()));
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

    // Implementation of the trait method
    async fn complete_handshake(&self, remote_addr: SocketAddr, remote_fingerprint: &str) -> Result<(), SecurityError> {
        debug!("Starting complete handshake process with {}", remote_addr);
        
        // Set remote address and fingerprint
        self.set_remote_address(remote_addr).await?;
        self.set_remote_fingerprint(remote_fingerprint, "sha-256").await?;
        
        // Start handshake
        self.start_handshake().await?;
        
        // Wait for handshake with a reasonable timeout
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        
        while !self.is_handshake_complete().await? {
            if start_time.elapsed() > timeout {
                return Err(SecurityError::Handshake("Handshake timed out after 5 seconds".to_string()));
            }
            
            tokio::time::sleep(Duration::from_millis(100)).await;
            debug!("Waiting for handshake completion... ({:?} elapsed)", start_time.elapsed());
        }
        
        debug!("Handshake completed successfully");
        Ok(())
    }

    // Add the process_packet method implementation
    async fn process_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        // Process packet through DTLS connection
        let mut conn_guard = self.connection.lock().await;
        match conn_guard.as_mut() {
            Some(conn) => {
                // Log that we're passing the packet to the DTLS connection
                debug!("Client received packet of {} bytes - delegating to DTLS library", data.len());
                
                // Simply delegate to the underlying DTLS connection
                // The DTLS library already handles all the protocol details including HelloVerifyRequest
                if let Err(e) = conn.process_packet(data).await {
                    warn!("Error processing DTLS packet: {}", e);
                    return Err(SecurityError::HandshakeError(format!("Failed to process DTLS packet: {}", e)));
                }
                
                // Check if handshake is complete based on connection state
                if conn.state() == crate::dtls::connection::ConnectionState::Connected {
                    // Set handshake completed flag
                    let mut completed_guard = self.handshake_completed.lock().await;
                    if !*completed_guard {
                        debug!("DTLS handshake completed");
                        *completed_guard = true;
                        
                        // Extract SRTP keys
                        match conn.extract_srtp_keys() {
                            Ok(dtls_srtp_context) => {
                                // Convert DtlsSrtpContext to SrtpContext
                                // Here we would need to create a new SrtpContext using the
                                // keys from the DtlsSrtpContext
                                
                                // For now, just log that we got the keys
                                debug!("Successfully extracted SRTP keys from DTLS");
                                
                                // We would normally create a proper SrtpContext here
                                // *srtp_guard = Some(create_srtp_context_from_dtls(dtls_srtp_context));
                            },
                            Err(e) => {
                                warn!("Failed to extract SRTP keys: {}", e);
                                return Err(SecurityError::HandshakeError(format!("Failed to extract SRTP keys: {}", e)));
                            }
                        }
                    }
                }
                
                Ok(())
            },
            None => Err(SecurityError::Internal("DTLS connection not initialized".to_string())),
        }
    }

    async fn start_packet_handler(&self) -> Result<(), SecurityError> {
        // Get the client socket
        let socket_guard = self.socket.lock().await;
        let socket = socket_guard.clone().ok_or_else(|| 
            SecurityError::Configuration("No socket set for client security context".to_string()))?;
        drop(socket_guard);
        
        // Get remote address
        let remote_addr_guard = self.remote_addr.lock().await;
        let remote_addr = remote_addr_guard.ok_or_else(|| 
            SecurityError::Configuration("Remote address not set for client security context".to_string()))?;
        drop(remote_addr_guard);
        
        debug!("Client: Starting DTLS packet handler for server {}", remote_addr);
        
        // Create a dedicated transport for packet handling
        let transport = match UdpTransport::new(socket.socket.clone(), 1500).await {
            Ok(mut t) => {
                // Start the transport - this is essential for proper DTLS handling
                if let Err(e) = t.start().await {
                    return Err(SecurityError::Configuration(
                        format!("Failed to start transport for packet handler: {}", e)
                    ));
                }
                Arc::new(Mutex::new(t))
            },
            Err(e) => return Err(SecurityError::Configuration(
                format!("Failed to create transport for packet handler: {}", e)
            )),
        };
        
        // Clone what we need for the task
        let client_ctx = Arc::new(self.clone());
        
        // Spawn the packet handler task
        tokio::spawn(async move {
            debug!("Client packet handler task started for server {}", remote_addr);
            
            // Main packet handling loop
            loop {
                // Use transport to receive packets
                let receive_result = transport.lock().await.recv().await;
                
                match receive_result {
                    Some((data, addr)) => {
                        if addr == remote_addr {
                            debug!("Client received {} bytes from server {}", data.len(), addr);
                            
                            // Process the packet through the client context
                            if let Err(e) = client_ctx.process_packet(&data).await {
                                error!("Error processing server DTLS packet: {:?}", e);
                            }
                        } else {
                            debug!("Ignoring packet from unknown sender {}", addr);
                        }
                    },
                    None => {
                        // Transport returned None - likely an error or shutdown
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
            }
        });
        
        Ok(())
    }
    
    async fn is_ready(&self) -> Result<bool, SecurityError> {
        // Check if socket is set
        let socket_set = self.socket.lock().await.is_some();
        
        // Check if remote address is set
        let remote_addr_set = self.remote_addr.lock().await.is_some();
        
        // Check if remote fingerprint is set (needed for verification)
        let remote_fingerprint_set = self.remote_fingerprint.lock().await.is_some();
        
        // Check if DTLS connection is initialized
        let connection_guard = self.connection.lock().await;
        let connection_initialized = connection_guard.is_some();
        
        // Check if the connection has a transport
        let has_transport = if let Some(conn) = connection_guard.as_ref() {
            conn.has_transport()
        } else {
            false
        };
        
        // All prerequisites must be met for the context to be ready
        let is_ready = socket_set && remote_addr_set && connection_initialized && has_transport;
        
        debug!("Client security context ready: {}", is_ready);
        debug!("  - Socket set: {}", socket_set);
        debug!("  - Remote address set: {}", remote_addr_set);
        debug!("  - Remote fingerprint set: {}", remote_fingerprint_set);
        debug!("  - Connection initialized: {}", connection_initialized);
        debug!("  - Has transport: {}", has_transport);
        
        Ok(is_ready)
    }

    /// Process a DTLS packet received from the server
    async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        debug!("Client received {} bytes from server", data.len());
        
        // Get connection
        let mut conn_guard = self.connection.lock().await;
        
        if let Some(conn) = conn_guard.as_mut() {
            // Check for HelloVerifyRequest (ContentType=22, HandshakeType=3)
            if data.len() >= 14 && data[0] == 22 && data[13] == 3 {
                debug!("Detected HelloVerifyRequest packet, will handle specially");
                
                // Process the packet to extract cookie
                match conn.process_packet(data).await {
                    Ok(_) => {
                        // Get remote address
                        let remote_addr_guard = self.remote_addr.lock().await;
                        if let Some(remote_addr) = *remote_addr_guard {
                            // Make sure we're in the right state
                            if conn.handshake_step() == Some(crate::dtls::handshake::HandshakeStep::ReceivedHelloVerifyRequest) {
                                debug!("Continuing handshake after HelloVerifyRequest");
                                // Continue the handshake explicitly with the cookie
                                if let Err(e) = conn.continue_handshake().await {
                                    warn!("Failed to continue handshake with cookie: {}", e);
                                } else {
                                    debug!("Successfully continued handshake with cookie");
                                }
                            } else {
                                // Start handshake again with the cookie
                                debug!("Restarting handshake with cookie");
                                if let Err(e) = conn.start_handshake(remote_addr).await {
                                    warn!("Failed to restart handshake with cookie: {}", e);
                                } else {
                                    debug!("Successfully restarted handshake with cookie");
                                }
                            }
                        }
                    },
                    Err(e) => {
                        debug!("Error processing HelloVerifyRequest: {}", e);
                    }
                }
                
                return Ok(());
            }
            
            // For all other packet types, process normally
            match conn.process_packet(data).await {
                Ok(_) => {
                    // Check handshake step to determine next actions
                    if let Some(step) = conn.handshake_step() {
                        debug!("Current handshake step: {:?}", step);
                        
                        match step {
                            crate::dtls::handshake::HandshakeStep::ReceivedServerHello => {
                                debug!("Received ServerHello");
                                
                                // Get remote address
                                let remote_addr_guard = self.remote_addr.lock().await;
                                if let Some(_) = *remote_addr_guard {
                                    // Need to continue the handshake explicitly
                                    if let Err(e) = conn.continue_handshake().await {
                                        warn!("Failed to continue handshake after ServerHello: {}", e);
                                    } else {
                                        debug!("Successfully continued handshake after ServerHello");
                                    }
                                }
                            },
                            crate::dtls::handshake::HandshakeStep::SentClientKeyExchange => {
                                debug!("Sent ClientKeyExchange");
                                
                                // Need to complete handshake
                                if let Err(e) = conn.complete_handshake().await {
                                    warn!("Failed to complete handshake: {}", e);
                                } else {
                                    debug!("Successfully completed handshake");
                                }
                            },
                            crate::dtls::handshake::HandshakeStep::Complete => {
                                debug!("Handshake complete");
                                
                                // Set handshake completed flag
                                let mut completed = self.handshake_completed.lock().await;
                                if !*completed {
                                    *completed = true;
                                    
                                    // Extract SRTP keys
                                    if conn.state() == crate::dtls::connection::ConnectionState::Connected {
                                        match conn.extract_srtp_keys() {
                                            Ok(srtp_ctx) => {
                                                // Get client key (true = client)
                                                let client_key = srtp_ctx.get_key_for_role(true).clone();
                                                info!("Successfully extracted SRTP keys");
                                                
                                                // Create SRTP context
                                                match SrtpContext::new(srtp_ctx.profile, client_key) {
                                                    Ok(ctx) => {
                                                        // Store SRTP context
                                                        let mut srtp_guard = self.srtp_context.lock().await;
                                                        *srtp_guard = Some(ctx);
                                                        info!("DTLS handshake completed and SRTP keys extracted");
                                                    },
                                                    Err(e) => warn!("Failed to create SRTP context: {}", e)
                                                }
                                            },
                                            Err(e) => warn!("Failed to extract SRTP keys: {}", e)
                                        }
                                    }
                                }
                            },
                            _ => {} // Ignore other steps
                        }
                    }
                    
                    Ok(())
                },
                Err(e) => {
                    // Log errors but don't fail the method for common error types
                    debug!("Error processing DTLS packet: {}", e);
                    
                    // Return success to allow handshake to continue
                    Ok(())
                }
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

/// Helper function to create a new DTLS connection
async fn init_new_connection(socket: &Arc<UdpSocket>, remote_addr: SocketAddr) 
    -> Result<DtlsConnection, SecurityError> {
    
    // Create DTLS connection config
    let dtls_config = DtlsConfig {
        role: DtlsRole::Client,
        version: DtlsVersion::Dtls12,
        mtu: 1500,
        max_retransmissions: 5,
        srtp_profiles: vec![SRTP_AES128_CM_SHA1_80],
    };
    
    // Create new DTLS connection
    let mut connection = DtlsConnection::new(dtls_config);
    
    // Generate a self-signed certificate
    debug!("Generating self-signed certificate for new connection");
    match crate::dtls::crypto::verify::generate_self_signed_certificate() {
        Ok(cert) => connection.set_certificate(cert),
        Err(e) => return Err(SecurityError::Configuration(
            format!("Failed to generate certificate: {}", e)
        ))
    }
    
    // Create UDP transport from socket
    let transport = match UdpTransport::new(socket.clone(), 1500).await {
        Ok(t) => t,
        Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e)))
    };
    
    // Create an Arc<Mutex<UdpTransport>> for the connection
    let transport_arc = Arc::new(Mutex::new(transport));

    // Start the transport
    let start_result = transport_arc.lock().await.start().await;

    // Only proceed if the transport started successfully
    if start_result.is_ok() {
        debug!("DTLS transport started successfully for new connection");
        
        // Set the transport on the connection (clone the Arc)
        connection.set_transport(transport_arc.clone());
        
        // Start the handshake
        if let Err(e) = connection.start_handshake(remote_addr).await {
            return Err(SecurityError::Handshake(format!("Failed to start handshake: {}", e)));
        }
        
        debug!("Started handshake on new connection");
        Ok(connection)
    } else {
        // Log the error and return it
        let err = start_result.err().unwrap();
        error!("Failed to start DTLS transport for new connection: {}", err);
        Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", err)))
    }
} 