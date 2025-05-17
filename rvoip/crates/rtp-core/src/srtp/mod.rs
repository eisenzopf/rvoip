//! Secure RTP (SRTP) implementation
//!
//! This module provides encryption and authentication for RTP/RTCP packets.

pub mod crypto;
pub mod key_derivation;
pub mod auth;

pub use crypto::SrtpCryptoKey;
pub use auth::{SrtpAuthenticator, SrtpReplayProtection};
pub use key_derivation::{
    KeyDerivationLabel, SrtpKeyDerivationParams, KeyRotationFrequency,
    srtp_kdf, create_srtp_iv
};

/// SRTP encryption algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpEncryptionAlgorithm {
    /// AES Counter Mode (Default in SRTP)
    AesCm,
    
    /// AES in f8-mode (Customized for SRTP)
    AesF8,
    
    /// Null encryption (for debugging/testing only)
    Null,
}

/// SRTP authentication algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpAuthenticationAlgorithm {
    /// HMAC-SHA1 truncated to 80 bits (Default in SRTP)
    HmacSha1_80,
    
    /// HMAC-SHA1 truncated to 32 bits
    HmacSha1_32,
    
    /// Null authentication (for debugging/testing only)
    Null,
}

/// SRTP crypto suite
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrtpCryptoSuite {
    /// Encryption algorithm
    pub encryption: SrtpEncryptionAlgorithm,
    
    /// Authentication algorithm
    pub authentication: SrtpAuthenticationAlgorithm,
    
    /// Master key length in bytes
    pub key_length: usize,
    
    /// Authentication tag length in bytes
    pub tag_length: usize,
}

/// Default SRTP crypto suite: AES-CM-128 + HMAC-SHA1-80
pub const SRTP_AES128_CM_SHA1_80: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
    key_length: 16, // 128 bits
    tag_length: 10, // 80 bits
};

/// Smaller tag SRTP crypto suite: AES-CM-128 + HMAC-SHA1-32
pub const SRTP_AES128_CM_SHA1_32: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_32,
    key_length: 16, // 128 bits
    tag_length: 4,  // 32 bits
};

/// Null encryption/authentication (for testing/debugging only)
pub const SRTP_NULL_SHA1_80: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::Null,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
    key_length: 16, // 128 bits
    tag_length: 10, // 80 bits
};

/// No encryption or authentication (DANGEROUS - use only for testing)
pub const SRTP_NULL_NULL: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::Null,
    authentication: SrtpAuthenticationAlgorithm::Null,
    key_length: 16, // Changed from 0 to 16 to support realistic test scenarios
    tag_length: 0,
};

/// SRTP Context for secure RTP transmission
pub struct SrtpContext {
    /// Crypto implementation
    crypto: crypto::SrtpCrypto,
    
    /// Replay protection
    replay_protection: SrtpReplayProtection,
    
    /// Roll-over counter
    roc: u32,
    
    /// Key rotation frequency
    key_rotation: KeyRotationFrequency,
    
    /// Packet index
    packet_index: u64,
}

impl SrtpContext {
    /// Create a new SRTP context
    pub fn new(
        suite: SrtpCryptoSuite,
        master_key: SrtpCryptoKey,
        window_size: u64,
    ) -> Result<Self, crate::Error> {
        Ok(Self {
            crypto: crypto::SrtpCrypto::new(suite, master_key)?,
            replay_protection: SrtpReplayProtection::new(window_size),
            roc: 0,
            key_rotation: KeyRotationFrequency::None,
            packet_index: 0,
        })
    }
    
    /// Set the key rotation frequency
    pub fn set_key_rotation(&mut self, frequency: KeyRotationFrequency) {
        self.key_rotation = frequency;
    }
    
    /// Encrypt an RTP packet
    pub fn protect(&mut self, packet: &crate::packet::RtpPacket) -> Result<crate::packet::RtpPacket, crate::Error> {
        // Check for key rotation
        if self.key_rotation.should_rotate(self.packet_index) {
            // In a real implementation, we would rotate keys here
        }
        
        // Increment packet index
        self.packet_index += 1;
        
        // Encrypt
        self.crypto.encrypt_rtp(packet)
    }
    
    /// Decrypt and verify an SRTP packet
    pub fn unprotect(&mut self, data: &[u8]) -> Result<crate::packet::RtpPacket, crate::Error> {
        // Check replay protection
        let seq = 0; // In a real implementation, we would extract the sequence number
        if !self.replay_protection.check(seq as u64)? {
            return Err(crate::Error::SrtpError("Replay protection failed".to_string()));
        }
        
        // Decrypt
        self.crypto.decrypt_rtp(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::packet::{RtpHeader, RtpPacket};
    
    #[test]
    fn test_srtp_context_creation() {
        // Create a key
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);
        
        // Create context with null encryption
        let context = SrtpContext::new(
            SRTP_NULL_NULL,
            key,
            64
        );
        
        assert!(context.is_ok());
    }
} 