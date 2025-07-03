//! SDP line parsing utilities
//!
//! This module provides the low-level parsing functionality for SDP lines according to RFC 8866.
//! Each SDP line has the format `<type>=<value>` where type is a single character.
//!
//! The SDP format consists of a series of lines, each with a specific type denoted by a single character:
//! - v= (Protocol Version)
//! - o= (Origin)
//! - s= (Session Name)
//! - i= (Session Information) - optional
//! - u= (URI) - optional
//! - e= (Email Address) - optional
//! - p= (Phone Number) - optional
//! - c= (Connection Information) - optional if included at session level or media level
//! - b= (Bandwidth Information) - optional
//! - t= (Timing)
//! - r= (Repeat Times) - optional
//! - z= (Time Zones) - optional
//! - k= (Encryption Keys) - optional and deprecated
//! - a= (Session Attributes) - optional
//! - m= (Media Descriptions)
//!
//! This module uses the `nom` parser combinator library to efficiently parse SDP lines.

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
/// This function handles different kinds of line endings (CRLF, LF) and
/// trims any whitespace from the values.
///
/// # Parameters
///
/// - `input`: The SDP line to parse
///
/// # Returns
///
/// - A nom `IResult` with the remaining input and a tuple of (type char, value string)
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp::parser::parse_sdp_line;
/// 
/// // Parse a version line
/// let (remaining, (key, value)) = parse_sdp_line("v=0").unwrap();
/// assert_eq!(key, 'v');
/// assert_eq!(value, "0");
/// 
/// // Parse a session name line
/// let (remaining, (key, value)) = parse_sdp_line("s=SDP Seminar").unwrap();
/// assert_eq!(key, 's');
/// assert_eq!(value, "SDP Seminar");
/// 
/// // Parse an attribute line
/// let (remaining, (key, value)) = parse_sdp_line("a=rtpmap:96 VP8/90000").unwrap();
/// assert_eq!(key, 'a');
/// assert_eq!(value, "rtpmap:96 VP8/90000");
/// ```
pub fn parse_sdp_line(input: &str) -> IResult<&str, (char, &str)> {
    let (input, key) = nom::character::complete::anychar(input)?;
    let (input, _) = char('=')(input)?;
    let (input, value) = not_line_ending(input)?;
    
    // Handle different line endings (CRLF, LF, etc.)
    let input = input.trim_start_matches(['\r', '\n']);
    
    Ok((input, (key, value.trim())))
}

/// Parse a bandwidth line (b=)
///
/// Bandwidth lines specify the proposed bandwidth to be used by the session or media.
/// The bwtype is usually either "AS" (Application Specific) or "CT" (Conference Total).
///
/// # Format
///
/// b=<bwtype>:<bandwidth>
///
/// Where:
/// - bwtype: A token specifying the bandwidth type
/// - bandwidth: A numeric value in kilobits per second
///
/// # Parameters
///
/// - `input`: The value part of the bandwidth line
///
/// # Returns
///
/// - A nom `IResult` with the remaining input and a tuple of (bandwidth_type, bandwidth_value)
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp::parser::parse_bandwidth_line;
/// 
/// // Parse an Application Specific bandwidth line
/// let (remaining, (bw_type, bw_value)) = parse_bandwidth_line("AS:128").unwrap();
/// assert_eq!(bw_type, "AS");
/// assert_eq!(bw_value, 128);
/// 
/// // Parse a Conference Total bandwidth line
/// let (remaining, (bw_type, bw_value)) = parse_bandwidth_line("CT:1000").unwrap();
/// assert_eq!(bw_type, "CT");
/// assert_eq!(bw_value, 1000);
/// ```
///
/// # RFC References
///
/// - [RFC 8866 Section 5.8](https://datatracker.ietf.org/doc/html/rfc8866#section-5.8)
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