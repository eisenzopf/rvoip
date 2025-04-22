// Parser for Content-Length header (RFC 3261 Section 20.14)
// Content-Length = ( "Content-Length" / "l" ) HCOLON 1*DIGIT

use nom::{
    character::complete::digit1,
    combinator::map_res,
    IResult,
    error::{ErrorKind, NomError},
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;

pub(crate) fn parse_content_length(input: &[u8]) -> ParseResult<u32> {
    map_res(
        digit1, 
        |bytes| {
            let s = str::from_utf8(bytes).map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Char)))?;
            s.parse::<u32>().map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Digit)))
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_length() {
        let (rem, val) = parse_content_length(b"3495").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3495);

        let (rem_zero, val_zero) = parse_content_length(b"0").unwrap();
        assert!(rem_zero.is_empty());
        assert_eq!(val_zero, 0);
    }

    #[test]
    fn test_invalid_content_length() {
        assert!(parse_content_length(b"").is_err());
        assert!(parse_content_length(b"abc").is_err());
        assert!(parse_content_length(b"-10").is_err());
        assert!(parse_content_length(b"10 ").is_err()); // Trailing space not allowed by digit1
    }
} 