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
//!     .body(concat!(
//!         "v=0\r\n",
//!         "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
//!         "s=Call with Alice\r\n",
//!         "c=IN IP4 192.0.2.1\r\n",  // Connection info required by SDP
//!         "t=0 0\r\n",
//!         "m=audio 49170 RTP/AVP 0 8\r\n",
//!         "a=rtpmap:0 PCMU/8000\r\n",
//!         "a=rtpmap:8 PCMA/8000\r\n"
//!     ))
//!     .build();
//!     
//! // Create a request with Content-Type set to text/plain
//! let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
//!     .content_type_text()
//!     .body("Hello, this is a text message sent via SIP")
//!     .build();
//!     
//! // Create a request with a custom Content-Type
//! let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .content_type_custom("application", "xml+soap")
//!     .body(concat!(
//!         "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">\r\n",
//!         "  <soap:Body>\r\n",
//!         "    <registerRequest xmlns=\"http://example.org/register\">\r\n",
//!         "      <username>alice</username>\r\n",
//!         "      <password>secret</password>\r\n", 
//!         "    </registerRequest>\r\n",
//!         "  </soap:Body>\r\n",
//!         "</soap:Envelope>\r\n"
//!     ))
//!     .build();
//!
//! // Create a request with Content-Type including parameters
//! let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
//!     .content_type("application/xml; charset=UTF-8")
//!     .body(concat!(
//!         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
//!         "<message>\r\n",
//!         "  <to>Bob</to>\r\n",
//!         "  <from>Alice</from>\r\n",
//!         "  <content>Hello, this is an XML message with UTF-8 encoding</content>\r\n",
//!         "</message>\r\n"
//!     ))
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
/// ## Basic Content-Type Headers
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with Content-Type set to application/sdp
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .content_type_sdp()
///     .body(concat!(
///         "v=0\r\n",
///         "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
///         "s=Call with Alice\r\n",
///         "c=IN IP4 192.0.2.1\r\n",
///         "t=0 0\r\n",
///         "m=audio 49170 RTP/AVP 0 8\r\n",
///         "a=rtpmap:0 PCMU/8000\r\n",
///         "a=rtpmap:8 PCMA/8000\r\n"
///     ))
///     .build();
///     
/// // Create a request with Content-Type set to application/json
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .content_type_json()
///     .body(r#"{"message": "Hello world", "from": "Alice", "timestamp": 1620000000}"#)
///     .build();
/// ```
///
/// ## Content-Type with Parameters
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with Content-Type including character set parameter
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .content_type("application/xml; charset=UTF-8")
///     .body(concat!(
///         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
///         "<message>\r\n",
///         "  <to>Bob</to>\r\n",
///         "  <from>Alice</from>\r\n",
///         "  <content>Hello with non-ASCII characters: áéíóú</content>\r\n",
///         "</message>\r\n"
///     ))
///     .build();
/// ```
///
/// ## Special Content Types for SIP Extensions
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a NOTIFY request with SIP fragment body (used in REFER/NOTIFY scenarios)
/// let request = SimpleRequestBuilder::new(Method::Notify, "sip:user@example.com").unwrap()
///     .content_type_sipfrag()  // Sets Content-Type: message/sipfrag
///     .body("SIP/2.0 200 OK\r\nCSeq: 1 INVITE\r\nContact: <sip:bob@192.0.2.4>\r\n")
///     .build();
///     
/// // Create a MESSAGE with multimedia messaging content
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
///     .content_type_custom("application", "vnd.3gpp.sms")  // 3GPP SMS format
///     .body("01000B915121551532F40000A723719C0E4ACF41F4329E0E")  // Binary SMS content (hex encoded)
///     .build();
///     
/// // Create a MESSAGE with CPIM content for instant messaging federation
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
///     .content_type_custom("message", "cpim")  // Common Presence and Instant Messaging
///     .body(concat!(
///         "From: <sip:alice@example.com>\r\n",
///         "To: <sip:bob@example.net>\r\n",
///         "DateTime: 2023-05-15T14:33:22Z\r\n",
///         "Content-Type: text/plain; charset=utf-8\r\n",
///         "\r\n",
///         "Hello Bob, this is a CPIM wrapped message"
///     ))
///     .build();
/// ```
///
/// ## With Accept Header for Content Negotiation
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, AcceptExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create an OPTIONS request with content negotiation headers
/// let request = SimpleRequestBuilder::new(Method::Options, "sip:example.com").unwrap()
///     // Add Accept headers to indicate supported content types for the response
///     .accepts(vec![
///         ("application/sdp", Some(1.0)),
///         ("application/json", Some(0.8)),
///     ])
///     // Add Content-Type for the request body
///     .content_type_json()
///     .body(r#"{"supported_codecs":["PCMU","PCMA","opus"],"ice":true,"dtls":true}"#)
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
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create an INVITE with SDP body for call setup
    /// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .content_type_sdp()
    ///     .body(concat!(
    ///         "v=0\r\n",
    ///         "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
    ///         "s=Call with Alice\r\n",
    ///         "c=IN IP4 192.0.2.1\r\n",
    ///         "t=0 0\r\n",
    ///         "m=audio 49170 RTP/AVP 0 8\r\n",
    ///         "a=rtpmap:0 PCMU/8000\r\n",
    ///         "a=rtpmap:8 PCMA/8000\r\n"
    ///     ))
    ///     .build();
    /// ```
    fn content_type_sdp(self) -> Self;
    
    /// Add a Content-Type header specifying 'text/plain'
    ///
    /// This is a convenience method for setting the Content-Type to plain text.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a MESSAGE request with plain text body
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_text()
    ///     .body("Hello, this is a text message")
    ///     .build();
    /// ```
    fn content_type_text(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/xml'
    ///
    /// This is a convenience method for setting the Content-Type to XML.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a MESSAGE request with XML body
    /// let xml_content = concat!(
    ///     "<?xml version=\"1.0\"?>\r\n",
    ///     "<message>\r\n",
    ///     "  <text>Hello world</text>\r\n",
    ///     "  <importance>high</importance>\r\n",
    ///     "</message>\r\n"
    /// );
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_xml()
    ///     .body(xml_content)
    ///     .build();
    /// ```
    fn content_type_xml(self) -> Self;
    
    /// Add a Content-Type header specifying 'application/json'
    ///
    /// This is a convenience method for setting the Content-Type to JSON.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a MESSAGE request with JSON body
    /// let json_content = r#"{
    ///   "message": "Hello world",
    ///   "priority": "normal",
    ///   "sender": {
    ///     "name": "Alice",
    ///     "uri": "sip:alice@example.com"
    ///   },
    ///   "timestamp": 1620000000
    /// }"#;
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_json()
    ///     .body(json_content)
    ///     .build();
    /// ```
    fn content_type_json(self) -> Self;
    
    /// Add a Content-Type header specifying 'message/sipfrag'
    ///
    /// This is a convenience method for setting the Content-Type to SIP fragments,
    /// commonly used in REFER responses.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a NOTIFY request with SIP fragment body
    /// // This is typically used when reporting status of a REFER request
    /// let notify = SimpleRequestBuilder::new(Method::Notify, "sip:bob@example.com").unwrap()
    ///     .content_type_sipfrag()
    ///     .body(concat!(
    ///         "SIP/2.0 200 OK\r\n",
    ///         "CSeq: 1 INVITE\r\n",
    ///         "Contact: <sip:alice@192.0.2.3>\r\n",
    ///         "Content-Type: application/sdp\r\n",
    ///         "Content-Length: 0\r\n"
    ///     ))
    ///     .build();
    /// ```
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
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Typical custom Content-Types in SIP applications:
    ///
    /// // SMS over SIP (3GPP specification)
    /// let sms_message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_custom("application", "vnd.3gpp.sms")
    ///     .body("01000B915121551532F40000A723719C0E4ACF41F4329E0E")  // Binary SMS content (hex encoded)
    ///     .build();
    ///     
    /// // MSRP relay setup (Message Session Relay Protocol)
    /// let msrp_message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type_custom("message", "msrp-setup")
    ///     .body(concat!(
    ///         "m=message 7654 TCP/MSRP *\r\n",
    ///         "a=accept-types:text/plain text/html message/cpim\r\n",
    ///         "a=path:msrp://atlanta.example.com:7654/jshA7weztas;tcp\r\n"
    ///     ))
    ///     .build();
    ///     
    /// // PIDF presence information
    /// let publish = SimpleRequestBuilder::new(Method::Publish, "sip:bob@example.com").unwrap()
    ///     .content_type_custom("application", "pidf+xml")
    ///     .body(concat!(
    ///         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
    ///         "<presence xmlns=\"urn:ietf:params:xml:ns:pidf\" entity=\"sip:alice@example.com\">\r\n",
    ///         "  <tuple id=\"a123\">\r\n",
    ///         "    <status><basic>open</basic></status>\r\n",
    ///         "    <contact>sip:alice@192.0.2.1</contact>\r\n",
    ///         "  </tuple>\r\n",
    ///         "</presence>\r\n"
    ///     ))
    ///     .build();
    /// ```
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a message with Content-Type including parameters
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type("application/xml; charset=UTF-8")
    ///     .body(concat!(
    ///         "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
    ///         "<message>\r\n",
    ///         "  <to>Bob</to>\r\n",
    ///         "  <from>Alice</from>\r\n",
    ///         "  <content>Message with UTF-8 characters: áéíóú</content>\r\n",
    ///         "</message>\r\n"
    ///     ))
    ///     .build();
    ///     
    /// // Create a message with multipart Content-Type
    /// let boundary = "unique-boundary-1";
    /// let multipart_body = concat!(
    ///     "--unique-boundary-1\r\n",
    ///     "Content-Type: text/plain\r\n",
    ///     "\r\n",
    ///     "This is the first part of the multipart message.\r\n",
    ///     "--unique-boundary-1\r\n",
    ///     "Content-Type: application/sdp\r\n",
    ///     "\r\n",
    ///     "v=0\r\n",
    ///     "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
    ///     "s=Session\r\n",
    ///     "c=IN IP4 192.0.2.1\r\n",
    ///     "t=0 0\r\n",
    ///     "m=audio 49170 RTP/AVP 0\r\n",
    ///     "a=rtpmap:0 PCMU/8000\r\n",
    ///     "--unique-boundary-1--\r\n"
    /// );
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .content_type(format!("multipart/mixed; boundary={}", boundary).as_str())
    ///     .body(multipart_body)
    ///     .build();
    /// ```
    ///
    /// # Note
    ///
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
            .body(concat!(
                "v=0\r\n",
                "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
                "s=Call with Alice\r\n",
                "c=IN IP4 192.0.2.1\r\n",  // Connection info required by SDP
                "t=0 0\r\n",
                "m=audio 49170 RTP/AVP 0 8\r\n",
                "a=rtpmap:0 PCMU/8000\r\n",
                "a=rtpmap:8 PCMA/8000\r\n"
            ))
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
        
        // Test text content type
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
            .content_type_text()
            .body("Hello, this is a text message sent via SIP")
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
            .body(concat!(
                "v=0\r\n",
                "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
                "s=Call with Alice\r\n",
                "c=IN IP4 192.0.2.1\r\n",
                "t=0 0\r\n",
                "m=audio 49170 RTP/AVP 0 8\r\n",
                "a=rtpmap:0 PCMU/8000\r\n",
                "a=rtpmap:8 PCMA/8000\r\n"
            ))
            .build();
            
        let content_type_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentType(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert_eq!(content_type_headers[0].to_string(), "application/sdp");
        
        // Test content_type method with parameters
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type("application/sdp; charset=UTF-8")
            .body(concat!(
                "v=0\r\n",
                "o=alice 2890844526 2890844526 IN IP4 192.0.2.1\r\n",
                "s=Call with Alice\r\n",
                "c=IN IP4 192.0.2.1\r\n",
                "t=0 0\r\n",
                "m=audio 49170 RTP/AVP 0 8\r\n",
                "a=rtpmap:0 PCMU/8000\r\n",
                "a=rtpmap:8 PCMA/8000\r\n"
            ))
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
            .body(concat!(
                "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">\r\n",
                "  <soap:Body>\r\n",
                "    <registerRequest xmlns=\"http://example.org/register\">\r\n",
                "      <username>alice</username>\r\n",
                "      <password>secret</password>\r\n", 
                "    </registerRequest>\r\n",
                "  </soap:Body>\r\n",
                "</soap:Envelope>\r\n"
            ))
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
            .body(concat!(
                "SIP/2.0 200 OK\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:alice@192.0.2.3>\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: 0\r\n"
            ))
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