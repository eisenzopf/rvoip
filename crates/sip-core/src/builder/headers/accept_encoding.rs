use crate::error::{Error, Result};
use ordered_float::NotNan;
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::accept_encoding::AcceptEncoding;
use crate::parser::headers::accept_encoding::EncodingInfo;
use crate::types::param::Param;
use super::HeaderSetter;

/// Accept-Encoding Header Builder for SIP Messages
///
/// This module provides builder methods for the Accept-Encoding header in SIP messages,
/// which indicates what content encodings the User Agent can understand.
///
/// ## SIP Accept-Encoding Header Overview
///
/// The Accept-Encoding header is defined in [RFC 3261 Section 20.2](https://datatracker.ietf.org/doc/html/rfc3261#section-20.2)
/// as part of the core SIP protocol. It follows the syntax and semantics defined in 
/// [RFC 2616 Section 14.3](https://datatracker.ietf.org/doc/html/rfc2616#section-14.3) for HTTP.
/// The header specifies which content encodings are acceptable in responses or future messages.
///
/// ## Purpose of Accept-Encoding Header
///
/// The Accept-Encoding header serves several important purposes in SIP:
///
/// 1. It enables the use of compression to reduce bandwidth requirements
/// 2. It allows UAs to indicate which compression algorithms they support
/// 3. It provides a mechanism to express preferences via quality values (q-values)
/// 4. It helps optimize transmission efficiency, particularly for large message bodies
///
/// ## Common Encodings in SIP
///
/// - **gzip**: Standard GZIP compression, widely supported
/// - **deflate**: ZLIB compression format
/// - **compress**: UNIX "compress" program method (less common)
/// - **identity**: No encoding/compression (the content is sent as-is)
/// - **\***: Wildcard to indicate all other encodings not explicitly listed
///
/// ## Quality Values (q-values)
///
/// The Accept-Encoding header can include quality values (q-values) to indicate preference order:
///
/// - Values range from 0.0 to 1.0, with 1.0 being the highest priority
/// - Default value is 1.0 when not specified
/// - A q-value of 0.0 explicitly indicates rejection of that encoding
/// - The wildcard "\*" with q=0.0 rejects all unlisted encodings
///
/// ## Special Considerations
///
/// 1. **Performance Trade-offs**: Compression reduces bandwidth but increases CPU usage
/// 2. **Multiple Headers**: The Accept-Encoding header can appear multiple times in a request
/// 3. **Default Behavior**: If no Accept-Encoding header is present, all encodings are acceptable
/// 4. **Identity Encoding**: Explicitly including "identity" indicates a preference for unencoded content
///
/// ## Relationship with other headers
///
/// - **Accept-Encoding** + **Content-Encoding**: Accept-Encoding specifies what can be received, Content-Encoding specifies what is being sent
/// - **Accept-Encoding** vs **Accept**: Accept is for content types, Accept-Encoding is for compression methods
/// - **Accept-Encoding** works with **Content-Length**: Compression affects message size and thus Content-Length
/// - **Accept-Encoding** considerations with **Max-Forwards**: In constrained networks, appropriate encoding can prevent message fragmentation
///
/// # Examples
///
/// ## Basic Usage with Compression
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptEncodingExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request that accepts gzip-compressed responses
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:recipient@example.com").unwrap()
///     .accept_encoding("gzip", None)  // Accept gzip content with default priority
///     .build();
/// ```
///
/// ## Multiple Encodings with Priorities
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptEncodingExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request that specifies encoding preferences
/// let encodings = vec![
///     ("gzip", Some(1.0)),         // Preferred encoding
///     ("deflate", Some(0.8)),      // Acceptable alternative
///     ("identity", Some(0.5)),     // Unencoded content as fallback
///     ("*", Some(0.0)),            // Reject all other encodings
/// ];
///
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
///     .accept_encodings(encodings)
///     .build();
/// ```
///
/// ## Mobile Client Optimization
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptEncodingExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a SUBSCRIBE request optimized for mobile networks
/// let request = SimpleRequestBuilder::new(Method::Subscribe, "sip:presence@example.com").unwrap()
///     .accept_encoding("gzip", Some(1.0))       // Strongly prefer compression
///     .accept_encoding("deflate", Some(0.8))    // Accept deflate if gzip isn't available
///     .accept_encoding("identity", Some(0.1))   // Strongly discourage uncompressed content
///     .build();
/// ```
///
/// ## When to use Accept-Encoding Headers
///
/// Accept-Encoding headers are particularly useful in the following scenarios:
///
/// 1. **Bandwidth-constrained environments**: Mobile networks, satellite links
/// 2. **Large message bodies**: When transferring substantial content like images or documents
/// 3. **High-latency networks**: Where reducing message size can improve responsiveness
/// 4. **Optimizing battery life**: On mobile devices, compression can reduce transmission energy
/// 5. **Network cost reduction**: For metered connections where data volume has direct cost implications
///
/// ## Best Practices
///
/// - Include Accept-Encoding when bandwidth efficiency matters
/// - Always accept "identity" with some q-value to provide a fallback
/// - Use the wildcard with q=0 to explicitly reject unlisted encodings when necessary
/// - For mobile clients, prioritize compression by assigning higher q-values
/// - Don't request compression for very small messages where overhead outweighs benefits
///
/// # Examples
///
/// ## SIP Client on Limited Bandwidth
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptEncodingExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request from a client on a constrained network
/// let request = SimpleRequestBuilder::new(Method::Options, "sip:service@example.com").unwrap()
///     .from("Mobile Client", "sip:mobile@example.com", Some("tag1234"))
///     .to("Service", "sip:service@example.com", None)
///     // Strongly prefer compressed responses
///     .accept_encoding("gzip", Some(1.0))
///     .accept_encoding("deflate", Some(0.9))
///     .accept_encoding("identity", Some(0.1))  // Discourage uncompressed responses
///     .build();
/// ```
///
/// ## File Transfer Optimization
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptEncodingExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE request that will receive a large file transfer
/// let encodings = vec![
///     ("gzip", Some(1.0)),         // Best compression for large files
///     ("deflate", Some(0.9)),      // Good alternative
///     ("identity", Some(0.2)),     // Accept uncompressed as last resort
/// ];
///
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:file-service@example.com").unwrap()
///     .from("Recipient", "sip:user@example.com", Some("file-req"))
///     .to("File Service", "sip:file-service@example.com", None)
///     .accept_encodings(encodings)  // Specify compression preferences for the response
///     .body("Please send me the requested document")
///     .build();
/// ```
pub trait AcceptEncodingExt {
    /// Add an Accept-Encoding header with a single encoding
    ///
    /// This method specifies a single content encoding that the UA can process,
    /// optionally with a quality value (q-value) to indicate preference when
    /// multiple Accept-Encoding headers are present.
    ///
    /// # Arguments
    ///
    /// * `encoding` - The encoding type (e.g., "gzip", "identity")
    /// * `q` - Optional quality value (0.0 to 1.0, where 1.0 is highest priority)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptEncodingExt};
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a request that accepts gzip-compressed responses
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag5678"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .accept_encoding("gzip", Some(0.9))  // Accept gzip compression with high priority
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 3261 Section 20.2](https://datatracker.ietf.org/doc/html/rfc3261#section-20.2),
    /// the Accept-Encoding header field follows the syntax defined in 
    /// [RFC 2616 Section 14.3](https://datatracker.ietf.org/doc/html/rfc2616#section-14.3),
    /// including the use of q-values to indicate relative preference.
    fn accept_encoding(self, encoding: &str, q: Option<f32>) -> Self;

    /// Add an Accept-Encoding header with multiple encodings
    ///
    /// This method specifies multiple content encodings that the UA can process,
    /// each with an optional quality value to indicate preference order.
    /// This is more efficient than adding multiple individual Accept-Encoding headers.
    ///
    /// # Arguments
    ///
    /// * `encodings` - A vector of tuples containing (encoding, q_value)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptEncodingExt};
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a comprehensive set of encoding preferences
    /// let encodings = vec![
    ///     ("gzip", Some(1.0)),         // Highest priority - prefer gzip
    ///     ("deflate", Some(0.8)),      // Second priority
    ///     ("identity", Some(0.5)),     // Third priority - uncompressed content
    ///     ("*", Some(0.0)),            // Reject all other encodings not listed
    /// ];
    /// 
    /// // Create a request with encoding preferences
    /// let request = SimpleRequestBuilder::new(Method::Subscribe, "sip:presence@example.com").unwrap()
    ///     .from("Watcher", "sip:watcher@example.com", Some("watch123"))
    ///     .to("Presence Service", "sip:presence@example.com", None)
    ///     .accept_encodings(encodings)  // Set all encoding preferences at once
    ///     .build();
    /// ```
    ///
    /// # Special Values
    ///
    /// - **identity**: Represents no encoding/compression (content sent as-is)
    /// - **\***: Wildcard that matches any encoding not explicitly listed
    /// - **q=0.0**: When used with an encoding, explicitly rejects that encoding
    /// - **\* with q=0.0**: Rejects all encodings not explicitly listed
    fn accept_encodings(self, encodings: Vec<(&str, Option<f32>)>) -> Self;
}

