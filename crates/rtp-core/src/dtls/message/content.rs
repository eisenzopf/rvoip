//! DTLS content message types
//!
//! This module contains the different content message types used in DTLS.

use bytes::{Bytes, BytesMut, BufMut};

use crate::dtls::record::ContentType;
use super::handshake::HandshakeMessage;

/// DTLS content message
#[derive(Debug, Clone)]
pub enum ContentMessage {
    /// ChangeCipherSpec message
    ChangeCipherSpec(ChangeCipherSpecMessage),
    
    /// Alert message
    Alert(crate::dtls::alert::Alert),
    
    /// Handshake message
    Handshake(HandshakeMessage),
    
    /// Application data
    ApplicationData(Bytes),
}

impl ContentMessage {
    /// Get the content type for this message
    pub fn content_type(&self) -> ContentType {
        match self {
            Self::ChangeCipherSpec(_) => ContentType::ChangeCipherSpec,
            Self::Alert(_) => ContentType::Alert,
            Self::Handshake(_) => ContentType::Handshake,
            Self::ApplicationData(_) => ContentType::ApplicationData,
        }
    }
    
    /// Get the message data
    pub fn data(&self) -> Bytes {
        match self {
            Self::ChangeCipherSpec(msg) => msg.data.clone(),
            Self::Alert(alert) => {
                // Serialize the alert
                let result = alert.serialize();
                match result {
                    Ok(bytes) => bytes,
                    Err(_) => Bytes::new(),
                }
            }
            Self::Handshake(_) => {
                // Handshake messages are complex to serialize
                // This is just a placeholder for now
                Bytes::new()
            }
            Self::ApplicationData(data) => data.clone(),
        }
    }
}

/// ChangeCipherSpec message (single byte with value 1)
#[derive(Debug, Clone)]
pub struct ChangeCipherSpecMessage {
    /// Raw message data
    pub data: Bytes,
}

impl ChangeCipherSpecMessage {
    /// Create a new ChangeCipherSpec message
    pub fn new() -> Self {
        let mut buf = BytesMut::with_capacity(1);
        buf.put_u8(1);
        Self {
            data: buf.freeze(),
        }
    }
}

impl Default for ChangeCipherSpecMessage {
    fn default() -> Self {
        Self::new()
    }
} 