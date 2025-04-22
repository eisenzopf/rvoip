use nom::{
    bytes::complete::{tag, take_while_m_n},
    combinator::{map_res, recognize},
    sequence::tuple,
    IResult,
};
use std::net::IpAddr;
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;

// IPv4address = 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT
// Recognizes the pattern and then uses std::net::IpAddr parsing for validation.
pub fn ipv4_address(input: &[u8]) -> ParseResult<Host> {
    map_res(
        recognize(
            tuple((
                take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()), tag(b"."),
                take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()), tag(b"."),
                take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()), tag(b"."),
                take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()),
            ))
        ),
        |bytes| {
            str::from_utf8(bytes)
                .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Char)))
                .and_then(|s| s.parse::<IpAddr>()
                    .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Verify)))) // Verify ensures it's a valid IPv4
                .map(Host::Address)
        }
    )(input)
} 