impl<T> AcceptEncodingExt for T 
where 
    T: HeaderSetter,
{
    fn accept_encoding(self, encoding: &str, q: Option<f32>) -> Self {
        let mut params = Vec::new();
        
        // Create q value if provided
        if let Some(v) = q {
            if let Ok(nn) = NotNan::new(v) {
                params.push(Param::Q(nn));
            }
        }

        // Create the encoding info
        let encoding_info = EncodingInfo {
            coding: encoding.to_string(),
            params,
        };

        // Create the Accept-Encoding header with the single encoding
        let header_value = AcceptEncoding::from_encodings(vec![encoding_info]);
        self.set_header(header_value)
    }

    fn accept_encodings(self, encodings: Vec<(&str, Option<f32>)>) -> Self {
        // Convert the encodings input to the required format
        let encoding_infos = encodings.into_iter().map(|(encoding, q)| {
            let mut params = Vec::new();
            
            // Create q value if provided
            if let Some(v) = q {
                if let Ok(nn) = NotNan::new(v) {
                    params.push(Param::Q(nn));
                }
            }

            // Create the encoding info
            EncodingInfo {
                coding: encoding.to_string(),
                params,
            }
        }).collect::<Vec<_>>();

        // If we have no encodings, just return self
        if encoding_infos.is_empty() {
            return self;
        }

        // Create the Accept-Encoding header with all encodings
        let header_value = AcceptEncoding::from_encodings(encoding_infos);
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    use crate::types::AcceptEncoding; // Import the actual type
    
    #[test]
    fn test_accept_encoding_single() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accept_encoding("gzip", Some(0.9))
            .build();
            
        // Check if Accept-Encoding header exists with the correct value
        let header = request.header(&HeaderName::AcceptEncoding);
        assert!(header.is_some(), "Accept-Encoding header not found");
        
        if let Some(TypedHeader::AcceptEncoding(accept_encoding)) = header {
            let encodings = accept_encoding.encodings();
            assert_eq!(encodings.len(), 1, "Expected 1 encoding, got {}", encodings.len());
            assert_eq!(encodings[0].coding, "gzip", "Expected encoding 'gzip', got '{}'", encodings[0].coding);
            
            // Check q parameter - need to look through params
            let has_q = encodings[0].params.iter().any(|p| {
                match p {
                    Param::Q(q) => (q.into_inner() - 0.9).abs() < 0.00001,
                    _ => false,
                }
            });
            assert!(has_q, "Q parameter with value 0.9 not found");
        } else {
            panic!("Expected Accept-Encoding header, got {:?}", header);
        }
    }
    
    #[test]
    fn test_accept_encodings_multiple() {
        let encodings = vec![
            ("gzip", Some(1.0)),
            ("identity", Some(0.5)),
            ("*", Some(0.0)), // Reject all other encodings
        ];
        
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accept_encodings(encodings)
            .build();
            
        // Check if Accept-Encoding header exists with the correct values
        let header = request.header(&HeaderName::AcceptEncoding);
        assert!(header.is_some(), "Accept-Encoding header not found");
        
        if let Some(TypedHeader::AcceptEncoding(accept_encoding)) = header {
            let encodings = accept_encoding.encodings();
            assert_eq!(encodings.len(), 3, "Expected 3 encodings, got {}", encodings.len());
            
            // Check for gzip with q=1.0
            let has_gzip = encodings.iter().any(|enc| {
                enc.coding == "gzip" && enc.params.iter().any(|p| {
                    match p {
                        Param::Q(q) => (q.into_inner() - 1.0).abs() < 0.00001,
                        _ => false,
                    }
                })
            });
            assert!(has_gzip, "gzip encoding not found with q=1.0");
            
            // Check for identity with q=0.5
            let has_identity = encodings.iter().any(|enc| {
                enc.coding == "identity" && enc.params.iter().any(|p| {
                    match p {
                        Param::Q(q) => (q.into_inner() - 0.5).abs() < 0.00001,
                        _ => false,
                    }
                })
            });
            assert!(has_identity, "identity encoding not found with q=0.5");
            
            // Check for wildcard with q=0.0
            let has_wildcard = encodings.iter().any(|enc| {
                enc.coding == "*" && enc.params.iter().any(|p| {
                    match p {
                        Param::Q(q) => q.into_inner() < 0.00001,
                        _ => false,
                    }
                })
            });
            assert!(has_wildcard, "* encoding not found with q=0.0");
        } else {
            panic!("Expected Accept-Encoding header");
        }
    }
} 