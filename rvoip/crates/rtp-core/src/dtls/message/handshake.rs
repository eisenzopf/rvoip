//! DTLS handshake message types
//!
//! This module contains the different handshake message types used in DTLS.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use std::io::Cursor;
use rand::Rng;

use crate::dtls::Result;
use crate::dtls::DtlsVersion;
use super::extension::Extension;

/// DTLS handshake message type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HandshakeType {
    /// HelloRequest message (sent by server)
    HelloRequest = 0,
    
    /// ClientHello message (sent by client)
    ClientHello = 1,
    
    /// ServerHello message (sent by server)
    ServerHello = 2,
    
    /// HelloVerifyRequest message (sent by server for DTLS)
    HelloVerifyRequest = 3,
    
    /// Certificate message
    Certificate = 11,
    
    /// ServerKeyExchange message
    ServerKeyExchange = 12,
    
    /// CertificateRequest message
    CertificateRequest = 13,
    
    /// ServerHelloDone message
    ServerHelloDone = 14,
    
    /// CertificateVerify message
    CertificateVerify = 15,
    
    /// ClientKeyExchange message
    ClientKeyExchange = 16,
    
    /// Finished message
    Finished = 20,
    
    /// Invalid message type
    Invalid = 255,
}

impl From<u8> for HandshakeType {
    fn from(value: u8) -> Self {
        match value {
            0 => HandshakeType::HelloRequest,
            1 => HandshakeType::ClientHello,
            2 => HandshakeType::ServerHello,
            3 => HandshakeType::HelloVerifyRequest,
            11 => HandshakeType::Certificate,
            12 => HandshakeType::ServerKeyExchange,
            13 => HandshakeType::CertificateRequest,
            14 => HandshakeType::ServerHelloDone,
            15 => HandshakeType::CertificateVerify,
            16 => HandshakeType::ClientKeyExchange,
            20 => HandshakeType::Finished,
            _ => HandshakeType::Invalid,
        }
    }
}

/// DTLS handshake message header
#[derive(Debug, Clone)]
pub struct HandshakeHeader {
    /// Message type
    pub msg_type: HandshakeType,
    
    /// Message length (24 bits)
    pub length: u32,
    
    /// Message sequence number
    pub message_seq: u16,
    
    /// Fragment offset (24 bits)
    pub fragment_offset: u32,
    
    /// Fragment length (24 bits)
    pub fragment_length: u32,
}

impl HandshakeHeader {
    /// Create a new handshake header
    pub fn new(
        msg_type: HandshakeType,
        length: u32,
        message_seq: u16,
        fragment_offset: u32,
        fragment_length: u32,
    ) -> Self {
        Self {
            msg_type,
            length,
            message_seq,
            fragment_offset,
            fragment_length,
        }
    }
    
    /// Serialize the handshake header to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let mut buf = BytesMut::with_capacity(12);
        
        // Message type (1 byte)
        buf.put_u8(self.msg_type as u8);
        
        // Length (3 bytes)
        buf.put_u8((self.length >> 16) as u8);
        buf.put_u8((self.length >> 8) as u8);
        buf.put_u8(self.length as u8);
        
        // Message sequence (2 bytes)
        buf.put_u16(self.message_seq);
        
        // Fragment offset (3 bytes)
        buf.put_u8((self.fragment_offset >> 16) as u8);
        buf.put_u8((self.fragment_offset >> 8) as u8);
        buf.put_u8(self.fragment_offset as u8);
        
        // Fragment length (3 bytes)
        buf.put_u8((self.fragment_length >> 16) as u8);
        buf.put_u8((self.fragment_length >> 8) as u8);
        buf.put_u8(self.fragment_length as u8);
        
