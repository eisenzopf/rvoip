//! DTLS cryptography ciphers
//!
//! This module implements the cryptographic ciphers used in DTLS.

use std::fmt;

use crate::dtls::Result;
use bytes::{Bytes, BytesMut, BufMut};

// Add crypto imports
use aes::{Aes128, Aes256};
use aes::cipher::{
    BlockCipher, BlockEncrypt, BlockDecrypt,
    KeyInit, Key,
};
use aes_gcm::{
    Aes128Gcm, Aes256Gcm, AesGcm, KeySizeUser,
    aead::{Aead, Payload, Tag, Nonce}
};
use ctr::{Ctr128BE, Ctr32BE};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha384};

// Type aliases for HMAC implementations
type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;
type HmacSha384 = Hmac<Sha384>;

/// DTLS cipher suite identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum CipherSuiteId {
    /// TLS_RSA_WITH_AES_128_CBC_SHA (0x002F)
    TLS_RSA_WITH_AES_128_CBC_SHA = 0x002F,
    
    /// TLS_RSA_WITH_AES_256_CBC_SHA (0x0035)
    TLS_RSA_WITH_AES_256_CBC_SHA = 0x0035,
    
    /// TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA (0xC009)
    TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA = 0xC009,
    
    /// TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA (0xC00A)
    TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA = 0xC00A,
    
    /// TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA (0xC013)
    TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA = 0xC013,
    
    /// TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA (0xC014)
    TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA = 0xC014,
    
    /// TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 (0xC02B)
    TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 = 0xC02B,
    
    /// TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 (0xC02C)
    TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 = 0xC02C,
    
    /// TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 (0xC02F)
    TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 = 0xC02F,
    
    /// TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 (0xC030)
    TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 = 0xC030,
}

impl CipherSuiteId {
    /// Check if this cipher suite uses GCM
    pub fn is_gcm(&self) -> bool {
        matches!(
            self,
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
                | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
                | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
                | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
        )
    }
    
    /// Check if this cipher suite uses ECDSA
    pub fn is_ecdsa(&self) -> bool {
        matches!(
            self,
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
                | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA
                | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
                | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
        )
    }
    
    /// Check if this cipher suite uses RSA
    pub fn is_rsa(&self) -> bool {
        matches!(
            self,
            CipherSuiteId::TLS_RSA_WITH_AES_128_CBC_SHA
                | CipherSuiteId::TLS_RSA_WITH_AES_256_CBC_SHA
                | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA
                | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA
                | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
                | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
        )
    }
    
    /// Get the key exchange algorithm
    pub fn key_exchange(&self) -> KeyExchangeAlgorithm {
        match self {
            CipherSuiteId::TLS_RSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_RSA_WITH_AES_256_CBC_SHA => KeyExchangeAlgorithm::Rsa,
            
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => KeyExchangeAlgorithm::EcDhe,
        }
    }
    
    /// Get the cipher type
    pub fn cipher(&self) -> CipherType {
        match self {
            CipherSuiteId::TLS_RSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA => CipherType::Aes128Cbc,
            
            CipherSuiteId::TLS_RSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA => CipherType::Aes256Cbc,
            
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => CipherType::Aes128Gcm,
            
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => CipherType::Aes256Gcm,
        }
    }
    
    /// Get the MAC algorithm
    pub fn mac(&self) -> MacAlgorithm {
        match self {
            CipherSuiteId::TLS_RSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_RSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA => MacAlgorithm::HmacSha1,
            
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => MacAlgorithm::HmacSha256,
            
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => MacAlgorithm::HmacSha384,
        }
    }
    
    /// Get the hash function
    pub fn hash(&self) -> HashAlgorithm {
        match self {
            CipherSuiteId::TLS_RSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_RSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA => HashAlgorithm::Sha1,
            
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => HashAlgorithm::Sha256,
            
            CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
            | CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => HashAlgorithm::Sha384,
        }
    }
}

impl TryFrom<u16> for CipherSuiteId {
    type Error = crate::error::Error;

