use nom::{
    bytes::complete::{tag, take_while1},
    combinator::map_res,
    sequence::delimited,
    IResult,
};
use std::net::IpAddr;
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;

// IPv6reference = "[" IPv6address "]"
// Simplified IPv6address parser: Recognizes bracketed content and uses std::net::IpAddr parsing for validation.
// Allows hex, colons, dots (for IPv4-mapped), and percent (for scope IDs).
pub fn ipv6_reference(input: &[u8]) -> ParseResult<Host> {
    map_res(
        delimited(
            tag(b"["),
            take_while1(|c: u8| c.is_ascii_hexdigit() || c == b':' || c == b'.' || c == b'%'),
            tag(b"]"),
        ),
        |bytes| {
            str::from_utf8(bytes)
                .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Char)))
                .and_then(|s| s.parse::<IpAddr>()
                    .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Verify)))) // Verify ensures valid IPv6 syntax
                .map(Host::Address)
        }
    )(input)
} 