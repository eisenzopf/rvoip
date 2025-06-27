//! Security context functionality
//!
//! This module handles security context initialization and management.

use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::server::security::{ServerSecurityContext, ServerSecurityConfig, SocketHandle};
use crate::dtls::DtlsConnection;

/// Initialize security context if needed
pub async fn initialize_security_context(
    config: &ServerSecurityConfig,
    socket: Option<SocketHandle>,
    connection_template: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<(), SecurityError> {
    debug!("Initializing server security context");
    
    // Verify that the connection template is initialized
    let template = connection_template.lock().await;
    if template.is_none() {
        return Err(SecurityError::Configuration("DTLS connection template not initialized".to_string()));
    }
    
    // Nothing more to do for initialization - each client connection
    // will be initialized individually on first contact
    Ok(())
}

/// Get security information for SDP exchange
pub async fn get_security_info(
    config: &ServerSecurityConfig,
    connection_template: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<SecurityInfo, SecurityError> {
    // Get fingerprint if available
    let fingerprint = match get_fingerprint_from_template(connection_template).await {
        Ok(fp) => Some(fp),
        Err(_) => None,
    };
    
    // Create security info
    let security_info = SecurityInfo {
        mode: config.security_mode,
        fingerprint: fingerprint,
        fingerprint_algorithm: Some(config.fingerprint_algorithm.clone()),
        crypto_suites: config.srtp_profiles.iter()
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
    };
    
    Ok(security_info)
}

/// Check if the security context is ready
pub async fn is_security_context_ready(
    socket: &Arc<Mutex<Option<SocketHandle>>>,
    connection_template: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<bool, SecurityError> {
    // Check if socket is set
    let socket_set = socket.lock().await.is_some();
    
    // Check if template connection is initialized (needed for certificate)
    let connection_initialized = connection_template.lock().await.is_some();
    
    // Check if template certificate is initialized
    let certificate_initialized = get_fingerprint_from_template(connection_template).await.is_ok();
    
    // All prerequisites must be met for the context to be ready
    let is_ready = socket_set && connection_initialized && certificate_initialized;
    
    debug!("Server security context ready: {}", is_ready);
    debug!("  - Socket set: {}", socket_set);
    debug!("  - Connection initialized: {}", connection_initialized);
    debug!("  - Certificate initialized: {}", certificate_initialized);
    
    Ok(is_ready)
}

/// Get the fingerprint from the template
pub async fn get_fingerprint_from_template(
    connection_template: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<String, SecurityError> {
    let template = connection_template.lock().await;
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
pub async fn get_fingerprint_algorithm_from_template(
    connection_template: &Arc<Mutex<Option<DtlsConnection>>>,
) -> Result<String, SecurityError> {
    // We hardcode this for now since the algorithm is set during certificate creation
    Ok("sha-256".to_string())
} 