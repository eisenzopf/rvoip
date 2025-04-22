// This file now only declares the submodule
pub mod contact;

// RFC 3261 Section 20.10 Contact

use nom::{
    branch::alt,
    bytes::complete::tag,
    combinator::{map, value},
    multi::{many0, separated_list0},
    sequence::{pair, preceded},
    IResult,
};

// Import helpers
use crate::parser::address::name_addr_or_addr_spec;
use crate::parser::common::{comma_separated_list0, comma_separated_list1};
use crate::parser::common_params::contact_param_item;
use crate::parser::separators::{comma, semi, star};
use crate::parser::ParseResult;

// Import types (assuming)
use crate::types::address::Address;
use crate::types::param::Param;
use crate::types::header::ContactValue; // Assuming ContactValue::Star and ContactValue::Params
use crate::types::contact::ContactParamInfo; // Assuming struct { address: Address, params: Vec<Param> }

// contact-param = (name-addr / addr-spec) *(SEMI contact-params)
// contact-params = c-p-q / c-p-expires / contact-extension
fn contact_param(input: &[u8]) -> ParseResult<ContactParamInfo> {
    map(
        pair(
            name_addr_or_addr_spec,
            many0(preceded(semi, contact_param_item))
        ),
        |(mut addr, params_vec)| {
            addr.params = params_vec;
            ContactParamInfo { address: addr }
        }
    )(input)
}

// Contact = ("Contact" / "m") HCOLON (STAR / (contact-param *(COMMA contact-param)))
// Note: HCOLON and compact form handled elsewhere.
pub(crate) fn parse_contact(input: &[u8]) -> ParseResult<ContactValue> {
    alt((
        // Handle the STAR case
        value(ContactValue::Star, star), // Use star parser which handles SWS * SWS
        
        // Handle the comma-separated list case
        // Use comma_separated_list1 to require at least one contact-param if not STAR
        map(comma_separated_list1(contact_param), |params| ContactValue::Params(params))
    ))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::address::{Address};
    use crate::types::param::{GenericValue};
    use crate::types::uri::Uri;
    use ordered_float::NotNan;

    #[test]
    fn test_parse_contact_star() {
        let input = b" * ";
        let result = parse_contact(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert!(matches!(val, ContactValue::Star));
    }

    #[test]
    fn test_parse_contact_single_addr_spec() {
        let input = b"<sip:user@host.com>";
        let result = parse_contact(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        if let ContactValue::Params(params) = val {
            assert_eq!(params.len(), 1);
            assert!(params[0].address.display_name.is_none());
            assert_eq!(params[0].address.uri.scheme, "sip");
            assert!(params[0].address.params.is_empty());
        } else {
            panic!("Expected Params variant");
        }
    }
    
    #[test]
    fn test_parse_contact_single_name_addr_params() {
        let input = b"\"Mr. Watson\" <sip:watson@bell.com>;q=0.7;expires=3600";
        let result = parse_contact(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        if let ContactValue::Params(params) = val {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].address.display_name, Some("Mr. Watson".to_string()));
            assert_eq!(params[0].address.uri.scheme, "sip");
            assert_eq!(params[0].address.params.len(), 2);
            assert!(params[0].address.params.contains(&Param::Q(NotNan::new(0.7).unwrap())));
            assert!(params[0].address.params.contains(&Param::Expires(3600)));
        } else {
            panic!("Expected Params variant");
        }
    }
    
    #[test]
    fn test_parse_contact_multiple() {
        let input = b"<sip:A@atlanta.com>, \"Bob\" <sip:bob@biloxi.com>;tag=123";
        let result = parse_contact(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        if let ContactValue::Params(params) = val {
            assert_eq!(params.len(), 2);
            // First contact
            assert!(params[0].address.display_name.is_none());
            assert!(params[0].address.params.is_empty());
            // Second contact
            assert_eq!(params[1].address.display_name, Some("Bob".to_string()));
            assert_eq!(params[1].address.params.len(), 1);
            assert!(matches!(params[1].address.params[0], Param::Tag(ref s) if s == "123"));
        } else {
            panic!("Expected Params variant");
        }
    }
}