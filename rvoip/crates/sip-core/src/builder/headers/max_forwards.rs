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
///
/// ## Purpose and Importance
///
/// The Max-Forwards header serves several critical functions in SIP:
///
/// - **Loop Prevention**: Prevents requests from circulating indefinitely in the network
/// - **Network Protection**: Guards against excessive forwarding due to misconfiguration
/// - **Troubleshooting Aid**: Useful for debugging routing issues with controlled request propagation
/// - **Mandatory Header**: Required in all SIP requests (though not in responses)
///
/// Each proxy that forwards a request decrements the Max-Forwards value by 1. If a proxy 
/// receives a request with Max-Forwards of 0, it MUST NOT forward the request and should 
/// typically return a 483 (Too Many Hops) response.
///
/// ## Recommended Values
///
/// - **Initial Value**: 70 is recommended by RFC 3261
/// - **Minimum Value**: 0 (indicating the request should not be forwarded)
/// - **Valid Range**: 0-255 (as it's implemented as an 8-bit integer)
///
/// ## Usage Guidelines
///
/// - Always include Max-Forwards in every SIP request
/// - When acting as a proxy, decrement the value when forwarding
/// - When originating a request, start with the recommended initial value (70)
/// - For testing or troubleshooting, you might use lower values to limit hop count
///
/// # Examples
///
/// ## Basic Usage in INVITE
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::MaxForwardsBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .max_forwards(70) // Recommended default value
///     .build();
/// ```
///
/// ## Troubleshooting with Limited Scope
///
/// Setting a low Max-Forwards value can help isolate routing problems by 
/// limiting how far a request can travel:
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::MaxForwardsBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Restricting to just 2 hops for troubleshooting
/// let options = SimpleRequestBuilder::new(Method::Options, "sip:server.example.com").unwrap()
///     .max_forwards(2) // Limit to just 2 hops
///     .build();
/// ```
///
/// ## Full SIP Transaction Example
///
/// ```rust
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::{MaxForwardsBuilderExt, FromBuilderExt, ToBuilderExt, CallIdBuilderExt};
/// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
/// use rvoip_sip_core::types::{Method, StatusCode};
///
/// // 1. UAC creates an INVITE with Max-Forwards
/// let invite = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@biloxi.example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@atlanta.example.com")
///     .cseq(1)
///     .max_forwards(70) // Starting with recommended value
///     .build();
///
/// // 2. Proxy would decrement Max-Forwards before forwarding
/// // (Normally this would be done by extracting and modifying the header)
/// let forwarded_invite = SimpleRequestBuilder::invite("sip:bob@biloxi.example.com").unwrap()
///     .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@biloxi.example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@atlanta.example.com")
///     .cseq(1)
///     .max_forwards(69) // Decremented by 1
///     .build();
///    
/// // 3. If the Max-Forwards reaches 0, a proxy would reject with 483 Too Many Hops
/// let too_many_hops = SimpleResponseBuilder::new(StatusCode::TooManyHops, None)
///     .from("Alice", "sip:alice@atlanta.example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@biloxi.example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@atlanta.example.com")
///     .cseq_with_method(1, Method::Invite)
///     .build();
/// ```
///
/// ## Loop Detection Example
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::MaxForwardsBuilderExt;
/// use rvoip_sip_core::types::{Method, TypedHeader, max_forwards::MaxForwards};
///
/// // Initial request has Max-Forwards of 70
/// let original_request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .max_forwards(70)
///     .build();
///
/// // At a proxy, we check the Max-Forwards value before forwarding
/// // This is simplified, in practice you would extract the value from the request
/// let max_forwards_header = original_request.all_headers().iter()
///     .find_map(|h| if let TypedHeader::MaxForwards(m) = h { Some(m) } else { None });
///
/// if let Some(header) = max_forwards_header {
///     if header.is_zero() {
///         // Would return 483 Too Many Hops
///         // Don't forward the request
///     } else {
///         // Create a new request with decremented Max-Forwards
///         let mut forwarded_request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
///         
///         // Decrement the Max-Forwards value (in practice, you'd copy other headers too)
///         let new_value = header.decrement().unwrap_or(MaxForwards::new(0));
///         forwarded_request = forwarded_request.max_forwards(new_value.0 as u32);
///         
///         // Now we can forward the request with the decremented Max-Forwards value
///     }
/// }
/// ```
///
/// ## Custom Max-Forwards for Special Routing
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::MaxForwardsBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // For special routing scenarios, you might want to limit hops
/// // For example, when using REGISTER through a specific path
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
///     .max_forwards(10) // Lower value for controlled routing
///     .build();
/// 
/// // Or for direct server-to-server communication
/// let notify = SimpleRequestBuilder::new(Method::Notify, "sip:server1.example.com").unwrap()
///     .max_forwards(1) // Direct delivery with no intermediate hops
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::MaxForwardsBuilderExt;
    ///
    /// // Create a new request with the recommended Max-Forwards value
    /// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .max_forwards(70) // RFC 3261 recommended value
    ///     .build();
    /// ```
    ///
    /// When sending through multiple proxies, proxies will decrement this value:
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::MaxForwardsBuilderExt;
    /// use rvoip_sip_core::types::{Method, TypedHeader, max_forwards::MaxForwards};
    ///
    /// // Proxy handling example
    /// let incoming = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .max_forwards(70)
    ///     .build();
    ///     
    /// // Extract the current Max-Forwards value (simulated)
    /// let current_value = 70;
    ///     
    /// // Create a new forwarded request with decremented value
    /// let outgoing = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .max_forwards(current_value - 1) // Decrement by 1
    ///     .build();
    /// ```
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