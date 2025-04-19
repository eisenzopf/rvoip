// This file is being refactored. 
// Core types (StatusCode, Request, Response, Message) moved to src/types/
// Parsing logic for request/response lines moved to parser/request.rs and parser/response.rs

use std::fmt;
use std::str::FromStr;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
// Adjust imports for moved types
use crate::types::{Method, StatusCode, Request, Response, Message, Via};
use crate::uri::Uri;
use crate::version::Version;


// Keep tests temporarily - they need relocation and updating for new structure
#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::Header;
    use crate::header::HeaderName;
    use crate::uri::Uri;
    use std::str::FromStr;

    #[test]
    fn test_status_code_properties() {
        assert!(StatusCode::Trying.is_provisional());
        assert!(!StatusCode::Trying.is_success());
        
        assert!(StatusCode::Ok.is_success());
        assert!(!StatusCode::Ok.is_error());
        
        assert!(StatusCode::BadRequest.is_client_error());
        assert!(StatusCode::BadRequest.is_error());
        
        assert!(StatusCode::ServerInternalError.is_server_error());
        assert!(StatusCode::ServerInternalError.is_error());
        
        assert!(StatusCode::Decline.is_global_failure());
        assert!(StatusCode::Decline.is_error());
    }

    #[test]
    fn test_request_creation() {
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let request = Request::new(Method::Invite, uri.clone())
            .with_header(Header::text(HeaderName::From, "sip:alice@example.com"))
            .with_header(Header::text(HeaderName::To, "sip:bob@example.com"))
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"))
            .with_header(Header::text(HeaderName::CSeq, "1 INVITE"));
            
        assert_eq!(request.method, Method::Invite);
        assert_eq!(request.uri, uri);
        assert_eq!(request.version, Version::sip_2_0());
        assert_eq!(request.headers.len(), 4);
        
        assert_eq!(request.call_id(), Some("abc123@example.com"));
        assert_eq!(request.from(), Some("sip:alice@example.com"));
        assert_eq!(request.to(), Some("sip:bob@example.com"));
        assert_eq!(request.cseq(), Some("1 INVITE"));
    }

    #[test]
    fn test_response_creation() {
        let response = Response::ok()
            .with_header(Header::text(HeaderName::From, "sip:alice@example.com"))
            .with_header(Header::text(HeaderName::To, "sip:bob@example.com;tag=789"))
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"))
            .with_header(Header::text(HeaderName::CSeq, "1 INVITE"));
            
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.version, Version::sip_2_0());
        assert_eq!(response.headers.len(), 4);
        assert_eq!(response.reason_phrase(), "OK");
        
        // Custom reason phrase
        let response = Response::new(StatusCode::Ok)
            .with_reason("Everything is fine");
        assert_eq!(response.reason_phrase(), "Everything is fine");
    }

    #[test]
    fn test_message_enum() {
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let request = Request::new(Method::Invite, uri)
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"));
            
        let response = Response::ok()
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"));
            
        let req_msg = Message::Request(request);
        let resp_msg = Message::Response(response);
        
        assert!(req_msg.is_request());
        assert!(!req_msg.is_response());
        assert!(resp_msg.is_response());
        assert!(!resp_msg.is_request());
        
        assert_eq!(req_msg.method(), Some(Method::Invite));
        assert_eq!(resp_msg.status(), Some(StatusCode::Ok));
        
        assert_eq!(req_msg.call_id(), Some("abc123@example.com"));
        assert_eq!(resp_msg.call_id(), Some("abc123@example.com"));
    }
} 