    fn try_from(value: u16) -> std::result::Result<Self, Self::Error> {
        match value {
            0x002F => Ok(CipherSuiteId::TLS_RSA_WITH_AES_128_CBC_SHA),
            0x0035 => Ok(CipherSuiteId::TLS_RSA_WITH_AES_256_CBC_SHA),
            0xC009 => Ok(CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA),
            0xC00A => Ok(CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA),
            0xC013 => Ok(CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA),
            0xC014 => Ok(CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA),
            0xC02B => Ok(CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256),
            0xC02C => Ok(CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384),
            0xC02F => Ok(CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256),
            0xC030 => Ok(CipherSuiteId::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384),
            _ => Err(crate::error::Error::UnsupportedFeature(
                format!("Unsupported cipher suite: 0x{:04X}", value),
            )),
        }
    }
}

impl From<CipherSuiteId> for u16 {
    fn from(id: CipherSuiteId) -> Self {
        id as u16
    }
}

/// Key exchange algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyExchangeAlgorithm {
    /// RSA key exchange
    Rsa,
    
    /// Ephemeral ECDH key exchange
    EcDhe,
}

/// Cipher type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherType {
    /// AES-128 in CBC mode
    Aes128Cbc,
    
    /// AES-256 in CBC mode
    Aes256Cbc,
    
    /// AES-128 in GCM mode
    Aes128Gcm,
    
    /// AES-256 in GCM mode
    Aes256Gcm,
}

impl CipherType {
    /// Get the key size in bytes
    pub fn key_size(&self) -> usize {
        match self {
            CipherType::Aes128Cbc | CipherType::Aes128Gcm => 16,
            CipherType::Aes256Cbc | CipherType::Aes256Gcm => 32,
        }
    }
    
    /// Get the IV size in bytes
    pub fn iv_size(&self) -> usize {
        match self {
            CipherType::Aes128Cbc | CipherType::Aes256Cbc => 16,
            CipherType::Aes128Gcm | CipherType::Aes256Gcm => 12,
        }
    }
    
    /// Check if this cipher uses GCM mode
    pub fn is_gcm(&self) -> bool {
        matches!(self, CipherType::Aes128Gcm | CipherType::Aes256Gcm)
    }
}

impl fmt::Display for CipherType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CipherType::Aes128Cbc => write!(f, "AES-128-CBC"),
            CipherType::Aes256Cbc => write!(f, "AES-256-CBC"),
            CipherType::Aes128Gcm => write!(f, "AES-128-GCM"),
            CipherType::Aes256Gcm => write!(f, "AES-256-GCM"),
        }
    }
}

/// MAC algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacAlgorithm {
    /// HMAC-SHA1
    HmacSha1,
    
    /// HMAC-SHA256
    HmacSha256,
    
    /// HMAC-SHA384
    HmacSha384,
}

impl MacAlgorithm {
    /// Get the hash size in bytes
    pub fn hash_size(&self) -> usize {
        match self {
            MacAlgorithm::HmacSha1 => 20,
            MacAlgorithm::HmacSha256 => 32,
            MacAlgorithm::HmacSha384 => 48,
        }
    }
}

impl fmt::Display for MacAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MacAlgorithm::HmacSha1 => write!(f, "HMAC-SHA1"),
            MacAlgorithm::HmacSha256 => write!(f, "HMAC-SHA256"),
            MacAlgorithm::HmacSha384 => write!(f, "HMAC-SHA384"),
        }
    }
}

/// Hash algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// SHA-1
    Sha1,
    
    /// SHA-256
    Sha256,
    
    /// SHA-384
    Sha384,
}

impl HashAlgorithm {
    /// Get the hash size in bytes
    pub fn hash_size(&self) -> usize {
        match self {
            HashAlgorithm::Sha1 => 20,
            HashAlgorithm::Sha256 => 32,
            HashAlgorithm::Sha384 => 48,
        }
    }
}

impl fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HashAlgorithm::Sha1 => write!(f, "SHA-1"),
            HashAlgorithm::Sha256 => write!(f, "SHA-256"),
            HashAlgorithm::Sha384 => write!(f, "SHA-384"),
        }
    }
}

/// Encryptor for protecting DTLS packets
pub trait Encryptor {
    /// Encrypt plaintext data
    ///
    /// The `epoch` and `sequence_number` are used to derive a unique per-record
    /// nonce for AEAD ciphers (RFC 6347 / RFC 5116). The 2-byte epoch and 6-byte
    /// sequence number form the 8-byte explicit nonce which is XORed with the
    /// implicit IV from the key material.
    fn encrypt(&self, plaintext: &[u8], additional_data: &[u8], epoch: u16, sequence_number: u64) -> Result<Bytes>;
}

