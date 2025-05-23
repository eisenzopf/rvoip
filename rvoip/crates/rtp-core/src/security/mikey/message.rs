//! MIKEY message implementation
//! 
//! This module implements the MIKEY message format as defined in RFC 3830.
//! MIKEY messages consist of a common header followed by a sequence of payloads.

use crate::Error;
use super::payloads::{
    PayloadType, CommonHeader, KeyDataPayload, 
    GeneralExtensionPayload, KeyValidationData,
    SecurityPolicyPayload, CertificatePayload, SignaturePayload,
    EncryptedPayload, PublicKeyPayload
};

/// MIKEY message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MikeyMessageType {
    /// Initiator's message (I_MESSAGE)
    InitiatorMessage,
    /// Responder's message (R_MESSAGE)
    ResponderMessage,
    /// Error message (E_MESSAGE)
    ErrorMessage,
}

/// MIKEY message
#[derive(Debug, Clone)]
pub struct MikeyMessage {
    /// Message type
    pub message_type: MikeyMessageType,
    /// Common header
    pub common_header: Option<CommonHeader>,
    /// Timestamp
    pub timestamp: Option<u64>,
    /// Raw payloads
    pub payloads: Vec<(PayloadType, Vec<u8>)>,
}

impl MikeyMessage {
    /// Create a new MIKEY message
    pub fn new(message_type: MikeyMessageType) -> Self {
        Self {
            message_type,
            common_header: None,
            timestamp: None,
            payloads: Vec::new(),
        }
    }
    
    /// Add common header to the message
    pub fn add_common_header(&mut self, header: CommonHeader) {
        self.common_header = Some(header);
    }
    
    /// Add timestamp to the message
    pub fn add_timestamp(&mut self, timestamp: u64) {
        self.timestamp = Some(timestamp);
        self.payloads.push((PayloadType::Timestamp, timestamp.to_be_bytes().to_vec()));
    }
    
    /// Add random value to the message
    pub fn add_rand(&mut self, rand: Vec<u8>) {
        self.payloads.push((PayloadType::Rand, rand));
    }
    
    /// Add key data to the message
    pub fn add_key_data(&mut self, key_data: KeyDataPayload) {
        // Serialize key data
        let mut data = Vec::new();
        
        // Key type (1 byte)
        data.push(key_data.key_type);
        
        // Key length (2 bytes)
        let key_len = key_data.key_data.len() as u16;
        data.extend_from_slice(&key_len.to_be_bytes());
        
        // Key data
        data.extend_from_slice(&key_data.key_data);
        
        // Add salt if present
        if let Some(salt) = &key_data.salt_data {
            // Salt length (2 bytes)
            let salt_len = salt.len() as u16;
            data.extend_from_slice(&salt_len.to_be_bytes());
            
            // Salt data
            data.extend_from_slice(salt);
        }
        
        // Add key validation data if present
        if let Some(kv_data) = &key_data.kv_data {
            data.extend_from_slice(kv_data);
        }
        
        self.payloads.push((PayloadType::KeyData, data));
    }
    
    /// Add security policy to the message
    pub fn add_security_policy(&mut self, policy: SecurityPolicyPayload) {
        let mut data = Vec::new();
        
        // Policy number (1 byte)
        data.push(policy.policy_no);
        
        // Policy type (1 byte) 
        data.push(policy.policy_type);
        
        // Policy parameters
        data.extend_from_slice(&policy.policy_param);
        
        self.payloads.push((PayloadType::SecurityPolicy, data));
    }
    
    /// Add MAC to the message
    pub fn add_mac(&mut self, mac: Vec<u8>) {
        self.payloads.push((PayloadType::Mac, mac));
    }

