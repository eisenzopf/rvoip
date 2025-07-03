use crate::types::{
    call_id::CallId,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Call-ID header builder
///
/// This trait provides builder methods for the Call-ID header in SIP messages, as defined
/// in [RFC 3261 Section 20.8](https://datatracker.ietf.org/doc/html/rfc3261#section-20.8).
///
/// ## SIP Call-ID Header Overview
///
/// The Call-ID header uniquely identifies a particular invitation or registration.
/// It serves as a globally unique identifier for all messages within a dialog or registration.
/// The Call-ID, combined with From and To tags, forms the dialog identifier.
///
/// Call-ID headers are critical for:
/// - Dialog identification - tying together messages in the same session
/// - Registration identification - grouping all registrations from a client
/// - Loop detection - preventing message loops in SIP networks
/// - Troubleshooting - enabling call tracing across network elements
///
/// ## Call-ID Format
///
/// The Call-ID has the following format:
/// - A unique identifier, typically a random UUID or hash
/// - Optionally followed by '@' and a domain name or host identifier
/// - Example: `f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com`
///
/// ## Generation Guidelines
///
/// When generating Call-IDs, consider:
/// - Using cryptographically random UUIDs to ensure global uniqueness
/// - Including a host identifier to aid in troubleshooting
/// - Maintaining consistency within a dialog (all messages must use the same Call-ID)
/// - Generating a new Call-ID for each new call or registration
///
/// ## Common Use Cases
///
/// - **Initial INVITE**: Establishing a new dialog with a unique identifier
/// - **In-dialog requests**: ACK, BYE, REFER sharing the same Call-ID
/// - **REGISTER**: Grouping multiple registrations from the same client
/// - **Forking scenarios**: Tracking multiple branches of the same call
/// - **Third-party call control**: Managing multiple related dialogs
///
/// ## Real-world Applications
///
/// - **Call tracking**: Tracing call flows across systems for troubleshooting
/// - **CDR generation**: Using Call-ID for billing record correlation
/// - **Fraud detection**: Identifying unusual calling patterns
/// - **Network diagnostics**: Following signaling paths through proxies
///
/// # Examples
///
/// ## Enterprise PBX Call Establishment
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::CallIdBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Enterprise PBX initiating a call with traceable Call-ID
///
/// // Create an INVITE with a structured Call-ID containing system information
/// let invite = SimpleRequestBuilder::invite("sip:bob@branch.example.com").unwrap()
///     .from("Alice", "sip:alice@hq.example.com", Some("a84b4c"))
///     .to("Bob", "sip:bob@branch.example.com", None)
///     .contact("<sip:alice@192.0.2.1:5060>", None)
///     // Call-ID includes system identifier and timestamp for tracking
///     .call_id("pbx1-1596485021-a73b@hq.example.com")
///     .build();
///
/// // This structured Call-ID helps locate call records in logs
/// ```
///
/// ## Registration Refresh
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::CallIdBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: SIP phone refreshing its registration
///
/// // Create a REGISTER request using the same Call-ID as initial registration
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("reg123"))
///     .to("User", "sip:user@example.com", None)
///     .contact("<sip:user@10.1.2.3:5060>", None)
///     // Using the same Call-ID links this with previous registrations
///     .call_id("89fjh4-293rt2-9823ar-av3n4@phone-model-123")
///     .build();
///
/// // The registrar will update the existing registration due to matching Call-ID
/// ```
///
/// ## SIP Response with Original Call-ID
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::CallIdBuilderExt;
/// use rvoip_sip_core::builder::headers::FromBuilderExt;
/// use rvoip_sip_core::builder::headers::ToBuilderExt;
/// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
/// use rvoip_sip_core::types::{StatusCode, Method};
///
/// // Scenario: SIP proxy responding to a request
///
/// // Create a 200 OK response that maintains the original Call-ID
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.com", Some("b38dlam6"))
///     .cseq_with_method(101, Method::Invite)
///     // Must use the same Call-ID from the request
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
///     .build();
///
/// // Using the same Call-ID allows the client to match the response to its request
/// ```
pub trait CallIdBuilderExt {
    /// Add a Call-ID header with a specific value
    ///
    /// Creates and adds a Call-ID header with the specified value. The Call-ID 
    /// uniquely identifies a particular invitation or registration and is used
    /// to correlate all messages within a dialog.
    ///
    /// # Parameters
    /// - `call_id`: The Call-ID value (e.g., "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::CallIdBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a BYE request using the same Call-ID as the original INVITE
    /// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b38dlam6")) // To tag present in dialog
    ///     // Must use the same Call-ID from the original INVITE
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
    ///     .build();
    ///
    /// // The same Call-ID ensures this BYE is associated with the correct dialog
    /// ```
    fn call_id(self, call_id: &str) -> Self;

