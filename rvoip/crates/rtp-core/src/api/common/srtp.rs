//! SRTP Configuration
//!
//! This module provides API types and utilities for SRTP (Secure RTP) configuration.

use crate::srtp::{
    SrtpCryptoSuite, SrtpCryptoKey, SrtpEncryptionAlgorithm, SrtpAuthenticationAlgorithm,
    SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32, SRTP_AEAD_AES_128_GCM, SRTP_AEAD_AES_256_GCM,
    SRTP_NULL_NULL
};

use crate::api::common::config::SrtpProfile;
use crate::api::common::error::MediaTransportError;

/// SRTP configuration for the API layer
#[derive(Debug, Clone)]
pub struct SrtpConfig {
    /// SRTP profile to use
    pub profile: SrtpProfile,
    
    /// Master key (if using pre-shared keys)
    pub master_key: Option<Vec<u8>>,
    
    /// Master salt (if using pre-shared keys)
    pub master_salt: Option<Vec<u8>>,
    
    /// Key derivation rate (0 means no rekeying)
    pub key_derivation_rate: u64,
    
    /// Allow unprotected SRTP (for interoperability with unsecured endpoints)
    pub allow_unprotected: bool,
}

impl Default for SrtpConfig {
    fn default() -> Self {
        Self {
            profile: SrtpProfile::AesCm128HmacSha1_80,
            master_key: None,
            master_salt: None,
            key_derivation_rate: 0,
            allow_unprotected: false,
        }
    }
}

impl SrtpConfig {
    /// Create an SRTP configuration with the specified profile
    pub fn with_profile(profile: SrtpProfile) -> Self {
        Self {
            profile,
            ..Default::default()
        }
    }
    
    /// Set the master key and salt
    pub fn with_key_material(mut self, key: Vec<u8>, salt: Vec<u8>) -> Self {
        self.master_key = Some(key);
        self.master_salt = Some(salt);
        self
    }
    
    /// Set the key derivation rate
    pub fn with_key_derivation_rate(mut self, rate: u64) -> Self {
        self.key_derivation_rate = rate;
        self
    }
    
    /// Allow unprotected SRTP (for interoperability)
    pub fn allow_unprotected(mut self, allow: bool) -> Self {
        self.allow_unprotected = allow;
        self
    }
    
    /// Convert API SrtpProfile to internal SrtpCryptoSuite
    pub fn to_crypto_suite(&self) -> SrtpCryptoSuite {
        match self.profile {
            SrtpProfile::AesCm128HmacSha1_80 => SRTP_AES128_CM_SHA1_80,
            SrtpProfile::AesCm128HmacSha1_32 => SRTP_AES128_CM_SHA1_32,
            SrtpProfile::AesGcm128 => SRTP_AEAD_AES_128_GCM,
            SrtpProfile::AesGcm256 => SRTP_AEAD_AES_256_GCM,
        }
    }
    
    /// Create an SrtpCryptoKey from this configuration
    pub fn to_crypto_key(&self) -> Result<SrtpCryptoKey, MediaTransportError> {
        match (&self.master_key, &self.master_salt) {
            (Some(key), Some(salt)) => {
                let key_clone = key.clone();
                let salt_clone = salt.clone();
                Ok(SrtpCryptoKey::new(key_clone, salt_clone))
            },
            _ => Err(MediaTransportError::ConfigError(
                "Missing SRTP master key or salt".to_string()
            )),
        }
    }
    
    /// Create a base64 encoded key+salt for SDP (RFC 4568)
    pub fn to_base64_keysalt(&self) -> Result<String, MediaTransportError> {
        match (&self.master_key, &self.master_salt) {
            (Some(key), Some(salt)) => {
                let mut combined = Vec::with_capacity(key.len() + salt.len());
                combined.extend_from_slice(key);
                combined.extend_from_slice(salt);
                Ok(base64::encode(&combined))
            },
            _ => Err(MediaTransportError::ConfigError(
                "Missing SRTP master key or salt".to_string()
            )),
        }
    }
    
    /// Parse a base64 encoded key+salt (as used in SDP)
    pub fn from_base64(data: &str) -> Result<Self, MediaTransportError> {
        let decoded = base64::decode(data)
            .map_err(|e| MediaTransportError::ConfigError(
                format!("Failed to decode base64 key: {}", e)
            ))?;
        
        // Typical format is 30 bytes = 16 bytes key + 14 bytes salt
        if decoded.len() < 16 {
            return Err(MediaTransportError::ConfigError(
                "Key material too short".to_string()
            ));
        }
        
        // Split into key and salt
        let key = decoded[0..16].to_vec();
        let salt = if decoded.len() > 16 {
            decoded[16..].to_vec()
        } else {
            Vec::new()
        };
        
        Ok(Self {
            profile: SrtpProfile::AesCm128HmacSha1_80, // Default profile
            master_key: Some(key),
            master_salt: Some(salt),
            key_derivation_rate: 0,
            allow_unprotected: false,
        })
    }
    
