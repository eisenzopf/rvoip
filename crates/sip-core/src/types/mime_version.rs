//! # SIP MIME-Version Header
//!
//! This module provides an implementation of the SIP MIME-Version header as defined in
//! [RFC 3261 Section 20.24](https://datatracker.ietf.org/doc/html/rfc3261#section-20.24).
//!
//! The MIME-Version header field indicates the version of the MIME protocol used to
//! construct the message. The syntax for the field is identical to the HTTP 1.1
//! MIME-Version field described in [RFC 2616 Section 14.23](https://datatracker.ietf.org/doc/html/rfc2616#section-14.23).
//!
//! This header field is often included in messages with multipart message bodies.
//!
//! ## Format
//!
//! ```text
//! MIME-Version: 1.0
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::MimeVersion;
//! use std::str::FromStr;
//!
//! // Create a MIME-Version header
//! let mime_version = MimeVersion::new(1, 0);
//! assert_eq!(mime_version.major(), 1);
//! assert_eq!(mime_version.minor(), 0);
//!
//! // Parse from a string
//! let mime_version = MimeVersion::from_str("1.0").unwrap();
//! assert_eq!(mime_version.to_string(), "1.0");
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the MIME-Version header field (RFC 3261 Section 20.24).
///
/// The MIME-Version header field indicates the version of the MIME protocol used to
/// construct the message. It consists of a major and minor version number.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::MimeVersion;
/// use std::str::FromStr;
///
/// // Create a MIME-Version header
/// let mime_version = MimeVersion::new(1, 0);
/// assert_eq!(mime_version.major(), 1);
/// assert_eq!(mime_version.minor(), 0);
///
/// // Convert to a string
/// assert_eq!(mime_version.to_string(), "1.0");
///
/// // Parse from a string
/// let mime_version = MimeVersion::from_str("1.0").unwrap();
/// assert_eq!(mime_version.major(), 1);
/// assert_eq!(mime_version.minor(), 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MimeVersion {
    /// Major version number
    major: u32,
    /// Minor version number
    minor: u32,
}

impl MimeVersion {
    /// Creates a new MIME-Version header with the specified version numbers.
    ///
    /// # Parameters
    ///
    /// - `major`: The major version number
    /// - `minor`: The minor version number
    ///
    /// # Returns
    ///
    /// A new `MimeVersion` instance with the specified version numbers
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::MimeVersion;
    ///
    /// let mime_version = MimeVersion::new(1, 0);
    /// assert_eq!(mime_version.major(), 1);
    /// assert_eq!(mime_version.minor(), 0);
    /// ```
    pub fn new(major: u32, minor: u32) -> Self {
        MimeVersion { major, minor }
    }

    /// Creates a new MIME-Version header with version 1.0.
    ///
    /// # Returns
    ///
    /// A new `MimeVersion` instance with version 1.0
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::MimeVersion;
    ///
    /// let mime_version = MimeVersion::v1_0();
    /// assert_eq!(mime_version.major(), 1);
    /// assert_eq!(mime_version.minor(), 0);
    /// ```
    pub fn v1_0() -> Self {
        MimeVersion { major: 1, minor: 0 }
    }

    /// Returns the major version number.
    ///
    /// # Returns
    ///
    /// The major version number
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::MimeVersion;
    ///
    /// let mime_version = MimeVersion::new(1, 0);
    /// assert_eq!(mime_version.major(), 1);
    /// ```
    pub fn major(&self) -> u32 {
        self.major
    }

    /// Returns the minor version number.
    ///
    /// # Returns
    ///
    /// The minor version number
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::MimeVersion;
    ///
    /// let mime_version = MimeVersion::new(1, 0);
    /// assert_eq!(mime_version.minor(), 0);
    /// ```
    pub fn minor(&self) -> u32 {
        self.minor
    }
}

impl Default for MimeVersion {
    fn default() -> Self {
        MimeVersion::v1_0()
    }
}

