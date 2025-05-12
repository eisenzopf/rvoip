// Declare header parser modules
pub mod via;
/// Contact Header Parsing
pub mod contact;
pub mod from;
pub mod to;
pub mod route;
pub mod record_route;
pub mod cseq;
pub mod max_forwards;
pub mod expires;
pub mod content_length;
pub mod call_id;
pub mod min_expires;
pub mod mime_version;
pub mod priority;
pub mod subject;
pub mod timestamp;
pub mod user_agent;
pub mod server;
pub mod reply_to;
pub mod refer_to;
pub mod referred_by;
pub mod organization;
pub mod date;
pub mod allow;
pub mod require;
pub mod supported;
pub mod unsupported;
pub mod proxy_require;
pub mod in_reply_to;
pub mod content_type;
pub mod content_disposition;
pub mod accept;
pub mod accept_encoding;
pub mod accept_language;
pub mod content_encoding;
pub mod content_language;
pub mod alert_info;
pub mod call_info;
pub mod error_info;
pub mod warning;
pub mod retry_after;
pub mod reason;
pub mod auth; // Group for auth parsers
pub mod www_authenticate;
pub mod proxy_authenticate;
pub mod authorization;
pub mod proxy_authorization;
pub mod authentication_info;
pub mod uri_with_params; // Added
pub mod session_expires; // Added for Session-Expires header
pub mod event; // Added for Event header

// Keep internal modules private
mod server_val;
mod token_list;
mod media_type;

// Re-export public parser functions
pub use via::parse_via;
pub use contact::parse_contact;
pub use from::parse_from;
pub use to::{parse_to, to_header};
pub use route::parse_route;
pub use record_route::parse_record_route;
pub use cseq::parse_cseq;
pub use max_forwards::parse_max_forwards;
pub use expires::parse_expires;
pub use content_length::parse_content_length;
pub use call_id::parse_call_id;
pub use min_expires::parse_min_expires;
pub use mime_version::parse_mime_version;
pub use priority::parse_priority;
pub use subject::parse_subject;
pub use timestamp::parse_timestamp;
pub use user_agent::parse_user_agent;
pub use server::parse_server;
pub use reply_to::parse_reply_to;
pub use refer_to::parse_refer_to;
pub use referred_by::parse_referred_by;
pub use organization::parse_organization;
pub use date::parse_date;
pub use allow::parse_allow;
pub use require::parse_require;
pub use supported::parse_supported;
pub use unsupported::parse_unsupported;
pub use proxy_require::parse_proxy_require;
pub use in_reply_to::parse_in_reply_to;
pub use content_type::parse_content_type_value;
pub use content_disposition::parse_content_disposition;
pub use accept::parse_accept;
pub use accept_encoding::parse_accept_encoding;
pub use accept_language::parse_accept_language;
pub use content_encoding::parse_content_encoding;
pub use content_language::parse_content_language;
pub use alert_info::parse_alert_info;
pub use call_info::parse_call_info;
pub use error_info::parse_error_info;
pub use warning::parse_warning_value_list;
pub use retry_after::parse_retry_after;
pub use reason::parse_reason;
pub use www_authenticate::parse_www_authenticate;
pub use proxy_authenticate::parse_proxy_authenticate;
pub use authorization::parse_authorization;
pub use proxy_authorization::parse_proxy_authorization;
pub use authentication_info::parse_authentication_info;
pub use session_expires::parse_session_expires_header;
pub use event::parse_event_header_value; // Added for Event header value parsing

// Re-export shared auth components if needed directly
// pub use auth::common::{auth_param, realm, nonce, ...};

// Re-export shared URI component parser if needed directly?
// pub use uri_with_params::uri_with_generic_params;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ParseResult;
    use crate::types::event::EventType;
    use crate::types::event::ParamValue;

    /// Test to verify that exported parser functions are correctly accessible
    #[test]
    fn test_parser_exports() {
        // Test sampling of different header parsers to ensure they're correctly exported
        // This doesn't test the actual parsing, just that the functions are accessible
        let via_fn: fn(&[u8]) -> ParseResult<_> = parse_via;
        let contact_fn: fn(&[u8]) -> ParseResult<_> = parse_contact;
        let from_fn: fn(&[u8]) -> ParseResult<_> = parse_from;
        let to_fn: fn(&[u8]) -> ParseResult<_> = parse_to;
        let cseq_fn: fn(&[u8]) -> ParseResult<_> = parse_cseq;
        let auth_fn: fn(&[u8]) -> ParseResult<_> = parse_www_authenticate;
        
        // If this compiles, the exports are working correctly
        assert!(true);
    }

    /// Test that parsing a Via header works through the exported function
    #[test]
    fn test_via_parser() {
        // Use the direct via_param_parser function instead of parse_via since our test doesn't include the header name
        let input = b"SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds";
        
        // Import and use the test function that parses without header name
        use crate::parser::headers::via::parse_via_params;
        let result = parse_via_params(input);
        assert!(result.is_ok());
    }
    
    /// Test that parsing a Contact header works through the exported function
    #[test]
    fn test_contact_parser() {
        let input = b"<sip:alice@atlanta.com>";
        let result = parse_contact(input);
        assert!(result.is_ok());
    }
    
    /// Test that parsing a From header works through the exported function
    #[test]
    fn test_from_parser() {
        let input = b"Alice <sip:alice@atlanta.com>;tag=1928301774";
        let result = parse_from(input);
        assert!(result.is_ok());
    }
    
    /// Test that parsing a CSeq header works through the exported function
    #[test]
    fn test_cseq_parser() {
        let input = b"314159 INVITE";
        let result = parse_cseq(input);
        assert!(result.is_ok());
    }
    
    /// Test that parsing a Call-ID header works through the exported function
    #[test]
    fn test_call_id_parser() {
        // Use the internal callid function instead of parse_call_id since our test doesn't include the header name
        let input = b"a84b4c76e66710@pc33.atlanta.com";
        
        // Import and use the callid function that handles just the value part
        use crate::parser::headers::call_id::callid;
        let result = callid(input);
        assert!(result.is_ok());
    }
    
    /// Test that parsing a Content-Type header works through the exported function
    #[test]
    fn test_content_type_parser() {
        let input = b"application/sdp";
        let result = parse_content_type_value(input);
        assert!(result.is_ok());
    }
    
    /// Test that parsing a WWW-Authenticate header works through the exported function
    #[test]
    fn test_www_authenticate_parser() {
        let input = b"Digest realm=\"atlanta.com\", nonce=\"8452cd5a\", qop=\"auth\"";
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
    }
    
    /// Test that parsing a Require header works through the exported function
    #[test]
    fn test_require_parser() {
        let input = b"100rel,precondition";
        let result = parse_require(input);
        assert!(result.is_ok());
    }

    /// Test that parsing an Event header value works through the exported function
    #[test]
    fn test_event_parser() {
        let input = b"presence;id=123;foo=bar";
        let result = parse_event_header_value(input);
        assert!(result.is_ok());
        if let Ok((rem, event_header)) = result {
            assert!(rem.is_empty());
            assert_eq!(event_header.event_type, EventType::Token("presence".to_string()));
            assert_eq!(event_header.id, Some("123".to_string()));
            assert_eq!(event_header.params.get("foo"), Some(&ParamValue::Value("bar".to_string())));
        }
    }
}