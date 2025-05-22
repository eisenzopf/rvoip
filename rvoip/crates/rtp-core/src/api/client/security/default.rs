//! Default implementation of client security context
//!
//! This module contains the DefaultClientSecurityContext struct and its implementation
//! of the ClientSecurityContext trait.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::any::Any;
use std::collections::HashMap;
use tokio::sync::Mutex;
use async_trait::async_trait;
use tracing::{debug, error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::UdpSocket;
use uuid::Uuid;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::client::security::{ClientSecurityContext, ClientSecurityConfig, create_dtls_config};
use crate::api::server::security::SocketHandle;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_NULL_NULL, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM};
use crate::srtp::crypto::SrtpCryptoKey;
use crate::dtls::{DtlsConnection, DtlsConfig, DtlsRole, DtlsVersion};
use crate::dtls::transport::udp::UdpTransport;
use crate::dtls::record::{Record, ContentType};
use crate::dtls::message::handshake::HandshakeHeader;

// Import module functions
use crate::api::client::security::dtls::{connection, handshake, transport};
use crate::api::client::security::srtp::keys;
use crate::api::client::security::fingerprint::verify;
use crate::api::client::security::packet::processor;

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
        connection::init_connection(&self.config, &self.socket, &self.connection).await
    }
    
    /// Start a handshake monitor task
    async fn start_handshake_monitor(&self) -> Result<(), SecurityError> {
        handshake::start_handshake_monitor(
            &self.handshake_monitor_running,
            &self.remote_addr,
            &self.socket,
            &self.connection,
            &self.handshake_completed
        ).await
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
        transport::setup_transport(&socket, &self.connection).await?;
        
        // Store socket
        let mut socket_lock = self.socket.lock().await;
        *socket_lock = Some(socket.clone());
        
        // Set remote address if available
        if let Some(remote_addr) = socket.remote_addr {
            let mut remote_addr_guard = self.remote_addr.lock().await;
            *remote_addr_guard = Some(remote_addr);
        }
        
        // Start packet handler
        let remote_addr = self.remote_addr.lock().await.unwrap_or_else(|| {
            // Use a default if not set - this shouldn't happen in practice
            SocketAddr::from(([127, 0, 0, 1], 8000))
        });
        
        // Create a context reference for the packet handler
        let context = Arc::new(self.clone()) as Arc<dyn ClientSecurityContext>;
        
        // Start the packet handler in the background
        transport::start_packet_handler(&socket, remote_addr, context).await?;
        
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
        
        // Get the fingerprint from our connection
        let fingerprint = self.get_fingerprint().await.unwrap_or_else(|_| {
            "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF".to_string()
        });
        
        Ok(SecurityInfo {
            mode: self.config.security_mode,
            fingerprint: Some(fingerprint),
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
        verify::generate_fingerprint(&self.connection).await
    }

    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError> {
        // Return the default algorithm used
        Ok(verify::get_fingerprint_algorithm())
    }

    async fn has_transport(&self) -> Result<bool, SecurityError> {
        let conn_guard = self.connection.lock().await;
        if let Some(conn) = conn_guard.as_ref() {
            Ok(conn.has_transport())
        } else {
            Ok(false)
        }
    }

    async fn wait_for_handshake(&self) -> Result<(), SecurityError> {
        handshake::wait_for_handshake(
            &self.connection,
            &self.handshake_completed,
            &self.srtp_context
        ).await
    }

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

    async fn process_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        processor::process_packet(
            data,
            &self.connection,
            &self.remote_addr,
            &self.handshake_completed,
            &self.srtp_context
        ).await
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
        
        // Create a context reference for the packet handler
        let context = Arc::new(self.clone()) as Arc<dyn ClientSecurityContext>;
        
        // Delegate to the transport module's packet handler implementation
        transport::start_packet_handler(&socket, remote_addr, context).await
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

    async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError> {
        processor::process_dtls_packet(
            data,
            &self.connection,
            &self.remote_addr,
            &self.handshake_completed,
            &self.srtp_context
        ).await
    }

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