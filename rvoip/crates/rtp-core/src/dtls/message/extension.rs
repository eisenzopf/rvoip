//! DTLS extension types
//!
//! This module contains the extension types used in DTLS.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use std::io::Cursor;

use crate::dtls::Result;

/// DTLS extension type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ExtensionType {
    /// Server Name Indication
    ServerName = 0,
    
    /// Maximum Fragment Length
    MaxFragmentLength = 1,
    
    /// Client Certificate URL
    ClientCertificateUrl = 2,
    
    /// Trusted CA Keys
    TrustedCaKeys = 3,
    
    /// Truncated HMAC
    TruncatedHmac = 4,
    
    /// Status Request
    StatusRequest = 5,
    
    /// User Mapping
    UserMapping = 6,
    
    /// Client Authentication
    ClientAuthz = 7,
    
    /// Server Authentication
    ServerAuthz = 8,
    
    /// Cert Type
    CertType = 9,
    
    /// Supported Groups
    SupportedGroups = 10,
    
    /// EC Point Formats
    EcPointFormats = 11,
    
    /// SRP
    Srp = 12,
    
    /// Signature Algorithms
    SignatureAlgorithms = 13,
    
    /// Use SRTP (RFC 5764)
    UseSrtp = 14,
    
    /// Heartbeat
    Heartbeat = 15,
    
    /// Application Layer Protocol Negotiation
    Alpn = 16,
    
    /// Signed Certificate Timestamp
    SignedCertificateTimestamp = 18,
    
    /// Client Certificate Type
    ClientCertificateType = 19,
    
    /// Server Certificate Type
    ServerCertificateType = 20,
    
    /// Padding
    Padding = 21,
    
    /// Encrypt-then-MAC
    EncryptThenMac = 22,
    
    /// Extended Master Secret
    ExtendedMasterSecret = 23,
    
    /// Token Binding
    TokenBinding = 24,
    
    /// Cache Info
    CacheInfo = 25,
    
    /// Renegotiation Info
    RenegotiationInfo = 0xff01,
    
    /// Unknown extension type
    Unknown(u16),
}

impl From<u16> for ExtensionType {
    fn from(value: u16) -> Self {
        match value {
            0 => ExtensionType::ServerName,
            1 => ExtensionType::MaxFragmentLength,
            2 => ExtensionType::ClientCertificateUrl,
            3 => ExtensionType::TrustedCaKeys,
            4 => ExtensionType::TruncatedHmac,
            5 => ExtensionType::StatusRequest,
            6 => ExtensionType::UserMapping,
            7 => ExtensionType::ClientAuthz,
            8 => ExtensionType::ServerAuthz,
            9 => ExtensionType::CertType,
            10 => ExtensionType::SupportedGroups,
            11 => ExtensionType::EcPointFormats,
            12 => ExtensionType::Srp,
            13 => ExtensionType::SignatureAlgorithms,
            14 => ExtensionType::UseSrtp,
            15 => ExtensionType::Heartbeat,
            16 => ExtensionType::Alpn,
            18 => ExtensionType::SignedCertificateTimestamp,
            19 => ExtensionType::ClientCertificateType,
            20 => ExtensionType::ServerCertificateType,
            21 => ExtensionType::Padding,
            22 => ExtensionType::EncryptThenMac,
            23 => ExtensionType::ExtendedMasterSecret,
            24 => ExtensionType::TokenBinding,
            25 => ExtensionType::CacheInfo,
            0xff01 => ExtensionType::RenegotiationInfo,
            _ => ExtensionType::Unknown(value),
        }
    }
}

impl From<ExtensionType> for u16 {
    fn from(value: ExtensionType) -> Self {
        match value {
            ExtensionType::ServerName => 0,
            ExtensionType::MaxFragmentLength => 1,
            ExtensionType::ClientCertificateUrl => 2,
            ExtensionType::TrustedCaKeys => 3,
            ExtensionType::TruncatedHmac => 4,
            ExtensionType::StatusRequest => 5,
            ExtensionType::UserMapping => 6,
            ExtensionType::ClientAuthz => 7,
            ExtensionType::ServerAuthz => 8,
            ExtensionType::CertType => 9,
            ExtensionType::SupportedGroups => 10,
            ExtensionType::EcPointFormats => 11,
            ExtensionType::Srp => 12,
            ExtensionType::SignatureAlgorithms => 13,
            ExtensionType::UseSrtp => 14,
            ExtensionType::Heartbeat => 15,
            ExtensionType::Alpn => 16,
            ExtensionType::SignedCertificateTimestamp => 18,
            ExtensionType::ClientCertificateType => 19,
            ExtensionType::ServerCertificateType => 20,
            ExtensionType::Padding => 21,
            ExtensionType::EncryptThenMac => 22,
            ExtensionType::ExtendedMasterSecret => 23,
            ExtensionType::TokenBinding => 24,
            ExtensionType::CacheInfo => 25,
            ExtensionType::RenegotiationInfo => 0xff01,
            ExtensionType::Unknown(value) => value,
        }
    }
}

/// DTLS extension
#[derive(Debug, Clone)]
pub enum Extension {
    /// Use SRTP extension (RFC 5764)
    UseSrtp(UseSrtpExtension),
    
    /// Unknown extension
    Unknown {
        /// Extension type
        typ: u16,
        
        /// Extension data
        data: Bytes,
    },
}

impl Extension {
    /// Get the extension type
    pub fn extension_type(&self) -> ExtensionType {
        match self {
            Self::UseSrtp(_) => ExtensionType::UseSrtp,
            Self::Unknown { typ, .. } => ExtensionType::from(*typ),
        }
    }
    
