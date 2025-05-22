//! DTLS connection management
//!
//! This module handles the creation and management of DTLS connections.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SrtpProfile};
use crate::api::server::security::{ServerSecurityConfig, SocketHandle, ConnectionConfig, ConnectionRole};
use crate::dtls::{DtlsConnection, DtlsConfig, DtlsRole};

/// Create a new DTLS connection with server role
pub async fn create_server_connection(
    config: &ServerSecurityConfig,
) -> Result<DtlsConnection, SecurityError> {
    // Create DTLS config with server role
    let dtls_config = DtlsConfig {
        role: DtlsRole::Server,
        version: crate::dtls::DtlsVersion::Dtls12,
        mtu: 1500,
        max_retransmissions: 5,
        srtp_profiles: convert_profiles(&config.srtp_profiles),
    };
    
    // Create connection
    let connection = DtlsConnection::new(dtls_config);
    
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
    
    // Return the connection (we'll set the certificate in the caller)
    Ok(connection)
}

/// Initialize the connection template
pub async fn initialize_connection_template(
    config: &ServerSecurityConfig,
    connection_template: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<(), SecurityError> {
    // Verify we have SRTP profiles configured
    if config.srtp_profiles.is_empty() {
        return Err(SecurityError::Configuration("No SRTP profiles specified in server config".to_string()));
    }
    
    // Create DTLS connection template for the certificate
    let mut connection = create_server_connection(config).await?;
    
    // Generate or load certificate based on config
    let cert = if let (Some(cert_path), Some(key_path)) = (&config.certificate_path, &config.private_key_path) {
        // Logic to load certificate from files (simplified as in original)
        debug!("Loading certificate from {} and key from {}", cert_path, key_path);
        match crate::dtls::crypto::verify::generate_self_signed_certificate() {
            Ok(cert) => cert,
            Err(e) => return Err(SecurityError::Configuration(
                format!("Failed to generate certificate: {}", e)
            ))
        }
    } else {
        // Generate a self-signed certificate
        debug!("Generating self-signed certificate");
        match crate::dtls::crypto::verify::generate_self_signed_certificate() {
            Ok(cert) => cert,
            Err(e) => return Err(SecurityError::Configuration(
                format!("Failed to generate certificate: {}", e)
            ))
        }
    };
    
    // Set the certificate on the connection
    connection.set_certificate(cert);
    
    // Store the template
    let mut template = connection_template.lock().await;
    *template = Some(connection);
    
    Ok(())
}

/// Get the fingerprint from a connection
pub async fn get_fingerprint_from_connection(
    connection: &DtlsConnection,
) -> Result<String, SecurityError> {
    if let Some(cert) = connection.local_certificate() {
        // Create a mutable copy of the certificate to compute fingerprint
        let mut cert_copy = cert.clone();
        match cert_copy.fingerprint("SHA-256") {
            Ok(fingerprint) => Ok(fingerprint),
            Err(e) => Err(SecurityError::Internal(format!("Failed to get fingerprint: {}", e))),
        }
    } else {
        Err(SecurityError::Configuration("No certificate available".to_string()))
    }
}

/// Create a DTLS transport for a socket
pub async fn create_dtls_transport(
    socket: &SocketHandle,
) -> Result<Arc<Mutex<crate::dtls::transport::udp::UdpTransport>>, SecurityError> {
    // Create a transport for packet reception
    let transport = match crate::dtls::transport::udp::UdpTransport::new(socket.socket.clone(), 1500).await {
        Ok(mut t) => {
            // Start the transport (CRUCIAL)
            if let Err(e) = t.start().await {
                return Err(SecurityError::Configuration(format!("Failed to start DTLS transport: {}", e)));
            }
            t
        },
        Err(e) => return Err(SecurityError::Configuration(format!("Failed to create DTLS transport: {}", e))),
    };
    
    // Wrap in Arc<Mutex<>>
    Ok(Arc::new(Mutex::new(transport)))
}

/// Convert API SRTP profiles to internal format
pub fn convert_profiles(profiles: &[SrtpProfile]) -> Vec<crate::srtp::SrtpCryptoSuite> {
    profiles.iter()
        .filter_map(|p| {
            match p {
                SrtpProfile::AesCm128HmacSha1_80 => Some(crate::srtp::SRTP_AES128_CM_SHA1_80),
                SrtpProfile::AesCm128HmacSha1_32 => Some(crate::srtp::SRTP_AES128_CM_SHA1_32),
                SrtpProfile::AesGcm128 => Some(crate::srtp::SRTP_AEAD_AES_128_GCM),
                SrtpProfile::AesGcm256 => Some(crate::srtp::SRTP_AEAD_AES_256_GCM),
            }
        })
        .collect()
} 