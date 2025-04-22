// Parser for Unsupported header (RFC 3261 Section 20.41)
// Unsupported = "Unsupported" HCOLON option-tag *(COMMA option-tag)
// option-tag = token

use nom::{
    multi::separated_list1, // Unsupported needs at least one tag
    IResult,
};
use std::str;

// Import shared parser
use super::token_list::token_string; // Need underlying token parser
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;

// Unsupported = "Unsupported" HCOLON option-tag *(COMMA option-tag)
// Note: HCOLON handled elsewhere.
pub(crate) fn parse_unsupported(input: &[u8]) -> ParseResult<Vec<String>> {
    // Unsupported MUST have at least one tag if present
    comma_separated_list1(token_string)(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unsupported() {
        let input = b"bad-feature, worse-feature";
        let (rem, unsup_list) = parse_unsupported(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(unsup_list, vec!["bad-feature", "worse-feature"]);
    }

     #[test]
    fn test_parse_unsupported_empty_fail() {
        assert!(parse_unsupported(b"").is_err());
    }
} 