use super::{SrtpAuthenticationAlgorithm, SrtpCryptoSuite, SrtpEncryptionAlgorithm};
use crate::error::Error;
use crate::packet::RtpPacket;
use crate::Result;
use aes::{
    cipher::{generic_array::GenericArray, KeyIvInit, StreamCipher},
    Aes128, Aes256,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bytes::{BufMut, Bytes, BytesMut};
use ctr::Ctr64BE;
use hmac::{Hmac, Mac};
use sha1::Sha1;

// Define types for AES-CM
type Aes128Ctr64BE = Ctr64BE<Aes128>;
type Aes256Ctr64BE = Ctr64BE<Aes256>;
// Define type for HMAC-SHA1
type HmacSha1 = Hmac<Sha1>;

/// Standard SRTP master salt length in bytes (RFC 3711 §8.2, all suites in
/// this module use a 112-bit salt).
const SRTP_SALT_LENGTH: usize = 14;

/// Basic cryptographic key/salt for SRTP.
///
/// Zeroized on drop and never printed in full via `Debug` — this holds raw
/// key material that must not end up in logs.
#[derive(Clone, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct SrtpCryptoKey {
    /// Raw key material
    key: Vec<u8>,

    /// Salt for the key
    salt: Vec<u8>,
}

impl std::fmt::Debug for SrtpCryptoKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtpCryptoKey")
            .field("key", &format_args!("[REDACTED {} bytes]", self.key.len()))
            .field(
                "salt",
                &format_args!("[REDACTED {} bytes]", self.salt.len()),
            )
            .finish()
    }
}

impl SrtpCryptoKey {
    /// Create a new SRTP key from raw bytes
    pub fn new(key: Vec<u8>, salt: Vec<u8>) -> Self {
        Self { key, salt }
    }

    /// Get a reference to the key material
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Get a reference to the salt
    pub fn salt(&self) -> &[u8] {
        &self.salt
    }

    /// Create a key from a base64 string (as used in SDP `a=crypto` lines),
    /// validated against exactly what `suite` requires.
    ///
    /// Rejects anything that isn't exactly `suite.key_length +
    /// SRTP_SALT_LENGTH` decoded bytes — a short or padded salt would
    /// silently derive the wrong session keys instead of failing loudly.
    pub fn from_base64(data: &str, suite: &SrtpCryptoSuite) -> Result<Self> {
        let decoded = BASE64
            .decode(data)
            .map_err(|e| Error::SrtpError(format!("Failed to decode base64 key: {}", e)))?;

        let expected_len = suite.key_length + SRTP_SALT_LENGTH;
        if decoded.len() != expected_len {
            return Err(Error::SrtpError(format!(
                "SRTP key material is {} bytes, expected exactly {} ({} key + {} salt) for this suite",
                decoded.len(),
                expected_len,
                suite.key_length,
                SRTP_SALT_LENGTH
            )));
        }

        let key = decoded[0..suite.key_length].to_vec();
        let salt = decoded[suite.key_length..].to_vec();

        Ok(Self { key, salt })
    }
}

/// SRTP context for encryption/decryption
pub struct SrtpCrypto {
    /// Crypto suite in use
    suite: SrtpCryptoSuite,

    /// Master key for encryption
    master_key: SrtpCryptoKey,

    /// Session keys derived from master key
    session_keys: Option<SrtpSessionKeys>,
}

/// Derived session keys for SRTP. Zeroized on drop — these are as
/// sensitive as the master key they were derived from.
#[derive(Clone, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct SrtpSessionKeys {
    /// Key for RTP encryption
    rtp_enc_key: Vec<u8>,

    /// Key for RTP authentication
    rtp_auth_key: Vec<u8>,

    /// Salt for RTP encryption
    rtp_salt: Vec<u8>,

    /// Key for RTCP encryption
    rtcp_enc_key: Vec<u8>,

    /// Key for RTCP authentication
    rtcp_auth_key: Vec<u8>,

    /// Salt for RTCP encryption
    rtcp_salt: Vec<u8>,
}

