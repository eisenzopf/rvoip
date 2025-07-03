use std::str::FromStr;

use crate::types::{
    uri::Uri,
    contact::{Contact, ContactParamInfo},
    Address,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Contact header builder
///
/// This trait provides builder methods for the Contact header in SIP messages, as defined
/// in [RFC 3261 Section 20.10](https://datatracker.ietf.org/doc/html/rfc3261#section-20.10).
///
/// ## SIP Contact Header Overview
///
/// The Contact header provides a URI that can be used to contact the user agent directly
/// for subsequent requests. Unlike the From and To headers which identify logical entities,
/// the Contact header contains a direct routable address.
///
/// Contact headers are critical for:
/// - Dialog routing - determining where subsequent in-dialog requests should be sent
/// - User location - registering user's actual location with a registrar
/// - NAT traversal - providing reachable addresses for behind-NAT endpoints
/// - Load balancing - directing specific calls to specific servers
///
/// ## Common Parameters
///
/// The Contact header may include important parameters:
/// - **expires**: Duration in seconds that the contact is valid
/// - **q-value**: Priority when multiple contacts are provided
/// - **transport**: Protocol to use (e.g., UDP, TCP, TLS)
/// - **methods**: Methods supported at this contact address
/// - **+sip.instance**: Unique instance ID for the UA
///
/// ## Common Use Cases
///
/// - **REGISTER**: Binding a user's AOR to their actual IP and port
/// - **INVITE**: Providing the address for mid-dialog communication
/// - **REFER**: Specifying where the referred call should be established
/// - **SUBSCRIBE**: Indicating where NOTIFY requests should be sent
/// - **3xx responses**: Providing alternative contacts for redirection
///
/// ## Real-world Applications
///
/// - **SIP phone registration**: Mapping user identities to physical devices
/// - **NAT traversal**: Enabling communication with endpoints behind firewalls
/// - **Media negotiation**: Establishing dialog paths for RTP traffic
/// - **Call centers**: Routing calls to specific agent endpoints
///
/// # Examples
///
/// ## SIP Phone Registration
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContactBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: IP phone registering with a SIP registrar
///
/// // Create a REGISTER request with device contact information
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
///     .from("Alice Smith", "sip:alice@example.com", Some("reg78392"))
///     .to("Alice Smith", "sip:alice@example.com", None)
///     // Actual IP and port where the phone can be reached
///     .contact("<sip:alice@192.0.2.1:5060;transport=tcp>;+sip.instance=\"<urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF>\";expires=3600", None)
///     .build();
///
/// // The registrar will store this binding for 3600 seconds
/// ```
///
/// ## SIP Trunk with TLS
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContactBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: PBX making a call through a secure SIP trunk
///
/// // Create an INVITE with secure contact information
/// let invite = SimpleRequestBuilder::invite("sip:+15551234567@sip-trunk.example.net").unwrap()
///     .from("Company Name", "sip:+14085550100@company.example", Some("pbx45678"))
///     .to("Customer", "sip:+15551234567@sip-trunk.example.net", None)
///     // Secure connection details for return signaling
///     .contact("<sip:pbx@203.0.113.50:5061;transport=tls>", None)
///     .build();
///
/// // The SIP trunk will send responses back to the secure TLS address
/// ```
///
/// ## SIP Proxy Response
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::ContactBuilderExt;
/// use rvoip_sip_core::builder::headers::FromBuilderExt;
/// use rvoip_sip_core::builder::headers::ToBuilderExt;
/// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
/// use rvoip_sip_core::types::{StatusCode, Method};
///
/// // Scenario: SIP proxy redirecting a call with multiple options
///
/// // Create a 302 Moved Temporarily response with contact options
/// let response = SimpleResponseBuilder::new(StatusCode::MovedTemporarily, None)
///     .from("Reception", "sip:reception@company.example", Some("orig-tag"))
///     .to("Caller", "sip:user@example.net", Some("resp-tag"))
///     .cseq_with_method(101, Method::Invite)
///     // Primary contact with high priority
///     .contact("<sip:alice-mobile@203.0.113.5:5060>;q=1.0", Some("Alice Mobile"))
///     .build();
///
/// // The caller will try the highest q-value contact first
/// ```
pub trait ContactBuilderExt {
    /// Add a Contact header.
    ///
    /// Creates and adds a Contact header with the specified URI and optional display name.
    /// The Contact header provides the exact address where subsequent requests should be sent,
    /// and is crucial for dialog establishment, registration, and proper routing.
    ///
    /// # Parameters
    /// - `uri`: The URI for the Contact header (e.g., "sip:alice@192.168.1.1:5060" or "<sip:alice@192.168.1.1:5060;transport=tls>")
    /// - `display_name`: Optional display name (e.g., "Alice Smith")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContactBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a SIP message with a WebRTC gateway contact
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("msg123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     // WebSocket transport for WebRTC gateway
    ///     .contact("<sip:alice@webrtc-gw.example.com:443;transport=ws>", None)
    ///     .build();
    ///
    /// // Responses will be sent to the WebSocket address
    /// ```
    fn contact(self, uri: &str, display_name: Option<&str>) -> Self;
}

impl ContactBuilderExt for SimpleRequestBuilder {
    fn contact(self, uri: &str, display_name: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                // Create an address with or without display name
                let address = match display_name {
                    Some(name) => Address::new_with_display_name(name, uri),
                    None => Address::new(uri)
                };
                
                // Create a contact param with the address
                let contact_param = ContactParamInfo { address };
                let contact = Contact::new_params(vec![contact_param]);
                
                self.header(TypedHeader::Contact(contact))
            },
            Err(_) => {
                // Silently fail - contact is not critical
                self
            }
        }
    }
}

