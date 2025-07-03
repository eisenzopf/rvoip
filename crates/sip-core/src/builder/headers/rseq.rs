use crate::error::{Error, Result};
use crate::types::{
    rseq::RSeq,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// RSeq header builder
///
/// This module provides builder methods for the RSeq header in SIP responses.
///
/// ## SIP RSeq Header Overview
///
/// The RSeq (Response Sequence) header is defined in [RFC 3262](https://datatracker.ietf.org/doc/html/rfc3262)
/// as part of the extension for reliable provisional responses. It contains a sequence number that
/// is used to order provisional responses reliably.
///
/// ## Purpose of RSeq Header
///
/// The RSeq header serves several important purposes in SIP:
///
/// 1. It provides a sequence number to uniquely identify each reliable provisional response
/// 2. It allows the User Agent Client (UAC) to acknowledge receipt of provisional responses via PRACK
/// 3. It works together with the RAck header to establish reliable provisional response handling
/// 4. It enables features like early media to work reliably before a call is established
///
/// ## Requirements and Usage
///
/// - RSeq MUST only be used in provisional (1xx) responses
/// - RSeq MUST be used with the Require: 100rel header
/// - RSeq values MUST be unique for each provisional response within a transaction
/// - Each successive provisional response in the same transaction should increment the RSeq value
///
/// ## Relationship with other headers
///
/// - **RSeq** + **Require: 100rel**: Marks a provisional response as requiring acknowledgment
/// - **RSeq** → **RAck**: The client uses the RSeq value in the RAck header of its PRACK request
/// - **RSeq** + **CSeq**: Together they uniquely identify a provisional response for acknowledgment
///
/// # Examples
///
/// ## 180 Ringing with Reliability
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::{RSeqBuilderExt, RequireBuilderExt}};
///
/// // Scenario: Server sending a reliable 180 Ringing response
///
/// // Create a reliable 180 Ringing response
/// let ringing = SimpleResponseBuilder::new(StatusCode::Ringing, Some("Ringing"))
///     .from("Bob", "sip:bob@example.com", Some("to-tag"))
///     .to("Alice", "sip:alice@example.com", Some("from-tag"))
///     .call_id("abcdef123456")
///     .cseq(1, Method::Invite)
///     .via("192.168.1.1", "UDP", Some("z9hG4bK123456"))
///     // Mark as a reliable provisional response
///     .require_tag("100rel")
///     // Add sequence number starting at 1
///     .rseq(1)
///     .build();
///
/// // Client will acknowledge with a PRACK containing RAck: 1 1 INVITE
/// ```
///
/// ## Multiple Provisional Responses
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::{RSeqBuilderExt, RequireBuilderExt}};
///
/// // Scenario: Sequence of reliable provisional responses
/// 
/// // First provisional response (183 Session Progress)
/// let progress = SimpleResponseBuilder::new(StatusCode::SessionProgress, Some("Session Progress"))
///     .from("Bob", "sip:bob@example.com", Some("to-tag"))
///     .to("Alice", "sip:alice@example.com", Some("from-tag"))
///     .call_id("abcdef123456")
///     .cseq(1, Method::Invite)
///     .via("192.168.1.1", "UDP", Some("z9hG4bK123456"))
///     .require_tag("100rel")
///     .rseq(1) // First sequence number
///     .build();
///
/// // Second provisional response (180 Ringing) - note the sequence increment
/// let ringing = SimpleResponseBuilder::new(StatusCode::Ringing, Some("Ringing"))
///     .from("Bob", "sip:bob@example.com", Some("to-tag"))
///     .to("Alice", "sip:alice@example.com", Some("from-tag"))
///     .call_id("abcdef123456")
///     .cseq(1, Method::Invite)
///     .via("192.168.1.1", "UDP", Some("z9hG4bK123456"))
///     .require_tag("100rel")
///     .rseq(2) // Incremented sequence number
///     .build();
///
/// // These responses will be acknowledged with PRACK requests containing
/// // the corresponding RAck values: 1 1 INVITE and 2 1 INVITE
/// ```
///
/// ## Early Media with Reliability
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::{RSeqBuilderExt, RequireBuilderExt}};
///
/// // Scenario: Sending early media (ringback tone) reliably
///
/// // Create SDP content for early media (simplified example)
/// let sdp_body = "v=0\r\n\
///                 o=- 1234567890 1234567890 IN IP4 192.168.1.2\r\n\
///                 s=Ringback Tone\r\n\
///                 c=IN IP4 192.168.1.2\r\n\
///                 t=0 0\r\n\
///                 m=audio 49170 RTP/AVP 0\r\n\
///                 a=sendonly\r\n";
///
/// // Create a reliable 183 Session Progress with early media
/// let early_media = SimpleResponseBuilder::new(StatusCode::SessionProgress, Some("Session Progress"))
///     .from("Bob", "sip:bob@example.com", Some("to-tag"))
///     .to("Alice", "sip:alice@example.com", Some("from-tag"))
///     .call_id("abcdef123456")
///     .cseq(1, Method::Invite)
///     .via("192.168.1.1", "UDP", Some("z9hG4bK123456"))
///     .require_tag("100rel")
///     .rseq(1)
///     .content_type("application/sdp")
///     .body(sdp_body)
///     .build();
///
/// // Client will send a PRACK to acknowledge receipt of the early media offer
/// ```
pub trait RSeqBuilderExt {
    /// Add an RSeq header with a sequence number
    ///
    /// This method adds an RSeq header with a sequence number for reliable provisional responses.
    /// The sequence number should be unique for each provisional response within a transaction.
    ///
    /// # Parameters
    ///
    /// * `seq` - The sequence number to use in the RSeq header
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the RSeq header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RSeqBuilderExt};
    ///
    /// // Adding an RSeq header to a provisional response
    /// let response = SimpleResponseBuilder::new(StatusCode::Ringing, Some("Ringing"))
    ///     .from("Bob", "sip:bob@example.com", Some("to-tag"))
    ///     .to("Alice", "sip:alice@example.com", Some("from-tag"))
    ///     .rseq(1)
    ///     .build();
    ///
    /// // The response now contains an RSeq: 1 header
    /// ```
    fn rseq(self, seq: u32) -> Self;
    
    /// Add an RSeq header with a sequence number incremented from a previous value
    ///
    /// This method adds an RSeq header with a sequence number that is one greater than
    /// the provided previous sequence number. This is useful when sending multiple
    /// provisional responses in sequence.
    ///
    /// # Parameters
    ///
    /// * `previous_seq` - The previous sequence number from which to increment
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the incremented RSeq header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::RSeqBuilderExt};
    ///
    /// // First provisional response with RSeq: 1
    /// let rseq_value = 1;
    /// let first_response = SimpleResponseBuilder::new(StatusCode::SessionProgress, Some("Session Progress"))
    ///     .rseq(rseq_value)
    ///     .build();
    ///
    /// // Second provisional response with RSeq: 2 (incremented)
    /// let second_response = SimpleResponseBuilder::new(StatusCode::Ringing, Some("Ringing"))
    ///     .rseq_next(rseq_value)
    ///     .build();
    ///
    /// // The second response now contains an RSeq: 2 header
    /// ```
    fn rseq_next(self, previous_seq: u32) -> Self;
}