/// Decryptor for unprotecting DTLS packets
pub trait Decryptor {
    /// Decrypt ciphertext data
    ///
    /// The `epoch` and `sequence_number` are used to derive the unique per-record
    /// nonce for AEAD ciphers (RFC 6347 / RFC 5116). Must match the values used
    /// during encryption for the corresponding record.
    fn decrypt(&self, ciphertext: &[u8], additional_data: &[u8], epoch: u16, sequence_number: u64) -> Result<Bytes>;
}

/// AEAD implementation for GCM ciphers
///
/// Per RFC 6347 (DTLS 1.2) and RFC 5116, the nonce for each record is derived
/// by XORing the implicit IV (from key material) with an 8-byte explicit nonce
/// composed of the 2-byte epoch and 6-byte sequence number, left-padded to the
/// IV length (12 bytes for GCM). Reusing a nonce with GCM is catastrophic and
/// allows key recovery, so this struct enforces per-record nonce derivation.
pub struct AeadImpl {
    /// Cipher key
    key: Bytes,

    /// Implicit IV (write_iv from key material, used as base for nonce derivation)
    implicit_iv: Bytes,

    /// Cipher type
    cipher_type: CipherType,
}

impl AeadImpl {
    /// Create a new AEAD implementation
    ///
    /// The `implicit_iv` parameter is the write IV from the TLS key material.
    /// It is NOT used directly as the nonce; instead, it is XORed with the
    /// per-record explicit nonce (epoch + sequence number) to produce a unique
    /// nonce for each record.
    pub fn new(key: Bytes, implicit_iv: Bytes, cipher_type: CipherType) -> Self {
        Self {
            key,
            implicit_iv,
            cipher_type,
        }
    }

    /// Derive a per-record nonce from the implicit IV and the record's
    /// epoch + sequence number, per RFC 6347 Section 4.1.2.1.
    ///
    /// The explicit nonce is formed as:
    ///   bytes[0..2]  = epoch (big-endian)
    ///   bytes[2..8]  = sequence_number (big-endian, 48-bit)
    ///
    /// This 8-byte value is left-padded with zeros to match the IV length
    /// (12 bytes for GCM), then XORed with the implicit IV.
    fn derive_nonce(&self, epoch: u16, sequence_number: u64) -> Result<Vec<u8>> {
        let iv_len = self.implicit_iv.len();
        if iv_len == 0 {
            return Err(crate::error::Error::CryptoError(
                "Implicit IV has zero length".to_string()
            ));
        }

        // Build the 8-byte explicit nonce: 2 bytes epoch + 6 bytes sequence number
        let mut explicit_nonce = [0u8; 8];
        explicit_nonce[0] = (epoch >> 8) as u8;
        explicit_nonce[1] = epoch as u8;
        // sequence_number is a 48-bit value (6 bytes), stored in the lower 48 bits of u64
        explicit_nonce[2] = ((sequence_number >> 40) & 0xFF) as u8;
        explicit_nonce[3] = ((sequence_number >> 32) & 0xFF) as u8;
        explicit_nonce[4] = ((sequence_number >> 24) & 0xFF) as u8;
        explicit_nonce[5] = ((sequence_number >> 16) & 0xFF) as u8;
        explicit_nonce[6] = ((sequence_number >> 8) & 0xFF) as u8;
        explicit_nonce[7] = (sequence_number & 0xFF) as u8;

        // Left-pad to IV length (for GCM, iv_len = 12, so 4 zero bytes + 8 explicit bytes)
        let mut padded = vec![0u8; iv_len];
        if iv_len >= 8 {
            padded[iv_len - 8..].copy_from_slice(&explicit_nonce);
        } else {
            // IV shorter than 8 bytes: take the rightmost iv_len bytes of explicit_nonce
            padded.copy_from_slice(&explicit_nonce[8 - iv_len..]);
        }

        // XOR with implicit IV
        for (i, byte) in padded.iter_mut().enumerate() {
            *byte ^= self.implicit_iv[i];
        }

        Ok(padded)
    }
}

