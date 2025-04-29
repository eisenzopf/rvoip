//! SDP Data Channel Attribute Parsers
//!
//! Implements parsers for WebRTC data channel attributes as defined in RFC 8841.
//! Includes parsers for sctp-port and max-message-size attributes.

use crate::error::{Error, Result};
use nom::{
    character::complete::digit1,
    combinator::map_res,
    IResult,
};

/// Parser for sctp-port attribute
/// Format: a=sctp-port:<port>
fn sctp_port_parser(input: &str) -> IResult<&str, u16> {
    map_res(
        digit1,
        |s: &str| s.parse::<u16>()
    )(input)
}

/// Parser for max-message-size attribute
/// Format: a=max-message-size:<size>
fn max_message_size_parser(input: &str) -> IResult<&str, u64> {
    map_res(
        digit1,
        |s: &str| s.parse::<u64>()
    )(input)
}

/// Parses sctp-port attribute, which specifies the SCTP port for data channels
pub fn parse_sctp_port(value: &str) -> Result<u16> {
    let value = value.trim();
    match sctp_port_parser(value) {
        Ok((remaining, port)) => {
            // Ensure there's no trailing content
            if !remaining.is_empty() {
                return Err(Error::SdpParsingError(format!("Invalid sctp-port format, trailing characters: {}", value)));
            }
            Ok(port)
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid sctp-port format: {}", value)))
    }
}

/// Parses max-message-size attribute, which specifies the maximum message size in bytes
pub fn parse_max_message_size(value: &str) -> Result<u64> {
    let value = value.trim();
    match max_message_size_parser(value) {
        Ok((remaining, size)) => {
            // Ensure there's no trailing content
            if !remaining.is_empty() {
                return Err(Error::SdpParsingError(format!("Invalid max-message-size format, trailing characters: {}", value)));
            }
            
            if size < 1 {
                return Err(Error::SdpParsingError(format!(
                    "Invalid max-message-size, must be greater than 0: {}", size
                )));
            }
            Ok(size)
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid max-message-size format: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SCTP Port Tests
    
    #[test]
    fn test_valid_sctp_port() {
        // RFC 8841 allows valid SCTP port values (1-65535)
        assert_eq!(parse_sctp_port("5000").unwrap(), 5000);
        assert_eq!(parse_sctp_port("1").unwrap(), 1);
        assert_eq!(parse_sctp_port("65535").unwrap(), 65535);
    }
    
    #[test]
    fn test_sctp_port_whitespace_handling() {
        // Test with whitespace which should be trimmed
        assert_eq!(parse_sctp_port(" 5000").unwrap(), 5000);
        assert_eq!(parse_sctp_port("5000 ").unwrap(), 5000);
        assert_eq!(parse_sctp_port(" 5000 ").unwrap(), 5000);
        assert_eq!(parse_sctp_port("\t5000\n").unwrap(), 5000);
    }
    
    #[test]
    fn test_invalid_sctp_port_formats() {
        // Non-numeric values should be rejected
        assert!(parse_sctp_port("port").is_err());
        assert!(parse_sctp_port("12a34").is_err());
        assert!(parse_sctp_port("").is_err());
        assert!(parse_sctp_port(" ").is_err());
        
        // Negative values should be rejected (although the parser can't actually 
        // reach this case since the digit1 parser only accepts digits)
        assert!(parse_sctp_port("-5000").is_err());
        
        // Values with decimal points should be rejected
        assert!(parse_sctp_port("5000.5").is_err());
    }
    
    #[test]
    fn test_out_of_range_sctp_port() {
        // The u16 parser will automatically limit values to 0-65535
        // Values outside this range should cause parse errors
        assert!(parse_sctp_port("65536").is_err());
        assert!(parse_sctp_port("70000").is_err());
        assert!(parse_sctp_port("4294967295").is_err()); // Max u32
    }
    
    // Max Message Size Tests
    
    #[test]
    fn test_valid_max_message_size() {
        // RFC 8841 requires max-message-size to be a positive integer
        assert_eq!(parse_max_message_size("1").unwrap(), 1);
        assert_eq!(parse_max_message_size("1024").unwrap(), 1024);
        assert_eq!(parse_max_message_size("16384").unwrap(), 16384);
        assert_eq!(parse_max_message_size("1073741824").unwrap(), 1073741824); // 1GB
        assert_eq!(parse_max_message_size("18446744073709551615").unwrap(), 18446744073709551615); // Max u64
    }
    
    #[test]
    fn test_max_message_size_whitespace_handling() {
        // Test with whitespace which should be trimmed
        assert_eq!(parse_max_message_size(" 1024").unwrap(), 1024);
        assert_eq!(parse_max_message_size("1024 ").unwrap(), 1024);
        assert_eq!(parse_max_message_size(" 1024 ").unwrap(), 1024);
        assert_eq!(parse_max_message_size("\t1024\n").unwrap(), 1024);
    }
    
    #[test]
    fn test_invalid_max_message_size_formats() {
        // Non-numeric values should be rejected
        assert!(parse_max_message_size("size").is_err());
        assert!(parse_max_message_size("1024b").is_err());
        assert!(parse_max_message_size("").is_err());
        assert!(parse_max_message_size(" ").is_err());
        
        // Negative values should be rejected
        assert!(parse_max_message_size("-1024").is_err());
        
        // Values with decimal points should be rejected
        assert!(parse_max_message_size("1024.5").is_err());
    }
    
    #[test]
    fn test_zero_max_message_size() {
        // RFC 8841 requires max-message-size to be greater than 0
        assert!(parse_max_message_size("0").is_err());
    }
} 