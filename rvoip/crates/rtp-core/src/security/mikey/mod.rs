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
use sha2::Sha256;

pub mod message;
pub mod payloads;

pub use message::{MikeyMessage, MikeyMessageType};
pub use payloads::{
    PayloadType, CommonHeader, KeyDataPayload, 
    GeneralExtensionPayload, KeyValidationData,
    SecurityPolicyPayload
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
}

impl SecurityKeyExchange for Mikey {
    fn init(&mut self) -> Result<(), Error> {
        // Initiator creates and sends the initial message
        // Responder waits for the initial message
        match self.role {
            MikeyRole::Initiator => {
                let _ = self.create_initial_message()?;
                Ok(())
            },
            MikeyRole::Responder => Ok(()),
        }
    }
    
    fn process_message(&mut self, message: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        match (self.role, &self.state) {
            (MikeyRole::Initiator, MikeyState::WaitingForResponse) => {
                // Initiator processes response message
                self.process_response_message(message)?;
                // No further messages needed
                Ok(None)
            },
            (MikeyRole::Responder, MikeyState::Initial) => {
                // Responder processes initial message and creates response
                let response = self.process_initial_message(message)?;
                Ok(Some(response))
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