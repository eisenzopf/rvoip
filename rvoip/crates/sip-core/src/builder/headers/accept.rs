use crate::error::{Error, Result};
use std::collections::HashMap;
use ordered_float::NotNan;
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::accept::Accept;
use crate::parser::headers::accept::AcceptValue;
use super::HeaderSetter;

/// Accept Header Builder for SIP Messages
///
/// This module provides builder methods for the Accept header in SIP messages,
/// which indicates what content types the User Agent can understand.
///
/// ## SIP Accept Header Overview
///
/// The Accept header is defined in [RFC 3261 Section 20.1](https://datatracker.ietf.org/doc/html/rfc3261#section-20.1)
/// as part of the core SIP protocol. It follows the syntax and semantics defined in 
/// [RFC 2616 Section 14.1](https://datatracker.ietf.org/doc/html/rfc2616#section-14.1) for HTTP.
/// The header specifies which media types are acceptable for the response or future requests
/// in the same dialog.
///
/// ## Purpose of Accept Header
///
/// The Accept header serves several important purposes in SIP:
///
/// 1. It allows a UA to specify which content types it can process
/// 2. It enables content type negotiation between UAs
/// 3. It provides a mechanism to indicate preferences via quality values (q-values)
/// 4. It helps prevent servers from sending content the client cannot understand
///
/// ## Common Media Types in SIP
///
/// - **application/sdp**: Session Description Protocol, used for negotiating media sessions
/// - **application/pidf+xml**: Presence Information Data Format, used for presence services
/// - **application/dialog-info+xml**: Dialog state information
/// - **application/xpidf+xml**: Legacy presence format
/// - **application/simple-message-summary**: Message waiting indicator information
/// - **multipart/mixed**: Container for multiple content types
/// - **application/vnd.3gpp.mcptt-info+xml**: Mission Critical Push-to-Talk information
/// - **application/resource-lists+xml**: Resource list document
///
/// ## Quality Values (q-values)
///
/// The Accept header can include quality values (q-values) to indicate preference order:
///
/// - Values range from 0.0 to 1.0, with 1.0 being the highest priority
/// - Default value is 1.0 when not specified
/// - When multiple acceptable types are specified, the q-values help the server choose the best match
///
/// ## Special Considerations
///
/// 1. **Content Type Matching**: Servers should favor exact matches over wildcard matches
/// 2. **Multiple Headers**: The Accept header can appear multiple times in a request
/// 3. **Default Behavior**: If no Accept header is present, the UA is assumed to accept all content types
/// 4. **OPTIONS Requests**: In OPTIONS requests, the Accept header indicates supported content types
///
/// ## Relationship with other headers
///
/// - **Accept** + **Content-Type**: Accept specifies what can be received, Content-Type specifies what is being sent
/// - **Accept** vs **Accept-Encoding**: Accept is for content types, Accept-Encoding is for compression methods
/// - **Accept** vs **Accept-Language**: Accept is for media types, Accept-Language is for natural languages
/// - **Accept** + **Supported**: Together they define the UA's capabilities for content and extensions
///
/// # Examples
///
/// ## Basic Usage with SDP
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create an INVITE that accepts SDP responses
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:recipient@example.com").unwrap()
///     .accept("application/sdp", None)  // Accept SDP content with default priority
///     .build();
/// ```
///
/// ## Multiple Media Types with Priorities
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create an OPTIONS request with multiple accepted content types
/// let media_types = vec![
///     ("application/sdp", Some(1.0)),          // Highest priority
///     ("application/pidf+xml", Some(0.8)),     // Second priority
///     ("application/xpidf+xml", Some(0.5)),    // Lower priority
/// ];
///
/// let request = SimpleRequestBuilder::new(Method::Options, "sip:server@example.com").unwrap()
///     .accepts(media_types)
///     .build();
/// ```
///
/// ## Presence Subscription
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a SUBSCRIBE request for presence information
/// let request = SimpleRequestBuilder::new(Method::Subscribe, "sip:alice@example.com").unwrap()
///     .accept("application/pidf+xml", Some(1.0))       // Prefer PIDF format
///     .accept("application/xpidf+xml", Some(0.5))      // Accept XPIDF as fallback
///     .build();
/// ```
/// ## When to use Accept Headers
///
/// Accept headers are particularly useful in the following scenarios:
///
/// 1. **Content negotiation**: When a client supports multiple content formats
/// 2. **OPTIONS requests**: To indicate what content types a client can process
/// 3. **SUBSCRIBE requests**: To specify acceptable notification formats
/// 4. **Service discovery**: To inform servers about client capabilities
/// 5. **Rich communication**: When supporting advanced content types beyond SDP
///
/// ## Best Practices
///
/// - Include Accept headers in OPTIONS requests to advertise capabilities
/// - Specify q-values when supporting multiple formats with preferences
/// - Always include application/sdp for UAs that support audio/video sessions
/// - For presence services, support both PIDF and XPIDF formats for compatibility
/// - In mission-critical applications, be explicit about supported formats
///
/// # Examples
///
/// ## OPTIONS Request with Capabilities
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create an OPTIONS request that advertises client capabilities
/// let request = SimpleRequestBuilder::new(Method::Options, "sip:server@example.com").unwrap()
///     .from("Client", "sip:client@example.com", Some("options-123"))
///     .to("Server", "sip:server@example.com", None)
///     // List all content types the client can handle
///     .accept("application/sdp", Some(1.0))
///     .accept("application/pidf+xml", Some(0.9))
///     .accept("application/dialog-info+xml", Some(0.8))
///     .accept("application/resource-lists+xml", Some(0.7))
///     .accept("multipart/mixed", Some(1.0))
///     .build();
/// ```
///
/// ## SIP Messaging Support
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE request that indicates supported response formats
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
///     .from("Sender", "sip:sender@example.com", Some("msg567"))
///     .to("User", "sip:user@example.com", None)
///     // Indicate support for delivery receipts in different formats
///     .accept("message/imdn+xml", Some(1.0))      // Prefer IMDN format
///     .accept("message/delivery-notification", Some(0.8))
///     .body("Hello, this is a test message")
///     .build();
/// ```
pub trait AcceptExt {
    /// Add an Accept header with a single media type
    ///
    /// This method specifies a single content type that the UA can process,
    /// optionally with a quality value (q-value) to indicate preference when
    /// multiple Accept headers are present.
    ///
    /// # Arguments
    ///
    /// * `media_type` - The media type (e.g., "application/sdp")
    /// * `q` - Optional quality value (0.0 to 1.0, where 1.0 is highest priority)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptExt};
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create an INVITE that accepts SDP responses
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("invite-234"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .accept("application/sdp", Some(0.9))  // Accept SDP with high priority
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 3261 Section 20.1](https://datatracker.ietf.org/doc/html/rfc3261#section-20.1),
    /// the Accept header field follows the syntax defined in 
    /// [RFC 2616 Section 14.1](https://datatracker.ietf.org/doc/html/rfc2616#section-14.1),
    /// including the use of q-values to indicate relative preference.
    fn accept(
        self, 
        media_type: &str, 
        q: Option<f32>
    ) -> Self;

    /// Add an Accept header with multiple media types
    ///
    /// This method specifies multiple content types that the UA can process,
    /// each with an optional quality value to indicate preference order.
    /// This is more efficient than adding multiple individual Accept headers.
    ///
    /// # Arguments
    ///
    /// * `media_types` - A vector of tuples containing (media_type, q_value)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptExt};
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a SUBSCRIBE request for presence information with format preferences
    /// let media_types = vec![
    ///     ("application/pidf+xml", Some(1.0)),      // Preferred format
    ///     ("application/xpidf+xml", Some(0.8)),     // Acceptable alternative
    ///     ("application/cpim-pidf+xml", Some(0.5)), // Least preferred format
    /// ];
    /// 
    /// let request = SimpleRequestBuilder::new(Method::Subscribe, "sip:presence@example.com").unwrap()
    ///     .from("Watcher", "sip:watcher@example.com", Some("sub876"))
    ///     .to("Presence Service", "sip:presence@example.com", None)
    ///     .accepts(media_types)  // Specify all acceptable formats with priorities
    ///     .build();
    /// ```
    ///
    /// # Media Type Format
    ///
    /// Each media type should follow the format `type/subtype`, such as:
    ///
    /// - `application/sdp`
    /// - `text/plain`
    /// - `application/pidf+xml`
    /// - `multipart/mixed`
    ///
    /// Invalid media types will be silently ignored.
    fn accepts(
        self, 
        media_types: Vec<(&str, Option<f32>)>
    ) -> Self;
}

impl<T> AcceptExt for T 
where 
    T: HeaderSetter,
{
    fn accept(
        self, 
        media_type: &str, 
        q: Option<f32>
    ) -> Self {
        // Parse the media type (format: type/subtype)
        let parts: Vec<&str> = media_type.split('/').collect();
        if parts.len() != 2 {
            return self; // Return self unchanged if format is invalid
        }

        let m_type = parts[0].to_string();
        let m_subtype = parts[1].to_string();

        // Create q value if provided
        let q_value = match q {
            Some(v) => match NotNan::new(v) {
                Ok(nn) => Some(nn),
                Err(_) => None,
            },
            None => None,
        };

        // Create the Accept header with the single media type
        let accept_value = AcceptValue {
            m_type,
            m_subtype,
            q: q_value,
            params: HashMap::new(),
        };

        let header_value = Accept::from_media_types(vec![accept_value]);
        self.set_header(header_value)
    }

    fn accepts(
        self, 
        media_types: Vec<(&str, Option<f32>)>
    ) -> Self {
        // Convert the media types input to the required format
        let mut accept_values = Vec::with_capacity(media_types.len());

        for (media_type, q) in media_types {
            // Parse the media type (format: type/subtype)
            let parts: Vec<&str> = media_type.split('/').collect();
            if parts.len() != 2 {
                continue; // Skip invalid media types
            }

            let m_type = parts[0].to_string();
            let m_subtype = parts[1].to_string();

            // Create q value if provided
            let q_value = match q {
                Some(v) => match NotNan::new(v) {
                    Ok(nn) => Some(nn),
                    Err(_) => None,
                },
                None => None,
            };

            // Create the Accept value
            let accept_value = AcceptValue {
                m_type,
                m_subtype,
                q: q_value,
                params: HashMap::new(),
            };

            accept_values.push(accept_value);
        }

        // If we have no valid media types, just return self
        if accept_values.is_empty() {
            return self;
        }

        // Create the Accept header with all media types
        let header_value = Accept::from_media_types(accept_values);
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    use crate::types::Accept;
    
    #[test]
    fn test_accept_single() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accept("application/sdp", Some(0.8))
            .build();
            
        // Check if Accept header exists with the correct value
        let header = request.header(&HeaderName::Accept);
        assert!(header.is_some(), "Accept header not found");
        
        if let Some(TypedHeader::Accept(accept)) = header {
            // Check if the accept includes "application/sdp"
            assert!(accept.accepts_type("application", "sdp"), "application/sdp not found in Accept header");
            
            // Check the q value
            let media_types = accept.media_types();
            assert_eq!(media_types.len(), 1);
            
            let media_type = &media_types[0];
            assert_eq!(media_type.m_type, "application");
            assert_eq!(media_type.m_subtype, "sdp");
            
            // Check if the q param is present
            let has_q = media_type.q.map(|q| (q.into_inner() - 0.8).abs() < 0.001).unwrap_or(false);
            assert!(has_q, "q parameter with value 0.8 not found");
        } else {
            panic!("Expected Accept header");
        }
    }
    
    #[test]
    fn test_accepts_multiple() {
        let media_types = vec![
            ("application/sdp", Some(1.0)),
            ("application/json", Some(0.5)),
        ];
        
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accepts(media_types)
            .build();
            
        // Check if Accept header exists with the correct values
        let header = request.header(&HeaderName::Accept);
        assert!(header.is_some(), "Accept header not found");
        
        if let Some(TypedHeader::Accept(accept)) = header {
            // Check if the accept includes both media types
            assert!(accept.accepts_type("application", "sdp"), "application/sdp not found in Accept header");
            assert!(accept.accepts_type("application", "json"), "application/json not found in Accept header");
            
            // Check the q values
            let media_types = accept.media_types();
            assert_eq!(media_types.len(), 2);
            
            // Find the application/sdp media type and check its q value
            let sdp_type = media_types.iter().find(|m| m.m_type == "application" && m.m_subtype == "sdp");
            assert!(sdp_type.is_some(), "application/sdp not found in Accept header");
            let has_q = sdp_type.unwrap().q.map(|q| (q.into_inner() - 1.0).abs() < 0.001).unwrap_or(false);
            assert!(has_q, "q parameter with value 1.0 not found for application/sdp");
            
            // Find the application/json media type and check its q value
            let json_type = media_types.iter().find(|m| m.m_type == "application" && m.m_subtype == "json");
            assert!(json_type.is_some(), "application/json not found in Accept header");
            let has_q = json_type.unwrap().q.map(|q| (q.into_inner() - 0.5).abs() < 0.001).unwrap_or(false);
            assert!(has_q, "q parameter with value 0.5 not found for application/json");
        } else {
            panic!("Expected Accept header");
        }
    }
} 