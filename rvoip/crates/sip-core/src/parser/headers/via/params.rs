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
use crate::types::uri::Host as UriHost;

/// Parser for the ttl parameter of a Via header (RFC 3261 Section 20.42)
/// ttl is a 1*3DIGIT value (0-255)
/// Example: ttl=10
fn via_ttl(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(pair(tag_no_case(b"ttl"), equal),
                 take_while_m_n(1, 3, |c: u8| c.is_ascii_digit())),
        |b| {
            let s = str::from_utf8(b)
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))?;
            let parsed = s.parse::<u16>()
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Digit)))?;
            
            // Cap the TTL value at 255 as per the SIP specification
            let capped_ttl = if parsed > 255 { 255 } else { parsed as u8 };
            Ok::<Param, nom::Err<NomError<&[u8]>>>(Param::Ttl(capped_ttl))
        }
    )(input)
}

/// Parser for the maddr parameter of a Via header (RFC 3261 Section 20.42)
/// maddr is a host value (domain or IP address)
/// Example: maddr=example.com
fn via_maddr(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(pair(tag_no_case(b"maddr"), equal), host),
        |h: UriHost| {
            // Convert the host to a string representation for the Param
            Ok::<Param, nom::Err<NomError<&[u8]>>>(Param::Maddr(h.to_string()))
        }
    )(input)
}

/// Parser for the received parameter of a Via header (RFC 3261 Section 20.42)
/// received is an IPv4address or IPv6address
/// Example: received=192.0.2.1
fn via_received(input: &[u8]) -> ParseResult<Param> {
     map_res(
        preceded(pair(tag_no_case(b"received"), equal), host), // host parser handles IPs
        |h: UriHost| {
            match h {
                UriHost::Address(ip) => Ok(Param::Received(ip)),
                UriHost::Domain(domain) => {
                    // Try to convert domain to IP if possible
                    if let Ok(ip) = domain.parse::<std::net::IpAddr>() {
                        Ok(Param::Received(ip))
                    } else {
                        // Invalid received parameter - must be an IP address
                        Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Tag)))
                    }
                }
            }
        }
    )(input)
}

/// Parser for the branch parameter of a Via header (RFC 3261 Section 20.42)
/// branch is a token value, must start with z9hG4bK for RFC 3261 compliant requests
/// Example: branch=z9hG4bK776asdhds
fn via_branch(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(pair(tag_no_case(b"branch"), equal), token),
        |b| {
            let branch_str = str::from_utf8(b)
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))?
                .to_string();
            
            // Note: RFC 3261 compliant branch parameter must start with magic cookie z9hG4bK
            // But we don't enforce this at parse time to allow parsing legacy implementations
            
            Ok::<Param, nom::Err<NomError<&[u8]>>>(Param::Branch(branch_str))
        }
    )(input)
}

/// Parser for a single via parameter (RFC 3261 Section 20.42)
/// This function combines all the specific via param parsers in priority order
/// Returns a Param enum representing the parsed parameter
pub fn via_param_item(input: &[u8]) -> ParseResult<Param> {
    alt((
        via_ttl,
        via_maddr,
        via_received,
        via_branch,
        generic_param, // Fallback for any other parameter (must be last)
    ))(input)
}

// The list parsing *( SEMI via-params ) should happen in the main via parser (via/mod.rs)
// using semicolon_separated_params0(via_param_item) 