    /// Add a randomly generated Call-ID header
    ///
    /// This method sets a Call-ID header with a randomly generated UUID,
    /// providing a high probability of global uniqueness. This is ideal for
    /// initiating new dialogs or registrations where a unique identifier is required.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::CallIdBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a new INVITE with a random Call-ID for a new dialog
    /// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .contact("<sip:alice@192.0.2.10:5060>", None)
    ///     // Generate a random UUID as Call-ID
    ///     .random_call_id()
    ///     .build();
    ///
    /// // The random Call-ID ensures this is a new dialog, not related to any existing ones
    /// ```
    fn random_call_id(self) -> Self;

    /// Add a randomly generated Call-ID header with a host part
    ///
    /// This method sets a Call-ID header with a random UUID and appends
    /// a host part, following the recommended format in RFC 3261. This format
    /// helps with troubleshooting by identifying the originating host.
    ///
    /// # Parameters
    /// - `host`: The host part to append (domain name or IP address)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::CallIdBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a new SUBSCRIBE with a random Call-ID including system domain
    /// let subscribe = SimpleRequestBuilder::new(Method::Subscribe, "sip:presence@example.com").unwrap()
    ///     .from("Alice", "sip:alice@company.example", Some("sub456"))
    ///     .to("Presence Service", "sip:presence@example.com", None)
    ///     .contact("<sip:alice@192.0.2.20:5060>", None)
    ///     // Random Call-ID with domain for troubleshooting
    ///     .random_call_id_with_host("company.example")
    ///     .build();
    ///
    /// // The domain in the Call-ID helps identify which system originated the request
    /// ```
    fn random_call_id_with_host(self, host: &str) -> Self;
}

impl CallIdBuilderExt for SimpleRequestBuilder {
    fn call_id(self, call_id: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::new(call_id)))
    }
    
    fn random_call_id(self) -> Self {
        self.header(TypedHeader::CallId(CallId::random()))
    }
    
    fn random_call_id_with_host(self, host: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::random_with_host(host)))
    }
}

impl CallIdBuilderExt for SimpleResponseBuilder {
    fn call_id(self, call_id: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::new(call_id)))
    }
    
    fn random_call_id(self) -> Self {
        self.header(TypedHeader::CallId(CallId::random()))
    }
    
    fn random_call_id_with_host(self, host: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::random_with_host(host)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    use crate::types::headers::HeaderAccess;
    use crate::builder::headers::cseq::CSeqBuilderExt;

    #[test]
    fn test_request_call_id_header() {
        let call_id_value = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@host.example.com";
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_id(call_id_value)
            .build();
            
        let call_id_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        assert_eq!(call_id_headers[0].value(), call_id_value);
    }
    
    #[test]
    fn test_response_call_id_header() {
        let call_id_value = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@host.example.com";
        
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id(call_id_value)
            .cseq_with_method(101, Method::Invite)
            .build();
            
        let call_id_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        assert_eq!(call_id_headers[0].value(), call_id_value);
    }
    
    #[test]
    fn test_request_with_random_call_id() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .random_call_id()
            .build();
            
        let call_id_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        // Check it's not empty and is a valid UUID
        let value = call_id_headers[0].value();
        assert!(!value.is_empty());
        assert!(uuid::Uuid::parse_str(&value).is_ok());
    }

    #[test]
    fn test_request_with_random_call_id_with_host() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .random_call_id_with_host("example.com")
            .build();
            
        let call_id_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        let value = call_id_headers[0].value();
        let parts: Vec<&str> = value.split('@').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1], "example.com");
        // Check the first part is a valid UUID
        assert!(uuid::Uuid::parse_str(parts[0]).is_ok());
    }
} 