        Ok(buf)
    }
    
    /// Parse a handshake header from bytes
    pub fn parse(data: &[u8]) -> Result<(Self, usize)> {
        if data.len() < 12 {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Message type (1 byte)
        let msg_type = HandshakeType::from(cursor.get_u8());
        
        // Length (3 bytes)
        let length = (cursor.get_u8() as u32) << 16
            | (cursor.get_u8() as u32) << 8
            | cursor.get_u8() as u32;
        
        // Message sequence (2 bytes)
        let message_seq = cursor.get_u16();
        
        // Fragment offset (3 bytes)
        let fragment_offset = (cursor.get_u8() as u32) << 16
            | (cursor.get_u8() as u32) << 8
            | cursor.get_u8() as u32;
        
        // Fragment length (3 bytes)
        let fragment_length = (cursor.get_u8() as u32) << 16
            | (cursor.get_u8() as u32) << 8
            | cursor.get_u8() as u32;
        
        let header = Self {
            msg_type,
            length,
            message_seq,
            fragment_offset,
            fragment_length,
        };
        
        Ok((header, 12))
    }
}

/// CipherSuite identifier (16 bits)
pub type CipherSuite = u16;

/// DTLS handshake message
#[derive(Debug, Clone)]
pub enum HandshakeMessage {
    /// ClientHello message
    ClientHello(ClientHello),
    
    /// ServerHello message
    ServerHello(ServerHello),
    
    /// HelloVerifyRequest message
    HelloVerifyRequest(HelloVerifyRequest),
    
    /// Certificate message
    Certificate(Certificate),
    
    /// ServerKeyExchange message
    ServerKeyExchange(ServerKeyExchange),
    
    /// CertificateRequest message
    CertificateRequest(CertificateRequest),
    
    /// ServerHelloDone message
    ServerHelloDone(ServerHelloDone),
    
    /// CertificateVerify message
    CertificateVerify(CertificateVerify),
    
    /// ClientKeyExchange message
    ClientKeyExchange(ClientKeyExchange),
    
    /// Finished message
    Finished(Finished),
}

impl HandshakeMessage {
    /// Get the handshake message type
    pub fn message_type(&self) -> HandshakeType {
        match self {
            Self::ClientHello(_) => HandshakeType::ClientHello,
            Self::ServerHello(_) => HandshakeType::ServerHello,
            Self::HelloVerifyRequest(_) => HandshakeType::HelloVerifyRequest,
            Self::Certificate(_) => HandshakeType::Certificate,
            Self::ServerKeyExchange(_) => HandshakeType::ServerKeyExchange,
            Self::CertificateRequest(_) => HandshakeType::CertificateRequest,
            Self::ServerHelloDone(_) => HandshakeType::ServerHelloDone,
            Self::CertificateVerify(_) => HandshakeType::CertificateVerify,
            Self::ClientKeyExchange(_) => HandshakeType::ClientKeyExchange,
            Self::Finished(_) => HandshakeType::Finished,
        }
    }
    
    /// Serialize the handshake message to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        match self {
            Self::ClientHello(msg) => {
                let serialized = msg.serialize()?;
                buf.extend_from_slice(&serialized);
            }
            Self::ServerHello(msg) => {
                let serialized = msg.serialize()?;
                buf.extend_from_slice(&serialized);
            }
            Self::HelloVerifyRequest(msg) => {
                let serialized = msg.serialize()?;
                buf.extend_from_slice(&serialized);
            }
            Self::ClientKeyExchange(msg) => {
                let serialized = msg.serialize()?;
                buf.extend_from_slice(&serialized);
            }
            Self::ServerKeyExchange(msg) => {
                let serialized = msg.serialize()?;
                buf.extend_from_slice(&serialized);
            }
            Self::Finished(msg) => {
                let serialized = msg.serialize()?;
                buf.extend_from_slice(&serialized);
            }
            // Add other message types as needed
            _ => {
                return Err(crate::error::Error::NotImplemented(
                    format!("Serialization for {:?} not yet implemented", self.message_type())
                ));
            }
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse a handshake message from bytes
    pub fn parse(msg_type: HandshakeType, data: &[u8]) -> Result<Self> {
        match msg_type {
            HandshakeType::ClientHello => {
                let hello = ClientHello::parse(data)?;
                Ok(Self::ClientHello(hello))
            }
            HandshakeType::ServerHello => {
                let hello = ServerHello::parse(data)?;
                Ok(Self::ServerHello(hello))
            }
            HandshakeType::HelloVerifyRequest => {
                let request = HelloVerifyRequest::parse(data)?;
                Ok(Self::HelloVerifyRequest(request))
            }
            HandshakeType::ClientKeyExchange => {
                let key_exchange = ClientKeyExchange::parse(data)?;
                Ok(Self::ClientKeyExchange(key_exchange))
            }
            HandshakeType::ServerKeyExchange => {
                let server_key_exchange = ServerKeyExchange::parse(data)?;
                Ok(Self::ServerKeyExchange(server_key_exchange))
            }
            HandshakeType::Finished => {
                let finished = Finished::parse(data)?;
                Ok(Self::Finished(finished))
            }
            // Add other message types as needed
            _ => {
                Err(crate::error::Error::NotImplemented(
                    format!("Parsing for {:?} not yet implemented", msg_type)
                ))
            }
        }
    }
}

