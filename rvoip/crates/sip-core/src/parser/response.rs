use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_while, take_while1},
    character::complete::{char, digit1, line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize, verify},
    multi::{many0, many1, many_till, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    Err, IResult, Needed,
};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::types::{Message, Response, StatusCode};
use crate::version::Version;

/// Parser for a SIP response line
pub fn parse_response_line(input: &str) -> IResult<&str, (Version, StatusCode, String)> {
    let (input, version) = map_res(
        take_till(|c| c == ' '),
        |s: &str| Version::from_str(s)
    )(input)?;

    let (input, _) = space1(input)?;

    // First map to u16, then handle potential errors when creating StatusCode
    let (input, status_code) = map_res(
        digit1,
        |s: &str| s.parse::<u16>()
    )(input)?;

    // Convert u16 to StatusCode
    let status = match StatusCode::from_u16(status_code) {
        Ok(status) => status,
        Err(_) => return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag))), // TODO: Better error
    };

    let (input, _) = space1(input)?;

    let (input, reason) = map(
        take_till(|c| c == '\r' || c == '\n'),
        |s: &str| s.to_string()
    )(input)?;

    Ok((input, (version, status, reason)))
}

// Rename original parser
fn response_parser_inner(input: &str) -> IResult<&str, Response> {
    // Parse the response line and consume the following CRLF
    let (input, (version, status, reason)) = terminated(parse_response_line, super::utils::crlf)(input)?;

    // Parse headers using the remaining input
    let (input, headers) = terminated(
        many0(super::headers::header_parser),
        super::utils::crlf
    )(input)?;

    // Create the response
    let mut response = Response {
        version,
        status,
        reason: if reason.is_empty() { None } else { Some(reason) },
        headers: vec![], // Initialize headers vec
        body: Default::default(),
    };

    // Add headers
    response.headers = headers; // Assign parsed headers

    // Parse the body if present - preserve it exactly as is without modifying line endings
    if !input.is_empty() {
        response.body = input.into();
    }

    Ok(("", response))
}

/// Parser for a complete SIP response, mapped to Message enum
pub fn response_parser(input: &str) -> IResult<&str, Message> {
    map(response_parser_inner, Message::Response)(input)
} 