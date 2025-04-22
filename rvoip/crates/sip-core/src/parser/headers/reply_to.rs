// Parser for the Reply-To header (RFC 3261 Section 20.32)
// Reply-To = "Reply-To" HCOLON rplyto-spec
// rplyto-spec = ( name-addr / addr-spec ) *( SEMI rplyto-param )
// rplyto-param = generic-param

use nom::{
    bytes::complete::tag_no_case,
    combinator::{map, map_res},
    sequence::{pair, preceded},
    IResult,
    multi::many0,
};

// Import from base parser modules
use crate::parser::separators::{hcolon, semi};
use crate::parser::address::name_addr_or_addr_spec;
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::ParseResult;

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
    map(rplyto_spec, ReplyToHeader)(input)
}

/// Parses a Reply-To header value.
pub fn parse_reply_to_public(input: &[u8]) -> ParseResult<Address> {
    // For now, assume it's just a single name-addr
    name_addr(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::Address;
    use crate::types::uri::Scheme;
    use crate::types::param::{Param, GenericValue};
    use std::collections::HashMap;

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
} 