impl Encryptor for AeadImpl {
    fn encrypt(&self, plaintext: &[u8], additional_data: &[u8], epoch: u16, sequence_number: u64) -> Result<Bytes> {
        // Derive a unique per-record nonce from implicit IV XOR (epoch || sequence_number)
        let nonce_bytes = self.derive_nonce(epoch, sequence_number)?;

        // Encrypt the data based on cipher type
        match self.cipher_type {
            CipherType::Aes128Gcm => {
                let expected_nonce_len = 12; // AES-128-GCM nonce size
                if nonce_bytes.len() != expected_nonce_len {
                    return Err(crate::error::Error::CryptoError(
                        format!("AES-128-GCM nonce length mismatch: expected {}, got {}", expected_nonce_len, nonce_bytes.len())
                    ));
                }
                let nonce = Nonce::<Aes128Gcm>::from_slice(&nonce_bytes);
                let cipher = Aes128Gcm::new_from_slice(&self.key)
                    .map_err(|e| crate::error::Error::CryptoError(format!("Failed to initialize AES-128-GCM: {}", e)))?;

                let payload = Payload {
                    msg: plaintext,
                    aad: additional_data,
                };

                let ciphertext = cipher.encrypt(nonce, payload)
                    .map_err(|e| crate::error::Error::CryptoError(format!("AEAD encryption failed: {}", e)))?;

                Ok(Bytes::from(ciphertext))
            },
            CipherType::Aes256Gcm => {
                let expected_nonce_len = 12; // AES-256-GCM nonce size
                if nonce_bytes.len() != expected_nonce_len {
                    return Err(crate::error::Error::CryptoError(
                        format!("AES-256-GCM nonce length mismatch: expected {}, got {}", expected_nonce_len, nonce_bytes.len())
                    ));
                }
                let nonce = Nonce::<Aes256Gcm>::from_slice(&nonce_bytes);
                let cipher = Aes256Gcm::new_from_slice(&self.key)
                    .map_err(|e| crate::error::Error::CryptoError(format!("Failed to initialize AES-256-GCM: {}", e)))?;

                let payload = Payload {
                    msg: plaintext,
                    aad: additional_data,
                };

                let ciphertext = cipher.encrypt(nonce, payload)
                    .map_err(|e| crate::error::Error::CryptoError(format!("AEAD encryption failed: {}", e)))?;

                Ok(Bytes::from(ciphertext))
            },
            _ => Err(crate::error::Error::UnsupportedFeature(
                format!("Cipher type {} is not supported for AEAD", self.cipher_type)
            )),
        }
    }
}

impl Decryptor for AeadImpl {
    fn decrypt(&self, ciphertext: &[u8], additional_data: &[u8], epoch: u16, sequence_number: u64) -> Result<Bytes> {
        // Derive the same per-record nonce used during encryption
        let nonce_bytes = self.derive_nonce(epoch, sequence_number)?;

        // Decrypt the data based on cipher type
        match self.cipher_type {
            CipherType::Aes128Gcm => {
                let expected_nonce_len = 12; // AES-128-GCM nonce size
                if nonce_bytes.len() != expected_nonce_len {
                    return Err(crate::error::Error::CryptoError(
                        format!("AES-128-GCM nonce length mismatch: expected {}, got {}", expected_nonce_len, nonce_bytes.len())
                    ));
                }
                let nonce = Nonce::<Aes128Gcm>::from_slice(&nonce_bytes);
                let cipher = Aes128Gcm::new_from_slice(&self.key)
                    .map_err(|e| crate::error::Error::CryptoError(format!("Failed to initialize AES-128-GCM: {}", e)))?;

                let payload = Payload {
                    msg: ciphertext,
                    aad: additional_data,
                };

                let plaintext = cipher.decrypt(nonce, payload)
                    .map_err(|e| crate::error::Error::CryptoError(format!("AEAD decryption failed: {}", e)))?;

                Ok(Bytes::from(plaintext))
            },
            CipherType::Aes256Gcm => {
                let expected_nonce_len = 12; // AES-256-GCM nonce size
                if nonce_bytes.len() != expected_nonce_len {
                    return Err(crate::error::Error::CryptoError(
                        format!("AES-256-GCM nonce length mismatch: expected {}, got {}", expected_nonce_len, nonce_bytes.len())
                    ));
                }
                let nonce = Nonce::<Aes256Gcm>::from_slice(&nonce_bytes);
                let cipher = Aes256Gcm::new_from_slice(&self.key)
                    .map_err(|e| crate::error::Error::CryptoError(format!("Failed to initialize AES-256-GCM: {}", e)))?;

                let payload = Payload {
                    msg: ciphertext,
                    aad: additional_data,
                };

                let plaintext = cipher.decrypt(nonce, payload)
                    .map_err(|e| crate::error::Error::CryptoError(format!("AEAD decryption failed: {}", e)))?;

                Ok(Bytes::from(plaintext))
            },
            _ => Err(crate::error::Error::UnsupportedFeature(
                format!("Cipher type {} is not supported for AEAD", self.cipher_type)
            )),
        }
    }
}

