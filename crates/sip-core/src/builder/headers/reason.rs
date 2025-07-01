use crate::error::{Error, Result};
use crate::types::{
    reason::Reason,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;
use crate::types::StatusCode;

/// Reason header builder
///
/// This module provides builder methods for the Reason header in SIP messages.
///
/// ## SIP Reason Header Overview
///
/// The Reason header is defined in [RFC 3326](https://datatracker.ietf.org/doc/html/rfc3326)
/// and provides information about why a particular SIP request was generated, especially
/// for requests like BYE and CANCEL that terminate dialogs or transactions.
///
/// ## Format
///
/// ```text
/// Reason: protocol ;cause=code ;text="comment"
/// ```
///
/// ## Purpose of Reason Header
///
/// The Reason header serves several important purposes in SIP:
///
/// 1. It provides diagnostic information about why a call or transaction was terminated
/// 2. It enables interworking with other protocols like Q.850/ISDN by carrying their cause codes
/// 3. It helps applications make informed decisions based on the specific reason for session termination
/// 4. It provides useful information for call detail records (CDRs) and troubleshooting
///
/// ## Common Protocols and Cause Codes
///
/// ### SIP Protocol
///
/// SIP Reason headers using the "SIP" protocol typically use SIP response codes as cause values:
///
/// - **480** (Temporarily Unavailable): The user is currently unavailable
/// - **486** (Busy Here): The user is busy and cannot take the call
/// - **487** (Request Terminated): The request was terminated by a BYE or CANCEL
/// - **600** (Busy Everywhere): The user is busy on all devices
/// - **603** (Decline): The user explicitly declined the call
///
/// ### Q.850 Protocol
///
/// Q.850 Reason headers carry ISDN/PSTN cause codes:
///
/// - **16** (Normal Clearing): Normal call clearing 
/// - **17** (User Busy): The called party is busy
/// - **18** (No User Responding): The user is not responding to the call request
/// - **19** (No Answer): The user has been alerted but does not answer
/// - **31** (Normal, Unspecified): Normal, unspecified reason
///
/// ## Usage Scenarios
///
/// - **Call Termination**: Include in BYE requests to explain why a call is being ended
/// - **Request Cancellation**: Include in CANCEL requests to explain why a pending request is being canceled
/// - **Session Rejection**: Include in rejection responses to explain why a session is being rejected
/// - **Gateway Interworking**: Used by gateways to translate between SIP and PSTN/ISDN networks
///
/// # Examples
///
/// ## BYE Request with Call Completion Elsewhere
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
///
/// // Scenario: A forking proxy detected the call was answered elsewhere
///
/// // Create a BYE request with Reason indicating completion elsewhere
/// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:alice@192.168.1.2:5060").unwrap()
///     .from("Bob", "sip:bob@example.com", Some("xyz123"))
///     .to("Alice", "sip:alice@example.com", Some("abc456"))
///     .reason_sip(200, Some("Call completed elsewhere"))
///     .build();
///
/// // This BYE informs the endpoint that the call wasn't dropped,
/// // but was answered on another device (forking)
/// ```
///
/// ## BYE Request with PSTN Disconnection Reason
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
///
/// // Scenario: PSTN gateway translating a Q.850 cause code to SIP
///
/// // Create a BYE with the Q.850 cause code from the PSTN network
/// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:gw1@pstn-gw.example.com:5060").unwrap()
///     .from("PSTN", "sip:+15551234567@pstn-gw.example.com", Some("gw-123"))
///     .to("Alice", "sip:alice@example.com", Some("abc987"))
///     .reason_q850(16, Some("Normal clearing"))
///     .build();
///
/// // The Q.850 cause=16 indicates normal call clearing from the PSTN side
/// ```
///
/// ## CANCEL Request with User Busy Reason
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
///
/// // Scenario: Proxy canceling INVITE forks after receiving 486 from one device
///
/// // Create a CANCEL request for pending INVITE branches
/// let cancel = SimpleRequestBuilder::new(Method::Cancel, "sip:bob@192.168.1.3:5060").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("xyz789"))
///     .to("Bob", "sip:bob@example.com", None)
///     .reason_sip(486, Some("Busy Here"))
///     .build();
///
/// // This CANCEL informs other branches that the user was busy on another device
/// // This allows UAs to provide a specific busy notification rather than generic cancellation
/// ```
pub trait ReasonBuilderExt {
    /// Add a Reason header with the specified protocol, cause code, and optional text
    ///
    /// This method adds a Reason header with the given protocol, cause code, and optional
    /// explanatory text. This is the most general method for creating a Reason header.
    ///
    /// # Parameters
    ///
    /// * `protocol` - The protocol causing the event (e.g., "SIP", "Q.850")
    /// * `cause` - The protocol-specific cause code
    /// * `text` - Optional human-readable text explaining the reason
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Reason header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
    ///
    /// // Adding a custom reason protocol
    /// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:alice@example.com").unwrap()
    ///     .from("Bob", "sip:bob@example.com", Some("tag123"))
    ///     .to("Alice", "sip:alice@example.com", Some("tag456"))
    ///     .reason("INTERNAL", 102, Some("Call quota exceeded"))
    ///     .build();
    ///
    /// // This provides application-specific reason information
    /// // that might be useful for analytics or debugging
    /// ```
    fn reason(self, protocol: impl Into<String>, cause: u16, text: Option<impl Into<String>>) -> Self;
    
    /// Add a Reason header with SIP protocol and the specified cause code and text
    ///
    /// This convenience method adds a Reason header with the "SIP" protocol and
    /// the given cause code (typically a SIP response code) and optional text.
    ///
    /// # Parameters
    ///
    /// * `cause` - The SIP response code
    /// * `text` - Optional human-readable text explaining the reason
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the SIP Reason header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
    ///
    /// // Create a BYE request indicating the user declined the call
    /// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:alice@192.168.1.2:5060").unwrap()
    ///     .from("Bob", "sip:bob@example.com", Some("xyz123"))
    ///     .to("Alice", "sip:alice@example.com", Some("abc456"))
    ///     .reason_sip(603, Some("Declined"))
    ///     .build();
    ///
    /// // This indicates to the other endpoint that the call
    /// // was explicitly declined, not just disconnected
    /// ```
    fn reason_sip(self, cause: u16, text: Option<impl Into<String>>) -> Self;
    
    /// Add a Reason header with Q.850 protocol and the specified cause code and text
    ///
    /// This convenience method adds a Reason header with the "Q.850" protocol and
    /// the given cause code (from the Q.850/ISDN specification) and optional text.
    ///
    /// # Parameters
    ///
    /// * `cause` - The Q.850 cause code
    /// * `text` - Optional human-readable text explaining the reason
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Q.850 Reason header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
    ///
    /// // Create a BYE request from a PSTN gateway indicating user busy
    /// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:proxy.example.com:5060").unwrap()
    ///     .from("Gateway", "sip:+12125551234@pstn-gw.example.com", Some("gw1"))
    ///     .to("Alice", "sip:alice@example.com", Some("a457j"))
    ///     .reason_q850(17, Some("User busy"))
    ///     .build();
    ///
    /// // This carries the PSTN/ISDN Q.850 cause code 17 (User Busy)
    /// // which helps maintain diagnostic information across network boundaries
    /// ```
    fn reason_q850(self, cause: u16, text: Option<impl Into<String>>) -> Self;
    
    /// Add a Reason header for SIP request termination
    ///
    /// This convenience method adds a Reason header with the "SIP" protocol and
    /// cause code 487 (Request Terminated), which is the standard reason for
    /// ending a transaction with a CANCEL request.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Request Terminated Reason header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
    ///
    /// // Create a CANCEL request with standard request termination reason
    /// let cancel = SimpleRequestBuilder::new(Method::Cancel, "sip:proxy.example.com:5060").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a1b2c3"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .reason_terminated()
    ///     .build();
    ///
    /// // This is the standard Reason header for CANCELing an INVITE transaction
    /// ```
    fn reason_terminated(self) -> Self;
    
    /// Add a Reason header for busy indication
    ///
    /// This convenience method adds a Reason header with the "SIP" protocol and
    /// cause code 486 (Busy Here), which indicates the user is busy.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Busy Here Reason header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
    ///
    /// // Create a CANCEL request indicating the user is busy
    /// let cancel = SimpleRequestBuilder::new(Method::Cancel, "sip:proxy.example.com:5060").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a1b2c3"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .reason_busy()
    ///     .build();
    ///
    /// // This informs other endpoints that the CANCEL is due to the user being busy
    /// // not because the caller hung up or due to some other error
    /// ```
    fn reason_busy(self) -> Self;
    
    /// Add a Reason header for normal call clearing
    ///
    /// This convenience method adds a Reason header with the "Q.850" protocol and
    /// cause code 16 (Normal Clearing), which is the standard reason for normal
    /// call termination in PSTN/ISDN networks.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Normal Clearing Reason header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
    ///
    /// // Create a BYE request with normal clearing reason
    /// let bye = SimpleRequestBuilder::new(Method::Bye, "sip:bob@example.com:5060").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .to("Bob", "sip:bob@example.com", Some("tag456"))
    ///     .reason_normal_clearing()
    ///     .build();
    ///
    /// // This indicates the call was cleared normally (user hang-up)
    /// // using the same code as would be used in PSTN networks
    /// ```
    fn reason_normal_clearing(self) -> Self;
    
    /// Add a Reason header for call rejected/declined
    ///
    /// This convenience method adds a Reason header with the "SIP" protocol and
    /// cause code 603 (Decline), which indicates the user explicitly declined the call.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Decline Reason header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReasonBuilderExt};
    ///
    /// // Create a CANCEL request indicating the user declined the call
    /// let cancel = SimpleRequestBuilder::new(Method::Cancel, "sip:proxy.example.com:5060").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a1b2c3"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .reason_declined()
    ///     .build();
    ///
    /// // This informs other endpoints that the CANCEL is due to the user
    /// // explicitly declining the call on another device
    /// ```
    fn reason_declined(self) -> Self;
}

impl<T> ReasonBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn reason(self, protocol: impl Into<String>, cause: u16, text: Option<impl Into<String>>) -> Self {
        let reason = Reason::new(protocol, cause, text);
        self.set_header(reason)
    }
    
    fn reason_sip(self, cause: u16, text: Option<impl Into<String>>) -> Self {
        self.reason("SIP", cause, text)
    }
    
    fn reason_q850(self, cause: u16, text: Option<impl Into<String>>) -> Self {
        self.reason("Q.850", cause, text)
    }
    
    fn reason_terminated(self) -> Self {
        self.reason_sip(487, Some("Request Terminated"))
    }
    
    fn reason_busy(self) -> Self {
        self.reason_sip(486, Some("Busy Here"))
    }
    
    fn reason_normal_clearing(self) -> Self {
        self.reason_q850(16, Some("Normal Clearing"))
    }
    
    fn reason_declined(self) -> Self {
        self.reason_sip(603, Some("Declined"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::request::SimpleRequestBuilder;
    use crate::types::Method;
    use crate::types::reason::Reason;
    use crate::types::headers::HeaderName;
    use crate::types::StatusCode;
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_reason() {
        let request = RequestBuilder::new(Method::Bye, "sip:bob@example.com").unwrap()
            .reason("SIP", 200, Some("Call completed elsewhere"))
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Reason(reason)) = request.header(&HeaderName::Reason) {
            assert_eq!(reason.protocol(), "SIP");
            assert_eq!(reason.cause(), 200);
            assert_eq!(reason.text(), Some("Call completed elsewhere"));
        } else {
            panic!("Reason header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_reason_sip() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .reason_sip(486, Some("Busy Here"))
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Reason(reason)) = response.header(&HeaderName::Reason) {
            assert_eq!(reason.protocol(), "SIP");
            assert_eq!(reason.cause(), 486);
            assert_eq!(reason.text(), Some("Busy Here"));
        } else {
            panic!("Reason header not found or has wrong type");
        }
    }

    #[test]
    fn test_reason_q850() {
        let request = RequestBuilder::new(Method::Bye, "sip:bob@example.com").unwrap()
            .reason_q850(16, Some("Normal Clearing"))
            .build();
            
        if let Some(TypedHeader::Reason(reason)) = request.header(&HeaderName::Reason) {
            assert_eq!(reason.protocol(), "Q.850");
            assert_eq!(reason.cause(), 16);
            assert_eq!(reason.text(), Some("Normal Clearing"));
        } else {
            panic!("Reason header not found or has wrong type");
        }
    }

    #[test]
    fn test_reason_convenience_methods() {
        // Test reason_terminated
        let terminate_request = RequestBuilder::new(Method::Cancel, "sip:bob@example.com").unwrap()
            .reason_terminated()
            .build();
            
        if let Some(TypedHeader::Reason(reason)) = terminate_request.header(&HeaderName::Reason) {
            assert_eq!(reason.protocol(), "SIP");
            assert_eq!(reason.cause(), 487);
            assert_eq!(reason.text(), Some("Request Terminated"));
        } else {
            panic!("Reason header not found or has wrong type");
        }
        
        // Test reason_busy
        let busy_request = RequestBuilder::new(Method::Cancel, "sip:bob@example.com").unwrap()
            .reason_busy()
            .build();
            
        if let Some(TypedHeader::Reason(reason)) = busy_request.header(&HeaderName::Reason) {
            assert_eq!(reason.protocol(), "SIP");
            assert_eq!(reason.cause(), 486);
            assert_eq!(reason.text(), Some("Busy Here"));
        } else {
            panic!("Reason header not found or has wrong type");
        }
        
        // Test reason_normal_clearing
        let clearing_request = RequestBuilder::new(Method::Bye, "sip:bob@example.com").unwrap()
            .reason_normal_clearing()
            .build();
            
        if let Some(TypedHeader::Reason(reason)) = clearing_request.header(&HeaderName::Reason) {
            assert_eq!(reason.protocol(), "Q.850");
            assert_eq!(reason.cause(), 16);
            assert_eq!(reason.text(), Some("Normal Clearing"));
        } else {
            panic!("Reason header not found or has wrong type");
        }
        
        // Test reason_declined
        let declined_request = RequestBuilder::new(Method::Cancel, "sip:bob@example.com").unwrap()
            .reason_declined()
            .build();
            
        if let Some(TypedHeader::Reason(reason)) = declined_request.header(&HeaderName::Reason) {
            assert_eq!(reason.protocol(), "SIP");
            assert_eq!(reason.cause(), 603);
            assert_eq!(reason.text(), Some("Declined"));
        } else {
            panic!("Reason header not found or has wrong type");
        }
    }

    #[test]
    fn test_multiple_reasons() {
        let request = SimpleRequestBuilder::new(Method::Bye, "sip:bob@biloxi.com")
            .unwrap()
            .reason_sip(487, Some("Request Terminated"))
            .reason_q850(16, Some("Normal Clearing"))
            .build();

        let header = request.typed_header::<Reason>();
        assert!(header.is_some(), "Reason header should be present");
        let reason = header.unwrap();
        assert_eq!(reason.protocol(), "Q.850", "Protocol should be Q.850");
        assert_eq!(reason.cause(), 16, "Cause should be 16");
        assert_eq!(reason.text().as_deref(), Some("Normal Clearing"), "Text should be 'Normal Clearing'");
    }
} 