    /// Serialize the extension to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut buf = BytesMut::new();
        
        // Extension type (2 bytes)
        let typ: u16 = self.extension_type().into();
        buf.put_u16(typ);
        
        // Extension data
        match self {
            Self::UseSrtp(ext) => {
                let data = ext.serialize()?;
                
                // Extension length (2 bytes)
                buf.put_u16(data.len() as u16);
                
                // Extension data
                buf.extend_from_slice(&data);
            }
            Self::Unknown { data, .. } => {
                // Extension length (2 bytes)
                buf.put_u16(data.len() as u16);
                
                // Extension data
                buf.extend_from_slice(data);
            }
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse an extension from bytes
    pub fn parse(data: &[u8]) -> Result<(Self, usize)> {
        if data.len() < 4 {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Extension type (2 bytes)
        let typ = cursor.get_u16();
        
        // Extension length (2 bytes)
        let length = cursor.get_u16() as usize;
        
        if data.len() < 4 + length {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let ext_data = &data[4..4 + length];
        
        let extension = match ExtensionType::from(typ) {
            ExtensionType::UseSrtp => {
                let use_srtp = UseSrtpExtension::parse(ext_data)?;
                Extension::UseSrtp(use_srtp)
            }
            _ => Extension::Unknown {
                typ,
                data: Bytes::copy_from_slice(ext_data),
            },
        };
        
        Ok((extension, 4 + length))
    }
}

/// SRTP protection profile identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SrtpProtectionProfile {
    /// SRTP_AES128_CM_HMAC_SHA1_80 (RFC 5764)
    Aes128CmSha1_80 = 0x0001,
    
    /// SRTP_AES128_CM_HMAC_SHA1_32 (RFC 5764)
    Aes128CmSha1_32 = 0x0002,
    
    /// SRTP_AEAD_AES_128_GCM (RFC 7714)
    AeadAes128Gcm = 0x0007,
    
    /// SRTP_AEAD_AES_256_GCM (RFC 7714)
    AeadAes256Gcm = 0x0008,
    
    /// Unknown profile
    Unknown(u16),
}

impl From<u16> for SrtpProtectionProfile {
    fn from(value: u16) -> Self {
        match value {
            0x0001 => SrtpProtectionProfile::Aes128CmSha1_80,
            0x0002 => SrtpProtectionProfile::Aes128CmSha1_32,
            0x0007 => SrtpProtectionProfile::AeadAes128Gcm,
            0x0008 => SrtpProtectionProfile::AeadAes256Gcm,
            _ => SrtpProtectionProfile::Unknown(value),
        }
    }
}

impl From<SrtpProtectionProfile> for u16 {
    fn from(value: SrtpProtectionProfile) -> Self {
        match value {
            SrtpProtectionProfile::Aes128CmSha1_80 => 0x0001,
            SrtpProtectionProfile::Aes128CmSha1_32 => 0x0002,
            SrtpProtectionProfile::AeadAes128Gcm => 0x0007,
            SrtpProtectionProfile::AeadAes256Gcm => 0x0008,
            SrtpProtectionProfile::Unknown(value) => value,
        }
    }
}

/// Use SRTP extension (RFC 5764)
#[derive(Debug, Clone)]
pub struct UseSrtpExtension {
    /// SRTP protection profiles
    pub profiles: Vec<SrtpProtectionProfile>,
    
    /// MKI (Master Key Identifier) value
    pub mki: Bytes,
}

impl UseSrtpExtension {
    /// Create a new Use SRTP extension
    pub fn new(profiles: Vec<SrtpProtectionProfile>, mki: Bytes) -> Self {
        Self {
            profiles,
            mki,
        }
    }
    
    /// Create a new Use SRTP extension with no MKI
    pub fn with_profiles(profiles: Vec<SrtpProtectionProfile>) -> Self {
        Self {
            profiles,
            mki: Bytes::new(),
        }
    }
    
    /// Serialize the extension to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        // Calculate profiles length (2 bytes per profile)
        let profiles_len = self.profiles.len() * 2;
        
        // Calculate total length
        let total_len = 2 + profiles_len + 1 + self.mki.len();
        
        let mut buf = BytesMut::with_capacity(total_len);
        
        // Profiles length (2 bytes)
        buf.put_u16(profiles_len as u16);
        
        // Profiles
        for profile in &self.profiles {
            buf.put_u16((*profile).into());
        }
        
        // MKI length (1 byte)
        buf.put_u8(self.mki.len() as u8);
        
        // MKI value
        if !self.mki.is_empty() {
            buf.extend_from_slice(&self.mki);
        }
        
        Ok(buf.freeze())
    }
    
    /// Parse a Use SRTP extension from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 3 {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Profiles length (2 bytes)
        let profiles_len = cursor.get_u16() as usize;
        
        if profiles_len % 2 != 0 {
            return Err(crate::error::Error::InvalidPacket(
                "SRTP profiles length must be a multiple of 2".to_string()
            ));
        }
        
        if data.len() < 3 + profiles_len {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        // Profiles
        let mut profiles = Vec::with_capacity(profiles_len / 2);
        for _ in 0..(profiles_len / 2) {
            let profile_id = cursor.get_u16();
            profiles.push(SrtpProtectionProfile::from(profile_id));
        }
        
        // MKI length (1 byte)
        let mki_len = cursor.get_u8() as usize;
        
        if data.len() < 3 + profiles_len + mki_len {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        // MKI value
        let mki = if mki_len > 0 {
            let offset = 3 + profiles_len;
            Bytes::copy_from_slice(&data[offset..offset + mki_len])
        } else {
            Bytes::new()
        };
        
        Ok(Self {
            profiles,
            mki,
        })
    }
} 