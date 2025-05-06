//! Content-Type header builder
//!
//! This module provides builder methods for the Content-Type header,
//! which specifies the media type of the message body.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::builder::SimpleRequestBuilder;
//! use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
//! use rvoip_sip_core::types::Method;
//!
//! // Create a request with Content-Type set to application/sdp
//! let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
//!     .content_type_sdp()
//!     .build();
//!     
//! // Create a request with Content-Type set to text/plain
//! let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
//!     .content_type_text()
//!     .build();
//!     
//! // Create a request with a custom Content-Type
//! let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .content_type_custom("application", "xml+soap")
//!     .build();
//! ```

use crate::types::{
    content_type::ContentType,
    TypedHeader,
};
use crate::parser::headers::content_type::ContentTypeValue;
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;
use std::collections::HashMap;
use std::str::FromStr;

/// Extension trait for adding Content-Type headers to SIP message builders.
///
/// This trait provides a standard way to add Content-Type headers to both request and response builders
/// as specified in [RFC 3261 Section 20.15](https://datatracker.ietf.org/doc/html/rfc3261#section-20.15).
/// The Content-Type header specifies the media type of the message body.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with Content-Type set to application/sdp
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .content_type_sdp()
///     .build();
///     
/// // Create a request with Content-Type set to application/json
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .content_type_json()
///     .build();
/// ```
pub trait ContentTypeBuilderExt {
    /// Add a Content-Type header specifying 'application/sdp'
    ///
    /// This is a convenience method for setting the Content-Type to SDP,
    /// which is commonly used in SIP for session descriptions.
    ///
    /// # Returns
    /// Self for method chaining
    fn content_type_sdp(self) -> Self;
    
    /// Add a Content-Type header specifying 'text/plain'
    ///
    /// This is a convenience method for setting the Content-Type to plain text.
    ///
    /// # Returns
    /// Self for method chaining
    fn content_type_text(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/xml'
    ///
    /// This is a convenience method for setting the Content-Type to XML.
    ///
    /// # Returns
    /// Self for method chaining
    fn content_type_xml(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/json'
    ///
    /// This is a convenience method for setting the Content-Type to JSON.
    ///
    /// # Returns
    /// Self for method chaining
    fn content_type_json(self) -> Self;
    
    /// Add a Content-Type header specifying 'message/sipfrag'
    ///
    /// This is a convenience method for setting the Content-Type to SIP fragments,
    /// commonly used in REFER responses.
    ///
    /// # Returns
    /// Self for method chaining
    fn content_type_sipfrag(self) -> Self;
    
    /// Add a Content-Type header with a custom media type
    ///
    /// # Parameters
    ///
    /// - `media_type`: Primary media type (e.g., "text", "application")
    /// - `media_subtype`: Subtype (e.g., "plain", "json")
    ///
    /// # Returns
    /// Self for method chaining
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self;
    
    /// Add a Content-Type header
    ///
    /// Creates and adds a Content-Type header as specified in [RFC 3261 Section 20.15](https://datatracker.ietf.org/doc/html/rfc3261#section-20.15).
    /// This method allows setting a Content-Type directly from a string.
    ///
    /// # Parameters
    /// - `content_type`: The content type string (e.g., "application/sdp")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Note
    /// If the content type cannot be parsed, this method will silently fail.
    fn content_type(self, content_type: &str) -> Self;
}

impl ContentTypeBuilderExt for SimpleRequestBuilder {
    fn content_type_sdp(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_text(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_xml(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "xml".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_json(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "json".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_sipfrag(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "message".to_string(),
            m_subtype: "sipfrag".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: media_type.to_string(),
            m_subtype: media_subtype.to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type(self, content_type: &str) -> Self {
        match ContentType::from_str(content_type) {
            Ok(ct) => {
                self.header(TypedHeader::ContentType(ct))
            },
            Err(_) => self // Silently fail if content type is invalid
        }
    }
}

impl ContentTypeBuilderExt for SimpleResponseBuilder {
    fn content_type_sdp(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_text(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_xml(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "xml".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_json(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "json".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_sipfrag(self) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: "message".to_string(),
            m_subtype: "sipfrag".to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self {
        let ct = ContentType::new(ContentTypeValue {
            m_type: media_type.to_string(),
            m_subtype: media_subtype.to_string(),
            parameters: HashMap::new(),
        });
        self.header(TypedHeader::ContentType(ct))
    }
    
    fn content_type(self, content_type: &str) -> Self {
        match ContentType::from_str(content_type) {
            Ok(ct) => {
                self.header(TypedHeader::ContentType(ct))
            },
            Err(_) => self // Silently fail if content type is invalid
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    use crate::types::headers::HeaderAccess;
    use crate::builder::headers::cseq::CSeqBuilderExt;
    use crate::builder::headers::from::FromBuilderExt;
    use crate::builder::headers::to::ToBuilderExt;
    use std::str::FromStr;

    #[test]
    fn test_request_content_type_shortcuts() {
        // Test SDP content type
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type_sdp()
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
        
        // Test text content type
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
            .content_type_text()
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "text/plain");
    }
    
    #[test]
    fn test_response_content_type_shortcuts() {
        // Test XML content type
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .content_type_xml()
            .build();
            
        let content_type_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/xml");
        
        // Test JSON content type
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .content_type_json()
            .build();
            
        let content_type_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/json");
    }
    
    #[test]
    fn test_content_type_string() {
        // Test content_type method with valid content type
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type("application/sdp")
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
        
        // Test content_type method with parameters
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type("application/sdp; charset=UTF-8")
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp;charset=\"UTF-8\"");
    }
    
    #[test]
    fn test_custom_content_type() {
        // Test custom content type
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type_custom("application", "vnd.3gpp.sms")
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/vnd.3gpp.sms");
    }
    
    #[test]
    fn test_content_type_sipfrag() {
        // Test sipfrag content type for request
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type_sipfrag()
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "message/sipfrag");
        
        // Test sipfrag content type for response
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .content_type_sipfrag()
            .build();
            
        let content_type_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "message/sipfrag");
    }
} 