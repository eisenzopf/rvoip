//! # SIP Media Type
//!
//! This module provides an implementation of MIME media types used in SIP headers such as
//! Content-Type and Accept. Media types follow the format defined in
//! [RFC 2046](https://datatracker.ietf.org/doc/html/rfc2046) and
//! [RFC 3261 Section 20](https://datatracker.ietf.org/doc/html/rfc3261#section-20).
//!
//! A media type consists of a type and subtype, optionally followed by parameters:
//!
//! ```
//! // Example format:
//! // type/subtype;parameter=value;another=parameter
//! ```
//!
//! ## Common Media Types in SIP
//!
//! SIP commonly uses several media types for different content:
//!
//! - `application/sdp`: Session Description Protocol bodies (SDP)
//! - `application/sip`: Embedded SIP messages
//! - `text/plain`: Plain text bodies
//! - `multipart/mixed`: Multiple body parts with different content types
//!
//! ## Headers Using Media Types
//!
//! In SIP, media types are used in several headers:
//!
//! - **Content-Type**: Indicates the media type of the message body
//! - **Accept**: Indicates media types acceptable for the response
//! - **Accept-Encoding**: Acceptable encodings for the response body
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a media type for SDP
//! let sdp = MediaType::new("application", "sdp");
//! assert_eq!(sdp.to_string(), "application/sdp");
//!
//! // Create a media type with parameters
//! let text = MediaType::new("text", "plain")
//!     .with_param("charset", "utf-8");
//! assert_eq!(text.to_string(), "text/plain;charset=utf-8");
//!
//! // Use built-in common types
//! let sdp = MediaType::sdp();
//! ```

// MediaType representation for SIP Content-Type and Accept headers
// Format is type/subtype;parameter=value

use std::collections::HashMap;
use std::fmt;
use serde::{Serialize, Deserialize};

/// MediaType represents a MIME media type with optional parameters
///
/// A media type (also known as MIME type or content type) identifies the format of a message body
/// in SIP communications. It consists of a type, subtype, and optional parameters that provide
/// additional metadata about the content.
///
/// Media types follow the format `type/subtype;param=value` where:
/// - `type` is a general category (e.g., "application", "text", "audio")
/// - `subtype` is a specific format (e.g., "sdp", "plain", "xml")
/// - parameters provide additional specifications (e.g., "charset=utf-8")
///
/// In SIP, media types are used in Content-Type headers to describe message bodies,
/// and in Accept headers to indicate which media types are acceptable in responses.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::collections::HashMap;
///
/// // Create a basic media type
/// let mt = MediaType::new("application", "sdp");
/// assert_eq!(mt.to_string(), "application/sdp");
///
/// // Create with parameters
/// let mut params = HashMap::new();
/// params.insert("charset".to_string(), "utf-8".to_string());
/// let mt = MediaType::with_params("text", "plain", params);
/// assert_eq!(mt.to_string(), "text/plain;charset=utf-8");
///
/// // Use builder pattern to add parameters
/// let mt = MediaType::new("application", "json")
///     .with_param("charset", "utf-8");
/// assert_eq!(mt.to_string(), "application/json;charset=utf-8");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    ///
    /// This method creates a new media type with the specified type and subtype,
    /// and no parameters. The type and subtype are converted to lowercase
    /// as per the standard requirements for media types.
    ///
    /// # Parameters
    ///
    /// - `typ`: The main type (e.g., "application", "text", "audio")
    /// - `subtype`: The specific format subtype (e.g., "sdp", "plain", "xml")
    ///
    /// # Returns
    ///
    /// A new `MediaType` instance with the specified type and subtype
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a media type for SDP
    /// let sdp = MediaType::new("application", "sdp");
    /// assert_eq!(sdp.typ, "application");
    /// assert_eq!(sdp.subtype, "sdp");
    /// assert!(sdp.parameters.is_empty());
    ///
    /// // Type and subtype are normalized to lowercase
    /// let txt = MediaType::new("TEXT", "PLAIN");
    /// assert_eq!(txt.typ, "text");
    /// assert_eq!(txt.subtype, "plain");
    /// ```
    pub fn new(typ: &str, subtype: &str) -> Self {
        MediaType {
            typ: typ.to_lowercase(),
            subtype: subtype.to_lowercase(),
            parameters: HashMap::new(),
        }
    }

    /// Create a new MediaType with parameters
    ///
    /// This method creates a new media type with the specified type, subtype,
    /// and a hashmap of parameter name-value pairs.
    ///
    /// # Parameters
    ///
    /// - `typ`: The main type (e.g., "application", "text", "audio")
    /// - `subtype`: The specific format subtype (e.g., "sdp", "plain", "xml")
    /// - `parameters`: A HashMap containing parameter names and values
    ///
    /// # Returns
    ///
    /// A new `MediaType` instance with the specified type, subtype, and parameters
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::collections::HashMap;
    ///
    /// // Create a media type with parameters
    /// let mut params = HashMap::new();
    /// params.insert("charset".to_string(), "utf-8".to_string());
    /// params.insert("boundary".to_string(), "boundary_string".to_string());
    ///
    /// let mt = MediaType::with_params("multipart", "mixed", params);
    /// assert_eq!(mt.typ, "multipart");
    /// assert_eq!(mt.subtype, "mixed");
    /// assert_eq!(mt.parameters.len(), 2);
    /// assert_eq!(mt.parameters.get("charset"), Some(&"utf-8".to_string()));
    /// ```
    pub fn with_params(typ: &str, subtype: &str, parameters: HashMap<String, String>) -> Self {
        MediaType {
            typ: typ.to_lowercase(),
            subtype: subtype.to_lowercase(),
            parameters,
        }
    }

    /// Add a parameter to the MediaType
    ///
    /// This method adds a single parameter to the media type using a builder pattern,
    /// allowing for chained method calls. Parameter names are converted to lowercase
    /// for consistency.
    ///
    /// # Parameters
    ///
    /// - `name`: Parameter name (e.g., "charset", "boundary")
    /// - `value`: Parameter value (e.g., "utf-8", "boundary_string")
    ///
    /// # Returns
    ///
    /// The MediaType instance with the added parameter, enabling method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Add a single parameter
    /// let mt = MediaType::new("text", "plain")
    ///     .with_param("charset", "utf-8");
    /// assert_eq!(mt.to_string(), "text/plain;charset=utf-8");
    ///
    /// // Chain multiple parameters
    /// let mt = MediaType::new("multipart", "mixed")
    ///     .with_param("boundary", "unique-boundary-1")
    ///     .with_param("charset", "utf-8");
    /// assert!(mt.to_string().contains("boundary=unique-boundary-1"));
    /// assert!(mt.to_string().contains("charset=utf-8"));
    /// ```
    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.parameters.insert(name.to_lowercase(), value.to_string());
        self
    }

    /// Check if this media type matches another (ignoring parameters)
    ///
    /// This method determines if two media types are compatible, considering wildcards.
    /// A media type matches another if:
    /// - Both types are identical, OR
    /// - Either type is the wildcard "*", OR
    /// - Both subtypes are identical, OR
    /// - Either subtype is the wildcard "*"
    ///
    /// Parameters are ignored in this comparison.
    ///
    /// # Parameters
    ///
    /// - `other`: The MediaType to compare against
    ///
    /// # Returns
    ///
    /// `true` if the media types match (are compatible), `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Exact match
    /// let mt1 = MediaType::new("application", "sdp");
    /// let mt2 = MediaType::new("application", "sdp");
    /// assert!(mt1.matches_type(&mt2));
    ///
    /// // Subtype wildcard match
    /// let mt3 = MediaType::new("application", "*");
    /// assert!(mt1.matches_type(&mt3));
    /// assert!(mt3.matches_type(&mt1)); // Works in both directions
    ///
    /// // Full wildcard match
    /// let mt4 = MediaType::new("*", "*");
    /// assert!(mt1.matches_type(&mt4));
    ///
    /// // Non-matching
    /// let mt5 = MediaType::new("text", "plain");
    /// assert!(!mt1.matches_type(&mt5));
    /// ```
    pub fn matches_type(&self, other: &MediaType) -> bool {
        (self.typ == "*" || other.typ == "*" || self.typ == other.typ) &&
        (self.subtype == "*" || other.subtype == "*" || self.subtype == other.subtype)
    }
}

