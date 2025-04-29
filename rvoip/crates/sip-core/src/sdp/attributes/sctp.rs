//! SDP SCTP Attribute Parser
//!
//! Implements parsers for SCTP-related attributes as defined in RFC 8841.
//! This includes sctp-port and max-message-size attributes used in 
//! WebRTC data channels.

use crate::error::{Error, Result};
use nom::{
    character::complete::{digit1, space0},
    combinator::{map_res, opt, recognize},
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
    match parse_uint(value.trim()) {
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
    match parse_uint(value.trim()) {
        Ok((_, size)) => Ok(size),
        Err(_) => Err(Error::SdpParsingError(format!("Invalid max-message-size value: {}", value)))
    }
} 