/// ClientHello message
#[derive(Debug, Clone)]
pub struct ClientHello {
    /// Protocol version
    pub version: u16,
    
    /// Random data (32 bytes)
    pub random: [u8; 32],
    
    /// Session ID
    pub session_id: Bytes,
    
    /// Cookie (DTLS only)
    pub cookie: Bytes,
    
    /// Supported cipher suites
    pub cipher_suites: Vec<CipherSuite>,
    
    /// Supported compression methods
    pub compression_methods: Vec<u8>,
    
    /// Extensions
    pub extensions: Vec<Extension>,
}

impl ClientHello {
    /// Create a new ClientHello message
    pub fn new(
        version: DtlsVersion,
        session_id: Bytes,
        cookie: Bytes,
        cipher_suites: Vec<CipherSuite>,
        compression_methods: Vec<u8>,
        extensions: Vec<Extension>,
    ) -> Self {
        // Generate random data (4 bytes timestamp + 28 bytes random)
        let mut rng = rand::thread_rng();
        let mut random = [0u8; 32];
        
        // First 4 bytes are timestamp in seconds since UNIX epoch
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        
        random[0] = (now >> 24) as u8;
        random[1] = (now >> 16) as u8;
        random[2] = (now >> 8) as u8;
        random[3] = now as u8;
        
        // Remaining 28 bytes are random
        rng.fill(&mut random[4..]);
        
        Self {
            version: version as u16,
            random,
            session_id,
            cookie,
            cipher_suites,
            compression_methods,
            extensions,
        }
    }
    
    /// Create a new ClientHello message with default values
    pub fn with_defaults(version: DtlsVersion) -> Self {
        let cipher_suites = vec![
            // ECDHE-ECDSA ciphers
            0xC02B, // TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            0xC02F, // TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
            0xC009, // TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA
            0xC013, // TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA
            0x002F, // TLS_RSA_WITH_AES_128_CBC_SHA
        ];
        
        // No compression
        let compression_methods = vec![0];
        
        // Add SRTP extension
        let srtp_extension = crate::dtls::message::extension::UseSrtpExtension::with_profiles(
            vec![
                crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_80,
                crate::dtls::message::extension::SrtpProtectionProfile::Aes128CmSha1_32,
            ]
        );
        
        let extensions = vec![
            crate::dtls::message::extension::Extension::UseSrtp(srtp_extension),
        ];
        
        Self::new(
            version,
            Bytes::new(), // Empty session ID
            Bytes::new(), // Empty cookie
            cipher_suites,
            compression_methods,
            extensions,
        )
    }
    
    /// Serialize the ClientHello message to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        // Protocol version (2 bytes)
        buf.put_u16(self.version);
        
        // Random (32 bytes)
        buf.extend_from_slice(&self.random);
        
        // Session ID length (1 byte) and data
        buf.put_u8(self.session_id.len() as u8);
        if !self.session_id.is_empty() {
            buf.extend_from_slice(&self.session_id);
        }
        
        // Cookie length (1 byte) and data
        buf.put_u8(self.cookie.len() as u8);
        if !self.cookie.is_empty() {
            buf.extend_from_slice(&self.cookie);
        }
        
        // Cipher suites length (2 bytes) and data
        buf.put_u16((self.cipher_suites.len() * 2) as u16);
        for suite in &self.cipher_suites {
            buf.put_u16(*suite);
        }
        
        // Compression methods length (1 byte) and data
        buf.put_u8(self.compression_methods.len() as u8);
        for method in &self.compression_methods {
            buf.put_u8(*method);
        }
        
