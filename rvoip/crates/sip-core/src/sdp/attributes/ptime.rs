//! SDP Packet Time Attribute Parsers
//!
//! Implements parsers for ptime and maxptime attributes as defined in RFC 8866.
//! Format: a=ptime:<packet time>
//! Format: a=maxptime:<maximum packet time>

use crate::error::{Error, Result};
use crate::sdp::attributes::common::{positive_integer, to_result};
use crate::types::sdp::ParsedAttribute;
use nom::{
    combinator::verify,
    IResult,
};

/// Parser for ptime value (positive integer)
fn ptime_parser(input: &str) -> IResult<&str, u32> {
    positive_integer(input)
}

/// Parser for maxptime value (positive integer with reasonable constraints)
fn maxptime_parser(input: &str) -> IResult<&str, u32> {
    verify(
        positive_integer,
        |&v| v >= 10 && v <= 5000 // Reasonable range for packet time
    )(input)
}

/// Parses ptime attribute: a=ptime:<packet time>
pub fn parse_ptime(value: &str) -> Result<u32> {
    to_result(
        ptime_parser(value.trim()),
        &format!("Invalid ptime value: {}", value)
    )
}

/// Parses maxptime attribute: a=maxptime:<maximum packet time>
pub fn parse_maxptime(value: &str) -> Result<u32> {
    to_result(
        maxptime_parser(value.trim()),
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
    }
} 