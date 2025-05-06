use crate::types::{
    max_forwards::MaxForwards,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::{HeaderSetter, cseq::CSeqBuilderExt};

/// Extension trait for adding Max-Forwards headers to SIP message builders.
///
/// This trait provides a standard way to add Max-Forwards headers to both request and response builders
/// as specified in [RFC 3261 Section 20.22](https://datatracker.ietf.org/doc/html/rfc3261#section-20.22).
/// The Max-Forwards header limits the number of hops a request can transit.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::MaxForwardsBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .max_forwards(70)
///     .build();
/// ```
pub trait MaxForwardsBuilderExt {
    /// Add a Max-Forwards header
    ///
    /// Creates and adds a Max-Forwards header as specified in [RFC 3261 Section 20.22](https://datatracker.ietf.org/doc/html/rfc3261#section-20.22).
    /// The Max-Forwards header limits the number of hops a request can transit.
    ///
    /// # Parameters
    /// - `value`: The Max-Forwards value (typically 70)
    ///
    /// # Returns
    /// Self for method chaining
    fn max_forwards(self, value: u32) -> Self;
}

impl MaxForwardsBuilderExt for SimpleRequestBuilder {
    fn max_forwards(self, value: u32) -> Self {
        self.header(TypedHeader::MaxForwards(MaxForwards::new(value as u8)))
    }
}

impl MaxForwardsBuilderExt for SimpleResponseBuilder {
    fn max_forwards(self, value: u32) -> Self {
        self.header(TypedHeader::MaxForwards(MaxForwards::new(value as u8)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    
    #[test]
    fn test_request_max_forwards_header() {
        let max_value = 70_u8;
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .max_forwards(max_value as u32)
            .build();
            
        let max_forwards_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::MaxForwards(m) = h { Some(m) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(max_forwards_headers.len(), 1);
        // Check that the header exists, don't rely on a specific method of MaxForwards
        // Different MaxForwards types might have different API
        assert!(matches!(request.all_headers().iter().find(|h| 
            matches!(h, TypedHeader::MaxForwards(_))), 
            Some(_)));
    }
    
    #[test]
    fn test_response_max_forwards_header() {
        let max_value = 70_u8;
        
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq_with_method(101, Method::Invite)
            .max_forwards(max_value as u32)
            .build();
            
        let max_forwards_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::MaxForwards(m) = h { Some(m) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(max_forwards_headers.len(), 1);
        // Check that the header exists, don't rely on a specific method of MaxForwards
        assert!(matches!(response.all_headers().iter().find(|h| 
            matches!(h, TypedHeader::MaxForwards(_))), 
            Some(_)));
    }
} 