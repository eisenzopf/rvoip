//! Secure RTP (SRTP) implementation
//!
//! This module provides encryption and authentication for RTP/RTCP packets.

pub mod auth;
pub mod crypto;
pub mod key_derivation;

pub use auth::{SrtpAuthenticator, SrtpReplayProtection};
pub use crypto::SrtpCryptoKey;
pub use key_derivation::{
    create_srtp_iv, srtp_kdf, KeyDerivationLabel, KeyRotationFrequency, SrtpKeyDerivationParams,
};

/// SRTP encryption algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpEncryptionAlgorithm {
    /// AES Counter Mode (Default in SRTP)
    AesCm,

    /// AES in f8-mode (Customized for SRTP)
    AesF8,

    /// Retained NULL-encryption identity. Construction is unsupported in 0.3.5.
    Null,

    /// AEAD AES-128-GCM (RFC 7714).
    ///
    /// The profile identity is retained so configuration and negotiation can
    /// fail closed. Encryption is not implemented in this release.
    AeadAes128Gcm,

    /// AEAD AES-256-GCM (RFC 7714).
    ///
    /// The profile identity is retained so configuration and negotiation can
    /// fail closed. Encryption is not implemented in this release.
    AeadAes256Gcm,
}

/// SRTP authentication algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpAuthenticationAlgorithm {
    /// HMAC-SHA1 truncated to 80 bits (Default in SRTP)
    HmacSha1_80,

    /// HMAC-SHA1 truncated to 32 bits
    HmacSha1_32,

    /// Retained NULL-authentication identity. Construction is unsupported in 0.3.5.
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

/// AES-CM-256 + HMAC-SHA1-80 (RFC 6188)
pub const SRTP_AES256_CM_SHA1_80: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
    key_length: 32, // 256 bits
    tag_length: 10, // 80 bits
};

/// AES-CM-256 + HMAC-SHA1-32 (RFC 6188)
pub const SRTP_AES256_CM_SHA1_32: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AesCm,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_32,
    key_length: 32, // 256 bits
    tag_length: 4,  // 32 bits
};

/// Integrity-only NULL-encryption profile identity.
///
/// Retained for source compatibility. It provides no confidentiality and is
/// rejected by `SrtpCryptoSuite::validate`, `SrtpCrypto`, and `SrtpContext` in
/// 0.3.5.
pub const SRTP_NULL_SHA1_80: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::Null,
    authentication: SrtpAuthenticationAlgorithm::HmacSha1_80,
    key_length: 16, // 128 bits
    tag_length: 10, // 80 bits
};

/// Unprotected NULL/NULL profile identity.
///
/// Retained for source compatibility only. It is rejected by
/// `SrtpCryptoSuite::validate`, `SrtpCrypto`, and `SrtpContext`; it can never
/// be installed as a secure production transport.
pub const SRTP_NULL_NULL: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::Null,
    authentication: SrtpAuthenticationAlgorithm::Null,
    key_length: 16, // Changed from 0 to 16 to support realistic test scenarios
    tag_length: 0,
};

/// AEAD AES-128 GCM
pub const SRTP_AEAD_AES_128_GCM: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AeadAes128Gcm,
    authentication: SrtpAuthenticationAlgorithm::Null, // Authentication is part of AEAD
    key_length: 16,                                    // 128 bits
    tag_length: 16,                                    // 128 bits for GCM
};

/// AEAD AES-256 GCM
pub const SRTP_AEAD_AES_256_GCM: SrtpCryptoSuite = SrtpCryptoSuite {
    encryption: SrtpEncryptionAlgorithm::AeadAes256Gcm,
    authentication: SrtpAuthenticationAlgorithm::Null, // Authentication is part of AEAD
    key_length: 32,                                    // 256 bits
    tag_length: 16,                                    // 128 bits for GCM
};

