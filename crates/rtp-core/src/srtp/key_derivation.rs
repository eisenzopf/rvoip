use super::crypto::SrtpCryptoKey;
use crate::error::Error;
use crate::Result;
use aes::{
    cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit},
    Aes128, Aes256,
};

/// SRTP key derivation parameters
/// Based on RFC 3711 Section 4.3
#[derive(Debug, Clone)]
pub struct SrtpKeyDerivationParams {
    /// Label values for different key types
    pub label: KeyDerivationLabel,

    /// Key derivation rate
    pub key_derivation_rate: u64,

    /// Index for the key derivation
    pub index: u64,
}

/// Label values for SRTP key derivation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyDerivationLabel {
    /// RTP encryption key
    RtpEncryption = 0,

    /// RTP authentication key
    RtpAuthentication = 1,

    /// RTP salt (for IV creation)
    RtpSalt = 2,

    /// RTCP encryption key
    RtcpEncryption = 3,

    /// RTCP authentication key
    RtcpAuthentication = 4,

    /// RTCP salt (for IV creation)
    RtcpSalt = 5,
}

/// Perform key derivation function as specified in RFC 3711
///
/// # Arguments
/// * `master_key` - The master key to derive from
/// * `params` - Key derivation parameters
/// * `output_len` - Length of the derived key
pub fn srtp_kdf(
    master_key: &SrtpCryptoKey,
    params: &SrtpKeyDerivationParams,
    output_len: usize,
) -> Result<Vec<u8>> {
    if master_key.salt().len() < 14 {
        return Err(Error::SrtpError(format!(
            "Salt too short: expected at least 14 bytes, got {}",
            master_key.salt().len()
        )));
    }

    let r = if params.key_derivation_rate == 0 {
        0
    } else {
        params.index / params.key_derivation_rate
    };
    if r > 0x0000_FFFF_FFFF_FFFF {
        return Err(Error::SrtpError(format!(
            "Key derivation index too large for 48-bit SRTP key-id: {}",
            r
        )));
    }

    // RFC 3711 Section 4.3.1: x = key_id XOR master_salt, where key_id is the
    // right-aligned 7-octet value label || (index DIV key_derivation_rate).
    let mut x = [0u8; 14];
    x.copy_from_slice(&master_key.salt()[0..14]);
    x[7] ^= params.label as u8;
    for i in 0..6 {
        x[8 + i] ^= ((r >> (8 * (5 - i))) & 0xFF) as u8;
    }

    // Determine number of blocks needed
    let num_blocks = (output_len + 15) / 16;

    // Create buffer for key material
    let mut key_material = Vec::with_capacity(num_blocks * 16);

    match master_key.key().len() {
        16 => {
            let cipher = Aes128::new_from_slice(master_key.key())
                .map_err(|e| Error::SrtpError(format!("Failed to create AES-128 cipher: {}", e)))?;
            for i in 0..num_blocks {
                let mut iv = [0u8; 16];
                iv[0..14].copy_from_slice(&x);
                iv[14] = ((i >> 8) & 0xFF) as u8;
                iv[15] = (i & 0xFF) as u8;

                let mut block = GenericArray::clone_from_slice(&iv);
                cipher.encrypt_block(&mut block);
                key_material.extend_from_slice(&block);
            }
        }
        32 => {
            let cipher = Aes256::new_from_slice(master_key.key())
                .map_err(|e| Error::SrtpError(format!("Failed to create AES-256 cipher: {}", e)))?;
            for i in 0..num_blocks {
                let mut iv = [0u8; 16];
                iv[0..14].copy_from_slice(&x);
                iv[14] = ((i >> 8) & 0xFF) as u8;
                iv[15] = (i & 0xFF) as u8;

                let mut block = GenericArray::clone_from_slice(&iv);
                cipher.encrypt_block(&mut block);
                key_material.extend_from_slice(&block);
            }
        }
        len => {
            return Err(Error::SrtpError(format!(
                "unsupported SRTP KDF master key length: {} bytes",
                len
            )));
        }
    }

    // Truncate to the requested size
    key_material.truncate(output_len);

    Ok(key_material)
}

