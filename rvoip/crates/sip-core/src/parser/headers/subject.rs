// Parser for Subject header (RFC 3261 Section 20.38)
// Subject = ( "Subject" / "s" ) HCOLON [TEXT-UTF8-TRIM]

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, opt, map_res},
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::values::text_utf8_trim;
use crate::parser::ParseResult;

// Returns Option<&[u8]> representing the trimmed text
pub(crate) fn parse_subject(input: &[u8]) -> ParseResult<String> {
    // text_utf8_trim expects at least one char if not empty.
    // If input is empty, this will correctly fail (as it should for optional field)
    // If header exists but value is empty (e.g., "Subject: \r\n"), input will be empty.
    map_res(text_utf8_trim, |bytes| str::from_utf8(bytes).map(String::from))(input)
    // Note: text_utf8_trim doesn't actually trim leading/trailing LWS currently.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_subject() {
        let input = b"Urgent Meeting Request";
        let (rem, val) = parse_subject(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "Urgent Meeting Request");
    }

    #[test]
    fn test_parse_subject_empty() {
        // If the header value is truly empty after the colon, parse should fail
        // as text_utf8_trim requires 1*char
        let input = b""; 
        assert!(parse_subject(input).is_err());
    }
} 