    /// Add certificate to the message (PKE mode)
    pub fn add_certificate(&mut self, cert: CertificatePayload) {
        let mut data = Vec::new();
        
        // Certificate type (1 byte)
        data.push(cert.cert_type as u8);
        
        // Certificate length (2 bytes)
        let cert_len = cert.cert_data.len() as u16;
        data.extend_from_slice(&cert_len.to_be_bytes());
        
        // Certificate data
        data.extend_from_slice(&cert.cert_data);
        
        // Add certificate chain if present
        for chain_cert in &cert.cert_chain {
            // Chain certificate length (2 bytes)
            let chain_len = chain_cert.len() as u16;
            data.extend_from_slice(&chain_len.to_be_bytes());
            
            // Chain certificate data
            data.extend_from_slice(chain_cert);
        }
        
        self.payloads.push((PayloadType::Certificate, data));
    }
    
    /// Add signature to the message (PKE mode)
    pub fn add_signature(&mut self, sig: SignaturePayload) {
        let mut data = Vec::new();
        
        // Signature algorithm (1 byte)
        data.push(sig.sig_algorithm as u8);
        
        // Signature length (2 bytes)
        let sig_len = sig.signature.len() as u16;
        data.extend_from_slice(&sig_len.to_be_bytes());
        
        // Signature data
        data.extend_from_slice(&sig.signature);
        
        self.payloads.push((PayloadType::Signature, data));
    }
    
    /// Add encrypted payload to the message (PKE mode)
    pub fn add_encrypted(&mut self, enc: EncryptedPayload) {
        let mut data = Vec::new();
        
        // Encryption algorithm (1 byte)
        data.push(enc.enc_algorithm as u8);
        
        // IV length and data (if present)
        if let Some(iv) = &enc.iv {
            data.push(iv.len() as u8);
            data.extend_from_slice(iv);
        } else {
            data.push(0); // No IV
        }
        
        // Encrypted data length (2 bytes)
        let enc_len = enc.encrypted_data.len() as u16;
        data.extend_from_slice(&enc_len.to_be_bytes());
        
        // Encrypted data
        data.extend_from_slice(&enc.encrypted_data);
        
        self.payloads.push((PayloadType::Encrypted, data));
    }
    
    /// Add public key to the message (PKE mode)
    pub fn add_public_key(&mut self, pubkey: PublicKeyPayload) {
        let mut data = Vec::new();
        
        // Public key algorithm (1 byte)
        data.push(pubkey.key_algorithm as u8);
        
        // Key parameters length and data (if present)
        if let Some(params) = &pubkey.key_params {
            data.push(params.len() as u8);
            data.extend_from_slice(params);
        } else {
            data.push(0); // No parameters
        }
        
        // Public key length (2 bytes)
        let key_len = pubkey.key_data.len() as u16;
        data.extend_from_slice(&key_len.to_be_bytes());
        
        // Public key data
        data.extend_from_slice(&pubkey.key_data);
        
        self.payloads.push((PayloadType::PublicKey, data));
    }
    
    /// Get timestamp from the message
    pub fn get_timestamp(&self) -> Option<&u64> {
        self.timestamp.as_ref()
    }
    
