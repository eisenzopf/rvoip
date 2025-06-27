//! Server security functionality
//!
//! This module handles security context initialization and management.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};
use std::collections::HashMap;

use crate::api::common::error::MediaTransportError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::server::security::{ServerSecurityContext, DefaultServerSecurityContext};
use crate::api::server::config::ServerConfig;

/// Initialize security context if needed
pub async fn init_security_if_needed(
    config: &ServerConfig,
    security_context: &Arc<RwLock<Option<Arc<dyn ServerSecurityContext + Send + Sync>>>>,
) -> Result<(), MediaTransportError> {
    if config.security_config.security_mode.is_enabled() {
        // Check if we already have a security context
        let security_exists = {
            let context = security_context.read().await;
            context.is_some()
        };

        if !security_exists {
            // Create security context
            let context = DefaultServerSecurityContext::new(
                config.security_config.clone(),
            ).await.map_err(|e| MediaTransportError::Security(format!("Failed to create server security context: {}", e)))?;
            
            // Store it directly - no need for extra wrapping since it should already be an Arc<dyn ServerSecurityContext>
            let mut context_write = security_context.write().await;
            *context_write = Some(context);
            
            debug!("Created server security context with mode: {:?}", config.security_config.security_mode);
        }
    }
    
    Ok(())
}

/// Get security information
pub async fn get_security_info(
    config: &ServerConfig,
    security_context: &Arc<RwLock<Option<Arc<dyn ServerSecurityContext + Send + Sync>>>>,
) -> Result<SecurityInfo, MediaTransportError> {
    // Initialize security if needed
    init_security_if_needed(config, security_context).await?;
    
    // Get security context
    let security_context_guard = security_context.read().await;
    
    if let Some(security_ctx) = security_context_guard.as_ref() {
        // Get the fingerprint and algorithm directly from the concrete context
        let fingerprint = security_ctx.get_fingerprint().await
            .map_err(|e| MediaTransportError::Security(format!("Failed to get fingerprint: {}", e)))?;
            
        let algorithm = security_ctx.get_fingerprint_algorithm().await
            .map_err(|e| MediaTransportError::Security(format!("Failed to get fingerprint algorithm: {}", e)))?;
            
        // Get supported SRTP profiles
        let profiles = security_ctx.get_supported_srtp_profiles().await;
        
        // Create crypto suites list from profiles
        let crypto_suites = profiles.iter()
            .map(|p| match p {
                SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
                SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
                SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM",
                SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM",
            })
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
            
        // Create security info
        Ok(SecurityInfo {
            mode: config.security_config.security_mode,
            fingerprint: Some(fingerprint),
            fingerprint_algorithm: Some(algorithm),
            crypto_suites,
            key_params: None,
            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()), // Default profile
        })
    } else {
        Err(MediaTransportError::Security("Security context not initialized".to_string()))
    }
}

/// Check if security is enabled
pub async fn is_security_enabled(
    config: &ServerConfig,
) -> bool {
    config.security_config.security_mode.is_enabled()
}

/// Get the security mode
pub async fn get_security_mode(
    config: &ServerConfig,
) -> SecurityMode {
    config.security_config.security_mode
}

/// Check if a client connection is secure
pub async fn is_client_secure(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, crate::api::server::transport::core::connection::ClientConnection>>>,
) -> Result<bool, MediaTransportError> {
    // Get the client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Check if security context exists
    Ok(client.security.is_some())
} 