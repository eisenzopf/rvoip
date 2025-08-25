use crate::error::{Error, Result};
use crate::types::uri::Uri;
use crate::types::address::Address;
use crate::types::headers::header::Header;
use crate::types::headers::header_name::HeaderName;
use crate::types::referred_by::ReferredBy;
use crate::types::headers::typed_header::TypedHeaderTrait;
use crate::types::headers::TypedHeader;
use super::HeaderSetter;

/// # Referred-By Header Extension
///
/// This module provides extension traits for easily adding Referred-By headers
/// to SIP requests and responses. The Referred-By header is defined in 
/// [RFC 3892](https://datatracker.ietf.org/doc/html/rfc3892) and is used
/// to identify the entity that requested the current referral.
///
/// ## Purpose of Referred-By Header
///
/// The Referred-By header serves several important purposes in SIP:
///
/// 1. It provides identity information about the referring party in a REFER transaction
/// 2. It offers a more secure way to communicate who initiated a referral
/// 3. It can include an authenticated identity using S/MIME body parts
/// 4. It helps distinguish between different referral scenarios (e.g., call transfers)
///
/// ## Structure and Format
///
/// The Referred-By header contains an address specification (URI and optional display name)
/// that identifies the referring party. It can optionally include a "cid" parameter
/// that references an S/MIME body part containing a signature.
///
/// ## Common Use Cases
///
/// - **Call Transfer Scenarios**: Identify who initiated a call transfer
/// - **Consultation Transfers**: Track the origin of a consultation transfer
/// - **Click-to-Dial Applications**: Identify the web page or service initiating a call
/// - **Call Centers**: Track who transferred a call to which agent
///
/// ## Example usage:
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
/// 
/// // Create a new request builder
/// let mut request = RequestBuilder::new(Method::Refer, "sip:alice@example.com").unwrap();
/// 
/// // Add a Referred-By header with a URI
/// let uri = Uri::from_str("sip:bob@example.com").unwrap();
/// request = request.referred_by_uri(uri);
/// 
/// // Or add a Referred-By header with Address (display name + URI)
/// let uri = Uri::from_str("sip:carol@example.com").unwrap();
/// let address = Address::new_with_display_name("Carol", uri);
/// request = request.referred_by(address);
/// 
/// // Build the request
/// let refer_request = request.build();
/// ```
/// Extension trait for adding Referred-By headers to SIP messages.
///
/// This trait extends the message builders to make it easier to add
/// Referred-By headers in various formats. The Referred-By header identifies
/// the entity that requested the current referral as defined in RFC 3892.
///
/// ## SIP Referred-By Header Overview
///
/// The Referred-By header is used in SIP call transfer scenarios to identify
/// the party who initiated the referral. It is particularly important for:
///
/// - Providing identity assurance for the referred-to party
/// - Enabling proper attribution of referrals in multi-party scenarios
/// - Supporting call tracking in enterprise environments
/// - Enabling cryptographic verification of the referrer's identity
///
/// ## Relationship with other headers
///
/// - **Referred-By** vs **Refer-To**: Referred-By identifies who initiated the referral, 
///   while Refer-To specifies the target of the referral
/// - **Referred-By** vs **From**: When a REFER is sent, From identifies the sender, 
///   while Referred-By may identify a third party on whose behalf the referral is made
///
/// # Examples
///
/// ## Basic Call Transfer
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Scenario: Alice transfers Bob to Carol
///
/// // Create a REFER request to Bob, referring him to Carol
/// let refer = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
///     // Add Alice as the referring party
///     .referred_by_uri(Uri::from_str("sip:alice@example.com").unwrap())
///     .build();
///
/// // Now Bob knows that Alice initiated this referral
/// ```
///
/// ## Call Transfer with Display Name
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Scenario: A helpdesk transfers a customer to a specialist
///
/// // Create a REFER request with display name
/// let uri = Uri::from_str("sip:helpdesk@example.com").unwrap();
/// let agent = Address::new_with_display_name("Helpdesk Agent #42", uri);
///
/// let refer = RequestBuilder::new(Method::Refer, "sip:customer@example.com").unwrap()
///     // Add helpdesk agent identity with display name
///     .referred_by(agent)
///     .build();
///
/// // The customer's phone can now display who transferred them
/// ```
pub trait ReferredByExt {
    /// Add a Referred-By header with the given Address.
    ///
    /// This method adds a Referred-By header containing both URI and optional 
    /// display name to identify the referring party.
    ///
    /// # Parameters
    ///
    /// - `address`: An Address containing a URI and optional display name
    ///
    /// # Returns
    ///
    /// The modified builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::prelude::*;
    /// # use std::str::FromStr;
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new_with_display_name("Alice Smith", uri);
    ///
    /// let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .referred_by(address)
    ///     .build();
    /// ```
    fn referred_by(self, address: Address) -> Self;

