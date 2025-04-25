// Parser for the From header (RFC 3261 Section 20.20)
// From = ( "From" / "f" ) HCOLON from-spec
// from-spec = ( name-addr / addr-spec ) *( SEMI from-param )
// from-param = tag-param / generic-param
// tag-param = "tag" EQUAL token

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case},
    combinator::{map, map_res, opt, recognize, value},
    multi::{many0, many1},
    sequence::{delimited, pair, preceded, terminated},
    IResult,
    error::{Error as NomError, ErrorKind},
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, equal, laquot, raquot};
use crate::parser::address::name_addr_or_addr_spec; // Use shared address parser
use crate::parser::token::token; // Added token
use crate::parser::quoted::quoted_string; // Added quoted_string
use crate::parser::whitespace::lws; // Added lws
use crate::parser::uri::parse_uri; // Added parse_uri
// Import specific param parser and list helper
use crate::parser::common_params::{from_to_param, semicolon_separated_params0, generic_param}; // Added generic_param
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::uri::Uri;
use crate::types::address::Address;
use crate::types::from::From as FromHeader; // Use specific type alias

// NOTE: name_addr and addr_spec are duplicated from contact.rs for now.
// Consider extracting to a shared address.rs module later.

// display-name = *(token LWS)/ quoted-string
fn display_name(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        quoted_string,
        recognize(many1(terminated(token, lws)))
    ))(input)
}

// addr-spec = SIP-URI / SIPS-URI / absoluteURI
fn addr_spec(input: &[u8]) -> ParseResult<Uri> {
    parse_uri(input)
}

// name-addr = [ display-name ] LAQUOT addr-spec RAQUOT
fn name_addr(input: &[u8]) -> ParseResult<(Option<&[u8]>, Uri)> {
    pair(
        opt(terminated(display_name, lws)),
        delimited(laquot, addr_spec, raquot)
    )(input)
}

// tag-param = "tag" EQUAL token
fn tag_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(tag_no_case(b"tag".as_slice()), preceded(equal, token)),
        |tag_bytes| {
            match str::from_utf8(tag_bytes) {
                Ok(tag_str) => Ok(Param::Tag(tag_str.to_string())),
                Err(_) => Err(nom::Err::Failure(NomError::new(tag_bytes, ErrorKind::Tag)))
            }
        }
    )(input)
}

// Special case for the "lr" flag parameter
fn lr_param(input: &[u8]) -> ParseResult<Param> {
    value(Param::Lr, tag_no_case(b"lr"))(input)
}

// transport-param = "transport=" token
fn transport_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(tag_no_case(b"transport".as_slice()), preceded(equal, token)),
        |transport_bytes| str::from_utf8(transport_bytes).map(|s| Param::Transport(s.to_string()))
    )(input)
}

// from-param = tag-param / generic-param
fn from_param_item(input: &[u8]) -> ParseResult<Param> {
    alt((tag_param, lr_param, transport_param, generic_param))(input)
}

// from-spec = ( name-addr / addr-spec ) *( SEMI from-param )
// Returns Address struct with params included
fn from_spec(input: &[u8]) -> ParseResult<Address> {
    map(
        pair(
            name_addr_or_addr_spec, // Returns Address{..., params: []}
            many0(preceded(semi, from_param_item)) // Changed to use from_param_item
        ),
        |(mut addr, params_vec)| { // params_vec is now Vec<Param>
            // Extend existing URI params (if any) with header params
            addr.params.extend(params_vec); 
            addr // Return the modified Address
        }
    )(input)
}

