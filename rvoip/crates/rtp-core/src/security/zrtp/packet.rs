//! ZRTP packet implementation
//!
//! This module implements the ZRTP packet format as defined in RFC 6189.

use crate::Error;
use super::{ZrtpCipher, ZrtpHash, ZrtpAuthTag, ZrtpKeyAgreement, ZrtpSasType};

/// ZRTP message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpMessageType {
    /// Hello message
    Hello,
    /// HelloACK message
    HelloAck,
    /// Commit message
    Commit,
    /// DH Part 1 message
    DHPart1,
    /// DH Part 2 message
    DHPart2,
    /// Confirm1 message
    Confirm1,
    /// Confirm2 message
    Confirm2,
    /// Conf2ACK message
    Conf2Ack,
    /// Error message
    Error,
    /// ErrorACK message
    ErrorAck,
    /// GoClear message
    GoClear,
    /// ClearACK message
    ClearAck,
    /// SASrelay message
    SasRelay,
    /// RelayACK message
    RelayAck,
    /// Ping message
    Ping,
    /// PingACK message
    PingAck,
}

impl ZrtpMessageType {
    /// Get the message type string (4 characters)
    pub fn to_str(&self) -> &'static str {
        match self {
            ZrtpMessageType::Hello => "Hello   ",
            ZrtpMessageType::HelloAck => "HelloACK",
            ZrtpMessageType::Commit => "Commit  ",
            ZrtpMessageType::DHPart1 => "DHPart1 ",
            ZrtpMessageType::DHPart2 => "DHPart2 ",
            ZrtpMessageType::Confirm1 => "Confirm1",
            ZrtpMessageType::Confirm2 => "Confirm2",
            ZrtpMessageType::Conf2Ack => "Conf2ACK",
            ZrtpMessageType::Error => "Error   ",
            ZrtpMessageType::ErrorAck => "ErrorACK",
            ZrtpMessageType::GoClear => "GoClear ",
            ZrtpMessageType::ClearAck => "ClearACK",
            ZrtpMessageType::SasRelay => "SASrelay",
            ZrtpMessageType::RelayAck => "RelayACK",
            ZrtpMessageType::Ping => "Ping    ",
            ZrtpMessageType::PingAck => "PingACK ",
        }
    }
    
    /// Parse a message type from a string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Hello   " => Some(ZrtpMessageType::Hello),
            "HelloACK" => Some(ZrtpMessageType::HelloAck),
            "Commit  " => Some(ZrtpMessageType::Commit),
            "DHPart1 " => Some(ZrtpMessageType::DHPart1),
            "DHPart2 " => Some(ZrtpMessageType::DHPart2),
            "Confirm1" => Some(ZrtpMessageType::Confirm1),
            "Confirm2" => Some(ZrtpMessageType::Confirm2),
            "Conf2ACK" => Some(ZrtpMessageType::Conf2Ack),
            "Error   " => Some(ZrtpMessageType::Error),
            "ErrorACK" => Some(ZrtpMessageType::ErrorAck),
            "GoClear " => Some(ZrtpMessageType::GoClear),
            "ClearACK" => Some(ZrtpMessageType::ClearAck),
            "SASrelay" => Some(ZrtpMessageType::SasRelay),
            "RelayACK" => Some(ZrtpMessageType::RelayAck),
            "Ping    " => Some(ZrtpMessageType::Ping),
            "PingACK " => Some(ZrtpMessageType::PingAck),
            _ => None,
        }
    }
}

/// ZRTP protocol version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpVersion {
    /// Version 1.2 (current version)
    V12,
}

impl ZrtpVersion {
    /// Get the version string
    pub fn to_str(&self) -> &'static str {
        match self {
            ZrtpVersion::V12 => "1.2",
        }
    }
}