    /// Add a Referred-By header with the given URI.
    ///
    /// This is a convenience method that creates an Address with just the URI
    /// to identify the referring party without a display name.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI to use for the Referred-By header
    ///
    /// # Returns
    ///
    /// The modified builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::prelude::*;
    /// # use std::str::FromStr;
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    ///
    /// let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .referred_by_uri(uri)
    ///     .build();
    /// ```
    fn referred_by_uri(self, uri: Uri) -> Self;

    /// Add a Referred-By header parsed from a string.
    ///
    /// This method parses the provided string as a Referred-By header value,
    /// which should be a properly formatted SIP address.
    ///
    /// # Parameters
    ///
    /// - `value`: A string representing the Referred-By header value (e.g., "<sip:alice@example.com>")
    ///
    /// # Returns
    ///
    /// The modified builder for method chaining, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::prelude::*;
    /// let result = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .referred_by_str("<sip:alice@example.com>");
    /// 
    /// match result {
    ///     Ok(builder) => {
    ///         let request = builder.build();
    ///         // Process request
    ///     }
    ///     Err(e) => tracing::error!("Failed to parse Referred-By header: {}", e),
    /// }
    /// ```
    fn referred_by_str(self, value: &str) -> Result<Self> where Self: Sized;
    
    /// Add a Referred-By header with URI and optional cid parameter.
    ///
    /// This method adds a Referred-By header with a URI and optionally specifies
    /// a Content-ID (cid) parameter referencing an S/MIME body part containing
    /// signature information for authentication purposes.
    ///
    /// # Parameters
    ///
    /// - `uri`: The URI to use for the Referred-By header
    /// - `cid`: Optional Content-ID parameter value (without the 'cid=' prefix)
    ///
    /// # Returns
    ///
    /// The modified builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::prelude::*;
    /// # use std::str::FromStr;
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    ///
    /// let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .referred_by_uri_with_cid(uri, Some("12345@example.com"))
    ///     .build();
    /// ```
    fn referred_by_uri_with_cid(self, uri: Uri, cid: Option<&str>) -> Self;
    
    /// Add a Referred-By header with Address and optional cid parameter.
    ///
    /// This method adds a Referred-By header with an Address (URI and optional display name)
    /// and optionally specifies a Content-ID (cid) parameter for authentication purposes.
    ///
    /// # Parameters
    ///
    /// - `address`: An Address containing a URI and optional display name
    /// - `cid`: Optional Content-ID parameter value (without the 'cid=' prefix)
    ///
    /// # Returns
    ///
    /// The modified builder for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_sip_core::prelude::*;
    /// # use std::str::FromStr;
    /// let uri = Uri::from_str("sip:alice@example.com").unwrap();
    /// let address = Address::new_with_display_name("Alice", uri);
    ///
    /// let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
    ///     .referred_by_with_cid(address, Some("12345@example.com"))
    ///     .build();
    /// ```
    fn referred_by_with_cid(self, address: Address, cid: Option<&str>) -> Self;
}

