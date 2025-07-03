// Parser for the Reply-To header (RFC 3261 Section 20.32)
// Reply-To = "Reply-To" HCOLON rplyto-spec
// rplyto-spec = ( name-addr / addr-spec ) *( SEMI rplyto-param )
// rplyto-param = generic-param

use nom::{
    bytes::complete::tag_no_case,
    combinator::{map, map_res, verify},
    sequence::{pair, preceded},
    IResult,
    multi::many0,
    error::{Error},
};

// Import from base parser modules
use crate::parser::separators::{hcolon, semi};
use crate::parser::address::name_addr_or_addr_spec;
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::ParseResult;
use crate::parser::quoted;

use crate::types::param::Param;
use crate::types::uri::Uri;
use crate::types::address::Address;
use crate::types::reply_to::ReplyTo as ReplyToHeader;
use crate::parser::address::name_addr;
use serde::{Serialize, Deserialize};

// Define a struct to represent the Reply-To header value
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ReplyToValue {
    pub display_name: Option<String>,
    pub uri: Uri,
    pub params: Vec<Param>,
}

// rplyto-spec = ( name-addr / addr-spec ) *( SEMI rplyto-param )
// rplyto-param = generic-param
// Returns Address struct with params included
fn rplyto_spec(input: &[u8]) -> ParseResult<Address> {
    // Verify we have input to parse
    if input.is_empty() {
        return Err(nom::Err::Error(Error::new(input, nom::error::ErrorKind::Eof)));
    }

    map(
        pair(
            name_addr_or_addr_spec, // Returns Address{..., params: []}
            many0(preceded(semi, generic_param))
        ),
        |(mut addr, params_vec)| {
            addr.params = params_vec; // Assign parsed generic params
            addr
        }
    )(input)
}

// Reply-To = "Reply-To" HCOLON rplyto-spec
// Note: HCOLON handled elsewhere
pub fn parse_reply_to(input: &[u8]) -> ParseResult<ReplyToHeader> {
    // Validate that the input can be parsed as a Reply-To header
    map(
        rplyto_spec,
        ReplyToHeader
    )(input)
}

