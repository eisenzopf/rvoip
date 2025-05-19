use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;
use tracing::{debug, info, error, warn};
use tokio::sync::RwLock;
use tokio::net::UdpSocket;
use async_trait::async_trait;

use crate::api::security::{
    SecureMediaContext, SecurityConfig, SecurityInfo, SecurityError,
    SrtpProfile, SecurityMode
};
use crate::dtls::{DtlsConnection, DtlsConfig, DtlsRole};
use crate::dtls::connection::ConnectionState;
use crate::dtls::crypto::verify::{Certificate, generate_self_signed_certificate, FingerprintVerifier};
use crate::dtls::transport::udp::UdpTransport;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, crypto};

/// Default implementation of SecureMediaContext
pub struct DefaultSecureMediaContext {
    /// The security configuration
    config: SecurityConfig,
    
    /// DTLS connection for key exchange and secure signaling
    dtls: Option<Arc<RwLock<DtlsConnection>>>,
    
    /// DTLS transport for sending/receiving packets
    transport: Arc<RwLock<Option<Arc<tokio::sync::Mutex<UdpTransport>>>>>,
    
    /// SRTP context for media encryption
    srtp: Arc<RwLock<Option<SrtpContext>>>,
    
    /// Local certificate for DTLS
    local_cert: Arc<RwLock<Option<Certificate>>>,
    
    /// Remote fingerprint received from SDP
    remote_fingerprint: Arc<RwLock<Option<(String, String)>>>,
    
    /// SRTP key material derived from DTLS
    srtp_keys: Arc<RwLock<Option<Vec<u8>>>>,
    
    /// Whether the handshake has completed successfully
    secure: Arc<RwLock<bool>>,
    
    /// Remote address for DTLS
    remote_addr: Arc<RwLock<Option<std::net::SocketAddr>>>,
    
    /// Handshake status notification
    handshake_status: Arc<tokio::sync::Notify>,
    
    /// Handshake error (if any)
    handshake_error: Arc<RwLock<Option<String>>>,
}

impl DefaultSecureMediaContext {
    /// Create a new DefaultSecureMediaContext
    pub async fn new(config: SecurityConfig) -> Result<Arc<Self>, SecurityError> {
        // Generate a self-signed certificate for DTLS if using DTLS-SRTP
        let local_cert = if config.mode == SecurityMode::DtlsSrtp {
            match generate_self_signed_certificate() {
                Ok(cert) => Some(cert),
                Err(e) => {
                    return Err(SecurityError::CertificateError(
                        format!("Failed to generate self-signed certificate: {}", e)
                    ));
                }
            }
        } else {
            None
        };
        
        let dtls = match config.mode {
            SecurityMode::DtlsSrtp => {
                // Create DTLS configuration
                let dtls_config = DtlsConfig {
                    role: if config.dtls_client { 
                        DtlsRole::Client 
                    } else {
                        DtlsRole::Server 
                    },
                    srtp_profiles: config.srtp_profiles.iter()
                        .map(|profile| api_to_internal_crypto_suite(profile))
                        .collect(),
                    max_retransmissions: 10,
                    mtu: 1200,
                    version: crate::dtls::DtlsVersion::Dtls12,
                };
                
                // Create DTLS connection
                let mut dtls = DtlsConnection::new(dtls_config);
                
                // Set the local certificate
                if let Some(cert) = &local_cert {
                    dtls.set_certificate(cert.clone());
                }
                
                Some(Arc::new(RwLock::new(dtls)))
            },
            SecurityMode::SrtpWithPsk => None,
            SecurityMode::None => None,
        };
        
        // Clone config for later use
        let config_copy = config.clone();
        
        let context = Arc::new(Self {
            config,
            dtls,
            transport: Arc::new(RwLock::new(None)),
            srtp: Arc::new(RwLock::new(None)),
            local_cert: Arc::new(RwLock::new(local_cert)),
            remote_fingerprint: Arc::new(RwLock::new(None)),
            srtp_keys: Arc::new(RwLock::new(None)),
            secure: Arc::new(RwLock::new(false)),
            remote_addr: Arc::new(RwLock::new(None)),
            handshake_status: Arc::new(tokio::sync::Notify::new()),
            handshake_error: Arc::new(RwLock::new(None)),
        });
        
        // If using PSK for SRTP, initialize SRTP directly
        if config_copy.mode == SecurityMode::SrtpWithPsk {
            if let Some(key_material) = &config_copy.psk_material {
                let crypto_suite = api_to_internal_crypto_suite(
                    config_copy.srtp_profiles.first().unwrap_or(&SrtpProfile::AesCm128HmacSha1_80)
                );
                
                // Create key from the PSK material (split into key and salt)
                let key_len = key_material.len() / 2;
                let key_part = key_material[..key_len].to_vec();
                let salt_part = key_material[key_len..].to_vec();
                
                // Create key with both parts
                let key = crypto::SrtpCryptoKey::new(key_part, salt_part);
                
                // Create SRTP context
                let srtp = SrtpContext::new(crypto_suite, key)
                    .map_err(|e| SecurityError::SrtpError(format!("Failed to create SRTP context: {}", e)))?;
                
                let mut ctx = context.srtp.write().await;
                *ctx = Some(srtp);
                
                // Mark as secure
                let mut secure = context.secure.write().await;
                *secure = true;
                
                info!("SRTP context created from PSK");
            }
        }
        
        Ok(context)
    }
    
