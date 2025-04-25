//! SIP protocol parser implementation
//!
//! This module contains parsers for SIP messages, headers, and related structures.
//! All parsers use the nom parser combinator library.

// Core parsing modules
pub mod common;
// mod basic_rules; // REMOVED
pub mod headers;
pub mod message;
pub mod multipart;
mod request;
mod response;
pub mod uri;
pub mod utils;
pub mod address;
// Re-export the address parser under the expected name
pub use address::name_addr_or_addr_spec as parse_address;
mod common_params;
mod utf8;

// Re-export top-level parsers and types, consolidate duplicates
pub use message::{parse_message /*, IncrementalParser, ParseState*/ }; // Removed unresolved imports
// pub use request::request_parser; // Removed
// pub use response::response_parser; // Removed
// Commenting out potentially unresolved imports
pub use uri::{parse_uri /*, parse_uri_params, parse_host_port*/ };
pub use multipart::{parse_multipart};
pub use crate::types::multipart::{MimePart, MultipartBody};

// Re-export specific header parsers needed by types/header.rs
// TODO: Update these exports once individual header parsers are implemented in headers/
pub use headers::{
    parse_via,
    // parse_address, // Keep commented until implemented
    parse_contact,
    parse_from,
    parse_to,
    parse_route,
    parse_record_route,
    parse_cseq,
    parse_max_forwards,
    parse_expires,
    parse_content_length,
    parse_call_id,
    parse_reply_to,
    parse_allow,
    parse_content_type_value,
    parse_content_disposition,
    parse_accept,
    parse_warning_value_list,
    parse_accept_encoding,
    parse_accept_language,
    parse_content_encoding,
    parse_content_language,
    parse_alert_info,
    parse_call_info,
    parse_error_info,
    parse_retry_after,
    parse_www_authenticate,
    parse_authorization,
    parse_proxy_authenticate,
    parse_proxy_authorization,
    parse_authentication_info,
};

// Comment out missing exports
// pub use message::{parse_message, IncrementalParser, ParseState };

// Maybe re-export specific header parsers if needed directly?
// pub use headers::{parse_via, parse_cseq, ...}; 

// Type alias for parser result
pub type ParseResult<'a, O> = nom::IResult<&'a [u8], O, nom::error::Error<&'a [u8]>>;

// Re-export common nom traits and types
pub use nom::error::{Error as NomError, ErrorKind, ParseError};

// Declare parser submodules
pub mod common_chars;
pub mod whitespace;
pub mod separators;
pub mod token;
pub mod quoted;
pub mod values;

// pub use basic_rules::{ParseResult, ...}; // REMOVE OR UPDATE COMMENT

