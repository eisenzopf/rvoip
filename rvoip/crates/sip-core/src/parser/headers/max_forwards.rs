// Parser for Max-Forwards header (RFC 3261 Section 20.22)
// Max-Forwards = "Max-Forwards" HCOLON 1*DIGIT

use nom::{
    character::complete::digit1,
    combinator::map_res,
    IResult,
    error::{ErrorKind, Error as NomError, ParseError},
};
use std::str;

use crate::parser::ParseResult;

/// Parses the Max-Forwards header value according to RFC 3261 Section 20.22
/// Max-Forwards = "Max-Forwards" HCOLON 1*DIGIT
/// 
/// Note: This parser handles only the value part (1*DIGIT).
/// The "Max-Forwards" token and HCOLON are parsed separately.
/// 
/// Returns a u8 for consistency with the MaxForwards type. Values exceeding u8::MAX
/// will be rejected with an error as per SIP RFC 3261 implementation guidelines.
pub fn parse_max_forwards(input: &[u8]) -> ParseResult<u8> {
    map_res(
        digit1, 
        |bytes| {
            let s = str::from_utf8(bytes).map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Digit)))?;
            s.parse::<u8>().map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Digit)))
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_max_forwards() {
        // Basic valid cases
        let (rem, val) = parse_max_forwards(b"70").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 70);

        // Zero is valid (used to terminate loops)
        let (rem_zero, val_zero) = parse_max_forwards(b"0").unwrap();
        assert!(rem_zero.is_empty());
        assert_eq!(val_zero, 0);
        
        // Maximum valid value for u8
        let (rem_max, val_max) = parse_max_forwards(b"255").unwrap();
        assert!(rem_max.is_empty());
        assert_eq!(val_max, 255);
        
        // Values exceeding u8::MAX should fail
        assert!(parse_max_forwards(b"256").is_err());
        assert!(parse_max_forwards(b"300").is_err());
    }

    #[test]
    fn test_remaining_input() {
        // Parser should only consume digits and leave other characters
        let (rem, val) = parse_max_forwards(b"70;param=value").unwrap();
        assert_eq!(rem, b";param=value");
        assert_eq!(val, 70);
        
        // Should stop at whitespace too
        let (rem_ws, val_ws) = parse_max_forwards(b"42 trailing").unwrap();
        assert_eq!(rem_ws, b" trailing");
        assert_eq!(val_ws, 42);
    }

    #[test]
    fn test_rfc3261_examples() {
        // From RFC 3261 examples (Section 7.3.1 and various message examples)
        // Example: "Max-Forwards: 70"
        let (rem, val) = parse_max_forwards(b"70").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 70);
        
        // Used in OPTION ping: "Max-Forwards: 0"
        let (rem, val) = parse_max_forwards(b"0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 0);
    }

    #[test]
    fn test_invalid_max_forwards() {
        // Invalid inputs per RFC 3261
        assert!(parse_max_forwards(b"").is_err()); // Empty (fails 1*DIGIT)
        assert!(parse_max_forwards(b"abc").is_err()); // Non-numeric
        assert!(parse_max_forwards(b"-10").is_err()); // Negative (fails DIGIT)
        assert!(parse_max_forwards(b"+10").is_err()); // Sign not allowed (fails DIGIT)
        
        // For decimal values, the parser will stop at the decimal point
        // This is correct per RFC 3261 as it only specifies DIGIT characters
        let (rem, val) = parse_max_forwards(b"10.5").unwrap();
        assert_eq!(rem, b".5");
        assert_eq!(val, 10);
        
        // Leading zeros are valid per RFC grammar but should parse as expected
        let (_, val) = parse_max_forwards(b"0070").unwrap();
        assert_eq!(val, 70);
        
        // Leading whitespace should fail (DIGIT doesn't include whitespace)
        // Note: In practice, HCOLON would consume leading whitespace before this parser
        assert!(parse_max_forwards(b" 70").is_err());
    }
} 