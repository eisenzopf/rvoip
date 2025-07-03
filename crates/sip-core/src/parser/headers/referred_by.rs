// Parser for the Referred-By header (RFC 3892)
// Referred-By = "Referred-By" HCOLON (name-addr / addr-spec) *(SEMI referredby-param)
// referredby-param = generic-param / "cid" EQUAL token

use nom::{
    bytes::complete::{tag, tag_no_case, take_while1},
    combinator::{map, map_res},
    sequence::{pair, preceded},
    IResult,
    multi::many0,
    branch::alt,
};

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, equal};
use crate::parser::address::name_addr_or_addr_spec;
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::ParseResult;
use crate::parser::token::{token, is_token_char};

use crate::types::param::Param;
use crate::types::uri::Uri;
use crate::types::address::Address;
use crate::types::referred_by::ReferredBy;
use serde::{Serialize, Deserialize};
use std::str::{self, FromStr};

// cid parameter parser for Referred-By header
// cid-param = "cid" EQUAL token
fn cid_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(
            pair(tag_no_case(b"cid"), equal),
            take_while1(|c| {
                // Token characters plus '@' for email-like identifiers
                is_token_char(c) || c == b'@'
            })
        ),
        |cid_bytes| std::str::from_utf8(cid_bytes).map(|s| Param::new("cid", Some(s.to_string())))
    )(input)
}

// referredby-param parser that handles specific params before falling back to generic
fn referredby_param(input: &[u8]) -> ParseResult<Param> {
    alt((
        cid_param,      // First try cid parameter
        generic_param,  // Then generic parameters
    ))(input)
}

/// Parse a Referred-By header value according to RFC 3892
/// 
/// Syntax:
/// Referred-By = "Referred-By" HCOLON (name-addr / addr-spec) *(SEMI referredby-param)
/// referredby-param = generic-param / "cid" EQUAL token
///
/// Returns an Address struct with parameters
fn referred_by_spec(input: &[u8]) -> ParseResult<Address> {
    map(
        pair(
            name_addr_or_addr_spec, // Parse the address part (with or without display name)
            many0(preceded(semi, referredby_param)) // Parse any parameters that follow
        ),
        |(mut addr, params_vec)| {
            addr.params = params_vec; // Assign parsed parameters
            addr // Return Address directly
        }
    )(input)
}

/// Parse a complete Referred-By header value
pub fn parse_referred_by(input: &[u8]) -> ParseResult<Address> {
    referred_by_spec(input)
}