impl<T> RSeqBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn rseq(self, seq: u32) -> Self {
        let rseq = RSeq::new(seq);
        self.set_header(rseq)
    }
    
    fn rseq_next(self, previous_seq: u32) -> Self {
        let rseq = RSeq::new(previous_seq + 1);
        self.set_header(rseq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_response_rseq() {
        let response = ResponseBuilder::new(StatusCode::Ringing, None)
            .rseq(42)
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::RSeq(rseq)) = response.header(&HeaderName::RSeq) {
            assert_eq!(rseq.value, 42);
        } else {
            panic!("RSeq header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_rseq_next() {
        let previous_seq = 99;
        let response = ResponseBuilder::new(StatusCode::SessionProgress, None)
            .rseq_next(previous_seq)
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::RSeq(rseq)) = response.header(&HeaderName::RSeq) {
            assert_eq!(rseq.value, 100); // 99 + 1
        } else {
            panic!("RSeq header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_rseq_multiple() {
        // First response with initial RSeq
        let first_response = ResponseBuilder::new(StatusCode::SessionProgress, None)
            .rseq(1)
            .build();
            
        if let Some(TypedHeader::RSeq(rseq1)) = first_response.header(&HeaderName::RSeq) {
            assert_eq!(rseq1.value, 1);
            
            // Second response with incremented RSeq
            let second_response = ResponseBuilder::new(StatusCode::Ringing, None)
                .rseq_next(rseq1.value)
                .build();
                
            if let Some(TypedHeader::RSeq(rseq2)) = second_response.header(&HeaderName::RSeq) {
                assert_eq!(rseq2.value, 2);
            } else {
                panic!("RSeq header not found in second response");
            }
        } else {
            panic!("RSeq header not found in first response");
        }
    }
} 