impl SrtpCrypto {
    /// Create a new SRTP crypto context
    pub fn new(suite: SrtpCryptoSuite, master_key: SrtpCryptoKey) -> Result<Self> {
        // Validate key length
        if master_key.key().len() != suite.key_length {
            return Err(Error::SrtpError(format!(
                "Key length mismatch: expected {} but got {}",
                suite.key_length,
                master_key.key().len()
            )));
        }

        // Validate salt length. Every suite in this module uses the
        // standard RFC 3711 §8.2 112-bit (14-byte) master salt; a short or
        // empty salt would silently weaken (or, with an empty salt, zero
        // out) the derived IVs instead of failing loudly.
        if master_key.salt().len() != SRTP_SALT_LENGTH {
            return Err(Error::SrtpError(format!(
                "Salt length mismatch: expected {} but got {}",
                SRTP_SALT_LENGTH,
                master_key.salt().len()
            )));
        }

        let mut crypto = Self {
            suite,
            master_key,
            session_keys: None,
        };

        // Derive session keys
        crypto.derive_keys()?;

        Ok(crypto)
    }

    /// Derive session keys from master key
    fn derive_keys(&mut self) -> Result<()> {
        // Use our KDF to derive session keys according to RFC 3711

        // Derive RTP encryption key
        let rtp_enc_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtpEncryption,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtp_enc_key =
            super::srtp_kdf(&self.master_key, &rtp_enc_params, self.suite.key_length)?;

        // Derive RTP authentication key (20 bytes for HMAC-SHA1)
        let rtp_auth_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtpAuthentication,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtp_auth_key = super::srtp_kdf(&self.master_key, &rtp_auth_params, 20)?;

        // Derive RTP salt
        let rtp_salt_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtpSalt,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtp_salt = super::srtp_kdf(&self.master_key, &rtp_salt_params, 14)?;

        // Derive RTCP encryption key
        let rtcp_enc_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtcpEncryption,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtcp_enc_key =
            super::srtp_kdf(&self.master_key, &rtcp_enc_params, self.suite.key_length)?;

        // Derive RTCP authentication key
        let rtcp_auth_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtcpAuthentication,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtcp_auth_key = super::srtp_kdf(&self.master_key, &rtcp_auth_params, 20)?;

        // Derive RTCP salt
        let rtcp_salt_params = super::SrtpKeyDerivationParams {
            label: super::KeyDerivationLabel::RtcpSalt,
            key_derivation_rate: 0,
            index: 0,
        };
        let rtcp_salt = super::srtp_kdf(&self.master_key, &rtcp_salt_params, 14)?;

        // Store the derived keys
        let session_keys = SrtpSessionKeys {
            rtp_enc_key,
            rtp_auth_key,
            rtp_salt,
            rtcp_enc_key,
            rtcp_auth_key,
            rtcp_salt,
        };

        self.session_keys = Some(session_keys);
        Ok(())
    }

    /// Encrypt an RTP packet.
    ///
    /// `roc` is the rollover counter for `packet.header.ssrc`'s stream at
    /// this point, as tracked by the caller (`SrtpContext`, per RFC 3711
    /// §3.3.1) — this function does no rollover tracking of its own, it
    /// only uses whatever `roc` it's given to build the IV and auth tag.
    pub fn encrypt_rtp(
        &self,
        packet: &RtpPacket,
        roc: u32,
    ) -> Result<(RtpPacket, Option<Vec<u8>>)> {
        if self.suite.encryption == SrtpEncryptionAlgorithm::Null {
            // Null encryption, just return the original packet
            return if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
                // No authentication either
                Ok((packet.clone(), None))
            } else {
                // Authentication is enabled, calculate tag
                let serialized = packet.serialize()?;
                let auth_tag = self.calculate_auth_tag(&serialized, roc)?;
                Ok((packet.clone(), Some(auth_tag)))
            };
        }

        // Get session keys
        let session_keys = self
            .session_keys
            .as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Extract header and payload
        let header = packet.header.clone();
        let payload = packet.payload.clone();

        // Create an IV for encryption
        let ssrc = packet.header.ssrc;
        let sequence = packet.header.sequence_number as u64;
        let packet_index = (roc as u64) << 16 | sequence;