/// ZRTP packet structure
#[derive(Debug, Clone)]
pub struct ZrtpPacket {
    /// Message type
    message_type: ZrtpMessageType,
    /// Protocol version
    version: Option<ZrtpVersion>,
    /// Client identifier
    client_id: Option<String>,
    /// ZRTP identifier (ZID)
    zid: Option<[u8; 12]>,
    /// Supported ciphers
    ciphers: Vec<ZrtpCipher>,
    /// Supported hashes
    hashes: Vec<ZrtpHash>,
    /// Supported authentication tags
    auth_tags: Vec<ZrtpAuthTag>,
    /// Supported key agreement methods
    key_agreements: Vec<ZrtpKeyAgreement>,
    /// Supported SAS types
    sas_types: Vec<ZrtpSasType>,
    /// Selected cipher
    selected_cipher: Option<ZrtpCipher>,
    /// Selected hash
    selected_hash: Option<ZrtpHash>,
    /// Selected authentication tag
    selected_auth_tag: Option<ZrtpAuthTag>,
    /// Selected key agreement
    selected_key_agreement: Option<ZrtpKeyAgreement>,
    /// Selected SAS type
    selected_sas_type: Option<ZrtpSasType>,
    /// Public key
    public_key: Option<Vec<u8>>,
    /// Message authentication code
    mac: Option<Vec<u8>>,
    /// Raw packet data
    raw_data: Vec<u8>,
}

impl ZrtpPacket {
    /// Create a new ZRTP packet
    pub fn new(message_type: ZrtpMessageType) -> Self {
        Self {
            message_type,
            version: None,
            client_id: None,
            zid: None,
            ciphers: Vec::new(),
            hashes: Vec::new(),
            auth_tags: Vec::new(),
            key_agreements: Vec::new(),
            sas_types: Vec::new(),
            selected_cipher: None,
            selected_hash: None,
            selected_auth_tag: None,
            selected_key_agreement: None,
            selected_sas_type: None,
            public_key: None,
            mac: None,
            raw_data: Vec::new(),
        }
    }
    
    /// Get the message type
    pub fn message_type(&self) -> ZrtpMessageType {
        self.message_type
    }
    
    /// Set the protocol version
    pub fn set_version(&mut self, version: ZrtpVersion) {
        self.version = Some(version);
    }
    
    /// Set the client identifier
    pub fn set_client_id(&mut self, client_id: &str) {
        self.client_id = Some(client_id.to_string());
    }
    
    /// Set the ZID
    pub fn set_zid(&mut self, zid: &[u8; 12]) {
        self.zid = Some(*zid);
    }
    
    /// Get the ZID
    pub fn zid(&self) -> Option<[u8; 12]> {
        self.zid
    }
    
    /// Add a supported cipher
    pub fn add_cipher(&mut self, cipher: ZrtpCipher) {
        self.ciphers.push(cipher);
    }
    
    /// Get supported ciphers
    pub fn ciphers(&self) -> &[ZrtpCipher] {
        &self.ciphers
    }
    
    /// Add a supported hash
    pub fn add_hash(&mut self, hash: ZrtpHash) {
        self.hashes.push(hash);
    }
    
    /// Get supported hashes
    pub fn hashes(&self) -> &[ZrtpHash] {
        &self.hashes
    }
    
    /// Add a supported authentication tag
    pub fn add_auth_tag(&mut self, auth_tag: ZrtpAuthTag) {
        self.auth_tags.push(auth_tag);
    }
    
    /// Get supported authentication tags
    pub fn auth_tags(&self) -> &[ZrtpAuthTag] {
        &self.auth_tags
    }
    
    /// Add a supported key agreement method
    pub fn add_key_agreement(&mut self, key_agreement: ZrtpKeyAgreement) {
        self.key_agreements.push(key_agreement);
    }
    
    /// Get supported key agreement methods
    pub fn key_agreements(&self) -> &[ZrtpKeyAgreement] {
        &self.key_agreements
    }
    
    /// Add a supported SAS type
    pub fn add_sas_type(&mut self, sas_type: ZrtpSasType) {
        self.sas_types.push(sas_type);
    }
    
    /// Get supported SAS types
    pub fn sas_types(&self) -> &[ZrtpSasType] {
        &self.sas_types
    }
    
    /// Set the selected cipher
    pub fn set_cipher(&mut self, cipher: ZrtpCipher) {
        self.selected_cipher = Some(cipher);
    }
    
    /// Get the selected cipher
    pub fn cipher(&self) -> Option<ZrtpCipher> {
        self.selected_cipher
    }
    
    /// Set the selected hash
    pub fn set_hash(&mut self, hash: ZrtpHash) {
        self.selected_hash = Some(hash);
    }
    
    /// Get the selected hash
    pub fn hash(&self) -> Option<ZrtpHash> {
        self.selected_hash
    }
    
    /// Set the selected authentication tag
    pub fn set_auth_tag(&mut self, auth_tag: ZrtpAuthTag) {
        self.selected_auth_tag = Some(auth_tag);
    }
    