// From = "From" HCOLON from-spec
// Note: HCOLON handled elsewhere
// Make this function public
pub fn parse_from(input: &[u8]) -> ParseResult<FromHeader> {
    map(
        from_spec,
        FromHeader
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::{Address};
    use crate::types::uri::{Uri, Host, Scheme};
    use crate::types::param::{Param, GenericValue};
    use std::collections::HashMap;

    #[test]
    fn test_parse_from_simple_addr_spec() {
        let input = b"<sip:user@example.com>;tag=asdf";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap(); // Returns FromHeader now
        let addr = from_header.0; // Access the inner Address
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, None);
        assert_eq!(addr.uri.scheme, Scheme::Sip);
        assert_eq!(addr.params.len(), 1);
        assert!(matches!(addr.params[0], Param::Tag(ref s) if s == "asdf"));
    }
    
    #[test]
    fn test_parse_from_name_addr() {
        let input = b"\"Bob\" <sips:bob@example.com>;tag=12345";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("Bob".to_string()));
        assert_eq!(addr.uri.scheme, Scheme::Sips);
        assert_eq!(addr.params.len(), 1);
        assert!(matches!(addr.params[0], Param::Tag(ref s) if s == "12345"));
    }

    #[test]
    fn test_parse_from_with_generic_param() {
        let input = b"Alice <sip:alice@host>;tag=xyz;myparam=value";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("Alice".to_string()));
        assert_eq!(addr.params.len(), 2);
        assert!(addr.params.contains(&Param::Tag("xyz".to_string())));
        assert!(addr.params.contains(&Param::Other("myparam".to_string(), Some(GenericValue::Token("value".to_string())))));
    }

    #[test]
    fn test_parse_from_no_params() {
        let input = b"sip:carol@chicago.com";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, None);
        assert!(addr.params.is_empty());
    }
    
    // Additional tests for RFC 3261 compliance
    
    #[test]
    fn test_from_multiple_parameters() {
        let input = b"<sip:alice@atlanta.com>;tag=1928301774;lr;transport=udp";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.params.len(), 3);
        assert!(addr.params.contains(&Param::Tag("1928301774".to_string())));
        assert!(addr.params.contains(&Param::Lr));
        assert!(addr.params.contains(&Param::Transport("udp".to_string())));
    }
    
    #[test]
    fn test_from_ipv4_addr() {
        let input = b"<sip:alice@192.168.1.1>;tag=123";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, None);
        match &addr.uri.host {
            Host::Address(ip) => assert!(ip.is_ipv4()),
            _ => panic!("Expected IPv4 address"),
        }
    }
    
    #[test]
    fn test_from_ipv6_addr() {
        let input = b"<sip:alice@[2001:db8::1]>;tag=456";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        match &addr.uri.host {
            Host::Address(ip) => assert!(ip.is_ipv6()),
            _ => panic!("Expected IPv6 address"),
        }
    }
    
    #[test]
    fn test_from_with_port() {
        let input = b"<sip:alice@example.com:5060>;tag=789";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.uri.port, Some(5060));
    }
    
    #[test]
    fn test_from_with_quoted_display_name() {
        let input = b"\"John Doe\" <sip:john@example.com>;tag=abc123";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("John Doe".to_string()));
    }
    
    #[test]
    fn test_from_with_escaped_chars_in_display_name() {
        let input = b"\"John \\\"Johnny\\\" Doe\" <sip:john@example.com>;tag=def456";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        // The display name should have the quotes properly unescaped
        assert_eq!(addr.display_name, Some("John \"Johnny\" Doe".to_string()));
    }
    
    #[test]
    fn test_from_uri_with_parameters() {
        let input = b"<sip:alice@example.com;transport=tcp>;tag=ghi789";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        // The transport parameter should be in the URI parameters, not the header parameters
        assert!(addr.uri.parameters.contains(&Param::Transport("tcp".to_string())));
        assert_eq!(addr.params.len(), 1); // Only the tag parameter should be in the header params
    }
    
    #[test]
    fn test_from_tag_parameter_case_insensitivity() {
        let input = b"<sip:alice@example.com>;TAG=case-test";
        let result = parse_from(input);
        assert!(result.is_ok());
        let (rem, from_header) = result.unwrap();
        let addr = from_header.0;
        assert!(rem.is_empty());
        assert!(addr.params.contains(&Param::Tag("case-test".to_string())));
    }
} 