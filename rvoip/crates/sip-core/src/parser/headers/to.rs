// Parser for the To header (RFC 3261 Section 20.39)
// To = ( "To" / "t" ) HCOLON ( name-addr / addr-spec ) *( SEMI to-param )
// to-param = tag-param / generic-param

use nom::{
    branch::alt,
    combinator::{map, map_res},
    multi::many0,
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import parsers from other modules
use crate::parser::address::name_addr_or_addr_spec;
use crate::parser::common_params::{from_to_param, semicolon_separated_params0};
use crate::parser::separators::{hcolon, semi};
use crate::parser::ParseResult;

// Import types
use crate::types::address::Address;
use crate::types::param::Param;
use crate::types::to::To as ToHeader; // Import the specific header type
use crate::types::uri::{Host, Scheme};

// to-spec = ( name-addr / addr-spec ) *( SEMI to-param )
// to-param = tag-param / generic-param
// Returns Address struct with params included
fn to_spec(input: &[u8]) -> ParseResult<Address> {
    map(
        pair(
            name_addr_or_addr_spec, // Returns Address{..., params: []}
            many0(preceded(semi, from_to_param))
        ),
        |(mut addr, params_vec)| { // Make addr mutable
            addr.params = params_vec; // Assign parsed params
            addr // Return the modified Address
        }
    )(input)
}

// To = "To" / "t" HCOLON to-spec
// Note: HCOLON handled elsewhere
// Make this function public
pub fn parse_to(input: &[u8]) -> ParseResult<ToHeader> {
    map(to_spec, ToHeader)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::Address;
    use crate::types::uri::{Host, Scheme};
    use crate::types::param::{Param, GenericValue};
    use std::collections::HashMap;

    #[test]
    fn test_parse_to_simple_addr_spec() {
        let input = b"<sip:user@example.com>";
        let result = parse_to(input);
        assert!(result.is_ok());
        let (rem, to_header) = result.unwrap(); // Returns ToHeader
        let addr = to_header.0; // Access inner Address
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, None);
        assert_eq!(addr.uri.scheme, Scheme::Sip);
        assert!(addr.params.is_empty());
    }
    
    #[test]
    fn test_parse_to_name_addr_with_tag() {
        let input = b"\"Receiver\" <sips:recv@example.com>;tag=zxcv";
        let result = parse_to(input);
        assert!(result.is_ok());
        let (rem, to_header) = result.unwrap();
        let addr = to_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("Receiver".to_string()));
        assert_eq!(addr.uri.scheme, Scheme::Sips);
        assert_eq!(addr.params.len(), 1);
        assert!(matches!(addr.params[0], Param::Tag(ref s) if s == "zxcv"));
    }

    #[test]
    fn test_parse_to_with_generic_param() {
        let input = b"Alice <sip:alice@host>;myparam=value;tag=abc";
        let result = parse_to(input);
        assert!(result.is_ok());
        let (rem, to_header) = result.unwrap();
        let addr = to_header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("Alice".to_string()));
        assert_eq!(addr.params.len(), 2);
        assert!(addr.params.contains(&Param::Tag("abc".to_string())));
        assert!(addr.params.contains(&Param::Other("myparam".to_string(), Some(GenericValue::Token("value".to_string())))));
    }
} 