impl ContactBuilderExt for SimpleResponseBuilder {
    fn contact(self, uri: &str, display_name: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                // Create an address with or without display name
                let address = match display_name {
                    Some(name) => Address::new_with_display_name(name, uri),
                    None => Address::new(uri)
                };
                
                // Create a contact param with the address
                let contact_param = ContactParamInfo { address };
                let contact = Contact::new_params(vec![contact_param]);
                
                self.header(TypedHeader::Contact(contact))
            },
            Err(_) => {
                // Silently fail - contact is not critical
                self
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    
    #[test]
    fn test_request_contact_header() {
        // Test with valid URI and display name
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .contact("sip:alice@192.168.1.1:5060", Some("Alice"))
            .build();
            
        let contact_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Contact(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(contact_headers.len(), 1);
        let address = contact_headers[0].address().unwrap();
        assert_eq!(address.display_name(), Some("Alice"));
        assert_eq!(address.uri().to_string(), "sip:alice@192.168.1.1:5060");
        
        // Test without display name
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .contact("sip:alice@192.168.1.1:5060", None)
            .build();
            
        let contact_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Contact(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(contact_headers.len(), 1);
        let address = contact_headers[0].address().unwrap();
        assert_eq!(address.display_name(), None);
        assert_eq!(address.uri().to_string(), "sip:alice@192.168.1.1:5060");
        
        // Test with invalid URI (should not add the header)
        let initial_headers_count = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap().build().all_headers().len();
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .contact("invalid-uri", Some("Alice"))
            .build();
            
        assert_eq!(request.all_headers().len(), initial_headers_count);
    }
    
    #[test]
    fn test_response_contact_header() {
        // Test with valid URI and display name
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .contact("sip:bob@192.168.1.2:5060", Some("Bob"))
            .build();
            
        let contact_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Contact(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(contact_headers.len(), 1);
        let address = contact_headers[0].address().unwrap();
        assert_eq!(address.display_name(), Some("Bob"));
        assert_eq!(address.uri().to_string(), "sip:bob@192.168.1.2:5060");
        
        // Test without display name
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .contact("sip:bob@192.168.1.2:5060", None)
            .build();
            
        let contact_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::Contact(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(contact_headers.len(), 1);
        let address = contact_headers[0].address().unwrap();
        assert_eq!(address.display_name(), None);
        assert_eq!(address.uri().to_string(), "sip:bob@192.168.1.2:5060");
        
        // Test with invalid URI (should not add the header)
        let initial_response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .build();
            
        let initial_headers_count = initial_response.all_headers().len();
        
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .contact("invalid-uri", Some("Bob"))
            .build();
            
        assert_eq!(response.all_headers().len(), initial_headers_count);
    }
} 