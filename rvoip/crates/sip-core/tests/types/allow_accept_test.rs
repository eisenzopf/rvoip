// Tests for Allow and Accept types

use crate::common::{assert_parses_ok, assert_parse_fails, assert_display_parses_back};
use rvoip_sip_core::types::{Allow, Accept, Method, MediaType, ContentType};
use rvoip_sip_core::parser::headers::accept::AcceptValue;
use std::str::FromStr;
use std::collections::HashMap;
use ordered_float::NotNan;

#[test]
fn test_allow_display_parse_roundtrip() {
    let allow1 = Allow(vec![Method::Invite, Method::Ack, Method::Options]);
    assert_display_parses_back(&allow1);
    
    let allow2 = Allow(vec![Method::Register]);
    assert_display_parses_back(&allow2);

    // Test with an extension method
    let allow3 = Allow(vec![Method::Invite, Method::Extension("CUSTOM".to_string())]);
    assert_display_parses_back(&allow3);
    
    // Test FromStr directly for edge cases
    assert_parses_ok(
        "INVITE, ACK, OPTIONS, CANCEL, BYE", 
        Allow(vec![Method::Invite, Method::Ack, Method::Options, Method::Cancel, Method::Bye])
    );
    // Parsing should succeed even with unknown methods
     assert_parses_ok("INVITE, BAD", Allow(vec![Method::Invite, Method::Extension("BAD".to_string())])); 
     assert_parses_ok(" CUSTOM , INVITE ", Allow(vec![Method::Extension("CUSTOM".to_string()), Method::Invite])); // Check trimming
    
    assert_parse_fails::<Allow>(""); // Empty fails
    assert_parse_fails::<Allow>("INVITE,"); // Trailing comma fails
    // Add test for invalid token if needed: assert_parse_fails::<Allow>("INVITE, BAD METHOD");
}

#[test]
fn test_accept_display_parse_roundtrip() {
    let mut params = HashMap::new();
    params.insert("level".to_string(), "1".to_string());
    
    // Note: Our new implementation sorts by q-value, so we need to create the expected Accept objects accordingly
    let accept1 = Accept(vec![
        AcceptValue { 
            m_type: "application".to_string(), 
            m_subtype: "sdp".to_string(), 
            q: None, 
            params: HashMap::new() 
        },
        AcceptValue { 
            m_type: "application".to_string(), 
            m_subtype: "json".to_string(), 
            q: None, 
            params: params.clone() 
        }
    ]);
    assert_display_parses_back(&accept1);

    let accept2 = Accept(vec![
        AcceptValue { 
            m_type: "text".to_string(), 
            m_subtype: "html".to_string(), 
            q: None, 
            params: HashMap::new() 
        }
    ]);
    assert_display_parses_back(&accept2);

    // Test FromStr directly for edge cases
    assert_parses_ok("application/sdp, application/json;level=1", accept1);
    assert_parses_ok(" text/html ", accept2); // Check trimming

    assert_parse_fails::<Accept>("");
    assert_parse_fails::<Accept>("application/sdp,"); // Trailing comma fails
    assert_parse_fails::<Accept>("badtype");
}

#[test]
fn test_accept_with_q_value_sorting() {
    // Test that Accept values are sorted by q-value in descending order
    
    // Create Accept with various q-values in random order
    let input_str = "text/html;q=0.5, application/xml, image/*;q=0.2, application/json;q=0.9";
    let accept = Accept::from_str(input_str).unwrap();
    
    // Verify the parsed result is sorted by q-value (highest first)
    assert_eq!(accept.0.len(), 4);
    
    // application/xml has implicit q=1.0 (highest)
    assert_eq!(accept.0[0].m_type, "application");
    assert_eq!(accept.0[0].m_subtype, "xml");
    assert!(accept.0[0].q.is_none());
    
    // application/json has q=0.9 (second highest)
    assert_eq!(accept.0[1].m_type, "application");
    assert_eq!(accept.0[1].m_subtype, "json");
    assert_eq!(accept.0[1].q.unwrap().into_inner(), 0.9);
    
    // text/html has q=0.5 (third highest)
    assert_eq!(accept.0[2].m_type, "text");
    assert_eq!(accept.0[2].m_subtype, "html");
    assert_eq!(accept.0[2].q.unwrap().into_inner(), 0.5);
    
    // image/* has q=0.2 (lowest)
    assert_eq!(accept.0[3].m_type, "image");
    assert_eq!(accept.0[3].m_subtype, "*");
    assert_eq!(accept.0[3].q.unwrap().into_inner(), 0.2);
}