        // Create an IV using salt and packet info
        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtp_salt, ssrc, packet_index)?
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        };

        // Create a mutable copy of the payload for encryption
        let mut encrypted_payload = BytesMut::from(&payload[..]);

        // Encrypt the payload
        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_encrypt(&mut encrypted_payload, &session_keys.rtp_enc_key, &iv)?;
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        }

        // Create a new packet with the encrypted payload
        let encrypted_packet = RtpPacket::new(header, encrypted_payload.freeze());

        // Calculate authentication tag if authentication is enabled
        let auth_tag = if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            // Serialize the encrypted packet for authentication
            let encrypted_serialized = encrypted_packet.serialize()?;

            // Calculate the authentication tag
            let auth_tag = self.calculate_auth_tag(&encrypted_serialized, roc)?;
            Some(auth_tag)
        } else {
            None
        };

        Ok((encrypted_packet, auth_tag))
    }

    /// Calculate authentication tag for a packet
    fn calculate_auth_tag(&self, packet_data: &[u8], roc: u32) -> Result<Vec<u8>> {
        if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
            return Err(Error::SrtpError(
                "Authentication is not enabled".to_string(),
            ));
        }

        // Get session keys
        let session_keys = self
            .session_keys
            .as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Create an authenticator
        let authenticator = super::auth::SrtpAuthenticator::new(
            self.suite.authentication,
            session_keys.rtp_auth_key.clone(),
            self.suite.tag_length,
        );

        // Calculate the authentication tag
        authenticator.calculate_auth_tag(packet_data, roc)
    }

    /// Decrypt an SRTP packet.
    ///
    /// `roc` is the rollover counter the caller (`SrtpContext`) has already
    /// resolved for this packet's SSRC/sequence number (RFC 3711 Appendix A
    /// guess-and-check) — this function trusts it as-is for both auth
    /// verification and IV construction, it does no rollover estimation
    /// itself.
    pub fn decrypt_rtp(&self, data: &[u8], roc: u32) -> Result<RtpPacket> {
        if self.suite.encryption == SrtpEncryptionAlgorithm::Null
            && self.suite.authentication == SrtpAuthenticationAlgorithm::Null
        {
            // Null encryption and authentication, just parse the packet
            return RtpPacket::parse(data);
        }

        // Get session keys
        let session_keys = self
            .session_keys
            .as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Determine authentication tag size
        let auth_tag_size = if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            self.suite.tag_length
        } else {
            0
        };

        // Check if the packet has an authentication tag
        if auth_tag_size > 0 && data.len() < auth_tag_size {
            return Err(Error::SrtpError(
                "Packet too short to contain authentication tag".to_string(),
            ));
        }

        // Split data into packet and authentication tag
        let (packet_data, auth_tag) = if auth_tag_size > 0 {
            let tag_start = data.len() - auth_tag_size;
            (&data[0..tag_start], &data[tag_start..])
        } else {
            (data, &[][..])
        };

        // Verify authentication if enabled
        if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            // Create an authenticator
            let authenticator = super::auth::SrtpAuthenticator::new(
                self.suite.authentication,
                session_keys.rtp_auth_key.clone(),
                self.suite.tag_length,
            );

            // Verify the authentication tag
            let is_valid = authenticator.verify_auth_tag(packet_data, auth_tag, roc)?;
            if !is_valid {
                return Err(Error::SrtpError("Authentication failed".to_string()));
            }
        }

        // Parse the RTP header first (it's not encrypted)
        let packet = RtpPacket::parse(packet_data)?;

        if self.suite.encryption == SrtpEncryptionAlgorithm::Null {
            // If only authentication is enabled, return the parsed packet
            return Ok(packet);
        }

        // Create an IV for decryption
        let ssrc = packet.header.ssrc;
        let sequence = packet.header.sequence_number as u64;
        let packet_index = (roc as u64) << 16 | sequence;

        // Create an IV using salt and packet info
        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtp_salt, ssrc, packet_index)?
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        };

        // Create a mutable copy of the payload for decryption
        let mut decrypted_payload = BytesMut::from(&packet.payload[..]);

        // Decrypt the payload
        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_decrypt(&mut decrypted_payload, &session_keys.rtp_enc_key, &iv)?;
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        }

        // Create a new packet with the decrypted payload
        let decrypted_packet = RtpPacket::new(packet.header, decrypted_payload.freeze());

        Ok(decrypted_packet)
    }

    /// Encrypt an RTCP packet.
    ///
    /// `index` is the 31-bit SRTCP index for this packet (RFC 3711 §3.4) —
    /// the caller (`SrtpContext`) owns incrementing it per SSRC across
    /// sends; this function just writes whatever it's given into the
    /// trailing E-bit/index word and uses it to build the IV.
    pub fn encrypt_rtcp(&self, data: &[u8], index: u32) -> Result<(Bytes, Option<Vec<u8>>)> {
        if self.suite.encryption == SrtpEncryptionAlgorithm::Null {
            // Null encryption, just return the original data
            return if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
                // No authentication either
                Ok((Bytes::copy_from_slice(data), None))
            } else {
                // Only authentication
                let auth_tag = self.calculate_rtcp_auth_tag(data)?;
                Ok((Bytes::copy_from_slice(data), Some(auth_tag)))
            };
        }

        // Get session keys
        let session_keys = self
            .session_keys
            .as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Everything after the first 8-byte header of the (possibly
        // compound) RTCP packet is encrypted (RFC 3711 §3.4); the sender's
        // SSRC used for the IV comes from that same first header.
        if data.len() <= 8 {
            return Err(Error::SrtpError("RTCP packet too short".to_string()));
        }

        // Extract header and payload
        let header = &data[0..8];
        let payload = &data[8..];
        let ssrc = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);

        // Create a mutable buffer for our result
        let mut result = BytesMut::with_capacity(data.len() + 4); // Space for index (4)

        // Copy the header
        result.extend_from_slice(header);

        // Create a mutable copy of the payload for encryption
        let mut encrypted_payload = BytesMut::from(payload);

        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtcp_salt, ssrc, index as u64)?
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        };

        // Encrypt the payload
        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_encrypt(&mut encrypted_payload, &session_keys.rtcp_enc_key, &iv)?;
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        }

        // Add encrypted payload to result
        result.extend_from_slice(&encrypted_payload);

        // Add SRTCP index and E flag (E=1: this packet is encrypted)
        result.put_u32(0x80000000 | (index & 0x7FFF_FFFF));

        // Calculate authentication tag if needed. `result` already has the
        // index/E-bit word appended, so it's covered by the tag per RFC
        // 3711 §4.2.
        let auth_tag = if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            let auth_tag = self.calculate_rtcp_auth_tag(&result)?;
            Some(auth_tag)
        } else {
            None
        };

        Ok((result.freeze(), auth_tag))
    }

    /// Calculate authentication tag for an RTCP packet. `data` must already
    /// include the trailing SRTCP index/E-bit word — RFC 3711 §4.2 requires
    /// it be covered by the tag.
    fn calculate_rtcp_auth_tag(&self, data: &[u8]) -> Result<Vec<u8>> {
        if self.suite.authentication == SrtpAuthenticationAlgorithm::Null {
            return Err(Error::SrtpError(
                "Authentication is not enabled".to_string(),
            ));
        }

        // Get session keys
        let session_keys = self
            .session_keys
            .as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Create HMAC-SHA1 instance
        let tag = hmac_sha1(data, &session_keys.rtcp_auth_key, self.suite.tag_length)?;

        Ok(tag)
    }

    /// Peek the sender SSRC, SRTCP index, and E-bit from a (possibly still
    /// encrypted) SRTCP packet, without verifying authentication or
    /// decrypting anything. Used by `SrtpContext::unprotect_rtcp` to do
    /// per-SSRC replay-window bookkeeping before spending cycles on HMAC
    /// verification — the header bytes and the trailing index/E-bit word
    /// are never encrypted, only the RTCP payload in between is.
    pub fn peek_srtcp_header(&self, data: &[u8]) -> Result<(u32, u32, bool)> {
        if data.len() < 8 {
            return Err(Error::SrtpError("RTCP packet too short".to_string()));
        }
        let ssrc = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

        if self.suite.encryption == SrtpEncryptionAlgorithm::Null
            && self.suite.authentication == SrtpAuthenticationAlgorithm::Null
        {
            return Ok((ssrc, 0, false));
        }

        let tag_length = if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            self.suite.tag_length
        } else {
            0
        };
        if data.len() < 8 + 4 + tag_length {
            return Err(Error::SrtpError(format!(
                "SRTCP packet too short: {} bytes",
                data.len()
            )));
        }
        let auth_tag_pos = data.len() - tag_length;
        let index_pos = auth_tag_pos - 4;
        let index_value = u32::from_be_bytes([
            data[index_pos],
            data[index_pos + 1],
            data[index_pos + 2],
            data[index_pos + 3],
        ]);
        let e_flag = (index_value & 0x8000_0000) != 0;
        let index = index_value & 0x7FFF_FFFF;
        Ok((ssrc, index, e_flag))
    }

    /// Decrypt an SRTCP packet
    pub fn decrypt_rtcp(&self, data: &[u8]) -> Result<Bytes> {
        if self.suite.encryption == SrtpEncryptionAlgorithm::Null
            && self.suite.authentication == SrtpAuthenticationAlgorithm::Null
        {
            // Null encryption and authentication, just return the original data
            return Ok(Bytes::copy_from_slice(data));
        }

        // Get session keys
        let session_keys = self
            .session_keys
            .as_ref()
            .ok_or_else(|| Error::SrtpError("Session keys not derived".to_string()))?;

        // Check packet minimum length (header + index + auth tag)
        let min_len = 8
            + 4
            + (if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
                self.suite.tag_length
            } else {
                0
            });

        if data.len() < min_len {
            return Err(Error::SrtpError(format!(
                "SRTCP packet too short: {} bytes",
                data.len()
            )));
        }

        // Calculate authentication tag position
        let auth_tag_pos = data.len() - self.suite.tag_length;

        // Verify authentication tag if authentication is enabled
        if self.suite.authentication != SrtpAuthenticationAlgorithm::Null {
            let packet_data = &data[0..auth_tag_pos];
            let auth_tag = &data[auth_tag_pos..];

            // Calculate authentication tag to compare
            let calculated_tag = self.calculate_rtcp_auth_tag(packet_data)?;

            // Constant-time comparison to prevent timing attacks
            let mut result = 0;
            if calculated_tag.len() != auth_tag.len() {
                return Err(Error::SrtpError(
                    "Authentication tag length mismatch".to_string(),
                ));
            }

            for (a, b) in calculated_tag.iter().zip(auth_tag.iter()) {
                result |= a ^ b;
            }

            if result != 0 {
                return Err(Error::SrtpError("SRTCP authentication failed".to_string()));
            }
        }

        // Get the index and E flag
        let index_pos = auth_tag_pos - 4;
        let index_bytes = [
            data[index_pos],
            data[index_pos + 1],
            data[index_pos + 2],
            data[index_pos + 3],
        ];
        let index_value = u32::from_be_bytes(index_bytes);
        let e_flag = (index_value & 0x80000000) != 0;
        let index = index_value & 0x7FFFFFFF;

        // If E flag is not set, packet is not encrypted
        if !e_flag {
            // Remove the index and auth tag
            let mut result = BytesMut::with_capacity(index_pos);
            result.extend_from_slice(&data[0..index_pos]);
            return Ok(result.freeze());
        }

        // Extract header and payload
        let header = &data[0..8];
        let payload = &data[8..index_pos];

        // Create a mutable buffer for our result
        let mut result = BytesMut::with_capacity(index_pos);

        // Copy the header
        result.extend_from_slice(header);

        // Create a mutable copy of the payload for decryption
        let mut decrypted_payload = BytesMut::from(payload);

        // The sender's SSRC used for the IV comes from the (unencrypted)
        // first 8-byte RTCP header, bytes 4-7.
        let ssrc = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);

        let iv = match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                super::create_srtp_iv(&session_keys.rtcp_salt, ssrc, index as u64)?
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        };

        // Decrypt the payload
        match self.suite.encryption {
            SrtpEncryptionAlgorithm::AesCm => {
                aes_cm_decrypt(&mut decrypted_payload, &session_keys.rtcp_enc_key, &iv)?;
            }
            _ => {
                return Err(Error::SrtpError(
                    "Unsupported encryption algorithm".to_string(),
                ))
            }
        }

        // Add decrypted payload to result
        result.extend_from_slice(&decrypted_payload);

        Ok(result.freeze())
    }
}

