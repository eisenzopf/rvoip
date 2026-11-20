// Parser for Via header parameters as per RFC 3261 Section 20.42
// via-params = via-ttl / via-maddr / via-received / via-branch / via-extension
// via-ttl = "ttl" EQUAL ttl
// via-maddr = "maddr" EQUAL host
// via-received = "received" EQUAL host
// via-branch = "branch" EQUAL token
// via-extension = generic-param

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while_m_n},
    character::complete::digit1,
    combinator::{map_res, opt},
    error::{Error as NomError, ErrorKind, ParseError},
    sequence::{pair, preceded},
};
use std::str;

// Import from base modules
use crate::parser::common_params::generic_param;
use crate::parser::separators::equal;
use crate::parser::token::token;
use crate::parser::uri::host::host; // For maddr and received
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::uri::Host as UriHost;

/// Parser for the ttl parameter of a Via header (RFC 3261 Section 20.42)
/// ttl is a 1*3DIGIT value (0-255)
/// Example: ttl=10
fn via_ttl(input: &[u8]) -> ParseResult<'_, Param> {
    map_res(
        preceded(
            pair(tag_no_case(b"ttl"), equal),
            take_while_m_n(1, 3, |c: u8| c.is_ascii_digit()),
        ),
        |b| {
            let s = str::from_utf8(b).map_err(|_| {
                nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char))
            })?;
            let parsed = s.parse::<u16>().map_err(|_| {
                nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Digit))
            })?;

            // Cap the TTL value at 255 as per the SIP specification
            let capped_ttl = if parsed > 255 { 255 } else { parsed as u8 };
            Ok::<Param, nom::Err<NomError<&[u8]>>>(Param::Ttl(capped_ttl))
        },
    )(input)
}

/// Parser for the maddr parameter of a Via header (RFC 3261 Section 20.42)
/// maddr is a host value (domain or IP address)
/// Example: maddr=example.com
fn via_maddr(input: &[u8]) -> ParseResult<'_, Param> {
    map_res(
        preceded(pair(tag_no_case(b"maddr"), equal), host),
        |h: UriHost| {
            // maddr = host: an IPv6 address keeps its IPv6reference form.
            let host = match h {
                UriHost::Address(std::net::IpAddr::V6(address)) => format!("[{address}]"),
                other => other.to_string(),
            };
            Ok::<Param, nom::Err<NomError<&[u8]>>>(Param::Maddr(host))
        },
    )(input)
}

/// Parser for the received parameter of a Via header.
///
/// via-received = "received" EQUAL (IPv4address / IPv6address) (RFC 3261
/// §20.42): an IPv6 address without "[" "]". RFC 5118 §4.5 requires
/// accepting it with or without the delimiters; it is always sent without.
fn via_received(input: &[u8]) -> ParseResult<'_, Param> {
    let (rest, _) = pair(tag_no_case(b"received"), equal)(input)?;
    let (address, rest) = match rest.strip_prefix(b"[") {
        Some(inner) => {
            let close = inner.iter().position(|&c| c == b']').ok_or_else(|| {
                nom::Err::Error(NomError::from_error_kind(input, ErrorKind::Char))
            })?;
            (&inner[..close], &inner[close + 1..])
        }
        None => {
            let length = rest
                .iter()
                .take_while(|&&c| c.is_ascii_hexdigit() || c == b':' || c == b'.')
                .count();
            (&rest[..length], &rest[length..])
        }
    };
    let ip = str::from_utf8(address)
        .ok()
        .and_then(|text| text.parse::<std::net::IpAddr>().ok())
        .ok_or_else(|| nom::Err::Error(NomError::from_error_kind(input, ErrorKind::Tag)))?;
    Ok((rest, Param::Received(ip)))
}

/// Parser for the branch parameter of a Via header (RFC 3261 Section 20.42)
/// branch is a token value, must start with z9hG4bK for RFC 3261 compliant requests
/// Example: branch=z9hG4bK776asdhds
fn via_branch(input: &[u8]) -> ParseResult<'_, Param> {
    map_res(preceded(pair(tag_no_case(b"branch"), equal), token), |b| {
        let branch_str = str::from_utf8(b)
            .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))?
            .to_string();

        // Note: RFC 3261 compliant branch parameter must start with magic cookie z9hG4bK
        // But we don't enforce this at parse time to allow parsing legacy implementations

        Ok::<Param, nom::Err<NomError<&[u8]>>>(Param::Branch(branch_str))
    })(input)
}

/// Parser for the rport parameter of a Via header (RFC 3261 Section 18.2.1)
/// rport can be a flag or have a port value in responses
/// Examples: rport (request) or rport=5060 (response)
fn via_rport(input: &[u8]) -> ParseResult<'_, Param> {
    let (input, _) = tag_no_case(b"rport")(input)?;
    let (input, port_val) = opt(preceded(
        equal,
        map_res(digit1, |b| {
            let s = str::from_utf8(b).map_err(|_| {
                nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char))
            })?;
            let port = s.parse::<u16>().map_err(|_| {
                nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Digit))
            })?;
            Ok::<u16, nom::Err<NomError<&[u8]>>>(port)
        }),
    ))(input)?;

    Ok((input, Param::Rport(port_val)))
}

/// Parser for a single via parameter (RFC 3261 Section 20.42)
/// This function combines all the specific via param parsers in priority order
/// Returns a Param enum representing the parsed parameter
pub fn via_param_item(input: &[u8]) -> ParseResult<'_, Param> {
    alt((
        via_ttl,
        via_maddr,
        via_received,
        via_branch,
        via_rport,
        generic_param, // Fallback for any other parameter (must be last)
    ))(input)
}

// The list parsing *( SEMI via-params ) should happen in the main via parser (via/mod.rs)
// using semicolon_separated_params0(via_param_item)
