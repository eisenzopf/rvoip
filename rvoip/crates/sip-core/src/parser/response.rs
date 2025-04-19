use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_while, take_while1},
    character::complete::{char, digit1, line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize, verify},
    multi::{many0, many1, many_till, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    bytes::complete::take,
    combinator::rest,
    Err, IResult, Needed,
};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::types::{Message, Response, StatusCode};
use crate::version::Version;
use bytes::Bytes;

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

/// Helper to parse headers and body
fn parse_headers_and_body(input: &str) -> IResult<&str, (Vec<Header>, Bytes), nom::error::Error<&str>> {
    map(
        tuple((
            terminated(many0(super::headers::header_parser), super::utils::crlf),
            rest
        )),
        |(headers, body_str)| (headers, Bytes::from(body_str))
    )(input)
}

/// Top-level parser for a complete SIP response
pub fn response_parser(input: &str) -> IResult<&str, Message, nom::error::Error<&str>> {
    // 1. Parse the start line and consume CRLF
    let (rest_after_start_line, (version, status, reason)) = 
        terminated(parse_response_line, super::utils::crlf)(input)?;

    // 2. Parse headers and body from the rest of the input
    let (remaining_input_after_all, (headers, body)) = 
        parse_headers_and_body(rest_after_start_line)?;

    // 3. Construct the Response (all components are now owned)
    let response = Response {
        version,
        status,
        reason: if reason.is_empty() { None } else { Some(reason) },
        headers,
        body,
    };
    
    // 4. Wrap in Message enum
    Ok((remaining_input_after_all, Message::Response(response)))
} 