    /// Get the crypto suite name for SDP (RFC 4568)
    pub fn get_crypto_name(&self) -> &'static str {
        match self.profile {
            SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
            SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
            SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM",
            SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM",
        }
    }
    
    /// Create SDP crypto line (RFC 4568)
    pub fn to_sdp_crypto_line(&self) -> Result<String, MediaTransportError> {
        let base64_key = self.to_base64_keysalt()?;
        let crypto_name = self.get_crypto_name();
        
        if self.key_derivation_rate > 0 {
            Ok(format!("1 {} inline:{} KDR={}", crypto_name, base64_key, self.key_derivation_rate))
        } else {
            Ok(format!("1 {} inline:{}", crypto_name, base64_key))
        }
    }
    
    /// Parse SDP crypto line (RFC 4568)
    pub fn from_sdp_crypto_line(line: &str) -> Result<Self, MediaTransportError> {
        // Simplified parsing for crypto line, e.g.:
        // "1 AES_CM_128_HMAC_SHA1_80 inline:d0RmdmcmVCspeEc3QGZiNWpVLFJhQX1cfHAwJSoj|2^20|1:32"
        
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            return Err(MediaTransportError::ConfigError(
                "Invalid crypto line format".to_string()
            ));
        }
        
        // Get profile
        let profile = match parts[1] {
            "AES_CM_128_HMAC_SHA1_80" => SrtpProfile::AesCm128HmacSha1_80,
            "AES_CM_128_HMAC_SHA1_32" => SrtpProfile::AesCm128HmacSha1_32,
            "AEAD_AES_128_GCM" => SrtpProfile::AesGcm128,
            "AEAD_AES_256_GCM" => SrtpProfile::AesGcm256,
            _ => return Err(MediaTransportError::ConfigError(
                format!("Unsupported crypto suite: {}", parts[1])
            )),
        };
        
        // Parse key info
        let key_parts: Vec<&str> = parts[2].split(':').collect();
        if key_parts.len() < 2 || key_parts[0] != "inline" {
            return Err(MediaTransportError::ConfigError(
                "Invalid key format".to_string()
            ));
        }
        
        // Extract base64 key
        let base64_key = key_parts[1].split('|').next().unwrap_or("");
        
        // Parse KDR if present
        let mut kdr = 0;
        for part in &parts[3..] {
            if part.starts_with("KDR=") {
                if let Ok(value) = part[4..].parse::<u64>() {
                    kdr = value;
                }
            }
        }
        
        // Create config from base64 key
        let mut config = Self::from_base64(base64_key)?;
        config.profile = profile;
        config.key_derivation_rate = kdr;
        
        Ok(config)
    }
}

/// Information about an SRTP context
#[derive(Debug, Clone)]
pub struct SrtpContextInfo {
    /// The profile being used
    pub profile: SrtpProfile,
    
    /// Algorithm name as string
    pub algorithm_name: String,
    
    /// Key size in bytes
    pub key_size: usize,
    
    /// Salt size in bytes
    pub salt_size: usize,
    
    /// Whether the context is ready for use
    pub is_ready: bool,
}

/// Create an SRTP context from configuration
/// 
/// This is intentionally a free function rather than a method to maintain proper API boundaries.
/// It hides the internal SRTP implementation and returns the raw SrtpContext that the existing
/// code uses. Eventually, we should create a proper abstraction layer with interfaces.
pub fn create_srtp_context(config: &SrtpConfig) -> Result<crate::srtp::SrtpContext, MediaTransportError> {
    // Create crypto suite from profile
    let suite = config.to_crypto_suite();
    
    // Create crypto key from master key/salt
    let key = config.to_crypto_key()?;
    
    // Create SRTP context
    crate::srtp::SrtpContext::new(suite, key)
        .map_err(|e| MediaTransportError::Security(format!("Failed to create SRTP context: {}", e)))
}

/// Get information about an SRTP context
/// 
/// This is a utility function to extract displayable information from an internal SrtpContext
/// without exposing its implementation details.
pub fn get_srtp_context_info(context: &crate::srtp::SrtpContext) -> SrtpContextInfo {
    // Determine the profile based on parameters
    let (profile, algorithm_name) = match context.get_crypto_params() {
        (alg, auth) => {
            match (alg, auth) {
                (SrtpEncryptionAlgorithm::AesCm128, SrtpAuthenticationAlgorithm::HmacSha1_80) => 
                    (SrtpProfile::AesCm128HmacSha1_80, "AES_CM_128_HMAC_SHA1_80"),
                (SrtpEncryptionAlgorithm::AesCm128, SrtpAuthenticationAlgorithm::HmacSha1_32) => 
                    (SrtpProfile::AesCm128HmacSha1_32, "AES_CM_128_HMAC_SHA1_32"),
                (SrtpEncryptionAlgorithm::AesGcm128, _) => 
                    (SrtpProfile::AesGcm128, "AEAD_AES_128_GCM"),
                (SrtpEncryptionAlgorithm::AesGcm256, _) => 
                    (SrtpProfile::AesGcm256, "AEAD_AES_256_GCM"),
                _ => (SrtpProfile::AesCm128HmacSha1_80, "UNKNOWN"),
            }
        }
    };
    
    // Extract key and salt information
    let key_size = context.get_key_size();
    let salt_size = context.get_salt_size();
    
    SrtpContextInfo {
        profile,
        algorithm_name: algorithm_name.to_string(),
        key_size,
        salt_size,
        is_ready: key_size > 0 && salt_size > 0,
    }
} 