/// Create initialization vector (IV) for SRTP
///
/// # Arguments
/// * `salt` - The salt value
/// * `ssrc` - Synchronization source identifier
/// * `packet_index` - Index of the packet
pub fn create_srtp_iv(salt: &[u8], ssrc: u32, packet_index: u64) -> Result<Vec<u8>> {
    if salt.len() < 14 {
        return Err(Error::SrtpError(format!(
            "Salt too short: expected at least 14 bytes, got {}",
            salt.len()
        )));
    }

    // Create an IV according to RFC 3711 Section 4.1.2
    let mut iv = Vec::with_capacity(16);
    iv.extend_from_slice(&salt[0..14]);

    // Set the last two bytes to zero
    iv.push(0);
    iv.push(0);

    // XOR the salt with the SSRC and packet index
    // SSRC goes into bytes 4-7 (indexed 0)
    iv[4] ^= ((ssrc >> 24) & 0xFF) as u8;
    iv[5] ^= ((ssrc >> 16) & 0xFF) as u8;
    iv[6] ^= ((ssrc >> 8) & 0xFF) as u8;
    iv[7] ^= (ssrc & 0xFF) as u8;

    // Packet index goes into bytes 8-13 (indexed 0)
    // For typical 48-bit RTP sequence number + roll-over-counter combination
    iv[8] ^= ((packet_index >> 40) & 0xFF) as u8;
    iv[9] ^= ((packet_index >> 32) & 0xFF) as u8;
    iv[10] ^= ((packet_index >> 24) & 0xFF) as u8;
    iv[11] ^= ((packet_index >> 16) & 0xFF) as u8;
    iv[12] ^= ((packet_index >> 8) & 0xFF) as u8;
    iv[13] ^= (packet_index & 0xFF) as u8;

    Ok(iv)
}

/// Key rotation frequency
/// This is used to determine when to rekey based on packet index
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRotationFrequency {
    /// Key is never rotated
    None,

    /// Key is rotated every 2^n packets
    Power2(u8),
}

impl KeyRotationFrequency {
    /// Check if key rotation is needed for the given packet index
    pub fn should_rotate(&self, packet_index: u64) -> bool {
        match self {
            Self::None => false,
            Self::Power2(power) => {
                let mask = (1u64 << power) - 1;
                (packet_index & mask) == 0
            }
        }
    }
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
    fn test_key_rotation_frequency() {
        // Never rotate
        let freq = KeyRotationFrequency::None;
        assert!(!freq.should_rotate(0));
        assert!(!freq.should_rotate(1000));

        // Rotate every 2^8 = 256 packets
        let freq = KeyRotationFrequency::Power2(8);
        assert!(freq.should_rotate(0));
        assert!(!freq.should_rotate(1));
        assert!(!freq.should_rotate(255));
        assert!(freq.should_rotate(256));
        assert!(!freq.should_rotate(257));
        assert!(freq.should_rotate(512));
    }

    #[test]
    fn test_srtp_kdf() {
        // Create a master key
        let master_key = SrtpCryptoKey::new(vec![0; 16], vec![0; 14]);

        // Create key derivation parameters
        let params = SrtpKeyDerivationParams {
            label: KeyDerivationLabel::RtpEncryption,
            key_derivation_rate: 0,
            index: 0,
        };

        // Derive a key
        let result = srtp_kdf(&master_key, &params, 16);
        assert!(result.is_ok());

        let key = result.unwrap();
        assert_eq!(key.len(), 16);

        // Try deriving keys with different labels
        let params2 = SrtpKeyDerivationParams {
            label: KeyDerivationLabel::RtpAuthentication,
            key_derivation_rate: 0,
            index: 0,
        };

        let key2 = srtp_kdf(&master_key, &params2, 16).unwrap();

        // Create a key that's definitely different for comparison
        let different_key = vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ];

