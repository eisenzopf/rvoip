//! Unit tests for the HeaderAccess trait

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::types::headers::HeaderAccess;
    use crate::types::header::{HeaderName, TypedHeader};
    use crate::types::uri::Uri;
    use crate::types::method::Method;
    use crate::types::address::Address;
    use crate::types::via::{Via, ViaHeader, SentProtocol};
    use crate::types::param::Param;
    use std::str::FromStr;

    // Helper function to create a test request with headers
    fn create_test_request() -> Request {
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        
        Request::new(Method::Invite, uri)
            .with_header(TypedHeader::From(From::new(Address::new_with_display_name(
                "Alice", 
                "sip:alice@example.com".parse().unwrap()
            ))))
            .with_header(TypedHeader::To(To::new(Address::new_with_display_name(
                "Bob", 
                "sip:bob@example.com".parse().unwrap()
            ))))
            .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
            .with_header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
            .with_header(TypedHeader::Via(Via(vec![
                ViaHeader {
                    sent_protocol: SentProtocol {
                        name: "SIP".to_string(),
                        version: "2.0".to_string(),
                        transport: "UDP".to_string(),
                    },
                    sent_by_host: Host::domain("pc33.atlanta.com"),
                    sent_by_port: None,
                    params: vec![Param::new("branch".to_string(), Some("z9hG4bK776asdhds".to_string()))]
                },
                ViaHeader {
                    sent_protocol: SentProtocol {
                        name: "SIP".to_string(),
                        version: "2.0".to_string(),
                        transport: "UDP".to_string(),
                    },
                    sent_by_host: Host::domain("bigbox3.site3.atlanta.com"),
                    sent_by_port: None,
                    params: vec![]
                },
            ])))
    }

    // Helper function to create a test response with headers
    fn create_test_response() -> Response {
        Response::new(StatusCode::Ok)
            .with_header(TypedHeader::From(From::new(Address::new_with_display_name(
                "Alice", 
                "sip:alice@example.com".parse().unwrap()
            ))))
            .with_header(TypedHeader::To(To::new(Address::new_with_display_name(
                "Bob", 
                "sip:bob@example.com".parse().unwrap()
            ))))
            .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
            .with_header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
            .with_header(TypedHeader::Via(Via(vec![
                ViaHeader {
                    sent_protocol: SentProtocol {
                        name: "SIP".to_string(),
                        version: "2.0".to_string(),
                        transport: "UDP".to_string(),
                    },
                    sent_by_host: Host::domain("pc33.atlanta.com"),
                    sent_by_port: None,
                    params: vec![Param::new("branch".to_string(), Some("z9hG4bK776asdhds".to_string()))]
                },
                ViaHeader {
                    sent_protocol: SentProtocol {
                        name: "SIP".to_string(),
                        version: "2.0".to_string(),
                        transport: "UDP".to_string(),
                    },
                    sent_by_host: Host::domain("bigbox3.site3.atlanta.com"),
                    sent_by_port: None,
                    params: vec![]
                },
            ])))
    }

    #[test]
    fn test_request_typed_headers() {
        let request = create_test_request();
        
        // Test typed_header for From
        let from = request.typed_header::<From>();
        assert!(from.is_some());
        
        // Test typed_headers for Via (multiple)
        let vias = request.typed_headers::<Via>();
        assert!(!vias.is_empty());
        assert_eq!(vias.len(), 2);
    }
    
    #[test]
    fn test_response_typed_headers() {
        let response = create_test_response();
        
        // Test typed_header for From
        let from = response.typed_header::<From>();
        assert!(from.is_some());
        
        // Test typed_headers for Via (multiple)
        let vias = response.typed_headers::<Via>();
        assert!(!vias.is_empty());
        assert_eq!(vias.len(), 2);
    }
    
    #[test]
    fn test_message_typed_headers() {
        let request = create_test_request();
        let message: Message = request.into();
        
        // Test typed_header for From
        let from = message.typed_header::<From>();
        assert!(from.is_some());
        
        // Test typed_headers for Via (multiple)
        let vias = message.typed_headers::<Via>();
        assert!(!vias.is_empty());
        assert_eq!(vias.len(), 2);
    }
    
    #[test]
    fn test_header_by_name() {
        let request = create_test_request();
        
        // Test with HeaderName
        let via_headers = request.headers(&HeaderName::Via);
        assert_eq!(via_headers.len(), 1);
        
        // Test with string name
        let via_headers = request.headers_by_name("Via");
        assert_eq!(via_headers.len(), 1);
        
        // Test with invalid string name
        let invalid_headers = request.headers_by_name("InvalidHeader");
        assert_eq!(invalid_headers.len(), 0);
    }
    
    #[test]
    fn test_raw_header_value() {
        let request = create_test_request();
        
        let call_id_value = request.raw_header_value(&HeaderName::CallId);
        assert!(call_id_value.is_some());
        assert!(call_id_value.unwrap().contains("a84b4c76e66710@pc33.atlanta.com"));
    }
    
    #[test]
    fn test_header_names() {
        let request = create_test_request();
        
        let names = request.header_names();
        assert!(names.contains(&HeaderName::From));
        assert!(names.contains(&HeaderName::To));
        assert!(names.contains(&HeaderName::CallId));
        assert!(names.contains(&HeaderName::CSeq));
        assert!(names.contains(&HeaderName::Via));
        assert_eq!(names.len(), 5); // Each header should only appear once
    }
    
    #[test]
    fn test_has_header() {
        let request = create_test_request();
        
        assert!(request.has_header(&HeaderName::From));
        assert!(request.has_header(&HeaderName::To));
        assert!(request.has_header(&HeaderName::CallId));
        assert!(request.has_header(&HeaderName::CSeq));
        assert!(request.has_header(&HeaderName::Via));
        assert!(!request.has_header(&HeaderName::ContentType));
    }
} 