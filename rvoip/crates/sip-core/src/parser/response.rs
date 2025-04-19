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

/// Parser for a complete SIP response, mapped to Message enum
pub fn response_parser(input: &str) -> IResult<&str, Message, nom::error::Error<&str>> {
    map(
        response_parser_inner, 
        |((version, status, reason), headers, body)| {
            let response = Response {
                version,
                status,
                reason: if reason.is_empty() { None } else { Some(reason) },
                headers,
                body,
            };
            Message::Response(response)
        }
    )(input)
} 

// Change return type to tuple of components
fn response_parser_inner(input: &str) -> IResult<&str, ((Version, StatusCode, String), Vec<Header>, Bytes), nom::error::Error<&str>> {
    // Use tuple combinator
    map(
        tuple((
            // 1. Parse start line and consume CRLF
            terminated(parse_response_line, super::utils::crlf),
            // 2. Parse headers and consume CRLF
            terminated(many0(super::headers::header_parser), super::utils::crlf),
            // 3. Take the rest as the body (&str)
            rest
        )),
        // Map the resulting tuple ((Version, StatusCode, String), Vec<Header>, &str) to include owned Bytes
        |(start_line_components, headers, body_str)| {
            (start_line_components, headers, Bytes::from(body_str))
        }
    )(input)
} 