/// Parses a Reply-To header value.
pub fn parse_reply_to_public(input: &[u8]) -> ParseResult<Address> {
    rplyto_spec(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::Address;
    use crate::types::uri::{Scheme, Host};
    use crate::types::param::{Param, GenericValue};
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::str::FromStr;

    #[test]
    fn test_parse_reply_to_simple() {
        let input = b"<sip:user@example.com>";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, None);
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert!(address.params.is_empty());
    }
    
    #[test]
    fn test_parse_reply_to_name_addr_params() {
        let input = b"\"Support\" <sip:support@example.com>;dept=billing";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, Some("Support".to_string()));
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 1);
        assert!(address.params.contains(&Param::Other("dept".to_string(), Some(GenericValue::Token("billing".to_string())))));
    }
    
    #[test]
    fn test_parse_reply_to_addr_spec() {
        // addr-spec format (no angle brackets)
        let input = b"sip:sales@example.com";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, None);
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert!(address.params.is_empty());
    }
    
    #[test]
    fn test_parse_reply_to_with_multiple_params() {
        // Reply-To with multiple parameters
        let input = b"\"Help Desk\" <sip:helpdesk@example.com>;hours=24x7;priority=high";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, Some("Help Desk".to_string()));
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 2);
        
        // Check for parameters
        assert!(address.params.iter().any(|p| 
            matches!(p, Param::Other(n, Some(GenericValue::Token(v))) 
                if n == "hours" && v == "24x7")
        ));
        assert!(address.params.iter().any(|p| 
            matches!(p, Param::Other(n, Some(GenericValue::Token(v))) 
                if n == "priority" && v == "high")
        ));
    }
    
    #[test]
    fn test_parse_reply_to_sips_uri() {
        // SIPS URI
        let input = b"<sips:secure@example.com>";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sips);
    }
    
    #[test]
    fn test_parse_reply_to_uri_with_params() {
        // URI with parameters inside angle brackets
        let input = b"<sip:support@example.com;transport=tcp>";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sip);
        
        // URI parameters should be in the uri.parameters field, not address.params
        assert!(address.params.is_empty());
        assert!(address.uri.parameters.contains(&Param::Transport("tcp".to_string())));
    }
    
    #[test]
    fn test_parse_reply_to_complex() {
        // Complex example with URI params and header params
        let input = b"\"Customer Service\" <sip:cs@example.com;transport=udp>;department=sales;language=en";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, Some("Customer Service".to_string()));
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 2);
        
        // Check URI parameters
        assert!(address.uri.parameters.contains(&Param::Transport("udp".to_string())));
        
        // Check header parameters
        assert!(address.params.iter().any(|p| 
            matches!(p, Param::Other(n, Some(GenericValue::Token(v))) 
                if n == "department" && v == "sales")
        ));
        assert!(address.params.iter().any(|p| 
            matches!(p, Param::Other(n, Some(GenericValue::Token(v))) 
                if n == "language" && v == "en")
        ));
    }
    
    #[test]
    fn test_parse_reply_to_flag_param() {
        // Flag parameter (no value)
        let input = b"<sip:emergency@example.com>;urgent";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 1);
        
        // Check flag parameter
        assert!(address.params.iter().any(|p| 
            matches!(p, Param::Other(n, None) if n == "urgent")
        ));
    }
    
    #[test]
    fn test_parse_reply_to_quoted_string_param() {
        // Parameter with quoted string value
        let input = b"<sip:user@example.com>;note=\"Call back ASAP\"";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 1);
        
        // Check quoted string parameter
        assert!(address.params.iter().any(|p| 
            matches!(p, Param::Other(n, Some(GenericValue::Quoted(v))) 
                if n == "note" && v == "Call back ASAP")
        ));
    }
    
    #[test]
    fn test_parse_reply_to_empty_should_fail() {
        let input = b"";
        let result = parse_reply_to_public(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_parse_reply_to_invalid_uri_should_fail() {
        let input = b"<invalid:uri>";
        let result = parse_reply_to_public(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_parse_reply_to_unsupported_scheme_should_fail() {
        let input = b"<http://example.com>";
        let result = parse_reply_to_public(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_parse_reply_to_tel_uri() {
        // When parsing tel URIs in SIP, use the addr-spec format instead of name-addr
        // Unlike SIP/SIPS URIs, TEL URIs use the whole number as host (not user)
        let input = b"tel:+1-212-555-0123";
        let result = parse_reply_to_public(input);
        
        // Print detailed debug info if it fails
        if result.is_err() {
            println!("Failed to parse TEL URI in Reply-To: {:?}", result);
        }
        
        assert!(result.is_ok());
        
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Tel);
        // TEL URIs typically store the number in the host part
        if let Host::Domain(number) = &address.uri.host {
            assert_eq!(number, "+1-212-555-0123");
        } else {
            panic!("Expected domain type host for TEL URI");
        }
    }
    
    #[test]
    fn test_parse_reply_to_with_escaping() {
        let input = b"\"Support\\\"Team\" <sip:support@example.com>";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, Some("Support\"Team".to_string()));
    }
    
    #[test]
    fn test_parse_reply_to_with_ipv6() {
        let input = b"<sip:[2001:db8::1]>";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        
        if let Host::Address(addr) = &address.uri.host {
            if let IpAddr::V6(ipv6) = addr {
                let expected = Ipv6Addr::from_str("2001:db8::1").unwrap();
                assert_eq!(ipv6.octets(), expected.octets());
            } else {
                panic!("Expected IPv6 address");
            }
        } else {
            panic!("Expected address type host");
        }
    }
    
    #[test]
    fn test_parse_reply_to_with_whitespace() {
        // Test with various whitespace formatting
        let input = b"  \"Sales Team\"   <sip:sales@example.com>  ;  priority = high  ";
        let result = parse_reply_to_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        
        // Remaining content should only be the trailing whitespace
        assert_eq!(rem, b"  ");
        assert_eq!(address.display_name, Some("Sales Team".to_string()));
        assert!(address.params.iter().any(|p| 
            matches!(p, Param::Other(n, Some(GenericValue::Token(v))) 
                if n == "priority" && v == "high")
        ));
    }
} 