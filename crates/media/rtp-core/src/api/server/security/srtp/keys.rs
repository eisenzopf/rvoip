//! SRTP key management
//!
//! This module handles SRTP key extraction and management for secure media.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error};

use crate::api::common::config::SrtpProfile;
use crate::api::common::error::SecurityError;
use crate::dtls::DtlsConnection;
use crate::srtp::{SrtpContext, SrtpCryptoSuite};
use crate::srtp::{SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80};

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
                }
                Err(e) => {
                    error!("Failed to create SRTP context for {}: {}", address, e);
                    Err(SecurityError::Internal(format!(
                        "Failed to create SRTP context: {}",
                        e
                    )))
                }
            }
        }
        Err(e) => {
            error!("Failed to extract SRTP keys for {}: {}", address, e);
            Err(SecurityError::Internal(format!(
                "Failed to extract SRTP keys: {}",
                e
            )))
        }
    }
}

/// Convert SrtpProfile to SrtpCryptoSuite
pub fn convert_profile(profile: SrtpProfile) -> Result<SrtpCryptoSuite, SecurityError> {
    match profile {
        SrtpProfile::AesCm128HmacSha1_80 => Ok(SRTP_AES128_CM_SHA1_80),
        SrtpProfile::AesCm128HmacSha1_32 => Ok(SRTP_AES128_CM_SHA1_32),
        SrtpProfile::AesGcm128 | SrtpProfile::AesGcm256 => Err(SecurityError::UnsupportedFeature(
            format!("SRTP profile {profile:?} is not implemented"),
        )),
    }
}

/// Convert a list of SrtpProfiles to SrtpCryptoSuites
pub fn convert_profiles(profiles: &[SrtpProfile]) -> Result<Vec<SrtpCryptoSuite>, SecurityError> {
    profiles
        .iter()
        .map(|profile| convert_profile(*profile))
        .collect()
}

/// Convert u16 profile ID to SrtpCryptoSuite
pub fn profile_id_to_suite(profile_id: u16) -> Result<SrtpCryptoSuite, SecurityError> {
    match profile_id {
        0x0001 => Ok(SRTP_AES128_CM_SHA1_80),
        0x0002 => Ok(SRTP_AES128_CM_SHA1_32),
        0x0007 | 0x0008 => Err(SecurityError::UnsupportedFeature(format!(
            "DTLS-SRTP profile 0x{profile_id:04x} is not implemented"
        ))),
        _ => Err(SecurityError::UnsupportedFeature(format!(
            "unknown DTLS-SRTP profile 0x{profile_id:04x}"
        ))),
    }
}

/// Generate a string representation of an SRTP profile
pub fn profile_to_string(profile: SrtpProfile) -> Result<String, SecurityError> {
    profile
        .advertised_name()
        .map(str::to_string)
        .map_err(SecurityError::from)
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
