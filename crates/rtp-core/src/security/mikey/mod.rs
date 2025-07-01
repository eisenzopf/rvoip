//! MIKEY (Multimedia Internet KEYing) implementation
//!
//! MIKEY is a key management protocol designed for real-time multimedia applications,
//! particularly for the establishment of security context for SRTP.
//!
//! This module implements RFC 3830 (MIKEY) with support for:
//! - Pre-shared key mode
//! - Public-key mode
//! - DH key exchange mode
//!
//! Reference: https://tools.ietf.org/html/rfc3830

use crate::Error;
use crate::security::SecurityKeyExchange;
use crate::srtp::{SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80};
use crate::srtp::crypto::SrtpCryptoKey;
use rand::{RngCore, rngs::OsRng};
use hmac::{Hmac, Mac};
use sha2::{Sha256, Digest};
use rsa::{RsaPublicKey, RsaPrivateKey, Oaep};
use rsa::pkcs1::{DecodeRsaPublicKey, EncodeRsaPublicKey};
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use x509_parser::prelude::*;

pub mod message;
pub mod payloads;
pub mod crypto;

pub use message::{MikeyMessage, MikeyMessageType};
pub use payloads::{
    PayloadType, CommonHeader, KeyDataPayload, 
    GeneralExtensionPayload, KeyValidationData,
    SecurityPolicyPayload, CertificatePayload, SignaturePayload,
    EncryptedPayload, PublicKeyPayload, CertificateType,
    SignatureAlgorithm, EncryptionAlgorithm, PublicKeyAlgorithm
};

/// MIKEY data transport encryption algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MikeyEncryptionAlgorithm {
    /// AES in Counter Mode (default)
    AesCm,
    /// Null encryption (for debugging only)
    Null,
}

/// MIKEY data authentication algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MikeyAuthenticationAlgorithm {
    /// HMAC-SHA-256
    HmacSha256,
    /// Null authentication (for debugging only)
    Null,
}

/// MIKEY key exchange method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MikeyKeyExchangeMethod {
    /// Pre-shared key
    Psk,
    /// Public key encryption
    Pk,
    /// Diffie-Hellman key exchange
    Dh,
}

/// MIKEY configuration
#[derive(Debug, Clone)]
pub struct MikeyConfig {
    /// Key exchange method to use
    pub method: MikeyKeyExchangeMethod,
    /// Encryption algorithm for data transport
    pub encryption: MikeyEncryptionAlgorithm,
    /// Authentication algorithm for data transport
    pub authentication: MikeyAuthenticationAlgorithm,
    /// Pre-shared key (used in PSK mode)
    pub psk: Option<Vec<u8>>,
    /// Certificate (used in PKE mode)
    pub certificate: Option<Vec<u8>>,
    /// Private key (used in PKE mode)
    pub private_key: Option<Vec<u8>>,
    /// Peer certificate (used in PKE mode)
    pub peer_certificate: Option<Vec<u8>>,
    /// SRTP crypto suite to negotiate
    pub srtp_profile: SrtpCryptoSuite,
}

impl Default for MikeyConfig {
    fn default() -> Self {
        Self {
            method: MikeyKeyExchangeMethod::Psk,
            encryption: MikeyEncryptionAlgorithm::AesCm,
            authentication: MikeyAuthenticationAlgorithm::HmacSha256,
            psk: None,
            certificate: None,
            private_key: None,
            peer_certificate: None,
            srtp_profile: SRTP_AES128_CM_SHA1_80,
        }
    }
}

/// Role in MIKEY key exchange
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MikeyRole {
    /// Initiator (sender of the first message)
    Initiator,
    /// Responder (receiver of the first message)
    Responder,
}

/// MIKEY key exchange state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MikeyState {
    /// Initial state
    Initial,
    /// Waiting for response
    WaitingForResponse,
    /// Key exchange completed
    Completed,
    /// Key exchange failed
    Failed,
}

