//! Reason header as defined in RFC 3326
//!
//! The Reason header field provides information on why a SIP request was issued,
//! often conveying details about a call failure or a redirection reason.
//!
//! # Format
//!
//! ```text
//! Reason: protocol ;cause=code ;text="comment"
//! ```
//!
//! # Examples
//!
//! ```text
//! Reason: SIP ;cause=200 ;text="Call completed elsewhere"
//! Reason: Q.850 ;cause=16 ;text="Terminated"
//! ```

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::types::headers::{Header, HeaderName, TypedHeaderTrait};
use serde::{Serialize, Deserialize};
use crate::types::headers::HeaderValue;

/// The Reason header as defined in RFC 3326.
/// 
/// This header is used to indicate why a particular SIP request was generated,
/// particularly for requests like BYE and CANCEL that terminate dialogs or
/// transactions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    /// The protocol causing the event (e.g., "SIP", "Q.850")
    protocol: String,
    
    /// The protocol cause value
    cause: u16,
    
    /// Optional human-readable text explaining the reason
    text: Option<String>,
}

impl Reason {
    /// Create a new Reason header with the given protocol, cause, and optional text
    ///
    /// # Arguments
    ///
    /// * `protocol` - The protocol identifier (e.g., "SIP", "Q.850")
    /// * `cause` - The cause code (e.g., 200, 486, 603)
    /// * `text` - Optional text explaining the reason
    ///
    /// # Returns
    ///
    /// A new Reason header
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::types::reason::Reason;
    ///
    /// let reason = Reason::new("SIP", 486, Some("Busy Here"));
    /// ```
    pub fn new(protocol: impl Into<String>, cause: u16, text: Option<impl Into<String>>) -> Self {
        Self {
            protocol: protocol.into(),
            cause,
            text: text.map(Into::into),
        }
    }
    
    /// Get the protocol value
    ///
    /// # Returns
    ///
    /// The protocol string
    pub fn protocol(&self) -> &str {
        &self.protocol
    }
    
    /// Get the cause code
    ///
    /// # Returns
    ///
    /// The cause code
    pub fn cause(&self) -> u16 {
        self.cause
    }
    
    /// Get the text value, if any
    ///
    /// # Returns
    ///
    /// The text, if present
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }
    
    /// Set the text value
    ///
    /// # Arguments
    ///
    /// * `text` - The new text value
    pub fn set_text(&mut self, text: Option<impl Into<String>>) {
        self.text = text.map(Into::into);
    }
    
    /// Set the cause code
    ///
    /// # Arguments
    ///
    /// * `cause` - The new cause code
    pub fn set_cause(&mut self, cause: u16) {
        self.cause = cause;
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ;cause={}", self.protocol, self.cause)?;
        
        if let Some(text) = &self.text {
            write!(f, " ;text=\"{}\"", text)?;
        }
        
        Ok(())
    }
}

impl FromStr for Reason {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        
        // Split by semicolons to get the parts
        let parts: Vec<&str> = s.split(';').map(|p| p.trim()).collect();
        
        if parts.is_empty() {
            return Err(Error::ParseError("Empty Reason header".to_string()));
        }
        
        // First part is the protocol
        let protocol = parts[0].to_string();
        
        // Parse cause and text from the parameters
        let mut cause: Option<u16> = None;
        let mut text: Option<String> = None;
        
        for part in &parts[1..] {
            if let Some(param) = part.strip_prefix("cause=") {
                match param.parse::<u16>() {
                    Ok(c) => cause = Some(c),
                    Err(_) => return Err(Error::ParseError(format!("Invalid cause value: {}", param))),
                }
            } else if let Some(param) = part.strip_prefix("text=") {
                let param = param.trim();
                if param.starts_with('"') && param.ends_with('"') && param.len() >= 2 {
                    text = Some(param[1..param.len()-1].to_string());
                } else {
                    text = Some(param.to_string());
                }
            }
            // Ignore other parameters
        }
        
        match cause {
            Some(c) => Ok(Reason {
                protocol,
                cause: c,
                text,
            }),
            None => Err(Error::ParseError("Missing required 'cause' parameter".to_string())),
        }
    }
}

impl TypedHeaderTrait for Reason {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::Reason
    }
    
    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::text(&self.to_string()))
    }
    
    fn from_header(header: &Header) -> Result<Self> {
        if header.name != HeaderName::Reason {
            return Err(Error::InvalidHeader(format!(
                "Expected Reason header, got {:?}",
                header.name
            )));
        }
        
        let value = header.value.to_string();
        Reason::from_str(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_reason_new() {
        let reason = Reason::new("SIP", 486, Some("Busy Here"));
        assert_eq!(reason.protocol(), "SIP");
        assert_eq!(reason.cause(), 486);
        assert_eq!(reason.text(), Some("Busy Here"));
    }
    
    #[test]
    fn test_reason_display() {
        let reason = Reason::new("SIP", 486, Some("Busy Here"));
        assert_eq!(reason.to_string(), "SIP ;cause=486 ;text=\"Busy Here\"");
        
        let reason = Reason::new("Q.850", 16, None::<String>);
        assert_eq!(reason.to_string(), "Q.850 ;cause=16");
    }
    
    #[test]
    fn test_reason_from_str() {
        let reason = Reason::from_str("SIP ;cause=486 ;text=\"Busy Here\"").unwrap();
        assert_eq!(reason.protocol(), "SIP");
        assert_eq!(reason.cause(), 486);
        assert_eq!(reason.text(), Some("Busy Here"));
        
        let reason = Reason::from_str("Q.850;cause=16").unwrap();
        assert_eq!(reason.protocol(), "Q.850");
        assert_eq!(reason.cause(), 16);
        assert_eq!(reason.text(), None);
    }
    
    #[test]
    fn test_reason_header_conversion() {
        let reason = Reason::new("SIP", 486, Some("Busy Here"));
        let header = reason.to_header();
        assert_eq!(header.name, HeaderName::Reason);
        assert_eq!(header.value.to_string(), "SIP ;cause=486 ;text=\"Busy Here\"");
        
        let reason2 = Reason::from_header(&header).unwrap();
        assert_eq!(reason, reason2);
    }
} 