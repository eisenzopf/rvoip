use std::net::{IpAddr, SocketAddr};

use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use rand::Rng;

use crate::error::{Error, Result};

/// STUN message header size (20 bytes)
const STUN_HEADER_SIZE: usize = 20;

/// STUN magic cookie value (RFC 5389)
const STUN_MAGIC_COOKIE: u32 = 0x2112A442;

/// STUN message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StunMessageType {
    /// Binding request
    BindingRequest,
    /// Binding response
    BindingResponse,
    /// Binding error response
    BindingErrorResponse,
    /// Other message type (with class and method fields)
    Other { class: u8, method: u16 },
}

impl StunMessageType {
    /// Convert to u16 for encoding
    pub fn to_u16(self) -> u16 {
        match self {
            Self::BindingRequest => 0x0001,
            Self::BindingResponse => 0x0101,
            Self::BindingErrorResponse => 0x0111,
            Self::Other { class, method } => {
                let class_value = (class & 0x03) as u16;
                // STUN message method is 12 bits
                let method_value = method & 0x0FFF;
                // Class bits are split in STUN message type
                // See RFC 5389 section 6
                let c0 = (class_value & 0x01) << 4;
                let c1 = (class_value & 0x02) << 7;
                let m0 = method_value & 0x0F;
                let m1 = (method_value & 0x0F0) >> 4;
                let m2 = (method_value & 0xF00) >> 8;
                (m2 << 12) | (c1) | (m1 << 4) | (c0) | m0
            }
        }
    }

    /// Convert from u16 to message type
    pub fn from_u16(value: u16) -> Self {
        match value {
            0x0001 => Self::BindingRequest,
            0x0101 => Self::BindingResponse,
            0x0111 => Self::BindingErrorResponse,
            _ => {
                // Extract class and method
                let c0 = (value & 0x0010) >> 4;
                let c1 = (value & 0x0100) >> 7;
                let class = (c1 | c0) as u8;
                
                let m0 = value & 0x000F;
                let m1 = (value & 0x00E0) >> 4;
                let m2 = (value & 0x3E00) >> 8;
                let method = (m2 << 8) | (m1 << 4) | m0;
                
                Self::Other { class, method }
            }
        }
    }
}

/// STUN attribute types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum StunAttributeType {
    MappedAddress = 0x0001,
    XorMappedAddress = 0x0020,
    Username = 0x0006,
    MessageIntegrity = 0x0008,
    ErrorCode = 0x0009,
    UnknownAttributes = 0x000A,
    Realm = 0x0014,
    Nonce = 0x0015,
    Software = 0x8022,
    Priority = 0x0024,
    UseCandidate = 0x0025,
    Fingerprint = 0x8028,
    IceControlled = 0x8029,
    IceControlling = 0x802A,
    Other(u16),
}

impl From<u16> for StunAttributeType {
    fn from(value: u16) -> Self {
        match value {
            0x0001 => Self::MappedAddress,
            0x0020 => Self::XorMappedAddress,
            0x0006 => Self::Username,
            0x0008 => Self::MessageIntegrity,
            0x0009 => Self::ErrorCode,
            0x000A => Self::UnknownAttributes,
            0x0014 => Self::Realm,
            0x0015 => Self::Nonce,
            0x8022 => Self::Software,
            0x0024 => Self::Priority,
            0x0025 => Self::UseCandidate,
            0x8028 => Self::Fingerprint,
            0x8029 => Self::IceControlled,
            0x802A => Self::IceControlling,
            _ => Self::Other(value),
        }
    }
}

impl From<StunAttributeType> for u16 {
    fn from(attr_type: StunAttributeType) -> Self {
        match attr_type {
            StunAttributeType::MappedAddress => 0x0001,
            StunAttributeType::XorMappedAddress => 0x0020,
            StunAttributeType::Username => 0x0006,
            StunAttributeType::MessageIntegrity => 0x0008,
            StunAttributeType::ErrorCode => 0x0009,
            StunAttributeType::UnknownAttributes => 0x000A,
            StunAttributeType::Realm => 0x0014,
            StunAttributeType::Nonce => 0x0015,
            StunAttributeType::Software => 0x8022,
            StunAttributeType::Priority => 0x0024,
            StunAttributeType::UseCandidate => 0x0025,
            StunAttributeType::Fingerprint => 0x8028,
            StunAttributeType::IceControlled => 0x8029,
            StunAttributeType::IceControlling => 0x802A,
            StunAttributeType::Other(value) => value,
        }
    }
}

/// STUN attribute
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StunAttribute {
    /// Attribute type
    pub attr_type: StunAttributeType,
    /// Attribute value
    pub value: Bytes,
}

impl StunAttribute {
    /// Create a new attribute
    pub fn new(attr_type: StunAttributeType, value: Bytes) -> Self {
        Self { attr_type, value }
    }

