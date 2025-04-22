// MediaType representation for SIP Content-Type and Accept headers
// Format is type/subtype;parameter=value

use std::collections::HashMap;
use std::fmt;
use serde::{Serialize, Deserialize};

/// MediaType represents a MIME media type with optional parameters
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaType {
    /// Main type (e.g., "application", "text", "audio")
    pub typ: String,
    
    /// Subtype (e.g., "sdp", "plain", "sip")
    pub subtype: String,
    
    /// Optional parameters (e.g., charset=utf-8)
    pub parameters: HashMap<String, String>,
}

impl MediaType {
    /// Create a new MediaType without parameters
    pub fn new(typ: &str, subtype: &str) -> Self {
        MediaType {
            typ: typ.to_lowercase(),
            subtype: subtype.to_lowercase(),
            parameters: HashMap::new(),
        }
    }

    /// Create a new MediaType with parameters
    pub fn with_params(typ: &str, subtype: &str, parameters: HashMap<String, String>) -> Self {
        MediaType {
            typ: typ.to_lowercase(),
            subtype: subtype.to_lowercase(),
            parameters,
        }
    }

    /// Add a parameter to the MediaType
    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.parameters.insert(name.to_lowercase(), value.to_string());
        self
    }

    /// Check if this media type matches another (ignoring parameters)
    pub fn matches_type(&self, other: &MediaType) -> bool {
        (self.typ == "*" || other.typ == "*" || self.typ == other.typ) &&
        (self.subtype == "*" || other.subtype == "*" || self.subtype == other.subtype)
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.typ, self.subtype)?;
        
        // Add parameters if any exist
        for (name, value) in &self.parameters {
            write!(f, ";{}={}", name, value)?;
        }
        
        Ok(())
    }
}

// Common media types used in SIP
impl MediaType {
    /// application/sdp - Session Description Protocol
    pub fn sdp() -> Self {
        MediaType::new("application", "sdp")
    }
    
    /// application/sip - SIP message
    pub fn sip() -> Self {
        MediaType::new("application", "sip")
    }
    
    /// text/plain - Plain text
    pub fn text_plain() -> Self {
        MediaType::new("text", "plain")
    }
    
    /// multipart/mixed - Multipart message with mixed content
    pub fn multipart_mixed() -> Self {
        MediaType::new("multipart", "mixed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_media_type_display() {
        let mt = MediaType::new("application", "sdp");
        assert_eq!(mt.to_string(), "application/sdp");
        
        let mt_with_params = MediaType::new("text", "plain")
            .with_param("charset", "utf-8");
        assert_eq!(mt_with_params.to_string(), "text/plain;charset=utf-8");
    }
    
    #[test]
    fn test_media_type_matching() {
        let mt1 = MediaType::new("application", "sdp");
        let mt2 = MediaType::new("application", "sdp");
        assert!(mt1.matches_type(&mt2));
        
        let mt3 = MediaType::new("application", "*");
        assert!(mt1.matches_type(&mt3));
        
        let mt4 = MediaType::new("*", "*");
        assert!(mt1.matches_type(&mt4));
        
        let mt5 = MediaType::new("text", "plain");
        assert!(!mt1.matches_type(&mt5));
    }
    
    #[test]
    fn test_common_media_types() {
        assert_eq!(MediaType::sdp().to_string(), "application/sdp");
        assert_eq!(MediaType::text_plain().to_string(), "text/plain");
    }
} 