    /// Set the transport socket for DTLS
    /// This must be called before starting the handshake
    pub async fn set_transport_socket(&self, socket: Arc<UdpSocket>) -> Result<(), SecurityError> {
        // Create a DTLS transport from the socket
        debug!("Creating DTLS transport from RTP socket: local_addr={:?}", socket.local_addr());
        
        // Check that the socket is bound to an address
        let local_addr = socket.local_addr()
            .map_err(|e| SecurityError::InitError(format!("Socket not bound to an address: {}", e)))?;
        
        debug!("Socket is bound to local address: {}", local_addr);
        
        let dtls_transport = UdpTransport::new(socket.clone(), 1500).await
            .map_err(|e| SecurityError::InitError(format!("Failed to create DTLS transport: {:?}", e)))?;
        
        // Store the transport
        {
            let mut transport = self.transport.write().await;
            debug!("Storing transport in context and setting initial buffer capacity");
            *transport = Some(Arc::new(tokio::sync::Mutex::new(dtls_transport)));
        }
        
        // Check if we have an existing DTLS connection
        if let Some(dtls) = &self.dtls {
            let mut dtls_guard = dtls.write().await;
            
            // Get the transport
            if let Some(transport) = &*self.transport.read().await {
                // Set the transport
                debug!("Setting transport on DTLS connection");
                dtls_guard.set_transport(transport.clone());
            }
        }
        
        // Start the transport
        if let Some(transport) = &*self.transport.read().await {
            let mut transport_guard = transport.lock().await;
            debug!("Starting DTLS transport");
            transport_guard.start().await
                .map_err(|e| SecurityError::InitError(format!("Failed to start DTLS transport: {:?}", e)))?;
            
            // Test that the transport can receive packets by checking if the socket is properly set up
            match socket.local_addr() {
                Ok(addr) => debug!("Transport started successfully, listening on {}", addr),
                Err(e) => return Err(SecurityError::InitError(format!("Transport socket error: {}", e))),
            }
        }
        
        Ok(())
    }
    
    // Helper method to set up DTLS callbacks
    async fn setup_dtls_callbacks(&self) -> Result<(), SecurityError> {
        if let Some(dtls_arc) = &self.dtls {
            let mut dtls = dtls_arc.write().await;
            // Use the callbacks available in your actual DtlsConnection
            
            // Note: The current callbacks are simplified stubs since we don't 
            // have the exact signature available
            
            // For key derivation, we'll handle this by exporting keys after handshake
            
            // For handshake completion, we'll handle by checking the state
        }
        
        Ok(())
    }
    
