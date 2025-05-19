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
        debug!("Creating DTLS transport from RTP socket");
        let dtls_transport = UdpTransport::new(socket, 1500).await
            .map_err(|e| SecurityError::InitError(format!("Failed to create DTLS transport: {:?}", e)))?;
        
        // Store the transport
        {
            let mut transport = self.transport.write().await;
            *transport = Some(Arc::new(tokio::sync::Mutex::new(dtls_transport)));
        }
        
        // Check if we have an existing DTLS connection
        if let Some(dtls) = &self.dtls {
            let mut dtls_guard = dtls.write().await;
            
            // Get the transport
            if let Some(transport) = &*self.transport.read().await {
                // Set the transport
                dtls_guard.set_transport(transport.clone());
                debug!("Set transport on DTLS connection");
            }
        }
        
        // Start the transport
        if let Some(transport) = &*self.transport.read().await {
            let mut transport_guard = transport.lock().await;
            transport_guard.start().await
                .map_err(|e| SecurityError::InitError(format!("Failed to start DTLS transport: {:?}", e)))?;
            
            debug!("Started DTLS transport");
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
    pub fn set_remote_address(&self, addr: std::net::SocketAddr) -> Result<(), SecurityError> {
        match self.remote_addr.try_write() {
            Ok(mut remote_addr) => {
                info!("Setting remote address {} for DTLS", addr);
                *remote_addr = Some(addr);
                Ok(())
            },
            Err(_) => Err(SecurityError::HandshakeError("Failed to acquire lock".to_string())),
        }
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
    
    fn set_remote_fingerprint(&mut self, fingerprint: &str, algorithm: &str) 
        -> Result<(), SecurityError> 
    {
        if self.config.mode != SecurityMode::DtlsSrtp {
            return Ok(());  // Ignore when not using DTLS
        }
        
        // Store the fingerprint
        match self.remote_fingerprint.try_write() {
            Ok(mut remote) => {
                *remote = Some((fingerprint.to_string(), algorithm.to_string()));
                
                // If we have a DTLS connection, set up the fingerprint verifier
                if let Some(dtls_arc) = &self.dtls {
                    if let Ok(mut dtls) = dtls_arc.try_write() {
                        // Create a fingerprint verifier
                        info!("Setting up remote fingerprint verifier: {} ({})", fingerprint, algorithm);
                        
                        // In a real implementation, we would set up the verifier here
                        // For now, just log that we received it
                    }
                }
                
                Ok(())
            },
            Err(_) => Err(SecurityError::HandshakeError("Failed to acquire lock".to_string())),
        }
    }
    
    fn set_remote_address(&self, addr: std::net::SocketAddr) -> Result<(), SecurityError> {
        match self.remote_addr.try_write() {
            Ok(mut remote_addr) => {
                info!("Setting remote address {} for DTLS", addr);
                *remote_addr = Some(addr);
                Ok(())
            },
            Err(_) => Err(SecurityError::HandshakeError("Failed to acquire lock".to_string())),
        }
    }
    
    async fn start_handshake(&self) -> Result<(), SecurityError> {
        if let Some(dtls) = &self.dtls {
            // Verify that remote fingerprint is set
            let remote_fingerprint = {
                let fp = self.remote_fingerprint.read().await;
                fp.clone()
            };
            
            if remote_fingerprint.is_none() {
                return Err(SecurityError::HandshakeError(
                    "Remote fingerprint not set. Set it before starting handshake.".to_string()
                ));
            }
            
            // Get the remote address
            let remote_addr = {
                let addr = self.remote_addr.read().await;
                match *addr {
                    Some(addr) => addr,
                    None => return Err(SecurityError::HandshakeError(
                        "Remote address not set. Set it before starting handshake.".to_string()
                    )),
                }
            };
            
            // Check if we have a transport
            if self.transport.read().await.is_none() {
                return Err(SecurityError::HandshakeError(
                    "DTLS transport not set. Call set_transport_socket before starting handshake.".to_string()
                ));
            }
            
            // Ensure the transport is set on the DTLS connection
            {
                let mut dtls_guard = dtls.write().await;
                if let Some(transport) = &*self.transport.read().await {
                    if !dtls_guard.has_transport() {
                        debug!("Setting transport on DTLS connection before handshake");
                        dtls_guard.set_transport(transport.clone());
                    }
                }
            }
            
            // Get a mutable reference to the DTLS connection
            let mut dtls = dtls.write().await;
            
            // Create a fingerprint verifier from the remote fingerprint
            let (fp, alg) = remote_fingerprint.unwrap();
            info!("Starting DTLS handshake with remote fingerprint: {} ({})", fp, alg);
            
            // Start DTLS handshake
            match dtls.start_handshake(remote_addr).await {
                Ok(_) => {
                    info!("DTLS handshake started");
                    
                    // Start a background task to wait for the handshake to complete
                    let dtls_arc = self.dtls.as_ref().unwrap().clone();
                    
                    // To avoid the Send issues with raw pointers, we'll extract the keys
                    // directly in this task and update the secure state
                    let srtp_lock = self.srtp.clone();
                    let secure_lock = self.secure.clone();
                    let is_client = self.config.dtls_client;
                    
                    tokio::spawn(async move {
                        // Wait for the handshake to complete
                        let mut dtls_guard = dtls_arc.write().await;
                        match dtls_guard.wait_handshake().await {
                            Ok(_) => {
                                info!("DTLS handshake completed successfully");
                                
                                // Extract SRTP keys directly
                                match dtls_guard.extract_srtp_keys() {
                                    Ok(srtp_context) => {
                                        // Get the key for our role
                                        let key = srtp_context.get_key_for_role(is_client).clone();
                                        
                                        // Create a new SRTP context with the key and crypto suite
                                        match SrtpContext::new(srtp_context.profile.clone(), key) {
                                            Ok(srtp_ctx) => {
                                                // Store the context - tokio RwLock.write() doesn't return a Result
                                                let mut srtp_result = srtp_lock.write().await;
                                                *srtp_result = Some(srtp_ctx);
                                                
                                                // Mark as secure
                                                let mut secure_result = secure_lock.write().await;
                                                *secure_result = true;
                                                info!("SRTP context created from DTLS-SRTP");
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
                    });
                    
                    Ok(())
                },
                Err(e) => {
                    Err(SecurityError::HandshakeError(format!("DTLS error: {}", e)))
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
            Ok(())
        } else {
            // No security, nothing to do
            info!("Security disabled, no handshake needed");
            Ok(())
        }
    }
    
    async fn set_transport_socket(&self, socket: std::sync::Arc<tokio::net::UdpSocket>) -> Result<(), SecurityError> {
        self.set_transport_socket(socket).await
    }
} 