    /// Get the selected authentication tag
    pub fn auth_tag(&self) -> Option<ZrtpAuthTag> {
        self.selected_auth_tag
    }
    
    /// Set the selected key agreement
    pub fn set_key_agreement(&mut self, key_agreement: ZrtpKeyAgreement) {
        self.selected_key_agreement = Some(key_agreement);
    }
    
    /// Get the selected key agreement
    pub fn key_agreement(&self) -> Option<ZrtpKeyAgreement> {
        self.selected_key_agreement
    }
    
    /// Set the selected SAS type
    pub fn set_sas_type(&mut self, sas_type: ZrtpSasType) {
        self.selected_sas_type = Some(sas_type);
    }
    
    /// Get the selected SAS type
    pub fn sas_type(&self) -> Option<ZrtpSasType> {
        self.selected_sas_type
    }
    
    /// Set the public key
    pub fn set_public_key(&mut self, public_key: &[u8]) {
        self.public_key = Some(public_key.to_vec());
    }
    
    /// Get the public key
    pub fn public_key(&self) -> Option<Vec<u8>> {
        self.public_key.clone()
    }
    
    /// Set the MAC
    pub fn set_mac(&mut self, mac: &[u8]) {
        self.mac = Some(mac.to_vec());
    }
    
    /// Get the MAC
    pub fn mac(&self) -> Option<Vec<u8>> {
        self.mac.clone()
    }
    
