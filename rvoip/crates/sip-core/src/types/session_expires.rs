//! # Session-Expires Header
//!
//! This module provides the types for the Session-Expires header as defined in
//! [RFC 4028](https://datatracker.ietf.org/doc/html/rfc4028).
//!
//! The Session-Expires header field conveys the session interval for a SIP session.
//! It is placed only in INVITE or UPDATE requests, as well as in 2xx responses to an
//! INVITE or UPDATE. The session interval is the maximum amount of time that can occur
//! between session refresh requests in a dialog before the session will be considered timed out.
//!
//! This header defines the duration of the session as well as which party is responsible
//! for sending the refresh requests.
//!
//! ## Format
//!
//! ```text
//! Session-Expires: 3600;refresher=uac
//! ```
//!
//! ## Examples
//!
//! ```
//! use rvoip_sip_core::types::session_expires::{SessionExpires, Refresher};
//! use rvoip_sip_core::types::Param;
//! use std::str::FromStr;
//!
//! // Create a Session-Expires header with a 3600-second timeout and UAC refresher
//! let session_expires = SessionExpires::new(3600, Some(Refresher::Uac));
//! assert_eq!(session_expires.delta_seconds, 3600);
//! assert_eq!(session_expires.refresher, Some(Refresher::Uac));
//!
//! // Create from string
//! let session_expires = SessionExpires::from_str("3600;refresher=uas").unwrap();
//! assert_eq!(session_expires.delta_seconds, 3600);
//! assert_eq!(session_expires.refresher, Some(Refresher::Uas));
//! ```

use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};

use crate::error::{Error, Result};
use crate::parser::headers::session_expires::parse_session_expires;
use crate::types::TypedHeaderTrait;
use crate::types::headers::header_name::HeaderName;
use crate::types::headers::header::Header;
use crate::types::headers::header_value::HeaderValue;
use crate::types::param::Param;
use std::str;

/// Refresher entity for Session-Expires header
///
/// Indicates which entity is responsible for refreshing the session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refresher {
    /// User Agent Client refreshes the session
    Uac,
    /// User Agent Server refreshes the session
    Uas,
}

impl fmt::Display for Refresher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refresher::Uac => write!(f, "uac"),
            Refresher::Uas => write!(f, "uas"),
        }
    }
}

/// Session-Expires header as defined in RFC 4028
///
/// The Session-Expires header field conveys the session interval for a SIP session.
/// It is placed only in INVITE or UPDATE requests, as well as in 2xx responses to
/// those requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExpires {
    /// Session interval in seconds
    pub delta_seconds: u32,
    /// Entity responsible for session refresh
    pub refresher: Option<Refresher>,
    /// Additional parameters
    pub params: Vec<Param>,
}

impl SessionExpires {
    /// Create a new SessionExpires header
    pub fn new(delta_seconds: u32, refresher: Option<Refresher>) -> Self {
        SessionExpires {
            delta_seconds,
            refresher,
            params: Vec::new(),
        }
    }

    /// Create a new SessionExpires with additional parameters
    pub fn new_with_params(delta_seconds: u32, refresher: Option<Refresher>, params: Vec<Param>) -> Self {
        SessionExpires {
            delta_seconds,
            refresher,
            params,
        }
    }

    /// Returns the session interval in seconds.
    pub fn delta_seconds(&self) -> u32 {
        self.delta_seconds
    }

    /// Returns the entity responsible for session refresh, if specified.
    pub fn refresher(&self) -> Option<Refresher> {
        self.refresher
    }

    /// Returns a slice of additional parameters.
    pub fn params(&self) -> &[Param] {
        &self.params
    }

    /// Checks if a parameter with the given key exists (case-insensitive).
    pub fn has_param(&self, key: &str) -> bool {
        self.params.iter().any(|param| param.key().eq_ignore_ascii_case(key))
    }

    /// Gets the value of a parameter with the given key (case-insensitive).
    /// Returns Some(Some(value)) if param exists with value,
    /// Some(None) if param exists but is valueless,
    /// None if param does not exist.
    pub fn get_param(&self, key: &str) -> Option<Option<String>> {
        self.params.iter()
            .find(|param| param.key().eq_ignore_ascii_case(key))
            .map(|param| param.value())
    }
}

impl FromStr for SessionExpires {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let bytes = s.as_bytes();
        match parse_session_expires(bytes) {
            Ok((rem, (delta_seconds, refresher, params))) => {
                if !rem.is_empty() {
                    let remainder_str = str::from_utf8(rem).unwrap_or("[invalid UTF-8]");
                    return Err(Error::ParseError(format!(
                        "Trailing characters after parsing Session-Expires: \"{}\"",
                        remainder_str
                    )));
                }
                Ok(SessionExpires {
                    delta_seconds,
                    refresher,
                    params,
                })
            },
            Err(e) => Err(Error::ParseError(format!("Failed to parse Session-Expires: {:?}", e))),
        }
    }
}