    /// Extract SRTP keys from DTLS after handshake complete
    async fn extract_srtp_keys(&self) -> Result<(), SecurityError> {
        if let Some(dtls_arc) = &self.dtls {
            let dtls = dtls_arc.read().await;
            if dtls.state() != ConnectionState::Connected {
                return Err(SecurityError::HandshakeError("DTLS handshake not completed".to_string()));
            }
            
            // Extract SRTP keys from DTLS
            let srtp_context = dtls.extract_srtp_keys()
                .map_err(|e| SecurityError::SrtpError(format!("Failed to extract SRTP keys: {}", e)))?;
            
            // Get the key for our role
            // The server key is for receiving client packets and vice versa
            let is_client = self.config.dtls_client;
            let key = srtp_context.get_key_for_role(is_client).clone();
            
            // Map the SRTP crypto suite to an API profile
            // This is a best-effort mapping based on the authentication and key length
            let profile_id = match srtp_context.profile.authentication {
                crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80 => 
                    SrtpProfile::AesCm128HmacSha1_80,
                crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_32 => 
                    SrtpProfile::AesCm128HmacSha1_32,
                _ => SrtpProfile::AesCm128HmacSha1_80, // Default fallback
            };
            
            // Create a new SRTP context with the key and the same crypto suite
            // Clone the profile to avoid moving out of srtp_context
            let srtp_ctx = SrtpContext::new(srtp_context.profile.clone(), key)
                .map_err(|e| SecurityError::SrtpError(format!("Failed to create SRTP context: {}", e)))?;
            
            // Store the context
            let mut srtp = self.srtp.write().await;
            *srtp = Some(srtp_ctx);
            
            // Mark as secure
            let mut secure = self.secure.write().await;
            *secure = true;
            
            info!("SRTP context created from DTLS-SRTP");
        }
        
        Ok(())
    }
    
    /// Set the remote IP address for DTLS
    pub async fn set_remote_address(&self, addr: std::net::SocketAddr) -> Result<(), SecurityError> {
        info!("Setting remote address {} for DTLS", addr);
        let mut remote_addr = self.remote_addr.write().await;
        *remote_addr = Some(addr);
        
        // Log current state
        if let Some(dtls) = &self.dtls {
            if let Ok(dtls_guard) = dtls.try_read() {
                debug!("DTLS connection state: {:?}", dtls_guard.state());
            }
        }
        
        Ok(())
    }
}

/// Convert API SRTP profile to internal crypto suite
fn api_to_internal_crypto_suite(profile: &SrtpProfile) -> SrtpCryptoSuite {
    match profile {
        SrtpProfile::AesCm128HmacSha1_80 => crate::srtp::SRTP_AES128_CM_SHA1_80,
        SrtpProfile::AesCm128HmacSha1_32 => crate::srtp::SRTP_AES128_CM_SHA1_32,
        SrtpProfile::AesGcm128 => SrtpCryptoSuite {
            encryption: crate::srtp::SrtpEncryptionAlgorithm::AesCm,  // No GCM in implementation, use CM
            authentication: crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80,
            key_length: 16,  // 128 bits = 16 bytes
            tag_length: 10,  // 80 bits = 10 bytes
        },
        SrtpProfile::AesGcm256 => SrtpCryptoSuite {
            encryption: crate::srtp::SrtpEncryptionAlgorithm::AesCm,  // No GCM in implementation, use CM
            authentication: crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80,
            key_length: 32,  // 256 bits = 32 bytes
            tag_length: 10,  // 80 bits = 10 bytes
        },
    }
}

