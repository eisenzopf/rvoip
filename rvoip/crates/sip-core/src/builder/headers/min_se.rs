use crate::types::{
    min_se::MinSE,
    headers::TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Min-SE Header Builder Extension
///
/// This module provides builder methods for the `Min-SE` (Minimum Session Expires) header
/// in SIP messages, as defined in [RFC 4028](https://datatracker.ietf.org/doc/html/rfc4028#section-3).
///
/// ## SIP Min-SE Header Overview
///
/// The `Min-SE` header field is used in SIP to indicate the minimum session expiration interval
/// supported by a User Agent Client (UAC) or User Agent Server (UAS). It plays a crucial
/// role in the session timer mechanism, ensuring that sessions do not persist indefinitely
/// if one of the parties becomes unresponsive.
///
/// ## Purpose of Min-SE Header
///
/// - **Negotiating Session Timers**: When a UAC sends an INVITE request with a `Session-Expires`
///   header, it may also include a `Min-SE` header. This `Min-SE` value indicates the shortest
///   session expiration time the UAC is willing to accept.
/// - **Handling 422 Responses**: If a UAS receiving the request cannot honor a session
///   duration at least as long as the `Min-SE` value (or its own configured minimum,
///   whichever is higher), it must reject the request with a `422 Session Interval Too Small`
///   response. This response should include a `Min-SE` header field indicating the minimum
///   interval it can support. The default minimum value for `Min-SE` is 90 seconds.
///
/// ## Structure
///
/// The `Min-SE` header field contains a single numeric value representing delta-seconds
/// (an integer number of seconds). While the ABNF allows for generic parameters,
/// they are not commonly used with `Min-SE`, and this builder focuses on setting the
/// `delta-seconds` value.
///
/// ## Relationship with other headers
///
/// - **Session-Expires**: `Min-SE` is often used in conjunction with `Session-Expires`.
///   The `Session-Expires` header indicates the desired session duration, while `Min-SE`
///   specifies the minimum acceptable duration.
///
/// # Examples
///
/// ## UAC Sending INVITE with Min-SE
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::MinSEBuilderExt};
///
/// // Scenario: UAC wants a 30-minute session but will accept no less than 5 minutes.
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("call-123"))
///     .to("Bob", "sip:bob@example.com", None)
///     // Session-Expires would also be set
///     .min_se(300) // Min-SE: 300 seconds (5 minutes)
///     .build();
///
/// // The UAS now knows Alice's minimum acceptable session duration.
/// ```
///
/// ## UAS Responding with 422 and Min-SE
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::MinSEBuilderExt};
///
/// // Scenario: UAS receives an INVITE with Session-Expires of 60s, but its minimum is 90s.
/// let response_422 = SimpleResponseBuilder::new(
///         StatusCode::from_u16(422).expect("422 is a valid status code"),
///         Some("Session Interval Too Small")
///     )
///     .from("Bob", "sip:bob@example.com", Some("uas-tag-1"))
///     .to("Alice", "sip:alice@example.com", Some("uac-tag-xyz"))
///     .call_id("call-123")
///     .cseq(1, Method::Invite)
///     .min_se(90) // Min-SE: 90 (UAS's minimum)
///     .build();
///
/// // Alice receives this 422 and knows she must propose a session interval of at least 90s.
/// ```
pub trait MinSEBuilderExt {
    /// Sets the `Min-SE` header with the specified delta-seconds value.
    ///
    /// This method adds a `Min-SE` header to the SIP message, indicating the minimum
    /// session expiration interval in seconds.
    ///
    /// # Parameters
    ///
    /// * `delta_seconds` - The minimum session interval in seconds. According to RFC 4028,
    ///                     the default value if not present is 90 seconds.
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the `Min-SE` header added.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::MinSEBuilderExt};
    ///
    /// // UAC indicates it supports a minimum session timer of 120 seconds.
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .min_se(120)
    ///     .build();
    /// ```
    fn min_se(self, delta_seconds: u32) -> Self;
}

impl<T> MinSEBuilderExt for T
where
    T: HeaderSetter,
{
    fn min_se(self, delta_seconds: u32) -> Self {
        self.set_header(MinSE::new(delta_seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode, headers::HeaderName, MinSE as MinSEType};
    use crate::types::headers::HeaderAccess;

    #[test]
    fn test_request_builder_min_se() {
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:test@example.com").unwrap()
            .min_se(120)
            .build();

        let headers = request.headers(&HeaderName::MinSE);
        assert_eq!(headers.len(), 1, "Min-SE header should be present ");

        match headers.first().unwrap() {
            TypedHeader::MinSE(min_se_val) => {
                assert_eq!(min_se_val.delta_seconds, 120, "Min-SE delta_seconds mismatch ");
            }
            _ => panic!("Expected TypedHeader::MinSE variant "),
        }

        // Also test access via typed_header helper if available
        if let Some(min_se_header) = request.typed_header::<MinSEType>() {
            assert_eq!(min_se_header.delta_seconds, 120);
        } else {
            panic!("Could not get MinSE header using typed_header ");
        }
    }

    #[test]
    fn test_response_builder_min_se() {
        let response = SimpleResponseBuilder::new(StatusCode::from_u16(422).expect("Failed to create status code 422"), Some("Session Interval Too Small"))
            .min_se(90)
            .build();

        let headers = response.headers(&HeaderName::MinSE);
        assert_eq!(headers.len(), 1, "Min-SE header should be present in response ");

        match headers.first().unwrap() {
            TypedHeader::MinSE(min_se_val) => {
                assert_eq!(min_se_val.delta_seconds, 90, "Min-SE delta_seconds mismatch in response ");
            }
            _ => panic!("Expected TypedHeader::MinSE variant in response "),
        }

        // Also test access via typed_header helper if available
         if let Some(min_se_header) = response.typed_header::<MinSEType>() {
            assert_eq!(min_se_header.delta_seconds, 90);
        } else {
            panic!("Could not get MinSE header using typed_header on response ");
        }
    }

    #[test]
    fn test_min_se_default_value_implication() {
        // This test doesn't directly test the builder setting a default,
        // as the builder always sets an explicit value.
        // It's more about acknowledging the RFC default.
        let request_without_min_se = SimpleRequestBuilder::new(Method::Invite, "sip:test@example.com").unwrap()
            .build();
        assert!(request_without_min_se.headers(&HeaderName::MinSE).is_empty(), "Min-SE should not be present if not set");
        // The interpretation of a missing Min-SE header (e.g., defaulting to 90s)
        // is up to the SIP entities processing the message, not the builder itself.
    }
} 