/// BlockCipher implementation for CBC ciphers
pub struct BlockCipherImpl {
    /// Cipher key
    key: Bytes,
    
    /// Initialization vector
    iv: Bytes,
    
    /// Cipher type
    cipher_type: CipherType,
    
    /// MAC key
    mac_key: Bytes,
    
    /// MAC algorithm
    mac_algorithm: MacAlgorithm,
}

impl BlockCipherImpl {
    /// Create a new block cipher implementation
    pub fn new(
        key: Bytes,
        iv: Bytes,
        cipher_type: CipherType,
        mac_key: Bytes,
        mac_algorithm: MacAlgorithm,
    ) -> Self {
        Self {
            key,
            iv,
            cipher_type,
            mac_key,
            mac_algorithm,
        }
    }

    /// Compute the MAC for the given data
    fn compute_mac(&self, data: &[u8]) -> Result<Bytes> {
        match self.mac_algorithm {
            MacAlgorithm::HmacSha1 => {
                let mut mac = <HmacSha1 as hmac::Mac>::new_from_slice(&self.mac_key)
                    .map_err(|e| crate::error::Error::CryptoError(format!("Failed to initialize HMAC-SHA1: {}", e)))?;
                mac.update(data);
                let result = mac.finalize().into_bytes();
                Ok(Bytes::copy_from_slice(&result))
            },
            MacAlgorithm::HmacSha256 => {
                let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(&self.mac_key)
                    .map_err(|e| crate::error::Error::CryptoError(format!("Failed to initialize HMAC-SHA256: {}", e)))?;
                mac.update(data);
                let result = mac.finalize().into_bytes();
                Ok(Bytes::copy_from_slice(&result))
            },
            MacAlgorithm::HmacSha384 => {
                let mut mac = <HmacSha384 as hmac::Mac>::new_from_slice(&self.mac_key)
                    .map_err(|e| crate::error::Error::CryptoError(format!("Failed to initialize HMAC-SHA384: {}", e)))?;
                mac.update(data);
                let result = mac.finalize().into_bytes();
                Ok(Bytes::copy_from_slice(&result))
            }
        }
    }

    /// Verify the MAC for the given data
    fn verify_mac(&self, data: &[u8], expected_mac: &[u8]) -> Result<bool> {
        let computed_mac = self.compute_mac(data)?;
        
        // Constant-time comparison to prevent timing attacks
        if computed_mac.len() != expected_mac.len() {
            return Ok(false);
        }
        
        let mut result = 0;
        for (a, b) in computed_mac.iter().zip(expected_mac.iter()) {
            result |= a ^ b;
        }
        
        Ok(result == 0)
    }
}

