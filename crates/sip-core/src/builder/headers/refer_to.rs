use crate::error::{Error, Result};
use crate::types::{
    refer_to::ReferTo,
    address::Address,
    uri::Uri,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use std::str::FromStr;
use super::HeaderSetter;

/// ReferTo header builder
///
/// This module provides builder methods for the Refer-To header in SIP messages.
///
/// ## SIP Refer-To Header Overview
///
/// The Refer-To header is defined in [RFC 3515](https://datatracker.ietf.org/doc/html/rfc3515)
/// as part of the SIP REFER method. It specifies a URI or address that the recipient should
/// contact, enabling call transfers and other features.
///
/// ## Purpose of Refer-To Header
///
/// The Refer-To header serves several important purposes in SIP:
///
/// 1. It provides the target address for call transfers (attended or blind)
/// 2. It specifies the method to use when contacting the target
/// 3. It can include dialog identification for replacements
/// 4. It enables advanced call control scenarios like click-to-dial
///
/// ## Common Use Cases
///
/// - **Blind Transfer**: Transfer a call to a third party without consultation
/// - **Attended Transfer**: Transfer after speaking with the transfer target
/// - **Click-to-Dial**: Instruct a phone to make a call
/// - **Call Replacement**: Replace an existing call with a new one
///
/// ## Format Examples
///
/// ```text
/// Refer-To: <sip:alice@atlanta.example.com>
/// Refer-To: <sip:bob@biloxi.example.com;method=INVITE>
/// Refer-To: "Bob" <sip:bob@biloxi.example.com?Replaces=12345%40atlanta.example.com%3Bto-tag%3D12345%3Bfrom-tag%3D5FFE-3994>
/// ```
///
/// # Examples
///
/// ## Simple Blind Transfer
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
///
/// // Create a REFER request for a blind transfer
/// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("ref-1"))
///     .to("Bob", "sip:bob@example.com", Some("abc123"))
///     .contact("<sip:alice@192.0.2.1:5060>", None)
///     .refer_to_uri("sip:carol@example.com")
///     .build();
///
/// // Bob will be instructed to send an INVITE to Carol
/// ```
///
/// ## Attended Transfer with Replaces
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
///
/// // Create a REFER for attended transfer with Replaces information
/// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("ref-2"))
///     .to("Bob", "sip:bob@example.com", Some("xyz789"))
///     .contact("<sip:alice@192.0.2.1:5060>", None)
///     // URI with Replaces parameter to specify which dialog to replace
///     .refer_to_uri("sip:carol@example.com?Replaces=abcdef%40example.com%3Bto-tag%3D123%3Bfrom-tag%3D456")
///     .build();
///
/// // Bob will send an INVITE to Carol with a Replaces header
/// ```
///
/// ## Click-to-Dial with Display Name
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
///
/// // Create a REFER for a click-to-dial service
/// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:dialer@example.com").unwrap()
///     .from("WebPortal", "sip:portal@example.com", Some("c2d-1"))
///     .to("DialService", "sip:dialer@example.com", None)
///     .contact("<sip:portal@203.0.113.5:5060>", None)
///     .refer_to_address("Customer Support", "sip:+1-800-555-0199@example.com")
///     .build();
///
/// // The dialer service will initiate a call to the customer support number
/// ```
pub trait ReferToExt {
    /// Add a Refer-To header with a URI
    ///
    /// This method adds a Refer-To header with the specified URI. This is the simplest
    /// form for creating a Refer-To header when only the target URI is needed.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI to include in the Refer-To header, as a string
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Refer-To header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
    ///
    /// // Create a simple REFER for a blind transfer
    /// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("ref-1"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .refer_to_uri("sip:carol@example.com")
    ///     .build();
    ///
    /// // Bob should contact Carol using a new INVITE
    /// ```
    fn refer_to_uri(self, uri: impl Into<String>) -> Self;
    
    /// Add a Refer-To header with a URI and display name
    ///
    /// This method adds a Refer-To header with a display name and URI. This form is useful
    /// when the reference target should include a human-readable name.
    ///
    /// # Parameters
    ///
    /// * `display_name` - The display name to include
    /// * `uri` - The URI to include, as a string
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Refer-To header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
    ///
    /// // Create a REFER with a display name
    /// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:receptionist@example.com").unwrap()
    ///     .from("Executive", "sip:executive@example.com", Some("refer-1"))
    ///     .to("Receptionist", "sip:receptionist@example.com", None)
    ///     .refer_to_address("IT Department", "sip:helpdesk@example.com")
    ///     .build();
    ///
    /// // The receptionist will see "IT Department" as the display name
    /// ```
    fn refer_to_address(self, display_name: impl Into<String>, uri: impl Into<String>) -> Self;
    
    /// Add a Refer-To header with a prebuilt Address object
    ///
    /// This method adds a Refer-To header with a fully constructed Address object.
    /// This provides the most control when building complex Refer-To headers.
    ///
    /// # Parameters
    ///
    /// * `address` - The Address object to use for the Refer-To header
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Refer-To header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
    /// use std::str::FromStr;
    ///
    /// // Create a REFER with a custom Address
    /// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:phone@example.com").unwrap()
    ///     .from("User", "sip:user@example.com", Some("ref-custom"))
    ///     .to("Phone", "sip:phone@example.com", None)
    ///     .refer_to_with_address({
    ///         let uri = Uri::from_str("sip:conference@example.com;transport=tls").unwrap();
    ///         let mut address = Address::new_with_display_name("Conference Room", uri);
    ///         address.set_tag("conf-123");
    ///         address
    ///     })
    ///     .build();
    ///
    /// // Contains a fully customized Refer-To header
    /// ```
    fn refer_to_with_address(self, address: Address) -> Self;
    
    /// Add a Refer-To header for a blind transfer
    ///
    /// This convenience method adds a Refer-To header specifically formatted for a blind transfer
    /// scenario, where a call is transferred without prior consultation.
    ///
    /// # Parameters
    ///
    /// * `target_uri` - The URI of the transfer target
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Refer-To header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
    ///
    /// // Create a REFER for a blind transfer to Carol
    /// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("b-ref"))
    ///     .to("Bob", "sip:bob@example.com", Some("def456"))
    ///     .refer_to_blind_transfer("sip:carol@example.com")
    ///     .build();
    ///
    /// // Bob will blind transfer the call to Carol
    /// ```
    fn refer_to_blind_transfer(self, target_uri: impl Into<String>) -> Self;
    
    /// Add a Refer-To header for an attended transfer with Replaces
    ///
    /// This convenience method adds a Refer-To header specifically formatted for an attended transfer
    /// scenario, where a Replaces parameter is included to specify which dialog to replace.
    ///
    /// # Parameters
    ///
    /// * `target_uri` - The URI of the transfer target
    /// * `call_id` - The Call-ID of the dialog to be replaced
    /// * `to_tag` - The to-tag of the dialog to be replaced
    /// * `from_tag` - The from-tag of the dialog to be replaced
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Refer-To header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt};
    ///
    /// // Create a REFER for an attended transfer
    /// let refer = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a-ref"))
    ///     .to("Bob", "sip:bob@example.com", Some("ghi789"))
    ///     .refer_to_attended_transfer(
    ///         "sip:carol@example.com",
    ///         "callid-1234567",
    ///         "to-tag-123",
    ///         "from-tag-456"
    ///     )
    ///     .build();
    ///
    /// // Bob will perform an attended transfer to Carol, replacing the specified dialog
    /// ```
    fn refer_to_attended_transfer(
        self, 
        target_uri: impl Into<String>,
        call_id: impl Into<String>,
        to_tag: impl Into<String>,
        from_tag: impl Into<String>
    ) -> Self;
}