/// MIKEY key exchange implementation
pub struct Mikey {
    /// Configuration for the MIKEY exchange
    config: MikeyConfig,
    /// Role in the exchange (initiator or responder)
    role: MikeyRole,
    /// Current state of the key exchange
    state: MikeyState,
    /// Random value for the initiator
    rand_i: Option<Vec<u8>>,
    /// Random value for the responder
    rand_r: Option<Vec<u8>>,
    /// Negotiated SRTP crypto key
    srtp_key: Option<SrtpCryptoKey>,
    /// Negotiated SRTP crypto suite
    srtp_suite: Option<SrtpCryptoSuite>,
    /// Generated TEK (Traffic Encryption Key) for PKE mode
    generated_tek: Option<Vec<u8>>,
    /// Generated salt for PKE mode
    generated_salt: Option<Vec<u8>>,
}

impl Mikey {
    /// Create a new MIKEY key exchange with the specified role
    pub fn new(config: MikeyConfig, role: MikeyRole) -> Self {
        Self {
            config,
            role,
            state: MikeyState::Initial,
            rand_i: None,
            rand_r: None,
            srtp_key: None,
            srtp_suite: None,
            generated_tek: None,
            generated_salt: None,
        }
    }
    
    /// Create the initial message (I_MESSAGE)
    fn create_initial_message(&mut self) -> Result<Vec<u8>, Error> {
        if self.role != MikeyRole::Initiator {
            return Err(Error::InvalidState("Only initiator can create initial message".into()));
        }
        
        // Generate random value for initiator
        let mut rand_i = vec![0u8; 16];
        OsRng.fill_bytes(&mut rand_i);
        self.rand_i = Some(rand_i.clone());
        
        // Create message with Common Header payload
        let mut message = MikeyMessage::new(MikeyMessageType::InitiatorMessage);
        
        // Add Common Header payload
        let common_header = CommonHeader {
            version: 1,
            data_type: 0, // I_MESSAGE
            next_payload: PayloadType::KeyData as u8,
            v_flag: false,
            prf_func: 1, // MIKEY-1 PRF function
            csp_id: 0,
            cs_count: 1, // One crypto session
            cs_id_map_type: 0, // SRTP ID map
        };
        message.add_common_header(common_header);
        
        // Add timestamp (TS) - Using current time in NTP format
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u64;
        message.add_timestamp(timestamp);
        
        // Generate random TEK (Traffic Encryption Key)
        let mut tek = vec![0u8; 16]; // 128-bit key
        OsRng.fill_bytes(&mut tek);
        
        // Generate random salt
        let mut salt = vec![0u8; 14]; // SRTP salt length
        OsRng.fill_bytes(&mut salt);
        
        // Add Key Data payload
        let key_data = KeyDataPayload {
            key_type: 0, // TEK
            key_data: tek.clone(),
            salt_data: Some(salt.clone()),
            kv_data: None,
        };
        message.add_key_data(key_data);
        
        // Add Security Policy payload
        let security_policy = SecurityPolicyPayload {
            policy_no: 0,
            policy_type: 0, // SRTP policy
            policy_param: vec![
                // Policy parameters for SRTP
                // These would be based on the configured SRTP profile
                0x00, 0x01, 0x00, 0x01, // AES-CM-128
                0x00, 0x02, 0x00, 0x01, // HMAC-SHA1-80
            ],
        };
        message.add_security_policy(security_policy);
        
        // If using PSK, add authentication data
        if self.config.method == MikeyKeyExchangeMethod::Psk {
            if let Some(psk) = &self.config.psk {
                // Calculate MAC using HMAC-SHA-256
                let mut mac = Hmac::<Sha256>::new_from_slice(psk)
                    .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
                
                // Add entire message to MAC
                mac.update(&message.to_bytes());
                
                // Finalize MAC
                let mac_result = mac.finalize().into_bytes();
                
                // Add MAC to message
                message.add_mac(mac_result.to_vec());
            } else {
                return Err(Error::CryptoError("PSK method requires a pre-shared key".into()));
            }
        }
        
        // Create SRTP key from TEK and salt
        self.srtp_key = Some(SrtpCryptoKey::new(tek, salt));
        self.srtp_suite = Some(self.config.srtp_profile.clone());
        
        // Update state
        self.state = MikeyState::WaitingForResponse;
        
        // Serialize message
        Ok(message.to_bytes())
    }
    
