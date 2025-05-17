//! DTLS alert protocol implementation
//!
//! This module handles the alert protocol for DTLS.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use std::io::Cursor;
use super::Result;

/// DTLS alert level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AlertLevel {
    /// Warning alert (not fatal)
    Warning = 1,
    
    /// Fatal alert (connection must be terminated)
    Fatal = 2,
    
    /// Invalid alert level
    Invalid = 255,
}

impl From<u8> for AlertLevel {
    fn from(value: u8) -> Self {
        match value {
            1 => AlertLevel::Warning,
            2 => AlertLevel::Fatal,
            _ => AlertLevel::Invalid,
        }
    }
}

/// DTLS alert description
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AlertDescription {
    /// Close notification (sent when closing connection)
    CloseNotify = 0,
    
    /// Unexpected message received
    UnexpectedMessage = 10,
    
    /// Bad record MAC
    BadRecordMac = 20,
    
    /// Decryption failed
    DecryptionFailed = 21,
    
    /// Record overflow
    RecordOverflow = 22,
    
    /// Decompression failure
    DecompressionFailure = 30,
    
    /// Handshake failure
    HandshakeFailure = 40,
    
    /// No certificate
    NoCertificate = 41,
    
    /// Bad certificate
    BadCertificate = 42,
    
    /// Unsupported certificate
    UnsupportedCertificate = 43,
    
    /// Certificate revoked
    CertificateRevoked = 44,
    
    /// Certificate expired
    CertificateExpired = 45,
    
    /// Certificate unknown
    CertificateUnknown = 46,
    
    /// Illegal parameter
    IllegalParameter = 47,
    
    /// Unknown CA
    UnknownCa = 48,
    
    /// Access denied
    AccessDenied = 49,
    
    /// Decode error
    DecodeError = 50,
    
    /// Decrypt error
    DecryptError = 51,
    
    /// Export restriction
    ExportRestriction = 60,
    
    /// Protocol version
    ProtocolVersion = 70,
    
    /// Insufficient security
    InsufficientSecurity = 71,
    
    /// Internal error
    InternalError = 80,
    
    /// User canceled
    UserCanceled = 90,
    
    /// No renegotiation
    NoRenegotiation = 100,
    
    /// Unsupported extension
    UnsupportedExtension = 110,
    
    /// Invalid alert description
    Invalid = 255,
}

impl From<u8> for AlertDescription {
    fn from(value: u8) -> Self {
        match value {
            0 => AlertDescription::CloseNotify,
            10 => AlertDescription::UnexpectedMessage,
            20 => AlertDescription::BadRecordMac,
            21 => AlertDescription::DecryptionFailed,
            22 => AlertDescription::RecordOverflow,
            30 => AlertDescription::DecompressionFailure,
            40 => AlertDescription::HandshakeFailure,
            41 => AlertDescription::NoCertificate,
            42 => AlertDescription::BadCertificate,
            43 => AlertDescription::UnsupportedCertificate,
            44 => AlertDescription::CertificateRevoked,
            45 => AlertDescription::CertificateExpired,
            46 => AlertDescription::CertificateUnknown,
            47 => AlertDescription::IllegalParameter,
            48 => AlertDescription::UnknownCa,
            49 => AlertDescription::AccessDenied,
            50 => AlertDescription::DecodeError,
            51 => AlertDescription::DecryptError,
            60 => AlertDescription::ExportRestriction,
            70 => AlertDescription::ProtocolVersion,
            71 => AlertDescription::InsufficientSecurity,
            80 => AlertDescription::InternalError,
            90 => AlertDescription::UserCanceled,
            100 => AlertDescription::NoRenegotiation,
            110 => AlertDescription::UnsupportedExtension,
            _ => AlertDescription::Invalid,
        }
    }
}

/// DTLS alert message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alert {
    /// Alert level
    pub level: AlertLevel,
    
    /// Alert description
    pub description: AlertDescription,
}

impl Alert {
    /// Create a new alert message
    pub fn new(level: AlertLevel, description: AlertDescription) -> Self {
        Self {
            level,
            description,
        }
    }
    
    /// Create a close notify alert
    pub fn close_notify() -> Self {
        Self {
            level: AlertLevel::Warning,
            description: AlertDescription::CloseNotify,
        }
    }
    
    /// Check if this is a fatal alert
    pub fn is_fatal(&self) -> bool {
        self.level == AlertLevel::Fatal
    }
    
    /// Serialize the alert to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::with_capacity(2);
        
        buf.put_u8(self.level as u8);
        buf.put_u8(self.description as u8);
        
        Ok(buf.freeze())
    }
    
    /// Parse an alert from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 2 {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        let level = AlertLevel::from(cursor.get_u8());
        let description = AlertDescription::from(cursor.get_u8());
        
        Ok(Self {
            level,
            description,
        })
    }
} 