        // Assert that the derived key is not equal to our deliberately different key
        assert_ne!(key, different_key);
        assert_ne!(key, key2);
    }

    #[test]
    fn test_rfc3711_appendix_b3_key_derivation_vectors() {
        let master_key = SrtpCryptoKey::new(
            hex_bytes("E1F97A0D3E018BE0D64FA32C06DE4139"),
            hex_bytes("0EC675AD498AFEEBB6960B3AABE6"),
        );

        let enc_key = srtp_kdf(
            &master_key,
            &SrtpKeyDerivationParams {
                label: KeyDerivationLabel::RtpEncryption,
                key_derivation_rate: 0,
                index: 0,
            },
            16,
        )
        .unwrap();
        assert_eq!(enc_key, hex_bytes("C61E7A93744F39EE10734AFE3FF7A087"));

        let salt = srtp_kdf(
            &master_key,
            &SrtpKeyDerivationParams {
                label: KeyDerivationLabel::RtpSalt,
                key_derivation_rate: 0,
                index: 0,
            },
            14,
        )
        .unwrap();
        assert_eq!(salt, hex_bytes("30CBBC08863D8C85D49DB34A9AE1"));

        let auth_key = srtp_kdf(
            &master_key,
            &SrtpKeyDerivationParams {
                label: KeyDerivationLabel::RtpAuthentication,
                key_derivation_rate: 0,
                index: 0,
            },
            20,
        )
        .unwrap();
        assert_eq!(
            auth_key,
            hex_bytes("CEBE321F6FF7716B6FD4AB49AF256A156D38BAA4")
        );
    }

    #[test]
    fn test_create_srtp_iv() {
        // Create a salt
        let salt = vec![0; 14];

        // Create an IV
        let result = create_srtp_iv(&salt, 0x12345678, 1000);
        assert!(result.is_ok());

        let iv = result.unwrap();
        assert_eq!(iv.len(), 16);

        // Verify the IV construction - salt XORed with SSRC and packet index
        assert_eq!(iv[4], 0x12); // SSRC byte 0
        assert_eq!(iv[5], 0x34); // SSRC byte 1
        assert_eq!(iv[6], 0x56); // SSRC byte 2
        assert_eq!(iv[7], 0x78); // SSRC byte 3

        assert_eq!(iv[12], 0x03); // packet index byte 4 (1000 >> 8 = 3)
        assert_eq!(iv[13], 0xE8); // packet index byte 5 (1000 & 0xFF = 232 = 0xE8)

        // Test with invalid salt
        let short_salt = vec![0; 8];
        let result = create_srtp_iv(&short_salt, 0x12345678, 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_kdf_with_different_output_sizes() {
        // Create a master key
        let master_key = SrtpCryptoKey::new(
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            vec![21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34],
        );

        let params = SrtpKeyDerivationParams {
            label: KeyDerivationLabel::RtpEncryption,
            key_derivation_rate: 0,
            index: 0,
        };

        // Test 16-byte key (standard AES-128 key)
        let key16 = srtp_kdf(&master_key, &params, 16).unwrap();
        assert_eq!(key16.len(), 16);

        // Test 20-byte key (for authentication)
        let key20 = srtp_kdf(&master_key, &params, 20).unwrap();
        assert_eq!(key20.len(), 20);

        // Test 32-byte key (for AES-256, though not supported in our implementation)
        let key32 = srtp_kdf(&master_key, &params, 32).unwrap();
        assert_eq!(key32.len(), 32);

        // The first 16 bytes should be the same
        assert_eq!(key16, key20[0..16]);
        assert_eq!(key16, key32[0..16]);
    }

    #[test]
    fn test_kdf_with_aes256_master_key() {
        let master_key = SrtpCryptoKey::new(
            (1..=32).collect(),
            vec![21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34],
        );
        let params = SrtpKeyDerivationParams {
            label: KeyDerivationLabel::RtpEncryption,
            key_derivation_rate: 0,
            index: 0,
        };

        let key32 = srtp_kdf(&master_key, &params, 32).unwrap();
        assert_eq!(key32.len(), 32);

        let auth_key = srtp_kdf(
            &master_key,
            &SrtpKeyDerivationParams {
                label: KeyDerivationLabel::RtpAuthentication,
                key_derivation_rate: 0,
                index: 0,
            },
            20,
        )
        .unwrap();
        assert_eq!(auth_key.len(), 20);
    }
}