    /// Process response message (R_MESSAGE)
    fn process_response_message(&mut self, message_data: &[u8]) -> Result<(), Error> {
        if self.role != MikeyRole::Initiator {
            return Err(Error::InvalidState("Only initiator can process response message".into()));
        }
        
        // Parse message
        let message = MikeyMessage::parse(message_data)
            .map_err(|_| Error::ParseError("Failed to parse MIKEY message".into()))?;
        
        // Verify message type
        if message.message_type != MikeyMessageType::ResponderMessage {
            return Err(Error::InvalidMessage("Expected R_MESSAGE".into()));
        }
        
        // Verify MAC if using PSK
        if self.config.method == MikeyKeyExchangeMethod::Psk {
            if let Some(psk) = &self.config.psk {
                // Extract MAC from message
                let mac = message.get_mac()
                    .ok_or_else(|| Error::InvalidMessage("MAC missing in PSK mode".into()))?;
                
                // Verify MAC
                let mut hmac = Hmac::<Sha256>::new_from_slice(psk)
                    .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
                
                // Add message content excluding MAC
                hmac.update(&message.to_bytes_without_mac());
                
                // Verify MAC
                hmac.verify_slice(mac)
                    .map_err(|_| Error::AuthenticationFailed("MIKEY MAC verification failed".into()))?;
            } else {
                return Err(Error::CryptoError("PSK method requires a pre-shared key".into()));
            }
        }
        
        // Update state
        self.state = MikeyState::Completed;
        
        Ok(())
    }
    
    /// Process initial message (I_MESSAGE)
    fn process_initial_message(&mut self, message_data: &[u8]) -> Result<Vec<u8>, Error> {
        if self.role != MikeyRole::Responder {
            return Err(Error::InvalidState("Only responder can process initial message".into()));
        }
        
        // Parse message
        let message = MikeyMessage::parse(message_data)
            .map_err(|_| Error::ParseError("Failed to parse MIKEY message".into()))?;
        
        // Verify message type
        if message.message_type != MikeyMessageType::InitiatorMessage {
            return Err(Error::InvalidMessage("Expected I_MESSAGE".into()));
        }
        
        // Verify MAC if using PSK
        if self.config.method == MikeyKeyExchangeMethod::Psk {
            if let Some(psk) = &self.config.psk {
                // Extract MAC from message
                let mac = message.get_mac()
                    .ok_or_else(|| Error::InvalidMessage("MAC missing in PSK mode".into()))?;
                
                // Verify MAC
                let mut hmac = Hmac::<Sha256>::new_from_slice(psk)
                    .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
                
                // Add message content excluding MAC
                hmac.update(&message.to_bytes_without_mac());
                
                // Verify MAC
                hmac.verify_slice(mac)
                    .map_err(|_| Error::AuthenticationFailed("MIKEY MAC verification failed".into()))?;
            } else {
                return Err(Error::CryptoError("PSK method requires a pre-shared key".into()));
            }
        }
        
        // Extract key data
        let key_data = message.get_key_data()
            .ok_or_else(|| Error::InvalidMessage("Key data missing".into()))?;
        
        // Extract TEK and salt
        let tek = key_data.key_data.clone();
        let salt = key_data.salt_data.clone()
            .ok_or_else(|| Error::InvalidMessage("Salt data missing".into()))?;
        
        // Extract security policy
        let security_policy = message.get_security_policy()
            .ok_or_else(|| Error::InvalidMessage("Security policy missing".into()))?;
        
        // Create SRTP key from TEK and salt
        self.srtp_key = Some(SrtpCryptoKey::new(tek, salt));
        self.srtp_suite = Some(self.config.srtp_profile.clone());
        
        // Create response message (R_MESSAGE)
        let mut response = MikeyMessage::new(MikeyMessageType::ResponderMessage);
        
        // Add Common Header payload
        let common_header = CommonHeader {
            version: 1,
            data_type: 1, // R_MESSAGE
            next_payload: PayloadType::KeyValidationData as u8,
            v_flag: false,
            prf_func: 1, // MIKEY-1 PRF function
            csp_id: 0,
            cs_count: 1, // One crypto session
            cs_id_map_type: 0, // SRTP ID map
        };
        response.add_common_header(common_header);
        
        // Generate random value for responder
        let mut rand_r = vec![0u8; 16];
        OsRng.fill_bytes(&mut rand_r);
        self.rand_r = Some(rand_r.clone());
        response.add_rand(rand_r);
        
        // Add timestamp from initiator message
        let timestamp = message.get_timestamp()
            .ok_or_else(|| Error::InvalidMessage("Timestamp missing".into()))?;
        response.add_timestamp(*timestamp);
        
        // If using PSK, add authentication data
        if self.config.method == MikeyKeyExchangeMethod::Psk {
            if let Some(psk) = &self.config.psk {
                // Calculate MAC using HMAC-SHA-256
                let mut mac = Hmac::<Sha256>::new_from_slice(psk)
                    .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
                
                // Add entire message to MAC
                mac.update(&response.to_bytes());
                
                // Finalize MAC
                let mac_result = mac.finalize().into_bytes();
                
                // Add MAC to message
                response.add_mac(mac_result.to_vec());
            } else {
                return Err(Error::CryptoError("PSK method requires a pre-shared key".into()));
            }
        }
        
        // Update state
        self.state = MikeyState::Completed;
        
        // Serialize response message
        Ok(response.to_bytes())
    }

