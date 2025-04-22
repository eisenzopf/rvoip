// Parser for Require header (RFC 3261 Section 20.32)
// Require = "Require" HCOLON option-tag *(COMMA option-tag)
// option-tag = token

use nom::{
    multi::separated_list1, // Require needs at least one tag
    IResult,
};
use std::str;

// Import shared parser
use super::token_list::token_string; // Need underlying token parser
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;

// Require = "Require" HCOLON option-tag *(COMMA option-tag)
// Note: HCOLON handled elsewhere. option-tag is token.
pub fn parse_require(input: &[u8]) -> ParseResult<Vec<String>> {
    // Require MUST have at least one tag if present
    comma_separated_list1(token_string)(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_require() {
        let input = b"100rel, precondition";
        let (rem, req_list) = parse_require(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(req_list, vec!["100rel", "precondition"]);

        let input_single = b"timer";
        let (rem_single, req_single) = parse_require(input_single).unwrap();
        assert!(rem_single.is_empty());
        assert_eq!(req_single, vec!["timer"]);
    }

    #[test]
    fn test_parse_require_empty_fail() {
        assert!(parse_require(b"").is_err());
    }
} 