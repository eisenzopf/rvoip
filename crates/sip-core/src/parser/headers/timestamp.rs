// Parser for the Timestamp header (RFC 3261 Section 20.40)
// Timestamp = "Timestamp" HCOLON 1*(DIGIT) [ "." *(DIGIT) ] [ LWS delay ]
// delay = *(DIGIT) [ "." *(DIGIT) ]

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while, take_while1},
    character::complete::{char, digit1, space0},
    combinator::{map, map_res, opt, recognize},
    sequence::{pair, preceded, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;
use ordered_float::NotNan; // For parsing float strings

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;


// Parses a float-like sequence of digits: 1*(DIGIT) [ "." *(DIGIT) ]
// Returns the raw byte slice representing the float string.
fn float_digits(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        digit1, // Must have at least one digit before decimal
        opt(pair(tag(b"."), take_while(|c: u8| c.is_ascii_digit())))
    ))(input)
}

// Parses a delay value according to ABNF: *(DIGIT) [ "." *(DIGIT) ]
// This allows formats like ".456" (no leading digits) per RFC 3261
fn delay_digits(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(alt((
        // Case 1: With decimal point - can have optional digits before
        recognize(tuple((
            take_while(|c: u8| c.is_ascii_digit()), // 0 or more digits
            tag(b"."),
            take_while(|c: u8| c.is_ascii_digit()) // 0 or more digits
        ))),
        // Case 2: Just digits, no decimal
        digit1
    )))(input)
}

// Define structure for Timestamp value
#[derive(Debug, PartialEq, Clone)]
pub struct TimestampValue {
    pub timestamp: f32,
    pub delay: Option<f32>,
}

// Parses 1*(DIGIT) [ "." *(DIGIT) ] into NotNan<f32>
pub fn parse_timestamp_value(input: &[u8]) -> ParseResult<NotNan<f32>> {
    // RFC 3261 doesn't allow negative values (no minus sign in grammar)
    // Check if input starts with a minus sign and reject
    if !input.is_empty() && input[0] == b'-' {
        return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::Digit)));
    }

    map_res(
        // Recognize a sequence like "123" or "123.456"
        float_digits,
        |bytes: &[u8]| -> Result<NotNan<f32>, NomError<&[u8]>> { // Return Result<_, NomError>
            let s = str::from_utf8(bytes)
                .map_err(|_| NomError::from_error_kind(bytes, ErrorKind::Char))?; // Map to NomError
            let f = s.parse::<f32>()
                .map_err(|_| NomError::from_error_kind(bytes, ErrorKind::Float))?; // Map to NomError
                
            NotNan::new(f)
                .map_err(|_| NomError::from_error_kind(bytes, ErrorKind::Verify)) // Map NotNan error to NomError
        }
    )(input)
}

// Parses delay value: *(DIGIT) [ "." *(DIGIT) ] into NotNan<f32>
pub fn parse_delay_value(input: &[u8]) -> ParseResult<NotNan<f32>> {
    // RFC 3261 doesn't allow negative values (no minus sign in grammar)
    // Check if input starts with a minus sign and reject
    if !input.is_empty() && input[0] == b'-' {
        return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::Digit)));
    }

    map_res(
        // Recognize a sequence following delay format
        delay_digits,
        |bytes: &[u8]| -> Result<NotNan<f32>, NomError<&[u8]>> {
            let s = str::from_utf8(bytes)
                .map_err(|_| NomError::from_error_kind(bytes, ErrorKind::Char))?;
            
            // For formats like ".456", we prepend a "0" for parsing
            let parse_str = if s.starts_with('.') {
                format!("0{}", s)
            } else {
                s.to_string()
            };
            
            let f = parse_str.parse::<f32>()
                .map_err(|_| NomError::from_error_kind(bytes, ErrorKind::Float))?;
                
            NotNan::new(f)
                .map_err(|_| NomError::from_error_kind(bytes, ErrorKind::Verify))
        }
    )(input)
}