impl fmt::Display for MimeVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for MimeVersion {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') {
            // Strip the "MIME-Version:" prefix
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid MIME-Version header format".to_string()));
            }
            parts[1].trim()
        } else {
            s.trim()
        };
        
        // Parse the version number in format "major.minor"
        let version_parts: Vec<&str> = value_str.split('.').collect();
        if version_parts.len() != 2 {
            return Err(Error::ParseError("Invalid MIME-Version format, expected major.minor".to_string()));
        }
        
        let major = version_parts[0].trim().parse::<u32>()
            .map_err(|e| Error::ParseError(format!("Invalid MIME-Version major: {}", e)))?;
            
        let minor = version_parts[1].trim().parse::<u32>()
            .map_err(|e| Error::ParseError(format!("Invalid MIME-Version minor: {}", e)))?;
            
        Ok(MimeVersion { major, minor })
    }
}

// Implement TypedHeaderTrait for MimeVersion
impl TypedHeaderTrait for MimeVersion {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::MimeVersion
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    MimeVersion::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name().as_str())
                    ))
                }
            },
            HeaderValue::MimeVersion((major, minor)) => {
                Ok(MimeVersion::new(*major as u32, *minor as u32))
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}: {:?}", Self::header_name().as_str(), header.value)
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let mime_version = MimeVersion::new(1, 0);
        assert_eq!(mime_version.major(), 1);
        assert_eq!(mime_version.minor(), 0);
    }
    
    #[test]
    fn test_v1_0() {
        let mime_version = MimeVersion::v1_0();
        assert_eq!(mime_version.major(), 1);
        assert_eq!(mime_version.minor(), 0);
    }
    
    #[test]
    fn test_default() {
        let mime_version = MimeVersion::default();
        assert_eq!(mime_version.major(), 1);
        assert_eq!(mime_version.minor(), 0);
    }
    
    #[test]
    fn test_display() {
        let mime_version = MimeVersion::new(1, 0);
        assert_eq!(mime_version.to_string(), "1.0");
        
        let mime_version = MimeVersion::new(2, 1);
        assert_eq!(mime_version.to_string(), "2.1");
    }
    
    #[test]
    fn test_from_str() {
        // Simple case
        let mime_version: MimeVersion = "1.0".parse().unwrap();
        assert_eq!(mime_version.major(), 1);
        assert_eq!(mime_version.minor(), 0);
        
        // With header name
        let mime_version: MimeVersion = "MIME-Version: 1.0".parse().unwrap();
        assert_eq!(mime_version.major(), 1);
        assert_eq!(mime_version.minor(), 0);
        
        // With spaces
        let mime_version: MimeVersion = "  1 . 0  ".parse().unwrap();
        assert_eq!(mime_version.major(), 1);
        assert_eq!(mime_version.minor(), 0);
        
        // Invalid format
        let result: Result<MimeVersion> = "1".parse();
        assert!(result.is_err());
        
        let result: Result<MimeVersion> = "1.0.0".parse();
        assert!(result.is_err());
        
        let result: Result<MimeVersion> = "not_a_number".parse();
        assert!(result.is_err());
    }
    
    #[test]
    fn test_typed_header_trait() {
        let version = MimeVersion::new(1, 0);
        let header = version.to_header();

        assert_eq!(header.name, HeaderName::MimeVersion);
        assert_eq!(header.value.as_text().unwrap(), "1.0");

        // Test parsing from a Header with the correct MimeVersion variant if possible
        // HeaderValue::MimeVersion stores (u8, u8)
        let direct_header_value = HeaderValue::MimeVersion((1, 0));
        let direct_header = Header::new(HeaderName::MimeVersion, direct_header_value);
        let parsed_direct = MimeVersion::from_header(&direct_header).unwrap();
        assert_eq!(parsed_direct, MimeVersion::new(1, 0));

        // Test parsing from Raw
        let raw_header_value = HeaderValue::Raw(b"1.0".to_vec());
        let raw_header = Header::new(HeaderName::MimeVersion, raw_header_value);
        let parsed_from_raw = MimeVersion::from_header(&raw_header).unwrap();
        assert_eq!(parsed_from_raw, version);

        // Example from previous error log (line 312)
        let version_header = Header::new(HeaderName::MimeVersion, HeaderValue::MimeVersion((1,0)));
        let parsed_version_header = MimeVersion::from_header(&version_header).unwrap();
        assert_eq!(parsed_version_header, MimeVersion::new(1,0));

        let invalid_header = Header::new(HeaderName::ContentType, HeaderValue::text("text/plain"));
        assert!(MimeVersion::from_header(&invalid_header).is_err());
    }
} 