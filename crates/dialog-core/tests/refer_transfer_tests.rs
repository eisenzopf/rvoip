//! Tests for REFER request handling and call transfers

use rvoip_dialog_core::events::SessionCoordinationEvent;
use rvoip_dialog_core::dialog::DialogId;
use rvoip_dialog_core::transaction::TransactionKey;
use rvoip_sip_core::types::refer_to::ReferTo;
use rvoip_sip_core::{Request, Method, StatusCode};
use std::str::FromStr;

#[test]
fn test_transfer_request_event_exists() {
    // Create a sample ReferTo header
    let refer_to_str = "sip:bob@example.com";
    let refer_to = ReferTo::from_str(refer_to_str).expect("Should parse ReferTo");
    
    // Verify TransferRequest event variant exists
    let event = SessionCoordinationEvent::TransferRequest {
        dialog_id: DialogId::new(),
        transaction_id: TransactionKey::new("test-branch".to_string(), Method::Refer, true),
        refer_to: refer_to.clone(),
        referred_by: Some("sip:alice@example.com".to_string()),
        replaces: None,
    };
    
    match event {
        SessionCoordinationEvent::TransferRequest { 
            dialog_id, 
            transaction_id, 
            refer_to: parsed_refer_to,
            referred_by,
            replaces 
        } => {
            assert!(!dialog_id.to_string().is_empty());
            assert!(!transaction_id.to_string().is_empty());
            assert_eq!(parsed_refer_to.uri().to_string(), refer_to.uri().to_string());
            assert_eq!(referred_by, Some("sip:alice@example.com".to_string()));
            assert_eq!(replaces, None);
        }
        _ => panic!("Expected TransferRequest event"),
    }
}

#[test]
fn test_transfer_request_with_replaces() {
    let refer_to = ReferTo::from_str("sip:charlie@example.com").expect("Should parse ReferTo");
    
    // Test attended transfer with Replaces header
    let event = SessionCoordinationEvent::TransferRequest {
        dialog_id: DialogId::new(),
        transaction_id: TransactionKey::new("test-branch-2".to_string(), Method::Refer, true),
        refer_to,
        referred_by: None,
        replaces: Some("call-id=abc123;to-tag=456;from-tag=789".to_string()),
    };
    
    match event {
        SessionCoordinationEvent::TransferRequest { replaces, .. } => {
            assert_eq!(replaces, Some("call-id=abc123;to-tag=456;from-tag=789".to_string()));
        }
        _ => panic!("Expected TransferRequest event"),
    }
}

#[test]
fn test_refer_to_parsing() {
    // Test various ReferTo formats
    let test_cases = vec![
        "sip:bob@example.com",
        "sip:+15551234567@gateway.example.com",
        "sips:secure@example.com:5061",
        "sip:user@192.168.1.100:5060",
    ];
    
    for uri_str in test_cases {
        let refer_to = ReferTo::from_str(uri_str);
        assert!(refer_to.is_ok(), "Failed to parse: {}", uri_str);
        
        let parsed = refer_to.unwrap();
        assert!(!parsed.uri().to_string().is_empty());
    }
}