    /// Create a XOR-MAPPED-ADDRESS attribute
    pub fn xor_mapped_address(addr: SocketAddr, transaction_id: &[u8; 12]) -> Self {
        let mut value = BytesMut::with_capacity(8);
        
        // First byte is reserved and should be 0
        value.put_u8(0);
        
        // Second byte is address family (IPv4 = 1, IPv6 = 2)
        let family = match addr.ip() {
            IpAddr::V4(_) => 1,
            IpAddr::V6(_) => 2,
        };
        value.put_u8(family);
        
        // Port (XORed with the first 2 bytes of the magic cookie)
        let xor_port = addr.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16;
        value.put_u16(xor_port);
        
        // Address (XORed with the magic cookie and optionally transaction ID)
        match addr.ip() {
            IpAddr::V4(ipv4) => {
                let ip_bytes = ipv4.octets();
                let xor_ip = u32::from_be_bytes(ip_bytes) ^ STUN_MAGIC_COOKIE;
                value.put_u32(xor_ip);
            }
            IpAddr::V6(ipv6) => {
                let ip_bytes = ipv6.octets();
                let mut xor_ip = [0u8; 16];
                
                // XOR with magic cookie
                for i in 0..4 {
                    xor_ip[i] = ip_bytes[i] ^ ((STUN_MAGIC_COOKIE >> (24 - i * 8)) & 0xff) as u8;
                }
                
                // XOR with transaction ID
                for i in 0..12 {
                    xor_ip[i + 4] = ip_bytes[i + 4] ^ transaction_id[i];
                }
                
                value.put_slice(&xor_ip);
            }
        }
        
        Self::new(StunAttributeType::XorMappedAddress, value.freeze())
    }

    /// Create a USERNAME attribute
    pub fn username(username: &str) -> Self {
        Self::new(StunAttributeType::Username, Bytes::copy_from_slice(username.as_bytes()))
    }

    /// Create a SOFTWARE attribute
    pub fn software(software: &str) -> Self {
        Self::new(StunAttributeType::Software, Bytes::copy_from_slice(software.as_bytes()))
    }

    /// Create a PRIORITY attribute
    pub fn priority(priority: u32) -> Self {
        let mut value = BytesMut::with_capacity(4);
        value.put_u32(priority);
        Self::new(StunAttributeType::Priority, value.freeze())
    }

    /// Create a USE-CANDIDATE attribute
    pub fn use_candidate() -> Self {
        Self::new(StunAttributeType::UseCandidate, Bytes::new())
    }

    /// Create an ICE-CONTROLLING attribute
    pub fn ice_controlling(tiebreaker: u64) -> Self {
        let mut value = BytesMut::with_capacity(8);
        value.put_u64(tiebreaker);
        Self::new(StunAttributeType::IceControlling, value.freeze())
    }

    /// Create an ICE-CONTROLLED attribute
    pub fn ice_controlled(tiebreaker: u64) -> Self {
        let mut value = BytesMut::with_capacity(8);
        value.put_u64(tiebreaker);
        Self::new(StunAttributeType::IceControlled, value.freeze())
    }

    /// Get socket address from XOR-MAPPED-ADDRESS attribute
    pub fn get_xor_mapped_address(&self, transaction_id: &[u8; 12]) -> Result<SocketAddr> {
        if self.attr_type != StunAttributeType::XorMappedAddress {
            return Err(Error::StunError("Not a XOR-MAPPED-ADDRESS attribute".to_string()));
        }

        let mut value = self.value.clone();
        
        // Skip reserved byte
        value.advance(1);
        
        // Get address family
        let family = value.get_u8();
        
        // Get XORed port
        let xor_port = value.get_u16();
        let port = xor_port ^ (STUN_MAGIC_COOKIE >> 16) as u16;

        // Get XORed address
        let addr = match family {
            1 => { // IPv4
                let xor_ip = value.get_u32();
                let ip_int = xor_ip ^ STUN_MAGIC_COOKIE;
                let octets = ip_int.to_be_bytes();
                IpAddr::from(octets)
            },
            2 => { // IPv6
                let mut ip_bytes = [0u8; 16];
                value.copy_to_slice(&mut ip_bytes);
                
                // XOR with magic cookie and transaction ID
                let mut xor_ip = [0u8; 16];
                
                // XOR with magic cookie (first 4 bytes)
                for i in 0..4 {
                    xor_ip[i] = ip_bytes[i] ^ ((STUN_MAGIC_COOKIE >> (24 - i * 8)) & 0xff) as u8;
                }
                
                // XOR with transaction ID (remaining 12 bytes)
                for i in 0..12 {
                    xor_ip[i + 4] = ip_bytes[i + 4] ^ transaction_id[i];
                }
                
                IpAddr::from(xor_ip)
            },
            _ => return Err(Error::StunError(format!("Unsupported address family: {}", family))),
        };

        Ok(SocketAddr::new(addr, port))
    }
}