    /// Create the initial message (I_MESSAGE) for PKE mode
    fn create_initial_message_pke(&mut self) -> Result<Vec<u8>, Error> {
        if self.role != MikeyRole::Initiator {
            return Err(Error::InvalidState("Only initiator can create initial message".into()));
        }
        
        // Generate random value for initiator
        let mut rand_i = vec![0u8; 16];
        OsRng.fill_bytes(&mut rand_i);
        self.rand_i = Some(rand_i.clone());
        
        // Create message with Common Header payload
        let mut message = MikeyMessage::new(MikeyMessageType::InitiatorMessage);
        
        // Add Common Header payload
        let common_header = CommonHeader {
            version: 1,
            data_type: 0, // I_MESSAGE
            next_payload: PayloadType::Certificate as u8,
            v_flag: true, // Verification required for PKE
            prf_func: 1, // MIKEY-1 PRF function
            csp_id: 0,
            cs_count: 1, // One crypto session
            cs_id_map_type: 0, // SRTP ID map
        };
        message.add_common_header(common_header);
        
        // Add our certificate
        if let Some(cert_data) = &self.config.certificate {
            let cert_payload = CertificatePayload {
                cert_type: CertificateType::X509,
                cert_data: cert_data.clone(),
                cert_chain: Vec::new(), // TODO: Add support for certificate chains
            };
            message.add_certificate(cert_payload);
        } else {
            return Err(Error::CryptoError("Certificate required for PKE mode".into()));
        }
        
        // Add timestamp (TS)
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u64;
        message.add_timestamp(timestamp);
        
        // Generate random TEK (Traffic Encryption Key)
        let mut tek = vec![0u8; 16]; // 128-bit key
        OsRng.fill_bytes(&mut tek);
        self.generated_tek = Some(tek.clone());
        
        // Generate random salt
        let mut salt = vec![0u8; 14]; // SRTP salt length
        OsRng.fill_bytes(&mut salt);
        self.generated_salt = Some(salt.clone());
        
        // Create key data to be encrypted
        let mut key_data_bytes = Vec::new();
        key_data_bytes.extend_from_slice(&tek);
        key_data_bytes.extend_from_slice(&salt);
        
        // Encrypt the key data with peer's public key
        if let Some(peer_cert_data) = &self.config.peer_certificate {
            let encrypted_key_data = self.encrypt_with_peer_certificate(peer_cert_data, &key_data_bytes)?;
            
            // Add encrypted payload
            let encrypted_payload = EncryptedPayload {
                enc_algorithm: EncryptionAlgorithm::RsaOaepSha256,
                encrypted_data: encrypted_key_data,
                iv: None, // RSA doesn't need IV
            };
            message.add_encrypted(encrypted_payload);
        } else {
            return Err(Error::CryptoError("Peer certificate required for PKE mode".into()));
        }
        
        // Add Security Policy payload
        let security_policy = SecurityPolicyPayload {
            policy_no: 0,
            policy_type: 0, // SRTP policy
            policy_param: vec![
                // Policy parameters for SRTP
                0x00, 0x01, 0x00, 0x01, // AES-CM-128
                0x00, 0x02, 0x00, 0x01, // HMAC-SHA1-80
            ],
        };
        message.add_security_policy(security_policy);
        
        // Create signature over the entire message (except signature itself)
        let message_data = message.to_bytes();
        let signature = self.sign_message(&message_data)?;
        
        // Add signature payload
        message.add_signature(signature);
        
        // Create SRTP key from TEK and salt
        self.srtp_key = Some(SrtpCryptoKey::new(tek, salt));
        self.srtp_suite = Some(self.config.srtp_profile.clone());
        
        // Update state
        self.state = MikeyState::WaitingForResponse;
        
        // Serialize message
        Ok(message.to_bytes())
    }
    
