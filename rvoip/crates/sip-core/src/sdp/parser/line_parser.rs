//! SDP line parsing utilities
//!
//! This module provides the low-level parsing functionality for SDP lines.
//! Each SDP line has the format `<type>=<value>` where type is a single character.

use nom::{
    IResult,
    bytes::complete::{tag, take_until},
    character::complete::{char, not_line_ending, space0, space1},
    combinator::{map, opt},
    sequence::{preceded, terminated, tuple},
};

/// Parse an SDP line into a key-value pair
///
/// SDP lines have the format `x=value` where:
/// - x is a single character line type
/// - value is the content for that line type
///
/// # Parameters
///
/// - `input`: The SDP line to parse
///
/// # Returns
///
/// - A nom `IResult` with the remaining input and a tuple of (type char, value string)
pub fn parse_sdp_line(input: &str) -> IResult<&str, (char, &str)> {
    let (input, key) = nom::character::complete::anychar(input)?;
    let (input, _) = char('=')(input)?;
    let (input, value) = not_line_ending(input)?;
    
    // Handle different line endings (CRLF, LF, etc.)
    let input = input.trim_start_matches(|c| c == '\r' || c == '\n');
    
    Ok((input, (key, value.trim())))
}

/// Parse a bandwidth line (b=)
///
/// # Format
///
/// b=<bwtype>:<bandwidth>
///
/// # Parameters
///
/// - `input`: The value part of the bandwidth line
///
/// # Returns
///
/// - A nom `IResult` with the remaining input and a tuple of (bandwidth_type, bandwidth_value)
pub fn parse_bandwidth_line(input: &str) -> IResult<&str, (&str, u64)> {
    let (input, bw_type) = take_until(":")(input)?;
    let (input, _) = tag(":")(input)?;
    let (input, bw_value) = nom::character::complete::digit1(input)?;
    
    let bw_value = match bw_value.parse::<u64>() {
        Ok(val) => val,
        Err(_) => return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit))),
    };
    
    Ok((input, (bw_type, bw_value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sdp_line() {
        // Test a simple SDP line
        let (_, (key, value)) = parse_sdp_line("v=0").unwrap();
        assert_eq!(key, 'v');
        assert_eq!(value, "0");
        
        // Test an SDP line with spaces
        let (_, (key, value)) = parse_sdp_line("s=My Session Name").unwrap();
        assert_eq!(key, 's');
        assert_eq!(value, "My Session Name");
        
        // Test an SDP line with multiple equals signs
        let (_, (key, value)) = parse_sdp_line("a=rtpmap:0 PCMU/8000").unwrap();
        assert_eq!(key, 'a');
        assert_eq!(value, "rtpmap:0 PCMU/8000");
    }
    
    #[test]
    fn test_parse_bandwidth_line() {
        // Test a simple bandwidth line
        let (_, (bw_type, bw_value)) = parse_bandwidth_line("AS:128").unwrap();
        assert_eq!(bw_type, "AS");
        assert_eq!(bw_value, 128);
        
        // Test a bandwidth line with a different type
        let (_, (bw_type, bw_value)) = parse_bandwidth_line("CT:1000").unwrap();
        assert_eq!(bw_type, "CT");
        assert_eq!(bw_value, 1000);
    }
} 