        // Extensions length (2 bytes) and data
        if !self.extensions.is_empty() {
            let mut extensions_data = BytesMut::new();
            
            for ext in &self.extensions {
                let ext_data = ext.serialize()?;
                extensions_data.extend_from_slice(&ext_data);
            }
            
            // Extensions length (2 bytes)
            buf.put_u16(extensions_data.len() as u16);
            
            // Extensions data
            buf.extend_from_slice(&extensions_data);
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse a ClientHello message from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 38 { // Minimum: version(2) + random(32) + session_id_len(1) + cookie_len(1) + cipher_suites_len(2)
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Protocol version (2 bytes)
        let version = cursor.get_u16();
        
        // Random (32 bytes)
        let mut random = [0u8; 32];
        cursor.copy_to_slice(&mut random);
        
        // Session ID length (1 byte) and data
        let session_id_len = cursor.get_u8() as usize;
        if data.len() < 35 + session_id_len {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let session_id = if session_id_len > 0 {
            let mut session_id_data = vec![0u8; session_id_len];
            cursor.copy_to_slice(&mut session_id_data);
            Bytes::from(session_id_data)
        } else {
            Bytes::new()
        };
        
        // Cookie length (1 byte) and data
        let cookie_len = cursor.get_u8() as usize;
        if data.len() < 36 + session_id_len + cookie_len {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let cookie = if cookie_len > 0 {
            let mut cookie_data = vec![0u8; cookie_len];
            cursor.copy_to_slice(&mut cookie_data);
            Bytes::from(cookie_data)
        } else {
            Bytes::new()
        };
        
        // Cipher suites length (2 bytes) and data
        let cipher_suites_len = cursor.get_u16() as usize;
        if cipher_suites_len % 2 != 0 {
            return Err(crate::error::Error::InvalidPacket(
                "Cipher suites length must be a multiple of 2".to_string()
            ));
        }
        
        if data.len() < 38 + session_id_len + cookie_len + cipher_suites_len {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cipher_suites = Vec::with_capacity(cipher_suites_len / 2);
        for _ in 0..(cipher_suites_len / 2) {
            cipher_suites.push(cursor.get_u16());
        }
        
        // Compression methods length (1 byte) and data
        let compression_methods_len = cursor.get_u8() as usize;
        if data.len() < 39 + session_id_len + cookie_len + cipher_suites_len + compression_methods_len {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut compression_methods = Vec::with_capacity(compression_methods_len);
        for _ in 0..compression_methods_len {
            compression_methods.push(cursor.get_u8());
        }
        
        // Extensions length (2 bytes) and data
        let mut extensions = Vec::new();
        
        if cursor.position() < data.len() as u64 {
            let extensions_len = cursor.get_u16() as usize;
            let extensions_end = cursor.position() as usize + extensions_len;
            
            if extensions_end > data.len() {
                return Err(crate::error::Error::PacketTooShort);
            }
            
            while cursor.position() < extensions_end as u64 {
                let (extension, _) = Extension::parse(&data[cursor.position() as usize..])?;
                extensions.push(extension);
                
                // Skip over the parsed extension
                let (parsed_type, parsed_len) = {
                    let ext_start = cursor.position() as usize;
                    let mut temp_cursor = Cursor::new(&data[ext_start..]);
                    let typ = temp_cursor.get_u16();
                    let len = temp_cursor.get_u16() as usize;
                    (typ, len)
                };
                
                cursor.set_position(cursor.position() + 4 + parsed_len as u64);
            }
        }
        
        Ok(Self {
            version,
            random,
            session_id,
            cookie,
            cipher_suites,
            compression_methods,
            extensions,
        })
    }
}

/// ServerHello message
#[derive(Debug, Clone)]
pub struct ServerHello {
    /// Protocol version
    pub version: u16,
    
    /// Random data (32 bytes)
    pub random: [u8; 32],
    
    /// Session ID
    pub session_id: Bytes,
    
    /// Selected cipher suite
    pub cipher_suite: CipherSuite,
    
    /// Selected compression method
    pub compression_method: u8,
    
    /// Extensions
    pub extensions: Vec<Extension>,
}

impl ServerHello {
    /// Create a new ServerHello message
    pub fn new(
        version: DtlsVersion,
        session_id: Bytes,
        cipher_suite: CipherSuite,
        compression_method: u8,
        extensions: Vec<Extension>,
    ) -> Self {
        // Generate random data (4 bytes timestamp + 28 bytes random)
        let mut rng = rand::thread_rng();
        let mut random = [0u8; 32];
        
        // First 4 bytes are timestamp in seconds since UNIX epoch
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        
        random[0] = (now >> 24) as u8;
        random[1] = (now >> 16) as u8;
        random[2] = (now >> 8) as u8;
        random[3] = now as u8;
        
        // Remaining 28 bytes are random
        rng.fill(&mut random[4..]);
        
        Self {
            version: version as u16,
            random,
            session_id,
            cipher_suite,
            compression_method,
            extensions,
        }
    }
    
    /// Serialize the ServerHello message to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        // Protocol version (2 bytes)
        buf.put_u16(self.version);
        
        // Random (32 bytes)
        buf.extend_from_slice(&self.random);
        
        // Session ID length (1 byte) and data
        buf.put_u8(self.session_id.len() as u8);
        if !self.session_id.is_empty() {
            buf.extend_from_slice(&self.session_id);
        }
        
        // Cipher suite (2 bytes)
        buf.put_u16(self.cipher_suite);
        
        // Compression method (1 byte)
        buf.put_u8(self.compression_method);
        
        // Extensions length (2 bytes) and data
        if !self.extensions.is_empty() {
            let mut extensions_data = BytesMut::new();
            
            for ext in &self.extensions {
                let ext_data = ext.serialize()?;
                extensions_data.extend_from_slice(&ext_data);
            }
            
            // Extensions length (2 bytes)
            buf.put_u16(extensions_data.len() as u16);
            
            // Extensions data
            buf.extend_from_slice(&extensions_data);
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse a ServerHello message from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 38 { // Minimum: version(2) + random(32) + session_id_len(1) + cipher_suite(2) + compression(1)
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Protocol version (2 bytes)
        let version = cursor.get_u16();
        
        // Random (32 bytes)
        let mut random = [0u8; 32];
        cursor.copy_to_slice(&mut random);
        
        // Session ID length (1 byte) and data
        let session_id_len = cursor.get_u8() as usize;
        if data.len() < 35 + session_id_len + 3 { // +3 for cipher_suite and compression
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let session_id = if session_id_len > 0 {
            let mut session_id_data = vec![0u8; session_id_len];
            cursor.copy_to_slice(&mut session_id_data);
            Bytes::from(session_id_data)
        } else {
            Bytes::new()
        };
        
        // Cipher suite (2 bytes)
        let cipher_suite = cursor.get_u16();
        
        // Compression method (1 byte)
        let compression_method = cursor.get_u8();
        
        // Extensions length (2 bytes) and data
        let mut extensions = Vec::new();
        
        if cursor.position() < data.len() as u64 {
            let extensions_len = cursor.get_u16() as usize;
            let extensions_end = cursor.position() as usize + extensions_len;
            
            if extensions_end > data.len() {
                return Err(crate::error::Error::PacketTooShort);
            }
            
            while cursor.position() < extensions_end as u64 {
                let (extension, _) = Extension::parse(&data[cursor.position() as usize..])?;
                extensions.push(extension);
                
                // Skip over the parsed extension
                let (parsed_type, parsed_len) = {
                    let ext_start = cursor.position() as usize;
                    let mut temp_cursor = Cursor::new(&data[ext_start..]);
                    let typ = temp_cursor.get_u16();
                    let len = temp_cursor.get_u16() as usize;
                    (typ, len)
                };
                
                cursor.set_position(cursor.position() + 4 + parsed_len as u64);
            }
        }
        
        Ok(Self {
            version,
            random,
            session_id,
            cipher_suite,
            compression_method,
            extensions,
        })
    }
}

/// HelloVerifyRequest message (DTLS only)
#[derive(Debug, Clone)]
pub struct HelloVerifyRequest {
    /// Protocol version
    pub version: u16,
    
    /// Cookie
    pub cookie: Bytes,
}

impl HelloVerifyRequest {
    /// Create a new HelloVerifyRequest message
    pub fn new(version: DtlsVersion, cookie: Bytes) -> Self {
        Self {
            version: version as u16,
            cookie,
        }
    }
    
    /// Serialize the HelloVerifyRequest message to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        // Protocol version (2 bytes)
        buf.put_u16(self.version);
        
        // Cookie length (1 byte) and data
        buf.put_u8(self.cookie.len() as u8);
        if !self.cookie.is_empty() {
            buf.extend_from_slice(&self.cookie);
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse a HelloVerifyRequest message from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 3 { // Minimum: version(2) + cookie_len(1)
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Protocol version (2 bytes)
        let version = cursor.get_u16();
        
        // Cookie length (1 byte) and data
        let cookie_len = cursor.get_u8() as usize;
        if data.len() < 3 + cookie_len {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let cookie = if cookie_len > 0 {
            let mut cookie_data = vec![0u8; cookie_len];
            cursor.copy_to_slice(&mut cookie_data);
            Bytes::from(cookie_data)
        } else {
            Bytes::new()
        };
        
        Ok(Self {
            version,
            cookie,
        })
    }
}

/// Certificate message
#[derive(Debug, Clone)]
pub struct Certificate {
    /// Certificate chain
    pub certificates: Vec<Bytes>,
}

/// ServerKeyExchange message
#[derive(Debug, Clone)]
pub struct ServerKeyExchange {
    /// ECDHE curve type (named_curve = 3)
    pub curve_type: u8,
    
    /// Named curve (secp256r1 = 23)
    pub named_curve: u16,
    
    /// Public key length
    pub public_key_length: u8,
    
    /// Public key data in SEC1 format
    pub public_key: Bytes,
    
    /// Signature algorithm (if available)
    pub signature_algorithm: Option<u16>,
    
    /// Signature length
    pub signature_length: Option<u16>,
    
    /// Signature data
    pub signature: Option<Bytes>,
}

impl ServerKeyExchange {
    /// Create a new ECDHE ServerKeyExchange
    pub fn new_ecdhe(public_key: Bytes) -> Self {
        Self {
            curve_type: 3, // named_curve
            named_curve: 23, // secp256r1
            public_key_length: public_key.len() as u8,
            public_key,
            signature_algorithm: None,
            signature_length: None,
            signature: None,
        }
    }
    
    /// Create a new ECDHE ServerKeyExchange with signature
    pub fn new_ecdhe_with_signature(
        public_key: Bytes, 
        signature_algorithm: u16,
        signature: Bytes
    ) -> Self {
        Self {
            curve_type: 3, // named_curve
            named_curve: 23, // secp256r1
            public_key_length: public_key.len() as u8,
            public_key,
            signature_algorithm: Some(signature_algorithm),
            signature_length: Some(signature.len() as u16),
            signature: Some(signature),
        }
    }
    
    /// Serialize the ServerKeyExchange message to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        // Curve type (1 byte)
        buf.put_u8(self.curve_type);
        
        // Named curve (2 bytes)
        buf.put_u16(self.named_curve);
        
        // Public key length (1 byte)
        buf.put_u8(self.public_key_length);
        
        // Public key data
        buf.extend_from_slice(&self.public_key);
        
        // Signature if present
        if let Some(sig_alg) = self.signature_algorithm {
            // Signature algorithm (2 bytes)
            buf.put_u16(sig_alg);
            
            // Signature length (2 bytes)
            if let Some(sig_len) = self.signature_length {
                buf.put_u16(sig_len);
                
                // Signature data
                if let Some(ref sig) = self.signature {
                    buf.extend_from_slice(sig);
                }
            }
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse a ServerKeyExchange message from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 4 { // Minimum: curve_type(1) + named_curve(2) + pubkey_len(1)
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Curve type (1 byte)
        let curve_type = cursor.get_u8();
        if curve_type != 3 { // Only named_curve = 3 is supported
            return Err(crate::error::Error::UnsupportedFeature(
                format!("Unsupported curve type: {}", curve_type)
            ));
        }
        
        // Named curve (2 bytes)
        let named_curve = cursor.get_u16();
        if named_curve != 23 { // Only secp256r1 = 23 is supported
            return Err(crate::error::Error::UnsupportedFeature(
                format!("Unsupported curve: {}", named_curve)
            ));
        }
        
        // Public key length (1 byte)
        let public_key_length = cursor.get_u8();
        
        // Check if we have enough data for the public key
        if data.len() < 4 + public_key_length as usize {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        // Public key data
        let mut public_key = vec![0u8; public_key_length as usize];
        cursor.copy_to_slice(&mut public_key);
        let public_key = Bytes::from(public_key);
        
        // Check if there's signature data
        let mut signature_algorithm = None;
        let mut signature_length = None;
        let mut signature = None;
        
        if cursor.position() < data.len() as u64 && data.len() - cursor.position() as usize >= 4 {
            // Signature algorithm (2 bytes)
            signature_algorithm = Some(cursor.get_u16());
            
            // Signature length (2 bytes)
            let sig_len = cursor.get_u16();
            signature_length = Some(sig_len);
            
            // Check if we have enough data for the signature
            if data.len() - cursor.position() as usize >= sig_len as usize {
                // Signature data
                let mut sig_data = vec![0u8; sig_len as usize];
                cursor.copy_to_slice(&mut sig_data);
                signature = Some(Bytes::from(sig_data));
            }
        }
        
        Ok(Self {
            curve_type,
            named_curve,
            public_key_length,
            public_key,
            signature_algorithm,
            signature_length,
            signature,
        })
    }
}

/// CertificateRequest message
#[derive(Debug, Clone)]
pub struct CertificateRequest {
    /// Certificate types
    pub certificate_types: Vec<u8>,
    
    /// Signature algorithms
    pub signature_algorithms: Vec<u16>,
    
    /// CA names
    pub ca_names: Vec<Bytes>,
}

/// ServerHelloDone message
#[derive(Debug, Clone)]
pub struct ServerHelloDone {
    // This message has no fields
}

/// CertificateVerify message
#[derive(Debug, Clone)]
pub struct CertificateVerify {
    /// Signature algorithm (TLS 1.2+)
    pub algorithm: Option<u16>,
    
    /// Signature
    pub signature: Bytes,
}

/// ClientKeyExchange message
#[derive(Debug, Clone)]
pub struct ClientKeyExchange {
    /// Public key length (for ECDHE)
    pub public_key_length: u8,
    
    /// Public key data in SEC1 format
    pub exchange_data: Bytes,
}

impl ClientKeyExchange {
    /// Create a new ClientKeyExchange message for ECDHE
    pub fn new_ecdhe(public_key: Bytes) -> Self {
        Self {
            public_key_length: public_key.len() as u8,
            exchange_data: public_key,
        }
    }
    
    /// Create a new ClientKeyExchange message (for RSA, not used in ECDHE)
    pub fn new(exchange_data: Bytes) -> Self {
        Self {
            public_key_length: exchange_data.len() as u8,
            exchange_data,
        }
    }
    
    /// Serialize the ClientKeyExchange message to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        // For ECDHE, we need to include the public key length (1 byte)
        buf.put_u8(self.public_key_length);
        
        // Public key data
        buf.extend_from_slice(&self.exchange_data);
        
        Ok(buf.freeze())
    }
    
    /// Parse a ClientKeyExchange message from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // For ECDHE, first byte is the public key length
        let public_key_length = cursor.get_u8();
        
        // Check if we have enough data for the public key
        if data.len() < 1 + public_key_length as usize {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        // Public key data
        let mut exchange_data = vec![0u8; public_key_length as usize];
        cursor.copy_to_slice(&mut exchange_data);
        
        Ok(Self {
            public_key_length,
            exchange_data: Bytes::from(exchange_data),
        })
    }
}

/// Finished message
#[derive(Debug, Clone)]
pub struct Finished {
    /// Verify data
    pub verify_data: Bytes,
}

impl Finished {
    /// Create a new Finished message with the provided verify data
    pub fn new(verify_data: Bytes) -> Self {
        Self {
            verify_data,
        }
    }
    
    /// Serialize the Finished message to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        // Just return the verify data directly
        Ok(self.verify_data.clone())
    }
    
    /// Parse a Finished message from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        // The entire message data is the verify data
        Ok(Self {
            verify_data: Bytes::copy_from_slice(data),
        })
    }
} 