    /// Convert cipher to 4-character code
    fn cipher_to_str(cipher: ZrtpCipher) -> &'static str {
        match cipher {
            ZrtpCipher::Aes1 => "AES1",
            ZrtpCipher::Aes3 => "AES3",
            ZrtpCipher::TwoF => "TwoF",
        }
    }
    
    /// Convert hash to 4-character code
    fn hash_to_str(hash: ZrtpHash) -> &'static str {
        match hash {
            ZrtpHash::S256 => "S256",
            ZrtpHash::S384 => "S384",
        }
    }
    
    /// Convert authentication tag to 4-character code
    fn auth_tag_to_str(auth_tag: ZrtpAuthTag) -> &'static str {
        match auth_tag {
            ZrtpAuthTag::HS32 => "HS32",
            ZrtpAuthTag::HS80 => "HS80",
        }
    }
    
    /// Convert key agreement to 4-character code
    fn key_agreement_to_str(key_agreement: ZrtpKeyAgreement) -> &'static str {
        match key_agreement {
            ZrtpKeyAgreement::DH3k => "DH3k",
            ZrtpKeyAgreement::DH4k => "DH4k",
            ZrtpKeyAgreement::EC25 => "EC25",
            ZrtpKeyAgreement::EC38 => "EC38",
        }
    }
    
    /// Convert SAS type to 4-character code
    fn sas_type_to_str(sas_type: ZrtpSasType) -> &'static str {
        match sas_type {
            ZrtpSasType::B32 => "B32 ",
            ZrtpSasType::B32E => "B32E",
        }
    }
    
    /// Parse cipher from 4-character code
    fn str_to_cipher(s: &str) -> Option<ZrtpCipher> {
        match s {
            "AES1" => Some(ZrtpCipher::Aes1),
            "AES3" => Some(ZrtpCipher::Aes3),
            "TwoF" => Some(ZrtpCipher::TwoF),
            _ => None,
        }
    }
    
    /// Parse hash from 4-character code
    fn str_to_hash(s: &str) -> Option<ZrtpHash> {
        match s {
            "S256" => Some(ZrtpHash::S256),
            "S384" => Some(ZrtpHash::S384),
            _ => None,
        }
    }
    
    /// Parse authentication tag from 4-character code
    fn str_to_auth_tag(s: &str) -> Option<ZrtpAuthTag> {
        match s {
            "HS32" => Some(ZrtpAuthTag::HS32),
            "HS80" => Some(ZrtpAuthTag::HS80),
            _ => None,
        }
    }
    
    /// Parse key agreement from 4-character code
    fn str_to_key_agreement(s: &str) -> Option<ZrtpKeyAgreement> {
        match s {
            "DH3k" => Some(ZrtpKeyAgreement::DH3k),
            "DH4k" => Some(ZrtpKeyAgreement::DH4k),
            "EC25" => Some(ZrtpKeyAgreement::EC25),
            "EC38" => Some(ZrtpKeyAgreement::EC38),
            _ => None,
        }
    }
    
    /// Parse SAS type from 4-character code
    fn str_to_sas_type(s: &str) -> Option<ZrtpSasType> {
        match s {
            "B32 " => Some(ZrtpSasType::B32),
            "B32E" => Some(ZrtpSasType::B32E),
            _ => None,
        }
    }
    
    /// Serialize the packet to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        // For the sake of this implementation, we'll create a simplified ZRTP packet format
        // In a real implementation, we'd follow the exact format in RFC 6189
        let mut data = Vec::new();
        
        // ZRTP magic cookie
        data.extend_from_slice(b"ZRTP");
        
        // Message type
        data.extend_from_slice(self.message_type.to_str().as_bytes());
        
        // For Hello message, include supported algorithms
        if self.message_type == ZrtpMessageType::Hello {
            // Version
            if let Some(version) = &self.version {
                data.extend_from_slice(version.to_str().as_bytes());
            } else {
                data.extend_from_slice(b"1.2");
            }
            
            // Client ID
            if let Some(client_id) = &self.client_id {
                // Fixed-length client ID (16 bytes)
                let mut client_id_bytes = [0u8; 16];
                for (i, b) in client_id.bytes().enumerate().take(16) {
                    client_id_bytes[i] = b;
                }
                data.extend_from_slice(&client_id_bytes);
            } else {
                data.extend_from_slice(&[0u8; 16]);
            }
            
            // ZID
            if let Some(zid) = &self.zid {
                data.extend_from_slice(zid);
            } else {
                data.extend_from_slice(&[0u8; 12]);
            }
            
            // Cipher count
            data.push(self.ciphers.len() as u8);
            
            // Ciphers
            for cipher in &self.ciphers {
                data.extend_from_slice(Self::cipher_to_str(*cipher).as_bytes());
            }
            
            // Hash count
            data.push(self.hashes.len() as u8);
            
            // Hashes
            for hash in &self.hashes {
                data.extend_from_slice(Self::hash_to_str(*hash).as_bytes());
            }
            
            // Auth tag count
            data.push(self.auth_tags.len() as u8);
            
            // Auth tags
            for auth_tag in &self.auth_tags {
                data.extend_from_slice(Self::auth_tag_to_str(*auth_tag).as_bytes());
            }
            
            // Key agreement count
            data.push(self.key_agreements.len() as u8);
            
            // Key agreements
            for key_agreement in &self.key_agreements {
                data.extend_from_slice(Self::key_agreement_to_str(*key_agreement).as_bytes());
            }
            
            // SAS type count
            data.push(self.sas_types.len() as u8);
            
            // SAS types
            for sas_type in &self.sas_types {
                data.extend_from_slice(Self::sas_type_to_str(*sas_type).as_bytes());
            }
        }
        
        // For Commit message, include selected algorithms
        if self.message_type == ZrtpMessageType::Commit {
            // ZID
            if let Some(zid) = &self.zid {
                data.extend_from_slice(zid);
            } else {
                data.extend_from_slice(&[0u8; 12]);
            }
            
            // Selected cipher
            if let Some(cipher) = &self.selected_cipher {
                data.extend_from_slice(Self::cipher_to_str(*cipher).as_bytes());
            } else {
                data.extend_from_slice(b"AES1");
            }
            
            // Selected hash
            if let Some(hash) = &self.selected_hash {
                data.extend_from_slice(Self::hash_to_str(*hash).as_bytes());
            } else {
                data.extend_from_slice(b"S256");
            }
            
            // Selected auth tag
            if let Some(auth_tag) = &self.selected_auth_tag {
                data.extend_from_slice(Self::auth_tag_to_str(*auth_tag).as_bytes());
            } else {
                data.extend_from_slice(b"HS80");
            }
            
            // Selected key agreement
            if let Some(key_agreement) = &self.selected_key_agreement {
                data.extend_from_slice(Self::key_agreement_to_str(*key_agreement).as_bytes());
            } else {
                data.extend_from_slice(b"EC25");
            }
            
            // Selected SAS type
            if let Some(sas_type) = &self.selected_sas_type {
                data.extend_from_slice(Self::sas_type_to_str(*sas_type).as_bytes());
            } else {
                data.extend_from_slice(b"B32 ");
            }
        }
        
        // For DH Part 1/2 messages, include public key
        if self.message_type == ZrtpMessageType::DHPart1 || self.message_type == ZrtpMessageType::DHPart2 {
            if let Some(public_key) = &self.public_key {
                // Public key length
                let key_len = public_key.len() as u16;
                data.extend_from_slice(&key_len.to_be_bytes());
                
                // Public key
                data.extend_from_slice(public_key);
            }
        }
        
        // For Confirm messages, include MAC
        if self.message_type == ZrtpMessageType::Confirm1 || self.message_type == ZrtpMessageType::Confirm2 {
            // ZID
            if let Some(zid) = &self.zid {
                data.extend_from_slice(zid);
            } else {
                data.extend_from_slice(&[0u8; 12]);
            }
            
            if let Some(mac) = &self.mac {
                // MAC length
                let mac_len = mac.len() as u16;
                data.extend_from_slice(&mac_len.to_be_bytes());
                
                // MAC
                data.extend_from_slice(mac);
            }
        }
        
        data
    }
    
    /// Parse a ZRTP packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 12 {
            return Err(Error::ParseError("ZRTP packet too short".into()));
        }
        
        // Check ZRTP magic cookie
        if &data[0..4] != b"ZRTP" {
            return Err(Error::ParseError("Invalid ZRTP magic cookie".into()));
        }
        
        // Parse message type
        let msg_type_str = std::str::from_utf8(&data[4..12])
            .map_err(|_| Error::ParseError("Invalid UTF-8 in message type".into()))?;
        
        let message_type = ZrtpMessageType::from_str(msg_type_str)
            .ok_or_else(|| Error::ParseError(format!("Unknown message type: {}", msg_type_str)))?;
        
        let mut packet = ZrtpPacket::new(message_type);
        packet.raw_data = data.to_vec();
        
        // Parse Hello message
        if message_type == ZrtpMessageType::Hello {
            if data.len() < 28 {
                return Err(Error::ParseError("Hello message too short".into()));
            }
            
            // Version
            let version_str = std::str::from_utf8(&data[12..15])
                .map_err(|_| Error::ParseError("Invalid UTF-8 in version".into()))?;
            
            if version_str == "1.2" {
                packet.version = Some(ZrtpVersion::V12);
            }
            
            // Client ID
            let client_id = std::str::from_utf8(&data[15..31])
                .map_err(|_| Error::ParseError("Invalid UTF-8 in client ID".into()))?
                .trim_end_matches('\0')
                .to_string();
            
            packet.client_id = Some(client_id);
            
            // ZID
            let mut zid = [0u8; 12];
            zid.copy_from_slice(&data[31..43]);
            packet.zid = Some(zid);
            
            // Parse supported algorithms
            let mut pos = 43;
            
            if pos < data.len() {
                // Cipher count
                let cipher_count = data[pos] as usize;
                pos += 1;
                
                // Ciphers
                for _ in 0..cipher_count {
                    if pos + 4 > data.len() {
                        break;
                    }
                    
                    let cipher_str = std::str::from_utf8(&data[pos..pos+4])
                        .map_err(|_| Error::ParseError("Invalid UTF-8 in cipher".into()))?;
                    
                    if let Some(cipher) = Self::str_to_cipher(cipher_str) {
                        packet.ciphers.push(cipher);
                    }
                    
                    pos += 4;
                }
            }
            
            if pos < data.len() {
                // Hash count
                let hash_count = data[pos] as usize;
                pos += 1;
                
                // Hashes
                for _ in 0..hash_count {
                    if pos + 4 > data.len() {
                        break;
                    }
                    
                    let hash_str = std::str::from_utf8(&data[pos..pos+4])
                        .map_err(|_| Error::ParseError("Invalid UTF-8 in hash".into()))?;
                    
                    if let Some(hash) = Self::str_to_hash(hash_str) {
                        packet.hashes.push(hash);
                    }
                    
                    pos += 4;
                }
            }
            
            if pos < data.len() {
                // Auth tag count
                let auth_tag_count = data[pos] as usize;
                pos += 1;
                
                // Auth tags
                for _ in 0..auth_tag_count {
                    if pos + 4 > data.len() {
                        break;
                    }
                    
                    let auth_tag_str = std::str::from_utf8(&data[pos..pos+4])
                        .map_err(|_| Error::ParseError("Invalid UTF-8 in auth tag".into()))?;
                    
                    if let Some(auth_tag) = Self::str_to_auth_tag(auth_tag_str) {
                        packet.auth_tags.push(auth_tag);
                    }
                    
                    pos += 4;
                }
            }
            
            if pos < data.len() {
                // Key agreement count
                let key_agreement_count = data[pos] as usize;
                pos += 1;
                
                // Key agreements
                for _ in 0..key_agreement_count {
                    if pos + 4 > data.len() {
                        break;
                    }
                    
                    let key_agreement_str = std::str::from_utf8(&data[pos..pos+4])
                        .map_err(|_| Error::ParseError("Invalid UTF-8 in key agreement".into()))?;
                    
                    if let Some(key_agreement) = Self::str_to_key_agreement(key_agreement_str) {
                        packet.key_agreements.push(key_agreement);
                    }
                    
                    pos += 4;
                }
            }
            
            if pos < data.len() {
                // SAS type count
                let sas_type_count = data[pos] as usize;
                pos += 1;
                
                // SAS types
                for _ in 0..sas_type_count {
                    if pos + 4 > data.len() {
                        break;
                    }
                    
                    let sas_type_str = std::str::from_utf8(&data[pos..pos+4])
                        .map_err(|_| Error::ParseError("Invalid UTF-8 in SAS type".into()))?;
                    
                    if let Some(sas_type) = Self::str_to_sas_type(sas_type_str) {
                        packet.sas_types.push(sas_type);
                    }
                    
                    pos += 4;
                }
            }
        }
        
        // Parse Commit message
        if message_type == ZrtpMessageType::Commit {
            if data.len() < 32 {
                return Err(Error::ParseError("Commit message too short".into()));
            }
            
            // ZID
            let mut zid = [0u8; 12];
            zid.copy_from_slice(&data[12..24]);
            packet.zid = Some(zid);
            
            // Selected cipher
            let cipher_str = std::str::from_utf8(&data[24..28])
                .map_err(|_| Error::ParseError("Invalid UTF-8 in cipher".into()))?;
            
            packet.selected_cipher = Self::str_to_cipher(cipher_str);
            
            // Selected hash
            let hash_str = std::str::from_utf8(&data[28..32])
                .map_err(|_| Error::ParseError("Invalid UTF-8 in hash".into()))?;
            
            packet.selected_hash = Self::str_to_hash(hash_str);
            
            // Selected auth tag
            let auth_tag_str = std::str::from_utf8(&data[32..36])
                .map_err(|_| Error::ParseError("Invalid UTF-8 in auth tag".into()))?;
            
            packet.selected_auth_tag = Self::str_to_auth_tag(auth_tag_str);
            
            // Selected key agreement
            let key_agreement_str = std::str::from_utf8(&data[36..40])
                .map_err(|_| Error::ParseError("Invalid UTF-8 in key agreement".into()))?;
            
            packet.selected_key_agreement = Self::str_to_key_agreement(key_agreement_str);
            
            // Selected SAS type
            let sas_type_str = std::str::from_utf8(&data[40..44])
                .map_err(|_| Error::ParseError("Invalid UTF-8 in SAS type".into()))?;
            
            packet.selected_sas_type = Self::str_to_sas_type(sas_type_str);
        }
        
        // Parse DH Part 1/2 messages
        if message_type == ZrtpMessageType::DHPart1 || message_type == ZrtpMessageType::DHPart2 {
            if data.len() < 14 {
                return Err(Error::ParseError("DH Part message too short".into()));
            }
            
            // Public key length
            let key_len = u16::from_be_bytes([data[12], data[13]]) as usize;
            
            if data.len() < 14 + key_len {
                return Err(Error::ParseError("Public key data incomplete".into()));
            }
            
            // Public key
            packet.public_key = Some(data[14..14+key_len].to_vec());
        }
        
        // Parse Confirm messages
        if message_type == ZrtpMessageType::Confirm1 || message_type == ZrtpMessageType::Confirm2 {
            if data.len() < 26 {
                return Err(Error::ParseError("Confirm message too short".into()));
            }
            
            // ZID
            let mut zid = [0u8; 12];
            zid.copy_from_slice(&data[12..24]);
            packet.zid = Some(zid);
            
            // MAC length
            let mac_len = u16::from_be_bytes([data[24], data[25]]) as usize;
            
            if data.len() < 26 + mac_len {
                return Err(Error::ParseError("MAC data incomplete".into()));
            }
            
            // MAC
            packet.mac = Some(data[26..26+mac_len].to_vec());
        }
        
        Ok(packet)
    }
} 