// Comprehensive tests for the parser modules
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode, Uri, Via, Request, Response};
    use crate::types::header::HeaderName;
    use nom::error::ErrorKind;

    #[test]
    fn test_parse_result_type() {
        // Verify that ParseResult type alias is working properly
        let result: ParseResult<'_, u32> = Ok((&b""[..], 42));
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert_eq!(rem, &b""[..]);
        assert_eq!(val, 42);
    }

    #[test]
    fn test_parse_uri() {
        // Test basic SIP URI
        let input = b"sip:alice@example.com";
        let result = parse_uri(input);
        assert!(result.is_ok());
        let (rem, uri) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri.to_string(), "sip:alice@example.com");

        // Test SIP URI with parameters
        let input = b"sip:alice@example.com;transport=udp";
        let result = parse_uri(input);
        assert!(result.is_ok());
        let (rem, uri) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri.to_string(), "sip:alice@example.com;transport=udp");

        // Test SIPS URI
        let input = b"sips:bob@secure.example.org";
        let result = parse_uri(input);
        assert!(result.is_ok());
        let (rem, uri) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri.to_string(), "sips:bob@secure.example.org");

        // Test invalid URI
        let input = b"invalid:uri";
        let result = parse_uri(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_address() {
        // Test basic address
        let input = b"<sip:alice@example.com>";
        let result = parse_address(input);
        assert!(result.is_ok());
        
        // Test display name and address
        let input = b"Alice <sip:alice@example.com>";
        let result = parse_address(input);
        assert!(result.is_ok());
        
        // Test just URI as address
        let input = b"sip:bob@example.org";
        let result = parse_address(input);
        assert!(result.is_ok());
        
        // Test with parameters
        let input = b"Alice <sip:alice@example.com>;tag=1234";
        let result = parse_address(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_message() {
        // Test a basic SIP request
        let input = b"REGISTER sip:example.com SIP/2.0\r\n\
                     Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
                     Max-Forwards: 70\r\n\
                     To: Bob <sip:bob@biloxi.com>\r\n\
                     From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
                     CSeq: 314159 REGISTER\r\n\
                     Contact: <sip:alice@pc33.atlanta.com>\r\n\
                     Content-Length: 0\r\n\r\n";
        let result = parse_message(input);
        assert!(result.is_ok());
        let message = result.unwrap();
        match message {
            crate::types::Message::Request(request) => {
                assert_eq!(request.method, Method::Register);
            }
            _ => panic!("Expected Request type"),
        }

        // Test a basic SIP response
        let input = b"SIP/2.0 200 OK\r\n\
                     Via: SIP/2.0/UDP server10.biloxi.com;branch=z9hG4bK4b43c2ff8.1\r\n\
                     Via: SIP/2.0/UDP bigbox3.site3.atlanta.com;branch=z9hG4bK77ef4c2312983.1\r\n\
                     Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
                     To: Bob <sip:bob@biloxi.com>;tag=a6c85cf\r\n\
                     From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
                     Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
                     CSeq: 314159 INVITE\r\n\
                     Contact: <sip:bob@192.0.2.4>\r\n\
                     Content-Type: application/sdp\r\n\
                     Content-Length: 0\r\n\r\n";
        let result = parse_message(input);
        assert!(result.is_ok());
        let message = result.unwrap();
        match message {
            crate::types::Message::Response(response) => {
                assert_eq!(response.status, StatusCode::Ok);
            }
            _ => panic!("Expected Response type"),
        }
    }

    #[test]
    fn test_parse_multipart() {
        // Test a basic multipart message
        let boundary = "boundary1";
        let input = format!(
            "--{boundary}\r\n\
             Content-Type: text/plain\r\n\r\n\
             This is the first part\r\n\
             --{boundary}\r\n\
             Content-Type: application/sdp\r\n\r\n\
             v=0\r\n\
             o=alice 2890844526 2890844526 IN IP4 atlanta.example.com\r\n\
             --{boundary}--"
        ).into_bytes();

        let result = parse_multipart(&input, boundary);
        assert!(result.is_ok());
        let body = result.unwrap();
        assert_eq!(body.parts.len(), 2);
        assert_eq!(body.parts[0].content_type().unwrap(), "text/plain");
        assert_eq!(body.parts[1].content_type().unwrap(), "application/sdp");
    }

    #[test]
    fn test_header_exports() {
        // Test that header parsers are correctly re-exported
        use super::headers::{
            parse_via, parse_contact, parse_from, parse_to, 
            parse_cseq, parse_call_id, parse_content_type_value
        };
        use super::headers::via::parse_via_params;

        // Via - uncommented for debugging
        let input = b"SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds";
        let result = parse_via_params(input);
        assert!(result.is_ok(), "Via header should parse successfully");
        
        // Contact
        let input = b"<sip:alice@atlanta.com>";
        let result = parse_contact(input);
        assert!(result.is_ok());

        // From
        let input = b"Alice <sip:alice@atlanta.com>;tag=1928301774";
        let result = parse_from(input);
        assert!(result.is_ok());

        // CSeq
        let input = b"314159 INVITE";
        let result = parse_cseq(input);
        assert!(result.is_ok());
    }

    // Add specific test for Via headers
    #[test]
    fn test_via_headers() {
        use super::headers::parse_via;
        use super::headers::via::parse_via_params;
        
        // Test basic Via header
        let input = b"SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds";
        let result = parse_via_params(input);
        assert!(result.is_ok(), "Basic Via header should parse successfully");
        if let Ok((_, vias)) = result {
            assert_eq!(vias.len(), 1, "Should parse exactly one Via header");
            let via = &vias[0];
            assert_eq!(via.protocol(), "SIP/2.0");
            assert_eq!(via.transport(), "UDP");
            assert_eq!(via.host().to_string(), "pc33.atlanta.com");
            assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
        }
        
        // Test Via header with port
        let input = b"SIP/2.0/TCP 192.168.1.1:5060;branch=z9hG4bKnashds7";
        let result = parse_via_params(input);
        assert!(result.is_ok(), "Via header with port should parse successfully");
        if let Ok((_, vias)) = result {
            let via = &vias[0];
            assert_eq!(via.port(), Some(5060));
            assert_eq!(via.transport(), "TCP");
        }
        
        // Test Via header with multiple parameters
        let input = b"SIP/2.0/UDP biloxi.com:5060;branch=z9hG4bK123;received=192.0.2.3;ttl=16;maddr=224.2.0.1";
        let result = parse_via_params(input);
        assert!(result.is_ok(), "Via header with multiple parameters should parse successfully");
        if let Ok((_, vias)) = result {
            let via = &vias[0];
            assert_eq!(via.branch(), Some("z9hG4bK123"));
            assert_eq!(via.received(), Some("192.0.2.3".to_string()));
            assert_eq!(via.ttl(), Some(16));
            assert_eq!(via.maddr(), Some("224.2.0.1"));
        }
        
        // Test Via header with hidden branch parameter
        let input = b"SIP/2.0/UDP 192.168.0.1;hidden;branch=z9hG4bK776asdhds";
        let result = parse_via_params(input);
        assert!(result.is_ok(), "Via header with hidden parameter should parse successfully");
        if let Ok((_, vias)) = result {
            let via = &vias[0];
            assert_eq!(via.branch(), Some("z9hG4bK776asdhds"));
            assert!(via.has_param("hidden"), "Should have hidden parameter");
            assert_eq!(via.param_value("hidden"), None, "Hidden parameter should have no value");
        }
        
        // Test multiple Via headers in one go
        let input = b"SIP/2.0/UDP first.com:5060;branch=z9hG4bK876, SIP/2.0/TCP second.com;branch=z9hG4bK321";
        let result = parse_via_params(input);
        assert!(result.is_ok(), "Multiple Via headers should parse successfully");
        if let Ok((_, vias)) = result {
            assert_eq!(vias.len(), 2, "Should parse two Via headers");
            assert_eq!(vias[0].host().to_string(), "first.com");
            assert_eq!(vias[0].transport(), "UDP");
            assert_eq!(vias[1].host().to_string(), "second.com");
            assert_eq!(vias[1].transport(), "TCP");
        }
        
        // Test IPv6 address in Via header
        let input = b"SIP/2.0/UDP [2001:db8::1]:5060;branch=z9hG4bK776asdhds";
        let result = parse_via_params(input);
        assert!(result.is_ok(), "Via header with IPv6 address should parse successfully");
        if let Ok((_, vias)) = result {
            let via = &vias[0];
            assert!(matches!(via.host(), crate::types::uri::Host::Address(_)), "Host should be an IPv6 address");
            assert_eq!(via.port(), Some(5060));
        }
    }

    #[test]
    fn test_parser_integration() {
        // Parse a message, then extract and parse specific headers
        let input = b"INVITE sip:bob@biloxi.com SIP/2.0\r\n\
                    Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
                    Max-Forwards: 70\r\n\
                    To: Bob <sip:bob@biloxi.com>\r\n\
                    From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
                    Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
                    CSeq: 314159 INVITE\r\n\
                    Contact: <sip:alice@pc33.atlanta.com>\r\n\
                    Content-Length: 0\r\n\r\n";
        
        // Parse the full message
        let message = parse_message(input).unwrap();
        
        // Verify message type
        match message {
            crate::types::Message::Request(request) => {
                assert_eq!(request.method, Method::Invite);
                assert_eq!(request.uri.to_string(), "sip:bob@biloxi.com");
                
                // Extract and verify headers
                let via_headers = request.via_headers();
                // Via headers should now be properly parsed
                assert_eq!(via_headers.len(), 1, "Should have one Via header");
                if !via_headers.is_empty() {
                    let via = &via_headers[0];
                    assert_eq!(via.0[0].sent_by_host.to_string(), "pc33.atlanta.com");
                    assert_eq!(via.0[0].sent_protocol.transport, "UDP");
                    assert_eq!(via.0[0].branch(), Some("z9hG4bK776asdhds"));
                }
                
                let from_header = request.header(&HeaderName::From);
                assert!(from_header.is_some());
                
                let to_header = request.header(&HeaderName::To);
                assert!(to_header.is_some());
                
                let cseq_header = request.header(&HeaderName::CSeq);
                assert!(cseq_header.is_some());
            },
            _ => panic!("Expected Request")
        }
    }
} 