impl Encryptor for BlockCipherImpl {
    fn encrypt(&self, plaintext: &[u8], additional_data: &[u8], _epoch: u16, _sequence_number: u64) -> Result<Bytes> {
        // First compute the MAC over the additional data and plaintext
        let mut mac_input = BytesMut::with_capacity(additional_data.len() + plaintext.len());
        mac_input.extend_from_slice(additional_data);
        mac_input.extend_from_slice(plaintext);
        
        let mac = self.compute_mac(&mac_input)?;
        
        // Pad the plaintext to a multiple of the block size (16 bytes for AES)
        let block_size = 16;
        let padding_len = block_size - ((plaintext.len() + mac.len()) % block_size);
        let padding_value = (padding_len - 1) as u8;
        
        let mut padded_data = BytesMut::with_capacity(plaintext.len() + mac.len() + padding_len);
        padded_data.extend_from_slice(plaintext);
        padded_data.extend_from_slice(&mac);
        for _ in 0..padding_len {
            padded_data.put_u8(padding_value);
        }
        
        // Encrypt the data
        let encrypted = match self.cipher_type {
            CipherType::Aes128Cbc => {
                let key = Key::<Aes128>::from_slice(&self.key);
                let cipher = Aes128::new(key);
                
                // Implement CBC mode encryption
                let mut ciphertext = BytesMut::with_capacity(padded_data.len());
                let mut iv = self.iv.clone();
                
                for chunk in padded_data.chunks(16) {
                    let mut block = [0u8; 16];
                    block.copy_from_slice(chunk);
                    
                    // XOR with IV
                    for i in 0..16 {
                        block[i] ^= iv[i];
                    }
                    
                    // Encrypt the block
                    cipher.encrypt_block((&mut block).into());
                    
                    // Add to ciphertext
                    ciphertext.extend_from_slice(&block);
                    
                    // Update IV for next block
                    iv = Bytes::copy_from_slice(&block);
                }
                
                ciphertext.freeze()
            },
            CipherType::Aes256Cbc => {
                let key = Key::<Aes256>::from_slice(&self.key);
                let cipher = Aes256::new(key);
                
                // Implement CBC mode encryption (same as above)
                let mut ciphertext = BytesMut::with_capacity(padded_data.len());
                let mut iv = self.iv.clone();
                
                for chunk in padded_data.chunks(16) {
                    let mut block = [0u8; 16];
                    block.copy_from_slice(chunk);
                    
                    // XOR with IV
                    for i in 0..16 {
                        block[i] ^= iv[i];
                    }
                    
                    // Encrypt the block
                    cipher.encrypt_block((&mut block).into());
                    
                    // Add to ciphertext
                    ciphertext.extend_from_slice(&block);
                    
                    // Update IV for next block
                    iv = Bytes::copy_from_slice(&block);
                }
                
                ciphertext.freeze()
            },
            _ => return Err(crate::error::Error::UnsupportedFeature(
                format!("Cipher type {} is not supported for block cipher", self.cipher_type)
            )),
        };
        
        Ok(encrypted)
    }
}

