use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while_m_n},
    character::complete::digit1,
    combinator::{map, map_res, opt},
    sequence::{pair, preceded},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Import from base modules
use crate::parser::common_params::generic_param;
use crate::parser::separators::equal;
use crate::parser::token::token;
use crate::parser::uri::host::host; // For maddr and received
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::uri::Host as UriHost;

// via-ttl = "ttl" EQUAL ttl (1*3 DIGIT)
fn via_ttl(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(pair(tag_no_case(b"ttl"), equal),
                 take_while_m_n(1, 3, |c: u8| c.is_ascii_digit())),
        |b| {
            let s = str::from_utf8(b)
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))?;
            s.parse::<u8>()
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Digit)))
                .map(Param::Ttl)
        }
    )(input)
}

// via-maddr = "maddr" EQUAL host
fn via_maddr(input: &[u8]) -> ParseResult<Param> {
    map(
        preceded(pair(tag_no_case(b"maddr"), equal), host),
        |h: UriHost| Param::Maddr(h)
    )(input)
}

// via-received = "received" EQUAL (IPv4address / IPv6address)
fn via_received(input: &[u8]) -> ParseResult<Param> {
     map(
        preceded(pair(tag_no_case(b"received"), equal), host), // host parser handles IPs
        |h: UriHost| Param::Received(h)
    )(input)
}

// via-branch = "branch" EQUAL token
fn via_branch(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(pair(tag_no_case(b"branch"), equal), token),
        |b| str::from_utf8(b).map(|s| Param::Branch(s.to_string()))
    )(input)
}

// via-params = via-ttl / via-maddr / via-received / via-branch / via-extension
// via-extension = generic-param
// This function parses ONE via parameter.
pub(crate) fn via_param_item(input: &[u8]) -> ParseResult<Param> {
    alt((
        via_ttl,
        via_maddr,
        via_received,
        via_branch,
        generic_param, // Must be last
    ))(input)
}

// The list parsing *( SEMI via-params ) should happen in the main via parser (via/mod.rs)
// using semicolon_separated_params0(via_param_item) 