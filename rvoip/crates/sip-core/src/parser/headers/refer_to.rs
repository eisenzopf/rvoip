// Parser for the Refer-To header (RFC 3515)
// Refer-To = "Refer-To" HCOLON (name-addr / addr-spec) *( SEMI refer-param )
// refer-param = generic-param

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
use crate::types::refer_to::ReferTo as ReferToHeader;
use crate::parser::address::name_addr;
use serde::{Serialize, Deserialize};

// Define a struct to represent the Refer-To header value
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ReferToValue {
    pub display_name: Option<String>,
    pub uri: Uri,
    pub params: Vec<Param>,
}

// refer-to-spec = ( name-addr / addr-spec ) *( SEMI refer-param )
// refer-param = generic-param
// Returns Address struct with params included
fn refer_to_spec(input: &[u8]) -> ParseResult<Address> {
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

// Refer-To = "Refer-To" HCOLON refer-to-spec
// Note: HCOLON handled elsewhere
pub fn parse_refer_to(input: &[u8]) -> ParseResult<ReferToHeader> {
    map(refer_to_spec, ReferToHeader)(input)
}

/// Parses a Refer-To header value.
pub fn parse_refer_to_public(input: &[u8]) -> ParseResult<Address> {
    // For now, assume it's just a single name-addr
    name_addr(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::{AddressSpec, NameAddr};
    use crate::types::param::{Param, GenericValue};
    use std::collections::HashMap;

    #[test]
    fn test_parse_refer_to_simple() {
        let input = b"<sip:user@example.com>";
        let result = parse_refer_to_public(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        let addr = header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, None);
        assert_eq!(addr.uri.scheme, "sip");
        assert!(addr.params.is_empty());
    }
    
    #[test]
    fn test_parse_refer_to_name_addr_params() {
        let input = b"\"Transfer Target\" <sip:target@example.com>;method=INVITE";
        let result = parse_refer_to_public(input);
        assert!(result.is_ok());
        let (rem, header) = result.unwrap();
        let addr = header.0;
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("Transfer Target".to_string()));
        assert_eq!(addr.uri.scheme, "sip");
        assert_eq!(addr.params.len(), 1);
        assert!(addr.params.contains(&Param::Other("method".to_string(), Some(GenericValue::Token("INVITE".to_string())))));
    }
} 