    /// Get MAC from the message
    pub fn get_mac(&self) -> Option<&[u8]> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::Mac {
                return Some(data);
            }
        }
        None
    }
    
    /// Get key data from the message
    pub fn get_key_data(&self) -> Option<KeyDataPayload> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::KeyData {
                if data.len() < 3 {
                    return None; // Too short
                }
                
                // Key type (1 byte)
                let key_type = data[0];
                
                // Key length (2 bytes)
                let key_len = u16::from_be_bytes([data[1], data[2]]) as usize;
                
                if data.len() < 3 + key_len {
                    return None; // Too short
                }
                
                // Key data
                let key_data = data[3..3 + key_len].to_vec();
                
                // Salt data (if present)
                let mut salt_data = None;
                let mut pos = 3 + key_len;
                
                if data.len() >= pos + 2 {
                    // Salt length (2 bytes)
                    let salt_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
                    pos += 2;
                    
                    if data.len() >= pos + salt_len {
                        salt_data = Some(data[pos..pos + salt_len].to_vec());
                        pos += salt_len;
                    }
                }
                
                // Key validation data (if present)
                let kv_data = if pos < data.len() {
                    Some(data[pos..].to_vec())
                } else {
                    None
                };
                
                return Some(KeyDataPayload {
                    key_type,
                    key_data,
                    salt_data,
                    kv_data,
                });
            }
        }
        None
    }
    
    /// Get certificate from the message (PKE mode)
    pub fn get_certificate(&self) -> Option<CertificatePayload> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::Certificate {
                if data.len() < 3 {
                    return None; // Too short
                }
                
                // Certificate type (1 byte)
                let cert_type = match data[0] {
                    0 => super::payloads::CertificateType::X509,
                    1 => super::payloads::CertificateType::X509Chain,
                    2 => super::payloads::CertificateType::Pgp,
                    _ => super::payloads::CertificateType::Reserved,
                };
                
                // Certificate length (2 bytes)
                let cert_len = u16::from_be_bytes([data[1], data[2]]) as usize;
                
                if data.len() < 3 + cert_len {
                    return None; // Too short
                }
                
                // Certificate data
                let cert_data = data[3..3 + cert_len].to_vec();
                
                // Parse certificate chain (if present)
                let mut cert_chain = Vec::new();
                let mut pos = 3 + cert_len;
                
                while pos + 2 < data.len() {
                    // Chain certificate length (2 bytes)
                    let chain_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
                    pos += 2;
                    
                    if pos + chain_len <= data.len() {
                        cert_chain.push(data[pos..pos + chain_len].to_vec());
                        pos += chain_len;
                    } else {
                        break;
                    }
                }
                
                return Some(CertificatePayload {
                    cert_type,
                    cert_data,
                    cert_chain,
                });
            }
        }
        None
    }
    
    /// Get signature from the message (PKE mode)
    pub fn get_signature(&self) -> Option<SignaturePayload> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::Signature {
                if data.len() < 3 {
                    return None; // Too short
                }
                
                // Signature algorithm (1 byte)
                let sig_algorithm = match data[0] {
                    0 => super::payloads::SignatureAlgorithm::RsaSha256,
                    1 => super::payloads::SignatureAlgorithm::RsaSha512,
                    2 => super::payloads::SignatureAlgorithm::EcdsaSha256,
                    3 => super::payloads::SignatureAlgorithm::EcdsaSha512,
                    4 => super::payloads::SignatureAlgorithm::RsaPssSha256,
                    5 => super::payloads::SignatureAlgorithm::RsaPssSha512,
                    _ => return None, // Unknown algorithm
                };
                
                // Signature length (2 bytes)
                let sig_len = u16::from_be_bytes([data[1], data[2]]) as usize;
                
                if data.len() < 3 + sig_len {
                    return None; // Too short
                }
                
                // Signature data
                let signature = data[3..3 + sig_len].to_vec();
                
                return Some(SignaturePayload {
                    sig_algorithm,
                    signature,
                });
            }
        }
        None
    }
    
    /// Get encrypted payload from the message (PKE mode)
    pub fn get_encrypted(&self) -> Option<EncryptedPayload> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::Encrypted {
                if data.len() < 4 {
                    return None; // Too short
                }
                
                // Encryption algorithm (1 byte)
                let enc_algorithm = match data[0] {
                    0 => super::payloads::EncryptionAlgorithm::RsaPkcs1,
                    1 => super::payloads::EncryptionAlgorithm::RsaOaepSha256,
                    2 => super::payloads::EncryptionAlgorithm::Ecies,
                    _ => return None, // Unknown algorithm
                };
                
                // IV length (1 byte)
                let iv_len = data[1] as usize;
                let mut pos = 2;
                
                // IV data (if present)
                let iv = if iv_len > 0 {
                    if data.len() < pos + iv_len {
                        return None; // Too short
                    }
                    let iv_data = data[pos..pos + iv_len].to_vec();
                    pos += iv_len;
                    Some(iv_data)
                } else {
                    None
                };
                
                // Encrypted data length (2 bytes)
                if data.len() < pos + 2 {
                    return None; // Too short
                }
                let enc_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
                pos += 2;
                
                if data.len() < pos + enc_len {
                    return None; // Too short
                }
                
                // Encrypted data
                let encrypted_data = data[pos..pos + enc_len].to_vec();
                
                return Some(EncryptedPayload {
                    enc_algorithm,
                    encrypted_data,
                    iv,
                });
            }
        }
        None
    }
    
    /// Get public key from the message (PKE mode)
    pub fn get_public_key(&self) -> Option<PublicKeyPayload> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::PublicKey {
                if data.len() < 4 {
                    return None; // Too short
                }
                
                // Public key algorithm (1 byte)
                let key_algorithm = match data[0] {
                    0 => super::payloads::PublicKeyAlgorithm::Rsa,
                    1 => super::payloads::PublicKeyAlgorithm::EcdsaP256,
                    2 => super::payloads::PublicKeyAlgorithm::EcdsaP384,
                    3 => super::payloads::PublicKeyAlgorithm::EcdsaP521,
                    4 => super::payloads::PublicKeyAlgorithm::Ed25519,
                    _ => return None, // Unknown algorithm
                };
                
                // Key parameters length (1 byte)
                let params_len = data[1] as usize;
                let mut pos = 2;
                
                // Key parameters (if present)
                let key_params = if params_len > 0 {
                    if data.len() < pos + params_len {
                        return None; // Too short
                    }
                    let params_data = data[pos..pos + params_len].to_vec();
                    pos += params_len;
                    Some(params_data)
                } else {
                    None
                };
                
                // Public key length (2 bytes)
                if data.len() < pos + 2 {
                    return None; // Too short
                }
                let key_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
                pos += 2;
                
                if data.len() < pos + key_len {
                    return None; // Too short
                }
                
                // Public key data
                let key_data = data[pos..pos + key_len].to_vec();
                
                return Some(PublicKeyPayload {
                    key_algorithm,
                    key_data,
                    key_params,
                });
            }
        }
        None
    }
    
    /// Get the security policy from the message
    pub fn get_security_policy(&self) -> Option<SecurityPolicyPayload> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::SecurityPolicy {
                if data.len() < 2 {
                    return None; // Too short
                }
                
                // Policy number (1 byte)
                let policy_no = data[0];
                
                // Policy type (1 byte)
                let policy_type = data[1];
                
                // Policy parameters
                let policy_param = data[2..].to_vec();
                
                return Some(SecurityPolicyPayload {
                    policy_no,
                    policy_type,
                    policy_param,
                });
            }
        }
        None
    }
    
    /// Serialize the message to bytes (excluding MAC)
    pub fn to_bytes_without_mac(&self) -> Vec<u8> {
        let mut data = Vec::new();
        
        // Add common header if present
        if let Some(header) = &self.common_header {
            // Version (3 bits), Data type (3 bits), Next payload (8 bits)
            let v_dt_next = ((header.version & 0x07) << 5) | 
                           ((header.data_type & 0x07) << 2) |
                           ((header.next_payload >> 6) & 0x03);
            data.push(v_dt_next);
            
            // Next payload (cont.) (6 bits), V flag (1 bit), PRF func (7 bits)
            let next_v_prf = ((header.next_payload & 0x3F) << 2) |
                            (if header.v_flag { 0x02 } else { 0x00 }) |
                            ((header.prf_func >> 6) & 0x01);
            data.push(next_v_prf);
            
            // PRF func (cont.) (6 bits), CSP ID (10 bits)
            let prf_csp = ((header.prf_func & 0x3F) << 2) |
                          (((header.csp_id >> 8) & 0x03) as u8);
            data.push(prf_csp);
            
            // CSP ID (cont.) (8 bits)
            data.push((header.csp_id & 0xFF) as u8);
            
            // CS count (8 bits)
            data.push(header.cs_count);
            
            // CS ID map type (8 bits)
            data.push(header.cs_id_map_type);
        }
        
        // Add payloads except MAC
        for (payload_type, payload_data) in &self.payloads {
            if *payload_type != PayloadType::Mac {
                // Payload type (8 bits)
                data.push(*payload_type as u8);
                
                // Payload length (16 bits)
                let length = payload_data.len() as u16;
                data.extend_from_slice(&length.to_be_bytes());
                
                // Payload data
                data.extend_from_slice(payload_data);
            }
        }
        
        data
    }
    
    /// Serialize the message to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = self.to_bytes_without_mac();
        
        // Add MAC if present
        if let Some(mac) = self.get_mac() {
            // Payload type (8 bits)
            data.push(PayloadType::Mac as u8);
            
            // Payload length (16 bits)
            let length = mac.len() as u16;
            data.extend_from_slice(&length.to_be_bytes());
            
            // Payload data
            data.extend_from_slice(mac);
        }
        
        data
    }
    
    /// Parse a MIKEY message from bytes
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 6 {
            return Err(Error::ParseError("MIKEY message too short".into()));
        }
        
        // Parse common header
        let version = (data[0] >> 5) & 0x07;
        let data_type = (data[0] >> 2) & 0x07;
        let next_payload = ((data[0] & 0x03) << 6) | ((data[1] >> 2) & 0x3F);
        let v_flag = (data[1] & 0x02) != 0;
        let prf_func = ((data[1] & 0x01) << 6) | ((data[2] >> 2) & 0x3F);
        let csp_id = ((data[2] & 0x03) as u16) << 8 | (data[3] as u16);
        let cs_count = data[4];
        let cs_id_map_type = data[5];
        
        let common_header = CommonHeader {
            version,
            data_type,
            next_payload,
            v_flag,
            prf_func,
            csp_id,
            cs_count,
            cs_id_map_type,
        };
        
        // Determine message type
        let message_type = match data_type {
            0 => MikeyMessageType::InitiatorMessage,
            1 => MikeyMessageType::ResponderMessage,
            2 => MikeyMessageType::ErrorMessage,
            _ => return Err(Error::ParseError(format!("Unknown MIKEY message type: {}", data_type))),
        };
        
        let mut message = MikeyMessage::new(message_type);
        message.add_common_header(common_header);
        
        // Parse payloads
        let mut pos = 6;
        let mut current_payload_type = next_payload;
        
        while pos < data.len() && current_payload_type != 0 {
            // Payload type already known from previous header or common header
            
            // Parse payload length
            if pos + 2 >= data.len() {
                return Err(Error::ParseError("Incomplete payload length".into()));
            }
            
            let payload_length = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            
            // Parse payload data
            if pos + payload_length > data.len() {
                return Err(Error::ParseError("Incomplete payload data".into()));
            }
            
            let payload_data = &data[pos..pos + payload_length];
            pos += payload_length;
            
            // Store payload
            let payload_type = match current_payload_type {
                0 => PayloadType::Last,
                1 => PayloadType::KeyData,
                2 => PayloadType::Timestamp,
                3 => PayloadType::Rand,
                4 => PayloadType::SecurityPolicy,
                5 => PayloadType::KeyValidationData,
                6 => PayloadType::GeneralExtension,
                9 => PayloadType::Mac,
                _ => PayloadType::Unknown,
            };
            
            message.payloads.push((payload_type, payload_data.to_vec()));
            
            // Special processing for certain payload types
            match payload_type {
                PayloadType::Timestamp => {
                    if payload_data.len() == 8 {
                        let mut timestamp_bytes = [0u8; 8];
                        timestamp_bytes.copy_from_slice(payload_data);
                        message.timestamp = Some(u64::from_be_bytes(timestamp_bytes));
                    }
                },
                _ => {}
            }
            
            // Get next payload type
            if pos < data.len() {
                current_payload_type = data[pos];
                pos += 1;
            } else {
                current_payload_type = 0; // End of message
            }
        }
        
        Ok(message)
    }
} 