// Timestamp = "Timestamp" HCOLON 1*(DIGIT) [ "." *(DIGIT) ] [ LWS delay ]
// delay = *(DIGIT) [ "." *(DIGIT) ]
// Note: HCOLON handled elsewhere
// Returns (timestamp: f32, delay: Option<f32>)
pub fn parse_timestamp(input: &[u8]) -> ParseResult<(NotNan<f32>, Option<NotNan<f32>>)> {
    // First parse the timestamp value, which must have at least one digit
    let (remaining, timestamp) = parse_timestamp_value(input)?;
    
    // Check for whitespace followed by negative sign, which would be invalid
    // This helps catch cases like "123 -456" which our delay parser might not catch
    if remaining.len() >= 2 {
        let mut i = 0;
        // Skip any whitespace
        while i < remaining.len() && (remaining[i] == b' ' || remaining[i] == b'\t') {
            i += 1;
        }
        // If we find a minus sign after whitespace, reject it
        if i < remaining.len() && remaining[i] == b'-' {
            return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::Digit)));
        }
    }
    
    // Then optionally parse the delay, which can have a leading decimal point
    let (remaining, delay) = opt(preceded(
        space0, // LWS (Linear White Space)
        parse_delay_value // delay value has different rules from timestamp
    ))(remaining)?;
    
    Ok((remaining, (timestamp, delay)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_float_val() {
        let (rem, f) = float_digits(b"123.456").unwrap();
        assert!(rem.is_empty());
        assert_eq!(f, b"123.456");

        let (rem_int, f_int) = float_digits(b"789").unwrap();
        assert!(rem_int.is_empty());
        assert_eq!(f_int, b"789");
    }
    
    #[test]
    fn test_parse_timestamp_no_delay() {
        let input = b"83.38";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 83.38f32);
        assert!(delay.is_none());
    }
    
    #[test]
    fn test_parse_timestamp_with_delay() {
        let input = b"100 0.5";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 100.0f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 0.5f32);
    }

    #[test]
    fn test_parse_timestamp_fractional_delay() {
        let input = b"99.9 0.01";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 99.9f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 0.01f32);
    }
    
    #[test]
    fn test_rfc3261_examples() {
        // Example from RFC 3261 Section 20.40
        let input = b"54.21 0.3421";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 54.21f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 0.3421f32);
    }
    
    #[test]
    fn test_abnf_edge_cases() {
        // Test timestamp without fractional part
        let input = b"123";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 123.0f32);
        assert!(delay.is_none());
        
        // Test timestamp with empty fractional part
        let input = b"123.";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, _)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 123.0f32);
        
        // Test timestamp with zero fractional part
        let input = b"123.0";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, _)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 123.0f32);
        
        // Test delay without fractional part
        let input = b"123 456";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 123.0f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 456.0f32);
        
        // Test delay with empty fractional part
        let input = b"123 456.";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 123.0f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 456.0f32);
    }
    
    #[test]
    fn test_whitespace_handling() {
        // Multiple spaces between timestamp and delay
        let input = b"123    456";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 123.0f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 456.0f32);
        
        // Tab between timestamp and delay
        let input = b"123\t456";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ts.into_inner(), 123.0f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 456.0f32);
    }
    
    #[test]
    fn test_remaining_input() {
        // Test with additional content after valid timestamp
        let input = b"123;param=value";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, _)) = result.unwrap();
        assert_eq!(rem, b";param=value");
        assert_eq!(ts.into_inner(), 123.0f32);
        
        // Test with additional content after valid timestamp with delay
        let input = b"123 456;param=value";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (rem, (ts, delay)) = result.unwrap();
        assert_eq!(rem, b";param=value");
        assert_eq!(ts.into_inner(), 123.0f32);
        assert!(delay.is_some());
        assert_eq!(delay.unwrap().into_inner(), 456.0f32);
    }
    
    #[test]
    fn test_invalid_inputs() {
        // Empty input
        assert!(parse_timestamp(b"").is_err());
        
        // Non-numeric timestamp
        assert!(parse_timestamp(b"abc").is_err());
        
        // Missing timestamp (only delay)
        assert!(parse_timestamp(b" 123").is_err());
        
        // Invalid format for timestamp
        assert!(parse_timestamp(b".123").is_err());
        
        // Delay with leading decimal point is valid per RFC grammar:
        // delay = *(DIGIT) [ "." *(DIGIT) ] 
        // Which means digits before decimal are optional for delay
        let input = b"123 .456";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        
        // Negative values not allowed according to RFC grammar
        assert!(parse_timestamp(b"-123").is_err());
        assert!(parse_timestamp(b"123 -456").is_err());
    }
    
    #[test]
    fn test_large_values() {
        // Test large timestamp value
        let input = b"12345678901234567890.12345678901234567890";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        
        // Test large delay value
        let input = b"123 12345678901234567890.12345678901234567890";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
    }
} 