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

/// Extension trait for adding Accept-Encoding header building capabilities
pub trait AcceptEncodingExt {
    /// Add an Accept-Encoding header with a single encoding
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
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .accept_encoding("gzip", Some(0.9))
    ///     .build();
    /// ```
    fn accept_encoding(self, encoding: &str, q: Option<f32>) -> Self;

    /// Add an Accept-Encoding header with multiple encodings
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
    /// 
    /// let encodings = vec![
    ///     ("gzip", Some(1.0)),
    ///     ("identity", Some(0.5)),
    ///     ("*", Some(0.0)), // Reject all other encodings
    /// ];
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .accept_encodings(encodings)
    ///     .build();
    /// ```
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