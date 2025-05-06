//! Call-Info header builder
//!
//! This module provides builder methods for the Call-Info header,
//! which provides additional information about the caller or callee.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::builder::SimpleRequestBuilder;
//! use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
//! use rvoip_sip_core::types::Method;
//!
//! // Create a request with Call-Info headers
//! let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
//!     .call_info_uri("http://example.com/alice/photo.jpg", Some("icon"))
//!     .call_info_uri("http://example.com/alice/card.html", Some("card"))
//!     .build();
//! ```

use crate::types::{
    call_info::{CallInfo, CallInfoValue, InfoPurpose},
    TypedHeader,
    uri::Uri,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;
use std::str::FromStr;

/// Extension trait for adding Call-Info headers to SIP message builders.
///
/// This trait provides a standard way to add Call-Info headers to both request and response builders
/// as specified in [RFC 3261 Section 20.9](https://datatracker.ietf.org/doc/html/rfc3261#section-20.9).
/// The Call-Info header provides additional information about the caller or callee.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with a Call-Info header for an icon
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .call_info_uri("http://example.com/alice/photo.jpg", Some("icon"))
///     .build();
///
/// // Create a request with a Call-Info header for a business card
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .call_info_uri("http://example.com/alice/card.html", Some("card"))
///     .build();
/// ```
pub trait CallInfoBuilderExt {
    /// Add a Call-Info header using a URI string
    ///
    /// Creates and adds a Call-Info header as specified in [RFC 3261 Section 20.9](https://datatracker.ietf.org/doc/html/rfc3261#section-20.9).
    /// The Call-Info header provides additional information about the caller or callee.
    ///
    /// # Parameters
    /// - `uri`: The URI string pointing to the additional information
    /// - `purpose`: Optional purpose parameter (e.g., "icon", "info", "card")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Note
    /// If the URI cannot be parsed, this method will silently fail.
    fn call_info_uri(self, uri: &str, purpose: Option<&str>) -> Self;

    /// Add a Call-Info header with a pre-constructed CallInfoValue
    ///
    /// # Parameters
    /// - `value`: A pre-constructed CallInfoValue object
    ///
    /// # Returns
    /// Self for method chaining
    fn call_info_value(self, value: CallInfoValue) -> Self;

    /// Add a Call-Info header with multiple values
    ///
    /// # Parameters
    /// - `values`: A vector of CallInfoValue objects
    ///
    /// # Returns
    /// Self for method chaining
    fn call_info_values(self, values: Vec<CallInfoValue>) -> Self;
}

impl CallInfoBuilderExt for SimpleRequestBuilder {
    fn call_info_uri(self, uri: &str, purpose: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(parsed_uri) => {
                let mut value = CallInfoValue::new(parsed_uri);
                
                if let Some(purpose_str) = purpose {
                    let purpose_enum = match purpose_str {
                        "icon" => InfoPurpose::Icon,
                        "info" => InfoPurpose::Info,
                        "card" => InfoPurpose::Card,
                        other => InfoPurpose::Other(other.to_string()),
                    };
                    value = value.with_purpose(purpose_enum);
                }
                
                self.call_info_value(value)
            },
            Err(_) => self // Silently fail if URI is invalid
        }
    }
    
    fn call_info_value(self, value: CallInfoValue) -> Self {
        self.header(TypedHeader::CallInfo(CallInfo::with_value(value)))
    }
    
    fn call_info_values(self, values: Vec<CallInfoValue>) -> Self {
        if values.is_empty() {
            return self;
        }
        self.header(TypedHeader::CallInfo(CallInfo::new(values)))
    }
}

