use crate::error::{Error, Result};
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::content_encoding::ContentEncoding;
use super::HeaderSetter;

/// Extension trait for adding Content-Encoding header building capabilities
pub trait ContentEncodingExt {
    /// Add a Content-Encoding header with a single encoding
    ///
    /// # Arguments
    ///
    /// * `encoding` - The content encoding to specify
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentEncodingExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_encoding("gzip")
    ///     .build();
    /// ```
    fn content_encoding(self, encoding: &str) -> Self;

    /// Add a Content-Encoding header with multiple encodings
    ///
    /// # Arguments
    ///
    /// * `encodings` - A slice of content encodings to specify
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentEncodingExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_encodings(&["gzip", "deflate"])
    ///     .build();
    /// ```
    fn content_encodings<T: AsRef<str>>(self, encodings: &[T]) -> Self;
}

impl<T> ContentEncodingExt for T 
where 
    T: HeaderSetter,
{
    fn content_encoding(self, encoding: &str) -> Self {
        let header_value = ContentEncoding::single(encoding);
        self.set_header(header_value)
    }

    fn content_encodings<S: AsRef<str>>(self, encodings: &[S]) -> Self {
        let header_value = ContentEncoding::with_encodings(encodings);
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    use crate::types::ContentEncoding; // Import the actual type
    
    #[test]
    fn test_content_encoding_single() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_encoding("gzip")
            .build();
            
        // Check if Content-Encoding header exists with the correct value
        let header = request.header(&HeaderName::ContentEncoding);
        assert!(header.is_some(), "Content-Encoding header not found");
        
        if let Some(TypedHeader::ContentEncoding(content_encoding)) = header {
            // Check if the content encoding includes "gzip"
            assert!(content_encoding.has_encoding("gzip"), "gzip encoding not found");
            assert_eq!(content_encoding.encodings().len(), 1);
        } else {
            panic!("Expected Content-Encoding header");
        }
    }
    
    #[test]
    fn test_content_encodings_multiple() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_encodings(&["gzip", "deflate"])
            .build();
            
        // Check if Content-Encoding header exists with the correct values
        let header = request.header(&HeaderName::ContentEncoding);
        assert!(header.is_some(), "Content-Encoding header not found");
        
        if let Some(TypedHeader::ContentEncoding(content_encoding)) = header {
            // Check if the content encoding includes both "gzip" and "deflate"
            assert!(content_encoding.has_encoding("gzip"), "gzip encoding not found");
            assert!(content_encoding.has_encoding("deflate"), "deflate encoding not found");
            assert_eq!(content_encoding.encodings().len(), 2);
        } else {
            panic!("Expected Content-Encoding header");
        }
    }
} 