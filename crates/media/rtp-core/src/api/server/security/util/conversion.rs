//! Type conversion utilities
//!
//! This module provides utilities for converting between different types
//! used in the security module.

use std::net::SocketAddr;

use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};
use crate::api::common::error::SecurityError;
use crate::api::server::security::{ConnectionConfig, ConnectionRole};
use crate::dtls::{DtlsRole, DtlsVersion};

use crate::api::server::security::srtp::keys;

/// Convert ConnectionRole to DtlsRole
pub fn role_to_dtls_role(role: ConnectionRole) -> DtlsRole {
    match role {
        ConnectionRole::Client => DtlsRole::Client,
        ConnectionRole::Server => DtlsRole::Server,
    }
}

/// Convert ConnectionConfig to a DtlsConfig
pub fn connection_config_to_dtls_config(
    config: &ConnectionConfig,
) -> Result<crate::dtls::DtlsConfig, SecurityError> {
    Ok(crate::dtls::DtlsConfig {
        role: role_to_dtls_role(config.role.clone()),
        version: DtlsVersion::Dtls12, // Currently only DTLS 1.2 is supported
        mtu: 1500,                    // Default MTU
        max_retransmissions: 5,       // Default retransmissions
        srtp_profiles: keys::convert_profiles(&config.srtp_profiles)?,
    })
}

/// Create a SecurityInfo struct from mode, fingerprint, and profiles
pub fn create_security_info(
    mode: SecurityMode,
    fingerprint: Option<String>,
    fingerprint_algorithm: &str,
    profiles: &[SrtpProfile],
) -> Result<SecurityInfo, SecurityError> {
    let crypto_suites = crate::api::common::config::implemented_srtp_profile_names(profiles)?;
    let srtp_profile = crypto_suites.first().cloned();
    Ok(SecurityInfo {
        mode,
        fingerprint,
        fingerprint_algorithm: Some(fingerprint_algorithm.to_string()),
        crypto_suites,
        key_params: None,
        srtp_profile,
    })
}

/// Create an address string representation
pub fn addr_to_string(addr: SocketAddr) -> String {
    format!("{}:{}", addr.ip(), addr.port())
}

/// Convert a string to a SecurityMode enum
pub fn string_to_security_mode(mode: &str) -> Result<SecurityMode, SecurityError> {
    match mode.to_lowercase().as_str() {
        "dtls-srtp" => Ok(SecurityMode::DtlsSrtp),
        "srtp" => Ok(SecurityMode::Srtp),
        "none" => Ok(SecurityMode::None),
        _ => Err(SecurityError::UnsupportedFeature(format!(
            "unknown security mode {mode:?}"
        ))),
    }
}

/// Convert a SecurityMode enum to a string
pub fn security_mode_to_string(mode: SecurityMode) -> &'static str {
    match mode {
        SecurityMode::DtlsSrtp => "dtls_srtp",
        SecurityMode::Srtp => "srtp",
        SecurityMode::None => "none",
        SecurityMode::SdesSrtp => "sdes_srtp",
        SecurityMode::MikeySrtp => "mikey_srtp",
        SecurityMode::ZrtpSrtp => "zrtp_srtp",
    }
}

/// Create a string representation of a security context for debugging
pub fn security_context_to_string(
    address: SocketAddr,
    mode: SecurityMode,
    fingerprint: Option<&str>,
    handshake_complete: bool,
) -> String {
    let mode_str = security_mode_to_string(mode);
    let fingerprint_str = fingerprint.unwrap_or("none");
    let handshake_str = if handshake_complete {
        "complete"
    } else {
        "incomplete"
    };

    format!(
        "SecurityContext({}, mode={}, fingerprint={}, handshake={})",
        addr_to_string(address),
        mode_str,
        fingerprint_str,
        handshake_str
    )
}

/// Convert API SrtpProfile array to internal SrtpCryptoSuite array
pub fn convert_srtp_profiles(
    profiles: &[SrtpProfile],
) -> Result<Vec<crate::srtp::SrtpCryptoSuite>, SecurityError> {
    keys::convert_profiles(profiles)
}

/// Convert SrtpProfile to string
pub fn srtp_profile_to_string(profile: SrtpProfile) -> Result<String, SecurityError> {
    keys::profile_to_string(profile)
}

/// Get crypto suites as strings
pub fn get_crypto_suite_strings(profiles: &[SrtpProfile]) -> Result<Vec<String>, SecurityError> {
    profiles
        .iter()
        .map(|profile| keys::profile_to_string(*profile))
        .collect()
}
