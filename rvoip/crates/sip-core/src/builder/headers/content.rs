//! Content header builders
//!
//! This module provides builder methods for content-related headers.

use crate::error::{Error, Result};
use crate::types::{
    header::{Header, HeaderName},
    headers::TypedHeader,
    content_type::ContentType,
};
use crate::types::headers::typed_header::TypedHeaderTrait;
use crate::builder::headers::HeaderSetter;

/// Extension trait that adds content-related building capabilities to request and response builders
pub trait ContentBuilderExt {
    /// Add a Content-Type header specifying 'application/sdp'
    ///
    /// This is a convenience method for setting the Content-Type to SDP,
    /// which is commonly used in SIP for session descriptions.
    fn content_type_sdp(self) -> Self;
    
    /// Add a Content-Type header specifying 'text/plain'
    ///
    /// This is a convenience method for setting the Content-Type to plain text.
    fn content_type_text(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/xml'
    ///
    /// This is a convenience method for setting the Content-Type to XML.
    fn content_type_xml(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/json'
    ///
    /// This is a convenience method for setting the Content-Type to JSON.
    fn content_type_json(self) -> Self;
    
    /// Add a Content-Type header specifying 'message/sipfrag'
    ///
    /// This is a convenience method for setting the Content-Type to SIP fragments,
    /// commonly used in REFER responses.
    fn content_type_sipfrag(self) -> Self;
    
    /// Add a Content-Type header with a custom media type
    ///
    /// # Parameters
    ///
    /// - `media_type`: Primary media type (e.g., "text", "application")
    /// - `media_subtype`: Subtype (e.g., "plain", "json")
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self;
}

impl<T> ContentBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn content_type_sdp(self) -> Self {
        let content_type = ContentType::from_type_subtype("application", "sdp");
        self.set_header(content_type)
    }
    
    fn content_type_text(self) -> Self {
        let content_type = ContentType::from_type_subtype("text", "plain");
        self.set_header(content_type)
    }
    
    fn content_type_xml(self) -> Self {
        let content_type = ContentType::from_type_subtype("application", "xml");
        self.set_header(content_type)
    }
    
    fn content_type_json(self) -> Self {
        let content_type = ContentType::from_type_subtype("application", "json");
        self.set_header(content_type)
    }
    
    fn content_type_sipfrag(self) -> Self {
        let content_type = ContentType::from_type_subtype("message", "sipfrag");
        self.set_header(content_type)
    }
    
    fn content_type_custom(self, media_type: &str, media_subtype: &str) -> Self {
        let content_type = ContentType::from_type_subtype(media_type, media_subtype);
        self.set_header(content_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;
    use crate::types::headers::HeaderAccess;

    #[test]
    fn test_request_content_type_shortcuts() {
        // Test SDP content type
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .content_type_sdp()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = request.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/sdp");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
        
        // Test text content type
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .content_type_text()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = request.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "text/plain");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_response_content_type_shortcuts() {
        // Test XML content type
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .content_type_xml()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/xml");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
        
        // Test JSON content type
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .content_type_json()
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/json");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_custom_content_type() {
        // Test custom content type
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .content_type_custom("application", "pidf+xml")
            .build();
            
        if let Some(TypedHeader::ContentType(content_type)) = request.header(&HeaderName::ContentType) {
            assert_eq!(content_type.to_string(), "application/pidf+xml");
        } else {
            panic!("Content-Type header not found or has wrong type");
        }
    }
} 