/// AES Counter Mode encryption for SRTP
fn aes_cm_encrypt(data: &mut [u8], key: &[u8], iv: &[u8]) -> Result<()> {
    if iv.len() < 16 {
        return Err(Error::SrtpError(format!(
            "AES-CM IV too short: expected 16 bytes, got {}",
            iv.len()
        )));
    }
    let iv = GenericArray::from_slice(&iv[0..16]);

    match key.len() {
        16 => {
            let key = GenericArray::from_slice(key);
            let mut cipher = Aes128Ctr64BE::new(key, iv);
            cipher.apply_keystream(data);
        }
        32 => {
            let key = GenericArray::from_slice(key);
            let mut cipher = Aes256Ctr64BE::new(key, iv);
            cipher.apply_keystream(data);
        }
        len => {
            return Err(Error::SrtpError(format!(
                "unsupported AES-CM key length: {} bytes",
                len
            )));
        }
    }

    Ok(())
}

/// AES Counter Mode decryption for SRTP
fn aes_cm_decrypt(data: &mut [u8], key: &[u8], iv: &[u8]) -> Result<()> {
    // AES-CM is symmetric, so encryption and decryption are the same
    aes_cm_encrypt(data, key, iv)
}

/// HMAC-SHA1 authentication for SRTP
fn hmac_sha1(data: &[u8], key: &[u8], tag_length: usize) -> Result<Vec<u8>> {
    // Create a new HMAC-SHA1 instance
    let mut mac = HmacSha1::new_from_slice(key)
        .map_err(|e| Error::SrtpError(format!("Failed to create HMAC: {}", e)))?;

    // Update with data
    mac.update(data);

    // Finalize and get the result
    let result = mac.finalize().into_bytes();

    // Truncate to the requested tag length. Guard against a misconfigured
    // suite whose tag_length exceeds the 20-byte HMAC-SHA1 output (otherwise
    // the slice below panics).
    if tag_length > result.len() {
        return Err(Error::SrtpError(format!(
            "auth tag_length {} exceeds HMAC-SHA1 output {}",
            tag_length,
            result.len()
        )));
    }
    let tag = result.as_slice()[..tag_length].to_vec();

    Ok(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_bytes(hex: &str) -> Vec<u8> {
        assert_eq!(hex.len() % 2, 0);
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn test_srtp_key_from_base64() {
        // 16-byte key + 14-byte salt = 30 bytes, exactly what
        // SRTP_AES128_CM_SHA1_80 requires.
        let raw: Vec<u8> = (0..30u8).collect();
        let base64_key = BASE64.encode(&raw);

        let key = SrtpCryptoKey::from_base64(&base64_key, &super::super::SRTP_AES128_CM_SHA1_80);
        assert!(key.is_ok());

        let key = key.unwrap();
        assert_eq!(key.key().len(), 16);
        assert_eq!(key.salt().len(), 14);

        // Invalid base64
        let invalid_key = "invalid-base64!";
        let key = SrtpCryptoKey::from_base64(invalid_key, &super::super::SRTP_AES128_CM_SHA1_80);
        assert!(key.is_err());
    }

    #[test]
    fn test_srtp_key_from_base64_rejects_wrong_length() {
        // 24 bytes decoded, but the suite needs exactly 30 (16 key + 14 salt).
        let raw: Vec<u8> = (0..24u8).collect();
        let base64_key = BASE64.encode(&raw);

        let key = SrtpCryptoKey::from_base64(&base64_key, &super::super::SRTP_AES128_CM_SHA1_80);
        assert!(key.is_err());

        // A 256-bit suite needs 32 + 14 = 46 bytes; the same 30-byte
        // material that satisfies the 128-bit suite must be rejected here.
        let raw128: Vec<u8> = (0..30u8).collect();
        let base64_key128 = BASE64.encode(&raw128);
        let key = SrtpCryptoKey::from_base64(&base64_key128, &super::super::SRTP_AES256_CM_SHA1_80);
        assert!(key.is_err());
    }

    #[test]
    fn test_null_encryption() {
        // Create a key
        let key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);

        // Use a modified SRTP_NULL_NULL with correct key length for testing
        let null_suite = SrtpCryptoSuite {
            encryption: SrtpEncryptionAlgorithm::Null,
            authentication: SrtpAuthenticationAlgorithm::Null,
            key_length: 16, // Changed from 0 to 16 to match our test key
            tag_length: 0,
        };

        // Create crypto context with null encryption
        let crypto = SrtpCrypto::new(null_suite, key).unwrap();

        // Create a test packet
        let header = crate::packet::RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new(header, payload);

        // Encrypt and verify it returns the same packet (null encryption)
        let encrypted_result = crypto.encrypt_rtp(&packet, 0);
        assert!(encrypted_result.is_ok());
        let (encrypted, _auth_tag) = encrypted_result.unwrap();

        // Packets should be equal with null encryption
        assert_eq!(encrypted.header.payload_type, packet.header.payload_type);
        assert_eq!(
            encrypted.header.sequence_number,
            packet.header.sequence_number
        );
        assert_eq!(encrypted.header.timestamp, packet.header.timestamp);
        assert_eq!(encrypted.header.ssrc, packet.header.ssrc);
        assert_eq!(encrypted.payload, packet.payload);
    }

    #[test]
    fn test_aes_cm_encryption() {
        // Test data
        let mut data = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let key = vec![0; 16]; // 16-byte AES key (all zeros)
        let iv = vec![0; 16]; // 16-byte IV (all zeros)

        // Encrypt
        let result = aes_cm_encrypt(&mut data, &key, &iv);
        assert!(result.is_ok());

        // Data should now be encrypted - it should differ from the original
        assert_ne!(data, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

        // Make a copy of the encrypted data
        let _encrypted = data.clone();

        // Now decrypt
        let result = aes_cm_decrypt(&mut data, &key, &iv);
        assert!(result.is_ok());

        // Data should now be decrypted back to the original
        assert_eq!(data, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn test_rfc3711_appendix_b2_aes_cm_vector() {
        let key = hex_bytes("2B7E151628AED2A6ABF7158809CF4F3C");
        let iv = hex_bytes("F0F1F2F3F4F5F6F7F8F9FAFBFCFD0000");
        let mut data = vec![0u8; 16];

        aes_cm_encrypt(&mut data, &key, &iv).unwrap();

        assert_eq!(data, hex_bytes("E03EAD0935C95E80E166B16DD92B4EB4"));
    }

    #[test]
    fn test_hmac_sha1() {
        // Test data
        let data = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let key = vec![0; 20]; // 20-byte key (all zeros)

        // Calculate tag with length 10 (80 bits)
        let tag = hmac_sha1(&data, &key, 10);
        assert!(tag.is_ok());
        let tag = tag.unwrap();

        // Tag should be 10 bytes long
        assert_eq!(tag.len(), 10);

        // Calculate tag with length 4 (32 bits)
        let tag32 = hmac_sha1(&data, &key, 4);
        assert!(tag32.is_ok());
        let tag32 = tag32.unwrap();

        // Tag should be 4 bytes long
        assert_eq!(tag32.len(), 4);

        // First 4 bytes should match between the two tags
        assert_eq!(tag[0..4], tag32[0..4]);
    }

    #[test]
    fn test_complete_srtp_process() {
        // Create a master key and salt
        let master_key = vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ];
        let master_salt = vec![
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
        ];

        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);

        // Test both AES-CM suites
        let suites = vec![
            super::super::SRTP_AES128_CM_SHA1_80,
            super::super::SRTP_AES128_CM_SHA1_32,
        ];

        for suite in suites {
            // Create SRTP crypto context
            let crypto = SrtpCrypto::new(suite.clone(), srtp_key.clone()).unwrap();

            // Create a test packet
            let header = crate::packet::RtpHeader::new(96, 1000, 12345, 0xabcdef01);
            let payload = Bytes::from_static(
                b"Hello SRTP World! This is a test of SRTP encryption and decryption.",
            );
            let packet = RtpPacket::new(header, payload);

            // Encrypt the packet
            let encrypted_result = crypto.encrypt_rtp(&packet, 0).unwrap();
            let (encrypted_packet, auth_tag) = encrypted_result;

            // Payload should be encrypted (different from original)
            assert_ne!(encrypted_packet.payload, packet.payload);

            // Header should not be encrypted
            assert_eq!(
                encrypted_packet.header.payload_type,
                packet.header.payload_type
            );
            assert_eq!(
                encrypted_packet.header.sequence_number,
                packet.header.sequence_number
            );
            assert_eq!(encrypted_packet.header.timestamp, packet.header.timestamp);
            assert_eq!(encrypted_packet.header.ssrc, packet.header.ssrc);

            // Serialize the packet
            let serialized = encrypted_packet.serialize().unwrap();

            // Add authentication tag (if provided)
            let mut protected_data = BytesMut::with_capacity(serialized.len() + 10);
            protected_data.extend_from_slice(&serialized);
            if let Some(tag) = auth_tag {
                protected_data.extend_from_slice(&tag);
            }

            // Decrypt the packet
            let decrypted = crypto.decrypt_rtp(&protected_data, 0);
            assert!(decrypted.is_ok());
            let decrypted = decrypted.unwrap();

            // Decrypted packet should match original
            assert_eq!(decrypted.header.payload_type, packet.header.payload_type);
            assert_eq!(
                decrypted.header.sequence_number,
                packet.header.sequence_number
            );
            assert_eq!(decrypted.header.timestamp, packet.header.timestamp);
            assert_eq!(decrypted.header.ssrc, packet.header.ssrc);
            assert_eq!(decrypted.payload, packet.payload);
        }
    }

    #[test]
    fn test_complete_srtp_process_aes256() {
        let master_key: Vec<u8> = (1..=32).collect();
        let master_salt = vec![
            0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D,
        ];
        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
        let suites = vec![
            super::super::SRTP_AES256_CM_SHA1_80,
            super::super::SRTP_AES256_CM_SHA1_32,
        ];

        for suite in suites {
            let crypto = SrtpCrypto::new(suite.clone(), srtp_key.clone()).unwrap();
            let header = crate::packet::RtpHeader::new(96, 1000, 12345, 0xabcdef01);
            let payload = Bytes::from_static(b"Hello AES-256 SRTP World");
            let packet = RtpPacket::new(header, payload);

            let (encrypted_packet, auth_tag) = crypto.encrypt_rtp(&packet, 0).unwrap();
            assert_ne!(encrypted_packet.payload, packet.payload);

            let serialized = encrypted_packet.serialize().unwrap();
            let mut protected_data = BytesMut::with_capacity(serialized.len() + suite.tag_length);
            protected_data.extend_from_slice(&serialized);
            let tag = auth_tag.expect("AES-256 SRTP suites authenticate RTP");
            assert_eq!(tag.len(), suite.tag_length);
            protected_data.extend_from_slice(&tag);

            let decrypted = crypto.decrypt_rtp(&protected_data, 0).unwrap();
            assert_eq!(decrypted.header.payload_type, packet.header.payload_type);
            assert_eq!(
                decrypted.header.sequence_number,
                packet.header.sequence_number
            );
            assert_eq!(decrypted.header.timestamp, packet.header.timestamp);
            assert_eq!(decrypted.header.ssrc, packet.header.ssrc);
            assert_eq!(decrypted.payload, packet.payload);
        }
    }

    #[test]
    fn test_tamper_detection() {
        // Create master key and crypto context
        let master_key = vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ];
        let master_salt = vec![
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
        ];

        let srtp_key = SrtpCryptoKey::new(master_key, master_salt);
        let crypto = SrtpCrypto::new(super::super::SRTP_AES128_CM_SHA1_80, srtp_key).unwrap();

        // Create a test packet
        let header = crate::packet::RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"Protected data");
        let packet = RtpPacket::new(header, payload);

        // Encrypt the packet
        let encrypted_result = crypto.encrypt_rtp(&packet, 0).unwrap();
        let (encrypted_packet, auth_tag) = encrypted_result;

        // Ensure auth tag is present
        assert!(auth_tag.is_some());

        // Clone the auth tag for later use
        let auth_tag_clone = auth_tag.clone();

        // Serialize the packet
        let serialized = encrypted_packet.serialize().unwrap();

        // Create protected data with auth tag
        let mut protected_data = BytesMut::with_capacity(serialized.len() + 10);
        protected_data.extend_from_slice(&serialized);

        // Add the auth tag to the protected data
        if let Some(tag) = auth_tag {
            protected_data.extend_from_slice(&tag);
        }
        let protected_data = protected_data.freeze();

        // Test 1: Verify normal decryption works
        let decrypted = crypto.decrypt_rtp(&protected_data, 0);
        assert!(decrypted.is_ok());

        // Test 2: Tamper with the payload and verify it fails authentication
        let _tampered_size = protected_data.len();
        let mut tampered = protected_data.to_vec();

        // Change one byte in the middle of the packet
        let middle = tampered.len() / 2;
        tampered[middle] ^= 0xFF;

        let decrypted = crypto.decrypt_rtp(&tampered, 0);
        assert!(decrypted.is_err());

        // Test 3: Tamper with the authentication tag and verify it fails
        let mut tampered = protected_data.to_vec();
        if let Some(_tag) = auth_tag_clone {
            // Calculate position of the last byte in the auth tag
            let tag_idx = tampered.len() - 1;
            // Store the value before changing it
            let tag_value = tampered[tag_idx];
            // Flip the bits in the last byte
            tampered[tag_idx] = tag_value ^ 0xFF;

            let decrypted = crypto.decrypt_rtp(&tampered, 0);
            assert!(decrypted.is_err());
        }
    }
}
