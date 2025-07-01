use crate::error::{Error, Result};
use crate::types::{
    header::{Header, HeaderName},
    headers::TypedHeader,
    reply_to::ReplyTo,
    address::Address,
    uri::Uri,
};
use crate::builder::headers::HeaderSetter;
use std::str::FromStr;

/// Reply-To header builder
///
/// This module provides builder methods for the Reply-To header,
/// which specifies where the user would prefer responses to be sent,
/// as defined in RFC 3261 Section 20.31.
///
/// ## SIP Reply-To Header Overview
///
/// The Reply-To header suggests an address where the recipient should send replies,
/// which may differ from the From address. Unlike Contact (which affects dialog routing),
/// Reply-To is a suggestion that can be ignored by recipients.
///
/// ## Common Use Cases
///
/// - **Support teams**: Directing replies to a specific support address
/// - **Call centers**: Routing return calls to appropriate departments
/// - **Delegated communication**: When sending on behalf of someone else
/// - **Group addresses**: Sending from a group but directing replies to an individual
/// - **Load balancing**: Distributing replies across multiple response handlers
///
/// ## Real-world Applications
///
/// - **Enterprise environments**: Department-specific response routing
/// - **Customer service**: Directing replies to specific teams
/// - **Conferencing systems**: Managing reply routing for multiparty sessions
/// - **Automated systems**: Directing human responses to appropriate handlers
///
/// # Examples
///
/// Basic usage with a string URI:
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with a Reply-To header
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .from("Sender", "sip:sender@example.com", None)
///     .to("Recipient", "sip:recipient@example.com", None)
///     .reply_to("sip:support@example.com").unwrap()
///     .build();
/// ```
///
/// With a display name:
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with a Reply-To header including a display name
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
///     .from("Sales", "sip:sales@example.com", None)
///     .to("Customer", "sip:customer@example.net", None)
///     .reply_to_with_display_name("Support Team", "sip:support@example.com").unwrap()
///     .build();
/// ```

/// Extension trait that adds Reply-To building capabilities to request and response builders
///
/// The Reply-To header allows SIP messages to indicate a preferred address for replies,
/// which may be different from the From or Contact addresses. This is particularly useful
/// in scenarios where replies should be directed to a different entity than the sender.
///
/// # Reply-To Header Usage
///
/// 1. **Alternate reply address**: Directing responses to a different address
/// 2. **Team-based routing**: Routing replies to appropriate teams or departments
/// 3. **Delegation**: Handling replies when sending on behalf of others
///
/// # Examples
///
/// ## Enterprise Department Routing
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Sales department sending a message but routing replies to support
///
/// // Create an INVITE with department-specific reply routing
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:customer@example.net").unwrap()
///     .from("Sales Department", "sip:sales@company.example.com", Some("sales1"))
///     .to("Customer", "sip:customer@example.net", None)
///     .contact("<sip:sales@192.0.2.1:5060>", None)
///     // Direct replies to the support team instead of sales
///     .reply_to_with_display_name("Customer Support", "sip:support@company.example.com").unwrap()
///     .build();
///
/// // The customer's reply will be suggested to go to support instead of sales
/// ```
///
/// ## Call Center Agent Transfer
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Call center agent transferring a call but maintaining thread continuity
///
/// // Create an INVITE for a transferred call
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:customer@example.com").unwrap()
///     .from("Agent Smith", "sip:agent42@call-center.example.com", Some("agent42"))
///     .to("Customer", "sip:customer@example.com", None)
///     .contact("<sip:agent42@192.0.2.42:5060>", None)
///     // Direct future communication to a specific team queue
///     .reply_to("sip:premium-support@call-center.example.com").unwrap()
///     .build();
///
/// // Customer replies will be suggested to go to the premium support queue
/// ```
///
/// ## Conference System with Moderator
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Conference system sending notifications with moderator contact
///
/// // Create a MESSAGE for a conference notification
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:participant@example.com").unwrap()
///     .from("Conference System", "sip:conference@meetings.example.org", Some("conf123"))
///     .to("Participant", "sip:participant@example.com", None)
///     .contact("<sip:conference@192.0.2.100:5060>", None)
///     // Direct questions to the conference moderator
///     .reply_to_with_display_name("Conference Moderator", "sip:moderator@meetings.example.org").unwrap()
///     .build();
///
/// // Participants can direct questions to the moderator instead of the system
/// ```
pub trait ReplyToBuilderExt {
    /// Set a Reply-To header using a URI string
    ///
    /// This method sets the Reply-To header with the provided URI string.
    /// The Reply-To header indicates where the user would prefer replies to be sent,
    /// which may be different from the From address or Contact address.
    ///
    /// # Parameters
    ///
    /// - `uri_str`: A string representation of the SIP URI where replies should be directed
    ///
    /// # Returns
    ///
    /// The builder with the Reply-To header set, or an error if the URI is invalid
    ///
    /// # Errors
    ///
    /// Returns an error if the URI string is invalid or cannot be parsed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a business message with a dedicated reply address
    /// let message = SimpleRequestBuilder::new(Method::Message, "sip:client@example.com").unwrap()
    ///     .from("Business", "sip:info@business.example.com", None)
    ///     .to("Client", "sip:client@example.com", None)
    ///     // Direct replies to a specific support address
    ///     .reply_to("sip:support@business.example.com").unwrap()
    ///     .build();
    /// ```
    fn reply_to(self, uri_str: &str) -> Result<Self>
    where
        Self: Sized;

