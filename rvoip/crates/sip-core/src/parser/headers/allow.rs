// Parser for Allow header (RFC 3261 Section 20.5)
// Allow = "Allow" HCOLON [Method *(COMMA Method)]
// Method = token

use nom::{
    bytes::complete::tag_no_case,
    combinator::{map, opt, map_res},
    sequence::{pair, preceded},
    IResult,
};

// Import from new modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token; // Method is token
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;

// Import shared parser
use super::token_list::parse_token_list0;

use crate::types::allow::Allow;
use crate::types::method::Method;
use nom::combinator::{map, map_res, opt};
use nom::multi::many0; // Import many0
use std::str::{self, FromStr}; // Import self for FromStr

// Define a separate function for the item parser
fn parse_method_token(input: &[u8]) -> ParseResult<Method> {
     map_res(token, |m| Method::from_str(std::str::from_utf8(m)?))(input) // Ensure std::str::from_utf8 is used
}

// Allow = "Allow" HCOLON [ Method *(COMMA Method) ]
// Note: HCOLON handled elsewhere
pub fn parse_allow(input: &[u8]) -> ParseResult<Allow> {
    map(
        comma_separated_list0(token), // Methods are tokens
        |methods| {
            Allow(methods)
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_allow() {
        let input = b"INVITE, ACK, OPTIONS, CANCEL, BYE";
        let (rem, allow_list) = parse_allow(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(allow_list, Allow(vec!["INVITE", "ACK", "OPTIONS", "CANCEL", "BYE"].iter().map(|&m| Method::from_str(m).unwrap()).collect()));

        let input_empty = b"";
        let (rem_empty, allow_empty) = parse_allow(input_empty).unwrap();
        assert!(rem_empty.is_empty());
        assert!(allow_empty.is_empty());
    }
}