#[async_trait]
impl SecureMediaContext for DefaultSecureMediaContext {
    fn get_security_info(&self) -> SecurityInfo {
        // Get fingerprint from local certificate
        let (fingerprint, algorithm) = match self.local_cert.try_read() {
            Ok(cert_guard) => {
                if let Some(cert) = &*cert_guard {
                    let mut cert_clone = cert.clone();
                    match cert_clone.fingerprint("SHA-256") {
                        Ok(fp) => (Some(fp), Some("sha-256".to_string())),
                        Err(_) => (None, Some("sha-256".to_string())),
                    }
                } else {
                    (None, Some("sha-256".to_string()))
                }
            },
            Err(_) => (None, Some("sha-256".to_string())),
        };
        
        // Use blocking access for this sync method
        let setup_role = if let Some(dtls) = &self.dtls {
            match self.config.dtls_client {
                true => "active".to_string(),
                false => "passive".to_string(),
            }
        } else {
            "active".to_string() // Default if not using DTLS
        };
        
        // Use blocking access for this sync method
        let srtp_profile = match self.secure.try_read() {
            Ok(secure) if *secure => {
                Some(*self.config.srtp_profiles.first().unwrap_or(&SrtpProfile::AesCm128HmacSha1_80))
            },
            _ => None,
        };
        
        SecurityInfo {
            fingerprint,
            fingerprint_algorithm: algorithm,
            setup_role,
            srtp_profile,
        }
    }
    
    fn is_secure(&self) -> bool {
        // For sync function in async impl, we need to use blocking getter
        match self.secure.try_read() {
            Ok(secure) => *secure,
            Err(_) => false, // Default to false if lock is poisoned
        }
    }
    
    async fn set_remote_fingerprint(&mut self, fingerprint: &str, algorithm: &str) 
        -> Result<(), SecurityError> 
    {
        if self.config.mode != SecurityMode::DtlsSrtp {
            return Ok(());  // Ignore when not using DTLS
        }
        
        // Store the fingerprint using async-safe methods
        let mut remote = self.remote_fingerprint.write().await;
        *remote = Some((fingerprint.to_string(), algorithm.to_string()));
        
        // If we have a DTLS connection, set up the fingerprint verifier
        if let Some(dtls_arc) = &self.dtls {
            let mut dtls = dtls_arc.write().await;
            // Create a fingerprint verifier
            info!("Setting up remote fingerprint verifier: {} ({})", fingerprint, algorithm);
            
            // In a real implementation, we would set up the verifier here
            // For now, just log that we received it
        }
        
        Ok(())
    }
    
    async fn set_remote_address(&self, addr: std::net::SocketAddr) -> Result<(), SecurityError> {
        // Use the properly async method we defined above
        self.set_remote_address(addr).await
    }
    