impl<T> ReferredByExt for T 
where 
    T: HeaderSetter,
{
    fn referred_by(self, address: Address) -> Self {
        let referred_by = ReferredBy::new(address);
        self.set_header(referred_by)
    }

    fn referred_by_uri(self, uri: Uri) -> Self {
        let address = Address::new(uri);
        self.referred_by(address)
    }

    fn referred_by_str(self, value: &str) -> Result<Self> where Self: Sized {
        let referred_by = value.parse::<ReferredBy>()?;
        Ok(self.set_header(referred_by))
    }
    
    fn referred_by_uri_with_cid(self, uri: Uri, cid: Option<&str>) -> Self {
        let address = Address::new(uri);
        self.referred_by_with_cid(address, cid)
    }
    
    fn referred_by_with_cid(self, address: Address, cid: Option<&str>) -> Self {
        let mut referred_by = ReferredBy::new(address);
        
        if let Some(cid_value) = cid {
            referred_by = referred_by.with_cid(cid_value);
        }
        
        self.set_header(referred_by)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_with_referred_by() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let address = Address::new_with_display_name("Alice", uri);
        
        let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .referred_by(address)
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::ReferredBy(referred_by)) = request.header(&HeaderName::ReferredBy) {
            assert_eq!(referred_by.address().display_name().unwrap(), "Alice");
            assert_eq!(referred_by.address().uri().to_string(), "sip:alice@example.com");
        } else {
            panic!("ReferredBy header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_referred_by_uri() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        
        let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .referred_by_uri(uri)
            .build();
            
        if let Some(TypedHeader::ReferredBy(referred_by)) = request.header(&HeaderName::ReferredBy) {
            assert!(referred_by.address().display_name().is_none());
            assert_eq!(referred_by.address().uri().to_string(), "sip:alice@example.com");
        } else {
            panic!("ReferredBy header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_referred_by_str() {
        let result = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .referred_by_str("<sip:alice@example.com>");
        
        let request = match result {
            Ok(builder) => builder.build(),
            Err(e) => panic!("Failed to parse ReferredBy: {}", e),
        };
            
        if let Some(TypedHeader::ReferredBy(referred_by)) = request.header(&HeaderName::ReferredBy) {
            assert!(referred_by.address().display_name().is_none());
            assert_eq!(referred_by.address().uri().to_string(), "sip:alice@example.com");
        } else {
            panic!("ReferredBy header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_request_with_referred_by_and_cid() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let address = Address::new_with_display_name("Alice", uri);
        
        let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .referred_by_with_cid(address, Some("12345@example.com"))
            .build();
            
        if let Some(TypedHeader::ReferredBy(referred_by)) = request.header(&HeaderName::ReferredBy) {
            assert_eq!(referred_by.address().display_name().unwrap(), "Alice");
            assert_eq!(referred_by.address().uri().to_string(), "sip:alice@example.com");
            assert_eq!(referred_by.cid().unwrap(), "12345@example.com");
        } else {
            panic!("ReferredBy header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_request_with_referred_by_uri_and_cid() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        
        let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .referred_by_uri_with_cid(uri, Some("12345@example.com"))
            .build();
            
        if let Some(TypedHeader::ReferredBy(referred_by)) = request.header(&HeaderName::ReferredBy) {
            assert!(referred_by.address().display_name().is_none());
            assert_eq!(referred_by.address().uri().to_string(), "sip:alice@example.com");
            assert_eq!(referred_by.cid().unwrap(), "12345@example.com");
        } else {
            panic!("ReferredBy header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_response_with_referred_by() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        let address = Address::new_with_display_name("Alice", uri);
        
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .referred_by(address)
            .build();
            
        if let Some(TypedHeader::ReferredBy(referred_by)) = response.header(&HeaderName::ReferredBy) {
            assert_eq!(referred_by.address().display_name().unwrap(), "Alice");
        } else {
            panic!("ReferredBy header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_chained_header_operations() {
        // Test that the ReferredByExt methods can be chained with other header operations
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        
        let request = RequestBuilder::new(Method::Refer, "sip:bob@example.com").unwrap()
            .referred_by_uri(uri)
            .max_forwards(70) // Add another header
            .build();
        
        // Verify both headers are present
        assert!(request.header(&HeaderName::ReferredBy).is_some());
        assert!(request.header(&HeaderName::MaxForwards).is_some());
    }
} 