//! DTLS key management
//!
//! This module handles key derivation and management for DTLS.

use bytes::{Bytes, BytesMut, BufMut};
use crate::dtls::Result;
use crate::dtls::crypto::cipher::{HashAlgorithm, MacAlgorithm};

// Add crypto imports
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha384, Digest};
use p256::{PublicKey, SecretKey, ecdh::diffie_hellman};

// Type aliases for HMAC implementations
type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;
type HmacSha384 = Hmac<Sha384>;

/// DTLS keying material derived from the handshake
#[derive(Debug, Clone)]
pub struct DtlsKeyingMaterial {
    /// Master secret
    master_secret: Bytes,
    
    /// Client random
    client_random: Bytes,
    
    /// Server random
    server_random: Bytes,
    
    /// Client write MAC key
    client_write_mac_key: Bytes,
    
    /// Server write MAC key
    server_write_mac_key: Bytes,
    
    /// Client write key
    client_write_key: Bytes,
    
    /// Server write key
    server_write_key: Bytes,
    
    /// Client write IV
    client_write_iv: Bytes,
    
    /// Server write IV
    server_write_iv: Bytes,
}

impl DtlsKeyingMaterial {
    /// Create a new instance with the given secrets
    pub fn new(
        master_secret: Bytes,
        client_random: Bytes,
        server_random: Bytes,
        client_write_mac_key: Bytes,
        server_write_mac_key: Bytes,
        client_write_key: Bytes,
        server_write_key: Bytes,
        client_write_iv: Bytes,
        server_write_iv: Bytes,
    ) -> Self {
        Self {
            master_secret,
            client_random,
            server_random,
            client_write_mac_key,
            server_write_mac_key,
            client_write_key,
            server_write_key,
            client_write_iv,
            server_write_iv,
        }
    }
    
    /// Get the master secret
    pub fn master_secret(&self) -> &Bytes {
        &self.master_secret
    }
    
    /// Get the client random
    pub fn client_random(&self) -> &Bytes {
        &self.client_random
    }
    
    /// Get the server random
    pub fn server_random(&self) -> &Bytes {
        &self.server_random
    }
    
    /// Get the client write MAC key
    pub fn client_write_mac_key(&self) -> &Bytes {
        &self.client_write_mac_key
    }
    
    /// Get the server write MAC key
    pub fn server_write_mac_key(&self) -> &Bytes {
        &self.server_write_mac_key
    }
    
    /// Get the client write key
    pub fn client_write_key(&self) -> &Bytes {
        &self.client_write_key
    }
    
    /// Get the server write key
    pub fn server_write_key(&self) -> &Bytes {
        &self.server_write_key
    }
    
    /// Get the client write IV
    pub fn client_write_iv(&self) -> &Bytes {
        &self.client_write_iv
    }
    
    /// Get the server write IV
    pub fn server_write_iv(&self) -> &Bytes {
        &self.server_write_iv
    }
    
    /// Export keying material according to RFC 5705/RFC 5764
    pub fn export_keying_material(
        &self,
        label: &str,
        context: Option<&[u8]>,
        length: usize,
    ) -> Result<Bytes> {
        let seed = if let Some(ctx) = context {
            // If context is provided, include it in the seed
            let mut seed_data = BytesMut::with_capacity(self.client_random.len() + self.server_random.len() + ctx.len());
            seed_data.extend_from_slice(&self.client_random);
            seed_data.extend_from_slice(&self.server_random);
            seed_data.extend_from_slice(ctx);
            seed_data.freeze()
        } else {
            // Otherwise, just use client_random + server_random
            let mut seed_data = BytesMut::with_capacity(self.client_random.len() + self.server_random.len());
            seed_data.extend_from_slice(&self.client_random);
            seed_data.extend_from_slice(&self.server_random);
            seed_data.freeze()
        };
        
        // Use the PRF to generate the keying material
        prf_tls12(
            &self.master_secret,
            label.as_bytes(),
            &seed,
            length,
            HashAlgorithm::Sha256,
        )
    }
}

