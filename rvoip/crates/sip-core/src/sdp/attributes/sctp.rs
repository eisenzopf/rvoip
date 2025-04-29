//! SDP SCTP Attribute Parser
//!
//! Implements parsers for SCTP-related attributes as defined in RFC 8841.
//! This includes sctp-port and max-message-size attributes used in 
//! WebRTC data channels.

use crate::error::{Error, Result};
use nom::{
    character::complete::{digit1, space0},
    combinator::{map_res, opt, recognize, all_consuming},
    sequence::{preceded, tuple},
    IResult,
};

/// Parse a decimal unsigned integer
fn parse_uint(input: &str) -> IResult<&str, u64> {
    map_res(
        recognize(preceded(opt(space0), digit1)),
        |s: &str| s.trim().parse::<u64>()
    )(input)
}

/// Parses the sctp-port attribute
/// 
/// Format: a=sctp-port:<port>
/// 
/// The sctp-port attribute is used to indicate the SCTP port number to be used
/// for data channels. A port value of 0 indicates that no SCTP association is
/// to be established.
pub fn parse_sctp_port(value: &str) -> Result<u16> {
    // Ensure all input is consumed
    match all_consuming(parse_uint)(value.trim()) {
        Ok((_, port)) => {
            if port <= u16::MAX as u64 {
                Ok(port as u16)
            } else {
                Err(Error::SdpParsingError(format!("SCTP port value out of range: {}", port)))
            }
        },
        Err(_) => Err(Error::SdpParsingError(format!("Invalid SCTP port value: {}", value)))
    }
}

/// Parses the max-message-size attribute
/// 
/// Format: a=max-message-size:<size>
/// 
/// The max-message-size attribute indicates the maximum size of the message
/// that can be sent on a data channel in bytes. A value of 0 indicates that
/// the implementation can handle messages of any size.
pub fn parse_max_message_size(value: &str) -> Result<u64> {
    // Ensure all input is consumed
    match all_consuming(parse_uint)(value.trim()) {
        Ok((_, size)) => Ok(size),
        Err(_) => Err(Error::SdpParsingError(format!("Invalid max-message-size value: {}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sctp_port_valid() {
        // Basic valid cases
        assert_eq!(parse_sctp_port("5000").unwrap(), 5000);
        assert_eq!(parse_sctp_port("0").unwrap(), 0);
        assert_eq!(parse_sctp_port("65535").unwrap(), 65535);
        
        // With whitespace
        assert_eq!(parse_sctp_port(" 1234 ").unwrap(), 1234);
        assert_eq!(parse_sctp_port("\t5678\t").unwrap(), 5678);
    }

    #[test]
    fn test_sctp_port_invalid() {
        // Non-numeric values
        assert!(parse_sctp_port("").is_err(), "Empty string should be rejected");
        assert!(parse_sctp_port("abc").is_err(), "Non-numeric value should be rejected");
        assert!(parse_sctp_port("123abc").is_err(), "Alpha-numeric value should be rejected");
        
        // Out of range values
        assert!(parse_sctp_port("65536").is_err(), "Value exceeding u16::MAX should be rejected");
        assert!(parse_sctp_port("100000").is_err(), "Value exceeding u16::MAX should be rejected");
        
        // Negative values (should be caught by parser as non-numeric)
        assert!(parse_sctp_port("-1").is_err(), "Negative value should be rejected");
        
        // Special characters
        assert!(parse_sctp_port("1,234").is_err(), "Value with comma should be rejected");
        assert!(parse_sctp_port("1.234").is_err(), "Value with decimal point should be rejected");
    }

    #[test]
    fn test_max_message_size_valid() {
        // Basic valid cases
        assert_eq!(parse_max_message_size("1024").unwrap(), 1024);
        assert_eq!(parse_max_message_size("0").unwrap(), 0);
        assert_eq!(parse_max_message_size("4294967295").unwrap(), 4294967295); // 2^32 - 1
        assert_eq!(parse_max_message_size("18446744073709551615").unwrap(), 18446744073709551615); // u64::MAX
        
        // With whitespace
        assert_eq!(parse_max_message_size(" 1024 ").unwrap(), 1024);
        assert_eq!(parse_max_message_size("\t16384\t").unwrap(), 16384);
    }

    #[test]
    fn test_max_message_size_invalid() {
        // Non-numeric values
        assert!(parse_max_message_size("").is_err(), "Empty string should be rejected");
        assert!(parse_max_message_size("abc").is_err(), "Non-numeric value should be rejected");
        assert!(parse_max_message_size("1024kb").is_err(), "Alpha-numeric value should be rejected");
        
        // Negative values (should be caught by parser as non-numeric)
        assert!(parse_max_message_size("-1024").is_err(), "Negative value should be rejected");
        
        // Special characters
        assert!(parse_max_message_size("1,048,576").is_err(), "Value with commas should be rejected");
        assert!(parse_max_message_size("1.5e6").is_err(), "Scientific notation should be rejected");
    }

    #[test]
    fn test_rfc8841_examples() {
        // Examples from RFC 8841 section 5.1.9
        assert_eq!(parse_sctp_port("5000").unwrap(), 5000);
        
        // Examples from RFC 8841 section 5.1.10
        assert_eq!(parse_max_message_size("65536").unwrap(), 65536);
        
        // Zero values are valid according to the RFC
        assert_eq!(parse_sctp_port("0").unwrap(), 0);
        assert_eq!(parse_max_message_size("0").unwrap(), 0);
    }

    #[test]
    fn test_edge_cases() {
        // Test boundary cases for sctp-port
        assert_eq!(parse_sctp_port("1").unwrap(), 1);
        assert_eq!(parse_sctp_port("65535").unwrap(), 65535); // u16::MAX
        
        // Leading zeros should be handled correctly
        assert_eq!(parse_sctp_port("0000123").unwrap(), 123);
        assert_eq!(parse_max_message_size("0000123").unwrap(), 123);
        
        // Multiple spaces
        assert_eq!(parse_sctp_port("   123   ").unwrap(), 123);
        assert_eq!(parse_max_message_size("   123   ").unwrap(), 123);
    }
} 