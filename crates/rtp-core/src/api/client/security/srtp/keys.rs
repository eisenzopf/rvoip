//! SRTP key extraction and management
//!
//! This module provides functions for extracting and managing SRTP keys.

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::api::common::error::SecurityError;
use crate::api::common::config::SrtpProfile;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_NULL_NULL, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM};
use crate::srtp::crypto::SrtpCryptoKey;
use crate::dtls::DtlsConnection;

/// Extract SRTP keys from a DTLS connection
pub async fn extract_srtp_keys(
    connection: &Arc<Mutex<Option<DtlsConnection>>>,
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
    handshake_completed: &Arc<Mutex<bool>>,
) -> Result<(), SecurityError> {
    // Get the connection
    let mut conn_guard = connection.lock().await;
    
    if let Some(conn) = conn_guard.as_mut() {
        // Check if connection is in a state where we can extract SRTP keys
        if conn.state() != crate::dtls::connection::ConnectionState::Connected {
            return Err(SecurityError::HandshakeError("DTLS handshake not complete, cannot extract SRTP keys".to_string()));
        }
        
        // Extract keys from the DTLS connection
        match conn.extract_srtp_keys() {
            Ok(dtls_srtp_context) => {
                // Get the client key (true = client role)
                let client_key = dtls_srtp_context.get_key_for_role(true).clone();
                debug!("Successfully extracted SRTP keys");
                
                // Get the profile from the DTLS-SRTP context
                let profile = dtls_srtp_context.profile.clone();
                
                // Create an SRTP context using the extracted keys
                match create_srtp_context(profile, client_key) {
                    Ok(ctx) => {
                        // Store the SRTP context
                        let mut srtp_guard = srtp_context.lock().await;
                        *srtp_guard = Some(ctx);
                        
                        // Set handshake completed flag
                        let mut completed = handshake_completed.lock().await;
                        *completed = true;
                        
                        info!("DTLS handshake completed and SRTP keys extracted");
                        Ok(())
                    },
                    Err(e) => {
                        error!("Failed to create SRTP context from extracted keys: {}", e);
                        Err(e)
                    }
                }
            },
            Err(e) => {
                error!("Failed to extract SRTP keys: {}", e);
                Err(SecurityError::Handshake(format!("Failed to extract SRTP keys: {}", e)))
            }
        }
    } else {
        Err(SecurityError::NotInitialized("DTLS connection not initialized".to_string()))
    }
}

/// Convert an SrtpProfile to an SrtpCryptoSuite
pub fn profile_to_suite(profile: SrtpProfile) -> SrtpCryptoSuite {
    match profile {
        SrtpProfile::AesCm128HmacSha1_80 => SRTP_AES128_CM_SHA1_80,
        SrtpProfile::AesCm128HmacSha1_32 => SRTP_AES128_CM_SHA1_32,
        SrtpProfile::AesGcm128 => SRTP_AEAD_AES_128_GCM,
        SrtpProfile::AesGcm256 => SRTP_AEAD_AES_256_GCM,
    }
}

/// Create an SRTP context from extracted keys
pub fn create_srtp_context(
    suite: SrtpCryptoSuite,
    key: SrtpCryptoKey,
) -> Result<SrtpContext, SecurityError> {
    match SrtpContext::new(suite, key) {
        Ok(context) => Ok(context),
        Err(e) => Err(SecurityError::Internal(format!("Failed to create SRTP context: {}", e)))
    }
} 