// SIP Parser tests entry point

// Public module declarations to include tests
pub mod parser {
    pub mod header_parser_test;
    pub mod uri_parser_test;
}

// Import common test utilities
mod common;

// Basic SIP Parser tests

#[cfg(test)]
mod parser_tests {
    use std::str::FromStr;
    
    // Import core types directly with current paths
    use rvoip_sip_core::{
        types::{
            content_type::ContentType,
            cseq::CSeq,
            call_id::CallId,
            content_length::ContentLength,
            method::Method,
            uri::{Uri, Scheme},
        }
    };

    #[test]
    fn test_content_type_parsing() {
        let ct = ContentType::from_str("application/sdp").unwrap();
        assert_eq!(ct.0.m_type, "application");
        assert_eq!(ct.0.m_subtype, "sdp");
    }

    #[test]
    fn test_cseq_parsing() {
        let cseq = CSeq::from_str("42 INVITE").unwrap();
        assert_eq!(cseq.seq, 42);
        assert_eq!(cseq.method, Method::Invite);
    }

    #[test]
    fn test_uri_parsing() {
        let uri = Uri::from_str("sip:alice@example.com").unwrap();
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user.as_deref(), Some("alice"));
        assert_eq!(uri.host.to_string(), "example.com");
    }

    #[test]
    fn test_call_id_parsing() {
        let call_id = CallId::from_str("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@foo.bar.com").unwrap();
        assert!(!call_id.0.is_empty());
    }

    #[test]
    fn test_content_length_parsing() {
        let len = ContentLength::from_str("42").unwrap();
        assert_eq!(len.0, 42);
    }

    // We won't test Via header directly since it seems to have a more complex API
} 