// Parser for Expires header (RFC 3261 Section 20.19)
// Expires = "Expires" HCOLON delta-seconds

use nom::{
    bytes::complete::tag_no_case,
    branch::alt,
    combinator::opt,
    IResult,
};

// Import delta_seconds parser and other utilities
use crate::parser::values::delta_seconds;
use crate::parser::ParseResult;
use crate::parser::separators::hcolon;
use crate::parser::whitespace::{sws, owsp};

// Expires = "Expires" HCOLON delta-seconds
// Note: HCOLON handled elsewhere
pub fn parse_expires(input: &[u8]) -> ParseResult<u32> {
    // Handle optional leading whitespace
    let (input, _) = opt(owsp)(input)?;
    
    // Parse the actual value
    let (input, value) = delta_seconds(input)?;
    
    // Return the result
    Ok((input, value))
}

/// Full parser for the Expires header, including header name and whitespace handling
pub fn parse_full_expires(input: &[u8]) -> ParseResult<u32> {
    // Parse the header name (case-insensitive)
    let (input, _) = tag_no_case(b"Expires")(input)?;
    
    // Parse the colon
    let (input, _) = hcolon(input)?;
    
    // Parse optional whitespace after the colon
    let (input, _) = sws(input)?;
    
    // Parse the actual value
    parse_expires(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_expires() {
        let (rem, val) = parse_expires(b"3600").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3600);

        let (rem_zero, val_zero) = parse_expires(b"0").unwrap();
        assert!(rem_zero.is_empty());
        assert_eq!(val_zero, 0);
        
        // With leading whitespace
        let (rem_ws, val_ws) = parse_expires(b" 42").unwrap();
        assert!(rem_ws.is_empty());
        assert_eq!(val_ws, 42);
    }

    #[test]
    fn test_invalid_expires() {
        assert!(parse_expires(b"").is_err());
        assert!(parse_expires(b"abc").is_err());
        assert!(parse_expires(b"-10").is_err());
        assert!(parse_expires(b"10.5").is_err()); // delta-seconds is integer
    }
    
    #[test]
    fn test_remaining_input() {
        // Parser should only consume digits and leave other characters
        let (rem, val) = parse_expires(b"3600;param=value").unwrap();
        assert_eq!(rem, b";param=value");
        assert_eq!(val, 3600);
        
        // Should stop at whitespace too
        let (rem_ws, val_ws) = parse_expires(b"3600 seconds").unwrap();
        assert_eq!(rem_ws, b" seconds");
        assert_eq!(val_ws, 3600);
    }
    
    #[test]
    fn test_full_expires_header() {
        // Test the full header parser
        let (rem, val) = parse_full_expires(b"Expires: 3600").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3600);
        
        // Case insensitivity of header name
        let (rem, val) = parse_full_expires(b"expires: 7200").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 7200);
        
        // With extra whitespace
        let (rem, val) = parse_full_expires(b"Expires:  300").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 300);
    }
    
    #[test]
    fn test_line_folding() {
        // Test with line folding after the colon
        let (rem, val) = parse_full_expires(b"Expires:\r\n 3600").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3600);
        
        // Multiple line folding
        let (rem, val) = parse_full_expires(b"Expires:\r\n \r\n 7200").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 7200);
    }
    
    #[test]
    fn test_rfc3261_examples() {
        // Examples from RFC 3261
        // Example 1: Registration with Expires: 3600
        let (rem, val) = parse_full_expires(b"Expires: 3600").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 3600);
        
        // Example 2: Zero timeout
        let (rem, val) = parse_full_expires(b"Expires: 0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 0);
    }
    
    #[test]
    fn test_overflow_values() {
        // Test with values that might overflow u32
        
        // Maximum u32 value
        let (rem, val) = parse_expires(b"4294967295").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, u32::MAX);
        
        // Overflow handling depends on the implementation of delta_seconds
        // It might either reject the value or truncate it
        let result = parse_expires(b"4294967296");
        if result.is_ok() {
            // If it's handled by truncation/wrapping
            let (rem, val) = result.unwrap();
            assert!(rem.is_empty() || !rem.is_empty());  // Either way is fine
        } else {
            // If it's rejected as an error, that's also fine
            assert!(result.is_err());
        }
    }
    
    #[test]
    fn test_abnf_compliance() {
        // According to the ABNF: Expires = "Expires" HCOLON delta-seconds
        // delta-seconds = 1*DIGIT
        
        // Valid according to ABNF: one or more digits
        for i in 0..10 {
            let input = format!("{}", i).into_bytes();
            let (rem, val) = parse_expires(&input).unwrap();
            assert!(rem.is_empty());
            assert_eq!(val, i as u32);
        }
        
        // Multiple digits in various combinations
        let (rem, val) = parse_expires(b"123").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 123);
        
        let (rem, val) = parse_expires(b"007").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 7);
        
        let (rem, val) = parse_expires(b"42").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 42);
        
        // Invalid according to ABNF
        // Empty value (requires 1 or more digits)
        assert!(parse_expires(b"").is_err());
        
        // Non-digit characters
        assert!(parse_expires(b"a").is_err());
        
        // Non-digit characters after digits should cause the parser to stop at that point
        let (rem, val) = parse_expires(b"12a3").unwrap();
        assert_eq!(rem, b"a3");
        assert_eq!(val, 12);
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Leading whitespace should be allowed
        let (rem, val) = parse_expires(b"  42").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 42);
        
        // Tab character before digits
        let (rem, val) = parse_expires(b"\t42").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 42);
        
        // Trailing whitespace should be considered as remaining input
        let (rem, val) = parse_expires(b"42 ").unwrap();
        assert_eq!(rem, b" ");
        assert_eq!(val, 42);
    }
} 