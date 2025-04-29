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
    match sctp_port_parser(value.trim()) {
        Ok((_, port)) => Ok(port),
        Err(_) => Err(Error::SdpParsingError(format!("Invalid sctp-port format: {}", value)))
    }
}

/// Parses max-message-size attribute, which specifies the maximum message size in bytes
pub fn parse_max_message_size(value: &str) -> Result<u64> {
    match max_message_size_parser(value.trim()) {
        Ok((_, size)) => {
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