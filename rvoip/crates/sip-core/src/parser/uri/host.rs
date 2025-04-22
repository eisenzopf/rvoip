use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::digit1,
    combinator::{map_res, opt},
    sequence::{pair, preceded},
    IResult,
};
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;
use crate::parser::common_chars::digit; // Keep digit if still used by port

// Import the specific host type parsers
use super::hostname::hostname;
use super::ipv4::ipv4_address;
use super::ipv6::ipv6_reference;

// host = hostname / IPv4address / IPv6reference
pub(crate) fn host(input: &[u8]) -> ParseResult<Host> {
    // Order is important: try hostname first as IP addresses might contain valid domain chars
    alt((hostname, ipv4_address, ipv6_reference))(input)
}

// port = 1*DIGIT
pub(crate) fn port(input: &[u8]) -> ParseResult<u16> {
    map_res(digit1, |bytes| { // Use digit1 directly
        str::from_utf8(bytes)
            .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Char)))
            .and_then(|s| s.parse::<u16>().map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Verify))))
    })(input)
}

// hostport = host [ ":" port ]
pub(crate) fn hostport(input: &[u8]) -> ParseResult<(Host, Option<u16>)> {
    pair(host, opt(preceded(tag(b":"), port)))(input)
} 