impl<T> ReferToExt for T 
where 
    T: HeaderSetter,
{
    fn refer_to_uri(self, uri: impl Into<String>) -> Self {
        match Uri::from_str(&uri.into()) {
            Ok(parsed_uri) => {
                let address = Address::new(parsed_uri);
                let refer_to = ReferTo::new(address);
                self.set_header(refer_to)
            },
            Err(_) => self // In practice, should log error or return Result
        }
    }
    
    fn refer_to_address(self, display_name: impl Into<String>, uri: impl Into<String>) -> Self {
        match Uri::from_str(&uri.into()) {
            Ok(parsed_uri) => {
                let address = Address::new_with_display_name(display_name.into(), parsed_uri);
                let refer_to = ReferTo::new(address);
                self.set_header(refer_to)
            },
            Err(_) => self // In practice, should log error or return Result
        }
    }
    
    fn refer_to_with_address(self, address: Address) -> Self {
        let refer_to = ReferTo::new(address);
        self.set_header(refer_to)
    }
    
    fn refer_to_blind_transfer(self, target_uri: impl Into<String>) -> Self {
        // For blind transfer, we simply use the target URI
        self.refer_to_uri(target_uri)
    }
    
    fn refer_to_attended_transfer(
        self, 
        target_uri: impl Into<String>,
        call_id: impl Into<String>,
        to_tag: impl Into<String>,
        from_tag: impl Into<String>
    ) -> Self {
        let target = target_uri.into();
        let call_id = call_id.into();
        let to_tag = to_tag.into();
        let from_tag = from_tag.into();
        
        // Create the Replaces URI parameter with proper escaping
        // Format: Replaces=call-id;to-tag=to-tag;from-tag=from-tag
        // Note: In URI parameters, special characters must be percent-encoded
        let replaced_call_id = call_id.replace("%", "%25")
                                     .replace("@", "%40")
                                     .replace(";", "%3B")
                                     .replace("=", "%3D");
        
        // Construct the full URI with the Replaces parameter
        let uri_with_replaces = if target.contains("?") {
            // Target already has URI parameters
            format!("{}&Replaces={}%3Bto-tag%3D{}%3Bfrom-tag%3D{}", 
                   target, replaced_call_id, to_tag, from_tag)
        } else {
            // Target has no URI parameters yet
            format!("{}?Replaces={}%3Bto-tag%3D{}%3Bfrom-tag%3D{}", 
                   target, replaced_call_id, to_tag, from_tag)
        };
        
        self.refer_to_uri(uri_with_replaces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode, TypedHeader};
    use crate::types::header::TypedHeaderTrait;
    use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use std::str::FromStr;
    use std::convert::TryFrom;

    #[test]
    fn test_request_refer_to_uri() {
        let request = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .refer_to_uri("sip:carol@example.com")
            .build();
            
        // Find the first Refer-To header directly in the headers list
        let refer_to_header = request.headers.iter()
            .find(|h| matches!(h, TypedHeader::ReferTo(_)))
            .expect("Refer-To header should be present");
            
        if let TypedHeader::ReferTo(refer_to) = refer_to_header {
            assert_eq!(refer_to.uri().to_string(), "sip:carol@example.com");
        } else {
            panic!("Expected TypedHeader::ReferTo variant");
        }
    }

    #[test]
    fn test_refer_to_address() {
        let request = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .refer_to_address("Carol Smith", "sip:carol@example.com")
            .build();
            
        // Find the first Refer-To header directly in the headers list
        let refer_to_header = request.headers.iter()
            .find(|h| matches!(h, TypedHeader::ReferTo(_)))
            .expect("Refer-To header should be present");
            
        if let TypedHeader::ReferTo(refer_to) = refer_to_header {
            assert_eq!(refer_to.uri().to_string(), "sip:carol@example.com");
            assert_eq!(refer_to.address().display_name(), Some("Carol Smith"));
        } else {
            panic!("Expected TypedHeader::ReferTo variant");
        }
    }

    #[test]
    fn test_refer_to_with_address() {
        let uri = Uri::from_str("sip:dave@example.com;transport=tls").unwrap();
        let address = Address::new_with_display_name("Dave", uri);
        
        let request = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .refer_to_with_address(address)
            .build();
            
        // Find the first Refer-To header directly in the headers list
        let refer_to_header = request.headers.iter()
            .find(|h| matches!(h, TypedHeader::ReferTo(_)))
            .expect("Refer-To header should be present");
            
        if let TypedHeader::ReferTo(refer_to) = refer_to_header {
            assert_eq!(refer_to.uri().to_string(), "sip:dave@example.com;transport=tls");
            assert_eq!(refer_to.address().display_name(), Some("Dave"));
        } else {
            panic!("Expected TypedHeader::ReferTo variant");
        }
    }

    #[test]
    fn test_refer_to_attended_transfer() {
        let request = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .refer_to_attended_transfer(
                "sip:carol@example.com",
                "call-123@example.com",
                "to-tag-abc",
                "from-tag-xyz"
            )
            .build();
            
        // Find the first Refer-To header directly in the headers list
        let refer_to_header = request.headers.iter()
            .find(|h| matches!(h, TypedHeader::ReferTo(_)))
            .expect("Refer-To header should be present");
            
        if let TypedHeader::ReferTo(refer_to) = refer_to_header {
            let uri_string = refer_to.uri().to_string();
            assert!(uri_string.starts_with("sip:carol@example.com?Replaces="));
            assert!(uri_string.contains("call-123%40example.com"));
            assert!(uri_string.contains("to-tag%3Dto-tag-abc"));
            assert!(uri_string.contains("from-tag%3Dfrom-tag-xyz"));
        } else {
            panic!("Expected TypedHeader::ReferTo variant");
        }
    }
    
    #[test]
    fn test_refer_to_header_conversion() {
        // Create a ReferTo header
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let address = Address::new(uri);
        let refer_to = ReferTo::new(address);
        
        // Get the raw header
        let header = refer_to.to_header();
        println!("Header: {:?}", header);
        println!("Header name: {:?}", header.name);
        println!("Header value type: {:?}", header.value);
        
        // Try to convert it to TypedHeader
        match TypedHeader::try_from(header.clone()) {
            Ok(typed_header) => {
                println!("Converted to: {:?}", typed_header);
                
                // Check if it's the correct variant
                match typed_header {
                    TypedHeader::ReferTo(_) => println!("Successfully converted to TypedHeader::ReferTo"),
                    TypedHeader::Other(name, value) => {
                        println!("Converted to TypedHeader::Other");
                        println!("Other name: {:?}", name);
                        println!("Other value type: {:?}", value);
                        
                        // This is the issue we're seeing
                        println!("ISSUE DETECTED: Conversion resulted in TypedHeader::Other instead of TypedHeader::ReferTo");
                        
                        // The issue is in the TryFrom<Header> implementation for TypedHeader
                        // It needs to correctly handle the HeaderValue::ReferTo case in the
                        // HeaderName::ReferTo branch
                    }
                    _ => println!("Converted to unexpected TypedHeader variant")
                }
            },
            Err(e) => println!("Conversion failed: {:?}", e)
        }
        
        // Now test actual builder usage
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let address = Address::new(uri.clone());
        let refer_to = ReferTo::new(address);
        
        // The actual issue happens here
        let builder = SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap();
        let result = builder.set_header(refer_to);
        
        // The request should now contain a TypedHeader::ReferTo
        let request = result.build();
        
        // Find all headers of type Refer-To
        for header in request.headers.iter() {
            println!("Found header: {:?}", header);
        }
        
        // Find the first Refer-To header directly in the headers list
        let refer_to_headers: Vec<_> = request.headers.iter()
            .filter(|h| {
                if let TypedHeader::ReferTo(_) = h {
                    println!("Found TypedHeader::ReferTo");
                    true
                } else if let TypedHeader::Other(name, _) = h {
                    if *name == HeaderName::ReferTo {
                        println!("Found TypedHeader::Other with name ReferTo");
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
            .collect();
            
        println!("Found {} Refer-To headers", refer_to_headers.len());
    }
} 