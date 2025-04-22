// Parser for the Timestamp header (RFC 3261 Section 20.40)
// Timestamp = "Timestamp" HCOLON 1*(DIGIT) [ "." *(DIGIT) ] [ LWS delay ]
// delay = *(DIGIT) [ "." *(DIGIT) ]

use nom::{
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

// Define structure for Timestamp value
#[derive(Debug, PartialEq, Clone)]
pub struct TimestampValue {
    pub timestamp: f32,
    pub delay: Option<f32>,
}

// Parses 1*(DIGIT) [ "." *(DIGIT) ] into NotNan<f32>
pub(crate) fn parse_timestamp_value(input: &[u8]) -> ParseResult<NotNan<f32>> {
    map_res(
        // Recognize a sequence like "123" or "123.456"
        recognize(
            pair(
                digit1, 
                opt(pair(tag(b".".as_slice()), take_while(|c: u8| c.is_ascii_digit()))) // Use .as_slice()
            )
        ),
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

// Timestamp = "Timestamp" HCOLON 1*(DIGIT) [ "." *(DIGIT) ] [ LWS delay ]
// delay = *(DIGIT) [ "." *(DIGIT) ]
// Note: HCOLON handled elsewhere
// Returns (timestamp: f32, delay: Option<f32>)
pub fn parse_timestamp(input: &[u8]) -> ParseResult<(NotNan<f32>, Option<NotNan<f32>>)> {
    tuple((
        parse_timestamp_value, // timestamp-value
        opt(preceded(space0, parse_timestamp_value)) // [ LWS delay-value ]
    ))(input)
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
} 