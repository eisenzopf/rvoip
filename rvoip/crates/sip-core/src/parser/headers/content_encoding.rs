// Parser for Content-Encoding header (RFC 3261 Section 20.14)
// Content-Encoding = ( "Content-Encoding" / "e" ) HCOLON content-coding *(COMMA content-coding)
// content-coding = token

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
use crate::parser::common::comma_separated_list1; // Requires at least one
use crate::parser::ParseResult;
use std::str;
use nom::combinator::map_res;

// Parse the comma-separated list of content-codings (tokens)
fn content_coding_list(input: &[u8]) -> ParseResult<Vec<&[u8]>> {
    comma_separated_list1(token)(input)
}

// content-coding = token
fn content_coding(input: &[u8]) -> ParseResult<String> {
    map_res(token, |bytes| str::from_utf8(bytes).map(String::from))(input)
}

pub fn parse_content_encoding(input: &[u8]) -> ParseResult<Vec<String>> {
    // Use comma_separated_list1 as at least one coding is needed if header is present
    comma_separated_list1(content_coding)(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_coding() {
        let (rem, coding) = content_coding(b"gzip").unwrap();
        assert!(rem.is_empty());
        assert_eq!(coding, "gzip");
    }

    #[test]
    fn test_parse_content_encoding_single() {
        let input = b"deflate";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["deflate".to_string()]);
    }
    
    #[test]
    fn test_parse_content_encoding_multiple() {
        let input = b"gzip, identity";
        let result = parse_content_encoding(input);
        assert!(result.is_ok());
        let (rem, codings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(codings, vec!["gzip".to_string(), "identity".to_string()]);
    }

     #[test]
    fn test_parse_content_encoding_empty_fail() {
        // Header value cannot be empty according to ABNF (1*content-coding)
        let input = b"";
        let result = parse_content_encoding(input);
        assert!(result.is_err());
    }
} 