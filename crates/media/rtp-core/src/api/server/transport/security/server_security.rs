//! Server security functionality
//!
//! This module handles security context initialization and management.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::api::common::config::SecurityInfo;
use crate::api::common::error::MediaTransportError;
use crate::api::server::config::ServerConfig;
use crate::api::server::security::{DefaultServerSecurityContext, ServerSecurityContext};

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
            let context = DefaultServerSecurityContext::new(config.security_config.clone())
                .await
                .map_err(|e| {
                    MediaTransportError::Security(format!(
                        "Failed to create server security context: {}",
                        e
                    ))
                })?;

            // Store it directly - no need for extra wrapping since it should already be an Arc<dyn ServerSecurityContext>
            let mut context_write = security_context.write().await;
            *context_write = Some(context);

            debug!(
                "Created server security context with mode: {:?}",
                config.security_config.security_mode
            );
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
        let fingerprint = security_ctx.get_fingerprint().await.map_err(|e| {
            MediaTransportError::Security(format!("Failed to get fingerprint: {}", e))
        })?;

        let algorithm = security_ctx
            .get_fingerprint_algorithm()
            .await
            .map_err(|e| {
                MediaTransportError::Security(format!("Failed to get fingerprint algorithm: {}", e))
            })?;

        // Get supported SRTP profiles
        let profiles = security_ctx.get_supported_srtp_profiles().await;

        // Create crypto suites list from profiles
        let crypto_suites = profiles
            .iter()
            .filter_map(|profile| profile.advertised_name().ok())
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let srtp_profile = crypto_suites.first().cloned();

        // Create security info
        Ok(SecurityInfo {
            mode: config.security_config.security_mode,
            fingerprint: Some(fingerprint),
            fingerprint_algorithm: Some(algorithm),
            crypto_suites,
            key_params: None,
            srtp_profile,
        })
    } else {
        Err(MediaTransportError::Security(
            "Security context not initialized".to_string(),
        ))
    }
}
