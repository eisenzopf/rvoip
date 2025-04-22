// Generic parser for headers that are lists of tokens, possibly with a short form.

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, map_res, opt},
    sequence::{pair, preceded},
    multi::{separated_list1},
    IResult,
};

// Import from new modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token;
use crate::parser::common::comma_separated_list0;
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;

// Import shared parsers
// Removed duplicate imports:
// use crate::parser::common::comma_separated_list0;
// use crate::parser::token::token;
// use crate::parser::ParseResult;

use std::str;

/// Parses HCOLON [ token *(COMMA token) ]
/// Requires at least one token if the value is present.
fn token_list1(input: &[u8]) -> ParseResult<Vec<&[u8]>> {
    comma_separated_list1(token)(input)
}

/// Parses HCOLON [ token *(COMMA token) ]
/// Allows an empty list.
fn token_list0(input: &[u8]) -> ParseResult<Vec<&[u8]>> {
    comma_separated_list0(token)(input)
}

/// Parses a header (long form only) with a comma-separated list of tokens (at least one required).
/// Example: "HeaderName: token1, token2"
pub fn parse_header_token_list1<'a>(name: &'a [u8], input: &'a [u8]) -> ParseResult<'a, Vec<&'a [u8]>> {
    preceded(
        pair(tag_no_case(name), hcolon),
        token_list1
    )(input)
}

/// Parses a header (long form only) with an optional comma-separated list of tokens.
/// Example: "HeaderName: token1, token2" or "HeaderName:"
pub fn parse_header_token_list0<'a>(name: &'a [u8], input: &'a [u8]) -> ParseResult<'a, Vec<&'a [u8]>> {
    preceded(
        pair(tag_no_case(name), hcolon),
        token_list0
    )(input)
}

/// Parses a header (long or short form) with a comma-separated list of tokens (at least one required).
pub fn parse_header_token_list1_short<'a>(
    long_name: &'a [u8],
    short_name: &'a [u8],
    input: &'a [u8],
) -> ParseResult<'a, Vec<&'a [u8]>> {
    preceded(
        pair(alt((tag_no_case(long_name), tag_no_case(short_name))), hcolon),
        token_list1
    )(input)
}

/// Parses a header (long or short form) with an optional comma-separated list of tokens.
pub fn parse_header_token_list0_short<'a>(
    long_name: &'a [u8],
    short_name: &'a [u8],
    input: &'a [u8],
) -> ParseResult<'a, Vec<&'a [u8]>> {
    preceded(
        pair(alt((tag_no_case(long_name), tag_no_case(short_name))), hcolon),
        token_list0
    )(input)
}

// Define structure for a list of tokens
#[derive(Debug, PartialEq, Clone)]
pub struct TokenList(pub Vec<String>); // Use String to hold tokens

// Parses a comma-separated list of tokens
pub fn parse_token_list0(input: &[u8]) -> ParseResult<Vec<String>> {
    comma_separated_list0(token_string)(input)
}

pub fn parse_token_list1(input: &[u8]) -> ParseResult<Vec<String>> {
    comma_separated_list1(token_string)(input)
}

// Helper to parse a token into a String
pub fn token_string(input: &[u8]) -> ParseResult<String> { // Already pub
    map_res(token, |b| str::from_utf8(b).map(String::from))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_token_list0() {
        let (rem, list) = parse_token_list0(b"token1, token-2 , tag3").unwrap();
        assert!(rem.is_empty());
        assert_eq!(list, vec!["token1".to_string(), "token-2".to_string(), "tag3".to_string()]);

        let (rem_single, list_single) = parse_token_list0(b"ACK").unwrap();
        assert!(rem_single.is_empty());
        assert_eq!(list_single, vec!["ACK".to_string()]);

        let (rem_empty, list_empty) = parse_token_list0(b"").unwrap();
        assert!(rem_empty.is_empty());
        assert!(list_empty.is_empty());
    }
} 