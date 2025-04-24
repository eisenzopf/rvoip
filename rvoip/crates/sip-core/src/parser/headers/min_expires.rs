// Parser for Min-Expires header (RFC 3261 Section 20.23)
// Min-Expires = "Min-Expires" HCOLON delta-seconds
// delta-seconds = 1*DIGIT
//
// This header is used in SIP REGISTER responses (423 Interval Too Brief) 
// to indicate the minimum registration period accepted by the server.

use nom::IResult;

// Import delta_seconds parser
use crate::parser::values::delta_seconds;
use crate::parser::ParseResult;

/// Parses the Min-Expires header value according to RFC 3261 Section 20.23
/// Min-Expires = "Min-Expires" HCOLON delta-seconds
/// delta-seconds = 1*DIGIT
///
/// This parser handles only the value part (delta-seconds).
/// The "Min-Expires" token and HCOLON are parsed separately.
///
/// The value represents a time duration in seconds.
pub fn parse_min_expires(input: &[u8]) -> ParseResult<u32> {
    delta_seconds(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_min_expires() {
        // Basic valid cases
        let (rem, val) = parse_min_expires(b"3600").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3600);

        let (rem_zero, val_zero) = parse_min_expires(b"0").unwrap();
        assert!(rem_zero.is_empty());
        assert_eq!(val_zero, 0);

        // Maximum value test
        let (rem_max, val_max) = parse_min_expires(b"4294967295").unwrap();
        assert!(rem_max.is_empty());
        assert_eq!(val_max, 4294967295); // Max u32 value
    }

    #[test]
    fn test_remaining_input() {
        // Ensure parser stops at non-digit characters
        let (rem, val) = parse_min_expires(b"60;expires=3600").unwrap();
        assert_eq!(rem, b";expires=3600");
        assert_eq!(val, 60);

        // Ensure parser stops at whitespace
        let (rem, val) = parse_min_expires(b"180 ").unwrap();
        assert_eq!(rem, b" ");
        assert_eq!(val, 180);
    }

    #[test]
    fn test_rfc3261_examples() {
        // No explicit examples in RFC 3261 for Min-Expires, 
        // but we can test with common values
        let (rem, val) = parse_min_expires(b"1800").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 1800);

        // Section 10.3 example for "423 Interval Too Brief" response
        let (rem, val) = parse_min_expires(b"60").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 60);
    }

    #[test]
    fn test_invalid_min_expires() {
        // Empty input
        assert!(parse_min_expires(b"").is_err());
        
        // Non-numeric values
        assert!(parse_min_expires(b"abc").is_err());
        
        // Negative values
        assert!(parse_min_expires(b"-60").is_err());
        
        // Leading whitespace is not allowed
        assert!(parse_min_expires(b" 180").is_err());
        
        // Decimal values are not allowed in delta-seconds
        assert!(parse_min_expires(b"1800.5").is_err());
    }
} 