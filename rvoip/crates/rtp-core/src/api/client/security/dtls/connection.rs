//! DTLS connection setup and management
//!
//! This module provides functions for initializing and managing DTLS connections.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::api::common::error::SecurityError;
use crate::api::client::security::ClientSecurityConfig;
use crate::api::client::security::create_dtls_config as api_create_dtls_config;
use crate::api::server::security::SocketHandle;
use crate::dtls::{DtlsConnection, DtlsConfig, DtlsRole, DtlsVersion};
use crate::dtls::transport::udp::UdpTransport;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_NULL_NULL, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM};
use tokio::net::UdpSocket;

/// Initialize a DTLS connection with the given configuration
pub async fn init_connection(
    config: &ClientSecurityConfig,
    socket: &Arc<Mutex<Option<SocketHandle>>>,
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<(), SecurityError> {
    // Check if we have a socket
    let socket_guard = socket.lock().await;
    let socket = socket_guard.clone().ok_or_else(|| 
        SecurityError::Configuration("No socket set for security context".to_string()))?;
    drop(socket_guard);
    
    // Verify we have SRTP profiles configured
    if config.srtp_profiles.is_empty() {
        return Err(SecurityError::Configuration("No SRTP profiles specified".to_string()));
    }
    
    // Create DTLS connection config from our API config
    let dtls_config = api_create_dtls_config(config);
    
    // Create DTLS connection
    let mut connection_obj = DtlsConnection::new(dtls_config);
    
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
    connection_obj.set_certificate(cert);
    
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
        connection_obj.set_transport(transport);
        
        // Store the connection
        let mut conn_guard = connection.lock().await;
        *conn_guard = Some(connection_obj);
        
        Ok(())
    } else {
        // Log the error and return it
        let err = start_result.err().unwrap();
        error!("Failed to start DTLS transport: {}", err);
        Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", err)))
    }
}

/// Create a new DTLS connection
pub async fn create_connection(
    socket: &Arc<UdpSocket>,
    remote_addr: SocketAddr,
) -> Result<DtlsConnection, SecurityError> {
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
        
        debug!("Transport set on new connection");
        Ok(connection)
    } else {
        // Log the error and return it
        let err = start_result.err().unwrap();
        error!("Failed to start DTLS transport for new connection: {}", err);
        Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", err)))
    }
}

/// Create a DTLS configuration from the client security configuration
pub fn create_dtls_config(config: &ClientSecurityConfig) -> DtlsConfig {
    api_create_dtls_config(config)
} 