impl SrtpCryptoSuite {
    /// Validate that the suite is one of the four reviewed AES-CM/HMAC suites.
    ///
    /// The HMAC-SHA1 authentication tag is truncated from a fixed 20-byte
    /// digest, so any HMAC-SHA1 suite must declare `tag_length <= 20`.
    /// Public identities for AES-GCM, AES-f8, and NULL modes remain available
    /// for source compatibility but fail closed here. Exact matching also
    /// prevents a hand-constructed literal from creating an unreviewed
    /// encryption/authentication combination that a transport could report as
    /// secure.
    pub fn validate(&self) -> Result<(), crate::Error> {
        const HMAC_SHA1_OUTPUT_LEN: usize = 20;
        if matches!(
            self.encryption,
            SrtpEncryptionAlgorithm::AeadAes128Gcm | SrtpEncryptionAlgorithm::AeadAes256Gcm
        ) {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP AEAD AES-GCM profiles are not implemented".to_string(),
            ));
        }

        if self.encryption == SrtpEncryptionAlgorithm::AesF8 {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP AES-f8 is not implemented".to_string(),
            ));
        }

        if self.encryption == SrtpEncryptionAlgorithm::Null {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP NULL encryption profiles are retained identities but are unavailable because they provide no confidentiality"
                    .to_string(),
            ));
        }

        if self.authentication == SrtpAuthenticationAlgorithm::Null {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP NULL authentication is unavailable for AES-CM profiles".to_string(),
            ));
        }

        if !matches!(self.key_length, 16 | 32) {
            return Err(crate::Error::SrtpError(format!(
                "unsupported SRTP master key length: {} bytes",
                self.key_length
            )));
        }

        let expected_tag_length = match self.authentication {
            SrtpAuthenticationAlgorithm::HmacSha1_80 => 10,
            SrtpAuthenticationAlgorithm::HmacSha1_32 => 4,
            SrtpAuthenticationAlgorithm::Null => 0,
        };
        if self.tag_length != expected_tag_length {
            return Err(crate::Error::SrtpError(format!(
                "SRTP authentication mode requires a {expected_tag_length}-byte tag, got {}",
                self.tag_length
            )));
        }
        if self.tag_length > HMAC_SHA1_OUTPUT_LEN {
            return Err(crate::Error::SrtpError(format!(
                "SRTP suite tag_length {} exceeds HMAC-SHA1 output {}",
                self.tag_length, HMAC_SHA1_OUTPUT_LEN
            )));
        }

        if self != &SRTP_AES128_CM_SHA1_80
            && self != &SRTP_AES128_CM_SHA1_32
            && self != &SRTP_AES256_CM_SHA1_80
            && self != &SRTP_AES256_CM_SHA1_32
        {
            return Err(crate::Error::UnsupportedFeature(
                "unreviewed SRTP suite combination is unavailable; use an exact AES-CM/HMAC-SHA1 built-in profile"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

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
        self.serialize_into(&mut bytes::BytesMut::new())
    }

    /// Serialize the protected packet into a caller-owned scratch buffer.
    pub fn serialize_into(
        &self,
        buffer: &mut bytes::BytesMut,
    ) -> Result<bytes::Bytes, crate::Error> {
        buffer.clear();
        buffer.reserve(self.packet.size() + self.auth_tag.as_ref().map_or(0, Vec::len));

        self.packet.header.serialize(buffer)?;
        buffer.extend_from_slice(&self.packet.payload);
        if !self.packet.header.padding && self.packet.padding_size != 0 {
            return Err(crate::Error::InvalidParameter(format!(
                "RTP padding flag ({}) does not match padding length ({})",
                self.packet.header.padding, self.packet.padding_size
            )));
        }
        if self.packet.header.padding
            && self.packet.padding_size == 0
            && self.packet.payload.is_empty()
        {
            return Err(crate::Error::InvalidParameter(
                "protected RTP padding flag is set but the encrypted payload is empty".to_string(),
            ));
        }
        if self.packet.padding_size != 0 {
            for _ in 1..self.packet.padding_size {
                buffer.extend_from_slice(&[0]);
            }
            buffer.extend_from_slice(&[self.packet.padding_size]);
        }
        if let Some(tag) = &self.auth_tag {
            buffer.extend_from_slice(tag);
        }

        Ok(buffer.split().freeze())
    }
}

impl SrtpContext {
    /// Create a new SRTP context
    pub fn new(suite: SrtpCryptoSuite, key: crypto::SrtpCryptoKey) -> Result<Self, crate::Error> {
        suite.validate()?;
        let crypto = crypto::SrtpCrypto::new(suite, key)?;

        Ok(Self {
            enabled: true,
            crypto,
            key_rotation: key_derivation::KeyRotationFrequency::None,
            packet_index: 0,
        })
    }

    /// Create a new SRTP context from separate local and remote keys
    pub fn new_from_keys(
        local_key: Vec<u8>,
        _remote_key: Vec<u8>,
        local_salt: Vec<u8>,
        _remote_salt: Vec<u8>,
        profile: SrtpCryptoSuite,
    ) -> Result<Self, crate::Error> {
        // Create a combined key for simplicity in this implementation
        // In a full implementation, you'd want to handle local and remote keys separately
        let combined_key = crypto::SrtpCryptoKey::new(local_key, local_salt);

        Self::new(profile, combined_key)
    }

    /// Enable or disable SRTP
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Validate that this context can be installed in a secure transport.
    pub(crate) fn validate_for_secure_transport(&self) -> Result<(), crate::Error> {
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot be installed as secure media".to_string(),
            ));
        }
        self.crypto.suite().validate()
    }

    /// Set key rotation frequency.
    ///
    /// Non-`None` schedules are retained for API compatibility, but key
    /// rotation is not implemented in this release. The first packet whose
    /// index requires rotation is rejected before encryption or state changes.
    pub fn set_key_rotation(&mut self, frequency: key_derivation::KeyRotationFrequency) {
        self.key_rotation = frequency;
    }

    /// Protect an RTP packet (SRTP encryption)
    /// Returns the encrypted packet with its authentication tag
    pub fn protect(
        &mut self,
        packet: &crate::packet::RtpPacket,
    ) -> Result<ProtectedRtpPacket, crate::Error> {
        // Check if SRTP is enabled
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot pass RTP through as plaintext".to_string(),
            ));
        }

        // Check for key rotation
        if self.key_rotation.should_rotate(self.packet_index) {
            return Err(crate::Error::UnsupportedFeature(
                "SRTP key rotation is not implemented; refusing to reuse the old key after the configured rotation boundary"
                    .to_string(),
            ));
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
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot accept RTP as plaintext".to_string(),
            ));
        }

        // Decrypt using SRTP (which handles authentication verification internally)
        self.crypto.decrypt_rtp(data)
    }

    /// Protect an RTCP packet with SRTCP.
    ///
    /// Authenticated SRTCP state management is not complete in this release.
    /// Enabled SRTP contexts therefore fail closed instead of calling the
    /// incomplete crypto path. This method is the compatibility hook for the
    /// complete per-SSRC SRTCP implementation planned for the next repair.
    pub fn protect_rtcp(&mut self, _data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot pass RTCP through as plaintext".to_string(),
            ));
        }

        Err(crate::Error::UnsupportedFeature(
            "authenticated SRTCP is not implemented; RTCP is disabled for SRTP contexts"
                .to_string(),
        ))
    }

    /// Unprotect an RTCP packet with SRTCP.
    ///
    /// See [`Self::protect_rtcp`]. Enabled SRTP contexts reject all input until
    /// authenticated SRTCP with replay/index state is complete.
    pub fn unprotect_rtcp(&mut self, _data: &[u8]) -> Result<bytes::Bytes, crate::Error> {
        if !self.enabled {
            return Err(crate::Error::InvalidState(
                "disabled SRTP context cannot accept RTCP as plaintext".to_string(),
            ));
        }

        Err(crate::Error::UnsupportedFeature(
            "authenticated SRTCP is not implemented; RTCP is disabled for SRTP contexts"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn null_profiles_are_retained_identities_but_cannot_construct_contexts() {
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);
        assert!(matches!(
            SrtpContext::new(SRTP_NULL_NULL, key.clone()),
            Err(crate::Error::UnsupportedFeature(_))
        ));
        assert!(matches!(
            SrtpContext::new(SRTP_NULL_SHA1_80, key),
            Err(crate::Error::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn hand_built_unreviewed_suite_combinations_fail_closed() {
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);
        let aes_without_authentication = SrtpCryptoSuite {
            encryption: SrtpEncryptionAlgorithm::AesCm,
            authentication: SrtpAuthenticationAlgorithm::Null,
            key_length: 16,
            tag_length: 0,
        };
        assert!(matches!(
            SrtpContext::new(aes_without_authentication, key),
            Err(crate::Error::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn enabled_srtp_context_rejects_incomplete_srtcp() {
        let key = SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]);
        let mut context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key).unwrap();
        let rtcp = [0x80, 200, 0, 1, 0, 0, 0, 1];

        assert!(matches!(
            context.protect_rtcp(&rtcp),
            Err(crate::Error::UnsupportedFeature(_))
        ));
        assert!(matches!(
            context.unprotect_rtcp(&rtcp),
            Err(crate::Error::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn disabled_srtp_context_rejects_all_plaintext_passthrough_without_mutation() {
        let key = SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]);
        let packet = crate::packet::RtpPacket::new(
            crate::packet::RtpHeader::new(0, 7, 1_234, 0x1122_3344),
            bytes::Bytes::from_static(b"must-not-pass-plain"),
        );
        let serialized = packet.serialize().unwrap();
        let rtcp = [0x80, 200, 0, 1, 0, 0, 0, 1];
        let mut context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key).unwrap();
        context.set_enabled(false);

        assert!(matches!(
            context.protect(&packet),
            Err(crate::Error::InvalidState(_))
        ));
        assert!(matches!(
            context.unprotect(&serialized),
            Err(crate::Error::InvalidState(_))
        ));
        assert!(matches!(
            context.protect_rtcp(&rtcp),
            Err(crate::Error::InvalidState(_))
        ));
        assert!(matches!(
            context.unprotect_rtcp(&rtcp),
            Err(crate::Error::InvalidState(_))
        ));
        assert_eq!(context.packet_index, 0);
    }

    #[test]
    fn required_key_rotation_fails_before_encryption_or_state_mutation() {
        let key = SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]);
        let packet = crate::packet::RtpPacket::new(
            crate::packet::RtpHeader::new(0, 7, 1_234, 0x1122_3344),
            bytes::Bytes::from_static(b"rotation-boundary"),
        );

        for power in [8, 64, 255] {
            let mut context = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone()).unwrap();
            context.set_key_rotation(key_derivation::KeyRotationFrequency::Power2(power));
            assert!(matches!(
                context.protect(&packet),
                Err(crate::Error::UnsupportedFeature(_))
            ));
            assert_eq!(context.packet_index, 0);

            // Repeating the failed operation must observe the same boundary,
            // and clearing the unsupported schedule must produce the same
            // first packet as a fresh context.
            assert!(matches!(
                context.protect(&packet),
                Err(crate::Error::UnsupportedFeature(_))
            ));
            assert_eq!(context.packet_index, 0);

            context.set_key_rotation(key_derivation::KeyRotationFrequency::None);
            let after_failure = context.protect(&packet).unwrap().serialize().unwrap();
            let from_fresh = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone())
                .unwrap()
                .protect(&packet)
                .unwrap()
                .serialize()
                .unwrap();
            assert_eq!(after_failure, from_fresh);
        }
    }

    #[test]
    fn rtp_padding_is_encrypted_and_restored() {
        let key = SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]);
        let mut sender = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone()).unwrap();
        let mut receiver = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key).unwrap();
        let mut original = crate::packet::RtpPacket::new_with_payload(
            96,
            42,
            123_456,
            0x4567_89ab,
            Bytes::from_static(b"padded media"),
        );
        original.set_padding(4);

        let plaintext = original.serialize().unwrap();
        let header_size = original.header.size();
        let protected = sender.protect(&original).unwrap();
        let wire = protected.serialize().unwrap();
        let ciphertext_end = wire.len() - SRTP_AES128_CM_SHA1_80.tag_length;

        assert_ne!(
            &wire[header_size..ciphertext_end],
            &plaintext[header_size..],
            "RTP padding must be encrypted together with the media payload"
        );

        let restored = receiver.unprotect(&wire).unwrap();
        assert_eq!(restored, original);
        assert_eq!(restored.serialize().unwrap(), plaintext);
    }
}

#[cfg(test)]
mod integration_tests;