/// STUN message
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StunMessage {
    /// Message type
    pub msg_type: StunMessageType,
    /// Transaction ID
    pub transaction_id: [u8; 12],
    /// Attributes
    pub attributes: Vec<StunAttribute>,
}

impl StunMessage {
    /// Create a new STUN message
    pub fn new(msg_type: StunMessageType) -> Self {
        let mut transaction_id = [0u8; 12];
        rand::thread_rng().fill(&mut transaction_id);
        
        Self {
            msg_type,
            transaction_id,
            attributes: Vec::new(),
        }
    }

    /// Create a new binding request
    pub fn binding_request() -> Self {
        Self::new(StunMessageType::BindingRequest)
    }

    /// Create a binding response
    pub fn binding_response() -> Self {
        Self::new(StunMessageType::BindingResponse)
    }

    /// Add an attribute
    pub fn add_attribute(&mut self, attr: StunAttribute) -> &mut Self {
        self.attributes.push(attr);
        self
    }

    /// Encode message to bytes
    pub fn encode(&self) -> Bytes {
        // Calculate attributes size
        let attr_size: usize = self.attributes.iter()
            .map(|attr| attr.value.len() + 4) // 4 bytes for type and length
            .sum();
        
        // Pad to multiple of 4 bytes if needed
        let padding_size = (4 - (attr_size % 4)) % 4;
        
        // Create buffer for header + attributes
        let mut buf = BytesMut::with_capacity(STUN_HEADER_SIZE + attr_size + padding_size);
        
        // Write message type
        buf.put_u16(self.msg_type.to_u16());
        
        // Write message length (attributes size, will be filled later)
        buf.put_u16(0);
        
        // Write magic cookie
        buf.put_u32(STUN_MAGIC_COOKIE);
        
        // Write transaction ID
        buf.put_slice(&self.transaction_id);
        
        // Write attributes
        for attr in &self.attributes {
            let attr_type: u16 = attr.attr_type.into();
            let attr_len = attr.value.len() as u16;
            
            buf.put_u16(attr_type);
            buf.put_u16(attr_len);
            buf.put_slice(&attr.value);
            
            // Add padding if needed
            let padding = (4 - (attr.value.len() % 4)) % 4;
            for _ in 0..padding {
                buf.put_u8(0);
            }
        }
        
        // Fill message length
        let msg_len = buf.len() - STUN_HEADER_SIZE;
        let slice = &mut buf[2..4];
        BigEndian::write_u16(slice, msg_len as u16);
        
        buf.freeze()
    }

    /// Decode message from bytes
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < STUN_HEADER_SIZE {
            return Err(Error::StunError("Packet too small for STUN header".to_string()));
        }
        
        // Check first two bits are 0 (required by STUN)
        if (bytes[0] & 0xC0) != 0 {
            return Err(Error::StunError("Invalid STUN message".to_string()));
        }
        
        // Extract message type
        let msg_type = u16::from_be_bytes([bytes[0], bytes[1]]);
        let msg_type = StunMessageType::from_u16(msg_type);
        
        // Extract message length
        let msg_length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        
        // Validate magic cookie
        let cookie = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if cookie != STUN_MAGIC_COOKIE {
            return Err(Error::StunError("Invalid STUN magic cookie".to_string()));
        }
        
        // Extract transaction ID
        let mut transaction_id = [0u8; 12];
        transaction_id.copy_from_slice(&bytes[8..20]);
        
        // Make sure the buffer has enough data for header + attributes
        if bytes.len() < STUN_HEADER_SIZE + msg_length {
            return Err(Error::StunError("Packet too small for STUN attributes".to_string()));
        }
        
        // Parse attributes
        let mut attributes = Vec::new();
        let mut offset = STUN_HEADER_SIZE;
        
        while offset < STUN_HEADER_SIZE + msg_length {
            // Make sure there's enough data for attribute header
            if offset + 4 > bytes.len() {
                return Err(Error::StunError("Incomplete STUN attribute".to_string()));
            }
            
            // Get attribute type and length
            let attr_type = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
            let attr_length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
            
            offset += 4;
            
            // Make sure there's enough data for attribute value
            if offset + attr_length > bytes.len() {
                return Err(Error::StunError("Incomplete STUN attribute value".to_string()));
            }
            
            // Get attribute value
            let value = Bytes::copy_from_slice(&bytes[offset..offset + attr_length]);
            offset += attr_length;
            
            // Skip padding
            let padding = (4 - (attr_length % 4)) % 4;
            offset += padding;
            
            // Create attribute
            let attr = StunAttribute {
                attr_type: attr_type.into(),
                value,
            };
            
            attributes.push(attr);
        }
        
        Ok(Self {
            msg_type,
            transaction_id,
            attributes,
        })
    }

    /// Get an attribute by type
    pub fn get_attribute(&self, attr_type: StunAttributeType) -> Option<&StunAttribute> {
        self.attributes.iter().find(|attr| attr.attr_type == attr_type)
    }
} 