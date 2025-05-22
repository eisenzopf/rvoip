//! Type conversion utilities
//!
//! This module provides utilities for converting between different types.

use crate::api::common::config::{SecurityMode, SrtpProfile};
use crate::dtls::{DtlsRole};
use crate::api::server::security::{ConnectionRole};

/// Convert ConnectionRole to DtlsRole
pub fn connection_role_to_dtls_role(role: ConnectionRole) -> DtlsRole {
    // This function will be fully implemented in Phase 6
    todo!("Implement connection_role_to_dtls_role in Phase 6")
}

/// Convert API SrtpProfile array to internal SrtpCryptoSuite array
pub fn convert_srtp_profiles(profiles: &[SrtpProfile]) -> Vec<crate::srtp::SrtpCryptoSuite> {
    // This function will be fully implemented in Phase 6
    todo!("Implement convert_srtp_profiles in Phase 6")
}

/// Convert SrtpProfile to string
pub fn srtp_profile_to_string(profile: SrtpProfile) -> String {
    // This function will be fully implemented in Phase 6
    todo!("Implement srtp_profile_to_string in Phase 6")
}

/// Get crypto suites as strings
pub fn get_crypto_suite_strings(profiles: &[SrtpProfile]) -> Vec<String> {
    // This function will be fully implemented in Phase 6
    todo!("Implement get_crypto_suite_strings in Phase 6")
} 