use crate::error::{Error, Result};
use crate::types::{
    organization::Organization,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Organization header builder
///
/// This module provides builder methods for the Organization header in SIP messages.
///
/// ## SIP Organization Header Overview
///
/// The Organization header is defined in [RFC 3261 Section 20.27](https://datatracker.ietf.org/doc/html/rfc3261#section-20.27)
/// as part of the core SIP protocol. It conveys the name of the organization to which the
/// entity issuing the request or response belongs.
///
/// ## Format
///
/// ```text
/// Organization: Rudeless Ventures
/// ```
///
/// The value is a simple text string representing the organization name.
///
/// ## Purpose of Organization Header
///
/// The Organization header serves primarily informational purposes in SIP:
///
/// 1. It identifies the organizational entity associated with the user agent
/// 2. It can be used by the recipient's user agent to display organization information about the caller or callee
/// 3. It helps with call tracking and forensics in enterprise environments
/// 4. It can be useful for policy decisions in some deployments
///
/// ## Examples
///
/// ## Enterprise PBX Identifying Itself
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::OrganizationBuilderExt};
///
/// // Scenario: Enterprise PBX registering with a SIP service provider
///
/// // Create a REGISTER request with organization information
/// let register = SimpleRequestBuilder::new(Method::Register, "sip:sip-provider.com").unwrap()
///     .from("PBX", "sip:pbx@company.example.com", Some("reg123"))
///     .to("PBX", "sip:pbx@company.example.com", None)
///     .contact("<sip:pbx@192.168.1.2:5060>", None)
///     // Set organization information
///     .organization("Example Corporation")
///     .build();
///
/// // The service provider now knows which organization this PBX belongs to
/// ```
///
/// ## Outbound Call with Company Information
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::OrganizationBuilderExt};
///
/// // Scenario: Making an outbound call with company information attached
///
/// // Create an INVITE request that includes organization information
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:recipient@example.com").unwrap()
///     .from("Caller", "sip:caller@company.example.com", Some("call456"))
///     .to("Recipient", "sip:recipient@example.com", None)
///     // Add organization information that may be displayed to the recipient
///     .organization("Rudeless Ventures")
///     .build();
///
/// // The recipient's phone may display "Incoming call from Rudeless Ventures"
/// ```
pub trait OrganizationBuilderExt {
    /// Add an Organization header with the specified organization name
    ///
    /// This method adds an Organization header with the given organization name.
    /// The organization name is typically the company or institution name.
    ///
    /// # Parameters
    ///
    /// * `name` - The organization name as a string
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Organization header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::OrganizationBuilderExt};
    ///
    /// // Create a request with organization information
    /// let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
    ///     .from("User", "sip:user@company.example.com", Some("tag123"))
    ///     .to("User", "sip:user@company.example.com", None)
    ///     .organization("Example Corporation")
    ///     .build();
    ///
    /// // The SIP message now includes the organization name
    /// ```
    fn organization<S: Into<String>>(self, name: S) -> Self;
}

impl<T> OrganizationBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn organization<S: Into<String>>(self, name: S) -> Self {
        let org = Organization::new(name);
        self.set_header(org)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_organization() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .organization("Example Corporation")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Organization(org)) = request.header(&HeaderName::Organization) {
            assert_eq!(org.as_str(), "Example Corporation");
        } else {
            panic!("Organization header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_organization() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .organization("Rudeless Ventures")
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Organization(org)) = response.header(&HeaderName::Organization) {
            assert_eq!(org.as_str(), "Rudeless Ventures");
        } else {
            panic!("Organization header not found or has wrong type");
        }
    }

    #[test]
    fn test_organization_empty() {
        // Test with empty organization value
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .organization("")
            .build();
            
        if let Some(TypedHeader::Organization(org)) = request.header(&HeaderName::Organization) {
            assert_eq!(org.as_str(), "");
        } else {
            panic!("Organization header not found or has wrong type");
        }
    }

    #[test]
    fn test_organization_display() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .organization("Example Corporation")
            .build();
            
        if let Some(TypedHeader::Organization(org)) = request.header(&HeaderName::Organization) {
            assert_eq!(org.to_string(), "Example Corporation");
        } else {
            panic!("Organization header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_multiple_organization() {
        // In the actual header processing, headers are added to a list
        // rather than replacing the previous value
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .organization("First Organization")
            .organization("Second Organization") // This adds another Organization header
            .build();
            
        // Check that the header exists in the list
        if let Some(TypedHeader::Organization(org)) = request.header(&HeaderName::Organization) {
            // The exact behavior depends on the header extraction implementation
            // In our current implementation, we're getting the first header, not the last
            assert_eq!(org.as_str(), "First Organization");
        } else {
            panic!("Organization header not found or has wrong type");
        }
    }
    
    #[test]
    fn test_organization_with_special_chars() {
        let special_org = "Acme, Inc. & Partners (2023)";
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .organization(special_org)
            .build();
            
        if let Some(TypedHeader::Organization(org)) = request.header(&HeaderName::Organization) {
            assert_eq!(org.as_str(), special_org);
        } else {
            panic!("Organization header not found or has wrong type");
        }
    }
} 