    /// Set a Reply-To header using a URI string and display name
    ///
    /// This method sets the Reply-To header with the provided URI string and
    /// a display name. The Reply-To header indicates where the user would
    /// prefer replies to be sent, and the display name helps identify the
    /// recipient role or identity.
    ///
    /// # Parameters
    ///
    /// - `display_name`: A descriptive name for the reply address (e.g., "Support Team")
    /// - `uri_str`: A string representation of the SIP URI where replies should be directed
    ///
    /// # Returns
    ///
    /// The builder with the Reply-To header set, or an error if the URI is invalid
    ///
    /// # Errors
    ///
    /// Returns an error if the URI string is invalid or cannot be parsed
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a notification with a friendly reply-to address
    /// let notify = SimpleRequestBuilder::new(Method::Notify, "sip:user@example.com").unwrap()
    ///     .from("System", "sip:system@example.com", None)
    ///     .to("User", "sip:user@example.com", None)
    ///     // Direct replies to the help desk with a friendly name
    ///     .reply_to_with_display_name("Friendly Help Desk", "sip:help@example.com").unwrap()
    ///     .build();
    /// ```
    fn reply_to_with_display_name(self, display_name: &str, uri_str: &str) -> Result<Self>
    where
        Self: Sized;

    /// Set a Reply-To header using a pre-constructed Address
    ///
    /// This method sets the Reply-To header with the provided Address object.
    /// This is useful when you have a pre-constructed Address with advanced
    /// parameters or when reusing an existing Address object.
    ///
    /// # Parameters
    ///
    /// - `address`: A fully constructed Address object with URI and optional display name
    ///
    /// # Returns
    ///
    /// The builder with the Reply-To header set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
    /// use rvoip_sip_core::types::{Method, address::Address, uri::Uri};
    /// use std::str::FromStr;
    ///
    /// // Create an address with custom parameters
    /// let uri = Uri::from_str("sip:team@example.com;transport=tls").unwrap();
    /// let address = Address::new_with_display_name("Special Team", uri);
    ///
    /// // Use the pre-constructed address in a request
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:recipient@example.com").unwrap()
    ///     .from("Sender", "sip:sender@example.com", None)
    ///     .to("Recipient", "sip:recipient@example.com", None)
    ///     // Use the custom address for replies
    ///     .reply_to_address(address)
    ///     .build();
    /// ```
    fn reply_to_address(self, address: Address) -> Self;
}

impl<T> ReplyToBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn reply_to(self, uri_str: &str) -> Result<Self> {
        let uri = Uri::from_str(uri_str)?;
        let address = Address::new(uri);
        Ok(self.reply_to_address(address))
    }

    fn reply_to_with_display_name(self, display_name: &str, uri_str: &str) -> Result<Self> {
        let uri = Uri::from_str(uri_str)?;
        let address = Address::new_with_display_name(display_name, uri);
        Ok(self.reply_to_address(address))
    }

    fn reply_to_address(self, address: Address) -> Self {
        let reply_to = ReplyTo::new(address);
        self.set_header(reply_to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use crate::types::headers::HeaderAccess;
    use std::str::FromStr;

    #[test]
    fn test_request_with_reply_to_uri() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .reply_to("sip:support@example.com").unwrap()
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = request.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:support@example.com");
            assert_eq!(reply_to.address().display_name(), None);
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_reply_to_and_display_name() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .reply_to_with_display_name("Support Team", "sip:support@example.com").unwrap()
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = request.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:support@example.com");
            assert_eq!(reply_to.address().display_name(), Some("Support Team"));
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_reply_to_address() {
        let uri = Uri::from_str("sip:sales@example.com").unwrap();
        let address = Address::new_with_display_name("Sales Department", uri);
        
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .reply_to_address(address)
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = request.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:sales@example.com");
            assert_eq!(reply_to.address().display_name(), Some("Sales Department"));
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_with_reply_to() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .reply_to("sip:support@example.com").unwrap()
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = response.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:support@example.com");
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }
} 