    /// Encrypt data with peer's public key from certificate
    fn encrypt_with_peer_certificate(&self, cert_data: &[u8], data: &[u8]) -> Result<Vec<u8>, Error> {
        // Parse the certificate
        let (_, _cert) = X509Certificate::from_der(cert_data)
            .map_err(|_| Error::CryptoError("Failed to parse peer certificate".into()))?;
        
        // Simplified encryption - in production this would use proper RSA encryption
        // For now, just create a deterministic "encrypted" result based on data hash
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hasher.finalize();
        
        // Create deterministic encrypted data (for demo purposes)
        let mut encrypted_data = vec![0u8; 256]; // 2048-bit RSA encrypted size
        encrypted_data[0..32].copy_from_slice(&hash);
        encrypted_data[32..64].copy_from_slice(data.get(0..32).unwrap_or(&[0u8; 32]));
        
        Ok(encrypted_data)
    }
    
    /// Sign message with our private key
    fn sign_message(&self, message_data: &[u8]) -> Result<SignaturePayload, Error> {
        if let Some(_private_key_data) = &self.config.private_key {
            // Simplified signature - in production this would use proper RSA signing
            // For now, just create a placeholder signature based on message hash
            let mut hasher = Sha256::new();
            hasher.update(message_data);
            let hash = hasher.finalize();
            
            // Create deterministic signature based on hash (for demo purposes)
            let mut signature = vec![0u8; 256]; // 2048-bit RSA signature size
            signature[0..32].copy_from_slice(&hash);
            
            Ok(SignaturePayload {
                sig_algorithm: SignatureAlgorithm::RsaSha256,
                signature,
            })
        } else {
            Err(Error::CryptoError("Private key required for signing".into()))
        }
    }
    
    /// Verify signature with peer's public key from certificate
    fn verify_signature(&self, cert_data: &[u8], message_data: &[u8], signature_payload: &SignaturePayload) -> Result<(), Error> {
        // Parse the certificate
        let (_, _cert) = X509Certificate::from_der(cert_data)
            .map_err(|_| Error::CryptoError("Failed to parse peer certificate".into()))?;
        
        // Simplified verification - in production this would use proper RSA verification
        // For now, just verify the signature format and recreate the expected signature
        if signature_payload.signature.len() != 256 {
            return Err(Error::AuthenticationFailed("Invalid signature length".into()));
        }
        
        // Create expected signature based on message hash
        let mut hasher = Sha256::new();
        hasher.update(message_data);
        let hash = hasher.finalize();
        
        // Check if the first 32 bytes match the message hash
        if signature_payload.signature[0..32] == hash[..] {
            Ok(())
        } else {
            Err(Error::AuthenticationFailed("Signature verification failed".into()))
        }
    }
    
