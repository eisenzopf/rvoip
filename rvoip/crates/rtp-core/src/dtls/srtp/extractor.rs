//! DTLS-SRTP key extraction
//!
//! This module implements SRTP key extraction from DTLS handshakes.

use bytes::Bytes;
use crate::dtls::Result;
use crate::dtls::crypto::keys::{DtlsKeyingMaterial, extract_srtp_keys};
use crate::dtls::message::extension::SrtpProtectionProfile;
use crate::srtp::{SrtpCryptoSuite, SrtpCryptoKey, SrtpEncryptionAlgorithm, SrtpAuthenticationAlgorithm};

/// DTLS-SRTP context after key extraction
#[derive(Debug, Clone)]
pub struct DtlsSrtpContext {
    /// Negotiated SRTP protection profile
    pub profile: SrtpCryptoSuite,
    
    /// Client write key (used by the client to encrypt data)
    pub client_write_key: SrtpCryptoKey,
    
    /// Server write key (used by the server to encrypt data)
    pub server_write_key: SrtpCryptoKey,
}

impl DtlsSrtpContext {
    /// Create a new DTLS-SRTP context
    pub fn new(
        profile: SrtpCryptoSuite,
        client_write_key: SrtpCryptoKey,
        server_write_key: SrtpCryptoKey,
    ) -> Self {
        Self {
            profile,
            client_write_key,
            server_write_key,
        }
    }
    
    /// Get the appropriate key for a given role
    pub fn get_key_for_role(&self, is_client: bool) -> &SrtpCryptoKey {
        if is_client {
            &self.client_write_key
        } else {
            &self.server_write_key
        }
    }
}

/// Extract SRTP keys from DTLS keying material
pub fn extract_srtp_keys_from_dtls(
    keying_material: &DtlsKeyingMaterial,
    profile: SrtpProtectionProfile,
    is_client: bool,
) -> Result<DtlsSrtpContext> {
    // Convert DTLS protection profile to SRTP crypto suite
    let crypto_suite = convert_profile_to_suite(profile)?;
    
    // Get key and salt lengths based on the crypto suite
    let key_length = crypto_suite.key_length;
    let salt_length = 14; // SRTP standard salt length
    
    // Extract client and server keys
    let (client_key, client_salt) = extract_srtp_keys(
        keying_material,
        key_length,
        salt_length,
        true,  // Client perspective
    )?;
    
    let (server_key, server_salt) = extract_srtp_keys(
        keying_material,
        key_length,
        salt_length,
        false, // Server perspective
    )?;
    
    // Create the SRTP crypto keys
    let client_write_key = SrtpCryptoKey::new(client_key.to_vec(), client_salt.to_vec());
    let server_write_key = SrtpCryptoKey::new(server_key.to_vec(), server_salt.to_vec());
    
    // Create and return the DTLS-SRTP context
    Ok(DtlsSrtpContext::new(
        crypto_suite,
        client_write_key,
        server_write_key,
    ))
}

/// Convert a DTLS SRTP protection profile to an SRTP crypto suite
fn convert_profile_to_suite(profile: SrtpProtectionProfile) -> Result<SrtpCryptoSuite> {
    match profile {
        SrtpProtectionProfile::Aes128CmSha1_80 => {
            Ok(SrtpCryptoSuite {
                encryption: SrtpEncryptionAlgorithm::AesCm,
                authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
                key_length: 16, // 128 bits
                tag_length: 10, // 80 bits
            })
        }
        SrtpProtectionProfile::Aes128CmSha1_32 => {
            Ok(SrtpCryptoSuite {
                encryption: SrtpEncryptionAlgorithm::AesCm,
                authentication: SrtpAuthenticationAlgorithm::HmacSha1_32,
                key_length: 16, // 128 bits
                tag_length: 4,  // 32 bits
            })
        }
        SrtpProtectionProfile::AeadAes128Gcm => {
            // This would require adding an AEAD encryption algorithm to the SRTP module
            Err(crate::error::Error::UnsupportedFeature("AEAD GCM crypto suite not yet supported".to_string()))
        }
        SrtpProtectionProfile::AeadAes256Gcm => {
            // This would require adding an AEAD encryption algorithm to the SRTP module
            Err(crate::error::Error::UnsupportedFeature("AEAD GCM crypto suite not yet supported".to_string()))
        }
        SrtpProtectionProfile::Unknown(_) => {
            Err(crate::error::Error::UnsupportedFeature("Unknown SRTP protection profile".to_string()))
        }
    }
} 