impl fmt::Display for MediaType {
    /// Formats the MediaType as a string.
    ///
    /// The output follows the standard format: `type/subtype;param1=value1;param2=value2`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Basic media type
    /// let mt = MediaType::new("application", "sdp");
    /// assert_eq!(mt.to_string(), "application/sdp");
    ///
    /// // Media type with parameters
    /// let mt = MediaType::new("text", "plain")
    ///     .with_param("charset", "utf-8");
    /// assert_eq!(mt.to_string(), "text/plain;charset=utf-8");
    ///
    /// // Using in a Content-Type header
    /// let content_type = format!("Content-Type: {}", mt);
    /// assert_eq!(content_type, "Content-Type: text/plain;charset=utf-8");
    /// ```
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
    ///
    /// Creates a media type for SDP (Session Description Protocol), which is
    /// commonly used in SIP messages to describe multimedia sessions.
    ///
    /// # Returns
    ///
    /// A `MediaType` instance for application/sdp
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let sdp = MediaType::sdp();
    /// assert_eq!(sdp.to_string(), "application/sdp");
    ///
    /// // Using in a Content-Type header
    /// let header = format!("Content-Type: {}", sdp);
    /// assert_eq!(header, "Content-Type: application/sdp");
    /// ```
    pub fn sdp() -> Self {
        MediaType::new("application", "sdp")
    }
    
    /// application/sip - SIP message
    ///
    /// Creates a media type for embedded SIP messages, used when
    /// a SIP message contains another SIP message as its body.
    ///
    /// # Returns
    ///
    /// A `MediaType` instance for application/sip
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let sip = MediaType::sip();
    /// assert_eq!(sip.to_string(), "application/sip");
    /// ```
    pub fn sip() -> Self {
        MediaType::new("application", "sip")
    }
    
    /// text/plain - Plain text
    ///
    /// Creates a media type for plain text content, which might be used
    /// for simple message bodies in SIP communications.
    ///
    /// # Returns
    ///
    /// A `MediaType` instance for text/plain
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let text = MediaType::text_plain();
    /// assert_eq!(text.to_string(), "text/plain");
    ///
    /// // Add charset parameter for proper text encoding
    /// let text_utf8 = text.with_param("charset", "utf-8");
    /// assert_eq!(text_utf8.to_string(), "text/plain;charset=utf-8");
    /// ```
    pub fn text_plain() -> Self {
        MediaType::new("text", "plain")
    }
    
    /// multipart/mixed - Multipart message with mixed content
    ///
    /// Creates a media type for multipart mixed content, used when a SIP message
    /// contains multiple body parts with different content types.
    ///
    /// # Returns
    ///
    /// A `MediaType` instance for multipart/mixed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create multipart type with a boundary parameter
    /// let multipart = MediaType::multipart_mixed()
    ///     .with_param("boundary", "boundary-string-1234");
    ///
    /// assert_eq!(multipart.typ, "multipart");
    /// assert_eq!(multipart.subtype, "mixed");
    /// assert_eq!(multipart.parameters.get("boundary"), Some(&"boundary-string-1234".to_string()));
    /// ```
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