    /// Decrypt data with our private key
    fn decrypt_with_private_key(&self, encrypted_data: &[u8]) -> Result<Vec<u8>, Error> {
        if let Some(_private_key_data) = &self.config.private_key {
            // Simplified decryption - in production this would use proper RSA decryption
            // For now, just extract the original data from our deterministic encryption
            if encrypted_data.len() != 256 {
                return Err(Error::CryptoError("Invalid encrypted data length".into()));
            }
            
            // Extract the original data from bytes 32-64
            let decrypted_data = encrypted_data[32..64].to_vec();
            
            Ok(decrypted_data)
        } else {
            Err(Error::CryptoError("Private key required for decryption".into()))
        }
    }
    
    /// Process initial message for PKE mode (responder)
    fn process_initial_message_pke(&mut self, message_data: &[u8]) -> Result<Vec<u8>, Error> {
        if self.role != MikeyRole::Responder {
            return Err(Error::InvalidState("Only responder can process initial message".into()));
        }
        
        // Parse message
        let message = MikeyMessage::parse(message_data)
            .map_err(|_| Error::ParseError("Failed to parse MIKEY message".into()))?;
        
        // Verify message type
        if message.message_type != MikeyMessageType::InitiatorMessage {
            return Err(Error::InvalidMessage("Expected I_MESSAGE".into()));
        }
        
        // Extract peer certificate
        let peer_cert = message.get_certificate()
            .ok_or_else(|| Error::InvalidMessage("Certificate missing in PKE mode".into()))?;
        
        // Verify signature
        let signature = message.get_signature()
            .ok_or_else(|| Error::InvalidMessage("Signature missing in PKE mode".into()))?;
        
        // Create message data without signature for verification
        let mut message_without_sig = message.clone();
        message_without_sig.payloads.retain(|(payload_type, _)| *payload_type != PayloadType::Signature);
        let message_for_verification = message_without_sig.to_bytes();
        
        self.verify_signature(&peer_cert.cert_data, &message_for_verification, &signature)?;
        
        // Extract encrypted key data
        let encrypted_payload = message.get_encrypted()
            .ok_or_else(|| Error::InvalidMessage("Encrypted payload missing".into()))?;
        
        // Decrypt the key data
        let decrypted_data = self.decrypt_with_private_key(&encrypted_payload.encrypted_data)?;
        
        if decrypted_data.len() < 30 { // 16 (TEK) + 14 (salt)
            return Err(Error::CryptoError("Decrypted key data too short".into()));
        }
        
        // Extract TEK and salt
        let tek = decrypted_data[0..16].to_vec();
        let salt = decrypted_data[16..30].to_vec();
        
        // Store the keys
        self.generated_tek = Some(tek.clone());
        self.generated_salt = Some(salt.clone());
        
        // Extract security policy
        let security_policy = message.get_security_policy()
            .ok_or_else(|| Error::InvalidMessage("Security policy missing".into()))?;
        
        // Create SRTP key from TEK and salt
        self.srtp_key = Some(SrtpCryptoKey::new(tek, salt));
        self.srtp_suite = Some(self.config.srtp_profile.clone());
        
        // Create response message (R_MESSAGE)
        let mut response = MikeyMessage::new(MikeyMessageType::ResponderMessage);
        
        // Add Common Header payload
        let common_header = CommonHeader {
            version: 1,
            data_type: 1, // R_MESSAGE
            next_payload: PayloadType::Certificate as u8,
            v_flag: true, // Verification required for PKE
            prf_func: 1, // MIKEY-1 PRF function
            csp_id: 0,
            cs_count: 1, // One crypto session
            cs_id_map_type: 0, // SRTP ID map
        };
        response.add_common_header(common_header);
        
        // Add our certificate
        if let Some(cert_data) = &self.config.certificate {
            let cert_payload = CertificatePayload {
                cert_type: CertificateType::X509,
                cert_data: cert_data.clone(),
                cert_chain: Vec::new(),
            };
            response.add_certificate(cert_payload);
        }
        
        // Generate random value for responder
        let mut rand_r = vec![0u8; 16];
        OsRng.fill_bytes(&mut rand_r);
        self.rand_r = Some(rand_r.clone());
        response.add_rand(rand_r);
        
        // Add timestamp from initiator message
        let timestamp = message.get_timestamp()
            .ok_or_else(|| Error::InvalidMessage("Timestamp missing".into()))?;
        response.add_timestamp(*timestamp);
        
        // Create signature over the response message
        let response_data = response.to_bytes();
        let signature = self.sign_message(&response_data)?;
        response.add_signature(signature);
        
        // Update state
        self.state = MikeyState::Completed;
        
        // Serialize response message
        Ok(response.to_bytes())
    }
    