impl TypedHeaderTrait for SessionExpires {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::SessionExpires
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(&self.to_string()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        // Check if the header name is either "Session-Expires" or its compact form "x"
        let compact_name = HeaderName::from_str("x").unwrap_or_else(|_| HeaderName::Other("x".to_string())); // Fallback for safety, though unwrap should be fine if "x" is added
        if header.name != HeaderName::SessionExpires && header.name != compact_name {
            return Err(Error::ParseError(format!("Expected Session-Expires header (or compact 'x'), got {:?}", header.name)));
        }

        Self::from_str(header.value.to_string().as_str())
    }
}

impl fmt::Display for SessionExpires {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Format the delta-seconds
        write!(f, "{}", self.delta_seconds)?;

        // Add refresher if present
        if let Some(refresher) = &self.refresher {
            write!(f, ";refresher={}", refresher)?;
        }

        // Add other parameters
        for param in &self.params {
            write!(f, ";{}", param)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::headers::typed_header::TypedHeader;

    #[test]
    fn test_session_expires_new() {
        let se = SessionExpires::new(3600, Some(Refresher::Uac));
        assert_eq!(se.delta_seconds, 3600);
        assert_eq!(se.refresher, Some(Refresher::Uac));
        assert!(se.params.is_empty());
    }

    #[test]
    fn test_session_expires_from_str() {
        let se = SessionExpires::from_str("3600").unwrap();
        assert_eq!(se.delta_seconds, 3600);
        assert_eq!(se.refresher, None);
        assert!(se.params.is_empty());

        let se = SessionExpires::from_str("3600;refresher=uac").unwrap();
        assert_eq!(se.delta_seconds, 3600);
        assert_eq!(se.refresher, Some(Refresher::Uac));
        assert!(se.params.is_empty());

        let se = SessionExpires::from_str("1800;refresher=uas;custom=value").unwrap();
        assert_eq!(se.delta_seconds, 1800);
        assert_eq!(se.refresher, Some(Refresher::Uas));
        assert_eq!(se.params.len(), 1);
        assert_eq!(se.params[0].key(), "custom");
        assert_eq!(se.params[0].value().map(|s| s.to_owned()), Some("value".to_string()));
    }

    #[test]
    fn test_session_expires_to_string() {
        let se = SessionExpires::new(3600, None);
        assert_eq!(se.to_string(), "3600");

        let se = SessionExpires::new(3600, Some(Refresher::Uac));
        assert_eq!(se.to_string(), "3600;refresher=uac");

        let mut se = SessionExpires::new(1800, Some(Refresher::Uas));
        se.params.push(Param::new("custom", Some("value".to_string())));
        assert_eq!(se.to_string(), "1800;refresher=uas;custom=value");
    }

    #[test]
    fn test_session_expires_header_name() {
        assert_eq!(SessionExpires::header_name(), HeaderName::SessionExpires);
        assert_eq!(SessionExpires::header_name().as_str(), "Session-Expires");
    }
    
    #[test]
    fn test_session_expires_to_header() {
        let se = SessionExpires::new(3600, Some(Refresher::Uac));
        let header = se.to_header();
        assert_eq!(header.name, HeaderName::SessionExpires);
        assert_eq!(header.value.to_string(), "3600;refresher=uac");
    }
    
    #[test]
    fn test_session_expires_from_header() {
        let header = Header::text(
            HeaderName::SessionExpires, // Changed from Other(...)
            "3600;refresher=uac"
        );
        
        let se = SessionExpires::from_header(&header).unwrap();
        assert_eq!(se.delta_seconds, 3600);
        assert_eq!(se.refresher, Some(Refresher::Uac));
        assert!(se.params.is_empty());
    }
    
    #[test]
    fn test_session_expires_from_compact_header() {
        let header = Header::text(
            HeaderName::from_str("x").unwrap(), // More robust way to get compact form
            "1800;refresher=uas"
        );
        
        let se = SessionExpires::from_header(&header).unwrap();
        assert_eq!(se.delta_seconds, 1800);
        assert_eq!(se.refresher, Some(Refresher::Uas));
        assert!(se.params.is_empty());
    }

    #[test]
    fn test_session_expires_invalid_input() {
        let result = SessionExpires::from_str("invalid");
        assert!(result.is_err());
        
        let result = SessionExpires::from_str("3600;refresher=invalid");
        assert!(result.is_err());
    }
} 