#[test]
fn test_transfer_request_minimal() {
    // Test with minimal required fields
    let refer_to = ReferTo::from_str("sip:dest@example.com").expect("Should parse ReferTo");
    
    let event = SessionCoordinationEvent::TransferRequest {
        dialog_id: DialogId::new(),
        transaction_id: TransactionKey::new("minimal-branch".to_string(), Method::Refer, true),
        refer_to,
        referred_by: None,
        replaces: None,
    };
    
    match event {
        SessionCoordinationEvent::TransferRequest { 
            referred_by, 
            replaces, 
            .. 
        } => {
            assert!(referred_by.is_none());
            assert!(replaces.is_none());
        }
        _ => panic!("Expected TransferRequest event"),
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ReferToExt, headers::ReferredByExt};
    use rvoip_sip_core::{HeaderName, Header};
    use std::net::SocketAddr;
    
    fn create_refer_request(refer_to_uri: &str, include_referred_by: bool) -> Request {
        let mut builder = SimpleRequestBuilder::new(Method::Refer, "sip:alice@example.com")
            .expect("Should create request builder")
            .from("Alice", "sip:alice@example.com", Some("123456"))
            .to("Bob", "sip:bob@example.com", Some("789012"))
            .call_id("test-call-id@example.com")
            .cseq(1)
            .max_forwards(70)
            .refer_to_blind_transfer(refer_to_uri);  // Use blind transfer method
        
        // Optionally add Referred-By (important for traceability)
        if include_referred_by {
            use rvoip_sip_core::types::uri::Uri;
            use std::str::FromStr;
            let referred_by_uri = Uri::from_str("sip:alice@example.com").expect("Valid URI");
            builder = builder.referred_by_uri(referred_by_uri);
        }
        
        builder.build()
    }
    
    #[test]
    fn test_refer_request_parsing() {
        // Always include Referred-By for our implementation
        let request = create_refer_request("sip:charlie@example.com", true);
        
        // Verify the request has the expected method
        assert_eq!(request.method(), Method::Refer);
        
        // Check headers directly like sip-core tests do
        let refer_to_header = request.headers.iter()
            .find(|h| matches!(h, rvoip_sip_core::types::headers::TypedHeader::ReferTo(_)))
            .expect("Refer-To header should be present");
            
        if let rvoip_sip_core::types::headers::TypedHeader::ReferTo(refer_to) = refer_to_header {
            assert!(refer_to.uri().to_string().contains("charlie@example.com"));
        } else {
            panic!("Expected TypedHeader::ReferTo variant");
        }
        
        // Check for Referred-By header - IMPORTANT: We always want this
        // Check using typed header approach like sip-core tests do
        let referred_by_header = request.headers.iter()
            .find(|h| matches!(h, rvoip_sip_core::types::headers::TypedHeader::ReferredBy(_)))
            .expect("Should have Referred-By header");
            
        if let rvoip_sip_core::types::headers::TypedHeader::ReferredBy(referred_by) = referred_by_header {
            assert!(referred_by.address().uri().to_string().contains("alice@example.com"), 
                    "Referred-By should identify the transferor");
        }
    }
    
    #[test]
    fn test_refer_with_referred_by_always() {
        // Test that Referred-By is properly included
        let request = create_refer_request("sip:dave@example.com", true);
        
        // Verify Refer-To exists using direct header access
        let refer_to_header = request.headers.iter()
            .find(|h| matches!(h, rvoip_sip_core::types::headers::TypedHeader::ReferTo(_)))
            .expect("Must have Refer-To header");
            
        if let rvoip_sip_core::types::headers::TypedHeader::ReferTo(refer_to) = refer_to_header {
            assert!(refer_to.uri().to_string().contains("dave@example.com"));
        }
        
        // Verify Referred-By exists and has correct value
        let referred_by_header = request.headers.iter()
            .find(|h| matches!(h, rvoip_sip_core::types::headers::TypedHeader::ReferredBy(_)))
            .expect("Must have Referred-By header for traceability");
            
        if let rvoip_sip_core::types::headers::TypedHeader::ReferredBy(referred_by) = referred_by_header {
            assert!(referred_by.address().uri().to_string().contains("alice@example.com"), 
                    "Referred-By should contain the transferor's SIP URI");
        }
    }
    
    #[test]
    fn test_blind_transfer_headers() {
        // Test blind transfer - only Refer-To and Referred-By
        // No Replaces header needed for blind transfer
        let request = create_refer_request("sip:charlie@example.com", true);
        
        // Verify REFER method
        assert_eq!(request.method(), Method::Refer);
        
        // Verify Refer-To header exists using direct access
        let refer_to_header = request.headers.iter()
            .find(|h| matches!(h, rvoip_sip_core::types::headers::TypedHeader::ReferTo(_)))
            .expect("Should have Refer-To header");
            
        if let rvoip_sip_core::types::headers::TypedHeader::ReferTo(refer_to) = refer_to_header {
            assert!(refer_to.uri().to_string().contains("charlie@example.com"));
        }
        
        // Verify Referred-By header exists (important for our implementation)
        let referred_by_header = request.headers.iter()
            .find(|h| matches!(h, rvoip_sip_core::types::headers::TypedHeader::ReferredBy(_)))
            .expect("Should have Referred-By header");
            
        if let rvoip_sip_core::types::headers::TypedHeader::ReferredBy(referred_by) = referred_by_header {
            assert!(referred_by.address().uri().to_string().contains("alice@example.com"));
        }
        
        // Verify NO Replaces header (blind transfer)
        let replaces = request.all_headers().iter()
            .find(|h| h.name().to_string().eq_ignore_ascii_case("replaces"));
        assert!(replaces.is_none(), "Should NOT have Replaces header for blind transfer");
    }
}