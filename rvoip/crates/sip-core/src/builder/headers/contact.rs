use std::str::FromStr;

use crate::types::{
    uri::Uri,
    contact::{Contact, ContactParamInfo},
    Address,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding Contact headers to SIP message builders.
///
/// This trait provides a standard way to add Contact headers to both request and response builders
/// as specified in [RFC 3261 Section 20.10](https://datatracker.ietf.org/doc/html/rfc3261#section-20.10).
/// The Contact header field provides a URI that can be used to directly contact the user agent for subsequent requests.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContactBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .contact("sip:alice@192.168.1.1:5060", Some("Alice"))
///     .build();
/// ```
pub trait ContactBuilderExt {
    /// Add a Contact header.
    ///
    /// Creates and adds a Contact header with the specified URI and optional display name.
    ///
    /// # Parameters
    /// - `uri`: The URI for the Contact header (e.g., "sip:alice@192.168.1.1:5060")
    /// - `display_name`: Optional display name (e.g., "Alice")
    ///
    /// # Returns
    /// Self for method chaining
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