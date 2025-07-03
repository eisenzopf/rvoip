use std::fmt;
use crate::types::headers::header_name::HeaderName;
use crate::types::headers::header_value::HeaderValue;

/// SIP header, consisting of a name and value
///
/// This struct represents a SIP header with a [`HeaderName`] and a [`HeaderValue`].
/// It provides a more generic representation of headers compared to [`TypedHeader`],
/// and is primarily used during parsing and in cases where type-safety is not required.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a text header
/// let header = Header::text(HeaderName::Subject, "Meeting tomorrow");
/// assert_eq!(header.to_wire_format(), "Subject: Meeting tomorrow");
///
/// // Create an integer header
/// let header = Header::integer(HeaderName::ContentLength, 123);
/// assert_eq!(header.to_wire_format(), "Content-Length: 123");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Header name
    pub name: HeaderName,
    /// Header value
    pub value: HeaderValue,
}

impl Header {
    /// Create a new header
    pub fn new(name: HeaderName, value: HeaderValue) -> Self {
        Header { name, value }
    }

    /// Create a new text header
    pub fn text(name: HeaderName, value: impl Into<String>) -> Self {
        Header::new(name, HeaderValue::text(value))
    }

    /// Create a new integer header
    pub fn integer(name: HeaderName, value: i64) -> Self {
        Header::new(name, HeaderValue::integer(value))
    }

    /// Create a Content-Type header for SDP
    pub fn content_type_sdp() -> Self {
        Header::new(HeaderName::ContentType, HeaderValue::content_type_sdp())
    }

    /// Create a Content-Type header for plain text
    pub fn content_type_text_plain() -> Self {
        Header::new(HeaderName::ContentType, HeaderValue::content_type_text_plain())
    }

    /// Create a Content-Type header for JSON
    pub fn content_type_json() -> Self {
        Header::new(HeaderName::ContentType, HeaderValue::content_type_json())
    }

    /// Create a Content-Type header for multipart/mixed
    pub fn content_type_multipart_mixed(boundary: impl Into<String>) -> Self {
        Header::new(HeaderName::ContentType, HeaderValue::content_type_multipart_mixed(boundary))
    }

    /// Create a Content-Length header
    pub fn content_length(length: usize) -> Self {
        Header::new(HeaderName::ContentLength, HeaderValue::content_length(length))
    }

    /// Create a Max-Forwards header
    pub fn max_forwards(value: u8) -> Self {
        Header::new(HeaderName::MaxForwards, HeaderValue::max_forwards(value))
    }

    /// Get the header as a formatted string, ready for wire transmission
    pub fn to_wire_format(&self) -> String {
        format!("{}: {}", self.name, self.value)
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_header_creation() {
        let h = Header::text(HeaderName::To, "sip:alice@example.com");
        assert_eq!(h.name, HeaderName::To);
        assert_eq!(h.value.as_text(), Some("sip:alice@example.com"));
        
        let h = Header::integer(HeaderName::ContentLength, 42);
        assert_eq!(h.name, HeaderName::ContentLength);
        assert_eq!(h.value.as_integer(), Some(42));
    }

    #[test]
    fn test_header_wire_format() {
        let h = Header::text(HeaderName::To, "sip:alice@example.com");
        assert_eq!(h.to_wire_format(), "To: sip:alice@example.com");
        
        let h = Header::integer(HeaderName::ContentLength, 42);
        assert_eq!(h.to_wire_format(), "Content-Length: 42");
        
        let h = Header::new(
            HeaderName::Via, 
            HeaderValue::text("SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK776asdhds")
        );
        assert_eq!(
            h.to_wire_format(), 
            "Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK776asdhds"
        );
    }
} 