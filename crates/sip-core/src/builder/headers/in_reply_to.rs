use crate::error::{Error, Result};
use crate::types::{
    header::{Header, HeaderName},
    headers::TypedHeader,
    in_reply_to::InReplyTo,
    call_id::CallId,
};
use crate::builder::headers::HeaderSetter;

/// In-Reply-To header builder
///
/// This module provides builder methods for the In-Reply-To header,
/// which allows SIP requests to reference previous Call-IDs as defined
/// in RFC 3261 Section 20.22.
///
/// ## SIP In-Reply-To Header Overview
///
/// The In-Reply-To header field contains a list of Call-IDs from previous requests.
/// It is primarily used to reference earlier messages in the same dialog or across dialogs.
/// This is useful for tracking related calls, threading conversations, and referencing
/// previous communications.
///
/// ## Common Use Cases
///
/// - **Call returns**: When returning a missed call
/// - **Call threading**: Establishing relationships between separate communications
/// - **Follow-up calls**: Referencing previous discussions
/// - **Customer service**: Linking calls to previous support tickets or calls
/// - **Auditing**: Tracking call histories for compliance or record-keeping
///
/// ## Real-world Applications
///
/// - **Enterprise environments**: Tracking business communication threads
/// - **Contact centers**: Linking customer interactions
/// - **Call-back services**: Returning missed calls with context
/// - **VoIP systems**: Managing multi-device call transfers
///
/// # Examples
///
/// ## Returning a Missed Call
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::InReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Office worker returning a missed call
///
/// // Create an INVITE that references a missed call
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
///     .from("Bob", "sip:bob@example.com", Some("tag123"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:bob@192.0.2.4:5060>", None)
///     // Reference the Call-ID from Alice's missed call
///     .in_reply_to("a84b4c76e66710@alice-phone.example.com")
///     .build();
///
/// // Alice's phone can display "Returning your call" or link to call history
/// ```
///
/// ## Multi-party Conference Follow-up
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::InReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Setting up a follow-up to a multi-party conference call
///
/// // Create an INVITE referencing multiple previous calls
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:conference@example.com").unwrap()
///     .from("Organizer", "sip:organizer@example.com", Some("xyz789"))
///     .to("Conference", "sip:conference@example.com", None)
///     .contact("<sip:organizer@203.0.113.5:5060>", None)
///     // Reference Call-IDs from two previous conference sessions
///     .in_reply_to_multiple(vec![
///         "conf123@conference.example.com",  // Monday's meeting
///         "conf456@conference.example.com"   // Wednesday's follow-up
///     ])
///     .build();
///
/// // Conference server can link to records from previous meetings
/// ```
///
/// ## Customer Service Call-back
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::InReplyToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Customer service agent returning a customer call
///
/// // Create an INVITE for a customer call-back with context
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:+15551234567@example.com").unwrap()
///     .from("Agent", "sip:agent42@support.example.com", Some("cs42"))
///     .to("Customer", "sip:+15551234567@example.com", None)
///     .contact("<sip:agent42@192.0.2.42:5060>", None)
///     // Reference the initial customer call and support ticket
///     .in_reply_to_multiple(vec![
///         "customer-call-12345@ivr.support.example.com",  // Original call
///         "ticket-54321@crm.support.example.com"          // Support ticket
///     ])
///     .build();
///
/// // CRM system can display customer history when the call connects
/// ```
pub trait InReplyToBuilderExt {
    /// Add an In-Reply-To header with a single Call-ID
    ///
    /// This method sets the In-Reply-To header with a single Call-ID value.
    /// The In-Reply-To header is used to reference Call-IDs of previous requests,
    /// creating a relationship between the current request and a previous one.
    ///
    /// # Parameters
    ///
    /// - `call_id`: The Call-ID value to reference
    ///
    /// # Returns
    ///
    /// The builder with the In-Reply-To header set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::InReplyToBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a request returning a missed call
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:user@example.com").unwrap()
    ///     .from("Caller", "sip:caller@example.com", None)
    ///     .to("User", "sip:user@example.com", None)
    ///     .in_reply_to("missed-call-123@pbx.example.com")
    ///     .build();
    /// ```
    fn in_reply_to(self, call_id: &str) -> Self;

    /// Add an In-Reply-To header with multiple Call-IDs
    ///
    /// This method sets the In-Reply-To header with multiple Call-ID values.
    /// The In-Reply-To header is used to reference Call-IDs of previous requests,
    /// creating relationships between the current request and multiple previous ones.
    /// This is particularly useful for complex call flows or threading conversations.
    ///
    /// # Parameters
    ///
    /// - `call_ids`: A vector of Call-ID strings to reference
    ///
    /// # Returns
    ///
    /// The builder with the In-Reply-To header set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::InReplyToBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a request referencing multiple previous calls
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:support@example.com").unwrap()
    ///     .from("Customer", "sip:customer@example.com", None)
    ///     .to("Support", "sip:support@example.com", None)
    ///     .in_reply_to_multiple(vec![
    ///         "initial-call-123@pbx.example.com",
    ///         "follow-up-456@pbx.example.com",
    ///         "ticket-789@crm.example.com"
    ///     ])
    ///     .build();
    /// ```
    fn in_reply_to_multiple(self, call_ids: Vec<&str>) -> Self;
}

impl<T> InReplyToBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn in_reply_to(self, call_id: &str) -> Self {
        let in_reply_to = InReplyTo::new(call_id);
        self.set_header(in_reply_to)
    }

    fn in_reply_to_multiple(self, call_ids: Vec<&str>) -> Self {
        let call_id_strings = call_ids.into_iter().map(|s| s.to_string()).collect();
        let in_reply_to = InReplyTo::with_multiple_strings(call_id_strings);
        self.set_header(in_reply_to)
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
    fn test_request_with_single_in_reply_to() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .in_reply_to("70710@saturn.bell-tel.com")
            .build();
            
        if let Some(TypedHeader::InReplyTo(in_reply_to)) = request.header(&HeaderName::InReplyTo) {
            assert_eq!(in_reply_to.len(), 1);
            assert!(in_reply_to.contains("70710@saturn.bell-tel.com"));
        } else {
            panic!("In-Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_multiple_in_reply_to() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .in_reply_to_multiple(vec![
                "70710@saturn.bell-tel.com", 
                "17320@venus.bell-tel.com"
            ])
            .build();
            
        if let Some(TypedHeader::InReplyTo(in_reply_to)) = request.header(&HeaderName::InReplyTo) {
            assert_eq!(in_reply_to.len(), 2);
            assert!(in_reply_to.contains("70710@saturn.bell-tel.com"));
            assert!(in_reply_to.contains("17320@venus.bell-tel.com"));
        } else {
            panic!("In-Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_with_in_reply_to() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .in_reply_to("70710@saturn.bell-tel.com")
            .build();
            
        if let Some(TypedHeader::InReplyTo(in_reply_to)) = response.header(&HeaderName::InReplyTo) {
            assert_eq!(in_reply_to.len(), 1);
            assert!(in_reply_to.contains("70710@saturn.bell-tel.com"));
        } else {
            panic!("In-Reply-To header not found or has wrong type");
        }
    }
} 