impl Decryptor for BlockCipherImpl {
    fn decrypt(&self, ciphertext: &[u8], additional_data: &[u8], _epoch: u16, _sequence_number: u64) -> Result<Bytes> {
        // Make sure ciphertext is a multiple of the block size
        if ciphertext.len() % 16 != 0 {
            return Err(crate::error::Error::InvalidPacket("Ciphertext length is not a multiple of block size".to_string()));
        }
        
        // Decrypt the data
        let decrypted = match self.cipher_type {
            CipherType::Aes128Cbc => {
                let key = Key::<Aes128>::from_slice(&self.key);
                let cipher = Aes128::new(key);
                
                // Implement CBC mode decryption
                let mut plaintext = BytesMut::with_capacity(ciphertext.len());
                let mut iv = self.iv.clone();
                
                for chunk in ciphertext.chunks(16) {
                    let mut block = [0u8; 16];
                    block.copy_from_slice(chunk);
                    
                    // Save current ciphertext block for next IV
                    let current_block = Bytes::copy_from_slice(&block);
                    
                    // Decrypt the block
                    cipher.decrypt_block((&mut block).into());
                    
                    // XOR with IV
                    for i in 0..16 {
                        block[i] ^= iv[i];
                    }
                    
                    // Add to plaintext
                    plaintext.extend_from_slice(&block);
                    
                    // Update IV for next block
                    iv = current_block;
                }
                
                plaintext.freeze()
            },
            CipherType::Aes256Cbc => {
                let key = Key::<Aes256>::from_slice(&self.key);
                let cipher = Aes256::new(key);
                
                // Implement CBC mode decryption (same as above)
                let mut plaintext = BytesMut::with_capacity(ciphertext.len());
                let mut iv = self.iv.clone();
                
                for chunk in ciphertext.chunks(16) {
                    let mut block = [0u8; 16];
                    block.copy_from_slice(chunk);
                    
                    // Save current ciphertext block for next IV
                    let current_block = Bytes::copy_from_slice(&block);
                    
                    // Decrypt the block
                    cipher.decrypt_block((&mut block).into());
                    
                    // XOR with IV
                    for i in 0..16 {
                        block[i] ^= iv[i];
                    }
                    
                    // Add to plaintext
                    plaintext.extend_from_slice(&block);
                    
                    // Update IV for next block
                    iv = current_block;
                }
                
                plaintext.freeze()
            },
            _ => return Err(crate::error::Error::UnsupportedFeature(
                format!("Cipher type {} is not supported for block cipher", self.cipher_type)
            )),
        };
        
        // Remove padding
        let padding_value = decrypted[decrypted.len() - 1];
        let padding_len = padding_value as usize + 1;
        
        if padding_len > decrypted.len() {
            return Err(crate::error::Error::InvalidPacket("Invalid padding length".to_string()));
        }
        
        // Verify padding
        for i in 1..=padding_len {
            if decrypted[decrypted.len() - i] != padding_value {
                return Err(crate::error::Error::InvalidPacket("Invalid padding".to_string()));
            }
        }
        
        // Extract plaintext and MAC
        let mac_size = self.mac_algorithm.hash_size();
        let plaintext_len = decrypted.len() - padding_len - mac_size;
        
        let plaintext = &decrypted[..plaintext_len];
        let received_mac = &decrypted[plaintext_len..plaintext_len + mac_size];
        
        // Verify MAC
        let mut mac_input = BytesMut::with_capacity(additional_data.len() + plaintext.len());
        mac_input.extend_from_slice(additional_data);
        mac_input.extend_from_slice(plaintext);
        
        if !self.verify_mac(&mac_input, received_mac)? {
            return Err(crate::error::Error::InvalidPacket("MAC verification failed".to_string()));
        }
        
        Ok(Bytes::copy_from_slice(plaintext))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cipher_suite_properties() {
        let suite = CipherSuiteId::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256;
        assert!(suite.is_gcm());
        assert!(suite.is_ecdsa());
        assert!(!suite.is_rsa());
        assert_eq!(suite.key_exchange(), KeyExchangeAlgorithm::EcDhe);
        assert_eq!(suite.cipher(), CipherType::Aes128Gcm);
        assert_eq!(suite.mac(), MacAlgorithm::HmacSha256);
        assert_eq!(suite.hash(), HashAlgorithm::Sha256);
        
        let suite = CipherSuiteId::TLS_RSA_WITH_AES_256_CBC_SHA;
        assert!(!suite.is_gcm());
        assert!(!suite.is_ecdsa());
        assert!(suite.is_rsa());
        assert_eq!(suite.key_exchange(), KeyExchangeAlgorithm::Rsa);
        assert_eq!(suite.cipher(), CipherType::Aes256Cbc);
        assert_eq!(suite.mac(), MacAlgorithm::HmacSha1);
        assert_eq!(suite.hash(), HashAlgorithm::Sha1);
    }
    
    #[test]
    fn test_cipher_type_properties() {
        let cipher = CipherType::Aes128Gcm;
        assert_eq!(cipher.key_size(), 16);
        assert_eq!(cipher.iv_size(), 12);
        assert!(cipher.is_gcm());
        
        let cipher = CipherType::Aes256Cbc;
        assert_eq!(cipher.key_size(), 32);
        assert_eq!(cipher.iv_size(), 16);
        assert!(!cipher.is_gcm());
    }
    
    #[test]
    fn test_mac_algorithm_properties() {
        let mac = MacAlgorithm::HmacSha1;
        assert_eq!(mac.hash_size(), 20);

        let mac = MacAlgorithm::HmacSha256;
        assert_eq!(mac.hash_size(), 32);

        let mac = MacAlgorithm::HmacSha384;
        assert_eq!(mac.hash_size(), 48);
    }

    #[test]
    fn test_aead_nonce_derivation_basic() {
        // 12-byte implicit IV (GCM standard)
        let implicit_iv = Bytes::from(vec![0xAA; 12]);
        let aead = AeadImpl::new(
            Bytes::from(vec![0u8; 16]), // key (unused for nonce test)
            implicit_iv.clone(),
            CipherType::Aes128Gcm,
        );

        let nonce = aead.derive_nonce(0, 0).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));
        // epoch=0, seq=0 => explicit nonce is all zeros => padded is all zeros
        // XOR with 0xAA... => all 0xAA
        assert_eq!(nonce, vec![0xAA; 12]);
    }

    #[test]
    fn test_aead_nonce_derivation_with_epoch_and_seq() {
        let implicit_iv = Bytes::from(vec![0x00; 12]);
        let aead = AeadImpl::new(
            Bytes::from(vec![0u8; 16]),
            implicit_iv,
            CipherType::Aes128Gcm,
        );

        // epoch=1, sequence_number=5
        let nonce = aead.derive_nonce(1, 5).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));
        // explicit_nonce = [0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05]
        // padded (12 bytes) = [0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05]
        // XOR with all-zero IV => same as padded
        let expected = vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05];
        assert_eq!(nonce, expected);
    }

    #[test]
    fn test_aead_nonce_derivation_xor_with_iv() {
        // Non-zero IV to verify XOR behavior
        let implicit_iv = Bytes::from(vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
        ]);
        let aead = AeadImpl::new(
            Bytes::from(vec![0u8; 16]),
            implicit_iv,
            CipherType::Aes128Gcm,
        );

        // epoch=0, seq=0 => padded is all zeros => nonce = IV itself
        let nonce = aead.derive_nonce(0, 0).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));
        assert_eq!(nonce, vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C]);

        // epoch=0, seq=1 => padded = [0,0,0,0, 0,0,0,0, 0,0,0,1]
        // XOR: last byte = 0x0C ^ 0x01 = 0x0D
        let nonce = aead.derive_nonce(0, 1).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));
        assert_eq!(nonce[11], 0x0C ^ 0x01);
    }

    #[test]
    fn test_aead_different_sequences_produce_different_nonces() {
        let implicit_iv = Bytes::from(vec![0x42; 12]);
        let aead = AeadImpl::new(
            Bytes::from(vec![0u8; 16]),
            implicit_iv,
            CipherType::Aes128Gcm,
        );

        let nonce_0 = aead.derive_nonce(1, 0).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));
        let nonce_1 = aead.derive_nonce(1, 1).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));
        let nonce_2 = aead.derive_nonce(1, 2).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));
        let nonce_diff_epoch = aead.derive_nonce(2, 0).unwrap_or_else(|e| panic!("derive_nonce failed: {}", e));

        // All nonces must be unique
        assert_ne!(nonce_0, nonce_1);
        assert_ne!(nonce_1, nonce_2);
        assert_ne!(nonce_0, nonce_diff_epoch);
    }

    #[test]
    fn test_aead_encrypt_decrypt_roundtrip_aes128gcm() {
        let key = Bytes::from(vec![0x55u8; 16]);
        let iv = Bytes::from(vec![0xBB; 12]);
        let aead = AeadImpl::new(key, iv, CipherType::Aes128Gcm);

        let plaintext = b"hello DTLS world";
        let aad = b"additional data";
        let epoch = 1u16;
        let seq = 42u64;

        let ciphertext = aead.encrypt(plaintext, aad, epoch, seq)
            .unwrap_or_else(|e| panic!("encrypt failed: {}", e));
        let decrypted = aead.decrypt(&ciphertext, aad, epoch, seq)
            .unwrap_or_else(|e| panic!("decrypt failed: {}", e));

        assert_eq!(&decrypted[..], plaintext);
    }

    #[test]
    fn test_aead_decrypt_wrong_sequence_fails() {
        let key = Bytes::from(vec![0x55u8; 16]);
        let iv = Bytes::from(vec![0xBB; 12]);
        let aead = AeadImpl::new(key, iv, CipherType::Aes128Gcm);

        let plaintext = b"secret message";
        let aad = b"aad";
        let epoch = 1u16;
        let seq = 10u64;

        let ciphertext = aead.encrypt(plaintext, aad, epoch, seq)
            .unwrap_or_else(|e| panic!("encrypt failed: {}", e));

        // Decrypting with a different sequence number must fail (wrong nonce)
        let result = aead.decrypt(&ciphertext, aad, epoch, seq + 1);
        assert!(result.is_err(), "Decryption with wrong sequence number should fail");
    }

    #[test]
    fn test_aead_encrypt_decrypt_roundtrip_aes256gcm() {
        let key = Bytes::from(vec![0x77u8; 32]);
        let iv = Bytes::from(vec![0xCC; 12]);
        let aead = AeadImpl::new(key, iv, CipherType::Aes256Gcm);

        let plaintext = b"AES-256-GCM test data";
        let aad = b"more aad";
        let epoch = 2u16;
        let seq = 100u64;

        let ciphertext = aead.encrypt(plaintext, aad, epoch, seq)
            .unwrap_or_else(|e| panic!("encrypt failed: {}", e));
        let decrypted = aead.decrypt(&ciphertext, aad, epoch, seq)
            .unwrap_or_else(|e| panic!("decrypt failed: {}", e));

        assert_eq!(&decrypted[..], plaintext);
    }
}