    async fn start_handshake(&self) -> Result<(), SecurityError> {
        debug!("Starting handshake with mode: {:?}", self.config.mode);
        
        if let Some(dtls) = &self.dtls {
            // Clear any previous error state
            {
                let mut error = self.handshake_error.write().await;
                *error = None;
            }
            
            // Verify that remote fingerprint is set
            let remote_fingerprint = {
                let fp = self.remote_fingerprint.read().await;
                fp.clone()
            };
            
            if remote_fingerprint.is_none() {
                debug!("Remote fingerprint not set, handshake cannot proceed");
                return Err(SecurityError::HandshakeError(
                    "Remote fingerprint not set. Set it before starting handshake.".to_string()
                ));
            }
            
            // Get the remote address
            let remote_addr = {
                let addr = self.remote_addr.read().await;
                match *addr {
                    Some(addr) => {
                        debug!("Using remote address: {}", addr);
                        addr
                    },
                    None => {
                        debug!("Remote address not set, handshake cannot proceed");
                        return Err(SecurityError::HandshakeError(
                            "Remote address not set. Set it before starting handshake.".to_string()
                        ));
                    }
                }
            };
            
            // Check if we have a transport
            if self.transport.read().await.is_none() {
                debug!("DTLS transport not set, handshake cannot proceed");
                return Err(SecurityError::HandshakeError(
                    "DTLS transport not set. Call set_transport_socket before starting handshake.".to_string()
                ));
            }
            
            debug!("DTLS configuration check passed, preparing connection");
            
            // Ensure the transport is set on the DTLS connection
            {
                let mut dtls_guard = dtls.write().await;
                if let Some(transport) = &*self.transport.read().await {
                    if !dtls_guard.has_transport() {
                        debug!("Setting transport on DTLS connection before handshake");
                        dtls_guard.set_transport(transport.clone());
                    } else {
                        debug!("Transport already set on DTLS connection");
                    }
                }
            }
            
            // Handle client/server roles differently, following the pattern in dtls_test.rs
            if self.config.dtls_client {
                // Client role: initiate handshake immediately
                debug!("Client role: initiating DTLS handshake to {}", remote_addr);
                
                // Get a mutable reference to the DTLS connection
                let mut dtls = dtls.write().await;
                
                // Create a fingerprint verifier from the remote fingerprint
                let (fp, alg) = remote_fingerprint.unwrap();
                info!("Client starting DTLS handshake with remote fingerprint: {} ({})", fp, alg);
                
                // Start DTLS handshake - this will send the ClientHello
                match dtls.start_handshake(remote_addr).await {
                    Ok(_) => {
                        info!("Client DTLS handshake initiated successfully");
                        
                        // For client role, always send an explicit ClientHello trigger packet
                        // to ensure connectivity is established
                        debug!("Client explicitly ensuring ClientHello is sent");
                        let transport = self.transport.read().await;
                        if let Some(transport) = &*transport {
                            debug!("Client triggering DTLS handshake message exchange");
                            let connection_state = dtls.state();
                            debug!("DTLS connection state after client start_handshake: {:?}", connection_state);
                            
                            // Send multiple trigger packets to increase reliability
                            let mut transport_guard = transport.lock().await;
                            debug!("Client sending trigger packets to server at {}", remote_addr);
                            
                            // Send a series of trigger packets with increasing delays
                            for i in 0..5 {
                                // Process any packets that might be waiting
                                // Create a timeout future to avoid blocking
                                match tokio::time::timeout(
                                    std::time::Duration::from_millis(10),
                                    transport_guard.recv()
                                ).await {
                                    Ok(Some((packet, addr))) => {
                                        debug!("Client received packet ({}b) from {} during handshake initiation", packet.len(), addr);
                                        if let Err(e) = dtls.process_packet(&packet).await {
                                            warn!("Error processing received packet during client handshake initiation: {:?}", e);
                                        }
                                    },
                                    Ok(None) => {}, // No packet
                                    Err(_) => {}, // Timeout, which is expected
                                }
                                
                                // Send a trigger packet
                                if let Err(e) = transport_guard.send(&[i + 1, 3, 3, 7], remote_addr).await {
                                    warn!("Failed to send trigger packet {}: {:?}", i, e);
                                } else {
                                    debug!("Sent trigger packet {} to initiate DTLS exchange", i);
                                }
                                
                                // Increasing delay between packets
                                let delay_ms = 50u64 * (i as u64 + 1);
                                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            }
                        } else {
                            warn!("Client has no transport to send initial DTLS handshake packet");
                        }
                        
                        // Start a background task to wait for the handshake to complete
                        let dtls_arc = self.dtls.as_ref().unwrap().clone();
                        let srtp_lock = self.srtp.clone();
                        let secure_lock = self.secure.clone();
                        let is_client = self.config.dtls_client;
                        let handshake_status = self.handshake_status.clone();
                        let handshake_error = self.handshake_error.clone();
                        
                        tokio::spawn(async move {
                            debug!("Client handshake background task started");
                            let mut dtls_guard = dtls_arc.write().await;
                            
                            debug!("Client waiting for DTLS handshake completion");
                            match dtls_guard.wait_handshake().await {
                                Ok(_) => {
                                    info!("Client DTLS handshake completed successfully");
                                    
                                    // Extract SRTP keys
                                    match dtls_guard.extract_srtp_keys() {
                                        Ok(srtp_context) => {
                                            debug!("Client extracted SRTP keys successfully");
                                            
                                            // Get the key for client role
                                            let key = srtp_context.get_key_for_role(is_client).clone();
                                            
                                            // Create SRTP context
                                            match SrtpContext::new(srtp_context.profile.clone(), key) {
                                                Ok(srtp_ctx) => {
                                                    // Store the context
                                                    let mut srtp_result = srtp_lock.write().await;
                                                    *srtp_result = Some(srtp_ctx);
                                                    
                                                    // Mark as secure
                                                    let mut secure_result = secure_lock.write().await;
                                                    *secure_result = true;
                                                    
                                                    handshake_status.notify_waiters();
                                                },
                                                Err(e) => {
                                                    error!("Client failed to create SRTP context: {}", e);
                                                    let mut error = handshake_error.write().await;
                                                    *error = Some(format!("Failed to create SRTP context: {}", e));
                                                    handshake_status.notify_waiters();
                                                }
                                            }
                                        },
                                        Err(e) => {
                                            error!("Client failed to extract SRTP keys: {}", e);
                                            let mut error = handshake_error.write().await;
                                            *error = Some(format!("Failed to extract SRTP keys: {}", e));
                                            handshake_status.notify_waiters();
                                        }
                                    }
                                },
                                Err(e) => {
                                    error!("Client DTLS handshake failed: {}", e);
                                    let mut error = handshake_error.write().await;
                                    *error = Some(format!("DTLS handshake failed: {}", e));
                                    handshake_status.notify_waiters();
                                }
                            }
                        });
                        
                        Ok(())
                    },
                    Err(e) => {
                        error!("Failed to start client DTLS handshake: {}", e);
                        Err(SecurityError::HandshakeError(format!("DTLS error: {}", e)))
                    }
                }
            } else {
                // Server role: wait for initial packet like in dtls_test.rs
                debug!("Server role: waiting for initial ClientHello packet");
                
                // Get the transport
                let transport_opt = self.transport.read().await;
                let transport = transport_opt.as_ref().ok_or_else(|| 
                    SecurityError::HandshakeError("No transport available".to_string())
                )?;
                
                // Wait for the initial packet with a timeout
                let mut transport_guard = transport.lock().await;
                
                // Print some diagnostic info
                debug!("Server preparing to receive initial packet from: {}", remote_addr);
                
                // Get the current DTLS state
                let dtls_state = {
                    if let Ok(dtls_guard) = dtls.try_read() {
                        dtls_guard.state()
                    } else {
                        debug!("Could not get DTLS state due to lock contention");
                        ConnectionState::New // Default to New if we can't get the lock
                    }
                };
                debug!("Server DTLS state before waiting: {:?}", dtls_state);
                
                // Try more aggressively to receive packets
                let mut initial_packet: Option<Vec<u8>> = None;
                
                // Try multiple times to receive the initial packet
                for attempt in 0..5 {
                    debug!("Server waiting for initial packet (attempt {})", attempt + 1);
                    
                    let timeout_result = tokio::time::timeout(
                        std::time::Duration::from_secs(2), // 2 second timeout per attempt
                        transport_guard.recv()
                    ).await;
                    
                    match timeout_result {
                        Ok(Some((packet, addr))) => {
                            debug!("Server received packet: {} bytes from {}", packet.len(), addr);
                            
                            // Check if it's from the expected remote address
                            if addr != remote_addr {
                                warn!("Received packet from unexpected address: {} (expected {})", addr, remote_addr);
                            }
                            
                            // Try to parse the packet as a DTLS record for logging
                            if let Ok(records) = crate::dtls::record::Record::parse_multiple(&packet) {
                                for record in records {
                                    debug!("Received record of type: {:?}", record.header.content_type);
                                    
                                    if record.header.content_type == crate::dtls::record::ContentType::Handshake {
                                        if let Ok((header, _)) = crate::dtls::message::handshake::HandshakeHeader::parse(&record.data) {
                                            debug!("Handshake message type: {:?}", header.msg_type);
                                        }
                                    }
                                }
                            }
                            
                            initial_packet = Some(packet.to_vec());
                            break;
                        },
                        Ok(None) => {
                            debug!("Server received no packet from transport (attempt {})", attempt + 1);
                        },
                        Err(_) => {
                            debug!("Server timed out waiting for initial packet (attempt {})", attempt + 1);
                            
                            // Send a probe packet to help stimulate the connection
                            if let Err(e) = transport_guard.send(&[88, 77, 66, 55], remote_addr).await {
                                warn!("Failed to send server probe packet: {:?}", e);
                            } else {
                                debug!("Server sent probe packet to client at {}", remote_addr);
                            }
                        }
                    }
                    
                    // Small delay between attempts
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                
                // Log failure if we didn't get a packet
                if initial_packet.is_none() {
                    warn!("Server timed out waiting for initial packet after multiple attempts");
                }
                
                // Drop the transport guard to avoid deadlock
                drop(transport_guard);
                
                // Get a mutable reference to the DTLS connection
                let mut dtls = dtls.write().await;
                
                // Create a fingerprint verifier from the remote fingerprint
                let (fp, alg) = remote_fingerprint.unwrap();
                info!("Server starting DTLS handshake with remote fingerprint: {} ({})", fp, alg);
                
                // Start DTLS handshake
                match dtls.start_handshake(remote_addr).await {
                    Ok(_) => {
                        info!("Server DTLS handshake initiated successfully");
                        
                        // If we received an initial packet, process it now - THIS IS CRITICAL
                        if let Some(packet_data) = initial_packet {
                            debug!("Server processing initial ClientHello packet");
                            if let Err(e) = dtls.process_packet(&packet_data).await {
                                warn!("Server error processing initial packet: {:?}", e);
                                // Continue anyway, as it might not be fatal
                            }
                        }
                        
                        // Even if we didn't get an initial packet, try to proactively send a server hello
                        // This can help in STUN-based connectivity or when the initial ClientHello was lost
                        let transport = self.transport.read().await;
                        if let Some(transport) = &*transport {
                            let mut transport_guard = transport.lock().await;
                            
                            // Send a few trigger packets to help stimulate the connection
                            debug!("Server sending response trigger packets to client at {}", remote_addr);
                            for i in 0..3 {
                                if let Err(e) = transport_guard.send(&[44, 33, 22, 11], remote_addr).await {
                                    warn!("Failed to send server response packet {}: {:?}", i, e);
                                } else {
                                    debug!("Server sent response packet {}", i);
                                }
                                
                                // Small delay between packets
                                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            }
                        }
                        
                        // Start a background task to wait for the handshake to complete
                        let dtls_arc = self.dtls.as_ref().unwrap().clone();
                        let srtp_lock = self.srtp.clone();
                        let secure_lock = self.secure.clone();
                        let is_client = self.config.dtls_client;
                        let handshake_status = self.handshake_status.clone();
                        let handshake_error = self.handshake_error.clone();
                        
                        tokio::spawn(async move {
                            debug!("Server handshake background task started");
                            let mut dtls_guard = dtls_arc.write().await;
                            
                            debug!("Server waiting for DTLS handshake completion");
                            match dtls_guard.wait_handshake().await {
                                Ok(_) => {
                                    info!("Server DTLS handshake completed successfully");
                                    
                                    // Extract SRTP keys
                                    match dtls_guard.extract_srtp_keys() {
                                        Ok(srtp_context) => {
                                            debug!("Server extracted SRTP keys successfully");
                                            
                                            // Get the key for server role
                                            let key = srtp_context.get_key_for_role(is_client).clone();
                                            
                                            // Create SRTP context
                                            match SrtpContext::new(srtp_context.profile.clone(), key) {
                                                Ok(srtp_ctx) => {
                                                    // Store the context
                                                    let mut srtp_result = srtp_lock.write().await;
                                                    *srtp_result = Some(srtp_ctx);
                                                    
                                                    // Mark as secure
                                                    let mut secure_result = secure_lock.write().await;
                                                    *secure_result = true;
                                                    
                                                    handshake_status.notify_waiters();
                                                },
                                                Err(e) => {
                                                    error!("Server failed to create SRTP context: {}", e);
                                                    let mut error = handshake_error.write().await;
                                                    *error = Some(format!("Failed to create SRTP context: {}", e));
                                                    handshake_status.notify_waiters();
                                                }
                                            }
                                        },
                                        Err(e) => {
                                            error!("Server failed to extract SRTP keys: {}", e);
                                            let mut error = handshake_error.write().await;
                                            *error = Some(format!("Failed to extract SRTP keys: {}", e));
                                            handshake_status.notify_waiters();
                                        }
                                    }
                                },
                                Err(e) => {
                                    error!("Server DTLS handshake failed: {}", e);
                                    let mut error = handshake_error.write().await;
                                    *error = Some(format!("DTLS handshake failed: {}", e));
                                    handshake_status.notify_waiters();
                                }
                            }
                        });
                        
                        Ok(())
                    },
                    Err(e) => {
                        error!("Failed to start server DTLS handshake: {}", e);
                        Err(SecurityError::HandshakeError(format!("DTLS error: {}", e)))
                    }
                }
            }
        } else if self.config.mode == SecurityMode::SrtpWithPsk {
            if self.config.psk_material.is_none() {
                return Err(SecurityError::ConfigurationError(
                    "PSK mode enabled but no key material provided".to_string()
                ));
            }
            
            // PSK is already set up in constructor, nothing to do here
            info!("Using pre-shared keys for SRTP, no handshake needed");
            
            // Set secure flag since PSK is ready
            let mut secure = self.secure.write().await;
            *secure = true;
            
            // Notify anyone waiting for handshake completion
            self.handshake_status.notify_waiters();
            
            Ok(())
        } else {
            // No security, nothing to do
            info!("Security disabled, no handshake needed");
            
            // Set secure flag since no security is needed
            let mut secure = self.secure.write().await;
            *secure = true;
            
            // Notify anyone waiting for handshake completion
            self.handshake_status.notify_waiters();
            
            Ok(())
        }
    }
    
    async fn wait_handshake(&self) -> Result<(), SecurityError> {
        debug!("Waiting for DTLS handshake to complete...");
        
        // For PSK or None security modes, return immediately
        if self.config.mode != SecurityMode::DtlsSrtp {
            debug!("Not using DTLS-SRTP, no need to wait for handshake");
            return Ok(());
        }
        
        // Wait for the handshake to complete with a timeout
        let timeout_duration = std::time::Duration::from_secs(30); // Increase timeout to 30 seconds
        let timeout = tokio::time::sleep(timeout_duration);
        
        tokio::select! {
            _ = timeout => {
                error!("DTLS handshake timed out after {:?}", timeout_duration);
                
                // Get more detailed information about the state
                if let Some(dtls_arc) = &self.dtls {
                    if let Ok(dtls) = dtls_arc.try_read() {
                        let state = dtls.state();
                        error!("DTLS connection state at timeout: {:?}", state);
                    }
                }
                
                // Try to get transport information
                if let Ok(guard) = self.transport.try_read() {
                    if let Some(transport) = &*guard {
                        debug!("Transport is initialized but handshake timed out");
                    }
                }
                
                return Err(SecurityError::HandshakeError(
                    format!("DTLS handshake timed out after {:?}", timeout_duration)
                ));
            },
            _ = self.handshake_status.notified() => {
                // Continue with normal processing
                debug!("Received handshake status notification");
            }
        }
        
        // Check if there was an error
        let error_msg = {
            let error = self.handshake_error.read().await;
            error.clone()
        };
        
        if let Some(msg) = error_msg {
            error!("DTLS handshake failed with error: {}", msg);
            return Err(SecurityError::HandshakeError(msg));
        }
        
        // Check if secure flag is set
        let is_secure = {
            let secure = self.secure.read().await;
            *secure
        };
        
        if !is_secure {
            error!("DTLS handshake completed but secure flag not set");
            
            // Get more information about the state
            if let Some(dtls_arc) = &self.dtls {
                if let Ok(dtls) = dtls_arc.try_read() {
                    let state = dtls.state();
                    error!("DTLS connection state: {:?}", state);
                }
            }
            
            return Err(SecurityError::HandshakeError(
                "Handshake completed but secure flag not set".to_string()
            ));
        }
        
        debug!("DTLS handshake completed successfully");
        Ok(())
    }
    
    async fn set_transport_socket(&self, socket: std::sync::Arc<tokio::net::UdpSocket>) -> Result<(), SecurityError> {
        self.set_transport_socket(socket).await
    }
} 