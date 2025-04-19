use std::str::FromStr;
use nom::{
    bytes::complete::{take_till},
    character::complete::{digit1, space1},
    combinator::{map, map_res},
    IResult,
};
// Keep Result for FromStr impls if needed elsewhere
use crate::error::{Error, Result};
use crate::types::{StatusCode};
use crate::version::Version;

/// Parser for a SIP response line
/// Returns components needed by IncrementalParser
pub fn parse_response_line(input: &str) -> IResult<&str, (Version, StatusCode, String)> {
    let (input, version) = map_res(
        take_till(|c| c == ' '),
        |s: &str| Version::from_str(s)
    )(input)?;

    let (input, _) = space1(input)?;

    let (input, status_code) = map_res(
        digit1,
        |s: &str| s.parse::<u16>()
    )(input)?;

    let status = match StatusCode::from_u16(status_code) {
        Ok(status) => status,
        // Use Failure for semantic errors, match nom::error::Error structure
        Err(_) => return Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify))), 
    };

    let (input, _) = space1(input)?;

    let (input, reason) = map(
        take_till(|c| c == '\r' || c == '\n'),
        |s: &str| s.to_string()
    )(input)?;

    Ok((input, (version, status, reason)))
} 

// Removed response_parser, response_parser_nom, parse_headers_and_body functions. 