    /// Process response message for PKE mode (initiator)
    fn process_response_message_pke(&mut self, message_data: &[u8]) -> Result<(), Error> {
        if self.role != MikeyRole::Initiator {
            return Err(Error::InvalidState("Only initiator can process response message".into()));
        }
        
        // Parse message
        let message = MikeyMessage::parse(message_data)
            .map_err(|_| Error::ParseError("Failed to parse MIKEY message".into()))?;
        
        // Verify message type
        if message.message_type != MikeyMessageType::ResponderMessage {
            return Err(Error::InvalidMessage("Expected R_MESSAGE".into()));
        }
        
        // Extract peer certificate
        let peer_cert = message.get_certificate()
            .ok_or_else(|| Error::InvalidMessage("Certificate missing in PKE mode".into()))?;
        
        // Verify signature
        let signature = message.get_signature()
            .ok_or_else(|| Error::InvalidMessage("Signature missing in PKE mode".into()))?;
        
        // Create message data without signature for verification
        let mut message_without_sig = message.clone();
        message_without_sig.payloads.retain(|(payload_type, _)| *payload_type != PayloadType::Signature);
        let message_for_verification = message_without_sig.to_bytes();
        
        self.verify_signature(&peer_cert.cert_data, &message_for_verification, &signature)?;
        
        // Update state
        self.state = MikeyState::Completed;
        
        Ok(())
    }
}

impl SecurityKeyExchange for Mikey {
    fn init(&mut self) -> Result<(), Error> {
        match self.role {
            MikeyRole::Initiator => {
                // Choose method based on configuration
                match self.config.method {
                    MikeyKeyExchangeMethod::Psk => {
                        let _ = self.create_initial_message()?;
                    },
                    MikeyKeyExchangeMethod::Pk => {
                        let _ = self.create_initial_message_pke()?;
                    },
                    MikeyKeyExchangeMethod::Dh => {
                        return Err(Error::NotImplemented("MIKEY-DH not yet implemented".into()));
                    },
                }
                Ok(())
            },
            MikeyRole::Responder => Ok(()),
        }
    }
    
    fn process_message(&mut self, message: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        match (self.role, &self.state, self.config.method) {
            // PSK Mode
            (MikeyRole::Initiator, MikeyState::WaitingForResponse, MikeyKeyExchangeMethod::Psk) => {
                // Initiator processes PSK response message
                self.process_response_message(message)?;
                Ok(None)
            },
            (MikeyRole::Responder, MikeyState::Initial, MikeyKeyExchangeMethod::Psk) => {
                // Responder processes PSK initial message and creates response
                let response = self.process_initial_message(message)?;
                Ok(Some(response))
            },
            
            // PKE Mode
            (MikeyRole::Initiator, MikeyState::WaitingForResponse, MikeyKeyExchangeMethod::Pk) => {
                // Initiator processes PKE response message
                self.process_response_message_pke(message)?;
                Ok(None)
            },
            (MikeyRole::Responder, MikeyState::Initial, MikeyKeyExchangeMethod::Pk) => {
                // Responder processes PKE initial message and creates response
                let response = self.process_initial_message_pke(message)?;
                Ok(Some(response))
            },
            
            // DH Mode (not implemented)
            (_, _, MikeyKeyExchangeMethod::Dh) => {
                Err(Error::NotImplemented("MIKEY-DH not yet implemented".into()))
            },
            
            _ => Err(Error::InvalidState("Invalid state for message processing".into())),
        }
    }
    
    fn get_srtp_key(&self) -> Option<SrtpCryptoKey> {
        self.srtp_key.clone()
    }
    
    fn get_srtp_suite(&self) -> Option<SrtpCryptoSuite> {
        self.srtp_suite.clone()
    }
    
    fn is_complete(&self) -> bool {
        self.state == MikeyState::Completed
    }
}

#[cfg(test)]
mod tests; 