/// HMAC implementation using the specified hash algorithm
///
/// This uses the RustCrypto HMAC implementation.
fn hmac(key: &[u8], message: &[u8], hash_algorithm: HashAlgorithm) -> Result<Bytes> {
    match hash_algorithm {
        HashAlgorithm::Sha1 => {
            let mut mac = HmacSha1::new_from_slice(key)
                .map_err(|e| crate::error::Error::CryptoError(format!("HMAC-SHA1 error: {}", e)))?;
            mac.update(message);
            let result = mac.finalize().into_bytes();
            Ok(Bytes::copy_from_slice(&result))
        },
        HashAlgorithm::Sha256 => {
            let mut mac = HmacSha256::new_from_slice(key)
                .map_err(|e| crate::error::Error::CryptoError(format!("HMAC-SHA256 error: {}", e)))?;
            mac.update(message);
            let result = mac.finalize().into_bytes();
            Ok(Bytes::copy_from_slice(&result))
        },
        HashAlgorithm::Sha384 => {
            let mut mac = HmacSha384::new_from_slice(key)
                .map_err(|e| crate::error::Error::CryptoError(format!("HMAC-SHA384 error: {}", e)))?;
            mac.update(message);
            let result = mac.finalize().into_bytes();
            Ok(Bytes::copy_from_slice(&result))
        }
    }
}

/// P_hash function from TLS 1.2 (RFC 5246)
///
/// This is the core of the PRF, using HMAC with a specific hash function.
fn p_hash(secret: &[u8], seed: &[u8], output_len: usize, hash_algorithm: HashAlgorithm) -> Result<Bytes> {
    let mut result = BytesMut::with_capacity(output_len);
    let hash_len = hash_algorithm.hash_size();
    
    // A(0) = seed
    let mut a = Bytes::copy_from_slice(seed);
    
    // Generate enough output
    while result.len() < output_len {
        // A(i) = HMAC_hash(secret, A(i-1))
        a = hmac(secret, &a, hash_algorithm)?;
        
        // P_hash = HMAC_hash(secret, A(1) + seed) + HMAC_hash(secret, A(2) + seed) + ...
        let mut hmac_input = BytesMut::with_capacity(a.len() + seed.len());
        hmac_input.extend_from_slice(&a);
        hmac_input.extend_from_slice(seed);
        
        let hmac_output = hmac(secret, &hmac_input, hash_algorithm)?;
        
        // Add as much of the HMAC output as needed
        let to_copy = std::cmp::min(output_len - result.len(), hash_len);
        result.extend_from_slice(&hmac_output[..to_copy]);
    }
    
    Ok(result.freeze())
}

/// TLS 1.2 PRF (RFC 5246)
pub fn prf_tls12(secret: &[u8], label: &[u8], seed: &[u8], output_len: usize, hash_algorithm: HashAlgorithm) -> Result<Bytes> {
    // Combine label and seed
    let mut combined_seed = BytesMut::with_capacity(label.len() + seed.len());
    combined_seed.extend_from_slice(label);
    combined_seed.extend_from_slice(seed);
    
    // Debug info
    if label == b"master secret" {
        println!("PRF input for master secret:");
        println!("  - Label: {:?}", std::str::from_utf8(label).unwrap_or("invalid utf8"));
        println!("  - Secret length: {}", secret.len());
        println!("  - Secret first bytes: {:02X?}", &secret[..std::cmp::min(secret.len(), 8)]);
        println!("  - Seed length: {}", seed.len());
        println!("  - Seed first bytes: {:02X?}", &seed[..std::cmp::min(seed.len(), 8)]);
        println!("  - Combined seed length: {}", combined_seed.len());
        println!("  - Combined seed first bytes: {:02X?}", &combined_seed[..std::cmp::min(combined_seed.len(), 8)]);
    }
    
    // Use P_hash with the specified hash algorithm
    p_hash(secret, &combined_seed, output_len, hash_algorithm)
}