/// Public API for parsing a Referred-By header value.
/// This properly handles both name-addr and addr-spec formats,
/// and includes any parameters that follow.
pub fn parse_referred_by_public(input: &[u8]) -> ParseResult<Address> {
    referred_by_spec(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::{Scheme, Host};
    use crate::types::param::{Param, GenericValue};
    use nom::combinator::all_consuming;

    #[test]
    fn test_parse_referred_by_simple() {
        // Simple SIP URI in angle brackets (name-addr format)
        let input = b"<sip:alice@example.com>";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, None);
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert!(address.params.is_empty());
    }
    
    #[test]
    fn test_parse_referred_by_with_display_name() {
        // name-addr format with display name
        let input = b"\"Alice\" <sip:alice@example.com>";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, Some("Alice".to_string()));
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert!(address.params.is_empty());
    }
    
    #[test]
    fn test_parse_referred_by_addr_spec() {
        // addr-spec format (no angle brackets)
        let input = b"sip:alice@example.com";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, None);
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert!(address.params.is_empty());
    }
    
    #[test]
    fn test_parse_referred_by_with_params() {
        // Referred-By with parameters
        let input = b"<sip:alice@example.com>;cid=12345@atlanta.example.com";
        println!("Input: {:?}", std::str::from_utf8(input).unwrap());
        let result = parse_referred_by_public(input);
        println!("Parser result: {:?}", result);
        
        match &result {
            Ok((rem, _)) => println!("Remaining: {:?}", std::str::from_utf8(rem).unwrap_or("Invalid UTF-8")),
            Err(e) => println!("Error: {:?}", e),
        }
        
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 1);
        
        // Check for cid parameter - allow for both Token and Quoted values
        let cid_param_found = address.params.iter().any(|p| 
            if let Param::Other(n, Some(v)) = p {
                n == "cid" && v.to_string().contains("12345")
            } else {
                false
            }
        );
        assert!(cid_param_found);
        
        // Dump the params for debugging
        println!("Params: {:?}", address.params);
    }
    
    #[test]
    fn test_parse_referred_by_with_generic_param() {
        // Referred-By with generic parameter
        let input = b"<sip:alice@example.com>;purpose=call-transfer";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 1);
        
        // Check for purpose parameter
        assert!(address.params.iter().any(|p| 
            if let Param::Other(n, Some(GenericValue::Token(v))) = p {
                n == "purpose" && v == "call-transfer"
            } else {
                false
            }
        ));
    }
    
    #[test]
    fn test_parse_referred_by_with_display_name_and_params() {
        // Full example with display name and parameters
        let input = b"\"Alice\" <sip:alice@atlanta.example.com>;cid=12345@atlanta.example.com;purpose=transfer";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, Some("Alice".to_string()));
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 2);
        
        // Check for cid parameter - allow for both Token and Quoted values
        let cid_param_found = address.params.iter().any(|p| 
            if let Param::Other(n, Some(v)) = p {
                n == "cid" && v.to_string().contains("12345")
            } else {
                false
            }
        );
        assert!(cid_param_found);
        
        // Check for purpose parameter
        let purpose_param_found = address.params.iter().any(|p| 
            if let Param::Other(n, Some(v)) = p {
                n == "purpose" && v.to_string().contains("transfer")
            } else {
                false
            }
        );
        assert!(purpose_param_found);
    }
    
    #[test]
    fn test_parse_referred_by_sips_uri() {
        // SIPS URI
        let input = b"<sips:alice@secure.example.com>";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sips);
    }
    
    #[test]
    fn test_parse_referred_by_uri_with_params() {
        // URI with parameters inside angle brackets
        let input = b"<sip:alice@example.com;transport=tcp>";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.uri.scheme, Scheme::Sip);
        
        // URI parameters should be in the uri.parameters field, not address.params
        assert!(address.params.is_empty());
        assert!(address.uri.parameters.contains(&Param::Transport("tcp".to_string())));
    }
    
    #[test]
    fn test_parse_referred_by_complex() {
        // Complex example with multiple parameters
        let input = b"\"Sales Department\" <sip:sales@example.com;transport=udp>;cid=sales-ref-12345@example.com;expires=3600";
        let result = parse_referred_by_public(input);
        assert!(result.is_ok());
        let (rem, address) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(address.display_name, Some("Sales Department".to_string()));
        assert_eq!(address.uri.scheme, Scheme::Sip);
        assert_eq!(address.params.len(), 2);
        assert!(address.uri.parameters.contains(&Param::Transport("udp".to_string())));
        
        // Check parameters - allow for both Token and Quoted values
        let cid_param_found = address.params.iter().any(|p| 
            if let Param::Other(n, Some(v)) = p {
                n == "cid" && v.to_string().contains("sales-ref-12345")
            } else {
                false
            }
        );
        assert!(cid_param_found);
        
        // Check expires parameter
        let expires_param_found = address.params.iter().any(|p| 
            if let Param::Other(n, Some(v)) = p {
                n == "expires" && v.to_string().contains("3600")
            } else {
                false
            }
        );
        assert!(expires_param_found);
    }
    
    #[test]
    fn test_parse_referred_by_empty_should_fail() {
        let input = b"";
        let result = parse_referred_by_public(input);
        assert!(result.is_err());
    }
} 