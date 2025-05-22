//! SRTP key management
//!
//! This module handles SRTP key extraction and management for secure media.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SrtpProfile};
use crate::dtls::DtlsConnection;
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SrtpCryptoKey};
use crate::srtp::{SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM};

/// Extract SRTP keys from a DTLS connection
pub async fn extract_srtp_keys(
    conn: &DtlsConnection,
    address: SocketAddr,
    is_server: bool,
) -> Result<SrtpContext, SecurityError> {
    debug!("Extracting SRTP keys for {}", address);
    
    match conn.extract_srtp_keys() {
        Ok(srtp_ctx) => {
            // Get the appropriate key based on role
            let key = srtp_ctx.get_key_for_role(is_server).clone();
            debug!("Successfully extracted SRTP keys for {}", address);
            
            // Create SRTP context
            match SrtpContext::new(srtp_ctx.profile, key) {
                Ok(ctx) => {
                    debug!("Created SRTP context for {}", address);
                    Ok(ctx)
                },
                Err(e) => {
                    error!("Failed to create SRTP context for {}: {}", address, e);
                    Err(SecurityError::Internal(format!("Failed to create SRTP context: {}", e)))
                }
            }
        },
        Err(e) => {
            error!("Failed to extract SRTP keys for {}: {}", address, e);
            Err(SecurityError::Internal(format!("Failed to extract SRTP keys: {}", e)))
        }
    }
}

/// Convert SrtpProfile to SrtpCryptoSuite
pub fn convert_profile(profile: SrtpProfile) -> SrtpCryptoSuite {
    match profile {
        SrtpProfile::AesGcm128 => SRTP_AEAD_AES_128_GCM,
        SrtpProfile::AesGcm256 => SRTP_AEAD_AES_256_GCM,
        SrtpProfile::AesCm128HmacSha1_80 => SRTP_AES128_CM_SHA1_80,
        SrtpProfile::AesCm128HmacSha1_32 => SRTP_AES128_CM_SHA1_32,
    }
}

/// Convert a list of SrtpProfiles to SrtpCryptoSuites
pub fn convert_profiles(profiles: &[SrtpProfile]) -> Vec<SrtpCryptoSuite> {
    profiles.iter().map(|p| convert_profile(*p)).collect()
}

/// Convert u16 profile ID to SrtpCryptoSuite
pub fn profile_id_to_suite(profile_id: u16) -> SrtpCryptoSuite {
    match profile_id {
        0x0001 => SRTP_AES128_CM_SHA1_80,
        0x0002 => SRTP_AES128_CM_SHA1_32,
        0x0007 => SRTP_AEAD_AES_128_GCM,
        0x0008 => SRTP_AEAD_AES_256_GCM,
        _ => SRTP_AES128_CM_SHA1_80, // Default to AES128_CM_SHA1_80
    }
}

/// Generate a string representation of an SRTP profile
pub fn profile_to_string(profile: SrtpProfile) -> String {
    match profile {
        SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80".to_string(),
        SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32".to_string(),
        SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM".to_string(),
        SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM".to_string(),
    }
}

/// Store extracted SRTP context
pub async fn store_srtp_context(
    srtp_context: &Arc<Mutex<Option<SrtpContext>>>,
    ctx: SrtpContext,
    address: SocketAddr,
) -> Result<(), SecurityError> {
    let mut srtp_guard = srtp_context.lock().await;
    *srtp_guard = Some(ctx);
    debug!("Stored SRTP context for {}", address);
    Ok(())
} 