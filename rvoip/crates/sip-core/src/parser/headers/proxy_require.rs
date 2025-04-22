// Parser for Proxy-Require header (RFC 3261 Section 20.29)
// Proxy-Require = "Proxy-Require" HCOLON option-tag *(COMMA option-tag)
// option-tag = token

use nom::{
    multi::separated_list1, // Proxy-Require needs at least one tag
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token;
use crate::parser::common::comma_separated_list1; // Proxy-Require needs at least one
use crate::parser::ParseResult;

// Import shared parser
use super::token_list::token_string; // Need underlying token parser

// Parse the comma-separated list of option-tags (tokens)
fn option_tag_list(input: &[u8]) -> ParseResult<Vec<&[u8]>> {
    comma_separated_list1(token)(input)
}

pub(crate) fn parse_proxy_require(input: &[u8]) -> ParseResult<Vec<String>> {
    // Proxy-Require MUST have at least one tag if present
    comma_separated_list1(token_string)(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_proxy_require() {
        let input = b"sec-agree";
        let (rem, preq_list) = parse_proxy_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(preq_list, vec!["sec-agree"]);

        let input_multi = b"foo , bar";
        let (rem_multi, preq_multi) = parse_proxy_require(input_multi).unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(preq_multi, vec!["foo", "bar"]);
    }

     #[test]
    fn test_parse_proxy_require_empty_fail() {
        assert!(parse_proxy_require(b"").is_err());
    }
} 