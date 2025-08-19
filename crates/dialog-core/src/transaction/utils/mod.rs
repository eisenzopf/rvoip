//! Utility functions for transaction-core
//!
//! This module is organized into focused sub-modules for better maintainability:
//! - `dialog_utils` - Dialog-related utilities and templates
//! - `message_extractors` - Functions for extracting data from SIP messages  
//! - `response_builders` - SIP response creation utilities
//! - `transaction_helpers` - Transaction-related utilities
//! - `request_builders` - SIP request creation utilities

pub mod dialog_utils;
pub mod message_extractors;
pub mod response_builders;
pub mod transaction_helpers;
pub mod request_builders;

// Re-export everything for backward compatibility
// This ensures existing code using `use crate::transaction::utils::*` continues to work

// Dialog utilities
pub use dialog_utils::{
    DialogRequestTemplate,
    generate_branch,
    create_request_from_dialog_template,
};

// Message extractors
pub use message_extractors::{
    extract_branch,
    extract_call_id,
    extract_cseq,
    extract_client_branch_from_response,
    extract_destination,
};

// Response builders
pub use response_builders::{
    create_response,
    create_trying_response,
    create_ringing_response,
    create_ok_response,
    create_ok_response_for_bye,
    create_ok_response_for_cancel,
    create_ok_response_for_options,
    create_ok_response_for_message,
    create_ok_response_for_register,
    create_ok_response_with_dialog_info,
    create_ringing_response_with_tag,
    create_ringing_response_with_dialog_info,
};

// Transaction helpers
pub use transaction_helpers::{
    extract_transaction_parts,
    transaction_key_from_message,
    determine_transaction_kind,
};

// Request builders
pub use request_builders::{
    create_ack_from_invite,
    create_test_request,
};

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::prelude::*;

    #[test]
    fn test_create_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ok);

        // Check status code
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check version (as string instead of enum constant)
        assert_eq!(response.version.to_string(), "SIP/2.0");
        
        // Check headers were copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // Branch should exist but we don't check its exact value since it's generated
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        if let Some(TypedHeader::From(from)) = response.header(&HeaderName::From) {
            assert_eq!(from.address().uri.host, Host::Domain("example.com".to_string()));
        } else {
            panic!("Missing From header in response");
        }
        
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert_eq!(to.address().uri.host, Host::Domain("example.net".to_string()));
            assert!(to.tag().is_none(), "To tag should not be present");
        } else {
            panic!("Missing To header in response");
        }
        
        if let Some(TypedHeader::CallId(_)) = response.header(&HeaderName::CallId) {
            // Call-ID exists, which is what we care about
        } else {
            panic!("Missing Call-ID header in response");
        }
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
            assert_eq!(cseq.sequence(), 1);
        } else {
            panic!("Missing CSeq header in response");
        }
        
        if let Some(TypedHeader::ContentLength(content_length)) = response.header(&HeaderName::ContentLength) {
            assert_eq!(content_length.0, 0);
        } else {
            panic!("Missing Content-Length header in response");
        }
    }

    #[test]
    fn test_create_response_with_to_tag() {
        let request = create_test_request(Method::Invite);
        let mut response = create_response(&request, StatusCode::Ok);

        // Add a tag to the To header
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            // Create a new To header with a tag
            let new_to = to.clone().with_tag("totag");
            
            // Replace the To header
            response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
            response.headers.push(TypedHeader::To(new_to));
        }

        // Check status code is correct
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check Via header was copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // We only check that a branch exists, not its specific value
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        // Check the To header now has a tag
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert_eq!(to.tag().unwrap(), "totag");
        } else {
            panic!("Missing To header in response");
        }
    }

    #[test]
    fn test_create_trying_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Trying);
        assert_eq!(response.status(), StatusCode::Trying);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_ringing_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ringing);
        assert_eq!(response.status(), StatusCode::Ringing);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_ok_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ok);
        assert_eq!(response.status(), StatusCode::Ok);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }
} 