/// Derive key material from TLS PRF
///
/// This function derives the key material needed for DTLS 1.2 from the master secret.
pub fn derive_key_material(
    master_secret: &[u8],
    client_random: &[u8],
    server_random: &[u8],
    mac_algorithm: MacAlgorithm,
    key_size: usize,
    iv_size: usize,
) -> Result<DtlsKeyingMaterial> {
    // Key material constants
    const CLIENT_MAC_KEY_LABEL: &[u8] = b"client write MAC key";
    const SERVER_MAC_KEY_LABEL: &[u8] = b"server write MAC key";
    const CLIENT_WRITE_KEY_LABEL: &[u8] = b"client write key";
    const SERVER_WRITE_KEY_LABEL: &[u8] = b"server write key";
    const CLIENT_WRITE_IV_LABEL: &[u8] = b"client write IV";
    const SERVER_WRITE_IV_LABEL: &[u8] = b"server write IV";
    
    // Calculate sizes
    let mac_key_size = mac_algorithm.hash_size();
    
    // Calculate total size of all the keys and IVs
    let total_size = 2 * mac_key_size + 2 * key_size + 2 * iv_size;
    
    // Seed is client_random + server_random
    let mut seed = BytesMut::with_capacity(client_random.len() + server_random.len());
    seed.extend_from_slice(client_random);
    seed.extend_from_slice(server_random);
    
    // Use PRF to generate key material
    let key_block = prf_tls12(
        master_secret,
        b"key expansion",
        &seed,
        total_size,
        HashAlgorithm::Sha256,
    )?;
    
    // Extract client and server keys
    let mut offset = 0;
    
    // Client MAC key
    let client_mac_key = Bytes::copy_from_slice(&key_block[offset..offset + mac_key_size]);
    offset += mac_key_size;
    
    // Server MAC key
    let server_mac_key = Bytes::copy_from_slice(&key_block[offset..offset + mac_key_size]);
    offset += mac_key_size;
    
    // Client write key
    let client_write_key = Bytes::copy_from_slice(&key_block[offset..offset + key_size]);
    offset += key_size;
    
    // Server write key
    let server_write_key = Bytes::copy_from_slice(&key_block[offset..offset + key_size]);
    offset += key_size;
    
    // Client write IV
    let client_write_iv = Bytes::copy_from_slice(&key_block[offset..offset + iv_size]);
    offset += iv_size;
    
    // Server write IV
    let server_write_iv = Bytes::copy_from_slice(&key_block[offset..offset + iv_size]);
    
    // Create and return the keying material
    Ok(DtlsKeyingMaterial::new(
        Bytes::copy_from_slice(master_secret),
        Bytes::copy_from_slice(client_random),
        Bytes::copy_from_slice(server_random),
        client_mac_key,
        server_mac_key,
        client_write_key,
        server_write_key,
        client_write_iv,
        server_write_iv,
    ))
}

/// Generate a pre-master secret for ECDHE key exchange
pub fn generate_ecdhe_pre_master_secret(public_key_bytes: &[u8], private_key_bytes: &[u8]) -> Result<Bytes> {
    // Parse the public key
    let public_key = PublicKey::from_sec1_bytes(public_key_bytes)
        .map_err(|e| crate::error::Error::CryptoError(format!("Failed to parse public key: {}", e)))?;
    
    // Parse the private key
    let private_key = SecretKey::from_bytes(private_key_bytes.into())
        .map_err(|e| crate::error::Error::CryptoError(format!("Failed to parse private key: {}", e)))?;
    
    // Perform the ECDH operation
    let shared_secret = diffie_hellman(
        private_key.to_nonzero_scalar(),
        public_key.as_affine(),
    );
    
    // Convert the shared point to bytes
    let shared_secret_bytes = shared_secret.raw_secret_bytes();
    
    // Return the shared secret
    Ok(Bytes::copy_from_slice(shared_secret_bytes.as_slice()))
}

/// Generate an ephemeral ECDH key pair for P-256 curve
pub fn generate_ecdh_keypair() -> Result<(SecretKey, PublicKey)> {
    use rand::rngs::OsRng;
    
    // Generate a new random private key
    let private_key = SecretKey::random(&mut OsRng);
    
    // Derive the public key from the private key
    let public_key = PublicKey::from_secret_scalar(&private_key.to_nonzero_scalar());
    
    Ok((private_key, public_key))
}

/// Convert a P-256 public key to its SEC1 encoded format (for network transmission)
pub fn encode_public_key(public_key: &PublicKey) -> Result<Bytes> {
    let encoded = public_key.to_sec1_bytes();
    Ok(Bytes::copy_from_slice(&encoded))
}

/// Convert a P-256 private key to bytes (for storage)
pub fn encode_private_key(private_key: &SecretKey) -> Result<Bytes> {
    let bytes = private_key.to_bytes();
    Ok(Bytes::copy_from_slice(bytes.as_slice()))
}

/// Calculate master secret from pre-master secret
pub fn calculate_master_secret(
    pre_master_secret: &[u8],
    client_random: &[u8],
    server_random: &[u8],
) -> Result<Bytes> {
    // Master secret is 48 bytes
    const MASTER_SECRET_LENGTH: usize = 48;
    
    // Print first bytes of input for debugging
    println!("Calculate master secret inputs:");
    println!("  - Pre-master secret first bytes: {:02X?}", 
             &pre_master_secret[..std::cmp::min(pre_master_secret.len(), 8)]);
    println!("  - Client random first bytes: {:02X?}", 
             &client_random[..std::cmp::min(client_random.len(), 8)]);
    println!("  - Server random first bytes: {:02X?}", 
             &server_random[..std::cmp::min(server_random.len(), 8)]);
    
    // Seed is client_random + server_random
    let mut seed = BytesMut::with_capacity(client_random.len() + server_random.len());
    seed.extend_from_slice(client_random);
    seed.extend_from_slice(server_random);
    
    println!("  - Combined seed size: {}", seed.len());
    
    // Use PRF to generate master secret
    let master_secret = prf_tls12(
        pre_master_secret,
        b"master secret",
        &seed,
        MASTER_SECRET_LENGTH,
        HashAlgorithm::Sha256,
    )?;
    
    // Print first bytes of master secret for debugging
    println!("  - Generated master secret first bytes: {:02X?}", 
             &master_secret[..std::cmp::min(master_secret.len(), 8)]);
    
    Ok(master_secret)
}

