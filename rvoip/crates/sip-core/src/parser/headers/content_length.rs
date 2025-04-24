// Parser for Content-Length header (RFC 3261 Section 20.14)
// Content-Length = ( "Content-Length" / "l" ) HCOLON 1*DIGIT

use nom::{
    character::complete::digit1,
    combinator::map_res,
    IResult,
    error::{ErrorKind, Error as NomError, ParseError},
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;

/// Parses the Content-Length header value according to RFC 3261 Section 20.14
/// Content-Length = ( "Content-Length" / "l" ) HCOLON 1*DIGIT
///
/// Note: This parser handles only the value part (1*DIGIT).
/// The "Content-Length"/"l" token and HCOLON are parsed separately.
pub fn parse_content_length(input: &[u8]) -> ParseResult<u32> {
    map_res(
        digit1, 
        |bytes| {
            let s = str::from_utf8(bytes).map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Digit)))?;
            s.parse::<u32>().map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Digit)))
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_length() {
        // Basic valid cases
        let (rem, val) = parse_content_length(b"3495").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3495);

        // Zero is valid per RFC (empty body)
        let (rem_zero, val_zero) = parse_content_length(b"0").unwrap();
        assert!(rem_zero.is_empty());
        assert_eq!(val_zero, 0);
        
        // Large values
        let (rem_large, val_large) = parse_content_length(b"4294967295").unwrap(); // max u32
        assert!(rem_large.is_empty());
        assert_eq!(val_large, 4294967295);
    }
    
    #[test]
    fn test_remaining_input() {
        // Parser should only consume digits and leave other characters
        let (rem, val) = parse_content_length(b"123;charset=utf-8").unwrap();
        assert_eq!(rem, b";charset=utf-8");
        assert_eq!(val, 123);
        
        // Should stop at whitespace too
        let (rem_ws, val_ws) = parse_content_length(b"456 bytes").unwrap();
        assert_eq!(rem_ws, b" bytes");
        assert_eq!(val_ws, 456);
    }
    
    #[test]
    fn test_rfc3261_examples() {
        // From RFC 3261 examples (various message examples)
        // Example: "Content-Length: 13"
        let (rem, val) = parse_content_length(b"13").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 13);
        
        // Example: "l: 0" (empty body)
        let (rem, val) = parse_content_length(b"0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 0);
    }

    #[test]
    fn test_invalid_content_length() {
        // Invalid inputs per RFC 3261
        assert!(parse_content_length(b"").is_err()); // Empty (fails 1*DIGIT)
        assert!(parse_content_length(b"abc").is_err()); // Non-numeric
        assert!(parse_content_length(b"-10").is_err()); // Negative (fails DIGIT)
        assert!(parse_content_length(b"+10").is_err()); // Sign not allowed (fails DIGIT)
        
        // For decimal values, the parser stops at the decimal point
        let (rem, val) = parse_content_length(b"123.45").unwrap();
        assert_eq!(rem, b".45");
        assert_eq!(val, 123);
        
        // Leading zeros are valid per RFC grammar
        let (_, val) = parse_content_length(b"0042").unwrap();
        assert_eq!(val, 42);
        
        // Leading whitespace should fail (DIGIT doesn't include whitespace)
        assert!(parse_content_length(b" 123").is_err());
        
        // Trailing whitespace is handled correctly - parser stops at whitespace
        let (rem, val) = parse_content_length(b"10 ").unwrap();
        assert_eq!(rem, b" ");
        assert_eq!(val, 10);
    }
} 