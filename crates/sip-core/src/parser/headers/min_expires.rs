// Parser for Min-Expires header (RFC 3261 Section 20.23)
// Min-Expires = "Min-Expires" HCOLON delta-seconds
// delta-seconds = 1*DIGIT
//
// This header is used in SIP REGISTER responses (423 Interval Too Brief) 
// to indicate the minimum registration period accepted by the server.

use nom::{
    sequence::delimited,
    IResult,
};

// Import delta_seconds parser
use crate::parser::values::delta_seconds;
use crate::parser::ParseResult;
use crate::parser::whitespace::sws;

/// Parses the Min-Expires header value according to RFC 3261 Section 20.23
/// Min-Expires = "Min-Expires" HCOLON delta-seconds
/// delta-seconds = 1*DIGIT
///
/// This parser handles only the value part (delta-seconds).
/// The "Min-Expires" token and HCOLON are parsed separately.
///
/// The value represents a time duration in seconds, typically indicating
/// the minimum registration period accepted by a server. This is commonly 
/// used in 423 (Interval Too Brief) responses to REGISTER requests.
///
/// RFC 3261 does not specify a maximum value, but excessively large values
/// should be used with caution. Values of 0 are allowed by the specification.
pub fn parse_min_expires(input: &[u8]) -> ParseResult<u32> {
    // Use delimited to handle optional whitespace around the value
    // sws = [LWS] (optional linear whitespace) - RFC 3261 Section 25.1
    delimited(
        sws,
        delta_seconds,
        sws
    )(input)
}

/// A validator function that checks if a Min-Expires value is reasonable
/// 
/// While RFC 3261 doesn't specify limits, this function can be used to
/// check if a value is within a reasonable range for typical SIP usage.
/// 
/// Returns true if the value is reasonable, false otherwise.
pub fn is_reasonable_min_expires(value: u32) -> bool {
    // RFC 3261 doesn't specify an upper limit, but one year seems 
    // like a reasonable maximum for most applications
    value <= 31_536_000 // One year in seconds
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
        
        // With whitespace
        let (rem_ws, val_ws) = parse_min_expires(b" 1800 ").unwrap();
        assert!(rem_ws.is_empty());
        assert_eq!(val_ws, 1800);
    }
    
    #[test]
    fn test_leading_zeros() {
        // According to ABNF, leading zeros are allowed in 1*DIGIT
        let (rem, val) = parse_min_expires(b"0060").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 60);
        
        // Multiple leading zeros
        let (rem_multi, val_multi) = parse_min_expires(b"00001800").unwrap();
        assert!(rem_multi.is_empty());
        assert_eq!(val_multi, 1800);
        
        // All zeros
        let (rem_all, val_all) = parse_min_expires(b"000").unwrap();
        assert!(rem_all.is_empty());
        assert_eq!(val_all, 0);
    }

    #[test]
    fn test_remaining_input() {
        // Ensure parser stops at non-digit characters
        let (rem, val) = parse_min_expires(b"60;expires=3600").unwrap();
        assert_eq!(rem, b";expires=3600");
        assert_eq!(val, 60);

        // Ensure parser handles whitespace correctly
        let (rem, val) = parse_min_expires(b"180 ;param").unwrap();
        assert_eq!(rem, b";param");
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
    fn test_abnf_compliance() {
        // Tests specifically for ABNF compliance with RFC 3261
        
        // 1*DIGIT - at least one digit
        assert!(parse_min_expires(b"1").is_ok());
        
        // Delta-seconds should be an integer
        assert!(parse_min_expires(b"10.5").is_err());
        
        // Ensure a single digit is allowed
        let (_, single_digit) = parse_min_expires(b"7").unwrap();
        assert_eq!(single_digit, 7);
        
        // Ensure upper boundary of u32 works
        let (_, max_val) = parse_min_expires(b"4294967295").unwrap();
        assert_eq!(max_val, 4294967295);
    }

    #[test]
    fn test_invalid_min_expires() {
        // Empty input
        assert!(parse_min_expires(b"").is_err());
        
        // Non-numeric values
        assert!(parse_min_expires(b"abc").is_err());
        
        // Negative values
        assert!(parse_min_expires(b"-60").is_err());
        
        // Note: The delta_seconds parser doesn't actually reject internal whitespace,
        // it just stops parsing at the whitespace character.
        let (rem, val) = parse_min_expires(b"18 00").unwrap();
        assert_eq!(val, 18);
        assert_eq!(rem, b"00");
        
        // Decimal values are not allowed in delta-seconds
        assert!(parse_min_expires(b"1800.5").is_err());
    }
    
    #[test]
    fn test_reasonable_min_expires() {
        // Test the validator function
        
        // Typical values should be reasonable
        assert!(is_reasonable_min_expires(0));
        assert!(is_reasonable_min_expires(60));
        assert!(is_reasonable_min_expires(3600));
        assert!(is_reasonable_min_expires(86400)); // 1 day
        assert!(is_reasonable_min_expires(604800)); // 1 week
        assert!(is_reasonable_min_expires(2592000)); // 30 days
        
        // Very large values might be unreasonable
        assert!(!is_reasonable_min_expires(u32::MAX));
        assert!(!is_reasonable_min_expires(63072000)); // 2 years
    }
} 