/// Extract keys for SRTP from DTLS keying material (RFC 5764)
pub fn extract_srtp_keys(
    keying_material: &DtlsKeyingMaterial,
    profile_key_length: usize,
    profile_salt_length: usize,
    is_client: bool,
) -> Result<(Bytes, Bytes)> {
    // SRTP master key and salt length
    let key_length = profile_key_length;
    let salt_length = profile_salt_length;
    let total_length = (key_length + salt_length) * 2; // For client and server
    
    // Extract master key and salt using the exporter
    let key_material = keying_material.export_keying_material(
        "EXTRACTOR-dtls_srtp",
        None,
        total_length,
    )?;
    
    // Split the key material into client and server keys and salts
    let mut offset = 0;
    
    // Client master key
    let client_master_key = Bytes::copy_from_slice(&key_material[offset..offset + key_length]);
    offset += key_length;
    
    // Server master key
    let server_master_key = Bytes::copy_from_slice(&key_material[offset..offset + key_length]);
    offset += key_length;
    
    // Client master salt
    let client_master_salt = Bytes::copy_from_slice(&key_material[offset..offset + salt_length]);
    offset += salt_length;
    
    // Server master salt
    let server_master_salt = Bytes::copy_from_slice(&key_material[offset..offset + salt_length]);
    
    // Return the appropriate keys based on whether we're the client or server
    if is_client {
        // client uses the client write key and server read key (client's key)
        let master_key = client_master_key;
        let master_salt = client_master_salt;
        Ok((master_key, master_salt))
    } else {
        // server uses the server write key and client read key (server's key)
        let master_key = server_master_key;
        let master_salt = server_master_salt;
        Ok((master_key, master_salt))
    }
}

/// Calculate verify data for the Finished message
///
/// This implements the TLS 1.2 PRF for generating the verify data
/// in the Finished message as defined in RFC 5246 section 7.4.9.
pub fn calculate_verify_data(
    master_secret: &[u8],
    handshake_messages: &[u8],
    is_client: bool,
    hash_algorithm: HashAlgorithm,
) -> Result<Bytes> {
    // Calculate the hash of all handshake messages
    let handshake_hash = match hash_algorithm {
        HashAlgorithm::Sha1 => {
            let mut hasher = sha1::Sha1::new();
            hasher.update(handshake_messages);
            Bytes::copy_from_slice(&hasher.finalize())
        },
        HashAlgorithm::Sha256 => {
            let mut hasher = sha2::Sha256::new();
            hasher.update(handshake_messages);
            Bytes::copy_from_slice(&hasher.finalize())
        },
        HashAlgorithm::Sha384 => {
            let mut hasher = sha2::Sha384::new();
            hasher.update(handshake_messages);
            Bytes::copy_from_slice(&hasher.finalize())
        }
    };
    
    // Debugging: print hash of handshake messages for verification checks
    println!("Handshake hash for {} verification: {:02X?}",  
             if is_client { "client" } else { "server" },
             &handshake_hash[..std::cmp::min(handshake_hash.len(), 16)]);
    
    // Choose the appropriate label
    let label = if is_client {
        b"client finished"
    } else {
        b"server finished"
    };
    
    // Use PRF to generate verify data (12 bytes as per RFC 5246)
    let verify_data = prf_tls12(
        master_secret,
        label,
        &handshake_hash,
        12, // Fixed size for TLS 1.2
        hash_algorithm,
    )?;
    
    // Print debug info for PRF inputs
    println!("PRF inputs for verification:");
    println!("  - Label: {:?}", std::str::from_utf8(label).unwrap_or("invalid utf8"));
    println!("  - Master secret length: {}", master_secret.len());
    println!("  - Master secret first bytes: {:02X?}", &master_secret[..std::cmp::min(master_secret.len(), 8)]);
    println!("  - Hash algorithm: {:?}", hash_algorithm);
    
    Ok(verify_data)
} 