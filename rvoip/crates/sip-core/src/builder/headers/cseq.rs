use crate::types::{
    Method,
    cseq::CSeq,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding CSeq headers to SIP message builders.
///
/// This trait provides a standard way to add CSeq headers to both request and response builders
/// as specified in [RFC 3261 Section 20.16](https://datatracker.ietf.org/doc/html/rfc3261#section-20.16).
/// The CSeq header serves as a way to identify and order transactions.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::CSeqBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // For requests, the method is automatically set from the request method
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .cseq(1)
///     .build();
///
/// // For responses, both sequence number and method must be provided
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// 
/// let response = SimpleResponseBuilder::ok()
///     .cseq_with_method(1, Method::Invite)
///     .build();
/// ```
pub trait CSeqBuilderExt {
    /// Add a CSeq header with specified sequence number
    ///
    /// # Parameters
    /// - `seq`: The sequence number (e.g., 1, 2, 3)
    ///
    /// # Returns
    /// Self for method chaining
    fn cseq(self, seq: u32) -> Self;

    /// Add a CSeq header with specified sequence number and method
    ///
    /// # Parameters
    /// - `seq`: The sequence number (e.g., 1, 2, 3)
    /// - `method`: The SIP method to use in the CSeq
    ///
    /// # Returns
    /// Self for method chaining
    fn cseq_with_method(self, seq: u32, method: Method) -> Self;
}

impl CSeqBuilderExt for SimpleRequestBuilder {
    fn cseq(self, seq: u32) -> Self {
        // For requests, use the request's method
        let method = self.method().clone();
        self.header(TypedHeader::CSeq(CSeq::new(seq, method)))
    }

    fn cseq_with_method(self, seq: u32, method: Method) -> Self {
        // This variant allows specifying a custom method different from the request method
        self.header(TypedHeader::CSeq(CSeq::new(seq, method)))
    }
}

impl CSeqBuilderExt for SimpleResponseBuilder {
    fn cseq(self, seq: u32) -> Self {
        // This is a simplified version, but responses typically require a method too
        // So this is mainly for compatibility
        // In practice, responses should use cseq_with_method instead
        self.cseq_with_method(seq, Method::Extension("".to_string()))
    }

    fn cseq_with_method(self, seq: u32, method: Method) -> Self {
        self.header(TypedHeader::CSeq(CSeq::new(seq, method)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StatusCode;
    
    #[test]
    fn test_request_cseq_header() {
        // Test with default method (from request)
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .cseq(42)
            .build();
            
        let cseq_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CSeq(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(cseq_headers.len(), 1);
        assert_eq!(cseq_headers[0].sequence(), 42);
        assert_eq!(cseq_headers[0].method(), &Method::Invite);
        
        // Test with explicitly provided method
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .cseq_with_method(43, Method::Options)
            .build();
            
        let cseq_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CSeq(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(cseq_headers.len(), 1);
        assert_eq!(cseq_headers[0].sequence(), 43);
        assert_eq!(cseq_headers[0].method(), &Method::Options);
    }
    
    #[test]
    fn test_response_cseq_header() {
        // Test with method 
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq_with_method(101, Method::Invite)
            .build();
            
        let cseq_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CSeq(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(cseq_headers.len(), 1);
        assert_eq!(cseq_headers[0].sequence(), 101);
        assert_eq!(cseq_headers[0].method(), &Method::Invite);
        
        // Test simple cseq for responses (less common)
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq_with_method(102, Method::Extension("".to_string()))
            .build();
            
        let cseq_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CSeq(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(cseq_headers.len(), 1);
        assert_eq!(cseq_headers[0].sequence(), 102);
        // Should use Method::Extension
        assert!(matches!(cseq_headers[0].method(), Method::Extension(_)));
    }
} 