//! MIKEY message implementation
//! 
//! This module implements the MIKEY message format as defined in RFC 3830.
//! MIKEY messages consist of a common header followed by a sequence of payloads.

use crate::Error;
use super::payloads::{
    PayloadType, CommonHeader, KeyDataPayload, 
    GeneralExtensionPayload, KeyValidationData,
    SecurityPolicyPayload
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
        // Serialize security policy
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
    
    /// Get the MAC from the message
    pub fn get_mac(&self) -> Option<&[u8]> {
        for (payload_type, data) in &self.payloads {
            if *payload_type == PayloadType::Mac {
                return Some(data);
            }
        }
        None
    }
    
    /// Get the timestamp from the message
    pub fn get_timestamp(&self) -> Option<&u64> {
        self.timestamp.as_ref()
    }
    
    /// Get the key data from the message
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