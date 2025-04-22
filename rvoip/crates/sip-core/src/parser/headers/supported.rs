// Parser for Supported header (RFC 3261 Section 20.38)
// Supported = ( "Supported" / "k" ) HCOLON [ option-tag *(COMMA option-tag) ]
// option-tag = token

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, opt},
    sequence::{pair, preceded},
    IResult,
};

// Import from new modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token;
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;

// Import shared parser
use super::token_list::parse_token_list0;

// Parse the comma-separated list of option-tags (tokens)
fn option_tag_list(input: &[u8]) -> ParseResult<Vec<&[u8]>> {
    comma_separated_list0(token)(input)
}

pub fn parse_supported(input: &[u8]) -> ParseResult<Vec<String>> {
    parse_token_list0(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_supported() {
        let input = b"timer, 100rel";
        let (rem, sup_list) = parse_supported(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(sup_list, vec!["timer", "100rel"]);

        let input_empty = b"";
        let (rem_empty, sup_empty) = parse_supported(input_empty).unwrap();
        assert!(rem_empty.is_empty());
        assert!(sup_empty.is_empty());
    }
} 