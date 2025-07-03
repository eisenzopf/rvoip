//! SDP Packet Time Attribute Parsers
//!
//! Implements parsers for ptime and maxptime attributes as defined in RFC 8866.
//! Format: a=ptime:<packet time>
//! Format: a=maxptime:<maximum packet time>

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, to_result};
use crate::types::sdp::ParsedAttribute;
use nom::{
    combinator::{verify, all_consuming},
    sequence::terminated,
    IResult,
};

/// Parser for ptime value (positive integer)
fn ptime_parser(input: &str) -> IResult<&str, u32> {
    // Ensure the input doesn't contain decimals before parsing
    if input.contains('.') {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit)));
    }
    
    // Validate that the entire input is a valid integer
    all_consuming(positive_integer)(input)
}

/// Parser for maxptime value (positive integer with reasonable constraints)
fn maxptime_parser(input: &str) -> IResult<&str, u32> {
    // Ensure the input doesn't contain decimals before parsing
    if input.contains('.') {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit)));
    }
    
    // Validate that the entire input is a valid integer within constraints
    all_consuming(verify(
        positive_integer,
        |&v| (10..=5000).contains(&v) // Reasonable range for packet time
    ))(input)
}

/// Parses ptime attribute: a=ptime:<packet time>
pub fn parse_ptime(value: &str) -> Result<u32> {
    let trimmed = value.trim();
    
    // Explicitly check for decimal points
    if trimmed.contains('.') {
        return Err(Error::SdpParsingError(format!("ptime must be an integer value without decimals: {}", value)));
    }
    
    // Check if the input contains only digits after trimming
    if !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::SdpParsingError(format!("ptime must contain only digits: {}", value)));
    }
    
    to_result(
        ptime_parser(trimmed),
        &format!("Invalid ptime value: {}", value)
    )
}

/// Parses maxptime attribute: a=maxptime:<maximum packet time>
pub fn parse_maxptime(value: &str) -> Result<u32> {
    let trimmed = value.trim();
    
    // Explicitly check for decimal points
    if trimmed.contains('.') {
        return Err(Error::SdpParsingError(format!("maxptime must be an integer value without decimals: {}", value)));
    }
    
    // Check if the input contains only digits after trimming
    if !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::SdpParsingError(format!("maxptime must contain only digits: {}", value)));
    }
    
    to_result(
        maxptime_parser(trimmed),
        &format!("Invalid maxptime value: {}", value)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ptime_attribute_comprehensive() {
        // Valid cases
        assert_eq!(parse_ptime("20").unwrap(), 20);
        assert_eq!(parse_ptime("0").unwrap(), 0);
        assert_eq!(parse_ptime("1000").unwrap(), 1000);
        
        // Edge cases
        
        // Whitespace handling
        assert_eq!(parse_ptime(" 20 ").unwrap(), 20);
        
        // Error cases
        
        // Invalid format - non-numeric
        assert!(parse_ptime("twenty").is_err());
        
        // Invalid format - negative
        assert!(parse_ptime("-20").is_err());
        
        // Invalid format - decimal
        assert!(parse_ptime("20.5").is_err());
        
        // Invalid format - empty
        assert!(parse_ptime("").is_err());
    }
    
    #[test]
    fn test_maxptime_attribute_comprehensive() {
        // Valid cases
        assert_eq!(parse_maxptime("20").unwrap(), 20);
        assert_eq!(parse_maxptime("1000").unwrap(), 1000);
        
        // Edge cases
        
        // Whitespace handling
        assert_eq!(parse_maxptime(" 50 ").unwrap(), 50);
        
        // Minimum reasonable value
        assert_eq!(parse_maxptime("10").unwrap(), 10);
        
        // Maximum reasonable value
        assert_eq!(parse_maxptime("5000").unwrap(), 5000);
        
        // Error cases
        
        // Invalid format - non-numeric
        assert!(parse_maxptime("maximum").is_err());
        
        // Invalid format - negative
        assert!(parse_maxptime("-50").is_err());
        
        // Invalid format - decimal
        assert!(parse_maxptime("50.5").is_err());
        
        // Invalid format - empty
        assert!(parse_maxptime("").is_err());
        
        // Invalid format - too small
        assert!(parse_maxptime("9").is_err());
        
        // Invalid format - too large
        assert!(parse_maxptime("5001").is_err());
    }
    
    #[test]
    fn test_parser_functions() {
        // Test the ptime_parser function directly
        let result = ptime_parser("20");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, 20);
        
        // Test the maxptime_parser function directly
        let result = maxptime_parser("30");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1, 30);
        
        // Test maxptime with out-of-range values
        let result = maxptime_parser("5");
        assert!(result.is_err());
        
        let result = maxptime_parser("6000");
        assert!(result.is_err());
        
        // Test with decimal values
        let result = ptime_parser("20.5");
        assert!(result.is_err());
        
        let result = maxptime_parser("30.0");
        assert!(result.is_err());
    }
    
    #[test]
    fn test_real_world_ptime_examples() {
        // Common ptime values from real SDP messages
        
        // G.711 typical ptime
        assert_eq!(parse_ptime("20").unwrap(), 20);
        
        // G.729 typical ptime
        assert_eq!(parse_ptime("10").unwrap(), 10);
        
        // AMR typical ptime
        assert_eq!(parse_ptime("20").unwrap(), 20);
        
        // OPUS typical ptime
        assert_eq!(parse_ptime("20").unwrap(), 20);
        
        // Higher latency configuration
        assert_eq!(parse_ptime("40").unwrap(), 40);
        assert_eq!(parse_ptime("60").unwrap(), 60);
    }
    
    #[test]
    fn test_ptime_with_invalid_characters() {
        // Test with special characters
        assert!(parse_ptime("20ms").is_err());
        assert!(parse_ptime("20,000").is_err());
        assert!(parse_ptime("20+10").is_err());
        
        // Test with trailing or leading characters
        assert!(parse_ptime("a20").is_err());
        assert!(parse_ptime("20a").is_err());
    }
    
    #[test]
    fn test_maxptime_with_invalid_characters() {
        // Test with special characters
        assert!(parse_maxptime("50ms").is_err());
        assert!(parse_maxptime("1,000").is_err());
        assert!(parse_maxptime("50+10").is_err());
        
        // Test with trailing or leading characters
        assert!(parse_maxptime("a50").is_err());
        assert!(parse_maxptime("50a").is_err());
    }
    
    #[test]
    fn test_integer_overflow_handling() {
        // Test with very large values
        assert!(parse_ptime("4294967296").is_err()); // 2^32, which is too large for u32
        assert!(parse_ptime("9999999999").is_err());
        
        // Similarly for maxptime, but also considering the upper bound of 5000
        assert!(parse_maxptime("9999999999").is_err());
    }
} 