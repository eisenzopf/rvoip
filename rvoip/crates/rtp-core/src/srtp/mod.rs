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

/// SRTP context for a session
pub struct SrtpContext {
    /// Whether encryption is enabled
    enabled: bool,
    
    /// SRTP crypto context
    crypto: crypto::SrtpCrypto,
    
    /// Key rotation frequency
    key_rotation: key_derivation::KeyRotationFrequency,
    
    /// Current packet index (sequence number + rollover counter)
    packet_index: u64,
}

/// Protected RTP packet with authentication tag
pub struct ProtectedRtpPacket {
    /// The encrypted RTP packet
    pub packet: crate::packet::RtpPacket,
    
    /// Authentication tag (if used)
    pub auth_tag: Option<Vec<u8>>,
}

impl ProtectedRtpPacket {
    /// Serialize the protected packet with its authentication tag
    pub fn serialize(&self) -> Result<bytes::Bytes, crate::Error> {
        let packet_bytes = self.packet.serialize()?;
        
        if let Some(tag) = &self.auth_tag {
            // Combine packet and authentication tag
            let mut buffer = bytes::BytesMut::with_capacity(packet_bytes.len() + tag.len());
            buffer.extend_from_slice(&packet_bytes);
            buffer.extend_from_slice(tag);
            Ok(buffer.freeze())
        } else {
            // No authentication tag
            Ok(packet_bytes)
        }
    }
}

impl SrtpContext {
    /// Create a new SRTP context
    pub fn new(suite: SrtpCryptoSuite, key: crypto::SrtpCryptoKey) -> Result<Self, crate::Error> {
        let crypto = crypto::SrtpCrypto::new(suite, key)?;
        
        Ok(Self {
            enabled: true,
            crypto,
            key_rotation: key_derivation::KeyRotationFrequency::None,
            packet_index: 0,
        })
    }
    
    /// Enable or disable SRTP
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    
    /// Set key rotation frequency
    pub fn set_key_rotation(&mut self, frequency: key_derivation::KeyRotationFrequency) {
        self.key_rotation = frequency;
    }
    
    /// Protect an RTP packet (SRTP encryption)
    /// Returns the encrypted packet with its authentication tag
    pub fn protect(&mut self, packet: &crate::packet::RtpPacket) -> Result<ProtectedRtpPacket, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Ok(ProtectedRtpPacket {
                packet: packet.clone(),
                auth_tag: None,
            });
        }
        
        // Check for key rotation
        if self.key_rotation.should_rotate(self.packet_index) {
            // In a real implementation, we would rotate keys here
        }
        
        // Increment packet index
        self.packet_index += 1;
        
        // Encrypt the packet using SRTP
        let (encrypted, auth_tag) = self.crypto.encrypt_rtp(packet)?;
        
        // Return the encrypted packet with its authentication tag
        Ok(ProtectedRtpPacket {
            packet: encrypted,
            auth_tag,
        })
    }
    
    /// Unprotect an RTP packet (SRTP decryption)
    /// The input data should include the authentication tag if used
    pub fn unprotect(&mut self, data: &[u8]) -> Result<crate::packet::RtpPacket, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return crate::packet::RtpPacket::parse(data);
        }
        
        // Decrypt using SRTP (which handles authentication verification internally)
        self.crypto.decrypt_rtp(data)
    }
    
    /// Protect an RTCP packet (SRTCP encryption)
    /// Returns the encrypted data with the authentication tag appended
    pub fn protect_rtcp(&mut self, data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Ok(bytes::Bytes::copy_from_slice(data));
        }
        
        // Encrypt using SRTCP
        let (encrypted, auth_tag) = self.crypto.encrypt_rtcp(data)?;
        
        // If authentication is used, append the tag
        if let Some(tag) = auth_tag {
            let mut buffer = bytes::BytesMut::with_capacity(encrypted.len() + tag.len());
            buffer.extend_from_slice(&encrypted);
            buffer.extend_from_slice(&tag);
            Ok(buffer.freeze())
        } else {
            Ok(encrypted)
        }
    }
    
    /// Unprotect an RTCP packet (SRTCP decryption)
    /// The input data should include the authentication tag if used
    pub fn unprotect_rtcp(&mut self, data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Ok(bytes::Bytes::copy_from_slice(data));
        }
        
        // Decrypt using SRTCP (which handles authentication verification internally)
        self.crypto.decrypt_rtcp(data)
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
        );
        
        assert!(context.is_ok());
    }
} 