#[test]
fn test_accept_rfc_examples() {
    // Examples from RFC 3261 Section 20.1
    
    // Example: Accept: application/sdp;level=1, application/x-private, text/html
    let input_str = "application/sdp;level=1, application/x-private, text/html";
    let accept = Accept::from_str(input_str).unwrap();
    
    // All should have implicit q=1.0 and maintain order
    assert_eq!(accept.0.len(), 3);
    
    // Each media type should be correctly parsed with parameters
    let sdp = &accept.0[0];
    assert_eq!(sdp.m_type, "application");
    assert_eq!(sdp.m_subtype, "sdp");
    assert_eq!(sdp.params.get("level"), Some(&"1".to_string()));
    
    // Test wildcard types per RFC 3261
    let wildcard_input = "*/*;q=0.8, application/*;q=0.9, text/html";
    let wildcard_accept = Accept::from_str(wildcard_input).unwrap();
    
    // Should be sorted by q-value, with text/html first (q=1.0 implicit)
    assert_eq!(wildcard_accept.0.len(), 3);
    assert_eq!(wildcard_accept.0[0].m_type, "text");
    assert_eq!(wildcard_accept.0[0].m_subtype, "html");
    
    assert_eq!(wildcard_accept.0[1].m_type, "application");
    assert_eq!(wildcard_accept.0[1].m_subtype, "*");
    
    assert_eq!(wildcard_accept.0[2].m_type, "*");
    assert_eq!(wildcard_accept.0[2].m_subtype, "*");
}

#[test]
fn test_accept_edge_cases() {
    // Test extreme q-values
    let edge_input = "text/html;q=0.000, application/xml;q=1.000";
    let edge_accept = Accept::from_str(edge_input).unwrap();
    
    // Should be sorted by q-value
    assert_eq!(edge_accept.0.len(), 2);
    assert_eq!(edge_accept.0[0].m_type, "application");
    assert_eq!(edge_accept.0[0].m_subtype, "xml");
    assert_eq!(edge_accept.0[0].q.unwrap().into_inner(), 1.0);
    
    assert_eq!(edge_accept.0[1].m_type, "text");
    assert_eq!(edge_accept.0[1].m_subtype, "html");
    assert_eq!(edge_accept.0[1].q.unwrap().into_inner(), 0.0);
    
    // Test case insensitivity
    let case_input = "ApPliCaTion/JsOn, TeXt/HtMl";
    let case_accept = Accept::from_str(case_input).unwrap();
    
    assert_eq!(case_accept.0.len(), 2);
    assert_eq!(case_accept.0[0].m_type, "application"); // Should be lowercased
    assert_eq!(case_accept.0[0].m_subtype, "json"); // Should be lowercased
    assert_eq!(case_accept.0[1].m_type, "text"); // Should be lowercased
    assert_eq!(case_accept.0[1].m_subtype, "html"); // Should be lowercased
    
    // Test with various whitespace
    let whitespace_input = " text/html  ;  q=0.8  ,  application/xml ";
    let whitespace_accept = Accept::from_str(whitespace_input).unwrap();
    
    assert_eq!(whitespace_accept.0.len(), 2);
    assert_eq!(whitespace_accept.0[0].m_type, "application"); 
    assert_eq!(whitespace_accept.0[1].m_type, "text");
    
    // Invalid q-values should fail parsing (according to RFC)
    assert_parse_fails::<Accept>("text/html;q=1.1"); // q > 1.0
    assert_parse_fails::<Accept>("text/html;q=-0.5"); // q < 0.0
    assert_parse_fails::<Accept>("text/html;q=1.0000"); // Too many decimal places
} 