impl CallInfoBuilderExt for SimpleResponseBuilder {
    fn call_info_uri(self, uri: &str, purpose: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(parsed_uri) => {
                let mut value = CallInfoValue::new(parsed_uri);
                
                if let Some(purpose_str) = purpose {
                    let purpose_enum = match purpose_str {
                        "icon" => InfoPurpose::Icon,
                        "info" => InfoPurpose::Info,
                        "card" => InfoPurpose::Card,
                        other => InfoPurpose::Other(other.to_string()),
                    };
                    value = value.with_purpose(purpose_enum);
                }
                
                self.call_info_value(value)
            },
            Err(_) => self // Silently fail if URI is invalid
        }
    }
    
    fn call_info_value(self, value: CallInfoValue) -> Self {
        self.header(TypedHeader::CallInfo(CallInfo::with_value(value)))
    }
    
    fn call_info_values(self, values: Vec<CallInfoValue>) -> Self {
        if values.is_empty() {
            return self;
        }
        self.header(TypedHeader::CallInfo(CallInfo::new(values)))
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

    #[test]
    fn test_request_call_info_uri() {
        let uri = "http://example.com/alice/photo.jpg";
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_info_uri(uri, Some("icon"))
            .build();
            
        let call_info_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_info_headers.len(), 1);
        assert_eq!(call_info_headers[0].0.len(), 1);
        assert_eq!(call_info_headers[0].0[0].uri.to_string(), uri);
        assert_eq!(call_info_headers[0].0[0].purpose(), Some(InfoPurpose::Icon));
    }
    
    #[test]
    fn test_response_call_info_uri() {
        let uri = "http://example.com/alice/photo.jpg";
        
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .call_info_uri(uri, Some("icon"))
            .build();
            
        let call_info_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_info_headers.len(), 1);
        assert_eq!(call_info_headers[0].0.len(), 1);
        assert_eq!(call_info_headers[0].0[0].uri.to_string(), uri);
        assert_eq!(call_info_headers[0].0[0].purpose(), Some(InfoPurpose::Icon));
    }
    
    #[test]
    fn test_request_call_info_multiple_purposes() {
        let icon_uri = "http://example.com/alice/photo.jpg";
        let card_uri = "http://example.com/alice/card.html";
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_info_uri(icon_uri, Some("icon"))
            .call_info_uri(card_uri, Some("card"))
            .build();
            
        let call_info_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        // Note: The implementation adds a new Call-Info header for each call to call_info_uri,
        // rather than appending values to an existing header. This is consistent with SIP
        // which allows multiple instances of the same header.
        assert_eq!(call_info_headers.len(), 2);
        
        // Find the icon header
        let icon_header = call_info_headers.iter()
            .find(|h| h.0[0].purpose() == Some(InfoPurpose::Icon))
            .unwrap();
        assert_eq!(icon_header.0[0].uri.to_string(), icon_uri);
        
        // Find the card header
        let card_header = call_info_headers.iter()
            .find(|h| h.0[0].purpose() == Some(InfoPurpose::Card))
            .unwrap();
        assert_eq!(card_header.0[0].uri.to_string(), card_uri);
    }
    
    #[test]
    fn test_request_call_info_values() {
        let icon_uri = Uri::from_str("http://example.com/alice/photo.jpg").unwrap();
        let card_uri = Uri::from_str("http://example.com/alice/card.html").unwrap();
        
        let icon_value = CallInfoValue::new(icon_uri).with_purpose(InfoPurpose::Icon);
        let card_value = CallInfoValue::new(card_uri).with_purpose(InfoPurpose::Card);
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_info_values(vec![icon_value, card_value])
            .build();
            
        let call_info_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_info_headers.len(), 1);
        assert_eq!(call_info_headers[0].0.len(), 2);
        
        // Check first value (icon)
        assert_eq!(call_info_headers[0].0[0].purpose(), Some(InfoPurpose::Icon));
        assert_eq!(call_info_headers[0].0[0].uri.to_string(), "http://example.com/alice/photo.jpg");
        
        // Check second value (card)
        assert_eq!(call_info_headers[0].0[1].purpose(), Some(InfoPurpose::Card));
        assert_eq!(call_info_headers[0].0[1].uri.to_string(), "http://example.com/alice/card.html");
    }
} 