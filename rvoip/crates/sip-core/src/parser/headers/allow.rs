// Parser for Allow header (RFC 3261 Section 20.5)
// Allow = "Allow" HCOLON [Method *(COMMA Method)]
// Method = token

use nom::{
    bytes::complete::tag_no_case,
    combinator::{map, opt},
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

// Parse the comma-separated list of Methods (tokens)
fn method_list(input: &[u8]) -> ParseResult<Vec<&[u8]>> {
    comma_separated_list0(token)(input)
}

pub(crate) fn parse_allow(input: &[u8]) -> ParseResult<Vec<String>> {
    parse_token_list0(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_allow() {
        let input = b"INVITE, ACK, OPTIONS, CANCEL, BYE";
        let (rem, allow_list) = parse_allow(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(allow_list, vec!["INVITE", "ACK", "OPTIONS", "CANCEL", "BYE"]);

        let input_empty = b"";
        let (rem_empty, allow_empty) = parse_allow(input_empty).unwrap();
        assert!(rem_empty.is_empty());
        assert!(allow_empty.is_empty());
    }
}