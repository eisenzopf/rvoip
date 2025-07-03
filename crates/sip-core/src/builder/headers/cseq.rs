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
/// 
/// ## Purpose and Importance
/// 
/// The CSeq header serves multiple critical functions in SIP:
/// 
/// - **Transaction Identification**: Along with Call-ID and other headers, it uniquely identifies SIP transactions
/// - **Request Ordering**: Allows recipients to properly order requests
/// - **Request-Response Matching**: Enables matching responses to their corresponding requests
/// - **Retransmission Detection**: Helps identify retransmitted messages versus new requests
/// 
/// The CSeq header contains two parts:
/// 1. A sequence number (32-bit unsigned integer)
/// 2. A request method that matches the method of the request
///
/// ## Usage Guidelines
/// 
/// - For new dialogs, start with a low number (typically 1)
/// - Increment the sequence number for each subsequent request within the same dialog
/// - Responses must echo the same CSeq number and method as the request they're answering
/// - For forked requests, each fork maintains the same CSeq
/// - ACK and CANCEL use the same CSeq number as the request they reference
///
/// # Examples
///
/// ## Basic Request Example
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
/// ```
///
/// ## Response Example
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::CSeqBuilderExt;
/// use rvoip_sip_core::types::Method;
/// 
/// // For responses, both sequence number and method must be provided
/// let response = SimpleResponseBuilder::ok()
///     .cseq_with_method(1, Method::Invite)
///     .build();
/// ```
///
/// ## Real-World Dialog Example
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::{CSeqBuilderExt, FromBuilderExt, ToBuilderExt, CallIdBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Step 1: Initial INVITE with CSeq = 1
/// let call_id = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com";
/// let alice_tag = "1928301774";
/// 
/// let invite = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some(alice_tag))
///     .to("Bob", "sip:bob@biloxi.example.com", None)
///     .call_id(call_id)
///     .cseq(1) // Initial CSeq for the dialog
///     .build();
///
/// // Step 2: 200 OK response maintains the same CSeq
/// let bob_tag = "a6c85cf";
/// 
/// let ok_response = SimpleResponseBuilder::ok()
///     .from("Alice", "sip:alice@atlanta.example.com", Some(alice_tag))
///     .to("Bob", "sip:bob@biloxi.example.com", Some(bob_tag)) // Response adds To tag
///     .call_id(call_id)
///     .cseq_with_method(1, Method::Invite) // Same as the request
///     .build();
///
/// // Step 3: ACK request with same CSeq as INVITE
/// let ack = SimpleRequestBuilder::new(Method::Ack, "sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some(alice_tag))
///     .to("Bob", "sip:bob@biloxi.example.com", Some(bob_tag))
///     .call_id(call_id)
///     .cseq(1) // ACK uses same CSeq number as the INVITE it acknowledges
///     .build();
///
/// // Step 4: Later BYE request with incremented CSeq
/// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some(alice_tag))
///     .to("Bob", "sip:bob@biloxi.example.com", Some(bob_tag))
///     .call_id(call_id)
///     .cseq(2) // Increment CSeq for new request in the dialog
///     .build();
///
/// // Step 5: 200 OK response to BYE
/// let bye_ok = SimpleResponseBuilder::ok()
///     .from("Alice", "sip:alice@atlanta.example.com", Some(alice_tag))
///     .to("Bob", "sip:bob@biloxi.example.com", Some(bob_tag))
///     .call_id(call_id)
///     .cseq_with_method(2, Method::Bye) // Same as the BYE request
///     .build();
/// ```
///
/// ## Handling Mid-Dialog Requests
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{CSeqBuilderExt, FromBuilderExt, ToBuilderExt, CallIdBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Mid-dialog INFO request with an updated CSeq
/// let info = SimpleRequestBuilder::new(Method::Info, "sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@biloxi.example.com", Some("a6c85cf"))
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
///     .cseq(3) // Next sequential number after previous requests
///     .build();
/// ```
pub trait CSeqBuilderExt {
    /// Add a CSeq header with specified sequence number
    ///
    /// For requests, this method automatically uses the request method in the CSeq header.
    /// For responses, this method is less commonly used as responses should match both
    /// the sequence number and method of the request they're answering.
    ///
    /// # Parameters
    /// - `seq`: The sequence number (e.g., 1, 2, 3)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::CSeqBuilderExt;
    ///
    /// // New INVITE with CSeq 1
    /// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .cseq(1)
    ///     .build();
    /// ```
    fn cseq(self, seq: u32) -> Self;

    /// Add a CSeq header with specified sequence number and method
    ///
    /// This method is particularly important for responses, which must echo both
    /// the sequence number and method from the request being answered.
    /// For requests, it can be used when the CSeq method needs to differ from
    /// the request method (rare but sometimes needed for special cases).
    ///
    /// # Parameters
    /// - `seq`: The sequence number (e.g., 1, 2, 3)
    /// - `method`: The SIP method to use in the CSeq
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::CSeqBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // 200 OK response to an INVITE
    /// let response = SimpleResponseBuilder::ok()
    ///     .cseq_with_method(101, Method::Invite)
    ///     .build();
    /// ```
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